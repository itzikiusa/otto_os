//! Sessions REST router (contract endpoints #17–#22).
//!
//! The router is generic over a context the server implements; handlers read
//! the authenticated user from `Extension<AuthUser>` (inserted by the
//! server's auth middleware).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::{CreateSessionReq, Problem, UpdateSessionReq};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{Session, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;

use crate::manager::SessionManager;

/// Server-side context required by the sessions routes.
pub trait SessionsCtx: Clone + Send + Sync + 'static {
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
        // Distinct prefix so it can't collide with `/sessions/{id}`.
        .route("/app/kill-sessions", post(kill_all_sessions::<S>))
}

/// POST /app/kill-sessions — terminate every live PTY. Called by the desktop
/// app on quit so no agent processes are left running.
async fn kill_all_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<serde_json::Value>> {
    let n = ctx.manager().shutdown_all().await;
    Ok(Json(serde_json::json!({ "killed": n })))
}

/// #17 GET /workspaces/{id}/sessions — viewer
async fn list_sessions<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<Session>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(ctx.manager().list_by_workspace(&ws_id).await?))
}

/// #18 POST /workspaces/{id}/sessions — editor
async fn create_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<CreateSessionReq>,
) -> ApiResult<Json<Session>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let ws = ctx.workspaces().get(&ws_id).await?;
    let session = ctx.manager().create(&ws, &user.id, req, None).await?;
    Ok(Json(session))
}

/// #19 GET /sessions/{id} — viewer (of the session's workspace)
async fn get_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(session))
}

/// #20 PATCH /sessions/{id} — editor
async fn patch_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateSessionReq>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await?;
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

/// #21 DELETE /sessions/{id} — editor; kills PTY + removes row
async fn delete_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let session = ctx.manager().get(&id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.manager().remove(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// #22 POST /sessions/{id}/restart — editor
async fn restart_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(ctx.manager().restart(&id, None).await?))
}

/// POST /sessions/{id}/archive — editor; kills PTY, keeps row + history
async fn archive_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(ctx.manager().archive(&id).await?))
}

/// POST /sessions/{id}/unarchive — editor
async fn unarchive_session<S: SessionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Session>> {
    let session = ctx.manager().get(&id).await?;
    ctx.roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(ctx.manager().unarchive(&id).await?))
}
