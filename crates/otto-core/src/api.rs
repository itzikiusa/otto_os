//! API request/response DTOs for `/api/v1`.
//!
//! These types are mirrored by `ui/src/lib/api/types.ts`. Endpoint shapes are
//! documented in `docs/contracts/api.md`; the WS protocol in `docs/contracts/ws.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{
    Connection, ConnectionKind, Environment, GitProviderKind, IssueProviderKind, ReviewAgentCfg,
    Session, SessionKind, User, Workspace, WorkspaceRole,
};
use crate::Id;

/// Deserialize a present-or-absent nullable field into a double-`Option`.
/// `Some(Some(v))` ⇒ an explicit value; `Some(None)` ⇒ an explicit JSON `null`;
/// `None` ⇒ the key was absent. Pair with `#[serde(default)]` so an absent key
/// stays `None` while a present `null` becomes `Some(None)`.
fn de_double_option<'de, D, T>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

// ---------------------------------------------------------------------------
// Meta / onboarding / auth
// ---------------------------------------------------------------------------

/// `GET /api/v1/meta`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaResp {
    pub version: String,
    pub api_version: u32,
    pub needs_onboarding: bool,
    pub network_listener: bool,
    /// Detected external tools: name -> found on PATH.
    pub tools: Vec<ToolStatus>,
    /// Available agent providers (from the provider registry).
    pub providers: Vec<String>,
    /// The configured default agent (a provider name), if one is set.
    /// New sessions and channel replies fall back to this when no explicit
    /// provider is chosen.
    pub default_provider: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatus {
    pub name: String,
    pub found: bool,
    pub version: Option<String>,
}

/// `POST /api/v1/onboarding/root`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardRootReq {
    pub password: String,
    pub display_name: Option<String>,
}

/// `POST /api/v1/auth/login`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginReq {
    pub username: String,
    pub password: String,
}

/// Response for login and onboarding (root is auto-logged-in).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginResp {
    pub token: String,
    pub user: User,
}

/// `GET /api/v1/auth/me` — returns both the effective and real identities so
/// the UI can render the impersonation banner and recover after a page reload.
///
/// `user` is the **effective** user (the identity authorisation runs as), kept
/// at the `user` key so callers that only need the effective user remain
/// compatible. `real_user` is the token's actual owner (the admin when
/// impersonating). `impersonating` is `true` iff `real_user.id != user.id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeResp {
    /// Effective user — the identity the session currently acts as.
    pub user: User,
    /// Real token owner — equals `user` for a normal session.
    pub real_user: User,
    /// `true` when the caller holds an impersonation token.
    pub impersonating: bool,
}

/// `POST /api/v1/auth/tokens` — mint a long-lived API (personal access) token.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateApiTokenReq {
    /// Human-friendly name, e.g. "cli" or "ci". Optional.
    pub label: Option<String>,
}

/// Metadata for one API token. NEVER carries the secret (only its prefix).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiTokenInfo {
    pub id: Id,
    pub label: Option<String>,
    /// First 12 chars of the raw token, for identification in a list.
    pub token_prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Response for `POST /api/v1/auth/tokens`: the raw secret is returned exactly
/// once (it is only ever stored hashed) alongside the new token's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiTokenResp {
    pub token: String,
    pub info: ApiTokenInfo,
}

/// Metadata for one scoped **share-link** token (mobile remote-access).
///
/// A share token is a capability bound to ONE session, default read-only, with a
/// short FIXED TTL and an explicit kill switch. Like [`ApiTokenInfo`] this NEVER
/// carries the secret (only its prefix); the raw token is returned exactly once
/// at mint time. `role` is the capped ceiling the share grants on the session —
/// `Viewer` (read-only) or `Editor` (read + input); never `Admin`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareInfo {
    pub id: Id,
    /// The single session this token may reach.
    pub session_id: Id,
    /// Capped role on that session: `Viewer` or `Editor` (never `Admin`).
    pub role: WorkspaceRole,
    /// First 12 chars of the raw token, for identification in a list.
    pub token_prefix: String,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
    /// FIXED expiry (`created_at + ttl`); never slid for share tokens.
    pub expires_at: DateTime<Utc>,
}

/// `POST /api/v1/sessions/{id}/share` — mint a scoped share-link token.
///
/// `role` must be `"viewer"` or `"editor"` (never `"admin"`). `ttl_secs`
/// defaults to 3600 and is clamped server-side to `[60, 86400]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateShareReq {
    /// `"viewer"` (read-only) or `"editor"` (read + input). Never `"admin"`.
    pub role: String,
    /// Fixed TTL in seconds. Absent → 3600. Clamped to `[60, 86400]`.
    /// Ignored when `recipient_email` is set (then `duration_secs` governs the
    /// session window instead).
    #[serde(default)]
    pub ttl_secs: Option<i64>,
    /// Human-friendly label (e.g. "for Alice"). Optional.
    #[serde(default)]
    pub label: Option<String>,
    /// Email-OTP gate (mobile plan Task 7.2). When set, the recipient must redeem
    /// a 6-digit code emailed to THIS address (via `POST /api/v1/share/verify`)
    /// before the share can attach — a leaked link alone is then useless. The
    /// address is LOCKED for the share's lifetime (Task 7.4 extension re-emails
    /// only this address). Requires the caller to have a verified email sender;
    /// absent → a plain scoped share with no OTP gate (backward compatible).
    #[serde(default)]
    pub recipient_email: Option<String>,
    /// Session window for an OTP-gated share, in seconds — how long the guest may
    /// stay attached once verified. Clamped server-side to `(0, 43200]` (≤12h).
    /// Only meaningful with `recipient_email`; absent → default 1h.
    #[serde(default)]
    pub duration_secs: Option<i64>,
}

/// `POST /api/v1/share/verify` — redeem an emailed OTP for a share token
/// (mobile plan Task 7.3). Public/Exempt: the `token` (the share link) is the
/// auth. On success the share's `verified_at` is set and the guest may attach
/// (`/ws/term`) until the share's `max_expires_at` (≤12h). Single-use + IP
/// rate-limited.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyShareReq {
    /// The raw share token (from the `#/s/<session>/<token>` link).
    pub token: String,
    /// The 6-digit one-time code the recipient received by email.
    pub otp: String,
}

/// Response for `POST /api/v1/share/verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyShareResp {
    /// `true` once the code matched and the share is now verified for attach.
    pub verified: bool,
}

/// `POST /api/v1/share/extend` — re-issue a FRESH OTP for an existing OTP share,
/// emailed to the **LOCKED original recipient ONLY** (mobile plan Task 7.4).
///
/// Public/Exempt: the `token` (the share link) is the auth. The request carries
/// NO email field by design — the destination is read from the share row's
/// immutable `recipient_email`, never from the request. This prevents redirecting
/// access to a different mailbox. The fresh code re-pends the share
/// (`verified_at` cleared) and opens a fresh ≤12h window; the guest re-verifies
/// via `POST /api/v1/share/verify`. IP rate-limited (the share throttle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtendShareReq {
    /// The raw share token (from the `#/s/<session>/<token>` link).
    pub token: String,
}

/// Response for `POST /api/v1/sessions/{id}/share`. The raw token is returned
/// exactly once (only its SHA-256 hash is stored). `url` is the ready-to-share
/// fragment URL (`<origin>/#/s/<session_id>/<token>`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateShareResp {
    /// The raw share token (shown exactly once — store it safely).
    pub token: String,
    /// Ready-to-use share URL (`<origin>/#/s/<session_id>/<token>`).
    pub url: String,
    /// Metadata for the newly-minted share.
    pub info: ShareInfo,
}

/// `GET /api/v1/sessions/{id}/shares` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListSharesResp {
    pub shares: Vec<ShareInfo>,
}

