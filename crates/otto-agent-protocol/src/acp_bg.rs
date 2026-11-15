//! Cross-provider background-task detection (`run_command`, `schedule`,
//! `swarm_wait`, subagent tools, and `manage_task` cancellation) for
//! [`super::AcpDriver`].

use agent_client_protocol::v1::schema::ToolCallStatus;
use otto_core::agent_runtime::BackgroundTask;
use serde_json::Value;

use super::{command_text, AcpDriver, Tool};
use crate::driver::{DriverEvent, DriverStep};

impl AcpDriver {
    pub(super) fn note_tool_background(&mut self, step: &mut DriverStep, call_id: &str, t: &Tool) {
        if t.status != ToolCallStatus::Completed {
            return;
        }
        let lower = t.title.trim().to_ascii_lowercase();
        let item_id = Some(format!("{}:tool:{call_id}", self.turn()));
        let mut changed = false;

        if lower.contains("manage_task") {
            let kill = t
                .raw_input
                .as_ref()
                .and_then(|v| v.get("Action").and_then(Value::as_str))
                == Some("kill");
            if let (true, Some(tid)) = (
                kill,
                t.raw_input
                    .as_ref()
                    .and_then(|v| v.get("TaskId").and_then(Value::as_str)),
            ) {
                let before = self.bg_tasks.len();
                self.bg_tasks.retain(|b| b.id != tid);
                changed |= self.bg_tasks.len() != before;
            }
        }

        if let Some((_, rest)) = t.output.split_once("background task with task id:") {
            if let Some(tid) = rest
                .split_whitespace()
                .next()
                .map(|s| {
                    s.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
                })
                .filter(|s| !s.is_empty())
            {
                let label = command_text(t.raw_input.as_ref()).unwrap_or_else(|| t.title.clone());
                changed |= upsert_bg_task(
                    &mut self.bg_tasks,
                    BackgroundTask {
                        id: tid.to_string(),
                        kind: "command".into(),
                        label,
                        started_at: chrono::Utc::now(),
                        item_id: item_id.clone(),
                    },
                );
            }
        } else if is_background_command(t.raw_input.as_ref()) {
            let label = command_text(t.raw_input.as_ref()).unwrap_or_else(|| t.title.clone());
            changed |= upsert_bg_task(
                &mut self.bg_tasks,
                BackgroundTask {
                    id: format!("cmd:{call_id}"),
                    kind: "command".into(),
                    label,
                    started_at: chrono::Utc::now(),
                    item_id: item_id.clone(),
                },
            );
        } else if lower == "schedule"
            || t.raw_input.as_ref().is_some_and(|v| {
                v.get("DurationSeconds").is_some() || v.get("CronExpression").is_some()
            })
        {
            let label = schedule_label(t.raw_input.as_ref(), &t.title);
            changed |= upsert_bg_task(
                &mut self.bg_tasks,
                BackgroundTask {
                    id: format!("schedule:{call_id}"),
                    kind: "schedule".into(),
                    label,
                    started_at: chrono::Utc::now(),
                    item_id: item_id.clone(),
                },
            );
        } else if lower.contains("swarm_wait") {
            changed |= upsert_bg_task(
                &mut self.bg_tasks,
                BackgroundTask {
                    id: "swarm_wait".into(),
                    kind: "swarm".into(),
                    label: "Waiting for swarm messages".into(),
                    started_at: chrono::Utc::now(),
                    item_id,
                },
            );
        } else if is_subagent_tool(&lower, t.raw_input.as_ref()) {
            let label = t
                .raw_input
                .as_ref()
                .and_then(|v| {
                    v.get("Description")
                        .or_else(|| v.get("description"))
                        .or_else(|| v.get("Role"))
                        .or_else(|| v.get("AgentName"))
                        .or_else(|| v.get("subagent_type"))
                        .or_else(|| v.get("name"))
                        .and_then(Value::as_str)
                })
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| t.title.clone());
            changed |= upsert_bg_task(
                &mut self.bg_tasks,
                BackgroundTask {
                    id: format!("subagent:{call_id}"),
                    kind: "agent".into(),
                    label,
                    started_at: chrono::Utc::now(),
                    item_id,
                },
            );
        }

        if changed {
            step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
        }
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

fn is_subagent_tool(lower: &str, raw: Option<&Value>) -> bool {
    if lower.contains("invoke_subagent")
        || lower.contains("run_workflow")
        || lower.contains("delegate_to_agent")
        || lower.contains("spawn_agent")
    {
        return true;
    }
    lower == "task"
        && raw.is_some_and(|v| v.get("subagent_type").is_some() || v.get("prompt").is_some())
}

fn upsert_bg_task(list: &mut Vec<BackgroundTask>, task: BackgroundTask) -> bool {
    if let Some(existing) = list.iter_mut().find(|t| t.id == task.id) {
        if existing.label == task.label && existing.kind == task.kind {
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
