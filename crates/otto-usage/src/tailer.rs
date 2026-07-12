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
//!     from cumulative `payload.info.total_token_usage` and differenced by a
//!     persisted per-session counter store. Codex's `input_tokens` is
//!     cache-inclusive, so we subtract `cached_input_tokens` to keep `input`
//!     disjoint from `cache_read`. Model/session id come from `session_meta`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
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
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
}

/// Read a `u64` from a JSON object field, defaulting to 0 when absent or not a
/// number. Defensive on purpose: transcript schemas drift between CLI versions.
fn u64_field(obj: &Value, key: &str) -> u64 {
    obj.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// A parsed Claude Code assistant line: normalized usage plus the identity
/// needed to count each API response exactly once.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaudeLine {
    pub usage: ParsedUsage,
    /// `message.id` (+ `requestId` when present). One API response is written
    /// as *multiple* transcript lines — one per content block, each repeating
    /// the same `message.usage` — and resumed sessions replay prior lines into
    /// a new file. Billing happens once per response, so usage must be counted
    /// once per key. `None` when the line carries no `message.id` (count it —
    /// there is nothing to collide with).
    pub dedup_key: Option<String>,
    /// The line's own `timestamp` (RFC3339), the true time of the API call.
    pub timestamp: Option<String>,
}

/// Parse a single Claude Code transcript line.
///
/// Returns `Some` only for `type=="assistant"` lines that carry a
/// `message.usage` object. Missing token fields default to 0; non-assistant
/// lines, lines without usage, and parse failures all yield `None`.
pub fn parse_claude_line(line: &str) -> Option<ClaudeLine> {
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
    let dedup_key = message.get("id").and_then(Value::as_str).map(|mid| {
        let rid = v.get("requestId").and_then(Value::as_str).unwrap_or_default();
        format!("{mid}:{rid}")
    });
    let timestamp = v
        .get("timestamp")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(ClaudeLine {
        usage: ParsedUsage {
            model,
            input: u64_field(usage, "input_tokens"),
            output: u64_field(usage, "output_tokens"),
            cache_read: u64_field(usage, "cache_read_input_tokens"),
            cache_write: u64_field(usage, "cache_creation_input_tokens"),
        },
        dedup_key,
        timestamp,
    })
}

