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
//!   `--permission-mode` per access mode (`permissionModeForClaudeRuntimePolicy`,
//!   `ClaudeAdapterV2.ts:2573-2620`: approval-required → `default`,
//!   auto-accept-edits → `acceptEdits`, full-access → `bypassPermissions`
//!   plus the SDK's `allowDangerouslySkipPermissions` →
//!   `--allow-dangerously-skip-permissions`, `:2681`), `--include-partial-messages`
//!   (`:1617`), `--model`, and `--session-id <uuid>` on a fresh thread /
//!   `--resume <id>` after (`threadIdentity`, `:1578-1580`). Env
//!   `CLAUDE_CODE_ENTRYPOINT=sdk-ts`, as the SDK sets it.
//! * control protocol — `{type:"control_request", request_id, request:
//!   {subtype:"initialize"}}` first; `can_use_tool` requests from the CLI
//!   answered with `{type:"control_response", response:{subtype:"success",
//!   request_id, response: PermissionResult}}`; `{subtype:"interrupt"}` to
//!   stop; `control_cancel_request` withdraws a pending question;
//!   `{subtype:"set_permission_mode", mode}` switches the access mode live
//!   (the SDK's `setPermissionMode`) — into `bypassPermissions` only when
//!   the process was launched with the allow flag, else the runtime
//!   respawns with `--resume`. Like the SDK, the prompt is written right
//!   away; the CLI reads stdin in order. Images go as `image` blocks, PDFs as
//!   base64 `document` blocks.
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
//! Subagents and background work (`claude_tasks.rs`): an `Agent` / `Task`
//! call is a `subagent` card whose frames (`parent_tool_use_id` set) are
//! projected under it; `run_in_background` work is tracked from the CLI's
//! `task_*` frames into the session's background-task list. Subagent
//! permission requests reach a person like the root's.
//!
//! Auto turns: after `result` the CLI may run a turn on its own (a
//! background task finished and its notification was fed to the model).
//! Root frames arriving with no prompt of ours running open one
//! ([`DriverEvent::TurnStarted`]); its `result` ends it.

use std::collections::HashMap;

#[path = "claude_tasks.rs"]
mod tasks;

use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, BackgroundTask, PlanStep, PlanStepStatus, ProviderCommand,
    ProviderFailure, RequestKind, RuntimeAccessMode, RuntimePromptReq, RuntimeProvider,
    RuntimeRequestStatus, ThreadTokenUsage, TurnItem, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, tool_error, DriverError, DriverEvent, DriverStep, LaunchContext, LaunchSpec,
    StructuredDriver,
};

/// Claude Code's switch for connecting MCP servers before the first request.
const MCP_NONBLOCKING: &str = "MCP_CONNECTION_NONBLOCKING";

