//! otto-transcript — the ONE parser for the agent CLIs' on-disk transcripts.
//!
//! Two consumers share it so they can never disagree:
//!   * the usage store (`otto-usage`, `ottod/src/usage_tailer.rs`) — the
//!     per-line token parsers + cursor/seen stores in [`usage`], extracted from
//!     `otto-usage` verbatim (frozen on-disk formats);
//!   * the conversation view — [`fold`] turns a whole file into the normalized
//!     [`model::Transcript`] (turns, blocks, tool calls with results, system
//!     notes, tasks, artifacts, stats), paged by record index.
//!
//! Providers: Claude Code (`~/.claude/projects/<cwd-slug>/<sid>.jsonl` +
//! `<sid>/subagents/`) and Codex (`~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`,
//! two eras). agy keeps its history in SQLite and is `unsupported` here.
//!
//! Everything is tolerant: transcript JSON is accessed through
//! `serde_json::Value`, a record the parser does not know becomes a
//! `notice{kind:"other"}` block plus `stats.unknown_records` (never a panic, never
//! a silent drop), and the whole local corpus is replayed by the `#[ignore]`d
//! test in `tests/corpus.rs`.

pub mod claude;
pub mod codex;
pub mod fold;
pub mod images;
pub mod model;
pub mod peek;
pub mod records;
pub mod subagents;
pub mod tailer;
pub mod usage;
pub mod util;

pub use fold::PAGE_BYTES_BUDGET;
pub use fold::{FoldOpts, Folded, FoldedTurn, PriceFn};
pub use images::ImageStore;
pub use model::*;
pub use peek::{peek, Peek};
pub use records::{for_each_record, parse_records, read_head_tail, read_records};
pub use subagents::{
    read_subagents, read_subagents_limited, subagent_charge, subagent_path, subagents_dir,
    SubagentScanner,
};
pub use tailer::{TailDelta, Tailer};
pub use util::{TOOL_INPUT_CAP, TOOL_TEXT_CAP};

/// Fold parsed records for `provider`. agy yields an empty fold (the adapter is
/// a stub — the route reports `provider_unsupported` before getting here).
pub fn fold(provider: Provider, records: &[serde_json::Value], opts: FoldOpts<'_>) -> Folded {
    match provider {
        Provider::Claude => claude::fold_claude(records, opts),
        Provider::Codex => codex::fold_codex(records, opts),
        Provider::Agy => fold::Fold::new(Provider::Agy, opts).finish(0),
    }
}

