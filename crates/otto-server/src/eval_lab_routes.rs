//! Eval-lab HTTP routes: per-repo **golden tasks** (CRUD + run) and **matrices**
//! (provider × skill × prompt comparison runs + their runner).
//!
//! Matrix cells reuse the regular eval engine via
//! [`crate::skill_eval::launch_eval`]; this module owns the fan-out and the
//! harvest of each cell's composite/proof back into the matrix view.

use std::collections::HashMap;

use axum::extract::{Path as AxPath, Query as AxQuery, State};
use axum::routing::{get, post};
use axum::{Json, Router};

use otto_core::api::{GoldenTaskReq, RunGoldenReq, SkillSourceReq, StartMatrixReq, StartSkillEvalReq};
use otto_core::domain::{EvalMatrix, GoldenTask, MatrixCell, WorkspaceRole};
use otto_core::{Error, Id};
use otto_state::GoldenTaskInput;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::skill_eval::launch_eval;
use crate::state::ServerCtx;

fn to_input(req: &GoldenTaskReq) -> GoldenTaskInput {
    GoldenTaskInput {
        name: req.name.trim().to_string(),
        prompt: req.prompt.clone(),
        skill: req.skill.clone(),
        test_cmd: req.test_cmd.clone(),
        lint_cmd: req.lint_cmd.clone(),
        build_cmd: req.build_cmd.clone(),
        rubric: req.rubric.clone(),
        tags: req.tags.clone(),
        enabled: req.enabled,
    }
}

// ---------------------------------------------------------------------------
// Golden tasks
// ---------------------------------------------------------------------------

async fn list_golden(
    AxPath(ws_id): AxPath<Id>,
    AxQuery(q): AxQuery<HashMap<String, String>>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<GoldenTask>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let tasks = ctx
        .golden_tasks_store
        .list(&ws_id, q.get("repo_key").map(|s| s.as_str()))
        .await
        .map_err(ApiError)?;
    Ok(Json(tasks))
}

async fn create_golden(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GoldenTaskReq>,
) -> ApiResult<Json<GoldenTask>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError(Error::Invalid("name is required".into())));
    }
    if req.prompt.trim().is_empty() {
        return Err(ApiError(Error::Invalid("prompt is required".into())));
    }
    let repo_key = req
        .repo_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| ws_id.clone());
    let task = ctx
        .golden_tasks_store
        .create(&ws_id, &repo_key, &to_input(&req), "manual", None, None, &user.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(task))
}

async fn get_golden(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<GoldenTask>> {
    let task = ctx.golden_tasks_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(task))
}

async fn update_golden(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<GoldenTaskReq>,
) -> ApiResult<Json<GoldenTask>> {
    let existing = ctx.golden_tasks_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    let task = ctx
        .golden_tasks_store
        .update(&id, &to_input(&req))
        .await
        .map_err(ApiError)?;
    Ok(Json(task))
}

async fn delete_golden(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::http::StatusCode> {
    let existing = ctx.golden_tasks_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &existing.workspace_id, WorkspaceRole::Editor).await?;
    ctx.golden_tasks_store.delete(&id).await.map_err(ApiError)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Build a StartSkillEvalReq from a golden task + run options.
fn golden_to_eval_req(g: &GoldenTask, run: &RunGoldenReq) -> StartSkillEvalReq {
    let mode = if run.mode.trim().is_empty() {
        "score_only".to_string()
    } else {
        run.mode.trim().to_string()
    };
    StartSkillEvalReq {
        source: SkillSourceReq {
            kind: "library".to_string(),
            reference: g.skill.clone(),
            provider: None,
        },
        task: g.prompt.clone(),
        impl_cli: run.provider.clone().unwrap_or_default(),
        validations: Vec::new(),
        iterations: 1,
        improver: None,
        base_ref: None,
        validator_passes: 1,
        mode,
        golden_task_id: Some(g.id.clone()),
        target: run.target.clone(),
        test_cmd: Some(g.test_cmd.clone()),
        lint_cmd: Some(g.lint_cmd.clone()),
        weights: None,
    }
}

async fn run_golden(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(run): Json<RunGoldenReq>,
) -> ApiResult<Json<otto_core::domain::SkillEval>> {
    let g = ctx.golden_tasks_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &g.workspace_id, WorkspaceRole::Editor).await?;
    let req = golden_to_eval_req(&g, &run);
    let eval = launch_eval(&ctx, &g.workspace_id, req, None)
        .await
        .map_err(ApiError)?;
    Ok(Json(eval))
}

// ---------------------------------------------------------------------------
// Matrices
// ---------------------------------------------------------------------------

/// Build a per-cell StartSkillEvalReq for one (provider, skill, prompt) triple.
fn cell_req(m: &StartMatrixReq, provider: &str, skill: &SkillSourceReq, prompt: &otto_core::domain::MatrixPrompt) -> StartSkillEvalReq {
    let mode = if m.mode.trim().is_empty() {
        "generate".to_string()
    } else {
        m.mode.trim().to_string()
    };
    StartSkillEvalReq {
        source: skill.clone(),
        task: prompt.task.clone(),
        impl_cli: provider.to_string(),
        validations: m.validations.clone(),
        iterations: m.iterations.max(1),
        improver: None,
        base_ref: m.base_ref.clone(),
        validator_passes: 1,
        mode,
        golden_task_id: prompt.golden_task_id.clone(),
        target: m.target.clone(),
        test_cmd: m.test_cmd.clone(),
        lint_cmd: m.lint_cmd.clone(),
        weights: m.weights.clone(),
    }
}

