//! HTTP routes for Goal Loops: AI-assisted goal definition, CRUD, and the
//! start/pause/resume/stop lifecycle. The controller engine lives in
//! [`crate::goal_loop`].

use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};

use otto_core::api::{CreateGoalLoopReq, DefineGoalReq, GoalLoopDraft, UpdateGoalLoopReq};
use otto_core::auth::AuthUser;
use otto_core::domain::{
    GoalLoop, GoalLoopConfig, GoalLoopDetail, GoalLoopLimits, GoalLoopStatus, WorkspaceRole,
};
use otto_core::{Error, Id};
use otto_state::NewGoalLoop;

use crate::error::{ApiError, ApiResult};
use crate::goal_loop;
use crate::goal_loop_parse::parse_definition;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{id}/goal-loops",
            get(list).post(create),
        )
        .route("/workspaces/{id}/goal-loops/define", post(define))
        .route("/goal-loops/{id}", get(detail).patch(patch).delete(remove))
        .route("/goal-loops/{id}/start", post(start))
        .route("/goal-loops/{id}/pause", post(pause))
        .route("/goal-loops/{id}/resume", post(resume))
        .route("/goal-loops/{id}/stop", post(stop))
        .route(
            "/goal-loops/{id}/iterations/{idx}/agents/{agent}/retry",
            post(retry),
        )
}

async fn check(ctx: &ServerCtx, user: &AuthUser, ws: &Id, role: WorkspaceRole) -> ApiResult<()> {
    ctx.roles.check(&user.0, ws, role).await.map_err(ApiError)
}

/// Resolve a loop and verify the caller's role on its workspace.
async fn loop_for(
    ctx: &ServerCtx,
    user: &AuthUser,
    id: &Id,
    role: WorkspaceRole,
) -> ApiResult<GoalLoop> {
    let loop_ = ctx.goal_loops_repo.get(id).await.map_err(ApiError)?;
    check(ctx, user, &loop_.workspace_id, role).await?;
    Ok(loop_)
}

fn budget_gate(verdict: crate::routes::usage::BudgetVerdict) -> ApiResult<()> {
    if verdict.blocked {
        return Err(ApiError(Error::Invalid(format!(
            "Budget exceeded: {}",
            verdict.reason.unwrap_or_else(|| "cap reached".to_string())
        ))));
    }
    Ok(())
}

// --- Define (AI-assisted) --------------------------------------------------

async fn define(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Json(req): Json<DefineGoalReq>,
) -> ApiResult<Json<GoalLoopDraft>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    budget_gate(crate::routes::usage::check_budget(&ctx, &ws, "").await)?;

    let cfg = GoalLoopConfig::default();
    let mut prompt = format!("{}\n\n## Rough goal\n{}\n", cfg.definer.prompt, req.seed);
    if let Some(c) = req.context.as_deref().filter(|s| !s.trim().is_empty()) {
        prompt.push_str(&format!("\n## Existing draft / context\n{c}\n"));
    }
    if let Some(f) = req.feedback.as_deref().filter(|s| !s.trim().is_empty()) {
        prompt.push_str(&format!("\n## Refine with this feedback\n{f}\n"));
    }
    prompt.push_str(
        "\n---\nReply with ONLY a JSON object of this exact shape (no prose, no fence):\n\
         {\"title\": string, \"summary\": string, \"objectives\": [string], \
         \"acceptance_criteria\": [{\"id\": string, \"text\": string, \"verify\": string, \
         \"verify_kind\": \"command\"|\"manual\", \"verify_cmd\": string|null}], \
         \"constraints\": [string], \"out_of_scope\": [string], \"success_signal\": string}\n\
         Every criterion's \"verify\" MUST be concrete. Prefer verify_kind=\"command\" with a \
         shell verify_cmd that exits 0 when the criterion is met.",
    );

    let model = if cfg.definer.model.is_empty() {
        None
    } else {
        Some(cfg.definer.model.as_str())
    };
    let fut = ctx
        .orchestrator
        .run_agent(&prompt, &req.repo_path, model, Duration::from_secs(240));
    let text = match tokio::time::timeout(Duration::from_secs(300), fut).await {
        Ok(Ok(t)) => t,
        Ok(Err(e)) => return Err(ApiError(e)),
        Err(_) => {
            return Err(ApiError(Error::Internal(
                "goal definer timed out".to_string(),
            )))
        }
    };
    let definition = parse_definition(&text).ok_or_else(|| {
        ApiError(Error::Internal(
            "could not parse a goal definition from the agent's reply".to_string(),
        ))
    })?;

    Ok(Json(GoalLoopDraft {
        definition,
        suggested_limits: GoalLoopLimits::default(),
        suggested_config: cfg,
    }))
}

// --- CRUD ------------------------------------------------------------------