/// `PUT /api/v1/email-sender` — configure the caller's Gmail App Password sender
/// (foundation of the email-OTP share gate, mobile plan Task 7.1).
///
/// The `app_password` is stored in the macOS **Keychain** (never the DB); only an
/// opaque `secret_ref` is persisted. The handler validates the pair via a real
/// Gmail SMTP login before marking it verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetEmailSenderReq {
    /// The Gmail address mail is sent from (also the SMTP AUTH username).
    pub gmail_address: String,
    /// The 16-char Gmail App Password (SMTP AUTH password). Never stored in the
    /// DB nor echoed back.
    pub app_password: String,
}

/// Response for `PUT` and `GET /api/v1/email-sender`. NEVER carries the app
/// password. `gmail_address` is absent on `GET` when no sender is configured;
/// `verified` is `true` once a real SMTP login with the app password succeeded.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmailSenderResp {
    /// The configured Gmail address, or `None` when no sender is set up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gmail_address: Option<String>,
    /// `true` once the app password passed a real Gmail SMTP login.
    pub verified: bool,
}

// ---------------------------------------------------------------------------
// Grants / capabilities (RBAC Task 2.1)
// ---------------------------------------------------------------------------

/// One `(feature, capability)` grant entry, serialised as snake_case strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantEntry {
    pub feature: String,
    pub capability: String,
}

/// `GET /api/v1/users/{id}/grants` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGrantsResp {
    pub grants: Vec<GrantEntry>,
}

/// `PUT /api/v1/users/{id}/grants` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGrantsReq {
    pub grants: Vec<GrantEntry>,
}

/// `GET /api/v1/auth/capabilities` response — the caller's effective
/// `{feature: capability}` map.  Root users receive `admin` for every feature.
/// Any authenticated user may call this endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilitiesResp {
    /// snake_case feature name → snake_case capability string.
    pub capabilities: std::collections::HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// Admin active-sessions overview (RBAC Task 4.2)
// ---------------------------------------------------------------------------

/// One row of the daemon-wide admin active-sessions overview
/// (`GET /api/v1/admin/sessions`). Each entry is a persisted session enriched
/// with its owner's username and the in-memory live state from the
/// `SessionManager` (`live`, `viewers`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSessionRow {
    /// Session id.
    pub id: String,
    /// `created_by` — the owning user's id.
    pub owner_id: String,
    /// The owning user's username (resolved via a batch user load); falls back
    /// to the owner id when the user row no longer exists.
    pub owner_username: String,
    /// Workspace the session belongs to.
    pub workspace_id: String,
    /// `agent` | `connection`.
    pub kind: String,
    /// CLI provider / connection driver (`claude`, `codex`, `shell`, `mysql`, …).
    pub provider: String,
    /// Display title.
    pub title: String,
    /// Persisted status (`running`/`working`/`idle`/`reconnectable`/`exited`).
    pub status: String,
    /// True when the session has a live PTY in this daemon process
    /// (`SessionManager::is_live`).
    pub live: bool,
    /// Number of WS terminal viewers currently attached
    /// (`SessionManager::attached_count`).
    pub viewers: u32,
}

/// `GET /api/v1/admin/sessions` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSessionsResp {
    pub sessions: Vec<AdminSessionRow>,
}

// ---------------------------------------------------------------------------
// Impersonation (RBAC Task 5.2)
// ---------------------------------------------------------------------------

/// `POST /api/v1/admin/impersonate/{user_id}` response — the short-lived
/// impersonation bearer token the admin swaps to in order to act as the target
/// user. The raw secret is returned exactly once (only its hash is stored).
/// Every authorization decision then runs against the *target* (effective)
/// user; every audit entry records the *admin* (real) user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpersonateResp {
    pub token: String,
}

// ---------------------------------------------------------------------------
// Users / workspaces
// ---------------------------------------------------------------------------

/// `POST /api/v1/users` (root only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateUserReq {
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
}

/// `PATCH /api/v1/users/{id}` (root only)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateUserReq {
    pub display_name: Option<String>,
    pub password: Option<String>,
    pub disabled: Option<bool>,
}

/// `POST /api/v1/workspaces`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWorkspaceReq {
    pub name: String,
    pub root_path: String,
}

/// `PATCH /api/v1/workspaces/{id}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceReq {
    pub name: Option<String>,
    pub root_path: Option<String>,
    pub settings: Option<Value>,
    pub archived: Option<bool>,
}

/// One row of `GET /api/v1/workspaces/{id}/members`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberEntry {
    pub user_id: Id,
    pub username: String,
    pub display_name: String,
    pub role: WorkspaceRole,
}

/// `PUT /api/v1/workspaces/{id}/members` — full replacement list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMembersReq {
    pub members: Vec<SetMemberEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetMemberEntry {
    pub user_id: Id,
    pub role: WorkspaceRole,
}

/// Workspace plus the calling user's role in it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceWithRole {
    #[serde(flatten)]
    pub workspace: Workspace,
    pub my_role: WorkspaceRole,
}

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

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/orchestrate`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrateReq {
    pub text: String,
    #[serde(default)]
    pub optimize: bool,
    #[serde(default)]
    pub ai_fallback: bool,
    /// Session that currently has focus in the UI (fallback target).
    pub focused_session_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrateResp {
    pub plan: ActionPlan,
    /// The optimized prompt, when `optimize` was requested.
    pub optimized_text: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/orchestrate/execute`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutePlanReq {
    pub plan: ActionPlan,
}

pub type ActionPlan = Vec<Action>;

/// A single orchestrator action, produced by parsing plain English.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Spawn `count` agent sessions of `provider`.
    SpawnSessions { provider: String, count: u8 },
    /// Send `text` to every running agent session in the workspace.
    Broadcast { text: String },
    /// Open a saved connection as a new session.
    OpenConnection { connection_id: Id },
    /// Send `text` to one specific session.
    RunCommand { session_id: Id, text: String },
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
// Connections
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/connections` and PATCH variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertConnectionReq {
    pub name: String,
    pub kind: ConnectionKind,
    pub params: Value,
    /// Write-only secret; stored in Keychain, never echoed back.
    pub secret: Option<String>,
    pub first_command: Option<String>,
    /// Section to place this profile in; None/absent = ungrouped.
    #[serde(default)]
    pub section_id: Option<Id>,
    /// Deployment environment (dev/staging/prod). On create, absent = `Dev`.
    /// On PATCH, absent KEEPS the stored value (so a PATCH that omits it never
    /// silently downgrades a `Prod` connection to `Dev` and disables the guard).
    /// `Prod` connections are write-guarded in the DB Explorer.
    #[serde(default)]
    pub environment: Option<Environment>,
    /// Lock the profile against writes/DDL regardless of environment. On create,
    /// absent = `false`. On PATCH, absent KEEPS the stored value (so a PATCH that
    /// omits it never silently un-locks a read-only connection).
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// `POST /api/v1/workspaces/{id}/connection-sections` and `PATCH /connection-sections/{id}`.
/// On create, `parent_id` nests the section (absent/None = top-level); rename ignores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertSectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
    /// Tree this section lives in on create: "connections" (default) or "db".
    /// Ignored on rename. See [`crate::domain::ConnectionSection::scope`].
    #[serde(default)]
    pub scope: Option<String>,
}

/// Query string for `GET /workspaces/{id}/connection-sections?scope=…`.
/// Absent → "connections" (the Connections page tree).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SectionScopeQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/connection-sections/reorder`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorderSectionsReq {
    pub ids: Vec<Id>,
}

/// `POST /api/v1/connection-sections/{id}/move` — reparent (None = top-level).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveSectionReq {
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// `POST /api/v1/workspaces/{id}/mcp-servers` — create a workspace MCP server.
/// `enabled` defaults to `false` (servers are never auto-enabled).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMcpServerReq {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Usage budgets (spend caps, opt-in enforcement)
// ---------------------------------------------------------------------------

/// Per-workspace spend cap (USD over the budget window).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceBudget {
    pub workspace_id: Id,
    /// Cap in USD for the window. `0` (or absent) = no cap for this workspace.
    pub monthly_usd: f64,
}

/// Per-provider spend cap (USD over the budget window).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderBudget {
    pub provider: String,
    pub monthly_usd: f64,
}

