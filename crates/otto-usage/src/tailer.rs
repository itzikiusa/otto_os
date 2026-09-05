//! Pure parsing core for the background usage tailer — now lives in
//! `otto-transcript` (`otto_transcript::usage`) so the conversation view and the
//! usage store share one parser. Everything is re-exported here under its old
//! path; the on-disk formats (`usage_tailer.json`, `usage_tailer_seen.json`,
//! `usage_tailer_codex_totals.json`) are frozen and tested in that crate.

pub use otto_transcript::usage::{
    parse_claude_line, parse_codex_line, parse_codex_session_meta, ClaudeLine, CodexCounterStore,
    CodexMeta, CursorStore, ParsedUsage, SeenKeys,
};
