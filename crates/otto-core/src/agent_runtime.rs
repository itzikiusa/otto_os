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

/// The env pair that keeps Muse's launcher from checking for a new binary on
/// every spawn: set on every `muse` Otto runs except its update command.
pub const MUSE_NO_AUTO_UPDATE: (&str, &str) = ("MUSE_NO_AUTO_UPDATE", "1");

/// The agent CLIs the structured runtime can drive (distinct from the PTY
/// provider registry): `claude` over the Agent SDK's stream-json control
/// protocol, `codex` over `codex app-server`, `gemini` (`gemini --acp`) and
/// `grok` (`grok agent stdio`), `opencode` (`opencode acp`) and `muse`
/// (Meta's Muse Code, `muse acp`) over ACP, `agy` (Antigravity) over its own
/// ACP server, `pi` over its RPC mode.
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
    Muse,
}

impl RuntimeProvider {
    pub const ALL: [Self; 8] = [
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Grok,
        Self::Agy,
        Self::Opencode,
        Self::Pi,
        Self::Muse,
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
            Self::Muse => "muse",
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
            "muse" => Some(Self::Muse),
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
    /// The thread's title, so every client sees a rename or the agent's own
    /// name for the conversation as it lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The provider the thread runs on now (`RuntimeSession::provider`) and,
    /// for a named account, its slug (`meta.provider_slug`): a provider
    /// switch changes them in every client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<RuntimeProvider>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_slug: Option<String>,
    /// The client-owned thread meta (`THREAD_META_KEYS`: pinned, History,
    /// seen at), so a pin on the Mac shows on the phone as it lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<serde_json::Value>,
}

/// One piece of background work an agent started (Claude Code's
/// `task_started` … `task_notification`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackgroundTask {
    /// The provider's task id.
    pub id: String,
    /// `command`, `agent`, `timer`, or the provider's own task type.
    pub kind: String,
    /// What it is doing (the command, the subagent's description).
    pub label: String,
    pub started_at: DateTime<Utc>,
    /// The turn item of the tool call that started it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<Id>,
    /// Optional scheduled fire timestamp for one-shot timers (`schedule`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_at: Option<DateTime<Utc>>,
    /// A person asked to stop it and the provider has not confirmed yet
    /// (`POST …/tasks/{task_id}/stop`). Set by the runtime, never a driver.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stopping: bool,
    /// Why the last stop did not take (the provider refused it, or the task
    /// was still running when the stop grace ran out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_error: Option<String>,
}

/// Structured settlement metadata attached to a [`TurnItemKind::SystemNotice`]
/// when background work (`command`, `agent`, `timer`) finishes, stops, or fails,
/// eliminating English-prose regex coupling in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSettlement {
    pub task_id: String,
    pub kind: String,
    /// `"completed"`, `"stopped"`, or `"failed"`.
    pub status: String,
    pub label: String,
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TurnTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// What the turn cost in US dollars, when the provider reports it
    /// (Claude `total_cost_usd`, an ACP agent's `usage_update.cost`, pi's
    /// session stats): the difference between its running totals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
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
    /// The model this turn ran on, as far as Otto knows when it starts: the
    /// pinned one, else the one the agent reported (`None` = its default,
    /// unreported). A refused switch rewrites it to the model kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The reasoning effort sent with this turn (`None` = the provider's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
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
    /// Reject and remember it (ACP `reject_always`); a provider with no
    /// such grant treats it as `Decline`.
    DeclineAlways,
    Cancel,
}

/// `ProviderApprovalOption`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ApprovalOptionWire", into = "ApprovalOptionWire")]
pub struct ApprovalOption {
    pub decision: ApprovalDecision,
    pub label: String,
}

/// An option as stored and sent: `DeclineAlways` is written `decline` +
/// `always: true`, so the build before it (no such decision; it ignores
/// `always`) still reads the row on a rollback.
#[derive(Serialize, Deserialize)]
struct ApprovalOptionWire {
    decision: ApprovalDecision,
    label: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    always: bool,
}

impl From<ApprovalOptionWire> for ApprovalOption {
    fn from(w: ApprovalOptionWire) -> Self {
        let decision = match w.decision {
            ApprovalDecision::Decline if w.always => ApprovalDecision::DeclineAlways,
            d => d,
        };
        Self {
            decision,
            label: w.label,
        }
    }
}

impl From<ApprovalOption> for ApprovalOptionWire {
    fn from(o: ApprovalOption) -> Self {
        let always = o.decision == ApprovalDecision::DeclineAlways;
        Self {
            decision: if always {
                ApprovalDecision::Decline
            } else {
                o.decision
            },
            label: o.label,
            always,
        }
    }
}

