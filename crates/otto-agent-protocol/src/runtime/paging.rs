//! Tail snapshot slicing and cursor-based older-turn pagination for
//! `GET /runtime-sessions/{id}/events` so long conversations open in <50 ms on
//! mobile and desktop instead of serializing thousands of historical items at once.

use std::collections::HashSet;

use otto_core::agent_runtime::{
    PlanArtifact, ProviderTurn, RuntimeReplay, RuntimeRequest, RuntimeRequestStatus,
    RuntimeSnapshot, TurnItem, TurnItemKind,
};
use otto_core::{Id, Result};

use super::{
    flush_held, poll_driver_sideband, AgentRuntime, Projection, QueuedRecord, MAX_REPLAY_BYTES,
    MAX_REPLAY_EVENTS,
};

const MAX_TAIL_TURNS: usize = 200;
const MAX_PAGE_TURNS: usize = 100;
const COMPACT_TAIL_MAX_ITEMS: usize = 320;
const COMPACT_MAX_ITEMS_WITH_LINES: usize = 600;
const MIN_MAX_LINES: usize = 100;
const MAX_MAX_LINES: usize = 50_000;

fn text_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        1 + s.bytes().filter(|&b| b == b'\n').count() + s.len() / 100
    }
}

fn estimate_item_lines(item: &TurnItem, compact: bool) -> usize {
    let omit_settled = compact && !item.status.is_open() && !item.is_streaming();
    let err_lines = item.error.as_deref().map_or(0, text_lines);
    match &item.kind {
        TurnItemKind::UserMessage { text, attachments } => {
            text_lines(text).max(1) + attachments.len()
        }
        TurnItemKind::AssistantMessage { text, .. } | TurnItemKind::Reasoning { text, .. } => {
            text_lines(text).max(1)
        }
        TurnItemKind::TodoList {
            steps, explanation, ..
        } => 1 + steps.len() + explanation.as_deref().map_or(0, text_lines),
        TurnItemKind::FileChange {
            diff_str,
            old_str,
            new_str,
            ..
        } => {
            if omit_settled {
                2 + err_lines
            } else {
                1 + diff_str.as_deref().map_or(0, text_lines)
                    + old_str.as_deref().map_or(0, text_lines)
                    + new_str.as_deref().map_or(0, text_lines)
                    + err_lines
            }
        }
        TurnItemKind::CommandExecution { input, output, .. } => {
            if omit_settled {
                2 + err_lines
            } else {
                text_lines(input).max(1) + output.as_deref().map_or(0, text_lines) + err_lines
            }
        }
        TurnItemKind::DynamicTool {
            input,
            output,
            images,
            ..
        } => {
            if omit_settled {
                2 + images.len() + err_lines
            } else {
                2 + input.as_str().map_or(1, text_lines)
                    + output.as_deref().map_or(0, text_lines)
                    + images.len()
                    + err_lines
            }
        }
        TurnItemKind::WebSearch { patterns } => 1 + patterns.len().min(8),
        TurnItemKind::ApprovalRequest {
            prompt,
            reason,
            options,
            ..
        } => {
            1 + options.len()
                + prompt.as_deref().map_or(0, text_lines)
                + reason.as_deref().map_or(0, text_lines)
        }
        TurnItemKind::SessionBoundary => 1,
        TurnItemKind::SystemNotice { message } => text_lines(message).max(1),
        TurnItemKind::Subagent { prompt, output, .. } => {
            if omit_settled {
                2 + output.as_deref().map_or(0, |s| text_lines(s).min(12)) + err_lines
            } else {
                2 + prompt.as_deref().map_or(0, text_lines)
                    + output.as_deref().map_or(0, text_lines)
                    + err_lines
            }
        }
        TurnItemKind::Error { failure } => text_lines(&failure.message).max(1),
    }
}

impl Projection {
    /// Return a tail snapshot bounded to the newest `max_turns` turns (always
    /// keeping any non-terminal turn, open item, or pending request visible).
    #[cfg(test)]
    pub(super) fn snapshot_tail(&self, max_turns: usize) -> RuntimeSnapshot {
        self.snapshot_tail_opts(max_turns, false)
    }

    #[cfg(test)]
    pub(super) fn snapshot_tail_opts(&self, max_turns: usize, compact: bool) -> RuntimeSnapshot {
        self.snapshot_tail_with_lines(max_turns, compact, None)
    }

