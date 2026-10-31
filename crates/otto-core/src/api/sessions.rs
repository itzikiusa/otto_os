//! Sessions; Broadcast (dedicated, AI-free relay to multiple sessions); Relay (name-addressed send: "ronaldo: do X", "all: stand down"); Session name themes (auto-naming new sessions: "Ronaldo", "Messi", …); Session input.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/sessions.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/sessions`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionReq {
    pub kind: SessionKind,
    /// Agent provider ("claude" | "codex" | "shell") for kind=agent;
    /// ignored for kind=connection (derived from the connection).
    pub provider: Option<String>,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub connection_id: Option<Id>,
    pub meta: Option<Value>,
    /// Model to pin for THIS session only (e.g. "claude-opus-4-8"). Copied into
    /// `meta.model` at create (winning over any `meta.model` in `meta`), which
    /// the spawn/resume path expands through the provider's `model_args`
    /// template. Absent/empty ⇒ the provider's own default model.
    #[serde(default)]
    pub model: Option<String>,
}

/// `PATCH /api/v1/sessions/{id}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSessionReq {
    pub title: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
}

/// Session list/detail responses use `otto_core::domain::Session` directly.
pub type SessionResp = Session;

/// `POST /api/v1/workspaces/{id}/sessions/open` — open an agent session and,
/// optionally, hand it an opening prompt once its TUI is up. The programmatic
/// twin of "new tab + paste the brief": a lead agent (Claude or Codex) uses it
/// to start workers it then drives with `/sessions/{id}/message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAgentSessionReq {
    /// Agent provider ("claude" | "codex" | …) — same registry as `CreateSessionReq`.
    pub provider: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Model to pin for this session only (see `CreateSessionReq::model`).
    #[serde(default)]
    pub model: Option<String>,
    /// Submitted as the session's first user message after the TUI has drawn.
    /// Delivery is asynchronous: the response returns as soon as the session
    /// row exists, with `prompt_dispatch: "queued"`.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub meta: Option<Value>,
}

/// Response of `POST /api/v1/workspaces/{id}/sessions/open`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAgentSessionResp {
    pub session: Session,
    /// `"queued"` when an opening prompt is being delivered, `"none"` otherwise.
    pub prompt_dispatch: String,
}

/// `POST /api/v1/sessions/{id}/message` — one message to ONE live agent
/// session, submitted as if typed + Enter (the targeted counterpart of
/// `/workspaces/{id}/broadcast`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageReq {
    pub text: String,
    /// Optional sender session ID when a person relays a peer message on behalf
    /// of another session in the same workspace. Ignored when authenticating
    /// with an agent session credential (its own session ID is authoritative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_session_id: Option<Id>,
}

/// Response of `POST /api/v1/sessions/{id}/message`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessageResp {
    pub session_id: Id,
    pub delivered: bool,
}

/// Response of `GET /api/v1/sessions/{id}/wait` — the session as observed when
/// one of the awaited statuses was reached, or at the deadline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitSessionResp {
    pub session: Session,
    /// True when the session's status was in the awaited set before the deadline.
    pub reached: bool,
}

// ---------------------------------------------------------------------------
// Broadcast (dedicated, AI-free relay to multiple sessions)
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/broadcast` — relay `text` verbatim to live
/// agent sessions. This is deliberately separate from the orchestrator: no
/// parsing, no AI, no fallback — it always broadcasts the literal text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastReq {
    /// The message to send. Submitted as if typed + Enter.
    pub text: String,
    /// Sessions to target. `None`/absent (or empty) → every live agent session
    /// in the workspace. When `Some`, only the listed sessions that are live
    /// agents receive it.
    #[serde(default)]
    pub session_ids: Option<Vec<Id>>,
    /// Optional sender session ID when a person relays a peer broadcast on
    /// behalf of another session in the same workspace. Ignored when
    /// authenticating with an agent session credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_session_id: Option<Id>,
}

/// Result of a broadcast: the sessions that actually received the message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastResp {
    pub session_ids: Vec<Id>,
}

// ---------------------------------------------------------------------------
// Relay (name-addressed send: "ronaldo: do X", "all: stand down")
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/relay` — deliver a message addressed to live
/// sessions BY NAME. The leading token(s) of `text` may name session handles
/// (e.g. `ronaldo: …`, `ronaldo, messi: …`, bare `ronaldo do X`) or the
/// broadcast keyword `all`/`everyone`. When no leading token matches a session,
/// the call is a no-op with `unaddressed = true` so the UI can fall back to its
/// normal handling (e.g. the AI orchestrator).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayReq {
    pub text: String,
}

