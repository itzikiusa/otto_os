//! Antigravity (`agy acp`) sideband adapter (`transcript_full.jsonl` +
//! fallback `transcript.jsonl`).

use std::collections::VecDeque;
use std::path::PathBuf;

use agent_client_protocol::v1::schema::ToolCallStatus;
use otto_core::agent_runtime::TurnTokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::burst::{
    classify_tool, is_safe_session_id, now_ms, parse_rfc3339_ms, trim_ascii, unquote_args,
    MirroredCounts,
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
    pub in_autonomous_gap: bool,
    #[serde(default)]
    pub awaiting_planner: bool,
    #[serde(default)]
    pub awaiting_since_ms: u64,
    #[serde(default)]
    pub pending_tools: VecDeque<String>,
}

impl AgyState {
    pub fn reset_turn_flags(&mut self) {
        self.in_autonomous_gap = false;
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
            let Some(step_type) = v.get("type").and_then(Value::as_str) else {
                continue;
            };
            let source = v.get("source").and_then(Value::as_str);
            let created_ms = v
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(parse_rfc3339_ms);

            match (source, step_type) {
                (_, "USER_INPUT") => {
                    self.reset_turn_flags();
                }
                (Some("SYSTEM"), "SYSTEM_MESSAGE" | "ERROR_MESSAGE") => {
                    // Skip mid-prompt retries written before the prompted turn ended.
                    if idle_since_ms > 0
                        && created_ms.is_some_and(|ts| ts.saturating_add(50) < idle_since_ms)
                    {
                        continue;
                    }
                    if step_type == "SYSTEM_MESSAGE" || self.in_autonomous_gap {
                        self.in_autonomous_gap = true;
                        self.awaiting_since_ms = now_ms();
                    }
                }
                (Some("SYSTEM"), "CHECKPOINT") if self.in_autonomous_gap => {
                    self.awaiting_since_ms = now_ms();
                }
                (_, "PLANNER_RESPONSE") if self.in_autonomous_gap => {
                    self.parse_planner_step(&v, step_idx.unwrap_or(0), mirrored, &mut actions);
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
                let id = format!("tlog-{step_idx}-{i}");
                let raw_name = call.get("name").and_then(Value::as_str).unwrap_or("tool");
                let args = call
                    .get("args")
                    .or_else(|| call.get("arguments"))
                    .map(unquote_args);
                let desc = classify_tool(raw_name, args.as_ref());
                self.pending_tools.push_back(id.clone());
                actions.push(SidebandAction::ToolStart {
                    id,
                    title: desc.title,
                    kind: desc.kind,
                    raw_input: args,
                    file: desc.file,
                    diff: desc.diff,
                });
            }
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
