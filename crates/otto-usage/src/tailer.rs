//! Pure parsing core for the background usage tailer — now lives in
//! `otto-transcript` (`otto_transcript::usage`) so the conversation view and the
//! usage store share one parser. Everything is re-exported here under its old
//! path; the on-disk formats (`usage_tailer.json`, `usage_tailer_seen.json`,
//! `usage_tailer_codex_totals.json`) are frozen and tested in that crate.
//!
//! The tailer DRIVER (file watching, the ingestion journal, attribution and
//! delivery) is [`driver`]; the daemon only starts it.

pub mod driver;
pub use driver::{UsageTailer, UsageTailerHandle};

/// The one Codex model-id reader (`model` as a string or `{name|id|slug}`).
pub use otto_transcript::codex::model_name as codex_model_name;
pub use otto_transcript::usage::{
    merge_max, parse_claude_line, parse_codex_line, parse_codex_session_meta, seen_key_hash,
    ClaudeLine, CodexCounterStore, CodexMeta, CursorStore, HeldResponse, ParsedUsage,
    ResponseFolder, SeenKeys,
};