/// A recorded decision as the build before reads it (`decline_always` →
/// `decline`; the card says "Declined" either way).
fn stored_decision<S: serde::Serializer>(
    d: &Option<ApprovalDecision>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    match d {
        Some(ApprovalDecision::DeclineAlways) => Some(ApprovalDecision::Decline).serialize(s),
        d => d.serialize(s),
    }
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

/// [`ProviderFailure::code`] of a driver error that says the agent refused a
/// LIVE model switch (Claude `set_model`, ACP `-32602`): the runtime puts the
/// thread back on the model it runs and says so in the timeline.
pub const MODEL_SWITCH_REFUSED: &str = "model_switch_refused";

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
        /// Images the tool left on disk (Codex `imageView` /
        /// `imageGeneration`): absolute paths, served read-only by
        /// `GET /runtime-sessions/{id}/items/{item_id}/images/{n}`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        image_files: Vec<String>,
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
    /// A fresh provider conversation began in this same visible thread;
    /// with `handoff`, on another provider that was handed the history.
    SessionBoundary {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handoff: Option<ContextHandoff>,
    },
    SystemNotice {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_settlement: Option<TaskSettlement>,
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

/// What a provider switch handed the new provider (`session_boundary.handoff`):
/// a bounded, newest-first selection of whole turns, after T3 Code's portable
/// handoffs. `text` is the block exactly as the agent read it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ContextHandoff {
    /// Provider slugs (a named account's own slug).
    pub from_provider: String,
    pub to_provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_model: Option<String>,
    /// Turns handed over, and earlier turns the budget left out.
    pub turns: u32,
    #[serde(default)]
    pub omitted_turns: u32,
    /// Images sent earlier in the thread: never handed over.
    #[serde(default)]
    pub images_omitted: u32,
    #[serde(default)]
    pub text: String,
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
    /// True when heavy tool/diff/output fields were omitted from a compact
    /// snapshot (`GET /runtime-sessions/{id}/events?compact=1`); fetch the
    /// full item on demand via `GET /runtime-sessions/{id}/items/{item_id}`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detail_omitted: bool,
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

    /// Return a lightweight snapshot clone omitting large tool payloads,
    /// command outputs, and file patches on settled items (`detail_omitted = true`).
    /// `UserMessage`, `AssistantMessage`, and all open/streaming items are
    /// returned intact so conversation text and live rows render immediately.
    pub fn compact_for_snapshot(&self) -> Self {
        if self.status.is_open() || self.is_streaming() {
            return self.clone();
        }
        let mut out = self.clone();
        match &mut out.kind {
            TurnItemKind::FileChange {
                diff_str,
                old_str,
                new_str,
                ..
            } => {
                if diff_str.is_some() || old_str.is_some() || new_str.is_some() {
                    *diff_str = None;
                    *old_str = None;
                    *new_str = None;
                    out.detail_omitted = true;
                }
            }
            TurnItemKind::CommandExecution { input, output, .. } => {
                let long_input = input.len() > 240;
                if long_input {
                    *input = truncate_compact_str(input, 240);
                }
                if output.is_some() || long_input {
                    *output = None;
                    out.detail_omitted = true;
                }
            }
            TurnItemKind::DynamicTool {
                tool_name,
                input,
                output,
                images,
                ..
            } => {
                if !is_thread_link_tool(tool_name)
                    && (output.is_some() || !images.is_empty() || json_value_exceeds(input, 256))
                {
                    *input = Value::Null;
                    *output = None;
                    images.clear();
                    out.detail_omitted = true;
                }
            }
            TurnItemKind::Reasoning { text, .. } => {
                if text.len() > 600 {
                    *text = truncate_compact_markdown(text, 560);
                    out.detail_omitted = true;
                }
            }
            TurnItemKind::Subagent { prompt, output, .. } => {
                let first_line = output
                    .as_deref()
                    .and_then(|s| s.lines().find(|l| !l.trim().is_empty()))
                    .map(|l| truncate_compact_str(l.trim(), 200));
                let had_large_output = output
                    .as_deref()
                    .is_some_and(|s| first_line.as_deref() != Some(s.trim()));
                if prompt.is_some() || had_large_output {
                    *prompt = None;
                    *output = first_line;
                    out.detail_omitted = true;
                }
            }
            _ => {}
        }
        out
    }
}

fn is_thread_link_tool(name: &str) -> bool {
    let bare = name.rsplit(['.', '_']).next().unwrap_or(name);
    let tail = name
        .rsplit_once("otto_")
        .or_else(|| name.rsplit_once("otto."))
        .map(|(_, t)| t)
        .unwrap_or(bare);
    matches!(
        tail,
        "thread_open" | "open_session" | "thread_message" | "thread_read"
    )
}

fn json_value_exceeds(v: &Value, max_bytes: usize) -> bool {
    fn walk(v: &Value, acc: &mut usize, max_bytes: usize) -> bool {
        if *acc > max_bytes {
            return true;
        }
        match v {
            Value::Null | Value::Bool(_) => *acc += 5,
            Value::Number(n) => *acc += n.to_string().len(),
            Value::String(s) => *acc += s.len() + 2,
            Value::Array(items) => {
                *acc += 2;
                for item in items {
                    if walk(item, acc, max_bytes) {
                        return true;
                    }
                }
            }
            Value::Object(map) => {
                *acc += 2;
                for (k, val) in map {
                    *acc += k.len() + 3;
                    if walk(val, acc, max_bytes) {
                        return true;
                    }
                }
            }
        }
        *acc > max_bytes
    }
    let mut acc = 0usize;
    walk(v, &mut acc, max_bytes)
}

fn truncate_compact_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn truncate_compact_markdown(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut slice = s[..end].to_string();
    let fences = slice
        .lines()
        .filter(|l| l.starts_with("```") || l.starts_with("~~~"))
        .count();
    if fences % 2 == 1 {
        slice.push_str("\n```");
    }
    slice
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

mod outcome;
pub use outcome::{
    error_class, TurnOutcome, ERROR_CLASS_AUTH, ERROR_CLASS_PROVIDER, ERROR_CLASS_USAGE_LIMIT,
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "stored_decision"
    )]
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
    /// The agent's own slash commands, whole (replaces the last list).
    #[serde(rename = "commands.updated")]
    CommandsUpdated { commands: Vec<ProviderCommand> },
    /// The MCP servers the thread's agent has, whole (replaces the last list).
    #[serde(rename = "mcp_servers.updated")]
    McpServersUpdated { mcp_servers: Vec<McpServerStatus> },
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
            Self::CommandsUpdated { .. } => "commands.updated",
            Self::McpServersUpdated { .. } => "mcp_servers.updated",
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

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
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
    /// The agent's own slash commands (`commands.updated`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<ProviderCommand>,
    /// The MCP servers its agent has (`mcp_servers.updated`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerStatus>,
    /// Set when `tail_turns` / `before_turn_ordinal` omitted earlier turns.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_older: bool,
    /// `ordinal` of the earliest turn included in this slice, when paginated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_turn_ordinal: Option<u32>,
    /// Count of earlier turns omitted before `oldest_turn_ordinal`.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub older_turns: u32,
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
    /// A prompt may ask for plan mode (`plan: true`): the agent researches
    /// and proposes, and changes nothing until a later turn leaves it.
    #[serde(default)]
    pub plan: bool,
}

