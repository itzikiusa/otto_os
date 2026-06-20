//! Connections REST router (contract endpoints #25–#30).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::{
    MoveSectionReq, Problem, ReorderSectionsReq, SectionScopeQuery, TestConnectionResp,
    UpsertConnectionReq, UpsertSectionReq,
};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{Connection, ConnectionSection, Session, User, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::SqlitePool;
use serde::Deserialize;

use crate::service::{ConnectionsService, Spawner};

/// Server-side context required by the connections routes.
pub trait ConnectionsCtx: Clone + Send + Sync + 'static {
    fn connections(&self) -> &Arc<ConnectionsService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn spawner(&self) -> &Arc<dyn Spawner>;
    /// SQLite pool used to read daemon settings (e.g. `connections.owner_private`).
    fn pool(&self) -> SqlitePool;
}

/// Returns `true` when the `connections.owner_private` setting is explicitly
/// set to `true`. Absent or non-boolean ⇒ `false` (default-off).
pub async fn owner_private_enabled<S: ConnectionsCtx>(ctx: &S) -> bool {
    otto_state::SettingsRepo::new(ctx.pool())
        .get("connections.owner_private")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// When `connections.owner_private` is ON, require the caller to be root or
/// the connection's creator, else 403.
pub fn require_conn_owner_or_root(user: &User, conn: &Connection) -> Result<(), Error> {
    if user.is_root || conn.created_by == user.id {
        Ok(())
    } else {
        Err(Error::Forbidden(
            "connection belongs to another user".into(),
        ))
    }
}

/// Local problem-details mapper (orphan rule: cannot impl IntoResponse for
/// `otto_core::Error` here).
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

/// Body of `POST /connections/{id}/open`. `workspace_id` is only needed for
/// global connections (which have no workspace of their own).
#[derive(Debug, Default, Deserialize)]
pub struct OpenConnectionReq {
    pub title: Option<String>,
    pub workspace_id: Option<Id>,
}

/// Body of `PATCH /connections/{id}/pin`.
#[derive(Debug, Deserialize)]
pub struct PinConnectionReq {
    pub pinned: bool,
}

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: ConnectionsCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/connections",
            get(list_connections::<S>).post(create_connection::<S>),
        )
        .route(
            "/connections/{id}",
            axum::routing::patch(update_connection::<S>).delete(delete_connection::<S>),
        )
        .route("/connections/{id}/open", post(open_connection::<S>))
        .route("/connections/{id}/test", post(test_connection::<S>))
        .route("/connections/{id}/pin", patch(pin_connection::<S>))
        .route(
            "/workspaces/{id}/connection-sections",
            get(list_sections::<S>).post(create_section::<S>),
        )
        .route(
            "/workspaces/{id}/connection-sections/reorder",
            post(reorder_sections::<S>),
        )
        .route(
            "/connection-sections/{id}",
            axum::routing::patch(rename_section::<S>).delete(delete_section::<S>),
        )
        .route("/connection-sections/{id}/move", post(move_section::<S>))
}

/// Editor in the connection's workspace; for global connections: root only.
async fn check_conn_role<S: ConnectionsCtx>(
    ctx: &S,
    user: &User,
    conn: &Connection,
    min: WorkspaceRole,
) -> Result<(), Error> {
    match &conn.workspace_id {
        Some(ws) => ctx.roles().check(user, ws, min).await,
        None => {
            if user.is_root {
                Ok(())
            } else {
                Err(Error::Forbidden(
                    "global connections are managed by root".into(),
                ))
            }
        }
    }
}

/// #25 GET /workspaces/{id}/connections — viewer
async fn list_connections<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<Connection>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    // When owner-private is ON, non-root users see only their own connections.
    if owner_private_enabled(&ctx).await && !user.is_root {
        Ok(Json(ctx.connections().list_for(&ws_id, &user.id).await?))
    } else {
        Ok(Json(ctx.connections().list(&ws_id).await?))
    }
}

/// #26 POST /workspaces/{id}/connections — editor
async fn create_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpsertConnectionReq>,
) -> ApiResult<Json<Connection>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    // Connections are a global library — created workspace-independent. The
    // path workspace is used only to authorize the caller.
    let conn = ctx.connections().create(None, &user.id, req).await?;
    Ok(Json(conn))
}

