//! Grok CLI usage, isolated from the frozen Claude/Codex tailer parsers.
//!
//! Call `otto_transcript::grok_usage`. This module is not re-exported from
//! [`crate::usage`] or from `otto-usage`, and it does not add a `Provider` variant.
//! The usage-event provider is the string [`PROVIDER`] (`"grok"`).
//!
//! Source of truth, matching ccusage 20.0.36 `adapters/grok` (`ccusage/ccusage`):
//! each `updates.jsonl` line whose `params.update.sessionUpdate` is
//! `turn_completed`. The `method` field is not required. `usage.json` is a
//! rewritten snapshot and is never read: re-summing it double-counts, and on
//! this corpus it disagrees with `updates.jsonl` for some sessions. `signals.json`,
//! `events.jsonl`, `chat_history.jsonl`, and `logs/unified.jsonl` are not billing
//! ledgers. A session with none of these turn rows has unknown usage.
//!
//! The live loop streams new bytes with the tailer's `read_new_lines` and parses
//! each line with [`parse_turn_completed_line`] plus a [`GrokFallback`] from
//! [`fallback_for_updates`]. It does not call the whole-file readers
//! ([`read_updates_history`], [`read_session_history`], [`read_grok_home`]) on
//! that path. Those helpers dedupe inside one file for a single pass only.
//!
//! Dedupe is file-specific, matching ccusage: the same [`GrokDelta::dedupe_key`]
//! in two `updates.jsonl` files is two bills. A historical scan keeps a durable
//! native-key map namespaced by file plus `dedupe_key`. The FIFO `SeenKeys` set
//! evicts old keys and is the wrong store for that scan.
//!
//! Token shape is OpenAI-style. `inputTokens` already includes cache.
//! `reasoningTokens` is already inside `outputTokens`. Cache is not added on top
//! of input, and reasoning is not added on top of output. Reported buckets are
//! disjoint and line up with Claude's `ParsedUsage`:
//!
//! * `input` — uncached remainder
//! * `cache_read` — `min(cachedReadTokens, inputTokens)`
//! * `cache_write` — `min(cacheCreationTokens, uncached remainder)`
//! * `output` — `outputTokens`
//! * `reasoning` — informational subset of output; [`GrokDelta::parsed_usage`] omits it
//!
//! A row whose five buckets (uncached, cache read, cache write, output, reasoning)
//! are all zero is dropped. A reasoning-only row is kept and still contributes
//! no output tokens. When `modelUsage` is non-empty, only those rows are emitted
//! (the top-level usage would double-count). Models are emitted in name order.
//! The displayed model is the raw `modelUsage` key. With no `modelUsage`, the
//! row uses `summary.json` `current_model_id`, else `"unknown"`.
//!
//! Source cost is `costUsdTicks / 1e10` from the same model row the tokens came
//! from. [`GrokDelta::cost_usd`] returns `Some` only when ticks are positive.
//! `None` means the row recorded no bill. This module does not invent a rate
//! and does not return `0.0` for that case. The journal then uses the existing
//! `estimate_cost` path and flags the figure (`is_priced` is false for these
//! model ids, so the UI's `fallback_priced` tag applies). A recorded `Some` is
//! the source cost and stays off that estimate.
//!
//! Timestamp: `params._meta.agentTimestampMs` when it is a positive number,
//! otherwise the envelope `timestamp` (Unix seconds) times 1000. A missing clock
//! leaves `timestamp_ms` empty; the dedupe fallback still uses `0`. A root-level
//! `_meta` is used only when `params._meta` is absent (ccusage reads `params`).
//! `usage.apiDurationMs` is copied onto every model row of that turn; sum tokens
//! across those rows and take the duration once per `event_id`. `elapsed_ms` is
//! wall time and is ignored. `usageIsIncomplete` rows are still counted.
//!
//! Session id, in order: `params.sessionId`, else `summary.json` `info.id`, else
//! the session directory name. That string is the only attribution id. The
//! journal writes it to `session_id` and does not write `project_path`,
//! `info.cwd`, `git_root_dir`, or the percent-decoded parent directory into
//! `session_id` or `workspace_id`. `project_path` is still filled
//! (`info.cwd`, else `git_root_dir`, else the decoded parent) for callers that
//! want the ccusage project label. Each session directory is its own bill,
//! including a subagent that has its own `updates.jsonl`. The in-file key is
//! `{eventId}|{model}` when `eventId` is non-empty, otherwise
//! `{sessionId}|{timestamp_ms}|{model}|{uncached}|{output}|{cache_read}|{cache_write}|{reasoning}`.
//!
//! Ingestion sketch (Root owns re-exports, the tailer, and the durable map):
//! stream the file, parse with [`parse_turn_completed_line`], provider
//! [`PROVIDER`], kind `"completion"`, `session_id` from the delta only, tokens
//! from [`GrokDelta::parsed_usage`], `ts` from [`GrokDelta::timestamp_rfc3339`].
//! Store [`GrokDelta::cost_usd`] when it is `Some`. When it is `None`, store
//! `estimate_cost` and leave the existing unpriced-model flag set. Do not
//! classify this source with `infer_provider`. Antigravity SQLite and
//! `~/.claude-work` are other adapters.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::usage::ParsedUsage;
use crate::util::format_rfc3339_millis;

