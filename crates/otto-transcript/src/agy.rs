//! Antigravity (`agy`) usage history parser (`ccusage v20.0.36` parity).
//!
//! Antigravity stores authoritative per-step token counters and model metadata
//! in SQLite databases under `<root>/conversations/<session-uuid>.db` (with
//! optional `<session-uuid>.db-wal` while a session is live). Each database
//! contains three protobuf-backed tables used for usage accounting:
//!
//! 1. `trajectory_metadata_blob(id TEXT PRIMARY KEY, data BLOB)`
//!    - Field `2` (`Timestamp`: `1=seconds, 2=nanos`): fallback trajectory timestamp
//!    - Field `5` (`string`): optional `parent_conversation_id` (subagent parent)
//!    - Field `7` (repeated `string`): `workspace_uris` (`file:///...`)
//! 2. `gen_metadata(idx INTEGER PRIMARY KEY, data BLOB, size INTEGER)`
//!    - Field `1` (`ChatModelMetadata` message):
//!      - Field `3` (`varint`): `model_enum`
//!      - Field `4` (`BotUsage` message): primary generation token usage
//!      - Field `9` (message) -> Field `4` (`Timestamp`): optional generation timestamp
//!      - Field `17` (repeated message): `(2: BotUsage, 4: request_hash)`
//!      - Field `19` (`string`): raw model name (e.g. `"gemini-4-argon"`)
//!      - Field `20` (repeated KV `(1: key, 2: val)`): `"model_enum"`, `"request_id"`, `"last_step_index"`
//! 3. `steps(idx INTEGER PRIMARY KEY, step_type INTEGER, status INTEGER, has_subtrajectory NUMERIC, metadata BLOB, ...)`
//!    - `metadata` (`StepMetadata` message, column index 4):
//!      - Field `8` (`Timestamp`), fallback Field `1` (`Timestamp`): step timestamp
//!      - Field `9` (`BotUsage` message): step token usage
//!      - Field `28` (repeated message): `(2: BotUsage, 4: request_hash)` (checkpoint/compaction usage)
//!
//! This module provides pure protobuf/conversation blob parsing and discovery
//! helpers ([`parse_agy_conversation_blobs`], [`parse_agy_trajectory_lineage`],
//! [`default_agy_conversation_dirs`]) consumed by the SQLx read-only snapshot
//! reader in `ottod`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::usage::{ClaudeLine, ParsedUsage};

/// Standard Antigravity conversation database subdirectories under `$HOME`,
/// matching `ccusage v20.0.36` discovery order.
pub const AGY_HOME_CONVERSATION_REL_DIRS: &[&str] = &[
    ".gemini/antigravity/conversations",
    ".gemini/antigravity-cli/conversations",
    ".gemini/antigravity-ide/conversations",
    ".gemini/antigravity-backup/conversations",
    ".gemini/antigravity-acp/conversations",
    ".config/antigravity/conversations",
];

/// A single deduplicated Antigravity model invocation extracted from
/// `conversations/<session-uuid>.db`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgyUsageRecord {
    /// Stable deduplication key within Otto's native state (`agy:<session_id>:<id>`).
    pub key: String,
    /// Stable deduplication key within Otto's `SeenKeys` / native state (`agy:<session_id>:<id>`).
    pub dedup_key: String,
    /// All stable alias keys for this merged record (`agy:<session_id>:step:...`,
    /// `agy:<session_id>:gen:...`, `agy:<session_id>:resp:...`, `agy:<session_id>:bot:...`,
    /// `agy:<session_id>:req:...`, `agy:<session_id>:hash:...`).
    pub aliases: Vec<String>,
    /// Conversation UUID (stem of `<uuid>.db`).
    pub session_id: String,
    /// Parent conversation UUID when this conversation is a spawned subagent (field 5).
    pub parent_session_id: Option<String>,
    /// Root conversation UUID when this conversation is a spawned subagent (field 10).
    pub root_session_id: Option<String>,
    /// First workspace path decoded from `trajectory_metadata_blob` (`file:///...`).
    pub workspace_cwd: Option<String>,
    /// UTC RFC3339 timestamp string with millisecond precision (`YYYY-MM-DDTHH:MM:SS.mmmZ`).
    pub timestamp: String,
    /// UTC timestamp in milliseconds since Unix epoch.
    pub timestamp_ms: i64,
    /// Normalized canonical model name.
    pub model: String,
    /// Input tokens (non-cached).
    pub input_tokens: u64,
    /// Total output tokens (`completion_output_tokens + reasoning_tokens`),
    /// including thinking/reasoning once for Otto's `UsageEvent.output_tokens`.
    pub output_tokens: u64,
    /// Completion-only output tokens excluding thinking/reasoning (`BotUsage`
    /// field `10`, falling back to field `3` when fields `9` and `10` are zero),
    /// matching `ccusage` `outputTokens`.
    pub completion_output_tokens: u64,
    /// Cache read tokens.
    pub cache_read_tokens: u64,
    /// Cache creation / write tokens.
    pub cache_write_tokens: u64,
    /// Normalized `ParsedUsage` (`input`, `output`, `cache_read`, `cache_write`, `model`).
    ///
    /// `usage.output` equals [`Self::output_tokens`] (`completion_output_tokens + reasoning_tokens`)
    /// so Otto's `UsageEvent` includes thinking/reasoning tokens once.
    pub usage: ParsedUsage,
    /// Thinking / reasoning tokens (`BotUsage` field `9`).
    pub reasoning_tokens: u64,
}

impl AgyUsageRecord {
    /// Total tokens including thinking/reasoning tokens
    /// (`input_tokens + output_tokens + cache_read_tokens + cache_write_tokens`),
    /// matching `ccusage` `totalTokens`.
    pub fn total_tokens_with_reasoning(&self) -> u64 {
        self.usage.input + self.usage.output + self.usage.cache_read + self.usage.cache_write
    }
}

/// Lineage and workspace metadata decoded from `trajectory_metadata_blob`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgyTrajectoryLineage {
    pub parent_conversation_id: Option<String>,
    pub root_conversation_id: Option<String>,
    pub workspace_cwd: Option<String>,
    pub timestamp_ms: Option<i64>,
}

/// Return all candidate `conversations/` directories for Antigravity under `home`.
/// Pure function of `home` so test/E2E isolation is never polluted by process env.
pub fn default_agy_conversation_dirs(home: &Path) -> Vec<PathBuf> {
    AGY_HOME_CONVERSATION_REL_DIRS
        .iter()
        .map(|rel| home.join(rel))
        .collect()
}

