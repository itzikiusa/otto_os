//! Claude Code's background work and subagents, for [`super::ClaudeDriver`].
//!
//! Frames (Claude Code 2.1.318 stream-json, `system` subtypes):
//!
//! * `task_started {task_id, tool_use_id, description, task_type,
//!   is_backgrounded, subagent_type, prompt}` — `task_type` is `local_bash`
//!   (a Bash `run_in_background`), `local_agent` / `remote_agent` (an `Agent`
//!   / `Task` call), …;
//! * `task_progress {task_id, tool_use_id, last_tool_name, usage:{tool_uses}}`;
//! * `task_updated {task_id, patch:{status, is_backgrounded, …}}` — a status
//!   of `completed` / `failed` / `killed` is terminal (`Ko` in the CLI);
//! * `task_notification {task_id, tool_use_id, status, summary,
//!   output_file}` — the task finished. Its result is then fed back to the
//!   model as a `<task-notification>` user message, and when the agent was
//!   idle the CLI runs a turn on its own (the driver's auto turn).
//!
//! Older CLIs without `task_started` are covered by the launch tool results
//! ("Command running in background with ID: <id>", "Async agent launched
//! … agentId: <id>").
//!
//! A task counts as BACKGROUND (and keeps the session busy) when the CLI says
//! so (`is_backgrounded`) or its tool call asked for `run_in_background`; a
//! foreground task is forgotten when its tool returns or its turn ends.
//!
//! Subagents: a root `Agent` / `Task` tool call is one `subagent` item. The
//! subagent's own frames (`parent_tool_use_id` = that call) become items
//! with `parent_item_id` = the card, in the card's turn — text from its
//! complete `assistant` frames (its partial stream events would interleave
//! with the root's blocks, so they are skipped), tool calls settled by its
//! `user` tool results. A background subagent's card stays `running` past
//! its turn until its `task_notification`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use otto_core::agent_runtime::{BackgroundTask, TurnItem, TurnItemKind, TurnItemStatus};
use serde_json::Value;

use super::{classify, result_text, ClaudeDriver, Tool, ToolType};
use crate::driver::{item, tool_error, DriverEvent, DriverStep};

pub(super) struct Task {
    kind: String,
    label: String,
    started_at: DateTime<Utc>,
    tool_use_id: Option<String>,
    item_id: Option<String>,
    background: bool,
}

pub(super) struct Agent {
    item_id: String,
    turn_id: String,
    tool_name: String,
    description: Option<String>,
    subagent_type: Option<String>,
    prompt: Option<String>,
    output: Option<String>,
    error: Option<String>,
    background: bool,
    last_tool: Option<String>,
    tool_uses: u32,
    status: TurnItemStatus,
}

/// A subagent's own tool call.
pub(super) struct SubTool {
    tool: Tool,
    parent: String,
    item_id: String,
    turn_id: String,
}

/// `Agent` (current CLIs) / `Task` (older ones) launch a subagent.
pub(super) fn is_agent_tool(name: &str) -> bool {
    matches!(name, "Agent" | "Task")
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

fn kind_of(task_type: Option<&str>) -> String {
    match task_type {
        Some("local_bash") => "command".into(),
        Some("local_agent" | "remote_agent" | "in_process_teammate") => "agent".into(),
        Some(t) if !t.is_empty() => t.to_string(),
        _ => "task".into(),
    }
}

fn is_terminal(status: &str) -> bool {
    matches!(
        status,
        "completed" | "failed" | "killed" | "stopped" | "cancelled"
    )
}

/// The id after `marker` up to the next whitespace / `.` / `)`.
fn id_after(text: &str, marker: &str) -> Option<String> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let id: String = rest
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '.' | ')' | ','))
        .collect();
    (!id.is_empty()).then_some(id)
}

