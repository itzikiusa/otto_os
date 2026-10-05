//! Story analysis / rewrite / test-plan / plan-gen REST handlers: validate,
//! role-check, gate on the usage budget, persist the run row, then spawn the
//! matching [`crate::run`] runner in the background. Plus the per-agent
//! retry / stop controls and the test-case approval → skill-improvement hook.
//!
//! Mounted by the host (it needs the runners' [`ProductStudioHost`] hooks),
//! not by [`crate::router`].

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};

use crate::host::ProductStudioHost;
use crate::http::{ApiError, ApiResult, CurrentUser};

/// `GET /workspaces/{id}/product/lenses` — the curated analysis-lens catalog
/// the Analysis tab renders as configurable checks. Read-only: Viewer role.
pub async fn product_lenses<C: ProductStudioHost>(
    Path(ws_id): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<otto_core::api::ProductLens>>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(crate::analysis_lenses()))
}

/// `POST /workspaces/{id}/product/stories/{sid}/analyze` — create a
/// `ProductAnalysis` row and spawn the multi-agent fan-out in the background.
pub async fn analyze<C: ProductStudioHost>(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<crate::types::AnalyzeReq>,
) -> ApiResult<Json<otto_state::ProductAnalysis>> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    // Load story and verify it belongs to the requested workspace.
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Point-of-action budget gate (A2): check the workspace budget before
    // spawning any agent sessions. Mirrors the review start_review gate exactly.
    {
        let verdict = ctx
            .usage_budget(
                &ws_id, "", // provider resolved below; gate workspace-level cap here
            )
            .await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — analysis blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    // Resolve default provider (workspace → global → "claude"), mirroring the
    // `orchestrate` handler exactly.
    let ws = ctx.workspaces().get(&ws_id).await.map_err(ApiError)?;
    let global_default = otto_state::SettingsRepo::new(ctx.pool().clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);

    // Build the flat AgentSpec list: one entry per (lens × provider). Each lens
    // can be analyzed by multiple providers (claude/codex/agy), each as its own
    // real, openable session — exactly like a PR-review fan-out.
    let specs: Vec<crate::run::AgentSpec> = if !req.agents.is_empty() {
        req.agents
            .into_iter()
            .flat_map(|a| {
                let name = a.name.clone().unwrap_or_else(|| a.skill.clone());
                // An agent with no providers defaults to the default provider.
                let providers = if a.providers.is_empty() {
                    vec![default_provider.clone()]
                } else {
                    a.providers.clone()
                };
                let skill = a.skill.clone();
                let model = a.model.clone();
                providers
                    .into_iter()
                    .filter(|p| !p.trim().is_empty())
                    .map(move |provider| crate::run::AgentSpec {
                        provider,
                        model: model.clone(),
                        skill: skill.clone(),
                        name: name.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    } else {
        // Default three-lens set, each on the resolved default provider.
        [
            ("po-story-overview", "PO Overview"),
            ("story-architecture-overview", "Architecture"),
            ("story-clarifying-questions", "Clarifying Questions"),
        ]
        .iter()
        .map(|(skill, name)| crate::run::AgentSpec {
            provider: default_provider.clone(),
            model: None,
            skill: skill.to_string(),
            name: name.to_string(),
        })
        .collect()
    };

    // Summarizer provider: request override → default provider.
    let summarizer_provider = req
        .summarizer_provider
        .clone()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| default_provider.clone());

    // Resolve cwd: req → story.cwd → temp dir.
    let cwd = req
        .cwd
        .or_else(|| story.cwd.clone())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    // Latest source version id for the analysis row.
    let source_version_id = ctx
        .product_repo()
        .latest_source_version(&sid)
        .await
        .map_err(ApiError)?
        .map(|v| v.id);

    // Persist the analysis row (status = "running").
    let analysis = ctx
        .product_repo()
        .create_analysis(otto_state::NewAnalysis {
            story_id: sid.clone(),
            source_version_id,
            status: "running".to_string(),
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;

    // Spawn the fan-out; errors are isolated inside run_analysis. Each lens
    // (and the summarizer) runs as a real session on behalf of the current
    // user, mirroring the PR-review mechanism.
    tokio::spawn(crate::run::run_analysis(
        ctx.clone(),
        ws.clone(),
        user.id.clone(),
        sid.clone(),
        analysis.id.clone(),
        specs,
        summarizer_provider,
        cwd,
        req.focus,
    ));

    Ok(Json(analysis))
}

/// `POST /workspaces/{id}/product/stories/{sid}/rewrite` — spawn the writer
/// agent as a background task and return 202 Accepted immediately.
pub async fn rewrite<C: ProductStudioHost>(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<crate::types::RewriteReq>>,
) -> ApiResult<StatusCode> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    let req = body.map(|b| b.0).unwrap_or_default();

    // Load story and verify it belongs to this workspace.
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Point-of-action budget gate (A2): check workspace-level cap before spawning.
    {
        let verdict = ctx.usage_budget(&ws_id, "").await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — rewrite blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    // Resolve default provider (workspace → global → "claude"), mirroring analyze.
    let ws = ctx.workspaces().get(&ws_id).await.map_err(ApiError)?;
    let global_default = otto_state::SettingsRepo::new(ctx.pool().clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    let provider = req.provider.clone().unwrap_or(default_provider);

    // Resolve cwd: req → story.cwd → temp dir.
    let cwd = req
        .cwd
        .or_else(|| story.cwd.clone())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    // Spawn background task; errors are isolated inside run_rewrite.
    tokio::spawn(crate::run::run_rewrite(
        ctx.clone(),
        ws.clone(),
        user.id.clone(),
        sid,
        provider,
        req.model,
        cwd,
        req.focus,
    ));

    Ok(StatusCode::ACCEPTED)
}

/// `POST /workspaces/{id}/product/stories/{sid}/testcases/generate` — spawn
/// the test-case generation agent as a background task and return 202 Accepted.
pub async fn generate_tests<C: ProductStudioHost>(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<crate::types::GenerateTestsReq>>,
) -> ApiResult<StatusCode> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    let req = body.map(|b| b.0).unwrap_or_default();

    // Load story and verify it belongs to this workspace.
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Point-of-action budget gate (A2): check workspace-level cap before spawning.
    {
        let verdict = ctx.usage_budget(&ws_id, "").await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — test generation blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    // Resolve default provider (workspace → global → "claude"), mirroring rewrite.
    let ws = ctx.workspaces().get(&ws_id).await.map_err(ApiError)?;
    let global_default = otto_state::SettingsRepo::new(ctx.pool().clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    let provider = req.provider.clone().unwrap_or(default_provider);

    // Resolve cwd: req → story.cwd → temp dir.
    let cwd = req
        .cwd
        .or_else(|| story.cwd.clone())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    // Spawn background task; errors are isolated inside run_generate_tests.
    tokio::spawn(crate::run::run_generate_tests(
        ctx.clone(),
        ws.clone(),
        user.id.clone(),
        sid,
        provider,
        req.model,
        cwd,
        req.focus,
    ));

    Ok(StatusCode::ACCEPTED)
}

/// `POST /workspaces/{id}/product/stories/{sid}/plan/generate` — spawn the
/// task-breakdown agent as a background task and return 202 Accepted. Mirrors
/// `rewrite`/`generate_tests`.
pub async fn generate_plan<C: ProductStudioHost>(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<crate::types::GeneratePlanReq>>,
) -> ApiResult<StatusCode> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    let req = body.map(|b| b.0).unwrap_or_default();

    // Load story and verify it belongs to this workspace.
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Point-of-action budget gate (A2): check workspace-level cap before spawning.
    {
        let verdict = ctx.usage_budget(&ws_id, "").await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — plan generation blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }

    // Resolve default provider (workspace → global → "claude"), mirroring rewrite.
    let ws = ctx.workspaces().get(&ws_id).await.map_err(ApiError)?;
    let global_default = otto_state::SettingsRepo::new(ctx.pool().clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);

    // Resolve the planning provider list (multi-agent). Prefer `providers`
    // (non-empty); else the single back-compat `provider`; else the default.
    // Blanks are dropped, and an empty result falls back to the default provider.
    let providers: Vec<String> = {
        let mut list: Vec<String> = req
            .providers
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        if list.is_empty() {
            list = vec![req
                .provider
                .clone()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .unwrap_or_else(|| default_provider.clone())];
        }
        list
    };

    // Summarizer provider: request override → default provider.
    let summarizer_provider = req
        .summarizer_provider
        .clone()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| default_provider.clone());

    // Interactivity: `None` ⇒ non-interactive (the default). `Some(true)` only
    // when the UI explicitly turned the autonomy toggle OFF.
    let interactive = req.interactive.unwrap_or(false);

    // Resolve cwd: req → story.cwd → temp dir.
    let cwd = req
        .cwd
        .or_else(|| story.cwd.clone())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    // Spawn background task; errors are isolated inside run_generate_plan.
    tokio::spawn(crate::run::run_generate_plan(
        ctx.clone(),
        ws.clone(),
        user.id.clone(),
        sid,
        providers,
        summarizer_provider,
        interactive,
        req.model,
        cwd,
        req.focus,
    ));

    Ok(StatusCode::ACCEPTED)
}

/// `POST /workspaces/{id}/product/stories/{sid}/plan` — persist PO checkbox
/// toggles by overwriting the latest `kind="plan"` version's body in place (no
/// new version, so we never spam the version history). Returns 204 No Content.
pub async fn save_plan<C: ProductStudioHost>(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<crate::types::SavePlanReq>,
) -> ApiResult<StatusCode> {
    ctx.roles()
        .check(&user, &ws_id, WorkspaceRole::Editor)
        .await?;

    // Load story and verify it belongs to this workspace.
    let story = ctx.product_repo().get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Find the latest plan version and overwrite its body in place (preserving
    // its existing title — reuses the 3-arg update_version_body repo method).
    let plan = ctx
        .product_repo()
        .latest_plan_version(&sid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound("no plan version for story".into())))?;

    ctx.product_repo()
        .update_version_body(&plan.id, &plan.title, &req.body_md)
        .await
        .map_err(ApiError)?;

    Ok(StatusCode::NO_CONTENT)
}

/// `POST /product/testcase-runs/{rid}/approve` — approve all testcases in the
/// run and trigger a background self-improvement pass on the `story-test-cases`
/// skill using the PO's review outcomes as the learning signal.
///
/// Mounted by the host (it needs the `ImprovementEngine` via
/// [`ProductStudioHost::improve_engine`]); [`crate::router`] does not register
/// this route, avoiding an axum duplicate-route panic.
pub async fn approve_testcase_run<C: ProductStudioHost>(
    Path(rid): Path<Id>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<otto_state::ProductTestcaseRun>> {
    // Resolve workspace via run → story, then role-check.
    let run = ctx
        .product_repo()
        .get_testcase_run(&rid)
        .await
        .map_err(ApiError)?;
    let story = ctx
        .product_repo()
        .get_story(&run.story_id)
        .await
        .map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;

    // Flip all testcases in this run to "approved", and mark the run row approved
    // so clients can read the run's aggregate status (spec: approve "marks the run approved").
    ctx.product_repo()
        .approve_run_testcases(&rid)
        .await
        .map_err(ApiError)?;
    ctx.product_repo()
        .set_testcase_run(&rid, Some("approved"), None, None)
        .await
        .map_err(ApiError)?;

    // Fetch the cases (now approved) for the improvement narrative.
    let cases = ctx
        .product_repo()
        .list_testcases(&rid)
        .await
        .map_err(ApiError)?;

    // Build the narrative describing what the PO did with the test cases.
    let narrative = crate::run::build_improve_narrative_from_tests(&story, &cases);

    // Spawn background self-improvement — don't block the response.
    let engine = Arc::clone(ctx.improve_engine());
    let ws_id = story.workspace_id.clone();
    tokio::spawn(async move {
        if let Err(e) = engine
            .run_for_narrative(
                &ws_id,
                "test-cases",
                &narrative,
                &["story-test-cases".to_string()],
                otto_core::domain::ImprovementTrigger::Manual,
            )
            .await
        {
            tracing::warn!("test-case skill improvement failed: {e}");
        }
    });

    // Return the (now-approved) run row.
    let updated = ctx
        .product_repo()
        .get_testcase_run(&rid)
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

/// `POST /product/analyses/{aid}/agents/{agent_id}/retry` — re-run a single
/// failed or stuck analysis lens agent without re-running the full analysis.
///
/// Resolves the workspace via agent → analysis → story → workspace, performs
/// an Editor role check, then spawns `retry_analysis_agent` as a background
/// task and returns 202 Accepted immediately (exactly like a PR-review retry).
pub async fn retry_analysis_agent<C: ProductStudioHost>(
    Path((aid, agent_id)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    // Resolve workspace: agent → analysis → story → workspace, then role-check.
    let analysis = ctx
        .product_repo()
        .get_analysis(&aid)
        .await
        .map_err(ApiError)?;
    let story = ctx
        .product_repo()
        .get_story(&analysis.story_id)
        .await
        .map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;

    // Resolve workspace domain object (needed by run_lens_session → session create).
    let ws = ctx
        .workspaces()
        .get(&story.workspace_id)
        .await
        .map_err(ApiError)?;

    // Spawn background retry; errors are isolated inside retry_analysis_agent.
    tokio::spawn(crate::run::retry_analysis_agent(
        ctx.clone(),
        ws,
        user.id.clone(),
        aid,
        agent_id,
    ));

    Ok(StatusCode::ACCEPTED)
}

/// `POST /product/analyses/{aid}/agents/{agent_id}/stop` — stop a running/waiting
/// analysis agent on demand. Trips its cancel flag (so the recovery loop does NOT
/// treat the kill as a failure and retry), kills the live session, and marks the
/// agent errored ("stopped by user"). Idempotent.
pub async fn stop_analysis_agent<C: ProductStudioHost>(
    Path((aid, agent_id)): Path<(Id, Id)>,
    State(ctx): State<C>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let analysis = ctx
        .product_repo()
        .get_analysis(&aid)
        .await
        .map_err(ApiError)?;
    let story = ctx
        .product_repo()
        .get_story(&analysis.story_id)
        .await
        .map_err(ApiError)?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;

    // Signal the in-flight recovery loop FIRST so the kill below is seen as
    // intentional (no auto-retry).
    crate::run::signal_cancel(ctx.agent_cancels(), &agent_id);

    // Kill the current live session, if any.
    if let Ok(agent) = ctx.product_repo().get_analysis_agent(&agent_id).await {
        if let Some(sid) = agent.session_id.as_ref() {
            let _ = ctx.kill_session(sid).await;
        }
    }

    // Mark terminal so the UI reflects it immediately.
    let _ = ctx
        .product_repo()
        .set_agent_status(&agent_id, "error", None, Some("stopped by user"), true)
        .await;

    Ok(StatusCode::ACCEPTED)
}
