//! Pure parsing core for the background usage tailer.
//!
//! The daemon's [`crate::UsageEngine`] records *real* token usage by tailing the
//! transcript files the agent CLIs write to disk. This module holds the
//! provider-specific line parsers and the on-disk byte-offset cursor store. It
//! is deliberately free of any I/O orchestration (that lives in the daemon, in
//! `ottod/src/usage_tailer.rs`) so the parsing logic can be unit-tested against
//! the exact JSON shapes the CLIs emit.
//!
//! On-disk formats (see `.git/sdd/usage-formats-research.md` for the full
//! research):
//!   * **Claude Code** — `~/.claude/projects/<enc_cwd>/<session-uuid>.jsonl`.
//!     Usage on `type=="assistant"` lines at `message.usage.*`, model at
//!     `message.model`. Per-turn (one line per API call). Append-only.
//!   * **Codex** — `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
//!     Usage on `type=="event_msg"` + `payload.type=="token_count"` lines, read
//!     from `payload.info.last_token_usage` (per-turn; `total_token_usage` is
//!     cumulative and would double-count). Model only in `session_meta` (first
//!     line) if present, else falls back to the literal `"codex"`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Token counts parsed from a single transcript line, normalized across
/// providers and ready to be turned into a [`crate::UsageEvent`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedUsage {
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// Fields lifted from a Codex `session_meta` line (the first line of a rollout
/// file). Both fields are best-effort: the rollout JSONL does not always carry a
/// model, and the cwd shape can vary, so callers must tolerate `None`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodexMeta {
    pub cwd: Option<String>,
    pub model: Option<String>,
}

/// Read a `u64` from a JSON object field, defaulting to 0 when absent or not a
/// number. Defensive on purpose: transcript schemas drift between CLI versions.
fn u64_field(obj: &Value, key: &str) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Parse a single Claude Code transcript line.
///
/// Returns `Some` only for `type=="assistant"` lines that carry a
/// `message.usage` object. Missing token fields default to 0; non-assistant
/// lines, lines without usage, and parse failures all yield `None`.
pub fn parse_claude_line(line: &str) -> Option<ParsedUsage> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    let message = v.get("message")?;
    let usage = message.get("usage")?;
    if !usage.is_object() {
        return None;
    }
    let model = message
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(ParsedUsage {
        model,
        input: u64_field(usage, "input_tokens"),
        output: u64_field(usage, "output_tokens"),
        cache_read: u64_field(usage, "cache_read_input_tokens"),
        cache_write: u64_field(usage, "cache_creation_input_tokens"),
    })
}

/// Parse a single Codex rollout transcript line.
///
/// Returns `Some` only for `type=="event_msg"` lines whose
/// `payload.type=="token_count"`. Reads the **per-turn** counts from
/// `payload.info.last_token_usage` (using `total_token_usage` would double-count
/// because it is cumulative). Reasoning output bills as output, so
/// `output = output_tokens + reasoning_output_tokens`. Codex has no
/// cache-creation concept, so `cache_write` is always 0. The model is supplied
/// by the caller (from `session_meta`, or the literal `"codex"`).
pub fn parse_codex_line(line: &str, fallback_model: &str) -> Option<ParsedUsage> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = v.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let last = payload.get("info")?.get("last_token_usage")?;
    if !last.is_object() {
        return None;
    }
    let output = u64_field(last, "output_tokens") + u64_field(last, "reasoning_output_tokens");
    Some(ParsedUsage {
        model: fallback_model.to_string(),
        input: u64_field(last, "input_tokens"),
        output,
        cache_read: u64_field(last, "cached_input_tokens"),
        cache_write: 0,
    })
}

