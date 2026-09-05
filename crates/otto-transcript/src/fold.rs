//! The provider-neutral half of folding: turn/block bookkeeping, tool-result
//! attachment, artifact dedup, usage accumulation and paging. `claude.rs` and
//! `codex.rs` drive a [`Fold`] and only know their own record shapes.

use std::collections::HashMap;

use serde_json::Value;

use crate::images::ImageStore;
use crate::model::*;
use crate::util::{basename, cap_text, clip_input, mime_for_path, pr_label};

/// Prices a deduped usage sample (`model, input, output, cache_read,
/// cache_write → USD`). The server passes `otto_usage::estimate_cost` so the
/// chat header and the Usage module never disagree.
pub type PriceFn<'a> = &'a (dyn Fn(&str, u64, u64, u64, u64) -> f64 + Sync);

/// Knobs for one fold.
#[derive(Default, Clone)]
pub struct FoldOpts<'a> {
    /// Where extracted images are written. `None` → ids are still computed but
    /// nothing is written (unit tests, the history index). Owned (cheap: one
    /// path) so a live fold can hold its options across polls.
    pub images: Option<ImageStore>,
    pub price: Option<PriceFn<'a>>,
    /// The subagent tree, when the caller has read it (Claude only). Used to
    /// attach `subagent` blocks to the `Agent` tool calls that spawned them.
    /// Owned so a live fold can refresh it between snapshots.
    pub subagents: Vec<SubagentMeta>,
}

/// Total serialized bytes of turns one page may carry (design §6 "size");
/// `Folded::page` stops adding older turns past this (always ≥ 1 turn).
pub const PAGE_BYTES_BUDGET: usize = 2 * 1024 * 1024;

/// A folded turn plus the record span it came from — the span drives paging
/// (`first` is the cursor) and the live tail (`last` decides what changed).
#[derive(Debug, Clone)]
pub struct FoldedTurn {
    pub turn: Turn,
    pub first: usize,
    pub last: usize,
}

/// The whole folded file. Page it with [`Folded::page`].
#[derive(Debug, Clone)]
pub struct Folded {
    pub provider: Provider,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub turns: Vec<FoldedTurn>,
    pub stats: Stats,
    pub artifacts: Vec<Artifact>,
    /// Number of records folded (= the index the next appended record gets).
    pub record_count: usize,
    /// First user prompt (plain text), for the history index.
    pub first_prompt: Option<String>,
    /// Timestamp of the first / last dated record, for the history index.
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
}

impl Folded {
    /// The API page: the last `limit` turns whose first record index is
    /// `< before` (all turns when `before` is `None`). `has_earlier` says
    /// whether another page exists.
    pub fn page(&self, before: Option<usize>, limit: usize, subagents: Vec<SubagentMeta>) -> Transcript {
        let limit = limit.max(1);
        let eligible: Vec<&FoldedTurn> = self
            .turns
            .iter()
            .filter(|t| before.is_none_or(|b| t.first < b))
            .collect();
        let start = eligible.len().saturating_sub(limit);
        // Newest first within the window; stop once the byte budget is spent.
        let mut start = start;
        let mut bytes = 0usize;
        let mut kept = 0usize;
        for t in eligible[start..].iter().rev() {
            bytes = bytes.saturating_add(serde_json::to_vec(&t.turn).map(|v| v.len()).unwrap_or(0));
            if kept > 0 && bytes > PAGE_BYTES_BUDGET {
                break;
            }
            kept += 1;
        }
        start = eligible.len() - kept;
        let has_earlier = start > 0;
        let page = &eligible[start..];
        let cursor = page
            .first()
            .map(|t| t.first.to_string())
            .unwrap_or_else(|| before.map(|b| b.to_string()).unwrap_or_else(|| "0".into()));
        Transcript {
            session_id: self.session_id.clone(),
            provider: self.provider,
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            cursor,
            has_earlier,
            turns: page.iter().map(|t| t.turn.clone()).collect(),
            stats: self.stats.clone(),
            subagents,
            unavailable_reason: None,
        }
    }

