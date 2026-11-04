//! Structured agent runtime — the normalized provider-runtime event model and
//! REST DTOs (design `docs/features/threads-structured-runtime.md`).
//!
//! Ported from T3 Code's provider runtime model (MIT, Copyright (c) 2026 T3
//! Tools Inc.): `ProviderAdapterV2Event` in
//! `packages/provider-core/src/server/ProviderAdapter.ts` and the entities in
//! `packages/contracts/src/orchestrationV2.ts`. Same shape: adapters do not
//! stream deltas, they re-emit the WHOLE current state of an entity (a turn
//! item with its accumulated text and a `streaming` flag, a runtime request
//! with its status, …); `command_output.delta` is the one true delta. Field
//! names are snake_case (Otto's wire convention) where T3 Code uses camelCase;
//! event `type` tags are T3 Code's. Subset: no app threads, nodes, subagents,
//! background work, checkpoints or handoffs.
//!
//! Every event carries the Otto session id, a per-session `sequence` (strictly
//! increasing, never reused, also across daemon restarts) and a timestamp. A
//! client resumes with `GET /runtime-sessions/{id}/events?after_sequence=`:
//! the events since then, or — past T3 Code's replay limits — a snapshot of
//! every entity.
//!
//! TS mirror: `ui/src/lib/api/types/agent-runtime.ts`; contract:
//! `docs/contracts/api.md` "Structured agent runtime" + ws.md
//! `agent_runtime_event`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Id;

/// The agent CLIs the structured runtime can drive (distinct from the PTY
/// provider registry): `claude` over the Agent SDK's stream-json control
/// protocol, `codex` over `codex app-server`, `gemini` (`gemini --acp`) and
/// `grok` (`grok agent stdio`) over ACP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProvider {
    Claude,
    Codex,
    Gemini,
    Grok,
}

impl RuntimeProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "grok" => Some(Self::Grok),
            _ => None,
        }
    }
}

// ---- entities (T3 Code `OrchestrationV2*`) ---------------------------------

/// `OrchestrationV2ProviderSession.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSessionStatus {
    Starting,
    Ready,
    Running,
    Waiting,
    Stopped,
    Error,
}

/// The provider process / protocol session (`OrchestrationV2ProviderSession`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderSession {
    pub status: ProviderSessionStatus,
    pub cwd: String,
    pub model: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub last_error: Option<String>,
}

/// `ThreadTokenUsageSnapshot` (context-window usage).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadTokenUsage {
    pub used_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

/// The provider's own conversation (`OrchestrationV2ProviderThread`): its
/// native id is the resume key (Claude session id, Codex thread id, ACP
/// session id).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderThread {
    pub native_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ThreadTokenUsage>,
    pub updated_at: DateTime<Utc>,
}

/// `OrchestrationV2ProviderTurn.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderTurnStatus {
    Pending,
    Running,
    Completed,
    Interrupted,
    Failed,
    Cancelled,
}

impl ProviderTurnStatus {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

/// `TurnTokenUsage` (the turn's own usage; input includes cache reads).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
}

/// One prompt → response cycle (`OrchestrationV2ProviderTurn`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderTurn {
    pub id: Id,
    /// 1-based position in the session.
    pub ordinal: u32,
    pub status: ProviderTurnStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_token_usage: Option<TurnTokenUsage>,
}

/// `OrchestrationV2TurnItemStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnItemStatus {
    Idle,
    Pending,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl TurnItemStatus {
    pub fn is_open(self) -> bool {
        matches!(
            self,
            Self::Idle | Self::Pending | Self::Running | Self::Waiting
        )
    }
}

/// `OrchestrationV2PlanStep.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    Running,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub text: String,
    pub status: PlanStepStatus,
}

/// `ProviderRequestKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestKind {
    Command,
    FileRead,
    FileChange,
    McpElicitation,
    Permission,
}

/// `ProviderApprovalDecision` — what a person answers, provider-neutral. Each
/// driver maps it onto its protocol (ACP option kinds, Codex decisions, the
/// Claude permission result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Accept,
    AcceptForSession,
    AcceptAlways,
    Decline,
    Cancel,
}

