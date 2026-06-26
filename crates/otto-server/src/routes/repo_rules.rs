//! Repo-rules management endpoints (the Context-Engine half of the review loop).
//!
//! - `GET    /workspaces/{ws}/repo-rules`  — list all rules for a workspace
//! - `POST   /repo-rules/{id}/toggle`      — enable/disable a rule (re-renders context)
//! - `DELETE /repo-rules/{id}`             — delete a rule (re-renders context)
//!
//! Rules are CREATED from a finding via `POST /findings/{id}/repo-rule` (in
//! `routes/findings.rs`). On every mutation the workspace's enabled rules are
//! re-rendered into its context (`finding_context::apply_repo_rules_to_context`).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde::Deserialize;

use otto_core::domain::WorkspaceRole;
use otto_core::finding::RepoRule;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::finding_context::apply_repo_rules_to_context;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{ws}/repo-rules", get(list))
        .route("/repo-rules/{id}/toggle", post(toggle))
        .route("/repo-rules/{id}", delete(remove))
}

/// `GET /workspaces/{ws}/repo-rules` — all repo rules for the workspace.
async fn list(
    Path(ws): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<RepoRule>>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let rules = ctx.repo_rules_store.list(&ws).await.map_err(ApiError)?;
    Ok(Json(rules))
}

#[derive(Deserialize)]
struct ToggleReq {
    enabled: bool,
}

/// `POST /repo-rules/{id}/toggle` — enable/disable; re-renders the context block.
async fn toggle(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ToggleReq>,
) -> ApiResult<Json<RepoRule>> {
    let existing = ctx.repo_rules_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    let rule = ctx
        .repo_rules_store
        .set_enabled(&id, body.enabled)
        .await
        .map_err(ApiError)?;
    let _ = apply_repo_rules_to_context(&ctx, &rule.workspace_id).await;
    Ok(Json(rule))
}

/// `DELETE /repo-rules/{id}` — delete; re-renders the context block.
async fn remove(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let existing = ctx.repo_rules_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    ctx.repo_rules_store.delete(&id).await.map_err(ApiError)?;
    let _ = apply_repo_rules_to_context(&ctx, &existing.workspace_id).await;
    Ok(StatusCode::NO_CONTENT)
}
