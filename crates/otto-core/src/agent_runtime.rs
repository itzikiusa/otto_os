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
/// `grok` (`grok agent stdio`) over ACP, `agy` (Antigravity) over its
/// print-mode stream-json.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeProvider {
    Claude,
    Codex,
    Gemini,
    Grok,
    Agy,
    Opencode,
    Pi,
}

impl RuntimeProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Grok => "grok",
            Self::Agy => "agy",
            Self::Opencode => "opencode",
            Self::Pi => "pi",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "grok" => Some(Self::Grok),
            "agy" => Some(Self::Agy),
            "opencode" => Some(Self::Opencode),
            "pi" => Some(Self::Pi),
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
        /// The images and files sent with the prompt (bytes at
        /// `GET /runtime-sessions/{id}/attachments/{attachment_id}`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
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
        /// The turn item id of the tool call this request gates.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_item_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_reason: Option<CancelReason>,
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
    /// A failed tool's error text (about 2 KB at most).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<CancelReason>,
}

/// Why a request was cancelled: the provider withdrew it (`agent` — Claude
/// does when a parallel sibling ends), a person interrupted the turn, or the
/// session was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    Agent,
    Interrupt,
    Closed,
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
    pub access_mode: RuntimeAccessMode,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub last_sequence: u64,
    /// The ROOT thread of this thread's linked group (set when an agent
    /// opened it from another thread); `None` for a root / unlinked thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Id>,
    /// The agent-UI-control grant record (`{enabled, by, at}` — the same shape
    /// terminal sessions keep in `meta.ui_control`); `None` = never decided.
    /// Written only by `POST /sessions/{id}/ui-control`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_control: Option<serde_json::Value>,
}

impl RuntimeSession {
    /// This thread in terminal-session shape, for surfaces keyed by session
    /// (agent UI control): `meta.runtime = "structured"`, its grant under
    /// `meta.ui_control`, no device pin (any of the owner's windows).
    pub fn into_session(self) -> crate::domain::Session {
        let mut meta = serde_json::json!({"runtime": "structured"});
        if let Some(grant) = self.ui_control {
            meta["ui_control"] = grant;
        }
        crate::domain::Session {
            id: self.id,
            workspace_id: self.workspace_id,
            kind: crate::domain::SessionKind::Agent,
            provider: self.provider.as_str().to_string(),
            title: self.title,
            status: crate::domain::SessionStatus::Running,
            cwd: self.cwd,
            provider_session_id: self.native_thread_id,
            connection_id: None,
            created_by: self.created_by,
            created_at: self.created_at,
            last_active_at: self.last_active_at,
            archived: false,
            meta,
        }
    }
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
    #[serde(default)]
    pub access_mode: Option<RuntimeAccessMode>,
}

/// `POST /runtime-sessions/{id}/peers` — an agent (or its owner) opens a
/// linked thread: same workspace, cwd and access mode as `{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenPeerReq {
    pub provider: RuntimeProvider,
    /// The new thread's opening message (framed with who sent it).
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// Omitted = the provider's default model.
    #[serde(default)]
    pub model: Option<String>,
    /// Reasoning effort for the opening turn (`low` … `max`); omitted = the
    /// model's default.
    #[serde(default)]
    pub effort: Option<String>,
}

/// `POST /runtime-sessions/{id}/peers/{peer}/message`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeerMessageReq {
    pub text: String,
}

/// One member of a linked group, as an agent sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: Id,
    pub provider: RuntimeProvider,
    pub title: String,
    pub status: RuntimeSessionStatus,
    /// The caller itself.
    #[serde(default)]
    pub is_self: bool,
}

/// `GET /runtime-sessions/{id}/peers/{peer}/reply` — the peer's latest
/// assistant text (its last turn), after an optional wait for it to settle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerReply {
    pub peer: PeerInfo,
    /// True while the peer is still working its turn (the wait timed out).
    pub busy: bool,
    /// The last turn's final status (`completed` / `failed` / …), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_status: Option<String>,
    /// Its assistant messages, joined.
    pub text: String,
}

/// T3 Code's `RuntimeMode` (its `auto` mode is not offered): what the agent
/// may do without asking. Otto's Seatbelt, folder trust and prompt guard
/// apply in every mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeAccessMode {
    #[default]
    ApprovalRequired,
    AutoAcceptEdits,
    FullAccess,
}

impl RuntimeAccessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApprovalRequired => "approval-required",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::FullAccess => "full-access",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::ApprovalRequired,
            Self::AutoAcceptEdits,
            Self::FullAccess,
        ]
        .into_iter()
        .find(|m| m.as_str() == s)
    }
}

/// `PATCH /runtime-sessions/{id}` (rename; T3 Code `thread.metadata-updated`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRuntimeSessionReq {
    pub title: String,
}

/// `POST /runtime-sessions/{id}/prompt`. `model` / `effort` select this
/// turn's model (it sticks for later turns); `effort` is Claude / Codex only.
/// `access_mode` applies from this turn on and sticks, like `model`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimePromptReq {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PromptImage>,
    /// Non-image files (only `application/pdf`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PromptFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_mode: Option<RuntimeAccessMode>,
}

impl RuntimePromptReq {
    /// A text-only prompt.
    pub fn of(text: &str) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

/// An inline image attachment (`mime` is `image/*`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptImage {
    pub mime: String,
    pub data_base64: String,
}

/// A file attachment (`mime` is `application/pdf`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFile {
    pub mime: String,
    pub name: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
}

/// A stored prompt attachment, referenced from its `user_message` item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Id,
    pub kind: AttachmentKind,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
            error: None,
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
                    cancel_reason: Some(CancelReason::Interrupt),
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
        assert_eq!(
            serde_json::to_value(RuntimeAccessMode::AutoAcceptEdits).unwrap(),
            json!("auto-accept-edits")
        );
        for m in [
            RuntimeAccessMode::ApprovalRequired,
            RuntimeAccessMode::AutoAcceptEdits,
            RuntimeAccessMode::FullAccess,
        ] {
            assert_eq!(RuntimeAccessMode::parse(m.as_str()), Some(m));
            assert_eq!(serde_json::to_value(m).unwrap(), json!(m.as_str()));
        }
    }
}
