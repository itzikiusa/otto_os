//! Meta / onboarding / auth; Grants / capabilities (RBAC Task 2.1); Admin active-sessions overview (RBAC Task 4.2); Impersonation (RBAC Task 5.2); Users / workspaces.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/auth.ts / core.ts`.
use super::*;

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
    /// Per-provider: whether the CLI accepts a model flag (its spec carries a
    /// `model_args` template). Drives whether pickers show a model control —
    /// a provider without a template hides it (never a silent drop).
    #[serde(default)]
    pub model_flags: std::collections::HashMap<String, bool>,
    /// A second LOOPBACK base for the same daemon (`http://localhost:<port>`
    /// — `ottod` holds both `127.0.0.1` and `[::1]` on the port), or `null`
    /// when it could not bind `[::1]` (IPv6 off / port taken). Browsers pool
    /// HTTP/1.1 sockets per host, so the UI sends background polls and
    /// known-slow calls here and keeps `127.0.0.1` free for interactive ones.
    /// Only ever a loopback address the daemon itself holds.
    #[serde(default)]
    pub alt_loopback_base: Option<String>,
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
    /// Durable originating session, set only by Otto's session token minter.
    #[serde(default)]
    pub session_id: Option<Id>,
    /// Legacy label matches a session ID; candidate only, never ownership proof.
    #[serde(default)]
    pub legacy_session_id: Option<Id>,
    #[serde(default)]
    pub session_exists: Option<bool>,
}

/// Response for `POST /api/v1/auth/tokens`: the raw secret is returned exactly
/// once (it is only ever stored hashed) alongside the new token's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateApiTokenResp {
    pub token: String,
    pub info: ApiTokenInfo,
}

/// Metadata for one outward **MCP** token (`kind='mcp'`) — an access token an
/// external MCP client uses to reach Otto's `otto.*` tools over the HTTP
/// transport (or the legacy stdio bridge). NEVER carries the secret (only its
/// prefix); the raw token is returned exactly once at mint time. Unlike
/// [`ApiTokenInfo`] it also surfaces the owning user and the per-token
/// [`McpScope`] so an admin can see, at a glance, who holds which access.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTokenInfo {
    pub id: Id,
    /// The user this token authenticates as (identity + base RBAC).
    pub user_id: Id,
    /// The owning user's username, for display in the management UI.
    pub username: String,
    pub label: Option<String>,
    /// First 12 chars of the raw token, for identification in a list.
    pub token_prefix: String,
    /// The per-token permission scope (tools / read-only / workspace pin).
    pub scope: crate::auth::McpScope,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// `POST /api/v1/mcp/tokens` — mint a scoped MCP token. `user_id` is optional:
/// an admin may mint a token for another user; when omitted it defaults to the
/// caller. `scope` defaults to unrestricted (every globally-enabled tool).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateMcpTokenReq {
    /// Owning user; defaults to the caller when omitted.
    pub user_id: Option<Id>,
    /// Human-friendly name, e.g. "ci-readonly" or "alice-laptop".
    pub label: Option<String>,
    /// The per-token permission scope. Omitted ⇒ unrestricted.
    pub scope: Option<crate::auth::McpScope>,
}

/// Response for `POST /api/v1/mcp/tokens`: the raw secret is returned exactly
/// once (it is only ever stored hashed) alongside the new token's metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMcpTokenResp {
    pub token: String,
    pub info: McpTokenInfo,
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
    /// An email-OTP share whose window has lapsed but which the link holder can
    /// still revive with `POST /share/extend` (until its 7-day absolute
    /// lifetime). Listed so the owner can see — and revoke — it (S8-06).
    #[serde(default)]
    pub dormant: bool,
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
    /// Whether another device can open `url` — false when its origin is
    /// loopback or empty (no Public link domain, no network listener). The UI
    /// shows a "only works on this Mac" warning instead of the phone QR hint.
    #[serde(default)]
    pub reachable_remotely: bool,
    /// Who can open `url` (S20-303): `remote` (a public/routable origin),
    /// `lan` (a private/link-local address — the LAN listener — so only
    /// devices on this Mac's network, behind a self-signed-certificate
    /// warning) or `local` (loopback/empty: this Mac only).
    #[serde(default)]
    pub reach: ShareReach,
}

/// How far a share link's origin reaches (see [`CreateShareResp::reach`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareReach {
    /// Any device on the internet (a public domain / routable address).
    Remote,
    /// Only devices on this Mac's local network (RFC 1918, link-local, ULA,
    /// `.local`).
    Lan,
    /// This Mac only (loopback or no origin).
    #[default]
    Local,
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
    /// Opaque keyset cursor for the next (older) page — pass it back as
    /// `?cursor=`; absent on the last page (backend-performance-6-05).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// `GET /api/v1/admin/sessions` query. By default the overview lists the
/// unarchived sessions (live plus recent exited ones) newest first, one page
/// of `limit` (default 200, max 1000) at a time; `include_archived=true`
/// adds the archived history.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdminSessionsQuery {
    #[serde(default)]
    pub include_archived: bool,
    #[serde(default)]
    pub limit: Option<u32>,
    /// `next_cursor` of the previous page.
    #[serde(default)]
    pub cursor: Option<String>,
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
    /// REQUIRED when a caller changes its OWN password (S8-310): a stolen UI
    /// token must not be able to set a new password without knowing the old
    /// one. Ignored otherwise (root resetting another user's password).
    #[serde(default)]
    pub current_password: Option<String>,
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
    /// Atomic RFC 7396 merge patch. Null members remove keys; `context` is
    /// owned by the versioned context API. Mutually exclusive with `settings`.
    pub settings_patch: Option<Value>,
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
