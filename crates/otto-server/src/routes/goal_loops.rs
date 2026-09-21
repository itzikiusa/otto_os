//! HTTP routes for Goal Loops: AI-assisted goal definition, CRUD, and the
//! start/pause/resume/stop lifecycle. The controller engine lives in
//! [`crate::goal_loop`].

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Extension, Json, Router};

use otto_core::api::{
    AnswerGoalQuestionReq, CreateGoalLoopReq, DefineGoalReq, GoalLoopDraft, UpdateGoalLoopReq,
    VerifyGoalCriterionReq,
};
use otto_core::auth::AuthUser;
use otto_core::domain::{
    GoalLoop, GoalLoopConfig, GoalLoopDetail, GoalLoopLimits, GoalLoopStatus, WorkspaceRole,
};
use otto_core::{Error, Id};
use otto_state::NewGoalLoop;

use crate::auth::CurrentAuthContext;
use crate::error::{ApiError, ApiResult};
use crate::goal_loop;
use crate::goal_loop_parse::parse_definition;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/goal-loops", get(list).post(create))
        .route("/workspaces/{id}/goal-loops/define", post(define))
        .route("/goal-loops/{id}", get(detail).patch(patch).delete(remove))
        .route("/goal-loops/{id}/start", post(start))
        .route("/goal-loops/{id}/pause", post(pause))
        .route("/goal-loops/{id}/resume", post(resume))
        .route("/goal-loops/{id}/stop", post(stop))
        .route(
            "/goal-loops/{id}/criteria/{criterion}/verify",
            post(verify_criterion),
        )
        .route(
            "/goal-loops/{id}/questions/{question}/answer",
            post(answer_question),
        )
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

    let mut cfg = GoalLoopConfig::default();
    cfg.definer.provider = req.provider.clone().unwrap_or_default();
    cfg.definer.model = req.model.clone().unwrap_or_default();
    cfg.mode = req.mode.clone().unwrap_or_else(|| "build".into());
    validate_settings(&cfg, &GoalLoopLimits::default())?;
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
         \"verify_kind\": \"command\"|\"agent\"|\"human\", \"verify_cmd\": string|null}], \
         \"constraints\": [string], \"out_of_scope\": [string], \"success_signal\": string}\n\
         Every criterion's \"verify\" MUST be concrete. Prefer verify_kind=\"command\" with a \
         shell verify_cmd that exits 0 when the criterion is met.",
    );

    let workspace = ctx.workspaces.get(&ws).await.map_err(ApiError)?;
    let research_dir = tempfile::Builder::new()
        .prefix("otto-goal-research-")
        .tempdir()
        .map_err(|e| ApiError(Error::Internal(e.to_string())))?;
    let cwd = if cfg.mode == "research" {
        research_dir.path().to_string_lossy().into_owned()
    } else {
        req.repo_path.clone()
    };
    if cfg.mode == "research" {
        prompt.push_str("\nThis is research: define questions answered by cited evidence and a findings.md report. Do not require repository modifications, commits or code-review stages.");
    }
    let text =
        crate::goal_loop_roles::define(&ctx, &workspace, &user.0, &cfg.definer, &prompt, &cwd)
            .await
            .map_err(ApiError)?;
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
    Json(mut req): Json<CreateGoalLoopReq>,
) -> ApiResult<Json<GoalLoop>> {
    check(&ctx, &user, &ws, WorkspaceRole::Editor).await?;

    validate_settings(&req.config, &req.limits)?;
    if req.config.mode == "research" {
        // A durable report is part of research completion, even if the draft
        // forgot to list it. Reserve this id so callers cannot weaken its check.
        req.definition
            .acceptance_criteria
            .retain(|c| c.id != "otto-research-report");
        req.definition
            .acceptance_criteria
            .push(otto_core::domain::AcceptanceCriterion {
                id: "otto-research-report".into(),
                text: "Research findings are saved with cited sources".into(),
                verify: "A nonempty findings.md report exists; the evaluator checks its citations"
                    .into(),
                verify_kind: "command".into(),
                verify_cmd: Some("test -s findings.md".into()),
            });
    }

    let mut ids = std::collections::HashSet::new();
    for c in &req.definition.acceptance_criteria {
        if c.id.trim().is_empty() || !ids.insert(c.id.as_str()) || c.text.trim().is_empty() {
            return Err(ApiError(Error::Invalid(
                "criteria need unique nonempty ids and descriptions".into(),
            )));
        }
        if !matches!(
            c.verify_kind.as_str(),
            "command" | "agent" | "manual" | "human"
        ) {
            return Err(ApiError(Error::Invalid(
                "verify_kind must be command, agent or human".into(),
            )));
        }
    }
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
        goal_loop::start_loop(&ctx, &loop_.id)
            .await
            .map_err(ApiError)?;
        return Ok(Json(
            ctx.goal_loops_repo.get(&loop_.id).await.map_err(ApiError)?,
        ));
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
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;

    if let Some(name) = req.name {
        if loop_.status.is_terminal() {
            return Err(ApiError(Error::Invalid(
                "cannot rename a finished loop".into(),
            )));
        }
        ctx.goal_loops_repo
            .set_name(&id, &name)
            .await
            .map_err(ApiError)?;
    }
    if let Some(limits) = req.limits {
        validate_settings(&loop_.config, &limits)?;
        // Limits may be raised while editable or paused/blocked/exhausted (to
        // continue), but not while actively Running.
        if loop_.status == GoalLoopStatus::Running {
            return Err(ApiError(Error::Invalid(
                "pause the loop before changing limits".into(),
            )));
        }
        ctx.goal_loops_repo
            .set_limits(&id, &limits)
            .await
            .map_err(ApiError)?;
    }
    if let Some(config) = req.config {
        if config.mode != loop_.config.mode {
            return Err(ApiError(Error::Invalid(
                "mode cannot change after creation; create a new goal".into(),
            )));
        }
        validate_settings(&config, &loop_.limits)?;
        // Reshaping the executor lineup mid-run would break live agent indices;
        // config is editable in Draft only.
        if loop_.status != GoalLoopStatus::Draft {
            return Err(ApiError(Error::Invalid(
                "config can only be edited while the loop is a draft".into(),
            )));
        }
        ctx.goal_loops_repo
            .set_config(&id, &config)
            .await
            .map_err(ApiError)?;
    }
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn remove(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    let _loop = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    // Deleting history does not delete the retained working files.
    let _ = goal_loop::stop_loop(&ctx, &id).await;
    ctx.goal_loops_repo.delete(&id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Lifecycle -------------------------------------------------------------

async fn start(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<GoalLoop>> {
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if loop_.status != GoalLoopStatus::Draft {
        return Err(ApiError(Error::Invalid(
            "only a draft loop can be started; use resume to continue an existing loop".into(),
        )));
    }
    if loop_
        .summary
        .as_deref()
        .is_some_and(|s| s.contains("Work is ready for human verification"))
        && loop_
            .definition
            .acceptance_criteria
            .iter()
            .any(|c| c.verify_kind == "human" && loop_.ledger.verification(c).is_none())
    {
        return Err(ApiError(Error::Invalid(
            "verify the pending human criteria before resuming".into(),
        )));
    }
    if loop_.ledger.questions.iter().any(|q| q.answer.is_none()) {
        return Err(ApiError(Error::Invalid(
            "answer the pending loop questions before resuming".into(),
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
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
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
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
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
    if loop_
        .summary
        .as_deref()
        .is_some_and(|s| s.contains("Work is ready for human verification"))
        && loop_
            .definition
            .acceptance_criteria
            .iter()
            .any(|c| c.verify_kind == "human" && loop_.ledger.verification(c).is_none())
    {
        return Err(ApiError(Error::Invalid(
            "verify the pending human criteria before resuming".into(),
        )));
    }
    if loop_.ledger.questions.iter().any(|q| q.answer.is_none()) {
        return Err(ApiError(Error::Invalid(
            "answer the pending loop questions before resuming".into(),
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
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    let _ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    goal_loop::stop_loop(&ctx, &id).await.map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn retry(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    Path((id, idx, agent)): Path<(Id, u32, usize)>,
) -> ApiResult<StatusCode> {
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    let _ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    goal_loop::retry_executor(&ctx, &id, idx, agent)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::ACCEPTED)
}

fn validate_settings(config: &GoalLoopConfig, limits: &GoalLoopLimits) -> ApiResult<()> {
    if !matches!(config.mode.as_str(), "build" | "research") {
        return Err(ApiError(Error::Invalid(
            "mode must be build or research".into(),
        )));
    }
    if limits.max_cost_usd.is_some() {
        return Err(ApiError(Error::Invalid(
            "per-loop cost accounting is unavailable; use iteration and runtime limits".into(),
        )));
    }
    if limits.max_iterations == 0
        || limits.max_runtime_secs == 0
        || limits.per_phase_timeout_secs == 0
    {
        return Err(ApiError(Error::Invalid(
            "iteration, runtime and phase limits must be positive".into(),
        )));
    }
    Ok(())
}

async fn verify_criterion(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((id, criterion)): Path<(Id, String)>,
    Json(req): Json<VerifyGoalCriterionReq>,
) -> ApiResult<Json<GoalLoop>> {
    require_human_credential(auth.managed_session_id.as_deref(), auth.mcp_only)?;
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    ctx.goal_loops.require_idle(&id).map_err(ApiError)?;
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if !matches!(
        loop_.status,
        GoalLoopStatus::Paused | GoalLoopStatus::Blocked | GoalLoopStatus::Exhausted
    ) {
        return Err(ApiError(Error::Invalid(
            "human verification is available while paused, blocked or exhausted".into(),
        )));
    }
    if req.evidence.trim().is_empty() {
        return Err(ApiError(Error::Invalid(
            "record the evidence you verified".into(),
        )));
    }
    let c = loop_
        .definition
        .acceptance_criteria
        .iter()
        .find(|c| c.id == criterion && c.verify_kind == "human")
        .ok_or_else(|| ApiError(Error::NotFound("human criterion".into())))?;
    ctx.goal_loops_repo
        .edit_ledger(&id, |ledger| {
            ledger.verifications.retain(|v| v.criterion_id != c.id);
            ledger
                .verifications
                .push(otto_core::domain::GoalHumanVerification {
                    criterion_id: c.id.clone(),
                    criterion_revision: c.revision(),
                    verified_by: user.0.id.clone(),
                    evidence: req.evidence.trim().into(),
                    verified_at: chrono::Utc::now(),
                });
            Ok(())
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

async fn answer_question(
    State(ctx): State<ServerCtx>,
    Extension(user): Extension<AuthUser>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path((id, question)): Path<(Id, Id)>,
    Json(req): Json<AnswerGoalQuestionReq>,
) -> ApiResult<Json<GoalLoop>> {
    require_human_credential(auth.managed_session_id.as_deref(), auth.mcp_only)?;
    let operation = ctx.goal_loops.operation(&id);
    let _guard = operation.lock().await;
    ctx.goal_loops.require_idle(&id).map_err(ApiError)?;
    let loop_ = loop_for(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if loop_.status != GoalLoopStatus::Blocked || req.answer.trim().is_empty() {
        return Err(ApiError(Error::Invalid(
            "provide a nonempty answer while the loop is blocked".into(),
        )));
    }
    ctx.goal_loops_repo
        .edit_ledger(&id, |ledger| {
            let q = ledger
                .questions
                .iter_mut()
                .find(|q| q.id == question && q.answer.is_none())
                .ok_or_else(|| Error::NotFound("pending question".into()))?;
            q.answer = Some(req.answer.trim().into());
            q.answered_by = Some(user.0.id.clone());
            q.answered_at = Some(chrono::Utc::now());
            ledger.repeated_failures = 0;
            ledger.last_failure_signature.clear();
            Ok(())
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(ctx.goal_loops_repo.get(&id).await.map_err(ApiError)?))
}

fn require_human_credential(managed_session_id: Option<&str>, mcp_only: bool) -> ApiResult<()> {
    if managed_session_id.is_some() || mcp_only {
        return Err(ApiError(Error::Forbidden("a person must record goal verification or answer decisions; managed agent credentials cannot approve work".into())));
    }
    Ok(())
}

#[cfg(test)]
mod human_credential_tests {
    use super::*;
    #[test]
    fn managed_author_token_cannot_record_human_decisions() {
        assert!(require_human_credential(Some("author-session"), false).is_err());
        assert!(require_human_credential(Some("internal-session"), true).is_err());
        assert!(require_human_credential(None, true).is_err());
        assert!(require_human_credential(None, false).is_ok());
    }
}