async fn list(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
) -> ApiResult<Json<Vec<GoalLoop>>> {
    check(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let loops = ctx
        .goal_loops_repo
        .list_by_workspace(&ws)
        .await
        .map_err(ApiError)?;
    Ok(Json(loops))
}

async fn create(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Json(req): Json<CreateGoalLoopReq>,
) -> ApiResult<Json<GoalLoop>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;

    // Validate: every acceptance criterion must carry a non-empty verify (the
    // evaluator's anchor) and a command-kind criterion must carry a command.
    if req.definition.acceptance_criteria.is_empty() {
        return Err(ApiError(Error::Invalid(
            "a goal needs at least one acceptance criterion".into(),
        )));
    }
    for c in &req.definition.acceptance_criteria {
        if c.verify.trim().is_empty() {
            return Err(ApiError(Error::Invalid(format!(
                "acceptance criterion '{}' needs a non-empty verify",
                c.id
            ))));
        }
        if c.verify_kind == "command" && c.verify_cmd.as_deref().unwrap_or("").trim().is_empty() {
            return Err(ApiError(Error::Invalid(format!(
                "criterion '{}' is verify_kind=command but has no verify_cmd",
                c.id
            ))));
        }
    }
    if req.config.executors.is_empty() {
        return Err(ApiError(Error::Invalid(
            "a goal loop needs at least one executor".into(),
        )));
    }

    let loop_ = ctx
        .goal_loops_repo
        .create(NewGoalLoop {
            workspace_id: ws.clone(),
            name: req.name,
            repo_path: req.repo_path,
            definition: req.definition,
            limits: req.limits,
            config: req.config,
            created_by: user.0.id.clone(),
        })
        .await
        .map_err(ApiError)?;

    if req.autostart {
        budget_gate(crate::routes::usage::check_budget(&ctx, &ws, "").await)?;
        goal_loop::start_loop(&ctx, &loop_.id).await.map_err(ApiError)?;
        return Ok(Json(ctx.goal_loops_repo.get(&loop_.id).await.map_err(ApiError)?));
    }
    Ok(Json(loop_))
}

async fn detail(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GoalLoopDetail>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let detail = ctx
        .goal_loops_repo
        .get_detail(&loop_.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(detail))
}

async fn patch(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateGoalLoopReq>,
) -> ApiResult<Json<GoalLoop>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;

    if let Some(name) = req.name {
        if loop_.status.is_terminal() {
            return Err(ApiError(Error::Invalid(
                "cannot rename a finished loop".into(),
            )));
        }
        ctx.goal_loops_repo.set_name(&id, &name).await.map_err(ApiError)?;
    }
    if let Some(limits) = req.limits {
        // Limits may be raised while editable or paused/blocked/exhausted (to
        // continue), but not while actively Running.
        if loop_.status == GoalLoopStatus::Running {
            return Err(ApiError(Error::Invalid(
                "pause the loop before changing limits".into(),
            )));
        }
        ctx.goal_loops_repo.set_limits(&id, &limits).await.map_err(ApiError)?;
    }
    if let Some(config) = req.config {
        // Reshaping the executor lineup mid-run would break live agent indices;
        // config is editable in Draft only.
        if loop_.status != GoalLoopStatus::Draft {
            return Err(ApiError(Error::Invalid(
                "config can only be edited while the loop is a draft".into(),
            )));
        }
        ctx.goal_loops_repo.set_config(&id, &config).await.map_err(ApiError)?;
    }
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn remove(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    // Stop any controller + kill sessions, remove the worktree, then delete.
    let _ = goal_loop::stop_loop(&ctx, &id).await;
    crate::goal_loop_workspace::remove_worktree(&ctx, &loop_).await;
    ctx.goal_loops_repo.delete(&id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Lifecycle -------------------------------------------------------------

async fn start(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GoalLoop>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if loop_.status != GoalLoopStatus::Draft {
        return Err(ApiError(Error::Invalid(
            "only a draft loop can be started; use resume to continue an existing loop".into(),
        )));
    }
    budget_gate(crate::routes::usage::check_budget(&ctx, &loop_.workspace_id, "").await)?;
    goal_loop::start_loop(&ctx, &id).await.map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn pause(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GoalLoop>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if loop_.status != GoalLoopStatus::Running {
        return Err(ApiError(Error::Invalid("loop is not running".into())));
    }
    goal_loop::pause_loop(&ctx, &id).await.map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn resume(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GoalLoop>> {
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    // Only resumable states. Failed/Stopped/Succeeded are terminal (their
    // worktrees may be gone); Draft uses start; Running is already going.
    if !matches!(
        loop_.status,
        GoalLoopStatus::Paused | GoalLoopStatus::Blocked | GoalLoopStatus::Exhausted
    ) {
        return Err(ApiError(Error::Invalid(
            "only paused, blocked, or exhausted loops can be resumed".into(),
        )));
    }
    budget_gate(crate::routes::usage::check_budget(&ctx, &loop_.workspace_id, "").await)?;
    goal_loop::start_loop(&ctx, &id).await.map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn stop(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GoalLoop>> {
    let _ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    goal_loop::stop_loop(&ctx, &id).await.map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn retry(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((id, idx, agent)): Path<(Id, u32, usize)>,
) -> ApiResult<StatusCode> {
    let _ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    goal_loop::retry_executor(&ctx, &id, idx, agent)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::ACCEPTED)
}
