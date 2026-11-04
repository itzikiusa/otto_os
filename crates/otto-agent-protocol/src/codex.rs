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
//!   hook adds `-c developer_instructions=…` and `CODEX_HOME` exactly as for
//!   a PTY codex session.
//! * `initialize {clientInfo, capabilities: {experimentalApi,
//!   optOutNotificationMethods: ["turn/diff/updated"]}}` then the
//!   `initialized` notification (`CodexAdapterV2.ts:1763-1801`, `:233-237`).
//! * `thread/start {cwd, model, approvalPolicy, sandbox}` or, with a native
//!   id, `thread/resume {threadId, excludeTurns: true, …}` (`:7061-7108`); a
//!   failed resume starts a fresh thread.
//! * `turn/start {threadId, input: [{type: text}], approvalPolicy,
//!   sandboxPolicy}` (`buildCodexTurnStartParams`, `:744-818`) and
//!   `turn/interrupt {threadId, turnId}` (`:7567`).
//! * runtime mode: T3 Code's approval-required — `approvalPolicy:
//!   "untrusted"` + `sandboxPolicy: readOnly` (`:703-742`). Under Otto's
//!   Seatbelt the inner sandbox is replaced by `externalSandbox` (nested
//!   Seatbelt fails; the PTY path drops codex's own sandbox the same way,
//!   `otto-sessions` `interaction_mode_apply`) — approvals still go to a
//!   person.
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
//! * approvals (`:5277-5640`): `item/commandExecution/requestApproval`,
//!   `item/fileChange/requestApproval` answer `{decision: accept |
//!   acceptForSession | decline | cancel}`; the legacy `execCommandApproval`
//!   / `applyPatchApproval` answer `approved | approved_for_session | denied
//!   | abort` (`:576-590`); `item/permissions/requestApproval` answers the
//!   requested profile with scope turn / session, or an empty profile
//!   (`:609-621`). `serverRequest/resolved` closes a request Codex settled.

use std::collections::HashMap;

use chrono::Utc;
use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, PlanArtifact, PlanStep, PlanStepStatus, ProviderFailure,
    RequestKind, ResponseCapability, RuntimeProvider, RuntimeRequest, RuntimeRequestStatus,
    ThreadTokenUsage, TurnItem, TurnItemKind, TurnItemStatus, TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    DriverError, DriverEvent, DriverStep, LaunchContext, LaunchSpec, StructuredDriver,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReqProto {
    /// `item/*/requestApproval` — `{decision}` in v2 spelling.
    V2Decision,
    /// `execCommandApproval` / `applyPatchApproval` — `ReviewDecision`.
    Legacy,
    /// `item/permissions/requestApproval`.
    Permissions,
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
    thread_id: Option<String>,
    turn_id: Option<String>,
    queued: Option<String>,
    codex_turn: Option<String>,
    interrupt_when_started: bool,
    items: HashMap<String, Item>,
    pending: HashMap<String, Pending>,
    failure: Option<ProviderFailure>,
}