    pub(super) fn snapshot_tail_with_lines(
        &self,
        max_turns: usize,
        compact: bool,
        max_lines: Option<usize>,
    ) -> RuntimeSnapshot {
        let cap = max_turns.clamp(1, MAX_TAIL_TURNS);
        let mut all_turns: Vec<ProviderTurn> = self.turns.values().cloned().collect();
        all_turns.sort_by_key(|t| t.ordinal);
        if all_turns.is_empty() || (!compact && max_lines.is_none() && all_turns.len() <= cap) {
            return self.snapshot_compact(compact);
        }

        let pending_turn_ids: HashSet<&str> = self
            .requests
            .values()
            .filter(|r| r.status == RuntimeRequestStatus::Pending)
            .filter_map(|r| r.turn_id.as_deref())
            .collect();

        let mut split = all_turns.len().saturating_sub(cap);
        let mut earliest_pinned: Option<usize> = None;
        for (idx, t) in all_turns.iter().enumerate() {
            if !t.status.is_terminal() || pending_turn_ids.contains(t.id.as_str()) {
                earliest_pinned = Some(idx);
                break;
            }
        }
        if let Some(pin_idx) = earliest_pinned {
            split = split.min(pin_idx);
        }

        if let Some(raw_lines) = max_lines {
            let line_budget = raw_lines.clamp(MIN_MAX_LINES, MAX_MAX_LINES);
            let mut per_turn: std::collections::HashMap<&str, (usize, usize)> =
                std::collections::HashMap::new();
            for item in self.items.values() {
                if let Some(tid) = item.turn_id.as_deref() {
                    let entry = per_turn.entry(tid).or_insert((0, 0));
                    entry.0 = entry.0.saturating_add(estimate_item_lines(item, compact));
                    entry.1 += 1;
                }
            }
            let mut acc_lines = 0usize;
            let mut acc_items = 0usize;
            let mut line_split = all_turns.len().saturating_sub(1);
            for idx in (split..all_turns.len()).rev() {
                let (t_lines, t_items) = per_turn
                    .get(all_turns[idx].id.as_str())
                    .copied()
                    .unwrap_or((1, 0));
                let next_lines = acc_lines.saturating_add(t_lines.max(1));
                let next_items = acc_items.saturating_add(t_items);
                if idx < all_turns.len() - 1
                    && (next_lines > line_budget
                        || (compact && next_items > COMPACT_MAX_ITEMS_WITH_LINES))
                {
                    line_split = idx + 1;
                    break;
                }
                acc_lines = next_lines;
                acc_items = next_items;
                line_split = idx;
            }
            split = match earliest_pinned {
                Some(pin_idx) => line_split.min(pin_idx),
                None => line_split,
            };
        } else if compact && all_turns.len().saturating_sub(split) > 2 {
            let mut per_turn: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for item in self.items.values() {
                if let Some(tid) = item.turn_id.as_deref() {
                    *per_turn.entry(tid).or_default() += 1;
                }
            }
            let mut kept_count: usize = all_turns[split..]
                .iter()
                .map(|t| per_turn.get(t.id.as_str()).copied().unwrap_or(0))
                .sum();
            while all_turns.len() - split > 2 && kept_count > COMPACT_TAIL_MAX_ITEMS {
                let cand = &all_turns[split];
                if !cand.status.is_terminal() || pending_turn_ids.contains(cand.id.as_str()) {
                    break;
                }
                kept_count =
                    kept_count.saturating_sub(per_turn.get(cand.id.as_str()).copied().unwrap_or(0));
                split += 1;
            }
        }

        if split == 0 {
            return self.snapshot_compact(compact);
        }

        let older_turn_ids: HashSet<&str> =
            all_turns[..split].iter().map(|t| t.id.as_str()).collect();
        let kept_turns = all_turns[split..].to_vec();
        let kept_turn_ids: HashSet<&str> = kept_turns.iter().map(|t| t.id.as_str()).collect();

        let max_older_item_ord = self
            .items
            .values()
            .filter(|i| {
                i.turn_id
                    .as_deref()
                    .is_some_and(|tid| older_turn_ids.contains(tid))
            })
            .map(|i| i.ordinal)
            .max()
            .unwrap_or(0);

        let mut kept_item_ids: HashSet<&str> = self
            .items
            .values()
            .filter(|i| match i.turn_id.as_deref() {
                Some(tid) => kept_turn_ids.contains(tid) || i.status.is_open(),
                None => i.ordinal > max_older_item_ord || i.status.is_open(),
            })
            .map(|i| i.id.as_str())
            .collect();

        close_subagent_family(&self.items, &mut kept_item_ids);

        let mut turn_items: Vec<TurnItem> = self
            .items
            .values()
            .filter(|i| kept_item_ids.contains(i.id.as_str()))
            .map(|i| {
                if compact {
                    i.compact_for_snapshot()
                } else {
                    i.clone()
                }
            })
            .collect();
        turn_items.sort_by_key(|i| i.ordinal);

        let mut runtime_requests: Vec<RuntimeRequest> = self
            .requests
            .values()
            .filter(|r| {
                r.status == RuntimeRequestStatus::Pending
                    || r.turn_id
                        .as_deref()
                        .is_none_or(|tid| kept_turn_ids.contains(tid))
            })
            .cloned()
            .collect();
        runtime_requests.sort_by_key(|r| r.created_at);

        let mut plans: Vec<PlanArtifact> = self
            .plans
            .values()
            .filter(|p| {
                p.turn_id
                    .as_deref()
                    .is_none_or(|tid| kept_turn_ids.contains(tid))
            })
            .cloned()
            .collect();
        plans.sort_by(|a, b| a.id.cmp(&b.id));

        let oldest_turn_ordinal = kept_turns.first().map(|t| t.ordinal);
        RuntimeSnapshot {
            provider_session: self.session.clone(),
            provider_thread: self.thread.clone(),
            turns: kept_turns,
            turn_items,
            runtime_requests,
            plans,
            queued_prompts: self.queue.iter().map(QueuedRecord::public).collect(),
            has_older: true,
            oldest_turn_ordinal,
            older_turns: split as u32,
        }
    }

