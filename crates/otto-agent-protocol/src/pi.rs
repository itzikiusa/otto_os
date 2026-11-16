//! pi (earendil-works/pi) driver — pi's own RPC mode (`pi --mode rpc`).
//!
//! T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.) drives pi the same way
//! (`packages/provider-pi`). The protocol (`docs/rpc.md`,
//! `docs/rpc-commands.md`, `docs/json.md` in the pi package; verified live
//! against pi 1.5.0, still compatible with 0.75):
//!
//! * argv — `pi --mode rpc [--session <id>] [--model provider/id]
//!   [--tools … --exclude-tools mcp__<user server>__*]`. `--session` takes the id
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
//!   restricted modes exclude the user's servers (Otto's own stay). A
//!   switch respawns with `--session`.
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
    ApprovalDecision, BackgroundTask, ProviderCommand, ProviderFailure, QuestionKind,
    QuestionOption, RequestKind, RuntimeAccessMode, RuntimePromptReq, RuntimeProvider,
    RuntimeQuestion, ThreadTokenUsage, TurnItemKind, TurnItemStatus, TurnTerminalStatus,
    TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, restore_keeping_program, rpc_key, tool_error, with_file_paths, DriverError, DriverEvent,
    DriverStep, LaunchContext, LaunchSpec, StructuredDriver,
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

