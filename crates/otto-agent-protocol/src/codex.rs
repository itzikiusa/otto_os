//! Codex driver — `codex app-server` (JSON-RPC over stdio, one message per
//! line, no `"jsonrpc"` member).
//!
//! Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
//! `apps/server/src/orchestration-v2/Adapters/CodexAdapterV2.ts` with the
//! wire shapes of `packages/effect-codex-app-server` (generated from
//! `openai/codex` `codex-rs/app-server-protocol`). The crates.io
//! `codex-app-server-protocol` (0.63.0, published from a fork) lags the
//! installed codex-cli 0.166.0 by a hundred releases, so — like T3 Code's
//! generated client — Otto keeps a minimal typed surface here, checked
//! against `codex app-server generate-json-schema` of 0.166.0:
//!
//! * spawn `codex app-server` (`codexLaunchArgs.ts:12-15`); Otto's context
//!   hook adds `--add-dir=…`, `-c developer_instructions=…` (before the
//!   subcommand) and `CODEX_HOME`, exactly as for a PTY codex session.
//! * `initialize {clientInfo, capabilities: {experimentalApi,
//!   optOutNotificationMethods: ["turn/diff/updated"]}}` then the
//!   `initialized` notification (`CodexAdapterV2.ts:1763-1801`, `:233-237`).
//! * `thread/start {cwd, model, approvalPolicy, sandbox}` or, with a native
//!   id, `thread/resume {threadId, excludeTurns: true, …}` (`:7061-7108`); a
//!   failed resume starts a fresh thread.
//! * `turn/start {threadId, input: [{type: text}], approvalPolicy,
//!   sandboxPolicy}` (`buildCodexTurnStartParams`, `:744-818`) and
//!   `turn/interrupt {threadId, turnId}` (`:7567`).
//! * access mode, per turn (`codexRuntimeModeTurnDefaults`, `:703-742`):
//!   approval-required → `approvalPolicy: "untrusted"` + `sandboxPolicy:
//!   readOnly`; auto-accept-edits → `"on-request"` + `workspaceWrite`;
//!   full-access → `"never"` + `dangerFullAccess`. Under Otto's Seatbelt the
//!   inner sandbox is always `externalSandbox` (nested Seatbelt fails; the
//!   PTY path drops codex's own sandbox the same way, `otto-sessions`
//!   `interaction_mode_apply`). A failed item's error text (`error`, else
//!   the command's output) rides the item. No file inputs: 0.166.0's
//!   `UserInput` has none, so a PDF is refused.
//! * notifications → entities (`:4364-4955`, `:6529`): `item/started` /
//!   `item/completed` upsert one item per Codex item (`agentMessage` →
//!   `assistant_message`, `reasoning`, `commandExecution` →
//!   `command_execution`, `fileChange` → `file_change`, `mcpToolCall` /
//!   `dynamicToolCall` → `dynamic_tool`, `webSearch` → `web_search`);
//!   `item/agentMessage/delta` and `item/reasoning/*Delta` grow the text
//!   (`streaming: true`); `item/commandExecution/outputDelta` →
//!   `command_output.delta`; `turn/plan/updated` → the plan;
//!   `thread/tokenUsage/updated` → usage; `turn/completed` → terminal
//!   (`mapCodexTurnStatus`, `:359-372`); item status `codexItemStatus`
//!   (`:392-414`).
//! * approvals (`:5277-5420`): `item/commandExecution/requestApproval`,
//!   `item/fileChange/requestApproval` answer `{decision: accept |
//!   acceptForSession | decline | cancel}`; `item/permissions/requestApproval`
//!   answers the requested profile with scope turn / session, or an empty
//!   profile (`:609-621`). `serverRequest/resolved` closes a request Codex
//!   settled. The v1 `execCommandApproval` / `applyPatchApproval` belong to
//!   the conversation API Otto does not use.
//! * `mcpServer/startupStatus/updated` with `status: "failed"` becomes a
//!   `system_notice` in the running turn, or the next one when it arrives
//!   between turns (T3 Code #34771); another status clears that server.