/// `ProviderApprovalOption`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalOption {
    pub decision: ApprovalDecision,
    pub label: String,
}

/// `OrchestrationV2ProviderFailure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFailure {
    /// e.g. `provider_error`, `transport_error`, `server_restart`.
    pub class: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
}

/// The type-specific part of a turn item (`OrchestrationV2TurnItem`, subset).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnItemKind {
    UserMessage {
        text: String,
    },
    AssistantMessage {
        message_id: String,
        text: String,
        streaming: bool,
    },
    Reasoning {
        text: String,
        streaming: bool,
    },
    TodoList {
        plan_id: String,
        steps: Vec<PlanStep>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explanation: Option<String>,
    },
    FileChange {
        file_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff_str: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old_str: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_str: Option<String>,
    },
    CommandExecution {
        input: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
    DynamicTool {
        tool_name: String,
        #[serde(default)]
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
    /// Web search / fetch tools (Claude `WebSearch` / `WebFetch`, Codex
    /// `webSearch`); `patterns` are the queries or URLs.
    WebSearch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        patterns: Vec<String>,
    },
    ApprovalRequest {
        request_id: Id,
        request_kind: RequestKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        /// The provider's own justification (Codex `reason`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        options: Vec<ApprovalOption>,
    },
    SystemNotice {
        message: String,
    },
    Error {
        failure: ProviderFailure,
    },
}

/// One timeline item (`OrchestrationV2TurnItem`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnItem {
    pub id: Id,
    pub turn_id: Option<Id>,
    /// Display order within the session (assigned by the runtime).
    pub ordinal: u32,
    pub status: TurnItemStatus,
    pub title: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[serde(flatten)]
    pub kind: TurnItemKind,
}

impl TurnItem {
    /// Still accumulating streamed text (not persisted until it settles).
    pub fn is_streaming(&self) -> bool {
        matches!(
            self.kind,
            TurnItemKind::AssistantMessage {
                streaming: true,
                ..
            } | TurnItemKind::Reasoning {
                streaming: true,
                ..
            }
        )
    }
}

/// `OrchestrationV2RuntimeRequest.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRequestStatus {
    Pending,
    Resolved,
    Expired,
    Cancelled,
}

/// `OrchestrationV2RuntimeRequest.responseCapability`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseCapability {
    /// The live provider process can take the answer.
    Live,
    NotResumable {
        reason: String,
    },
}

/// An approval the agent is blocked on (`OrchestrationV2RuntimeRequest`).
/// Its options ride the matching `approval_request` turn item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRequest {
    pub id: Id,
    pub turn_id: Option<Id>,
    pub kind: RequestKind,
    pub status: RuntimeRequestStatus,
    pub response_capability: ResponseCapability,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ApprovalDecision>,
}

/// `OrchestrationV2PlanArtifact` (`todo_list` form).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanArtifact {
    pub id: String,
    pub turn_id: Option<Id>,
    pub steps: Vec<PlanStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
}

/// `turn.terminal` status (the ACP/Codex/Claude end-of-turn, normalized).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnTerminalStatus {
    Completed,
    Interrupted,
    Cancelled,
    Failed,
}

impl TurnTerminalStatus {
    pub fn turn_status(self) -> ProviderTurnStatus {
        match self {
            Self::Completed => ProviderTurnStatus::Completed,
            Self::Interrupted => ProviderTurnStatus::Interrupted,
            Self::Cancelled => ProviderTurnStatus::Cancelled,
            Self::Failed => ProviderTurnStatus::Failed,
        }
    }

    /// What a still-open item of the turn settles to.
    pub fn item_status(self) -> TurnItemStatus {
        match self {
            Self::Completed => TurnItemStatus::Completed,
            Self::Interrupted => TurnItemStatus::Interrupted,
            Self::Cancelled => TurnItemStatus::Cancelled,
            Self::Failed => TurnItemStatus::Failed,
        }
    }
}

