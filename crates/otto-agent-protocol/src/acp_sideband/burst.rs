//! Shared buffered-burst deduplicator, tool classifier, and time/string helpers
//! for all ACP sideband adapters (`agy`, `grok`, `gemini`, `opencode`, and
//! custom ACP servers).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_client_protocol::v1::schema::ToolKind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const MAX_TRACKED_SIDEBAND_IDS: usize = 4096;

pub(crate) fn insert_bounded_map<V>(map: &mut HashMap<String, V>, key: String, val: V) {
    if !map.contains_key(&key) && map.len() >= MAX_TRACKED_SIDEBAND_IDS {
        let drop_n = MAX_TRACKED_SIDEBAND_IDS / 4;
        let victims: Vec<String> = map.keys().take(drop_n).cloned().collect();
        for k in victims {
            map.remove(&k);
        }
    }
    map.insert(key, val);
}

pub(crate) fn insert_bounded_set(set: &mut HashSet<String>, key: String) -> bool {
    if set.contains(&key) {
        return false;
    }
    if set.len() >= MAX_TRACKED_SIDEBAND_IDS {
        let drop_n = MAX_TRACKED_SIDEBAND_IDS / 4;
        let victims: Vec<String> = set.iter().take(drop_n).cloned().collect();
        for k in victims {
            set.remove(&k);
        }
    }
    set.insert(key)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct BurstFilter {
    #[serde(default)]
    pub remaining_thought_chars: usize,
    #[serde(default)]
    pub remaining_msg_chars: usize,
    #[serde(default)]
    pub remaining_tool_calls: usize,
    #[serde(default)]
    pub preseeded_tool_ids: HashSet<String>,
    #[serde(default)]
    pub suppressed_tool_ids: HashSet<String>,
    #[serde(default)]
    pub armed_from_mirrored: bool,
}

impl BurstFilter {
    pub fn is_draining_pre_prompt(&self) -> bool {
        self.remaining_tool_calls > 0
            || self.remaining_thought_chars > 0
            || self.remaining_msg_chars > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MirroredCounts {
    #[serde(default)]
    pub thought_chars: usize,
    #[serde(default)]
    pub msg_chars: usize,
    #[serde(default)]
    pub tool_calls: usize,
    #[serde(default)]
    pub tool_ids: HashSet<String>,
}

impl MirroredCounts {
    pub fn has_any(&self) -> bool {
        self.thought_chars > 0
            || self.msg_chars > 0
            || self.tool_calls > 0
            || !self.tool_ids.is_empty()
    }

    pub fn preseed_tool_id(&mut self, id: &str) {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            insert_bounded_set(&mut self.tool_ids, trimmed.to_string());
        }
    }

    pub fn transfer_into(&mut self, burst: &mut BurstFilter) {
        if self.has_any() {
            burst.armed_from_mirrored = true;
        }
        burst.remaining_thought_chars = burst
            .remaining_thought_chars
            .saturating_add(std::mem::take(&mut self.thought_chars));
        burst.remaining_msg_chars = burst
            .remaining_msg_chars
            .saturating_add(std::mem::take(&mut self.msg_chars));
        burst.remaining_tool_calls = burst
            .remaining_tool_calls
            .saturating_add(std::mem::take(&mut self.tool_calls));
        for id in std::mem::take(&mut self.tool_ids) {
            insert_bounded_set(&mut burst.preseeded_tool_ids, id);
        }
    }
}

pub(crate) fn consume_budget(budget: &mut usize, text: &str) -> Option<String> {
    if *budget == 0 || text.is_empty() {
        return (!text.is_empty()).then(|| text.to_string());
    }
    let total = text.chars().count();
    if total <= *budget {
        *budget -= total;
        return None;
    }
    let skip = *budget;
    *budget = 0;
    let byte_idx = text
        .char_indices()
        .nth(skip)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    Some(text[byte_idx..].to_string())
}

pub(crate) fn is_safe_session_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn parse_rfc3339_ms(raw: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .and_then(|dt| u64::try_from(dt.timestamp_millis()).ok())
}

/// Upper-bound millisecond timestamp for RFC3339 strings that omit fractional
/// seconds (e.g. `agy` writes `2026-11-15T11:08:48Z` for events up to `.999Z`).
pub(crate) fn parse_rfc3339_end_ms(raw: &str) -> Option<u64> {
    let trimmed = raw.trim();
    let base = parse_rfc3339_ms(trimmed)?;
    let has_subsec = trimmed
        .split_once('T')
        .or_else(|| trimmed.split_once('t'))
        .is_some_and(|(_, time_part)| time_part.contains('.'));
    Some(if has_subsec {
        base
    } else {
        base.saturating_add(999)
    })
}

pub(crate) fn trim_ascii(mut b: &[u8]) -> &[u8] {
    while let Some((first, rest)) = b.split_first() {
        if first.is_ascii_whitespace() {
            b = rest;
        } else {
            break;
        }
    }
    while let Some((last, rest)) = b.split_last() {
        if last.is_ascii_whitespace() {
            b = rest;
        } else {
            break;
        }
    }
    b
}

pub(crate) fn unquote_args(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            for (k, val) in map {
                let decoded = match val {
                    Value::String(s)
                        if (s.starts_with('"') && s.ends_with('"'))
                            || (s.starts_with('{') && s.ends_with('}'))
                            || (s.starts_with('[') && s.ends_with(']'))
                            || s == "true"
                            || s == "false" =>
                    {
                        serde_json::from_str::<Value>(s).unwrap_or_else(|_| val.clone())
                    }
                    _ => val.clone(),
                };
                out.insert(k.clone(), decoded);
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

pub(crate) fn arg_str<'a>(args: Option<&'a Value>, keys: &[&str]) -> Option<&'a str> {
    let args = args?;
    keys.iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

pub(crate) struct ClassifiedTool {
    pub kind: ToolKind,
    pub title: String,
    pub file: Option<String>,
    pub diff: Option<(Option<String>, String)>,
}

/// Normalize tool calls across Agy, Grok, Gemini, and OpenCode into ACP
/// [`ToolKind`] + human-readable title + optional file/diff metadata.
pub(crate) fn classify_tool(raw_name: &str, args: Option<&Value>) -> ClassifiedTool {
    let bare = raw_name.strip_prefix("default_api:").unwrap_or(raw_name);
    let lower = bare.to_ascii_lowercase();
    match lower.as_str() {
        "run_command" | "run_shell_command" | "bash" | "shell" | "execute_command" | "terminal" => {
            let title = arg_str(
                args,
                &[
                    "toolSummary",
                    "description",
                    "CommandLine",
                    "command",
                    "cmd",
                ],
            )
            .unwrap_or(bare)
            .to_string();
            ClassifiedTool {
                kind: ToolKind::Execute,
                title,
                file: None,
                diff: None,
            }
        }
        "edit_file"
        | "replace_file_content"
        | "edit"
        | "replace"
        | "str_replace_editor"
        | "patch"
        | "apply_patch" => {
            let path = arg_str(
                args,
                &[
                    "Path",
                    "TargetFile",
                    "path",
                    "file_path",
                    "filePath",
                    "target_file",
                ],
            )
            .map(str::to_string);
            let title = path
                .as_deref()
                .map(|p| format!("Edit {p}"))
                .unwrap_or_else(|| format!("Run {bare}"));
            let old = arg_str(
                args,
                &[
                    "Target",
                    "TargetContent",
                    "old_string",
                    "oldString",
                    "old_str",
                ],
            )
            .map(str::to_string);
            let new = arg_str(
                args,
                &[
                    "Replacement",
                    "ReplacementContent",
                    "new_string",
                    "newString",
                    "new_str",
                ],
            )
            .map(str::to_string);
            let diff = new.map(|n| (old, n));
            ClassifiedTool {
                kind: ToolKind::Edit,
                title,
                file: path,
                diff,
            }
        }
        "write_file" | "write_to_file" | "write" | "create_file" => {
            let path = arg_str(
                args,
                &[
                    "Path",
                    "TargetFile",
                    "path",
                    "file_path",
                    "filePath",
                    "target_file",
                ],
            )
            .map(str::to_string);
            let title = path
                .as_deref()
                .map(|p| format!("Write {p}"))
                .unwrap_or_else(|| format!("Run {bare}"));
            let diff = arg_str(args, &["Content", "CodeContent", "content"])
                .map(|c| (None, c.to_string()));
            ClassifiedTool {
                kind: ToolKind::Edit,
                title,
                file: path,
                diff,
            }
        }
        "multi_replace_file_content" | "multiedit" => {
            let path = arg_str(
                args,
                &["TargetFile", "Path", "path", "file_path", "filePath"],
            )
            .map(str::to_string);
            let title = path
                .as_deref()
                .map(|p| format!("Edit {p}"))
                .unwrap_or_else(|| format!("Run {bare}"));
            ClassifiedTool {
                kind: ToolKind::Edit,
                title,
                file: path,
                diff: None,
            }
        }
        "read_file" | "view_file" | "view_file_outline" | "read" | "read_many_files" => {
            let path = arg_str(
                args,
                &[
                    "Path",
                    "AbsolutePath",
                    "path",
                    "file_path",
                    "filePath",
                    "target_file",
                ],
            )
            .map(str::to_string);
            let title = path
                .as_deref()
                .map(|p| format!("File {p}"))
                .unwrap_or_else(|| format!("Run {bare}"));
            ClassifiedTool {
                kind: ToolKind::Read,
                title,
                file: path,
                diff: None,
            }
        }
        "grep_search"
        | "find_by_name"
        | "codebase_search"
        | "search_web"
        | "grep"
        | "glob"
        | "search_file_content"
        | "web_search"
        | "google_web_search" => {
            let q = arg_str(args, &["Query", "Pattern", "query", "pattern"]);
            let title = match q {
                Some(q) => format!("Search {q}"),
                None => format!("Run {bare}"),
            };
            ClassifiedTool {
                kind: ToolKind::Search,
                title,
                file: None,
                diff: None,
            }
        }
        "schedule" => ClassifiedTool {
            kind: ToolKind::Other,
            title: "schedule".to_string(),
            file: None,
            diff: None,
        },
        _ => ClassifiedTool {
            kind: ToolKind::Other,
            title: format!("Run {bare}"),
            file: None,
            diff: None,
        },
    }
}
