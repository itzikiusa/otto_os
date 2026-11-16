//! Subagent promotion (`TurnItemKind::Subagent`), child item routing
//! (`parent_item_id`), active `.log` / `agy` child `transcript.jsonl` tailing,
//! and background task settlement for [`super::AcpDriver`].

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use agent_client_protocol::v1::schema::{ToolCallStatus, ToolKind};
use chrono::{DateTime, Utc};
use otto_core::agent_runtime::{
    BackgroundTask, RuntimeProvider, TaskSettlement, TurnItem, TurnItemKind, TurnItemStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{command_text, multi_file_patch, AcpDriver, Phase, SegmentKind, Tool};
use crate::acp_sideband::burst::{classify_tool, trim_ascii, unquote_args};
use crate::driver::{
    item, tool_error, ActiveTailChunk, ActiveTailKind, ActiveTailSource, DriverEvent, DriverStep,
    SidebandPoll, SidebandSource,
};

#[path = "acp_subagent_parse.rs"]
pub(crate) mod parse;

use self::parse::{
    canonical_subagent_tool_name, extract_parent_tool_id, extract_subagent_fields,
    non_empty_trimmed, parse_agy_subagent_output, parse_command_log_exit_code,
    parse_grok_bg_launch, short_tool_label,
};

const MAX_FINISHED_BG_IDS: usize = 256;
const MAX_ACCUMULATED_CMD_OUTPUT: usize = 64 * 1024;
const MAX_PENDING_CHILD_TOOLS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BgTaskMeta {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub command_input: Option<String>,
    #[serde(default)]
    pub accumulated_output: String,
    #[serde(default)]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub timer_condition: Option<String>,
    #[serde(default)]
    pub log_path: Option<PathBuf>,
    #[serde(default)]
    pub fallback_log_path: Option<PathBuf>,
    #[serde(default)]
    pub tail_offset: u64,
    #[serde(default)]
    pub tail_using_fallback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChildTextSegment {
    pub kind: SegmentKind,
    pub item_id: String,
    pub message_id: String,
    pub text: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChildTranscriptTool {
    pub item_id: String,
    pub title: String,
    pub kind: ToolKind,
    pub raw_input: Option<Value>,
    pub file: Option<String>,
    pub diff: Option<(Option<String>, String)>,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct AgyChildTranscriptState {
    #[serde(default)]
    pub last_step_index: Option<u64>,
    #[serde(default)]
    pub pending_tools: VecDeque<ChildTranscriptTool>,
    #[serde(default)]
    pub last_send_message: Option<String>,
    #[serde(default)]
    pub last_planner_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AcpSubagent {
    pub call_id: String,
    pub item_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
    pub tool_name: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub subagent_type: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub background: bool,
    pub status: TurnItemStatus,
    #[serde(default)]
    pub tool_uses: u32,
    #[serde(default)]
    pub last_tool: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub child_session_id: Option<String>,
    #[serde(default)]
    pub child_segments: u32,
    #[serde(default)]
    pub child_segment: Option<ChildTextSegment>,
    #[serde(default)]
    pub agy_tail: AgyChildTranscriptState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ChildToolEntry {
    pub parent_call_id: String,
    pub item_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub tool: Tool,
}

impl AcpDriver {
    pub(super) fn remember_finished_bg(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return false;
        }
        if self.state.finished_bg_ids.iter().any(|x| x == id) {
            return false;
        }
        if self.state.finished_bg_ids.len() >= MAX_FINISHED_BG_IDS {
            self.state.finished_bg_ids.pop_front();
        }
        self.state.finished_bg_ids.push_back(id.to_string());
        true
    }

    pub(super) fn subagent_turn_item(&self, a: &AcpSubagent) -> TurnItem {
        let mut it = item(
            a.item_id.clone(),
            a.status,
            a.description
                .clone()
                .or_else(|| Some(a.title.clone()).filter(|s| !s.is_empty())),
            TurnItemKind::Subagent {
                tool_name: a.tool_name.clone(),
                description: a.description.clone(),
                subagent_type: a.subagent_type.clone(),
                prompt: a.prompt.clone(),
                background: a.background,
                tool_uses: a.tool_uses,
                last_tool: a.last_tool.clone(),
                output: a.output.clone(),
            },
        );
        it.turn_id = a.turn_id.clone();
        it.started_at = Some(a.started_at);
        it.completed_at = if a.status.is_open() {
            None
        } else {
            Some(a.completed_at.unwrap_or_else(Utc::now))
        };
        it.error = a.error.clone();
        it
    }

    pub(super) fn emit_subagent(&self, step: &mut DriverStep, call_id: &str) {
        if let Some(a) = self.state.subagents.get(call_id) {
            step.push(DriverEvent::TurnItem(self.subagent_turn_item(a)));
        }
    }

    /// Resolve whether an incoming `session/update` JSON object belongs to an
    /// active subagent card (via `_meta.parentToolUseId` / `parentToolCallId`,
    /// child `sessionId`, or an active synchronous foreground subagent).
    pub(super) fn resolve_parent_subagent_call_id(&mut self, params: &Value) -> Option<String> {
        if self.state.subagents.is_empty() {
            return None;
        }
        let up = params.get("update");
        let self_tool_id = up.and_then(|u| u.get("toolCallId")).and_then(Value::as_str);
        if let Some(tid) = self_tool_id {
            if self.state.subagents.contains_key(tid) {
                return None;
            }
            if let Some(existing) = self.state.child_tools.get(tid) {
                if self.state.subagents.contains_key(&existing.parent_call_id) {
                    return Some(existing.parent_call_id.clone());
                }
            }
        }

        let explicit_parent = extract_parent_tool_id(params, up);
        if let Some(pid) = explicit_parent {
            if self.state.subagents.contains_key(&pid) {
                return Some(pid);
            }
            if let Some((cid, _)) = self
                .state
                .subagents
                .iter()
                .find(|(_, a)| a.task_id.as_deref() == Some(pid.as_str()) || a.item_id == pid)
            {
                return Some(cid.clone());
            }
        }

        let notif_sid = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let (Some(sid), Some(root_sid)) = (notif_sid, self.state.session_id.as_deref()) {
            if sid != root_sid {
                if let Some((cid, _)) = self.state.subagents.iter().find(|(_, a)| {
                    a.child_session_id.as_deref() == Some(sid) || a.task_id.as_deref() == Some(sid)
                }) {
                    return Some(cid.clone());
                }
                let mut open_cards = self
                    .state
                    .subagents
                    .iter_mut()
                    .filter(|(_, a)| a.status.is_open() && a.child_session_id.is_none());
                if let Some((cid, card)) = open_cards.next() {
                    if open_cards.next().is_none() {
                        card.child_session_id = Some(sid.to_string());
                        return Some(cid.clone());
                    }
                }
            }
        }

        // Synchronous foreground subagents (`codebase_investigator`, `cli_help`,
        // `delegate_to_agent`, `task`, `spawn_subagent` with `background == false`)
        // execute inside the parent tool call before it returns `completed`:
        // attribute non-subagent child tool calls emitted while the foreground
        // subagent is open.
        let is_new_subagent = up.is_some_and(|u| {
            let title = u.get("title").and_then(Value::as_str).unwrap_or("");
            canonical_subagent_tool_name(title, u.get("rawInput")).is_some()
        });
        if !is_new_subagent && self_tool_id.is_some() {
            return self
                .state
                .subagents
                .iter()
                .filter(|(_, a)| !a.background && a.status.is_open())
                .max_by_key(|(_, a)| a.started_at)
                .map(|(cid, _)| cid.clone());
        }
        None
    }

    pub(super) fn append_child_text(
        &mut self,
        step: &mut DriverStep,
        parent_call_id: &str,
        kind: SegmentKind,
        text: &str,
        mid: Option<String>,
    ) {
        let Some(card) = self.state.subagents.get_mut(parent_call_id) else {
            return;
        };
        let reuse = card
            .child_segment
            .as_ref()
            .is_some_and(|s| s.kind == kind && mid.as_ref().is_none_or(|m| *m == s.message_id));
        if !reuse {
            if let Some(prev) = card.child_segment.take() {
                step.push(DriverEvent::TurnItem(build_child_segment_item(
                    &card.item_id,
                    card.turn_id.clone(),
                    &prev,
                    false,
                )));
            }
            card.child_segments += 1;
            let item_id = format!("{}:cseg{}", card.item_id, card.child_segments);
            card.child_segment = Some(ChildTextSegment {
                kind,
                message_id: mid.unwrap_or_else(|| item_id.clone()),
                item_id,
                text: String::new(),
                started_at: Utc::now(),
            });
        }
        let seg = card.child_segment.as_mut().expect("set above");
        seg.text.push_str(text);
        let item = build_child_segment_item(&card.item_id, card.turn_id.clone(), seg, true);
        step.push(DriverEvent::TurnItem(item));
    }

    pub(super) fn close_child_segment(&mut self, step: &mut DriverStep, parent_call_id: &str) {
        let Some(card) = self.state.subagents.get_mut(parent_call_id) else {
            return;
        };
        if let Some(seg) = card.child_segment.take() {
            step.push(DriverEvent::TurnItem(build_child_segment_item(
                &card.item_id,
                card.turn_id.clone(),
                &seg,
                false,
            )));
        }
    }

    /// Emit or update a tool call, routing either to a child item under
    /// `parent_call_id`, a promoted `TurnItemKind::Subagent` card, or a
    /// standard root tool item. Returns `true` if handled as subagent/child
    /// (caller should skip `self.state.tools` compaction/insertion when async).
    pub(super) fn emit_tool_or_subagent(
        &mut self,
        step: &mut DriverStep,
        call_id: &str,
        t: &Tool,
        meta: Option<&Value>,
        raw_output: Option<&Value>,
        parent_call_id: Option<&str>,
    ) -> bool {
        if let Some(parent_cid) = parent_call_id {
            if self.state.subagents.contains_key(parent_cid) {
                self.emit_child_tool(step, parent_cid, call_id, t);
                return true;
            }
        }

        if self.state.subagents.contains_key(call_id)
            || canonical_subagent_tool_name(&t.title, t.raw_input.as_ref()).is_some()
        {
            self.upsert_subagent_from_tool(step, call_id, t, meta, raw_output);
            return true;
        }

        step.push(DriverEvent::TurnItem(self.tool_item(call_id, t)));
        self.note_tool_background(step, call_id, t);
        false
    }

    fn emit_child_tool(
        &mut self,
        step: &mut DriverStep,
        parent_cid: &str,
        child_call_id: &str,
        t: &Tool,
    ) {
        self.close_child_segment(step, parent_cid);
        let is_new = !self.state.child_tools.contains_key(child_call_id);
        let (parent_item_id, parent_turn_id) = match self.state.subagents.get_mut(parent_cid) {
            Some(card) => {
                if is_new {
                    card.tool_uses = card.tool_uses.saturating_add(1);
                    card.last_tool = Some(short_tool_label(&t.title));
                }
                (card.item_id.clone(), card.turn_id.clone())
            }
            None => return,
        };
        let entry = self
            .state
            .child_tools
            .entry(child_call_id.to_string())
            .or_insert_with(|| ChildToolEntry {
                parent_call_id: parent_cid.to_string(),
                item_id: format!("{parent_item_id}:child:{child_call_id}"),
                turn_id: parent_turn_id.clone(),
                started_at: Utc::now(),
                tool: t.clone(),
            });
        entry.tool = t.clone();
        let mut child_item = build_tool_turn_item(&entry.item_id, &entry.tool);
        child_item.parent_item_id = Some(parent_item_id);
        child_item.turn_id = entry.turn_id.clone();
        child_item.started_at = Some(entry.started_at);
        let terminal = matches!(
            entry.tool.status,
            ToolCallStatus::Completed | ToolCallStatus::Failed
        );
        if terminal {
            self.state.child_tools.remove(child_call_id);
        }
        step.push(DriverEvent::TurnItem(child_item));
        if is_new {
            self.emit_subagent(step, parent_cid);
        }
    }

    fn upsert_subagent_from_tool(
        &mut self,
        step: &mut DriverStep,
        call_id: &str,
        t: &Tool,
        meta: Option<&Value>,
        raw_output: Option<&Value>,
    ) {
        let canonical = canonical_subagent_tool_name(&t.title, t.raw_input.as_ref())
            .unwrap_or_else(|| "subagent".to_string());
        let turn_str = self.turn();
        let turn_opt = self.state.turn_id.clone();

        let (desc, sub_type, prompt, req_bg, req_child_sid) =
            extract_subagent_fields(t.raw_input.as_ref(), &t.title, &canonical, meta, raw_output);
        let raw_json_fallback = t
            .output
            .trim()
            .is_empty()
            .then(|| raw_output.and_then(|v| serde_json::to_string(v).ok()))
            .flatten();
        let launch_src = raw_json_fallback.as_deref().unwrap_or(&t.output);
        let agy_launch = parse_agy_subagent_output(launch_src, self.state.sideband.root_dir());
        let grok_launch = parse_grok_bg_launch(launch_src);
        let implied_agy_bg =
            self.provider == RuntimeProvider::Agy && canonical == "invoke_subagent";
        let is_bg = req_bg || implied_agy_bg || agy_launch.is_some() || grok_launch.is_some();
        if is_bg && matches!(t.status, ToolCallStatus::Completed) {
            if let Some((cid, _)) = &agy_launch {
                self.state.note_agy_conv_id(cid);
            }
        }

        let card = self
            .state
            .subagents
            .entry(call_id.to_string())
            .or_insert_with(|| AcpSubagent {
                call_id: call_id.to_string(),
                item_id: format!("{turn_str}:tool:{call_id}"),
                turn_id: turn_opt.clone(),
                started_at: Utc::now(),
                completed_at: None,
                tool_name: canonical.clone(),
                title: t.title.clone(),
                description: desc.clone(),
                subagent_type: sub_type.clone(),
                prompt: prompt.clone(),
                background: is_bg,
                status: TurnItemStatus::Running,
                tool_uses: 0,
                last_tool: None,
                output: None,
                error: None,
                task_id: None,
                child_session_id: req_child_sid.clone(),
                child_segments: 0,
                child_segment: None,
                agy_tail: AgyChildTranscriptState::default(),
            });

        if card.description.is_none() && desc.is_some() {
            card.description = desc;
        }
        if card.subagent_type.is_none() && sub_type.is_some() {
            card.subagent_type = sub_type;
        }
        if card.prompt.is_none() && prompt.is_some() {
            card.prompt = prompt;
        }
        if card.child_session_id.is_none() && req_child_sid.is_some() {
            card.child_session_id = req_child_sid;
        }
        if is_bg {
            card.background = true;
        }

        match t.status {
            ToolCallStatus::Failed => {
                card.status = TurnItemStatus::Failed;
                card.completed_at = Some(Utc::now());
                let out = non_empty_trimmed(&t.output);
                card.error = out.as_deref().and_then(tool_error);
                card.output = out;
                let maybe_tid = card.task_id.clone();
                self.emit_subagent(step, call_id);
                self.finish_subagent_children(step, call_id, TurnItemStatus::Failed);
                self.state.subagents.remove(call_id);
                if let Some(tid) = maybe_tid {
                    self.remove_bg_task_silent(step, &tid);
                }
            }
            ToolCallStatus::Completed if card.background => {
                let (task_id, log_paths) = if let Some((cid, paths)) = agy_launch {
                    (cid, paths)
                } else if let Some((aid, _)) = grok_launch {
                    if card.child_session_id.is_none() {
                        card.child_session_id = Some(aid.clone());
                    }
                    (aid, None)
                } else {
                    let fallback = card
                        .task_id
                        .clone()
                        .unwrap_or_else(|| format!("subagent:{call_id}"));
                    (fallback, None)
                };
                card.task_id = Some(task_id.clone());
                if self.state.finished_bg_ids.iter().any(|f| f == &task_id) {
                    // Out-of-order settlement arrived before the launch RPC completed:
                    // do not reopen or leave the card stuck in Running!
                    card.status = TurnItemStatus::Completed;
                    card.completed_at = Some(Utc::now());
                    self.emit_subagent(step, call_id);
                    self.finish_subagent_children(step, call_id, TurnItemStatus::Completed);
                    self.state.subagents.remove(call_id);
                    return;
                }
                card.status = TurnItemStatus::Running;
                let label = card
                    .description
                    .clone()
                    .or_else(|| card.subagent_type.clone())
                    .unwrap_or_else(|| card.title.clone());
                let item_id = card.item_id.clone();
                let started_at = card.started_at;
                let turn_id = card.turn_id.clone();
                self.emit_subagent(step, call_id);

                let (log_path, fallback_log_path) = match log_paths {
                    Some((p, fb)) => (Some(p), fb),
                    None => (None, None),
                };
                self.register_bg_task(
                    step,
                    BgTaskMeta {
                        id: task_id,
                        kind: "agent".into(),
                        label,
                        started_at,
                        call_id: Some(call_id.to_string()),
                        item_id: Some(item_id),
                        turn_id,
                        command_input: None,
                        accumulated_output: String::new(),
                        due_at: None,
                        timer_condition: None,
                        log_path,
                        fallback_log_path,
                        tail_offset: 0,
                        tail_using_fallback: false,
                    },
                );
            }
            ToolCallStatus::Completed => {
                // Synchronous subagent finished inside the parent tool call.
                card.status = TurnItemStatus::Completed;
                card.completed_at = Some(Utc::now());
                if let Some(out) = non_empty_trimmed(&t.output) {
                    card.output = Some(out);
                }
                let maybe_tid = card.task_id.clone();
                self.emit_subagent(step, call_id);
                self.finish_subagent_children(step, call_id, TurnItemStatus::Completed);
                self.state.subagents.remove(call_id);
                if let Some(tid) = maybe_tid {
                    self.remove_bg_task_silent(step, &tid);
                }
            }
            _ => {
                card.status = TurnItemStatus::Running;
                self.emit_subagent(step, call_id);
            }
        }
    }

    pub(super) fn finish_subagent_children(
        &mut self,
        step: &mut DriverStep,
        call_id: &str,
        terminal_status: TurnItemStatus,
    ) {
        self.close_child_segment(step, call_id);
        let parent_item_id = self.state.subagents.get(call_id).map(|a| a.item_id.clone());
        let child_ids: Vec<String> = self
            .state
            .child_tools
            .iter()
            .filter(|(_, e)| e.parent_call_id == call_id)
            .map(|(k, _)| k.clone())
            .collect();
        for cid in child_ids {
            if let Some(mut entry) = self.state.child_tools.remove(&cid) {
                entry.tool.status = match terminal_status {
                    TurnItemStatus::Failed => ToolCallStatus::Failed,
                    _ => ToolCallStatus::Completed,
                };
                let mut it = build_tool_turn_item(&entry.item_id, &entry.tool);
                it.parent_item_id = parent_item_id.clone();
                it.turn_id = entry.turn_id;
                it.started_at = Some(entry.started_at);
                it.status = terminal_status;
                it.completed_at = Some(Utc::now());
                step.push(DriverEvent::TurnItem(it));
            }
        }
        if let Some(card) = self.state.subagents.get_mut(call_id) {
            while let Some(pending) = card.agy_tail.pending_tools.pop_front() {
                let mut t = Tool {
                    title: pending.title,
                    kind: pending.kind,
                    status: match terminal_status {
                        TurnItemStatus::Failed => ToolCallStatus::Failed,
                        _ => ToolCallStatus::Completed,
                    },
                    raw_input: pending.raw_input,
                    file: pending.file,
                    diff: pending.diff,
                    ..Tool::default()
                };
                if let (Some(f), Some((old, new))) = (&t.file, &t.diff) {
                    t.diffs.push((f.clone(), old.clone(), new.clone()));
                }
                let mut it = build_tool_turn_item(&pending.item_id, &t);
                it.parent_item_id = Some(card.item_id.clone());
                it.turn_id = card.turn_id.clone();
                it.started_at = Some(pending.started_at);
                it.status = terminal_status;
                it.completed_at = Some(Utc::now());
                step.push(DriverEvent::TurnItem(it));
            }
        }
    }

    pub(super) fn register_bg_task(&mut self, step: &mut DriverStep, meta: BgTaskMeta) {
        if self.state.finished_bg_ids.iter().any(|f| f == &meta.id) {
            return;
        }
        let task = BackgroundTask {
            id: meta.id.clone(),
            kind: meta.kind.clone(),
            label: meta.label.clone(),
            started_at: meta.started_at,
            item_id: meta.item_id.clone(),
            due_at: meta.due_at,
        };
        self.state.bg_meta.insert(meta.id.clone(), meta);
        let changed = super::bg::upsert_bg_task(&mut self.state.bg_tasks, task);
        if changed {
            step.push(DriverEvent::BackgroundTasks(self.state.bg_tasks.clone()));
        }
    }

    pub(super) fn remove_bg_task_silent(&mut self, step: &mut DriverStep, task_id: &str) {
        self.state.bg_meta.remove(task_id);
        let before = self.state.bg_tasks.len();
        self.state.bg_tasks.retain(|b| b.id != task_id);
        if self.state.bg_tasks.len() != before {
            step.push(DriverEvent::BackgroundTasks(self.state.bg_tasks.clone()));
        }
    }

    /// Settle a tracked background task (`command`, `agent`, `timer`, `swarm`)
    /// with `status` (`Completed` / `Failed` / `Cancelled` / `Interrupted`),
    /// updating any linked `Subagent` or `CommandExecution` card, emitting the
    /// settlement `SystemNotice`, and updating `BackgroundTasks`.
    pub(super) fn settle_bg_task(
        &mut self,
        step: &mut DriverStep,
        task_id: &str,
        status: TurnItemStatus,
        output_override: Option<String>,
        exit_code: Option<i32>,
    ) -> bool {
        let meta = self.state.bg_meta.remove(task_id);
        let before = self.state.bg_tasks.len();
        let mut removed_task: Option<BackgroundTask> = None;
        self.state.bg_tasks.retain(|b| {
            if b.id == task_id {
                removed_task = Some(b.clone());
                false
            } else {
                true
            }
        });
        let bg_changed = self.state.bg_tasks.len() != before;

        let subagent_call_id = meta
            .as_ref()
            .and_then(|m| m.call_id.clone())
            .filter(|cid| self.state.subagents.contains_key(cid))
            .or_else(|| {
                self.state
                    .subagents
                    .iter()
                    .find(|(_, a)| a.task_id.as_deref() == Some(task_id))
                    .map(|(cid, _)| cid.clone())
            });

        if meta.is_none() && removed_task.is_none() && subagent_call_id.is_none() {
            return false;
        }
        let first_time = self.remember_finished_bg(task_id);

        if let Some(cid) = subagent_call_id {
            self.finish_subagent_children(step, &cid, status);
            if let Some(mut card) = self.state.subagents.remove(&cid) {
                card.status = status;
                card.completed_at = Some(Utc::now());
                let final_out = output_override
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| card.agy_tail.last_send_message.clone())
                    .or_else(|| card.agy_tail.last_planner_text.clone())
                    .or_else(|| card.output.clone());
                card.output = final_out;
                if status == TurnItemStatus::Failed {
                    card.error = card.output.as_deref().and_then(tool_error);
                }
                step.push(DriverEvent::TurnItem(self.subagent_turn_item(&card)));
                self.retire_tool_id(cid);
            }
        } else if let Some(m) = meta.as_ref().filter(|m| m.kind == "command") {
            if let Some(item_id) = &m.item_id {
                let final_out = output_override
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| non_empty_trimmed(&m.accumulated_output));
                if final_out.is_some() || exit_code.is_some() || status != TurnItemStatus::Completed
                {
                    let mut cmd_item = item(
                        item_id.clone(),
                        status,
                        Some(m.label.clone()),
                        TurnItemKind::CommandExecution {
                            input: m.command_input.clone().unwrap_or_else(|| m.label.clone()),
                            output: final_out.clone(),
                            exit_code,
                        },
                    );
                    cmd_item.turn_id = m.turn_id.clone();
                    cmd_item.started_at = Some(m.started_at);
                    cmd_item.completed_at = Some(Utc::now());
                    if status == TurnItemStatus::Failed {
                        cmd_item.error = final_out.as_deref().and_then(tool_error);
                    }
                    step.push(DriverEvent::TurnItem(cmd_item));
                }
            }
        }

        if first_time {
            let kind = meta
                .as_ref()
                .map(|m| m.kind.as_str())
                .or_else(|| removed_task.as_ref().map(|t| t.kind.as_str()))
                .unwrap_or("command");
            let label = meta
                .as_ref()
                .map(|m| m.label.as_str())
                .or_else(|| removed_task.as_ref().map(|t| t.label.as_str()))
                .unwrap_or(task_id);
            if matches!(kind, "command" | "agent" | "timer" | "schedule") {
                let noun = match kind {
                    "agent" => "agent",
                    "timer" | "schedule" => "timer",
                    _ => "command",
                };
                let (verb, settlement_status) = match status {
                    TurnItemStatus::Completed => ("finished", "completed"),
                    TurnItemStatus::Failed => ("failed", "failed"),
                    _ => ("stopped", "stopped"),
                };
                let mut notice = item(
                    format!("bg:{task_id}"),
                    status,
                    None,
                    TurnItemKind::SystemNotice {
                        message: format!("Background {noun} {verb}: {label}"),
                        task_settlement: Some(TaskSettlement {
                            task_id: task_id.to_string(),
                            kind: noun.to_string(),
                            status: settlement_status.to_string(),
                            label: label.to_string(),
                        }),
                    },
                );
                notice.turn_id = self
                    .state
                    .turn_id
                    .clone()
                    .or_else(|| meta.as_ref().and_then(|m| m.turn_id.clone()));
                step.push(DriverEvent::TurnItem(notice));
            }
        }

        if bg_changed {
            step.push(DriverEvent::BackgroundTasks(self.state.bg_tasks.clone()));
        }
        true
    }

    pub(super) fn has_expired_timer_at(&self, now: DateTime<Utc>) -> bool {
        self.state
            .bg_meta
            .values()
            .any(|m| m.due_at.is_some_and(|due| due <= now))
    }

    pub(super) fn next_timer_wait(&self, now: DateTime<Utc>) -> Option<Duration> {
        self.state
            .bg_meta
            .values()
            .filter_map(|m| m.due_at)
            .map(|due| {
                if due <= now {
                    Duration::from_millis(50)
                } else {
                    let ms = (due - now).num_milliseconds().max(50) as u64;
                    Duration::from_millis(ms)
                }
            })
            .min()
    }

    pub(super) fn settle_expired_timers(
        &mut self,
        step: &mut DriverStep,
        now: DateTime<Utc>,
    ) -> usize {
        let mut due_ids: Vec<(DateTime<Utc>, String)> = self
            .state
            .bg_meta
            .values()
            .filter_map(|m| m.due_at.filter(|d| *d <= now).map(|d| (d, m.id.clone())))
            .collect();
        due_ids.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut count = 0;
        for (_, id) in due_ids {
            if self.settle_bg_task(step, &id, TurnItemStatus::Completed, None, None) {
                count += 1;
            }
        }
        count
    }

    /// Settle expired timers and transient `swarm_wait` when the user sends a
    /// new prompt, while preserving still-running async subagents, active
    /// commands, and future timers.
    pub(super) fn settle_bg_on_user_prompt(&mut self, step: &mut DriverStep) {
        self.settle_expired_timers(step, Utc::now());
        if self.state.bg_tasks.iter().any(|b| b.id == "swarm_wait") {
            self.remove_bg_task_silent(step, "swarm_wait");
        }
    }

    /// Settle background tasks on a native stdout autonomous wakeup (no
    /// sideband `<SYSTEM_MESSAGE sender=...>`):
    /// - Settles all expired one-shot timers (`due_at <= now`); if none are
    ///   due yet (synthetic fast-clock tests), settles the earliest timer.
    /// - Settles `swarm_wait` and non-tailed commands (`log_path.is_none()`).
    /// - Keeps still-running async `Subagent`s and `.log`-tailed commands open!
    pub(super) fn settle_bg_on_native_wakeup(&mut self, step: &mut DriverStep) {
        let now = Utc::now();
        let settled_due = self.settle_expired_timers(step, now);
        if settled_due == 0 {
            let earliest_timer = self
                .state
                .bg_tasks
                .iter()
                .filter(|b| b.kind == "timer" || b.kind == "schedule")
                .min_by_key(|b| {
                    self.state
                        .bg_meta
                        .get(&b.id)
                        .and_then(|m| m.due_at)
                        .unwrap_or(b.started_at)
                })
                .map(|b| b.id.clone());
            if let Some(tid) = earliest_timer {
                self.settle_bg_task(step, &tid, TurnItemStatus::Completed, None, None);
            }
        }
        if self.state.bg_tasks.iter().any(|b| b.id == "swarm_wait") {
            self.remove_bg_task_silent(step, "swarm_wait");
        }
        let untailed_cmds: Vec<String> = self
            .state
            .bg_tasks
            .iter()
            .filter(|b| {
                b.kind == "command"
                    && self
                        .state
                        .bg_meta
                        .get(&b.id)
                        .is_none_or(|m| m.log_path.is_none())
            })
            .map(|b| b.id.clone())
            .collect();
        for cid in untailed_cmds {
            self.settle_bg_task(step, &cid, TurnItemStatus::Completed, None, None);
        }
    }

    /// Settle the specific background task/subagent/timer that triggered an
    /// `agy` `<SYSTEM_MESSAGE>` wakeup, preserving all other concurrent
    /// background tasks.
    pub(super) fn settle_bg_on_system_wakeup(
        &mut self,
        step: &mut DriverStep,
        sender: Option<&str>,
        content: &str,
    ) {
        let now = Utc::now();
        self.settle_expired_timers(step, now);

        if let Some(snd) = sender.map(str::trim).filter(|s| !s.is_empty()) {
            let matched_id = self
                .state
                .bg_tasks
                .iter()
                .find(|b| {
                    b.id == snd
                        || b.id.ends_with(&format!("/{snd}"))
                        || snd.ends_with(&format!("/{id}", id = b.id))
                        || self
                            .state
                            .bg_meta
                            .get(&b.id)
                            .and_then(|m| m.timer_condition.as_deref())
                            .is_some_and(|tc| tc == snd)
                })
                .map(|b| b.id.clone())
                .or_else(|| {
                    self.state
                        .subagents
                        .values()
                        .find_map(|a| a.task_id.as_ref().filter(|t| *t == snd).cloned())
                });
            if let Some(tid) = matched_id {
                let out = (!content.trim().is_empty()).then(|| content.to_string());
                self.settle_bg_task(step, &tid, TurnItemStatus::Completed, out, None);
                return;
            }
            // Only settle a `schedule` timer on an unrelated `sender` wakeup when
            // `TimerCondition: "any"` was explicitly configured on that timer!
            let any_conditioned_timer = self
                .state
                .bg_tasks
                .iter()
                .filter(|b| {
                    (b.kind == "timer" || b.kind == "schedule")
                        && self
                            .state
                            .bg_meta
                            .get(&b.id)
                            .and_then(|m| m.timer_condition.as_deref())
                            .is_some_and(|tc| tc.eq_ignore_ascii_case("any"))
                })
                .min_by_key(|b| {
                    self.state
                        .bg_meta
                        .get(&b.id)
                        .and_then(|m| m.due_at)
                        .unwrap_or(b.started_at)
                })
                .map(|b| b.id.clone());
            if let Some(tid) = any_conditioned_timer {
                self.settle_bg_task(step, &tid, TurnItemStatus::Completed, None, None);
            }
            return;
        }

        // No explicit sender (e.g. a `<SYSTEM_MESSAGE>` timer firing slightly
        // before wall-clock `due_at`, or `swarm_wait`): settle earliest timer
        // if none just expired above.
        if self.state.bg_tasks.iter().any(|b| b.id == "swarm_wait") {
            self.remove_bg_task_silent(step, "swarm_wait");
            return;
        }
        let earliest_timer = self
            .state
            .bg_tasks
            .iter()
            .filter(|b| b.kind == "timer" || b.kind == "schedule")
            .min_by_key(|b| {
                self.state
                    .bg_meta
                    .get(&b.id)
                    .and_then(|m| m.due_at)
                    .unwrap_or(b.started_at)
            })
            .map(|b| b.id.clone());
        if let Some(tid) = earliest_timer {
            self.settle_bg_task(step, &tid, TurnItemStatus::Completed, None, None);
        }
    }

    pub(super) fn active_tail_sources(&self) -> Vec<ActiveTailSource> {
        let allowed_cwd = {
            let c = self.state.cwd.trim();
            (!c.is_empty()).then(|| PathBuf::from(c))
        };
        let agy_brain_dir = self.state.sideband.root_dir().map(PathBuf::from);
        let allowed_agy_conv_ids = (self.provider == RuntimeProvider::Agy).then(|| {
            let mut ids: Vec<String> = self.state.observed_agy_conv_ids.iter().cloned().collect();
            for extra in [
                self.state.session_id.as_deref(),
                self.state.resume.as_deref(),
            ] {
                if let Some(id) = extra.map(str::trim).filter(|s| !s.is_empty()) {
                    if !ids.iter().any(|x| x == id) {
                        ids.push(id.to_string());
                    }
                }
            }
            ids
        });
        let mut out = Vec::new();
        for bg in &self.state.bg_tasks {
            let Some(meta) = self.state.bg_meta.get(&bg.id) else {
                continue;
            };
            let Some(path) = &meta.log_path else {
                continue;
            };
            let kind = if meta.kind == "agent" {
                ActiveTailKind::AgySubagentTranscript
            } else if meta.kind == "command" {
                ActiveTailKind::CommandLog
            } else {
                continue;
            };
            out.push(ActiveTailSource {
                task_id: bg.id.clone(),
                kind,
                path: path.clone(),
                fallback_path: meta.fallback_log_path.clone(),
                offset: meta.tail_offset,
                using_fallback: meta.tail_using_fallback,
                allowed_cwd: allowed_cwd.clone(),
                agy_brain_dir: agy_brain_dir.clone(),
                allowed_agy_conv_ids: allowed_agy_conv_ids.clone(),
                otto_session_id: self.state.otto_session_id.clone(),
            });
            if out.len() >= 8 {
                break;
            }
        }
        out
    }

    pub(super) fn build_sideband_poll(&self) -> Option<SidebandPoll> {
        if self.state.phase != Phase::Ready || self.state.cancelling {
            return None;
        }
        let idle_between_prompts = self.state.prompt_id.is_none() && self.state.queued.is_none();
        let has_open_subagent = self.state.subagents.values().any(|a| a.status.is_open());
        let has_open_bg_work = !self.state.bg_tasks.is_empty() || has_open_subagent;
        let poll_child_subagents = matches!(
            self.provider,
            RuntimeProvider::Opencode | RuntimeProvider::Grok | RuntimeProvider::Gemini
        ) && has_open_subagent;
        let primary = self.state.sideband.sideband_poll(
            self.provider,
            true,
            idle_between_prompts,
            poll_child_subagents,
            has_open_bg_work,
            &self.state.cwd,
            self.state.session_id.as_deref(),
        );
        let active_tails = self.active_tail_sources();
        let now = Utc::now();
        let timer_expired = self.has_expired_timer_at(now);
        let next_timer = self.next_timer_wait(now);

        if primary.is_none() && active_tails.is_empty() && !timer_expired && next_timer.is_none() {
            return None;
        }

        let mut interval = match &primary {
            Some(p) => p.interval,
            None => Duration::from_millis(400),
        };
        if !active_tails.is_empty() {
            interval = interval.min(Duration::from_millis(400));
        }
        if let Some(wait) = next_timer {
            interval = interval.min(wait);
        }
        if timer_expired {
            interval = Duration::from_millis(50);
        }

        Some(SidebandPoll {
            source: primary.map(|p| p.source).unwrap_or(SidebandSource::None),
            interval,
            active_tails,
            timer_expired,
        })
    }

    pub(super) fn apply_active_tail_chunks(
        &mut self,
        step: &mut DriverStep,
        tails: Vec<ActiveTailChunk>,
    ) {
        for chunk in tails {
            match chunk.kind {
                ActiveTailKind::CommandLog => self.apply_command_log_chunk(step, chunk),
                ActiveTailKind::AgySubagentTranscript => {
                    self.apply_agy_subagent_transcript_chunk(step, chunk);
                }
            }
        }
    }

    fn apply_command_log_chunk(&mut self, step: &mut DriverStep, chunk: ActiveTailChunk) {
        let Some(meta) = self.state.bg_meta.get_mut(&chunk.task_id) else {
            return;
        };
        meta.tail_offset = chunk.next_offset;
        if chunk.bytes.is_empty() {
            return;
        }
        let text = String::from_utf8_lossy(&chunk.bytes).into_owned();
        if text.is_empty() {
            return;
        }
        meta.accumulated_output.push_str(&text);
        if meta.accumulated_output.len() > MAX_ACCUMULATED_CMD_OUTPUT {
            let mut drop_len = meta.accumulated_output.len() - MAX_ACCUMULATED_CMD_OUTPUT;
            while !meta.accumulated_output.is_char_boundary(drop_len) {
                drop_len += 1;
            }
            meta.accumulated_output.drain(..drop_len);
        }
        if let Some(item_id) = meta.item_id.clone() {
            step.push(DriverEvent::CommandOutput {
                item_id,
                chunk: text.clone(),
            });
        }
        if let Some(exit_code) = parse_command_log_exit_code(&text) {
            let status = if exit_code == 0 {
                TurnItemStatus::Completed
            } else {
                TurnItemStatus::Failed
            };
            self.settle_bg_task(step, &chunk.task_id, status, None, Some(exit_code));
        }
    }

    fn apply_agy_subagent_transcript_chunk(
        &mut self,
        step: &mut DriverStep,
        chunk: ActiveTailChunk,
    ) {
        let call_id = {
            let Some(meta) = self.state.bg_meta.get_mut(&chunk.task_id) else {
                return;
            };
            meta.tail_offset = chunk.next_offset;
            meta.tail_using_fallback = chunk.using_fallback;
            meta.call_id.clone()
        };
        let Some(call_id) = call_id.or_else(|| {
            self.state
                .subagents
                .iter()
                .find(|(_, a)| a.task_id.as_deref() == Some(chunk.task_id.as_str()))
                .map(|(cid, _)| cid.clone())
        }) else {
            return;
        };

        let mut settle_status: Option<TurnItemStatus> = None;
        for raw in chunk.bytes.split(|b| *b == b'\n') {
            let line = trim_ascii(raw);
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let step_idx = v.get("step_index").and_then(Value::as_u64);
            let Some(raw_type) = v.get("type").and_then(Value::as_str) else {
                continue;
            };
            let step_type = raw_type
                .strip_prefix("CORTEX_STEP_TYPE_")
                .unwrap_or(raw_type);

            let mut parent_dirty = false;
            let mut new_items: Vec<TurnItem> = Vec::new();
            {
                let Some(card) = self.state.subagents.get_mut(&call_id) else {
                    break;
                };
                if let Some(idx) = step_idx {
                    if card
                        .agy_tail
                        .last_step_index
                        .is_some_and(|prev| idx <= prev)
                    {
                        continue;
                    }
                    card.agy_tail.last_step_index = Some(idx);
                }
                let idx_num = step_idx.unwrap_or(0);

                match step_type {
                    "PLANNER_RESPONSE" => {
                        let calls = v
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(Vec::as_slice)
                            .unwrap_or(&[]);
                        for (i, tc) in calls.iter().enumerate() {
                            let raw_name = tc.get("name").and_then(Value::as_str).unwrap_or("tool");
                            let args = tc
                                .get("args")
                                .or_else(|| tc.get("arguments"))
                                .map(unquote_args);
                            if raw_name == "send_message" || raw_name.ends_with(":send_message") {
                                if let Some(msg) = args
                                    .as_ref()
                                    .and_then(|a| {
                                        a.get("Message")
                                            .or_else(|| a.get("message"))
                                            .and_then(Value::as_str)
                                    })
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                {
                                    card.agy_tail.last_send_message = Some(msg.to_string());
                                }
                            }
                            let desc = classify_tool(raw_name, args.as_ref());
                            let child_id = format!("{}:sub:{idx_num}:{i}", card.item_id);
                            let started_at = Utc::now();
                            let mut t = Tool {
                                title: desc.title.clone(),
                                kind: desc.kind,
                                status: ToolCallStatus::InProgress,
                                raw_input: args.clone(),
                                file: desc.file.clone(),
                                diff: desc.diff.clone(),
                                ..Tool::default()
                            };
                            if let (Some(f), Some((old, new))) = (&t.file, &t.diff) {
                                t.diffs.push((f.clone(), old.clone(), new.clone()));
                            }
                            let mut child_it = build_tool_turn_item(&child_id, &t);
                            child_it.parent_item_id = Some(card.item_id.clone());
                            child_it.turn_id = card.turn_id.clone();
                            child_it.started_at = Some(started_at);
                            new_items.push(child_it);

                            if card.agy_tail.pending_tools.len() >= MAX_PENDING_CHILD_TOOLS {
                                card.agy_tail.pending_tools.pop_front();
                            }
                            card.agy_tail.pending_tools.push_back(ChildTranscriptTool {
                                item_id: child_id,
                                title: desc.title,
                                kind: desc.kind,
                                raw_input: args,
                                file: desc.file,
                                diff: desc.diff,
                                started_at,
                            });
                            card.tool_uses = card.tool_uses.saturating_add(1);
                            card.last_tool = Some(short_tool_label(raw_name));
                            parent_dirty = true;
                        }

                        if let Some(content) = v
                            .get("content")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            card.agy_tail.last_planner_text = Some(content.to_string());
                            let msg_id = format!("{}:msg:{idx_num}", card.item_id);
                            let mut msg_it = item(
                                msg_id.clone(),
                                TurnItemStatus::Completed,
                                None,
                                TurnItemKind::AssistantMessage {
                                    message_id: msg_id,
                                    text: content.to_string(),
                                    streaming: false,
                                },
                            );
                            msg_it.parent_item_id = Some(card.item_id.clone());
                            msg_it.turn_id = card.turn_id.clone();
                            new_items.push(msg_it);
                        }

                        let is_err = v.get("status").and_then(Value::as_str) == Some("ERROR");
                        if calls.is_empty()
                            && !is_err
                            && (card.agy_tail.last_send_message.is_some()
                                || card.agy_tail.last_planner_text.is_some()
                                || card.tool_uses > 0)
                        {
                            settle_status = Some(TurnItemStatus::Completed);
                        }
                    }
                    "GENERIC" => {
                        if let Some(pending) = card.agy_tail.pending_tools.pop_front() {
                            let failed = v.get("status").and_then(Value::as_str) == Some("ERROR");
                            let out = v
                                .get("content")
                                .and_then(Value::as_str)
                                .filter(|s| !s.is_empty())
                                .or_else(|| v.get("error").and_then(Value::as_str))
                                .unwrap_or("")
                                .to_string();
                            let mut t = Tool {
                                title: pending.title,
                                kind: pending.kind,
                                status: if failed {
                                    ToolCallStatus::Failed
                                } else {
                                    ToolCallStatus::Completed
                                },
                                raw_input: pending.raw_input,
                                output: out,
                                file: pending.file,
                                diff: pending.diff,
                                ..Tool::default()
                            };
                            if let (Some(f), Some((old, new))) = (&t.file, &t.diff) {
                                t.diffs.push((f.clone(), old.clone(), new.clone()));
                            }
                            let mut child_it = build_tool_turn_item(&pending.item_id, &t);
                            child_it.parent_item_id = Some(card.item_id.clone());
                            child_it.turn_id = card.turn_id.clone();
                            child_it.started_at = Some(pending.started_at);
                            child_it.completed_at = Some(Utc::now());
                            new_items.push(child_it);
                        }
                    }
                    _ => {}
                }
            }

            for it in new_items {
                step.push(DriverEvent::TurnItem(it));
            }
            if parent_dirty && settle_status.is_none() {
                self.emit_subagent(step, &call_id);
            }
            if let Some(st) = settle_status {
                self.settle_bg_task(step, &chunk.task_id, st, None, None);
                break;
            }
        }
    }
}

fn build_child_segment_item(
    parent_item_id: &str,
    turn_id: Option<String>,
    seg: &ChildTextSegment,
    streaming: bool,
) -> TurnItem {
    let status = if streaming {
        TurnItemStatus::Running
    } else {
        TurnItemStatus::Completed
    };
    let kind = match seg.kind {
        SegmentKind::Message => TurnItemKind::AssistantMessage {
            message_id: seg.message_id.clone(),
            text: seg.text.clone(),
            streaming,
        },
        SegmentKind::Thought => TurnItemKind::Reasoning {
            text: seg.text.clone(),
            streaming,
        },
    };
    let mut it = item(seg.item_id.clone(), status, None, kind);
    it.parent_item_id = Some(parent_item_id.to_string());
    it.turn_id = turn_id;
    it.started_at = Some(seg.started_at);
    it
}

fn build_tool_turn_item(item_id: &str, t: &Tool) -> TurnItem {
    let status = match t.status {
        ToolCallStatus::InProgress => TurnItemStatus::Running,
        ToolCallStatus::Completed => TurnItemStatus::Completed,
        ToolCallStatus::Failed => TurnItemStatus::Failed,
        _ => TurnItemStatus::Pending,
    };
    let output = (!t.output.is_empty()).then(|| t.output.clone());
    let error = (status == TurnItemStatus::Failed)
        .then(|| tool_error(&t.output))
        .flatten();
    let kind = match t.kind {
        ToolKind::Execute => TurnItemKind::CommandExecution {
            input: command_text(t.raw_input.as_ref()).unwrap_or_else(|| t.title.clone()),
            output,
            exit_code: None,
        },
        ToolKind::Edit | ToolKind::Delete | ToolKind::Move => {
            let file_name = t.file.clone().unwrap_or_else(|| t.title.clone());
            let raw_diff = (t.diffs.len() > 1).then(|| multi_file_patch(&t.diffs));
            let raw_old = if t.diffs.len() > 1 {
                None
            } else {
                t.diff.as_ref().and_then(|d| d.0.clone())
            };
            let raw_new = if t.diffs.len() > 1 {
                None
            } else {
                t.diff.as_ref().map(|d| d.1.clone())
            };
            let (diff_str, old_str, new_str) = crate::diff_compact::compact_file_change_fields(
                &file_name, raw_diff, raw_old, raw_new,
            );
            TurnItemKind::FileChange {
                file_name,
                diff_str,
                old_str,
                new_str,
            }
        }
        _ => TurnItemKind::DynamicTool {
            image_files: Vec::new(),
            tool_name: t.title.clone(),
            input: t.raw_input.clone().unwrap_or(Value::Null),
            output,
            images: t.images.clone(),
        },
    };
    let mut it = item(item_id.to_string(), status, Some(t.title.clone()), kind);
    it.error = error;
    it
}