/// One slash command an agent offers (`/name`): its skills, prompt
/// templates and built-ins, as it lists them (`commands.updated`). A prompt
/// starting with `/name` runs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCommand {
    /// Without the leading `/`.
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// What to type after it (ACP `input.hint`), when it takes input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// One MCP server a thread's agent has — Otto's own tools, a control-plane
/// server Otto handed the CLI, or one from the CLI's own config — and whether
/// the agent can use it (`mcp_servers.updated`). Set at spawn from what Otto
/// gave the agent (and skipped, with why), then refined by what the CLI
/// reports (Claude's `system init`, Codex's `mcpServer/startupStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerStatus {
    pub name: String,
    pub source: McpServerSource,
    pub state: McpServerState,
    /// Why it is not connected, in words (absent when it is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerSource {
    /// Otto's own tool server (`otto`, `otto-browser`).
    Otto,
    /// A control-plane server Otto handed the agent.
    Workspace,
    /// From the CLI's own config (Otto did not add it).
    Cli,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerState {
    Connected,
    /// The CLI is still starting it.
    Starting,
    /// Given to the agent; the CLI does not report whether it connected.
    Added,
    NeedsAuth,
    Failed,
    Disabled,
    /// The provider (or this mode) cannot take it.
    Unsupported,
}

impl McpServerStatus {
    pub fn new(name: impl Into<String>, source: McpServerSource, state: McpServerState) -> Self {
        Self {
            name: name.into(),
            source,
            state,
            reason: None,
        }
    }

    pub fn because(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }
}

impl ProviderCommand {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into().trim_start_matches('/').to_string(),
            description: description.into(),
            hint: None,
        }
    }
}

/// Every effort name a driver may offer, low → high (agents list theirs in
/// either order: grok advertises `xhigh, high, medium, low`).
pub const EFFORT_RANK: [&str; 8] = [
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

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
                plan: true,
            },
            // PDFs go as a saved file the agent reads (its input has no
            // document type).
            RuntimeProvider::Codex => Self {
                steer: true,
                images: true,
                files: true,
                efforts: levels(&["minimal", "low", "medium", "high", "xhigh"]),
                plan: true,
            },
            RuntimeProvider::Pi => Self {
                steer: true,
                images: true,
                files: true,
                efforts: levels(&["minimal", "low", "medium", "high", "xhigh"]),
                plan: false,
            },
            // ACP v1 has no way into a running prompt. Plan mode is a
            // session mode the agent advertises (agy's `plan`).
            RuntimeProvider::Agy => Self {
                steer: false,
                images: true,
                files: true,
                efforts: levels(&["low", "medium", "high"]),
                plan: true,
            },
            RuntimeProvider::Grok => Self {
                steer: false,
                images: true,
                files: true,
                efforts: levels(&["low", "medium", "high"]),
                plan: false,
            },
            // Muse's tiers are `none` … `ultra`; Otto's picker ranks them
            // on its five-level scale (`ultra` rides `max`). Its modes are
            // approval policies only — no plan mode.
            RuntimeProvider::Muse => Self {
                steer: false,
                images: true,
                files: true,
                efforts: levels(&["low", "medium", "high", "xhigh", "max"]),
                plan: false,
            },
            RuntimeProvider::Gemini | RuntimeProvider::Opencode => Self {
                steer: false,
                images: true,
                files: true,
                efforts: Vec::new(),
                plan: false,
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
    /// The reasoning effort the thread runs at (picked at create or with an
    /// accepted prompt), re-applied on every respawn; `None` = the provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
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
    /// The title is still the placeholder it was created with: the agent's
    /// own name for the conversation replaces it. A rename clears it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub title_auto: bool,
    /// How the latest turn ended; `None` before the first turn ends and
    /// while a turn runs (see [`Self::set_outcome`]). Survives a restart:
    /// the store derives it from the persisted latest `turn` entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_status: Option<TurnOutcome>,
    /// With `last_turn_status: failed`: [`error_class`] of its failure
    /// (`usage_limit` / `auth` / `provider`); `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_class: Option<String>,
}

