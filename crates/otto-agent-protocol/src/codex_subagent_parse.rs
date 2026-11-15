//! Pure parsing helpers for Codex collaboration/subagent items (`collabAgentToolCall`).

use std::collections::HashMap;

use otto_core::agent_runtime::{TurnItem, TurnItemKind, TurnItemStatus};
use serde_json::Value;

#[derive(Debug, Clone)]
pub(super) struct ParsedAgentState {
    pub status: Option<TurnItemStatus>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CollabKind {
    Spawn,
    SendInput,
    Wait,
    Close,
    Resume,
}

pub(super) fn normalize_collab_tool(raw: &str) -> CollabKind {
    match raw {
        "sendInput" | "send_input" => CollabKind::SendInput,
        "wait" | "waitForAgents" | "wait_for_agents" => CollabKind::Wait,
        "closeAgent" | "close_agent" => CollabKind::Close,
        "resumeAgent" | "resume_agent" => CollabKind::Resume,
        _ => CollabKind::Spawn,
    }
}

pub(super) fn item_status_is_terminal(it: &Value) -> bool {
    matches!(
        it.get("status").and_then(Value::as_str),
        Some("completed" | "failed" | "declined")
    )
}

pub(super) fn str_any(v: &Value, keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some(s) = v
            .get(*k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Some(s.to_string());
        }
    }
    None
}

pub(super) fn first_line_label(s: &str) -> Option<String> {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out: String = line.chars().take(80).collect();
    if line.chars().count() > 80 {
        out.push('…');
    }
    Some(out)
}

pub(super) fn extract_target_thread_ids(it: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let mut push_unique = |s: &str| {
        let trimmed = s.trim();
        if !trimmed.is_empty() && !out.iter().any(|e| e == trimmed) {
            out.push(trimmed.to_string());
        }
    };
    for key in [
        "newThreadId",
        "new_thread_id",
        "agentThreadId",
        "agent_thread_id",
        "receiverThreadId",
        "receiver_thread_id",
    ] {
        if let Some(s) = it.get(key).and_then(Value::as_str) {
            push_unique(s);
        }
    }
    for key in ["receiverThreadIds", "receiver_thread_ids"] {
        if let Some(arr) = it.get(key).and_then(Value::as_array) {
            for v in arr.iter().filter_map(Value::as_str) {
                push_unique(v);
            }
        }
    }
    for key in ["agentsStates", "agents_states"] {
        if let Some(obj) = it.get(key).and_then(Value::as_object) {
            let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
            keys.sort_unstable();
            for k in keys {
                push_unique(k);
            }
        }
    }
    out
}

pub(super) fn extract_agent_states(it: &Value) -> HashMap<String, ParsedAgentState> {
    let mut out = HashMap::new();
    for key in ["agentsStates", "agents_states"] {
        let Some(obj) = it.get(key).and_then(Value::as_object) else {
            continue;
        };
        for (tid, val) in obj {
            out.insert(tid.clone(), parse_agent_state(val));
        }
    }
    out
}

fn parse_agent_state(val: &Value) -> ParsedAgentState {
    if let Some(s) = val.as_str() {
        return ParsedAgentState {
            status: parse_status_str(s),
            message: None,
        };
    }
    let Some(obj) = val.as_object() else {
        return ParsedAgentState {
            status: None,
            message: None,
        };
    };
    let status = obj
        .get("status")
        .or_else(|| obj.get("state"))
        .and_then(Value::as_str)
        .and_then(parse_status_str)
        .or_else(|| {
            if obj.contains_key("completed") {
                Some(TurnItemStatus::Completed)
            } else if obj.contains_key("errored") || obj.contains_key("failed") {
                Some(TurnItemStatus::Failed)
            } else if obj.contains_key("shutdown") || obj.contains_key("interrupted") {
                Some(TurnItemStatus::Cancelled)
            } else {
                None
            }
        });
    let message = str_any(
        val,
        &[
            "message",
            "output",
            "finalOutput",
            "final_output",
            "error",
            "completed",
            "errored",
        ],
    )
    .or_else(|| {
        obj.get("completed")
            .or_else(|| obj.get("errored"))
            .and_then(|nested| str_any(nested, &["message", "output", "error"]))
    });
    ParsedAgentState { status, message }
}

pub(super) fn parse_status_str(s: &str) -> Option<TurnItemStatus> {
    match s {
        "pendingInit" | "pending_init" | "pending" | "running" | "inProgress" | "in_progress" => {
            Some(TurnItemStatus::Running)
        }
        "completed" | "done" | "succeeded" => Some(TurnItemStatus::Completed),
        "errored" | "failed" | "error" | "notFound" | "not_found" => Some(TurnItemStatus::Failed),
        "shutdown" | "closed" | "interrupted" | "cancelled" | "declined" => {
            Some(TurnItemStatus::Cancelled)
        }
        _ => None,
    }
}

pub(super) fn extract_collab_error(it: &Value) -> Option<String> {
    match it.get("error") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(v @ Value::Object(_)) => str_any(v, &["message", "error"]),
        _ => None,
    }
}

pub(super) fn child_tool_label(raw_item: &Value, ti: &TurnItem) -> Option<String> {
    match &ti.kind {
        TurnItemKind::CommandExecution { input, .. } => {
            first_line_label(input).or_else(|| Some("Command".into()))
        }
        TurnItemKind::FileChange { file_name, .. } => Some(format!("Edit {file_name}")),
        TurnItemKind::DynamicTool { tool_name, .. } => Some(tool_name.clone()),
        TurnItemKind::WebSearch { patterns } => patterns
            .first()
            .map(|q| format!("Search {q}"))
            .or_else(|| Some("Web search".into())),
        _ => raw_item
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}
