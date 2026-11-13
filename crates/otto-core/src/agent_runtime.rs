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
//! event `type` tags are T3 Code's. Subset: no app threads, nodes,
//! checkpoints or handoffs. Background work is the provider session's
//! [`BackgroundTask`] list; a subagent is one `subagent` item whose activity
//! items point back at it ([`TurnItem::parent_item_id`]).
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
    pub const ALL: [Self; 7] = [
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Grok,
        Self::Agy,
        Self::Opencode,
        Self::Pi,
    ];

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
    /// Work the agent started in the background that has not finished yet
    /// (a `run_in_background` command, a background subagent). The session
    /// is busy while any is pending, even between turns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_tasks: Vec<BackgroundTask>,
    /// What the running driver reported it takes (after its handshake).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<RuntimeCapabilities>,
}

/// One piece of background work an agent started (Claude Code's
/// `task_started` … `task_notification`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundTask {
    /// The provider's task id.
    pub id: String,
    /// `command`, `agent`, or the provider's own task type.
    pub kind: String,
    /// What it is doing (the command, the subagent's description).
    pub label: String,
    pub started_at: DateTime<Utc>,
    /// The turn item of the tool call that started it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<Id>,
}

/// Who started a turn: a person's prompt, or the agent itself (Claude Code
/// continues on its own when background work it started finishes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOrigin {
    #[default]
    User,
    Agent,
}