/// Server-owned `RuntimeSession::meta` key storing a custom provider slug from
/// `ProviderRegistry` (e.g. `"claudework"` mapped onto `RuntimeProvider::Claude`).
pub const PROVIDER_SLUG_META_KEY: &str = "provider_slug";

impl RuntimeSession {
    /// A boolean flag in [`Self::meta`] (`read_only`, …); absent = false.
    pub fn meta_flag(&self, key: &str) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    }

    /// Custom provider slug from `meta.provider_slug` when created from a
    /// custom `ProviderRegistry` entry, or the built-in `RuntimeProvider` id.
    pub fn provider_slug(&self) -> &str {
        self.meta
            .as_ref()
            .and_then(|m| m.get(PROVIDER_SLUG_META_KEY))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.provider.as_str())
    }

    /// This thread in terminal-session shape, for surfaces keyed by session
    /// (agent UI control): `meta.runtime = "structured"`, its grant under
    /// `meta.ui_control`, no device pin (any of the owner's windows).
    pub fn into_session(self) -> crate::domain::Session {
        let provider_slug = self.provider_slug().to_string();
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
            provider: provider_slug,
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
    /// `title` is a placeholder (the opening message's first words): the
    /// agent's own name for the conversation may replace it.
    #[serde(default)]
    pub title_auto: bool,
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
/// four a client may set on the server-owned meta (`source`, `origin`,
/// `account_id`, `network_profile_id`), and a custom provider slug
/// (`provider: "claudework"`). Nothing else in the body reaches `meta`
/// (`CreateRuntimeSessionReq::meta` stays `serde(skip)`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateRuntimeSessionBody {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub access_mode: Option<RuntimeAccessMode>,
    /// See [`CreateRuntimeSessionReq::title_auto`].
    #[serde(default)]
    pub title_auto: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub origin: Option<EmbeddedOrigin>,
    /// A named provider account (Settings › Accounts) the agent signs in
    /// with, as a terminal session's `meta.account_id`; checked at spawn.
    #[serde(default)]
    pub account_id: Option<String>,
    /// A network profile of the workspace the agent's traffic goes
    /// through (`meta.network_profile_id`); checked at spawn.
    #[serde(default)]
    pub network_profile_id: Option<String>,
    /// The reasoning effort the thread starts at (stored on the session and
    /// re-applied on every spawn). Not part of [`CreateRuntimeSessionReq`]:
    /// the route sets it once the thread exists.
    #[serde(default)]
    pub effort: Option<String>,
}

impl CreateRuntimeSessionBody {
    /// Convert into [`CreateRuntimeSessionReq`] for built-in [`RuntimeProvider`]
    /// names only (rejects unknown provider names with `400 Invalid`).
    pub fn into_req(self) -> crate::Result<CreateRuntimeSessionReq> {
        self.into_req_with_resolver(RuntimeProvider::parse)
    }

    /// Convert into [`CreateRuntimeSessionReq`], resolving both built-in and
    /// custom provider slugs (e.g. `"claudework"` -> `RuntimeProvider::Claude`)
    /// via `resolve_custom` so disabled built-in providers (e.g. `"grok"`) are
    /// rejected with `400 Invalid`, and storing `meta.provider_slug` only when
    /// `raw` is a non-built-in custom slug.
    pub fn into_req_with_resolver<F>(
        self,
        resolve_custom: F,
    ) -> crate::Result<CreateRuntimeSessionReq>
    where
        F: FnOnce(&str) -> Option<RuntimeProvider>,
    {
        let mut meta = embedded_meta(self.source.as_deref(), self.origin)?;
        let provider = match self
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            None => None,
            Some(raw) => {
                if let Some(mapped) = resolve_custom(raw) {
                    if RuntimeProvider::parse(raw).is_none() {
                        let obj = meta.get_or_insert_with(|| Value::Object(serde_json::Map::new()));
                        if let Some(map) = obj.as_object_mut() {
                            map.insert(
                                PROVIDER_SLUG_META_KEY.into(),
                                Value::String(raw.to_string()),
                            );
                        }
                    }
                    Some(mapped)
                } else {
                    return Err(crate::Error::Invalid(format!(
                        "unknown or disabled runtime provider '{raw}'"
                    )));
                }
            }
        };
        for (key, v) in [
            ("account_id", self.account_id),
            ("network_profile_id", self.network_profile_id),
        ] {
            if let Some(v) = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
                meta.get_or_insert_with(|| Value::Object(Default::default()))[key] =
                    Value::String(v);
            }
        }
        Ok(CreateRuntimeSessionReq {
            provider,
            cwd: self.cwd,
            title: self.title,
            model: self.model,
            access_mode: self.access_mode,
            title_auto: self.title_auto,
            meta,
        })
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

