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

pub use fold::{FoldOpts, Folded, FoldedTurn, PriceFn};
pub use images::ImageStore;
pub use model::*;
pub use peek::{peek, Peek};
pub use records::{parse_records, read_head_tail, read_records};
pub use subagents::{read_subagents, subagent_path, subagents_dir};
pub use tailer::{TailDelta, Tailer};
pub use util::{TOOL_INPUT_CAP, TOOL_TEXT_CAP};
pub use fold::PAGE_BYTES_BUDGET;

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

    pub fn snapshot(&self) -> Folded {
        match self {
            Folder::Claude(c) => c.snapshot(),
            Folder::Codex(c) => c.snapshot(),
        }
    }
}

/// Read + fold a transcript file in one go.
pub fn fold_file(provider: Provider, path: &std::path::Path, opts: FoldOpts<'_>) -> std::io::Result<Folded> {
    let records = read_records(path)?;
    Ok(fold(provider, &records, opts))
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