impl TurnOrigin {
    pub fn is_user(&self) -> bool {
        *self == Self::User
    }
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
    /// `agent` = a turn the agent started on its own (no user message).
    #[serde(default, skip_serializing_if = "TurnOrigin::is_user")]
    pub origin: TurnOrigin,
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
    Question,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<ToolOutputImage>,
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
    /// A fresh provider conversation began in this same visible thread.
    SessionBoundary,
    SystemNotice {
        message: String,
    },
    /// A subagent the agent launched (Claude's `Agent` / `Task` tool). Its
    /// own messages and tool calls are items whose `parent_item_id` is this
    /// item; a background one stays `running` past its turn until its task
    /// finishes.
    Subagent {
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt: Option<String>,
        /// Its final answer (the tool result / task summary).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default)]
        background: bool,
        /// The tool it used last, while it runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_tool: Option<String>,
        #[serde(default)]
        tool_uses: u32,
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
    /// The `subagent` item this item belongs to (its activity), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_item_id: Option<Id>,
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

    /// A background subagent still running — it outlives its turn, so a
    /// turn's end does not settle it.
    pub fn is_background_open(&self) -> bool {
        self.status.is_open()
            && matches!(
                self.kind,
                TurnItemKind::Subagent {
                    background: true,
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

mod questions;
pub use questions::{
    validate_question_answers, QuestionAnswer, QuestionKind, QuestionOption, RuntimeQuestion,
};

/// An approval or structured question the agent is blocked on (`OrchestrationV2RuntimeRequest`).
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
    /// Full form data lives outside the compact timeline/tool projection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<RuntimeQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<Vec<QuestionAnswer>>,
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
    /// The messages waiting for the running turn to end, whole (persisted:
    /// they survive a daemon restart and go out one per turn, oldest first).
    #[serde(rename = "queue.updated")]
    QueueUpdated { queued_prompts: Vec<QueuedPrompt> },
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
            Self::QueueUpdated { .. } => "queue.updated",
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
    /// Messages queued behind the running turn, oldest first.
    #[serde(default)]
    pub queued_prompts: Vec<QueuedPrompt>,
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

/// What a thread's agent can take besides a plain text prompt (T3 Code's
/// `ServerProvider` snapshot). A provider baseline ([`Self::for_provider`])
/// that the running driver refines after its handshake (an ACP agent's
/// `promptCapabilities`). The UI gates Send now, attachments and effort on
/// it instead of on the provider's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RuntimeCapabilities {
    /// A message can join the RUNNING turn (`delivery: steer`). Otherwise it
    /// waits in the queue until the turn ends.
    #[serde(default)]
    pub steer: bool,
    #[serde(default)]
    pub images: bool,
    /// PDFs (document / embedded-resource blocks).
    #[serde(default)]
    pub files: bool,
    /// Reasoning-effort levels a prompt's `effort` may carry; empty = none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
}

impl RuntimeCapabilities {
    /// What each driver takes before it reports more (or less) itself.
    pub fn for_provider(p: RuntimeProvider) -> Self {
        let levels = |l: &[&str]| l.iter().map(|s| s.to_string()).collect();
        match p {
            RuntimeProvider::Claude => Self {
                steer: true,
                images: true,
                files: true,
                efforts: levels(&["low", "medium", "high", "xhigh", "max"]),
            },
            RuntimeProvider::Codex => Self {
                steer: true,
                images: true,
                files: false,
                efforts: levels(&["minimal", "low", "medium", "high", "xhigh"]),
            },
            RuntimeProvider::Pi => Self {
                steer: true,
                images: true,
                files: false,
                efforts: levels(&["minimal", "low", "medium", "high", "xhigh"]),
            },
            // ACP v1 has no way into a running prompt.
            RuntimeProvider::Agy => Self {
                steer: false,
                images: true,
                files: true,
                efforts: levels(&["low", "medium", "high"]),
            },
            RuntimeProvider::Grok => Self {
                steer: false,
                images: true,
                files: true,
                efforts: levels(&["low", "medium", "high"]),
            },
            RuntimeProvider::Gemini | RuntimeProvider::Opencode => Self {
                steer: false,
                images: true,
                files: true,
                efforts: Vec::new(),
            },
        }
    }
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
    /// Background work still pending (live only; see
    /// [`ProviderSession::background_tasks`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_tasks: Vec<BackgroundTask>,
    /// The server-owned meta this thread runs under — the `sessions.meta`
    /// keys the spawn's safety pipeline and the tool policy read
    /// (`personal_agent`, `read_only`, `source`, `run_id`, `browser`, …).
    /// Set once at create by the daemon (a personal agent's run or chat);
    /// never by a client. `None` = a plain thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    /// What its agent takes (the provider baseline until a running driver
    /// reports its own; see [`ProviderSession::capabilities`]).
    #[serde(default)]
    pub capabilities: RuntimeCapabilities,
}

impl RuntimeSession {
    /// A boolean flag in [`Self::meta`] (`read_only`, …); absent = false.
    pub fn meta_flag(&self, key: &str) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }

    /// This thread in terminal-session shape, for surfaces keyed by session
    /// (agent UI control): `meta.runtime = "structured"`, its grant under
    /// `meta.ui_control`, no device pin (any of the owner's windows).
    pub fn into_session(self) -> crate::domain::Session {
        let mut meta = serde_json::json!({"runtime": "structured"});
        if let Some(serde_json::Value::Object(extra)) = self.meta {
            for (k, v) in extra {
                if k != "runtime" && k != "ui_control" {
                    meta[k] = v;
                }
            }
        }
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
    /// Server-owned [`RuntimeSession::meta`]. Never read from a request body
    /// (`serde(skip)`): only the daemon's own callers set it.
    #[serde(skip)]
    pub meta: Option<serde_json::Value>,
}

/// The `meta.source` of a thread a person opened INSIDE another page (a PR
/// review's Ask / Explain / Fix, a broken connection's diagnosis) rather than
/// from Threads. The only sources a client may name (`POST
/// …/runtime-sessions` `source`); Threads folds them into their own sections
/// (UI mirror: `EMBEDDED_THREAD_SOURCES` in `ui/src/modules/threads/threadOrigins.ts`)
/// and the runtime closes them after `embedded_thread_auto_close_hours` idle.
pub const EMBEDDED_THREAD_SOURCES: [&str; 2] = ["pr-agent", "connection-diagnose"];

/// The `origin.kind` each embedded source must carry.
fn embedded_origin_kind(source: &str) -> Option<&'static str> {
    match source {
        "pr-agent" => Some("pr"),
        "connection-diagnose" => Some("connection"),
        _ => None,
    }
}

/// Longest `origin.id` / `origin.label`, in characters.
pub const EMBEDDED_ORIGIN_MAX_CHARS: usize = 200;

/// What an embedded thread is about (`meta.origin`): the PR or connection it
/// was opened for, so the page finds it again (`GET …/runtime-sessions?
/// source=&origin_id=`). `kind` / `id` default to empty so a missing one is a
/// 400 from [`CreateRuntimeSessionBody::into_req`], not a serde 422.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddedOrigin {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `POST /workspaces/{id}/runtime-sessions` body: the create fields plus the
/// two a client may set on the server-owned meta. Nothing else in the body
/// reaches `meta` (`CreateRuntimeSessionReq::meta` stays `serde(skip)`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateRuntimeSessionBody {
    #[serde(flatten)]
    pub req: CreateRuntimeSessionReq,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub origin: Option<EmbeddedOrigin>,
}

impl CreateRuntimeSessionBody {
    /// The create request, its meta `{source, origin}` when the body names an
    /// embedded source. Refuses (400) a source outside
    /// [`EMBEDDED_THREAD_SOURCES`], an `origin` without a `source`, an origin
    /// kind that does not match the source, and an empty or over-long id /
    /// over-long label.
    pub fn into_req(self) -> crate::Result<CreateRuntimeSessionReq> {
        let mut req = self.req;
        req.meta = embedded_meta(self.source.as_deref(), self.origin)?;
        Ok(req)
    }
}

/// The meta an embedded thread is created with (`None` = a plain thread);
/// see [`CreateRuntimeSessionBody::into_req`].
pub fn embedded_meta(
    source: Option<&str>,
    origin: Option<EmbeddedOrigin>,
) -> crate::Result<Option<Value>> {
    let invalid = crate::Error::Invalid;
    let Some(source) = source else {
        return match origin {
            Some(_) => Err(invalid("`origin` needs a `source`".into())),
            None => Ok(None),
        };
    };
    let Some(kind) = embedded_origin_kind(source) else {
        return Err(invalid(format!(
            "unknown thread source '{source}' (expected one of: {})",
            EMBEDDED_THREAD_SOURCES.join(", ")
        )));
    };
    let mut meta = serde_json::Map::new();
    meta.insert("source".into(), Value::String(source.into()));
    if let Some(mut o) = origin {
        if o.kind != kind {
            return Err(invalid(format!(
                "a '{source}' thread's origin kind must be '{kind}', not '{}'",
                o.kind
            )));
        }
        let id_len = o.id.chars().count();
        if o.id.trim().is_empty() || id_len > EMBEDDED_ORIGIN_MAX_CHARS {
            return Err(invalid(format!(
                "origin id must be 1–{EMBEDDED_ORIGIN_MAX_CHARS} characters"
            )));
        }
        if o.label
            .as_ref()
            .is_some_and(|l| l.chars().count() > EMBEDDED_ORIGIN_MAX_CHARS)
        {
            return Err(invalid(format!(
                "origin label must be at most {EMBEDDED_ORIGIN_MAX_CHARS} characters"
            )));
        }
        o.label = o.label.filter(|l| !l.trim().is_empty());
        meta.insert(
            "origin".into(),
            serde_json::to_value(o).map_err(|e| crate::Error::Internal(e.to_string()))?,
        );
    }
    Ok(Some(Value::Object(meta)))
}

/// Whether a thread's meta names an embedded source ([`EMBEDDED_THREAD_SOURCES`]).
pub fn is_embedded_thread(meta: Option<&Value>) -> bool {
    meta.and_then(|m| m.get("source"))
        .and_then(Value::as_str)
        .is_some_and(|s| EMBEDDED_THREAD_SOURCES.contains(&s))
}

/// The daemon setting (`PUT /settings`) after how many idle hours an open
/// embedded thread is closed (kept in History, reopenable): absent or not a
/// non-negative integer = the default (12), `0` = never.
pub const EMBEDDED_THREAD_AUTO_CLOSE_KEY: &str = "embedded_thread_auto_close_hours";
pub const EMBEDDED_THREAD_AUTO_CLOSE_DEFAULT_HOURS: u64 = 12;

/// The auto-close setting's hours (see [`EMBEDDED_THREAD_AUTO_CLOSE_KEY`]).
pub fn embedded_auto_close_hours(setting: Option<&Value>) -> u64 {
    setting
        .and_then(Value::as_u64)
        .unwrap_or(EMBEDDED_THREAD_AUTO_CLOSE_DEFAULT_HOURS)
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
    /// Daemon-only (`serde(skip)`): the linked thread's own folder (default:
    /// the opener's) and its [`RuntimeSession::meta`] — a personal agent the
    /// main agent delegates to runs in ITS persona folder, as itself.
    #[serde(skip)]
    pub cwd: Option<String>,
    #[serde(skip)]
    pub meta: Option<serde_json::Value>,
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
    /// True while the peer is still working (the wait timed out): a turn —
    /// the person's or one it started itself — or background work.
    pub busy: bool,
    /// Background tasks still pending.
    #[serde(default)]
    pub background_tasks: u32,
    /// The last turn's final status (`completed` / `failed` / …), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_status: Option<String>,
    /// The assistant messages of its last turn that has any, joined.
    pub text: String,
}

// ---- threads inside a thread: a person asks another agent ------------------

/// The [`RuntimeSession::meta`] key of a thread opened FROM another one
/// ([`ChildOf`]): who opened it, so its finished turns reach that thread.
pub const CHILD_META_KEY: &str = "child_of";
/// Agents one "Ask several" sends the same message to.
pub const ASK_CHILD_MAX_TARGETS: usize = 4;

/// Who opened a child thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildOpener {
    /// A person, from the thread's composer ("Ask another agent").
    Person,
    /// The thread's own agent (`otto.thread_open`): the child's answers to
    /// it are delivered back to that agent.
    Agent,
}

/// `meta.child_of`: the thread this one was opened from (not the group's
/// root — that is `parent_id`). Server-owned like the rest of meta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildOf {
    pub id: Id,
    pub by: ChildOpener,
}