/// The daemon setting (`PUT /settings`) after how many idle minutes a
/// thread's agent process (and its MCP servers) is stopped; the thread stays
/// open and its next message resumes the provider's conversation. Absent or
/// not a non-negative integer = the default (30), `0` = never. A finished
/// child thread is released after at most [`CHILD_THREAD_IDLE_RELEASE_MINUTES`].
pub const THREAD_IDLE_RELEASE_KEY: &str = "thread_idle_release_minutes";
pub const THREAD_IDLE_RELEASE_DEFAULT_MINUTES: u64 = 30;
pub const CHILD_THREAD_IDLE_RELEASE_MINUTES: u64 = 5;

/// The idle-release setting's minutes (see [`THREAD_IDLE_RELEASE_KEY`]).
pub fn thread_idle_release_minutes(setting: Option<&Value>) -> u64 {
    setting
        .and_then(Value::as_u64)
        .unwrap_or(THREAD_IDLE_RELEASE_DEFAULT_MINUTES)
}

/// `POST /runtime-sessions/{id}/peers` — an agent (or its owner) opens a
/// linked thread: same workspace, cwd and access mode as `{id}`.
///
/// Accepts either a built-in [`RuntimeProvider`] (`"claude"`, `"codex"`, …) or
/// a custom provider slug (`"claudework"`) on the wire (`provider: String`),
/// while preserving `pub provider: RuntimeProvider` for internal Rust callers.
#[derive(Debug, Clone)]
pub struct OpenPeerReq {
    pub provider: RuntimeProvider,
    /// The new thread's opening message (framed with who sent it).
    pub prompt: Option<String>,
    pub title: Option<String>,
    /// Omitted = the provider's default model.
    pub model: Option<String>,
    /// Reasoning effort for the opening turn (`low` … `max`); omitted = the
    /// model's default.
    pub effort: Option<String>,
    /// Daemon-only (`serde(skip)`): the linked thread's own folder (default:
    /// the opener's) and its [`RuntimeSession::meta`] — a personal agent the
    /// main agent delegates to runs in ITS persona folder, as itself.
    pub cwd: Option<String>,
    pub meta: Option<serde_json::Value>,
}

impl OpenPeerReq {
    /// Effective provider slug requested (custom `meta.provider_slug` when
    /// present, otherwise `self.provider.as_str()`).
    pub fn provider_slug(&self) -> &str {
        self.meta
            .as_ref()
            .and_then(|m| m.get(PROVIDER_SLUG_META_KEY))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.provider.as_str())
    }

    /// Construct a peer request targeting either a built-in provider or a
    /// custom provider slug (e.g. `"claudework"`).
    pub fn with_provider_slug(slug: impl Into<String>) -> Self {
        let raw = slug.into();
        let (provider, custom_slug) = parse_provider_and_slug(&raw, None);
        let meta = custom_slug.map(|s| serde_json::json!({ PROVIDER_SLUG_META_KEY: s }));
        Self {
            provider,
            prompt: None,
            title: None,
            model: None,
            effort: None,
            cwd: None,
            meta,
        }
    }
}

impl Serialize for OpenPeerReq {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize)]
        struct Wire<'a> {
            provider: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            prompt: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            title: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            model: &'a Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            effort: &'a Option<String>,
        }
        Wire {
            provider: self.provider_slug(),
            prompt: &self.prompt,
            title: &self.title,
            model: &self.model,
            effort: &self.effort,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for OpenPeerReq {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: String,
            #[serde(default)]
            provider_slug: Option<String>,
            #[serde(default)]
            prompt: Option<String>,
            #[serde(default)]
            title: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            effort: Option<String>,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.provider.trim().is_empty() {
            return Err(serde::de::Error::custom("provider must not be empty"));
        }
        let (provider, custom_slug) = parse_provider_and_slug(&w.provider, w.provider_slug);
        let meta = custom_slug.map(|s| serde_json::json!({ PROVIDER_SLUG_META_KEY: s }));
        Ok(Self {
            provider,
            prompt: w.prompt,
            title: w.title,
            model: w.model,
            effort: w.effort,
            cwd: None,
            meta,
        })
    }
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

/// `POST /runtime-sessions/{id}/stop-family` — what one Stop for a thread and
/// its subagents did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyStopReq {
    /// Also stop the thread's own background work (the waiting line's Stop).
    #[serde(default)]
    pub background: bool,
}

