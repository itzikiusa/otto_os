//! Grok (`grok … agent stdio`) & generic ACP `updates.jsonl` sideband adapter.
//!
//! Grok appends real-time ACP `session/update` envelopes to
//! `<grok_home>/sessions/<percent_encoded_cwd>/<session_id>/updates.jsonl`.
//! Custom ACP servers can also point `OTTO_ACP_SIDEBAND_FILE` at an ACP
//! `session/update` JSONL stream and share the exact same parser + burst filter.

use std::path::PathBuf;

use agent_client_protocol::v1::schema::{ToolCallStatus, ToolKind};
use otto_core::agent_runtime::{PlanStep, PlanStepStatus, TurnTokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::burst::{classify_tool, is_safe_session_id, now_ms, trim_ascii, MirroredCounts};
use super::SidebandAction;
use crate::driver::SidebandSource;

const AWAITING_TOOL_MAX_MS: u64 = 90_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GrokState {
    #[serde(default)]
    pub resolved_path: Option<PathBuf>,
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub in_prompted_turn: bool,
    #[serde(default)]
    pub prompted_turn_baseline: bool,
    #[serde(default)]
    pub open_tools: usize,
    #[serde(default)]
    pub awaiting_since_ms: u64,
}

impl GrokState {
    pub fn reset_for_prompt(&mut self) {
        self.in_prompted_turn = false;
        self.prompted_turn_baseline = false;
        self.open_tools = 0;
    }

    pub fn on_turn_settled(&mut self) {
        self.open_tools = 0;
    }

    pub fn is_awaiting(&self) -> bool {
        self.open_tools > 0
            && now_ms().saturating_sub(self.awaiting_since_ms) < AWAITING_TOOL_MAX_MS
    }

    pub fn source_for_custom_file(&self, file_path: &str) -> Option<SidebandSource> {
        let trimmed = file_path.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(SidebandSource::AppendLines {
            path: PathBuf::from(trimmed),
            fallback_path: None,
            scan_sessions_dir: None,
            offset: self.offset,
            using_fallback: false,
        })
    }

    pub fn source_for_grok(
        &self,
        grok_home: &str,
        cwd: &str,
        session_id: &str,
        allow_scan: bool,
    ) -> Option<SidebandSource> {
        if grok_home.is_empty() || !is_safe_session_id(session_id) {
            return None;
        }
        if let Some(resolved) = &self.resolved_path {
            return Some(SidebandSource::AppendLines {
                path: resolved.clone(),
                fallback_path: None,
                scan_sessions_dir: None,
                offset: self.offset,
                using_fallback: false,
            });
        }
        let sessions_dir = PathBuf::from(grok_home).join("sessions");
        let encoded_cwd = percent_encode_cwd(cwd);
        let primary = sessions_dir
            .join(encoded_cwd)
            .join(session_id)
            .join("updates.jsonl");
        Some(SidebandSource::AppendLines {
            path: primary,
            fallback_path: None,
            scan_sessions_dir: allow_scan.then(|| (sessions_dir, session_id.to_string())),
            offset: self.offset,
            using_fallback: false,
        })
    }

    pub fn parse_bytes(
        &mut self,
        resolved_path: Option<PathBuf>,
        next_offset: u64,
        bytes: &[u8],
        root_session_id: Option<&str>,
        idle_since_ms: u64,
        mirrored: &mut MirroredCounts,
    ) -> Vec<SidebandAction> {
        if let Some(p) = resolved_path {
            self.resolved_path = Some(p);
        }
        self.offset = next_offset;
        let mut actions = Vec::new();

        for raw in bytes.split(|b| *b == b'\n') {
            let line = trim_ascii(raw);
            if line.is_empty() {
                continue;
            }
            let Ok(root) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let ts_ms = envelope_timestamp_ms(&root);
            let baseline_only = is_before_idle(ts_ms, idle_since_ms);
            let params = root.get("params").unwrap_or(&root);
            let Some(up) = params.get("update") else {
                continue;
            };
            let Some(tag) = up.get("sessionUpdate").and_then(Value::as_str) else {
                continue;
            };
            let line_sid = params
                .get("sessionId")
                .or_else(|| root.get("sessionId"))
                .or_else(|| root.get("session_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let parent_sid = params
                .get("parentSessionId")
                .or_else(|| params.get("parent_session_id"))
                .or_else(|| root.get("parent_session_id"))
                .or_else(|| up.get("parent_session_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let parent_cid = crate::acp::subagent::parse::extract_parent_tool_id(params, Some(up));
            let is_child = parent_cid.is_some()
                || parent_sid.is_some_and(|ps| root_session_id.is_none_or(|r| ps == r))
                || matches!((line_sid, root_session_id), (Some(ls), Some(r)) if ls != r);
            let child_sid = is_child.then(|| line_sid.or(parent_sid)).flatten();

            if !is_child {
                match tag {
                    "user_message_chunk" | "user_message" => {
                        self.in_prompted_turn = true;
                        self.prompted_turn_baseline = baseline_only;
                        self.open_tools = 0;
                        continue;
                    }
                    "turn_completed" => {
                        let was_prompted = std::mem::replace(&mut self.in_prompted_turn, false);
                        self.prompted_turn_baseline = false;
                        self.open_tools = 0;
                        if was_prompted || baseline_only {
                            continue;
                        }
                        if let Some(usage) = parse_grok_usage(up.get("usage")) {
                            actions.push(SidebandAction::Usage(usage));
                        }
                        let reason = up
                            .get("stop_reason")
                            .or_else(|| up.get("stopReason"))
                            .and_then(Value::as_str)
                            .filter(|r| *r != "end_turn")
                            .map(str::to_string);
                        actions.push(SidebandAction::TurnCompleted { reason });
                        continue;
                    }
                    _ => {}
                }

                if !baseline_only && self.prompted_turn_baseline {
                    self.in_prompted_turn = false;
                    self.prompted_turn_baseline = false;
                }

                if self.in_prompted_turn || baseline_only {
                    continue;
                }
            } else if baseline_only
                || matches!(
                    tag,
                    "user_message_chunk" | "user_message" | "turn_completed"
                )
            {
                continue;
            }

            let before_len = actions.len();
            match tag {
                "agent_thought_chunk" | "agent_thought" => {
                    if let Some(text) = extract_update_text(up).filter(|s| !s.is_empty()) {
                        if !is_child {
                            mirrored.thought_chars =
                                mirrored.thought_chars.saturating_add(text.chars().count());
                        }
                        actions.push(SidebandAction::Thought {
                            text,
                            streaming: true,
                        });
                    }
                }
                "agent_message_chunk" | "agent_message" => {
                    if let Some(text) = extract_update_text(up).filter(|s| !s.is_empty()) {
                        if !is_child {
                            mirrored.msg_chars =
                                mirrored.msg_chars.saturating_add(text.chars().count());
                        }
                        let mid = up
                            .get("messageId")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        actions.push(SidebandAction::Message {
                            text,
                            message_id: mid,
                            streaming: true,
                        });
                    }
                }
                "tool_call" => {
                    let Some(id) = up.get("toolCallId").and_then(Value::as_str) else {
                        continue;
                    };
                    if !is_child {
                        mirrored.tool_calls = mirrored.tool_calls.saturating_add(1);
                        mirrored.preseed_tool_id(id);
                    }
                    let raw_title = up.get("title").and_then(Value::as_str).unwrap_or("tool");
                    let raw_input = up.get("rawInput").cloned();
                    let classified = classify_tool(raw_title, raw_input.as_ref());
                    let kind = up
                        .get("kind")
                        .and_then(|v| serde_json::from_value::<ToolKind>(v.clone()).ok())
                        .unwrap_or(classified.kind);
                    let status = parse_raw_tool_status(up.get("status"))
                        .unwrap_or(ToolCallStatus::InProgress);
                    if !is_child
                        && matches!(status, ToolCallStatus::Pending | ToolCallStatus::InProgress)
                    {
                        self.open_tools = self.open_tools.saturating_add(1);
                        self.awaiting_since_ms = now_ms();
                    }
                    actions.push(SidebandAction::ToolStart {
                        id: id.to_string(),
                        title: classified.title,
                        kind,
                        status,
                        raw_input,
                        file: extract_location_path(up).or(classified.file),
                        diff: extract_diff(up).or(classified.diff),
                    });
                }
                "tool_call_update" => {
                    let Some(id) = up.get("toolCallId").and_then(Value::as_str) else {
                        continue;
                    };
                    let status = parse_raw_tool_status(up.get("status"))
                        .unwrap_or(ToolCallStatus::Completed);
                    if !is_child {
                        if matches!(status, ToolCallStatus::Completed | ToolCallStatus::Failed) {
                            self.open_tools = self.open_tools.saturating_sub(1);
                        }
                        self.awaiting_since_ms = now_ms();
                    }
                    let kind = up
                        .get("kind")
                        .and_then(|v| serde_json::from_value::<ToolKind>(v.clone()).ok());
                    actions.push(SidebandAction::ToolUpdate {
                        id: id.to_string(),
                        status,
                        title: up.get("title").and_then(Value::as_str).map(str::to_string),
                        kind,
                        raw_input: up.get("rawInput").cloned(),
                        output: extract_tool_output(up),
                        file: extract_location_path(up),
                        diff: extract_diff(up),
                    });
                }
                "plan" => {
                    if let Some(entries) = up.get("entries").and_then(Value::as_array) {
                        let steps = entries
                            .iter()
                            .enumerate()
                            .filter_map(|(i, e)| {
                                let text = e.get("content")?.as_str()?.to_string();
                                let status = match e.get("status").and_then(Value::as_str) {
                                    Some("in_progress") => PlanStepStatus::Running,
                                    Some("completed") => PlanStepStatus::Completed,
                                    _ => PlanStepStatus::Pending,
                                };
                                Some(PlanStep {
                                    id: (i + 1).to_string(),
                                    text,
                                    status,
                                })
                            })
                            .collect();
                        actions.push(SidebandAction::Plan(steps));
                    }
                }
                _ => {}
            }
            if is_child {
                for act in &mut actions[before_len..] {
                    let inner = std::mem::replace(act, SidebandAction::Plan(Vec::new()));
                    *act = super::wrap_child_action(child_sid, parent_cid.as_deref(), inner);
                }
            }
        }
        actions
    }
}

fn is_before_idle(ts_ms: Option<u64>, idle_since_ms: u64) -> bool {
    idle_since_ms > 0 && ts_ms.is_some_and(|ts| ts <= idle_since_ms.saturating_add(50))
}

fn envelope_timestamp_ms(root: &Value) -> Option<u64> {
    if let Some(ms) = root
        .pointer("/params/_meta/agentTimestampMs")
        .or_else(|| root.pointer("/_meta/agentTimestampMs"))
        .and_then(Value::as_f64)
        .filter(|v| *v > 0.0)
    {
        return Some(ms as u64);
    }
    root.get("timestamp")
        .and_then(Value::as_f64)
        .filter(|v| *v > 0.0)
        .map(|secs| (secs * 1000.0) as u64)
}

pub(crate) fn percent_encode_cwd(cwd: &str) -> String {
    let mut out = String::with_capacity(cwd.len() * 2);
    for b in cwd.as_bytes() {
        let keep = b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_' | b'.' | b'~');
        if keep {
            out.push(*b as char);
        } else {
            out.push('%');
            out.push(
                char::from_digit(((*b >> 4) & 0xf) as u32, 16)
                    .unwrap_or('0')
                    .to_ascii_uppercase(),
            );
            out.push(
                char::from_digit((*b & 0xf) as u32, 16)
                    .unwrap_or('0')
                    .to_ascii_uppercase(),
            );
        }
    }
    out
}

pub(crate) fn extract_update_text(up: &Value) -> Option<String> {
    if let Some(c) = up.get("content") {
        if let Some(t) = c.get("text").and_then(Value::as_str) {
            return Some(t.to_string());
        }
        if let Some(arr) = c.as_array() {
            let joined: String = arr
                .iter()
                .filter_map(|b| {
                    b.get("text")
                        .and_then(Value::as_str)
                        .or_else(|| b.pointer("/content/text").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("");
            if !joined.is_empty() {
                return Some(joined);
            }
        }
    }
    up.get("text").and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn parse_raw_tool_status(v: Option<&Value>) -> Option<ToolCallStatus> {
    match v.and_then(Value::as_str)? {
        "pending" => Some(ToolCallStatus::Pending),
        "in_progress" | "running" => Some(ToolCallStatus::InProgress),
        "completed" | "done" | "success" => Some(ToolCallStatus::Completed),
        "failed" | "error" => Some(ToolCallStatus::Failed),
        _ => None,
    }
}

pub(crate) fn extract_location_path(up: &Value) -> Option<String> {
    up.get("locations")
        .and_then(Value::as_array)?
        .first()?
        .get("path")?
        .as_str()
        .map(str::to_string)
}

pub(crate) fn extract_diff(up: &Value) -> Option<(Option<String>, String)> {
    let arr = up.get("content")?.as_array()?;
    for item in arr {
        if item.get("type").and_then(Value::as_str) == Some("diff") {
            let new_text = item.get("newText").and_then(Value::as_str)?.to_string();
            let old_text = item
                .get("oldText")
                .and_then(Value::as_str)
                .map(str::to_string);
            return Some((old_text, new_text));
        }
    }
    None
}

pub(crate) fn extract_tool_output(up: &Value) -> Option<String> {
    if let Some(txt) = extract_update_text(up).filter(|s| !s.is_empty()) {
        return Some(txt);
    }
    let raw = up.get("rawOutput")?;
    if let Some(s) = raw.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    raw.get("output")
        .or_else(|| raw.get("stdout"))
        .or_else(|| raw.get("error"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(crate) fn parse_grok_usage(u: Option<&Value>) -> Option<TurnTokenUsage> {
    let u = u?.as_object()?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let input = n("inputTokens");
    let output = n("outputTokens");
    let cached = n("cachedReadTokens");
    let reasoning = n("reasoningTokens").max(n("thoughtTokens"));
    if input == 0 && output == 0 && cached == 0 && reasoning == 0 {
        return None;
    }
    Some(TurnTokenUsage {
        input_tokens: input,
        output_tokens: output,
        cached_input_tokens: (cached > 0).then_some(cached),
        reasoning_tokens: (reasoning > 0).then_some(reasoning),
    })
}

pub(crate) fn default_grok_home() -> Option<String> {
    for var in ["OTTO_GROK_HOME", "GROK_HOME"] {
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
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed).join(".grok").display().to_string())
}
