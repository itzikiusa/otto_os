//! Goal routes (goal.md §3.1, §5, §7.2). Goals: start one, list goals beside
//! older loops (directory kind `goal`), MCP's Goal Loops request, Continue as
//! a goal. Any workflow run: Pause, Resume (with Extend), steering notes and a
//! person's criterion record. Older goal loops: the read-only history (list,
//! detail, one iteration, the kept work's diff, delete). The loop
//! controller's routes are gone and answer 404 like any unknown route.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use otto_core::api::{
    CreateGoalLoopReq, GoalSpec, GoalStartResp, ResumeReq, SteerReq, VerifyCriterionReq,
};
use otto_core::domain::{GoalLoop, GoalLoopDetail, GoalLoopIteration, WorkspaceRole};
use otto_core::workflows::{HoldKind, RunHold, RunScope, RunStatus, WorkflowRun};
use otto_core::{Error, Id};
use otto_state::WorkflowsRepo;
use serde_json::{json, Value};

use crate::auth::{require_human, require_ws_role, CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use crate::workflow_engine;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{wid}/goals", get(list).post(start))
        .route("/workspaces/{wid}/goals/from-loop", post(from_loop))
        .route("/goal-loops/{id}/continue", post(continue_loop))
        .route("/workflow-runs/{id}/pause", post(pause))
        .route("/workflow-runs/{id}/resume", post(resume))
        .route("/workflow-runs/{id}/steer", post(steer))
        .route(
            "/workflow-runs/{id}/nodes/{node_id}/criteria/{criterion_id}/verify",
            post(verify),
        )
        .route("/workspaces/{id}/goal-loops", get(loops))
        .route("/goal-loops/{id}", get(loop_detail).delete(delete_loop))
        .route("/goal-loops/{id}/iterations/{idx}", get(loop_iteration))
        .route("/goal-loops/{id}/diff", get(loop_diff))
}

fn repo(ctx: &ServerCtx) -> WorkflowsRepo {
    WorkflowsRepo::new(ctx.pool.clone())
}

fn started((workflow, run): (otto_core::workflows::Workflow, WorkflowRun)) -> Json<GoalStartResp> {
    Json(GoalStartResp { workflow, run })
}

async fn start(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(spec): Json<GoalSpec>,
) -> ApiResult<Json<GoalStartResp>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let origin_ref = (spec.from.as_deref() == Some("insights")).then_some("insights");
    let out = crate::goals::start(&ctx, &user, &wid, &spec, origin_ref).await;
    out.map(started).map_err(ApiError)
}

/// Goal runs, newest first (at most 50), then the workspace's older loops.
async fn list(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Value>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let last = otto_state::workflow_last_runs::last_runs(&ctx.pool, &wid)
        .await
        .map_err(ApiError)?;
    let mut goals: Vec<_> = repo(&ctx).list(&wid).await.map_err(ApiError)?;
    goals.retain(|wf| wf.origin.as_deref() == Some("goal"));
    goals.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let mut items: Vec<Value> = goals
        .iter()
        .filter_map(|wf| {
            last.iter().find(|r| r.0 == wf.id).map(|(_, r, _)| {
                json!({
            "id": r.id, "run_id": r.id, "workflow_id": wf.id, "name": wf.name,
            "state": r.overview_state.as_deref().unwrap_or(&r.status),
            "started_at": wf.created_at, "ended_at": r.ended_at})
            })
        })
        .take(50)
        .collect();
    let loops = ctx
        .goal_loops_repo
        .list_by_workspace(&wid)
        .await
        .map_err(ApiError)?;
    items.extend(
        loops
            .iter()
            .map(|l| json!({"id": l.id, "name": l.name, "status": l.status, "legacy": true})),
    );
    Ok(Json(items))
}

async fn from_loop(
    Path(wid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateGoalLoopReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let (workflow, run, ignored) = crate::goals::from_loop(&ctx, &user, &wid, &req)
        .await
        .map_err(ApiError)?;
    Ok(Json(
        json!({"workflow": workflow, "run": run, "workflow_id": workflow.id, "run_id": run.id, "ignored": ignored}),
    ))
}

async fn continue_loop(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(spec): Json<GoalSpec>,
) -> ApiResult<Json<GoalStartResp>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let out = crate::goals::continue_loop(&ctx, &user, &loop_, spec).await;
    out.map(started).map_err(ApiError)
}

/// The run and the caller's Editor role on its workspace.
async fn run_for(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &Id,
) -> ApiResult<WorkflowRun> {
    let run = repo(ctx).get_run(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &run.workspace_id, WorkspaceRole::Editor).await?;
    Ok(run)
}

