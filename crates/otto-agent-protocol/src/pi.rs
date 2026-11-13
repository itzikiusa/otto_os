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
//! * `extension_ui_request` dialogs (T3 Code `provider-pi/adapter.ts`):
//!   `confirm` is an approval (allow → `confirmed: true`), `select` a
//!   single-choice question, `input` / `editor` a text question; each is
//!   answered with `extension_ui_response {id, value|confirmed|cancelled}`.
//! * usage — each assistant `message_end` carries its `usage` (tokens and
//!   `cost.total`): the turn sums them, and the last one is the context it
//!   filled against the model's `contextWindow` (`get_state`).
//! * `get_commands` (after `get_state`) lists its prompt templates, skills
//!   and extension commands — run as a `/name` prompt.
//! * interrupt — `abort`; the run then ends through the normal events.

use std::collections::HashMap;

use otto_core::agent_runtime::{
    ApprovalDecision, ProviderCommand, ProviderFailure, QuestionKind, QuestionOption, RequestKind,
    RuntimeAccessMode, RuntimePromptReq, RuntimeProvider, RuntimeQuestion, ThreadTokenUsage,
    TurnItemKind, TurnItemStatus, TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, rpc_key, tool_error, with_file_paths, DriverError, DriverEvent, DriverStep,
    LaunchContext, LaunchSpec, StructuredDriver,
};

/// A multi-edit `edit` call as one patch: a hunk per `(oldText, newText)`.
fn edit_hunks(path: &str, edits: &[(String, String)]) -> String {
    let p = path.trim_start_matches('/');
    let mut out = format!("--- a/{p}\n+++ b/{p}\n");
    for (old, new) in edits {
        let (o, n): (Vec<&str>, Vec<&str>) = (old.lines().collect(), new.lines().collect());
        out.push_str(&format!("@@ -1,{} +1,{} @@\n", o.len(), n.len()));
        for l in o {
            out.push_str(&format!("-{l}\n"));
        }
        for l in n {
            out.push_str(&format!("+{l}\n"));
        }
    }
    out
}

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

/// An open `extension_ui_request` dialog.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Dialog {
    /// pi's request id, echoed in the response.
    pi_id: Value,
    method: String,
    questions: Vec<RuntimeQuestion>,
}

/// The running turn's summed usage.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Copy)]
struct Spent {
    input: u64,
    output: u64,
    cached: u64,
    cost: f64,
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
    /// The streaming thinking block: its item id and text so far.
    #[serde(default)]
    thinking: Option<(String, String)>,
    #[serde(default)]
    thoughts: u32,
    /// Bytes of each running tool's streamed output already sent.
    #[serde(default)]
    streamed: HashMap<String, usize>,
    #[serde(default)]
    spent: Spent,
    /// The model's context window (`get_state` `model.contextWindow`).
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    commands_req: Option<String>,
    /// Open extension dialogs by Otto request id.
    #[serde(default)]
    dialogs: HashMap<String, Dialog>,
    /// User MCP servers of this spawn (from its `-e` extension): the
    /// restricted access modes leave them out, Otto's own tools stay.
    #[serde(default)]
    user_mcp: Vec<String>,
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
            thinking: None,
            thoughts: 0,
            streamed: HashMap::new(),
            spent: Spent::default(),
            context_window: None,
            commands_req: None,
            dialogs: HashMap::new(),
            user_mcp: Vec::new(),
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
        self.close_thinking(step);
        self.cancel_dialogs(step);
        self.turn_id = None;
        self.prompt_req = None;
        self.ended = None;
        self.tools.clear();
        self.streamed.clear();
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

