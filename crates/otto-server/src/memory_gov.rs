//! Memory governance routes: lifecycle state transitions, soft-delete with undo,
//! merge, split, and governed import of AGENTS.md / CLAUDE.md / .cursorrules.
//!
//! Paths (relative to `/api/v1` mount point):
//!   POST /workspaces/{ws}/memory/{mid}/state
//!   POST /workspaces/{ws}/memory/{mid}/forget
//!   POST /workspaces/{ws}/memory/{mid}/forget/undo
//!   POST /workspaces/{ws}/memory/merge
//!   POST /workspaces/{ws}/memory/{mid}/split
//!   POST /workspaces/{ws}/memory/import
//!
//! Policy: all routes require `WorkspaceRole::Editor` (Product:Edit).

use axum::extract::{Path, State};
use axum::routing::post;
use axum::Json;
use axum::Router;
use otto_core::domain::WorkspaceRole;
use otto_core::Id;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

use otto_memory::governance::{
    ForgetResp, ImportReq, ImportResp, MergeReq, MergeResp, SetStateReq, SplitReq, SplitResp,
    UndoForgetReq,
};
use otto_memory::Memory;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{ws}/memory/{mid}/state`
///
/// Body: `{ "state": "suggested" | "accepted" | "stale" | "contradicted" }`
/// Response: the updated memory.
pub async fn set_state(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, mid)): Path<(Id, Id)>,
    Json(req): Json<SetStateReq>,
) -> ApiResult<Json<Memory>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let m = ctx.memory.set_state(&ws, &mid, &req.state).await?;
    Ok(Json(m))
}

/// `POST /api/v1/workspaces/{ws}/memory/{mid}/forget`
///
/// Soft-deletes the memory and returns an `undo_token` valid for later restore.
pub async fn forget(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, mid)): Path<(Id, Id)>,
) -> ApiResult<Json<ForgetResp>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let resp = ctx.memory.soft_forget(&ws, &mid).await?;
    Ok(Json(resp))
}

/// `POST /api/v1/workspaces/{ws}/memory/{mid}/forget/undo`
///
/// Body: `{ "undo_token": "..." }`
/// Restores the memory and returns the restored row.
pub async fn forget_undo(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, _mid)): Path<(Id, Id)>,
    Json(req): Json<UndoForgetReq>,
) -> ApiResult<Json<Memory>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let m = ctx.memory.undo_forget(&ws, &req.undo_token).await?;
    Ok(Json(m))
}

/// `POST /api/v1/workspaces/{ws}/memory/merge`
///
/// Body: `{ "ids": [...], "title": "...", "body": "..." }`
/// Response: the merged memory.
pub async fn merge(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(ws): Path<Id>,
    Json(req): Json<MergeReq>,
) -> ApiResult<Json<MergeResp>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let memory = ctx.memory.merge(&ws, &user.id, req).await?;
    Ok(Json(MergeResp { memory }))
}

/// `POST /api/v1/workspaces/{ws}/memory/{mid}/split`
///
/// Body: `{ "parts": [{ "title": "...", "body": "..." }, ...] }`
/// Response: the N child memories.
pub async fn split(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path((ws, mid)): Path<(Id, Id)>,
    Json(req): Json<SplitReq>,
) -> ApiResult<Json<SplitResp>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let resp = ctx.memory.split(&ws, &user.id, &mid, req).await?;
    Ok(Json(resp))
}

/// `POST /api/v1/workspaces/{ws}/memory/import`
///
/// Body: `{ "kind": "agents-md"|"claude-md"|"cursorrules"|"custom", "content": "...", "label": "..." }`
/// Response: `{ "imported": <count>, "import_id": "<id>" }`
pub async fn import_governed(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Path(ws): Path<Id>,
    Json(req): Json<ImportReq>,
) -> ApiResult<Json<ImportResp>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let resp = ctx.memory.import_governed(&ws, &user.id, req).await?;
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Memory governance router — merged into the protected API by `build_router`.
/// Paths are relative to the `/api/v1` mount point.
pub fn memory_gov_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{ws}/memory/{mid}/state",
            post(set_state),
        )
        .route(
            "/workspaces/{ws}/memory/{mid}/forget",
            post(forget),
        )
        .route(
            "/workspaces/{ws}/memory/{mid}/forget/undo",
            post(forget_undo),
        )
        .route("/workspaces/{ws}/memory/merge", post(merge))
        .route("/workspaces/{ws}/memory/{mid}/split", post(split))
        .route("/workspaces/{ws}/memory/import", post(import_governed))
}