/// The full usage-budget configuration, persisted under the `usage_budgets`
/// settings key. Enforcement is **opt-in**: `enforce` defaults to `false`, so a
/// budget never blocks or even warns until a root user turns it on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageBudgetConfig {
    /// Master opt-in flag. When `false`, budgets are purely informational and
    /// the daemon's budget check is a no-op.
    #[serde(default)]
    pub enforce: bool,
    /// When `true` (and `enforce` is on) an exceeded budget is a hard block;
    /// when `false` it only warns prominently. Default `false` (warn-only).
    #[serde(default)]
    pub block_on_exceed: bool,
    /// Window the caps apply to, in days (approximates a calendar month).
    /// Default 30, clamped 1..=3650 at the route.
    #[serde(default)]
    pub window_days: u32,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceBudget>,
    #[serde(default)]
    pub providers: Vec<ProviderBudget>,
}

/// One budget vs. its current spend, as surfaced in the Usage UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetStatusRow {
    /// "workspace" or "provider".
    pub scope: String,
    /// The workspace id or provider name.
    pub key: String,
    /// Human label (workspace name, or the provider name) when resolvable.
    #[serde(default)]
    pub label: Option<String>,
    pub limit_usd: f64,
    pub spent_usd: f64,
    /// `spent / limit` (0 when no limit).
    pub used_fraction: f64,
    /// True once spend crosses the warn threshold (80% of the cap).
    pub warning: bool,
    /// True once spend meets/exceeds the cap.
    pub exceeded: bool,
}

/// `GET /api/v1/usage/budgets` response: the config plus its live status rows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageBudgetStatus {
    pub config: UsageBudgetConfig,
    /// Window actually used to compute spend (after clamping/defaults).
    pub window_days: u32,
    pub rows: Vec<BudgetStatusRow>,
}

/// `PATCH /api/v1/mcp-servers/{id}` — partial update; absent fields are kept.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateMcpServerReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// `POST /api/v1/connections/{id}/test`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestConnectionResp {
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub message: String,
    /// True when the built command unavoidably exposes the secret in argv
    /// (clickhouse-client) — UI shows a warning banner.
    pub warn_argv: bool,
    /// Set when the SSH private key file used by this connection has insecure
    /// (group/other-readable) permissions — OpenSSH may refuse it. Carries a
    /// human-readable message including the exact `chmod 600 <path>` fix. This
    /// warning is independent of whether the test itself succeeded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warn_key_perms: Option<String>,
}

/// Connection responses use the domain type (secret_ref is opaque).
pub type ConnectionResp = Connection;

// ---------------------------------------------------------------------------
// SFTP file browser (over an SSH connection's existing auth)
// ---------------------------------------------------------------------------

/// One entry in a remote directory listing (`GET /connections/{id}/sftp/list`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpEntry {
    pub name: String,
    /// "dir" | "file" | "symlink" | "other".
    pub kind: String,
    pub size: u64,
    /// Raw date/time field from the listing (e.g. "Jun 20 12:00"), if present.
    pub mtime: Option<String>,
    /// The 10-char permission string (e.g. "drwxr-xr-x").
    pub perms: String,
    /// For symlinks, the link target (part after " -> "); `None` otherwise.
    pub symlink_target: Option<String>,
}

/// `GET /api/v1/connections/{id}/sftp/list?path=` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpListResp {
    /// The absolute remote path that was listed (resolved from `pwd` when the
    /// request omitted `path`).
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

/// `POST /api/v1/connections/{id}/sftp/download` — pull a remote file to local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpDownloadReq {
    pub remote_path: String,
    /// Local destination (file path, or a dir for the default-name case).
    /// A leading `~` is expanded to the daemon user's home.
    pub local_path: String,
}

/// `POST /api/v1/connections/{id}/sftp/download` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpDownloadResp {
    pub local_path: String,
    pub bytes: u64,
}

/// `POST /api/v1/connections/{id}/sftp/upload` — push a local file to remote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpUploadReq {
    /// Local source (leading `~` expanded to the daemon user's home).
    pub local_path: String,
    pub remote_path: String,
}

/// `POST /api/v1/connections/{id}/sftp/mkdir`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpMkdirReq {
    pub path: String,
}

/// `POST /api/v1/connections/{id}/sftp/remove` — `dir=true` → rmdir, else rm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpRemoveReq {
    pub path: String,
    #[serde(default)]
    pub dir: bool,
}

/// `POST /api/v1/connections/{id}/sftp/rename`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpRenameReq {
    pub from: String,
    pub to: String,
}

/// `GET /api/v1/connections/{id}/sftp/read?path=` — text view of a small file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpReadResp {
    pub text: String,
    /// True when the file exceeded the read cap (content is the capped prefix).
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Git: accounts, repos, local ops
// ---------------------------------------------------------------------------

/// `POST /api/v1/git/accounts`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateGitAccountReq {
    pub provider: GitProviderKind,
    pub label: String,
    pub username: String,
    /// Write-only; stored in Keychain.
    pub token: String,
    pub api_base_url: Option<String>,
    /// Namespace for remote-repo browsing: Bitbucket workspace, GitHub org,
    /// GitLab group. Optional.
    pub namespace: Option<String>,
    /// Optional user-entered token expiry (for providers that don't expose it,
    /// e.g. Bitbucket). Drives expiry notifications.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

/// `POST /api/v1/issue/accounts`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIssueAccountReq {
    pub provider: IssueProviderKind,
    pub label: String,
    pub email: String,
    pub base_url: String,
    /// Write-only; stored in Keychain.
    pub token: String,
    /// Optional user-entered token expiry. Drives expiry notifications.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

/// `PATCH /api/v1/git/accounts/{id}`
/// Any field present is updated; absent fields keep their current value.
/// `namespace` / `api_base_url`: empty string clears to NULL, non-empty sets, absent keeps.
/// `token`: non-empty rotates the Keychain secret; empty/absent keeps the existing secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateGitAccountReq {
    pub label: Option<String>,
    pub username: Option<String>,
    /// Empty string → clear to NULL; non-empty → set; absent (None) → keep current.
    pub namespace: Option<String>,
    /// Empty string → clear to NULL; non-empty → set; absent (None) → keep current.
    pub api_base_url: Option<String>,
    /// Non-empty → rotate Keychain secret; empty/absent → keep existing.
    pub token: Option<String>,
    /// Set the user-entered token expiry; absent (None) → keep current.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

/// `PATCH /api/v1/issue/accounts/{id}`
/// Any field present is updated; absent fields keep their current value.
/// `token`: non-empty rotates the Keychain secret; empty/absent keeps the existing secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateIssueAccountReq {
    pub label: Option<String>,
    pub email: Option<String>,
    pub base_url: Option<String>,
    /// Non-empty → rotate Keychain secret; empty/absent → keep existing.
    pub token: Option<String>,
    /// Set the user-entered token expiry; absent (None) → keep current.
    #[serde(default)]
    pub token_expires_at: Option<DateTime<Utc>>,
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// `GET /api/v1/notifications/settings` and `PUT` body (all fields required on PUT).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    /// Notify this many days before a credential expires.
    pub expiry_threshold_days: u32,
    /// Raise native OS notifications for warn/error notices.
    pub native_enabled: bool,
    /// Emit notices for session-progress events (finished / awaiting-input / exited).
    pub session_events: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            expiry_threshold_days: 3,
            native_enabled: true,
            session_events: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Trust & Safety: audit log + security posture (root only)
// ---------------------------------------------------------------------------

/// Query string for `GET /api/v1/audit-log` (root only). All filters optional;
/// `from`/`to` bound `ts` (inclusive). `limit`/`offset` page the result newest
/// first; `limit` is clamped server-side.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuditLogQuery {
    /// Lower bound on `ts` (RFC3339), inclusive.
    pub from: Option<DateTime<Utc>>,
    /// Upper bound on `ts` (RFC3339), inclusive.
    pub to: Option<DateTime<Utc>>,
    /// Exact `action` match, e.g. `"login.success"`.
    pub action: Option<String>,
    /// Exact acting-user match.
    pub user_id: Option<Id>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// `GET /api/v1/audit-log` — a page of audit entries plus the total matching the