/// The answer to [`FamilyStopReq`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyStopResp {
    /// Open subagents in the family (the thread itself not counted).
    pub subagents: u32,
    /// Turns that were running and got stopped.
    pub turns: u32,
    /// Background tasks asked to stop.
    pub tasks: u32,
}

/// `GET /runtime-sessions/{id}/subagent-targets` — a provider a thread's agent
/// may delegate to (`otto.thread_capabilities`): enabled here, driven over a
/// structured protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentTarget {
    /// `otto.thread_open`'s `provider` (a built-in or custom slug).
    pub provider: String,
    /// Model ids its CLI lists (at most 12); empty = its default only.
    #[serde(default)]
    pub models: Vec<String>,
    /// Effort levels a turn may ask for; empty = none.
    #[serde(default)]
    pub efforts: Vec<String>,
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

fn parse_provider_and_slug(
    raw_provider: &str,
    explicit_slug: Option<String>,
) -> (RuntimeProvider, Option<String>) {
    let trimmed = raw_provider.trim();
    if let Some(p) = RuntimeProvider::parse(trimmed) {
        let slug = explicit_slug
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s != p.as_str());
        (p, slug)
    } else {
        let slug = explicit_slug
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| trimmed.to_string());
        (RuntimeProvider::Claude, Some(slug))
    }
}

/// One agent of `POST /runtime-sessions/{id}/children`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AskChildTarget {
    pub provider: RuntimeProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl<'de> Deserialize<'de> for AskChildTarget {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: String,
            #[serde(default)]
            provider_slug: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            effort: Option<String>,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.provider.trim().is_empty() {
            return Err(serde::de::Error::custom("provider must not be empty"));
        }
        let (provider, provider_slug) = parse_provider_and_slug(&w.provider, w.provider_slug);
        Ok(Self {
            provider,
            provider_slug,
            model: w.model,
            effort: w.effort,
        })
    }
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
    /// The answer goes back to the thread's agent (default), like a subagent
    /// the agent opened; `true` = "answer to me only" (it stays with the person).
    #[serde(default)]
    pub to_me_only: bool,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CouncilSeatReq {
    pub provider: RuntimeProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Reasoning effort for the seat's first turn; omitted = the model's default.
    #[serde(default)]
    pub effort: Option<String>,
}

impl<'de> Deserialize<'de> for CouncilSeatReq {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: String,
            #[serde(default)]
            provider_slug: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            effort: Option<String>,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.provider.trim().is_empty() {
            return Err(serde::de::Error::custom("provider must not be empty"));
        }
        let (provider, provider_slug) = parse_provider_and_slug(&w.provider, w.provider_slug);
        Ok(Self {
            provider,
            provider_slug,
            model: w.model,
            effort: w.effort,
        })
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SynthesizeCouncilReq {
    pub provider: RuntimeProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    /// Stop a seat's leftover background work (a seat with nothing else
    /// running) instead of refusing the synthesis with 409.
    #[serde(default)]
    pub stop_background: bool,
}

impl<'de> Deserialize<'de> for SynthesizeCouncilReq {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            provider: String,
            #[serde(default)]
            provider_slug: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default)]
            effort: Option<String>,
            #[serde(default)]
            stop_background: bool,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.provider.trim().is_empty() {
            return Err(serde::de::Error::custom("provider must not be empty"));
        }
        let (provider, provider_slug) = parse_provider_and_slug(&w.provider, w.provider_slug);
        Ok(Self {
            provider,
            provider_slug,
            model: w.model,
            effort: w.effort,
            stop_background: w.stop_background,
        })
    }
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
    /// The seat's provider as picked (a custom slug such as `claudework`, else
    /// the built-in id) — two seats of one provider differ by model / effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The model the seat was started on; `None` = the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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

/// `PATCH /runtime-sessions/{id}` (rename and/or the client-owned thread
/// meta; T3 Code `thread.metadata-updated`). At least one of the two.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateRuntimeSessionReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A merge patch of [`THREAD_META_KEYS`] only (`null` clears a key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
}

/// The thread meta a person's client owns on a session — the same keys on a
/// terminal session (`PATCH /sessions/{id}` meta) and a structured one:
/// pinned, moved to History, and when the person last had it in front of them.
pub const THREAD_META_KEYS: &[&str] = &["thread_pinned", "thread_history", "thread_seen_at"];

/// Checks a thread-meta patch: only [`THREAD_META_KEYS`]; `thread_pinned` /
/// `thread_history` a bool, `thread_seen_at` an RFC 3339 time; `null` clears.
pub fn validate_thread_meta(
    patch: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    if patch.is_empty() {
        return Err("meta is empty".into());
    }
    for (k, v) in patch {
        let ok = match k.as_str() {
            "thread_pinned" | "thread_history" => v.is_boolean() || v.is_null(),
            "thread_seen_at" => {
                v.is_null()
                    || v.as_str()
                        .is_some_and(|s| DateTime::parse_from_rfc3339(s).is_ok())
            }
            _ => {
                return Err(format!(
                    "meta.{k} is not a client-owned thread key ({})",
                    THREAD_META_KEYS.join(", ")
                ))
            }
        };
        if !ok {
            return Err(format!("meta.{k} has the wrong type"));
        }
    }
    Ok(())
}