#[path = "pi_subagent.rs"]
mod pi_subagent;

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

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Tool {
    name: String,
    args: Value,
    #[serde(default)]
    item_id: String,
    #[serde(default)]
    is_subagent: bool,
    #[serde(default)]
    parent_item_id: Option<String>,
    #[serde(default)]
    parent_key: Option<String>,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default = "chrono::Utc::now")]
    started_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    streamed_output: String,
    #[serde(default)]
    tool_uses: u32,
    #[serde(default)]
    last_tool: Option<String>,
    #[serde(default)]
    bg_id: Option<String>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct PiBgEntry {
    call_id: String,
    item_id: String,
    turn_id: Option<String>,
    name: String,
    args: Value,
    kind: String,
    label: String,
    started_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    tool_uses: u32,
    #[serde(default)]
    last_tool: Option<String>,
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

/// Serialized whole for a daemon restart ([`StructuredDriver::snapshot_state`]):
/// every field must stay serializable, new ones `#[serde(default)]`.
/// Every field may be missing from a snapshot an older build wrote (a
/// deploy must never kill a live thread over a new field).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PiDriver {
    /// The factory's binary: kept across a restore, never snapshotted.
    #[serde(skip)]
    program: String,
    next_id: u64,
    state_req: Option<String>,
    prompt_req: Option<String>,
    steer_reqs: Vec<String>,
    ready: bool,
    turn_id: Option<String>,
    auto_turns: u64,
    /// The streaming assistant message: its item id, text so far, and start time.
    message: Option<(String, String, chrono::DateTime<chrono::Utc>)>,
    message_parent: Option<String>,
    /// The streaming thinking/reasoning block: its item id, text so far, and start time.
    thought: Option<(String, String, chrono::DateTime<chrono::Utc>)>,
    thought_parent: Option<String>,
    messages: u32,
    tools: HashMap<String, Tool>,
    subagent_order: Vec<String>,
    /// This pi sends `agent_settled` (1.x): a turn ends there, and a
    /// non-retrying `agent_end` only stashes its outcome in `ended`.
    settles: bool,
    ended: Option<(TurnTerminalStatus, Option<ProviderFailure>)>,
    #[serde(default)]
    in_agent_run: bool,
    #[serde(default)]
    pending_abort: bool,
    #[serde(default)]
    bg_tasks: Vec<BackgroundTask>,
    #[serde(default)]
    bg_entries: HashMap<String, PiBgEntry>,
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

impl Default for PiDriver {
    fn default() -> Self {
        Self::with_program("pi")
    }
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
            message_parent: None,
            thought: None,
            thought_parent: None,
            messages: 0,
            tools: HashMap::new(),
            subagent_order: Vec::new(),
            settles: false,
            ended: None,
            in_agent_run: false,
            pending_abort: false,
            bg_tasks: Vec::new(),
            bg_entries: HashMap::new(),
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

    fn ensure_autonomous_turn(&mut self, step: &mut DriverStep) {
        if !self.ready || self.turn_id.is_some() {
            return;
        }
        self.auto_turns += 1;
        let id = format!("auto-{}", self.auto_turns);
        self.turn_id = Some(id.clone());
        self.messages = 0;
        self.spent = Spent::default();
        self.ended = None;
        self.tools.retain(|_, t| t.bg_id.is_some());
        self.subagent_order.retain(|c| self.tools.contains_key(c));
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
        self.cancel_dialogs(step);
        self.settle_foreground_tools_on_finish(step, status);
        self.turn_id = None;
        self.prompt_req = None;
        self.steer_reqs.clear();
        self.ended = None;
        self.in_agent_run = false;
        self.pending_abort = false;
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
    }

    fn close_thought(&mut self, step: &mut DriverStep) {
        let parent_item_id = self.thought_parent.take();
        if let Some((id, text, started_at)) = self.thought.take() {
            if !text.is_empty() {
                let mut it = item(
                    id,
                    TurnItemStatus::Completed,
                    None,
                    TurnItemKind::Reasoning {
                        text,
                        streaming: false,
                    },
                );
                it.started_at = Some(started_at);
                it.completed_at = Some(chrono::Utc::now());
                it.parent_item_id = parent_item_id;
                step.push(DriverEvent::TurnItem(it));
            }
        }
    }

    fn close_message(&mut self, step: &mut DriverStep) {
        let parent_item_id = self.message_parent.take();
        if let Some((id, text, started_at)) = self.message.take() {
            if !text.is_empty() {
                let mut it = item(
                    id.clone(),
                    TurnItemStatus::Completed,
                    None,
                    TurnItemKind::AssistantMessage {
                        message_id: id,
                        text,
                        streaming: false,
                    },
                );
                it.started_at = Some(started_at);
                it.completed_at = Some(chrono::Utc::now());
                it.parent_item_id = parent_item_id;
                step.push(DriverEvent::TurnItem(it));
            }
        }
    }

    fn on_thought(&mut self, step: &mut DriverStep, delta: &str, parent_item_id: Option<String>) {
        if delta.is_empty() {
            return;
        }
        self.close_message(step);
        if self.thought.is_some() && self.thought_parent != parent_item_id {
            self.close_thought(step);
        }
        if self.thought.is_none() {
            self.messages += 1;
            self.thought_parent = parent_item_id.clone();
            self.thought = Some((
                format!("{}:thought:{}", self.turn(), self.messages),
                String::new(),
                chrono::Utc::now(),
            ));
        }
        let (id, text, started_at) = self.thought.as_mut().expect("set above");
        text.push_str(delta);
        let mut it = item(
            id.clone(),
            TurnItemStatus::Running,
            None,
            TurnItemKind::Reasoning {
                text: text.clone(),
                streaming: true,
            },
        );
        it.started_at = Some(*started_at);
        it.parent_item_id = self.thought_parent.clone();
        step.push(DriverEvent::TurnItem(it));
    }

    fn on_text(&mut self, step: &mut DriverStep, delta: &str, parent_item_id: Option<String>) {
        if delta.is_empty() {
            return;
        }
        self.close_thought(step);
        if self.message.is_some() && self.message_parent != parent_item_id {
            self.close_message(step);
        }
        if self.message.is_none() {
            self.messages += 1;
            self.message_parent = parent_item_id.clone();
            self.message = Some((
                format!("{}:msg:{}", self.turn(), self.messages),
                String::new(),
                chrono::Utc::now(),
            ));
        }
        let (id, text, started_at) = self.message.as_mut().expect("set above");
        text.push_str(delta);
        let mut it = item(
            id.clone(),
            TurnItemStatus::Running,
            None,
            TurnItemKind::AssistantMessage {
                message_id: id.clone(),
                text: text.clone(),
                streaming: true,
            },
        );
        it.started_at = Some(*started_at);
        it.parent_item_id = self.message_parent.clone();
        step.push(DriverEvent::TurnItem(it));
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

    fn tool_kind(
        name: &str,
        args: &Value,
        output: Option<String>,
        tool_uses: u32,
        last_tool: Option<String>,
    ) -> TurnItemKind {
        let s = |k: &str| args.get(k).and_then(Value::as_str).map(str::to_string);
        if Self::is_subagent_tool(name, args) {
            let (description, subagent_type, prompt, background) = Self::subagent_fields(args);
            return TurnItemKind::Subagent {
                tool_name: name.to_string(),
                description,
                subagent_type,
                prompt,
                background,
                tool_uses,
                last_tool,
                output,
            };
        }
        match name {
            "bash" => TurnItemKind::CommandExecution {
                input: s("command").unwrap_or_default(),
                output,
                exit_code: None,
            },
            // pi's `edit` takes `edits: [{oldText, newText}]` (a lone
            // top-level pair is the legacy shape): several are one patch.
            "edit" | "write" => {
                let file_name = s("path").or_else(|| s("file_path")).unwrap_or_default();
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
                let (diff, old, new) = match edits.len() {
                    0 => (
                        None,
                        s("oldText").or_else(|| s("old_string")),
                        s("newText")
                            .or_else(|| s("content"))
                            .or_else(|| s("new_string")),
                    ),
                    1 => {
                        let (o, n) = edits.remove(0);
                        (None, Some(o), Some(n))
                    }
                    _ => (Some(edit_hunks(&file_name, &edits)), None, None),
                };
                let (diff_str, old_str, new_str) =
                    crate::diff_compact::compact_file_change_fields(&file_name, diff, old, new);
                TurnItemKind::FileChange {
                    file_name,
                    diff_str,
                    old_str,
                    new_str,
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

    fn tool_title(name: &str, args: &Value) -> Option<String> {
        if Self::is_subagent_tool(name, args) {
            let (description, subagent_type, _, _) = Self::subagent_fields(args);
            return description
                .or(subagent_type)
                .or_else(|| Some(name.to_string()));
        }
        Some(name.to_string())
    }

    fn extract_launched_bg_id(result: &Value, output: Option<&str>) -> Option<(String, bool)> {
        let details = result.get("details");
        if let Some(d) = details {
            let id = Self::arg_str(
                d,
                &[
                    "taskId",
                    "task_id",
                    "backgroundTaskId",
                    "agentId",
                    "agent_id",
                    "shellId",
                    "shell_id",
                    "id",
                ],
            );
            let status = d
                .get("status")
                .or_else(|| d.get("state"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let still_running = matches!(
                status,
                "running" | "started" | "in_progress" | "inProgress" | "background" | "pending"
            ) || (id.is_some() && status.is_empty());
            if let Some(id) = id {
                return Some((id, still_running));
            }
            if still_running {
                return Some((String::new(), true));
            }
        }
        if let Some(text) = output {
            let trimmed = text.trim();
            for prefix in [
                "Background task started:",
                "Started background task:",
                "Task ID:",
                "Agent ID:",
            ] {
                if let Some(rest) = trimmed.strip_prefix(prefix) {
                    let token: String = rest
                        .trim()
                        .chars()
                        .take_while(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.')
                        })
                        .collect();
                    if !token.is_empty() {
                        return Some((token, true));
                    }
                }
            }
        }
        None
    }

    fn settle_bg_entry(
        &mut self,
        step: &mut DriverStep,
        bg_id: &str,
        status: TurnItemStatus,
        output: Option<String>,
        emit_card: bool,
    ) {
        let Some(entry) = self.bg_entries.remove(bg_id) else {
            return;
        };
        let before = self.bg_tasks.len();
        self.bg_tasks
            .retain(|t| t.id != bg_id && t.id != entry.call_id);
        if self.bg_tasks.len() != before {
            step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
        }
        Self::settle_bg_entry_inner(
            entry,
            bg_id,
            status,
            output,
            emit_card,
            self.turn_id.clone(),
            step,
        );
    }

    fn maybe_handle_bg_control_tool(
        &mut self,
        step: &mut DriverStep,
        name: &str,
        args: &Value,
        result: &Value,
        output: Option<&str>,
        failed: bool,
    ) {
        let lower = name.trim().to_ascii_lowercase();
        let bare = lower
            .rsplit_once("__")
            .map(|(_, tail)| tail)
            .unwrap_or(lower.as_str());
        let Some(target_id) = Self::arg_str(
            args,
            &[
                "task_id", "taskId", "shell_id", "shellId", "agent_id", "agentId", "id",
            ],
        ) else {
            return;
        };
        if matches!(
            bare,
            "kill_shell"
                | "killshell"
                | "task_stop"
                | "stop_task"
                | "cancel_subagent"
                | "kill_task"
        ) && !failed
        {
            self.settle_bg_entry(
                step,
                &target_id,
                TurnItemStatus::Cancelled,
                output.map(str::to_string),
                true,
            );
        } else if matches!(
            bare,
            "task_output" | "bash_output" | "subagent_output" | "get_task_output"
        ) && !failed
        {
            let still_running = result
                .pointer("/details/status")
                .and_then(Value::as_str)
                .is_some_and(|s| matches!(s, "running" | "in_progress" | "pending"));
            if !still_running {
                self.settle_bg_entry(
                    step,
                    &target_id,
                    TurnItemStatus::Completed,
                    output.map(str::to_string),
                    true,
                );
            }
        }
    }

    fn result_text(result: &Value) -> Option<String> {
        if let Some(s) = result.as_str().filter(|s| !s.is_empty()) {
            return Some(s.to_string());
        }
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
                if self.pending_abort {
                    self.pending_abort = false;
                    self.in_agent_run = false;
                    self.command(step, json!({"type": "abort"}));
                    return;
                }
                self.in_agent_run = true;
                self.ensure_autonomous_turn(step);
            }
            Some("message_start") => {
                self.in_agent_run = true;
                self.ensure_autonomous_turn(step);
                if self.turn_id.is_some() {
                    self.close_thought(step);
                    self.close_message(step);
                }
            }
            Some("message_update") => {
                self.in_agent_run = true;
                let ev = &v["assistantMessageEvent"];
                let seg_parent = self
                    .resolve_parent_subagent(v, None, None, false)
                    .map(|(_, pid)| pid);
                match ev.get("type").and_then(Value::as_str) {
                    Some("text_start") => {
                        self.ensure_autonomous_turn(step);
                        if self.turn_id.is_some() {
                            self.close_thought(step);
                            if self.message.is_some() && self.message_parent != seg_parent {
                                self.close_message(step);
                            }
                            if self.message.is_none() {
                                self.messages += 1;
                                self.message_parent = seg_parent;
                                self.message = Some((
                                    format!("{}:msg:{}", self.turn(), self.messages),
                                    String::new(),
                                    chrono::Utc::now(),
                                ));
                            }
                        }
                    }
                    Some("thinking_start") => {
                        self.ensure_autonomous_turn(step);
                        if self.turn_id.is_some() {
                            self.close_message(step);
                            if self.thought.is_some() && self.thought_parent != seg_parent {
                                self.close_thought(step);
                            }
                            if self.thought.is_none() {
                                self.messages += 1;
                                self.thought_parent = seg_parent;
                                self.thought = Some((
                                    format!("{}:thought:{}", self.turn(), self.messages),
                                    String::new(),
                                    chrono::Utc::now(),
                                ));
                            }
                        }
                    }
                    Some("text_delta") => {
                        let delta = ev.get("delta").and_then(Value::as_str).unwrap_or("");
                        if !delta.is_empty() {
                            self.ensure_autonomous_turn(step);
                            if self.turn_id.is_some() {
                                self.on_text(step, delta, seg_parent);
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        let delta = ev.get("delta").and_then(Value::as_str).unwrap_or("");
                        if !delta.is_empty() {
                            self.ensure_autonomous_turn(step);
                            if self.turn_id.is_some() {
                                self.on_thought(step, delta, seg_parent);
                            }
                        }
                    }
                    _ => {}
                }
            }
            Some("message_end") if self.turn_id.is_some() => {
                self.close_thought(step);
                self.close_message(step);
                if v.pointer("/message/role").and_then(Value::as_str) == Some("assistant") {
                    self.on_usage(step, &v["message"]);
                }
            }
            Some("tool_execution_start") => self.on_tool_start(step, v),
            Some("tool_execution_update") => self.on_tool_update(step, v),
            Some("tool_execution_end") => self.on_tool_end(step, v),
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
            // T3 Code #33151: pi's own retries and compactions, one row each.
            Some("auto_retry_start") if self.turn_id.is_some() => {
                let n = |k: &str| v.get(k).and_then(Value::as_u64);
                let why = v.get("errorMessage").and_then(Value::as_str);
                let row =
                    crate::driver::retry_notice(self.turn(), n("attempt"), n("maxAttempts"), why);
                step.push(DriverEvent::TurnItem(row));
            }
            Some(t @ ("auto_compaction_start" | "auto_compaction_end"))
                if self.turn_id.is_some() =>
            {
                let done = t == "auto_compaction_end";
                step.push(DriverEvent::TurnItem(crate::driver::compaction_notice(
                    self.turn(),
                    done,
                )));
            }
            _ => {}
        }
    }

    fn on_tool_start(&mut self, step: &mut DriverStep, v: &Value) {
        self.in_agent_run = true;
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
        let is_subagent = Self::is_subagent_tool(&name, &args);
        let parent = self.resolve_parent_subagent(v, Some(&args), Some(call), is_subagent);
        let parent_key = parent.as_ref().map(|(k, _)| k.clone());
        let parent_item_id = parent.as_ref().map(|(_, pid)| pid.clone());
        let started_at = chrono::Utc::now();
        let turn_id = self.turn_id.clone();
        let id = format!("{}:tool:{call}", self.turn());
        let mut ti = item(
            id.clone(),
            TurnItemStatus::Running,
            Self::tool_title(&name, &args),
            Self::tool_kind(&name, &args, None, 0, None),
        );
        ti.turn_id = turn_id.clone();
        ti.started_at = Some(started_at);
        ti.parent_item_id = parent_item_id.clone();
        step.push(DriverEvent::TurnItem(ti));

        if let Some(ref pkey) = parent_key {
            self.record_child_tool_on_parent(step, pkey, &name);
        }

        let bg_id = if Self::is_background_call(&name, &args) {
            let (kind, label) = Self::bg_kind_and_label(&name, &args);
            let due_at = Self::extract_due_at(&name, &args, started_at);
            let bg_id = call.to_string();
            self.bg_tasks.push(BackgroundTask {
                id: bg_id.clone(),
                kind: kind.to_string(),
                label: label.clone(),
                started_at,
                due_at,
                stopping: false,
                stop_error: None,
                item_id: Some(id.clone()),
            });
            self.bg_entries.insert(
                bg_id.clone(),
                PiBgEntry {
                    call_id: call.to_string(),
                    item_id: id.clone(),
                    turn_id: turn_id.clone(),
                    name: name.clone(),
                    args: args.clone(),
                    kind: kind.to_string(),
                    label,
                    started_at,
                    tool_uses: 0,
                    last_tool: None,
                },
            );
            step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
            Some(bg_id)
        } else {
            None
        };

        if is_subagent && bg_id.is_none() {
            self.subagent_order.retain(|c| c != call);
            self.subagent_order.push(call.to_string());
        }

        self.tools.insert(
            call.to_string(),
            Tool {
                name,
                args,
                item_id: id,
                is_subagent,
                parent_item_id,
                parent_key,
                turn_id,
                started_at,
                streamed_output: String::new(),
                tool_uses: 0,
                last_tool: None,
                bg_id,
            },
        );
    }

    fn on_tool_update(&mut self, step: &mut DriverStep, v: &Value) {
        let Some(call) = v.get("toolCallId").and_then(Value::as_str) else {
            return;
        };
        let partial = &v["partialResult"];
        let partial_text = Self::result_text(partial);
        let Some(tool) = self.tools.get_mut(call) else {
            return;
        };
        let turn_str = tool
            .turn_id
            .as_deref()
            .or(self.turn_id.as_deref())
            .unwrap_or_default();
        let item_id = if tool.item_id.is_empty() {
            format!("{turn_str}:tool:{call}")
        } else {
            tool.item_id.clone()
        };
        if tool.name == "bash" {
            if let Some(full) = partial_text {
                if let Some(delta) = full.strip_prefix(tool.streamed_output.as_str()) {
                    if !delta.is_empty() {
                        let chunk = delta.to_string();
                        tool.streamed_output = full;
                        step.push(DriverEvent::CommandOutput { item_id, chunk });
                    }
                }
            }
        } else if Self::is_subagent_tool(&tool.name, &tool.args) {
            let details = partial.get("details");
            let uses = details
                .and_then(|d| d.get("toolUses").or_else(|| d.get("tool_uses")))
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let last = details
                .and_then(|d| Self::arg_str(d, &["lastTool", "last_tool", "currentTool", "tool"]));
            let mut dirty = false;
            if let Some(u) = uses.filter(|u| *u != tool.tool_uses) {
                tool.tool_uses = u;
                dirty = true;
            } else if last.is_some() && last != tool.last_tool {
                tool.tool_uses = tool.tool_uses.saturating_add(1);
                dirty = true;
            }
            if last.is_some() && last != tool.last_tool {
                tool.last_tool = last;
                dirty = true;
            }
            if dirty {
                let mut ti = item(
                    item_id,
                    TurnItemStatus::Running,
                    Self::tool_title(&tool.name, &tool.args),
                    Self::tool_kind(
                        &tool.name,
                        &tool.args,
                        partial_text,
                        tool.tool_uses,
                        tool.last_tool.clone(),
                    ),
                );
                ti.turn_id = tool.turn_id.clone();
                ti.started_at = Some(tool.started_at);
                ti.parent_item_id = tool.parent_item_id.clone();
                step.push(DriverEvent::TurnItem(ti));
            }
        }
    }

    fn on_tool_end(&mut self, step: &mut DriverStep, v: &Value) {
        let call = v
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or("tool");
        self.subagent_order.retain(|c| c != call);
        let tool = self.tools.remove(call).unwrap_or_else(|| {
            let name: String = v
                .get("toolName")
                .and_then(Value::as_str)
                .unwrap_or("tool")
                .into();
            let args = v.get("args").cloned().unwrap_or(Value::Null);
            let is_subagent = Self::is_subagent_tool(&name, &args);
            let parent = self.resolve_parent_subagent(v, Some(&args), Some(call), is_subagent);
            Tool {
                name,
                args,
                item_id: String::new(),
                is_subagent,
                parent_item_id: parent.as_ref().map(|(_, pid)| pid.clone()),
                parent_key: parent.map(|(k, _)| k),
                turn_id: self.turn_id.clone(),
                started_at: chrono::Utc::now(),
                streamed_output: String::new(),
                tool_uses: 0,
                last_tool: None,
                bg_id: None,
            }
        });
        let result = &v["result"];
        let output = Self::result_text(result)
            .or_else(|| (!tool.streamed_output.is_empty()).then(|| tool.streamed_output.clone()));
        // pi's `read` of an image file returns `{type: image, data, mimeType}`.
        let images = crate::tool_images::extract(Some(result)).images;
        let failed = v.get("isError").and_then(Value::as_bool) == Some(true);
        let turn_str = tool
            .turn_id
            .clone()
            .or_else(|| self.turn_id.clone())
            .unwrap_or_default();
        let item_id = if tool.item_id.is_empty() {
            format!("{turn_str}:tool:{call}")
        } else {
            tool.item_id.clone()
        };
        if self.thought_parent.as_deref() == Some(item_id.as_str()) {
            self.close_thought(step);
        }
        if self.message_parent.as_deref() == Some(item_id.as_str()) {
            self.close_message(step);
        }
        let now = chrono::Utc::now();

        if let Some(orig_bg_id) = tool.bg_id.clone() {
            if !failed {
                if let Some((launched_id, true)) =
                    Self::extract_launched_bg_id(result, output.as_deref())
                {
                    let new_id = if launched_id.is_empty() {
                        orig_bg_id.clone()
                    } else {
                        launched_id
                    };
                    if new_id != orig_bg_id {
                        if let Some(mut entry) = self.bg_entries.remove(&orig_bg_id) {
                            entry.tool_uses = tool.tool_uses;
                            entry.last_tool = tool.last_tool.clone();
                            self.bg_entries.insert(new_id.clone(), entry);
                        }
                        if let Some(bt) = self.bg_tasks.iter_mut().find(|t| t.id == orig_bg_id) {
                            bt.id = new_id;
                            step.push(DriverEvent::BackgroundTasks(self.bg_tasks.clone()));
                        }
                    } else if let Some(entry) = self.bg_entries.get_mut(&orig_bg_id) {
                        entry.tool_uses = tool.tool_uses;
                        entry.last_tool = tool.last_tool.clone();
                    }
                    let mut it = item(
                        item_id,
                        TurnItemStatus::Running,
                        Self::tool_title(&tool.name, &tool.args),
                        Self::tool_kind(
                            &tool.name,
                            &tool.args,
                            output,
                            tool.tool_uses,
                            tool.last_tool,
                        ),
                    );
                    it.turn_id = tool.turn_id;
                    it.started_at = Some(tool.started_at);
                    it.parent_item_id = tool.parent_item_id;
                    step.push(DriverEvent::TurnItem(it));
                    return;
                }
            }

            let status = if failed {
                TurnItemStatus::Failed
            } else {
                TurnItemStatus::Completed
            };
            let mut it = item(
                item_id,
                status,
                Self::tool_title(&tool.name, &tool.args),
                Self::tool_kind(
                    &tool.name,
                    &tool.args,
                    output.clone(),
                    tool.tool_uses,
                    tool.last_tool,
                ),
            );
            it.turn_id = tool.turn_id;
            it.started_at = Some(tool.started_at);
            it.completed_at = Some(now);
            it.parent_item_id = tool.parent_item_id;
            if failed {
                it.error = output.as_deref().and_then(tool_error);
            }
            step.push(DriverEvent::TurnItem(it));
            self.settle_bg_entry(step, &orig_bg_id, status, output, false);
            return;
        }

        let mut it = item(
            item_id,
            if failed {
                TurnItemStatus::Failed
            } else {
                TurnItemStatus::Completed
            },
            Self::tool_title(&tool.name, &tool.args),
            Self::tool_kind(
                &tool.name,
                &tool.args,
                output.clone(),
                tool.tool_uses,
                tool.last_tool,
            ),
        );
        it.turn_id = tool.turn_id;
        it.started_at = Some(tool.started_at);
        it.completed_at = Some(now);
        it.parent_item_id = tool.parent_item_id;
        if failed {
            it.error = output.as_deref().and_then(tool_error);
        }
        if let TurnItemKind::DynamicTool { images: kept, .. } = &mut it.kind {
            *kept = images;
        }
        step.push(DriverEvent::TurnItem(it));
        self.maybe_handle_bg_control_tool(
            step,
            &tool.name,
            &tool.args,
            result,
            output.as_deref(),
            failed,
        );
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
                self.ready = true;
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
            // `--tools` keeps MCP tools, which can write: `adopt_spec`
            // drops the user's servers.
            args.extend(["--tools".into(), t.into()]);
        }
        LaunchSpec {
            program: self.program.clone(),
            args,
            ..LaunchSpec::default()
        }
    }

    /// `get_state` reports the session id (the resume key) and the model.
    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        let mut step = DriverStep::default();
        if ctx.resume.is_none() {
            self.tools.clear();
            self.subagent_order.clear();
            self.message = None;
            self.message_parent = None;
            self.thought = None;
            self.thought_parent = None;
            self.turn_id = None;
            self.ended = None;
            self.in_agent_run = false;
            self.pending_abort = false;
            self.bg_entries.clear();
            if !self.bg_tasks.is_empty() {
                self.bg_tasks.clear();
                step.push(DriverEvent::BackgroundTasks(Vec::new()));
            }
        }
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
        self.in_agent_run = false;
        self.pending_abort = false;
        self.spent = Spent::default();
        let mut cmd = json!({"type": "prompt", "message": with_file_paths(&req.text, &req.files)});
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
        if req.text.is_empty() && req.images.is_empty() && req.files.is_empty() {
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
        let mut cmd = json!({"type": "steer", "message": with_file_paths(&req.text, &req.files)});
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
        if self.turn_id.is_some() && !self.in_agent_run {
            self.finish(&mut step, TurnTerminalStatus::Interrupted, None);
            self.pending_abort = true;
        }
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
            || self.ready
            || v.get("type").and_then(Value::as_str) == Some("extension_ui_request")
        {
            self.on_event(&mut step, &v);
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        let mut step = DriverStep::default();
        self.steer_reqs.clear();
        self.subagent_order.clear();
        self.clear_all_bg_on_session_close(&mut step);
        step
    }

    fn snapshot_state(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn restore_state(&mut self, state: &Value) -> Result<(), DriverError> {
        restore_keeping_program(self, state, |d| &mut d.program)
    }
}

#[cfg(test)]
#[path = "pi_tests.rs"]
mod tests;