    /// Turns touched by records at index `>= since` — the live-tail delta
    /// (a turn re-sent whole; the client replaces it by id).
    pub fn turns_since(&self, since: usize) -> Vec<Turn> {
        self.turns
            .iter()
            .filter(|t| t.last >= since)
            .map(|t| t.turn.clone())
            .collect()
    }
}

/// Where a tool call lives, for result attachment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockRef {
    pub turn: usize,
    pub block: usize,
}

/// Shared fold state. Provider adapters push turns/blocks through this so the
/// invariants (stable ids, result attachment, note placement, artifact dedup,
/// stats) live in one place.
#[derive(Clone)]
pub(crate) struct Fold<'a> {
    pub provider: Provider,
    pub opts: FoldOpts<'a>,
    pub turns: Vec<FoldedTurn>,
    pub stats: Stats,
    pub session_id: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub first_prompt: Option<String>,
    pub first_ts: Option<String>,
    pub last_ts: Option<String>,
    /// tool call id → location, for `tool_result` attachment.
    pub tool_calls: HashMap<String, BlockRef>,
    /// `Agent` tool call id → the `agentId` its result reported (Claude), so a
    /// spawned agent with no `.meta.json` sidecar still gets a `subagent` block.
    pub agent_ids: HashMap<String, String>,
    /// Notes that arrived before any turn existed, or from records that
    /// produced no turn of their own; attached to the next turn created.
    pub pending_notes: Vec<SystemNote>,
    pub pending_blocks: Vec<Block>,
    artifacts: Vec<Artifact>,
    artifact_index: HashMap<String, usize>,
    /// Deduped token totals (Claude: per `(message.id, requestId)`).
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub usage_seen: bool,
    pub cost_usd: f64,
    pub cost_fallback: Option<f64>,
}

impl<'a> Fold<'a> {
    pub fn new(provider: Provider, opts: FoldOpts<'a>) -> Self {
        Self {
            provider,
            opts,
            turns: Vec::new(),
            stats: Stats::default(),
            session_id: None,
            title: None,
            cwd: None,
            model: None,
            first_prompt: None,
            first_ts: None,
            last_ts: None,
            tool_calls: HashMap::new(),
            agent_ids: HashMap::new(),
            pending_notes: Vec::new(),
            pending_blocks: Vec::new(),
            artifacts: Vec::new(),
            artifact_index: HashMap::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            usage_seen: false,
            cost_usd: 0.0,
            cost_fallback: None,
        }
    }

    /// Note a record's timestamp for the first/last bounds.
    pub fn saw_ts(&mut self, ts: Option<&str>) {
        if let Some(ts) = ts {
            if self.first_ts.is_none() {
                self.first_ts = Some(ts.to_string());
            }
            self.last_ts = Some(ts.to_string());
        }
    }

    /// Start a turn; drains pending notes/blocks into it. Returns its index.
    pub fn new_turn(&mut self, id: String, role: Role, ts: Option<String>, model: Option<String>, idx: usize) -> usize {
        let blocks = std::mem::take(&mut self.pending_blocks);
        let turn = Turn {
            id,
            role,
            ts,
            blocks,
            duration_ms: None,
            model,
            system: std::mem::take(&mut self.pending_notes),
            reasoning_steps: 0,
        };
        self.turns.push(FoldedTurn {
            turn,
            first: idx,
            last: idx,
        });
        self.stats.turns += 1;
        self.turns.len() - 1
    }

    /// Record index `idx` touched turn `t`.
    pub fn touch(&mut self, t: usize, idx: usize) {
        if let Some(ft) = self.turns.get_mut(t) {
            ft.last = ft.last.max(idx);
        }
    }

    pub fn last_turn(&self) -> Option<usize> {
        self.turns.len().checked_sub(1)
    }

    /// Index of the most recent turn with `role`.
    pub fn last_turn_with_role(&self, role: Role) -> Option<usize> {
        self.turns.iter().rposition(|t| t.turn.role == role)
    }