impl RuntimeSession {
    /// The thread this one was opened from, if any.
    pub fn child_of(&self) -> Option<ChildOf> {
        let v = self.meta.as_ref()?.get(CHILD_META_KEY)?;
        serde_json::from_value(v.clone()).ok()
    }
}

/// One agent of `POST /runtime-sessions/{id}/children`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskChildTarget {
    pub provider: RuntimeProvider,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

/// `POST /runtime-sessions/{id}/children` — a person sends `prompt` to 1–4
/// NEW threads linked to `{id}` (its folder; its access mode or a narrower
/// one), created all or none.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AskChildReq {
    pub prompt: String,
    pub targets: Vec<AskChildTarget>,
    #[serde(default)]
    pub title: Option<String>,
    /// Omitted = the parent's. Wider than the parent's is refused (400).
    #[serde(default)]
    pub access_mode: Option<RuntimeAccessMode>,
    #[serde(default)]
    pub images: Vec<PromptImage>,
}

/// The children `POST /runtime-sessions/{id}/children` opened, in target order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskChildResp {
    pub children: Vec<RuntimeSession>,
}

// ---- council: one prompt, several agents, then a synthesis ------------------

/// Seats one council holds (a person compares 2–4 answers side by side).
pub const COUNCIL_MIN_SEATS: usize = 2;
pub const COUNCIL_MAX_SEATS: usize = 4;
/// The [`RuntimeSession::meta`] key every council thread carries
/// ([`CouncilMeta`]).
pub const COUNCIL_META_KEY: &str = "council";