/// filters (so the UI can paginate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogResp {
    pub entries: Vec<crate::domain::AuditEntry>,
    /// Total rows matching the filters (ignores `limit`/`offset`).
    pub total: i64,
}

/// `GET /api/v1/security-posture` (root only) — a snapshot the Trust & Safety
/// Center renders, derived from settings + the auth store. No new state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPostureResp {
    /// Whether the daemon's network (0.0.0.0) listener is enabled.
    pub network_listener: bool,
    /// The port the network listener binds when enabled (None = daemon default).
    pub network_listener_port: Option<u16>,
    /// True when no network listener is enabled (daemon is loopback-only).
    pub loopback_only: bool,
    /// Count of currently-active (unexpired) API (personal access) tokens.
    pub active_api_tokens: i64,
}

/// `POST /api/v1/workspaces/{id}/repos` — register existing path or clone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddRepoReq {
    /// Existing local path to register; mutually exclusive with `clone_url`.
    pub path: Option<String>,
    /// Remote URL to clone into the workspace directory.
    pub clone_url: Option<String>,
    pub name: Option<String>,
    pub git_account_id: Option<Id>,
    /// Optional parent directory to clone INTO (the repo lands at
    /// `<clone_dir>/<name>`). A leading `~` is expanded. Defaults to the
    /// workspace `root_path` when absent. Only meaningful with `clone_url`.
    #[serde(default)]
    pub clone_dir: Option<String>,
}

/// One changed file in `GET /repos/{id}/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub orig_path: Option<String>,
    /// "modified" | "added" | "deleted" | "renamed" | "untracked" | "conflicted"
    pub kind: String,
    pub staged: bool,
    pub unstaged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatusResp {
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub subject: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefBranch {
    pub name: String,
    pub is_current: bool,
    pub upstream: Option<String>,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefTag {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefsResp {
    pub local: Vec<RefBranch>,
    pub remote: Vec<RefBranch>,
    pub tags: Vec<RefTag>,
}

/// One entry from `git stash list`. Read-only; surfaced in the graph's STASHES
/// sidebar section and rendered as a dashed node in the commit graph when the
/// stash commit happens to be present in the (`--all`) log. `parents` are
/// `[base, index, (untracked)]` — the helper commits the frontend de-emphasises.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StashInfo {
    /// N in `stash@{N}` (0 = most recent).
    pub index: u32,
    /// Reflog selector, e.g. "stash@{0}".
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// Full SHA of the stash (WIP) commit.
    pub sha: String,
    /// Parent SHAs: `[base, index, (untracked)]`.
    #[serde(default)]
    pub parents: Vec<String>,
    /// Author date, ISO 8601 (the frontend formats it).
    pub date: String,
    /// Reflog subject, e.g. "On main: my WIP work".
    pub message: String,
    /// Branch the stash was created on, parsed from the message (if present).
    pub branch: Option<String>,
}

/// One line origin in a diff hunk: "context" | "add" | "del".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineOrigin {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLine {
    pub origin: LineOrigin,
    pub content: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub is_binary: bool,
    pub hunks: Vec<Hunk>,
    /// True when the file diff was capped server-side due to size.
    #[serde(default)]
    pub too_large: Option<bool>,
    /// Number of added lines (populated by the diff parser).
    #[serde(default)]
    pub added: Option<u32>,
    /// Number of deleted lines (populated by the diff parser).
    #[serde(default)]
    pub deleted: Option<u32>,
    /// Detected language hint (e.g. "rust", "typescript").
    #[serde(default)]
    pub language: Option<String>,
}

/// `GET /repos/{id}/diff?target=worktree|staged|commit:<sha>|range:<a>..<b>`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResp {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagePathsReq {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitReq {
    pub message: String,
    #[serde(default)]
    pub amend: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutReq {
    pub branch: String,
    #[serde(default)]
    pub create: bool,
}

// ---------------------------------------------------------------------------
// Git: local merge + conflict resolution (#4)
// ---------------------------------------------------------------------------

/// Strategy for a local branch merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalMergeStrategy {
    /// `git merge --no-ff` — always create a merge commit.
    MergeCommit,
    /// `git merge` — fast-forward when possible, otherwise a merge commit.
    Ff,
    /// `git merge --ff-only` — fail (no write) when not fast-forwardable.
    FfOnly,
    /// `git merge --squash` — stage the merge as a single commit (no merge parent).
    Squash,
}

fn default_local_merge_strategy() -> LocalMergeStrategy {
    LocalMergeStrategy::MergeCommit
}

/// `POST /repos/{id}/merge` — merge `source` into `target` (target is checked
/// out first). Never auto-resolves conflicts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeBranchReq {
    pub source: String,
    pub target: String,
    #[serde(default = "default_local_merge_strategy")]
    pub strategy: LocalMergeStrategy,
    /// When true and the working tree is dirty, auto-stash before the merge and
    /// pop the stash afterwards (stash → merge → pop). When false, a dirty tree
    /// is refused. Default false.
    #[serde(default)]
    pub auto_stash: bool,
}

/// `POST /repos/{id}/merge/preview` — dry-run a merge of `source` into `target`
/// using `git merge-tree` (writes only to the object DB; the working tree and
/// index are NOT touched). Lets the UI warn about conflicts BEFORE starting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePreviewReq {
    pub source: String,
    pub target: String,
}

/// Result of a merge dry-run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePreview {
    /// True if merging would produce conflicts.
    pub conflicts: bool,
    /// Files that would conflict (best-effort; from `merge-tree --name-only`).
    #[serde(default)]
    pub conflicted_files: Vec<String>,
    /// True if the merge would be a no-op (source already in target).
    #[serde(default)]
    pub up_to_date: bool,
}

/// Outcome of a local merge or merge-completion. Conflicts are a NORMAL 200
/// result (`status == "conflicts"`), not an error, so the resolver can read the
/// conflicted-file list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeResult {
    /// "merged" | "conflicts" | "up_to_date"
    pub status: String,
    /// New HEAD sha when merged; None for conflicts / up_to_date.
    pub commit: Option<String>,
    #[serde(default)]
    pub conflicted_files: Vec<String>,
    /// Fresh repo status after the operation.
    pub repo_status: RepoStatusResp,
    /// Optional human-readable note (e.g. auto-stash outcome) for the UI to
    /// surface as a toast. None for the common case.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `GET /repos/{id}/merge/status`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConflictStatus {
    /// True when a merge is in progress (`MERGE_HEAD` present).
    pub merging: bool,
    /// Best-effort source branch/ref being merged, when known.
    pub source: Option<String>,
    #[serde(default)]
    pub conflicted_files: Vec<String>,
}

/// One segment of a conflicted file: either shared context or a conflict region
/// with both sides (and the merge base when diff3 data is available).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConflictSegment {
    Context {
        lines: Vec<String>,
    },
    Conflict {
        ours: Vec<String>,
        theirs: Vec<String>,
        #[serde(default)]
        base: Vec<String>,
    },
}

/// `GET /repos/{id}/conflict?path=<p>` — a conflicted file split into segments
/// so the client can render each conflict and deterministically rebuild the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictFile {
    pub path: String,
    pub is_binary: bool,
    pub segments: Vec<ConflictSegment>,
}

/// `POST /repos/{id}/conflict/resolve` — write the fully-resolved file content
/// (markers removed) and stage it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveConflictReq {
    pub path: String,
    pub content: String,
}

/// `POST /repos/{id}/merge/commit` — finish an in-progress merge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCommitReq {
    /// Commit message; when omitted, the prepared MERGE_MSG / `--no-edit` is used.
    #[serde(default)]
    pub message: Option<String>,
}