/// The client-owned thread keys set in `meta` (`None` when none is).
pub fn thread_meta_of(meta: Option<&serde_json::Value>) -> Option<serde_json::Value> {
    let m = meta?.as_object()?;
    let out: serde_json::Map<_, _> = THREAD_META_KEYS
        .iter()
        .filter_map(|k| {
            m.get(*k)
                .filter(|v| !v.is_null())
                .map(|v| ((*k).to_string(), v.clone()))
        })
        .collect();
    (!out.is_empty()).then_some(serde_json::Value::Object(out))
}

/// `POST /runtime-sessions/{id}/prompt`. `model` / `effort` select this
/// turn's model (it sticks for later turns); `effort` is Claude / Codex only.
/// `access_mode` applies from this turn on and sticks, like `model`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimePromptReq {
    /// Start a new provider conversation in this thread; refuses outstanding work.
    #[serde(default)]
    pub fresh_context: bool,
    /// Continue this thread on another provider (a slug: built-in or a named
    /// account): a fresh conversation there, handed the thread's history.
    /// Implies `fresh_context`; the current provider's slug is no switch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
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
    /// Run this turn in plan mode (only where `capabilities.plan`): the
    /// agent plans and changes nothing. A later turn without it leaves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plan: bool,
    /// What to do when a turn is already running. Omitted = refuse (409).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<PromptDelivery>,
    /// With `fresh_context` (and council synthesize): stop the thread's
    /// background work instead of refusing with 409 when nothing else (no
    /// turn, no pending request, no queued message) is in the way.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stop_background: bool,
}

/// How a message sent while a turn runs is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptDelivery {
    /// Wait in the session's queue; sent when the turn ends.
    Queue,
    /// Into the running turn now (the agent reads it at its next step) —
    /// where `capabilities.steer`; elsewhere it is queued.
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
    /// Why its last delivery failed: it stays queued (never lost) and is
    /// skipped by the automatic drain until a person retries (`POST
    /// …/queue/{id}` with any delivery), edits or removes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `POST /runtime-sessions/{id}/queue/{queued_id}` — deliver a queued
/// message now instead of waiting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedDeliverReq {
    /// `steer` or `interrupt` (`queue` is a no-op).
    pub delivery: PromptDelivery,
}

