//! Subagent classification, child `parent_item_id` routing, and background
//! task / `TaskSettlement` lifecycle helpers for `PiDriver`.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use otto_core::agent_runtime::{
    BackgroundTask, ProviderFailure, TaskSettlement, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus,
};
use serde_json::Value;

use crate::driver::{item, tool_error, DriverError, DriverEvent, DriverStep};

use super::{PiBgEntry, PiDriver, Tool};

#[derive(serde::Serialize, serde::Deserialize)]
struct PiDriverState {
    next_id: u64,
    state_req: Option<String>,
    prompt_req: Option<String>,
    steer_reqs: Vec<String>,
    ready: bool,
    turn_id: Option<String>,
    auto_turns: u64,
    message: Option<(String, String, DateTime<Utc>)>,
    #[serde(default)]
    message_parent: Option<String>,
    thought: Option<(String, String, DateTime<Utc>)>,
    #[serde(default)]
    thought_parent: Option<String>,
    messages: u32,
    tools: HashMap<String, Tool>,
    #[serde(default)]
    subagent_order: Vec<String>,
    settles: bool,
    ended: Option<(TurnTerminalStatus, Option<ProviderFailure>)>,
    #[serde(default)]
    bg_tasks: Vec<BackgroundTask>,
    #[serde(default)]
    bg_entries: HashMap<String, PiBgEntry>,
}

impl PiDriver {
    pub(super) fn bare_tool_name(name: &str) -> String {
        let lower = name.trim().to_ascii_lowercase();
        lower
            .rsplit_once("__")
            .map(|(_, tail)| tail.to_string())
            .unwrap_or(lower)
    }

    pub(super) fn is_subagent_tool(name: &str, args: &Value) -> bool {
        let bare = Self::bare_tool_name(name);
        matches!(
            bare.as_str(),
            "subagent"
                | "task"
                | "delegate"
                | "delegate_to_agent"
                | "spawn_agent"
                | "spawn_subagent"
                | "invoke_subagent"
                | "invoke_agent"
                | "codebase_investigator"
        ) || args.get("subagent_type").is_some()
            || args.get("subagentType").is_some()
            || args.get("agent_type").is_some()
            || args.get("agentType").is_some()
            || args.get("TypeName").is_some()
    }

    pub(super) fn is_timer_tool(name: &str, args: &Value) -> bool {
        let bare = Self::bare_tool_name(name);
        matches!(bare.as_str(), "schedule" | "timer" | "set_timer")
            || args.get("DurationSeconds").is_some()
            || args.get("duration_seconds").is_some()
    }

    pub(super) fn is_background_call(name: &str, args: &Value) -> bool {
        args.get("run_in_background").and_then(Value::as_bool) == Some(true)
            || args.get("background").and_then(Value::as_bool) == Some(true)
            || Self::is_timer_tool(name, args)
    }

    pub(super) fn extract_due_at(
        name: &str,
        args: &Value,
        started_at: DateTime<Utc>,
    ) -> Option<DateTime<Utc>> {
        if !Self::is_timer_tool(name, args) {
            return None;
        }
        let secs = args
            .get("DurationSeconds")
            .or_else(|| args.get("duration_seconds"))
            .or_else(|| args.get("delay_seconds"))
            .and_then(|v| v.as_i64().or_else(|| v.as_u64().map(|u| u as i64)))
            .filter(|s| *s > 0)?;
        Some(started_at + Duration::seconds(secs))
    }