impl Default for CodexDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexDriver {
    pub fn new() -> Self {
        Self::with_program("codex")
    }

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
            thread_id: None,
            turn_id: None,
            queued: None,
            codex_turn: None,
            interrupt_when_started: false,
            items: HashMap::new(),
            pending: HashMap::new(),
            failure: None,
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
        if self.sandboxed {
            "danger-full-access" // confined by Otto's Seatbelt instead
        } else {
            "read-only"
        }
    }

    /// The turn-level `SandboxPolicy`.
    fn sandbox_policy(&self) -> Value {
        if self.sandboxed {
            json!({"type": "externalSandbox", "networkAccess": "enabled"})
        } else {
            json!({"type": "readOnly"})
        }
    }

    fn start_thread(&mut self, step: &mut DriverStep, resume: bool) {
        let mut params = json!({
            "cwd": self.cwd,
            "approvalPolicy": "untrusted",
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

    fn send_turn(&mut self, step: &mut DriverStep, text: String) {
        let params = json!({
            "threadId": self.thread_id,
            "input": [{"type": "text", "text": text}],
            "approvalPolicy": "untrusted",
            "sandboxPolicy": self.sandbox_policy(),
        });
        let (id, line) = self.request("turn/start", params);
        self.turn_req = Some(id);
        step.write(line);
    }

    fn item(
        &self,
        id: String,
        status: TurnItemStatus,
        title: Option<String>,
        kind: TurnItemKind,
    ) -> TurnItem {
        let now = Utc::now();
        TurnItem {
            id,
            turn_id: self.turn_id.clone(),
            ordinal: 0,
            status,
            title,
            started_at: Some(now),
            completed_at: (!status.is_open()).then_some(now),
            updated_at: now,
            kind,
        }
    }

    fn item_id(&self, codex_id: &str) -> String {
        format!("{}:codex:{codex_id}", self.turn())
    }

    /// A Codex `ThreadItem` → an Otto item (`None` = not projected).
    fn project(&mut self, item: &Value, completed: bool) -> Option<TurnItem> {
        let cid = item.get("id").and_then(Value::as_str)?.to_string();
        let ty = item.get("type").and_then(Value::as_str)?;
        let st = self.items.entry(cid.clone()).or_default();
        let s = |k: &str| item.get(k).and_then(Value::as_str).map(str::to_string);
        let status = match item.get("status").and_then(Value::as_str) {
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
                    item.get(k)
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
                        exit_code: item
                            .get("exitCode")
                            .and_then(Value::as_i64)
                            .map(|c| c as i32),
                    },
                    Some("Command".to_string()),
                )
            }
            "fileChange" => {
                let changes = item
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
                let output = item
                    .get("result")
                    .filter(|v| !v.is_null())
                    .or_else(|| item.get("error").filter(|v| !v.is_null()))
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    });
                (
                    TurnItemKind::DynamicTool {
                        tool_name: name.clone(),
                        input: item.get("arguments").cloned().unwrap_or(Value::Null),
                        output,
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
        if completed {
            self.items.remove(&cid);
        }
        Some(self.item(self.item_id(&cid), status, title, kind))
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
        let item = self.item(self.item_id(cid), TurnItemStatus::Running, None, kind);
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
        let plan_id = format!("{}:plan", self.turn());
        step.push(DriverEvent::Plan(PlanArtifact {
            id: plan_id.clone(),
            turn_id: self.turn_id.clone(),
            steps: steps.clone(),
            explanation: explanation.clone(),
        }));
        step.push(DriverEvent::TurnItem(self.item(
            plan_id.clone(),
            TurnItemStatus::Running,
            None,
            TurnItemKind::TodoList {
                plan_id,
                steps,
                explanation,
            },
        )));
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
        let (proto, kind) = match method {
            "item/commandExecution/requestApproval" => (ReqProto::V2Decision, RequestKind::Command),
            "item/fileChange/requestApproval" => (ReqProto::V2Decision, RequestKind::FileChange),
            "execCommandApproval" => (ReqProto::Legacy, RequestKind::Command),
            "applyPatchApproval" => (ReqProto::Legacy, RequestKind::FileChange),
            "item/permissions/requestApproval" => (ReqProto::Permissions, RequestKind::Permission),
            _ => {
                // requestUserInput, elicitations, dynamic tool calls, auth
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
        let now = Utc::now();
        let request = RuntimeRequest {
            id: request_id.clone(),
            turn_id: self.turn_id.clone(),
            kind,
            status: RuntimeRequestStatus::Pending,
            response_capability: ResponseCapability::Live,
            created_at: now,
            resolved_at: None,
            decision: None,
        };
        let item = self.item(
            format!("{}:approval:{request_id}", self.turn()),
            TurnItemStatus::Waiting,
            Some(title.clone()),
            TurnItemKind::ApprovalRequest {
                request_id: request_id.clone(),
                request_kind: kind,
                prompt: prompt.or(Some(title)),
                reason: s("reason"),
                options,
            },
        );
        self.pending.insert(
            request_id,
            Pending {
                rpc_id,
                proto,
                permissions: params.get("permissions").cloned().unwrap_or(json!({})),
            },
        );
        step.push(DriverEvent::Request { request, item });
    }

    fn answer(p: &Pending, decision: ApprovalDecision) -> String {
        let result = match p.proto {
            ReqProto::V2Decision => json!({"decision": match decision {
                ApprovalDecision::Accept => "accept",
                ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => "acceptForSession",
                ApprovalDecision::Decline => "decline",
                ApprovalDecision::Cancel => "cancel",
            }}),
            ReqProto::Legacy => json!({"decision": match decision {
                ApprovalDecision::Accept => "approved",
                ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => "approved_for_session",
                ApprovalDecision::Decline => "denied",
                ApprovalDecision::Cancel => "abort",
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
                // T3 Code treats "Already initialized" as success (:1779-1784).
                let already = e
                    .get("message")
                    .and_then(Value::as_str)
                    .is_some_and(|m| m.contains("Already initialized"));
                if !already {
                    self.handshake_failed(step, &e);
                    return;
                }
            }
            step.write(json!({"method": "initialized"}).to_string());
            self.start_thread(step, true);
        } else if let Some((tid, resuming)) = self.thread_req.filter(|(t, _)| *t == id) {
            self.thread_req = None;
            if let Some(e) = error {
                if resuming {
                    // A thread Codex cannot resume: start a fresh one.
                    self.resume = None;
                    self.start_thread(step, false);
                    return;
                }
                self.handshake_failed(step, &e);
                return;
            }
            let _ = tid;
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
            if let Some(text) = self.queued.take() {
                self.send_turn(step, text);
            }
        } else if Some(id) == self.turn_req {
            if let Some(e) = error {
                self.turn_req = None;
                step.push(DriverEvent::Terminal {
                    status: TurnTerminalStatus::Failed,
                    failure: Some(failure("provider_error", &e)),
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

    fn handshake_failed(&mut self, step: &mut DriverStep, e: &Value) {
        step.push(DriverEvent::Error(failure("handshake_error", e)));
        if self.queued.take().is_some() {
            step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Failed,
                failure: Some(failure("handshake_error", e)),
                reason: None,
            });
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

fn rpc_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn failure(class: &str, e: &Value) -> ProviderFailure {
    ProviderFailure {
        class: class.into(),
        message: e
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("codex error")
            .to_string(),
        code: e.get("code").map(|c| c.to_string()),
        retryable: None,
    }
}

impl StructuredDriver for CodexDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Codex
    }

    fn launch(&self, _ctx: &LaunchContext) -> LaunchSpec {
        // The model goes in `thread/start` (`codexThreadRuntimeParams`).
        LaunchSpec {
            program: self.program.clone(),
            args: vec!["app-server".into()],
            tail_args: Vec::new(),
            env: Vec::new(),
        }
    }

    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        self.cwd = ctx.cwd.clone();
        self.model = ctx.model.clone();
        self.resume = ctx.resume.clone();
        self.sandboxed = ctx.sandboxed;
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

    fn prompt(&mut self, turn_id: &str, text: &str) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        self.turn_id = Some(turn_id.to_string());
        self.failure = None;
        let mut step = DriverStep::default();
        if self.ready {
            self.send_turn(&mut step, text.to_string());
        } else {
            self.queued = Some(text.to_string());
        }
        Ok(step)
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
        let p = self
            .pending
            .remove(request_id)
            .ok_or_else(|| DriverError(format!("unknown request {request_id}")))?;
        let mut step = DriverStep::default();
        step.write(Self::answer(&p, decision));
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
                // Notifications for another thread on a shared server are
                // not ours (one app-server per Otto session today).
                if let (Some(t), Some(mine)) = (
                    params.get("threadId").and_then(Value::as_str),
                    self.thread_id.as_deref(),
                ) {
                    if t != mine {
                        return step;
                    }
                }
                match m.as_str() {
                    "turn/started" => {
                        if let Some(t) = params.pointer("/turn/id").and_then(Value::as_str) {
                            self.codex_turn = Some(t.to_string());
                        }
                        self.maybe_interrupt(&mut step);
                    }
                    "item/started" | "item/completed" if self.turn_id.is_some() => {
                        if let Some(item) = params.get("item") {
                            if let Some(i) = self.project(item, m == "item/completed") {
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
                    "turn/plan/updated" if self.turn_id.is_some() => {
                        self.on_plan(&mut step, &params)
                    }
                    "thread/tokenUsage/updated" => self.on_usage(&mut step, &params),
                    "error" => {
                        if params.get("willRetry").and_then(Value::as_bool) != Some(true) {
                            if let Some(e) = params.get("error") {
                                self.failure = Some(failure("provider_error", e));
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
                }
            }
            (None, None) => {}
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
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
            session_id: "s".into(),
            cwd: "/w".into(),
            model: None,
            resume: resume.map(str::to_string),
            sandboxed,
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
        let mut d = CodexDriver::new();
        ready(&mut d, true);
        let s = d.prompt("t1", "hi").unwrap();
        let turn = parse(&s.writes[0]);
        assert_eq!(turn["params"]["sandboxPolicy"]["type"], "externalSandbox");
        let mut d = CodexDriver::new();
        ready(&mut d, false);
        let s = d.prompt("t1", "hi").unwrap();
        assert_eq!(
            parse(&s.writes[0])["params"]["sandboxPolicy"]["type"],
            "readOnly"
        );
    }

    #[test]
    fn resume_falls_back_to_a_fresh_thread() {
        let mut d = CodexDriver::new();
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
        let mut d = CodexDriver::new();
        ready(&mut d, false);
        let s = d.prompt("t1", "go").unwrap();
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
        let DriverEvent::Request { request, item } = &s.events[0] else {
            panic!()
        };
        assert_eq!(request.id, "codex-7");
        assert!(
            matches!(&item.kind, TurnItemKind::ApprovalRequest { prompt: Some(p), reason: Some(r), .. } if p == "rm -rf build" && r == "cleanup")
        );
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
    fn interrupt_cancels_requests_and_waits_for_the_codex_turn_id() {
        let mut d = CodexDriver::new();
        ready(&mut d, false);
        let s = d.prompt("t1", "go").unwrap();
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
        // Legacy approvals answer `abort` on cancel.
        feed(
            &mut d,
            json!({"id": "x1", "method": "execCommandApproval", "params": {"callId": "c", "command": ["ls"], "conversationId": "th1", "cwd": "/w", "parsedCmd": []}}),
        );
        let s = d.interrupt();
        assert_eq!(parse(&s.writes[0])["result"]["decision"], "abort");
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
        let mut d = CodexDriver::new();
        ready(&mut d, false);
        let s = feed(
            &mut d,
            json!({"id": 3, "method": "item/tool/requestUserInput", "params": {}}),
        );
        assert_eq!(parse(&s.writes[0])["error"]["code"], -32601);
    }
}