/// Pause (DR-106): the hold is written before the run's steps unwind.
async fn pause(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkflowRun>> {
    let run = run_for(&ctx, &user, &id).await?;
    let hold = RunHold {
        kind: HoldKind::Paused,
        reason: format!("Paused by {}", user.display_name),
        since: Utc::now(),
        by: Some(user.id.clone()),
        node_ids: vec![],
        iteration: None,
    };
    // The driver unwinds on the announced cancel and publishes the hold.
    if repo(&ctx)
        .request_pause(&run.id, &hold)
        .await
        .map_err(ApiError)?
        .is_none()
    {
        return Err(ApiError(Error::Conflict(
            "Only a queued or running run can be paused".into(),
        )));
    }
    repo(&ctx).get_run(&id).await.map(Json).map_err(ApiError)
}

/// Resume a held run (goal.md §2.3): Extend writes the loop's new limits,
/// then the run re-enters with every unsettled step, adopting the rest.
async fn resume(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ResumeReq>,
) -> ApiResult<Json<WorkflowRun>> {
    let run = run_for(&ctx, &user, &id).await?;
    let wf = repo(&ctx)
        .definition_for_run(&run)
        .await
        .map_err(ApiError)?;
    let extend = req.extend.unwrap_or_default();
    let conflict = |m: String| ApiError(Error::Conflict(m));
    let plan = otto_workflows::goal::resume_plan(&run, &wf.graph, &extend).map_err(conflict)?;
    if workflow_engine::driver_alive(&id) {
        return Err(conflict(
            "the run is still stopping — try again in a few seconds".into(),
        ));
    }
    if let Some(limits) = &plan.limits {
        repo(&ctx)
            .set_run_limits(&id, limits)
            .await
            .map_err(ApiError)?;
    }
    let scope = RunScope {
        continue_unfinished: true,
        ..Default::default()
    };
    repo(&ctx)
        .prepare_retry(&id, &plan.unsettled, false, &scope)
        .await
        .map_err(ApiError)?;
    let ws = ctx
        .workspaces
        .get(&run.workspace_id)
        .await
        .map_err(ApiError)?;
    workflow_engine::spawn_run(
        ctx.clone(),
        ws,
        wf,
        id.clone(),
        run.input.clone(),
        scope,
        Some(plan.nodes),
    );
    repo(&ctx).get_run(&id).await.map(Json).map_err(ApiError)
}

/// A note for the next agent step (goal.md §2.4); a person's credential only.
async fn steer(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    auth: CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SteerReq>,
) -> ApiResult<Json<Value>> {
    require_human(&auth.0)?;
    let run = run_for(&ctx, &user, &id).await?;
    if !matches!(run.status, RunStatus::Pending | RunStatus::Running) && run.hold.is_none() {
        return Err(ApiError(Error::Conflict(
            "This run has finished: a note would reach no step".into(),
        )));
    }
    let dir = otto_workflows::attachments::context_dir(&ctx.data_dir, &id);
    let queued = otto_workflows::steering::queue(&dir, &req.note)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({"queued": queued})))
}

/// A person's record for a criterion the run's check waits on (DR-107):
/// `node_id` is the top-level node (the check, or the loop around it).
async fn verify(
    Path((id, node_id, criterion_id)): Path<(Id, String, String)>,
    State(ctx): State<ServerCtx>,
    auth: CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<VerifyCriterionReq>,
) -> ApiResult<Json<Value>> {
    require_human(&auth.0)?;
    let run = run_for(&ctx, &user, &id).await?;
    let wf = repo(&ctx)
        .definition_for_run(&run)
        .await
        .map_err(ApiError)?;
    let at = (node_id.as_str(), criterion_id.as_str());
    let entry = otto_workflows::goal::verify_entry(
        &run,
        &wf.graph,
        at,
        req.met,
        &req.evidence,
        &user.display_name,
    )
    .map_err(ApiError)?;
    crate::goals::deliver_verify(&ctx, &id, &node_id, entry)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

// ---- Older goal loops: read-only history (goal.md §5.2) ----------------------

async fn loop_for(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &Id,
    role: WorkspaceRole,
) -> ApiResult<GoalLoop> {
    let loop_ = ctx.goal_loops_repo.get(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &loop_.workspace_id, role).await?;
    Ok(loop_)
}

async fn loops(
    Path(ws): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<GoalLoop>>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    ctx.goal_loops_repo
        .list_by_workspace(&ws)
        .await
        .map(Json)
        .map_err(ApiError)
}

#[derive(Default, serde::Deserialize)]
struct DetailQuery {
    /// `summary=true`: older iterations without their long texts.
    #[serde(default)]
    summary: bool,
}

#[derive(serde::Serialize)]
struct LoopDetail {
    #[serde(flatten)]
    detail: GoalLoopDetail,
    /// Never tracked (the controller refused any cost cap); kept for the page.
    spent_usd: Option<f64>,
    steering_notes: Vec<String>,
}

async fn loop_detail(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<DetailQuery>,
) -> ApiResult<Json<LoopDetail>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let repo = &ctx.goal_loops_repo;
    let detail = if q.summary {
        repo.get_detail_summary(&loop_.id).await
    } else {
        repo.get_detail(&loop_.id).await
    };
    let steering_notes =
        otto_automation::goal_loop_history::peek_notes(&ctx.data_dir, &loop_.id).await;
    Ok(Json(LoopDetail {
        detail: detail.map_err(ApiError)?,
        spent_usd: None,
        steering_notes,
    }))
}

async fn loop_iteration(
    Path((id, idx)): Path<(Id, u32)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<GoalLoopIteration>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    ctx.goal_loops_repo
        .get_iteration(&loop_.id, idx)
        .await
        .map(Json)
        .map_err(ApiError)
}

async fn loop_diff(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<otto_automation::goal_loop_history::GoalLoopDiff>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let cap = otto_automation::goal_loop_history::DEFAULT_DIFF_PATCH_CAP;
    let diff = otto_automation::goal_loop_history::worktree_diff(&loop_, cap).await;
    diff.map(Json).map_err(ApiError)
}

/// Delete a loop's history; its kept working files stay on disk.
async fn delete_loop(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    ctx.goal_loops_repo
        .delete(&loop_.id)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}