    /// Return up to `limit_turns` turns strictly older than `before_turn_ordinal`
    /// along with their items, requests, plans, and any turnless divider items
    /// falling in the ordinal gap immediately preceding `before_turn_ordinal`.
    #[cfg(test)]
    pub(super) fn snapshot_before(
        &self,
        before_turn_ordinal: u32,
        limit_turns: usize,
    ) -> RuntimeSnapshot {
        self.snapshot_before_opts(before_turn_ordinal, limit_turns, false)
    }

    #[cfg(test)]
    pub(super) fn snapshot_before_opts(
        &self,
        before_turn_ordinal: u32,
        limit_turns: usize,
        compact: bool,
    ) -> RuntimeSnapshot {
        self.snapshot_before_with_lines(before_turn_ordinal, limit_turns, compact, None)
    }

    pub(super) fn snapshot_before_with_lines(
        &self,
        before_turn_ordinal: u32,
        limit_turns: usize,
        compact: bool,
        max_lines: Option<usize>,
    ) -> RuntimeSnapshot {
        let cap = limit_turns.clamp(1, MAX_PAGE_TURNS);
        let mut all_turns: Vec<ProviderTurn> = self.turns.values().cloned().collect();
        all_turns.sort_by_key(|t| t.ordinal);

        let older_end = all_turns.partition_point(|t| t.ordinal < before_turn_ordinal);
        let older_all = &all_turns[..older_end];
        if older_all.is_empty() {
            return RuntimeSnapshot::default();
        }

        let mut split = older_all.len().saturating_sub(cap);
        if let Some(raw_lines) = max_lines {
            let line_budget = raw_lines.clamp(MIN_MAX_LINES, MAX_MAX_LINES);
            let mut per_turn: std::collections::HashMap<&str, (usize, usize)> =
                std::collections::HashMap::new();
            for item in self.items.values() {
                if let Some(tid) = item.turn_id.as_deref() {
                    let entry = per_turn.entry(tid).or_insert((0, 0));
                    entry.0 = entry.0.saturating_add(estimate_item_lines(item, compact));
                    entry.1 += 1;
                }
            }
            let mut acc_lines = 0usize;
            let mut acc_items = 0usize;
            let mut line_split = older_all.len().saturating_sub(1);
            for idx in (split..older_all.len()).rev() {
                let (t_lines, t_items) = per_turn
                    .get(older_all[idx].id.as_str())
                    .copied()
                    .unwrap_or((1, 0));
                let next_lines = acc_lines.saturating_add(t_lines.max(1));
                let next_items = acc_items.saturating_add(t_items);
                if idx < older_all.len() - 1
                    && (next_lines > line_budget
                        || (compact && next_items > COMPACT_MAX_ITEMS_WITH_LINES))
                {
                    line_split = idx + 1;
                    break;
                }
                acc_lines = next_lines;
                acc_items = next_items;
                line_split = idx;
            }
            split = line_split;
        }
        let page_turns = older_all[split..].to_vec();
        let page_turn_ids: HashSet<&str> = page_turns.iter().map(|t| t.id.as_str()).collect();
        let even_older_ids: HashSet<&str> =
            older_all[..split].iter().map(|t| t.id.as_str()).collect();

        let lower_excl = if split == 0 {
            0
        } else {
            self.items
                .values()
                .filter(|i| {
                    i.turn_id
                        .as_deref()
                        .is_some_and(|tid| even_older_ids.contains(tid))
                })
                .map(|i| i.ordinal)
                .max()
                .unwrap_or(0)
        };

        let prev_max_ord = self
            .items
            .values()
            .filter(|i| {
                i.turn_id
                    .as_deref()
                    .is_some_and(|tid| page_turn_ids.contains(tid) || even_older_ids.contains(tid))
            })
            .map(|i| i.ordinal)
            .max()
            .unwrap_or(0);

        let mut kept_item_ids: HashSet<&str> = self
            .items
            .values()
            .filter(|i| match i.turn_id.as_deref() {
                Some(tid) => page_turn_ids.contains(tid),
                None => (split == 0 || i.ordinal > lower_excl) && i.ordinal <= prev_max_ord,
            })
            .map(|i| i.id.as_str())
            .collect();

        close_subagent_family(&self.items, &mut kept_item_ids);

        let mut turn_items: Vec<TurnItem> = self
            .items
            .values()
            .filter(|i| kept_item_ids.contains(i.id.as_str()))
            .map(|i| {
                if compact {
                    i.compact_for_snapshot()
                } else {
                    i.clone()
                }
            })
            .collect();
        turn_items.sort_by_key(|i| i.ordinal);

        let mut runtime_requests: Vec<RuntimeRequest> = self
            .requests
            .values()
            .filter(|r| {
                r.turn_id
                    .as_deref()
                    .is_some_and(|tid| page_turn_ids.contains(tid))
            })
            .cloned()
            .collect();
        runtime_requests.sort_by_key(|r| r.created_at);

        let mut plans: Vec<PlanArtifact> = self
            .plans
            .values()
            .filter(|p| {
                p.turn_id
                    .as_deref()
                    .is_some_and(|tid| page_turn_ids.contains(tid))
            })
            .cloned()
            .collect();
        plans.sort_by(|a, b| a.id.cmp(&b.id));

        let oldest_turn_ordinal = page_turns.first().map(|t| t.ordinal);
        RuntimeSnapshot {
            provider_session: None,
            provider_thread: None,
            turns: page_turns,
            turn_items,
            runtime_requests,
            plans,
            queued_prompts: Vec::new(),
            has_older: split > 0,
            oldest_turn_ordinal,
            older_turns: split as u32,
        }
    }
}

