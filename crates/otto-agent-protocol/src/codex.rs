//! Codex driver — `codex app-server` (JSON-RPC over stdio, one message per
//! line, no `"jsonrpc"` member), ported from T3 Code (`CodexAdapterV2.ts`, MIT,
//! Copyright (c) 2026 T3 Tools Inc.) against `codex app-server` 0.166.0:
//!
//! * `initialize` + `initialized`, then `thread/start` or `thread/resume`.
//! * `turn/start`, `turn/steer` mid-turn, and `turn/interrupt`, with per-turn
//!   `approvalPolicy` + `sandboxPolicy` (`externalSandbox` under Otto Seatbelt).
//! * `item/started`, `item/completed`, deltas, `turn/plan/updated`,
//!   `thread/tokenUsage/updated`, approvals, and `collabAgentToolCall`
//!   subagent tracking (`codex_subagent.rs`).

#[path = "codex_subagent.rs"]
mod subagent;

use std::collections::HashMap;

use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, BackgroundTask, PlanStep, PlanStepStatus, ProviderCommand,
    ProviderFailure, RequestKind, RuntimeAccessMode, RuntimeCapabilities, RuntimePromptReq,
    RuntimeProvider, RuntimeRequestStatus, ThreadTokenUsage, TurnItem, TurnItemKind,
    TurnItemStatus, TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    drop_unsupported_effort, item, restore_keeping_program, rpc_failure, rpc_key, tool_error,
    with_file_paths, DriverError, DriverEvent, DriverStep, LaunchContext, LaunchSpec,
    StructuredDriver, TaskStop,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum ReqProto {
    /// `item/*/requestApproval` — `{decision}` in v2 spelling.
    V2Decision,
    /// `item/permissions/requestApproval`.
    Permissions,
    Question,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Pending {
    rpc_id: Value,
    proto: ReqProto,
    permissions: Value,
}

/// Accumulated state of one Codex item.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Item {
    pub(crate) text: String,
    pub(crate) output: String,
    pub(crate) started_at: chrono::DateTime<chrono::Utc>,
}

impl Default for Item {
    fn default() -> Self {
        Self {
            text: String::new(),
            output: String::new(),
            started_at: chrono::Utc::now(),
        }
    }
}

/// Every field may be missing from a snapshot an older build wrote (a
/// deploy must never kill a live thread over a new field).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CodexDriver {
    /// The factory's binary: kept across a restore, never snapshotted.
    #[serde(skip)]
    program: String,
    next_id: u64,
    init_id: u64,
    thread_req: Option<(u64, bool)>,
    turn_req: Option<u64>,
    steer_reqs: Vec<u64>,
    ready: bool,
    cwd: String,
    model: Option<String>,
    resume: Option<String>,
    sandboxed: bool,
    access: RuntimeAccessMode,
    pub(crate) thread_id: Option<String>,
    pub(crate) turn_id: Option<String>,
    auto_turns: u64,
    queued: Option<RuntimePromptReq>,
    codex_turn: Option<String>,
    interrupt_when_started: bool,
    items: HashMap<String, Item>,
    pending: HashMap<String, Pending>,
    failure: Option<ProviderFailure>,
    /// MCP servers that failed to start and are not shown yet: (name, error).
    mcp_failures: Vec<(String, String)>,
    #[serde(default)]
    pub(crate) subagents: HashMap<String, subagent::CodexSubagent>,
    #[serde(default)]
    pub(crate) child_thread_to_key: HashMap<String, String>,
    #[serde(default)]
    pub(crate) child_items: HashMap<String, HashMap<String, Item>>,
    #[serde(default)]
    pub(crate) bg_tasks: Vec<BackgroundTask>,
    /// The model the thread runs (a plan-mode turn names it).
    #[serde(default)]
    thread_model: Option<String>,
    /// The last turn ran in plan mode: the next one names `default` to leave.
    #[serde(default)]
    plan: bool,
    /// Otto's `-c developer_instructions=…` (the context hook's): a
    /// `collaborationMode` replaces the thread's instructions, so leaving
    /// plan mode hands them back.
    #[serde(default)]
    developer_instructions: Option<String>,
    /// Skills from `skills/list`: name → path (`/name` runs one).
    #[serde(default)]
    skills: HashMap<String, String>,
    #[serde(default)]
    skills_req: Option<u64>,
    /// The effort the running turn was started with (a steer at another
    /// effort waits for the next turn).
    #[serde(default)]
    effort: Option<String>,
    /// In-flight per-task stop requests: (rpc id, task id).
    #[serde(default)]
    task_stops: Vec<(u64, String)>,
}

impl Default for CodexDriver {
    fn default() -> Self {
        Self::with_program("codex")
    }
}

