//! Antigravity (`agy acp`) sideband adapter (`transcript_full.jsonl` +
//! fallback `transcript.jsonl`).

use std::collections::VecDeque;
use std::path::PathBuf;

use agent_client_protocol::v1::schema::ToolCallStatus;
use otto_core::agent_runtime::TurnTokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::burst::{
    classify_tool, is_safe_session_id, now_ms, parse_rfc3339_end_ms, parse_rfc3339_ms, trim_ascii,
    unquote_args, MirroredCounts, MAX_TRACKED_SIDEBAND_IDS,
};
use super::SidebandAction;
use crate::driver::SidebandSource;

const AWAITING_PLANNER_MAX_MS: u64 = 90_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AgyState {
    #[serde(default)]
    pub offset: u64,
    #[serde(default)]
    pub using_fallback: bool,
    #[serde(default)]
    pub last_step_index: Option<u64>,
    #[serde(default)]
    pub prompted_turn_open: bool,
    #[serde(default)]
    pub prompted_turn_pre_idle: bool,
    #[serde(default)]
    pub in_autonomous_gap: bool,
    #[serde(default)]
    pub awaiting_planner: bool,
    #[serde(default)]
    pub awaiting_since_ms: u64,
    #[serde(default)]
    pub pending_tools: VecDeque<String>,
}

impl AgyState {
    pub fn reset_for_prompt(&mut self) {
        self.prompted_turn_open = false;
        self.prompted_turn_pre_idle = false;
        self.in_autonomous_gap = false;
        self.awaiting_planner = false;
        self.pending_tools.clear();
    }

    /// Closing one visual `auto-N` turn in Otto must NOT clear
    /// `in_autonomous_gap`: if the next `<SYSTEM_MESSAGE>` arrived inside the
    /// 1.5 s settle window, `in_autonomous_gap` is already `true` for
    /// `auto-(N+1)` and clearing it would silently drop the entire wakeup.
    pub fn on_turn_settled(&mut self) {
        self.awaiting_planner = false;
        self.pending_tools.clear();
    }

    pub fn is_awaiting(&self) -> bool {
        self.awaiting_planner
            && now_ms().saturating_sub(self.awaiting_since_ms) < AWAITING_PLANNER_MAX_MS
    }

    pub fn source(&self, brain_dir: &str, session_id: &str) -> Option<SidebandSource> {
        if brain_dir.is_empty() || !is_safe_session_id(session_id) {
            return None;
        }
        let logs = PathBuf::from(brain_dir)
            .join(session_id)
            .join(".system_generated")
            .join("logs");
        Some(SidebandSource::AppendLines {
            path: logs.join("transcript_full.jsonl"),
            fallback_path: Some(logs.join("transcript.jsonl")),
            scan_sessions_dir: None,
            offset: self.offset,
            using_fallback: self.using_fallback,
        })
    }

