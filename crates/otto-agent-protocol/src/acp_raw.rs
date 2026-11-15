//! Raw ACP `session/update` fallback and sideband action application for
//! [`super::AcpDriver`].

use agent_client_protocol::v1::schema::{ToolCallContent, ToolCallStatus, ToolKind};
use otto_core::agent_runtime::{PlanStep, PlanStepStatus, ThreadTokenUsage, TurnTerminalStatus};
use serde_json::Value;

use super::{AcpDriver, Phase, SegmentKind, Tool};
use crate::acp_sideband::burst::classify_tool;
use crate::acp_sideband::grok::{
    extract_diff, extract_location_path, extract_tool_output, extract_update_text,
    parse_grok_usage, parse_raw_tool_status,
};
use crate::acp_sideband::SidebandAction;
use crate::driver::{DriverEvent, DriverStep};

impl AcpDriver {
    pub(super) fn note_stdout_activity(&mut self) {
        if self.prompt_id.is_none() {
            if self.sideband.has_mirrored() {
                self.sideband.arm_burst_filter();
            } else {
                self.sideband.note_native_autonomous();
            }
        }
    }

    pub(super) fn settle_autonomous_now(
        &mut self,
        step: &mut DriverStep,
        status: TurnTerminalStatus,
        reason: Option<String>,
    ) {
        if !self.autonomous || self.prompt_id.is_some() || self.turn_id.is_none() {
            return;
        }
        self.close_segment(step);
        self.turn_id = None;
        self.autonomous = false;
        self.closed_named.clear();
        self.tools.clear();
        self.sideband.on_turn_settled();
        step.push(DriverEvent::Terminal {
            status,
            failure: None,
            reason,
        });
    }

    /// Fallback when `SessionNotification` fails strict v1 schema parsing (e.g.
    /// Grok `tool_call` omitting `kind`/`status`, Grok `turn_completed`, ACP v2
    /// `agent_message` / `agent_thought`, or experimental vendor updates).
    pub(super) fn on_raw_update(&mut self, step: &mut DriverStep, params: &Value) {
        if matches!(self.phase, Phase::Creating { loading: true }) {
            return;
        }
        let Some(up) = params.get("update") else {
            return;
        };
        let Some(tag) = up.get("sessionUpdate").and_then(Value::as_str) else {
            return;
        };
        match tag {
            "agent_message_chunk" | "agent_message" => {
                if let Some(raw) = extract_update_text(up) {
                    self.note_stdout_activity();
                    if let Some(text) = self.sideband.filter_message(&raw).filter(|s| !s.is_empty())
                    {
                        self.ensure_autonomous_turn(step);
                        let mid = up
                            .get("messageId")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        self.append_text(step, SegmentKind::Message, &text, mid);
                    }
                }
            }
            "agent_thought_chunk" | "agent_thought" => {
                if let Some(raw) = extract_update_text(up) {
                    self.note_stdout_activity();
                    if let Some(text) = self.sideband.filter_thought(&raw).filter(|s| !s.is_empty())
                    {
                        self.ensure_autonomous_turn(step);
                        self.append_text(step, SegmentKind::Thought, &text, None);
                    }
                }
            }
            "tool_call" => self.on_raw_tool_call(step, up),
            "tool_call_update" => self.on_raw_tool_update(step, up),
            "plan" => self.on_raw_plan(step, up),
            "usage_update" => {
                if let Some(used) = up.get("used").and_then(Value::as_u64) {
                    let size = up.get("size").and_then(Value::as_u64).unwrap_or(0);
                    step.push(DriverEvent::ContextUsage(ThreadTokenUsage {
                        used_tokens: used,
                        max_tokens: (size > 0).then_some(size),
                        input_tokens: None,
                        cached_input_tokens: None,
                        output_tokens: None,
                    }));
                }
            }
            "turn_completed" => {
                if let Some(usage) = parse_grok_usage(up.get("usage")) {
                    if self.turn_id.is_some() {
                        step.push(DriverEvent::Usage(usage));
                    }
                }
                let raw_stop = up
                    .get("stop_reason")
                    .or_else(|| up.get("stopReason"))
                    .and_then(Value::as_str);
                let (status, reason) = match raw_stop {
                    Some("cancelled") => (TurnTerminalStatus::Interrupted, None),
                    Some("end_turn") | None => (TurnTerminalStatus::Completed, None),
                    Some(other) => (TurnTerminalStatus::Completed, Some(other.to_string())),
                };
                self.settle_autonomous_now(step, status, reason);
            }
            "config_option_update" => {}
            other => {
                tracing::debug!(
                    provider = self.provider.as_str(),
                    session_update = other,
                    "ignoring unrecognized ACP session/update variant"
                );
            }
        }
    }

