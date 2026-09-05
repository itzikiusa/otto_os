//! Sessions REST router (contract endpoints #17–#22).
//!
//! The router is generic over a context the server implements; handlers read
//! the authenticated user from `Extension<AuthUser>` (inserted by the
//! server's auth middleware).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::{CreateSessionReq, Problem, UpdateSessionReq};
use otto_core::auth::{session_owner_or_admin, AuthUser, RoleChecker};
use otto_core::domain::{Session, User, WorkspaceRole};
use otto_core::workref::WorkRef;
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;

use crate::manager::SessionManager;

/// Owner-or-admin gate for a single session: `Ok` iff the canonical
/// [`session_owner_or_admin`] helper allows the caller (root, the session's
/// creator, or a workspace Admin of the session's workspace). Returns a
/// `Forbidden` [`ApiErr`] otherwise. This is the chokepoint that stops one user
/// reading or controlling another user's session (#L1–#L7) — the single source
/// of truth lives in `otto_core::auth`, shared with `otto-server`.
async fn ensure_session_owner_or_admin<S: SessionsCtx>(
    ctx: &S,
    user: &User,
    session: &Session,
) -> ApiResult<()> {
    ctx.check_resource(user,session).await?;
    if session_owner_or_admin(ctx.roles().as_ref(), user, session).await {
        Ok(())
    } else {
        Err(ApiErr(Error::Forbidden(
            "not the session owner or a workspace admin".into(),
        )))
    }
}

/// Server-side context required by the sessions routes.
pub trait SessionsCtx: Clone + Send + Sync + 'static {
    /// Resource-bound sessions reauthorize on reads, control, and reconnect.
    fn check_resource<'a>(&'a self, _user: &'a User, _session: &'a Session) -> otto_core::auth::BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }

    fn manager(&self) -> &Arc<SessionManager>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
}

/// Maps `otto_core::Error` to the problem-details response (local newtype
/// because the orphan rule forbids `impl IntoResponse for otto_core::Error`).
pub(crate) struct ApiErr(pub Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: SessionsCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/sessions",
            get(list_sessions::<S>).post(create_session::<S>),
        )
        .route(
            "/sessions/{id}",
            get(get_session::<S>)
                .patch(patch_session::<S>)
                .delete(delete_session::<S>),
        )
        .route("/sessions/{id}/restart", post(restart_session::<S>))
        .route("/sessions/{id}/archive", post(archive_session::<S>))
        .route("/sessions/{id}/unarchive", post(unarchive_session::<S>))
        .route("/sessions/{id}/kill", post(kill_session::<S>))
        // Static segment beats the `{id}` capture in axum's route priority.
        .route("/sessions/bulk", post(bulk_sessions::<S>))
        // Distinct prefix so it can't collide with `/sessions/{id}`.
        .route("/app/kill-sessions", post(kill_all_sessions::<S>))
}

/// A [`Session`] as the list/get endpoints serve it: the domain row plus the
/// manager's TRANSIENT view of it — whether a PTY is live right now and how
/// many `/ws/term` viewers are attached. Serialize-only (never persisted); the
/// UI needs it to say "closing this will kill a running process" truthfully.
#[derive(serde::Serialize)]
struct SessionOut {
    #[serde(flatten)]
    session: Session,
    live: bool,
    viewers: u32,
}

fn with_live<S: SessionsCtx>(ctx: &S, session: Session) -> SessionOut {
    let live = ctx.manager().is_live(&session.id);
    let viewers = ctx.manager().attached_count(&session.id) as u32;
    SessionOut {
        session,
        live,
        viewers,
    }
}

/// Optional filters for `GET /workspaces/{id}/sessions`. All narrowing; absent
/// params keep today's full owner-scoped list.
#[derive(Default, serde::Deserialize)]
struct ListSessionsQuery {
    /// `true` → only archived rows; `false` → only active rows.
    archived: Option<bool>,
    /// "agent" | "connection".
    kind: Option<String>,
    /// Match `meta.source` exactly; the literal `"none"` matches sessions with
    /// NO source (the user-opened foreground ones).
    source: Option<String>,
    /// Match the session status string ("running", "idle", "exited", …).
    status: Option<String>,
}

