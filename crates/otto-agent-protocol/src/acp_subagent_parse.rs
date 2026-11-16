//! Pure parsing and lexical path-confinement helpers for ACP subagent promotion,
//! child routing, and background log/transcript discovery.

use std::path::PathBuf;

use serde_json::Value;

use crate::acp_sideband::confinement::{
    default_agy_brain_path, is_clean_absolute_path, is_forbidden_tail_target,
};

pub(crate) type AgySubagentLogPaths = (PathBuf, Option<PathBuf>);

pub(crate) fn canonical_subagent_tool_name(
    title: &str,
    raw_input: Option<&Value>,
) -> Option<String> {
    let norm = normalize_tool_name(title);
    if norm.contains("invoke_subagent") {
        return Some("invoke_subagent".into());
    }
    if norm.contains("delegate_to_agent") {
        return Some("delegate_to_agent".into());
    }
    if norm.contains("spawn_subagent") {
        return Some("spawn_subagent".into());
    }
    if norm.contains("spawn_agent") {
        return Some("spawn_agent".into());
    }
    if norm == "invoke_agent" || norm.ends_with("_invoke_agent") {
        return Some("invoke_agent".into());
    }
    if norm == "codebase_investigator" {
        return Some("codebase_investigator".into());
    }
    if norm == "cli_help" {
        return Some("cli_help".into());
    }
    if norm == "subagent" {
        return Some("subagent".into());
    }
    if norm == "task"
        && raw_input.is_some_and(|v| {
            v.get("subagent_type").is_some()
                || v.get("TypeName").is_some()
                || v.get("agent_type").is_some()
                || v.get("Role").is_some()
                || v.get("prompt").is_some()
                || v.get("Prompt").is_some()
                || v.get("description").is_some()
                || v.get("objective").is_some()
        })
    {
        return Some("task".into());
    }
    None
}

pub(super) fn normalize_tool_name(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("Run ") {
        s = rest.trim();
    }
    if let Some((_, tail)) = s.rsplit_once(':') {
        s = tail.trim();
    }
    if let Some((_, tail)) = s.rsplit_once('.') {
        if !tail.contains('/') && !tail.contains(' ') {
            s = tail.trim();
        }
    }
    s.to_ascii_lowercase()
}

pub(super) fn short_tool_label(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some((_, tail)) = s.rsplit_once(':') {
        s = tail.trim();
    }
    s.to_string()
}