    pub fn parse_bytes(
        &mut self,
        using_fallback: bool,
        next_offset: u64,
        bytes: &[u8],
        idle_since_ms: u64,
        mirrored: &mut MirroredCounts,
    ) -> Vec<SidebandAction> {
        self.using_fallback = using_fallback;
        self.offset = next_offset;
        let mut actions = Vec::new();

        for raw in bytes.split(|b| *b == b'\n') {
            let line = trim_ascii(raw);
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let step_idx = v.get("step_index").and_then(Value::as_u64);
            if let Some(idx) = step_idx {
                if self.last_step_index.is_some_and(|prev| idx <= prev) {
                    continue;
                }
                self.last_step_index = Some(idx);
            }
            let Some(raw_step_type) = v.get("type").and_then(Value::as_str) else {
                continue;
            };
            let step_type = raw_step_type
                .strip_prefix("CORTEX_STEP_TYPE_")
                .unwrap_or(raw_step_type);
            let source = v
                .get("source")
                .and_then(Value::as_str)
                .map(|s| s.strip_prefix("CORTEX_STEP_SOURCE_").unwrap_or(s));
            let raw_created = v.get("created_at").and_then(Value::as_str);
            let created_ms = raw_created.and_then(parse_rfc3339_ms);
            let created_end_ms = raw_created.and_then(parse_rfc3339_end_ms);
            let has_subsec = raw_created.is_some_and(|s| {
                s.split_once('T')
                    .or_else(|| s.split_once('t'))
                    .is_some_and(|(_, t)| t.contains('.'))
            });
            let is_pre_idle = idle_since_ms > 0
                && created_end_ms.is_some_and(|ts_end| {
                    if has_subsec {
                        ts_end <= idle_since_ms.saturating_add(50)
                    } else {
                        ts_end < idle_since_ms
                    }
                });

            match (source, step_type) {
                (_, "USER_INPUT") => {
                    self.reset_for_prompt();
                    self.prompted_turn_open = true;
                    self.prompted_turn_pre_idle = idle_since_ms == 0
                        || is_pre_idle
                        || created_ms.is_some_and(|ts| ts <= idle_since_ms.saturating_add(50));
                }
                (Some("SYSTEM") | None, "SYSTEM_MESSAGE") => {
                    if is_pre_idle {
                        continue;
                    }
                    // If a prior prompted turn was cancelled/errored before writing
                    // a terminal PLANNER_RESPONSE, do not let its pre-idle USER_INPUT
                    // block a genuine post-idle `<SYSTEM_MESSAGE>` wakeup.
                    if self.prompted_turn_open {
                        if self.prompted_turn_pre_idle
                            || idle_since_ms == 0
                            || created_end_ms.is_some_and(|ts_end| ts_end > idle_since_ms)
                        {
                            self.prompted_turn_open = false;
                            self.prompted_turn_pre_idle = false;
                        } else {
                            continue;
                        }
                    }
                    self.in_autonomous_gap = true;
                    self.awaiting_planner = true;
                    self.awaiting_since_ms = now_ms();
                    let raw_msg = v.get("content").and_then(Value::as_str).unwrap_or_default();
                    let (sender, content) = parse_system_message_wakeup(raw_msg);
                    actions.push(SidebandAction::SystemWakeup { sender, content });
                }
                (Some("SYSTEM"), "ERROR_MESSAGE") => {
                    if is_pre_idle {
                        continue;
                    }
                    if self.prompted_turn_open {
                        if self.prompted_turn_pre_idle
                            && idle_since_ms > 0
                            && created_ms.is_some_and(|ts| ts > idle_since_ms.saturating_add(50))
                        {
                            self.prompted_turn_open = false;
                            self.prompted_turn_pre_idle = false;
                        } else {
                            continue;
                        }
                    }
                    self.in_autonomous_gap = true;
                    self.awaiting_planner = true;
                    self.awaiting_since_ms = now_ms();
                    actions.push(SidebandAction::TurnStarted);
                }
                (Some("SYSTEM"), "CHECKPOINT") if self.in_autonomous_gap => {
                    self.awaiting_planner = true;
                    self.awaiting_since_ms = now_ms();
                }
                (_, "PLANNER_RESPONSE") => {
                    let has_tools = v
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .is_some_and(|c| !c.is_empty());
                    if self.prompted_turn_open {
                        if self.prompted_turn_pre_idle
                            && idle_since_ms > 0
                            && created_ms.is_some_and(|ts| ts > idle_since_ms.saturating_add(50))
                        {
                            self.prompted_turn_open = false;
                            self.prompted_turn_pre_idle = false;
                        } else {
                            if !has_tools
                                && v.get("status").and_then(Value::as_str) != Some("ERROR")
                            {
                                self.prompted_turn_open = false;
                                self.prompted_turn_pre_idle = false;
                            }
                            continue;
                        }
                    }
                    // Recover if daemon resumed mid-wakeup after `<SYSTEM_MESSAGE>`
                    // was already at EOF.
                    if !self.in_autonomous_gap
                        && idle_since_ms > 0
                        && created_ms.is_some_and(|ts| ts > idle_since_ms.saturating_add(1_000))
                    {
                        self.in_autonomous_gap = true;
                    }
                    if self.in_autonomous_gap {
                        self.parse_planner_step(&v, step_idx.unwrap_or(0), mirrored, &mut actions);
                    }
                }
                (_, "GENERIC") if self.in_autonomous_gap => {
                    self.parse_generic_step(&v, &mut actions);
                }
                _ => {}
            }
        }
        actions
    }

