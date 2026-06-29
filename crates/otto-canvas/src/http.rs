//! Canvas Studio router + handlers.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_state::{CanvasRepo, NewScene, SceneUpdate};
use serde::Deserialize;
use std::sync::Arc;

use crate::types::{empty_doc, CreateSceneReq, UpdateSceneReq};

// ---------------------------------------------------------------------------
// Context trait
// ---------------------------------------------------------------------------

/// Host-application context required by the canvas router.
pub trait CanvasCtx: Clone + Send + Sync + 'static {
    fn canvas_repo(&self) -> &CanvasRepo;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

struct ApiErr(Error);

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

// ---------------------------------------------------------------------------
// Path extractors
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WsPath {
    ws: Id,
}

#[derive(Deserialize)]
struct SceneIdPath {
    id: Id,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the canvas router. Paths are relative to the `/api/v1` mount point.
/// The collection routes MUST use the literal `{ws}` placeholder so the server
/// policy rule (`/workspaces/{ws}/canvas/`) matches in `policy_coverage`.
pub fn router<S: CanvasCtx>() -> Router<S> {
    Router::new()
        // Global list — Canvas is a workspace-independent tool, so you see your
        // scenes regardless of the active workspace. (Covered by the `/canvas/`
        // policy prefix; the per-ws route below is kept for creating scenes.)
        .route("/canvas/scenes", get(list_scenes_global::<S>))
        .route(
            "/workspaces/{ws}/canvas/scenes",
            get(list_scenes::<S>).post(create_scene::<S>),
        )
        .route(
            "/canvas/scenes/{id}",
            get(get_scene::<S>)
                .put(update_scene::<S>)
                .delete(delete_scene::<S>),
        )
}

// ---------------------------------------------------------------------------
// Helper: resolve workspace from a scene id, then role-check
// ---------------------------------------------------------------------------

async fn ws_from_scene<S: CanvasCtx>(
    ctx: &S,
    user: &otto_core::domain::User,
    id: &Id,
    role: WorkspaceRole,
) -> ApiResult<otto_state::CanvasScene> {
    let scene = ctx
        .canvas_repo()
        .get(id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("canvas scene {id}")))?;
    ctx.roles().check(user, &scene.workspace_id, role).await?;
    Ok(scene)
}

// ---------------------------------------------------------------------------
// Handlers — collection (workspace-scoped)
// ---------------------------------------------------------------------------

async fn list_scenes<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Viewer).await?;
    let scenes = ctx.canvas_repo().list_for_workspace(&ws).await?;
    Ok(Json(scenes).into_response())
}

/// Global list: the caller's own scenes across every workspace (Canvas is a
/// workspace-independent tool). The `Feature::Canvas` capability is enforced
/// upstream by the policy middleware.
async fn list_scenes_global<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Response> {
    let scenes = ctx.canvas_repo().list_for_user(&user.id).await?;
    Ok(Json(scenes).into_response())
}

async fn create_scene<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<CreateSceneReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    let doc = req.doc.unwrap_or_else(|| empty_doc(&req.title));
    let scene = ctx
        .canvas_repo()
        .create(NewScene {
            workspace_id: ws,
            story_id: req.story_id,
            title: req.title,
            doc_json: doc.to_string(),
            provider: req.provider.unwrap_or_else(|| "claude".into()),
            section: req.section,
            created_by: user.id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(scene)).into_response())
}

// ---------------------------------------------------------------------------
// Handlers — item (flat; workspace resolved from row)
// ---------------------------------------------------------------------------

async fn get_scene<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(SceneIdPath { id }): Path<SceneIdPath>,
) -> ApiResult<Response> {
    let scene = ws_from_scene(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(scene).into_response())
}

async fn update_scene<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(SceneIdPath { id }): Path<SceneIdPath>,
    Json(req): Json<UpdateSceneReq>,
) -> ApiResult<Response> {
    ws_from_scene(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let updated = ctx
        .canvas_repo()
        .update(
            &id,
            SceneUpdate {
                title: req.title,
                doc_json: req.doc.map(|v| v.to_string()),
                thumbnail: req.thumbnail,
                provider: req.provider,
                section: req.section,
                story_id: req.story_id,
            },
        )
        .await?;
    Ok(Json(updated).into_response())
}

async fn delete_scene<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(SceneIdPath { id }): Path<SceneIdPath>,
) -> ApiResult<Response> {
    ws_from_scene(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.canvas_repo().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
