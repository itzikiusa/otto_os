//! Gemini CLI (`gemini --acp`) sideband adapter (`chats/session-*.json`).
//!
//! Gemini rewrites per-session JSON snapshots under
//! `<gemini_home>/tmp/<project_slug_or_hash>/chats/session-*<sid_prefix8>*.json`.
//! While idle between prompts, [`GeminiState`] diffs newly added or grown
//! `"gemini"` messages, `thoughts`, `toolCalls`, and `tokens` written after
//! `idle_since_ms` (or after the baseline message set).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use agent_client_protocol::v1::schema::ToolCallStatus;
use otto_core::agent_runtime::TurnTokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::burst::{
    classify_tool, insert_bounded_map, insert_bounded_set, is_safe_session_id, now_ms,
    parse_rfc3339_ms, MirroredCounts,
};
use super::SidebandAction;
use crate::driver::SidebandSource;

const AWAITING_TOOL_MAX_MS: u64 = 90_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GeminiState {
    #[serde(default)]
    pub resolved_path: Option<PathBuf>,
    #[serde(default)]
    pub last_stamp: (u64, u64),
    #[serde(default)]
    pub seen_thought_count: HashMap<String, usize>,
    #[serde(default)]
    pub seen_msg_chars: HashMap<String, usize>,
    #[serde(default)]
    pub seen_tool_status: HashMap<String, String>,
    #[serde(default)]
    pub seen_tokens: HashSet<String>,
    #[serde(default)]
    pub prompted_msg_ids: HashSet<String>,
    #[serde(default)]
    pub open_tools: usize,
    #[serde(default)]
    pub awaiting_since_ms: u64,
}

impl GeminiState {
    pub fn reset_for_prompt(&mut self) {
        self.open_tools = 0;
    }

    pub fn on_turn_settled(&mut self) {}

    pub fn is_awaiting(&self) -> bool {
        self.open_tools > 0
            && now_ms().saturating_sub(self.awaiting_since_ms) < AWAITING_TOOL_MAX_MS
    }

    pub fn source(
        &self,
        gemini_home: &str,
        cwd: &str,
        session_id: &str,
        allow_scan: bool,
    ) -> Option<SidebandSource> {
        if gemini_home.is_empty() || !is_safe_session_id(session_id) {
            return None;
        }
        Some(SidebandSource::GeminiJson {
            gemini_home: PathBuf::from(gemini_home),
            cwd: cwd.to_string(),
            session_id: session_id.to_string(),
            resolved_path: self.resolved_path.clone(),
            last_stamp: self.last_stamp,
            allow_scan,
        })
    }

    pub fn parse_snapshot(
        &mut self,
        resolved_path: PathBuf,
        stamp: (u64, u64),
        doc: &Value,
        expected_session_id: Option<&str>,
        idle_since_ms: u64,
        mirrored: &mut MirroredCounts,
    ) -> Vec<SidebandAction> {
        self.resolved_path = Some(resolved_path);
        self.last_stamp = stamp;
        if let (Some(expected), Some(actual)) = (
            expected_session_id,
            doc.get("sessionId").and_then(Value::as_str),
        ) {
            if !expected.is_empty() && expected != actual {
                self.resolved_path = None;
                return Vec::new();
            }
        }
        let Some(messages) = doc.get("messages").and_then(Value::as_array) else {
            return Vec::new();
        };

        let mut actions = Vec::new();
        let mut active_open_tools = 0usize;
        let mut prompted_user_open = false;

        for (idx, msg) in messages.iter().enumerate() {
            let msg_type = msg.get("type").and_then(Value::as_str).unwrap_or("");
            let ts_ms = msg
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);
            if msg_type == "user" {
                prompted_user_open = idle_since_ms > 0
                    && ts_ms.is_none_or(|t| t <= idle_since_ms.saturating_add(250));
                continue;
            }
            if msg_type != "gemini" {
                if idle_since_ms > 0 && ts_ms.is_some_and(|t| t > idle_since_ms.saturating_add(250))
                {
                    prompted_user_open = false;
                }
                continue;
            }
            let msg_id = msg
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("gem-msg-{idx}"));

