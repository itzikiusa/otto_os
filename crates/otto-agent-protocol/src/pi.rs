//! pi (earendil-works/pi) driver — pi's own RPC mode (`pi --mode rpc`).
//!
//! T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.) drives pi the same way
//! (`packages/provider-pi`). The protocol (`docs/rpc.md`,
//! `docs/rpc-commands.md`, `docs/json.md` in the pi package; verified live
//! against pi 1.5.0, still compatible with 0.75):
//!
//! * argv — `pi --mode rpc [--session <id>] [--model provider/id]
//!   [--tools … --exclude-tools mcp__*]`. `--session` takes the id
//!   `get_state` reported (`data.sessionId`, a UUID), which is the resume key.
//! * stdin — strict JSONL commands with an optional `id`: `get_state`,
//!   `prompt {message, images?}`, `steer {message, images?}`,
//!   `set_thinking_level {level}`, `abort`.
//! * stdout — `response {id, command, success, data|error}` per command, and
//!   session events: `message_update {assistantMessageEvent: {type:
//!   text_delta|thinking_delta|toolcall_*, delta}}`, `message_end`,
//!   `tool_execution_start|end {toolCallId, toolName, args, result,
//!   isError}`, `agent_end {messages, willRetry}` and — pi 1.x —
//!   `agent_settled {aborted}`, after which pi does not continue on its own
//!   (an `agent_end` can still be followed by retries or compaction
//!   recovery). Once a process has sent `agent_settled` its turns end there;
//!   until then (pi 0.x never sends it) a non-retrying `agent_end` ends the
//!   turn. A prompt whose response disposition is `handled` started no run.
//!   A provider failure (auth, quota) is not a refused prompt: the prompt is
//!   `started` and the run ends with the last assistant message's
//!   `stopReason: "error"` + `errorMessage`.
//! * permissions — pi has NO approval round-trip: tools run unprompted. The
//!   access mode is therefore the TOOL SET: approval-required → read-only
//!   tools, auto-accept-edits → + edit/write, full-access → everything
//!   (bash included). MCP tools (`mcp__<server>__<tool>`, delivered by the
//!   host's launcher as a per-session `-e` extension) can write too, so the
//!   restricted modes exclude them. A switch respawns with `--session`.
//! * `extension_ui_request` dialogs (select / confirm / input / editor) are
//!   answered `cancelled` — Otto has no surface for an extension's dialog.
//! * interrupt — `abort`; the run then ends through the normal events.

use std::collections::HashMap;

use otto_core::agent_runtime::{
    ApprovalDecision, ProviderFailure, RuntimeAccessMode, RuntimePromptReq, RuntimeProvider,
    TurnItemKind, TurnItemStatus, TurnTerminalStatus,
};
use serde_json::{json, Value};

use crate::driver::{
    item, tool_error, DriverError, DriverEvent, DriverStep, LaunchContext, LaunchSpec,
    StructuredDriver,
};

/// pi's built-in tools per access mode (`--tools` allowlist; `None` = all).
fn tools_for(mode: RuntimeAccessMode) -> Option<&'static str> {
    match mode {
        RuntimeAccessMode::ApprovalRequired => Some("read,grep,find,ls"),
        RuntimeAccessMode::AutoAcceptEdits => Some("read,grep,find,ls,edit,write"),
        RuntimeAccessMode::FullAccess => None,
    }
}

/// Otto's effort names → pi's `--thinking` / `set_thinking_level` levels.
fn thinking_level(effort: &str) -> Option<&'static str> {
    Some(match effort.trim() {
        "minimal" => "minimal",
        "low" => "low",
        "medium" => "medium",
        "high" => "high",
        "xhigh" => "xhigh",
        // pi 1.x; a model without it clamps to its highest level.
        "max" | "ultra" => "max",
        "off" | "none" => "off",
        _ => return None,
    })
}

struct Tool {
    name: String,
    args: Value,
}