async fn list_matrices(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<EvalMatrix>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let matrices = ctx.eval_matrices_store.list(&ws_id).await.map_err(ApiError)?;
    Ok(Json(matrices))
}

async fn create_matrix(
    AxPath(ws_id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StartMatrixReq>,
) -> ApiResult<Json<EvalMatrix>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.providers.is_empty() || req.skills.is_empty() || req.prompts.is_empty() {
        return Err(ApiError(Error::Invalid(
            "a matrix needs at least one provider, skill, and prompt".into(),
        )));
    }
    let skill_names: Vec<String> = req.skills.iter().map(|s| s.reference.clone()).collect();
    let matrix = ctx
        .eval_matrices_store
        .create(
            &ws_id,
            req.name.trim(),
            if req.mode.trim().is_empty() { "generate" } else { req.mode.trim() },
            &ws_id,
            &req.providers,
            &skill_names,
            &req.prompts,
            &user.id,
        )
        .await
        .map_err(ApiError)?;

    // Fan out one cell run per (provider × skill × prompt). Each cell is a normal
    // eval carrying this matrix's id + the cell's dimensions; failures are logged
    // but don't abort the rest of the matrix.
    let matrix_id = matrix.id.clone();
    let total = req.providers.len() * req.skills.len() * req.prompts.len();
    for provider in &req.providers {
        for skill in &req.skills {
            for prompt in &req.prompts {
                let cell = cell_req(&req, provider, skill, prompt);
                let dims = (
                    matrix_id.clone(),
                    provider.clone(),
                    skill.reference.clone(),
                    prompt.label.clone(),
                );
                if let Err(e) = launch_eval(&ctx, &ws_id, cell, Some(dims)).await {
                    tracing::warn!(matrix = %matrix_id, "matrix cell failed to launch: {e}");
                }
            }
        }
    }
    tracing::info!(matrix = %matrix_id, %total, "matrix launched");
    Ok(Json(matrix))
}

/// Assemble a matrix view: header + each cell run's current composite/proof.
async fn matrix_view(ctx: &ServerCtx, mut matrix: EvalMatrix) -> EvalMatrix {
    let cells_runs = ctx
        .skill_evals_store
        .list_for_matrix(&matrix.id)
        .await
        .unwrap_or_default();
    let mut cells = Vec::with_capacity(cells_runs.len());
    let mut all_terminal = !cells_runs.is_empty();
    for ev in &cells_runs {
        let status = ev.status.as_str().to_string();
        if status == "running" {
            all_terminal = false;
        }
        let proof_status = ev
            .iterations
            .iter()
            .find_map(|i| i.scoring.as_ref().map(|s| s.proof_status.clone()))
            .unwrap_or_default();
        cells.push(MatrixCell {
            eval_id: ev.id.clone(),
            provider: ev.dim_provider.clone().unwrap_or_default(),
            skill: ev.dim_skill.clone().unwrap_or_default(),
            prompt: ev.dim_prompt.clone().unwrap_or_default(),
            status,
            composite_score: ev.composite_score,
            proof_status,
            best_iteration: ev.best_iteration,
        });
    }
    // Lazily settle the matrix status when all its cells are terminal.
    if all_terminal && matrix.status == "running" {
        let _ = ctx.eval_matrices_store.set_status(&matrix.id, "done").await;
        matrix.status = "done".to_string();
    }
    matrix.cells = cells;
    matrix
}

async fn get_matrix(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<EvalMatrix>> {
    let matrix = ctx.eval_matrices_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &matrix.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(matrix_view(&ctx, matrix).await))
}

async fn cancel_matrix(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<EvalMatrix>> {
    let matrix = ctx.eval_matrices_store.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &matrix.workspace_id, WorkspaceRole::Editor).await?;
    // Cancel each still-running cell run.
    let cells = ctx.skill_evals_store.list_for_matrix(&id).await.unwrap_or_default();
    for ev in &cells {
        if ev.status == otto_core::domain::SkillEvalStatus::Running {
            crate::skill_eval::cancel_run(&ctx, &ev.id).await;
        }
    }
    ctx.eval_matrices_store.set_status(&id, "cancelled").await.map_err(ApiError)?;
    let matrix = ctx.eval_matrices_store.get(&id).await.map_err(ApiError)?;
    Ok(Json(matrix_view(&ctx, matrix).await))
}

/// Routes under /api/v1 for the eval lab (golden tasks + matrices).
pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{id}/golden-tasks",
            get(list_golden).post(create_golden),
        )
        .route(
            "/golden-tasks/{id}",
            get(get_golden).put(update_golden).delete(delete_golden),
        )
        .route("/golden-tasks/{id}/run", post(run_golden))
        .route(
            "/workspaces/{id}/eval-matrices",
            get(list_matrices).post(create_matrix),
        )
        .route("/eval-matrices/{id}", get(get_matrix))
        .route("/eval-matrices/{id}/cancel", post(cancel_matrix))
}
