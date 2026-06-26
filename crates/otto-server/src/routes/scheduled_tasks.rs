//! Scheduled Tasks REST endpoints.
//!
//! Two-axis RBAC: `feature_guard` enforces `Feature::ScheduledTasks` (View for GET,
//! Edit for writes) via `policy.rs`; every handler *additionally* enforces the
//! workspace-role axis with `require_ws_role`. Flat by-id routes load the task (or
//! run) first and check the role on its `workspace_id` — the IDOR guard, since the
//! feature axis is workspace-blind.
//!
//! The report route serves a run's stored Markdown by **run id** (never an
//! arbitrary path), and canonicalizes the resolved path against the scheduled
//! reports root before reading (path-traversal/symlink guard).

use axum::extract::{Path, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::domain::{ScheduledTask, ScheduledTaskPreset, ScheduledTaskRun, WorkspaceRole};
use otto_core::Error;
use otto_state::{NewScheduledTask, ScheduledTaskPatch};

use crate::auth::{require_ws_role, CurrentUser};
use crate::cadence;
use crate::error::{ApiError, ApiResult};
use crate::scheduled_tasks_engine;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{id}/scheduled-tasks",
            get(list).post(create),
        )
        .route("/scheduled-tasks/presets", get(presets))
        .route(
            "/scheduled-tasks/{id}",
            get(get_one).patch(update).delete(remove),
        )
        .route("/scheduled-tasks/{id}/run", post(run_now))
        .route("/scheduled-tasks/{id}/runs", get(list_runs))
        .route("/scheduled-tasks/runs/{run_id}/report", get(report))
}

// --- Request bodies --------------------------------------------------------

#[derive(Deserialize)]
struct CreateReq {
    name: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    skill: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    schedule: Option<Value>,
    #[serde(default)]
    destination: Option<Value>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Default)]
struct UpdateReq {
    name: Option<String>,
    prompt: Option<String>,
    // present key (even null) => set; absent => leave unchanged.
    #[serde(default, deserialize_with = "double_option")]
    skill: Option<Option<String>>,
    provider: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    schedule: Option<Value>,
    destination: Option<Value>,
    enabled: Option<bool>,
}

/// Distinguish "key absent" from "key present and null" for `skill`.
fn double_option<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(de)?))
}

// --- Validation ------------------------------------------------------------

/// v1 supports claude only; reject other providers explicitly (the engine drives
/// `Orchestrator::run_agent`, which is claude).
fn check_provider(p: &str) -> Result<(), ApiError> {
    if p.trim().is_empty() || p == "claude" {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid(format!(
            "provider '{p}' is not supported yet — scheduled tasks run with claude in this version"
        ))))
    }
}

fn validate_schedule(schedule: &Value) -> Result<(), ApiError> {
    cadence::validate(schedule).map_err(ApiError)
}

// --- Handlers --------------------------------------------------------------

/// `GET /workspaces/{id}/scheduled-tasks`
async fn list(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ScheduledTask>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.scheduled_tasks.list_by_workspace(&ws_id).await.map_err(ApiError)?))
}

/// `POST /workspaces/{id}/scheduled-tasks`
async fn create(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateReq>,
) -> ApiResult<Json<ScheduledTask>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError(Error::Invalid("name is required".into())));
    }
    let provider = req.provider.unwrap_or_else(|| "claude".into());
    check_provider(&provider)?;
    let schedule = req.schedule.unwrap_or_else(|| json!({"cadence":"interval","every_min":60}));
    validate_schedule(&schedule)?;
    let destination = req.destination.unwrap_or_else(|| json!({"type":"none"}));
    let next = cadence::next_run(&schedule, chrono::Utc::now()).map(|d| d.to_rfc3339());

    let task = ctx
        .scheduled_tasks
        .create(NewScheduledTask {
            workspace_id: ws_id.clone(),
            name: req.name.trim().to_string(),
            kind: req.kind.unwrap_or_else(|| "agent_prompt".into()),
            prompt: req.prompt,
            skill: req.skill.filter(|s| !s.is_empty()),
            provider,
            model: req.model.unwrap_or_default(),
            cwd: req.cwd.unwrap_or_default(),
            schedule,
            destination,
            enabled: req.enabled,
            created_by: Some(user.id.clone()),
        })
        .await
        .map_err(ApiError)?;
    // Set next_run_at for immediate display.
    let _ = ctx
        .scheduled_tasks
        .set_runtime(&task.id, None, task.last_status.as_deref().unwrap_or(""), next.as_deref())
        .await;
    ctx.scheduled_tasks.get(&task.id).await.map(Json).map_err(ApiError)
}

/// `GET /scheduled-tasks/{id}`
async fn get_one(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ScheduledTask>> {
    let task = ctx.scheduled_tasks.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(task))
}