// ---- events (T3 Code `ProviderAdapterV2Event`) ---------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuntimeEventBody {
    #[serde(rename = "provider_session.updated")]
    ProviderSessionUpdated { provider_session: ProviderSession },
    #[serde(rename = "provider_thread.updated")]
    ProviderThreadUpdated { provider_thread: ProviderThread },
    #[serde(rename = "provider_turn.updated")]
    ProviderTurnUpdated { provider_turn: ProviderTurn },
    #[serde(rename = "turn_item.updated")]
    TurnItemUpdated { turn_item: TurnItem },
    #[serde(rename = "runtime_request.updated")]
    RuntimeRequestUpdated { runtime_request: RuntimeRequest },
    #[serde(rename = "plan.updated")]
    PlanUpdated { plan: PlanArtifact },
    /// Live command output (never persisted; the item's `output` holds the total).
    #[serde(rename = "command_output.delta")]
    CommandOutputDelta { item_id: Id, chunk: String },
    #[serde(rename = "turn.terminal")]
    TurnTerminal {
        turn_id: Id,
        status: TurnTerminalStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<ProviderFailure>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl RuntimeEventBody {
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::ProviderSessionUpdated { .. } => "provider_session.updated",
            Self::ProviderThreadUpdated { .. } => "provider_thread.updated",
            Self::ProviderTurnUpdated { .. } => "provider_turn.updated",
            Self::TurnItemUpdated { .. } => "turn_item.updated",
            Self::RuntimeRequestUpdated { .. } => "runtime_request.updated",
            Self::PlanUpdated { .. } => "plan.updated",
            Self::CommandOutputDelta { .. } => "command_output.delta",
            Self::TurnTerminal { .. } => "turn.terminal",
        }
    }
}

/// One sequenced event (ring, replay, WS `agent_runtime_event`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub session_id: Id,
    pub sequence: u64,
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub body: RuntimeEventBody,
}

/// Every entity's latest state — what a client starts from when the events
/// since its `after_sequence` are no longer (or too many to be) replayable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub provider_session: Option<ProviderSession>,
    pub provider_thread: Option<ProviderThread>,
    pub turns: Vec<ProviderTurn>,
    /// Ordered by `ordinal`.
    pub turn_items: Vec<TurnItem>,
    pub runtime_requests: Vec<RuntimeRequest>,
    pub plans: Vec<PlanArtifact>,
}

/// `GET /runtime-sessions/{id}/events` (T3 Code `subscribeThread` items).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeReplay {
    /// Exactly the events after `after_sequence`.
    Events {
        events: Vec<RuntimeEvent>,
        last_sequence: u64,
    },
    /// Replace everything with this; continue after `snapshot_sequence`.
    Snapshot {
        snapshot_sequence: u64,
        snapshot: Box<RuntimeSnapshot>,
    },
}

// ---- sessions + REST DTOs ---------------------------------------------------