// ---------------------------------------------------------------------------
// Git: pull requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Declined,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSummary {
    /// Provider-native id used in follow-up calls (number for GH/GL, id for BB).
    pub number: u64,
    pub title: String,
    pub author: String,
    pub state: PrState,
    pub source_branch: String,
    pub target_branch: String,
    pub updated_at: DateTime<Utc>,
    pub url: String,
    /// True if this is a draft PR.
    #[serde(default)]
    pub draft: Option<bool>,
    /// Simplified CI/check status: "passing" | "failing" | "pending" | "unknown".
    #[serde(default)]
    pub ci_status: Option<String>,
    /// Labels on the PR (empty when provider doesn't expose them).
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrComment {
    pub id: String,
    pub author: String,
    pub body: String,
    /// File path + line for inline comments; None for general comments.
    pub path: Option<String>,
    pub line: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub replies: Vec<PrComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrDetail {
    #[serde(flatten)]
    pub summary: PrSummary,
    pub description_md: String,
    pub comments: Vec<PrComment>,
    /// Display names of approvers (kept for back-compat; see `reviewers`).
    pub approved_by: Vec<String>,
    /// Structured reviewers with approval state and best-effort avatar/timestamp.
    #[serde(default)]
    pub reviewers: Vec<PrReviewer>,
    pub mergeable: Option<bool>,
}

/// A PR reviewer/participant with their approval state. `avatar_url` and
/// `reviewed_at` are best-effort (provider-dependent; None when unavailable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReviewer {
    pub name: String,
    pub approved: bool,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub reviewed_at: Option<DateTime<Utc>>,
}

/// One commit on a pull/merge request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrCommit {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub date: DateTime<Utc>,
    pub subject: String,
}

/// `POST /repos/{id}/prs/{number}/request-changes`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestChangesReq {
    #[serde(default)]
    pub body: Option<String>,
}

/// `GET /prs/...` diff reuses `DiffResp`.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePrReq {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
    /// Optional proof pack to gate this PR on. When set, Otto refuses to open the
    /// PR unless the pack is `passed`/`waived` (or `allow_unproven` is set).
    #[serde(default)]
    pub proof_pack_id: Option<String>,
    /// Open the PR even if its proof pack isn't passed — records an audit
    /// `approval` artifact on the pack ("PR opened over unproven proof").
    #[serde(default)]
    pub allow_unproven: Option<bool>,
}

/// `POST /repos/{id}/pr/draft` — ask an agent to draft a PR title + description
/// from the current branch's diff against `base`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPrReq {
    pub base: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftPrResp {
    pub title: String,
    pub description: String,
    pub source_branch: String,
    pub target_branch: String,
}

/// `POST /repos/{id}/draft-commit-message` — ask an agent to draft a commit
/// message from the staged diff (falls back to the full working diff when
/// nothing is staged). No request body is required.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DraftCommitMessageReq {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftCommitMessageResp {
    pub message: String,
    /// Whether the message was drafted from the staged diff (`true`) or fell
    /// back to the full working diff because nothing was staged (`false`).
    pub from_staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePrReq {
    pub title: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPrCommentReq {
    pub body: String,
    pub path: Option<String>,
    pub line: Option<u32>,
    /// Reply to an existing comment id, if any.
    pub in_reply_to: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergePrReq {
    #[serde(default = "default_merge_strategy")]
    pub strategy: MergeStrategy,
}

fn default_merge_strategy() -> MergeStrategy {
    MergeStrategy::Merge
}

// ---------------------------------------------------------------------------
// Channels / workspace integrations
// ---------------------------------------------------------------------------

/// `PUT /api/v1/workspaces/{id}/integrations/{channel}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertIntegrationReq {
    pub enabled: bool,
    /// Write-only; when Some and non-empty the token is (re-)stored in keychain.
    pub bot_token: Option<String>,
    /// Write-only; Slack app-level token (slack only).
    pub app_token: Option<String>,
    pub allowed_users: String,
    pub agent_reply: bool,
    pub reply_instructions: String,
    pub channel_id: String,
    /// Preferred agent CLI for this channel. Empty = use the default agent.
    #[serde(default)]
    pub preferred_cli: String,
}

// ---------------------------------------------------------------------------
// Provider updates
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/providers/update`
///
/// When `provider` is `Some`, only that provider's update command is run.
/// When `None`, all providers with a configured update command are updated.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateProvidersReq {
    /// Optionally restrict to a single named provider.
    pub provider: Option<String>,
}

// ---------------------------------------------------------------------------
// PR Review config
// ---------------------------------------------------------------------------

/// Persisted configuration for the PR review pipeline.
/// Stored in the `settings` table under the key `pr_review`.
/// `GET /api/v1/settings/pr-review` and `PUT /api/v1/settings/pr-review`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    pub agents: Vec<ReviewAgentCfg>,
    pub summarizer: ReviewAgentCfg,
    /// User-defined reusable reviewer presets the UI offers alongside the
    /// built-in ones. Persisted with the config; not used by the runner.
    #[serde(default)]
    pub custom_presets: Vec<ReviewAgentCfg>,
    /// Maximum total attempts per agent (initial + retries). `None` → default 3.
    #[serde(default)]
    pub max_attempts: Option<u32>,
    /// Per-agent timeout in seconds. When set, overrides the diff-size heuristic.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

// ---------------------------------------------------------------------------
// Goal Loops
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/goal-loops/define` — run the AI goal-definer to
/// turn a rough seed into a structured, loop-executable draft. Persists nothing.
/// Supplying `feedback` (with the prior draft echoed in `context`) refines it.
#[derive(Debug, Clone, Deserialize)]
pub struct DefineGoalReq {
    /// The rough goal text the user typed.
    pub seed: String,
    /// The repo the loop will work in (gives the definer codebase context).
    pub repo_path: String,
    /// Optional extra guidance, or the prior draft when refining.
    #[serde(default)]
    pub context: Option<String>,
    /// When refining: what to change about the prior draft.
    #[serde(default)]
    pub feedback: Option<String>,
}

/// The definer's structured suggestion. The user edits this before launching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalLoopDraft {
    pub definition: crate::domain::GoalLoopDefinition,
    pub suggested_limits: crate::domain::GoalLoopLimits,
    pub suggested_config: crate::domain::GoalLoopConfig,
}

/// `POST /api/v1/workspaces/{id}/goal-loops` — create a loop (optionally start it).
#[derive(Debug, Clone, Deserialize)]
pub struct CreateGoalLoopReq {
    pub name: String,
    pub repo_path: String,
    pub definition: crate::domain::GoalLoopDefinition,
    pub limits: crate::domain::GoalLoopLimits,
    pub config: crate::domain::GoalLoopConfig,
    /// When true, the controller starts immediately after creation.
    #[serde(default)]
    pub autostart: bool,
}

/// `PATCH /api/v1/goal-loops/{id}` — edit a loop. `config` is editable in Draft
/// only (reshaping executors mid-run breaks live agent indices); `limits` may be
/// raised while Paused/Blocked/Exhausted; `name` any non-terminal state.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateGoalLoopReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub limits: Option<crate::domain::GoalLoopLimits>,
    #[serde(default)]
    pub config: Option<crate::domain::GoalLoopConfig>,
}

// ---------------------------------------------------------------------------
// PR Review start request
// ---------------------------------------------------------------------------

/// `POST /api/v1/repos/{id}/prs/{number}/review` — optional request body.
/// All fields default to `None` so an empty (or absent) body still parses.
#[derive(Debug, Default, Deserialize)]
pub struct StartReviewReq {
    #[serde(default)]
    pub issue_account_id: Option<String>,
    #[serde(default)]
    pub issue_key: Option<String>,
    /// Free-text guidance from the user (e.g. "what to focus on"). Passed to the
    /// review agents alongside the diff. Empty/absent behaves as before.
    #[serde(default)]
    pub context: Option<String>,
}

// ---------------------------------------------------------------------------
// Local review
// ---------------------------------------------------------------------------