    /// Append a block to turn `t` and return its block index.
    pub fn push_block(&mut self, t: usize, block: Block, idx: usize) -> usize {
        // Size discipline (design §6): prose, queued text, tool input and
        // results are all capped so one record cannot dominate a page.
        let block = match block {
            Block::Text { md } => Block::Text { md: cap_text(&md).0 },
            Block::Queued { op, text, injected } => Block::Queued { op, text: cap_text(&text).0, injected },
            Block::ToolCall { id, name, tool, title, input, mut result } => {
                if let Some(r) = result.as_mut() {
                    r.cap();
                }
                Block::ToolCall { id, name, tool, title, input: clip_input(input), result }
            }
            other => other,
        };
        let is_call = matches!(block, Block::ToolCall { .. });
        let ft = &mut self.turns[t];
        ft.turn.blocks.push(block);
        ft.last = ft.last.max(idx);
        if is_call {
            self.stats.tool_calls += 1;
        }
        ft.turn.blocks.len() - 1
    }

    /// Append a tool call and register it for result attachment.
    #[allow(clippy::too_many_arguments)]
    pub fn push_tool_call(
        &mut self,
        t: usize,
        id: String,
        name: String,
        tool: ToolKind,
        title: String,
        input: Value,
        result: Option<ToolResult>,
        idx: usize,
    ) -> BlockRef {
        let block = Block::ToolCall {
            id: id.clone(),
            name,
            tool,
            title,
            input,
            result,
        };
        let b = self.push_block(t, block, idx);
        let r = BlockRef { turn: t, block: b };
        if !id.is_empty() {
            self.tool_calls.insert(id, r);
        }
        r
    }

    /// Attach `result` to the tool call with `id`. Returns the call's location
    /// (so the caller can add sibling blocks) or `None` for an orphan.
    pub fn attach_result(&mut self, id: &str, result: ToolResult, idx: usize) -> Option<BlockRef> {
        let r = *self.tool_calls.get(id)?;
        if let Some(Block::ToolCall { result: slot, .. }) = self.turns[r.turn].turn.blocks.get_mut(r.block) {
            // A structured enrichment (Codex `patch_apply_end`) may land before
            // the plain output for the same call: keep what only it knew.
            let mut result = result;
            result.cap();
            if let Some(prev) = slot.take() {
                if result.patch.is_none() {
                    result.patch = prev.patch;
                }
                if result.file_path.is_none() {
                    result.file_path = prev.file_path;
                }
                if result.text.is_none() {
                    result.text = prev.text;
                    result.bytes = prev.bytes;
                    result.truncated = prev.truncated;
                }
            }
            *slot = Some(result);
        }
        self.touch(r.turn, idx);
        Some(r)
    }

    /// Mutable access to a tool call's current result (to enrich it later).
    pub fn result_mut(&mut self, r: BlockRef) -> Option<&mut ToolResult> {
        match self.turns.get_mut(r.turn)?.turn.blocks.get_mut(r.block)? {
            Block::ToolCall { result, .. } => result.as_mut(),
            _ => None,
        }
    }

    /// The tool call's `input`, if `r` points at one.
    pub fn call_input(&self, r: BlockRef) -> Option<&Value> {
        match self.turns.get(r.turn)?.turn.blocks.get(r.block)? {
            Block::ToolCall { input, .. } => Some(input),
            _ => None,
        }
    }

    /// Attach a system note to turn `t` (or buffer it when there is none yet).
    pub fn note(&mut self, t: Option<usize>, note: SystemNote, idx: usize) {
        match t {
            Some(t) if t < self.turns.len() => {
                self.turns[t].turn.system.push(note);
                self.touch(t, idx);
            }
            _ => self.pending_notes.push(note),
        }
    }

    /// Append a non-call block to turn `t`, or buffer it for the next turn.
    pub fn block_or_pending(&mut self, t: Option<usize>, block: Block, idx: usize) {
        match t {
            Some(t) if t < self.turns.len() => {
                self.push_block(t, block, idx);
            }
            _ => self.pending_blocks.push(block),
        }
    }