pub(super) fn extract_subagent_fields(
    raw_input: Option<&Value>,
    title: &str,
    canonical: &str,
    meta: Option<&Value>,
    raw_output: Option<&Value>,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    bool,
    Option<String>,
) {
    let str_in = |keys: &[&str]| -> Option<String> {
        let v = raw_input?;
        keys.iter().find_map(|k| {
            v.get(*k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
    };
    let description =
        str_in(&["Role", "Description", "description", "AgentName", "name"]).or_else(|| {
            let trimmed = title.trim();
            let lower = normalize_tool_name(trimmed);
            (!trimmed.is_empty() && lower != canonical && lower != "tool")
                .then(|| trimmed.to_string())
        });
    let subagent_type = str_in(&["TypeName", "subagent_type", "agent_type"]).or_else(|| {
        matches!(canonical, "codebase_investigator" | "cli_help").then(|| canonical.to_string())
    });
    let prompt = str_in(&["Prompt", "prompt", "task", "objective", "question"]);
    let background = raw_input.is_some_and(|v| {
        ["run_in_background", "background", "async"]
            .iter()
            .any(|k| v.get(*k).and_then(Value::as_bool) == Some(true))
    });
    let child_sid = [meta, raw_output, raw_input]
        .into_iter()
        .flatten()
        .find_map(|v| {
            [
                "childSessionId",
                "child_session_id",
                "subagentSessionId",
                "session_id",
            ]
            .iter()
            .find_map(|k| {
                v.get(*k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
        });
    (description, subagent_type, prompt, background, child_sid)
}

pub(crate) fn extract_parent_tool_id(params: &Value, up: Option<&Value>) -> Option<String> {
    const POINTERS: &[&str] = &[
        "/_meta/parentToolUseId",
        "/_meta/parent_tool_use_id",
        "/_meta/parentToolCallId",
        "/_meta/parent_tool_call_id",
        "/_meta/parentId",
        "/parentToolUseId",
        "/parent_tool_use_id",
        "/parentToolCallId",
        "/parent_tool_call_id",
    ];
    for ptr in POINTERS {
        if let Some(u) = up {
            if let Some(s) = u
                .pointer(ptr)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(s.to_string());
            }
        }
        if let Some(s) = params
            .pointer(ptr)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

pub(super) fn parse_agy_subagent_output(
    output: &str,
    brain_root: Option<&str>,
) -> Option<(String, Option<AgySubagentLogPaths>)> {
    let trimmed = output.trim();
    if trimmed.is_empty() || !trimmed.contains("conversationId") {
        return None;
    }
    let parsed = parse_embedded_json(trimmed);
    let conv_id = parsed
        .as_ref()
        .and_then(|v| {
            v.get("conversationId")
                .or_else(|| v.get("conversation_id"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| extract_json_string_field(trimmed, "conversationId"))?;

    let log_uri = parsed
        .as_ref()
        .and_then(|v| {
            v.get("logAbsoluteUri")
                .or_else(|| v.get("log_absolute_uri"))
                .or_else(|| v.get("transcriptPath"))
                .and_then(Value::as_str)
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| extract_json_string_field(trimmed, "logAbsoluteUri"));

    let paths = resolve_agy_transcript_paths(&conv_id, log_uri.as_deref(), brain_root);
    Some((conv_id, paths))
}

/// Strictly confine `agy` child subagent transcript paths to
/// `<brain_root>/<safe_conv_id>/.system_generated/logs/transcript[_full].jsonl`.
pub(crate) fn resolve_agy_transcript_paths(
    conv_id: &str,
    log_uri: Option<&str>,
    brain_root: Option<&str>,
) -> Option<AgySubagentLogPaths> {
    let safe_conv = otto_core::paths::safe_component(conv_id.trim())?;
    let default_root = default_agy_brain_path();
    let root_path = match brain_root.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => PathBuf::from(r),
        None => default_root?,
    };
    if !is_clean_absolute_path(&root_path) || is_forbidden_tail_target(&root_path) {
        return None;
    }
    let conv_dir = root_path.join(safe_conv);
    let expected_logs = conv_dir.join(".system_generated").join("logs");

    if let Some(uri) = log_uri.map(str::trim).filter(|s| !s.is_empty()) {
        let p = file_uri_to_path(uri)?;
        if !is_clean_absolute_path(&p) || is_forbidden_tail_target(&p) {
            return None;
        }
        let allowed = p == expected_logs.join("transcript.jsonl")
            || p == expected_logs.join("transcript_full.jsonl")
            || p == expected_logs
            || p == conv_dir;
        if !allowed {
            return None;
        }
    }

    Some((
        expected_logs.join("transcript_full.jsonl"),
        Some(expected_logs.join("transcript.jsonl")),
    ))
}

pub(crate) fn parse_grok_bg_launch(output: &str) -> Option<(String, bool)> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(v) = parse_embedded_json(trimmed) {
        for key in ["subagent_id", "subagentId", "agent_id", "agentId"] {
            if let Some(id) = v
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some((id.to_string(), true));
            }
        }
        let is_bg_event = v
            .get("type")
            .or_else(|| v.get("event"))
            .or_else(|| v.get("status"))
            .and_then(Value::as_str)
            .is_some_and(|s| s.eq_ignore_ascii_case("BackgroundTaskStarted"))
            || trimmed.contains("BackgroundTaskStarted");
        if is_bg_event {
            for key in ["task_id", "taskId", "command_id", "commandId", "id"] {
                if let Some(id) = v
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return Some((id.to_string(), false));
                }
            }
        }
    }

    if let Some((_, rest)) = split_once_ignore_ascii_case(trimmed, "agent id:") {
        if let Some(tok) = take_id_token(rest) {
            return Some((tok, true));
        }
    }
    if let Some(id) = extract_json_string_field(trimmed, "subagent_id") {
        return Some((id, true));
    }
    if let Some((_, rest)) = trimmed.split_once("BackgroundTaskStarted") {
        let rest = rest.trim_start_matches([':', ' ', '-', '(']);
        if let Some(tok) = take_id_token(rest) {
            return Some((tok, false));
        }
    }
    None
}

pub(crate) fn file_uri_to_path(raw: &str) -> Option<PathBuf> {
    let s = raw.trim().trim_matches(['"', '\'', ')', '>', ',']);
    let p = if s.starts_with('/') {
        PathBuf::from(s)
    } else {
        otto_core::paths::file_uri_to_path(s)?
    };
    is_clean_absolute_path(&p).then_some(p)
}

pub(crate) fn parse_embedded_json(s: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    (end > start)
        .then(|| serde_json::from_str::<Value>(&s[start..=end]).ok())
        .flatten()
}

fn extract_json_string_field(s: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let idx = s.find(&needle)?;
    let rest = s[idx + needle.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let val = rest[..end].trim();
    (!val.is_empty()).then(|| val.to_string())
}

fn split_once_ignore_ascii_case<'a>(s: &'a str, pat_lower: &str) -> Option<(&'a str, &'a str)> {
    let lower = s.to_ascii_lowercase();
    let idx = lower.find(pat_lower)?;
    Some((&s[..idx], &s[idx + pat_lower.len()..]))
}

pub(crate) fn take_id_token(s: &str) -> Option<String> {
    let tok = s
        .split_whitespace()
        .next()?
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_' | '/'));
    (!tok.is_empty()).then(|| tok.to_string())
}

pub(super) fn non_empty_trimmed(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| s.to_string())
}

pub(super) fn parse_command_log_exit_code(text: &str) -> Option<i32> {
    for marker in ["Command exited with code ", "[Process exited with code "] {
        if let Some((_, rest)) = text.rsplit_once(marker) {
            let digits: String = rest
                .trim_start()
                .chars()
                .take_while(|c| *c == '-' || c.is_ascii_digit())
                .collect();
            if let Ok(code) = digits.parse::<i32>() {
                return Some(code);
            }
        }
    }
    None
}
