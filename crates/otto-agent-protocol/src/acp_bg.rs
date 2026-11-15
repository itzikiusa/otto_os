//! Cross-provider background-task detection (`run_command`, `schedule`,
//! `swarm_wait`, `manage_task`, `manage_subagents`, and Grok
//! `get_command_or_subagent_output`) for [`super::AcpDriver`].

use std::path::PathBuf;

use agent_client_protocol::v1::schema::ToolCallStatus;
use chrono::{Duration as ChronoDuration, Utc};
use otto_core::agent_runtime::{BackgroundTask, TurnItemStatus};
use serde_json::Value;

use super::subagent::{
    parse::{file_uri_to_path, parse_embedded_json, parse_grok_bg_launch, take_id_token},
    BgTaskMeta,
};
use super::{command_text, AcpDriver, Tool};
use crate::acp_sideband::confinement::validate_task_log_candidate;
use crate::driver::DriverStep;

impl AcpDriver {
    pub(super) fn note_tool_background(&mut self, step: &mut DriverStep, call_id: &str, t: &Tool) {
        let lower = t.title.trim().to_ascii_lowercase();

        if lower.contains("get_command_or_subagent_output")
            || lower.contains("get_subagent_output")
            || lower.contains("get_command_output")
        {
            if matches!(t.status, ToolCallStatus::Completed | ToolCallStatus::Failed) {
                self.note_grok_bg_output_poll(step, t);
            }
            return;
        }

        if t.status != ToolCallStatus::Completed {
            return;
        }

        let item_id = Some(format!("{}:tool:{call_id}", self.turn()));
        let turn_id = self.state.turn_id.clone();

        if lower.contains("manage_task") {
            let kill = t
                .raw_input
                .as_ref()
                .and_then(|v| {
                    v.get("Action")
                        .or_else(|| v.get("action"))
                        .and_then(Value::as_str)
                })
                .is_some_and(|a| a.eq_ignore_ascii_case("kill"));
            if let (true, Some(tid)) = (
                kill,
                t.raw_input.as_ref().and_then(|v| {
                    v.get("TaskId")
                        .or_else(|| v.get("task_id"))
                        .or_else(|| v.get("id"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                }),
            ) {
                self.settle_bg_task(step, tid, TurnItemStatus::Cancelled, None, None);
            }
            return;
        }

        if lower.contains("manage_subagents") {
            self.note_manage_subagents(step, t.raw_input.as_ref());
            return;
        }

        if let Some((_, rest)) = t.output.split_once("background task with task id:") {
            if let Some(tid) = take_id_token(rest) {
                if !self.state.finished_bg_ids.iter().any(|f| f == &tid) {
                    let cmd = command_text(t.raw_input.as_ref());
                    let label = cmd.clone().unwrap_or_else(|| t.title.clone());
                    let log_path = self.extract_confined_task_log_path(&t.output);
                    self.register_bg_task(
                        step,
                        BgTaskMeta {
                            id: tid,
                            kind: "command".into(),
                            label,
                            started_at: Utc::now(),
                            call_id: Some(call_id.to_string()),
                            item_id,
                            turn_id,
                            command_input: cmd,
                            accumulated_output: String::new(),
                            due_at: None,
                            timer_condition: None,
                            log_path,
                            fallback_log_path: None,
                            tail_offset: 0,
                            tail_using_fallback: false,
                        },
                    );
                }
            }
        } else if let Some((grok_id, false)) = parse_grok_bg_launch(&t.output) {
            if !self.state.finished_bg_ids.iter().any(|f| f == &grok_id) {
                let cmd = command_text(t.raw_input.as_ref());
                let label = cmd.clone().unwrap_or_else(|| t.title.clone());
                let log_path = self.extract_confined_task_log_path(&t.output);
                self.register_bg_task(
                    step,
                    BgTaskMeta {
                        id: grok_id,
                        kind: "command".into(),
                        label,
                        started_at: Utc::now(),
                        call_id: Some(call_id.to_string()),
                        item_id,
                        turn_id,
                        command_input: cmd,
                        accumulated_output: String::new(),
                        due_at: None,
                        timer_condition: None,
                        log_path,
                        fallback_log_path: None,
                        tail_offset: 0,
                        tail_using_fallback: false,
                    },
                );
            }
        } else if is_background_command(t.raw_input.as_ref()) {
            let tid = format!("cmd:{call_id}");
            if !self.state.finished_bg_ids.iter().any(|f| f == &tid) {
                let cmd = command_text(t.raw_input.as_ref());
                let label = cmd.clone().unwrap_or_else(|| t.title.clone());
                let log_path = self.extract_confined_task_log_path(&t.output);
                self.register_bg_task(
                    step,
                    BgTaskMeta {
                        id: tid,
                        kind: "command".into(),
                        label,
                        started_at: Utc::now(),
                        call_id: Some(call_id.to_string()),
                        item_id,
                        turn_id,
                        command_input: cmd,
                        accumulated_output: String::new(),
                        due_at: None,
                        timer_condition: None,
                        log_path,
                        fallback_log_path: None,
                        tail_offset: 0,
                        tail_using_fallback: false,
                    },
                );
            }
        } else if lower == "schedule"
            || lower.ends_with(":schedule")
            || t.raw_input.as_ref().is_some_and(|v| {
                v.get("DurationSeconds").is_some() || v.get("CronExpression").is_some()
            })
        {
            let now = Utc::now();
            let due_at = one_shot_due_at(t.raw_input.as_ref(), now);
            let timer_condition = t
                .raw_input
                .as_ref()
                .and_then(|v| v.get("TimerCondition").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let label = schedule_label(t.raw_input.as_ref(), &t.title);
            self.register_bg_task(
                step,
                BgTaskMeta {
                    id: format!("schedule:{call_id}"),
                    kind: "timer".into(),
                    label,
                    started_at: now,
                    call_id: Some(call_id.to_string()),
                    item_id,
                    turn_id,
                    command_input: None,
                    accumulated_output: String::new(),
                    due_at,
                    timer_condition,
                    log_path: None,
                    fallback_log_path: None,
                    tail_offset: 0,
                    tail_using_fallback: false,
                },
            );
        } else if lower.contains("swarm_wait") {
            self.register_bg_task(
                step,
                BgTaskMeta {
                    id: "swarm_wait".into(),
                    kind: "swarm".into(),
                    label: "Waiting for swarm messages".into(),
                    started_at: Utc::now(),
                    call_id: Some(call_id.to_string()),
                    item_id,
                    turn_id,
                    command_input: None,
                    accumulated_output: String::new(),
                    due_at: None,
                    timer_condition: None,
                    log_path: None,
                    fallback_log_path: None,
                    tail_offset: 0,
                    tail_using_fallback: false,
                },
            );
        } else if lower.contains("run_workflow") {
            let label = t
                .raw_input
                .as_ref()
                .and_then(|v| v.get("Description").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| t.title.clone());
            self.register_bg_task(
                step,
                BgTaskMeta {
                    id: format!("workflow:{call_id}"),
                    kind: "agent".into(),
                    label,
                    started_at: Utc::now(),
                    call_id: Some(call_id.to_string()),
                    item_id,
                    turn_id,
                    command_input: None,
                    accumulated_output: String::new(),
                    due_at: None,
                    timer_condition: None,
                    log_path: None,
                    fallback_log_path: None,
                    tail_offset: 0,
                    tail_using_fallback: false,
                },
            );
        }
    }

    fn extract_confined_task_log_path(&self, output: &str) -> Option<PathBuf> {
        let raw = extract_task_log_path(output)?;
        validate_task_log_candidate(
            &raw,
            &self.state.cwd,
            self.state.sideband.root_dir(),
            self.provider,
        )
    }

    fn note_manage_subagents(&mut self, step: &mut DriverStep, raw_input: Option<&Value>) {
        let Some(v) = raw_input else { return };
        let action = v
            .get("Action")
            .or_else(|| v.get("action"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if action.eq_ignore_ascii_case("kill_all") {
            let mut agent_ids: Vec<String> = self
                .state
                .bg_tasks
                .iter()
                .filter(|b| b.kind == "agent")
                .map(|b| b.id.clone())
                .collect();
            for a in self.state.subagents.values() {
                if let Some(tid) = &a.task_id {
                    if !agent_ids.iter().any(|x| x == tid) {
                        agent_ids.push(tid.clone());
                    }
                }
            }
            for id in agent_ids {
                self.settle_bg_task(step, &id, TurnItemStatus::Cancelled, None, None);
            }
        } else if action.eq_ignore_ascii_case("kill") {
            let mut ids: Vec<String> = v
                .get("ConversationIds")
                .or_else(|| v.get("conversation_ids"))
                .or_else(|| v.get("ids"))
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if let Some(single) = v
                .get("ConversationId")
                .or_else(|| v.get("conversation_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                ids.push(single.to_string());
            }
            for id in ids {
                self.settle_bg_task(step, &id, TurnItemStatus::Cancelled, None, None);
            }
        }
    }

    fn note_grok_bg_output_poll(&mut self, step: &mut DriverStep, t: &Tool) {
        let Some(target_id) = t.raw_input.as_ref().and_then(|v| {
            [
                "id",
                "task_id",
                "taskId",
                "subagent_id",
                "subagentId",
                "agent_id",
                "agentId",
                "command_id",
                "commandId",
            ]
            .iter()
            .find_map(|k| {
                v.get(*k)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
        }) else {
            return;
        };

        let parsed = parse_embedded_json(&t.output);
        if t.status == ToolCallStatus::Completed
            && is_grok_poll_still_running(&t.output, parsed.as_ref())
        {
            return;
        }

        let exit_code = parsed
            .as_ref()
            .and_then(|v| {
                v.get("exit_code")
                    .or_else(|| v.get("exitCode"))
                    .and_then(Value::as_i64)
            })
            .map(|c| c as i32);
        let failed = t.status == ToolCallStatus::Failed
            || exit_code.is_some_and(|c| c != 0)
            || parsed.as_ref().is_some_and(|v| {
                v.get("status").and_then(Value::as_str).is_some_and(|s| {
                    s.eq_ignore_ascii_case("failed") || s.eq_ignore_ascii_case("error")
                })
            });
        let status = if failed {
            TurnItemStatus::Failed
        } else {
            TurnItemStatus::Completed
        };
        let final_out = extract_grok_poll_output(&t.output, parsed.as_ref());
        self.settle_bg_task(step, &target_id, status, final_out, exit_code);
    }
}

fn is_background_command(raw: Option<&Value>) -> bool {
    let Some(v) = raw else { return false };
    [
        "is_background",
        "run_in_background",
        "background",
        "IsDaemon",
    ]
    .iter()
    .any(|k| v.get(*k).and_then(Value::as_bool) == Some(true))
}

fn extract_task_log_path(output: &str) -> Option<PathBuf> {
    if let Some((_, rest)) = output.split_once("Task logs are available at:") {
        if let Some(tok) = rest.split_whitespace().next() {
            if let Some(p) = file_uri_to_path(tok) {
                return Some(p);
            }
        }
    }
    if let Some(idx) = output.find("file:///") {
        if let Some(tok) = output[idx..].split_whitespace().next() {
            if let Some(p) = file_uri_to_path(tok) {
                if p.extension().and_then(|e| e.to_str()) == Some("log") {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn one_shot_due_at(
    raw: Option<&Value>,
    now: chrono::DateTime<Utc>,
) -> Option<chrono::DateTime<Utc>> {
    let v = raw?;
    let has_cron = v
        .get("CronExpression")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if has_cron {
        return None;
    }
    if let Some(secs) = v.get("DurationSeconds").and_then(Value::as_u64) {
        return Some(now + ChronoDuration::seconds(secs.min(86_400 * 365) as i64));
    }
    if let Some(secs_f) = v.get("DurationSeconds").and_then(Value::as_f64) {
        if secs_f.is_finite() && secs_f >= 0.0 {
            let ms = (secs_f.min((86_400 * 365) as f64) * 1_000.0) as i64;
            return Some(now + ChronoDuration::milliseconds(ms));
        }
    }
    None
}

fn is_grok_poll_still_running(output: &str, parsed: Option<&Value>) -> bool {
    if let Some(v) = parsed {
        if v.get("running").and_then(Value::as_bool) == Some(true)
            || v.get("done").and_then(Value::as_bool) == Some(false)
            || v.get("completed").and_then(Value::as_bool) == Some(false)
        {
            return true;
        }
        if let Some(st) = v
            .get("status")
            .or_else(|| v.get("state"))
            .and_then(Value::as_str)
        {
            if matches!(
                st.trim().to_ascii_lowercase().as_str(),
                "running" | "in_progress" | "inprogress" | "pending" | "active"
            ) {
                return true;
            }
        }
    }
    let lower = output.trim().to_ascii_lowercase();
    lower.starts_with("still running") || lower.starts_with("status: running")
}

fn extract_grok_poll_output(output: &str, parsed: Option<&Value>) -> Option<String> {
    if let Some(v) = parsed {
        for key in [
            "output", "result", "stdout", "response", "text", "content", "summary",
        ] {
            if let Some(s) = v
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return Some(s.to_string());
            }
        }
    }
    let t = output.trim();
    (!t.is_empty()).then(|| t.to_string())
}

pub(super) fn upsert_bg_task(list: &mut Vec<BackgroundTask>, task: BackgroundTask) -> bool {
    if let Some(existing) = list.iter_mut().find(|t| t.id == task.id) {
        if existing.label == task.label
            && existing.kind == task.kind
            && existing.item_id == task.item_id
            && existing.due_at == task.due_at
        {
            return false;
        }
        *existing = task;
        return true;
    }
    list.push(task);
    true
}

fn schedule_label(raw: Option<&Value>, fallback: &str) -> String {
    if let Some(v) = raw {
        let detail = v
            .get("Prompt")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(secs) = v.get("DurationSeconds").and_then(Value::as_u64) {
            return match detail {
                Some(p) => format!("Timer ({secs}s): {}", truncate_chars(p, 72)),
                None => format!("Scheduled timer ({secs}s)"),
            };
        }
        if let Some(cron) = v.get("CronExpression").and_then(Value::as_str) {
            return match detail {
                Some(p) => format!("Schedule ({cron}): {}", truncate_chars(p, 72)),
                None => format!("Recurring schedule ({cron})"),
            };
        }
    }
    fallback.to_string()
}

fn truncate_chars(s: &str, max: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}