    /// An unrecognized record: a `notice{kind:other}` on the last turn plus a
    /// stat, so nothing is dropped silently (design §3 "lossless, quiet").
    pub fn unknown(&mut self, label: &str, idx: usize) {
        self.stats.unknown_records += 1;
        let note = SystemNote {
            kind: SystemNoteKind::Other,
            title: format!("Unknown record: {label}"),
            body: None,
        };
        let t = self.last_turn();
        self.block_or_pending(t, Block::Notice { note }, idx);
    }

    /// Add / bump a `thinking` marker on turn `t`.
    pub fn thinking(&mut self, t: usize, idx: usize) {
        self.stats.thinking_steps += 1;
        let ft = &mut self.turns[t];
        ft.last = ft.last.max(idx);
        for b in ft.turn.blocks.iter_mut().rev() {
            if let Block::Thinking { count } = b {
                *count += 1;
                return;
            }
        }
        ft.turn.blocks.push(Block::Thinking { count: 1 });
    }

    /// Extract one base64 image: returns the image id (written when a store is
    /// configured).
    pub fn image(&mut self, media_type: &str, data_b64: &str) -> String {
        match self.opts.images.as_ref() {
            Some(store) => store.put(media_type, data_b64),
            None => ImageStore::id_for(data_b64),
        }
    }

    /// Build a [`ToolResult`] from free text (+ optional image ids), capped.
    pub fn result_from_text(ok: bool, text: &str, image_ids: Vec<String>) -> ToolResult {
        let bytes = text.len() as u64;
        let (capped, truncated) = cap_text(text);
        ToolResult {
            ok,
            text: (!capped.is_empty()).then_some(capped),
            truncated,
            bytes,
            image_ids,
            patch: None,
            file_path: None,
        }
    }

    /// Register an artifact (dedup PER PATH; the last producing turn wins).
    pub fn artifact(&mut self, kind: ArtifactKind, path: Option<String>, url: Option<String>, turn_id: &str, produced_at: Option<String>) -> Option<Artifact> {
        let key = path.clone().or_else(|| url.clone())?;
        let id = Artifact::id_for(kind, &key);
        let label = match (&path, &url) {
            (Some(p), _) => basename(p).to_string(),
            (None, Some(u)) if kind == ArtifactKind::Pr => pr_label(u),
            (None, Some(u)) => u.clone(),
            _ => key.clone(),
        };
        let mime = path.as_deref().and_then(mime_for_path).map(str::to_string);
        let art = Artifact {
            id: id.clone(),
            kind,
            label,
            path,
            url,
            mime,
            produced_at,
            turn_id: turn_id.to_string(),
        };
        match self.artifact_index.get(&id) {
            Some(&i) => {
                self.artifacts[i].turn_id = art.turn_id.clone();
                if art.produced_at.is_some() {
                    self.artifacts[i].produced_at = art.produced_at.clone();
                }
            }
            None => {
                self.artifact_index.insert(id, self.artifacts.len());
                self.artifacts.push(art.clone());
            }
        }
        Some(art)
    }

    /// Accumulate one deduped usage sample.
    pub fn usage(&mut self, model: &str, input: u64, output: u64, cache_read: u64, cache_write: u64) {
        self.usage_seen = true;
        self.input_tokens = self.input_tokens.saturating_add(input);
        self.output_tokens = self.output_tokens.saturating_add(output);
        self.cache_read = self.cache_read.saturating_add(cache_read);
        self.cache_write = self.cache_write.saturating_add(cache_write);
        if let Some(price) = self.opts.price {
            self.cost_usd += price(model, input, output, cache_read, cache_write);
        }
    }

    /// Add a turn's duration to the transcript total (saturating).
    pub fn add_duration(&mut self, ms: u64) {
        self.stats.duration_ms = Some(self.stats.duration_ms.unwrap_or(0).saturating_add(ms));
    }

    /// A [`Folded`] view of the fold so far without consuming it (live tails
    /// snapshot after every delta). Clones the turns; still far cheaper than
    /// re-parsing the file.
    pub fn snapshot(&self, record_count: usize) -> Folded {
        self.clone().finish(record_count)
    }