/// Discover all existing, non-empty `<session-uuid>.db` files across the
/// standard Antigravity roots under `home`. If the same `<session-uuid>.db`
/// appears in multiple roots (for example `antigravity-cli` and `antigravity-backup`),
/// the first root in discovery order wins.
pub fn discover_agy_conversation_dbs(home: &Path) -> Vec<PathBuf> {
    let mut seen_stems = HashSet::new();
    let mut out = Vec::new();
    for dir in default_agy_conversation_dirs(home) {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("db"))
            .collect();
        entries.sort();
        for path in entries {
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() < 100 {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if seen_stems.insert(stem.to_string()) {
                out.push(path);
            }
        }
    }
    out
}

/// Decode [`AgyTrajectoryLineage`] across one or more `trajectory_metadata_blob` rows.
pub fn parse_agy_trajectory_metadata(trajectory_blobs: &[Vec<u8>]) -> AgyTrajectoryLineage {
    let mut out = AgyTrajectoryLineage::default();
    for blob in trajectory_blobs {
        for (field_no, wire) in proto::iter_fields(blob) {
            match (field_no, wire) {
                (2, proto::WireValue::Bytes(sub)) => {
                    if let Some(ts) = parse_proto_timestamp_ms(sub) {
                        out.timestamp_ms = Some(ts);
                    }
                }
                (5, proto::WireValue::Bytes(sub)) => {
                    if let Ok(s) = std::str::from_utf8(sub) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            out.parent_conversation_id = Some(trimmed.to_string());
                        }
                    }
                }
                (7, proto::WireValue::Bytes(sub)) => {
                    if out.workspace_cwd.is_none() {
                        if let Some(path) = std::str::from_utf8(sub)
                            .ok()
                            .and_then(|s| s.trim().strip_prefix("file://"))
                        {
                            if !path.is_empty() {
                                out.workspace_cwd = Some(path.to_string());
                            }
                        }
                    }
                }
                (10, proto::WireValue::Bytes(sub)) => {
                    if let Ok(s) = std::str::from_utf8(sub) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            out.root_conversation_id = Some(trimmed.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Extract `(parent_session_id, root_session_id)` from `trajectory_metadata_blob`
/// protobuf blobs (`field 5` = parent conversation ID, `field 10` = root conversation ID).
pub fn parse_agy_trajectory_lineage(
    trajectory_blobs: &[Vec<u8>],
) -> (Option<String>, Option<String>) {
    let meta = parse_agy_trajectory_metadata(trajectory_blobs);
    (meta.parent_conversation_id, meta.root_conversation_id)
}

/// Parse raw protobuf blobs from `trajectory_metadata_blob`, `gen_metadata`,
/// and `steps` into deduplicated [`AgyUsageRecord`] items matching `ccusage v20.0.36`.
pub fn parse_agy_conversation_blobs(
    session_id: &str,
    trajectory_blobs: &[Vec<u8>],
    gen_metadata_blobs: &[(i64, Vec<u8>)],
    step_metadata_blobs: &[(i64, Vec<u8>)],
) -> Vec<AgyUsageRecord> {
    if gen_metadata_blobs.is_empty() && step_metadata_blobs.is_empty() {
        return Vec::new();
    }

    let traj_meta = parse_agy_trajectory_metadata(trajectory_blobs);
    let traj_ts_ms = traj_meta.timestamp_ms;
    let parent_session_id = traj_meta.parent_conversation_id;
    let root_session_id = traj_meta.root_conversation_id;
    let workspace_cwd = traj_meta.workspace_cwd;

    let mut candidates: Vec<UsageCandidate> = Vec::new();
    let mut fallback_model: Option<String> = None;

    // 1. Parse `gen_metadata` rows in ascending `idx` order.
    for (idx, blob) in gen_metadata_blobs {
        for (field_no, wire) in proto::iter_fields(blob) {
            if let (1, proto::WireValue::Bytes(cm_bytes)) = (field_no, wire) {
                parse_chat_model_metadata(*idx, cm_bytes, &mut fallback_model, &mut candidates);
            }
        }
    }

    // 2. Parse `steps` metadata rows in ascending `idx` order.
    let mut first_step_ts_ms: Option<i64> = None;
    for (idx, blob) in step_metadata_blobs {
        parse_step_metadata(
            *idx,
            blob,
            fallback_model.as_deref(),
            &mut first_step_ts_ms,
            &mut candidates,
        );
    }

    let fallback_ts_ms = first_step_ts_ms.or(traj_ts_ms);

    // 3. Merge/deduplicate candidates sharing any identifier key (`response_id`,
    //    `bot_id`, `request_id`, or `request_hash`), filling missing timestamps
    //    and models in-place exactly like `ccusage`.
    let mut key_to_merged_idx: HashMap<String, usize> = HashMap::new();
    let mut merged: Vec<UsageCandidate> = Vec::new();
    let mut merged_source_tags: Vec<Vec<String>> = Vec::new();

    for cand in candidates {
        let existing_idx = cand
            .keys
            .iter()
            .find_map(|k| key_to_merged_idx.get(k).copied());
        if let Some(m_idx) = existing_idx {
            let existing = &mut merged[m_idx];
            if existing.timestamp_ms.is_none() && cand.timestamp_ms.is_some() {
                existing.timestamp_ms = cand.timestamp_ms;
            }
            if existing.model.is_none() && cand.model.is_some() {
                existing.model = cand.model;
            }
            existing.input_tokens = existing.input_tokens.max(cand.input_tokens);
            existing.output_tokens = existing.output_tokens.max(cand.output_tokens);
            existing.completion_output_tokens = existing
                .completion_output_tokens
                .max(cand.completion_output_tokens);
            existing.reasoning_tokens = existing.reasoning_tokens.max(cand.reasoning_tokens);
            existing.cache_read_tokens = existing.cache_read_tokens.max(cand.cache_read_tokens);
            existing.cache_write_tokens = existing.cache_write_tokens.max(cand.cache_write_tokens);
            if !merged_source_tags[m_idx].contains(&cand.source_tag) {
                merged_source_tags[m_idx].push(cand.source_tag);
            }
            for k in cand.keys {
                if !existing.keys.contains(&k) {
                    existing.keys.push(k.clone());
                }
                key_to_merged_idx.insert(k, m_idx);
            }
        } else {
            let m_idx = merged.len();
            for k in &cand.keys {
                key_to_merged_idx.insert(k.clone(), m_idx);
            }
            merged_source_tags.push(vec![cand.source_tag.clone()]);
            merged.push(cand);
        }
    }

    let default_model = fallback_model.unwrap_or_else(|| "unknown".to_string());
    let mut records = Vec::with_capacity(merged.len());

    for (cand, source_tags) in merged.into_iter().zip(merged_source_tags) {
        let Some(timestamp_ms) = cand.timestamp_ms.or(fallback_ts_ms) else {
            continue;
        };
        let model = cand.model.unwrap_or_else(|| default_model.clone());
        // Anchor primary_key to the immutable SQLite step row tag (`step:<idx>:f9`
        // or `step:<idx>:f28:<sub>`) whenever a step row is part of the merged
        // candidate (falling back to the first `gen:<idx>:...` tag). This ensures
        // `key`/`dedup_key` never changes when `response_id`/`bot_id` or a
        // `gen_metadata` row is added on a later scan.
        let primary_key = source_tags
            .iter()
            .find(|t| t.starts_with("step:"))
            .or_else(|| source_tags.first())
            .cloned()
            .unwrap_or_else(|| cand.source_tag.clone());
        let dedup_key = format!("agy:{session_id}:{primary_key}");
        let mut aliases = Vec::with_capacity(source_tags.len() + cand.keys.len());
        aliases.push(dedup_key.clone());
        for tag in &source_tags {
            let a = format!("agy:{session_id}:{tag}");
            if !aliases.contains(&a) {
                aliases.push(a);
            }
        }
        for k in &cand.keys {
            let a = format!("agy:{session_id}:{k}");
            if !aliases.contains(&a) {
                aliases.push(a);
            }
        }
        let timestamp = format_rfc3339_millis(timestamp_ms);
        records.push(AgyUsageRecord {
            key: dedup_key.clone(),
            dedup_key,
            aliases,
            session_id: session_id.to_string(),
            parent_session_id: parent_session_id.clone(),
            root_session_id: root_session_id.clone(),
            workspace_cwd: workspace_cwd.clone(),
            timestamp,
            timestamp_ms,
            model: model.clone(),
            input_tokens: cand.input_tokens,
            output_tokens: cand.output_tokens,
            completion_output_tokens: cand.completion_output_tokens,
            cache_read_tokens: cand.cache_read_tokens,
            cache_write_tokens: cand.cache_write_tokens,
            usage: ParsedUsage {
                model,
                input: cand.input_tokens,
                output: cand.output_tokens,
                cache_read: cand.cache_read_tokens,
                cache_write: cand.cache_write_tokens,
            },
            reasoning_tokens: cand.reasoning_tokens,
        });
    }

    records
}

/// Normalize an Antigravity `model_enum` integer code to its canonical `ccusage`
/// model name.
pub fn normalize_agy_model_enum(code: u64) -> Option<String> {
    match code {
        0 => None,
        246 => Some("gemini-2.5-pro".to_string()),
        281 | 282 => Some("claude-4-sonnet".to_string()),
        290 | 291 => Some("claude-4-opus".to_string()),
        312 => Some("gemini-2.5-flash".to_string()),
        313 | 329 => Some("gemini-2.5-flash-thinking".to_string()),
        330 => Some("gemini-2.5-flash-lite".to_string()),
        333 | 334 => Some("claude-4.5-sonnet".to_string()),
        340 | 341 => Some("claude-4.5-haiku".to_string()),
        342 => Some("gpt-oss-120b-medium".to_string()),
        1050 => Some("gemini-3.1-flash-lite".to_string()),
        1071 => Some("gemini-3.6-flash-high".to_string()),
        1072 => Some("gemini-3.6-flash-medium".to_string()),
        1073 => Some("gemini-3.6-flash-low".to_string()),
        1298 => Some("gemini-3.7-flash-high".to_string()),
        1299 => Some("gemini-3.7-flash-medium".to_string()),
        1300 => Some("gemini-3.7-flash-low".to_string()),
        1318 => Some("gemini-3.8-flash-high".to_string()),
        1319 => Some("gemini-3.8-flash-medium".to_string()),
        1320 => Some("gemini-3.8-flash-low".to_string()),
        1417..=1419 => Some("gemini-4-argon".to_string()),
        n if n >= 1000 => normalize_agy_model(&format!("model_placeholder_m{}", n - 1000)),
        n => Some(format!("antigravity-model-{n}")),
    }
}

/// Normalize an Antigravity model string (`MODEL_PLACEHOLDER_M*` or raw model
/// identifier) to its canonical `ccusage v20.0.36` model name.
pub fn normalize_agy_model(raw: &str) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    let mapped = match lower.as_str() {
        "gemini 3.8 flash (high)" | "model_placeholder_m318" => "gemini-3.8-flash-high",
        "gemini 3.8 flash (medium)" | "model_placeholder_m319" => "gemini-3.8-flash-medium",
        "gemini 3.8 flash (low)" | "model_placeholder_m320" => "gemini-3.8-flash-low",
        "gemini 3.7 flash (high)" | "model_placeholder_m298" => "gemini-3.7-flash-high",
        "gemini 3.7 flash (medium)" | "model_placeholder_m299" => "gemini-3.7-flash-medium",
        "gemini 3.7 flash (low)" | "model_placeholder_m300" => "gemini-3.7-flash-low",
        "gemini 3.6 flash (high)" | "model_placeholder_m71" => "gemini-3.6-flash-high",
        "gemini 3.6 flash (medium)" | "model_placeholder_m72" => "gemini-3.6-flash-medium",
        "gemini 3.6 flash (low)" | "model_placeholder_m73" => "gemini-3.6-flash-low",
        "gemini 3.8 flash" | "gemini 3.8 flash thinking" => "gemini-3.8-flash",
        "gemini 3.7 flash" | "gemini 3.7 flash thinking" => "gemini-3.7-flash",
        "gemini 3.7 pro" | "gemini 3.7 pro thinking" => "gemini-3.7-pro",
        "gemini 3.6 flash" | "gemini 3 flash" => "gemini-3.6-flash",
        "gemini 3.6 pro" => "gemini-3.6-pro",
        "gemini 3 pro" | "gemini 3 pro thinking" | "gemini-3-pro-high" | "gemini-3-pro-low" => {
            "gemini-3-pro"
        }
        "gemini 2.5 flash" => "gemini-2.5-flash",
        "gemini 2.5 pro" => "gemini-2.5-pro",
        "gemini 2.0 flash" | "gemini 2 flash" => "gemini-2.0-flash",
        "gemini 2.0 pro" => "gemini-2.0-pro",
        "gemini 1.5 flash" => "gemini-1.5-flash",
        "gemini 1.5 pro" => "gemini-1.5-pro",
        "model_placeholder_m417" | "model_placeholder_m418" | "model_placeholder_m419" => {
            "gemini-4-argon"
        }
        "model_placeholder_m50" => "gemini-3.1-flash-lite",
        "model_placeholder_m26" => "claude-opus-4-6",
        "model_placeholder_m35" => "claude-sonnet-4-6",
        "model_placeholder_m36"
        | "model_placeholder_m37"
        | "model_placeholder_m16"
        | "gemini-pro-default"
        | "gemini-pro-agent"
        | "gemini-3.1-pro-high"
        | "gemini-3.1-pro-low" => "gemini-3.1-pro",
        "model_placeholder_m18"
        | "model_placeholder_m84"
        | "model_placeholder_m47"
        | "gemini-3-flash-c"
        | "gemini-3-flash" => "gemini-3-flash-preview",
        "model_placeholder_m132"
        | "model_placeholder_m133"
        | "gemini-3-flash-agent"
        | "gemini-3-flash-agent-a"
        | "gemini-3-flash-agent-b"
        | "gemini-3-flash-a"
        | "gemini-3-flash-b" => "gemini-3.5-flash-high",
        "model_placeholder_m187" => "gemini-3.5-flash-extra-low",
        "model_placeholder_m20" | "gemini-3.5-flash-low" => "gemini-3.5-flash-medium",
        "model_openai_gpt_oss_120b_medium" => "gpt-oss-120b-medium",
        "claude 3.7 sonnet" | "claude 3.7 sonnet thinking" => "claude-3-7-sonnet",
        "claude 3.5 sonnet" => "claude-3-5-sonnet",
        "claude 3.5 haiku" => "claude-3-5-haiku",
        "claude 3 opus" => "claude-3-opus",
        other => return Some(other.replace(' ', "-")),
    };
    Some(mapped.to_string())
}

/// Optional JSONL fallback parser for `brain/<id>/.system_generated/logs/transcript.jsonl`
/// (`type == "PLANNER_RESPONSE"` lines carrying `token_counts`).
///
/// Note: Antigravity's `transcript.jsonl` does not include model names and only
/// records token usage on recent CLI builds. SQLite blobs parsed by
/// [`parse_agy_conversation_blobs`] are the authoritative history source.
pub fn parse_agy_transcript_line(line: &str) -> Option<ClaudeLine> {
    let line = line.trim();
    if line.is_empty()
        || !line.contains("\"PLANNER_RESPONSE\"")
        || !line.contains("\"token_counts\"")
    {
        return None;
    }
    let v: AgyJsonlWire<'_> = serde_json::from_str(line).ok()?;
    if v.step_type.as_deref() != Some("PLANNER_RESPONSE") {
        return None;
    }
    let tc = v.token_counts?;
    let total_any = tc.input_tokens
        + tc.output_tokens
        + tc.thinking_tokens
        + tc.cached_read_tokens
        + tc.ram_cache_read_tokens;
    if total_any == 0 {
        return None;
    }
    let dedup_key = v
        .step_index
        .map(|idx| format!("step:{idx}:{}", v.created_at.as_deref().unwrap_or_default()));
    Some(ClaudeLine {
        usage: ParsedUsage {
            model: "gemini-4-argon".to_string(),
            input: tc.input_tokens,
            output: tc.output_tokens.saturating_add(tc.thinking_tokens),
            cache_read: tc.cached_read_tokens + tc.ram_cache_read_tokens,
            cache_write: 0,
        },
        dedup_key,
        timestamp: v.created_at.map(|s| s.into_owned()),
        complete: v.status.as_deref() == Some("DONE"),
    })
}

#[derive(Deserialize)]
struct AgyJsonlWire<'a> {
    #[serde(rename = "type", default, borrow)]
    step_type: Option<std::borrow::Cow<'a, str>>,
    #[serde(default)]
    step_index: Option<u64>,
    #[serde(default, borrow)]
    status: Option<std::borrow::Cow<'a, str>>,
    #[serde(default, borrow)]
    created_at: Option<std::borrow::Cow<'a, str>>,
    #[serde(default)]
    token_counts: Option<AgyJsonlTokenCounts>,
}

#[derive(Deserialize, Default)]
struct AgyJsonlTokenCounts {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    thinking_tokens: u64,
    #[serde(default)]
    cached_read_tokens: u64,
    #[serde(default)]
    ram_cache_read_tokens: u64,
}

#[derive(Debug, Clone)]
struct UsageCandidate {
    source_tag: String,
    timestamp_ms: Option<i64>,
    model: Option<String>,
    input_tokens: u64,
    output_tokens: u64,
    completion_output_tokens: u64,
    reasoning_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    keys: Vec<String>,
}

#[derive(Debug, Default)]
struct RawBotUsage {
    model_enum: u64,
    input_tokens: u64,
    total_output_tokens: u64,
    cache_write_tokens: u64,
    cache_read_tokens: u64,
    reasoning_tokens: u64,
    completion_output_tokens: u64,
    bot_id: Option<String>,
    response_id: Option<String>,
    model_override: Option<String>,
}

impl RawBotUsage {
    fn has_any_tokens(&self) -> bool {
        self.input_tokens > 0
            || self.total_output_tokens > 0
            || self.cache_write_tokens > 0
            || self.cache_read_tokens > 0
            || self.reasoning_tokens > 0
            || self.completion_output_tokens > 0
    }

    fn normalized_output_tokens(&self) -> u64 {
        if self.completion_output_tokens > 0 || self.reasoning_tokens > 0 {
            self.completion_output_tokens
                .saturating_add(self.reasoning_tokens)
        } else {
            self.total_output_tokens
        }
    }

    fn completion_only_tokens(&self) -> u64 {
        if self.completion_output_tokens > 0 || self.reasoning_tokens > 0 {
            self.completion_output_tokens
        } else {
            self.total_output_tokens
        }
    }

    fn resolved_model(&self) -> Option<String> {
        self.model_override
            .as_deref()
            .and_then(normalize_agy_model)
            .or_else(|| normalize_agy_model_enum(self.model_enum))
    }

    fn base_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(resp_id) = &self.response_id {
            keys.push(format!("resp:{resp_id}"));
        }
        if let Some(bot_id) = &self.bot_id {
            keys.push(format!("bot:{bot_id}"));
        }
        keys
    }
}

fn parse_bot_usage(bytes: &[u8]) -> Option<RawBotUsage> {
    let mut u = RawBotUsage::default();
    for (field_no, wire) in proto::iter_fields(bytes) {
        match (field_no, wire) {
            (1, proto::WireValue::Varint(v)) => u.model_enum = v,
            (2, proto::WireValue::Varint(v)) => u.input_tokens = v,
            (3, proto::WireValue::Varint(v)) => u.total_output_tokens = v,
            (4, proto::WireValue::Varint(v)) => u.cache_write_tokens = v,
            (5, proto::WireValue::Varint(v)) => u.cache_read_tokens = v,
            (7, proto::WireValue::Bytes(b)) => {
                if let Ok(s) = std::str::from_utf8(b) {
                    let t = s.trim();
                    if !t.is_empty() {
                        u.bot_id = Some(t.to_string());
                    }
                }
            }
            (9, proto::WireValue::Varint(v)) => u.reasoning_tokens = v,
            (10, proto::WireValue::Varint(v)) => u.completion_output_tokens = v,
            (11, proto::WireValue::Bytes(b)) => {
                if let Ok(s) = std::str::from_utf8(b) {
                    let t = s.trim();
                    if !t.is_empty() {
                        u.response_id = Some(t.to_string());
                    }
                }
            }
            (12, proto::WireValue::Bytes(b)) => {
                if let Ok(s) = std::str::from_utf8(b) {
                    let t = s.trim();
                    if !t.is_empty() {
                        u.model_override = Some(t.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    if u.has_any_tokens() {
        Some(u)
    } else {
        None
    }
}

fn parse_proto_timestamp_ms(bytes: &[u8]) -> Option<i64> {
    let mut secs: i64 = 0;
    let mut nanos: i64 = 0;
    for (field_no, wire) in proto::iter_fields(bytes) {
        match (field_no, wire) {
            (1, proto::WireValue::Varint(v)) => secs = v as i64,
            (2, proto::WireValue::Varint(v)) => nanos = v as i64,
            _ => {}
        }
    }
    if secs <= 0 {
        return None;
    }
    let clamped_nanos = nanos.clamp(0, 999_999_999);
    secs.checked_mul(1000)
        .and_then(|ms| ms.checked_add(clamped_nanos / 1_000_000))
}

fn format_rfc3339_millis(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000) as u32;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400) as u32;
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Howard Hinnant's `civil_from_days`. `days` is Unix epoch days.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let mut year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    (year as i32, month as u32, day as u32)
}

fn parse_chat_model_metadata(
    row_idx: i64,
    cm_bytes: &[u8],
    fallback_model: &mut Option<String>,
    candidates: &mut Vec<UsageCandidate>,
) {
    let mut f3_enum: u64 = 0;
    let mut f19_model: Option<String> = None;
    let mut kv_model_enum: Option<String> = None;
    let mut kv_request_id: Option<String> = None;
    let mut f9_ts_ms: Option<i64> = None;
    let mut u4: Option<RawBotUsage> = None;
    let mut f17_list: Vec<(RawBotUsage, Option<String>)> = Vec::new();

    for (field_no, wire) in proto::iter_fields(cm_bytes) {
        match (field_no, wire) {
            (3, proto::WireValue::Varint(v)) => f3_enum = v,
            (4, proto::WireValue::Bytes(sub)) => {
                if let Some(parsed) = parse_bot_usage(sub) {
                    u4 = Some(parsed);
                }
            }
            (9, proto::WireValue::Bytes(sub)) => {
                for (sf, sw) in proto::iter_fields(sub) {
                    if let (4, proto::WireValue::Bytes(ts_bytes)) = (sf, sw) {
                        if let Some(ts) = parse_proto_timestamp_ms(ts_bytes) {
                            f9_ts_ms = Some(ts);
                        }
                    }
                }
            }
            (17, proto::WireValue::Bytes(sub)) => {
                let mut sub_usage: Option<RawBotUsage> = None;
                let mut rhash: Option<String> = None;
                for (sf, sw) in proto::iter_fields(sub) {
                    match (sf, sw) {
                        (2, proto::WireValue::Bytes(ub)) => {
                            sub_usage = parse_bot_usage(ub);
                        }
                        (4, proto::WireValue::Bytes(hb)) => {
                            if let Ok(s) = std::str::from_utf8(hb) {
                                let t = s.trim();
                                if !t.is_empty() {
                                    rhash = Some(t.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(u) = sub_usage {
                    f17_list.push((u, rhash));
                }
            }
            (19, proto::WireValue::Bytes(sub)) => {
                if let Some(norm) = std::str::from_utf8(sub).ok().and_then(normalize_agy_model) {
                    f19_model = Some(norm);
                }
            }
            (20, proto::WireValue::Bytes(sub)) => {
                let mut k: Option<&str> = None;
                let mut v: Option<&str> = None;
                for (sf, sw) in proto::iter_fields(sub) {
                    match (sf, sw) {
                        (1, proto::WireValue::Bytes(kb)) => k = std::str::from_utf8(kb).ok(),
                        (2, proto::WireValue::Bytes(vb)) => v = std::str::from_utf8(vb).ok(),
                        _ => {}
                    }
                }
                if let (Some(key), Some(val)) = (k, v) {
                    match key.trim() {
                        "model_enum" => kv_model_enum = normalize_agy_model(val),
                        "request_id" => {
                            let t = val.trim();
                            if !t.is_empty() {
                                kv_request_id = Some(t.to_string());
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let row_model = normalize_agy_model_enum(f3_enum)
        .or(kv_model_enum)
        .or_else(|| f19_model.clone());
    if fallback_model.is_none() {
        *fallback_model = f19_model.or_else(|| row_model.clone());
    }

    if let Some(u) = u4 {
        let mut keys = u.base_keys();
        if let Some(req_id) = &kv_request_id {
            keys.push(format!("req:{req_id}"));
        }
        candidates.push(UsageCandidate {
            source_tag: format!("gen:{row_idx}:f4"),
            timestamp_ms: f9_ts_ms,
            model: u.resolved_model().or_else(|| row_model.clone()),
            input_tokens: u.input_tokens,
            output_tokens: u.normalized_output_tokens(),
            completion_output_tokens: u.completion_only_tokens(),
            reasoning_tokens: u.reasoning_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            keys,
        });
    }

    for (sub_idx, (u, rhash)) in f17_list.into_iter().enumerate() {
        let mut keys = u.base_keys();
        if let Some(h) = rhash {
            keys.push(format!("hash:{h}"));
        }
        candidates.push(UsageCandidate {
            source_tag: format!("gen:{row_idx}:f17:{sub_idx}"),
            timestamp_ms: f9_ts_ms,
            model: u.resolved_model().or_else(|| row_model.clone()),
            input_tokens: u.input_tokens,
            output_tokens: u.normalized_output_tokens(),
            completion_output_tokens: u.completion_only_tokens(),
            reasoning_tokens: u.reasoning_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            keys,
        });
    }
}

fn parse_step_metadata(
    step_idx: i64,
    step_bytes: &[u8],
    fallback_model: Option<&str>,
    first_step_ts_ms: &mut Option<i64>,
    candidates: &mut Vec<UsageCandidate>,
) {
    let mut ts_f8: Option<i64> = None;
    let mut ts_f1: Option<i64> = None;
    let mut u9: Option<RawBotUsage> = None;
    let mut f28_list: Vec<(RawBotUsage, Option<String>)> = Vec::new();

    for (field_no, wire) in proto::iter_fields(step_bytes) {
        match (field_no, wire) {
            (1, proto::WireValue::Bytes(sub)) => {
                if ts_f1.is_none() {
                    ts_f1 = parse_proto_timestamp_ms(sub);
                }
            }
            (8, proto::WireValue::Bytes(sub)) => {
                if ts_f8.is_none() {
                    ts_f8 = parse_proto_timestamp_ms(sub);
                }
            }
            (9, proto::WireValue::Bytes(sub)) => {
                if let Some(parsed) = parse_bot_usage(sub) {
                    u9 = Some(parsed);
                }
            }
            (28, proto::WireValue::Bytes(sub)) => {
                let mut sub_usage: Option<RawBotUsage> = None;
                let mut rhash: Option<String> = None;
                for (sf, sw) in proto::iter_fields(sub) {
                    match (sf, sw) {
                        (2, proto::WireValue::Bytes(ub)) => {
                            sub_usage = parse_bot_usage(ub);
                        }
                        (4, proto::WireValue::Bytes(hb)) => {
                            if let Ok(s) = std::str::from_utf8(hb) {
                                let t = s.trim();
                                if !t.is_empty() {
                                    rhash = Some(t.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(u) = sub_usage {
                    f28_list.push((u, rhash));
                }
            }
            _ => {}
        }
    }

    let step_ts = ts_f8.or(ts_f1);
    if first_step_ts_ms.is_none() && step_ts.is_some() {
        *first_step_ts_ms = step_ts;
    }

    if let Some(u) = u9 {
        let keys = u.base_keys();
        candidates.push(UsageCandidate {
            source_tag: format!("step:{step_idx}:f9"),
            timestamp_ms: step_ts,
            model: u
                .resolved_model()
                .or_else(|| fallback_model.map(ToOwned::to_owned)),
            input_tokens: u.input_tokens,
            output_tokens: u.normalized_output_tokens(),
            completion_output_tokens: u.completion_only_tokens(),
            reasoning_tokens: u.reasoning_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            keys,
        });
    }

    for (sub_idx, (u, rhash)) in f28_list.into_iter().enumerate() {
        let mut keys = u.base_keys();
        if let Some(h) = rhash {
            keys.push(format!("hash:{h}"));
        }
        candidates.push(UsageCandidate {
            source_tag: format!("step:{step_idx}:f28:{sub_idx}"),
            timestamp_ms: step_ts,
            model: u
                .resolved_model()
                .or_else(|| fallback_model.map(ToOwned::to_owned)),
            input_tokens: u.input_tokens,
            output_tokens: u.normalized_output_tokens(),
            completion_output_tokens: u.completion_only_tokens(),
            reasoning_tokens: u.reasoning_tokens,
            cache_read_tokens: u.cache_read_tokens,
            cache_write_tokens: u.cache_write_tokens,
            keys,
        });
    }
}

/// Minimal, zero-dependency protobuf wire reader (varint, 64-bit, length-delimited, 32-bit).
mod proto {
    #[derive(Debug, Clone, Copy)]
    pub enum WireValue<'a> {
        Varint(u64),
        Bytes(&'a [u8]),
        Fixed64,
        Fixed32,
    }

    pub fn read_varint(buf: &[u8], mut pos: usize) -> Option<(u64, usize)> {
        let mut result: u64 = 0;
        let mut shift: u32 = 0;
        while pos < buf.len() {
            let b = buf[pos];
            pos += 1;
            if shift < 64 {
                result |= u64::from(b & 0x7f) << shift;
            }
            if (b & 0x80) == 0 {
                return Some((result, pos));
            }
            shift += 7;
            if shift > 70 {
                return None;
            }
        }
        None
    }

    pub fn iter_fields(buf: &[u8]) -> FieldIter<'_> {
        FieldIter { buf, pos: 0 }
    }

    pub struct FieldIter<'a> {
        buf: &'a [u8],
        pos: usize,
    }

    impl<'a> Iterator for FieldIter<'a> {
        type Item = (u32, WireValue<'a>);

        fn next(&mut self) -> Option<Self::Item> {
            if self.pos >= self.buf.len() {
                return None;
            }
            let (tag, next_pos) = read_varint(self.buf, self.pos)?;
            self.pos = next_pos;
            let field_no = u32::try_from(tag >> 3).ok()?;
            if field_no == 0 {
                return None;
            }
            let wire_type = (tag & 0x07) as u8;
            match wire_type {
                0 => {
                    let (v, p) = read_varint(self.buf, self.pos)?;
                    self.pos = p;
                    Some((field_no, WireValue::Varint(v)))
                }
                1 => {
                    let end = self.pos.checked_add(8)?;
                    if end > self.buf.len() {
                        return None;
                    }
                    self.pos = end;
                    Some((field_no, WireValue::Fixed64))
                }
                2 => {
                    let (len_u64, p) = read_varint(self.buf, self.pos)?;
                    let len = usize::try_from(len_u64).ok()?;
                    let end = p.checked_add(len)?;
                    if end > self.buf.len() {
                        return None;
                    }
                    self.pos = end;
                    Some((field_no, WireValue::Bytes(&self.buf[p..end])))
                }
                5 => {
                    let end = self.pos.checked_add(4)?;
                    if end > self.buf.len() {
                        return None;
                    }
                    self.pos = end;
                    Some((field_no, WireValue::Fixed32))
                }
                _ => None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    fn encode_varint_field(field_no: u32, val: u64, out: &mut Vec<u8>) {
        encode_varint(u64::from(field_no) << 3, out);
        encode_varint(val, out);
    }

    fn encode_bytes_field(field_no: u32, bytes: &[u8], out: &mut Vec<u8>) {
        encode_varint((u64::from(field_no) << 3) | 2, out);
        encode_varint(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }

    fn make_timestamp_blob(secs: u64, nanos: u64) -> Vec<u8> {
        let mut b = Vec::new();
        encode_varint_field(1, secs, &mut b);
        encode_varint_field(2, nanos, &mut b);
        b
    }

    #[allow(clippy::too_many_arguments)]
    fn make_bot_usage_blob(
        model_enum: u64,
        input: u64,
        total_out: u64,
        cache_read: u64,
        reasoning: u64,
        completion_out: u64,
        bot_id: Option<&str>,
        response_id: Option<&str>,
    ) -> Vec<u8> {
        let mut b = Vec::new();
        if model_enum > 0 {
            encode_varint_field(1, model_enum, &mut b);
        }
        if input > 0 {
            encode_varint_field(2, input, &mut b);
        }
        if total_out > 0 {
            encode_varint_field(3, total_out, &mut b);
        }
        if cache_read > 0 {
            encode_varint_field(5, cache_read, &mut b);
        }
        if let Some(bid) = bot_id {
            encode_bytes_field(7, bid.as_bytes(), &mut b);
        }
        if reasoning > 0 {
            encode_varint_field(9, reasoning, &mut b);
        }
        if completion_out > 0 {
            encode_varint_field(10, completion_out, &mut b);
        }
        if let Some(rid) = response_id {
            encode_bytes_field(11, rid.as_bytes(), &mut b);
        }
        b
    }

    #[test]
    fn model_enum_and_placeholder_normalization_matches_ccusage() {
        assert_eq!(
            normalize_agy_model_enum(1418).as_deref(),
            Some("gemini-4-argon")
        );
        assert_eq!(
            normalize_agy_model_enum(1318).as_deref(),
            Some("gemini-3.8-flash-high")
        );
        assert_eq!(
            normalize_agy_model_enum(1298).as_deref(),
            Some("gemini-3.7-flash-high")
        );
        assert_eq!(
            normalize_agy_model_enum(1264).as_deref(),
            Some("model_placeholder_m264")
        );
        assert_eq!(
            normalize_agy_model_enum(1132).as_deref(),
            Some("gemini-3.5-flash-high")
        );
        assert_eq!(
            normalize_agy_model_enum(1084).as_deref(),
            Some("gemini-3-flash-preview")
        );
        assert_eq!(
            normalize_agy_model_enum(1050).as_deref(),
            Some("gemini-3.1-flash-lite")
        );
        assert_eq!(
            normalize_agy_model_enum(1016).as_deref(),
            Some("gemini-3.1-pro")
        );
        assert_eq!(
            normalize_agy_model("gemini-pro-default").as_deref(),
            Some("gemini-3.1-pro")
        );
        assert_eq!(
            normalize_agy_model("gemini-3-flash-a").as_deref(),
            Some("gemini-3.5-flash-high")
        );
        assert_eq!(
            normalize_agy_model("MODEL_PLACEHOLDER_M418").as_deref(),
            Some("gemini-4-argon")
        );
    }

    #[test]
    fn merges_gen_metadata_and_steps_including_checkpoint_f28() {
        // Trajectory metadata with workspace URI and parent session ID.
        let mut traj = Vec::new();
        encode_bytes_field(2, &make_timestamp_blob(1_760_000_000, 0), &mut traj);
        encode_bytes_field(5, b"parent-conv-123", &mut traj);
        encode_bytes_field(7, b"file:///Users/test/repo", &mut traj);

        // gen_metadata row 0: f3=1318, f19="gemini-3.8-flash", f4 BotUsage with response_id "resp-1".
        let u_gen = make_bot_usage_blob(
            1318,
            10_000,
            350,
            50_000,
            50,
            300,
            Some("bot-1"),
            Some("resp-1"),
        );
        let mut cm = Vec::new();
        encode_varint_field(3, 1318, &mut cm);
        encode_bytes_field(4, &u_gen, &mut cm);
        encode_bytes_field(19, b"gemini-3.8-flash", &mut cm);
        let mut gen_row = Vec::new();
        encode_bytes_field(1, &cm, &mut gen_row);

        // step 2: regular step with f9 sharing "resp-1" and carrying the authoritative step timestamp.
        let mut step2 = Vec::new();
        encode_bytes_field(
            8,
            &make_timestamp_blob(1_760_000_123, 456_000_000),
            &mut step2,
        );
        encode_bytes_field(9, &u_gen, &mut step2);

        // step 5: checkpoint step (f28.2) with model_enum=0 (inherits fallback_model "gemini-3.8-flash").
        let u_ckpt = make_bot_usage_blob(0, 2_500, 120, 8_000, 0, 120, None, None);
        let mut f28_entry = Vec::new();
        encode_bytes_field(2, &u_ckpt, &mut f28_entry);
        encode_bytes_field(4, b"ckpt-hash-99", &mut f28_entry);
        let mut step5 = Vec::new();
        encode_bytes_field(1, &make_timestamp_blob(1_760_000_200, 0), &mut step5);
        encode_bytes_field(28, &f28_entry, &mut step5);

        let records = parse_agy_conversation_blobs(
            "conv-abc",
            &[traj],
            &[(0, gen_row)],
            &[(2, step2), (5, step5)],
        );
        assert_eq!(records.len(), 2);

        // First record merged gen_metadata[0] + step[2]
        assert_eq!(records[0].session_id, "conv-abc");
        assert_eq!(records[0].key, records[0].dedup_key);
        assert_eq!(
            records[0].parent_session_id.as_deref(),
            Some("parent-conv-123")
        );
        assert_eq!(
            records[0].workspace_cwd.as_deref(),
            Some("/Users/test/repo")
        );
        assert_eq!(records[0].timestamp_ms, 1_760_000_123_456);
        assert!(records[0].timestamp.ends_with(".456Z"));
        assert_eq!(records[0].model, "gemini-3.8-flash-high");
        assert_eq!(records[0].usage.model, "gemini-3.8-flash-high");
        assert_eq!(records[0].input_tokens, 10_000);
        assert_eq!(records[0].output_tokens, 350);
        assert_eq!(records[0].completion_output_tokens, 300);
        assert_eq!(records[0].cache_read_tokens, 50_000);
        assert_eq!(records[0].usage.input, 10_000);
        assert_eq!(records[0].usage.output, 350);
        assert_eq!(records[0].reasoning_tokens, 50);
        assert_eq!(records[0].usage.cache_read, 50_000);
        assert_eq!(records[0].total_tokens_with_reasoning(), 60_350);

        // Second record from checkpoint f28.2 falls back to conversation f19 "gemini-3.8-flash"
        assert_eq!(records[1].timestamp_ms, 1_760_000_200_000);
        assert_eq!(records[1].usage.model, "gemini-3.8-flash");
        assert_eq!(records[1].usage.input, 2_500);
        assert_eq!(records[1].usage.output, 120);
        assert_eq!(records[1].usage.cache_read, 8_000);
    }

    #[test]
    fn record_key_remains_stable_across_later_blobs_and_in_place_growth() {
        let u1 = make_bot_usage_blob(1418, 100, 20, 400, 0, 0, None, None);
        let mut step1 = Vec::new();
        encode_bytes_field(1, &make_timestamp_blob(1_785_585_600, 0), &mut step1);
        encode_bytes_field(9, &u1, &mut step1);

        let first = parse_agy_conversation_blobs("conv-stable", &[], &[], &[(1, step1)]);
        assert_eq!(first.len(), 1);
        let initial_key = first[0].key.clone();
        assert_eq!(initial_key, "agy:conv-stable:step:1:f9");
        assert_eq!(first[0].output_tokens, 20);

        // Scan 2: step 1 gains bot_id ("bot-1") with unchanged usage; key must NOT flip to bot:bot-1.
        let u1_with_bot = make_bot_usage_blob(1418, 100, 20, 400, 0, 0, Some("bot-1"), None);
        let mut step1_with_bot = Vec::new();
        encode_bytes_field(
            1,
            &make_timestamp_blob(1_785_585_600, 0),
            &mut step1_with_bot,
        );
        encode_bytes_field(9, &u1_with_bot, &mut step1_with_bot);
        let with_bot =
            parse_agy_conversation_blobs("conv-stable", &[], &[], &[(1, step1_with_bot)]);
        assert_eq!(with_bot.len(), 1);
        assert_eq!(with_bot[0].key, initial_key);
        assert!(with_bot[0]
            .aliases
            .contains(&"agy:conv-stable:bot:bot-1".to_string()));

        // Scan 3: step 1 gains response_id ("resp-1") + grown usage; key must NOT flip to resp:resp-1.
        let u1_grown =
            make_bot_usage_blob(1418, 100, 35, 400, 5, 30, Some("bot-1"), Some("resp-1"));
        let mut step1_grown = Vec::new();
        encode_bytes_field(1, &make_timestamp_blob(1_785_585_600, 0), &mut step1_grown);
        encode_bytes_field(9, &u1_grown, &mut step1_grown);
        let with_resp =
            parse_agy_conversation_blobs("conv-stable", &[], &[], &[(1, step1_grown.clone())]);
        assert_eq!(with_resp.len(), 1);
        assert_eq!(with_resp[0].key, initial_key);
        assert!(with_resp[0]
            .aliases
            .contains(&"agy:conv-stable:resp:resp-1".to_string()));

        // Scan 4: a matching gen_metadata row (resp-1 / bot-1) AND an unrelated gen_metadata row (resp-0)
        // appear later; step 1 merges with the matching gen_metadata row and STILL keeps initial_key.
        let u_gen0 = make_bot_usage_blob(1418, 50, 10, 0, 0, 10, None, Some("resp-0"));
        let mut cm0 = Vec::new();
        encode_varint_field(3, 1418, &mut cm0);
        encode_bytes_field(4, &u_gen0, &mut cm0);
        let mut gen0 = Vec::new();
        encode_bytes_field(1, &cm0, &mut gen0);

        let u_gen1 = make_bot_usage_blob(1418, 100, 35, 400, 5, 30, Some("bot-1"), Some("resp-1"));
        let mut cm1 = Vec::new();
        encode_varint_field(3, 1418, &mut cm1);
        encode_bytes_field(4, &u_gen1, &mut cm1);
        let mut gen1 = Vec::new();
        encode_bytes_field(1, &cm1, &mut gen1);

        let fourth = parse_agy_conversation_blobs(
            "conv-stable",
            &[],
            &[(0, gen0), (1, gen1)],
            &[(1, step1_grown)],
        );
        assert_eq!(fourth.len(), 2);
        let updated_step1 = fourth.iter().find(|r| r.key == initial_key).expect(
            "step 1 key must remain stable when matching or earlier gen_metadata rows appear",
        );
        assert_eq!(updated_step1.output_tokens, 35);
        assert_eq!(updated_step1.completion_output_tokens, 30);
        assert_eq!(updated_step1.reasoning_tokens, 5);
        assert!(updated_step1
            .aliases
            .contains(&"agy:conv-stable:gen:1:f4".to_string()));
        assert!(updated_step1
            .aliases
            .contains(&"agy:conv-stable:resp:resp-1".to_string()));
    }

    #[test]
    fn ignores_truncated_or_zero_token_protobuf_blobs_safely() {
        let truncated = vec![0x0a, 0xff, 0x01, 0x02];
        let zero_usage = make_bot_usage_blob(1418, 0, 0, 0, 0, 0, Some("bot-0"), Some("resp-0"));
        let mut step_zero = Vec::new();
        encode_bytes_field(1, &make_timestamp_blob(1_760_000_000, 0), &mut step_zero);
        encode_bytes_field(9, &zero_usage, &mut step_zero);

        let records = parse_agy_conversation_blobs(
            "conv-bad",
            std::slice::from_ref(&truncated),
            &[(0, truncated.clone())],
            &[(1, truncated.clone()), (2, step_zero)],
        );
        assert!(records.is_empty());
    }

    #[test]
    fn parses_trajectory_lineage_parent_and_root() {
        let mut traj = Vec::new();
        encode_bytes_field(5, b"parent-uuid", &mut traj);
        encode_bytes_field(10, b"root-uuid", &mut traj);
        let (parent, root) = parse_agy_trajectory_lineage(&[traj]);
        assert_eq!(parent.as_deref(), Some("parent-uuid"));
        assert_eq!(root.as_deref(), Some("root-uuid"));
    }

    #[test]
    fn parses_jsonl_planner_response_fallback() {
        let line = r#"{"step_index":12,"type":"PLANNER_RESPONSE","status":"DONE","created_at":"2026-10-18T12:34:56Z","token_counts":{"input_tokens":1200,"output_tokens":340,"thinking_tokens":60,"cached_read_tokens":5000,"ram_cache_read_tokens":100}}"#;
        let parsed = parse_agy_transcript_line(line).expect("should parse PLANNER_RESPONSE");
        assert!(parsed.complete);
        assert_eq!(parsed.usage.input, 1200);
        assert_eq!(parsed.usage.output, 400);
        assert_eq!(parsed.usage.cache_read, 5100);
        assert_eq!(parsed.timestamp.as_deref(), Some("2026-10-18T12:34:56Z"));
    }
}