/// POST /app/kill-sessions — terminate every live PTY. Called by the desktop
/// app on quit so no agent processes are left running.
///
/// Root-only (#L8): any authenticated non-root caller receives 403. The
/// endpoint is still "Exempt" in the policy table (no workspace context) but
/// the handler enforces the root requirement directly.
async fn kill_all_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<serde_json::Value>> {
    if !user.is_root {
        return Err(ApiErr(Error::Forbidden(
            "only root may kill all sessions".into(),
        )));
    }
    let n = ctx.manager().shutdown_all().await;
    Ok(Json(serde_json::json!({ "killed": n })))
}

/// #17 GET /workspaces/{id}/sessions — viewer, owner-scoped.
///
/// Membership (Viewer+) is still required to list a workspace at all. Within it,
/// a non-admin caller sees only **their own** sessions (#L1); root and workspace
/// **Admins** keep the full cross-user list (the sanctioned team/admin view).
async fn list_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(q): Query<ListSessionsQuery>,
) -> ApiResult<Json<Vec<SessionOut>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    // Root or workspace-Admin → full list; otherwise scope to the caller's own.
    let admin = user.is_root
        || ctx
            .roles()
            .check(&user, &ws_id, WorkspaceRole::Admin)
            .await
            .is_ok();
    let sessions = if admin {
        ctx.manager().list_by_workspace(&ws_id).await?
    } else {
        ctx.manager()
            .list_by_workspace_for_user(&ws_id, &user.id)
            .await?
    };
    let mut visible = Vec::new();
    for session in sessions {
        if ctx.check_resource(&user,&session).await.is_ok() { visible.push(session); }
    }
    let sessions = visible
        .into_iter()
        .filter(|s| q.archived.is_none_or(|a| s.archived == a))
        .filter(|s| q.kind.as_deref().is_none_or(|k| kind_str(s) == k))
        .filter(|s| {
            q.source.as_deref().is_none_or(|want| {
                let src = s.meta.get("source").and_then(|v| v.as_str());
                if want == "none" {
                    src.is_none()
                } else {
                    src == Some(want)
                }
            })
        })
        .filter(|s| q.status.as_deref().is_none_or(|st| s.status.as_str() == st))
        .map(|s| with_live(&ctx, s))
        .collect();
    Ok(Json(sessions))
}

fn kind_str(s: &Session) -> &'static str {
    match s.kind {
        otto_core::domain::SessionKind::Agent => "agent",
        otto_core::domain::SessionKind::Connection => "connection",
    }
}

/// #18 POST /workspaces/{id}/sessions — editor
async fn create_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(mut req): Json<CreateSessionReq>,
) -> ApiResult<Json<Session>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    // Stamp a minimal WorkRef into the session meta so every manually-created
    // session carries at least `origin = "manual"` for usage attribution (B1).
    // Runners that create sessions programmatically (review, product, swarm …)
    // overwrite `meta["work"]` with a richer ref; this is the plain-create path
    // fallback. We merge into the existing meta object so any caller-supplied
    // fields are preserved.
    {
        let meta = req.meta.get_or_insert_with(|| serde_json::Value::Object(Default::default()));
        if let serde_json::Value::Object(m) = meta {
            // Only stamp if the caller hasn't already supplied a work ref.
            if !m.contains_key("work") {
                let work_ref = WorkRef {
                    origin: Some("manual".to_string()),
                    ..Default::default()
                };
                if let Ok(v) = serde_json::to_value(&work_ref) {
                    m.insert("work".to_string(), v);
                }
            }
        }
    }

    let ws = ctx.workspaces().get(&ws_id).await?;
    let session = ctx.manager().create(&ws, &user.id, req, None).await?;
    Ok(Json(session))
}

/// #19 GET /sessions/{id} — owner-or-admin.
async fn get_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<SessionOut>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(with_live(&ctx, session)))
}