    /// Finish: flush pending notes/blocks onto the last turn, attach subagent
    /// blocks, finalize stats.
    pub fn finish(mut self, record_count: usize) -> Folded {
        if let Some(t) = self.last_turn() {
            let notes = std::mem::take(&mut self.pending_notes);
            let blocks = std::mem::take(&mut self.pending_blocks);
            let last_idx = self.turns[t].last;
            for n in notes {
                self.note(Some(t), n, last_idx);
            }
            for b in blocks {
                self.push_block(t, b, last_idx);
            }
        } else if !self.pending_notes.is_empty() || !self.pending_blocks.is_empty() {
            // A file with system-only content (e.g. a fresh session that only
            // has sidecars): keep it visible as one system-only user turn.
            let t = self.new_turn("r0".into(), Role::User, self.first_ts.clone(), None, 0);
            let _ = t;
        }
        self.attach_subagents();
        self.stats.input_tokens = self
            .usage_seen
            .then_some(self.input_tokens.saturating_add(self.cache_read).saturating_add(self.cache_write));
        self.stats.output_tokens = self.usage_seen.then_some(self.output_tokens);
        self.stats.cost_usd = if self.usage_seen && self.opts.price.is_some() {
            Some(self.cost_usd)
        } else {
            self.cost_fallback
        };
        Folded {
            provider: self.provider,
            session_id: self.session_id,
            title: self.title,
            cwd: self.cwd,
            model: self.model,
            turns: self.turns,
            stats: self.stats,
            artifacts: self.artifacts,
            record_count,
            first_prompt: self.first_prompt,
            first_ts: self.first_ts,
            last_ts: self.last_ts,
        }
    }

    /// Insert a `subagent` block after every `Agent` tool call that a sidecar
    /// names via `toolUseId`; agents whose result carried an `agentId` but have
    /// no sidecar still get one from the call's input.
    fn attach_subagents(&mut self) {
        let mut placements: Vec<(BlockRef, Block)> = Vec::new();
        let mut covered: std::collections::HashSet<String> = std::collections::HashSet::new();
        let metas = self.opts.subagents.clone();
        for meta in &metas {
            let Some(tid) = meta.tool_use_id.as_deref() else { continue };
            let Some(&r) = self.tool_calls.get(tid) else { continue };
            let status = self.subagent_status(r);
            covered.insert(tid.to_string());
            placements.push((
                r,
                Block::Subagent {
                    agent_id: meta.agent_id.clone(),
                    description: meta.description.clone(),
                    agent_type: meta.agent_type.clone(),
                    status,
                },
            ));
        }
        // Agent calls whose result reported an `agentId` but have no sidecar.
        let orphans: Vec<(String, BlockRef)> = self
            .agent_ids
            .iter()
            .filter(|(tid, _)| !covered.contains(*tid))
            .filter_map(|(tid, agent_id)| self.tool_calls.get(tid).map(|r| (agent_id.clone(), *r)))
            .collect();
        for (agent_id, r) in orphans {
            let Some(input) = self.call_input(r).cloned() else { continue };
            let description = input.get("description").and_then(Value::as_str).unwrap_or("").to_string();
            let agent_type = input.get("subagent_type").and_then(Value::as_str).unwrap_or("agent").to_string();
            let status = self.subagent_status(r);
            placements.push((
                r,
                Block::Subagent {
                    agent_id,
                    description,
                    agent_type,
                    status,
                },
            ));
        }
        // Insert from the back so earlier block indices stay valid.
        placements.sort_by_key(|(r, _)| std::cmp::Reverse((r.turn, r.block)));
        for (r, block) in placements {
            if let Some(ft) = self.turns.get_mut(r.turn) {
                let at = (r.block + 1).min(ft.turn.blocks.len());
                ft.turn.blocks.insert(at, block);
            }
        }
    }