/// `POST /api/v1/repos/{id}/local-review` — start a review of the local
/// working tree against a base branch/ref.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewReq {
    /// The base git ref to diff against (e.g. `origin/develop`, `main`).
    pub base: String,
}

/// `POST /api/v1/reviews/{id}/handoff` — hand review findings to a new agent
/// session so the agent can fix them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffReq {
    /// The provider to spawn (e.g. "claude", "codex", "agy").
    pub provider: String,
    /// Optional list of comment ids to include. When `None`, all non-declined
    /// comments are included. When `Some`, only the listed comments are sent.
    #[serde(default)]
    pub comment_ids: Option<Vec<String>>,
}

/// Where a handover brief is delivered: a freshly spawned agent, or an existing
/// running agent in the same workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoverTarget {
    /// Spawn a new agent of `provider` ("claude" | "codex" | "agy" | …).
    NewAgent { provider: String },
    /// Inject into an existing agent session already in this workspace.
    ExistingSession { session_id: Id },
}

/// `POST /api/v1/sessions/{id}/handover` — pass the source agent's working
/// context (summarized best-effort, optionally with git state) into the target
/// agent, so it can continue the work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverReq {
    /// Where to deliver the brief.
    pub target: HandoverTarget,
    /// Free-text note describing what the receiving agent should focus on. The
    /// generated brief is weighted toward this.
    #[serde(default)]
    pub focus: Option<String>,
    /// Title for the new session (NewAgent only). Defaults to "Handover from <source>".
    #[serde(default)]
    pub title: Option<String>,
    /// A pre-generated/edited brief. When present, the server skips
    /// summarization and injects this verbatim (the "review before sending" flow).
    #[serde(default)]
    pub brief: Option<String>,
    /// Include the repo's git state (branch, changed files, recent commits) in
    /// the generated brief. Ignored when `brief` is supplied. Defaults to true.
    #[serde(default)]
    pub include_git: Option<bool>,
    /// Summarize with a fast model (haiku) instead of the default. Ignored when
    /// `brief` is supplied. Defaults to false.
    #[serde(default)]
    pub fast: Option<bool>,
    /// Archive the source session once the handover is sent. Defaults to false.
    #[serde(default)]
    pub archive_source: Option<bool>,
}

/// `POST /api/v1/sessions/{id}/handover/brief` — generate the handover brief
/// (synchronously) so the user can review/edit it before sending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverBriefReq {
    #[serde(default)]
    pub focus: Option<String>,
    #[serde(default)]
    pub include_git: Option<bool>,
    #[serde(default)]
    pub fast: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverBriefResp {
    /// The generated brief (markdown). Empty when there was no context at all.
    pub brief: String,
    /// True when summarization was unavailable and `brief` is raw context.
    pub fallback: bool,
    /// True when some source context (transcript/scrollback/git) was found.
    pub had_context: bool,
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
// Problem response
// ---------------------------------------------------------------------------

/// Error body returned by every endpoint on failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Problem {
    pub code: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Agent self-improvement
// ---------------------------------------------------------------------------

/// Per-workspace self-reflection config (stored under
/// `Workspace.settings.self_improvement`). `last_run_at`/`next_run_at` are
/// scheduler-managed; the rest are user-editable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfImprovementConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_cadence_minutes")]
    pub cadence_minutes: u32,
    #[serde(default = "default_lookback_hours")]
    pub lookback_hours: u32,
    #[serde(default)]
    pub skill_allowlist: Vec<String>,
    #[serde(default)]
    pub autonomy: crate::domain::Autonomy,
    /// Agent CLIs to run the analysis on. Each runs independently with its own
    /// default model, so you get a separate set of suggestions per provider.
    /// Defaults to `["claude"]`.
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    /// When true, the in-loop evolver watches this workspace's live agent
    /// sessions and improves the skills they use right after each interaction.
    #[serde(default)]
    pub live_evolve: bool,
    #[serde(default)]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub next_run_at: Option<DateTime<Utc>>,
}

fn default_cadence_minutes() -> u32 {
    60
}
fn default_lookback_hours() -> u32 {
    24
}
fn default_providers() -> Vec<String> {
    vec!["claude".to_string()]
}

impl Default for SelfImprovementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cadence_minutes: default_cadence_minutes(),
            lookback_hours: default_lookback_hours(),
            skill_allowlist: Vec::new(),
            autonomy: crate::domain::Autonomy::default(),
            providers: default_providers(),
            live_evolve: false,
            last_run_at: None,
            next_run_at: None,
        }
    }
}

/// `PUT /workspaces/{id}/self-improvement` — user-editable fields only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSelfImprovementReq {
    pub enabled: bool,
    pub cadence_minutes: u32,
    pub lookback_hours: u32,
    pub skill_allowlist: Vec<String>,
    pub autonomy: crate::domain::Autonomy,
    #[serde(default = "default_providers")]
    pub providers: Vec<String>,
    #[serde(default)]
    pub live_evolve: bool,
}

/// `POST /workspaces/{id}/self-improvement/run`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunNowResp {
    pub run_id: Id,
}

// ---------------------------------------------------------------------------
// Context provisioning (skills + souls + context library, per-workspace config)
// ---------------------------------------------------------------------------

/// A skill entry in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySkill {
    pub name: String,
    pub category: String,
    pub version: u32,
    pub description: String,
    pub body: String,
}

/// A soul (persona) entry in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySoul {
    pub name: String,
    pub body: String,
}

/// A reusable context snippet in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryContext {
    pub name: String,
    pub body: String,
}

/// `PUT /library/skills/{name}` (and souls/context — only `body` is sent;
/// `name` comes from the path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLibraryEntryReq {
    pub body: String,
}

/// `PUT /library/default-soul`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSoulReq {
    pub name: String,
}

/// `GET /library/default-soul`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSoulResp {
    pub name: Option<String>,
}

/// Per-workspace context config, stored under `Workspace.settings.context`.
/// `skills = None` ⇒ all library skills active; `soul = None` ⇒ global default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceContextConfig {
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub extra_context_md: String,
    #[serde(default = "default_include_memory")]
    pub include_memory: bool,
}

fn default_include_memory() -> bool {
    true
}

impl Default for WorkspaceContextConfig {
    fn default() -> Self {
        Self {
            skills: None,
            soul: None,
            extra_context_md: String::new(),
            include_memory: true,
        }
    }
}

/// `PUT /workspaces/{id}/context`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateWorkspaceContextReq {
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub extra_context_md: String,
    #[serde(default = "default_include_memory")]
    pub include_memory: bool,
}

/// One provider's result from a materialize action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeProviderResult {
    pub provider: String,
    pub files_written: Vec<String>,
    pub skipped: bool,
}

/// `POST /workspaces/{id}/context/materialize`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeResp {
    pub provider_results: Vec<MaterializeProviderResult>,
}

/// How binding a planned artifact is on the agent.
///
/// - `Advisory` — instruction files (`AGENTS.md`/`CLAUDE.md`) and skills:
///   guidance the model reads and *may ignore*.
/// - `Enforced` — hooks / runtime settings the daemon imposes regardless of
///   what the model decides (e.g. activity-forwarding hooks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextEnforcement {
    Advisory,
    Enforced,
}

/// A single artifact `materialize` would write, described without writing it.
/// Used by the dry-run context preview to show exactly what a spawn produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPlanFile {
    /// Absolute destination path the file would be written to.
    pub path: String,
    /// What this file is: `instructions` (AGENTS.md/CLAUDE.md), `skill`,
    /// `skill_asset` (a non-SKILL.md file in a multi-file skill dir), `hooks`,
    /// or `manifest` (Otto's managed-skill manifest).
    pub kind: String,
    /// Advisory (model may ignore) vs enforced (runtime imposes).
    pub enforcement: ContextEnforcement,
    /// Size in bytes of the content that would be written.
    pub size: u64,
    /// First lines of the content (a short excerpt for the preview list).
    pub first_lines: String,
    /// Whether the full content was elided from `first_lines` (file is larger).
    pub truncated: bool,
}