/// #20 PATCH /sessions/{id} — owner-or-admin
async fn patch_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(mut req): Json<UpdateSessionReq>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    if let Some(meta) = &req.meta {
        // These fields bind a terminal to its originating protected resource.
        // Ordinary metadata edits must not detach or replace that binding.
        if ["k8s","aws","connection_id","source","resource_node"].iter().any(|key| meta.get(*key).is_some() && meta.get(*key) != session.meta.get(*key)) {
            return Err(ApiErr(Error::Forbidden("resource session bindings cannot be edited".into())));
        }
    }
    // The manager replaces nested objects via two writes. Do not pass even
    // unchanged binding objects through that path, which could detach them
    // transiently while a concurrent input request authorizes the session.
    if let Some(object)=req.meta.as_mut().and_then(serde_json::Value::as_object_mut) {
        for key in ["k8s","aws","connection_id","source","resource_node"] {object.remove(key);}
    }
    let session = match req.title {
        Some(title) => ctx.manager().update_title(&id, &title).await?,
        None => session,
    };
    let session = match req.meta {
        Some(m) => ctx.manager().update_meta(&id, m).await?,
        None => session,
    };
    Ok(Json(session))
}

/// #21 DELETE /sessions/{id} — owner-or-admin; kills PTY + removes row
async fn delete_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    ctx.manager().remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// #22 POST /sessions/{id}/restart — owner-or-admin
async fn restart_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(ctx.manager().restart(&id, None).await?))
}

/// POST /sessions/{id}/archive — owner-or-admin; kills PTY, keeps row + history
async fn archive_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(ctx.manager().archive(&id).await?))
}

/// POST /sessions/{id}/unarchive — owner-or-admin
async fn unarchive_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    Ok(Json(ctx.manager().unarchive(&id).await?))
}

/// POST /sessions/{id}/kill — owner-or-admin; kills the PTY but KEEPS the row
/// un-archived ("stop the process, leave it in my list"). The session stays
/// resumable/reopenable exactly like an idle-suspended one. Previously the
/// only kill-but-keep path was the admin terminate route.
async fn kill_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ensure_session_owner_or_admin(&ctx, &user, &session).await?;
    ctx.manager().kill_session(&id).await?;
    Ok(Json(ctx.manager().get(&id).await?))
}

/// Body for `POST /sessions/bulk`.
#[derive(serde::Deserialize)]
struct BulkSessionsReq {
    /// "archive" | "delete" | "kill".
    action: String,
    ids: Vec<Id>,
}

/// Per-id outcome of a bulk action.
#[derive(serde::Serialize)]
struct BulkSessionResult {
    id: Id,
    ok: bool,
    /// Set when `ok` is false: "forbidden", "not found", or the error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// POST /sessions/bulk — owner-scoped bulk archive/delete/kill. Non-owned ids
/// are SKIPPED (reported `ok:false, error:"forbidden"`) rather than failing
/// the whole batch, so "clean up my exited sessions" degrades gracefully for
/// non-admins. Capped to keep one request from tying up the manager.
async fn bulk_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<BulkSessionsReq>,
) -> ApiResult<Json<Vec<BulkSessionResult>>> {
    const BULK_CAP: usize = 200;
    if !matches!(req.action.as_str(), "archive" | "delete" | "kill") {
        return Err(ApiErr(Error::Invalid(
            "action must be one of: archive, delete, kill".into(),
        )));
    }
    if req.ids.len() > BULK_CAP {
        return Err(ApiErr(Error::Invalid(format!(
            "at most {BULK_CAP} ids per bulk request"
        ))));
    }
    // `len()` is already rejected above BULK_CAP; the `min` keeps the bound
    // visible at the allocation itself.
    let mut out = Vec::with_capacity(req.ids.len().min(BULK_CAP));
    for id in req.ids {
        let outcome = async {
            let session = ctx.manager().get(&id).await?;
            ctx.check_resource(&user,&session).await?;
            if !session_owner_or_admin(ctx.roles().as_ref(), &user, &session).await {
                return Err(Error::Forbidden("forbidden".into()));
            }
            match req.action.as_str() {
                "archive" => ctx.manager().archive(&id).await.map(|_| ()),
                "delete" => ctx.manager().remove(&id).await,
                _ => ctx.manager().kill_session(&id).await,
            }
        }
        .await;
        out.push(match outcome {
            Ok(()) => BulkSessionResult {
                id,
                ok: true,
                error: None,
            },
            Err(e) => BulkSessionResult {
                id,
                ok: false,
                error: Some(e.to_string()),
            },
        });
    }
    Ok(Json(out))
}