    fn subagent_status(&self, r: BlockRef) -> Option<SubagentStatus> {
        match self.turns.get(r.turn)?.turn.blocks.get(r.block)? {
            Block::ToolCall { result: None, .. } => Some(SubagentStatus::Running),
            Block::ToolCall { result: Some(res), .. } => Some(if res.ok {
                SubagentStatus::Done
            } else {
                SubagentStatus::Error
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::TOOL_TEXT_CAP;

    fn folded(n: usize) -> Folded {
        let mut f = Fold::new(Provider::Claude, FoldOpts::default());
        for i in 0..n {
            let t = f.new_turn(format!("t{i}"), Role::User, None, None, i * 2);
            f.push_block(t, Block::Text { md: format!("m{i}") }, i * 2 + 1);
        }
        f.finish(n * 2)
    }

    #[test]
    fn paging_walks_backwards_by_cursor() {
        let f = folded(5);
        let p = f.page(None, 2, vec![]);
        assert_eq!(p.turns.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t3", "t4"]);
        assert!(p.has_earlier);
        assert_eq!(p.cursor, "6"); // first record of t3
        let p2 = f.page(Some(6), 2, vec![]);
        assert_eq!(p2.turns.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
        assert!(p2.has_earlier);
        let p3 = f.page(Some(2), 2, vec![]);
        assert_eq!(p3.turns.len(), 1);
        assert_eq!(p3.turns[0].id, "t0");
        assert!(!p3.has_earlier);
        assert_eq!(p3.cursor, "0");
        // Delta since record 7 → only t3 (last=7) and t4.
        let d = f.turns_since(7);
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn page_stops_at_the_byte_budget_but_keeps_one_turn() {
        let mut f = Fold::new(Provider::Claude, FoldOpts::default());
        for i in 0..4 {
            let t = f.new_turn(format!("t{i}"), Role::User, None, None, i);
            f.push_block(t, Block::Text { md: "x".repeat(60 * 1024) }, i);
        }
        let out = f.finish(4);
        // 4 × 60 KB fits in 2 MB → all four.
        assert_eq!(out.page(None, 10, vec![]).turns.len(), 4);
        // A huge single turn is still returned alone.
        let mut g = Fold::new(Provider::Claude, FoldOpts::default());
        for i in 0..3 {
            let t = g.new_turn(format!("t{i}"), Role::User, None, None, i);
            for _ in 0..20 {
                g.push_block(t, Block::Text { md: "y".repeat(TOOL_TEXT_CAP) }, i);
            }
        }
        let out = g.finish(3);
        let p = out.page(None, 10, vec![]);
        assert_eq!(p.turns.len(), 1, "each turn is ~1.3 MB; only the newest fits");
        assert!(p.has_earlier);
        assert_eq!(p.turns[0].id, "t2");
    }

    #[test]
    fn artifacts_dedup_per_path_last_turn_wins() {
        let mut f = Fold::new(Provider::Claude, FoldOpts::default());
        let a = f.artifact(ArtifactKind::File, Some("/repo/a.md".into()), None, "t1", None).unwrap();
        let b = f.artifact(ArtifactKind::File, Some("/repo/a.md".into()), None, "t2", Some("2026".into())).unwrap();
        assert_eq!(a.id, b.id);
        let out = f.finish(0);
        assert_eq!(out.artifacts.len(), 1);
        assert_eq!(out.artifacts[0].turn_id, "t2");
        assert_eq!(out.artifacts[0].mime.as_deref(), Some("text/markdown"));
        assert_eq!(out.artifacts[0].label, "a.md");
    }

    #[test]
    fn pending_notes_land_on_the_next_turn_and_leftovers_on_the_last() {
        let mut f = Fold::new(Provider::Claude, FoldOpts::default());
        f.note(None, SystemNote { kind: SystemNoteKind::Other, title: "early".into(), body: None }, 0);
        let t = f.new_turn("u".into(), Role::User, None, None, 1);
        assert_eq!(f.turns[t].turn.system.len(), 1);
        f.note(Some(99), SystemNote { kind: SystemNoteKind::Other, title: "late".into(), body: None }, 2);
        let out = f.finish(3);
        assert_eq!(out.turns[0].turn.system.len(), 2);
    }
}
