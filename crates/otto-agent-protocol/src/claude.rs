//! Claude Code driver — the Claude Agent SDK's transport, ported to Rust.
//!
//! T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.) drives Claude through the
//! TypeScript Claude Agent SDK (`@anthropic-ai/claude-agent-sdk`,
//! `apps/server/package.json:37`; `query()` at
//! `apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts:1090`). The
//! SDK itself only spawns the `claude` CLI and speaks its stream-json control
//! protocol over stdio; Otto speaks that protocol directly (no Node, no npm
//! package):
//!
//! * argv — the SDK's (`sdk.mjs` `ProcessTransport.initialize`):
//!   `--output-format stream-json --verbose --input-format stream-json`,
//!   `--permission-prompt-tool stdio` (the SDK's `canUseTool` callback),
//!   `--permission-mode default` (T3 Code's approval-required mode,
//!   `ClaudeAdapterV2.ts:2573-2620`), `--include-partial-messages`
//!   (`:1617`), `--model`, and `--session-id <uuid>` on a fresh thread /
//!   `--resume <id>` after (`threadIdentity`, `:1578-1580`). Env
//!   `CLAUDE_CODE_ENTRYPOINT=sdk-ts`, as the SDK sets it.
//! * control protocol — `{type:"control_request", request_id, request:
//!   {subtype:"initialize"}}` first; `can_use_tool` requests from the CLI
//!   answered with `{type:"control_response", response:{subtype:"success",
//!   request_id, response: PermissionResult}}`; `{subtype:"interrupt"}` to
//!   stop; `control_cancel_request` withdraws a pending question. Like the
//!   SDK, the prompt is written right away; the CLI reads stdin in order.
//! * decisions — `permissionResultFromDecision` (`ClaudeAdapterV2.ts:
//!   3502-3545`): accept → `allow` + the original input; accept-for-session →
//!   `allow` + `updatedPermissions` (the CLI's suggestions re-targeted at
//!   `session`, else an `addRules` allow rule for the tool); decline → `deny`;
//!   cancel → `deny` + `interrupt: true`. Otto drops `setMode` suggestions
//!   (a whole-session mode switch is a wider grant than the one tool the
//!   person approved — T3 Code drops them too under a managed ceiling).
//! * tools — `CLAUDE_KNOWN_TOOL_CLASSIFICATIONS` (`:2819-2860`): Bash →
//!   `command_execution`; Edit / MultiEdit / Write / NotebookEdit →
//!   `file_change`; WebFetch / WebSearch → `web_search`; the rest →
//!   `dynamic_tool`; a root `TodoWrite` is the plan (`:9313`).
//! * turn end — the `result` message (`terminalStatusFromResult`,
//!   `:3708-3738`): `terminal_reason` aborted_* → interrupted; `success`
//!   without `is_error` → completed; errors mentioning interrupt / cancel →
//!   interrupted / cancelled; anything else failed.
//!
//! Subagent frames (`parent_tool_use_id` set) are not projected (T3 Code
//! builds a separate subagent tree, which Otto does not port); their
//! permission requests still reach a person.

use std::collections::HashMap;

use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, PlanStep, PlanStepStatus, ProviderFailure, RequestKind,
    RuntimeProvider, RuntimeRequestStatus, TurnItem, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, DriverError, DriverEvent, DriverStep, LaunchContext, LaunchSpec, StructuredDriver,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolType {
    Command,
    FileChange,
    WebSearch,
    Dynamic,
}