/// Otto's summary status of a runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSessionStatus {
    Starting,
    Idle,
    Working,
    AwaitingPermission,
    /// No process; the next prompt respawns it and resumes `native_thread_id`.
    Dormant,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSession {
    pub id: Id,
    pub workspace_id: Id,
    pub provider: RuntimeProvider,
    pub title: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The provider's own session / thread id (resume key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_thread_id: Option<String>,
    pub status: RuntimeSessionStatus,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub last_sequence: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateRuntimeSessionReq {
    pub provider: Option<RuntimeProvider>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

/// `PATCH /runtime-sessions/{id}` (rename; T3 Code `thread.metadata-updated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRuntimeSessionReq {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePromptReq {
    pub text: String,
    /// Model for this turn and the ones after it (T3 Code's per-turn model
    /// selection: Codex `turn/start {model}`, Claude `set_model`). `None`
    /// keeps the session's model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning effort for this turn and later ones (`low` … `max`);
    /// providers without an effort ignore it. `None` keeps the current one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Images sent with the text (Claude `image` content blocks, Codex image
    /// input, ACP `image` content).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PromptImage>,
}

/// One prompt image, inline: `image/*` media type and base64 bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptImage {
    pub media_type: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePromptResp {
    pub turn_id: Id,
    /// `sequence` of the turn's first `provider_turn.updated`.
    pub sequence: u64,
}

/// `POST /runtime-sessions/{id}/requests/{request_id}` (T3 Code
/// `runtime-request.respond`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeRequestRespondReq {
    pub decision: ApprovalDecision,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-04T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn events_keep_t3_type_tags_and_flatten_the_envelope() {
        let item = TurnItem {
            id: "i1".into(),
            turn_id: Some("t1".into()),
            ordinal: 3,
            status: TurnItemStatus::Running,
            title: None,
            started_at: None,
            completed_at: None,
            updated_at: ts(),
            kind: TurnItemKind::AssistantMessage {
                message_id: "m1".into(),
                text: "Hel".into(),
                streaming: true,
            },
        };
        let ev = RuntimeEvent {
            session_id: "s1".into(),
            sequence: 7,
            ts: ts(),
            body: RuntimeEventBody::TurnItemUpdated { turn_item: item },
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "turn_item.updated");
        assert_eq!(v["sequence"], 7);
        assert_eq!(v["turn_item"]["type"], "assistant_message");
        assert_eq!(v["turn_item"]["streaming"], true);
        let back: RuntimeEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn every_event_round_trips_and_matches_type_name() {
        let bodies = vec![
            RuntimeEventBody::ProviderSessionUpdated {
                provider_session: ProviderSession {
                    status: ProviderSessionStatus::Ready,
                    cwd: "/w".into(),
                    model: None,
                    updated_at: ts(),
                    last_error: None,
                },
            },
            RuntimeEventBody::ProviderThreadUpdated {
                provider_thread: ProviderThread {
                    native_thread_id: Some("n".into()),
                    context_usage: Some(ThreadTokenUsage {
                        used_tokens: 5,
                        ..Default::default()
                    }),
                    updated_at: ts(),
                },
            },
            RuntimeEventBody::ProviderTurnUpdated {
                provider_turn: ProviderTurn {
                    id: "t1".into(),
                    ordinal: 1,
                    status: ProviderTurnStatus::Running,
                    started_at: Some(ts()),
                    completed_at: None,
                    turn_token_usage: None,
                },
            },
            RuntimeEventBody::RuntimeRequestUpdated {
                runtime_request: RuntimeRequest {
                    id: "r1".into(),
                    turn_id: Some("t1".into()),
                    kind: RequestKind::FileChange,
                    status: RuntimeRequestStatus::Expired,
                    response_capability: ResponseCapability::NotResumable {
                        reason: "restart".into(),
                    },
                    created_at: ts(),
                    resolved_at: None,
                    decision: Some(ApprovalDecision::AcceptForSession),
                },
            },
            RuntimeEventBody::PlanUpdated {
                plan: PlanArtifact {
                    id: "p".into(),
                    turn_id: None,
                    steps: vec![PlanStep {
                        id: "1".into(),
                        text: "x".into(),
                        status: PlanStepStatus::Running,
                    }],
                    explanation: None,
                },
            },
            RuntimeEventBody::CommandOutputDelta {
                item_id: "i".into(),
                chunk: "out".into(),
            },
            RuntimeEventBody::TurnTerminal {
                turn_id: "t1".into(),
                status: TurnTerminalStatus::Failed,
                failure: Some(ProviderFailure {
                    class: "transport_error".into(),
                    message: "exited".into(),
                    code: None,
                    retryable: None,
                }),
                reason: None,
            },
        ];
        for body in bodies {
            let ev = RuntimeEvent {
                session_id: "s".into(),
                sequence: 1,
                ts: ts(),
                body: body.clone(),
            };
            let v = serde_json::to_value(&ev).unwrap();
            assert_eq!(v["type"], body.type_name(), "{v}");
            assert_eq!(serde_json::from_value::<RuntimeEvent>(v).unwrap(), ev);
        }
    }

    #[test]
    fn replay_is_kind_tagged_and_decisions_snake_case() {
        let r = RuntimeReplay::Events {
            events: vec![],
            last_sequence: 4,
        };
        assert_eq!(
            serde_json::to_value(&r).unwrap(),
            json!({"kind":"events","events":[],"last_sequence":4})
        );
        assert_eq!(
            serde_json::to_value(ApprovalDecision::AcceptForSession).unwrap(),
            json!("accept_for_session")
        );
    }
}