/// Parse a single Codex rollout transcript line.
///
/// Returns `Some` only for `type=="event_msg"` lines whose
/// `payload.type=="token_count"`. Reads the cumulative counts from
/// `payload.info.total_token_usage`; callers must pass snapshots through
/// [`CodexCounterStore`] before recording them. `reasoning_output_tokens` is a
/// subset of `output_tokens`, so it is deliberately not added again. Codex has
/// no cache-creation concept, so `cache_write` is always 0.
///
/// Codex's `input_tokens` is **cache-inclusive** — it counts uncached *and*
/// cached prompt tokens (`total_tokens == input_tokens + output_tokens`,
/// `cached_input_tokens ⊆ input_tokens`). Claude's `input_tokens` instead
/// *excludes* cache, so its buckets are disjoint. We normalize codex to Claude's
/// convention by subtracting the cached portion, so `input` means uncached
/// prompt tokens for both providers. Without this, the shared
/// `input + output + cache_read + cache_write` total (and the per-token cost,
/// which prices `input` at the base rate and `cache_read` at ~0.1×) counts the
/// cached tokens twice — nearly doubling codex, which is overwhelmingly cache.
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
    let total = payload.get("info")?.get("total_token_usage")?;
    if !total.is_object() {
        return None;
    }
    let output = u64_field(total, "output_tokens");
    let cache_read = u64_field(total, "cached_input_tokens");
    // `saturating_sub` guards against schema drift where cached briefly exceeds
    // the reported input; codex's input is cache-inclusive (see doc above).
    let input = u64_field(total, "input_tokens").saturating_sub(cache_read);
    Some(ParsedUsage {
        model: fallback_model.to_string(),
        input,
        output,
        cache_read,
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
    let session_id = scope
        .get("id")
        .or_else(|| v.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string);
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
    Some(CodexMeta { session_id, cwd, model })
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

/// One session's last observed cumulative Codex counters, already normalized
/// into disjoint Otto buckets (`input` excludes `cache_read`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
struct CodexCounters {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

impl From<&ParsedUsage> for CodexCounters {
    fn from(value: &ParsedUsage) -> Self {
        Self {
            input: value.input,
            output: value.output,
            cache_read: value.cache_read,
            cache_write: value.cache_write,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CodexCounterState {
    counters: HashMap<String, CodexCounters>,
    order: std::collections::VecDeque<String>,
}

/// Bounded, persisted cumulative counters keyed by Codex session id.
///
/// Rollout files can repeat the same token snapshot and resumes can continue a
/// session in another file. File offsets cannot deduplicate either case, so
/// this store emits only the positive delta from the last session-wide total.
/// Any regressing snapshot is treated as an older/out-of-order replay and does
/// not move the baseline; false negatives are safer than counting a resume
/// replay as a fresh multi-million-token segment.
#[derive(Debug)]
pub struct CodexCounterStore {
    path: PathBuf,
    cap: usize,
    state: CodexCounterState,
}

impl CodexCounterStore {
    pub fn load(path: impl Into<PathBuf>, cap: usize) -> Self {
        let path = path.into();
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<CodexCounterState>(&s).ok())
            .unwrap_or_default();
        let mut store = Self { path, cap: cap.max(1), state };
        store.state.order.retain(|key| store.state.counters.contains_key(key));
        store.evict();
        store
    }

    /// Establish or replace a baseline without emitting usage. Used when the
    /// byte cursor skips pre-existing transcript history at startup.
    pub fn seed(&mut self, session_id: &str, usage: &ParsedUsage) {
        let current = CodexCounters::from(usage);
        if self.state.counters.get(session_id).is_some_and(|previous| {
            current.input < previous.input
                || current.output < previous.output
                || current.cache_read < previous.cache_read
                || current.cache_write < previous.cache_write
        }) {
            return;
        }
        self.state.counters.insert(session_id.to_string(), current);
        self.touch(session_id);
        self.evict();
    }

    /// Apply a cumulative snapshot and return only its newly accrued usage.
    pub fn apply(&mut self, session_id: &str, usage: &ParsedUsage) -> Option<ParsedUsage> {
        let current = CodexCounters::from(usage);
        let previous = match self.state.counters.get(session_id).copied() {
            Some(previous) => previous,
            None => {
                self.seed(session_id, usage);
                return (current != CodexCounters::default()).then(|| usage.clone());
            }
        };

        if current.input < previous.input
            || current.output < previous.output
            || current.cache_read < previous.cache_read
            || current.cache_write < previous.cache_write
        {
            return None;
        }

        let delta = ParsedUsage {
            model: usage.model.clone(),
            input: current.input - previous.input,
            output: current.output - previous.output,
            cache_read: current.cache_read - previous.cache_read,
            cache_write: current.cache_write - previous.cache_write,
        };
        if delta.input == 0 && delta.output == 0 && delta.cache_read == 0 && delta.cache_write == 0 {
            return None;
        }
        self.state.counters.insert(session_id.to_string(), current);
        self.touch(session_id);
        self.evict();
        Some(delta)
    }

    fn touch(&mut self, session_id: &str) {
        self.state.order.retain(|key| key != session_id);
        self.state.order.push_back(session_id.to_string());
    }

    fn evict(&mut self) {
        while self.state.order.len() > self.cap {
            if let Some(old) = self.state.order.pop_front() {
                self.state.counters.remove(&old);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.state.counters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.state.counters.is_empty()
    }

    pub fn contains(&self, session_id: &str) -> bool {
        self.state.counters.contains_key(session_id)
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(&self.state).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// A bounded, persisted set of already-counted Claude dedup keys
/// (see [`ClaudeLine::dedup_key`]).
///
/// The byte-offset cursor guarantees no *line* is read twice, but one API
/// response spans several lines and resumed sessions replay old lines into new
/// files — so a *response* can still arrive more than once. This set is the
/// response-level guard. It is persisted next to the cursor file (same atomic
/// tmp+rename pattern, insertion order preserved as a JSON array) so restarts
/// don't re-count, and FIFO-capped so it can't grow without bound: at real
/// transcript rates the cap covers months of history, far longer than any
/// resume replays reach back.
#[derive(Debug)]
pub struct SeenKeys {
    path: PathBuf,
    cap: usize,
    set: std::collections::HashSet<String>,
    order: std::collections::VecDeque<String>,
}

impl SeenKeys {
    /// Load from `path`, keeping at most `cap` keys (oldest evicted first). A
    /// missing or unparseable file yields an empty store bound to that path.
    pub fn load(path: impl Into<PathBuf>, cap: usize) -> Self {
        let path = path.into();
        let order: std::collections::VecDeque<String> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
            .unwrap_or_default()
            .into();
        let mut s = Self {
            path,
            cap: cap.max(1),
            set: order.iter().cloned().collect(),
            order,
        };
        s.evict();
        s
    }

    pub fn contains(&self, key: &str) -> bool {
        self.set.contains(key)
    }

    /// Insert `key`; returns `true` if it was new (i.e. this occurrence should
    /// be counted) and `false` if it was already present (a duplicate).
    pub fn insert(&mut self, key: &str) -> bool {
        if !self.set.insert(key.to_string()) {
            return false;
        }
        self.order.push_back(key.to_string());
        self.evict();
        true
    }

    fn evict(&mut self) {
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Atomically persist (tmp file + rename), like [`CursorStore::save`].
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let keys: Vec<&String> = self.order.iter().collect();
        let json = serde_json::to_string(&keys).map_err(std::io::Error::other)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
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
            "requestId": "req_011Ccd",
            "message": {
                "id": "msg_01Xezx",
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
            ClaudeLine {
                usage: ParsedUsage {
                    model: "claude-opus-4-8".to_string(),
                    input: 14354,
                    output: 332,
                    cache_read: 15826,
                    cache_write: 9729,
                },
                dedup_key: Some("msg_01Xezx:req_011Ccd".to_string()),
                timestamp: Some("2026-06-15T18:20:13.595Z".to_string()),
            }
        );
    }

    #[test]
    fn parse_claude_missing_token_fields_default_to_zero() {
        // assistant + usage object present, but only partial counts.
        let line = r#"{"type":"assistant","message":{"model":"claude-sonnet-4","usage":{"input_tokens":100}}}"#;
        let got = parse_claude_line(line).expect("parses");
        assert_eq!(
            got.usage,
            ParsedUsage {
                model: "claude-sonnet-4".to_string(),
                input: 100,
                output: 0,
                cache_read: 0,
                cache_write: 0,
            }
        );
        // No message.id → nothing to dedup on; no timestamp on the line.
        assert_eq!(got.dedup_key, None);
        assert_eq!(got.timestamp, None);
    }

    #[test]
    fn parse_claude_dedup_key_without_request_id_uses_message_id() {
        // message ids are unique per API response, so the id alone still
        // identifies the response when requestId is absent.
        let line = r#"{"type":"assistant","message":{"id":"msg_9","usage":{"input_tokens":1}}}"#;
        let got = parse_claude_line(line).expect("parses");
        assert_eq!(got.dedup_key, Some("msg_9:".to_string()));
    }

    #[test]
    fn parse_claude_multi_block_lines_share_one_dedup_key() {
        // One API response with N content blocks = N transcript lines, all
        // repeating the same message.id + requestId + usage. The key must be
        // identical across them so exactly one is counted.
        let mk = |uuid: &str| {
            format!(
                r#"{{"type":"assistant","uuid":"{uuid}","requestId":"req_1","message":{{"id":"msg_1","usage":{{"input_tokens":5,"output_tokens":7}}}}}}"#
            )
        };
        let a = parse_claude_line(&mk("aaaa")).unwrap();
        let b = parse_claude_line(&mk("bbbb")).unwrap();
        assert_eq!(a.dedup_key, b.dedup_key);
        assert!(a.dedup_key.is_some());
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
    fn parse_codex_token_count_uses_cumulative_total_without_reasoning_twice() {
        // Real shape: token_count snapshots can repeat, so the tailer must
        // difference the cumulative total rather than sum `last_token_usage`.
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
                input: 999999 - 888888,
                // `reasoning_output_tokens` is already a subset of output.
                output: 777777,
                cache_read: 888888,
                cache_write: 0,
            }
        );
        assert_eq!(got.input + got.cache_read, 999999);
    }

    #[test]
    fn parse_codex_uses_fallback_model() {
        let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":10,"output_tokens":2,"cached_input_tokens":0,"reasoning_output_tokens":1}}}}"#;
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
        assert_eq!(got.session_id.as_deref(), Some("019ed94a"));
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

    // ── CodexCounterStore ───────────────────────────────────────────────────

    fn codex_total(input: u64, output: u64, cache_read: u64) -> ParsedUsage {
        ParsedUsage {
            model: "gpt-5-codex".to_string(),
            input,
            output,
            cache_read,
            cache_write: 0,
        }
    }

    #[test]
    fn codex_counter_store_emits_only_monotonic_deltas() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CodexCounterStore::load(dir.path().join("codex.json"), 100);

        assert_eq!(store.apply("session-1", &codex_total(10, 2, 80)), Some(codex_total(10, 2, 80)));
        assert_eq!(store.apply("session-1", &codex_total(14, 5, 110)), Some(codex_total(4, 3, 30)));
        assert_eq!(store.apply("session-1", &codex_total(14, 5, 110)), None);
        // A replayed older snapshot must not reset the baseline or count again.
        assert_eq!(store.apply("session-1", &codex_total(12, 4, 90)), None);
        assert_eq!(store.apply("session-1", &codex_total(15, 6, 120)), Some(codex_total(1, 1, 10)));
    }

    #[test]
    fn codex_counter_store_persists_and_caps_recent_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("codex.json");
        let mut store = CodexCounterStore::load(&path, 2);
        store.seed("a", &codex_total(1, 1, 1));
        store.seed("b", &codex_total(2, 2, 2));
        store.seed("c", &codex_total(3, 3, 3));
        store.save().unwrap();

        let mut reloaded = CodexCounterStore::load(&path, 2);
        assert_eq!(reloaded.len(), 2);
        assert_eq!(reloaded.apply("c", &codex_total(4, 4, 4)), Some(codex_total(1, 1, 1)));
        // `a` was evicted, so seeing it again establishes a fresh baseline.
        assert_eq!(reloaded.apply("a", &codex_total(1, 1, 1)), Some(codex_total(1, 1, 1)));
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

    // ── SeenKeys ─────────────────────────────────────────────────────────────

    #[test]
    fn seen_keys_first_insert_counts_duplicate_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut seen = SeenKeys::load(dir.path().join("seen.json"), 100);
        assert!(seen.insert("msg_1:req_1")); // first sight → count
        assert!(!seen.insert("msg_1:req_1")); // duplicate → skip
        assert!(seen.insert("msg_2:req_2")); // different response → count
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn seen_keys_roundtrips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seen.json");
        let mut seen = SeenKeys::load(&path, 100);
        seen.insert("a");
        seen.insert("b");
        seen.save().unwrap();

        let reloaded = SeenKeys::load(&path, 100);
        assert!(reloaded.contains("a"));
        assert!(reloaded.contains("b"));
        assert!(!reloaded.contains("c"));
        assert_eq!(reloaded.len(), 2);
    }

    #[test]
    fn seen_keys_evicts_oldest_beyond_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut seen = SeenKeys::load(dir.path().join("seen.json"), 3);
        for k in ["k1", "k2", "k3", "k4"] {
            assert!(seen.insert(k));
        }
        assert_eq!(seen.len(), 3);
        assert!(!seen.contains("k1")); // oldest evicted
        assert!(seen.contains("k4"));
        // Eviction also applies on load (cap can shrink between versions).
        seen.save().unwrap();
        let reloaded = SeenKeys::load(dir.path().join("seen.json"), 2);
        assert_eq!(reloaded.len(), 2);
        assert!(!reloaded.contains("k2"));
        assert!(reloaded.contains("k3") && reloaded.contains("k4"));
    }

    #[test]
    fn seen_keys_missing_file_is_empty_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let seen = SeenKeys::load(dir.path().join("does-not-exist.json"), 10);
        assert!(seen.is_empty());
    }
}