pub struct PiDriver {
    program: String,
    next_id: u64,
    state_req: Option<String>,
    prompt_req: Option<String>,
    steer_reqs: Vec<String>,
    ready: bool,
    turn_id: Option<String>,
    auto_turns: u64,
    /// The streaming assistant message: its item id and text so far.
    message: Option<(String, String)>,
    /// The streaming thinking/reasoning block: its item id and text so far.
    thought: Option<(String, String)>,
    messages: u32,
    tools: HashMap<String, Tool>,
    /// This pi sends `agent_settled` (1.x): a turn ends there, and a
    /// non-retrying `agent_end` only stashes its outcome in `ended`.
    settles: bool,
    ended: Option<(TurnTerminalStatus, Option<ProviderFailure>)>,
}

impl PiDriver {
    pub fn with_program(program: &str) -> Self {
        Self {
            program: program.into(),
            next_id: 0,
            state_req: None,
            prompt_req: None,
            steer_reqs: Vec::new(),
            ready: false,
            turn_id: None,
            auto_turns: 0,
            message: None,
            thought: None,
            messages: 0,
            tools: HashMap::new(),
            settles: false,
            ended: None,
        }
    }

    fn command(&mut self, step: &mut DriverStep, mut cmd: Value) -> String {
        self.next_id += 1;
        let id = format!("otto-{}", self.next_id);
        cmd["id"] = json!(id);
        step.write(cmd.to_string());
        id
    }

    fn turn(&self) -> &str {
        self.turn_id.as_deref().unwrap_or_default()
    }

    fn ensure_autonomous_turn(&mut self, step: &mut DriverStep) {
        if !self.ready || self.turn_id.is_some() {
            return;
        }
        self.auto_turns += 1;
        let id = format!("auto-{}", self.auto_turns);
        self.turn_id = Some(id.clone());
        self.messages = 0;
        self.ended = None;
        self.tools.clear();
        step.push(DriverEvent::TurnStarted { turn_id: id });
    }

    fn finish(
        &mut self,
        step: &mut DriverStep,
        status: TurnTerminalStatus,
        failure: Option<ProviderFailure>,
    ) {
        if self.turn_id.is_none() {
            return;
        }
        self.close_thought(step);
        self.close_message(step);
        self.turn_id = None;
        self.prompt_req = None;
        self.steer_reqs.clear();
        self.ended = None;
        self.tools.clear();
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
    }

    fn close_thought(&mut self, step: &mut DriverStep) {
        if let Some((id, text)) = self.thought.take() {
            if !text.is_empty() {
                step.push(DriverEvent::TurnItem(item(
                    id,
                    TurnItemStatus::Completed,
                    None,
                    TurnItemKind::Reasoning {
                        text,
                        streaming: false,
                    },
                )));
            }
        }
    }

    fn close_message(&mut self, step: &mut DriverStep) {
        if let Some((id, text)) = self.message.take() {
            if !text.is_empty() {
                step.push(DriverEvent::TurnItem(item(
                    id.clone(),
                    TurnItemStatus::Completed,
                    None,
                    TurnItemKind::AssistantMessage {
                        message_id: id,
                        text,
                        streaming: false,
                    },
                )));
            }
        }
    }

    fn on_thought(&mut self, step: &mut DriverStep, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.close_message(step);
        if self.thought.is_none() {
            self.messages += 1;
            self.thought = Some((
                format!("{}:thought:{}", self.turn(), self.messages),
                String::new(),
            ));
        }
        let (id, text) = self.thought.as_mut().expect("set above");
        text.push_str(delta);
        step.push(DriverEvent::TurnItem(item(
            id.clone(),
            TurnItemStatus::Running,
            None,
            TurnItemKind::Reasoning {
                text: text.clone(),
                streaming: true,
            },
        )));
    }

    fn on_text(&mut self, step: &mut DriverStep, delta: &str) {
        if delta.is_empty() {
            return;
        }
        self.close_thought(step);
        if self.message.is_none() {
            self.messages += 1;
            self.message = Some((
                format!("{}:msg:{}", self.turn(), self.messages),
                String::new(),
            ));
        }
        let (id, text) = self.message.as_mut().expect("set above");
        text.push_str(delta);
        step.push(DriverEvent::TurnItem(item(
            id.clone(),
            TurnItemStatus::Running,
            None,
            TurnItemKind::AssistantMessage {
                message_id: id.clone(),
                text: text.clone(),
                streaming: true,
            },
        )));
    }

