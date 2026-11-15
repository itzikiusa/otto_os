//! OpenCode (`opencode acp`) sideband adapter (`opencode.db` SQLite `part` +
//! `message` tables).
//!
//! Off-lock async I/O (`super::io`) checks `(db_mtime, db_len, wal_mtime, wal_len)`
//! first and queries `part` + `message` read-only only when the stamp changes,
//! feeding JSONL rows (`{"id","message_id","time_updated","role","data"}`) into
//! [`OpencodeState::parse_rows`] so the driver itself remains 100% sans-IO.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use agent_client_protocol::v1::schema::ToolCallStatus;
use otto_core::agent_runtime::TurnTokenUsage;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::burst::{classify_tool, is_safe_session_id, now_ms, trim_ascii, MirroredCounts};
use super::SidebandAction;
use crate::driver::SidebandSource;

const AWAITING_STEP_MAX_MS: u64 = 90_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OpencodeState {
    #[serde(default)]
    pub last_stamp: (u64, u64, u64, u64),
    #[serde(default)]
    pub min_time_ms: i64,
    #[serde(default)]
    pub seen_text_chars: HashMap<String, usize>,
    #[serde(default)]
    pub seen_tool_status: HashMap<String, String>,
    #[serde(default)]
    pub seen_step_finish: HashSet<String>,
    #[serde(default)]
    pub awaiting_step_finish: bool,
    #[serde(default)]
    pub awaiting_since_ms: u64,
}

impl OpencodeState {
    pub fn reset_turn_flags(&mut self) {
        self.awaiting_step_finish = false;
    }

    pub fn is_awaiting(&self) -> bool {
        self.awaiting_step_finish
            && now_ms().saturating_sub(self.awaiting_since_ms) < AWAITING_STEP_MAX_MS
    }

    pub fn source(
        &self,
        data_dir: &str,
        session_id: &str,
        idle_since_ms: u64,
    ) -> Option<SidebandSource> {
        if data_dir.is_empty() || !is_safe_session_id(session_id) {
            return None;
        }
        let min_time_ms = self
            .min_time_ms
            .max(i64::try_from(idle_since_ms).unwrap_or(0));
        Some(SidebandSource::OpencodeSqlite {
            db_path: PathBuf::from(data_dir).join("opencode.db"),
            session_id: session_id.to_string(),
            min_time_ms,
            last_stamp: self.last_stamp,
        })
    }