    fn on_raw_tool_call(&mut self, step: &mut DriverStep, up: &Value) {
        let Some(id) = up.get("toolCallId").and_then(Value::as_str) else {
            return;
        };
        self.note_stdout_activity();
        if self.sideband.filter_tool_call(id) {
            return;
        }
        self.ensure_autonomous_turn(step);
        self.close_segment(step);
        let raw_title = up.get("title").and_then(Value::as_str).unwrap_or("tool");
        let raw_input = up.get("rawInput").cloned();
        let classified = classify_tool(raw_title, raw_input.as_ref());
        let kind = up
            .get("kind")
            .and_then(|v| serde_json::from_value::<ToolKind>(v.clone()).ok())
            .unwrap_or(classified.kind);
        let status = parse_raw_tool_status(up.get("status")).unwrap_or(ToolCallStatus::InProgress);
        let file = extract_location_path(up).or(classified.file);
        let diff = extract_diff(up).or(classified.diff);
        let diffs = match (&file, &diff) {
            (Some(f), Some((old, new))) => vec![(f.clone(), old.clone(), new.clone())],
            _ => Vec::new(),
        };
        let mut t = Tool {
            title: classified.title,
            kind,
            status,
            raw_input,
            file,
            diff,
            diffs,
            ..Tool::default()
        };
        merge_raw_content(&mut t, up);
        Self::merge_raw_images(&mut t, up.get("rawOutput"));
        step.push(DriverEvent::TurnItem(self.tool_item(id, &t)));
        self.note_tool_background(step, id, &t);
        self.tools.insert(id.to_string(), t);
    }

    fn on_raw_tool_update(&mut self, step: &mut DriverStep, up: &Value) {
        let Some(id) = up.get("toolCallId").and_then(Value::as_str) else {
            return;
        };
        self.note_stdout_activity();
        let known = self.tools.contains_key(id);
        if self.sideband.filter_tool_update(id, known) {
            return;
        }
        self.ensure_autonomous_turn(step);
        self.close_segment(step);
        let mut t = self.tools.remove(id).unwrap_or_default();
        if let Some(k) = up
            .get("kind")
            .and_then(|v| serde_json::from_value::<ToolKind>(v.clone()).ok())
        {
            t.kind = k;
        }
        if let Some(s) = parse_raw_tool_status(up.get("status")) {
            t.status = s;
        }
        if let Some(title) = up.get("title").and_then(Value::as_str) {
            t.title = title.to_string();
        }
        if let Some(i) = up.get("rawInput").cloned() {
            t.raw_input = Some(i);
        }
        if let Some(f) = extract_location_path(up) {
            t.file = Some(f);
        }
        if up.get("content").is_some() || up.get("rawOutput").is_some() {
            t.output.clear();
            merge_raw_content(&mut t, up);
        }
        Self::merge_raw_images(&mut t, up.get("rawOutput"));
        step.push(DriverEvent::TurnItem(self.tool_item(id, &t)));
        self.note_tool_background(step, id, &t);
        self.tools.insert(id.to_string(), t);
    }

    fn on_raw_plan(&mut self, step: &mut DriverStep, up: &Value) {
        let Some(entries) = up.get("entries").and_then(Value::as_array) else {
            return;
        };
        self.ensure_autonomous_turn(step);
        self.close_segment(step);
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
        step.push(DriverEvent::Plan {
            steps,
            explanation: None,
        });
    }

