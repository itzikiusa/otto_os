//! Subagent & background-task tracking for [`super::CodexDriver`].
//!
//! Maps Codex `collabAgentToolCall` items (`spawnAgent`, `sendInput`, `wait`,
//! `closeAgent`, `resumeAgent`) onto [`TurnItemKind::Subagent`] +
//! [`BackgroundTask`], and routes notifications carrying a spawned child
//! `threadId` (`turn/started`, `item/started`, `item/completed`,
//! `item/agentMessage/delta`, `item/reasoning/*Delta`,
//! `item/commandExecution/outputDelta`, `subAgentActivity`, `turn/completed`)
//! onto child [`TurnItem`]s with `parent_item_id = Some(subagent_item_id)`.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use otto_core::agent_runtime::{
    BackgroundTask, TaskSettlement, TurnItem, TurnItemKind, TurnItemStatus, TurnTerminalStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{project_codex_item, CodexDriver};
use crate::driver::{item, DriverEvent, DriverStep};

#[path = "codex_subagent_parse.rs"]
mod parse;

use self::parse::{
    child_tool_label, extract_agent_states, extract_collab_error, extract_target_thread_ids,
    first_line_label, item_status_is_terminal, normalize_collab_tool, parse_status_str, str_any,
    CollabKind,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CodexSubagent {
    pub key: String,
    pub item_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub parent_item_id: Option<String>,
    #[serde(default)]
    pub child_thread_id: Option<String>,
    pub tool_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subagent_type: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub tool_uses: u32,
    #[serde(default)]
    pub last_tool: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub last_message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub status: TurnItemStatus,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub seen_child_tools: HashSet<String>,
    #[serde(default)]
    pub settled: bool,
    #[serde(default)]
    pub settlement_seq: u32,
    /// The child thread's running turn (`turn/started`), so a person's
    /// Stop can `turn/interrupt` it.
    #[serde(default)]
    pub child_turn_id: Option<String>,
}

impl CodexSubagent {
    pub fn to_turn_item(&self) -> TurnItem {
        let mut ti = item(
            self.item_id.clone(),
            self.status,
            self.description.clone().or_else(|| Some("Subagent".into())),
            TurnItemKind::Subagent {
                tool_name: self.tool_name.clone(),
                description: self.description.clone(),
                subagent_type: self.subagent_type.clone(),
                prompt: self.prompt.clone(),
                background: true,
                tool_uses: self.tool_uses,
                last_tool: self.last_tool.clone(),
                output: self.output.clone(),
            },
        );
        ti.turn_id = self.turn_id.clone();
        ti.parent_item_id = self.parent_item_id.clone();
        ti.started_at = Some(self.started_at);
        ti.completed_at = if self.status.is_open() {
            None
        } else {
            Some(self.completed_at.unwrap_or_else(Utc::now))
        };
        ti.error = self.error.clone();
        ti
    }

    pub fn bg_id(&self) -> String {
        self.child_thread_id
            .clone()
            .unwrap_or_else(|| self.key.clone())
    }

    pub fn label(&self) -> String {
        self.description
            .as_deref()
            .or(self.subagent_type.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| "Subagent".to_string())
    }
}

impl CodexDriver {
    pub(crate) fn is_child_thread(&self, thread_id: &str) -> bool {
        !thread_id.is_empty()
            && self.thread_id.as_deref() != Some(thread_id)
            && self.child_thread_to_key.contains_key(thread_id)
    }

    pub(crate) fn item_id_for_thread(&self, thread_id: Option<&str>, cid: &str) -> String {
        if let Some(tid) = thread_id.filter(|t| self.is_child_thread(t)) {
            if let Some(key) = self.child_thread_to_key.get(tid) {
                if let Some(card) = self.subagents.get(key) {
                    return format!("{}:child:{cid}", card.item_id);
                }
            }
        }
        self.item_id(cid)
    }

    pub(crate) fn on_collab_item(
        &mut self,
        step: &mut DriverStep,
        it: &Value,
        completed: bool,
        parent_item_id: Option<String>,
    ) {
        let Some(cid) = it.get("id").and_then(Value::as_str) else {
            return;
        };
        let raw_tool = it
            .get("tool")
            .or_else(|| it.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("spawnAgent");
        let kind = normalize_collab_tool(raw_tool);
        match kind {
            CollabKind::Spawn => self.on_collab_spawn(step, cid, it, completed, parent_item_id),
            CollabKind::Close => {
                if completed || item_status_is_terminal(it) {
                    self.on_collab_close(step, it);
                }
            }
            CollabKind::Wait => {
                self.on_collab_wait(step, it, completed);
            }
            CollabKind::SendInput | CollabKind::Resume => {
                self.on_collab_resume_or_send(step, it, completed);
            }
        }
    }

    fn on_collab_spawn(
        &mut self,
        step: &mut DriverStep,
        cid: &str,
        it: &Value,
        completed: bool,
        parent_item_id: Option<String>,
    ) {
        let child_ids = extract_target_thread_ids(it);
        let child_tid = child_ids.first().cloned();
        let prompt = str_any(it, &["prompt", "message"]);
        let subagent_type = str_any(
            it,
            &[
                "role",
                "agentRole",
                "agent_role",
                "subagentType",
                "subagent_type",
                "agentType",
                "agent_type",
                "model",
            ],
        );
        let description = str_any(
            it,
            &[
                "description",
                "name",
                "title",
                "role",
                "agentRole",
                "agent_role",
            ],
        )
        .or_else(|| prompt.as_deref().and_then(first_line_label));

        let item_status = it.get("status").and_then(Value::as_str);
        let spawn_failed = matches!(item_status, Some("failed" | "declined"));

        let item_id = match &parent_item_id {
            Some(pid) => format!("{pid}:child:{cid}"),
            None => self.item_id(cid),
        };
        let turn_id = self.turn_id.clone();

        let card = self
            .subagents
            .entry(cid.to_string())
            .or_insert_with(|| CodexSubagent {
                key: cid.to_string(),
                item_id: item_id.clone(),
                turn_id: turn_id.clone(),
                parent_item_id: parent_item_id.clone(),
                child_thread_id: child_tid.clone(),
                tool_name: "spawn_agent".into(),
                description: description.clone(),
                subagent_type: subagent_type.clone(),
                prompt: prompt.clone(),
                tool_uses: 0,
                last_tool: None,
                output: None,
                last_message: None,
                error: None,
                status: TurnItemStatus::Running,
                started_at: Utc::now(),
                completed_at: None,
                seen_child_tools: HashSet::new(),
                settled: false,
                settlement_seq: 0,
                child_turn_id: None,
            });
        if card.description.is_none() && description.is_some() {
            card.description = description;
        }
        if card.subagent_type.is_none() && subagent_type.is_some() {
            card.subagent_type = subagent_type;
        }
        if card.prompt.is_none() && prompt.is_some() {
            card.prompt = prompt;
        }

        let prev_bg_id = card.bg_id();
        if let Some(tid) = &child_tid {
            card.child_thread_id = Some(tid.clone());
            self.child_thread_to_key
                .insert(tid.clone(), cid.to_string());
        }
        let bg_id = card.bg_id();
        let label = card.label();
        let started_at = card.started_at;
        let card_item_id = card.item_id.clone();

        if spawn_failed {
            let status = if item_status == Some("declined") {
                TurnItemStatus::Cancelled
            } else {
                TurnItemStatus::Failed
            };
            card.status = status;
            card.settled = true;
            card.completed_at = Some(Utc::now());
            card.error = extract_collab_error(it);
            let ti = card.to_turn_item();
            step.push(DriverEvent::TurnItem(ti));
            if self.remove_bg_task_ids(&[prev_bg_id, bg_id]) {
                step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
            }
            return;
        }

        // Check whether `agentsStates` already reports a terminal state for the spawned child.
        let states = extract_agent_states(it);
        if let Some(tid) = &child_tid {
            if let Some(st) = states.get(tid) {
                if let Some(term_status) = st.status.filter(|s| {
                    matches!(
                        s,
                        TurnItemStatus::Completed
                            | TurnItemStatus::Failed
                            | TurnItemStatus::Cancelled
                    )
                }) {
                    step.push(DriverEvent::TurnItem(card.to_turn_item()));
                    self.settle_subagent_by_key(
                        step,
                        cid,
                        term_status,
                        st.message.clone(),
                        (term_status == TurnItemStatus::Failed)
                            .then(|| st.message.clone())
                            .flatten(),
                    );
                    return;
                }
            }
        }

        // Keep `Subagent` Running across the `spawnAgent` RPC return until the child settles.
        let _ = completed;
        let ti = card.to_turn_item();
        step.push(DriverEvent::TurnItem(ti));

        // Update or insert BackgroundTask (promoting provisional `cid` -> `child_tid` if needed).
        if prev_bg_id != bg_id {
            self.bg_tasks.retain(|t| t.id != prev_bg_id);
        }
        if let Some(existing) = self.bg_tasks.iter_mut().find(|t| t.id == bg_id) {
            existing.label = label;
            existing.item_id = Some(card_item_id);
        } else {
            self.bg_tasks.push(BackgroundTask {
                id: bg_id,
                kind: "agent".into(),
                label,
                started_at,
                item_id: Some(card_item_id),
                due_at: None,
                stopping: false,
                stop_error: None,
            });
        }
        step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
    }

    fn on_collab_close(&mut self, step: &mut DriverStep, it: &Value) {
        let targets = extract_target_thread_ids(it);
        for tid in targets {
            if let Some(key) = self.child_thread_to_key.get(&tid).cloned() {
                self.settle_subagent_by_key(step, &key, TurnItemStatus::Cancelled, None, None);
            }
        }
    }

    fn on_collab_wait(&mut self, step: &mut DriverStep, it: &Value, completed: bool) {
        let states = extract_agent_states(it);
        for (tid, st) in states {
            let Some(key) = self.child_thread_to_key.get(&tid).cloned() else {
                continue;
            };
            match st.status {
                Some(
                    term @ (TurnItemStatus::Completed
                    | TurnItemStatus::Failed
                    | TurnItemStatus::Cancelled),
                ) => {
                    let err = (term == TurnItemStatus::Failed)
                        .then(|| st.message.clone())
                        .flatten();
                    self.settle_subagent_by_key(step, &key, term, st.message, err);
                }
                _ if completed && it.get("status").and_then(Value::as_str) == Some("completed") => {
                    if let Some(msg) = st.message {
                        self.settle_subagent_by_key(
                            step,
                            &key,
                            TurnItemStatus::Completed,
                            Some(msg),
                            None,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn on_collab_resume_or_send(&mut self, step: &mut DriverStep, it: &Value, completed: bool) {
        if !completed
            || matches!(
                it.get("status").and_then(Value::as_str),
                Some("failed" | "declined")
            )
        {
            return;
        }
        for tid in extract_target_thread_ids(it) {
            let Some(key) = self.child_thread_to_key.get(&tid).cloned() else {
                continue;
            };
            let Some(card) = self.subagents.get_mut(&key) else {
                continue;
            };
            if card.settled {
                card.settled = false;
                card.status = TurnItemStatus::Running;
                card.completed_at = None;
                card.error = None;
                let bg_id = card.bg_id();
                let label = card.label();
                let started_at = card.started_at;
                let item_id = card.item_id.clone();
                let ti = card.to_turn_item();
                step.push(DriverEvent::TurnItem(ti));
                if !self.bg_tasks.iter().any(|t| t.id == bg_id) {
                    self.bg_tasks.push(BackgroundTask {
                        id: bg_id,
                        kind: "agent".into(),
                        label,
                        started_at,
                        item_id: Some(item_id),
                        due_at: None,
                        stopping: false,
                        stop_error: None,
                    });
                    step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
                }
            }
        }
    }

    pub(crate) fn on_child_notification(
        &mut self,
        step: &mut DriverStep,
        child_tid: &str,
        method: &str,
        params: &Value,
    ) {
        let Some(key) = self.child_thread_to_key.get(child_tid).cloned() else {
            return;
        };
        match method {
            "turn/started" => {
                if let Some(card) = self.subagents.get_mut(&key) {
                    if let Some(t) = params.pointer("/turn/id").and_then(Value::as_str) {
                        card.child_turn_id = Some(t.to_string());
                    }
                    if card.settled {
                        card.settled = false;
                        card.status = TurnItemStatus::Running;
                        card.completed_at = None;
                        card.error = None;
                        let bg_id = card.bg_id();
                        let label = card.label();
                        let started_at = card.started_at;
                        let item_id = card.item_id.clone();
                        step.push(DriverEvent::TurnItem(card.to_turn_item()));
                        if !self.bg_tasks.iter().any(|t| t.id == bg_id) {
                            self.bg_tasks.push(BackgroundTask {
                                id: bg_id,
                                kind: "agent".into(),
                                label,
                                started_at,
                                item_id: Some(item_id),
                                due_at: None,
                                stopping: false,
                                stop_error: None,
                            });
                            step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
                        }
                    }
                }
            }
            "item/started" | "item/completed" => {
                let Some(raw_item) = params.get("item") else {
                    return;
                };
                let is_completed = method == "item/completed";
                let ty = raw_item.get("type").and_then(Value::as_str).unwrap_or("");
                if matches!(ty, "collabAgentToolCall" | "collabToolCall") {
                    let parent_id = self.subagents.get(&key).map(|c| c.item_id.clone());
                    self.on_collab_item(step, raw_item, is_completed, parent_id);
                    return;
                }
                let (card_item_id, card_turn_id) = match self.subagents.get(&key) {
                    Some(c) => (c.item_id.clone(), c.turn_id.clone()),
                    None => return,
                };
                let cid = raw_item
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let child_map = self.child_items.entry(child_tid.to_string()).or_default();
                let projected = project_codex_item(
                    child_map,
                    &format!("{card_item_id}:child"),
                    raw_item,
                    is_completed,
                );
                if let Some(mut ti) = projected {
                    ti.parent_item_id = Some(card_item_id);
                    ti.turn_id = card_turn_id;

                    let mut parent_updated = None;
                    if let Some(card) = self.subagents.get_mut(&key) {
                        if let TurnItemKind::AssistantMessage { text, .. } = &ti.kind {
                            if !text.trim().is_empty() {
                                card.last_message = Some(text.clone());
                            }
                        } else if !matches!(
                            ti.kind,
                            TurnItemKind::Reasoning { .. } | TurnItemKind::UserMessage { .. }
                        ) && !cid.is_empty()
                            && card.seen_child_tools.insert(cid)
                        {
                            card.tool_uses = card.tool_uses.saturating_add(1);
                            card.last_tool = child_tool_label(raw_item, &ti);
                            parent_updated = Some(card.to_turn_item());
                        }
                    }
                    step.push(DriverEvent::TurnItem(ti));
                    if let Some(parent_ti) = parent_updated {
                        step.push(DriverEvent::TurnItem(parent_ti));
                    }
                }
            }
            "item/agentMessage/delta"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/textDelta" => {
                let reasoning = method != "item/agentMessage/delta";
                let Some(cid) = params.get("itemId").and_then(Value::as_str) else {
                    return;
                };
                let delta = params
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let (card_item_id, card_turn_id) = match self.subagents.get(&key) {
                    Some(c) => (c.item_id.clone(), c.turn_id.clone()),
                    None => return,
                };
                let child_map = self.child_items.entry(child_tid.to_string()).or_default();
                let st = child_map.entry(cid.to_string()).or_default();
                let started_at = st.started_at;
                st.text.push_str(delta);
                let text = st.text.clone();
                if !reasoning && !text.trim().is_empty() {
                    if let Some(card) = self.subagents.get_mut(&key) {
                        card.last_message = Some(text.clone());
                    }
                }
                let kind = if reasoning {
                    TurnItemKind::Reasoning {
                        text,
                        streaming: true,
                    }
                } else {
                    TurnItemKind::AssistantMessage {
                        message_id: cid.to_string(),
                        text,
                        streaming: true,
                    }
                };
                let mut ti = item(
                    format!("{card_item_id}:child:{cid}"),
                    TurnItemStatus::Running,
                    None,
                    kind,
                );
                ti.started_at = Some(started_at);
                ti.parent_item_id = Some(card_item_id);
                ti.turn_id = card_turn_id;
                step.push(DriverEvent::TurnItem(ti));
            }
            "item/commandExecution/outputDelta" => {
                let (Some(cid), Some(d)) = (
                    params.get("itemId").and_then(Value::as_str),
                    params.get("delta").and_then(Value::as_str),
                ) else {
                    return;
                };
                let Some(card_item_id) = self.subagents.get(&key).map(|c| c.item_id.clone()) else {
                    return;
                };
                self.child_items
                    .entry(child_tid.to_string())
                    .or_default()
                    .entry(cid.to_string())
                    .or_default()
                    .output
                    .push_str(d);
                step.push(DriverEvent::CommandOutput {
                    item_id: format!("{card_item_id}:child:{cid}"),
                    chunk: d.to_string(),
                });
            }
            "turn/completed" => {
                let turn = params.get("turn").cloned().unwrap_or(Value::Null);
                let (status, err) = match turn.get("status").and_then(Value::as_str) {
                    Some("completed") => (TurnItemStatus::Completed, None),
                    Some("interrupted") => (TurnItemStatus::Cancelled, None),
                    Some("failed") => (
                        TurnItemStatus::Failed,
                        turn.pointer("/error/message")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    ),
                    _ => return,
                };
                self.child_items.remove(child_tid);
                if let Some(card) = self.subagents.get_mut(&key) {
                    card.child_turn_id = None;
                }
                self.settle_subagent_by_key(step, &key, status, None, err);
            }
            "subAgentActivity" | "subagent/activity" => {
                self.on_subagent_activity_payload(step, &key, params);
            }
            _ => {}
        }
    }

    pub(crate) fn on_subagent_activity(&mut self, step: &mut DriverStep, params: &Value) {
        let Some(tid) = str_any(
            params,
            &[
                "threadId",
                "thread_id",
                "agentThreadId",
                "agent_thread_id",
                "subagentThreadId",
            ],
        ) else {
            return;
        };
        let Some(key) = self.child_thread_to_key.get(&tid).cloned() else {
            return;
        };
        self.on_subagent_activity_payload(step, &key, params);
    }

    fn on_subagent_activity_payload(&mut self, step: &mut DriverStep, key: &str, params: &Value) {
        let tool = str_any(
            params,
            &["tool", "toolName", "tool_name", "lastTool", "last_tool"],
        );
        let msg = str_any(
            params,
            &["output", "message", "finalOutput", "final_output"],
        );
        let raw_status = str_any(params, &["status", "state"]);
        if let Some(status) = raw_status.as_deref().and_then(parse_status_str) {
            if matches!(
                status,
                TurnItemStatus::Completed | TurnItemStatus::Failed | TurnItemStatus::Cancelled
            ) {
                let err = (status == TurnItemStatus::Failed)
                    .then(|| msg.clone())
                    .flatten();
                self.settle_subagent_by_key(step, key, status, msg, err);
                return;
            }
        }
        if let Some(card) = self.subagents.get_mut(key) {
            let mut dirty = false;
            if let Some(t) = tool {
                card.tool_uses = card.tool_uses.saturating_add(1);
                card.last_tool = Some(t);
                dirty = true;
            }
            if let Some(m) = msg {
                card.last_message = Some(m);
            }
            if dirty {
                step.push(DriverEvent::TurnItem(card.to_turn_item()));
            }
        }
    }

    pub(crate) fn settle_open_subagents_on_parent_abort(
        &mut self,
        step: &mut DriverStep,
        terminal: TurnTerminalStatus,
    ) {
        let item_status = match terminal {
            TurnTerminalStatus::Failed => TurnItemStatus::Failed,
            TurnTerminalStatus::Interrupted | TurnTerminalStatus::Cancelled => {
                TurnItemStatus::Interrupted
            }
            TurnTerminalStatus::Completed => return,
        };
        // A card whose child thread runs stays open: Otto interrupted that
        // thread's own turn (`interrupt_children`) or the parent failed
        // without touching it, and its own `turn/completed` settles it.
        // Calling it stopped here would hide a child that may still run.
        let open_keys: Vec<String> = self
            .subagents
            .iter()
            .filter(|(_, c)| c.status.is_open() || !c.settled)
            .filter(|(_, c)| c.child_thread_id.is_none() || c.child_turn_id.is_none())
            .map(|(k, _)| k.clone())
            .collect();
        for key in open_keys {
            self.settle_subagent_by_key(step, &key, item_status, None, None);
        }
    }

    fn settle_subagent_by_key(
        &mut self,
        step: &mut DriverStep,
        key: &str,
        status: TurnItemStatus,
        output: Option<String>,
        error: Option<String>,
    ) {
        let (was_settled, seq, bg_id, key_id, label, turn_id, updated_item) = {
            let Some(card) = self.subagents.get_mut(key) else {
                return;
            };
            let was_settled = card.settled;
            if was_settled {
                // Allow a later `wait` result to populate `output` if child `turn/completed`
                // had no assistant message yet, without re-emitting a duplicate SystemNotice.
                if card.output.is_none() && output.is_some() {
                    card.output = output;
                    step.push(DriverEvent::TurnItem(card.to_turn_item()));
                }
                return;
            }
            card.settled = true;
            card.settlement_seq = card.settlement_seq.saturating_add(1);
            let seq = card.settlement_seq;
            card.status = status;
            card.completed_at = Some(Utc::now());
            if let Some(out) = output.or_else(|| card.last_message.clone()) {
                if !out.trim().is_empty() {
                    card.output = Some(out);
                }
            }
            if let Some(err) = error {
                if !err.trim().is_empty() {
                    card.error = Some(err);
                }
            }
            (
                was_settled,
                seq,
                card.bg_id(),
                card.key.clone(),
                card.label(),
                card.turn_id.clone().or_else(|| self.turn_id.clone()),
                card.to_turn_item(),
            )
        };
        let _ = was_settled;
        step.push(DriverEvent::TurnItem(updated_item));
        if self.remove_bg_task_ids(&[bg_id.clone(), key_id]) {
            step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
        }
        let (verb, settlement_status) = match status {
            TurnItemStatus::Failed => ("failed", "failed"),
            TurnItemStatus::Cancelled | TurnItemStatus::Interrupted => ("stopped", "stopped"),
            _ => ("finished", "completed"),
        };
        let notice_id = if seq <= 1 {
            format!("bg:{bg_id}")
        } else {
            format!("bg:{bg_id}:{seq}")
        };
        let mut notice = item(
            notice_id,
            status,
            None,
            TurnItemKind::SystemNotice {
                message: format!("Background agent {verb}: {label}"),
                task_settlement: Some(TaskSettlement {
                    task_id: bg_id,
                    kind: "agent".into(),
                    status: settlement_status.into(),
                    label,
                }),
            },
        );
        notice.turn_id = turn_id;
        step.push(DriverEvent::TurnItem(notice));
        self.evict_settled_subagents();
    }

    fn evict_settled_subagents(&mut self) {
        const MAX_SETTLED_SUBAGENTS: usize = 64;
        let mut settled: Vec<(String, Option<String>, DateTime<Utc>)> = self
            .subagents
            .iter()
            .filter(|(_, c)| c.settled && !c.status.is_open())
            .map(|(k, c)| {
                (
                    k.clone(),
                    c.child_thread_id.clone(),
                    c.completed_at.unwrap_or(c.started_at),
                )
            })
            .collect();
        if settled.len() <= MAX_SETTLED_SUBAGENTS {
            return;
        }
        settled.sort_by_key(|(_, _, ts)| *ts);
        let drop_count = settled.len() - MAX_SETTLED_SUBAGENTS;
        for (k, child_tid, _) in settled.into_iter().take(drop_count) {
            self.subagents.remove(&k);
            if let Some(tid) = child_tid {
                if self.child_thread_to_key.get(&tid) == Some(&k) {
                    self.child_thread_to_key.remove(&tid);
                }
                self.child_items.remove(&tid);
            }
        }
    }

    fn remove_bg_task_ids(&mut self, ids: &[String]) -> bool {
        let before = self.bg_tasks.len();
        self.bg_tasks
            .retain(|t| !ids.iter().any(|id| !id.is_empty() && id == &t.id));
        self.bg_tasks.len() != before
    }
}