impl CodexDriver {
    pub fn with_program(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            next_id: 0,
            init_id: 0,
            thread_req: None,
            turn_req: None,
            steer_reqs: Vec::new(),
            ready: false,
            cwd: String::new(),
            model: None,
            resume: None,
            sandboxed: false,
            access: RuntimeAccessMode::default(),
            thread_id: None,
            turn_id: None,
            auto_turns: 0,
            queued: None,
            codex_turn: None,
            interrupt_when_started: false,
            items: HashMap::new(),
            pending: HashMap::new(),
            failure: None,
            mcp_failures: Vec::new(),
            subagents: HashMap::new(),
            child_thread_to_key: HashMap::new(),
            child_items: HashMap::new(),
            bg_tasks: Vec::new(),
            thread_model: None,
            plan: false,
            developer_instructions: None,
            skills: HashMap::new(),
            skills_req: None,
            effort: None,
            task_stops: Vec::new(),
        }
    }

    fn request(&mut self, method: &str, params: Value) -> (u64, String) {
        self.next_id += 1;
        let id = self.next_id;
        (
            id,
            json!({"id": id, "method": method, "params": params}).to_string(),
        )
    }

    fn turn(&self) -> String {
        self.turn_id.clone().unwrap_or_default()
    }

    fn ensure_autonomous_turn(&mut self, step: &mut DriverStep, native: Option<&str>) {
        if !self.ready || self.turn_id.is_some() {
            return;
        }
        self.auto_turns += 1;
        let id = native
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("auto-{}", self.auto_turns));
        self.turn_id = Some(id.clone());
        self.failure = None;
        self.items.clear();
        step.push(DriverEvent::TurnStarted { turn_id: id });
    }

    /// The thread-level sandbox mode (`SandboxMode`).
    fn sandbox_mode(&self) -> &'static str {
        match self.access {
            _ if self.sandboxed => "danger-full-access", // confined by Otto's Seatbelt instead
            RuntimeAccessMode::ApprovalRequired => "read-only",
            RuntimeAccessMode::AutoAcceptEdits => "workspace-write",
            RuntimeAccessMode::FullAccess => "danger-full-access",
        }
    }

    /// The turn-level `SandboxPolicy`.
    fn sandbox_policy(&self) -> Value {
        match self.access {
            _ if self.sandboxed => json!({"type": "externalSandbox", "networkAccess": "enabled"}),
            RuntimeAccessMode::ApprovalRequired => json!({"type": "readOnly"}),
            RuntimeAccessMode::AutoAcceptEdits => json!({"type": "workspaceWrite"}),
            RuntimeAccessMode::FullAccess => json!({"type": "dangerFullAccess"}),
        }
    }

    fn approval_policy(&self) -> &'static str {
        match self.access {
            RuntimeAccessMode::ApprovalRequired => "untrusted",
            RuntimeAccessMode::AutoAcceptEdits => "on-request",
            RuntimeAccessMode::FullAccess => "never",
        }
    }

    fn start_thread(&mut self, step: &mut DriverStep, resume: bool) {
        let mut params = json!({
            "cwd": self.cwd,
            "approvalPolicy": self.approval_policy(),
            "sandbox": self.sandbox_mode(),
        });
        if let Some(m) = &self.model {
            params["model"] = json!(m);
        }
        let (method, params) = match (&self.resume, resume) {
            (Some(tid), true) => {
                params["threadId"] = json!(tid);
                params["excludeTurns"] = json!(true);
                ("thread/resume", params)
            }
            _ => ("thread/start", params),
        };
        let (id, line) = self.request(method, params);
        self.thread_req = Some((id, resume && self.resume.is_some()));
        step.write(line);
    }

    /// A prompt's images then text as Codex `UserInput`s. A leading
    /// `/skill` names one of its skills (`{type: skill}`); a PDF goes as the
    /// path it was saved at (Codex's input has no document type — T3 Code's
    /// `providerMessageTextWithAttachmentPaths`).
    fn input(&self, req: &RuntimePromptReq) -> Vec<Value> {
        let mut input: Vec<Value> = req
            .images
            .iter()
            .map(|i| {
                json!({
                    "type": "image",
                    "url": format!("data:{};base64,{}", i.mime, i.data_base64),
                })
            })
            .collect();
        let mut text = req.text.clone();
        if let Some((name, rest)) = text
            .strip_prefix('/')
            .map(|t| t.split_once(char::is_whitespace).unwrap_or((t, "")))
        {
            if let Some(path) = self.skills.get(name) {
                input.push(json!({"type": "skill", "name": name, "path": path}));
                text = rest.trim_start().to_string();
            }
        }
        text = with_file_paths(&text, &req.files);
        if !text.is_empty() {
            input.push(json!({"type": "text", "text": text}));
        }
        input
    }

    /// `model` / `effort` / `access_mode` apply to this turn and the ones after it.
    fn send_turn(&mut self, step: &mut DriverStep, req: &RuntimePromptReq) {
        self.access = req.access_mode.unwrap_or(self.access);
        let mut params = json!({
            "threadId": self.thread_id,
            "input": self.input(req),
            "approvalPolicy": self.approval_policy(),
            "approvalsReviewer": "user",
            "sandboxPolicy": self.sandbox_policy(),
        });
        if let Some(m) = &req.model {
            params["model"] = json!(m);
        }
        if let Some(e) = &req.effort {
            params["effort"] = json!(e);
        }
        if req.effort.is_some() {
            self.effort = req.effort.clone();
        }
        // Plan mode is a collaboration mode, sent while it is on and once
        // more to leave it (Codex keeps the last one). Its settings name the
        // model; `null` instructions are the mode's built-in ones.
        if req.plan || self.plan {
            let model = req
                .model
                .clone()
                .or_else(|| self.model.clone())
                .or_else(|| self.thread_model.clone());
            if let Some(model) = model {
                let mut settings = json!({"model": model});
                if let Some(e) = &req.effort {
                    settings["reasoning_effort"] = json!(e);
                }
                if !req.plan {
                    settings["developer_instructions"] = json!(self.developer_instructions);
                }
                params["collaborationMode"] = json!({
                    "mode": if req.plan { "plan" } else { "default" },
                    "settings": settings,
                });
                self.plan = req.plan;
            }
        }
        if req.model.is_some() {
            self.thread_model = req.model.clone();
        }
        let (id, line) = self.request("turn/start", params);
        self.turn_req = Some(id);
        step.write(line);
    }

    /// `mcpServer/startupStatus/updated`: remember a failed server (once),
    /// forget one that recovered, and show what is pending when a turn runs.
    fn on_mcp_startup(&mut self, step: &mut DriverStep, params: &Value) {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return;
        };
        let thread = params.get("threadId").and_then(Value::as_str);
        if thread.is_none() || (self.thread_id.is_some() && thread != self.thread_id.as_deref()) {
            return;
        }
        self.mcp_failures.retain(|(n, _)| n != name);
        if params.get("status").and_then(Value::as_str) != Some("failed") {
            return;
        }
        let error = params
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("MCP server {name} failed to start."));
        self.mcp_failures.push((name.to_string(), error));
        self.flush_mcp_failures(step);
    }

    /// Each recorded failure as a notice in the running turn, then dropped.
    fn flush_mcp_failures(&mut self, step: &mut DriverStep) {
        if self.turn_id.is_none() {
            return;
        }
        for (name, error) in std::mem::take(&mut self.mcp_failures) {
            step.push(DriverEvent::TurnItem(item(
                format!("{}:mcp-startup-failed:{name}", self.turn()),
                TurnItemStatus::Completed,
                Some(format!("MCP server {name} failed to start")),
                TurnItemKind::SystemNotice {
                    message: error,
                    task_settlement: None,
                },
            )));
        }
    }

    pub(crate) fn item_id(&self, codex_id: &str) -> String {
        format!("{}:codex:{codex_id}", self.turn())
    }

    /// A Codex `ThreadItem` → an Otto item (`None` = not projected).
    fn project(&mut self, it: &Value, completed: bool) -> Option<TurnItem> {
        let prefix = format!("{}:codex", self.turn());
        project_codex_item(&mut self.items, &prefix, it, completed)
    }

    /// Grow a streamed text item by `delta`.
    fn on_text_delta(&mut self, step: &mut DriverStep, params: &Value, reasoning: bool) {
        let Some(cid) = params.get("itemId").and_then(Value::as_str) else {
            return;
        };
        let delta = params
            .get("delta")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let st = self.items.entry(cid.to_string()).or_default();
        let started_at = st.started_at;
        st.text.push_str(delta);
        let text = st.text.clone();
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
        let mut item = item(self.item_id(cid), TurnItemStatus::Running, None, kind);
        item.started_at = Some(started_at);
        step.push(DriverEvent::TurnItem(item));
    }

    fn on_plan(&self, step: &mut DriverStep, params: &Value) {
        let steps: Vec<PlanStep> = params
            .get("plan")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .enumerate()
                    .map(|(i, p)| PlanStep {
                        id: format!("step-{}", i + 1),
                        text: p
                            .get("step")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        status: match p.get("status").and_then(Value::as_str) {
                            Some("inProgress") => PlanStepStatus::Running,
                            Some("completed") => PlanStepStatus::Completed,
                            _ => PlanStepStatus::Pending,
                        },
                    })
                    .collect()
            })
            .unwrap_or_default();
        let explanation = params
            .get("explanation")
            .and_then(Value::as_str)
            .map(str::to_string);
        step.push(DriverEvent::Plan { steps, explanation });
    }

    fn on_usage(&self, step: &mut DriverStep, params: &Value) {
        let Some(u) = params.get("tokenUsage") else {
            return;
        };
        let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64);
        if let Some(last) = u.get("last") {
            step.push(DriverEvent::Usage(TurnTokenUsage {
                cost_usd: None,
                input_tokens: n(last, "inputTokens").unwrap_or(0),
                output_tokens: n(last, "outputTokens").unwrap_or(0),
                cached_input_tokens: n(last, "cachedInputTokens").filter(|t| *t > 0),
                reasoning_tokens: n(last, "reasoningOutputTokens").filter(|t| *t > 0),
            }));
        }
        if let Some(total) = u.get("total") {
            step.push(DriverEvent::ContextUsage(ThreadTokenUsage {
                used_tokens: n(total, "totalTokens").unwrap_or(0),
                max_tokens: u.get("modelContextWindow").and_then(Value::as_u64),
                input_tokens: n(total, "inputTokens"),
                cached_input_tokens: n(total, "cachedInputTokens"),
                output_tokens: n(total, "outputTokens"),
            }));
        }
    }

    fn on_turn_completed(&mut self, step: &mut DriverStep, params: &Value) {
        let turn = params.get("turn").cloned().unwrap_or(Value::Null);
        let status = match turn.get("status").and_then(Value::as_str) {
            Some("completed") => TurnTerminalStatus::Completed,
            Some("interrupted") => TurnTerminalStatus::Interrupted,
            Some("failed") => TurnTerminalStatus::Failed,
            _ => return, // inProgress: not an end
        };
        let failure = (status == TurnTerminalStatus::Failed).then(|| {
            turn.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(|m| ProviderFailure {
                    class: "provider_error".into(),
                    message: m.to_string(),
                    code: None,
                    retryable: None,
                })
                .or_else(|| self.failure.clone())
                .unwrap_or(ProviderFailure {
                    class: "provider_error".into(),
                    message: "the Codex turn failed".into(),
                    code: None,
                    retryable: None,
                })
        });
        if matches!(
            status,
            TurnTerminalStatus::Failed | TurnTerminalStatus::Interrupted
        ) {
            self.settle_open_subagents_on_parent_abort(step, status);
        }
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
        self.turn_id = None;
        self.codex_turn = None;
        self.turn_req = None;
        self.steer_reqs.clear();
        self.interrupt_when_started = false;
        self.failure = None;
        self.items.clear();
    }

    fn on_server_request(
        &mut self,
        step: &mut DriverStep,
        rpc_id: Value,
        method: &str,
        params: Value,
    ) {
        if method == "item/tool/requestUserInput" {
            match crate::questions::native_questions(&params, false) {
                Ok(questions) => {
                    let id = format!("codex-{}", rpc_key(&rpc_id));
                    self.pending.insert(
                        id.clone(),
                        Pending {
                            rpc_id,
                            proto: ReqProto::Question,
                            permissions: params.clone(),
                        },
                    );
                    let req_tid = params
                        .get("threadId")
                        .or_else(|| params.get("thread_id"))
                        .and_then(Value::as_str);
                    step.push(DriverEvent::Question {
                        id,
                        title: None,
                        questions,
                        tool_item_id: params
                            .get("itemId")
                            .and_then(Value::as_str)
                            .map(|i| self.item_id_for_thread(req_tid, i)),
                    });
                }
                Err(message) => step.write(
                    json!({"id":rpc_id,"error":{"code":-32602,"message":message}}).to_string(),
                ),
            }
            return;
        }
        let (proto, kind) = match method {
            "item/commandExecution/requestApproval" => (ReqProto::V2Decision, RequestKind::Command),
            "item/fileChange/requestApproval" => (ReqProto::V2Decision, RequestKind::FileChange),
            "item/permissions/requestApproval" => (ReqProto::Permissions, RequestKind::Permission),
            _ => {
                // Elicitations, dynamic tool calls, auth
                // refresh: not offered by Otto's client.
                step.write(
                    json!({"id": rpc_id, "error": {"code": -32601, "message": format!("method not supported: {method}")}})
                        .to_string(),
                );
                return;
            }
        };
        let request_id = format!("codex-{}", rpc_key(&rpc_id));
        let s = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
        let command = match params.get("command") {
            Some(Value::String(c)) => Some(c.clone()),
            Some(Value::Array(a)) => Some(
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        };
        let (title, prompt) = match kind {
            RequestKind::Command => ("Run command".to_string(), command),
            RequestKind::FileChange => (
                "Apply file changes".to_string(),
                s("grantRoot").map(|r| format!("write access under {r}")),
            ),
            _ => ("Grant permissions".to_string(), None),
        };
        let mut options = vec![ApprovalOption {
            decision: ApprovalDecision::Accept,
            label: "Approve".into(),
        }];
        options.push(ApprovalOption {
            decision: ApprovalDecision::AcceptForSession,
            label: "Approve for this session".into(),
        });
        options.push(ApprovalOption {
            decision: ApprovalDecision::Decline,
            label: "Decline".into(),
        });
        self.pending.insert(
            request_id.clone(),
            Pending {
                rpc_id,
                proto,
                permissions: params.get("permissions").cloned().unwrap_or(json!({})),
            },
        );
        let req_tid = params
            .get("threadId")
            .or_else(|| params.get("thread_id"))
            .and_then(Value::as_str);
        step.push(DriverEvent::Request {
            id: request_id,
            kind,
            title: Some(title),
            prompt,
            reason: s("reason"),
            options,
            tool_item_id: s("itemId").map(|i| self.item_id_for_thread(req_tid, &i)),
        });
    }

    fn answer(p: &Pending, decision: ApprovalDecision) -> String {
        let result = match p.proto {
            ReqProto::Question => json!({"answers": {}}),
            ReqProto::V2Decision => json!({"decision": match decision {
                ApprovalDecision::Accept => "accept",
                ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => "acceptForSession",
                ApprovalDecision::Decline => "decline",
                ApprovalDecision::Cancel => "cancel",
            }}),
            ReqProto::Permissions => match decision {
                ApprovalDecision::Accept => json!({"permissions": p.permissions, "scope": "turn"}),
                ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => {
                    json!({"permissions": p.permissions, "scope": "session"})
                }
                ApprovalDecision::Decline | ApprovalDecision::Cancel => {
                    json!({"permissions": {}, "scope": "turn"})
                }
            },
        };
        json!({"id": p.rpc_id, "result": result}).to_string()
    }

    fn on_response(
        &mut self,
        step: &mut DriverStep,
        id: u64,
        result: Option<Value>,
        error: Option<Value>,
    ) {
        if id == self.init_id && !self.ready && self.thread_req.is_none() {
            if let Some(e) = error {
                step.push(DriverEvent::Fatal(rpc_failure("handshake_error", &e)));
                return;
            }
            step.write(json!({"method": "initialized"}).to_string());
            self.start_thread(step, true);
        } else if let Some((_, resuming)) = self.thread_req.filter(|(t, _)| *t == id) {
            self.thread_req = None;
            if let Some(e) = error {
                if resuming && thread_in_use(&e) {
                    // The thread is open in another Codex client (the CLI):
                    // a fresh thread would silently fork the conversation —
                    // refuse with a readable reason instead (T3 Code
                    // `ThreadInUseError`, a445272c78). The next prompt retries.
                    step.push(DriverEvent::Fatal(
                        otto_core::agent_runtime::ProviderFailure {
                            class: "thread_in_use".into(),
                            message:
                                "this Codex thread is open in another Codex client — close it \
                                  there, then send again"
                                    .into(),
                            code: e.get("code").map(|c| c.to_string()),
                            retryable: Some(true),
                        },
                    ));
                    return;
                }
                if resuming {
                    // A thread Codex cannot resume: start a fresh one.
                    self.resume = None;
                    self.start_thread(step, false);
                    return;
                }
                step.push(DriverEvent::Fatal(rpc_failure("handshake_error", &e)));
                return;
            }
            let r = result.unwrap_or(Value::Null);
            self.thread_id = r
                .pointer("/thread/id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| self.resume.clone());
            self.ready = true;
            if let Some(m) = r.get("model").and_then(Value::as_str) {
                self.thread_model = Some(m.to_string());
            }
            step.push(DriverEvent::Ready {
                native_thread_id: self.thread_id.clone(),
                model: r.get("model").and_then(Value::as_str).map(str::to_string),
            });
            // Its skills are its slash commands (`/name` → `{type: skill}`).
            let (id, line) = self.request("skills/list", json!({"cwds": [self.cwd]}));
            self.skills_req = Some(id);
            step.write(line);
            if let Some(req) = self.queued.take() {
                self.send_turn(step, &req);
            }
        } else if Some(id) == self.skills_req {
            self.skills_req = None;
            let skills = result
                .as_ref()
                .and_then(|r| r.get("data"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .flat_map(|e| {
                    e.get("skills")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter(|s| s.get("enabled").and_then(Value::as_bool) != Some(false))
                .filter_map(|s| {
                    let name = s.get("name")?.as_str()?.to_string();
                    let path = s.get("path")?.as_str()?.to_string();
                    let desc = s
                        .pointer("/interface/shortDescription")
                        .or_else(|| s.get("shortDescription"))
                        .or_else(|| s.get("description"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    Some((name, path, desc))
                })
                .collect::<Vec<_>>();
            self.skills = skills
                .iter()
                .map(|(n, p, _)| (n.clone(), p.clone()))
                .collect();
            step.push(DriverEvent::Commands(
                skills
                    .into_iter()
                    .map(|(n, _, d)| ProviderCommand::new(n, d))
                    .collect(),
            ));
        } else if Some(id) == self.turn_req {
            if let Some(e) = error {
                self.turn_req = None;
                step.push(DriverEvent::Terminal {
                    status: TurnTerminalStatus::Failed,
                    failure: Some(rpc_failure("provider_error", &e)),
                    reason: None,
                });
                self.turn_id = None;
                return;
            }
            if let Some(t) = result
                .as_ref()
                .and_then(|r| r.pointer("/turn/id"))
                .and_then(Value::as_str)
            {
                self.codex_turn = Some(t.to_string());
            }
            self.maybe_interrupt(step);
        } else if let Some(pos) = self.steer_reqs.iter().position(|r| *r == id) {
            self.steer_reqs.swap_remove(pos);
            if let Some(e) = error {
                step.push(DriverEvent::Error(rpc_failure("provider_error", &e)));
            }
        } else if let Some(pos) = self.task_stops.iter().position(|(r, _)| *r == id) {
            let (_, task_id) = self.task_stops.swap_remove(pos);
            // The child's own `turn/completed` settles its card.
            let error = error.map(|e| rpc_failure("provider_error", &e).message);
            step.push(DriverEvent::TaskStopAnswered { task_id, error });
        }
    }

    /// `turn/interrupt` on every spawned agent's running turn (Codex does
    /// not promise to cascade a parent's interrupt to its child threads).
    fn interrupt_children(&mut self, step: &mut DriverStep) {
        let mut running: Vec<(String, String)> = self
            .subagents
            .values()
            .filter(|c| c.status.is_open() && !c.settled)
            .filter_map(|c| Some((c.child_thread_id.clone()?, c.child_turn_id.clone()?)))
            .collect();
        running.sort();
        for (thread, turn) in running {
            let (_, line) = self.request(
                "turn/interrupt",
                json!({"threadId": thread, "turnId": turn}),
            );
            step.write(line);
        }
    }

    fn maybe_interrupt(&mut self, step: &mut DriverStep) {
        if !self.interrupt_when_started {
            return;
        }
        if let (Some(thread), Some(turn)) = (&self.thread_id, &self.codex_turn) {
            let params = json!({"threadId": thread, "turnId": turn});
            let (_, line) = self.request("turn/interrupt", params);
            step.write(line);
            self.interrupt_when_started = false;
        }
    }
}

/// Project a Codex `ThreadItem` into an Otto [`TurnItem`], sharing accumulated
/// stream state in `items` and formatting its ID as `{id_prefix}:{codex_id}`.
pub(crate) fn project_codex_item(
    items: &mut HashMap<String, Item>,
    id_prefix: &str,
    it: &Value,
    completed: bool,
) -> Option<TurnItem> {
    let cid = it.get("id").and_then(Value::as_str)?.to_string();
    let ty = it.get("type").and_then(Value::as_str)?;
    let empty_text = it
        .get("text")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty);
    let streamed = items.get(&cid).is_some_and(|i| !i.text.is_empty());
    let summary = it
        .get("summary")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    if matches!(ty, "agentMessage" | "reasoning") && empty_text && !streamed && !summary {
        // Shown from its first delta, never as an empty bubble; record its
        // start timestamp now so summary-only reasoning keeps real duration.
        if completed {
            items.remove(&cid);
        } else {
            items.entry(cid).or_default();
        }
        return None;
    }
    let st = items.entry(cid.clone()).or_default();
    let started_at = st.started_at;
    let s = |k: &str| it.get(k).and_then(Value::as_str).map(str::to_string);
    let status = match it.get("status").and_then(Value::as_str) {
        Some("inProgress") => TurnItemStatus::Running,
        Some("completed") => TurnItemStatus::Completed,
        Some("failed") => TurnItemStatus::Failed,
        Some("declined") => TurnItemStatus::Cancelled,
        _ if completed => TurnItemStatus::Completed,
        _ => TurnItemStatus::Running,
    };
    let (kind, title) = match ty {
        "agentMessage" => {
            if let Some(t) = s("text").filter(|t| !t.is_empty() || completed) {
                st.text = t;
            }
            (
                TurnItemKind::AssistantMessage {
                    message_id: cid.clone(),
                    text: st.text.clone(),
                    streaming: !completed,
                },
                None,
            )
        }
        "reasoning" => {
            let joined = |k: &str| {
                it.get(k)
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join("\n\n")
                    })
                    .filter(|t| !t.is_empty())
            };
            if let Some(t) = joined("summary").or_else(|| joined("content")) {
                st.text = t;
            }
            (
                TurnItemKind::Reasoning {
                    text: st.text.clone(),
                    streaming: !completed,
                },
                None,
            )
        }
        "commandExecution" => {
            if let Some(o) = s("aggregatedOutput") {
                st.output = o;
            }
            (
                TurnItemKind::CommandExecution {
                    input: s("command").unwrap_or_default(),
                    output: (!st.output.is_empty()).then(|| st.output.clone()),
                    exit_code: it.get("exitCode").and_then(Value::as_i64).map(|c| c as i32),
                },
                Some("Command".to_string()),
            )
        }
        "fileChange" => {
            let changes = it
                .get("changes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let paths: Vec<&str> = changes
                .iter()
                .filter_map(|c| c.get("path").and_then(Value::as_str))
                .collect();
            let diff: Vec<&str> = changes
                .iter()
                .filter_map(|c| c.get("diff").and_then(Value::as_str))
                .collect();
            let file_name = match paths.len() {
                0 => "files".to_string(),
                1 => paths[0].to_string(),
                n => format!("{} (+{} more)", paths[0], n - 1),
            };
            (
                TurnItemKind::FileChange {
                    file_name,
                    diff_str: (!diff.is_empty()).then(|| diff.join("\n")),
                    old_str: None,
                    new_str: None,
                },
                Some("Edit".to_string()),
            )
        }
        "mcpToolCall" | "dynamicToolCall" => {
            let tool = s("tool").unwrap_or_else(|| "tool".into());
            let name = match s("server") {
                Some(server) => format!("{server}.{tool}"),
                None => tool,
            };
            let result = crate::tool_images::extract(
                it.get("result")
                    .filter(|v| !v.is_null())
                    .or_else(|| it.get("error").filter(|v| !v.is_null())),
            );
            let output = (!result.text.is_empty()).then_some(result.text);
            (
                TurnItemKind::DynamicTool {
                    tool_name: name.clone(),
                    input: it.get("arguments").cloned().unwrap_or(Value::Null),
                    output,
                    images: result.images,
                    image_files: Vec::new(),
                },
                Some(name),
            )
        }
        "webSearch" => (
            TurnItemKind::WebSearch {
                patterns: s("query").into_iter().collect(),
            },
            Some("Web search".to_string()),
        ),
        "contextCompaction" => (
            TurnItemKind::SystemNotice {
                message: "Context compacted".into(),
                task_settlement: None,
            },
            None,
        ),
        // Images Codex looked at or made exist only as files: the item
        // names them and the runtime serves them read-only.
        "imageView" => (
            TurnItemKind::DynamicTool {
                tool_name: "view_image".into(),
                input: json!({"path": s("path")}),
                output: None,
                images: Vec::new(),
                image_files: s("path").into_iter().collect(),
            },
            Some("View image".to_string()),
        ),
        "imageGeneration" => {
            let failure = it.get("failure").filter(|f| !f.is_null()).map(|f| {
                f.get("message")
                    .and_then(Value::as_str)
                    .map_or_else(|| f.to_string(), str::to_string)
            });
            (
                TurnItemKind::DynamicTool {
                    tool_name: "image_generation".into(),
                    input: json!({"prompt": s("revisedPrompt")}),
                    output: failure,
                    images: Vec::new(),
                    image_files: s("savedPath").into_iter().collect(),
                },
                Some("Generate image".to_string()),
            )
        }
        _ => return None,
    };
    let status = match kind {
        TurnItemKind::AssistantMessage { .. } | TurnItemKind::Reasoning { .. } => {
            if completed {
                TurnItemStatus::Completed
            } else {
                TurnItemStatus::Running
            }
        }
        _ => status,
    };
    let error = (status == TurnItemStatus::Failed)
        .then(|| match it.get("error") {
            Some(Value::String(e)) => e.clone(),
            Some(e @ Value::Object(_)) => e
                .get("message")
                .and_then(Value::as_str)
                .map_or_else(|| e.to_string(), str::to_string),
            _ => st.output.clone(),
        })
        .and_then(|e| tool_error(&e));
    if completed {
        items.remove(&cid);
    }
    let mut out = item(format!("{id_prefix}:{cid}"), status, title, kind);
    out.started_at = Some(started_at);
    out.error = error;
    Some(out)
}

/// Codex's "another client owns this thread" resume refusal (`-32600`,
/// "… already has an active writer").
fn thread_in_use(e: &Value) -> bool {
    e.get("code").and_then(Value::as_i64) == Some(-32600)
        && e.get("message")
            .and_then(Value::as_str)
            .is_some_and(|m| m.contains("active writer"))
}

impl StructuredDriver for CodexDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Codex
    }

    fn launch(&self, _ctx: &LaunchContext) -> LaunchSpec {
        // The model goes in `thread/start` (`codexThreadRuntimeParams`). The
        // context hook's flags (`--add-dir=`, `-c …`) are top-level codex
        // options `app-server` rejects after it, so the subcommand is the tail.
        LaunchSpec {
            program: self.program.clone(),
            args: Vec::new(),
            tail_args: vec!["app-server".into()],
            env: Vec::new(),
        }
    }

    /// Otto's context rides `-c developer_instructions=…`; a plan-mode turn
    /// replaces the thread's instructions, so leaving it hands these back.
    fn adopt_spec(&mut self, spec: &mut LaunchSpec) {
        self.developer_instructions = spec
            .args
            .windows(2)
            .find(|w| w[0] == "-c" && w[1].starts_with("developer_instructions="))
            .map(|w| w[1]["developer_instructions=".len()..].to_string());
    }

    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        self.cwd = ctx.cwd.clone();
        self.model = ctx.model.clone();
        self.resume = ctx.resume.clone();
        self.sandboxed = ctx.sandboxed;
        self.access = ctx.access_mode;
        let mut step = DriverStep::default();
        let (id, line) = self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "otto", "title": "Otto", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {
                    "experimentalApi": true,
                    "optOutNotificationMethods": ["turn/diff/updated"],
                },
            }),
        );
        self.init_id = id;
        step.write(line);
        step
    }

    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        if req.plan
            && !self.plan
            && self.thread_model.is_none()
            && self.model.is_none()
            && self.ready
        {
            return Err(DriverError(
                "Codex has not named its model yet; send again in a moment".into(),
            ));
        }
        self.turn_id = Some(turn_id.to_string());
        self.failure = None;
        let mut step = DriverStep::default();
        let (req, dropped) = drop_unsupported_effort(
            RuntimeProvider::Codex,
            &RuntimeCapabilities::for_provider(RuntimeProvider::Codex).efforts,
            req,
        );
        if let Some(n) = dropped {
            step.push(DriverEvent::TurnItem(n));
        }
        self.flush_mcp_failures(&mut step);
        if self.ready {
            self.send_turn(&mut step, &req);
        } else {
            self.queued = Some(req);
        }
        Ok(step)
    }

    /// `turn/steer {threadId, expectedTurnId, input}` (app-server v2): the
    /// input joins the running turn at its next step. Only once Codex named
    /// the turn (`turn/start`'s answer or `turn/started`); before that the
    /// message stays queued.
    fn steer(&mut self, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if !self.ready || self.turn_id.is_none() || self.interrupt_when_started {
            return Err(DriverError("no turn is running".into()));
        }
        let Some(thread_id) = self.thread_id.clone() else {
            return Err(DriverError("no thread is open".into()));
        };
        let Some(expected_turn_id) = self.codex_turn.clone() else {
            return Err(DriverError("turn has not started on the server yet".into()));
        };
        // `turn/steer` carries no effort: a message at another effort waits
        // for the next turn (the runtime queues it) instead of running at
        // the old one.
        if req.effort.is_some() && req.effort != self.effort {
            return Err(DriverError(
                "a different reasoning effort applies from the next turn".into(),
            ));
        }
        let input = self.input(req);
        if input.is_empty() {
            return Err(DriverError("empty steer input".into()));
        }
        let mut step = DriverStep::default();
        let (id, line) = self.request(
            "turn/steer",
            json!({
                "threadId": thread_id,
                "input": input,
                "expectedTurnId": expected_turn_id,
            }),
        );
        self.steer_reqs.push(id);
        step.write(line);
        Ok(step)
    }

    fn switches_model_live(&self) -> bool {
        true
    }

    /// The policy rides every `turn/start`.
    fn switches_access_live(&self, _to: RuntimeAccessMode) -> bool {
        true
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        let mut open: Vec<(String, Pending)> = self.pending.drain().collect();
        open.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, p) in open {
            step.write(Self::answer(&p, ApprovalDecision::Cancel));
        }
        if self.queued.take().is_some()
            || (self.turn_req.is_none() && self.codex_turn.is_none() && self.turn_id.is_some())
        {
            self.turn_id = None;
            self.items.clear();
            step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                failure: None,
                reason: None,
            });
            return step;
        }
        self.interrupt_when_started = true;
        self.maybe_interrupt(&mut step);
        self.interrupt_children(&mut step);
        step
    }

    /// A spawned agent's running turn: `turn/interrupt {threadId, turnId}`
    /// on its child thread (shape checked against codex-cli 0.169.0's
    /// app-server schema). Unknown task, or a child whose turn Codex has
    /// not named: unsupported, so the runtime asks the agent.
    fn stop_task(&mut self, task_id: &str) -> TaskStop {
        let Some((thread, turn)) = self
            .subagents
            .values()
            .find(|c| c.bg_id() == task_id && c.status.is_open() && !c.settled)
            .and_then(|c| Some((c.child_thread_id.clone()?, c.child_turn_id.clone()?)))
        else {
            return TaskStop::Unsupported;
        };
        let (id, line) = self.request(
            "turn/interrupt",
            json!({"threadId": thread, "turnId": turn}),
        );
        self.task_stops.push((id, task_id.to_string()));
        let mut step = DriverStep::default();
        step.write(line);
        TaskStop::Native(step)
    }

    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        if self
            .pending
            .get(request_id)
            .is_some_and(|p| p.proto == ReqProto::Question)
        {
            return Err(DriverError("this request needs question answers".into()));
        }
        let p = self
            .pending
            .remove(request_id)
            .ok_or_else(|| DriverError(format!("unknown request {request_id}")))?;
        let mut step = DriverStep::default();
        step.write(Self::answer(&p, decision));
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
            .filter(|p| p.proto == ReqProto::Question)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let questions =
            crate::questions::native_questions(&p.permissions, false).map_err(DriverError)?;
        otto_core::agent_runtime::validate_question_answers(&questions, answers)
            .map_err(DriverError)?;
        let values: serde_json::Map<String, Value> = answers
            .iter()
            .map(|a| (a.question_id().to_string(), json!({"answers":a.values()})))
            .collect();
        let mut step = DriverStep::default();
        step.write(json!({"id":p.rpc_id,"result":{"answers":values}}).to_string());
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
            .filter(|p| p.proto == ReqProto::Question)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let mut step = DriverStep::default();
        step.write(json!({"id":p.rpc_id,"result":{"answers":{}}}).to_string());
        self.pending.remove(request_id);
        Ok(step)
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            step.push(DriverEvent::Error(ProviderFailure {
                class: "protocol_error".into(),
                message: format!(
                    "unparseable app-server output: {}",
                    line.chars().take(200).collect::<String>()
                ),
                code: None,
                retryable: None,
            }));
            return step;
        };
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = msg.get("id").cloned().filter(|v| !v.is_null());
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some(m), Some(id)) => self.on_server_request(&mut step, id, &m, params),
            (None, Some(id)) => {
                if let Some(n) = id.as_u64() {
                    let result = msg.get("result").cloned();
                    let error = msg.get("error").cloned();
                    self.on_response(&mut step, n, result, error);
                }
            }
            (Some(m), None) => {
                let notif_tid = params
                    .get("threadId")
                    .or_else(|| params.get("thread_id"))
                    .and_then(Value::as_str);
                if let Some(child_tid) = notif_tid.filter(|t| self.is_child_thread(t)) {
                    match m.as_str() {
                        "mcpServer/startupStatus/updated" => {
                            self.on_mcp_startup(&mut step, &params);
                        }
                        "serverRequest/resolved" => {
                            if let Some(rid) = params.get("requestId") {
                                let request_id = format!("codex-{}", rpc_key(rid));
                                if self.pending.remove(&request_id).is_some() {
                                    step.push(DriverEvent::RequestClosed {
                                        request_id,
                                        status: RuntimeRequestStatus::Cancelled,
                                    });
                                }
                            }
                        }
                        _ => self.on_child_notification(&mut step, child_tid, &m, &params),
                    }
                    return step;
                }
                match m.as_str() {
                    "turn/started" => {
                        let native = params.pointer("/turn/id").and_then(Value::as_str);
                        self.ensure_autonomous_turn(&mut step, native);
                        if let Some(t) = native {
                            self.codex_turn = Some(t.to_string());
                        }
                        self.maybe_interrupt(&mut step);
                    }
                    "item/started" | "item/completed" => {
                        self.ensure_autonomous_turn(&mut step, None);
                        if self.turn_id.is_some() {
                            if let Some(item) = params.get("item") {
                                let ty = item.get("type").and_then(Value::as_str).unwrap_or("");
                                if matches!(ty, "collabAgentToolCall" | "collabToolCall") {
                                    self.on_collab_item(
                                        &mut step,
                                        item,
                                        m == "item/completed",
                                        None,
                                    );
                                } else if let Some(i) = self.project(item, m == "item/completed") {
                                    step.push(DriverEvent::TurnItem(i));
                                }
                            }
                        }
                    }
                    "subAgentActivity" | "subagent/activity" => {
                        self.on_subagent_activity(&mut step, &params);
                    }
                    "item/agentMessage/delta" => {
                        self.ensure_autonomous_turn(&mut step, None);
                        if self.turn_id.is_some() {
                            self.on_text_delta(&mut step, &params, false);
                        }
                    }
                    "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                        self.ensure_autonomous_turn(&mut step, None);
                        if self.turn_id.is_some() {
                            self.on_text_delta(&mut step, &params, true);
                        }
                    }
                    "item/commandExecution/outputDelta" => {
                        self.ensure_autonomous_turn(&mut step, None);
                        if self.turn_id.is_some() {
                            if let (Some(cid), Some(d)) = (
                                params.get("itemId").and_then(Value::as_str),
                                params.get("delta").and_then(Value::as_str),
                            ) {
                                self.items
                                    .entry(cid.to_string())
                                    .or_default()
                                    .output
                                    .push_str(d);
                                step.push(DriverEvent::CommandOutput {
                                    item_id: self.item_id(cid),
                                    chunk: d.to_string(),
                                });
                            }
                        }
                    }
                    "turn/plan/updated" => {
                        self.ensure_autonomous_turn(&mut step, None);
                        if self.turn_id.is_some() {
                            self.on_plan(&mut step, &params);
                        }
                    }
                    "thread/tokenUsage/updated" => self.on_usage(&mut step, &params),
                    "thread/name/updated" => {
                        if let Some(name) = params.get("threadName").and_then(Value::as_str) {
                            step.push(DriverEvent::Title(name.to_string()));
                        }
                    }
                    "skills/changed" => {
                        let (id, line) = self.request(
                            "skills/list",
                            json!({"cwds": [self.cwd], "forceReload": true}),
                        );
                        self.skills_req = Some(id);
                        step.write(line);
                    }
                    "mcpServer/startupStatus/updated" => self.on_mcp_startup(&mut step, &params),
                    "error" => {
                        if params.get("willRetry").and_then(Value::as_bool) != Some(true) {
                            if let Some(e) = params.get("error") {
                                self.failure = Some(rpc_failure("provider_error", e));
                            }
                        } else if self.turn_id.is_some() {
                            // T3 Code #29208: a retry shows as the turn's retry row.
                            let why = params.pointer("/error/message").and_then(Value::as_str);
                            step.push(DriverEvent::TurnItem(crate::driver::retry_notice(
                                &self.turn(),
                                None,
                                None,
                                why,
                            )));
                        }
                    }
                    "serverRequest/resolved" => {
                        if let Some(rid) = params.get("requestId") {
                            let request_id = format!("codex-{}", rpc_key(rid));
                            if self.pending.remove(&request_id).is_some() {
                                step.push(DriverEvent::RequestClosed {
                                    request_id,
                                    status: RuntimeRequestStatus::Cancelled,
                                });
                            }
                        }
                    }
                    "turn/completed" if self.turn_id.is_some() => {
                        self.on_turn_completed(&mut step, &params)
                    }
                    _ => {}
                }
            }
            (None, None) => {}
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        self.steer_reqs.clear();
        self.pending.clear();
        self.items.clear();
        self.subagents.clear();
        self.child_thread_to_key.clear();
        self.child_items.clear();
        self.bg_tasks.clear();
        DriverStep::default()
    }

    fn snapshot_state(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn restore_state(&mut self, state: &Value) -> Result<(), DriverError> {
        restore_keeping_program(self, state, |d| &mut d.program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(resume: Option<&str>, sandboxed: bool) -> LaunchContext {
        LaunchContext {
            mcp_servers: Vec::new(),
            session_id: "s".into(),
            cwd: "/w".into(),
            model: None,
            resume: resume.map(str::to_string),
            sandboxed,
            access_mode: RuntimeAccessMode::default(),
        }
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn feed(d: &mut CodexDriver, v: Value) -> DriverStep {
        d.on_line(&v.to_string())
    }

    fn ready(d: &mut CodexDriver, sandboxed: bool) {
        let s = d.start(&ctx(None, sandboxed));
        let init = parse(&s.writes[0]);
        assert_eq!(init["method"], "initialize");
        assert!(
            init.get("jsonrpc").is_none(),
            "app-server framing has no jsonrpc"
        );
        let s = feed(d, json!({"id": 1, "result": {"userAgent": "otto/0.166.0"}}));
        assert_eq!(parse(&s.writes[0])["method"], "initialized");
        let start = parse(&s.writes[1]);
        assert_eq!(start["method"], "thread/start");
        assert_eq!(start["params"]["approvalPolicy"], "untrusted");
        let s = feed(
            d,
            json!({"id": start["id"], "result": {"thread": {"id": "th1"}, "model": "gpt"}}),
        );
        assert!(matches!(
            &s.events[0],
            DriverEvent::Ready { native_thread_id: Some(t), .. } if t == "th1"
        ));
    }

    #[test]
    fn sandbox_policy_follows_otto_seatbelt() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, true);
        let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
        let turn = parse(&s.writes[0]);
        assert_eq!(turn["params"]["sandboxPolicy"]["type"], "externalSandbox");
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
        assert_eq!(
            parse(&s.writes[0])["params"]["sandboxPolicy"]["type"],
            "readOnly"
        );
    }

    /// A thread another Codex client is writing is refused, not forked.
    #[test]
    fn resume_of_a_thread_in_use_is_refused_not_forked() {
        let mut d = CodexDriver::with_program("codex");
        let _ = d.start(&ctx(Some("old"), false));
        let s = feed(&mut d, json!({"id": 1, "result": {}}));
        let resume = parse(&s.writes[1]);
        let s = feed(
            &mut d,
            json!({"id": resume["id"], "error": {"code": -32600, "message": "thread old already has an active writer"}}),
        );
        assert!(s.writes.is_empty(), "no fresh thread/start");
        assert!(matches!(&s.events[0], DriverEvent::Fatal(f) if f.class == "thread_in_use"));
    }

    #[test]
    fn resume_falls_back_to_a_fresh_thread() {
        let mut d = CodexDriver::with_program("codex");
        let _ = d.start(&ctx(Some("old"), false));
        let s = feed(&mut d, json!({"id": 1, "result": {}}));
        let resume = parse(&s.writes[1]);
        assert_eq!(resume["method"], "thread/resume");
        assert_eq!(resume["params"]["threadId"], "old");
        assert_eq!(resume["params"]["excludeTurns"], true);
        let s = feed(
            &mut d,
            json!({"id": resume["id"], "error": {"code": -32600, "message": "no rollout"}}),
        );
        assert_eq!(parse(&s.writes[0])["method"], "thread/start");
    }

    #[test]
    fn deltas_items_approval_and_turn_end() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let turn_req = parse(&s.writes[0]);
        feed(
            &mut d,
            json!({"id": turn_req["id"], "result": {"turn": {"id": "ct1", "items": [], "status": "inProgress"}}}),
        );
        feed(
            &mut d,
            json!({"method": "item/started", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "agentMessage", "id": "a1", "text": ""}}}),
        );
        let s = feed(
            &mut d,
            json!({"method": "item/agentMessage/delta", "params": {"threadId": "th1", "turnId": "ct1", "itemId": "a1", "delta": "he"}}),
        );
        let s2 = feed(
            &mut d,
            json!({"method": "item/agentMessage/delta", "params": {"threadId": "th1", "turnId": "ct1", "itemId": "a1", "delta": "y"}}),
        );
        let DriverEvent::TurnItem(i) = &s2.events[0] else {
            panic!()
        };
        assert!(
            matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "hey")
        );
        drop(s);
        // A command approval.
        let s = feed(
            &mut d,
            json!({"id": 7, "method": "item/commandExecution/requestApproval", "params": {"threadId": "th1", "turnId": "ct1", "itemId": "c1", "startedAtMs": 0, "command": "rm -rf build", "reason": "cleanup"}}),
        );
        let DriverEvent::Request {
            id, prompt, reason, ..
        } = &s.events[0]
        else {
            panic!()
        };
        assert_eq!(id, "codex-7");
        assert_eq!(prompt.as_deref(), Some("rm -rf build"));
        assert_eq!(reason.as_deref(), Some("cleanup"));
        let s = d
            .respond("codex-7", ApprovalDecision::AcceptForSession)
            .unwrap();
        let r = parse(&s.writes[0]);
        assert_eq!(r["id"], 7);
        assert_eq!(r["result"]["decision"], "acceptForSession");
        // Output streams; completion settles.
        let s = feed(
            &mut d,
            json!({"method": "item/commandExecution/outputDelta", "params": {"threadId": "th1", "turnId": "ct1", "itemId": "c1", "delta": "ok\n"}}),
        );
        assert!(
            matches!(&s.events[0], DriverEvent::CommandOutput { chunk, .. } if chunk == "ok\n")
        );
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "commandExecution", "id": "c1", "command": "rm -rf build", "status": "completed", "exitCode": 0, "cwd": "/w", "commandActions": []}}}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(
            matches!(&i.kind, TurnItemKind::CommandExecution { exit_code: Some(0), output: Some(o), .. } if o == "ok\n")
        );
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "agentMessage", "id": "a1", "text": "hey"}}}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(!i.is_streaming() && i.status == TurnItemStatus::Completed);
        let s = feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "ct1", "items": [], "status": "completed"}}}),
        );
        assert!(matches!(
            s.events[0],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        ));
    }

    /// T3 Code #29208: `error{willRetry}` is the turn's retry row, not a
    /// silent drop; a final error still fails the turn.
    #[test]
    fn a_retrying_error_is_the_turns_retry_row() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = feed(
            &mut d,
            json!({"method":"error","params":{"error":{"message":"Reconnecting... 2/5"},"willRetry":true,"threadId":"th","turnId":"u1"}}),
        );
        let DriverEvent::TurnItem(row) = &s.events[0] else {
            panic!("{:?}", s.events)
        };
        assert!(row.id.ends_with(crate::driver::RETRY_NOTICE_SUFFIX));
        assert!(
            matches!(&row.kind, TurnItemKind::SystemNotice { message, .. }
            if message == "Provider retry · Reconnecting... 2/5")
        );
        assert!(d.failure.is_none());
    }

    #[test]
    fn interrupt_cancels_requests_and_waits_for_the_codex_turn_id() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let turn_req = parse(&s.writes[0]);
        // Interrupt before turn/start answered: deferred until the id is known.
        let s = d.interrupt();
        assert!(s.writes.is_empty());
        let s = feed(
            &mut d,
            json!({"id": turn_req["id"], "result": {"turn": {"id": "ct9", "items": [], "status": "inProgress"}}}),
        );
        let int = parse(&s.writes[0]);
        assert_eq!(int["method"], "turn/interrupt");
        assert_eq!(int["params"]["turnId"], "ct9");
        // Open approvals answer `cancel`.
        feed(
            &mut d,
            json!({"id": 8, "method": "item/fileChange/requestApproval", "params": {"threadId": "th1", "turnId": "ct9", "itemId": "f1", "startedAtMs": 0}}),
        );
        let s = d.interrupt();
        assert_eq!(parse(&s.writes[0])["result"]["decision"], "cancel");
        let s = feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "ct9", "items": [], "status": "interrupted"}}}),
        );
        assert!(matches!(
            s.events[0],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                ..
            }
        ));
    }

    #[test]
    fn unsupported_server_requests_get_method_not_found() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = feed(
            &mut d,
            json!({"id": 3, "method": "unsupported/method", "params": {}}),
        );
        assert_eq!(parse(&s.writes[0])["error"]["code"], -32601);
    }

    #[test]
    fn access_modes_set_each_turns_policy_and_failed_items_carry_errors() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        assert!(d.switches_access_live(RuntimeAccessMode::FullAccess));
        for (mode, approval, sandbox) in [
            (
                RuntimeAccessMode::AutoAcceptEdits,
                "on-request",
                "workspaceWrite",
            ),
            (RuntimeAccessMode::FullAccess, "never", "dangerFullAccess"),
            (RuntimeAccessMode::ApprovalRequired, "untrusted", "readOnly"),
        ] {
            let req = RuntimePromptReq {
                access_mode: Some(mode),
                ..RuntimePromptReq::of("go")
            };
            let p = parse(&d.prompt("t1", &req).unwrap().writes[0])["params"].clone();
            assert_eq!(p["approvalPolicy"], approval);
            assert_eq!(p["sandboxPolicy"]["type"], sandbox);
            feed(
                &mut d,
                json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "x", "items": [], "status": "completed"}}}),
            );
        }
        // Seatbelt-confined: always the external sandbox.
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, true);
        let req = RuntimePromptReq {
            access_mode: Some(RuntimeAccessMode::FullAccess),
            ..RuntimePromptReq::of("go")
        };
        let p = parse(&d.prompt("t1", &req).unwrap().writes[0])["params"].clone();
        assert_eq!(
            (
                p["approvalPolicy"].as_str(),
                p["sandboxPolicy"]["type"].as_str()
            ),
            (Some("never"), Some("externalSandbox"))
        );
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "mcpToolCall", "id": "m1", "server": "web", "tool": "fetch", "status": "failed", "error": {"message": "timed out"}}}}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert_eq!(i.error.as_deref(), Some("timed out"));
        let s = feed(
            &mut d,
            json!({"id": 9, "method": "item/fileChange/requestApproval", "params": {"threadId": "th1", "turnId": "ct1", "itemId": "f1"}}),
        );
        assert!(
            matches!(&s.events[0], DriverEvent::Request { tool_item_id: Some(t), .. } if t == "t1:codex:f1")
        );
        // A PDF goes as the path the runtime saved it at (no document input).
        let pdf = RuntimePromptReq {
            files: vec![otto_core::agent_runtime::PromptFile {
                path: Some("/data/att/x.pdf".into()),
                mime: "application/pdf".into(),
                name: "a.pdf".into(),
                data_base64: "JVBERg==".into(),
            }],
            ..RuntimePromptReq::of("read")
        };
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = d.prompt("t1", &pdf).unwrap();
        let turn: Value = serde_json::from_str(s.writes.last().unwrap()).unwrap();
        assert_eq!(
            turn["params"]["input"][0]["text"],
            "read\n\n[Attached file \"a.pdf\" (application/pdf) is saved at: /data/att/x.pdf]"
        );
    }

    #[test]
    fn plan_mode_rides_the_turn_and_leaving_it_restores_the_instructions() {
        let mut d = CodexDriver::with_program("codex");
        let mut spec = LaunchSpec {
            args: vec!["-c".into(), "developer_instructions=OTTO CONTEXT".into()],
            ..LaunchSpec::default()
        };
        d.adopt_spec(&mut spec);
        ready(&mut d, false);
        let plan = RuntimePromptReq {
            plan: true,
            ..RuntimePromptReq::of("think first")
        };
        let s = d.prompt("t1", &plan).unwrap();
        let turn: Value = serde_json::from_str(s.writes.last().unwrap()).unwrap();
        assert_eq!(turn["params"]["collaborationMode"]["mode"], "plan");
        assert_eq!(
            turn["params"]["collaborationMode"]["settings"]["model"],
            "gpt"
        );
        assert!(turn["params"]["collaborationMode"]["settings"]
            .get("developer_instructions")
            .is_none());
        feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "ct1", "status": "completed"}}}),
        );
        let s = d.prompt("t2", &RuntimePromptReq::of("now do it")).unwrap();
        let turn: Value = serde_json::from_str(s.writes.last().unwrap()).unwrap();
        assert_eq!(turn["params"]["collaborationMode"]["mode"], "default");
        assert_eq!(
            turn["params"]["collaborationMode"]["settings"]["developer_instructions"],
            "OTTO CONTEXT"
        );
        feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "ct2", "status": "completed"}}}),
        );
        // Out of plan mode, turns carry no collaboration mode at all.
        let s = d.prompt("t3", &RuntimePromptReq::of("more")).unwrap();
        let turn: Value = serde_json::from_str(s.writes.last().unwrap()).unwrap();
        assert!(turn["params"].get("collaborationMode").is_none());
    }

    #[test]
    fn skills_are_its_commands_and_a_slash_skill_runs_one() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = feed(
            &mut d,
            json!({"id": 3, "result": {"data": [{"cwd": "/w", "errors": [], "skills": [
                {"name": "review", "description": "Review a diff thoroughly", "interface": {"shortDescription": "Review a diff"}, "enabled": true, "path": "/s/review/SKILL.md", "scope": "user"},
                {"name": "off", "description": "x", "enabled": false, "path": "/s/off/SKILL.md", "scope": "user"}
            ]}]}}),
        );
        let DriverEvent::Commands(c) = &s.events[0] else {
            panic!("commands missing: {:?}", s.events)
        };
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].name, "review");
        assert_eq!(
            c[0].description, "Review a diff",
            "the short description wins"
        );
        let s = d
            .prompt("t1", &RuntimePromptReq::of("/review the last commit"))
            .unwrap();
        let turn: Value = serde_json::from_str(s.writes.last().unwrap()).unwrap();
        assert_eq!(
            turn["params"]["input"],
            json!([{"type": "skill", "name": "review", "path": "/s/review/SKILL.md"}, {"type": "text", "text": "the last commit"}])
        );
        // An unknown `/word` stays text.
        feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "ct1", "status": "completed"}}}),
        );
        let s = d
            .prompt("t2", &RuntimePromptReq::of("/etc/hosts?"))
            .unwrap();
        let turn: Value = serde_json::from_str(s.writes.last().unwrap()).unwrap();
        assert_eq!(
            turn["params"]["input"],
            json!([{"type": "text", "text": "/etc/hosts?"}])
        );
    }

    #[test]
    fn thread_names_and_image_items_are_projected() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = feed(
            &mut d,
            json!({"method": "thread/name/updated", "params": {"threadId": "th1", "threadName": "Fix the login bug"}}),
        );
        assert_eq!(
            s.events,
            vec![DriverEvent::Title("Fix the login bug".into())]
        );
        d.prompt("t1", &RuntimePromptReq::of("draw")).unwrap();
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "imageGeneration", "id": "ig1", "status": "completed", "revisedPrompt": "a cat", "savedPath": "/tmp/cat.png", "inputImages": []}}}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(
            matches!(&i.kind, TurnItemKind::DynamicTool { image_files, tool_name, .. } if image_files == &vec!["/tmp/cat.png".to_string()] && tool_name == "image_generation")
        );
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "imageView", "id": "iv1", "path": "/w/shot.png"}}}),
        );
        let DriverEvent::TurnItem(i) = &s.events[0] else {
            panic!()
        };
        assert!(
            matches!(&i.kind, TurnItemKind::DynamicTool { image_files, .. } if image_files == &vec!["/w/shot.png".to_string()])
        );
    }
    #[test]
    fn questions_preserve_rpc_id_and_validate_before_consuming() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = feed(
            &mut d,
            json!({"id":"native-7","method":"item/tool/requestUserInput","params":{"questions":[{"id":"q_raw","header":"Choice","question":"Choose?","options":[{"label":"A","description":"First"}]}]}}),
        );
        let DriverEvent::Question { id, questions, .. } = &s.events[0] else {
            panic!("question event missing")
        };
        assert_eq!(questions[0].id, "q_raw");
        assert!(d.respond(id, ApprovalDecision::Accept).is_err());
        assert!(d.respond_question(id, &[]).is_err());
        let answer = QuestionAnswer::Text {
            question_id: "q_raw".into(),
            text: "Custom".into(),
        };
        let response = d.respond_question(id, &[answer]).unwrap();
        assert_eq!(
            parse(&response.writes[0]),
            json!({"id":"native-7","result":{"answers":{"q_raw":{"answers":["Custom"]}}}})
        );
        assert!(d.respond_question(id, &[]).is_err());
    }

    /// T3 Code #34771: an MCP server that fails to start is a notice in the
    /// thread (once), whether Codex reports it before or during a turn.
    #[test]
    fn an_mcp_server_that_fails_to_start_shows_a_notice_once() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let status = |name: &str, status: &str, error: Value| json!({"method":"mcpServer/startupStatus/updated","params":{"threadId":"th1","name":name,"status":status,"error":error}});
        // Before the turn: held until one runs; a recovered server is dropped.
        assert!(feed(
            &mut d,
            status("jira", "failed", json!("spawn jira-mcp: ENOENT"))
        )
        .events
        .is_empty());
        feed(&mut d, status("slack", "failed", Value::Null));
        feed(&mut d, status("slack", "ready", Value::Null));
        // Another thread's report is not this one's.
        let mut other = status("db", "failed", Value::Null);
        other["params"]["threadId"] = json!("th9");
        feed(&mut d, other);
        let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
        let notices: Vec<&TurnItem> = s
            .events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::TurnItem(i) => Some(i),
                _ => None,
            })
            .collect();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].id, "t1:mcp-startup-failed:jira");
        assert_eq!(
            notices[0].title.as_deref(),
            Some("MCP server jira failed to start")
        );
        assert!(
            matches!(&notices[0].kind, TurnItemKind::SystemNotice { message, .. } if message == "spawn jira-mcp: ENOENT")
        );
        // During a turn: shown at once, with a fallback text.
        let s = feed(&mut d, status("slack", "failed", json!("  ")));
        assert!(matches!(&s.events[0], DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::SystemNotice { message, .. } if message == "MCP server slack failed to start.")));
        // Shown once: the next turn carries nothing.
        feed(
            &mut d,
            json!({"method":"turn/completed","params":{"turn":{"id":"c1","status":"completed"}}}),
        );
        let s = d.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
        assert!(s.events.is_empty(), "{:?}", s.events);
    }

    #[test]
    fn autonomous_server_turn_emits_turn_started_and_settles_on_completed() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let started = feed(
            &mut d,
            json!({"method":"turn/started","params":{"turn":{"id":"bg-turn-1","status":"inProgress"}}}),
        );
        assert!(started
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "bg-turn-1")));
        let delta = feed(
            &mut d,
            json!({"method":"item/agentMessage/delta","params":{"itemId":"m1","delta":"background reply"}}),
        );
        assert!(delta.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id.starts_with("bg-turn-1:")
                    && matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "background reply")
        )));
        let done = feed(
            &mut d,
            json!({"method":"turn/completed","params":{"turn":{"id":"bg-turn-1","status":"completed"}}}),
        );
        assert!(done.events.iter().any(|e| matches!(
            e,
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        )));
    }

    #[test]
    fn steer_sends_turn_steer_with_expected_turn_id_and_keeps_turn_open_on_rpc_error() {
        use otto_core::agent_runtime::PromptImage;
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        assert!(
            d.steer(&RuntimePromptReq::of("too early")).is_err(),
            "idle driver cannot steer"
        );
        let start = d.prompt("t1", &RuntimePromptReq::of("work")).unwrap();
        let start_rpc = parse(&start.writes[0]);
        assert!(
            d.steer(&RuntimePromptReq::of("before server turn id"))
                .is_err(),
            "must wait until expectedTurnId is known"
        );

        feed(
            &mut d,
            json!({"id": start_rpc["id"], "result": {"turn": {"id": "ct-1", "items": [], "status": "inProgress"}}}),
        );
        assert!(
            d.steer(&RuntimePromptReq::default()).is_err(),
            "empty steer is refused"
        );
        let steer_req = RuntimePromptReq {
            images: vec![PromptImage {
                mime: "image/png".into(),
                data_base64: "iVBORw0KGgo=".into(),
                name: None,
            }],
            ..RuntimePromptReq::of("focus on tests")
        };
        let s = d.steer(&steer_req).unwrap();
        let msg = parse(&s.writes[0]);
        assert_eq!(msg["method"], "turn/steer");
        assert!(msg.get("jsonrpc").is_none());
        assert_eq!(msg["params"]["threadId"], "th1");
        assert_eq!(msg["params"]["expectedTurnId"], "ct-1");
        assert_eq!(
            msg["params"]["input"],
            json!([
                {"type": "image", "url": "data:image/png;base64,iVBORw0KGgo="},
                {"type": "text", "text": "focus on tests"},
            ])
        );

        // An RPC error on `turn/steer` surfaces `DriverEvent::Error` without
        // ending the active turn.
        let err_step = feed(
            &mut d,
            json!({"id": msg["id"], "error": {"code": -32600, "message": "turn busy"}}),
        );
        assert!(
            err_step
                .events
                .iter()
                .any(|e| matches!(e, DriverEvent::Error(f) if f.message.contains("turn busy"))),
            "{:?}",
            err_step.events
        );
        assert!(
            !err_step
                .events
                .iter()
                .any(|e| matches!(e, DriverEvent::Terminal { .. })),
            "active turn must remain running"
        );

        let done = feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"turn": {"id": "ct-1", "status": "completed"}}}),
        );
        assert!(done.events.iter().any(|e| matches!(
            e,
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        )));
    }

    #[test]
    fn snapshot_and_restore_preserve_codex_turn_items_and_pending_rpc_state() {
        let mut d1 = CodexDriver::with_program("codex");
        d1.start(&ctx(None, false));
        feed(&mut d1, json!({"id": 1, "result": {}}));
        feed(
            &mut d1,
            json!({"id": 2, "result": {"thread": {"id": "th-live"}}}),
        );
        let p = d1.prompt("t1", &RuntimePromptReq::of("run")).unwrap();
        let req_id = parse(&p.writes[0])["id"].clone();
        feed(
            &mut d1,
            json!({"id": req_id, "result": {"turn": {"id": "ct-9"}}}),
        );
        feed(
            &mut d1,
            json!({"method": "item/agentMessage/delta", "params": {"itemId": "m1", "delta": "Part 1 "}}),
        );
        feed(
            &mut d1,
            json!({"id": 77, "method": "item/commandExecution/requestApproval", "params": {"itemId": "c1", "command": "cargo test"}}),
        );

        // An earlier build's snapshot: no `program`, none of the later fields.
        let mut snap = d1.snapshot_state();
        let o = snap.as_object_mut().unwrap();
        for k in [
            "program",
            "thread_model",
            "plan",
            "developer_instructions",
            "skills",
            "skills_req",
        ] {
            o.remove(k);
        }
        let mut d2 = CodexDriver::with_program("otto-fake-codex");
        d2.restore_state(&snap).unwrap();
        assert_eq!(d2.program, "otto-fake-codex");

        let approval = d2.respond("codex-77", ApprovalDecision::Accept).unwrap();
        assert_eq!(parse(&approval.writes[0])["id"], 77);
        let delta2 = feed(
            &mut d2,
            json!({"method": "item/agentMessage/delta", "params": {"itemId": "m1", "delta": "Part 2"}}),
        );
        assert!(delta2.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Part 1 Part 2")
        )));
        let fin = feed(
            &mut d2,
            json!({"method": "turn/completed", "params": {"turn": {"id": "ct-9", "status": "completed"}}}),
        );
        assert!(fin.events.iter().any(|e| matches!(
            e,
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        )));
    }

    #[test]
    fn collab_spawn_agent_routes_child_thread_items_and_settles_independently_of_root() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let p = d
            .prompt("t1", &RuntimePromptReq::of("parallel review"))
            .unwrap();
        let req_id = parse(&p.writes[0])["id"].clone();
        feed(
            &mut d,
            json!({"id": req_id, "result": {"turn": {"id": "ct-root", "items": [], "status": "inProgress"}}}),
        );

        // 1. Spawn child subagent (`collabAgentToolCall` / `spawnAgent`).
        let sp_start = feed(
            &mut d,
            json!({
                "method": "item/started",
                "params": {
                    "threadId": "th1",
                    "turnId": "ct-root",
                    "item": {
                        "type": "collabAgentToolCall",
                        "id": "col-1",
                        "tool": "spawnAgent",
                        "role": "security-reviewer",
                        "prompt": "Audit auth middleware\nCheck JWT validation",
                        "status": "inProgress"
                    }
                }
            }),
        );
        assert!(sp_start.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id == "t1:codex:col-1"
                    && i.status == TurnItemStatus::Running
                    && matches!(
                        &i.kind,
                        TurnItemKind::Subagent {
                            tool_name,
                            description: Some(desc),
                            subagent_type: Some(st),
                            background: true,
                            ..
                        } if tool_name == "spawn_agent"
                            && desc == "security-reviewer"
                            && st == "security-reviewer"
                    )
        )));

        let sp_done = feed(
            &mut d,
            json!({
                "method": "item/completed",
                "params": {
                    "threadId": "th1",
                    "turnId": "ct-root",
                    "item": {
                        "type": "collabAgentToolCall",
                        "id": "col-1",
                        "tool": "spawnAgent",
                        "role": "security-reviewer",
                        "prompt": "Audit auth middleware\nCheck JWT validation",
                        "receiverThreadIds": ["th-sub-1"],
                        "status": "completed"
                    }
                }
            }),
        );
        assert!(
            sp_done.events.iter().any(|e| matches!(
                e,
                DriverEvent::TurnItem(i)
                    if i.id == "t1:codex:col-1" && i.status == TurnItemStatus::Running
            )),
            "spawnAgent completion must keep Subagent Running until child settles"
        );
        assert!(sp_done.events.iter().any(|e| matches!(
            e,
            DriverEvent::BackgroundTasks(tasks)
                if tasks.iter().any(|t| t.id == "th-sub-1" && t.kind == "agent")
        )));

        // 2. Child thread notifications route under `parent_item_id == Some("t1:codex:col-1")`.
        feed(
            &mut d,
            json!({"method": "turn/started", "params": {"threadId": "th-sub-1", "turn": {"id": "ct-sub-1", "status": "inProgress"}}}),
        );
        let child_cmd = feed(
            &mut d,
            json!({
                "method": "item/started",
                "params": {
                    "threadId": "th-sub-1",
                    "turnId": "ct-sub-1",
                    "item": {
                        "type": "commandExecution",
                        "id": "sub-c1",
                        "command": "rg validate_jwt src/",
                        "status": "inProgress"
                    }
                }
            }),
        );
        assert!(child_cmd.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id == "t1:codex:col-1:child:sub-c1"
                    && i.parent_item_id.as_deref() == Some("t1:codex:col-1")
        )));
        assert!(child_cmd.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id == "t1:codex:col-1"
                    && matches!(
                        &i.kind,
                        TurnItemKind::Subagent {
                            tool_uses: 1,
                            last_tool: Some(lt),
                            ..
                        } if lt == "rg validate_jwt src/"
                    )
        )));

        let out_delta = feed(
            &mut d,
            json!({
                "method": "item/commandExecution/outputDelta",
                "params": {"threadId": "th-sub-1", "itemId": "sub-c1", "delta": "src/auth.rs:42\n"}
            }),
        );
        assert!(out_delta.events.iter().any(|e| matches!(
            e,
            DriverEvent::CommandOutput { item_id, chunk }
                if item_id == "t1:codex:col-1:child:sub-c1" && chunk == "src/auth.rs:42\n"
        )));

        let child_msg = feed(
            &mut d,
            json!({
                "method": "item/agentMessage/delta",
                "params": {"threadId": "th-sub-1", "itemId": "sub-m1", "delta": "JWT validation is strict."}
            }),
        );
        assert!(child_msg.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.parent_item_id.as_deref() == Some("t1:codex:col-1")
                    && matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "JWT validation is strict.")
        )));

        // Snapshot & restore while child is active!
        let snap = d.snapshot_state();
        let mut d = CodexDriver::with_program("codex");
        d.restore_state(&snap).unwrap();

        // 3. Child `turn/completed` settles ONLY the child subagent, NOT root turn `t1`!
        let child_done = feed(
            &mut d,
            json!({
                "method": "turn/completed",
                "params": {"threadId": "th-sub-1", "turn": {"id": "ct-sub-1", "status": "completed"}}
            }),
        );
        assert!(
            !child_done
                .events
                .iter()
                .any(|e| matches!(e, DriverEvent::Terminal { .. })),
            "child turn/completed must NOT terminate root turn"
        );
        assert!(child_done.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id == "t1:codex:col-1"
                    && i.status == TurnItemStatus::Completed
                    && matches!(
                        &i.kind,
                        TurnItemKind::Subagent { output: Some(o), .. } if o == "JWT validation is strict."
                    )
        )));
        assert!(child_done.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if matches!(
                    &i.kind,
                    TurnItemKind::SystemNotice {
                        message,
                        task_settlement: Some(ts),
                    } if message == "Background agent finished: security-reviewer"
                        && ts.status == "completed"
                )
        )));
        assert!(child_done.events.iter().any(|e| matches!(
            e,
            DriverEvent::BackgroundTasks(tasks) if tasks.is_empty()
        )));
        // 4. Spawn a second child and close it via `closeAgent`.
        feed(
            &mut d,
            json!({
                "method": "item/completed",
                "params": {
                    "threadId": "th1",
                    "turnId": "ct-root",
                    "item": {
                        "type": "collabAgentToolCall",
                        "id": "col-2",
                        "tool": "spawn_agent",
                        "agent_role": "perf-helper",
                        "new_thread_id": "th-sub-2",
                        "status": "completed"
                    }
                }
            }),
        );
        let closed = feed(
            &mut d,
            json!({
                "method": "item/completed",
                "params": {
                    "threadId": "th1",
                    "turnId": "ct-root",
                    "item": {
                        "type": "collabAgentToolCall",
                        "id": "col-3",
                        "tool": "closeAgent",
                        "receiverThreadIds": ["th-sub-2"],
                        "status": "completed"
                    }
                }
            }),
        );
        assert!(closed.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id == "t1:codex:col-2" && i.status == TurnItemStatus::Cancelled
        )));
        assert!(closed.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if matches!(
                    &i.kind,
                    TurnItemKind::SystemNotice {
                        message,
                        task_settlement: Some(ts),
                    } if message == "Background agent stopped: perf-helper"
                        && ts.status == "stopped"
                )
        )));
    }

    /// A prompted turn whose spawned agent's child thread `th-sub-1` runs
    /// turn `ct-sub-1`.
    fn with_running_child(d: &mut CodexDriver) {
        ready(d, false);
        let p = d.prompt("t1", &RuntimePromptReq::of("fan out")).unwrap();
        let req_id = parse(&p.writes[0])["id"].clone();
        feed(
            d,
            json!({"id": req_id, "result": {"turn": {"id": "ct-root", "status": "inProgress"}}}),
        );
        feed(
            d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct-root", "item": {
                "type": "collabAgentToolCall", "id": "col-1", "tool": "spawnAgent",
                "prompt": "Audit auth", "receiverThreadIds": ["th-sub-1"], "status": "completed"}}}),
        );
        feed(
            d,
            json!({"method": "turn/started", "params": {"threadId": "th-sub-1", "turn": {"id": "ct-sub-1", "status": "inProgress"}}}),
        );
    }

    #[test]
    fn stop_task_interrupts_the_spawned_agents_own_turn_and_relays_the_answer() {
        let mut d = CodexDriver::with_program("codex");
        with_running_child(&mut d);
        assert!(matches!(d.stop_task("unknown"), TaskStop::Unsupported));
        let TaskStop::Native(step) = d.stop_task("th-sub-1") else {
            panic!("a running child stops natively");
        };
        let req = parse(&step.writes[0]);
        assert_eq!(req["method"], "turn/interrupt");
        assert_eq!(
            req["params"],
            json!({"threadId": "th-sub-1", "turnId": "ct-sub-1"})
        );
        let s = feed(&mut d, json!({"id": req["id"], "result": {}}));
        assert_eq!(
            s.events,
            vec![DriverEvent::TaskStopAnswered {
                task_id: "th-sub-1".into(),
                error: None
            }]
        );
        // The child's own end settles the card and empties the list.
        let s = feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th-sub-1", "turn": {"id": "ct-sub-1", "status": "interrupted"}}}),
        );
        assert!(s.events.iter().any(|e| matches!(
            e,
            DriverEvent::BackgroundTasks(l) if l.is_empty()
        )));
    }

    /// The parent's Stop interrupts each running child thread too, and a
    /// child that still runs is not called stopped until it says so.
    #[test]
    fn a_parent_interrupt_interrupts_children_and_keeps_them_listed_until_they_end() {
        let mut d = CodexDriver::with_program("codex");
        with_running_child(&mut d);
        let step = d.interrupt();
        let sent: Vec<Value> = step.writes.iter().map(|w| parse(w)).collect();
        assert!(sent.iter().any(|m| m["method"] == "turn/interrupt"
            && m["params"] == json!({"threadId": "th1", "turnId": "ct-root"})));
        assert!(sent.iter().any(|m| m["method"] == "turn/interrupt"
            && m["params"] == json!({"threadId": "th-sub-1", "turnId": "ct-sub-1"})));
        let s = feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "th1", "turn": {"id": "ct-root", "status": "interrupted"}}}),
        );
        assert!(!s.events.iter().any(|e| matches!(
            e,
            DriverEvent::BackgroundTasks(l) if l.is_empty()
        )));
        assert!(!s.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::SystemNotice { .. })
        )));
        assert_eq!(d.bg_tasks.len(), 1, "the child still runs");
    }

    #[test]
    fn a_steer_at_another_effort_waits_and_an_unknown_effort_is_dropped_with_a_notice() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let mut req = RuntimePromptReq::of("go");
        req.effort = Some("high".into());
        let p = d.prompt("t1", &req).unwrap();
        let start = parse(&p.writes[0]);
        assert_eq!(start["params"]["effort"], "high");
        feed(
            &mut d,
            json!({"id": start["id"], "result": {"turn": {"id": "ct1", "status": "inProgress"}}}),
        );
        let mut same = RuntimePromptReq::of("and this");
        same.effort = Some("high".into());
        assert!(d.steer(&same).is_ok());
        let mut other = RuntimePromptReq::of("think harder");
        other.effort = Some("xhigh".into());
        assert!(d.steer(&other).is_err(), "queued for the next turn");
        feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"turn": {"id": "ct1", "status": "completed"}}}),
        );
        let mut max = RuntimePromptReq::of("go");
        max.effort = Some("max".into());
        let p = d.prompt("t2", &max).expect("dropped, not refused");
        assert!(p.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i) if matches!(
                &i.kind,
                TurnItemKind::SystemNotice { message, .. } if message.contains("max")
            )
        )));
        let start = parse(p.writes.last().unwrap());
        assert_eq!(start["method"], "turn/start");
        assert!(start["params"].get("effort").is_none());
    }
}