/// One seat of `POST /workspaces/{id}/runtime-sessions/council`. The same
/// provider may sit more than once (two efforts, two models).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilSeatReq {
    pub provider: RuntimeProvider,
    #[serde(default)]
    pub model: Option<String>,
    /// Reasoning effort for the seat's first turn; omitted = the model's default.
    #[serde(default)]
    pub effort: Option<String>,
}

/// `POST /workspaces/{id}/runtime-sessions/council` — one prompt to 2–4 agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateCouncilReq {
    pub prompt: String,
    #[serde(default)]
    pub title: Option<String>,
    /// Every seat's folder; omitted = the workspace root.
    #[serde(default)]
    pub cwd: Option<String>,
    pub seats: Vec<CouncilSeatReq>,
    #[serde(default)]
    pub images: Vec<PromptImage>,
}

/// The council's seats, seat 1 first (seat 1 is the group's root thread).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CouncilResp {
    pub seats: Vec<RuntimeSession>,
}

/// `POST /runtime-sessions/{id}/council/synthesize` — the judge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SynthesizeCouncilReq {
    pub provider: RuntimeProvider,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CouncilRole {
    Seat,
    Synthesis,
}

/// `meta.council` of a council thread: which council, which seat (1-based;
/// `0` for the synthesis) and its role. Server-owned like the rest of meta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CouncilMeta {
    pub id: Id,
    pub seat: u32,
    pub role: CouncilRole,
    /// The effort the seat's (or the judge's) turn asked for, shown in its header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl RuntimeSession {
    /// This thread's council membership, if it sits on one.
    pub fn council(&self) -> Option<CouncilMeta> {
        let v = self.meta.as_ref()?.get(COUNCIL_META_KEY)?;
        serde_json::from_value(v.clone()).ok()
    }
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
    /// Start a new provider conversation in this thread; refuses outstanding work.
    #[serde(default)]
    pub fresh_context: bool,
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
    /// What to do when a turn is already running. Omitted = refuse (409).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<PromptDelivery>,
}