/// `classifyClaudeNativeTool` (`ClaudeAdapterV2.ts:2815-2860`).
fn classify(tool: &str) -> (ToolType, RequestKind) {
    let n: String = tool
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .collect();
    match n.as_str() {
        "bash" => (ToolType::Command, RequestKind::Command),
        "edit" | "multiedit" | "notebookedit" | "write" => {
            (ToolType::FileChange, RequestKind::FileChange)
        }
        "glob" | "grep" | "ls" | "read" => (ToolType::Dynamic, RequestKind::FileRead),
        "webfetch" | "websearch" => (ToolType::WebSearch, RequestKind::Command),
        _ => (ToolType::Dynamic, RequestKind::Command),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
}

struct Block {
    kind: BlockKind,
    item_id: String,
    message_id: String,
    text: String,
}

struct Tool {
    name: String,
    input: Value,
    output: Option<String>,
    status: TurnItemStatus,
}

struct Pending {
    cli_request_id: String,
    tool_name: String,
    input: Value,
    tool_use_id: Option<String>,
    suggestions: Vec<Value>,
}

pub struct ClaudeDriver {
    program: String,
    /// `--session-id` for a fresh thread (the resume key from the start).
    fresh_session_id: String,
    session_id: Option<String>,
    next_req: u64,
    init_req: Option<String>,
    turn_id: Option<String>,
    /// Streamed content blocks of the current message, by block index.
    blocks: HashMap<u64, Block>,
    message_id: Option<String>,
    /// Messages whose text arrived as stream events (their final `assistant`
    /// frame then carries nothing new for text / thinking).
    streamed: Vec<String>,
    tools: HashMap<String, Tool>,
    pending: HashMap<String, Pending>,
}

impl Default for ClaudeDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeDriver {
    pub fn new() -> Self {
        Self {
            program: "claude".into(),
            fresh_session_id: uuid::Uuid::new_v4().to_string(),
            session_id: None,
            next_req: 0,
            init_req: None,
            turn_id: None,
            blocks: HashMap::new(),
            message_id: None,
            streamed: Vec::new(),
            tools: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    fn control_request(&mut self, request: Value) -> (String, String) {
        self.next_req += 1;
        let id = format!("otto-{}", self.next_req);
        let line = json!({"type":"control_request","request_id":id,"request":request}).to_string();
        (id, line)
    }

    fn control_success(request_id: &str, response: Value) -> String {
        json!({"type":"control_response","response":{"subtype":"success","request_id":request_id,"response":response}})
            .to_string()
    }

    fn turn(&self) -> String {
        self.turn_id.clone().unwrap_or_default()
    }

    fn send_user(&mut self, step: &mut DriverStep, text: String) {
        step.write(
            json!({
                "type": "user",
                "session_id": "",
                "message": {"role": "user", "content": [{"type": "text", "text": text}]},
                "parent_tool_use_id": null,
            })
            .to_string(),
        );
    }

    fn block_item(&self, b: &Block, streaming: bool) -> TurnItem {
        let status = if streaming {
            TurnItemStatus::Running
        } else {
            TurnItemStatus::Completed
        };
        let kind = match b.kind {
            BlockKind::Text => TurnItemKind::AssistantMessage {
                message_id: b.message_id.clone(),
                text: b.text.clone(),
                streaming,
            },
            BlockKind::Thinking => TurnItemKind::Reasoning {
                text: b.text.clone(),
                streaming,
            },
        };
        item(b.item_id.clone(), status, None, kind)
    }

    fn close_blocks(&mut self, step: &mut DriverStep) {
        let mut open: Vec<(u64, Block)> = self.blocks.drain().collect();
        open.sort_by_key(|(i, _)| *i);
        for (_, b) in open {
            if !b.text.is_empty() {
                step.push(DriverEvent::TurnItem(self.block_item(&b, false)));
            }
        }
    }

    fn on_stream_event(&mut self, step: &mut DriverStep, ev: &Value) {
        let index = ev.get("index").and_then(Value::as_u64).unwrap_or(0);
        match ev.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message_start" => {
                self.close_blocks(step);
                self.message_id = ev
                    .pointer("/message/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "content_block_start" => {
                let kind = match ev.pointer("/content_block/type").and_then(Value::as_str) {
                    Some("text") => BlockKind::Text,
                    Some("thinking") => BlockKind::Thinking,
                    _ => return,
                };
                let mid = self.message_id.clone().unwrap_or_else(|| self.turn());
                if !self.streamed.contains(&mid) {
                    self.streamed.push(mid.clone());
                }
                self.blocks.insert(
                    index,
                    Block {
                        kind,
                        item_id: format!("{}:{mid}:{index}", self.turn()),
                        message_id: mid,
                        text: String::new(),
                    },
                );
            }
            "content_block_delta" => {
                let delta = ev.get("delta").cloned().unwrap_or(Value::Null);
                let text = match delta.get("type").and_then(Value::as_str) {
                    Some("text_delta") => delta.get("text"),
                    Some("thinking_delta") => delta.get("thinking"),
                    _ => None,
                }
                .and_then(Value::as_str)
                .unwrap_or_default();
                if text.is_empty() {
                    return;
                }
                let item = {
                    let Some(b) = self.blocks.get_mut(&index) else {
                        return;
                    };
                    b.text.push_str(text);
                    let b = &self.blocks[&index];
                    self.block_item(b, true)
                };
                step.push(DriverEvent::TurnItem(item));
            }
            "content_block_stop" => {
                if let Some(b) = self.blocks.remove(&index) {
                    if !b.text.is_empty() {
                        step.push(DriverEvent::TurnItem(self.block_item(&b, false)));
                    }
                }
            }
            _ => {}
        }
    }

    fn tool_item(&self, id: &str, t: &Tool) -> TurnItem {
        let (ty, _) = classify(&t.name);
        let s = |k: &str| t.input.get(k).and_then(Value::as_str).map(str::to_string);
        let kind = match ty {
            ToolType::Command => TurnItemKind::CommandExecution {
                input: s("command").unwrap_or_else(|| t.name.clone()),
                output: t.output.clone(),
                exit_code: None,
            },
            ToolType::FileChange => TurnItemKind::FileChange {
                file_name: s("file_path")
                    .or_else(|| s("notebook_path"))
                    .unwrap_or_else(|| t.name.clone()),
                diff_str: None,
                old_str: s("old_string"),
                new_str: s("new_string").or_else(|| s("content")),
            },
            ToolType::WebSearch => TurnItemKind::WebSearch {
                patterns: s("query").or_else(|| s("url")).into_iter().collect(),
            },
            ToolType::Dynamic => TurnItemKind::DynamicTool {
                tool_name: t.name.clone(),
                input: t.input.clone(),
                output: t.output.clone(),
            },
        };
        item(
            format!("{}:tool:{id}", self.turn()),
            t.status,
            Some(t.name.clone()),
            kind,
        )
    }

    fn on_todo_write(&self, step: &mut DriverStep, input: &Value) {
        let steps: Vec<PlanStep> = input
            .get("todos")
            .and_then(Value::as_array)
            .map(|todos| {
                todos
                    .iter()
                    .enumerate()
                    .map(|(i, t)| PlanStep {
                        id: (i + 1).to_string(),
                        text: t
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        status: match t.get("status").and_then(Value::as_str) {
                            Some("in_progress") => PlanStepStatus::Running,
                            Some("completed") => PlanStepStatus::Completed,
                            _ => PlanStepStatus::Pending,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        step.push(DriverEvent::Plan {
            steps,
            explanation: None,
        });
    }

    fn on_assistant(&mut self, step: &mut DriverStep, msg: &Value) {
        let mid = msg
            .pointer("/message/id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let streamed = self.streamed.contains(&mid);
        let content = msg
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (n, block) in content.iter().enumerate() {
            match block.get("type").and_then(Value::as_str) {
                Some(kind @ ("text" | "thinking")) if !streamed => {
                    let (bk, key) = if kind == "text" {
                        (BlockKind::Text, "text")
                    } else {
                        (BlockKind::Thinking, "thinking")
                    };
                    let text = block.get(key).and_then(Value::as_str).unwrap_or_default();
                    if text.is_empty() {
                        continue;
                    }
                    let b = Block {
                        kind: bk,
                        item_id: format!("{}:{mid}:a{n}", self.turn()),
                        message_id: mid.clone(),
                        text: text.to_string(),
                    };
                    step.push(DriverEvent::TurnItem(self.block_item(&b, false)));
                }
                Some("tool_use") => {
                    let id = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    let input = block.get("input").cloned().unwrap_or(Value::Null);
                    if name == "TodoWrite" {
                        self.on_todo_write(step, &input);
                    }
                    let t = Tool {
                        name,
                        input,
                        output: None,
                        status: TurnItemStatus::Running,
                    };
                    step.push(DriverEvent::TurnItem(self.tool_item(&id, &t)));
                    self.tools.insert(id, t);
                }
                _ => {}
            }
        }
    }

    fn on_user(&mut self, step: &mut DriverStep, msg: &Value) {
        let content = msg
            .pointer("/message/content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let id = block
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let Some(mut t) = self.tools.remove(&id) else {
                continue;
            };
            t.output = Some(result_text(block.get("content")));
            t.status = if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                TurnItemStatus::Failed
            } else {
                TurnItemStatus::Completed
            };
            step.push(DriverEvent::TurnItem(self.tool_item(&id, &t)));
        }
    }

    fn on_can_use_tool(&mut self, step: &mut DriverStep, cli_request_id: &str, req: &Value) {
        self.close_blocks(step);
        let tool_name = req
            .get("tool_name")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string();
        let input = req.get("input").cloned().unwrap_or(Value::Null);
        let (_, kind) = classify(&tool_name);
        let request_id = format!("claude-{cli_request_id}");
        let title = req
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or(&tool_name)
            .to_string();
        let summary = input
            .get("command")
            .or_else(|| input.get("file_path"))
            .or_else(|| input.get("url"))
            .or_else(|| input.get("pattern"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                req.get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let prompt = match summary {
            Some(s) => format!("{title}: {s}"),
            None => title.clone(),
        };
        let reason = req
            .get("decision_reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        let options = vec![
            ApprovalOption {
                decision: ApprovalDecision::Accept,
                label: "Allow".into(),
            },
            ApprovalOption {
                decision: ApprovalDecision::AcceptForSession,
                label: "Allow for this session".into(),
            },
            ApprovalOption {
                decision: ApprovalDecision::Decline,
                label: "Deny".into(),
            },
        ];
        self.pending.insert(
            request_id.clone(),
            Pending {
                cli_request_id: cli_request_id.to_string(),
                tool_name,
                input,
                tool_use_id: req
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                suggestions: req
                    .get("permission_suggestions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            },
        );
        step.push(DriverEvent::Request {
            id: request_id,
            kind,
            title: Some(title),
            prompt: Some(prompt),
            reason,
            options,
        });
    }

    /// `permissionResultFromDecision` (`ClaudeAdapterV2.ts:3502-3545`).
    fn permission_result(p: &Pending, decision: ApprovalDecision) -> Value {
        match decision {
            ApprovalDecision::Accept => json!({
                "behavior": "allow",
                "updatedInput": p.input,
                "toolUseID": p.tool_use_id,
            }),
            ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => {
                let mut updates: Vec<Value> = p
                    .suggestions
                    .iter()
                    .filter(|s| s.get("type").and_then(Value::as_str) != Some("setMode"))
                    .map(|s| {
                        let mut s = s.clone();
                        s["destination"] = json!("session");
                        s
                    })
                    .collect();
                if updates.is_empty() {
                    updates.push(json!({
                        "type": "addRules",
                        "rules": [{"toolName": p.tool_name}],
                        "behavior": "allow",
                        "destination": "session",
                    }));
                }
                json!({
                    "behavior": "allow",
                    "updatedInput": p.input,
                    "updatedPermissions": updates,
                    "toolUseID": p.tool_use_id,
                })
            }
            ApprovalDecision::Decline => json!({
                "behavior": "deny",
                "message": "User declined tool execution.",
                "toolUseID": p.tool_use_id,
            }),
            ApprovalDecision::Cancel => json!({
                "behavior": "deny",
                "message": "User cancelled tool execution.",
                "interrupt": true,
                "toolUseID": p.tool_use_id,
            }),
        }
    }

    fn on_result(&mut self, step: &mut DriverStep, msg: &Value) {
        self.close_blocks(step);
        self.tools.clear();
        if let Some(u) = msg.get("usage") {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("cache_read_input_tokens");
            step.push(DriverEvent::Usage(TurnTokenUsage {
                input_tokens: n("input_tokens") + cached + n("cache_creation_input_tokens"),
                output_tokens: n("output_tokens"),
                cached_input_tokens: (cached > 0).then_some(cached),
                reasoning_tokens: u
                    .pointer("/output_tokens_details/thinking_tokens")
                    .and_then(Value::as_u64)
                    .filter(|t| *t > 0),
            }));
        }
        let (status, failure, reason) = terminal_from_result(msg);
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason,
        });
        self.turn_id = None;
        self.streamed.clear();
    }

    fn on_control_response(&mut self, step: &mut DriverStep, resp: &Value) {
        let id = resp.get("request_id").and_then(Value::as_str);
        if id.is_none() || id != self.init_req.as_deref() {
            return; // interrupt acks
        }
        self.init_req = None;
        if resp.get("subtype").and_then(Value::as_str) == Some("error") {
            step.push(DriverEvent::Fatal(ProviderFailure {
                class: "handshake_error".into(),
                message: resp
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("initialize refused")
                    .to_string(),
                code: None,
                retryable: None,
            }));
            return;
        }
        step.push(DriverEvent::Ready {
            native_thread_id: self.session_id.clone(),
            model: None,
        });
    }
}

/// `terminalStatusFromResult` (`ClaudeAdapterV2.ts:3708-3738`).
fn terminal_from_result(
    msg: &Value,
) -> (TurnTerminalStatus, Option<ProviderFailure>, Option<String>) {
    let reason = msg.get("terminal_reason").and_then(Value::as_str);
    if matches!(reason, Some("aborted_tools" | "aborted_streaming")) {
        return (TurnTerminalStatus::Interrupted, None, None);
    }
    let subtype = msg
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let is_error = msg.get("is_error").and_then(Value::as_bool) == Some(true);
    let stop = msg
        .get("stop_reason")
        .and_then(Value::as_str)
        .filter(|s| *s != "end_turn")
        .map(str::to_string);
    if subtype == "success" && !is_error {
        return (TurnTerminalStatus::Completed, None, stop);
    }
    let text = if subtype == "success" {
        msg.get("result")
            .and_then(Value::as_str)
            .unwrap_or("Claude reported an error")
            .to_string()
    } else {
        msg.get("errors")
            .and_then(Value::as_array)
            .map(|e| {
                e.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| subtype.to_string())
    };
    let lower = text.to_lowercase();
    if lower.contains("interrupt") {
        return (TurnTerminalStatus::Interrupted, None, None);
    }
    if lower.contains("cancel") {
        return (TurnTerminalStatus::Cancelled, None, None);
    }
    (
        TurnTerminalStatus::Failed,
        Some(ProviderFailure {
            class: "provider_error".into(),
            message: text,
            code: Some(subtype.to_string()).filter(|s| !s.is_empty()),
            retryable: None,
        }),
        None,
    )
}

/// A `tool_result` block's content: a string or an array of text blocks.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

impl StructuredDriver for ClaudeDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Claude
    }

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec {
        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--permission-prompt-tool",
            "stdio",
            "--permission-mode",
            "default",
            "--include-partial-messages",
        ]
        .map(String::from)
        .to_vec();
        if let Some(m) = &ctx.model {
            args.extend(["--model".into(), m.clone()]);
        }
        match &ctx.resume {
            Some(id) => args.extend(["--resume".into(), id.clone()]),
            None => args.extend(["--session-id".into(), self.fresh_session_id.clone()]),
        }
        LaunchSpec {
            program: self.program.clone(),
            args,
            tail_args: Vec::new(),
            env: vec![("CLAUDE_CODE_ENTRYPOINT".into(), "sdk-ts".into())],
        }
    }

    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        self.session_id = Some(
            ctx.resume
                .clone()
                .unwrap_or_else(|| self.fresh_session_id.clone()),
        );
        let mut step = DriverStep::default();
        let (id, line) = self.control_request(json!({"subtype": "initialize", "hooks": null}));
        self.init_req = Some(id);
        step.write(line);
        step
    }

    fn prompt(&mut self, turn_id: &str, text: &str) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        self.turn_id = Some(turn_id.to_string());
        // The CLI reads stdin in order: the prompt may follow `initialize`
        // before its answer (as the SDK streams it).
        let mut step = DriverStep::default();
        self.send_user(&mut step, text.to_string());
        Ok(step)
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        // T3 Code cancels every pending approval first (`cancel` → deny +
        // interrupt), then `queryRuntime.interrupt()` (`:1298-1300`).
        let mut open: Vec<(String, Pending)> = self.pending.drain().collect();
        open.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, p) in open {
            let r = Self::permission_result(&p, ApprovalDecision::Cancel);
            step.write(Self::control_success(&p.cli_request_id, r));
        }
        let (_, line) = self.control_request(json!({"subtype": "interrupt"}));
        step.write(line);
        step
    }

    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        let p = self
            .pending
            .remove(request_id)
            .ok_or_else(|| DriverError(format!("unknown request {request_id}")))?;
        let mut step = DriverStep::default();
        step.write(Self::control_success(
            &p.cli_request_id,
            Self::permission_result(&p, decision),
        ));
        Ok(step)
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            // The CLI prints the odd non-JSON warning; not a protocol break.
            tracing::debug!(
                line = %line.chars().take(200).collect::<String>(),
                "claude: non-JSON stdout line"
            );
            return step;
        };
        let root = msg.get("parent_tool_use_id").is_none_or(Value::is_null);
        match msg.get("type").and_then(Value::as_str).unwrap_or_default() {
            "control_request" => {
                let rid = msg
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let req = msg.get("request").cloned().unwrap_or(Value::Null);
                match req.get("subtype").and_then(Value::as_str) {
                    Some("can_use_tool") => self.on_can_use_tool(&mut step, &rid, &req),
                    other => step.write(
                        json!({"type":"control_response","response":{"subtype":"error","request_id":rid,"error":format!("unsupported control request: {}", other.unwrap_or("?"))}})
                            .to_string(),
                    ),
                }
            }
            "control_cancel_request" => {
                let rid = msg
                    .get("request_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let request_id = format!("claude-{rid}");
                if self.pending.remove(&request_id).is_some() {
                    step.push(DriverEvent::RequestClosed {
                        request_id,
                        status: RuntimeRequestStatus::Cancelled,
                    });
                }
            }
            "control_response" => {
                if let Some(resp) = msg.get("response") {
                    self.on_control_response(&mut step, resp);
                }
            }
            "system" if msg.get("subtype").and_then(Value::as_str) == Some("init") => {
                if let Some(sid) = msg.get("session_id").and_then(Value::as_str) {
                    self.session_id = Some(sid.to_string());
                }
                step.push(DriverEvent::Ready {
                    native_thread_id: self.session_id.clone(),
                    model: msg.get("model").and_then(Value::as_str).map(str::to_string),
                });
            }
            "stream_event" if root => {
                if let Some(ev) = msg.get("event") {
                    self.on_stream_event(&mut step, ev);
                }
            }
            "assistant" if root => self.on_assistant(&mut step, &msg),
            "user" if root => self.on_user(&mut step, &msg),
            "result" => self.on_result(&mut step, &msg),
            _ => {}
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        self.pending.clear();
        self.tools.clear();
        self.blocks.clear();
        DriverStep::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(resume: Option<&str>) -> LaunchContext {
        LaunchContext {
            session_id: "s".into(),
            cwd: "/w".into(),
            model: Some("haiku".into()),
            resume: resume.map(str::to_string),
            sandboxed: false,
        }
    }

    fn feed(d: &mut ClaudeDriver, v: Value) -> DriverStep {
        d.on_line(&v.to_string())
    }

    fn ready(d: &mut ClaudeDriver) {
        let step = d.start(&ctx(None));
        let init: Value = serde_json::from_str(&step.writes[0]).unwrap();
        assert_eq!(init["request"]["subtype"], "initialize");
        let rid = init["request_id"].as_str().unwrap().to_string();
        let s = feed(
            d,
            json!({"type":"control_response","response":{"subtype":"success","request_id":rid,"response":{}}}),
        );
        assert!(matches!(s.events[0], DriverEvent::Ready { .. }));
    }

    #[test]
    fn launch_is_the_sdk_argv_with_a_fresh_or_resumed_session() {
        let d = ClaudeDriver::new();
        let fresh = d.launch(&ctx(None));
        assert_eq!(fresh.program, "claude");
        for flag in [
            "--input-format",
            "--permission-prompt-tool",
            "--include-partial-messages",
            "--session-id",
        ] {
            assert!(fresh.args.iter().any(|a| a == flag), "{flag}");
        }
        let pm = fresh
            .args
            .iter()
            .position(|a| a == "--permission-mode")
            .unwrap();
        assert_eq!(fresh.args[pm + 1], "default");
        assert!(!fresh.args.iter().any(|a| a.contains("dangerously")));
        let resumed = d.launch(&ctx(Some("abc")));
        let r = resumed.args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(resumed.args[r + 1], "abc");
        assert!(!resumed.args.iter().any(|a| a == "--session-id"));
    }

    #[test]
    fn prompt_streams_text_and_completes() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        let s = d.prompt("t1", "hi").unwrap();
        let user: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(user["message"]["content"][0]["text"], "hi");
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}},"parent_tool_use_id":null}),
        );
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hel"}},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(i.is_streaming());
        let s = feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "hello"));
        let s = feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(!i.is_streaming() && i.status == TurnItemStatus::Completed);
        // The full assistant frame repeats the streamed text: not re-emitted.
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"hello"}]},"parent_tool_use_id":null}),
        );
        assert!(s.events.is_empty());
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","usage":{"input_tokens":2,"cache_read_input_tokens":10,"output_tokens":3}}),
        );
        assert!(
            matches!(&s.events[0], DriverEvent::Usage(u) if u.input_tokens == 12 && u.output_tokens == 3)
        );
        assert!(matches!(
            s.events[1],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        ));
    }

    #[test]
    fn permission_round_trip_maps_decisions_like_t3code() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", "write").unwrap();
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tu1","name":"Write","input":{"file_path":"/w/a.txt","content":"x"}}]},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnItem(tool) = &s.events[0] else {
            panic!()
        };
        assert!(
            matches!(&tool.kind, TurnItemKind::FileChange { file_name, .. } if file_name == "/w/a.txt")
        );
        let ask = json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Write","input":{"file_path":"/w/a.txt","content":"x"},"permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],"tool_use_id":"tu1"}});
        let s = feed(&mut d, ask.clone());
        let DriverEvent::Request { id, kind, .. } = &s.events[0] else {
            panic!()
        };
        assert_eq!(*kind, RequestKind::FileChange);
        assert_eq!(id, "claude-r1");
        // accept-for-session never forwards the setMode suggestion.
        let s = d
            .respond("claude-r1", ApprovalDecision::AcceptForSession)
            .unwrap();
        let r: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(r["response"]["request_id"], "r1");
        let pr = &r["response"]["response"];
        assert_eq!(pr["behavior"], "allow");
        assert_eq!(pr["updatedPermissions"][0]["type"], "addRules");
        assert_eq!(pr["updatedPermissions"][0]["rules"][0]["toolName"], "Write");
        // decline / cancel
        feed(&mut d, ask.clone());
        let s = d.respond("claude-r1", ApprovalDecision::Decline).unwrap();
        let r: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(r["response"]["response"]["behavior"], "deny");
        assert!(r["response"]["response"].get("interrupt").is_none());
        feed(&mut d, ask);
        let s = d.respond("claude-r1", ApprovalDecision::Cancel).unwrap();
        let r: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(r["response"]["response"]["interrupt"], true);
        // The tool result settles the tool item.
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"denied","is_error":true}]},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert_eq!(i.status, TurnItemStatus::Failed);
    }

    #[test]
    fn interrupt_cancels_open_questions_then_sends_the_interrupt_request() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", "x").unwrap();
        feed(
            &mut d,
            json!({"type":"control_request","request_id":"r9","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}),
        );
        let s = d.interrupt();
        let deny: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(deny["response"]["response"]["interrupt"], true);
        let int: Value = serde_json::from_str(&s.writes[1]).unwrap();
        assert_eq!(int["request"]["subtype"], "interrupt");
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"error_during_execution","is_error":true,"errors":["Request was interrupted"]}),
        );
        assert!(matches!(
            s.events.last(),
            Some(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                ..
            })
        ));
    }

    #[test]
    fn cli_withdrawn_question_closes_the_request_and_todowrite_is_the_plan() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", "x").unwrap();
        feed(
            &mut d,
            json!({"type":"control_request","request_id":"r2","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"}}}),
        );
        let s = feed(
            &mut d,
            json!({"type":"control_cancel_request","request_id":"r2"}),
        );
        assert!(matches!(
            &s.events[0],
            DriverEvent::RequestClosed { request_id, status: RuntimeRequestStatus::Cancelled } if request_id == "claude-r2"
        ));
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"tu2","name":"TodoWrite","input":{"todos":[{"content":"a","status":"in_progress"},{"content":"b","status":"pending"}]}}]},"parent_tool_use_id":null}),
        );
        let DriverEvent::Plan { steps, .. } = &s.events[0] else {
            panic!()
        };
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].status, PlanStepStatus::Running);
        // Subagent frames are not projected.
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"sub"}]},"parent_tool_use_id":"tu-agent"}),
        );
        assert!(s.events.is_empty());
    }
}
