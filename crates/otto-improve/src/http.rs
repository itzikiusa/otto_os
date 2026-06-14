//! Axum router for self-improvement endpoints. Paths relative to `/api/v1`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::api::{Problem, RunNowResp, SelfImprovementConfig, UpdateSelfImprovementReq};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{
    ImprovementEdit, ImprovementEditStatus, ImprovementRun, WorkspaceRole,
};
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;
use serde::Deserialize;

use crate::config::{effective_config, write_config};
use crate::engine::{ImprovementEngine, RUN_LIST_LIMIT};

pub trait ImproveCtx: Clone + Send + Sync + 'static {
    fn engine(&self) -> &Arc<ImprovementEngine>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
}

struct ApiErr(Error);
impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        Self(e)
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
        (status, Json(Problem { code: self.0.code().to_string(), message: self.0.to_string() }))
            .into_response()
    }
}
type ApiResult<T> = std::result::Result<T, ApiErr>;

pub fn router<S: ImproveCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/self-improvement",
            get(get_config::<S>).put(put_config::<S>),
        )
        .route("/workspaces/{id}/self-improvement/run", post(run_now::<S>))
        .route("/workspaces/{id}/improvement/runs", get(list_runs::<S>))
        .route("/improvement/runs/{run_id}", get(get_run::<S>))
        .route("/workspaces/{id}/improvement/edits", get(list_edits::<S>))
        .route("/improvement/edits/{eid}/approve", post(approve::<S>))
        .route("/improvement/edits/{eid}/reject", post(reject::<S>))
        .route("/improvement/edits/{eid}/rollback", post(rollback::<S>))
}

async fn get_config<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<SelfImprovementConfig>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Viewer).await?;
    let ws = s.workspaces().get(&ws_id).await?;
    Ok(Json(effective_config(&ws.settings)))
}

async fn put_config<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Json(req): Json<UpdateSelfImprovementReq>,
) -> ApiResult<Json<SelfImprovementConfig>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Admin).await?;
    let ws = s.workspaces().get(&ws_id).await?;
    let mut cfg = effective_config(&ws.settings);
    cfg.enabled = req.enabled;
    cfg.cadence_minutes = req.cadence_minutes;
    cfg.lookback_hours = req.lookback_hours;
    cfg.skill_allowlist = req.skill_allowlist;
    cfg.autonomy = req.autonomy;
    cfg.providers = if req.providers.is_empty() {
        vec!["claude".to_string()]
    } else {
        req.providers
    };
    cfg.live_evolve = req.live_evolve;
    // Changing config recomputes the next run lazily (clear so it's due soon if
    // enabled; the scheduler will set next_run after the next pass).
    let merged = write_config(&ws.settings, &cfg);
    s.workspaces().update(&ws_id, None, None, Some(&merged), None).await?;
    Ok(Json(cfg))
}

async fn run_now<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<RunNowResp>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Editor).await?;
    if s.engine().improvements.has_running(&ws_id).await? {
        return Err(ApiErr(Error::Conflict("a run is already in progress".into())));
    }
    // Mirror the PR-review pattern (modules.rs::start_review): create the run
    // row synchronously so we can return its id immediately, then run the heavy
    // analysis in the background.
    let run = s
        .engine()
        .improvements
        .create_run(&ws_id, otto_core::domain::ImprovementTrigger::Manual)
        .await?;
    let run_id = run.id.clone();
    let engine = Arc::clone(s.engine());
    let ws = ws_id.clone();
    let bg_run_id = run_id.clone();
    tokio::spawn(async move {
        if let Err(e) = engine
            .execute_run(&bg_run_id, &ws, otto_core::domain::ImprovementTrigger::Manual)
            .await
        {
            tracing::warn!(workspace = %ws, "self-improvement manual run failed: {e}");
        }
    });
    Ok(Json(RunNowResp { run_id }))
}

async fn list_runs<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<ImprovementRun>>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.engine().improvements.list_runs(&ws_id, RUN_LIST_LIMIT).await?))
}

async fn get_run<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(run_id): Path<Id>,
) -> ApiResult<Json<serde_json::Value>> {
    let run = s.engine().improvements.get_run(&run_id).await?;
    s.roles().check(&user.0, &run.workspace_id, WorkspaceRole::Viewer).await?;
    let edits = s.engine().improvements.list_edits_by_run(&run_id).await?;
    Ok(Json(serde_json::json!({ "run": run, "edits": edits })))
}

#[derive(Deserialize)]
struct EditQuery {
    status: Option<String>,
}

async fn list_edits<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
    Query(q): Query<EditQuery>,
) -> ApiResult<Json<Vec<ImprovementEdit>>> {
    s.roles().check(&user.0, &ws_id, WorkspaceRole::Viewer).await?;
    let status = q
        .status
        .as_deref()
        .and_then(ImprovementEditStatus::parse)
        .unwrap_or(ImprovementEditStatus::Pending);
    Ok(Json(s.engine().improvements.list_edits_by_status(&ws_id, status).await?))
}

async fn check_edit_ws<S: ImproveCtx>(s: &S, user: &AuthUser, edit_id: &Id) -> Result<(), ApiErr> {
    let edit = s.engine().improvements.get_edit(edit_id).await?;
    s.roles().check(&user.0, &edit.workspace_id, WorkspaceRole::Editor).await?;
    Ok(())
}

async fn approve<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(eid): Path<Id>,
) -> ApiResult<Json<ImprovementEdit>> {
    check_edit_ws(&s, &user, &eid).await?;
    Ok(Json(s.engine().approve_edit(&eid, &user.0.id).await?))
}

async fn reject<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(eid): Path<Id>,
) -> ApiResult<Json<ImprovementEdit>> {
    check_edit_ws(&s, &user, &eid).await?;
    Ok(Json(s.engine().reject_edit(&eid, &user.0.id).await?))
}

async fn rollback<S: ImproveCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(eid): Path<Id>,
) -> ApiResult<Json<ImprovementEdit>> {
    check_edit_ws(&s, &user, &eid).await?;
    Ok(Json(s.engine().rollback_edit(&eid, &user.0.id).await?))
}