/// A skill selected for a workspace, summarized for the preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPlanSkill {
    pub name: String,
    pub description: String,
    pub version: u32,
}

/// `POST /workspaces/{id}/context/preview` — exactly what a session spawn would
/// materialize for one provider, computed WITHOUT spawning or writing anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPreviewProvider {
    pub provider: String,
    /// True for providers that materialize nothing (shell/agy/…).
    pub skipped: bool,
    /// The skills that would be activated, resolved from the library.
    pub skills: Vec<ContextPlanSkill>,
    /// The soul (persona) name that would apply, if any.
    pub soul: Option<String>,
    /// Every file the spawn would write (paths, sizes, excerpts, enforcement).
    pub files: Vec<ContextPlanFile>,
    /// The generated instruction-file content (the bytes of the Otto region as
    /// merged into AGENTS.md/CLAUDE.md), for an exact preview of what the model
    /// will read. Empty when the provider writes no instruction file.
    pub generated_instructions: String,
    /// Name of the instruction file (`CLAUDE.md` or `AGENTS.md`), if any.
    pub instructions_file_name: Option<String>,
    /// The hooks/settings JSON the runtime would impose (enforced), if any.
    pub generated_hooks: Option<String>,
}

/// `POST /workspaces/{id}/context/preview`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPreviewResp {
    pub providers: Vec<ContextPreviewProvider>,
}

/// `POST /workspaces/{id}/context/preview` body. All fields optional: when
/// present they override the workspace's stored context selection so the UI can
/// preview a not-yet-saved choice (the same inputs a session spawn would use).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextPreviewReq {
    /// Provider(s) to preview; `None` ⇒ both `claude` and `codex`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Override the active skill allow-list. Double-`Option` so an absent key
    /// (`None`) inherits the stored config, while an explicit `null`
    /// (`Some(None)`) clears the allow-list to mean *all* library skills.
    #[serde(default, deserialize_with = "de_double_option")]
    pub skills: Option<Option<Vec<String>>>,
    /// Override the active soul. Double-`Option` so an absent key (`None`)
    /// inherits the stored config, while an explicit `null` (`Some(None)`)
    /// clears the soul to mean the global default.
    #[serde(default, deserialize_with = "de_double_option")]
    pub soul: Option<Option<String>>,
    /// Override the extra-context markdown (`None` ⇒ use stored config).
    #[serde(default)]
    pub extra_context_md: Option<String>,
    /// Override the include-memory toggle (`None` ⇒ use stored config).
    #[serde(default)]
    pub include_memory: Option<bool>,
    /// Working directory the spawn would use (`None` ⇒ the workspace root). The
    /// New Session sheet lets the user pick a cwd other than the workspace root;
    /// passing it here makes the preview match what that spawn would write.
    #[serde(default)]
    pub cwd: Option<String>,
}

// ---------------------------------------------------------------------------
// API client ("Postman" section). Collection/request/environment routes are
// workspace-scoped (`/workspaces/{wid}/api-client/...`), so `workspace_id`
// comes from the path, not these bodies.
// ---------------------------------------------------------------------------

fn default_body_mode() -> String {
    "none".to_string()
}

/// `POST/PATCH /workspaces/{wid}/api-client/collections[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiCollectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// `POST/PATCH /workspaces/{wid}/api-client/requests[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiRequestReq {
    #[serde(default)]
    pub collection_id: Option<Id>,
    pub name: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub query: Value,
    #[serde(default = "default_body_mode")]
    pub body_mode: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub auth: Value,
    /// Optional `ssh`-kind connection id to tunnel this request through.
    #[serde(default)]
    pub ssh_connection_id: Option<Id>,
}

/// `POST/PATCH /workspaces/{wid}/api-client/environments[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiEnvironmentReq {
    pub name: String,
    #[serde(default)]
    pub variables: Value,
}

/// `POST /workspaces/{wid}/api-client/execute` — run a request through the
/// daemon. `{{var}}` placeholders are substituted from `environment_id` (or the
/// workspace's active environment when absent). The run is recorded in history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteApiReq {
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub query: Value,
    #[serde(default = "default_body_mode")]
    pub body_mode: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub auth: Value,
    #[serde(default)]
    pub environment_id: Option<Id>,
    /// Per-request execution settings (timeout, redirects, TLS verification).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub verify_ssl: Option<bool>,
    #[serde(default)]
    pub vars: Option<Value>,
    /// Optional `ssh`-kind connection id: route this request through a SOCKS5
    /// tunnel over that SSH bastion (for IP-whitelisted upstreams). None =
    /// send directly.
    #[serde(default)]
    pub ssh_connection_id: Option<Id>,
}

/// Response of `POST .../api-client/execute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse {
    pub status: u16,
    pub status_text: String,
    /// `[{ "key", "value" }]`
    pub headers: Value,
    /// UTF-8 (lossy) body for display, truncated to a display cap when large.
    pub body: String,
    /// Full response bytes, base64-encoded — used for binary preview (images)
    /// and "save to disk". Empty when the response is `too_large`.
    #[serde(default)]
    pub body_base64: String,
    /// `body` was cut to the display cap (full bytes still in `body_base64`).
    #[serde(default)]
    pub truncated: bool,
    /// Body exceeded the inline cap: neither `body` nor `body_base64` is set.
    #[serde(default)]
    pub too_large: bool,
    pub duration_ms: i64,
    pub size_bytes: i64,
    pub content_type: Option<String>,
    /// Per-phase trace of the request lifecycle (resolved request, TTFB,
    /// download, redirects, completion) for the response "Trace" tab.
    #[serde(default)]
    pub trace: Vec<TraceStep>,
}

/// One step in a request's execution trace (see [`ApiResponse::trace`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceStep {
    pub label: String,
    pub detail: String,
    /// Duration of this phase in milliseconds, when measured.
    #[serde(default)]
    pub ms: Option<i64>,
    /// One of: info | timing | redirect | success | error (UI styling hint).
    #[serde(default)]
    pub level: String,
}

/// `POST /api-client/import-curl` — parse a curl command into request fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCurlReq {
    pub curl: String,
}

/// Parsed request fields from a curl command (drop into a request form).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedCurl {
    pub method: String,
    pub url: String,
    pub headers: Value,
    pub query: Value,
    pub body_mode: String,
    pub body: String,
    pub auth: Value,
}

/// `POST/PATCH /workspaces/{wid}/api-client/automations[/{id}]`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertApiAutomationReq {
    pub name: String,
    #[serde(default)]
    pub steps: Value,
}

/// One step's result in an automation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRunStepResult {
    pub request_id: Id,
    pub name: String,
    pub status: Option<u16>,
    pub duration_ms: i64,
    /// True when the request succeeded AND every assertion passed.
    pub ok: bool,
    /// `[{ "desc", "passed" }]`
    pub assertions: Value,
    pub error: Option<String>,
}

/// `POST /workspaces/{wid}/api-client/automations/{id}/run` → run report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiRunResult {
    pub automation_id: Id,
    pub steps: Vec<ApiRunStepResult>,
    /// True when every step was `ok`.
    pub passed: bool,
}

// ---------------------------------------------------------------------------
// Skills Evaluator
// ---------------------------------------------------------------------------

/// One validation the evaluator runs against the produced implementation. Each
/// validation fans out to one agent per entry in `providers` (so a single
/// validation can be cross-checked by several CLIs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEvalValidationCfg {
    /// Short identifier, e.g. "logs", "docs", "naming".
    pub name: String,
    /// What this validation checks and how to judge it. Passed to the agent.
    pub criteria: String,
    /// CLIs to run this validation on (one agent each). Empty falls back to the
    /// run's implementation CLI.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Model hint ("haiku" | "sonnet" | "opus" | ""). Empty = provider default.
    #[serde(default)]
    pub model: String,
}

/// Config for the agent that edits/improves the skill between iterations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEvalImproverCfg {
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