    pub fn parse_rows(
        &mut self,
        stamp: (u64, u64, u64, u64),
        rows_jsonl: &[u8],
        idle_since_ms: u64,
        mirrored: &mut MirroredCounts,
    ) -> Vec<SidebandAction> {
        self.last_stamp = stamp;
        let mut actions = Vec::new();

        for raw in rows_jsonl.split(|b| *b == b'\n') {
            let line = trim_ascii(raw);
            if line.is_empty() {
                continue;
            }
            let Ok(row) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let Some(part_id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            let time_updated = row.get("time_updated").and_then(Value::as_i64).unwrap_or(0);
            if time_updated > self.min_time_ms {
                self.min_time_ms = time_updated;
            }
            let role = row
                .get("role")
                .or_else(|| row.get("msg_role"))
                .and_then(Value::as_str)
                .unwrap_or("assistant");
            if role != "assistant" {
                continue;
            }
            let baseline_only = idle_since_ms > 0
                && u64::try_from(time_updated.max(0))
                    .ok()
                    .is_some_and(|t| t.saturating_add(50) < idle_since_ms);

            let data: Value = match row.get("data") {
                Some(Value::String(s)) => match serde_json::from_str(s) {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                Some(v @ Value::Object(_)) => v.clone(),
                _ => continue,
            };
            let Some(ptype) = data.get("type").and_then(Value::as_str) else {
                continue;
            };

            match ptype {
                "reasoning" => {
                    let full = data.get("text").and_then(Value::as_str).unwrap_or("");
                    if let Some(delta) =
                        take_part_delta(&mut self.seen_text_chars, part_id, full, baseline_only)
                    {
                        mirrored.thought_chars =
                            mirrored.thought_chars.saturating_add(delta.chars().count());
                        actions.push(SidebandAction::Thought {
                            text: delta,
                            streaming: true,
                        });
                    }
                }
                "text" => {
                    let full = data.get("text").and_then(Value::as_str).unwrap_or("");
                    if let Some(delta) =
                        take_part_delta(&mut self.seen_text_chars, part_id, full, baseline_only)
                    {
                        mirrored.msg_chars =
                            mirrored.msg_chars.saturating_add(delta.chars().count());
                        let mid = row
                            .get("message_id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        actions.push(SidebandAction::Message {
                            text: delta,
                            message_id: mid,
                            streaming: true,
                        });
                    }
                }
                "tool" => {
                    self.ingest_tool_part(part_id, &data, baseline_only, mirrored, &mut actions);
                }
                "step-finish" => {
                    if !self.seen_step_finish.insert(part_id.to_string()) || baseline_only {
                        continue;
                    }
                    let reason = data.get("reason").and_then(Value::as_str).unwrap_or("");
                    self.awaiting_step_finish = matches!(reason, "tool-calls" | "function_call");
                    if self.awaiting_step_finish {
                        self.awaiting_since_ms = now_ms();
                    }
                    if let Some(usage) = parse_opencode_tokens(data.get("tokens")) {
                        actions.push(SidebandAction::Usage(usage));
                    }
                    if !self.awaiting_step_finish {
                        let clean_reason = (!matches!(reason, "" | "stop" | "end_turn"))
                            .then(|| reason.to_string());
                        actions.push(SidebandAction::TurnCompleted {
                            reason: clean_reason,
                        });
                    }
                }
                _ => {}
            }
        }
        actions
    }

    fn ingest_tool_part(
        &mut self,
        part_id: &str,
        data: &Value,
        baseline_only: bool,
        mirrored: &mut MirroredCounts,
        actions: &mut Vec<SidebandAction>,
    ) {
        let call_id = data
            .get("callID")
            .and_then(Value::as_str)
            .unwrap_or(part_id);
        let state = data.get("state");
        let raw_status = state
            .and_then(|s| s.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("running");
        let prev = self.seen_tool_status.get(call_id).cloned();
        if prev.as_deref() == Some(raw_status) {
            return;
        }
        self.seen_tool_status
            .insert(call_id.to_string(), raw_status.to_string());
        if baseline_only {
            return;
        }
        let tool_name = data.get("tool").and_then(Value::as_str).unwrap_or("tool");
        let input = state.and_then(|s| s.get("input")).cloned();
        let desc = classify_tool(tool_name, input.as_ref());
        let status = match raw_status {
            "pending" => ToolCallStatus::Pending,
            "running" | "in_progress" => ToolCallStatus::InProgress,
            "error" | "failed" => ToolCallStatus::Failed,
            _ => ToolCallStatus::Completed,
        };

        if prev.is_none() {
            mirrored.tool_calls = mirrored.tool_calls.saturating_add(1);
            self.awaiting_step_finish = true;
            self.awaiting_since_ms = now_ms();
            actions.push(SidebandAction::ToolStart {
                id: call_id.to_string(),
                title: desc.title,
                kind: desc.kind,
                raw_input: input,
                file: desc.file,
                diff: desc.diff,
            });
        }
        if matches!(status, ToolCallStatus::Completed | ToolCallStatus::Failed) {
            self.awaiting_since_ms = now_ms();
            let output = state
                .and_then(|s| {
                    s.get("output")
                        .and_then(Value::as_str)
                        .filter(|o| !o.is_empty())
                        .or_else(|| s.get("error").and_then(Value::as_str))
                })
                .map(str::to_string);
            actions.push(SidebandAction::ToolUpdate {
                id: call_id.to_string(),
                status,
                title: None,
                kind: None,
                raw_input: None,
                output,
                file: None,
                diff: None,
            });
        }
    }
}

fn take_part_delta(
    seen: &mut HashMap<String, usize>,
    part_id: &str,
    full: &str,
    baseline_only: bool,
) -> Option<String> {
    if full.is_empty() {
        return None;
    }
    let total = full.chars().count();
    let prev = seen.get(part_id).copied().unwrap_or(0);
    if total <= prev {
        return None;
    }
    seen.insert(part_id.to_string(), total);
    if baseline_only {
        return None;
    }
    let byte_idx = full
        .char_indices()
        .nth(prev)
        .map(|(i, _)| i)
        .unwrap_or(full.len());
    let delta = &full[byte_idx..];
    (!delta.is_empty()).then(|| delta.to_string())
}

fn parse_opencode_tokens(tok: Option<&Value>) -> Option<TurnTokenUsage> {
    let tok = tok?.as_object()?;
    let n = |k: &str| tok.get(k).and_then(Value::as_u64).unwrap_or(0);
    let input = n("input");
    let output = n("output");
    let reasoning = n("reasoning");
    let cache_read = tok
        .get("cache")
        .and_then(|c| c.get("read"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if input == 0 && output == 0 && reasoning == 0 && cache_read == 0 {
        return None;
    }
    Some(TurnTokenUsage {
        input_tokens: input.saturating_add(cache_read),
        output_tokens: output,
        cached_input_tokens: (cache_read > 0).then_some(cache_read),
        reasoning_tokens: (reasoning > 0).then_some(reasoning),
    })
}

pub(crate) fn default_opencode_data_dir() -> Option<String> {
    if let Ok(dir) = std::env::var("OTTO_OPENCODE_DATA_DIR") {
        let trimmed = dir.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if otto_core::e2e::isolated() {
        return None;
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return Some(
                PathBuf::from(trimmed)
                    .join("opencode")
                    .display()
                    .to_string(),
            );
        }
    }
    let home = std::env::var("HOME").ok()?;
    let trimmed = home.trim();
    (!trimmed.is_empty()).then(|| {
        PathBuf::from(trimmed)
            .join(".local")
            .join("share")
            .join("opencode")
            .display()
            .to_string()
    })
}