    pub(super) fn arg_str(args: &Value, keys: &[&str]) -> Option<String> {
        for k in keys {
            if let Some(s) = args
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

    pub(super) fn first_line_truncated(s: &str) -> Option<String> {
        let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
        let mut out: String = line.chars().take(80).collect();
        if line.chars().count() > 80 {
            out.push('…');
        }
        Some(out)
    }

    pub(super) fn subagent_fields(
        args: &Value,
    ) -> (Option<String>, Option<String>, Option<String>, bool) {
        let prompt = Self::arg_str(args, &["prompt", "Prompt", "task", "message", "objective"]);
        let subagent_type = Self::arg_str(
            args,
            &[
                "subagent_type",
                "subagentType",
                "agent_type",
                "agentType",
                "TypeName",
                "agent",
                "role",
            ],
        );
        let description = Self::arg_str(
            args,
            &["description", "Description", "Role", "name", "title"],
        )
        .or_else(|| prompt.as_deref().and_then(Self::first_line_truncated));
        let background = args.get("run_in_background").and_then(Value::as_bool) == Some(true)
            || args.get("background").and_then(Value::as_bool) == Some(true);
        (description, subagent_type, prompt, background)
    }

    pub(super) fn bg_kind_and_label(name: &str, args: &Value) -> (&'static str, String) {
        if Self::is_timer_tool(name, args) {
            let label = Self::arg_str(args, &["Prompt", "prompt", "description", "label"])
                .and_then(|s| Self::first_line_truncated(&s))
                .unwrap_or_else(|| name.to_string());
            ("timer", label)
        } else if Self::is_subagent_tool(name, args) {
            let (description, subagent_type, _, _) = Self::subagent_fields(args);
            let label = description
                .or(subagent_type)
                .unwrap_or_else(|| name.to_string());
            ("agent", label)
        } else if name == "bash" {
            let label = args
                .get("description")
                .and_then(Value::as_str)
                .and_then(Self::first_line_truncated)
                .or_else(|| {
                    args.get("command")
                        .and_then(Value::as_str)
                        .and_then(Self::first_line_truncated)
                })
                .unwrap_or_else(|| "bash".into());
            ("command", label)
        } else {
            ("command", name.to_string())
        }
    }

    pub(super) fn extract_explicit_parent_id(v: &Value, args: Option<&Value>) -> Option<String> {
        const KEYS: &[&str] = &[
            "parentToolCallId",
            "parent_tool_call_id",
            "parentToolUseId",
            "parent_tool_use_id",
            "parentCallId",
            "parent_call_id",
            "subagentCallId",
            "subagent_call_id",
            "subagentId",
            "subagent_id",
        ];
        if let Some(s) = Self::arg_str(v, KEYS) {
            return Some(s);
        }
        if let Some(ev) = v.get("assistantMessageEvent") {
            if let Some(s) = Self::arg_str(ev, KEYS) {
                return Some(s);
            }
        }
        if let Some(meta) = v.get("_meta") {
            if let Some(s) = Self::arg_str(meta, KEYS) {
                return Some(s);
            }
        }
        if let Some(a) = args {
            if let Some(s) = Self::arg_str(
                a,
                &[
                    "parentToolCallId",
                    "parent_tool_call_id",
                    "parentToolUseId",
                    "parent_tool_use_id",
                ],
            ) {
                return Some(s);
            }
            if let Some(meta) = a.get("_meta") {
                if let Some(s) = Self::arg_str(meta, KEYS) {
                    return Some(s);
                }
            }
        }
        None
    }

    /// Resolve `(parent_key, parent_item_id)` for a child tool/message/thought.
    pub(super) fn resolve_parent_subagent(
        &self,
        v: &Value,
        args: Option<&Value>,
        current_call: Option<&str>,
        is_self_subagent: bool,
    ) -> Option<(String, String)> {
        if let Some(explicit) = Self::extract_explicit_parent_id(v, args) {
            if current_call != Some(explicit.as_str()) {
                if let Some(t) = self.tools.get(&explicit) {
                    if t.is_subagent {
                        let item_id = if t.item_id.is_empty() {
                            format!(
                                "{}:tool:{explicit}",
                                t.turn_id.as_deref().unwrap_or(self.turn())
                            )
                        } else {
                            t.item_id.clone()
                        };
                        return Some((explicit, item_id));
                    }
                }
                if let Some((bg_key, entry)) = self.bg_entries.iter().find(|(k, e)| {
                    *k == &explicit || e.call_id == explicit || e.item_id == explicit
                }) {
                    if entry.kind == "agent" {
                        return Some((bg_key.clone(), entry.item_id.clone()));
                    }
                }
                if explicit.contains(":tool:") {
                    return Some((explicit.clone(), explicit));
                }
                return Some((explicit.clone(), format!("{}:tool:{explicit}", self.turn())));
            }
        }
        if is_self_subagent {
            return None;
        }
        for sub_call in self.subagent_order.iter().rev() {
            if current_call == Some(sub_call.as_str()) {
                continue;
            }
            if let Some(t) = self.tools.get(sub_call) {
                if t.is_subagent && t.bg_id.is_none() {
                    let item_id = if t.item_id.is_empty() {
                        format!(
                            "{}:tool:{sub_call}",
                            t.turn_id.as_deref().unwrap_or(self.turn())
                        )
                    } else {
                        t.item_id.clone()
                    };
                    return Some((sub_call.clone(), item_id));
                }
            }
        }
        None
    }

    pub(super) fn record_child_tool_on_parent(
        &mut self,
        step: &mut DriverStep,
        parent_key: &str,
        child_tool_name: &str,
    ) {
        if let Some(parent) = self.tools.get_mut(parent_key) {
            if parent.is_subagent {
                parent.tool_uses = parent.tool_uses.saturating_add(1);
                parent.last_tool = Some(child_tool_name.to_string());
                let item_id = if parent.item_id.is_empty() {
                    format!(
                        "{}:tool:{parent_key}",
                        parent.turn_id.as_deref().unwrap_or_default()
                    )
                } else {
                    parent.item_id.clone()
                };
                let mut ti = item(
                    item_id,
                    TurnItemStatus::Running,
                    Self::tool_title(&parent.name, &parent.args),
                    Self::tool_kind(
                        &parent.name,
                        &parent.args,
                        None,
                        parent.tool_uses,
                        parent.last_tool.clone(),
                    ),
                );
                ti.turn_id = parent.turn_id.clone();
                ti.started_at = Some(parent.started_at);
                ti.parent_item_id = parent.parent_item_id.clone();
                step.push(DriverEvent::TurnItem(ti));
                return;
            }
        }
        if let Some(entry) = self.bg_entries.get_mut(parent_key) {
            if entry.kind == "agent" {
                entry.tool_uses = entry.tool_uses.saturating_add(1);
                entry.last_tool = Some(child_tool_name.to_string());
                let mut ti = item(
                    entry.item_id.clone(),
                    TurnItemStatus::Running,
                    Self::tool_title(&entry.name, &entry.args),
                    Self::tool_kind(
                        &entry.name,
                        &entry.args,
                        None,
                        entry.tool_uses,
                        entry.last_tool.clone(),
                    ),
                );
                ti.turn_id = entry.turn_id.clone();
                ti.started_at = Some(entry.started_at);
                step.push(DriverEvent::TurnItem(ti));
            }
        }
    }

    pub(super) fn settle_foreground_tools_on_finish(
        &mut self,
        step: &mut DriverStep,
        terminal: TurnTerminalStatus,
    ) {
        let item_status = match terminal {
            TurnTerminalStatus::Interrupted => TurnItemStatus::Interrupted,
            TurnTerminalStatus::Cancelled => TurnItemStatus::Cancelled,
            TurnTerminalStatus::Failed => TurnItemStatus::Failed,
            TurnTerminalStatus::Completed => TurnItemStatus::Completed,
        };
        let open_bg_agent_items: Vec<String> = self
            .bg_entries
            .values()
            .filter(|e| e.kind == "agent")
            .map(|e| e.item_id.clone())
            .collect();
        let mut keep: std::collections::HashMap<String, Tool> = std::collections::HashMap::new();
        let now = Utc::now();
        for (call, tool) in std::mem::take(&mut self.tools) {
            let under_open_bg_subagent = tool
                .parent_item_id
                .as_ref()
                .is_some_and(|pid| open_bg_agent_items.iter().any(|b| b == pid));
            if (tool.bg_id.is_some() || under_open_bg_subagent)
                && terminal != TurnTerminalStatus::Failed
            {
                keep.insert(call, tool);
                continue;
            }
            let turn_str = tool.turn_id.as_deref().unwrap_or_default();
            let item_id = if tool.item_id.is_empty() {
                format!("{turn_str}:tool:{call}")
            } else {
                tool.item_id.clone()
            };
            let output = (!tool.streamed_output.is_empty()).then_some(tool.streamed_output);
            let mut ti = item(
                item_id,
                item_status,
                Self::tool_title(&tool.name, &tool.args),
                Self::tool_kind(
                    &tool.name,
                    &tool.args,
                    output,
                    tool.tool_uses,
                    tool.last_tool,
                ),
            );
            ti.turn_id = tool.turn_id;
            ti.started_at = Some(tool.started_at);
            ti.completed_at = Some(now);
            ti.parent_item_id = tool.parent_item_id;
            step.push(DriverEvent::TurnItem(ti));
        }
        self.subagent_order.retain(|c| keep.contains_key(c));
        self.tools = keep;
    }

    pub(super) fn settle_bg_entry_inner(
        entry: PiBgEntry,
        bg_id: &str,
        status: TurnItemStatus,
        output: Option<String>,
        emit_card: bool,
        fallback_turn_id: Option<String>,
        step: &mut DriverStep,
    ) {
        let now = Utc::now();
        if emit_card {
            let mut it = item(
                entry.item_id.clone(),
                status,
                Self::tool_title(&entry.name, &entry.args),
                Self::tool_kind(
                    &entry.name,
                    &entry.args,
                    output.clone(),
                    entry.tool_uses,
                    entry.last_tool.clone(),
                ),
            );
            it.turn_id = entry.turn_id.clone();
            it.started_at = Some(entry.started_at);
            it.completed_at = Some(now);
            if status == TurnItemStatus::Failed {
                it.error = output.as_deref().and_then(tool_error);
            }
            step.push(DriverEvent::TurnItem(it));
        }
        let (verb, settlement_status) = match status {
            TurnItemStatus::Failed => ("failed", "failed"),
            TurnItemStatus::Cancelled | TurnItemStatus::Interrupted => ("stopped", "stopped"),
            _ => ("finished", "completed"),
        };
        let notice_status = match status {
            TurnItemStatus::Failed => TurnItemStatus::Failed,
            TurnItemStatus::Cancelled | TurnItemStatus::Interrupted => TurnItemStatus::Cancelled,
            _ => TurnItemStatus::Completed,
        };
        let mut notice = item(
            format!("bg:{bg_id}"),
            notice_status,
            None,
            TurnItemKind::SystemNotice {
                message: format!("Background {} {verb}: {}", entry.kind, entry.label),
                task_settlement: Some(TaskSettlement {
                    task_id: bg_id.to_string(),
                    kind: entry.kind,
                    status: settlement_status.to_string(),
                    label: entry.label,
                }),
            },
        );
        notice.turn_id = entry.turn_id.or(fallback_turn_id);
        notice.completed_at = Some(now);
        step.push(DriverEvent::TurnItem(notice));
    }

    pub(super) fn clear_all_bg_on_session_close(&mut self, step: &mut DriverStep) {
        let keys: Vec<String> = self.bg_entries.keys().cloned().collect();
        for key in keys {
            self.settle_bg_entry(step, &key, TurnItemStatus::Interrupted, None, true);
        }
        if !self.bg_tasks.is_empty() {
            self.bg_tasks.clear();
            step.push(DriverEvent::BackgroundTasks(Vec::new()));
        }
        self.bg_entries.clear();
    }

    pub(super) fn snapshot_driver_state(&self) -> Value {
        serde_json::to_value(PiDriverState {
            next_id: self.next_id,
            state_req: self.state_req.clone(),
            prompt_req: self.prompt_req.clone(),
            steer_reqs: self.steer_reqs.clone(),
            ready: self.ready,
            turn_id: self.turn_id.clone(),
            auto_turns: self.auto_turns,
            message: self.message.clone(),
            message_parent: self.message_parent.clone(),
            thought: self.thought.clone(),
            thought_parent: self.thought_parent.clone(),
            messages: self.messages,
            tools: self.tools.clone(),
            subagent_order: self.subagent_order.clone(),
            settles: self.settles,
            ended: self.ended.clone(),
            bg_tasks: self.bg_tasks.clone(),
            bg_entries: self.bg_entries.clone(),
        })
        .unwrap_or(Value::Null)
    }

    pub(super) fn restore_driver_state(&mut self, state: &Value) -> Result<(), DriverError> {
        if state.is_null() {
            return Ok(());
        }
        let s: PiDriverState = serde_json::from_value(state.clone())
            .map_err(|e| DriverError(format!("invalid pi driver state: {e}")))?;
        self.next_id = s.next_id;
        self.state_req = s.state_req;
        self.prompt_req = s.prompt_req;
        self.steer_reqs = s.steer_reqs;
        self.ready = s.ready;
        self.turn_id = s.turn_id;
        self.auto_turns = s.auto_turns;
        self.message = s.message;
        self.message_parent = s.message_parent;
        self.thought = s.thought;
        self.thought_parent = s.thought_parent;
        self.messages = s.messages;
        self.tools = s.tools;
        self.subagent_order = s.subagent_order;
        self.settles = s.settles;
        self.ended = s.ended;
        self.bg_tasks = s.bg_tasks;
        self.bg_entries = s.bg_entries;
        Ok(())
    }
}