    fn tool_kind(name: &str, args: &Value, output: Option<String>) -> TurnItemKind {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        match name {
            "bash" => TurnItemKind::CommandExecution {
                input: s("command").unwrap_or_default(),
                output,
                exit_code: None,
            },
            "edit" | "write" => TurnItemKind::FileChange {
                file_name: s("path").or_else(|| s("file_path")).unwrap_or_default(),
                diff_str: None,
                old_str: s("oldText").or_else(|| s("old_string")),
                new_str: s("newText")
                    .or_else(|| s("content"))
                    .or_else(|| s("new_string")),
            },
            _ => TurnItemKind::DynamicTool {
                tool_name: name.to_string(),
                input: args.clone(),
                output,
                images: Vec::new(),
            },
        }
    }

    fn result_text(result: &Value) -> Option<String> {
        let text: Vec<&str> = result
            .get("content")
            .and_then(Value::as_array)?
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect();
        (!text.is_empty()).then(|| text.join("\n"))
    }

    /// An `agent_end`'s outcome, from its last message's `stopReason`.
    fn run_outcome(v: &Value) -> (TurnTerminalStatus, Option<ProviderFailure>) {
        let last = v
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|m| m.last());
        match last
            .and_then(|m| m.get("stopReason"))
            .and_then(Value::as_str)
        {
            Some("aborted") => (TurnTerminalStatus::Interrupted, None),
            Some("error") => (
                TurnTerminalStatus::Failed,
                Some(ProviderFailure {
                    class: "provider_error".into(),
                    message: last
                        .and_then(|m| m.get("errorMessage"))
                        .and_then(Value::as_str)
                        .unwrap_or("pi reported an error")
                        .to_string(),
                    code: None,
                    retryable: None,
                }),
            ),
            _ => (TurnTerminalStatus::Completed, None),
        }
    }

    fn on_event(&mut self, step: &mut DriverStep, v: &Value) {
        match v.get("type").and_then(Value::as_str) {
            Some("agent_start") => {
                self.ensure_autonomous_turn(step);
            }
            Some("message_start") => {
                self.ensure_autonomous_turn(step);
                if self.turn_id.is_some() {
                    self.close_thought(step);
                    self.close_message(step);
                }
            }
            Some("message_update") => {
                let ev = &v["assistantMessageEvent"];
                match ev.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        let delta = ev.get("delta").and_then(Value::as_str).unwrap_or("");
                        if !delta.is_empty() {
                            self.ensure_autonomous_turn(step);
                            if self.turn_id.is_some() {
                                self.on_text(step, delta);
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        let delta = ev.get("delta").and_then(Value::as_str).unwrap_or("");
                        if !delta.is_empty() {
                            self.ensure_autonomous_turn(step);
                            if self.turn_id.is_some() {
                                self.on_thought(step, delta);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("message_end") if self.turn_id.is_some() => {
                self.close_thought(step);
                self.close_message(step);
            }
            Some("tool_execution_start") => {
                self.ensure_autonomous_turn(step);
                if self.turn_id.is_none() {
                    return;
                }
                self.close_thought(step);
                self.close_message(step);
                let call = v
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let name = v
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let args = v.get("args").cloned().unwrap_or(Value::Null);
                let id = format!("{}:tool:{call}", self.turn());
                step.push(DriverEvent::TurnItem(item(
                    id,
                    TurnItemStatus::Running,
                    Some(name.clone()),
                    Self::tool_kind(&name, &args, None),
                )));
                self.tools.insert(call.to_string(), Tool { name, args });
            }
            Some("tool_execution_end") => {
                let call = v
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let tool = self.tools.remove(call).unwrap_or_else(|| Tool {
                    name: v
                        .get("toolName")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .into(),
                    args: v.get("args").cloned().unwrap_or(Value::Null),
                });
                let output = Self::result_text(&v["result"]);
                let failed = v.get("isError").and_then(Value::as_bool) == Some(true);
                let mut it = item(
                    format!("{}:tool:{call}", self.turn()),
                    if failed {
                        TurnItemStatus::Failed
                    } else {
                        TurnItemStatus::Completed
                    },
                    Some(tool.name.clone()),
                    Self::tool_kind(&tool.name, &tool.args, output.clone()),
                );
                if failed {
                    it.error = output.as_deref().and_then(tool_error);
                }
                step.push(DriverEvent::TurnItem(it));
            }
            // pi will not continue on its own: the turn is over, with the
            // last run's outcome (an abort wins).
            Some("agent_settled") => {
                let aborted = v.get("aborted").and_then(Value::as_bool) == Some(true);
                let (status, failure) = match self.ended.take() {
                    _ if aborted => (TurnTerminalStatus::Interrupted, None),
                    Some(ended) => ended,
                    None => (TurnTerminalStatus::Completed, None),
                };
                self.finish(step, status, failure);
            }
            // A run that will not retry. pi 0.x has no `agent_settled`, so
            // until this process has sent one the run ends the turn (a later
            // `agent_settled` then finds no open turn).
            Some("agent_end") if v.get("willRetry").and_then(Value::as_bool) != Some(true) => {
                let (status, failure) = Self::run_outcome(v);
                if self.settles {
                    self.ended = Some((status, failure));
                } else {
                    self.finish(step, status, failure);
                }
            }
            Some("extension_ui_request") => {
                let blocking = matches!(
                    v.get("method").and_then(Value::as_str),
                    Some("select" | "confirm" | "input" | "editor")
                );
                if let (true, Some(id)) = (blocking, v.get("id")) {
                    step.write(
                        json!({"type": "extension_ui_response", "id": id, "cancelled": true})
                            .to_string(),
                    );
                }
            }
            _ => {}
        }
    }

    fn on_response(&mut self, step: &mut DriverStep, v: &Value) {
        let id = v.get("id").and_then(Value::as_str);
        let ok = v.get("success").and_then(Value::as_bool) == Some(true);
        if id.is_some() && id == self.state_req.as_deref() {
            self.state_req = None;
            if ok {
                self.ready = true;
                step.push(DriverEvent::Ready {
                    native_thread_id: v
                        .pointer("/data/sessionId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    model: v
                        .pointer("/data/model/provider")
                        .and_then(Value::as_str)
                        .zip(v.pointer("/data/model/id").and_then(Value::as_str))
                        .map(|(p, m)| format!("{p}/{m}")),
                });
            } else {
                step.push(DriverEvent::Fatal(ProviderFailure {
                    class: "handshake_error".into(),
                    message: v
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("pi refused get_state")
                        .to_string(),
                    code: None,
                    retryable: None,
                }));
            }
        } else if id.is_some() && id == self.prompt_req.as_deref() {
            self.prompt_req = None;
            if !ok {
                let message = v
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("pi refused the prompt")
                    .to_string();
                self.finish(
                    step,
                    TurnTerminalStatus::Failed,
                    Some(ProviderFailure {
                        class: "provider_error".into(),
                        message,
                        code: None,
                        retryable: None,
                    }),
                );
            } else if v.pointer("/data/disposition").and_then(Value::as_str) == Some("handled") {
                self.finish(step, TurnTerminalStatus::Completed, None);
            }
        } else if let Some(pos) = id.and_then(|id| self.steer_reqs.iter().position(|r| r == id)) {
            self.steer_reqs.swap_remove(pos);
            if !ok {
                let message = v
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("pi refused the steer")
                    .to_string();
                step.push(DriverEvent::Error(ProviderFailure {
                    class: "provider_error".into(),
                    message,
                    code: None,
                    retryable: None,
                }));
            }
        }
    }

    fn prompt_images(req: &RuntimePromptReq) -> Option<Value> {
        (!req.images.is_empty()).then(|| {
            Value::Array(
                req.images
                    .iter()
                    .map(|i| json!({"type": "image", "data": i.data_base64, "mimeType": i.mime}))
                    .collect(),
            )
        })
    }
}

impl StructuredDriver for PiDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Pi
    }

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec {
        let mut args: Vec<String> = vec!["--mode".into(), "rpc".into()];
        if let Some(id) = &ctx.resume {
            args.extend(["--session".into(), id.clone()]);
        }
        if let Some(m) = &ctx.model {
            args.extend(["--model".into(), m.clone()]);
        }
        if let Some(t) = tools_for(ctx.access_mode) {
            // `--tools` keeps MCP tools, which can write: drop them too.
            args.extend(["--tools".into(), t.into()]);
            args.extend(["--exclude-tools".into(), "mcp__*".into()]);
        }
        LaunchSpec {
            program: self.program.clone(),
            args,
            ..LaunchSpec::default()
        }
    }

    /// `get_state` reports the session id (the resume key) and the model.
    fn start(&mut self, _ctx: &LaunchContext) -> DriverStep {
        let mut step = DriverStep::default();
        let id = self.command(&mut step, json!({"type": "get_state"}));
        self.state_req = Some(id);
        step
    }

    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        if !req.files.is_empty() {
            return Err(DriverError(
                "pi takes text and images only (no files)".into(),
            ));
        }
        let mut step = DriverStep::default();
        if let Some(e) = req.effort.as_deref().filter(|e| !e.trim().is_empty()) {
            let level = thinking_level(e)
                .ok_or_else(|| DriverError(format!("pi has no '{e}' thinking level")))?;
            self.command(
                &mut step,
                json!({"type": "set_thinking_level", "level": level}),
            );
        }
        self.turn_id = Some(turn_id.to_string());
        self.messages = 0;
        let mut cmd = json!({"type": "prompt", "message": req.text});
        if let Some(images) = Self::prompt_images(req) {
            cmd["images"] = images;
        }
        let id = self.command(&mut step, cmd);
        self.prompt_req = Some(id);
        Ok(step)
    }

    fn steer(&mut self, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if !self.ready || self.turn_id.is_none() {
            return Err(DriverError("no prompt is running".into()));
        }
        if !req.files.is_empty() {
            return Err(DriverError(
                "pi takes text and images only (no files)".into(),
            ));
        }
        if req.text.is_empty() && req.images.is_empty() {
            return Err(DriverError("empty steer input".into()));
        }
        let level = match req.effort.as_deref().filter(|e| !e.trim().is_empty()) {
            Some(e) => Some(
                thinking_level(e)
                    .ok_or_else(|| DriverError(format!("pi has no '{e}' thinking level")))?,
            ),
            None => None,
        };
        let mut step = DriverStep::default();
        if let Some(level) = level {
            self.command(
                &mut step,
                json!({"type": "set_thinking_level", "level": level}),
            );
        }
        let mut cmd = json!({"type": "steer", "message": req.text});
        if let Some(images) = Self::prompt_images(req) {
            cmd["images"] = images;
        }
        let id = self.command(&mut step, cmd);
        self.steer_reqs.push(id);
        Ok(step)
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        self.command(&mut step, json!({"type": "abort"}));
        step
    }

    fn respond(
        &mut self,
        request_id: &str,
        _: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        Err(DriverError(format!("unknown request {request_id}")))
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return step;
        };
        if v.get("type").and_then(Value::as_str) == Some("agent_settled") {
            // Even after the turn closed (at a pi-0.x-style `agent_end`).
            self.settles = true;
        }
        if v.get("type").and_then(Value::as_str) == Some("response") {
            self.on_response(&mut step, &v);
        } else if self.turn_id.is_some()
            || self.ready
            || v.get("type").and_then(Value::as_str) == Some("extension_ui_request")
        {
            self.on_event(&mut step, &v);
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        self.steer_reqs.clear();
        DriverStep::default()
    }
}

#[cfg(test)]
#[path = "pi_tests.rs"]
mod tests;