/// Usage-event provider string. Not a `Provider` enum variant.
pub const PROVIDER: &str = "grok";

/// `costUsdTicks` fixed point: one tick is 1e-10 USD.
pub const COST_USD_TICKS_PER_USD: u64 = 10_000_000_000;

/// Path fallbacks used when a `turn_completed` line omits session, project, or model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrokFallback {
    pub session_id: String,
    pub project_path: String,
    pub default_model: Option<String>,
}

/// One billable model row from a `turn_completed` update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrokDelta {
    pub session_id: String,
    pub project_path: String,
    /// `params._meta.eventId` when it is a non-empty string.
    pub event_id: Option<String>,
    /// Stable inside one `updates.jsonl`. The same key in another file is a
    /// second bill. Historical scans namespace a durable map by file plus this
    /// key; the FIFO `SeenKeys` set is not that map.
    pub dedupe_key: String,
    /// Raw `modelUsage` key, or the summary/unknown fallback.
    pub model: String,
    /// Uncached input. Disjoint from the cache buckets.
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    /// Cache creation (`cacheCreationTokens`), clamped into the uncached remainder.
    pub cache_write: u64,
    /// Subset of `output` on real ledgers. Do not add it to `output` or to cost.
    pub reasoning: u64,
    pub cost_ticks: u64,
    /// Agent millis, else envelope seconds × 1000. `None` when neither is positive.
    pub timestamp_ms: Option<i64>,
    /// `usage.apiDurationMs` for the whole turn, shared by each model row.
    pub duration_ms: Option<u64>,
}

impl GrokDelta {
    /// Recorded USD for this model row. `None` when ticks are absent: unknown,
    /// and the journal's flagged `estimate_cost` applies. Never a stand-in `$0`.
    pub fn cost_usd(&self) -> Option<f64> {
        (self.cost_ticks > 0).then(|| self.cost_ticks as f64 / COST_USD_TICKS_PER_USD as f64)
    }

    /// UTC RFC3339 with millisecond precision. `None` when [`Self::timestamp_ms`] is.
    pub fn timestamp_rfc3339(&self) -> Option<String> {
        self.timestamp_ms.map(format_rfc3339_millis)
    }

    /// Disjoint buckets for an `otto_usage::UsageEvent`. Reasoning is omitted.
    pub fn parsed_usage(&self) -> ParsedUsage {
        ParsedUsage {
            model: self.model.clone(),
            input: self.input,
            output: self.output,
            cache_read: self.cache_read,
            cache_write: self.cache_write,
        }
    }
}

