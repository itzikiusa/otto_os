//! Tail snapshot slicing and cursor-based older-turn pagination for
//! `GET /runtime-sessions/{id}/events` so long conversations open in <50 ms on
//! mobile and desktop instead of serializing thousands of historical items at once.

use std::collections::HashSet;

use otto_core::agent_runtime::{
    PlanArtifact, ProviderTurn, RuntimeReplay, RuntimeRequest, RuntimeRequestStatus,
    RuntimeSnapshot, TurnItem,
};
use otto_core::{Id, Result};

use super::{
    flush_held, poll_driver_sideband, AgentRuntime, Projection, QueuedRecord, MAX_REPLAY_BYTES,
    MAX_REPLAY_EVENTS,
};

const MAX_TAIL_TURNS: usize = 200;
const MAX_PAGE_TURNS: usize = 100;

impl Projection {
    /// Return a tail snapshot bounded to the newest `max_turns` turns (always
    /// keeping any non-terminal turn, open item, or pending request visible).
    pub(super) fn snapshot_tail(&self, max_turns: usize) -> RuntimeSnapshot {
        let cap = max_turns.clamp(1, MAX_TAIL_TURNS);
        let mut all_turns: Vec<ProviderTurn> = self.turns.values().cloned().collect();
        all_turns.sort_by_key(|t| t.ordinal);
        if all_turns.len() <= cap {
            return self.snapshot();
        }

        let pending_turn_ids: HashSet<&str> = self
            .requests
            .values()
            .filter(|r| r.status == RuntimeRequestStatus::Pending)
            .filter_map(|r| r.turn_id.as_deref())
            .collect();

        let mut split = all_turns.len() - cap;
        for (idx, t) in all_turns[..split].iter().enumerate() {
            if !t.status.is_terminal() || pending_turn_ids.contains(t.id.as_str()) {
                split = idx;
                break;
            }
        }
        if split == 0 {
            return self.snapshot();
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
            .cloned()
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
    pub(super) fn snapshot_before(
        &self,
        before_turn_ordinal: u32,
        limit_turns: usize,
    ) -> RuntimeSnapshot {
        let cap = limit_turns.clamp(1, MAX_PAGE_TURNS);
        let mut all_turns: Vec<ProviderTurn> = self.turns.values().cloned().collect();
        all_turns.sort_by_key(|t| t.ordinal);

        let older_end = all_turns.partition_point(|t| t.ordinal < before_turn_ordinal);
        let older_all = &all_turns[..older_end];
        if older_all.is_empty() {
            return RuntimeSnapshot::default();
        }

        let split = older_all.len().saturating_sub(cap);
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
            .cloned()
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
        if before_turn_ordinal.is_none() && tail_turns.is_none() {
            return self.replay(id, after_sequence).await;
        }
        let live = self.live(id).await?;
        if before_turn_ordinal.is_none() {
            for _ in 0..4 {
                if !poll_driver_sideband(&self.inner, &live, None).await {
                    break;
                }
            }
        }
        let mut st = live.lock();
        flush_held(&self.inner, &live, &mut st);
        let last_sequence = st.next_seq - 1;
        if let Some(before) = before_turn_ordinal {
            let page = st.proj.snapshot_before(before, limit_turns.unwrap_or(20));
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
        let snap = match tail_turns {
            Some(n) if n > 0 => st.proj.snapshot_tail(n),
            _ => st.proj.snapshot(),
        };
        Ok(RuntimeReplay::Snapshot {
            snapshot_sequence: last_sequence,
            snapshot: Box::new(snap),
        })
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
}