            let already_prompted = self.prompted_msg_ids.contains(&msg_id);
            if prompted_user_open
                && !already_prompted
                && idle_since_ms > 0
                && ts_ms.is_some_and(|t| t > idle_since_ms.saturating_add(1_000))
            {
                prompted_user_open = false;
            }

            let ts_baseline =
                idle_since_ms > 0 && ts_ms.is_some_and(|t| t <= idle_since_ms.saturating_add(50));
            let baseline_only = already_prompted || prompted_user_open || ts_baseline;
            if baseline_only {
                insert_bounded_set(&mut self.prompted_msg_ids, msg_id.clone());
            }

            self.ingest_thoughts(msg, &msg_id, baseline_only, mirrored, &mut actions);
            active_open_tools +=
                self.ingest_tools(msg, &msg_id, baseline_only, mirrored, &mut actions);
            self.ingest_content(msg, &msg_id, baseline_only, mirrored, &mut actions);
            self.ingest_tokens(msg, &msg_id, baseline_only, &mut actions);

            let has_tool_calls = msg
                .get("toolCalls")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty());
            if prompted_user_open
                && !has_tool_calls
                && !extract_gemini_message_text(msg.get("content")).is_empty()
            {
                prompted_user_open = false;
            }
        }

        self.open_tools = active_open_tools;
        if active_open_tools > 0 {
            self.awaiting_since_ms = now_ms();
        }
        actions
    }

    fn ingest_thoughts(
        &mut self,
        msg: &Value,
        msg_id: &str,
        baseline_only: bool,
        mirrored: &mut MirroredCounts,
        actions: &mut Vec<SidebandAction>,
    ) {
        let Some(thoughts) = msg.get("thoughts").and_then(Value::as_array) else {
            return;
        };
        let prev = self.seen_thought_count.get(msg_id).copied().unwrap_or(0);
        if thoughts.len() <= prev {
            return;
        }
        insert_bounded_map(
            &mut self.seen_thought_count,
            msg_id.to_string(),
            thoughts.len(),
        );
        if baseline_only {
            return;
        }
        for th in &thoughts[prev..] {
            let text = format_gemini_thought(th);
            if !text.is_empty() {
                mirrored.thought_chars =
                    mirrored.thought_chars.saturating_add(text.chars().count());
                actions.push(SidebandAction::Thought {
                    text,
                    streaming: false,
                });
            }
        }
    }

    fn ingest_tools(
        &mut self,
        msg: &Value,
        msg_id: &str,
        baseline_only: bool,
        mirrored: &mut MirroredCounts,
        actions: &mut Vec<SidebandAction>,
    ) -> usize {
        let Some(calls) = msg.get("toolCalls").and_then(Value::as_array) else {
            return 0;
        };
        let mut open = 0usize;
        for (i, call) in calls.iter().enumerate() {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| format!("{msg_id}-tool-{i}"));
            let raw_status = call
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let status = match raw_status {
                "pending" | "validating" | "scheduled" => ToolCallStatus::Pending,
                "executing" | "in_progress" | "running" => ToolCallStatus::InProgress,
                "error" | "failed" | "cancelled" => ToolCallStatus::Failed,
                _ => ToolCallStatus::Completed,
            };
            if !baseline_only
                && matches!(status, ToolCallStatus::Pending | ToolCallStatus::InProgress)
            {
                open += 1;
            }
            let prev = self.seen_tool_status.get(&id).cloned();
            if prev.as_deref() == Some(raw_status) {
                continue;
            }
            insert_bounded_map(
                &mut self.seen_tool_status,
                id.clone(),
                raw_status.to_string(),
            );
            if baseline_only {
                continue;
            }
            let name = call.get("name").and_then(Value::as_str).unwrap_or("tool");
            let args = call.get("args").cloned();
            let desc = classify_tool(name, args.as_ref());
            if prev.is_none() {
                mirrored.tool_calls = mirrored.tool_calls.saturating_add(1);
                mirrored.preseed_tool_id(&id);
                actions.push(SidebandAction::ToolStart {
                    id: id.clone(),
                    title: desc.title.clone(),
                    kind: desc.kind,
                    status: ToolCallStatus::InProgress,
                    raw_input: args.clone(),
                    file: desc.file.clone(),
                    diff: desc.diff.clone(),
                });
            }
            if matches!(status, ToolCallStatus::Completed | ToolCallStatus::Failed) {
                actions.push(SidebandAction::ToolUpdate {
                    id,
                    status,
                    title: None,
                    kind: None,
                    raw_input: None,
                    output: extract_gemini_tool_output(call),
                    file: None,
                    diff: None,
                });
            }
        }
        open
    }

    fn ingest_content(
        &mut self,
        msg: &Value,
        msg_id: &str,
        baseline_only: bool,
        mirrored: &mut MirroredCounts,
        actions: &mut Vec<SidebandAction>,
    ) {
        let full = extract_gemini_message_text(msg.get("content"));
        if full.is_empty() {
            return;
        }
        let total = full.chars().count();
        let prev = self.seen_msg_chars.get(msg_id).copied().unwrap_or(0);
        if total <= prev {
            return;
        }
        insert_bounded_map(&mut self.seen_msg_chars, msg_id.to_string(), total);
        if baseline_only {
            return;
        }
        let byte_idx = full
            .char_indices()
            .nth(prev)
            .map(|(i, _)| i)
            .unwrap_or(full.len());
        let delta = &full[byte_idx..];
        if !delta.is_empty() {
            mirrored.msg_chars = mirrored.msg_chars.saturating_add(delta.chars().count());
            actions.push(SidebandAction::Message {
                text: delta.to_string(),
                message_id: Some(msg_id.to_string()),
                streaming: false,
            });
        }
    }

    fn ingest_tokens(
        &mut self,
        msg: &Value,
        msg_id: &str,
        baseline_only: bool,
        actions: &mut Vec<SidebandAction>,
    ) {
        let Some(tok) = msg.get("tokens").and_then(Value::as_object) else {
            return;
        };
        if !insert_bounded_set(&mut self.seen_tokens, msg_id.to_string()) || baseline_only {
            return;
        }
        let n = |k: &str| tok.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = n("input");
        let cached = n("cached");
        let output = n("output");
        let thoughts = n("thoughts");
        if input > 0 || cached > 0 || output > 0 || thoughts > 0 {
            actions.push(SidebandAction::Usage(TurnTokenUsage {
                input_tokens: input.saturating_add(cached),
                output_tokens: output,
                cached_input_tokens: (cached > 0).then_some(cached),
                reasoning_tokens: (thoughts > 0).then_some(thoughts),
            }));
        }
    }
}

