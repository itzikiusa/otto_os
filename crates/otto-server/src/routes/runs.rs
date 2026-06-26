//! Run with Otto REST endpoints — the one-button source→PR-draft pipeline.
//!
//! Two-axis RBAC: `policy.rs` enforces `Feature::RunWithOtto` (View for GET, Edit
//! for writes); every handler *additionally* enforces the workspace-role axis with
//! `require_ws_role`. Flat by-id routes load the run first and check the role on
//! its `workspace_id` (the IDOR guard). The webhook entry
//! (`POST /webhooks/{ws}/run`) lives in `channel_webhook` (public, key-guarded).

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::domain::WorkspaceRole;
use otto_core::run::{
    parse_source_ref, ApproveRunReq, LaunchRunReq, OttoRun, RunEvent, RunOrigin,
};
use otto_core::Id;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::run_service::{self, LaunchOrigin};
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{wid}/runs", get(list).post(launch))
        .route("/workspaces/{wid}/runs/detect", get(detect))
        .route("/runs/{id}", get(get_one))
        .route("/runs/{id}/events", get(events))
        .route("/runs/{id}/approve", post(approve))
        .route("/runs/{id}/cancel", post(cancel))
        .route("/runs/{id}/open-pr", post(open_pr))
}

const LIST_LIMIT: i64 = 200;

/// `GET /workspaces/{wid}/runs`
async fn list(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<OttoRun>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    Ok(Json(
        ctx.runs
            .list_by_workspace(&wid, LIST_LIMIT)
            .await
            .map_err(ApiError)?,
    ))
}

/// `POST /workspaces/{wid}/runs` — the one button (UI / programmatic launch).
async fn launch(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<LaunchRunReq>,
) -> ApiResult<Json<OttoRun>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let run = run_service::launch(
        &ctx,
        &wid,
        &user.id,
        RunOrigin::Ui,
        LaunchOrigin::default(),
        req,
    )
    .await
    .map_err(ApiError)?;
    Ok(Json(run))
}

#[derive(Deserialize)]
struct DetectQuery {
    q: String,
}

/// `GET /workspaces/{wid}/runs/detect?q=` — preview the source kind for a ref, so
/// the launcher can show "detected: Jira PROJ-1" before the user commits.
async fn detect(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<DetectQuery>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let detected = parse_source_ref(&q.q).map(|(kind, sref, url)| {
        json!({ "source_kind": kind.as_str(), "source_ref": sref, "url": url })
    });
    Ok(Json(json!({ "detected": detected })))
}

/// Load a run and enforce the workspace-role axis on its workspace (IDOR guard).
async fn load_for_role(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &Id,
    role: WorkspaceRole,
) -> Result<OttoRun, ApiError> {
    let run = ctx.runs.get(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &run.workspace_id, role).await?;
    Ok(run)
}

/// `GET /runs/{id}`
async fn get_one(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<OttoRun>> {
    Ok(Json(load_for_role(&ctx, &user, &id, WorkspaceRole::Viewer).await?))
}

/// `GET /runs/{id}/events` — the stage timeline.
async fn events(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<RunEvent>>> {
    load_for_role(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.runs.list_events(&id).await.map_err(ApiError)?))
}

/// `POST /runs/{id}/approve` — approve or reject at the gate.
async fn approve(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApproveRunReq>,
) -> ApiResult<Json<OttoRun>> {
    load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = if user.display_name.trim().is_empty() {
        user.id.clone()
    } else {
        user.display_name.clone()
    };
    Ok(Json(
        run_service::approve(&ctx, &id, &req, &who)
            .await
            .map_err(ApiError)?,
    ))
}

/// `POST /runs/{id}/cancel`
async fn cancel(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<OttoRun>> {
    load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(run_service::cancel(&ctx, &id).await.map_err(ApiError)?))
}

/// `POST /runs/{id}/open-pr` — open the actual PR from a completed, approved run.
async fn open_pr(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<otto_core::api::PrSummary>> {
    load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    Ok(Json(run_service::open_pr(&ctx, &id).await.map_err(ApiError)?))
}