fn close_subagent_family<'a>(
    items: &'a std::collections::HashMap<Id, TurnItem>,
    kept: &mut HashSet<&'a str>,
) {
    let mut parents_to_add: Vec<&'a str> = Vec::new();
    for item in items.values() {
        if kept.contains(item.id.as_str()) {
            if let Some(pid) = item.parent_item_id.as_deref() {
                if let Some(parent) = items.get(pid) {
                    parents_to_add.push(parent.id.as_str());
                }
            }
        }
    }
    for p in parents_to_add {
        kept.insert(p);
    }
    let children_to_add: Vec<&'a str> = items
        .values()
        .filter(|i| {
            i.parent_item_id
                .as_deref()
                .is_some_and(|pid| kept.contains(pid))
        })
        .map(|i| i.id.as_str())
        .collect();
    for c in children_to_add {
        kept.insert(c);
    }
}

impl AgentRuntime {
    /// Paginated variant of [`Self::replay`]: supports `tail_turns` on initial
    /// or ring-miss snapshots, and `before_turn_ordinal` + `limit_turns` to
    /// fetch older history pages without blocking on the full session payload.
    pub async fn replay_paged(
        &self,
        id: &Id,
        after_sequence: u64,
        tail_turns: Option<usize>,
        before_turn_ordinal: Option<u32>,
        limit_turns: Option<usize>,
    ) -> Result<RuntimeReplay> {
        self.replay_paged_with_compact(
            id,
            after_sequence,
            tail_turns,
            before_turn_ordinal,
            limit_turns,
            false,
        )
        .await
    }