impl ClaudeDriver {
    /// Pending background work as the runtime reports it, oldest first.
    pub(super) fn background_list(&self) -> Vec<BackgroundTask> {
        let mut v: Vec<BackgroundTask> = self
            .tasks
            .iter()
            .filter(|(_, t)| t.background)
            .map(|(id, t)| BackgroundTask {
                id: id.clone(),
                kind: t.kind.clone(),
                label: t.label.clone(),
                started_at: t.started_at,
                item_id: t.item_id.clone(),
            })
            .collect();
        v.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.id.cmp(&b.id)));
        v
    }

    fn push_background(&mut self, step: &mut DriverStep) {
        let list = self.background_list();
        if list != self.reported_background {
            self.reported_background = list.clone();
            step.push(DriverEvent::BackgroundTasks(list));
        }
    }

    fn agent_item(a: &Agent) -> TurnItem {
        let title = a
            .description
            .clone()
            .or_else(|| a.subagent_type.clone())
            .unwrap_or_else(|| "Subagent".into());
        let mut it = item(
            a.item_id.clone(),
            a.status,
            Some(title),
            TurnItemKind::Subagent {
                tool_name: a.tool_name.clone(),
                description: a.description.clone(),
                subagent_type: a.subagent_type.clone(),
                prompt: a.prompt.clone(),
                output: a.output.clone(),
                background: a.background,
                last_tool: a.last_tool.clone(),
                tool_uses: a.tool_uses,
            },
        );
        it.turn_id = Some(a.turn_id.clone());
        it.error = a.error.clone();
        it
    }

    fn emit_agent(&self, step: &mut DriverStep, tool_use_id: &str) {
        if let Some(a) = self.agents.get(tool_use_id) {
            step.push(DriverEvent::TurnItem(Self::agent_item(a)));
        }
    }

    /// A root `Agent` / `Task` tool call: its subagent card.
    pub(super) fn start_agent(
        &mut self,
        step: &mut DriverStep,
        id: &str,
        name: &str,
        input: &Value,
    ) {
        let a = Agent {
            item_id: self.tool_item_id(id),
            turn_id: self.turn(),
            tool_name: name.to_string(),
            description: s(input, "description"),
            subagent_type: s(input, "subagent_type"),
            prompt: s(input, "prompt"),
            output: None,
            error: None,
            background: input.get("run_in_background").and_then(Value::as_bool) == Some(true),
            last_tool: None,
            tool_uses: 0,
            status: TurnItemStatus::Running,
        };
        step.push(DriverEvent::TurnItem(Self::agent_item(&a)));
        self.agents.insert(id.to_string(), a);
    }

    /// A root tool's input asked to run in the background.
    pub(super) fn note_background_input(&mut self, id: &str, name: &str, input: &Value) {
        if input.get("run_in_background").and_then(Value::as_bool) == Some(true) {
            self.bg_inputs.insert(
                id.to_string(),
                (
                    name.to_string(),
                    s(input, "description").or_else(|| s(input, "command")),
                ),
            );
        }
    }

    /// A root tool result. `true` when it was a subagent's (handled here).
    pub(super) fn on_root_tool_result(
        &mut self,
        step: &mut DriverStep,
        id: &str,
        text: &str,
        is_error: bool,
    ) -> bool {
        // Older CLIs: the launch result is the only sign of background work.
        let launched = id_after(text, "Command running in background with ID:")
            .or_else(|| id_after(text, "was moved to the background (ID:"))
            .or_else(|| {
                text.contains("Async agent launched")
                    .then(|| id_after(text, "agentId:"))
                    .flatten()
            });
        if let Some(task_id) = launched {
            self.adopt_launch(id, &task_id);
            self.push_background(step);
        } else if !is_error {
            // A foreground task ended with its tool.
            self.tasks
                .retain(|_, t| t.background || t.tool_use_id.as_deref() != Some(id));
        }
        let Some(a) = self.agents.get_mut(id) else {
            return false;
        };
        let async_launch = text.contains("Async agent launched") || text.contains("agentId:");
        if !is_error && (a.background || async_launch) {
            a.background = true; // runs on; its task notification settles it
        } else {
            a.status = if is_error {
                TurnItemStatus::Failed
            } else {
                TurnItemStatus::Completed
            };
            a.output = Some(text.to_string()).filter(|t| !t.trim().is_empty());
            a.error = is_error.then(|| tool_error(text)).flatten();
        }
        self.emit_agent(step, id);
        if !self.agents.get(id).is_some_and(|a| a.status.is_open()) {
            self.finish_agent(step, id);
        }
        true
    }

    /// Record a launch result's task unless a `task_started` already did.
    fn adopt_launch(&mut self, tool_use_id: &str, task_id: &str) {
        if let Some(t) = self
            .tasks
            .values_mut()
            .find(|t| t.tool_use_id.as_deref() == Some(tool_use_id))
        {
            t.background = true;
            return;
        }
        if self.tasks.contains_key(task_id) || self.finished.iter().any(|f| f == task_id) {
            return;
        }
        let (kind, label) = match self.agents.get(tool_use_id) {
            Some(a) => (
                "agent".to_string(),
                a.description.clone().unwrap_or_else(|| "Subagent".into()),
            ),
            None => (
                "command".to_string(),
                self.bg_inputs
                    .get(tool_use_id)
                    .and_then(|(_, l)| l.clone())
                    .unwrap_or_else(|| "Background command".into()),
            ),
        };
        let item_id = self
            .agents
            .get(tool_use_id)
            .map(|a| a.item_id.clone())
            .unwrap_or_else(|| self.tool_item_id(tool_use_id));
        self.tasks.insert(
            task_id.to_string(),
            Task {
                kind,
                label,
                started_at: Utc::now(),
                tool_use_id: Some(tool_use_id.to_string()),
                item_id: Some(item_id),
                background: true,
            },
        );
    }

    /// `system` `task_*` frames. `true` when handled.
    pub(super) fn on_task_frame(&mut self, step: &mut DriverStep, msg: &Value) -> bool {
        let sub = msg
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(task_id) = s(msg, "task_id") else {
            return sub.starts_with("task_");
        };
        let tool_use_id = s(msg, "tool_use_id");
        match sub {
            "task_started" => self.task_started(step, &task_id, tool_use_id, msg),
            "task_progress" => {
                if let Some(a) = tool_use_id.as_deref().and_then(|t| self.agents.get_mut(t)) {
                    if let Some(tool) = s(msg, "last_tool_name") {
                        a.last_tool = Some(tool);
                    }
                    if let Some(n) = msg.pointer("/usage/tool_uses").and_then(Value::as_u64) {
                        a.tool_uses = a.tool_uses.max(n.min(u32::MAX as u64) as u32);
                    }
                    let id = tool_use_id.clone().unwrap_or_default();
                    self.emit_agent(step, &id);
                }
            }
            "task_updated" => {
                let patch = msg.get("patch").cloned().unwrap_or(Value::Null);
                if patch.get("is_backgrounded").and_then(Value::as_bool) == Some(true) {
                    if let Some(t) = self.tasks.get_mut(&task_id) {
                        t.background = true;
                    }
                    let tu = self.tasks.get(&task_id).and_then(|t| t.tool_use_id.clone());
                    if let Some(a) = tu.as_deref().and_then(|t| self.agents.get_mut(t)) {
                        a.background = true;
                    }
                }
                match patch.get("status").and_then(Value::as_str) {
                    Some(st) if is_terminal(st) => {
                        let summary = s(&patch, "error");
                        self.task_finished(step, &task_id, tool_use_id, st, summary);
                    }
                    _ => self.push_background(step),
                }
            }
            "task_notification" => {
                let st = s(msg, "status").unwrap_or_else(|| "completed".into());
                self.task_finished(step, &task_id, tool_use_id, &st, s(msg, "summary"));
            }
            _ => {}
        }
        true
    }

    fn task_started(
        &mut self,
        step: &mut DriverStep,
        task_id: &str,
        tool_use_id: Option<String>,
        msg: &Value,
    ) {
        if self.finished.iter().any(|f| f == task_id) {
            return;
        }
        let tu = tool_use_id.as_deref();
        // A launch result that arrived first recorded it under the same tool call.
        let earlier = self
            .tasks
            .iter()
            .find(|(_, t)| tu.is_some() && t.tool_use_id.as_deref() == tu)
            .map(|(k, _)| k.clone());
        let earlier = earlier.and_then(|k| self.tasks.remove(&k));
        let asked_bg = tu.is_some_and(|t| self.bg_inputs.contains_key(t))
            || tu
                .and_then(|t| self.agents.get(t))
                .is_some_and(|a| a.background);
        let background = msg.get("is_backgrounded").and_then(Value::as_bool) == Some(true)
            || asked_bg
            || earlier.as_ref().is_some_and(|t| t.background);
        let item_id = match tu {
            Some(t) => Some(
                self.agents
                    .get(t)
                    .map(|a| a.item_id.clone())
                    .or_else(|| self.sub_tools.get(t).map(|x| x.item_id.clone()))
                    .unwrap_or_else(|| self.tool_item_id(t)),
            ),
            None => None,
        };
        let label = s(msg, "description")
            .or_else(|| {
                tu.and_then(|t| self.bg_inputs.get(t))
                    .and_then(|(_, l)| l.clone())
            })
            .or_else(|| s(msg, "subagent_type"))
            .unwrap_or_else(|| "Background task".into());
        if let (Some(a), Some(sub_type)) = (
            tu.and_then(|t| self.agents.get_mut(t)),
            s(msg, "subagent_type"),
        ) {
            a.subagent_type.get_or_insert(sub_type);
        }
        self.tasks.insert(
            task_id.to_string(),
            Task {
                kind: kind_of(msg.get("task_type").and_then(Value::as_str)),
                label,
                started_at: earlier.map(|t| t.started_at).unwrap_or_else(Utc::now),
                tool_use_id,
                item_id,
                background,
            },
        );
        self.push_background(step);
    }

    fn task_finished(
        &mut self,
        step: &mut DriverStep,
        task_id: &str,
        tool_use_id: Option<String>,
        status: &str,
        summary: Option<String>,
    ) {
        let task = self.tasks.remove(task_id);
        if !self.finished.iter().any(|f| f == task_id) {
            self.finished.push(task_id.to_string());
            if self.finished.len() > 256 {
                self.finished.remove(0);
            }
        }
        let tu = tool_use_id.or_else(|| task.as_ref().and_then(|t| t.tool_use_id.clone()));
        let item_status = match status {
            "completed" => TurnItemStatus::Completed,
            "failed" => TurnItemStatus::Failed,
            _ => TurnItemStatus::Interrupted,
        };
        if let Some(id) = tu.as_deref() {
            if let Some(a) = self.agents.get_mut(id) {
                if a.status.is_open() {
                    a.status = item_status;
                    if a.output.is_none() {
                        a.output = summary.clone();
                    }
                    if item_status == TurnItemStatus::Failed {
                        a.error = summary.as_deref().and_then(tool_error);
                    }
                }
                self.emit_agent(step, id);
                self.finish_agent(step, id);
            }
            self.bg_inputs.remove(id);
        }
        if let Some(t) = task.filter(|t| t.background) {
            let what = if t.kind == "agent" {
                "Background agent"
            } else {
                "Background command"
            };
            let verb = match item_status {
                TurnItemStatus::Completed => "finished",
                TurnItemStatus::Failed => "failed",
                _ => "stopped",
            };
            let mut notice = item(
                format!("bg:{task_id}"),
                item_status,
                None,
                TurnItemKind::SystemNotice {
                    message: format!("{what} {verb}: {}", t.label),
                },
            );
            // Placed with the turn that is running, if any (the runtime fills it).
            notice.turn_id = self.turn_id.clone();
            step.push(DriverEvent::TurnItem(notice));
        }
        self.push_background(step);
    }

    /// Drop a settled subagent and settle its still-open tool calls.
    fn finish_agent(&mut self, step: &mut DriverStep, tool_use_id: &str) {
        let Some(a) = self.agents.remove(tool_use_id) else {
            return;
        };
        let mut open: Vec<String> = self
            .sub_tools
            .iter()
            .filter(|(_, t)| t.parent == tool_use_id)
            .map(|(k, _)| k.clone())
            .collect();
        open.sort();
        for k in open {
            if let Some(mut t) = self.sub_tools.remove(&k) {
                t.tool.status = if a.status == TurnItemStatus::Completed {
                    TurnItemStatus::Completed
                } else {
                    TurnItemStatus::Interrupted
                };
                step.push(DriverEvent::TurnItem(self.sub_tool_item(&t)));
            }
        }
    }

    fn sub_tool_item(&self, t: &SubTool) -> TurnItem {
        let mut it = self.tool_item(&t.item_id, &t.tool);
        it.id = t.item_id.clone();
        it.turn_id = Some(t.turn_id.clone());
        it.parent_item_id = self
            .agents
            .get(&t.parent)
            .map(|a| a.item_id.clone())
            .or_else(|| {
                // The card is gone (settled): its id is still derivable.
                Some(format!("{}:tool:{}", t.turn_id, t.parent))
            });
        it
    }

    /// A subagent's own frame (`parent_tool_use_id` = `parent`).
    pub(super) fn on_subagent_frame(&mut self, step: &mut DriverStep, parent: &str, msg: &Value) {
        let Some(a) = self.agents.get(parent) else {
            return; // not a card we know (a nested subagent, a resumed one)
        };
        let (card, turn_id) = (a.item_id.clone(), a.turn_id.clone());
        let content = msg
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        match msg.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                let mid = msg
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let mut touched = false;
                for (n, block) in content.iter().enumerate() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                            if text.trim().is_empty() {
                                continue;
                            }
                            let mut it = item(
                                format!("{turn_id}:sub:{parent}:{mid}:{n}"),
                                TurnItemStatus::Completed,
                                None,
                                TurnItemKind::AssistantMessage {
                                    message_id: mid.clone(),
                                    text: text.to_string(),
                                    streaming: false,
                                },
                            );
                            it.turn_id = Some(turn_id.clone());
                            it.parent_item_id = Some(card.clone());
                            step.push(DriverEvent::TurnItem(it));
                        }
                        Some("tool_use") => {
                            let id = s(block, "id").unwrap_or_default();
                            let name = s(block, "name").unwrap_or_else(|| "tool".into());
                            let t = SubTool {
                                tool: Tool {
                                    name: name.clone(),
                                    input: block.get("input").cloned().unwrap_or(Value::Null),
                                    output: None,
                                    error: None,
                                    status: TurnItemStatus::Running,
                                },
                                parent: parent.to_string(),
                                item_id: format!("{turn_id}:tool:{id}"),
                                turn_id: turn_id.clone(),
                            };
                            step.push(DriverEvent::TurnItem(self.sub_tool_item(&t)));
                            self.sub_tools.insert(id, t);
                            if let Some(a) = self.agents.get_mut(parent) {
                                a.tool_uses += 1;
                                a.last_tool = Some(name);
                            }
                            touched = true;
                        }
                        _ => {}
                    }
                }
                if touched {
                    self.emit_agent(step, parent);
                }
            }
            Some("user") => {
                for block in content {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = s(&block, "tool_use_id").unwrap_or_default();
                    let Some(mut t) = self.sub_tools.remove(&id) else {
                        continue;
                    };
                    let text = result_text(block.get("content"));
                    if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                        t.tool.status = TurnItemStatus::Failed;
                        t.tool.error = tool_error(&text);
                    } else {
                        t.tool.status = TurnItemStatus::Completed;
                    }
                    // A subagent's tool output can be large; the card is a summary.
                    let (ty, _) = classify(&t.tool.name);
                    if ty != ToolType::FileChange {
                        t.tool.output = Some(clip(&text, 4096));
                    }
                    step.push(DriverEvent::TurnItem(self.sub_tool_item(&t)));
                }
            }
            _ => {}
        }
    }

    /// The item a subagent's permission request gates, when it is one of its tools.
    pub(super) fn sub_tool_item_id(&self, tool_use_id: &str) -> Option<String> {
        self.sub_tools.get(tool_use_id).map(|t| t.item_id.clone())
    }

    /// A turn ended: forget its foreground tasks (background ones run on).
    pub(super) fn end_turn_tasks(&mut self, step: &mut DriverStep) {
        self.tasks.retain(|_, t| t.background);
        let fg: Vec<String> = self
            .agents
            .iter()
            .filter(|(_, a)| !a.background)
            .map(|(k, _)| k.clone())
            .collect();
        for k in fg {
            self.finish_agent(step, &k);
        }
        self.push_background(step);
    }

    pub(super) fn clear_tasks(&mut self) {
        self.tasks.clear();
        self.agents.clear();
        self.sub_tools.clear();
        self.bg_inputs.clear();
        self.reported_background.clear();
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

pub(super) type BgInputs = HashMap<String, (String, Option<String>)>;