/// Result of a relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayResp {
    /// Sessions the message was delivered to (empty when unaddressed).
    pub session_ids: Vec<Id>,
    /// True when the address was a broadcast keyword.
    pub broadcast: bool,
    /// True when `text` named no session — caller should fall back.
    pub unaddressed: bool,
    /// The message actually sent (address prefix stripped).
    pub text: String,
}

// ---------------------------------------------------------------------------
// Session name themes (auto-naming new sessions: "Ronaldo", "Messi", …)
// ---------------------------------------------------------------------------

/// One selectable name theme (built-in or a user's custom list).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameThemeInfo {
    /// Built-in id ("footballers") or a custom theme's id.
    pub id: String,
    pub label: String,
    /// `"builtin"` or `"custom"`.
    pub kind: String,
    /// How many distinct names the theme can yield (custom = list length).
    pub capacity: usize,
    /// A few example names for the picker preview.
    pub sample: Vec<String>,
}

/// `GET /api/v1/name-themes` — built-in themes + the caller's custom themes,
/// plus the caller's active selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameThemesResp {
    pub themes: Vec<NameThemeInfo>,
    /// The caller's active theme id: a built-in id, a custom id, or `"none"`
    /// (the legacy "{provider} #N" numbering).
    pub active: String,
}

/// `PUT /api/v1/name-themes/active` — set the caller's active theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetActiveThemeReq {
    pub theme_id: String,
}

/// `POST /api/v1/name-themes` — create a custom name theme owned by the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateNameThemeReq {
    pub label: String,
    /// Ordered list of names (e.g. family names). Empty entries are ignored at
    /// allocation time.
    pub names: Vec<String>,
}

/// `PUT /api/v1/name-themes/{id}` — replace a custom theme's label/names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateNameThemeReq {
    pub label: String,
    pub names: Vec<String>,
}

/// A custom name theme as returned to the owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomThemeResp {
    pub id: Id,
    pub label: String,
    pub names: Vec<String>,
}

// ---------------------------------------------------------------------------
// Session input
// ---------------------------------------------------------------------------

/// `POST /api/v1/sessions/{id}/input` — inject text into a session's PTY.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendInputReq {
    /// Text to write into the PTY.
    pub text: String,
    /// When `None` or `true`, append `"\n"` so the agent immediately executes
    /// the text.  When `false`, send the text verbatim (no newline) so the
    /// user can inspect / edit before pressing Enter.
    #[serde(default)]
    pub submit: Option<bool>,
}

// ---------------------------------------------------------------------------
// Conversation view (design docs/design/conversation-view.md §4.3)
// ---------------------------------------------------------------------------

/// `POST /sessions/{id}/tasks` — add a task to a session's board from the UI.
/// The row is `source: "user"`, `nudge_pending: true`; the nudge sweep hands
/// it to the agent once the session is idle.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateAgentTaskReq {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// `POST /sessions/{id}/inbox` — drop a pasted image next to the session so the
/// composer can reference it as `[Image: <path>]`. `image/*` only, ≤ 10 MB.
#[derive(Debug, Clone, Deserialize)]
pub struct InboxUploadReq {
    pub filename: String,
    pub mime: String,
    pub data_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxUploadResp {
    pub path: String,
}