    fn parse_planner_step(
        &mut self,
        v: &Value,
        step_idx: u64,
        mirrored: &mut MirroredCounts,
        actions: &mut Vec<SidebandAction>,
    ) {
        if let Some(thinking) = v.get("thinking").and_then(Value::as_str) {
            if !thinking.is_empty() {
                mirrored.thought_chars = mirrored
                    .thought_chars
                    .saturating_add(thinking.chars().count());
                actions.push(SidebandAction::Thought {
                    text: thinking.to_string(),
                    streaming: false,
                });
            }
        }

        let calls = v
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if !calls.is_empty() {
            self.awaiting_planner = true;
            self.awaiting_since_ms = now_ms();
            for (i, call) in calls.iter().enumerate() {
                mirrored.tool_calls = mirrored.tool_calls.saturating_add(1);
                let native_id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                if let Some(real) = native_id {
                    mirrored.preseed_tool_id(real);
                }
                let id = native_id
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("tlog-{step_idx}-{i}"));
                let raw_name = call.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = call
                    .get("args")
                    .or_else(|| call.get("arguments"))
                    .map(unquote_args);
                let desc = classify_tool(raw_name, args.as_ref());
                if self.pending_tools.len() >= MAX_TRACKED_SIDEBAND_IDS {
                    self.pending_tools.pop_front();
                }
                self.pending_tools.push_back(id.clone());
                actions.push(SidebandAction::ToolStart {
                    id,
                    title: desc.title,
                    kind: desc.kind,
                    status: ToolCallStatus::InProgress,
                    raw_input: args,
                    file: desc.file,
                    diff: desc.diff,
                });
            }
        } else if v.get("status").and_then(Value::as_str) == Some("ERROR") {
            self.awaiting_planner = true;
            self.awaiting_since_ms = now_ms();
        } else {
            self.awaiting_planner = false;
        }

        if let Some(content) = v.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                mirrored.msg_chars = mirrored.msg_chars.saturating_add(content.chars().count());
                actions.push(SidebandAction::Message {
                    text: content.to_string(),
                    message_id: None,
                    streaming: false,
                });
            }
        }

        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        let input = n("input_tokens");
        let cached = n("cache_read_tokens");
        let output = n("output_tokens");
        let thought = n("thinking_tokens");
        if input > 0 || cached > 0 || output > 0 || thought > 0 {
            actions.push(SidebandAction::Usage(TurnTokenUsage {
                input_tokens: input.saturating_add(cached),
                output_tokens: output,
                cached_input_tokens: (cached > 0).then_some(cached),
                reasoning_tokens: (thought > 0).then_some(thought),
            }));
        }
    }

    fn parse_generic_step(&mut self, v: &Value, actions: &mut Vec<SidebandAction>) {
        let Some(id) = self.pending_tools.pop_front() else {
            return;
        };
        self.awaiting_since_ms = now_ms();
        let failed = v.get("status").and_then(Value::as_str) == Some("ERROR");
        let output = v
            .get("content")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .or_else(|| v.get("error").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        actions.push(SidebandAction::ToolUpdate {
            id,
            status: if failed {
                ToolCallStatus::Failed
            } else {
                ToolCallStatus::Completed
            },
            title: None,
            kind: None,
            raw_input: None,
            output: Some(output),
            file: None,
            diff: None,
        });
    }
}

pub(crate) fn default_agy_brain_dir() -> Option<String> {
    if let Ok(dir) = std::env::var("OTTO_AGY_BRAIN_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if otto_core::e2e::isolated() {
        return None;
    }
    let home = std::env::var("HOME").ok()?;
    let trimmed = home.trim();
    if trimmed.is_empty() {
        return None;
    }
    let primary = PathBuf::from(trimmed)
        .join(".gemini")
        .join("antigravity-cli")
        .join("brain");
    Some(primary.display().to_string())
}

pub(crate) fn parse_system_message_wakeup(raw: &str) -> (Option<String>, String) {
    let mut trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("<SYSTEM_MESSAGE>") {
        trimmed = rest.trim_start();
    }
    if let Some(rest) = trimmed.strip_suffix("</SYSTEM_MESSAGE>") {
        trimmed = rest.trim_end();
    }

    let sender = extract_sender_token(trimmed);
    let body = if trimmed.starts_with("[Message]") {
        trimmed
            .find(" content=")
            .map(|idx| trimmed[idx + " content=".len()..].trim().to_string())
            .unwrap_or_else(|| trimmed.to_string())
    } else {
        trimmed.to_string()
    };
    (sender, body)
}

fn extract_sender_token(text: &str) -> Option<String> {
    let head = text.lines().take(4).collect::<Vec<_>>().join(" ");
    if let Some(idx) = head.find("sender=") {
        let rest = head[idx + "sender=".len()..].trim_start();
        let rest = rest.strip_prefix('"').unwrap_or(rest);
        let tok: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | ':' | '.'))
            .collect();
        let tok = tok.trim_end_matches(['.', ',']);
        if !tok.is_empty() {
            return Some(tok.to_string());
        }
    }
    let lower = head.to_ascii_lowercase();
    if let Some(idx) = lower.find("task id:") {
        let rest = head[idx + "task id:".len()..].trim_start();
        let tok: String = rest
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && !matches!(c, '-' | '_' | '/'))
            .to_string();
        if !tok.is_empty() {
            return Some(tok);
        }
    }
    None
}