    pub(super) fn apply_sideband_actions(
        &mut self,
        step: &mut DriverStep,
        actions: Vec<SidebandAction>,
    ) {
        for action in actions {
            match action {
                SidebandAction::TurnStarted => {
                    self.ensure_autonomous_turn(step);
                }
                SidebandAction::Thought { text, streaming } => {
                    self.ensure_autonomous_turn(step);
                    self.append_text(step, SegmentKind::Thought, &text, None);
                    if !streaming {
                        self.close_segment(step);
                    }
                }
                SidebandAction::Message {
                    text,
                    message_id,
                    streaming,
                } => {
                    self.ensure_autonomous_turn(step);
                    self.append_text(step, SegmentKind::Message, &text, message_id);
                    if !streaming {
                        self.close_segment(step);
                    }
                }
                SidebandAction::ToolStart {
                    id,
                    title,
                    kind,
                    raw_input,
                    file,
                    diff,
                } => {
                    self.ensure_autonomous_turn(step);
                    self.close_segment(step);
                    let diffs = match (&file, &diff) {
                        (Some(f), Some((old, new))) => vec![(f.clone(), old.clone(), new.clone())],
                        _ => Vec::new(),
                    };
                    let t = Tool {
                        title,
                        kind,
                        status: ToolCallStatus::InProgress,
                        raw_input,
                        file,
                        diff,
                        diffs,
                        ..Tool::default()
                    };
                    step.push(DriverEvent::TurnItem(self.tool_item(&id, &t)));
                    self.tools.insert(id, t);
                }
                SidebandAction::ToolUpdate {
                    id,
                    status,
                    title,
                    kind,
                    raw_input,
                    output,
                    file,
                    diff,
                } => {
                    self.ensure_autonomous_turn(step);
                    self.close_segment(step);
                    let mut t = self.tools.remove(&id).unwrap_or_default();
                    t.status = status;
                    if let Some(title) = title {
                        t.title = title;
                    }
                    if let Some(kind) = kind {
                        t.kind = kind;
                    }
                    if let Some(raw_input) = raw_input {
                        t.raw_input = Some(raw_input);
                    }
                    if let Some(f) = file {
                        t.file = Some(f);
                    }
                    if let Some((old, new)) = diff {
                        if let Some(path) = t.file.clone() {
                            t.diffs.retain(|(p, _, _)| *p != path);
                            t.diffs.push((path, old.clone(), new.clone()));
                        }
                        t.diff = Some((old, new));
                    }
                    if let Some(out) = output {
                        t.output = out;
                    }
                    step.push(DriverEvent::TurnItem(self.tool_item(&id, &t)));
                    self.note_tool_background(step, &id, &t);
                    self.tools.insert(id, t);
                }
                SidebandAction::Plan(steps) => {
                    self.ensure_autonomous_turn(step);
                    self.close_segment(step);
                    step.push(DriverEvent::Plan {
                        steps,
                        explanation: None,
                    });
                }
                SidebandAction::Usage(usage) => {
                    self.ensure_autonomous_turn(step);
                    step.push(DriverEvent::Usage(usage));
                }
                SidebandAction::TurnCompleted { reason } => {
                    let status = if reason.as_deref() == Some("cancelled") {
                        TurnTerminalStatus::Interrupted
                    } else {
                        TurnTerminalStatus::Completed
                    };
                    let clean_reason = reason.filter(|r| r != "cancelled");
                    self.settle_autonomous_now(step, status, clean_reason);
                }
            }
        }
    }
}

fn merge_raw_content(t: &mut Tool, up: &Value) {
    if let Some(content) = up
        .get("content")
        .and_then(|v| serde_json::from_value::<Vec<ToolCallContent>>(v.clone()).ok())
    {
        AcpDriver::merge_content(t, &content);
        if !t.output.is_empty() || !t.images.is_empty() || t.diff.is_some() {
            return;
        }
    }
    if let Some((old, new)) = extract_diff(up) {
        if let Some(path) = t.file.clone() {
            t.diffs.retain(|(p, _, _)| *p != path);
            t.diffs.push((path, old.clone(), new.clone()));
        }
        t.diff = Some((old, new));
    }
    if let Some(out) = extract_tool_output(up) {
        t.output = out;
    }
}
