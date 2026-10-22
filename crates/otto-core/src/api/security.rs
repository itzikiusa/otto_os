//! Trust & Safety: audit log + security posture (root only).
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/auth.ts`.
use super::*;

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
/// Center renders, derived from the live listener, settings and auth store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityPostureResp {
    /// Whether the daemon's network (0.0.0.0) listener is actually bound.
    pub network_listener: bool,
    /// The actual bound network port (None = no network listener).
    pub network_listener_port: Option<u16>,
    /// True when no network listener is bound (daemon is loopback-only).
    pub loopback_only: bool,
    /// Saved configuration differs from the running listener; restart to apply.
    #[serde(default)]
    pub network_listener_restart_required: bool,
    /// Count of currently-active (unexpired) API (personal access) tokens.
    pub active_api_tokens: i64,
}
