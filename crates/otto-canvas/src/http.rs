//! Canvas Studio router + handlers.

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
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

#[derive(Deserialize)]
struct VersionPath {
    id: Id,
    vid: Id,
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
            get(list_scenes::<S>)
                .merge(post(create_scene::<S>).layer(DefaultBodyLimit::max(SCENE_BODY_LIMIT))),
        )
        .route(
            "/canvas/scenes/{id}",
            get(get_scene::<S>)
                .merge(put(update_scene::<S>).layer(DefaultBodyLimit::max(SCENE_BODY_LIMIT)))
                .delete(delete_scene::<S>),
        )
        // Version history (C5): snapshots taken before agent commits, restores
        // and (throttled) user saves.
        .route("/canvas/scenes/{id}/versions", get(list_versions::<S>))
        .route(
            "/canvas/scenes/{id}/versions/{vid}/restore",
            post(restore_version::<S>),
        )
}

/// Body cap for scene create/update (C1). An Excalidraw board inlines its
/// pasted images as base64 in `doc.source` (`files`), so one screenshot blew
/// axum's 2 MB default and every autosave failed with 413 from then on. Only
/// the two document-carrying routes get the larger cap.
pub const SCENE_BODY_LIMIT: usize = 25 * 1024 * 1024;

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

/// Role-check against a scene's workspace WITHOUT loading its document (PUT /
/// DELETE only need the owner; SD-22).
async fn check_scene_role<S: CanvasCtx>(
    ctx: &S,
    user: &otto_core::domain::User,
    id: &Id,
    role: WorkspaceRole,
) -> ApiResult<()> {
    let ws = ctx
        .canvas_repo()
        .workspace_of(id)
        .await?
        .ok_or_else(|| Error::NotFound(format!("canvas scene {id}")))?;
    ctx.roles().check(user, &ws, role).await?;
    Ok(())
}

#[derive(Deserialize, Default)]
struct UpdateSceneQ {
    /// `?summary=true`: answer with the `CanvasSceneSummary` list row instead
    /// of echoing the full scene (the Canvas editor ignores the echo).
    #[serde(default)]
    summary: bool,
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
    Query(q): Query<UpdateSceneQ>,
    Json(req): Json<UpdateSceneReq>,
) -> ApiResult<Response> {
    check_scene_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if req.doc.is_some() {
        // Keep the pre-save document in the scene's history — at most one
        // user snapshot per window, so a 700 ms autosave stream doesn't churn
        // the 30-entry ring. Best-effort: a failed snapshot never blocks a save.
        if let Err(e) = ctx
            .canvas_repo()
            .snapshot(
                &id,
                "user",
                Some(&user.id),
                Some(otto_state::USER_SNAPSHOT_EVERY_SECS),
            )
            .await
        {
            tracing::warn!(scene = %id, error = %e, "canvas user-save snapshot failed");
        }
    }
    let patch = SceneUpdate {
        title: req.title,
        doc_json: req.doc.map(|v| v.to_string()),
        thumbnail: req.thumbnail,
        provider: req.provider,
        section: req.section,
        story_id: req.story_id,
        ..Default::default()
    };
    if q.summary {
        let row = ctx.canvas_repo().update_summary(&id, patch).await?;
        return Ok(Json(row).into_response());
    }
    let updated = ctx.canvas_repo().update(&id, patch).await?;
    Ok(Json(updated).into_response())
}

async fn delete_scene<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(SceneIdPath { id }): Path<SceneIdPath>,
) -> ApiResult<Response> {
    check_scene_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.canvas_repo().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

// ---------------------------------------------------------------------------
// Handlers — version history (C5)
// ---------------------------------------------------------------------------

async fn list_versions<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(SceneIdPath { id }): Path<SceneIdPath>,
) -> ApiResult<Response> {
    check_scene_role(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let versions = ctx.canvas_repo().list_versions(&id).await?;
    Ok(Json(versions).into_response())
}

/// Restore a version: the current document is snapshotted first (origin
/// `restore`), so a restore is itself undoable. Answers with the full scene.
async fn restore_version<S: CanvasCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(VersionPath { id, vid }): Path<VersionPath>,
) -> ApiResult<Response> {
    check_scene_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let doc = ctx
        .canvas_repo()
        .version_doc(&id, &vid)
        .await?
        .ok_or_else(|| Error::NotFound(format!("canvas scene version {vid}")))?;
    ctx.canvas_repo()
        .snapshot(&id, "restore", Some(&user.id), None)
        .await?;
    let updated = ctx
        .canvas_repo()
        .update(
            &id,
            SceneUpdate {
                doc_json: Some(doc),
                ..Default::default()
            },
        )
        .await?;
    Ok(Json(updated).into_response())
}