/// Parse one JSONL line. Empty when the line is not a `turn_completed` usage
/// row, the JSON is unusable, or every model row is all zeros. Does not dedupe
/// against other lines; the file readers do.
pub fn parse_turn_completed_line(line: &str, fallback: &GrokFallback) -> Vec<GrokDelta> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let Ok(root) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let Some(params) = root.get("params").filter(|v| v.is_object()) else {
        return Vec::new();
    };
    let Some(update) = params.get("update").filter(|v| v.is_object()) else {
        return Vec::new();
    };
    if update.get("sessionUpdate").and_then(Value::as_str) != Some("turn_completed") {
        return Vec::new();
    }
    let Some(usage) = update.get("usage").filter(|v| v.is_object()) else {
        return Vec::new();
    };

    let meta = params
        .get("_meta")
        .filter(|v| v.is_object())
        .or_else(|| root.get("_meta").filter(|v| v.is_object()));
    let event_id = meta.and_then(|m| non_empty_string(m.get("eventId")));
    let timestamp_ms = resolve_timestamp_ms(root.get("timestamp"), meta);
    let session_id = non_empty_string(params.get("sessionId")).unwrap_or_else(|| {
        if fallback.session_id.is_empty() {
            "unknown".to_string()
        } else {
            fallback.session_id.clone()
        }
    });
    let project_path = if fallback.project_path.is_empty() {
        "unknown".to_string()
    } else {
        fallback.project_path.clone()
    };
    let turn_duration = u64_value(usage.get("apiDurationMs"));

    let mut out = Vec::new();
    for (model, row) in model_rows(usage, fallback.default_model.as_deref()) {
        let mut buckets = buckets_from(&row);
        if buckets.duration_ms == 0 {
            buckets.duration_ms = turn_duration;
        }
        let (input, cache_read, cache_write) = split_input_tokens(
            buckets.input_tokens,
            buckets.cached_read,
            buckets.cache_creation,
        );
        if input == 0
            && cache_read == 0
            && cache_write == 0
            && buckets.output == 0
            && buckets.reasoning == 0
        {
            continue;
        }
        let output = buckets.output;
        let reasoning = buckets.reasoning;
        let stamp = (timestamp_ms > 0).then_some(timestamp_ms);
        let dedupe_key = match event_id.as_deref() {
            Some(event_id) => format!("{event_id}|{model}"),
            None => format!(
                "{session_id}|{timestamp_ms}|{model}|{input}|{output}|{cache_read}|{cache_write}|{reasoning}"
            ),
        };
        out.push(GrokDelta {
            session_id: session_id.clone(),
            project_path: project_path.clone(),
            event_id: event_id.clone(),
            dedupe_key,
            model,
            input,
            output,
            cache_read,
            cache_write,
            reasoning,
            cost_ticks: buckets.cost_ticks,
            timestamp_ms: stamp,
            duration_ms: (buckets.duration_ms > 0).then_some(buckets.duration_ms),
        });
    }
    out
}

/// Session id, project path, and default model for `updates.jsonl`.
///
/// A missing or unreadable summary is not an error: the directory names fill in.
/// `summary_json` that fails to parse is ignored the same way.
pub fn fallback_for_updates(updates: &Path, summary_json: Option<&str>) -> GrokFallback {
    let session_id = updates
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let project_path = updates
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(percent_decode)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let mut fallback = GrokFallback {
        session_id,
        project_path,
        default_model: None,
    };
    let Some(text) = summary_json else {
        return fallback;
    };
    let Ok(summary) = serde_json::from_str::<Value>(text) else {
        return fallback;
    };
    if let Some(id) = summary
        .get("info")
        .and_then(|info| non_empty_string(info.get("id")))
    {
        fallback.session_id = id;
    }
    if let Some(cwd) = summary
        .get("info")
        .and_then(|info| non_empty_string(info.get("cwd")))
        .or_else(|| non_empty_string(summary.get("git_root_dir")))
    {
        fallback.project_path = cwd;
    }
    fallback.default_model = non_empty_string(summary.get("current_model_id"));
    fallback
}

/// Read one `updates.jsonl`. Malformed lines are skipped. I/O errors surface.
/// Dedupe is inside this file only. `usage.json` is not consulted.
pub fn read_updates_history(path: &Path, fallback: &GrokFallback) -> io::Result<Vec<GrokDelta>> {
    let text = fs::read_to_string(path)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for line in text.lines() {
        if !line.contains("\"turn_completed\"") {
            continue;
        }
        for delta in parse_turn_completed_line(line, fallback) {
            if seen.insert(delta.dedupe_key.clone()) {
                out.push(delta);
            }
        }
    }
    Ok(out)
}