    fn close_thinking(&mut self, step: &mut DriverStep) {
        if let Some((id, text)) = self.thinking.take() {
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

    /// pi's `thinking_delta`: the model's reasoning, as its own item.
    fn on_thinking(&mut self, step: &mut DriverStep, delta: &str) {
        self.close_message(step);
        if self.thinking.is_none() {
            self.thoughts += 1;
            self.thinking = Some((
                format!("{}:thinking:{}", self.turn(), self.thoughts),
                String::new(),
            ));
        }
        let (id, text) = self.thinking.as_mut().expect("set above");
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

    /// An assistant `message_end`'s `usage`: the turn's sum so far, and the
    /// context that message filled.
    fn on_usage(&mut self, step: &mut DriverStep, msg: &Value) {
        let Some(u) = msg.get("usage").filter(|u| u.is_object()) else {
            return;
        };
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let cached = n("cacheRead");
        let input = n("input") + cached + n("cacheWrite");
        self.spent.input += input;
        self.spent.output += n("output");
        self.spent.cached += cached;
        if let Some(c) = u
            .pointer("/cost/total")
            .and_then(Value::as_f64)
            .filter(|c| c.is_finite())
        {
            self.spent.cost += c;
        }
        let sp = self.spent;
        step.push(DriverEvent::Usage(TurnTokenUsage {
            input_tokens: sp.input,
            output_tokens: sp.output,
            cached_input_tokens: (sp.cached > 0).then_some(sp.cached),
            reasoning_tokens: None,
            cost_usd: (sp.cost > 0.0).then_some(sp.cost),
        }));
        step.push(DriverEvent::ContextUsage(ThreadTokenUsage {
            used_tokens: input + n("output"),
            max_tokens: self.context_window,
            input_tokens: Some(input),
            cached_input_tokens: (cached > 0).then_some(cached),
            output_tokens: Some(n("output")),
        }));
    }

    /// An extension's dialog → an approval (`confirm`) or a question.
    fn on_dialog(&mut self, step: &mut DriverStep, v: &Value) {
        let Some(pi_id) = v.get("id").cloned() else {
            return;
        };
        let method = v.get("method").and_then(Value::as_str).unwrap_or_default();
        let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let title = s("title").unwrap_or_else(|| "pi asks".into());
        let request_id = format!("pi-dialog-{}", rpc_key(&pi_id));
        match method {
            "confirm" => {
                self.dialogs.insert(
                    request_id.clone(),
                    Dialog {
                        pi_id,
                        method: method.into(),
                        questions: Vec::new(),
                    },
                );
                step.push(DriverEvent::Request {
                    id: request_id,
                    kind: RequestKind::Permission,
                    title: Some(title),
                    prompt: s("message"),
                    reason: None,
                    options: Vec::new(),
                    tool_item_id: None,
                });
            }
            "select" | "input" | "editor" => {
                let options: Vec<QuestionOption> = v
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|o| QuestionOption {
                        value: o.to_string(),
                        label: o.to_string(),
                        description: None,
                    })
                    .collect();
                let select = method == "select";
                let question = RuntimeQuestion {
                    id: "answer".into(),
                    header: title.clone(),
                    prompt: s("message")
                        .or_else(|| s("placeholder"))
                        .or_else(|| s("prefill"))
                        .unwrap_or_else(|| title.clone()),
                    kind: if select {
                        QuestionKind::Single
                    } else {
                        QuestionKind::Text
                    },
                    required: true,
                    allow_empty: method == "editor",
                    allow_free_text: !select,
                    options,
                };
                self.dialogs.insert(
                    request_id.clone(),
                    Dialog {
                        pi_id,
                        method: method.into(),
                        questions: vec![question.clone()],
                    },
                );
                step.push(DriverEvent::Question {
                    id: request_id,
                    title: Some(title),
                    questions: vec![question],
                    tool_item_id: None,
                });
            }
            _ => {}
        }
    }

    /// pi gave up on a dialog (its `timeout`) or the turn ended: answer it
    /// cancelled and close its request.
    fn cancel_dialogs(&mut self, step: &mut DriverStep) {
        let mut open: Vec<(String, Dialog)> = self.dialogs.drain().collect();
        open.sort_by(|a, b| a.0.cmp(&b.0));
        for (id, d) in open {
            step.write(
                json!({"type": "extension_ui_response", "id": d.pi_id, "cancelled": true})
                    .to_string(),
            );
            step.push(DriverEvent::RequestClosed {
                request_id: id,
                status: otto_core::agent_runtime::RuntimeRequestStatus::Cancelled,
            });
        }
    }

    fn tool_kind(name: &str, args: &Value, output: Option<String>) -> TurnItemKind {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        match name {
            "bash" => TurnItemKind::CommandExecution {
                input: s("command").unwrap_or_default(),
                output,
                exit_code: None,
            },
            // pi's `edit` takes `edits: [{oldText, newText}]` (a lone
            // top-level pair is the legacy shape): one hunk per edit.
            "edit" | "write" => {
                let file = s("path").or_else(|| s("file_path")).unwrap_or_default();
                let mut edits: Vec<(String, String)> = args
                    .get("edits")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let t =
                            |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                        (t("oldText"), t("newText"))
                    })
                    .collect();
                if let (true, Some(old), Some(new)) = (
                    edits.is_empty(),
                    s("oldText").or_else(|| s("old_string")),
                    s("newText").or_else(|| s("new_string")),
                ) {
                    edits.push((old, new));
                }
                match edits.as_slice() {
                    [] => TurnItemKind::FileChange {
                        file_name: file,
                        diff_str: None,
                        old_str: None,
                        new_str: s("content"),
                    },
                    [(old, new)] => TurnItemKind::FileChange {
                        file_name: file,
                        diff_str: None,
                        old_str: Some(old.clone()),
                        new_str: Some(new.clone()),
                    },
                    many => TurnItemKind::FileChange {
                        diff_str: Some(edit_hunks(&file, many)),
                        file_name: file,
                        old_str: None,
                        new_str: None,
                    },
                }
            }
            _ => TurnItemKind::DynamicTool {
                image_files: Vec::new(),
                tool_name: name.to_string(),
                input: args.clone(),
                output,
                images: Vec::new(),
            },
        }
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
                let delta = ev.get("delta").and_then(Value::as_str).unwrap_or("");
                match ev.get("type").and_then(Value::as_str) {
                    Some("text_delta") => {
                        self.close_thinking(step);
                        self.on_text(step, delta);
                    }
                    Some("thinking_delta") => self.on_thinking(step, delta),
                    _ => {}
                }
            }
            Some("message_end") => {
                self.close_thinking(step);
                self.close_message(step);
                if v.pointer("/message/role").and_then(Value::as_str) == Some("assistant") {
                    self.on_usage(step, &v["message"]);
                }
            }
            Some("tool_execution_update") => {
                let call = v
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                if v.get("toolName").and_then(Value::as_str) != Some("bash") {
                    return;
                }
                let text: String = v
                    .pointer("/partialResult/content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .collect();
                let sent = self.streamed.entry(call.to_string()).or_default();
                if text.len() > *sent && text.is_char_boundary(*sent) {
                    let chunk = text[*sent..].to_string();
                    *sent = text.len();
                    step.push(DriverEvent::CommandOutput {
                        item_id: format!("{}:tool:{call}", self.turn()),
                        chunk,
                    });
                } else if text.len() < *sent {
                    // pi trimmed its window: only the end is kept now.
                    *sent = text.len();
                }
            }
            Some("tool_execution_start") => {
                self.close_thinking(step);
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
                // pi's `read` of an image file returns `{type: image, data, mimeType}`.
                let media = crate::tool_images::extract(Some(&v["result"]));
                let output = (!media.text.is_empty()).then_some(media.text);
                let failed = v.get("isError").and_then(Value::as_bool) == Some(true);
                let mut kind = Self::tool_kind(&tool.name, &tool.args, output.clone());
                if let TurnItemKind::DynamicTool { images, .. } = &mut kind {
                    *images = media.images;
                }
                let mut it = item(
                    format!("{}:tool:{call}", self.turn()),
                    if failed {
                        TurnItemStatus::Failed
                    } else {
                        TurnItemStatus::Completed
                    },
                    Some(tool.name.clone()),
                    kind,
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
            Some("extension_ui_request") => self.on_dialog(step, v),
            _ => {}
        }
    }

    fn on_response(&mut self, step: &mut DriverStep, v: &Value) {
        let id = v.get("id").and_then(Value::as_str);
        let ok = v.get("success").and_then(Value::as_bool) == Some(true);
        if id.is_some() && id == self.commands_req.as_deref() {
            self.commands_req = None;
            let list = v.pointer("/data/commands").and_then(Value::as_array);
            if let (true, Some(list)) = (ok, list) {
                step.push(DriverEvent::Commands(
                    list.iter()
                        .filter_map(|c| {
                            Some(ProviderCommand::new(
                                c.get("name")?.as_str()?,
                                c.get("description").and_then(Value::as_str).unwrap_or(""),
                            ))
                        })
                        .collect(),
                ));
            }
        } else if id.is_some() && id == self.state_req.as_deref() {
            self.state_req = None;
            if ok {
                self.context_window = v
                    .pointer("/data/model/contextWindow")
                    .and_then(Value::as_u64);
                let cid = self.command(step, json!({"type": "get_commands"}));
                self.commands_req = Some(cid);
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
        } else if !ok
            && v.get("command").and_then(Value::as_str) == Some("steer")
            && self.turn_id.is_some()
        {
            let why = v
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("pi refused it");
            step.push(DriverEvent::TurnItem(item(
                format!("{}:steer-failed:{}", self.turn(), id.unwrap_or_default()),
                TurnItemStatus::Failed,
                Some("pi did not take the message".into()),
                TurnItemKind::SystemNotice {
                    message: format!("pi did not take the message: {why}"),
                },
            )));
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
            args.extend(["--tools".into(), t.into()]);
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
        self.thoughts = 0;
        self.spent = Spent::default();
        let mut cmd = json!({"type": "prompt", "message": with_file_paths(&req.text, &req.files)});
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

    /// pi's RPC `steer {message, images?}`: delivered after the current
    /// assistant turn's tool calls, before its next model call.
    fn steer(&mut self, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_none() {
            return Err(DriverError("no turn is running".into()));
        }
        let mut cmd = json!({"type": "steer", "message": with_file_paths(&req.text, &req.files)});
        if !req.images.is_empty() {
            cmd["images"] = Value::Array(
                req.images
                    .iter()
                    .map(|i| json!({"type": "image", "data": i.data_base64, "mimeType": i.mime}))
                    .collect(),
            );
        }
        let mut step = DriverStep::default();
        self.command(&mut step, cmd);
        Ok(step)
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        self.command(&mut step, json!({"type": "abort"}));
        step
    }

    /// An extension's `confirm`: allowed → `confirmed: true`.
    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        let d = self
            .dialogs
            .get(request_id)
            .filter(|d| d.method == "confirm")
            .ok_or_else(|| DriverError(format!("unknown request {request_id}")))?;
        let yes = matches!(
            decision,
            ApprovalDecision::Accept
                | ApprovalDecision::AcceptForSession
                | ApprovalDecision::AcceptAlways
        );
        let answer = if decision == ApprovalDecision::Cancel {
            json!({"type": "extension_ui_response", "id": d.pi_id, "cancelled": true})
        } else {
            json!({"type": "extension_ui_response", "id": d.pi_id, "confirmed": yes})
        };
        let mut step = DriverStep::default();
        step.write(answer.to_string());
        self.dialogs.remove(request_id);
        Ok(step)
    }

    /// An extension's `select` / `input` / `editor`: the answer's value.
    fn respond_question(
        &mut self,
        request_id: &str,
        answers: &[otto_core::agent_runtime::QuestionAnswer],
    ) -> Result<DriverStep, DriverError> {
        let d = self
            .dialogs
            .get(request_id)
            .filter(|d| d.method != "confirm")
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        otto_core::agent_runtime::validate_question_answers(&d.questions, answers)
            .map_err(DriverError)?;
        let value = answers
            .first()
            .and_then(|a| a.values().into_iter().next())
            .unwrap_or_default();
        let mut step = DriverStep::default();
        step.write(
            json!({"type": "extension_ui_response", "id": d.pi_id, "value": value}).to_string(),
        );
        self.dialogs.remove(request_id);
        Ok(step)
    }

    fn dismiss_question(
        &mut self,
        request_id: &str,
        _action: otto_core::agent_runtime::QuestionDismissAction,
    ) -> Result<DriverStep, DriverError> {
        let d = self
            .dialogs
            .remove(request_id)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let mut step = DriverStep::default();
        step.write(
            json!({"type": "extension_ui_response", "id": d.pi_id, "cancelled": true}).to_string(),
        );
        Ok(step)
    }

    /// The restricted modes (`--tools` set) leave out the user's MCP
    /// servers — pi runs every tool unasked — but keep Otto's own (its
    /// control plane governs them). Their names come from this spawn's
    /// `-e` extension (`const servers = {…};`).
    fn adopt_spec(&mut self, spec: &mut LaunchSpec) {
        if !spec.args.iter().any(|a| a == "--tools") {
            return;
        }
        let ext = spec
            .args
            .windows(2)
            .find(|w| w[0] == "-e")
            .map(|w| std::path::PathBuf::from(&w[1]));
        let names: Vec<String> = ext
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|doc| {
                let line = doc
                    .lines()
                    .find_map(|l| l.strip_prefix("const servers = "))?;
                serde_json::from_str::<serde_json::Map<String, Value>>(line.trim_end_matches(';'))
                    .ok()
            })
            .map(|m| {
                m.keys()
                    .filter(|k| !matches!(k.as_str(), "otto" | "otto-browser"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        self.user_mcp = names;
        if !self.user_mcp.is_empty() {
            let patterns: Vec<String> = self
                .user_mcp
                .iter()
                .flat_map(|n| {
                    let unified = n.replace('-', "_");
                    let mut v = vec![format!("mcp__{n}__*")];
                    if unified != *n {
                        v.push(format!("mcp__{unified}__*"));
                    }
                    v
                })
                .collect();
            spec.args
                .extend(["--exclude-tools".into(), patterns.join(",")]);
        }
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
            ]
        );
        let full = d.launch(&ctx(None, RuntimeAccessMode::FullAccess)).args;
        assert!(!full.iter().any(|a| a == "--tools" || a == "--session"));
    }

    #[test]
    fn restricted_modes_leave_out_the_users_mcp_servers_but_keep_ottos() {
        let dir = std::env::temp_dir().join(format!("otto-pi-ext-{}", otto_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ext = dir.join("mcp.ts");
        std::fs::write(
            &ext,
            "// Otto-managed\nconst servers = {\"otto\":{},\"otto-browser\":{},\"git-hub\":{}};\nexport default function (pi) {}\n",
        )
        .unwrap();
        let mut d = PiDriver::with_program("pi");
        let mut spec = d.launch(&ctx(None, RuntimeAccessMode::AutoAcceptEdits));
        spec.args.extend(["-e".into(), ext.display().to_string()]);
        d.adopt_spec(&mut spec);
        let at = spec
            .args
            .iter()
            .position(|a| a == "--exclude-tools")
            .unwrap();
        assert_eq!(spec.args[at + 1], "mcp__git-hub__*,mcp__git_hub__*");
        // Full access runs everything.
        let mut full = d.launch(&ctx(None, RuntimeAccessMode::FullAccess));
        full.args.extend(["-e".into(), ext.display().to_string()]);
        d.adopt_spec(&mut full);
        assert!(!full.args.iter().any(|a| a == "--exclude-tools"));
        let _ = std::fs::remove_file(&ext);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn thinking_multi_edits_usage_and_commands() {
        let mut d = PiDriver::with_program("pi");
        let s = d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        let st = sent(&s, 0);
        let s = d.on_line(&format!(
            r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"s1","model":{{"id":"m","provider":"p","contextWindow":200000}}}}}}"#,
            st["id"].as_str().unwrap()
        ));
        let gc = sent(&s, 0);
        assert_eq!(gc["type"], "get_commands");
        let s = d.on_line(&format!(
            r#"{{"id":"{}","type":"response","command":"get_commands","success":true,"data":{{"commands":[{{"name":"fix-tests","description":"Fix failing tests"}},{{"name":"skill:brave"}}]}}}}"#,
            gc["id"].as_str().unwrap()
        ));
        let DriverEvent::Commands(c) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(
            c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["fix-tests", "skill:brave"]
        );
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.on_line(r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"hmm"}}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::Reasoning { text, streaming: true } if text == "hmm"))
        );
        let s = d.on_line(r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"ok"}}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::Reasoning { streaming: false, .. }))
        );
        let s = d.on_line(r#"{"type":"message_end","message":{"role":"assistant","usage":{"input":100,"output":50,"cacheRead":20,"cacheWrite":0,"cost":{"total":0.0105}}}}"#);
        let usage = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::Usage(u) => Some(u.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            (usage.input_tokens, usage.output_tokens, usage.cost_usd),
            (120, 50, Some(0.0105))
        );
        let ctxu = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::ContextUsage(u) => Some(u.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!((ctxu.used_tokens, ctxu.max_tokens), (170, Some(200000)));
        d.on_line(r#"{"type":"tool_execution_start","toolCallId":"e1","toolName":"edit","args":{"path":"/w/a.rs","edits":[{"oldText":"a","newText":"b"},{"oldText":"c","newText":"d"}]}}"#);
        let s = d.on_line(r#"{"type":"tool_execution_end","toolCallId":"e1","toolName":"edit","result":{"content":[{"type":"text","text":"ok"}],"details":{"diff":"x"}},"isError":false}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::FileChange { diff_str: Some(p), .. } if p.contains("-a\n+b\n") && p.contains("-c\n+d\n")))
        );
        d.on_line(r#"{"type":"tool_execution_start","toolCallId":"b1","toolName":"bash","args":{"command":"ls"}}"#);
        let s = d.on_line(r#"{"type":"tool_execution_update","toolCallId":"b1","toolName":"bash","partialResult":{"content":[{"type":"text","text":"a\nb"}]}}"#);
        assert_eq!(
            s.events,
            vec![DriverEvent::CommandOutput {
                item_id: "t1:tool:b1".into(),
                chunk: "a\nb".into()
            }]
        );
        let s = d.on_line(r#"{"type":"tool_execution_update","toolCallId":"b1","toolName":"bash","partialResult":{"content":[{"type":"text","text":"a\nbc"}]}}"#);
        assert_eq!(
            s.events,
            vec![DriverEvent::CommandOutput {
                item_id: "t1:tool:b1".into(),
                chunk: "c".into()
            }]
        );
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
    }

    #[test]
    fn extension_dialogs_become_approvals_and_questions() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = PiDriver::with_program("pi");
        d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        let s = d.on_line(
            r#"{"type":"extension_ui_request","id":"u1","method":"confirm","title":"Sure?","message":"All gone"}"#,
        );
        let DriverEvent::Request { id, prompt, .. } = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert_eq!(prompt.as_deref(), Some("All gone"));
        let r = sent(&d.respond(id, ApprovalDecision::Accept).unwrap(), 0);
        assert_eq!(
            (r["id"].as_str(), r["confirmed"].as_bool()),
            (Some("u1"), Some(true))
        );
        let s = d.on_line(
            r#"{"type":"extension_ui_request","id":"u2","method":"select","title":"Pick","options":["Allow","Block"]}"#,
        );
        let DriverEvent::Question { id, questions, .. } = &s.events[0] else {
            panic!()
        };
        assert_eq!(questions[0].options.len(), 2);
        assert!(d
            .respond_question(
                id,
                &[QuestionAnswer::Single {
                    question_id: "answer".into(),
                    value: "Nope".into()
                }]
            )
            .is_err());
        let r = sent(
            &d.respond_question(
                id,
                &[QuestionAnswer::Single {
                    question_id: "answer".into(),
                    value: "Block".into(),
                }],
            )
            .unwrap(),
            0,
        );
        assert_eq!(
            (r["id"].as_str(), r["value"].as_str()),
            (Some("u2"), Some("Block"))
        );
        let s = d.on_line(
            r#"{"type":"extension_ui_request","id":"u3","method":"input","title":"Name?"}"#,
        );
        let DriverEvent::Question { id, .. } = &s.events[0] else {
            panic!()
        };
        let r = sent(
            &d.respond_question(
                id,
                &[QuestionAnswer::Text {
                    question_id: "answer".into(),
                    text: "otto".into(),
                }],
            )
            .unwrap(),
            0,
        );
        assert_eq!(r["value"], "otto");
        // Fire-and-forget methods ask nothing.
        assert!(d
            .on_line(
                r#"{"type":"extension_ui_request","id":"u4","method":"notify","message":"hi"}"#
            )
            .events
            .is_empty());
    }

    #[test]
    fn a_read_image_result_keeps_its_image() {
        let mut d = PiDriver::with_program("pi");
        d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        d.prompt("t1", &RuntimePromptReq::of("look")).unwrap();
        d.on_line(r#"{"type":"tool_execution_start","toolCallId":"r1","toolName":"read","args":{"path":"a.png"}}"#);
        let s = d.on_line(r#"{"type":"tool_execution_end","toolCallId":"r1","toolName":"read","result":{"content":[{"type":"text","text":"Read image file [image/png]"},{"type":"image","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=","mimeType":"image/png"}]},"isError":false}"#);
        assert!(matches!(
            &s.events[0],
            DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::DynamicTool { images, output: Some(o), .. } if images.len() == 1 && o == "Read image file [image/png]")
        ));
    }

    #[test]
    fn steer_sends_pis_steer_command_mid_turn_only() {
        let mut d = PiDriver::with_program("pi");
        d.start(&ctx(None, RuntimeAccessMode::FullAccess));
        assert!(d.steer(&RuntimePromptReq::of("more")).is_err(), "idle");
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.steer(&RuntimePromptReq::of("look at b")).unwrap();
        let c = sent(&s, 0);
        assert_eq!(
            (c["type"].as_str(), c["message"].as_str()),
            (Some("steer"), Some("look at b"))
        );
        let s = d.on_line(&format!(
            r#"{{"id":"{}","type":"response","command":"steer","success":false,"error":"busy"}}"#,
            c["id"].as_str().unwrap()
        ));
        assert!(matches!(
            &s.events[0],
            DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::SystemNotice { message } if message.ends_with("busy"))
        ));
    }
}
