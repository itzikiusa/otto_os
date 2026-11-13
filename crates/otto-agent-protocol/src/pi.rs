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
//!   `prompt {message, images?}`, `set_thinking_level {level}`, `abort`.
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

#[derive(serde::Serialize, serde::Deserialize)]
struct Tool {
    name: String,
    args: Value,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct PiDriver {
    program: String,
    next_id: u64,
    state_req: Option<String>,
    prompt_req: Option<String>,
    turn_id: Option<String>,
    /// The streaming assistant message: its item id and text so far.
    message: Option<(String, String)>,
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
            turn_id: None,
            message: None,
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

    fn finish(
        &mut self,
        step: &mut DriverStep,
        status: TurnTerminalStatus,
        failure: Option<ProviderFailure>,
    ) {
        if self.turn_id.is_none() {
            return;
        }
        self.close_message(step);
        self.turn_id = None;
        self.prompt_req = None;
        self.ended = None;
        self.tools.clear();
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
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

    fn on_text(&mut self, step: &mut DriverStep, delta: &str) {
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
            Some("message_update") => {
                let ev = &v["assistantMessageEvent"];
                if ev.get("type").and_then(Value::as_str) == Some("text_delta") {
                    let delta = ev.get("delta").and_then(Value::as_str).unwrap_or("");
                    self.on_text(step, delta);
                }
            }
            Some("message_end") => self.close_message(step),
            Some("tool_execution_start") => {
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
        }
    }
}

impl StructuredDriver for PiDriver {
    fn export_state(&self) -> Option<Value> {
        serde_json::to_value(self).ok()
    }

    fn import_state(&mut self, state: Value) -> Result<(), DriverError> {
        *self = serde_json::from_value(state).map_err(|e| DriverError(e.to_string()))?;
        Ok(())
    }

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
        if !req.images.is_empty() {
            cmd["images"] = Value::Array(
                req.images
                    .iter()
                    .map(|i| json!({"type": "image", "data": i.data_base64, "mimeType": i.mime}))
                    .collect(),
            );
        }
        let id = self.command(&mut step, cmd);
        self.prompt_req = Some(id);
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
            || v.get("type").and_then(Value::as_str) == Some("extension_ui_request")
        {
            self.on_event(&mut step, &v);
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(resume: Option<&str>, access_mode: RuntimeAccessMode) -> LaunchContext {
        LaunchContext {
            mcp_servers: Vec::new(),
            session_id: "s".into(),
            cwd: "/w".into(),
            model: Some("openai-codex/gpt-5.5".into()),
            resume: resume.map(str::to_string),
            sandboxed: false,
            access_mode,
        }
    }

    fn sent(step: &DriverStep, i: usize) -> Value {
        serde_json::from_str(&step.writes[i]).unwrap()
    }

    #[test]
    fn launch_maps_resume_model_and_access_to_tools() {
        let d = PiDriver::with_program("pi");
        let a = d
            .launch(&ctx(Some("abc"), RuntimeAccessMode::ApprovalRequired))
            .args;
        assert_eq!(
            a,
            [
                "--mode",
                "rpc",
                "--session",
                "abc",
                "--model",
                "openai-codex/gpt-5.5",
                "--tools",
                "read,grep,find,ls",
                "--exclude-tools",
                "mcp__*"
            ]
        );
        let full = d.launch(&ctx(None, RuntimeAccessMode::FullAccess)).args;
        assert!(!full.iter().any(|a| a == "--tools" || a == "--session"));
    }

    #[test]
    fn handshake_turn_tools_and_settle() {
        let mut d = PiDriver::with_program("pi");
        let s = d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        let st = sent(&s, 0);
        assert_eq!(st["type"], "get_state");
        let s = d.on_line(&format!(
            r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-1","model":{{"id":"gpt-5.5","provider":"openai-codex"}}}}}}"#,
            st["id"].as_str().unwrap()
        ));
        assert!(
            matches!(&s.events[0], DriverEvent::Ready { native_thread_id: Some(t), model: Some(m) } if t == "sess-1" && m == "openai-codex/gpt-5.5")
        );

        let mut req = RuntimePromptReq::of("hi");
        req.effort = Some("max".into());
        let s = d.prompt("t1", &req).unwrap();
        assert_eq!(sent(&s, 0)["level"], "max");
        let p = sent(&s, 1);
        assert_eq!(
            (p["type"].as_str(), p["message"].as_str()),
            (Some("prompt"), Some("hi"))
        );
        d.on_line(&format!(
            r#"{{"id":"{}","type":"response","command":"prompt","success":true,"data":{{"disposition":"started"}}}}"#,
            p["id"].as_str().unwrap()
        ));
        let s = d.on_line(r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hel"}}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Hel"))
        );
        d.on_line(r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"lo"}}"#);
        let s = d.on_line(r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"ls"}}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "Hello"))
        );
        assert!(
            matches!(&s.events[1], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::CommandExecution { input, .. } if input == "ls"))
        );
        let s = d.on_line(r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[{"type":"text","text":"a.rs"}]},"isError":false}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if i.status == TurnItemStatus::Completed && matches!(&i.kind, TurnItemKind::CommandExecution { output: Some(o), .. } if o == "a.rs"))
        );
        let s = d.on_line(r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#);
        assert!(matches!(
            &s.events[0],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        ));
        // A later agent_settled finds no open turn.
        assert!(d
            .on_line(r#"{"type":"agent_settled","aborted":false}"#)
            .events
            .is_empty());
    }

    /// Once pi has sent `agent_settled` (1.x), `agent_end` no longer ends the
    /// turn — `agent_settled` does, with the stashed outcome. The lines are
    /// pi 1.5.0's for a provider whose OAuth refresh fails (trimmed).
    #[test]
    fn a_settling_pi_ends_turns_at_agent_settled_with_the_runs_outcome() {
        let mut d = PiDriver::with_program("pi");
        d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
        let s = d.on_line(r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#);
        assert!(matches!(
            &s.events[0],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        ));
        // Learns that this pi settles; nothing is open any more.
        assert!(d
            .on_line(r#"{"type":"agent_settled","aborted":false}"#)
            .events
            .is_empty());

        d.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
        let s = d.on_line(r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"OAuth refresh failed for openai-codex"}}"#);
        assert!(s.events.is_empty());
        let s = d.on_line(r#"{"type":"agent_end","messages":[{"role":"system","content":""},{"role":"user","content":[]},{"role":"assistant","content":[],"stopReason":"error","errorMessage":"OAuth refresh failed for openai-codex"}],"willRetry":false}"#);
        assert!(s.events.is_empty(), "agent_end must wait for agent_settled");
        let s = d.on_line(r#"{"type":"agent_settled","aborted":false}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::Terminal { status: TurnTerminalStatus::Failed, failure: Some(f), .. } if f.message == "OAuth refresh failed for openai-codex")
        );

        // An abort wins over the stashed outcome.
        d.prompt("t3", &RuntimePromptReq::of("go")).unwrap();
        d.on_line(r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#);
        let s = d.on_line(r#"{"type":"agent_settled","aborted":true}"#);
        assert!(matches!(
            &s.events[0],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                ..
            }
        ));
    }

    #[test]
    fn a_refused_prompt_fails_and_dialogs_are_cancelled() {
        let mut d = PiDriver::with_program("pi");
        d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
        let p = sent(&s, 0);
        let s = d.on_line(&format!(
            r#"{{"id":"{}","type":"response","command":"prompt","success":false,"error":"No model"}}"#,
            p["id"].as_str().unwrap()
        ));
        assert!(
            matches!(&s.events[0], DriverEvent::Terminal { status: TurnTerminalStatus::Failed, failure: Some(f), .. } if f.message == "No model")
        );
        let s = d.on_line(
            r#"{"type":"extension_ui_request","id":"u1","method":"confirm","title":"Sure?"}"#,
        );
        let r = sent(&s, 0);
        assert_eq!(
            (r["type"].as_str(), r["cancelled"].as_bool()),
            (Some("extension_ui_response"), Some(true))
        );
    }
}