/// Persisted defaults for the Skills Evaluator (settings key `skill_eval`).
/// `GET /api/v1/settings/skill-eval` and `PUT /api/v1/settings/skill-eval`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEvalConfig {
    /// Default validations offered when starting a new evaluation.
    pub validations: Vec<SkillEvalValidationCfg>,
    /// Default improver agent.
    pub improver: SkillEvalImproverCfg,
    /// Default number of iterations.
    #[serde(default = "default_iterations")]
    pub iterations: u32,
    /// Default validation passes (averaged) — see `StartSkillEvalReq`.
    #[serde(default = "default_validator_passes")]
    pub validator_passes: u32,
}

fn default_iterations() -> u32 {
    2
}

/// Where the skill under test comes from.
/// - `kind = "library"`: `reference` is the Otto library skill name.
/// - `kind = "path"`: `reference` is an absolute path to a skill folder, a
///   `SKILL.md`/`.md` file, or a `.zip`/`.gz`/`.tgz` archive containing one.
/// - `kind = "provider"`: `reference` is a skill name under
///   `~/.<provider>/skills/<name>/SKILL.md` (provider in `provider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceReq {
    pub kind: String,
    pub reference: String,
    #[serde(default)]
    pub provider: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/skill-evaluations` — start an evaluation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartSkillEvalReq {
    pub source: SkillSourceReq,
    /// The task to implement using the skill (e.g. "add endpoint X").
    pub task: String,
    /// The single CLI that implements the task.
    pub impl_cli: String,
    /// Validations to run after each implementation.
    pub validations: Vec<SkillEvalValidationCfg>,
    /// Total iterations (rounds). >= 1. Round 1 is the baseline; each later
    /// round improves the skill and re-runs.
    pub iterations: u32,
    /// Agent that edits the skill between iterations (defaults to the impl CLI).
    #[serde(default)]
    pub improver: Option<SkillEvalImproverCfg>,
    /// Git ref to create each iteration's worktree from (defaults to HEAD).
    #[serde(default)]
    pub base_ref: Option<String>,
    /// How many times to run each validation and average — higher reduces the
    /// noise from nondeterministic graders. 1–3, defaults to 1.
    #[serde(default = "default_validator_passes")]
    pub validator_passes: u32,
}

fn default_validator_passes() -> u32 {
    1
}

/// `POST /api/v1/skill-evaluations/{id}/promote` — save an iteration's skill
/// back into the Otto library under `name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteSkillReq {
    pub iteration_id: Id,
    /// "tested" = the skill that iteration ran with; "improved" = the edited
    /// version it produced for the next round.
    pub source: String,
    /// Target library skill name (safe segment).
    pub name: String,
}

/// `GET /api/v1/skill-evaluations/{id}/iterations/{iter_id}/diff` — the code the
/// implementation agent produced in that iteration's worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplDiffResp {
    pub diff: String,
    pub truncated: bool,
}

/// A discoverable skill source the UI can offer in the start form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceInfo {
    /// "library" | "provider".
    pub kind: String,
    /// Skill name.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Set for `kind = "provider"` (claude/codex/agy).
    #[serde(default)]
    pub provider: Option<String>,
}

/// `GET /api/v1/workspaces/{id}/skill-sources` — skills the user can pick from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourcesResp {
    pub sources: Vec<SkillSourceInfo>,
}

// ---------------------------------------------------------------------------
// Agent activity (live trail + task tracker)
// ---------------------------------------------------------------------------

/// `POST /workspaces/{wid}/sessions/{sid}/trail` — append one trail entry.
/// `source`/`kind` are lowercase strings (default `user`/`note`).
#[derive(Debug, Clone, Deserialize)]
pub struct AppendTrailReq {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    /// `info | warn | error` (default `info`).
    #[serde(default)]
    pub level: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub detail: Option<Value>,
}

/// One task in a [`PutTasksReq`].
#[derive(Debug, Clone, Deserialize)]
pub struct TaskInput {
    #[serde(default)]
    pub ext_id: Option<String>,
    pub title: String,
    /// `pending | in_progress | completed | blocked | cancelled`.
    pub status: String,
}

/// `PUT /workspaces/{wid}/sessions/{sid}/tasks` — replace the whole task list
/// (the task tracker is provider-synced; each push is the source of truth).
#[derive(Debug, Clone, Deserialize)]
pub struct PutTasksReq {
    pub tasks: Vec<TaskInput>,
}

/// One product-analysis lens offered in the Analysis tab. A curated subset of
/// the bundled product skills — only those that emit the Findings contract
/// (generative skills like `jira-story-writer`/`rfc-writer` are excluded).
/// Returned by `GET /workspaces/{id}/product/lenses`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductLens {
    /// Library skill name (e.g. `"po-story-overview"`).
    pub skill: String,
    /// Human label shown in the UI checkbox row.
    pub label: String,
    /// One-line description of what the lens does.
    pub description: String,
    /// Whether the lens is checked by default in the Analysis tab.
    pub default_on: bool,
}

// ---------------------------------------------------------------------------
// Proof Packs
// ---------------------------------------------------------------------------

/// A proof pack with its derived badge list (snake_case strings) and artifact
/// count — the list/summary row shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofPackResp {
    #[serde(flatten)]
    pub pack: crate::proof::ProofPack,
    /// Derived badges as snake_case strings (see `ProofBadge`).
    pub badges: Vec<String>,
    pub artifact_count: u32,
}

/// One artifact plus a capped inline preview for list/detail rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofArtifactView {
    #[serde(flatten)]
    pub artifact: crate::proof::ProofArtifact,
    /// Capped preview (`PREVIEW_CAP`) of inline content; absent for url/file refs.
    pub preview: Option<String>,
    pub truncated: bool,
}

/// `GET /proof-packs/{id}` — the pack, its badges, artifacts, and any children.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofPackDetailResp {
    pub pack: crate::proof::ProofPack,
    pub badges: Vec<String>,
    pub artifacts: Vec<ProofArtifactView>,
    pub children: Vec<ProofPackResp>,
}

/// `POST /workspaces/{id}/proof-packs`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProofPackReq {
    /// `session | goal_loop | review | workflow_run | task | manual`.
    pub work_item_kind: String,
    pub work_item_id: String,
    pub title: Option<String>,
    pub parent_pack_id: Option<String>,
}

/// `POST /proof-packs/{id}/artifacts`. Provide at most one of `content`/`content_url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddArtifactReq {
    /// One of the `ProofArtifactKind` snake_case strings.
    pub kind: String,
    pub title: String,
    /// Inline text content (redacted + capped on store).
    pub content: Option<String>,
    /// An external URL (CI build, screenshot) — stored as `ref_kind=url`.
    pub content_url: Option<String>,
    /// One of the `ProofArtifactStatus` strings; defaults to `info`.
    pub status: Option<String>,
    pub metadata: Option<Value>,
}

/// One command to run during `/assemble`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembleCmd {
    pub cmd: String,
    /// `test | build | lint`; inferred if omitted.
    pub kind: Option<String>,
}

/// `POST /proof-packs/{id}/assemble` — re-run auto-assembly then recompute.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembleReq {
    /// Working directory to assemble a diff from (and run commands in).
    pub cwd: Option<String>,
    /// Optional base ref for the diff (defaults to the working tree vs HEAD).
    pub base: Option<String>,
    /// Optional commands to run and capture as `command` artifacts.
    pub commands: Option<Vec<AssembleCmd>>,
}

/// `POST /proof-packs/{id}/waive`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaiveReq {
    pub reason: String,
}

/// One row of `GET /workspaces/{id}/proof-summary` — cheap badge lookup keyed by
/// work item (the UI maps `"<kind>:<work_item_id>"` -> this).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSummaryRow {
    pub work_item_kind: String,
    pub work_item_id: String,
    pub proof_pack_id: String,
    pub status: String,
    pub risk_score: u8,
    pub badges: Vec<String>,
}

/// `GET /workspaces/{id}/proof-summary`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofSummaryResp {
    pub rows: Vec<ProofSummaryRow>,
}
