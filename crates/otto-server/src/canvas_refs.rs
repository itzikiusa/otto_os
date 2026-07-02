//! Session ↔ Canvas scene references — lets a Canvas scene be attached to an
//! agent session so it shows up in that session's Canvas panel. A session may
//! reference many scenes; a scene may be referenced by many sessions.
//!
//! Lives in `otto-server` (not `otto-canvas`) because resolving a session to
//! its workspace needs `SessionManager`, which the canvas crate doesn't know
//! about (mirrors why `canvas_assist` lives here too).
//!
//! Routes (registered in modules.rs, gated by `Feature::Canvas`):
//!   GET    /api/v1/sessions/{sid}/canvas-refs              (ws viewer)  → CanvasSceneSummary[]
//!   POST   /api/v1/sessions/{sid}/canvas-refs               (ws editor) → 204
//!   DELETE /api/v1/sessions/{sid}/canvas-refs/{scene_id}    (ws editor) → 204

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::Id;
use otto_state::CanvasSceneSummary;
use serde::Deserialize;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

#[derive(Debug, Deserialize)]
pub struct AddRefReq {
    pub scene_id: Id,
}

/// `GET /sessions/{sid}/canvas-refs` — the scenes referenced by this session.
async fn list_refs(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<CanvasSceneSummary>>> {
    let session = ctx.manager.get(&sid).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Viewer).await?;
    let refs = ctx.canvas_repo.list_refs_for_session(&sid).await.map_err(ApiError)?;
    Ok(Json(refs))
}

/// `POST /sessions/{sid}/canvas-refs {scene_id}` — reference a scene from this
/// session. The scene must belong to the SAME workspace as the session (a
/// cross-workspace attach is a 404, not a silent success).
async fn add_ref(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(sid): Path<Id>,
    Json(req): Json<AddRefReq>,
) -> ApiResult<StatusCode> {
    let session = ctx.manager.get(&sid).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Editor).await?;

    let scene = ctx
        .canvas_repo
        .get(&req.scene_id)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(otto_core::Error::NotFound(format!("canvas scene {}", req.scene_id))))?;
    if scene.workspace_id != session.workspace_id {
        return Err(ApiError(otto_core::Error::NotFound(
            "canvas scene not in this session's workspace".into(),
        )));
    }

    ctx.canvas_repo
        .add_ref(&req.scene_id, &sid, &session.workspace_id, &user.id)
        .await
        .map_err(ApiError)?;

    let _ = ctx.events.send(Event::CanvasRefsChanged {
        workspace_id: session.workspace_id.clone(),
        session_id: sid,
    });

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /sessions/{sid}/canvas-refs/{scene_id}` — detach a scene from this
/// session (the scene itself is untouched).
async fn remove_ref(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((sid, scene_id)): Path<(Id, Id)>,
) -> ApiResult<StatusCode> {
    let session = ctx.manager.get(&sid).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Editor).await?;

    ctx.canvas_repo.remove_ref(&scene_id, &sid).await.map_err(ApiError)?;

    let _ = ctx.events.send(Event::CanvasRefsChanged {
        workspace_id: session.workspace_id.clone(),
        session_id: sid,
    });

    Ok(StatusCode::NO_CONTENT)
}

/// Routes for session canvas references.
pub fn canvas_refs_routes() -> axum::Router<ServerCtx> {
    axum::Router::new()
        .route(
            "/sessions/{sid}/canvas-refs",
            axum::routing::get(list_refs).post(add_ref),
        )
        .route(
            "/sessions/{sid}/canvas-refs/{scene_id}",
            axum::routing::delete(remove_ref),
        )
}