/// Parse a Codex `session_meta` line, extracting the working directory and (if
/// present) the model. Returns `None` for any other line type or a parse
/// failure. The cwd and model live under the `payload` object on a
/// `session_meta` line; both are looked up defensively (a few top-level
/// fallbacks are also tried since the shape has drifted between CLI versions).
pub fn parse_codex_session_meta(line: &str) -> Option<CodexMeta> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(Value::as_str) != Some("session_meta") {
        return None;
    }
    // The interesting fields normally live under `payload`; fall back to the
    // top level for older shapes.
    let scope = v.get("payload").unwrap_or(&v);
    let cwd = scope
        .get("cwd")
        .or_else(|| v.get("cwd"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // Model can appear as `model`, or nested under `model.name` / similar.
    let model = scope
        .get("model")
        .or_else(|| v.get("model"))
        .and_then(|m| match m {
            Value::String(s) => Some(s.clone()),
            Value::Object(_) => m
                .get("name")
                .or_else(|| m.get("id"))
                .or_else(|| m.get("slug"))
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        });
    Some(CodexMeta { cwd, model })
}

/// A persistent map of `absolute-file-path → byte offset` that lets the tailer
/// resume exactly where it left off, so no transcript line is ever counted
/// twice — not even across daemon restarts (there is no idempotency column in
/// ClickHouse).
///
/// Backed by a JSON file written atomically (tmp file + rename).
#[derive(Debug, Default)]
pub struct CursorStore {
    path: PathBuf,
    offsets: HashMap<String, u64>,
}

impl CursorStore {
    /// Load the cursor map from `path`. A missing or unparseable file yields an
    /// empty store bound to that path (so the next [`Self::save`] creates it).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let offsets = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<HashMap<String, u64>>(&s).ok())
            .unwrap_or_default();
        Self { path, offsets }
    }

    /// The persisted offset for `file`, if any. The key is the file's path as a
    /// lossy UTF-8 string.
    pub fn get(&self, file: &Path) -> Option<u64> {
        self.offsets.get(&key(file)).copied()
    }

    /// True if a cursor has ever been recorded for `file` (distinct from an
    /// offset of 0, which is a real value for a freshly-tailed-from-start file).
    pub fn contains(&self, file: &Path) -> bool {
        self.offsets.contains_key(&key(file))
    }

    /// Record the new byte offset for `file` (in memory; call [`Self::save`] to
    /// persist).
    pub fn set(&mut self, file: &Path, offset: u64) {
        self.offsets.insert(key(file), offset);
    }

    /// Number of tracked files.
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Atomically persist the cursor map: write to a sibling tmp file, then
    /// rename over the target so a crash mid-write never corrupts the cursors.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(&self.offsets).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Cursor-map key for a path: lossy UTF-8 of the absolute path.
fn key(file: &Path) -> String {
    file.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Claude ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_claude_assistant_line_extracts_all_fields() {
        // Real shape from the research doc.
        let line = r#"{
            "type": "assistant",
            "uuid": "41192d16-aaaa",
            "timestamp": "2026-06-15T18:20:13.595Z",
            "sessionId": "sess-1",
            "message": {
                "model": "claude-opus-4-8",
                "stop_reason": "tool_use",
                "usage": {
                    "input_tokens": 14354,
                    "output_tokens": 332,
                    "cache_read_input_tokens": 15826,
                    "cache_creation_input_tokens": 9729,
                    "service_tier": "standard"
                }
            }
        }"#;
        let got = parse_claude_line(line).expect("assistant line parses");
        assert_eq!(
            got,
            ParsedUsage {
                model: "claude-opus-4-8".to_string(),
                input: 14354,
                output: 332,
                cache_read: 15826,
                cache_write: 9729,
            }
        );
    }

    #[test]
    fn parse_claude_missing_token_fields_default_to_zero() {
        // assistant + usage object present, but only partial counts.
        let line = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","usage":{"input_tokens":100}}}"#;
        let got = parse_claude_line(line).expect("parses");
        assert_eq!(
            got,
            ParsedUsage {
                model: "claude-sonnet-4".to_string(),
                input: 100,
                output: 0,
                cache_read: 0,
                cache_write: 0,
            }
        );
    }

    #[test]
    fn parse_claude_non_assistant_line_is_none() {
        let line = r#"{"type":"user","message":{"content":"hi"}}"#;
        assert_eq!(parse_claude_line(line), None);
    }

    #[test]
    fn parse_claude_assistant_without_usage_is_none() {
        let line = r#"{"type":"assistant","message":{"model":"claude-opus-4"}}"#;
        assert_eq!(parse_claude_line(line), None);
    }

    #[test]
    fn parse_claude_malformed_line_is_none() {
        assert_eq!(parse_claude_line("{not json"), None);
        assert_eq!(parse_claude_line(""), None);
        assert_eq!(parse_claude_line("   "), None);
    }

    // ── Codex ───────────────────────────────────────────────────────────────

    #[test]
    fn parse_codex_token_count_uses_last_not_total() {
        // Real shape: last_token_usage is per-turn; total_token_usage is
        // cumulative. We MUST use last_ to avoid double-counting.
        let line = r#"{
            "timestamp": "2026-06-18T05:53:42.634Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "total_token_usage": {
                        "input_tokens": 999999,
                        "cached_input_tokens": 888888,
                        "output_tokens": 777777,
                        "reasoning_output_tokens": 666666,
                        "total_tokens": 999999
                    },
                    "last_token_usage": {
                        "input_tokens": 46563,
                        "cached_input_tokens": 8576,
                        "output_tokens": 500,
                        "reasoning_output_tokens": 152,
                        "total_tokens": 47063
                    },
                    "model_context_window": 258400
                }
            }
        }"#;
        let got = parse_codex_line(line, "codex").expect("token_count parses");
        assert_eq!(
            got,
            ParsedUsage {
                model: "codex".to_string(),
                input: 46563,
                output: 500 + 152, // reasoning bills as output
                cache_read: 8576,
                cache_write: 0,
            }
        );
    }

    #[test]
    fn parse_codex_uses_fallback_model() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":2,"cached_input_tokens":0,"reasoning_output_tokens":0}}}}"#;
        let got = parse_codex_line(line, "gpt-5").expect("parses");
        assert_eq!(got.model, "gpt-5");
        assert_eq!(got.input, 10);
        assert_eq!(got.output, 2);
    }

    #[test]
    fn parse_codex_non_token_count_event_is_none() {
        let line = r#"{"type":"event_msg","payload":{"type":"agent_message","message":"hi"}}"#;
        assert_eq!(parse_codex_line(line, "codex"), None);
    }

    #[test]
    fn parse_codex_non_event_msg_is_none() {
        let line = r#"{"type":"response_item","payload":{"type":"message"}}"#;
        assert_eq!(parse_codex_line(line, "codex"), None);
        assert_eq!(parse_codex_line("{garbage", "codex"), None);
        assert_eq!(parse_codex_line("", "codex"), None);
    }

    #[test]
    fn parse_codex_session_meta_extracts_cwd_and_model() {
        let line = r#"{
            "type": "session_meta",
            "payload": {
                "id": "019ed94a",
                "cwd": "/Users/itziklavon/otto_os",
                "model": "gpt-5-codex",
                "cli_version": "1.2.3"
            }
        }"#;
        let got = parse_codex_session_meta(line).expect("session_meta parses");
        assert_eq!(got.cwd.as_deref(), Some("/Users/itziklavon/otto_os"));
        assert_eq!(got.model.as_deref(), Some("gpt-5-codex"));
    }

    #[test]
    fn parse_codex_session_meta_without_model_is_ok() {
        let line = r#"{"type":"session_meta","payload":{"cwd":"/tmp/x"}}"#;
        let got = parse_codex_session_meta(line).expect("parses");
        assert_eq!(got.cwd.as_deref(), Some("/tmp/x"));
        assert_eq!(got.model, None);
    }

    #[test]
    fn parse_codex_session_meta_nested_model_object() {
        let line = r#"{"type":"session_meta","payload":{"cwd":"/tmp/x","model":{"name":"Gemini-ish","id":"m1"}}}"#;
        let got = parse_codex_session_meta(line).expect("parses");
        assert_eq!(got.model.as_deref(), Some("Gemini-ish"));
    }

    #[test]
    fn parse_codex_session_meta_on_other_line_is_none() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count"}}"#;
        assert_eq!(parse_codex_session_meta(line), None);
    }

    // ── CursorStore ───────────────────────────────────────────────────────────

    #[test]
    fn cursor_store_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let cursor_path = dir.path().join("usage_tailer.json");
        let file_a = dir.path().join("a.jsonl");
        let file_b = dir.path().join("b.jsonl");

        let mut store = CursorStore::load(&cursor_path);
        assert!(store.is_empty());
        assert_eq!(store.get(&file_a), None);
        assert!(!store.contains(&file_a));

        store.set(&file_a, 128);
        store.set(&file_b, 0); // 0 is a real, persisted value
        store.save().unwrap();

        let reloaded = CursorStore::load(&cursor_path);
        assert_eq!(reloaded.get(&file_a), Some(128));
        assert_eq!(reloaded.get(&file_b), Some(0));
        assert!(reloaded.contains(&file_b)); // distinct from "never seen"
        assert_eq!(reloaded.len(), 2);
    }

    #[test]
    fn cursor_store_missing_file_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = CursorStore::load(dir.path().join("does-not-exist.json"));
        assert!(store.is_empty());
    }
}