/// #27 PATCH /connections/{id} — ws editor (global: root)
async fn update_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertConnectionReq>,
) -> ApiResult<Json<Connection>> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    Ok(Json(ctx.connections().update(&id, req).await?))
}

/// #28 DELETE /connections/{id} — ws editor (global: root)
async fn delete_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    ctx.connections().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// #29 POST /connections/{id}/open — editor → Session
async fn open_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    body: Option<Json<OpenConnectionReq>>,
) -> ApiResult<Json<Session>> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let conn = ctx.connections().get(&id).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    let ws_id = conn
        .workspace_id
        .clone()
        .or(req.workspace_id)
        .ok_or_else(|| {
            Error::Invalid("opening a global connection requires 'workspace_id'".into())
        })?;
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let session = ctx
        .connections()
        .open(&conn, &ws_id, &user.id, req.title, ctx.spawner().as_ref())
        .await?;
    Ok(Json(session))
}

/// #30 POST /connections/{id}/test — editor
async fn test_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<TestConnectionResp>> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    if owner_private_enabled(&ctx).await {
        require_conn_owner_or_root(&user, &conn)?;
    }
    Ok(Json(ctx.connections().test(&conn).await?))
}

/// PATCH /connections/{id}/pin — editor (global: root); toggle the pinned flag.
async fn pin_connection<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<PinConnectionReq>,
) -> ApiResult<Json<Connection>> {
    let conn = ctx.connections().get(&id).await?;
    check_conn_role(&ctx, &user, &conn, WorkspaceRole::Editor).await?;
    Ok(Json(ctx.connections().set_pinned(&id, req.pinned).await?))
}

// --- Connection sections ----------------------------------------------------

/// Sections live in one of two independent trees. Absent/unknown → the default
/// Connections-page tree; the Database Explorer passes "db".
fn norm_scope(scope: Option<&str>) -> ApiResult<&'static str> {
    match scope.map(str::trim) {
        None | Some("") | Some("connections") => Ok("connections"),
        Some("db") => Ok("db"),
        Some(_) => Err(ApiErr(Error::Invalid("unknown section scope".into()))),
    }
}

/// GET /workspaces/{id}/connection-sections — viewer.
/// Returns the single global section tree (the `scope` query param is accepted
/// for backward compatibility but ignored — both pages share one tree). The
/// path workspace is used only to authorize the caller.
async fn list_sections<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(_q): Query<SectionScopeQuery>,
) -> ApiResult<Json<Vec<ConnectionSection>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(ctx.connections().list_sections().await?))
}

/// POST /workspaces/{id}/connection-sections — editor
async fn create_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpsertSectionReq>,
) -> ApiResult<Json<ConnectionSection>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiErr(Error::Invalid("section name required".into())));
    }
    let scope = norm_scope(req.scope.as_deref())?;
    Ok(Json(
        ctx.connections()
            .create_section(&ws_id, &user.id, req.parent_id.as_deref(), name, scope)
            .await?,
    ))
}

/// PATCH /connection-sections/{id} — editor in the section's workspace
async fn rename_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpsertSectionReq>,
) -> ApiResult<Json<ConnectionSection>> {
    let sec = ctx.connections().get_section(&id).await?;
    ctx.roles()
        .check(&user, &sec.workspace_id, WorkspaceRole::Editor)
        .await?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiErr(Error::Invalid("section name required".into())));
    }
    Ok(Json(ctx.connections().rename_section(&id, name).await?))
}

/// DELETE /connection-sections/{id} — editor; connections fall back to ungrouped
async fn delete_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let sec = ctx.connections().get_section(&id).await?;
    ctx.roles()
        .check(&user, &sec.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.connections().delete_section(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /workspaces/{id}/connection-sections/reorder — editor
async fn reorder_sections<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<ReorderSectionsReq>,
) -> ApiResult<StatusCode> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;
    ctx.connections().reorder_sections(&ws_id, &req.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /connection-sections/{id}/move — editor; reparent (None = top-level)
async fn move_section<S: ConnectionsCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<MoveSectionReq>,
) -> ApiResult<Json<ConnectionSection>> {
    let sec = ctx.connections().get_section(&id).await?;
    ctx.roles()
        .check(&user, &sec.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(Json(
        ctx.connections()
            .reparent_section(&id, req.parent_id.as_deref())
            .await?,
    ))
}