/// `DELETE /runtime-sessions/{id}/queue/{queued_id}` — pop a queued message
/// and return its original payload so "Edit" restores text and attachments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnqueuePromptResp {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<PromptImage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PromptFile>,
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
    /// Optional filename preserved across queue / unqueue (`Edit`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// A file attachment (`mime` is `application/pdf`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFile {
    pub mime: String,
    pub name: String,
    pub data_base64: String,
    /// Where the runtime stored it (absolute), for an agent that takes a
    /// file only as a path to read. Set by the runtime, never by a client.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
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
            detail_omitted: false,
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
    fn thread_meta_patch_takes_only_client_owned_keys() {
        let m = |v: serde_json::Value| v.as_object().unwrap().clone();
        assert!(validate_thread_meta(&m(serde_json::json!({"thread_pinned": true, "thread_history": null, "thread_seen_at": "2026-11-23T10:00:00Z"}))).is_ok());
        assert!(validate_thread_meta(&m(serde_json::json!({}))).is_err());
        assert!(
            validate_thread_meta(&m(serde_json::json!({"source": "pr-agent"})))
                .unwrap_err()
                .contains("not a client-owned")
        );
        assert!(validate_thread_meta(&m(serde_json::json!({"thread_pinned": "yes"}))).is_err());
        assert!(
            validate_thread_meta(&m(serde_json::json!({"thread_seen_at": "yesterday"}))).is_err()
        );
        let meta = serde_json::json!({"source": "pr-agent", "thread_pinned": true, "thread_history": null});
        assert_eq!(
            thread_meta_of(Some(&meta)),
            Some(serde_json::json!({"thread_pinned": true}))
        );
        assert_eq!(
            thread_meta_of(Some(&serde_json::json!({"source": "pr-agent"}))),
            None
        );
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
                        due_at: None,
                        stopping: false,
                        stop_error: None,
                    }],
                    capabilities: None,
                    title: None,
                    provider: Some(RuntimeProvider::Codex),
                    provider_slug: Some("codex".into()),
                    thread: Some(serde_json::json!({"thread_pinned": true})),
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
                    model: Some("opus".into()),
                    effort: Some("high".into()),
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
                    error: Some("the agent could not start".into()),
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
        assert_eq!(
            (turn.model.as_deref(), turn.effort.as_deref()),
            (None, None)
        );
        assert!(serde_json::to_value(&turn).unwrap().get("model").is_none());
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
            detail_omitted: false,
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
                task_settlement: None,
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
                provider: None,
                model: None,
            })
        );
        let j = session(json!({"council": {"id": "c1", "seat": 0, "role": "synthesis"}}));
        assert_eq!(j.council().map(|c| c.role), Some(CouncilRole::Synthesis));
        assert_eq!(session(json!({"read_only": true})).council(), None);
        assert_eq!(session(json!({"council": "nope"})).council(), None);

        let body: CreateCouncilReq = serde_json::from_value(json!({
            "prompt": "which cache?",
            "seats": [{"provider": "claude", "model": "opus", "effort": "high"},
                      {"provider": "claudework"}]
        }))
        .unwrap();
        assert_eq!(body.seats.len(), 2);
        assert_eq!(body.seats[0].provider_slug, None);
        assert_eq!(body.seats[1].provider_slug.as_deref(), Some("claudework"));
        assert_eq!(body.seats[1].model, None);
        assert!(body.images.is_empty() && body.title.is_none());

        let custom_body: CreateRuntimeSessionBody = serde_json::from_value(json!({
            "provider": "claudework",
            "cwd": "/tmp/p",
            "model": "opus",
        }))
        .unwrap();
        assert!(custom_body.clone().into_req().is_err());
        let resolved = custom_body
            .into_req_with_resolver(|slug| {
                (slug == "claudework").then_some(RuntimeProvider::Claude)
            })
            .unwrap();
        assert_eq!(resolved.provider, Some(RuntimeProvider::Claude));
        assert_eq!(
            resolved
                .meta
                .as_ref()
                .and_then(|m| m.get(PROVIDER_SLUG_META_KEY)),
            Some(&json!("claudework"))
        );
    }

    /// A rollback reads a switch's divider: the build before it had a unit
    /// `session_boundary` (internally tagged, flattened), which ignores
    /// `handoff`; and the new shape reads the old row.
    #[test]
    fn a_session_boundary_with_a_handoff_reads_on_the_previous_build() {
        #[derive(Debug, Deserialize)]
        #[serde(tag = "type", rename_all = "snake_case")]
        enum OldKind {
            SessionBoundary,
        }
        #[derive(Debug, Deserialize)]
        struct OldItem {
            #[allow(dead_code)]
            id: String,
            #[serde(flatten)]
            kind: OldKind,
        }
        let new = json!({"id": "b", "type": "session_boundary", "handoff": {
            "from_provider": "claude", "to_provider": "codex", "turns": 1, "text": "x"}});
        let old: OldItem = serde_json::from_value(new.clone()).unwrap();
        assert!(matches!(old.kind, OldKind::SessionBoundary));
        let back: TurnItemKind =
            serde_json::from_value(json!({"type": "session_boundary"})).unwrap();
        assert_eq!(back, TurnItemKind::SessionBoundary { handoff: None });
        let full: TurnItemKind = serde_json::from_value(new).unwrap();
        assert!(
            matches!(full, TurnItemKind::SessionBoundary { handoff: Some(h) } if h.turns == 1 && h.omitted_turns == 0)
        );
    }

    /// A rollback reads an ACP "Never" option and a request answered with it:
    /// the build before had no `decline_always`, so it is stored as `decline`
    /// with `always: true` (ignored there) and read back as `DeclineAlways`.
    #[test]
    fn a_decline_always_option_reads_on_the_previous_build() {
        #[derive(Debug, Deserialize, PartialEq)]
        #[serde(rename_all = "snake_case")]
        enum OldDecision {
            Accept,
            AcceptForSession,
            AcceptAlways,
            Decline,
            Cancel,
        }
        #[derive(Debug, Deserialize)]
        struct OldOption {
            decision: OldDecision,
            #[allow(dead_code)]
            label: String,
        }
        #[derive(Debug, Deserialize)]
        struct OldRequest {
            decision: Option<OldDecision>,
        }
        let never = ApprovalOption {
            decision: ApprovalDecision::DeclineAlways,
            label: "Never".into(),
        };
        let stored = serde_json::to_value(&never).unwrap();
        let old: OldOption = serde_json::from_value(stored.clone()).unwrap();
        assert_eq!(old.decision, OldDecision::Decline);
        assert_eq!(
            serde_json::from_value::<ApprovalOption>(stored).unwrap(),
            never
        );
        let plain = ApprovalOption {
            decision: ApprovalDecision::Decline,
            label: "No".into(),
        };
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({"decision": "decline", "label": "No"})
        );
        let req = RuntimeRequest {
            id: "r".into(),
            turn_id: None,
            kind: RequestKind::Permission,
            status: RuntimeRequestStatus::Resolved,
            response_capability: ResponseCapability::Live,
            created_at: ts(),
            resolved_at: None,
            decision: Some(ApprovalDecision::DeclineAlways),
            cancel_reason: None,
            questions: Vec::new(),
            question_title: None,
            answers: None,
        };
        let old: OldRequest = serde_json::from_value(serde_json::to_value(&req).unwrap()).unwrap();
        assert_eq!(old.decision, Some(OldDecision::Decline));
    }
}