/// `PATCH /scheduled-tasks/{id}`
async fn update(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateReq>,
) -> ApiResult<Json<ScheduledTask>> {
    let task = ctx.scheduled_tasks.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    if let Some(p) = req.provider.as_deref() {
        check_provider(p)?;
    }
    if let Some(s) = &req.schedule {
        validate_schedule(s)?;
    }
    let recompute_next = req.schedule.clone();
    let updated = ctx
        .scheduled_tasks
        .update(
            &id,
            ScheduledTaskPatch {
                name: req.name,
                prompt: req.prompt,
                skill: req.skill,
                provider: req.provider,
                model: req.model,
                cwd: req.cwd,
                schedule: req.schedule,
                destination: req.destination,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(ApiError)?;
    // If the cadence changed, refresh next_run_at for display.
    if recompute_next.is_some() {
        let next = cadence::next_run(&updated.schedule, chrono::Utc::now()).map(|d| d.to_rfc3339());
        let _ = ctx
            .scheduled_tasks
            .set_runtime(&id, None, updated.last_status.as_deref().unwrap_or(""), next.as_deref())
            .await;
    }
    ctx.scheduled_tasks.get(&id).await.map(Json).map_err(ApiError)
}

/// `DELETE /scheduled-tasks/{id}`
async fn remove(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let task = ctx.scheduled_tasks.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    ctx.scheduled_tasks.delete(&id).await.map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

/// `POST /scheduled-tasks/{id}/run` — run now (manual; does not move the cursor).
async fn run_now(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ScheduledTaskRun>> {
    let task = ctx.scheduled_tasks.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let run_id = scheduled_tasks_engine::run_task(&ctx, &task, "manual").await.map_err(ApiError)?;
    ctx.scheduled_tasks.get_run(&run_id).await.map(Json).map_err(ApiError)
}

/// `GET /scheduled-tasks/{id}/runs`
async fn list_runs(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ScheduledTaskRun>>> {
    let task = ctx.scheduled_tasks.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.scheduled_tasks.list_runs(&id, 100).await.map_err(ApiError)?))
}

/// `GET /scheduled-tasks/runs/{run_id}/report` — the stored Markdown report.
async fn report(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::response::Response> {
    let run = ctx.scheduled_tasks.get_run(&run_id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Viewer).await?;
    let rel = run
        .report_rel
        .ok_or_else(|| ApiError(Error::NotFound("no report for this run".into())))?;
    let root = ctx.data_dir.join("scheduled");
    let candidate = root.join(&rel);
    // Path-traversal/symlink guard: canonicalize BOTH and confirm containment.
    let canon_root = std::fs::canonicalize(&root)
        .map_err(|e| ApiError(Error::Internal(format!("reports root: {e}"))))?;
    let canon = std::fs::canonicalize(&candidate)
        .map_err(|_| ApiError(Error::NotFound("report file missing".into())))?;
    if !canon.starts_with(&canon_root) {
        return Err(ApiError(Error::Forbidden("report path escapes the reports root".into())));
    }
    let body = tokio::fs::read_to_string(&canon)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("read report: {e}"))))?;
    Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], body).into_response())
}

/// `GET /scheduled-tasks/presets` — built-in templates the UI offers.
async fn presets(
    State(_ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<Vec<ScheduledTaskPreset>>> {
    Ok(Json(builtin_presets()))
}

/// The built-in preset list. `ticket-followup-review` makes the motivating example
/// work out of the box (the agent uses the daemon's available Jira/Atlassian MCP
/// tools — see docs/features/scheduled-tasks.md for the prerequisite).
pub fn builtin_presets() -> Vec<ScheduledTaskPreset> {
    vec![ScheduledTaskPreset {
        id: "ticket-followup-review".into(),
        name: "Processed-ticket follow-up review".into(),
        description: "Hourly: re-analyze tickets updated in the last 24h, check for new \
                      post-triage comments, and produce a follow-up report."
            .into(),
        kind: "agent_prompt".into(),
        prompt: "Go over every ticket that was updated in the last 24 hours. Using the Jira/\
                 Atlassian tools available to you, re-analyze each one and check for new \
                 comments since it was last triaged. Produce a Markdown report titled \
                 \"Processed-ticket follow-up review\" whose summary lists: number Reviewed, \
                 number with New post-triage comments, any Improvements found (or \"No durable \
                 skill/memory improvements needed\"), and a Terminal/no-refetch count. Then, \
                 after a `---` rule, give the per-ticket details."
            .into(),
        schedule: json!({"cadence": "interval", "every_min": 60}),
        suggested_destination: json!({"type": "none"}),
        skill: None,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_check_allows_claude_and_blank_only() {
        assert!(check_provider("claude").is_ok());
        assert!(check_provider("").is_ok());
        assert!(check_provider("codex").is_err());
    }

    #[test]
    fn ticket_preset_is_present_and_shaped() {
        let p = builtin_presets();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, "ticket-followup-review");
        assert_eq!(p[0].schedule["every_min"], 60);
        assert!(p[0].prompt.contains("---"));
    }
}