use std::collections::HashMap;

use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, PlanStep, PlanStepStatus, ProviderFailure, RequestKind,
    RuntimeAccessMode, RuntimePromptReq, RuntimeProvider, RuntimeRequestStatus, ThreadTokenUsage,
    TurnItem, TurnItemKind, TurnItemStatus, TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, rpc_failure, rpc_key, tool_error, DriverError, DriverEvent, DriverStep, LaunchContext,
    LaunchSpec, StructuredDriver,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReqProto {
    /// `item/*/requestApproval` — `{decision}` in v2 spelling.
    V2Decision,
    /// `item/permissions/requestApproval`.
    Permissions,
    Question,
}

struct Pending {
    rpc_id: Value,
    proto: ReqProto,
    permissions: Value,
}

/// Accumulated state of one Codex item.
#[derive(Default)]
struct Item {
    text: String,
    output: String,
}

pub struct CodexDriver {
    program: String,
    next_id: u64,
    init_id: u64,
    thread_req: Option<(u64, bool)>,
    turn_req: Option<u64>,
    ready: bool,
    cwd: String,
    model: Option<String>,
    resume: Option<String>,
    sandboxed: bool,
    access: RuntimeAccessMode,
    thread_id: Option<String>,
    turn_id: Option<String>,
    queued: Option<RuntimePromptReq>,
    codex_turn: Option<String>,
    interrupt_when_started: bool,
    items: HashMap<String, Item>,
    pending: HashMap<String, Pending>,
    failure: Option<ProviderFailure>,
    /// MCP servers that failed to start and are not shown yet: (name, error).
    mcp_failures: Vec<(String, String)>,
    /// `turn/steer` requests in flight.
    steers: Vec<u64>,
    /// Sub-agent cards by the sub-agent's own Codex thread id.
    agents: HashMap<String, TurnItem>,
}

