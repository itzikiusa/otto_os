//! The workflow routes added on threads (engine.md §10.1, addendum §2.6),
//! mounted beside `protected_routes` as one sub-router:
//!
//! - `POST /workflow-runs/{id}/nodes/{node_id}/skip` — Editor; skips one
//!   running step (the chat `skip` marker); 409 when it is not running or is
//!   the approval gate the run waits on.
//! - `POST /workflows/{id}/optimize` `{change_ids?}` — Viewer; the rule-based
//!   draft for that selection (absent = the pre-selected changes).
//! - `POST /workflows/{id}/optimize/apply` `{base_version, change_ids}` —
//!   Editor and a person's credential (DR-110): the graph is re-derived from
//!   the stored version, never taken from the client, and saved with an
//!   atomic version check (409 when it changed, 422 when a change now fails).

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::workflows::{NodeStatus, WorkflowGraph};
use otto_core::{Error, Id};
use otto_state::WorkflowsRepo;
use otto_workflows::optimize::{self, Options};
use otto_workflows::validation::{validate, IssueClass};
use serde::Deserialize;
use serde_json::json;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub fn routes() -> axum::Router<ServerCtx> {
    use axum::routing::post;
    axum::Router::new()
        .route("/workflow-runs/{id}/nodes/{node_id}/skip", post(skip_node))
        .route("/workflows/{id}/optimize", post(draft))
        .route("/workflows/{id}/optimize/apply", post(apply))
}

async fn skip_node(
    Path((id, node_id)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let run = WorkflowsRepo::new(ctx.pool.clone()).get_run(&id).await?;
    require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    let running = run
        .nodes
        .iter()
        .any(|n| n.node_id == node_id && n.status == NodeStatus::Running);
    if !running {
        return Err(ApiError(Error::Conflict(format!(
            "step {node_id} is not running"
        ))));
    }
    if run.waiting_approval && run.approval_node_id.as_deref() == Some(node_id.as_str()) {
        return Err(ApiError(Error::Conflict(
            "an approval step can't be skipped — approve or reject it".into(),
        )));
    }
    otto_workflows::WorkflowCtx::request_skip_current(&ctx, &id, &node_id);
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize, Default)]
struct DraftReq {
    change_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ApplyReq {
    base_version: i64,
    change_ids: Vec<String>,
}

/// A draft must pass everything a run needs (warnings never block).
fn blocking(g: &WorkflowGraph) -> Vec<String> {
    let issues = validate(g).into_iter();
    issues
        .filter(|i| i.class != IssueClass::Warning)
        .map(|i| i.message)
        .collect()
}

/// `wf_max_parallel_steps` as the engine reads it (default 4, 1–8).
async fn max_parallel(ctx: &ServerCtx) -> usize {
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    let v = settings.get("wf_max_parallel_steps").await.ok().flatten();
    let n = v
        .and_then(|v| v.as_u64())
        .unwrap_or(otto_workflows::schedule::DEFAULT_MAX_PARALLEL_STEPS as u64);
    n.clamp(1, 8) as usize
}

async fn draft(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<DraftReq>>,
) -> ApiResult<Json<optimize::OptimizeDraft>> {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let wf = repo.get(&id).await?;
    require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Viewer).await?;
    let history = repo.list_runs(&id).await?;
    let req = body.map(|b| b.0).unwrap_or_default();
    let opts = Options {
        selection: req.change_ids.as_deref(),
        max_parallel_steps: max_parallel(&ctx).await,
        validate: &blocking,
    };
    Ok(Json(optimize::plan(&wf, &history, &opts)))
}

async fn apply(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApplyReq>,
) -> Result<Response, ApiError> {
    use crate::routes::workflows::{require_human_for, GRAPH_WRITE_REFUSED};
    require_human_for(&auth, GRAPH_WRITE_REFUSED)?;
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let wf = repo.get(&id).await?;
    require_ws_role(&ctx, &user, &wf.workspace_id, WorkspaceRole::Editor).await?;
    if wf.version != req.base_version {
        let now = wf.version;
        return Err(ApiError(Error::Conflict(format!(
            "workflow changed elsewhere (now version {now})"
        ))));
    }
    let history = repo.list_runs(&id).await?;
    let max_parallel_steps = max_parallel(&ctx).await;
    // Sync block: the options borrow a non-Sync validator.
    let derived = {
        let opts = Options {
            selection: Some(&req.change_ids),
            max_parallel_steps,
            validate: &blocking,
        };
        optimize::apply(&wf, &history, &req.change_ids, &opts).map(|g| {
            let draft = optimize::plan(&wf, &history, &opts);
            (
                g,
                optimize::version_note(&user.display_name, &draft, &req.change_ids),
            )
        })
    };
    let (graph, note) = match derived {
        Ok(v) => v,
        Err(message) => {
            let body = json!({"code": "invalid", "message": message});
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
        }
    };
    let saved = repo
        .publish(
            &id,
            None,
            None,
            None,
            Some(&graph),
            None,
            &note,
            Some(&user.id),
            None,
            Some(req.base_version),
        )
        .await?;
    Ok(Json(saved).into_response())
}

#[cfg(test)]
#[path = "workflows_ext_tests.rs"]
mod tests;