/// Claude Code's project folder for `cwd` (otto-transcript `encode_cwd`:
/// every non-ASCII-alphanumeric character becomes `-`).
fn claude_project_dir(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `permissionModeForClaudeRuntimePolicy` (`ClaudeAdapterV2.ts:2573-2620`).
fn permission_mode(mode: RuntimeAccessMode) -> &'static str {
    match mode {
        RuntimeAccessMode::ApprovalRequired => "default",
        RuntimeAccessMode::AutoAcceptEdits => "acceptEdits",
        RuntimeAccessMode::FullAccess => "bypassPermissions",
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum BlockKind {
    Text,
    Thinking,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Block {
    kind: BlockKind,
    item_id: String,
    message_id: String,
    text: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Tool {
    name: String,
    input: Value,
    output: Option<String>,
    images: Vec<otto_core::agent_runtime::ToolOutputImage>,
    error: Option<String>,
    status: TurnItemStatus,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Pending {
    cli_request_id: String,
    tool_name: String,
    input: Value,
    tool_use_id: Option<String>,
    suggestions: Vec<Value>,
}

#[derive(serde::Serialize, serde::Deserialize)]
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
    /// The root reply shown last, while nothing has followed it (a tool
    /// call, a new message): an `aborted_streaming` result cut it off.
    last_reply: Option<Block>,
    /// The last root message's `usage` and the model that wrote it: the
    /// context it filled (a turn's `result.usage` sums every request).
    last_usage: Option<(Option<String>, Value)>,
    tools: HashMap<String, Tool>,
    pending: HashMap<String, Pending>,
    /// The model / effort this process runs (switched live, as the SDK's
    /// `setModel` / `applyFlagSettings`).
    model: Option<String>,
    effort: Option<String>,
    access: RuntimeAccessMode,
    /// Launched with `--allow-dangerously-skip-permissions` (full access).
    bypass_allowed: bool,
    /// Ids of in-flight `set_model` / `apply_flag_settings` /
    /// `set_permission_mode` requests.
    switches: Vec<String>,
    /// CLI tasks by task id (see `claude_tasks.rs`).
    tasks: HashMap<String, tasks::Task>,
    /// Subagent cards by their `Agent` / `Task` tool_use id.
    agents: HashMap<String, tasks::Agent>,
    /// Subagents' own tool calls by tool_use id.
    sub_tools: HashMap<String, tasks::SubTool>,
    /// Root tool calls that asked for `run_in_background`: (tool, label).
    bg_inputs: tasks::BgInputs,
    /// Task ids already finished (a late duplicate frame is ignored).
    finished: Vec<String>,
    /// The background list last reported.
    reported_background: Vec<BackgroundTask>,
    /// This process's running `result.total_cost_usd` (it sums every turn
    /// the process ran): a turn's cost is the difference.
    #[serde(default)]
    cost_total: f64,
    /// The last prompt asked for plan mode (`set_permission_mode: plan`).
    #[serde(default)]
    plan: bool,
    /// Where Claude Code keeps this session's transcript (its `ai-title`).
    #[serde(default)]
    transcript: Option<std::path::PathBuf>,
    #[serde(default)]
    cwd: String,
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
            last_reply: None,
            last_usage: None,
            tools: HashMap::new(),
            pending: HashMap::new(),
            model: None,
            effort: None,
            access: RuntimeAccessMode::default(),
            bypass_allowed: false,
            switches: Vec::new(),
            tasks: HashMap::new(),
            agents: HashMap::new(),
            sub_tools: HashMap::new(),
            bg_inputs: HashMap::new(),
            finished: Vec::new(),
            reported_background: Vec::new(),
            cost_total: 0.0,
            plan: false,
            transcript: None,
            cwd: String::new(),
        }
    }

    /// A driver for `program` instead of `claude` (tests: `otto-fake-claude`).
    pub fn with_program(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            ..Self::new()
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

    fn send_user(&mut self, step: &mut DriverStep, req: &RuntimePromptReq) {
        let mut content: Vec<Value> = req
            .images
            .iter()
            .map(|i| json!({"type": "image", "source": {"type": "base64", "media_type": i.mime, "data": i.data_base64}}))
            .collect();
        content.extend(req.files.iter().map(|f| {
            json!({"type": "document", "title": f.name, "source": {"type": "base64", "media_type": f.mime, "data": f.data_base64}})
        }));
        if !req.text.is_empty() {
            content.push(json!({"type": "text", "text": req.text}));
        }
        step.write(
            json!({
                "type": "user",
                "session_id": "",
                "message": {"role": "user", "content": content},
                "parent_tool_use_id": null,
            })
            .to_string(),
        );
    }

    /// `set_model` / `apply_flag_settings` for a model / effort `req` picks
    /// that this process does not run yet (the SDK's `setModel` /
    /// `applyFlagSettings`).
    fn switch_selection(&mut self, step: &mut DriverStep, req: &RuntimePromptReq) {
        if req.model.is_some() && req.model != self.model {
            self.model = req.model.clone();
            let (id, line) =
                self.control_request(json!({"subtype": "set_model", "model": req.model}));
            self.switches.push(id);
            step.write(line);
        }
        if req.effort.is_some() && req.effort != self.effort {
            self.effort = req.effort.clone();
            let settings = json!({"effortLevel": req.effort});
            let (id, line) = self
                .control_request(json!({"subtype": "apply_flag_settings", "settings": settings}));
            self.switches.push(id);
            step.write(line);
        }
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
            self.emit_closed(step, b);
        }
    }

    /// A finished block; a reply's text is remembered as the last reply.
    fn emit_closed(&mut self, step: &mut DriverStep, b: Block) {
        if b.text.is_empty() {
            return;
        }
        step.push(DriverEvent::TurnItem(self.block_item(&b, false)));
        if b.kind == BlockKind::Text {
            self.last_reply = Some(b);
        }
    }

    /// A stopped turn's cut-off output reads as interrupted, not finished
    /// (T3 Code #34597): the blocks still streaming and, when the stream
    /// itself was aborted, the reply shown last.
    fn cut_off_blocks(&mut self, step: &mut DriverStep, aborted_streaming: bool) {
        let mut open: Vec<(u64, Block)> = self.blocks.drain().collect();
        open.sort_by_key(|(i, _)| *i);
        let last = self.last_reply.take();
        let mut cut_reply = false;
        for (_, b) in open.into_iter().filter(|(_, b)| !b.text.is_empty()) {
            cut_reply |= b.kind == BlockKind::Text;
            step.push(DriverEvent::TurnItem(self.interrupted_item(&b)));
        }
        if let Some(b) = last.filter(|_| aborted_streaming && !cut_reply) {
            step.push(DriverEvent::TurnItem(self.interrupted_item(&b)));
        }
    }

    fn interrupted_item(&self, b: &Block) -> TurnItem {
        let mut it = self.block_item(b, false);
        it.status = TurnItemStatus::Interrupted;
        it
    }

    fn on_stream_event(&mut self, step: &mut DriverStep, ev: &Value) {
        let index = ev.get("index").and_then(Value::as_u64).unwrap_or(0);
        match ev.get("type").and_then(Value::as_str).unwrap_or_default() {
            "message_start" => {
                self.close_blocks(step);
                self.last_reply = None; // a new message followed it
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
                    self.emit_closed(step, b);
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
                image_files: Vec::new(),
                tool_name: t.name.clone(),
                input: t.input.clone(),
                output: t.output.clone(),
                images: t.images.clone(),
            },
        };
        let mut it = item(self.tool_item_id(id), t.status, Some(t.name.clone()), kind);
        it.error = t.error.clone();
        it
    }

    fn tool_item_id(&self, tool_use_id: &str) -> String {
        format!("{}:tool:{tool_use_id}", self.turn())
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
        if let Some(u) = msg.pointer("/message/usage").filter(|u| u.is_object()) {
            let model = msg.pointer("/message/model").and_then(Value::as_str);
            self.last_usage = Some((model.map(str::to_string), u.clone()));
        }
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
                    self.emit_closed(step, b);
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
                    self.last_reply = None; // the reply was whole: a tool call followed it
                    if tasks::is_agent_tool(&name) {
                        self.start_agent(step, &id, &name, &input);
                        continue;
                    }
                    self.note_background_input(&id, &name, &input);
                    if name == "TodoWrite" {
                        self.on_todo_write(step, &input);
                    }
                    let t = Tool {
                        name,
                        input,
                        output: None,
                        images: Vec::new(),
                        error: None,
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
            let result = crate::tool_images::extract(block.get("content"));
            let text = result.text;
            let is_error = block.get("is_error").and_then(Value::as_bool) == Some(true);
            let stopped = non_execution_status(msg, &id);
            if self.on_root_tool_result(step, &id, &text, is_error, stopped) {
                continue;
            }
            let Some(mut t) = self.tools.remove(&id) else {
                continue;
            };
            match stopped {
                Some(status) => t.status = status,
                None if is_error => {
                    t.status = TurnItemStatus::Failed;
                    t.error = tool_error(&text);
                }
                None => t.status = TurnItemStatus::Completed,
            }
            t.output = Some(text);
            t.images = result.images;
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
        let tool_use_id = req
            .get("tool_use_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        let tool_item_id = tool_use_id.as_deref().map(|id| {
            self.sub_tool_item_id(id)
                .unwrap_or_else(|| self.tool_item_id(id))
        });
        let questions = if tool_name == "AskUserQuestion" {
            match crate::questions::native_questions(&input, true) {
                Ok(q) => Some(q),
                Err(message) => {
                    step.write(Self::control_success(
                        cli_request_id,
                        json!({"behavior":"deny","message":message}),
                    ));
                    return;
                }
            }
        } else {
            None
        };
        self.pending.insert(
            request_id.clone(),
            Pending {
                cli_request_id: cli_request_id.to_string(),
                tool_name,
                input,
                tool_use_id,
                suggestions: req
                    .get("permission_suggestions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            },
        );
        if let Some(questions) = questions {
            step.push(DriverEvent::Question {
                id: request_id,
                title: None,
                questions,
                tool_item_id,
            });
            return;
        }
        step.push(DriverEvent::Request {
            id: request_id,
            kind,
            title: Some(title),
            prompt: Some(prompt),
            reason,
            options,
            tool_item_id,
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
        let (status, failure, reason) = terminal_from_result(msg);
        if matches!(
            status,
            TurnTerminalStatus::Interrupted | TurnTerminalStatus::Cancelled
        ) {
            let aborted_streaming =
                msg.get("terminal_reason").and_then(Value::as_str) == Some("aborted_streaming");
            self.cut_off_blocks(step, aborted_streaming);
        } else {
            self.close_blocks(step);
        }
        self.last_reply = None;
        self.tools.clear();
        self.end_turn_tasks(step);
        if let Some(usage) = self.context_usage(msg) {
            step.push(DriverEvent::ContextUsage(usage));
        }
        let cost = msg
            .get("total_cost_usd")
            .and_then(Value::as_f64)
            .filter(|c| c.is_finite() && *c >= self.cost_total)
            .map(|total| total - std::mem::replace(&mut self.cost_total, total));
        if let Some(u) = msg.get("usage") {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let cached = n("cache_read_input_tokens");
            step.push(DriverEvent::Usage(TurnTokenUsage {
                cost_usd: cost,
                input_tokens: n("input_tokens") + cached + n("cache_creation_input_tokens"),
                output_tokens: n("output_tokens"),
                cached_input_tokens: (cached > 0).then_some(cached),
                reasoning_tokens: u
                    .pointer("/output_tokens_details/thinking_tokens")
                    .and_then(Value::as_u64)
                    .filter(|t| *t > 0),
            }));
        } else if let Some(c) = cost {
            step.push(DriverEvent::TurnCost(c));
        }
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason,
        });
        self.turn_id = None;
        self.streamed.clear();
    }

    /// The context the last message filled, against the window this Claude
    /// process enforces: `result.modelUsage[model].contextWindow` (settings
    /// like `CLAUDE_CODE_DISABLE_1M_CONTEXT` shrink it below the model's
    /// catalogue window — T3 Code #31370). Another model's entry (a side
    /// task on a smaller model) never stands in for it.
    fn context_usage(&mut self, msg: &Value) -> Option<ThreadTokenUsage> {
        let (model, u) = self.last_usage.take()?;
        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
        let cached = n("cache_read_input_tokens");
        let input = n("input_tokens") + cached + n("cache_creation_input_tokens");
        let output = n("output_tokens");
        let max_tokens = model.and_then(|m| {
            msg.get("modelUsage")?
                .get(m.as_str())?
                .get("contextWindow")?
                .as_u64()
                .filter(|w| *w > 0)
        });
        Some(ThreadTokenUsage {
            used_tokens: input + output,
            max_tokens,
            input_tokens: Some(input),
            cached_input_tokens: Some(cached),
            output_tokens: Some(output),
        })
    }

    /// Root output with no prompt of ours running: the CLI started a turn
    /// on its own (after a background task's notification).
    fn open_auto_turn(&mut self, step: &mut DriverStep) {
        if self.turn_id.is_some() || self.init_req.is_some() {
            return;
        }
        let id = otto_core::new_id();
        self.turn_id = Some(id.clone());
        self.streamed.clear();
        self.last_reply = None;
        step.push(DriverEvent::TurnStarted { turn_id: id });
    }

    fn on_control_response(&mut self, step: &mut DriverStep, resp: &Value) {
        let id = resp.get("request_id").and_then(Value::as_str);
        let error = resp.get("subtype").and_then(Value::as_str) == Some("error");
        if let Some(n) = self.switches.iter().position(|s| Some(s.as_str()) == id) {
            self.switches.remove(n);
            if error {
                step.push(DriverEvent::Error(ProviderFailure {
                    class: "provider_error".into(),
                    message: resp
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("the switch was refused")
                        .to_string(),
                    code: None,
                    retryable: None,
                }));
            }
            return;
        }
        if id.is_none() || id != self.init_req.as_deref() {
            return; // interrupt acks
        }
        self.init_req = None;
        if error {
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
        // Its skills, custom and built-in commands, with descriptions.
        if let Some(list) = resp.pointer("/response/commands").and_then(Value::as_array) {
            step.push(DriverEvent::Commands(
                list.iter()
                    .filter_map(|c| {
                        let mut cmd = ProviderCommand::new(
                            c.get("name")?.as_str()?,
                            c.get("description").and_then(Value::as_str).unwrap_or(""),
                        );
                        cmd.hint = c
                            .get("argumentHint")
                            .and_then(Value::as_str)
                            .filter(|h| !h.trim().is_empty())
                            .map(str::to_string);
                        Some(cmd)
                    })
                    .collect(),
            ));
        }
    }
}

/// Claude Code's own word on a tool call that never ran to its end
/// (`tool_result_meta[].non_execution_kind`): one a new message or Stop cut
/// short is stopped, not failed (T3 Code #34183); a withdrawn one cancelled.
fn non_execution_status(msg: &Value, tool_use_id: &str) -> Option<TurnItemStatus> {
    let kind = msg
        .get("tool_result_meta")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(Value::as_str) == Some(tool_use_id))?
        .get("non_execution_kind")?
        .as_str()?;
    match kind {
        "interrupted" => Some(TurnItemStatus::Interrupted),
        "cancelled" => Some(TurnItemStatus::Cancelled),
        _ => None,
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

impl StructuredDriver for ClaudeDriver {
    fn export_state(&self) -> Option<Value> {
        serde_json::to_value(self).ok()
    }

    fn import_state(&mut self, state: Value) -> Result<(), DriverError> {
        *self = serde_json::from_value(state).map_err(|e| DriverError(e.to_string()))?;
        Ok(())
    }

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
            permission_mode(ctx.access_mode),
            "--include-partial-messages",
        ]
        .map(String::from)
        .to_vec();
        if ctx.access_mode == RuntimeAccessMode::FullAccess {
            args.push("--allow-dangerously-skip-permissions".into());
        }
        if let Some(m) = &ctx.model {
            args.extend(["--model".into(), m.clone()]);
        }
        let mut env: Vec<(String, String)> =
            vec![("CLAUDE_CODE_ENTRYPOINT".into(), "sdk-ts".into())];
        match &ctx.resume {
            Some(id) => {
                args.extend(["--resume".into(), id.clone()]);
                // Claude Code connects MCP servers in the background; on a
                // resume the first request then finds the transcript's MCP
                // tools missing and tells the agent they all disconnected.
                // Wait for them (capped by its MCP_CONNECT_TIMEOUT_MS) unless
                // the daemon's environment already decides (T3 Code #34822).
                if std::env::var_os(MCP_NONBLOCKING).is_none() {
                    env.push((MCP_NONBLOCKING.into(), "0".into()));
                }
            }
            None => args.extend(["--session-id".into(), self.fresh_session_id.clone()]),
        }
        LaunchSpec {
            program: self.program.clone(),
            args,
            tail_args: Vec::new(),
            env,
        }
    }

    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        self.model = ctx.model.clone();
        self.cwd = ctx.cwd.clone();
        self.access = ctx.access_mode;
        self.bypass_allowed = ctx.access_mode == RuntimeAccessMode::FullAccess;
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

    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        self.turn_id = Some(turn_id.to_string());
        // The CLI reads stdin in order: switches and the prompt may follow
        // `initialize` before its answer (as the SDK streams them).
        let mut step = DriverStep::default();
        self.switch_selection(&mut step, req);
        let access = req.access_mode.unwrap_or(self.access);
        // Plan mode is a permission mode; leaving it restores the access
        // mode. Re-sent on every plan turn: approving `ExitPlanMode` leaves
        // plan mode inside the CLI.
        if access != self.access || req.plan || self.plan {
            self.access = access;
            self.plan = req.plan;
            let mode = if req.plan {
                "plan"
            } else {
                permission_mode(access)
            };
            let (id, line) =
                self.control_request(json!({"subtype": "set_permission_mode", "mode": mode}));
            self.switches.push(id);
            step.write(line);
        }
        self.send_user(&mut step, req);
        Ok(step)
    }

    /// The CLI reads stdin in order and hands a user message that arrives
    /// mid-turn to the model at its next step (the SDK's streaming input).
    /// A model / effort picked with it switches first, so the rest of the
    /// running turn uses it (T3 Code #34564).
    fn steer(&mut self, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_none() {
            return Err(DriverError("no turn is running".into()));
        }
        let mut step = DriverStep::default();
        self.switch_selection(&mut step, req);
        self.send_user(&mut step, req);
        Ok(step)
    }

    fn switches_model_live(&self) -> bool {
        true
    }

    fn switches_access_live(&self, to: RuntimeAccessMode) -> bool {
        to != RuntimeAccessMode::FullAccess || self.bypass_allowed
    }

    /// The config dir the CLI runs with (a subscription account's own, else
    /// `~/.claude`) locates its transcript, where it writes its `ai-title`.
    fn adopt_spec(&mut self, spec: &mut LaunchSpec) {
        let dir = spec
            .env
            .iter()
            .rev()
            .find(|(k, _)| k == "CLAUDE_CONFIG_DIR")
            .map(|(_, v)| std::path::PathBuf::from(v))
            .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(Into::into))
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".claude")));
        self.transcript = dir.map(|d| d.join("projects"));
    }

    fn title_transcript(&self) -> Option<std::path::PathBuf> {
        let sid = self.session_id.as_deref()?;
        Some(
            self.transcript
                .as_ref()?
                .join(claude_project_dir(&self.cwd))
                .join(format!("{sid}.jsonl")),
        )
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
        if self
            .pending
            .get(request_id)
            .is_some_and(|p| p.tool_name == "AskUserQuestion")
        {
            return Err(DriverError("this request needs question answers".into()));
        }
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

    fn respond_question(
        &mut self,
        request_id: &str,
        answers: &[otto_core::agent_runtime::QuestionAnswer],
    ) -> Result<DriverStep, DriverError> {
        let p = self
            .pending
            .get(request_id)
            .filter(|p| p.tool_name == "AskUserQuestion")
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let questions = crate::questions::native_questions(&p.input, true).map_err(DriverError)?;
        otto_core::agent_runtime::validate_question_answers(&questions, answers)
            .map_err(DriverError)?;
        // Claude's native answers map uses question text and comma-separated multi selections.
        let values: serde_json::Map<String, Value> = answers
            .iter()
            .map(|a| (a.question_id().to_string(), json!(a.values().join(", "))))
            .collect();
        let mut input = p.input.clone();
        input["answers"] = json!(values);
        let mut step = DriverStep::default();
        step.write(Self::control_success(
            &p.cli_request_id,
            json!({"behavior":"allow","updatedInput":input,"toolUseID":p.tool_use_id}),
        ));
        self.pending.remove(request_id);
        Ok(step)
    }

    fn dismiss_question(
        &mut self,
        request_id: &str,
        _action: otto_core::agent_runtime::QuestionDismissAction,
    ) -> Result<DriverStep, DriverError> {
        let p = self
            .pending
            .get(request_id)
            .filter(|p| p.tool_name == "AskUserQuestion")
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let mut step = DriverStep::default();
        step.write(Self::control_success(
            &p.cli_request_id,
            json!({"behavior":"deny","message":"The user skipped this question"}),
        ));
        self.pending.remove(request_id);
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
            "system" => {
                self.on_task_frame(&mut step, &msg);
            }
            "stream_event" if root => {
                self.open_auto_turn(&mut step);
                if let Some(ev) = msg.get("event") {
                    self.on_stream_event(&mut step, ev);
                }
            }
            "assistant" if root => {
                self.open_auto_turn(&mut step);
                self.on_assistant(&mut step, &msg);
            }
            "user" if root => {
                self.open_auto_turn(&mut step);
                self.on_user(&mut step, &msg);
            }
            "assistant" | "user" => {
                if let Some(parent) = msg.get("parent_tool_use_id").and_then(Value::as_str) {
                    self.on_subagent_frame(&mut step, parent, &msg);
                }
            }
            // A `result` with no turn open (nothing of it was shown) ends nothing.
            "result" if self.turn_id.is_some() => self.on_result(&mut step, &msg),
            "result" => {
                self.end_turn_tasks(&mut step);
            }
            _ => {}
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        self.pending.clear();
        self.tools.clear();
        self.blocks.clear();
        self.last_reply = None;
        self.last_usage = None;
        self.clear_tasks();
        self.turn_id = None;
        DriverStep::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(resume: Option<&str>) -> LaunchContext {
        LaunchContext {
            mcp_servers: Vec::new(),
            session_id: "s".into(),
            cwd: "/w".into(),
            model: Some("haiku".into()),
            resume: resume.map(str::to_string),
            sandboxed: false,
            access_mode: RuntimeAccessMode::default(),
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
    fn model_and_effort_switch_live_before_an_image_prompt() {
        let mut d = ClaudeDriver::new();
        d.start(&ctx(None)); // launched on `haiku`
        let req = RuntimePromptReq {
            model: Some("sonnet".into()),
            effort: Some("high".into()),
            images: vec![otto_core::agent_runtime::PromptImage {
                mime: "image/png".into(),
                data_base64: "AAAA".into(),
            }],
            ..RuntimePromptReq::of("what is this?")
        };
        let s = d.prompt("t1", &req).unwrap();
        let w: Vec<Value> = s
            .writes
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            w[0]["request"],
            json!({"subtype":"set_model","model":"sonnet"})
        );
        assert_eq!(w[1]["request"]["settings"], json!({"effortLevel":"high"}));
        let content = &w[2]["message"]["content"];
        assert_eq!(
            content[0]["source"],
            json!({"type":"base64","media_type":"image/png","data":"AAAA"})
        );
        assert_eq!(content[1]["text"], "what is this?");
        // A refused switch is reported; the session goes on.
        let rid = w[0]["request_id"].as_str().unwrap();
        let e = feed(
            &mut d,
            json!({"type":"control_response","response":{"subtype":"error","request_id":rid,"error":"no such model"}}),
        );
        assert!(matches!(&e.events[0], DriverEvent::Error(f) if f.message == "no such model"));
        // The same selection next turn sends nothing extra.
        feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        let s = d.prompt("t2", &req).unwrap();
        assert_eq!(s.writes.len(), 1);
        assert!(d.switches_model_live());
    }

    #[test]
    fn prompt_streams_text_and_completes() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
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
        d.prompt("t1", &RuntimePromptReq::of("write")).unwrap();
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
        d.prompt("t1", &RuntimePromptReq::of("x")).unwrap();
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
        d.prompt("t1", &RuntimePromptReq::of("x")).unwrap();
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
        // A subagent this driver never saw launched is not projected.
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"sub"}]},"parent_tool_use_id":"tu-agent"}),
        );
        assert!(s.events.is_empty());
    }

    #[test]
    fn access_modes_map_to_permission_modes_and_switch_live() {
        let at = |access_mode| LaunchContext {
            mcp_servers: Vec::new(),
            access_mode,
            ..ctx(None)
        };
        let mode_of = |spec: &LaunchSpec| {
            let pm = spec
                .args
                .iter()
                .position(|a| a == "--permission-mode")
                .unwrap();
            spec.args[pm + 1].clone()
        };
        let d = ClaudeDriver::new();
        let edits = d.launch(&at(RuntimeAccessMode::AutoAcceptEdits));
        assert_eq!(mode_of(&edits), "acceptEdits");
        assert!(!edits.args.iter().any(|a| a.contains("dangerously")));
        let full = d.launch(&at(RuntimeAccessMode::FullAccess));
        assert_eq!(mode_of(&full), "bypassPermissions");
        assert!(full
            .args
            .contains(&"--allow-dangerously-skip-permissions".to_string()));
        assert!(!full
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));

        // Launched in approval-required: edits switch live, bypass respawns.
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        assert!(d.switches_access_live(RuntimeAccessMode::AutoAcceptEdits));
        assert!(!d.switches_access_live(RuntimeAccessMode::FullAccess));
        let req = RuntimePromptReq {
            access_mode: Some(RuntimeAccessMode::AutoAcceptEdits),
            ..RuntimePromptReq::of("go")
        };
        let s = d.prompt("t1", &req).unwrap();
        let w: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(
            w["request"],
            json!({"subtype":"set_permission_mode","mode":"acceptEdits"})
        );
        feed(&mut d, json!({"type":"result","subtype":"success"}));
        assert_eq!(d.prompt("t2", &req).unwrap().writes.len(), 1, "unchanged");
        // Launched in full access: every mode switches live.
        let mut d = ClaudeDriver::new();
        d.start(&at(RuntimeAccessMode::FullAccess));
        assert!(d.switches_access_live(RuntimeAccessMode::FullAccess));
    }

    #[test]
    fn parallel_questions_are_all_live_and_answer_out_of_order() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        let pdf = RuntimePromptReq {
            files: vec![otto_core::agent_runtime::PromptFile {
                path: None,
                mime: "application/pdf".into(),
                name: "spec.pdf".into(),
                data_base64: "JVBERg==".into(),
            }],
            ..RuntimePromptReq::of("fetch both")
        };
        let s = d.prompt("t1", &pdf).unwrap();
        let user: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(
            user["message"]["content"][0],
            json!({"type":"document","title":"spec.pdf","source":{"type":"base64","media_type":"application/pdf","data":"JVBERg=="}})
        );
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[
                {"type":"tool_use","id":"tu1","name":"WebFetch","input":{"url":"https://a"}},
                {"type":"tool_use","id":"tu2","name":"WebFetch","input":{"url":"https://b"}}]},"parent_tool_use_id":null}),
        );
        for (r, tu) in [("r1", "tu1"), ("r2", "tu2")] {
            let s = feed(
                &mut d,
                json!({"type":"control_request","request_id":r,"request":{"subtype":"can_use_tool","tool_name":"WebFetch","input":{"url":"x"},"tool_use_id":tu}}),
            );
            let want = format!("t1:tool:{tu}");
            assert!(
                matches!(&s.events[0], DriverEvent::Request { tool_item_id: Some(t), .. } if *t == want)
            );
        }
        let second = d.respond("claude-r2", ApprovalDecision::Decline).unwrap();
        let first = d.respond("claude-r1", ApprovalDecision::Accept).unwrap();
        let rid =
            |s: &DriverStep| -> Value { serde_json::from_str::<Value>(&s.writes[0]).unwrap() };
        assert_eq!(rid(&second)["response"]["request_id"], "r2");
        assert_eq!(rid(&first)["response"]["request_id"], "r1");
        // The declined fetch fails with the CLI's text as its error.
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu2","content":[{"type":"text","text":"  User declined tool execution.  "}],"is_error":true}]},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert_eq!(i.status, TurnItemStatus::Failed);
        assert_eq!(i.error.as_deref(), Some("User declined tool execution."));
        assert!(matches!(&i.kind, TurnItemKind::WebSearch { .. }));
    }

    fn bg_lists(s: &DriverStep) -> Vec<Vec<BackgroundTask>> {
        s.events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::BackgroundTasks(l) => Some(l.clone()),
                _ => None,
            })
            .collect()
    }

    fn items(s: &DriverStep) -> Vec<TurnItem> {
        s.events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::TurnItem(i) => Some(i.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn background_bash_stays_pending_past_its_turn_until_its_notification() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("run it")).unwrap();
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"sleep 300","description":"Wait five minutes","run_in_background":true}}]},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_started","task_id":"b1","tool_use_id":"tu1","description":"Wait five minutes","task_type":"local_bash"}),
        );
        let l = bg_lists(&s);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0][0].id, "b1");
        assert_eq!(l[0][0].kind, "command");
        assert_eq!(l[0][0].item_id.as_deref(), Some("t1:tool:tu1"));
        // The launch result settles the tool call; the task runs on.
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"Command running in background with ID: b1. Output is being written to: /tmp/b1"}]},"parent_tool_use_id":null}),
        );
        assert_eq!(items(&s)[0].status, TurnItemStatus::Completed);
        assert!(bg_lists(&s).is_empty(), "no change");
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        assert!(matches!(
            s.events.last(),
            Some(DriverEvent::Terminal { .. })
        ));
        assert_eq!(d.background_list().len(), 1);
        // Done: a notice, the list empties.
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_notification","task_id":"b1","tool_use_id":"tu1","status":"completed","summary":"exit 0","output_file":"/tmp/b1"}),
        );
        let notice = &items(&s)[0];
        assert!(
            matches!(&notice.kind, TurnItemKind::SystemNotice { message } if message == "Background command finished: Wait five minutes")
        );
        assert_eq!(notice.turn_id, None, "no turn running");
        assert_eq!(bg_lists(&s), vec![Vec::<BackgroundTask>::new()]);
        // A duplicate notification changes nothing.
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_notification","task_id":"b1","status":"completed"}),
        );
        assert!(bg_lists(&s).is_empty() && items(&s).is_empty());
    }

    #[test]
    fn root_output_after_result_opens_an_auto_turn_its_result_ends() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        // Trailing frames that are not root output open nothing.
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_summary","detail":null}),
        );
        assert!(s.events.is_empty());
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        assert!(
            !s.events
                .iter()
                .any(|e| matches!(e, DriverEvent::Terminal { .. })),
            "a result with no turn open ends nothing"
        );
        let s = feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m9"}},"parent_tool_use_id":null}),
        );
        let DriverEvent::TurnStarted { turn_id } = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        let auto = turn_id.clone();
        // The person's prompt waits: one turn at a time.
        assert!(d.prompt("t2", &RuntimePromptReq::of("hi")).is_err());
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m9","content":[{"type":"tool_use","id":"tu9","name":"Read","input":{"file_path":"/w/a"}}]},"parent_tool_use_id":null}),
        );
        assert_eq!(items(&s)[0].id, format!("{auto}:tool:tu9"));
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        assert!(matches!(
            s.events.last(),
            Some(DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            })
        ));
        assert!(d.prompt("t2", &RuntimePromptReq::of("hi")).is_ok());
    }

    #[test]
    fn a_launch_result_alone_tracks_background_work_on_older_clis() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"npm test","run_in_background":true}}]},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu1","content":"Command running in background with ID: bash_7. Output is being written to: /tmp/x."}]},"parent_tool_use_id":null}),
        );
        let l = bg_lists(&s);
        assert_eq!(l[0][0].id, "bash_7");
        assert_eq!(l[0][0].label, "npm test");
        // A later task_started for the same call keeps one entry.
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_started","task_id":"bash_7","tool_use_id":"tu1","description":"Run tests","task_type":"local_bash"}),
        );
        assert_eq!(bg_lists(&s)[0].len(), 1);
        // A foreground task is not background work — until it is moved there.
        feed(
            &mut d,
            json!({"type":"system","subtype":"task_started","task_id":"f1","tool_use_id":"tu2","description":"build","task_type":"local_bash"}),
        );
        assert_eq!(d.background_list().len(), 1);
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_updated","task_id":"f1","patch":{"is_backgrounded":true}}),
        );
        assert_eq!(bg_lists(&s)[0].len(), 2);
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_updated","task_id":"f1","patch":{"status":"killed"}}),
        );
        assert_eq!(bg_lists(&s)[0].len(), 1);
        assert!(items(&s).iter().any(|i| matches!(&i.kind,
            TurnItemKind::SystemNotice { message } if message.starts_with("Background command stopped"))));
    }

    #[test]
    fn a_background_subagent_is_a_card_with_its_activity_nested_until_its_task_ends() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("sweep")).unwrap();
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tuA","name":"Agent","input":{"description":"PR sweep","prompt":"Sweep PRs","subagent_type":"general-purpose","run_in_background":true}}]},"parent_tool_use_id":null}),
        );
        let card = &items(&s)[0];
        assert_eq!(card.id, "t1:tool:tuA");
        assert!(card.is_background_open());
        assert!(
            matches!(&card.kind, TurnItemKind::Subagent { description: Some(dsc), background: true, .. } if dsc == "PR sweep")
        );
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"tuA","description":"PR sweep","task_type":"local_agent","is_backgrounded":true}),
        );
        assert_eq!(bg_lists(&s)[0][0].kind, "agent");
        assert_eq!(bg_lists(&s)[0][0].item_id.as_deref(), Some("t1:tool:tuA"));
        // Its own frames nest under the card (its stream events are skipped).
        let s = feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"s1"}},"parent_tool_use_id":"tuA"}),
        );
        assert!(s.events.is_empty());
        let s = feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"s1","content":[{"type":"text","text":"Looking"},{"type":"tool_use","id":"tuR","name":"Read","input":{"file_path":"/w/a"}}]},"parent_tool_use_id":"tuA"}),
        );
        let it = items(&s);
        assert!(it[..2]
            .iter()
            .all(|i| i.parent_item_id.as_deref() == Some("t1:tool:tuA")
                && i.turn_id.as_deref() == Some("t1")));
        assert!(
            matches!(&it[2].kind, TurnItemKind::Subagent { tool_uses: 1, last_tool: Some(t), .. } if t == "Read")
        );
        // A subagent's permission request points at ITS tool.
        let s = feed(
            &mut d,
            json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Read","input":{"file_path":"/w/a"},"tool_use_id":"tuR"}}),
        );
        assert!(
            matches!(&s.events[0], DriverEvent::Request { tool_item_id: Some(t), .. } if t == "t1:tool:tuR")
        );
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tuR","content":"# a"}]},"parent_tool_use_id":"tuA"}),
        );
        assert_eq!(items(&s)[0].status, TurnItemStatus::Completed);
        // Launched: the card runs on past the turn.
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tuA","content":"Async agent launched successfully. agentId: x1"}]},"parent_tool_use_id":null}),
        );
        assert_eq!(items(&s)[0].status, TurnItemStatus::Running);
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        assert!(!items(&s).iter().any(|i| i.id == "t1:tool:tuA"));
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_progress","task_id":"a1","tool_use_id":"tuA","last_tool_name":"Grep","usage":{"tool_uses":4}}),
        );
        assert!(matches!(
            &items(&s)[0].kind,
            TurnItemKind::Subagent { tool_uses: 4, .. }
        ));
        let s = feed(
            &mut d,
            json!({"type":"system","subtype":"task_notification","task_id":"a1","tool_use_id":"tuA","status":"completed","summary":"Found 2 open PRs"}),
        );
        let card = &items(&s)[0];
        assert_eq!(card.status, TurnItemStatus::Completed);
        assert_eq!(card.turn_id.as_deref(), Some("t1"), "stays in its turn");
        assert!(
            matches!(&card.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "Found 2 open PRs")
        );
        assert_eq!(bg_lists(&s), vec![Vec::<BackgroundTask>::new()]);
    }

    #[test]
    fn a_foreground_subagent_completes_with_its_tool_result() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("look")).unwrap();
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tuA","name":"Task","input":{"description":"Find files","prompt":"find"}}]},"parent_tool_use_id":null}),
        );
        feed(
            &mut d,
            json!({"type":"system","subtype":"task_started","task_id":"a1","tool_use_id":"tuA","task_type":"local_agent"}),
        );
        assert!(d.background_list().is_empty());
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tuA","content":[{"type":"text","text":"3 files"}]}]},"parent_tool_use_id":null}),
        );
        let card = &items(&s)[0];
        assert_eq!(card.status, TurnItemStatus::Completed);
        assert!(
            matches!(&card.kind, TurnItemKind::Subagent { output: Some(o), background: false, .. } if o == "3 files")
        );
    }

    #[test]
    fn steering_writes_the_message_into_the_running_turn_only() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        assert!(d.steer(&RuntimePromptReq::of("more")).is_err(), "idle");
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.steer(&RuntimePromptReq::of("also check b")).unwrap();
        let w: Value = serde_json::from_str(&s.writes[0]).unwrap();
        assert_eq!(w["type"], "user");
        assert_eq!(w["message"]["content"][0]["text"], "also check b");
        assert!(s.events.is_empty());
        // T3 Code #34564: a model picked with the steer switches the running
        // turn first (stdin order); the same model again sends nothing.
        let req = RuntimePromptReq {
            model: Some("opus".into()),
            ..RuntimePromptReq::of("use opus")
        };
        let s = d.steer(&req).unwrap();
        let w: Vec<Value> = s
            .writes
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            w[0]["request"],
            json!({"subtype":"set_model","model":"opus"})
        );
        assert_eq!(w[1]["message"]["content"][0]["text"], "use opus");
        assert_eq!(d.steer(&req).unwrap().writes.len(), 1);
    }
    #[test]
    fn ask_user_question_is_not_a_permission_and_returns_updated_input() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        let s = feed(
            &mut d,
            json!({"type":"control_request","request_id":"q-raw","request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","tool_use_id":"tu-q","input":{"questions":[{"header":"Targets","question":"Which targets?","multiSelect":true,"options":[{"label":"A"},{"label":"B"}]}]}}}),
        );
        let DriverEvent::Question { id, .. } = &s.events[0] else {
            panic!("question event missing")
        };
        assert!(d.respond(id, ApprovalDecision::AcceptForSession).is_err());
        let response = d
            .respond_question(
                id,
                &[QuestionAnswer::Multi {
                    question_id: "Which targets?".into(),
                    values: vec!["A".into(), "B".into()],
                }],
            )
            .unwrap();
        let value: Value = serde_json::from_str(&response.writes[0]).unwrap();
        assert_eq!(value["response"]["request_id"], "q-raw");
        let result = &value["response"]["response"];
        assert_eq!(result["updatedInput"]["answers"]["Which targets?"], "A, B");
        assert_eq!(result["updatedInput"]["questions"][0]["multiSelect"], true);
        assert!(result.get("updatedPermissions").is_none());
    }
    #[test]
    fn multiple_typed_questions_withdraw_only_the_named_request() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        for id in ["first", "second"] {
            let step = feed(
                &mut d,
                json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[{"question":"Name?","header":"Name"}]}}}),
            );
            assert!(matches!(&step.events[0], DriverEvent::Question { .. }));
        }
        let withdrawn = feed(
            &mut d,
            json!({"type":"control_cancel_request","request_id":"first"}),
        );
        assert!(
            matches!(&withdrawn.events[0], DriverEvent::RequestClosed { request_id, .. } if request_id == "claude-first")
        );
        let answer = QuestionAnswer::Text {
            question_id: "Name?".into(),
            text: "Ada".into(),
        };
        assert!(d
            .respond_question("claude-first", std::slice::from_ref(&answer))
            .is_err());
        assert!(d.respond_question("claude-second", &[answer]).is_ok());
    }

    /// T3 Code #34597: output a stopped turn cut off is interrupted, not a
    /// finished answer — the block still streaming, and on an
    /// `aborted_streaming` result the reply shown last.
    #[test]
    fn a_reply_cut_off_by_a_stop_reads_as_interrupted() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("essay")).unwrap();
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}},"parent_tool_use_id":null}),
        );
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}),
        );
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"The audit covers the `integ"}},"parent_tool_use_id":null}),
        );
        // The partial assistant frame, then the abort (no content_block_stop).
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"The audit covers the `integ"}]},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false,"terminal_reason":"aborted_streaming","result":"The audit covers the `integ"}),
        );
        let reply = &items(&s)[0];
        assert_eq!(reply.id, "t1:m1:0");
        assert_eq!(reply.status, TurnItemStatus::Interrupted);
        assert!(!reply.is_streaming());
        assert!(matches!(
            s.events.last(),
            Some(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                ..
            })
        ));

        // A reply whose block closed before the abort is re-marked.
        d.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"Half a"}]},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false,"terminal_reason":"aborted_streaming"}),
        );
        assert!(items(&s)
            .iter()
            .any(|i| i.id == "t2:m2:a0" && i.status == TurnItemStatus::Interrupted));

        // Stopped during a tool: the reply before the call was whole.
        d.prompt("t3", &RuntimePromptReq::of("run")).unwrap();
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"Running it."}]},"parent_tool_use_id":null}),
        );
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"tu3","name":"Bash","input":{"command":"sleep 9"}}]},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false,"terminal_reason":"aborted_streaming"}),
        );
        assert!(
            !items(&s)
                .iter()
                .any(|i| i.status == TurnItemStatus::Interrupted),
            "{:?}",
            items(&s)
        );

        // A completed turn closes its open blocks as completed, as before.
        d.prompt("t4", &RuntimePromptReq::of("hi")).unwrap();
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}),
        );
        feed(
            &mut d,
            json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"done"}},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false}),
        );
        assert_eq!(items(&s)[0].status, TurnItemStatus::Completed);
    }

    /// T3 Code #34183: a call a new message (or Stop) cut short is stopped,
    /// not failed; Claude Code says so in `tool_result_meta`.
    #[test]
    fn a_tool_call_interrupted_by_a_message_is_stopped_not_failed() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("wait")).unwrap();
        feed(
            &mut d,
            json!({"type":"assistant","message":{"id":"m1","content":[
                {"type":"tool_use","id":"tu1","name":"mcp__otto__wait","input":{}},
                {"type":"tool_use","id":"tu2","name":"Bash","input":{"command":"false"}},
                {"type":"tool_use","id":"tu3","name":"Read","input":{"file_path":"/w/a"}}
            ]},"parent_tool_use_id":null}),
        );
        let s = feed(
            &mut d,
            json!({"type":"user","message":{"content":[
                {"type":"tool_result","tool_use_id":"tu1","content":"Error: The tool call was interrupted before a result was received","is_error":true},
                {"type":"tool_result","tool_use_id":"tu2","content":"exit 1","is_error":true},
                {"type":"tool_result","tool_use_id":"tu3","content":"cancelled","is_error":true}
            ]},"tool_result_meta":[{"id":"tu1","non_execution_kind":"interrupted"},{"id":"tu2","non_execution_kind":null},{"id":"tu3","non_execution_kind":"cancelled"}],"parent_tool_use_id":null}),
        );
        let it = items(&s);
        assert_eq!(it[0].status, TurnItemStatus::Interrupted);
        assert!(it[0].error.is_none(), "not an error: {:?}", it[0].error);
        assert_eq!(it[1].status, TurnItemStatus::Failed);
        assert!(it[1].error.is_some());
        assert_eq!(it[2].status, TurnItemStatus::Cancelled);
    }

    /// T3 Code #34822: a resumed Claude waits for its MCP servers before the
    /// first request; a fresh thread starts as fast as before.
    #[test]
    fn a_resumed_session_waits_for_its_mcp_servers() {
        let d = ClaudeDriver::new();
        let has = |spec: &LaunchSpec| {
            spec.env
                .iter()
                .any(|(k, v)| k == MCP_NONBLOCKING && v == "0")
        };
        assert!(!has(&d.launch(&ctx(None))));
        // The daemon's own environment, when set, decides instead.
        assert_eq!(
            has(&d.launch(&ctx(Some("abc")))),
            std::env::var_os(MCP_NONBLOCKING).is_none()
        );
    }

    /// T3 Code #31370: the context meter's window is the one Claude Code
    /// reports for the model that ran (`result.modelUsage`), and its fill is
    /// the last message's context, not the turn's summed usage.
    #[test]
    fn the_context_meter_uses_the_window_claude_enforces() {
        let mut d = ClaudeDriver::new();
        ready(&mut d);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        for (id, input) in [("m1", 1000), ("m2", 1500)] {
            feed(
                &mut d,
                json!({"type":"assistant","message":{"id":id,"model":"claude-opus-5-5","content":[{"type":"text","text":"x"}],
                    "usage":{"input_tokens":input,"cache_read_input_tokens":180000,"cache_creation_input_tokens":0,"output_tokens":40}},"parent_tool_use_id":null}),
            );
        }
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false,
            "usage":{"input_tokens":2500,"cache_read_input_tokens":360000,"output_tokens":80},
            "modelUsage":{
                "claude-opus-5-5":{"inputTokens":2500,"contextWindow":200000},
                "claude-haiku-5-5":{"inputTokens":90,"contextWindow":1000000}
            }}),
        );
        let ctx = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::ContextUsage(u) => Some(u.clone()),
                _ => None,
            })
            .expect("context usage");
        assert_eq!(ctx.max_tokens, Some(200_000), "the enforced window");
        assert_eq!(ctx.used_tokens, 1500 + 180_000 + 40, "the last message");
        assert_eq!(ctx.cached_input_tokens, Some(180_000));
        // No message usage this turn: no reading, rather than a stale one.
        d.prompt("t2", &RuntimePromptReq::of("go")).unwrap();
        let s = feed(
            &mut d,
            json!({"type":"result","subtype":"success","is_error":false,"modelUsage":{}}),
        );
        assert!(!s
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::ContextUsage(_))));
    }
}