    pub async fn replay_paged_with_compact(
        &self,
        id: &Id,
        after_sequence: u64,
        tail_turns: Option<usize>,
        before_turn_ordinal: Option<u32>,
        limit_turns: Option<usize>,
        compact: bool,
    ) -> Result<RuntimeReplay> {
        self.replay_paged_opts(
            id,
            after_sequence,
            tail_turns,
            before_turn_ordinal,
            limit_turns,
            compact,
            None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn replay_paged_opts(
        &self,
        id: &Id,
        after_sequence: u64,
        tail_turns: Option<usize>,
        before_turn_ordinal: Option<u32>,
        limit_turns: Option<usize>,
        compact: bool,
        max_lines: Option<usize>,
    ) -> Result<RuntimeReplay> {
        if before_turn_ordinal.is_none() && tail_turns.is_none() && max_lines.is_none() && !compact
        {
            return self.replay(id, after_sequence).await;
        }
        let live = self.live(id).await?;
        if before_turn_ordinal.is_none() {
            otto_telemetry::context::measure("threads.sideband_poll", async {
                for _ in 0..4 {
                    if !poll_driver_sideband(&self.inner, &live, None).await {
                        break;
                    }
                }
            })
            .await;
        }
        otto_telemetry::context::measure_result("threads.snapshot_slice", async {
            let mut st = live.lock();
            flush_held(&self.inner, &live, &mut st);
            let last_sequence = st.next_seq - 1;
            if let Some(before) = before_turn_ordinal {
                let page = st.proj.snapshot_before_with_lines(
                    before,
                    limit_turns.unwrap_or(20),
                    compact,
                    max_lines,
                );
                return Ok(RuntimeReplay::Snapshot {
                    snapshot_sequence: last_sequence,
                    snapshot: Box::new(page),
                });
            }
            if after_sequence >= last_sequence {
                return Ok(RuntimeReplay::Events {
                    events: Vec::new(),
                    last_sequence,
                });
            }
            if after_sequence > 0 {
                if let Some(events) =
                    st.ring
                        .replayable(after_sequence, MAX_REPLAY_EVENTS, MAX_REPLAY_BYTES)
                {
                    return Ok(RuntimeReplay::Events {
                        events,
                        last_sequence,
                    });
                }
            }
            let snap = match (tail_turns, max_lines) {
                (Some(n), _) if n > 0 => st.proj.snapshot_tail_with_lines(n, compact, max_lines),
                (None, Some(_)) => {
                    st.proj
                        .snapshot_tail_with_lines(MAX_TAIL_TURNS, compact, max_lines)
                }
                _ => st.proj.snapshot_compact(compact),
            };
            Ok(RuntimeReplay::Snapshot {
                snapshot_sequence: last_sequence,
                snapshot: Box::new(snap),
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use otto_core::agent_runtime::{
        ProviderTurn, ProviderTurnStatus, TurnItem, TurnItemKind, TurnItemStatus, TurnOrigin,
    };

    use super::Projection;

    fn seed_projection(turn_count: u32) -> Projection {
        let mut p = Projection::default();
        let mut item_ord = 1u32;
        for ord in 1..=turn_count {
            if ord == 1 || ord == 26 {
                let bid = format!("b-{ord}");
                p.items.insert(
                    bid.clone(),
                    TurnItem {
                        id: bid,
                        turn_id: None,
                        ordinal: item_ord,
                        status: TurnItemStatus::Completed,
                        title: None,
                        error: None,
                        started_at: None,
                        completed_at: None,
                        updated_at: Utc::now(),
                        parent_item_id: None,
                        detail_omitted: false,
                        kind: TurnItemKind::SessionBoundary,
                    },
                );
                item_ord += 1;
            }
            let tid = format!("t-{ord}");
            p.turns.insert(
                tid.clone(),
                ProviderTurn {
                    id: tid.clone(),
                    ordinal: ord,
                    status: ProviderTurnStatus::Completed,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    turn_token_usage: None,
                    origin: TurnOrigin::User,
                },
            );
            let uid = format!("u-{ord}");
            p.items.insert(
                uid.clone(),
                TurnItem {
                    id: uid,
                    turn_id: Some(tid.clone()),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: TurnItemKind::UserMessage {
                        text: format!("q {ord}"),
                        attachments: Vec::new(),
                    },
                },
            );
            item_ord += 1;
            let aid = format!("a-{ord}");
            p.items.insert(
                aid.clone(),
                TurnItem {
                    id: aid,
                    turn_id: Some(tid),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: TurnItemKind::AssistantMessage {
                        message_id: format!("m-{ord}"),
                        text: format!("ans {ord}"),
                        streaming: false,
                    },
                },
            );
            item_ord += 1;
        }
        p
    }

    #[test]
    fn tail_and_before_pages_reconstruct_exact_timeline_without_gaps_or_dupes() {
        let p = seed_projection(55);
        let full = p.snapshot();

        let tail = p.snapshot_tail(20);
        assert!(tail.has_older);
        assert_eq!(tail.older_turns, 35);
        assert_eq!(tail.oldest_turn_ordinal, Some(36));
        assert_eq!(tail.turns.len(), 20);

        let page2 = p.snapshot_before(36, 20);
        assert!(page2.has_older);
        assert_eq!(page2.older_turns, 15);
        assert_eq!(page2.oldest_turn_ordinal, Some(16));
        assert_eq!(page2.turns.len(), 20);

        let page1 = p.snapshot_before(16, 20);
        assert!(!page1.has_older);
        assert_eq!(page1.older_turns, 0);
        assert_eq!(page1.oldest_turn_ordinal, Some(1));
        assert_eq!(page1.turns.len(), 15);

        let mut merged_turns = Vec::new();
        merged_turns.extend(page1.turns);
        merged_turns.extend(page2.turns);
        merged_turns.extend(tail.turns);
        assert_eq!(merged_turns, full.turns);

        let mut merged_items = Vec::new();
        merged_items.extend(page1.turn_items);
        merged_items.extend(page2.turn_items);
        merged_items.extend(tail.turn_items);
        assert_eq!(merged_items, full.turn_items);
    }

    #[test]
    fn compact_mobile_tail_snapshot_stays_under_100kb_and_50ms_on_4000_heavy_items() {
        let mut p = Projection::default();
        let mut item_ord = 1u32;
        let big_before: String = (0..12_000)
            .map(|i| format!("line {i}: original content in module.rs\n"))
            .collect();
        let mut big_after = big_before.clone();
        big_after.push_str("line 12000: newly added line\n");
        let big_cmd_out = "cargo test output line with diagnostics and timing\n".repeat(1_200);

        for ord in 1..=58u32 {
            let tid = format!("t-{ord}");
            p.turns.insert(
                tid.clone(),
                ProviderTurn {
                    id: tid.clone(),
                    ordinal: ord,
                    status: ProviderTurnStatus::Completed,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    turn_token_usage: None,
                    origin: TurnOrigin::User,
                },
            );
            let uid = format!("u-{ord}");
            p.items.insert(
                uid.clone(),
                TurnItem {
                    id: uid,
                    turn_id: Some(tid.clone()),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: TurnItemKind::UserMessage {
                        text: format!("user prompt {ord}"),
                        attachments: Vec::new(),
                    },
                },
            );
            item_ord += 1;

            for step in 0..68u32 {
                let cid = format!("w-{ord}-{step}");
                let mut item = TurnItem {
                    id: cid.clone(),
                    turn_id: Some(tid.clone()),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: Some(format!("work {ord}/{step}")),
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: if step == 0 {
                        TurnItemKind::FileChange {
                            file_name: format!("src/mod_{ord}.rs"),
                            diff_str: None,
                            old_str: Some(big_before.clone()),
                            new_str: Some(big_after.clone()),
                        }
                    } else {
                        TurnItemKind::CommandExecution {
                            input: format!("rg pattern_{step} crates/"),
                            output: Some(big_cmd_out.clone()),
                            exit_code: Some(0),
                        }
                    },
                };
                crate::diff_compact::bound_item_in_place(&mut item);
                if let TurnItemKind::FileChange {
                    diff_str,
                    old_str,
                    new_str,
                    ..
                } = &item.kind
                {
                    assert!(old_str.is_none());
                    assert!(new_str.is_none());
                    assert!(diff_str.as_ref().is_some_and(|d| d.len() <= 48 * 1024));
                }
                p.items.insert(cid, item);
                item_ord += 1;
            }

            let aid = format!("a-{ord}");
            p.items.insert(
                aid.clone(),
                TurnItem {
                    id: aid,
                    turn_id: Some(tid),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: TurnItemKind::AssistantMessage {
                        message_id: format!("m-{ord}"),
                        text: format!("Concise answer for turn {ord}."),
                        streaming: false,
                    },
                },
            );
            item_ord += 1;
        }

        assert!(p.items.len() > 4_000);
        let t0 = std::time::Instant::now();
        let snap = p.snapshot_tail_opts(8, true);
        let json = serde_json::to_vec(&snap).expect("serialize compact snapshot");
        let elapsed = t0.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "compact tail snapshot took {elapsed:?}"
        );
        assert!(
            json.len() < 100_000,
            "compact tail snapshot was {} bytes (expected < 100 KB)",
            json.len()
        );
        assert!(snap.has_older);
        assert!(snap.turns.len() >= 2 && snap.turns.len() <= 8);
        assert!(snap
            .turn_items
            .iter()
            .any(|i| i.detail_omitted && matches!(i.kind, TurnItemKind::CommandExecution { .. })));
    }

    #[test]
    fn max_lines_bounds_tail_and_older_pages_to_10k_lines_without_gaps_or_dupes() {
        let mut p = Projection::default();
        let mut item_ord = 1u32;
        let prose_950_lines: String = (0..950).map(|i| format!("line {i} reply\n")).collect();

        for ord in 1..=25u32 {
            let tid = format!("t-{ord}");
            p.turns.insert(
                tid.clone(),
                ProviderTurn {
                    id: tid.clone(),
                    ordinal: ord,
                    status: ProviderTurnStatus::Completed,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    turn_token_usage: None,
                    origin: TurnOrigin::User,
                },
            );
            let uid = format!("u-{ord}");
            p.items.insert(
                uid.clone(),
                TurnItem {
                    id: uid,
                    turn_id: Some(tid.clone()),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: TurnItemKind::UserMessage {
                        text: format!("q {ord}"),
                        attachments: Vec::new(),
                    },
                },
            );
            item_ord += 1;
            let aid = format!("a-{ord}");
            p.items.insert(
                aid.clone(),
                TurnItem {
                    id: aid,
                    turn_id: Some(tid),
                    ordinal: item_ord,
                    status: TurnItemStatus::Completed,
                    title: None,
                    error: None,
                    started_at: Some(Utc::now()),
                    completed_at: Some(Utc::now()),
                    updated_at: Utc::now(),
                    parent_item_id: None,
                    detail_omitted: false,
                    kind: TurnItemKind::AssistantMessage {
                        message_id: format!("m-{ord}"),
                        text: prose_950_lines.clone(),
                        streaming: false,
                    },
                },
            );
            item_ord += 1;
        }

        // Even with max_turns=40, max_lines=10_000 caps the initial window to <= 10k lines (< 10 turns).
        let tail = p.snapshot_tail_with_lines(40, true, Some(10_000));
        assert!(tail.has_older);
        assert!(tail.turns.len() >= 8 && tail.turns.len() <= 10);

        let mut pages = vec![tail];
        while pages.last().unwrap().has_older {
            let before = pages.last().unwrap().oldest_turn_ordinal.unwrap();
            let older = p.snapshot_before_with_lines(before, 40, true, Some(10_000));
            assert!(!older.turns.is_empty());
            assert!(older.turns.len() <= 10);
            pages.push(older);
        }

        pages.reverse();
        let merged_ords: Vec<u32> = pages
            .iter()
            .flat_map(|pg| pg.turns.iter().map(|t| t.ordinal))
            .collect();
        assert_eq!(merged_ords, (1..=25).collect::<Vec<u32>>());
    }
}