/// How a message sent while a turn runs is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptDelivery {
    /// Wait in the session's queue; sent when the turn ends.
    Queue,
    /// Into the running turn now (the agent reads it at its next step) —
    /// Claude only; elsewhere it is queued.
    Steer,
    /// Stop the running turn and send this next.
    Interrupt,
}

/// A message waiting in a session's queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedPrompt {
    pub id: Id,
    pub text: String,
    /// Images / files riding with it.
    #[serde(default)]
    pub attachments: u32,
    pub created_at: DateTime<Utc>,
}

/// `POST /runtime-sessions/{id}/queue/{queued_id}` — deliver a queued
/// message now instead of waiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedDeliverReq {
    /// `steer` or `interrupt` (`queue` is a no-op).
    pub delivery: PromptDelivery,
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

/// A bounded inline raster returned by a tool (base64 without a data URL prefix).
/// Drivers accept PNG/JPEG/GIF/WebP, at most four images, 1 MiB each / 2 MiB total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolOutputImage {
    pub mime: String,
    pub data_base64: String,
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
    /// The turn it started or steered into; empty when it was queued.
    pub turn_id: Id,
    /// `sequence` of the turn's first `provider_turn.updated` (of the
    /// `queue.updated` / steered `user_message` when it did not start one).
    pub sequence: u64,
    /// Set when the message was queued behind the running turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_id: Option<Id>,
    /// Set when the message was steered into the running turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub steered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionDismissAction {
    Decline,
    Cancel,
}