fn format_gemini_thought(th: &Value) -> String {
    if let Some(s) = th.as_str() {
        return s.to_string();
    }
    let subj = th
        .get("subject")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let desc = th
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    match (subj.is_empty(), desc.is_empty()) {
        (true, true) => String::new(),
        (false, true) => subj.to_string(),
        (true, false) => desc.to_string(),
        (false, false) => format!("{subj}: {desc}"),
    }
}

fn extract_gemini_message_text(content: Option<&Value>) -> String {
    let Some(c) = content else {
        return String::new();
    };
    if let Some(s) = c.as_str() {
        return s.to_string();
    }
    if let Some(arr) = c.as_array() {
        return arr
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("");
    }
    String::new()
}

fn extract_gemini_tool_output(call: &Value) -> Option<String> {
    if let Some(disp) = call
        .get("resultDisplay")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(disp.to_string());
    }
    let arr = call.get("result")?.as_array()?;
    for entry in arr {
        if let Some(out) = entry
            .pointer("/functionResponse/response/output")
            .or_else(|| entry.pointer("/functionResponse/response/error"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return Some(out.to_string());
        }
    }
    None
}

pub(crate) fn default_gemini_home() -> Option<String> {
    for var in ["OTTO_GEMINI_HOME", "GEMINI_CLI_HOME"] {
        if let Ok(dir) = std::env::var(var) {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    if otto_core::e2e::isolated() {
        return None;
    }
    let home = std::env::var("HOME").ok()?;
    let trimmed = home.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed).join(".gemini").display().to_string())
}