/// Provider-neutral incremental folder for live tails (design §4.4): push the
/// records a [`Tailer`] delivers, `snapshot()` after each delta. `push`
/// returns `true` when the file must be refolded from record 0 (Codex per-file
/// decisions flipped); a `TailDelta::restarted` is the other refold signal.
#[derive(Clone)]
pub enum Folder<'a> {
    Claude(claude::ClaudeFolder<'a>),
    Codex(codex::CodexFolder<'a>),
}

impl<'a> Folder<'a> {
    pub fn new(provider: Provider, opts: FoldOpts<'a>) -> Self {
        match provider {
            Provider::Codex => Folder::Codex(codex::CodexFolder::new(opts)),
            // agy is a stub: fold as Claude-shaped (yields unknown records).
            Provider::Claude | Provider::Agy => Folder::Claude(claude::ClaudeFolder::new(opts)),
        }
    }

    /// [`seed`](Self::seed) from raw file bytes. Claude streams them one
    /// record at a time (no whole-file `Vec<Value>`); Codex needs the
    /// per-file prescan first, so it parses them all.
    pub fn seed_bytes(&mut self, bytes: &[u8]) {
        match self {
            Folder::Claude(c) => {
                for_each_record(bytes, |v| c.push(&v));
            }
            Folder::Codex(_) => self.seed(&parse_records(bytes)),
        }
    }

    /// Whole-file start: prescan (Codex) then push every record.
    pub fn seed(&mut self, records: &[serde_json::Value]) {
        if let Folder::Codex(c) = self {
            for r in records {
                c.prescan(r);
            }
        }
        for r in records {
            self.push(r);
        }
    }

    pub fn push(&mut self, v: &serde_json::Value) -> bool {
        match self {
            Folder::Claude(c) => {
                c.push(v);
                false
            }
            Folder::Codex(c) => c.push(v),
        }
    }

    pub fn record_count(&self) -> usize {
        match self {
            Folder::Claude(c) => c.record_count(),
            Folder::Codex(c) => c.record_count(),
        }
    }

    pub fn set_subagents(&mut self, subagents: Vec<SubagentMeta>) {
        match self {
            Folder::Claude(c) => c.set_subagents(subagents),
            Folder::Codex(c) => c.set_subagents(subagents),
        }
    }

    /// Finalized newest turns without cloning older turns, provider indexes or
    /// the artifact registry. Cumulative stats/metadata are preserved. As for
    /// `page`, the newest eligible turn is kept even if it exceeds byte_limit.
    /// `artifacts` is empty: use `artifacts()` when that registry is needed.
    pub fn bounded_snapshot(&self, limit: usize, byte_limit: usize) -> Folded {
        match self {
            Folder::Claude(c) => c.bounded_snapshot(limit, byte_limit),
            Folder::Codex(c) => c.bounded_snapshot(limit, byte_limit),
        }
    }

    /// Equivalent to snapshot().page(), copying only the requested window.
    pub fn page(
        &self,
        before: Option<usize>,
        limit: usize,
        subagents: Vec<SubagentMeta>,
    ) -> Transcript {
        match self {
            Folder::Claude(c) => c.page(before, limit, subagents),
            Folder::Codex(c) => c.page(before, limit, subagents),
        }
    }

    pub fn snapshot(&self) -> Folded {
        match self {
            Folder::Claude(c) => c.snapshot(),
            Folder::Codex(c) => c.snapshot(),
        }
    }

    /// The live-tail delta: exactly `snapshot().turns_since(since)`, but only
    /// the touched turns are cloned (see `Fold::turns_since`).
    pub fn turns_since(&self, since: usize) -> Vec<Turn> {
        match self {
            Folder::Claude(c) => c.turns_since(since),
            Folder::Codex(c) => c.turns_since(since),
        }
    }

    /// Copy one tool block without snapshotting unrelated history.
    pub fn tool_block(&self, tool_id: &str) -> Option<Block> {
        match self {
            Folder::Claude(c) => c.tool_block(tool_id),
            Folder::Codex(c) => c.tool_block(tool_id),
        }
    }

    /// Artifacts registered so far, in first-seen order.
    pub fn artifacts(&self) -> &[Artifact] {
        match self {
            Folder::Claude(c) => c.artifacts(),
            Folder::Codex(c) => c.artifacts(),
        }
    }
}

/// Fold a transcript from its raw bytes — [`fold`] over
/// [`parse_records`], but Claude files are streamed (see
/// [`Folder::seed_bytes`]).
pub fn fold_bytes(provider: Provider, bytes: &[u8], opts: FoldOpts<'_>) -> Folded {
    match provider {
        Provider::Claude => {
            let mut c = claude::ClaudeFolder::new(opts);
            for_each_record(bytes, |v| c.push(&v));
            c.into_folded()
        }
        Provider::Codex | Provider::Agy => fold(provider, &parse_records(bytes), opts),
    }
}

/// Read + fold a transcript file in one go.
pub fn fold_file(
    provider: Provider,
    path: &std::path::Path,
    opts: FoldOpts<'_>,
) -> std::io::Result<Folded> {
    let bytes = std::fs::read(path)?;
    Ok(fold_bytes(provider, &bytes, opts))
}

/// Explicit interactive-read limits. The charge is conservative accounting,
/// not an RSS measurement; parsing holds at most one bounded record at a time.
#[derive(Clone, Copy)]
pub struct FoldReadLimits {
    pub input_bytes: usize,
    pub record_bytes: usize,
    pub charge_bytes: usize,
}
impl Default for FoldReadLimits {
    fn default() -> Self {
        Self {
            input_bytes: 128 * 1024 * 1024,
            record_bytes: 16 * 1024 * 1024,
            charge_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Stream a fixed file prefix; Codex first scans that SAME prefix for its
/// era flags. Refuse excessive input before allocation and excessive retained
/// charge before parsing/pushing another record. Never silently truncate.
pub fn fold_file_limited(
    provider: Provider,
    path: &std::path::Path,
    opts: FoldOpts<'_>,
    limits: FoldReadLimits,
) -> std::io::Result<Folded> {
    use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
    fn exceeded() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::OutOfMemory, "transcript resource limit exceeded; open the provider transcript directly or select a smaller subagent transcript")
    }
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > limits.input_bytes as u64 {
        return Err(exceeded());
    }
    let mut folder = Folder::new(provider, opts);
    let passes = if provider == Provider::Codex { 2 } else { 1 };
    for pass in 0..passes {
        file.seek(SeekFrom::Start(0))?;
        let mut reader = BufReader::new((&mut file).take(len));
        let mut line = Vec::new();
        let mut charge = 0usize;
        loop {
            line.clear();
            let size = (&mut reader)
                .take(limits.record_bytes.saturating_add(1) as u64)
                .read_until(b'\n', &mut line)?;
            if size > limits.record_bytes {
                return Err(exceeded());
            }
            if size == 0 || !line.ends_with(b"\n") {
                break;
            }
            let text = String::from_utf8_lossy(&line);
            if text.trim().is_empty() {
                continue;
            }
            charge = charge
                .saturating_add(size.saturating_mul(4))
                .saturating_add(2048);
            if charge > limits.charge_bytes {
                return Err(exceeded());
            }
            let record = records::parse_line(&text);
            if pass == 0 && passes == 2 {
                if let Folder::Codex(c) = &mut folder {
                    c.prescan(&record);
                }
            } else {
                folder.push(&record);
            }
        }
    }
    Ok(match folder {
        Folder::Claude(c) => c.into_folded(),
        Folder::Codex(c) => c.into_folded(),
    })
}

/// Guess the provider from a transcript path: Codex rollouts are named
/// `rollout-…`, everything else under a `projects/<slug>/` dir is Claude.
pub fn provider_for_path(path: &std::path::Path) -> Option<Provider> {
    let name = path.file_name()?.to_str()?;
    if !name.ends_with(".jsonl") {
        return None;
    }
    if name.starts_with("rollout-") {
        return Some(Provider::Codex);
    }
    Some(Provider::Claude)
}
