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
//! [`overview_rows`] reads the persisted rows across every workspace — only
//! the columns the overview shows, never `meta_json` (backend-performance-5-04);
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

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::api::{AdminSessionRow, AdminSessionsQuery, AdminSessionsResp};
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
    Query(q): Query<AdminSessionsQuery>,
) -> ApiResult<Json<AdminSessionsResp>> {
    let limit = q.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE) as usize;
    let after = match q.cursor.as_deref().filter(|c| !c.is_empty()) {
        Some(c) => Some(parse_cursor(c)?),
        None => None,
    };
    let mut sessions = overview_rows(
        &ctx.sessions_repo().pool(),
        q.include_archived,
        after,
        limit + 1,
    )
    .await?;
    let next_cursor = (sessions.len() > limit).then(|| {
        sessions.truncate(limit);
        sessions
            .last()
            .map(|r| hex::encode(format!("{}|{}", r.created_at, r.id)))
            .unwrap_or_default()
    });
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
                kind: s.kind,
                provider: s.provider,
                title: s.title,
                status: s.status,
            }
        })
        .collect();

    Ok(Json(AdminSessionsResp {
        sessions: rows,
        next_cursor,
    }))
}

/// Default and maximum overview page size.
const DEFAULT_PAGE: u32 = 200;
const MAX_PAGE: u32 = 1000;

/// A `next_cursor` — hex of `"<created_at>|<id>"`, so it needs no URL
/// escaping (an RFC 3339 `+00:00` would read back as a space) — → its keyset
/// position. Anything else is a 400.
fn parse_cursor(c: &str) -> ApiResult<(String, String)> {
    hex::decode(c)
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| {
            let (at, id) = s.split_once('|')?;
            (!at.is_empty() && !id.is_empty()).then(|| (at.to_string(), id.to_string()))
        })
        .ok_or_else(|| {
            crate::error::ApiError(otto_core::Error::Invalid(
                "malformed sessions cursor".into(),
            ))
        })
}

/// The persisted part of one overview row.
#[derive(sqlx::FromRow)]
struct OverviewRow {
    id: String,
    created_at: String,
    workspace_id: String,
    created_by: String,
    kind: String,
    provider: String,
    title: String,
    status: String,
}

/// One page of sessions, newest first by `(created_at, id)` — keyset-paged
/// after `after` — projected to the overview's columns: no `meta_json`
/// decode per row. Unarchived only unless `include_archived` (the archived
/// history is the bulk of the table — backend-performance-6-05).
async fn overview_rows(
    pool: &otto_state::DbPool,
    include_archived: bool,
    after: Option<(String, String)>,
    limit: usize,
) -> ApiResult<Vec<OverviewRow>> {
    let (at, id) = after.unzip();
    sqlx::query_as::<_, OverviewRow>(
        "SELECT id, created_at, workspace_id, created_by, kind, provider, title, status \
         FROM sessions WHERE (? OR archived = 0) \
         AND (? IS NULL OR created_at < ? OR (created_at = ? AND id < ?)) \
         ORDER BY created_at DESC, id DESC LIMIT ?",
    )
    .bind(include_archived)
    .bind(&at)
    .bind(&at)
    .bind(&at)
    .bind(&id)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!(
            "admin sessions overview: {e}"
        )))
    })
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

/// `POST /api/v1/admin/sessions/{id}/remove`
///
/// Evict attached viewers, kill the PTY, and DELETE the session row (+ history),
/// emitting `SessionRemoved`. Unlike `terminate` (which keeps the row so the
/// overview still shows it), this clears the session from the list entirely —
/// for pruning the exited/background sessions (insights, analysis, …) that would
/// otherwise accumulate without bound. Requires `Users:Admin` (feature guard) or
/// root. Returns `204`.
pub async fn remove<C: AdminSessionsCtx>(
    Path(id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let manager = ctx.manager();
    let session = manager.get(&id).await?;
    let owner_id = session.created_by.clone();
    let workspace_id = session.workspace_id.clone();

    // Drop any attached /ws/term viewers, then delete the row + kill the PTY.
    manager.evict(&id);
    manager.remove(&id).await?;

    ctx.audit_entry(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "session.removed".into(),
        target: Some(id.clone()),
        detail: Some(serde_json::json!({
            "owner_id": owner_id,
            "workspace_id": workspace_id,
        })),
        ip: None,
    })
    .await;

    Ok(StatusCode::NO_CONTENT)
}