/// `POST /runtime-sessions/{id}/requests/{request_id}` (T3 Code
/// `runtime-request.respond`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum RuntimeRequestRespondReq {
    Approval {
        decision: ApprovalDecision,
    },
    Question {
        answers: Vec<QuestionAnswer>,
    },
    DismissQuestion {
        question_action: QuestionDismissAction,
    },
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
            parent_item_id: None,
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
                    background_tasks: vec![BackgroundTask {
                        id: "b1".into(),
                        kind: "command".into(),
                        label: "sleep 300".into(),
                        started_at: ts(),
                        item_id: None,
                    }],
                    capabilities: None,
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
                    origin: TurnOrigin::Agent,
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
                    questions: vec![],
                    question_title: None,
                    answers: None,
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
            RuntimeEventBody::QueueUpdated {
                queued_prompts: vec![QueuedPrompt {
                    id: "q1".into(),
                    text: "after".into(),
                    attachments: 1,
                    created_at: ts(),
                }],
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
    fn new_fields_are_additive_on_the_wire() {
        // A user turn, a root item and an empty background list serialize
        // as before (older clients and stored rows read unchanged).
        let turn: ProviderTurn = serde_json::from_value(json!({
            "id": "t", "ordinal": 1, "status": "running", "started_at": null, "completed_at": null
        }))
        .unwrap();
        assert_eq!(turn.origin, TurnOrigin::User);
        assert!(serde_json::to_value(&turn).unwrap().get("origin").is_none());
        let agent = ProviderTurn {
            origin: TurnOrigin::Agent,
            ..turn
        };
        assert_eq!(serde_json::to_value(&agent).unwrap()["origin"], "agent");
        let card = TurnItem {
            id: "t:tool:a".into(),
            turn_id: Some("t".into()),
            ordinal: 1,
            status: TurnItemStatus::Running,
            title: Some("PR sweep".into()),
            started_at: None,
            completed_at: None,
            updated_at: ts(),
            error: None,
            parent_item_id: None,
            kind: TurnItemKind::Subagent {
                tool_name: "Agent".into(),
                description: Some("PR sweep".into()),
                subagent_type: None,
                prompt: None,
                output: None,
                background: true,
                last_tool: None,
                tool_uses: 0,
            },
        };
        assert!(card.is_background_open());
        let v = serde_json::to_value(&card).unwrap();
        assert_eq!(v["type"], "subagent");
        assert!(v.get("parent_item_id").is_none());
        let child = TurnItem {
            parent_item_id: Some(card.id.clone()),
            kind: TurnItemKind::SystemNotice {
                message: "x".into(),
            },
            ..card.clone()
        };
        assert!(!child.is_background_open());
        assert_eq!(
            serde_json::to_value(&child).unwrap()["parent_item_id"],
            "t:tool:a"
        );
        let req: RuntimePromptReq =
            serde_json::from_value(json!({"text": "hi", "delivery": "steer"})).unwrap();
        assert_eq!(req.delivery, Some(PromptDelivery::Steer));
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
    #[test]
    fn request_response_payloads_never_mix_answers_and_approval() {
        assert!(serde_json::from_value::<RuntimeRequestRespondReq>(json!({"answers":[]})).is_ok());
        assert!(serde_json::from_value::<RuntimeRequestRespondReq>(
            json!({"decision":"accept","answers":[]})
        )
        .is_err());
        assert!(serde_json::from_value::<RuntimeRequestRespondReq>(
            json!({"question_action":"decline"})
        )
        .is_ok());
        assert!(serde_json::from_value::<RuntimeRequestRespondReq>(
            json!({"question_action":"decline","answers":[]})
        )
        .is_err());
    }

    /// The create body: only `source` / `origin` reach the server-owned meta,
    /// validated by one rule (whitelist, kind pairing, lengths, no orphan origin).
    #[test]
    fn create_body_takes_only_whitelisted_embedded_meta() {
        let parse = |v: Value| serde_json::from_value::<CreateRuntimeSessionBody>(v).unwrap();
        let req = parse(json!({
            "provider": "claude",
            "title": "Ask about #12",
            "source": "pr-agent",
            "origin": {"kind": "pr", "id": "repo-1#12", "label": "Fix the parser"},
            "meta": {"read_only": false, "personal_agent": "x"},
        }))
        .into_req()
        .unwrap();
        assert_eq!(req.provider, Some(RuntimeProvider::Claude));
        assert_eq!(req.title.as_deref(), Some("Ask about #12"));
        assert_eq!(
            req.meta,
            Some(json!({
                "source": "pr-agent",
                "origin": {"kind": "pr", "id": "repo-1#12", "label": "Fix the parser"},
            }))
        );
        // No source: today's plain thread, whatever `meta` the body carries.
        let plain = parse(json!({"provider": "codex", "meta": {"read_only": true}}))
            .into_req()
            .unwrap();
        assert_eq!(plain.provider, Some(RuntimeProvider::Codex));
        assert!(plain.meta.is_none());
        // A source without an origin; a blank label is dropped.
        let bare = embedded_meta(Some("connection-diagnose"), None).unwrap();
        assert_eq!(bare, Some(json!({"source": "connection-diagnose"})));
        let blank = embedded_meta(
            Some("connection-diagnose"),
            Some(EmbeddedOrigin {
                kind: "connection".into(),
                id: "c1".into(),
                label: Some("  ".into()),
            }),
        )
        .unwrap();
        assert_eq!(
            blank.unwrap()["origin"],
            json!({"kind": "connection", "id": "c1"})
        );
        // Exactly the limit, in characters (not bytes), is fine.
        let long = "é".repeat(EMBEDDED_ORIGIN_MAX_CHARS);
        assert!(embedded_meta(
            Some("pr-agent"),
            Some(EmbeddedOrigin {
                kind: "pr".into(),
                id: long.clone(),
                label: Some(long),
            }),
        )
        .is_ok());
    }

    #[test]
    fn create_body_refuses_bad_embedded_meta_with_400() {
        let origin = |kind: &str, id: &str, label: Option<&str>| EmbeddedOrigin {
            kind: kind.into(),
            id: id.into(),
            label: label.map(str::to_string),
        };
        let over = "x".repeat(EMBEDDED_ORIGIN_MAX_CHARS + 1);
        let bad = [
            (Some("personal_agent"), None),
            (Some("PR-AGENT"), None),
            (Some(""), None),
            (None, Some(origin("pr", "1", None))),
            (Some("pr-agent"), Some(origin("connection", "1", None))),
            (Some("connection-diagnose"), Some(origin("pr", "1", None))),
            (Some("pr-agent"), Some(origin("", "1", None))),
            (Some("pr-agent"), Some(origin("pr", "", None))),
            (Some("pr-agent"), Some(origin("pr", "   ", None))),
            (Some("pr-agent"), Some(origin("pr", &over, None))),
            (Some("pr-agent"), Some(origin("pr", "1", Some(&over)))),
        ];
        for (source, o) in bad {
            let r = embedded_meta(source, o.clone());
            assert!(
                matches!(r, Err(crate::Error::Invalid(_))),
                "{source:?} {o:?} → {r:?}"
            );
        }
        // Through the body too: a missing origin id is a 400, not a 422.
        let body: CreateRuntimeSessionBody = serde_json::from_value(json!({
            "provider": "claude", "source": "pr-agent", "origin": {"kind": "pr"}
        }))
        .unwrap();
        assert!(matches!(body.into_req(), Err(crate::Error::Invalid(_))));
    }

    #[test]
    fn embedded_thread_helpers() {
        assert!(is_embedded_thread(Some(&json!({"source": "pr-agent"}))));
        assert!(is_embedded_thread(Some(
            &json!({"source": "connection-diagnose"})
        )));
        assert!(!is_embedded_thread(Some(
            &json!({"source": "personal_agent"})
        )));
        assert!(!is_embedded_thread(Some(&json!({"source": 7}))));
        assert!(!is_embedded_thread(None));
        assert_eq!(embedded_auto_close_hours(None), 12);
        assert_eq!(embedded_auto_close_hours(Some(&json!(0))), 0);
        assert_eq!(embedded_auto_close_hours(Some(&json!(48))), 48);
        assert_eq!(embedded_auto_close_hours(Some(&json!("5"))), 12);
        assert_eq!(embedded_auto_close_hours(Some(&json!(-1))), 12);
        assert_eq!(embedded_auto_close_hours(Some(&json!(1.5))), 12);
    }

    #[test]
    fn council_meta_reads_back_and_a_council_body_parses() {
        let session = |meta: Value| -> RuntimeSession {
            serde_json::from_value(json!({
                "id": "s1", "workspace_id": "w1", "provider": "codex", "title": "t",
                "cwd": "/w", "status": "idle", "access_mode": "approval-required",
                "created_by": "u1", "created_at": ts(), "last_active_at": ts(),
                "last_sequence": 0, "meta": meta,
            }))
            .unwrap()
        };
        let s = session(json!({"council": {"id": "c1", "seat": 2, "role": "seat"}}));
        assert_eq!(
            s.council(),
            Some(CouncilMeta {
                id: "c1".into(),
                seat: 2,
                role: CouncilRole::Seat,
                effort: None,
            })
        );
        let j = session(json!({"council": {"id": "c1", "seat": 0, "role": "synthesis"}}));
        assert_eq!(j.council().map(|c| c.role), Some(CouncilRole::Synthesis));
        assert_eq!(session(json!({"read_only": true})).council(), None);
        assert_eq!(session(json!({"council": "nope"})).council(), None);

        let body: CreateCouncilReq = serde_json::from_value(json!({
            "prompt": "which cache?",
            "seats": [{"provider": "claude", "model": "opus", "effort": "high"},
                      {"provider": "claude"}]
        }))
        .unwrap();
        assert_eq!(body.seats.len(), 2);
        assert_eq!(body.seats[1].model, None);
        assert!(body.images.is_empty() && body.title.is_none());
    }
}
