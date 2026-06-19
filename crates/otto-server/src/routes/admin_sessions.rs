//! Admin active-sessions overview + terminate — RBAC Task 4.2.
//!
//! ## Endpoints
//! - `GET  /api/v1/admin/sessions`                  — daemon-wide list across
//!   **all** workspaces and **all** users (Users:Admin / root).
//! - `POST /api/v1/admin/sessions/{id}/terminate`   — kill a session's PTY and
//!   forcibly evict its attached `/ws/term` viewers (Users:Admin / root).
//!
//! ## The sanctioned cross-user view
//! This is the **one** place where one user's sessions are intentionally visible
//! to another (a granted admin). Everywhere else, per-session ownership confines
//! a user to their own data (Phase 3, `session_owner_or_admin`). Here that gate
//! is deliberately bypassed: the route is governed by `Users:Admin` instead, so
//! **root and any non-root user holding `Users:Admin`** can use it. We rely on the
//! central feature guard (`policy.rs` maps `/admin/sessions*` → `Require(Users,
//! Admin)`) for that — the handlers add **no** extra `require_root`, by design.
//!
//! ## Live-state enrichment
//! `SessionsRepo::list_all()` gives the persisted rows across every workspace;
//! each is enriched with the in-memory truth from the `SessionManager`
//! (`is_live`, `attached_count`) and the owner's username (resolved once via a
//! single batched user load, not per-row).
//!
//! ## Terminate
//! `terminate` kills the PTY and marks the session `exited`
//! ([`SessionManager::kill_session`]) — the row and its history are **kept**
//! (non-destructive; the overview can still show it). It then fires
//! [`SessionManager::evict`] so every attached `/ws/term` viewer receives a
//! `{"type":"terminated"}` frame and the socket closes (the forced-eviction
//! signal added in Task 4.1). A `"session.terminated"` audit entry records the
//! actor, the session owner, and the session id.
//!
//! ## Generics
//! Handlers are generic over `AdminSessionsCtx` so they run against both the
//! production `ServerCtx` and a minimal test state, mirroring the `GrantsCtx` /
//! `HasGrants` pattern used elsewhere in the RBAC work.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{AdminSessionRow, AdminSessionsResp};
use otto_core::Id;
use otto_sessions::SessionManager;
use otto_state::{NewAuditEntry, SessionsRepo, UsersRepo};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// AdminSessionsCtx trait — lets the handlers run against a minimal test state
// without assembling the full ServerCtx (mirrors GrantsCtx / HasGrants).
// ---------------------------------------------------------------------------

/// State the admin-sessions handlers work against. Implemented for the
/// production [`ServerCtx`] and for a minimal test context.
pub trait AdminSessionsCtx: Clone + Send + Sync + 'static {
    fn sessions_repo(&self) -> SessionsRepo;
    fn users_repo(&self) -> UsersRepo;
    fn manager(&self) -> Arc<SessionManager>;
    /// Write a best-effort audit entry (failure logged, not propagated).
    fn audit_entry(&self, entry: NewAuditEntry) -> impl std::future::Future<Output = ()> + Send;
}

impl AdminSessionsCtx for ServerCtx {
    fn sessions_repo(&self) -> SessionsRepo {
        SessionsRepo::new(self.pool.clone())
    }
    fn users_repo(&self) -> UsersRepo {
        UsersRepo::new(self.pool.clone())
    }
    fn manager(&self) -> Arc<SessionManager> {
        self.manager.clone()
    }
    async fn audit_entry(&self, entry: NewAuditEntry) {
        self.audit(entry).await;
    }
}

// ---------------------------------------------------------------------------
// Handlers (generic over AdminSessionsCtx)
// ---------------------------------------------------------------------------

/// `GET /api/v1/admin/sessions`
///
/// The daemon-wide active-sessions overview across all workspaces and users.
/// Requires `Users:Admin` (enforced by the feature guard) or root.
pub async fn list_sessions<C: AdminSessionsCtx>(
    State(ctx): State<C>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<AdminSessionsResp>> {
    let sessions = ctx.sessions_repo().list_all().await?;
    let manager = ctx.manager();

    // Batch-load every user once and index by id → username (so we never do a
    // per-row DB round-trip resolving `created_by`).
    let username_by_id: HashMap<String, String> = ctx
        .users_repo()
        .list()
        .await?
        .into_iter()
        .map(|u| (u.id, u.username))
        .collect();

    let rows: Vec<AdminSessionRow> = sessions
        .into_iter()
        .map(|s| {
            let owner_username = username_by_id
                .get(&s.created_by)
                .cloned()
                // Fall back to the owner id if the user row is gone (deleted).
                .unwrap_or_else(|| s.created_by.clone());
            AdminSessionRow {
                live: manager.is_live(&s.id),
                viewers: manager.attached_count(&s.id) as u32,
                id: s.id,
                owner_id: s.created_by,
                owner_username,
                workspace_id: s.workspace_id,
                kind: s.kind.as_str().to_string(),
                provider: s.provider,
                title: s.title,
                status: s.status.as_str().to_string(),
            }
        })
        .collect();

    Ok(Json(AdminSessionsResp { sessions: rows }))
}

/// `POST /api/v1/admin/sessions/{id}/terminate`
///
/// Resolve the session in any workspace, kill its PTY + mark it exited, then
/// forcibly evict any attached `/ws/term` viewers. Writes a `session.terminated`
/// audit entry. Requires `Users:Admin` (feature guard) or root. Returns `204`.
pub async fn terminate<C: AdminSessionsCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let manager = ctx.manager();

    // Resolve the session (across workspaces) — 404 if it doesn't exist. We
    // capture the owner before killing for the audit trail.
    let session = manager.get(&id).await?;
    let owner_id = session.created_by.clone();

    // Kill the PTY and mark the session exited (keeps the row + history; this
    // is non-destructive — the overview can still show the terminated session).
    manager.kill_session(&id).await?;

    // Forcibly drop attached /ws/term viewers (the Task 4.1 eviction signal):
    // every viewer that subscribed via `evict_signal` receives the unit, sends
    // `{"type":"terminated"}` and closes. A no-op when no one is attached.
    manager.evict(&id);

    // Audit: who terminated, the session owner, the session id.
    ctx.audit_entry(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "session.terminated".into(),
        target: Some(id.clone()),
        detail: Some(serde_json::json!({
            "owner_id": owner_id,
            "workspace_id": session.workspace_id,
        })),
        ip: None,
    })
    .await;

    Ok(StatusCode::NO_CONTENT)
}