impl CodexDriver {
    pub fn with_program(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            next_id: 0,
            init_id: 0,
            thread_req: None,
            turn_req: None,
            ready: false,
            cwd: String::new(),
            model: None,
            resume: None,
            sandboxed: false,
            access: RuntimeAccessMode::default(),
            thread_id: None,
            turn_id: None,
            queued: None,
            codex_turn: None,
            interrupt_when_started: false,
            items: HashMap::new(),
            pending: HashMap::new(),
            failure: None,
            mcp_failures: Vec::new(),
            steers: Vec::new(),
            agents: HashMap::new(),
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

    /// A prompt's images then text as Codex `UserInput`s.
    fn input(req: &RuntimePromptReq) -> Vec<Value> {
        let mut input: Vec<Value> = req
            .images
            .iter()
            .map(|i| json!({"type": "image", "url": format!("data:{};base64,{}", i.mime, i.data_base64)}))
            .collect();
        if !req.text.is_empty() {
            input.push(json!({"type": "text", "text": req.text}));
        }
        input
    }

    /// `model` / `effort` / `access_mode` apply to this turn and the ones after it.
    fn send_turn(&mut self, step: &mut DriverStep, req: &RuntimePromptReq) {
        self.access = req.access_mode.unwrap_or(self.access);
        let mut params = json!({
            "threadId": self.thread_id,
            "input": Self::input(req),
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
                TurnItemKind::SystemNotice { message: error },
            )));
        }
    }

    fn item_id(&self, codex_id: &str) -> String {
        format!("{}:codex:{codex_id}", self.turn())
    }

    /// Codex sub-agents (`multi_agent`, on by default in 0.167): each runs
    /// in its own Codex thread. A `collabAgentToolCall` `spawnAgent` is one
    /// background subagent card for its receiver thread(s); later collab
    /// calls (`wait`, `sendInput`, `closeAgent`, …) and `subAgentActivity`
    /// only move the card's status (`agentsStates[thread]`); they are not
    /// rows of their own (T3 Code `CodexAdapterV2` `registerSubagentThreads`
    /// / `updateSubagentStates`).
    fn on_collab(&mut self, step: &mut DriverStep, it: &Value) {
        let s = |k: &str| it.get(k).and_then(Value::as_str).map(str::to_string);
        let ty = s("type").unwrap_or_default();
        if ty == "collabAgentToolCall" && s("tool").as_deref() == Some("spawnAgent") {
            let receivers: Vec<String> = it
                .get("receiverThreadIds")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let prompt = s("prompt").filter(|p| !p.trim().is_empty());
            let name = prompt
                .as_deref()
                .and_then(|p| p.lines().find(|l| !l.trim().is_empty()))
                .map(|l| {
                    let l = l.trim();
                    match l.char_indices().nth(60) {
                        Some((i, _)) => format!("{}…", &l[..i]),
                        None => l.to_string(),
                    }
                })
                .unwrap_or_else(|| "Sub-agent".into());
            let cid = s("id").unwrap_or_default();
            let card_id = self.item_id(&cid);
            let mut card = self
                .agents
                .values()
                .find(|c| c.id == card_id)
                .cloned()
                .unwrap_or_else(|| {
                    let mut c = item(
                        card_id.clone(),
                        TurnItemStatus::Running,
                        Some(name.clone()),
                        TurnItemKind::Subagent {
                            tool_name: "spawnAgent".into(),
                            description: Some(name.clone()),
                            subagent_type: s("model"),
                            prompt: prompt.clone(),
                            output: None,
                            background: true,
                            last_tool: None,
                            tool_uses: 0,
                        },
                    );
                    c.turn_id = self.turn_id.clone();
                    c
                });
            if s("status").as_deref() == Some("failed") && receivers.is_empty() {
                card.status = TurnItemStatus::Failed;
            }
            for r in &receivers {
                self.agents.insert(r.clone(), card.clone());
            }
            if receivers.is_empty() {
                step.push(DriverEvent::TurnItem(card));
                return;
            }
        }
        if ty == "subAgentActivity" {
            if let Some(thread) = s("agentThreadId") {
                let status = match s("kind").as_deref() {
                    Some("completed") => TurnItemStatus::Completed,
                    Some("interrupted") => TurnItemStatus::Interrupted,
                    _ => TurnItemStatus::Running,
                };
                let name = s("agentPath").unwrap_or_else(|| "Sub-agent".into());
                self.child_card(&thread, &name);
                self.set_agent(step, &thread, status, None);
            }
            return;
        }
        if let Some(states) = it.get("agentsStates").and_then(Value::as_object) {
            for (thread, st) in states {
                let status = match st.get("status").and_then(Value::as_str) {
                    Some("completed") => TurnItemStatus::Completed,
                    Some("errored") => TurnItemStatus::Failed,
                    Some("interrupted" | "shutdown" | "notFound") => TurnItemStatus::Interrupted,
                    _ => TurnItemStatus::Running,
                };
                let message = st
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.set_agent(step, thread, status, message);
            }
        }
        // A spawn whose receivers carry no state yet: shown running.
        for r in it
            .get("receiverThreadIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if it.pointer(&format!("/agentsStates/{r}")).is_none() {
                if let Some(c) = self.agents.get(r).cloned() {
                    step.push(DriverEvent::TurnItem(c));
                }
            }
        }
    }

    fn on_output_delta(&mut self, step: &mut DriverStep, params: &Value) {
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

    /// A notification of a sub-agent's own thread: its turns move its card,
    /// its items become the card's activity (`parent_item_id`), in the
    /// card's turn. A thread with no card yet (no spawn seen) gets one.
    fn on_child(&mut self, step: &mut DriverStep, thread: &str, m: &str, params: &Value) {
        if !self.agents.contains_key(thread) {
            if self.turn_id.is_none() || !(m.starts_with("item/") || m == "turn/started") {
                return;
            }
            self.child_card(thread, "Sub-agent");
        }
        let (card_id, card_turn) = {
            let c = &self.agents[thread];
            (c.id.clone(), c.turn_id.clone())
        };
        match m {
            "turn/started" => self.set_agent(step, thread, TurnItemStatus::Running, None),
            "turn/completed" => {
                let turn = &params["turn"];
                let status = match turn.get("status").and_then(Value::as_str) {
                    Some("completed") => TurnItemStatus::Completed,
                    Some("failed") => TurnItemStatus::Failed,
                    Some("interrupted") => TurnItemStatus::Interrupted,
                    _ => return,
                };
                let error = turn
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.set_agent(step, thread, status, error);
            }
            _ => {
                let saved = std::mem::replace(&mut self.turn_id, card_turn.clone());
                let mut sub = DriverStep::default();
                match m {
                    "item/started" | "item/completed" => {
                        if let Some(it) = params.get("item") {
                            let ty = it.get("type").and_then(Value::as_str).unwrap_or_default();
                            if matches!(ty, "collabAgentToolCall" | "subAgentActivity") {
                                self.on_collab(&mut sub, it);
                            } else if let Some(i) = self.project(it, m == "item/completed") {
                                if m == "item/started"
                                    && !matches!(ty, "agentMessage" | "reasoning")
                                {
                                    if let Some(c) = self.agent_step(thread, i.title.clone()) {
                                        sub.push(DriverEvent::TurnItem(c));
                                    }
                                }
                                sub.push(DriverEvent::TurnItem(i));
                            }
                        }
                    }
                    "item/agentMessage/delta" => self.on_text_delta(&mut sub, params, false),
                    "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                        self.on_text_delta(&mut sub, params, true)
                    }
                    "item/commandExecution/outputDelta" => self.on_output_delta(&mut sub, params),
                    _ => {}
                }
                self.turn_id = saved;
                for mut ev in sub.events {
                    if let DriverEvent::TurnItem(i) = &mut ev {
                        if i.id != card_id && i.parent_item_id.is_none() {
                            i.parent_item_id = Some(card_id.clone());
                        }
                        if i.turn_id.is_none() {
                            i.turn_id = card_turn.clone();
                        }
                    }
                    step.push(ev);
                }
            }
        }
    }

    /// The card of a sub-agent thread, made on first sight.
    fn child_card(&mut self, thread: &str, name: &str) -> String {
        if let Some(c) = self.agents.get(thread) {
            return c.id.clone();
        }
        let mut c = item(
            format!("{}:codex-agent:{thread}", self.turn()),
            TurnItemStatus::Running,
            Some(name.to_string()),
            TurnItemKind::Subagent {
                tool_name: "spawnAgent".into(),
                description: Some(name.to_string()),
                subagent_type: None,
                prompt: None,
                output: None,
                background: true,
                last_tool: None,
                tool_uses: 0,
            },
        );
        c.turn_id = self.turn_id.clone();
        let id = c.id.clone();
        self.agents.insert(thread.to_string(), c);
        id
    }

    /// Move a sub-agent card (every thread sharing it) and re-emit it.
    fn set_agent(
        &mut self,
        step: &mut DriverStep,
        thread: &str,
        status: TurnItemStatus,
        message: Option<String>,
    ) {
        let Some(card) = self.agents.get(thread) else {
            return;
        };
        let id = card.id.clone();
        let now = chrono::Utc::now();
        let mut out = None;
        for c in self.agents.values_mut().filter(|c| c.id == id) {
            c.status = status;
            c.updated_at = now;
            c.completed_at = (!status.is_open()).then_some(now);
            if let (TurnItemKind::Subagent { output, .. }, Some(m)) = (&mut c.kind, &message) {
                *output = Some(m.clone());
            }
            if status == TurnItemStatus::Failed {
                c.error = message.as_deref().and_then(tool_error);
            }
            out = Some(c.clone());
        }
        if let Some(c) = out {
            step.push(DriverEvent::TurnItem(c));
        }
    }

    /// A sub-agent thread's own step: one more on its card.
    fn agent_step(&mut self, thread: &str, title: Option<String>) -> Option<TurnItem> {
        let id = self.agents.get(thread)?.id.clone();
        let mut out = None;
        for c in self.agents.values_mut().filter(|c| c.id == id) {
            if let TurnItemKind::Subagent {
                last_tool,
                tool_uses,
                ..
            } = &mut c.kind
            {
                *tool_uses += 1;
                *last_tool = title.clone();
            }
            c.updated_at = chrono::Utc::now();
            out = Some(c.clone());
        }
        out
    }

    /// A Codex `ThreadItem` → an Otto item (`None` = not projected).
    fn project(&mut self, it: &Value, completed: bool) -> Option<TurnItem> {
        let cid = it.get("id").and_then(Value::as_str)?.to_string();
        let ty = it.get("type").and_then(Value::as_str)?;
        let empty_text = it
            .get("text")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty);
        let streamed = self.items.get(&cid).is_some_and(|i| !i.text.is_empty());
        let summary = it
            .get("summary")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        if matches!(ty, "agentMessage" | "reasoning") && empty_text && !streamed && !summary {
            // Shown from its first delta, never as an empty bubble.
            if completed {
                self.items.remove(&cid);
            }
            return None;
        }
        let st = self.items.entry(cid.clone()).or_default();
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
                },
                None,
            ),
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
            self.items.remove(&cid);
        }
        let mut out = item(self.item_id(&cid), status, title, kind);
        out.error = error;
        Some(out)
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
        let item = item(self.item_id(cid), TurnItemStatus::Running, None, kind);
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
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
        self.turn_id = None;
        self.codex_turn = None;
        self.turn_req = None;
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
                    step.push(DriverEvent::Question {
                        id,
                        title: None,
                        questions,
                        tool_item_id: params
                            .get("itemId")
                            .and_then(Value::as_str)
                            .map(|i| self.item_id(i)),
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
        step.push(DriverEvent::Request {
            id: request_id,
            kind,
            title: Some(title),
            prompt,
            reason: s("reason"),
            options,
            tool_item_id: s("itemId").map(|i| self.item_id(&i)),
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
            step.push(DriverEvent::Ready {
                native_thread_id: self.thread_id.clone(),
                model: r.get("model").and_then(Value::as_str).map(str::to_string),
            });
            if let Some(req) = self.queued.take() {
                self.send_turn(step, &req);
            }
        } else if let Some(n) = self.steers.iter().position(|s| *s == id) {
            self.steers.remove(n);
            // The turn ended (or moved on) before Codex took it.
            if let (Some(e), Some(_)) = (error, &self.turn_id) {
                let msg = e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Codex refused it");
                step.push(DriverEvent::TurnItem(item(
                    format!("{}:steer-failed:{id}", self.turn()),
                    TurnItemStatus::Failed,
                    Some("Codex did not take the message".to_string()),
                    TurnItemKind::SystemNotice {
                        message: format!("Codex did not take the message: {msg}"),
                    },
                )));
            }
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
        if !req.files.is_empty() {
            return Err(DriverError(
                "Codex does not accept file attachments (only images)".into(),
            ));
        }
        self.turn_id = Some(turn_id.to_string());
        self.failure = None;
        let mut step = DriverStep::default();
        self.flush_mcp_failures(&mut step);
        if self.ready {
            self.send_turn(&mut step, req);
        } else {
            self.queued = Some(req.clone());
        }
        Ok(step)
    }

    /// `turn/steer {threadId, expectedTurnId, input}` (app-server v2): the
    /// input joins the running turn at its next step. Only once Codex named
    /// the turn (`turn/start`'s answer or `turn/started`); before that the
    /// message stays queued.
    fn steer(&mut self, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        let (Some(_), Some(thread), Some(turn)) =
            (&self.turn_id, &self.thread_id, &self.codex_turn)
        else {
            return Err(DriverError("no Codex turn is running yet".into()));
        };
        if !req.files.is_empty() {
            return Err(DriverError(
                "Codex does not accept file attachments (only images)".into(),
            ));
        }
        let params = json!({"threadId": thread, "expectedTurnId": turn, "input": Self::input(req)});
        let (id, line) = self.request("turn/steer", params);
        self.steers.push(id);
        let mut step = DriverStep::default();
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
        if self.queued.take().is_some() {
            self.turn_id = None;
            step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                failure: None,
                reason: None,
            });
            return step;
        }
        self.interrupt_when_started = true;
        self.maybe_interrupt(&mut step);
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
            // A sub-agent's own thread: its items are its card's activity,
            // its turns move the card — never this thread's turn.
            (Some(m), None)
                if params
                    .get("threadId")
                    .and_then(Value::as_str)
                    .is_some_and(|t| self.thread_id.as_deref().is_some_and(|me| me != t)) =>
            {
                let thread = params["threadId"].as_str().unwrap_or_default().to_string();
                self.on_child(&mut step, &thread, &m, &params);
            }
            (Some(m), None) => match m.as_str() {
                "turn/started" => {
                    if let Some(t) = params.pointer("/turn/id").and_then(Value::as_str) {
                        self.codex_turn = Some(t.to_string());
                    }
                    self.maybe_interrupt(&mut step);
                }
                "item/started" | "item/completed" if self.turn_id.is_some() => {
                    if let Some(item) = params.get("item") {
                        if matches!(
                            item.get("type").and_then(Value::as_str),
                            Some("collabAgentToolCall" | "subAgentActivity")
                        ) {
                            self.on_collab(&mut step, item);
                        } else if let Some(i) = self.project(item, m == "item/completed") {
                            step.push(DriverEvent::TurnItem(i));
                        }
                    }
                }
                "item/agentMessage/delta" if self.turn_id.is_some() => {
                    self.on_text_delta(&mut step, &params, false)
                }
                "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta"
                    if self.turn_id.is_some() =>
                {
                    self.on_text_delta(&mut step, &params, true)
                }
                "item/commandExecution/outputDelta" if self.turn_id.is_some() => {
                    self.on_output_delta(&mut step, &params)
                }
                "turn/plan/updated" if self.turn_id.is_some() => self.on_plan(&mut step, &params),
                "thread/tokenUsage/updated" => self.on_usage(&mut step, &params),
                "mcpServer/startupStatus/updated" => self.on_mcp_startup(&mut step, &params),
                "error" => {
                    if params.get("willRetry").and_then(Value::as_bool) != Some(true) {
                        if let Some(e) = params.get("error") {
                            self.failure = Some(rpc_failure("provider_error", e));
                        }
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
            },
            (None, None) => {}
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        self.agents.clear();
        self.pending.clear();
        self.items.clear();
        DriverStep::default()
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

    #[test]
    fn steer_joins_the_running_codex_turn_once_it_has_an_id() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        assert!(d.steer(&RuntimePromptReq::of("more")).is_err(), "idle");
        let s = d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let turn_req = parse(&s.writes[0]);
        assert!(
            d.steer(&RuntimePromptReq::of("more")).is_err(),
            "turn not named yet"
        );
        feed(
            &mut d,
            json!({"id": turn_req["id"], "result": {"turn": {"id": "ct9", "items": [], "status": "inProgress"}}}),
        );
        let s = d.steer(&RuntimePromptReq::of("also b")).unwrap();
        let st = parse(&s.writes[0]);
        assert_eq!(st["method"], "turn/steer");
        assert_eq!(st["params"]["threadId"], "th1");
        assert_eq!(st["params"]["expectedTurnId"], "ct9");
        assert_eq!(
            st["params"]["input"],
            json!([{"type": "text", "text": "also b"}])
        );
        // A refused steer (the turn moved on) shows instead of vanishing.
        let s = feed(
            &mut d,
            json!({"id": st["id"], "error": {"code": -32600, "message": "no active turn"}}),
        );
        assert!(matches!(
            &s.events[0],
            DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::SystemNotice { message } if message.contains("no active turn"))
        ));
    }

    /// Codex 0.167 `multi_agent` (app-server schema v2): a spawn is a
    /// background card; the child thread's items are its activity and its
    /// turn never ends ours; `wait`'s `agentsStates` settle it.
    #[test]
    fn sub_agents_are_cards_with_their_threads_work() {
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        let s = d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let tr = parse(&s.writes[0]);
        feed(
            &mut d,
            json!({"id": tr["id"], "result": {"turn": {"id": "ct1", "status": "inProgress"}}}),
        );
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "collabAgentToolCall", "id": "sp1", "tool": "spawnAgent", "status": "completed", "senderThreadId": "th1", "receiverThreadIds": ["kid1"], "prompt": "Run echo sub-hello\nand report", "model": "gpt-5.5", "agentsStates": {"kid1": {"status": "running"}}}}}),
        );
        let card = match &s.events[0] {
            DriverEvent::TurnItem(i) => i.clone(),
            e => panic!("{e:?}"),
        };
        assert!(card.is_background_open());
        assert!(
            matches!(&card.kind, TurnItemKind::Subagent { description: Some(n), prompt: Some(p), .. } if n == "Run echo sub-hello" && p.ends_with("report"))
        );
        let s = feed(
            &mut d,
            json!({"method": "turn/started", "params": {"threadId": "kid1", "turn": {"id": "kt1", "status": "inProgress"}}}),
        );
        assert!(s
            .events
            .iter()
            .all(|e| matches!(e, DriverEvent::TurnItem(i) if i.id == card.id)));
        let s = feed(
            &mut d,
            json!({"method": "item/started", "params": {"threadId": "kid1", "turnId": "kt1", "item": {"type": "commandExecution", "id": "c9", "command": "echo sub-hello", "status": "inProgress"}}}),
        );
        let kid = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::TurnItem(i) if i.id != card.id => Some(i.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(kid.parent_item_id.as_ref(), Some(&card.id));
        assert!(s.events.iter().any(|e| matches!(e, DriverEvent::TurnItem(i) if i.id == card.id && matches!(i.kind, TurnItemKind::Subagent { tool_uses: 1, .. }))));
        // The child's turn ends: its card, not our turn.
        let s = feed(
            &mut d,
            json!({"method": "turn/completed", "params": {"threadId": "kid1", "turn": {"id": "kt1", "status": "completed"}}}),
        );
        assert!(!s
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::Terminal { .. })));
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if i.id == card.id && i.status == TurnItemStatus::Completed)
        );
        let s = feed(
            &mut d,
            json!({"method": "item/completed", "params": {"threadId": "th1", "turnId": "ct1", "item": {"type": "collabAgentToolCall", "id": "w1", "tool": "wait", "status": "completed", "senderThreadId": "th1", "receiverThreadIds": ["kid1"], "agentsStates": {"kid1": {"status": "completed", "message": "sub-hello"}}}}}),
        );
        assert!(
            matches!(&s.events[0], DriverEvent::TurnItem(i) if i.id == card.id && matches!(&i.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "sub-hello"))
        );
        assert!(d.turn_id.is_some(), "our turn still runs");
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
        let pdf = RuntimePromptReq {
            files: vec![otto_core::agent_runtime::PromptFile {
                mime: "application/pdf".into(),
                name: "a.pdf".into(),
                data_base64: "JVBERg==".into(),
            }],
            ..RuntimePromptReq::of("read")
        };
        let mut d = CodexDriver::with_program("codex");
        ready(&mut d, false);
        assert!(d.prompt("t1", &pdf).is_err());
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
            matches!(&notices[0].kind, TurnItemKind::SystemNotice { message } if message == "spawn jira-mcp: ENOENT")
        );
        // During a turn: shown at once, with a fallback text.
        let s = feed(&mut d, status("slack", "failed", json!("  ")));
        assert!(matches!(&s.events[0], DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::SystemNotice { message } if message == "MCP server slack failed to start.")));
        // Shown once: the next turn carries nothing.
        feed(
            &mut d,
            json!({"method":"turn/completed","params":{"turn":{"id":"c1","status":"completed"}}}),
        );
        let s = d.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
        assert!(s.events.is_empty(), "{:?}", s.events);
    }
}