/// Read `session_dir/updates.jsonl` plus an optional sibling `summary.json`.
///
/// Missing `updates.jsonl` is an empty ledger (`Ok` empty), not an error.
/// An unreadable `updates.jsonl` is returned. An unreadable or invalid
/// `summary.json` falls back to the directory names.
pub fn read_session_history(session_dir: &Path) -> io::Result<Vec<GrokDelta>> {
    let updates = session_dir.join("updates.jsonl");
    match updates.metadata() {
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Ok(Vec::new()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    }
    let summary_path = session_dir.join("summary.json");
    let summary_text = fs::read_to_string(&summary_path).ok();
    let fallback = fallback_for_updates(&updates, summary_text.as_deref());
    read_updates_history(&updates, &fallback)
}

/// Every `sessions/**/updates.jsonl` under a Grok home, in path order.
///
/// A missing `sessions` directory is an empty home. Directory symlinks are not
/// followed. A file named `updates.jsonl` is included even when it is a symlink.
pub fn discover_update_files(grok_home: &Path) -> io::Result<Vec<PathBuf>> {
    let sessions = grok_home.join("sessions");
    match sessions.metadata() {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(Vec::new()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    }
    let mut files = Vec::new();
    collect_updates(&sessions, &mut files)?;
    files.sort();
    Ok(files)
}

/// Backfill every session under `grok_home` (`$GROK_HOME` or `~/.grok`).
///
/// Each file keeps its own dedupe set. Subagent directories that contain their
/// own `updates.jsonl` are separate bills. `usage.json` is ignored everywhere.
pub fn read_grok_home(grok_home: &Path) -> io::Result<Vec<GrokDelta>> {
    let mut all = Vec::new();
    for updates in discover_update_files(grok_home)? {
        let Some(session_dir) = updates.parent() else {
            continue;
        };
        all.extend(read_session_history(session_dir)?);
    }
    Ok(all)
}

struct Buckets {
    input_tokens: u64,
    output: u64,
    cached_read: u64,
    cache_creation: u64,
    reasoning: u64,
    cost_ticks: u64,
    duration_ms: u64,
}

fn buckets_from(obj: &Value) -> Buckets {
    Buckets {
        input_tokens: u64_value(obj.get("inputTokens")),
        output: u64_value(obj.get("outputTokens")),
        cached_read: u64_value(obj.get("cachedReadTokens")),
        cache_creation: u64_value(obj.get("cacheCreationTokens")),
        reasoning: u64_value(obj.get("reasoningTokens")),
        cost_ticks: u64_value(obj.get("costUsdTicks")),
        duration_ms: u64_value(obj.get("apiDurationMs")),
    }
}

fn model_rows(usage: &Value, default_model: Option<&str>) -> Vec<(String, Value)> {
    if let Some(map) = usage.get("modelUsage").and_then(Value::as_object) {
        if !map.is_empty() {
            let mut rows: Vec<(String, Value)> = map
                .iter()
                .filter(|(_, row)| row.is_object())
                .map(|(model, row)| (model.clone(), row.clone()))
                .collect();
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            return rows;
        }
    }
    let model = default_model
        .filter(|model| !model.is_empty())
        .unwrap_or("unknown")
        .to_string();
    vec![(model, usage.clone())]
}

fn split_input_tokens(input: u64, cached_read: u64, cache_creation: u64) -> (u64, u64, u64) {
    let cache_read = cached_read.min(input);
    let uncached = input.saturating_sub(cache_read);
    let cache_write = cache_creation.min(uncached);
    (uncached - cache_write, cache_read, cache_write)
}

fn resolve_timestamp_ms(envelope: Option<&Value>, meta: Option<&Value>) -> i64 {
    if let Some(ms) = meta
        .and_then(|meta| meta.get("agentTimestampMs"))
        .and_then(value_as_i64)
        .filter(|ms| *ms > 0)
    {
        return ms;
    }
    if let Some(seconds) = envelope
        .and_then(value_as_i64)
        .filter(|seconds| *seconds > 0)
    {
        return seconds.saturating_mul(1000);
    }
    0
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| value.as_f64().map(|n| n as i64))
}

fn u64_value(value: Option<&Value>) -> u64 {
    let Some(value) = value else {
        return 0;
    };
    match value {
        Value::Number(number) => {
            if let Some(n) = number.as_u64() {
                n
            } else if let Some(n) = number.as_i64() {
                if n > 0 {
                    n as u64
                } else {
                    0
                }
            } else if let Some(n) = number.as_f64() {
                if n.is_finite() && n > 0.0 {
                    n.min(u64::MAX as f64) as u64
                } else {
                    0
                }
            } else {
                0
            }
        }
        Value::String(text) => text
            .parse::<u64>()
            .ok()
            .or_else(|| {
                text.parse::<f64>().ok().map(|n| {
                    if n.is_finite() && n > 0.0 {
                        n as u64
                    } else {
                        0
                    }
                })
            })
            .unwrap_or(0),
        _ => 0,
    }
}

fn non_empty_string(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(value.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (from_hex(bytes[i + 1]), from_hex(bytes[i + 2])) {
                out.push(hi * 16 + lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn collect_updates(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let entries = fs::read_dir(dir)?;
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let path = entry.path();
        if file_type.is_dir() {
            collect_updates(&path, out)?;
        } else if entry.file_name() == "updates.jsonl" {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_with(usage: Value, event_id: &str, agent_ms: Option<i64>) -> String {
        let mut meta = serde_json::json!({ "eventId": event_id });
        if let Some(ms) = agent_ms {
            meta["agentTimestampMs"] = serde_json::json!(ms);
        }
        serde_json::json!({
            "timestamp": 1_750_000_000,
            "params": {
                "sessionId": "sess-1",
                "update": { "sessionUpdate": "turn_completed", "usage": usage },
                "_meta": meta
            }
        })
        .to_string()
    }

    fn sample_usage() -> Value {
        serde_json::json!({
            "inputTokens": 100,
            "outputTokens": 20,
            "cachedReadTokens": 40,
            "reasoningTokens": 10,
            "totalTokens": 120,
            "modelUsage": {
                "grok-4.5-build": {
                    "inputTokens": 100,
                    "outputTokens": 20,
                    "cachedReadTokens": 40,
                    "reasoningTokens": 10,
                    "totalTokens": 120
                }
            }
        })
    }

    fn only(line: &str) -> GrokDelta {
        let rows = parse_turn_completed_line(line, &GrokFallback::default());
        assert_eq!(rows.len(), 1);
        rows.into_iter().next().unwrap()
    }

    #[test]
    fn splits_cache_out_of_inclusive_input_and_keeps_reasoning_aside() {
        let delta = only(&line_with(sample_usage(), "evt-1", None));
        assert_eq!(delta.model, "grok-4.5-build");
        assert_eq!(delta.input, 60);
        assert_eq!(delta.cache_read, 40);
        assert_eq!(delta.cache_write, 0);
        assert_eq!(delta.output, 20);
        assert_eq!(delta.reasoning, 10);
        assert_eq!(delta.dedupe_key, "evt-1|grok-4.5-build");
        assert_eq!(delta.parsed_usage().input, 60);
        assert_eq!(delta.parsed_usage().output, 20);
        assert_eq!(PROVIDER, "grok");
    }

    #[test]
    fn clamps_cache_read_and_cache_creation_into_input() {
        let over_read = serde_json::json!({
            "modelUsage": {
                "m": { "inputTokens": 10, "cachedReadTokens": 40, "outputTokens": 1 }
            }
        });
        let delta = only(&line_with(over_read, "evt-clamp", None));
        assert_eq!((delta.input, delta.cache_read, delta.output), (0, 10, 1));

        let creation = serde_json::json!({
            "modelUsage": {
                "grok-4.5-build": {
                    "inputTokens": 100,
                    "outputTokens": 20,
                    "cachedReadTokens": 40,
                    "cacheCreationTokens": 25,
                    "reasoningTokens": 10
                }
            }
        });
        let delta = only(&line_with(creation, "evt-cc", None));
        assert_eq!(
            (
                delta.input,
                delta.cache_read,
                delta.cache_write,
                delta.output
            ),
            (35, 40, 25, 20)
        );

        let creation_only = serde_json::json!({
            "modelUsage": {
                "grok-4.5-build": {
                    "inputTokens": 100,
                    "outputTokens": 5,
                    "cacheCreationTokens": 20
                }
            }
        });
        let delta = only(&line_with(creation_only, "evt-cc2", None));
        assert_eq!((delta.input, delta.cache_write, delta.output), (80, 20, 5));
    }

    #[test]
    fn model_usage_rows_are_sorted_and_replace_the_top_level_totals() {
        let usage = serde_json::json!({
            "inputTokens": 9999,
            "outputTokens": 9999,
            "costUsdTicks": 1,
            "modelUsage": {
                "model-b": { "inputTokens": 20, "outputTokens": 4, "cachedReadTokens": 5, "costUsdTicks": 20 },
                "model-a": { "inputTokens": 10, "outputTokens": 2, "reasoningTokens": 1, "costUsdTicks": 10 }
            }
        });
        let rows = parse_turn_completed_line(
            &line_with(usage, "evt-multi", None),
            &GrokFallback::default(),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].model, "model-a");
        assert_eq!(
            (rows[0].input, rows[0].output, rows[0].reasoning),
            (10, 2, 1)
        );
        assert_eq!(rows[0].cost_ticks, 10);
        assert_eq!(rows[1].model, "model-b");
        assert_eq!(
            (rows[1].input, rows[1].cache_read, rows[1].output),
            (15, 5, 4)
        );
        assert_eq!(rows[0].dedupe_key, "evt-multi|model-a");
        assert_eq!(rows[1].dedupe_key, "evt-multi|model-b");
    }

    #[test]
    fn empty_model_usage_uses_the_summary_model_or_unknown() {
        let usage = serde_json::json!({
            "inputTokens": 50,
            "outputTokens": 5,
            "cachedReadTokens": 10,
            "reasoningTokens": 2,
            "modelUsage": {}
        });
        let line = line_with(usage, "evt-top", None);
        let unknown = only(&line);
        assert_eq!(unknown.model, "unknown");
        assert_eq!(
            (unknown.input, unknown.cache_read, unknown.output),
            (40, 10, 5)
        );

        let named = parse_turn_completed_line(
            &line,
            &GrokFallback {
                default_model: Some("grok-4.6-build".into()),
                ..GrokFallback::default()
            },
        );
        assert_eq!(named[0].model, "grok-4.6-build");
    }

    #[test]
    fn drops_zero_rows_and_keeps_reasoning_only_rows() {
        let zero = serde_json::json!({
            "modelUsage": { "grok-4.5": { "inputTokens": 0, "outputTokens": 0, "reasoningTokens": 0 } }
        });
        assert!(parse_turn_completed_line(
            &line_with(zero, "zero", None),
            &GrokFallback::default()
        )
        .is_empty());

        let reasoning = serde_json::json!({
            "modelUsage": { "grok-4.5": { "reasoningTokens": 4 } }
        });
        let delta = only(&line_with(reasoning, "reason", None));
        assert_eq!((delta.input, delta.output, delta.reasoning), (0, 0, 4));
    }

    #[test]
    fn skips_other_updates_and_broken_json_without_requiring_method() {
        let other = r#"{"timestamp":1750000001,"params":{"update":{"sessionUpdate":"tool_call"}}}"#;
        assert!(parse_turn_completed_line(other, &GrokFallback::default()).is_empty());
        assert!(parse_turn_completed_line("{", &GrokFallback::default()).is_empty());
        let no_method = r#"{"timestamp":1750000000,"params":{"sessionId":"sess-1","update":{"sessionUpdate":"turn_completed","usage":{"inputTokens":8,"outputTokens":1,"modelUsage":{"m":{"inputTokens":8,"outputTokens":1}}}},"_meta":{"eventId":"evt-nm"}}}"#;
        let delta = only(no_method);
        assert_eq!(delta.input, 8);
        assert_eq!(delta.event_id.as_deref(), Some("evt-nm"));
    }

    #[test]
    fn agent_millis_win_over_envelope_seconds_and_format_as_utc() {
        let usage = sample_usage();
        let delta = only(&line_with(usage, "evt-ts", Some(1_785_328_986_355)));
        assert_eq!(delta.timestamp_ms, Some(1_785_328_986_355));
        assert_eq!(
            delta.timestamp_rfc3339().as_deref(),
            Some("2026-07-29T12:43:06.355Z")
        );

        let seconds = only(&line_with(sample_usage(), "evt-sec", None));
        assert_eq!(seconds.timestamp_ms, Some(1_750_000_000_000));
        assert_eq!(
            seconds.timestamp_rfc3339().as_deref(),
            Some("2025-06-15T15:06:40.000Z")
        );
        assert_eq!(format_rfc3339_millis(0), "1970-01-01T00:00:00.000Z");

        let missing = r#"{"params":{"sessionId":"s","update":{"sessionUpdate":"turn_completed","usage":{"outputTokens":1,"modelUsage":{"m":{"outputTokens":1}}}}}}"#;
        let delta = only(missing);
        assert_eq!(delta.timestamp_ms, None);
        assert!(delta.dedupe_key.starts_with("s|0|m|"));
    }

    #[test]
    fn root_meta_is_a_fallback_and_ticks_convert_only_when_positive() {
        let line = r#"{"timestamp":1750000000,"_meta":{"eventId":"evt-root","agentTimestampMs":1785328986355},"params":{"sessionId":"sess-1","update":{"sessionUpdate":"turn_completed","usage":{"outputTokens":1,"costUsdTicks":185192000,"apiDurationMs":321,"modelUsage":{"grok-4.5-build":{"outputTokens":1,"costUsdTicks":185192000}}}}}}"#;
        let delta = only(line);
        assert_eq!(delta.event_id.as_deref(), Some("evt-root"));
        assert_eq!(delta.timestamp_ms, Some(1_785_328_986_355));
        assert_eq!(delta.cost_ticks, 185_192_000);
        assert_eq!(delta.cost_usd(), Some(0.0185192));
        assert_eq!(delta.duration_ms, Some(321));

        let free = serde_json::json!({
            "modelUsage": { "m": { "outputTokens": 1, "costUsdTicks": 0 } }
        });
        assert_eq!(only(&line_with(free, "evt-0", None)).cost_usd(), None);
    }

    #[test]
    fn file_dedupe_keeps_the_first_event_and_distinct_bucket_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updates.jsonl");
        let first = line_with(sample_usage(), "evt-1", None);
        let repeat = line_with(
            serde_json::json!({
                "modelUsage": { "grok-4.5-build": { "inputTokens": 1, "outputTokens": 1 } }
            }),
            "evt-1",
            None,
        );
        let no_id = r#"{"timestamp":1750000100,"params":{"sessionId":"sess-1","update":{"sessionUpdate":"turn_completed","usage":{"modelUsage":{"grok-4.5-build":{"inputTokens":3,"outputTokens":1}}}}}}
{"timestamp":1750000100,"params":{"sessionId":"sess-1","update":{"sessionUpdate":"turn_completed","usage":{"modelUsage":{"grok-4.5-build":{"inputTokens":3,"outputTokens":1}}}}}}
{"timestamp":1750000100,"params":{"sessionId":"sess-1","update":{"sessionUpdate":"turn_completed","usage":{"modelUsage":{"grok-4.5-build":{"inputTokens":4,"outputTokens":1}}}}}}"#;
        fs::write(&path, format!("{first}\n{repeat}\nnot-json\n{no_id}\n")).unwrap();
        let rows = read_updates_history(&path, &GrokFallback::default()).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].input, 60);
        assert_eq!(rows[1].input, 3);
        assert_eq!(rows[2].input, 4);
        assert!(rows[1].event_id.is_none());
    }

    #[test]
    fn summary_overrides_identity_and_percent_decoded_parents_fill_the_gaps() {
        let dir = tempfile::tempdir().unwrap();
        let session = dir
            .path()
            .join("sessions")
            .join("%2FUsers%2Fme%2Fproj")
            .join("dir-id");
        fs::create_dir_all(&session).unwrap();
        let updates = session.join("updates.jsonl");
        fs::write(
            &updates,
            "{\"timestamp\":1750000000,\"params\":{\"update\":{\"sessionUpdate\":\"turn_completed\",\"usage\":{\"outputTokens\":2,\"modelUsage\":{\"m\":{\"outputTokens\":2}}}},\"_meta\":{\"eventId\":\"evt\"}}}\n",
        )
        .unwrap();
        let bare = read_session_history(&session).unwrap();
        assert_eq!(bare[0].session_id, "dir-id");
        assert_eq!(bare[0].project_path, "/Users/me/proj");

        fs::write(
            session.join("summary.json"),
            r#"{"info":{"id":"from-summary","cwd":"/work/repo"},"current_model_id":"grok-4.7-build-fast","git_root_dir":"/ignored"}"#,
        )
        .unwrap();
        let named = read_session_history(&session).unwrap();
        assert_eq!(named[0].session_id, "from-summary");
        assert_eq!(named[0].project_path, "/work/repo");

        fs::write(
            session.join("summary.json"),
            r#"{"git_root_dir":"/git/root","current_model_id":""}"#,
        )
        .unwrap();
        let rooted = fallback_for_updates(
            &updates,
            Some(&fs::read_to_string(session.join("summary.json")).unwrap()),
        );
        assert_eq!(rooted.session_id, "dir-id");
        assert_eq!(rooted.project_path, "/git/root");
        assert_eq!(rooted.default_model, None);
    }

    #[test]
    fn home_backfill_ignores_usage_json_and_bills_each_session_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("grok-home");
        let parent = home.join("sessions").join("proj").join("parent");
        let child = home.join("sessions").join("proj").join("child");
        fs::create_dir_all(&parent).unwrap();
        fs::create_dir_all(&child).unwrap();
        let row = |input: u64, event: &str| {
            format!(
                "{}\n",
                serde_json::json!({
                    "timestamp": 1_750_000_000,
                    "params": {
                        "update": {
                            "sessionUpdate": "turn_completed",
                            "usage": {
                                "inputTokens": input,
                                "modelUsage": { "grok-4.7-build-fast": { "inputTokens": input, "costUsdTicks": 10 } }
                            }
                        },
                        "_meta": { "eventId": event }
                    }
                })
            )
        };
        fs::write(parent.join("updates.jsonl"), row(10, "same-event")).unwrap();
        fs::write(child.join("updates.jsonl"), row(7, "same-event")).unwrap();
        fs::write(
            parent.join("usage.json"),
            r#"{"session":{"inputTokens":999999,"modelUsage":{"grok-4.7-build-fast":{"inputTokens":999999}}}}"#,
        )
        .unwrap();
        fs::write(parent.join("signals.json"), r#"{"contextTokensUsed":123}"#).unwrap();

        assert!(read_grok_home(dir.path()).unwrap().is_empty());
        let rows = read_grok_home(&home).unwrap();
        assert_eq!(rows.len(), 2);
        let input: u64 = rows.iter().map(|row| row.input).sum();
        assert_eq!(input, 17);
        assert!(rows.iter().all(|row| row.model == "grok-4.7-build-fast"));
        assert_eq!(discover_update_files(&home).unwrap().len(), 2);

        let quiet = home.join("sessions").join("proj").join("no-ledger");
        fs::create_dir_all(&quiet).unwrap();
        fs::write(quiet.join("usage.json"), r#"{"session":{"inputTokens":5}}"#).unwrap();
        assert!(read_session_history(&quiet).unwrap().is_empty());
    }

    #[test]
    fn incomplete_usage_is_counted_and_shares_turn_duration() {
        let line = r#"{"timestamp":1750000000,"params":{"sessionId":"s","update":{"sessionUpdate":"turn_completed","elapsed_ms":99999,"usage":{"usageIsIncomplete":true,"apiDurationMs":40,"inputTokens":6,"outputTokens":1,"modelUsage":{"a":{"inputTokens":2,"outputTokens":1},"b":{"inputTokens":4}}}},"_meta":{"eventId":"evt-inc"}}}"#;
        let rows = parse_turn_completed_line(line, &GrokFallback::default());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].input + rows[1].input, 6);
        assert_eq!(rows[0].duration_ms, Some(40));
        assert_eq!(rows[1].duration_ms, Some(40));
        assert!(rows.iter().all(|row| row.cost_usd().is_none()));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_updates_file_surfaces() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updates.jsonl");
        fs::write(&path, b"{}\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        let _restore = RestorePerms(path.clone());
        let err = read_updates_history(&path, &GrokFallback::default()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[cfg(unix)]
    struct RestorePerms(PathBuf);

    #[cfg(unix)]
    impl Drop for RestorePerms {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o644));
        }
    }
}
