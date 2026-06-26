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
    // v2
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    workflow_id: Option<String>,
    #[serde(default)]
    sandbox: Option<String>,
    #[serde(default)]
    max_retries: Option<i64>,
    #[serde(default)]
    notify_on_change: Option<bool>,
    #[serde(default)]
    attach_proof: Option<bool>,
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
    // v2
    timezone: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    workflow_id: Option<Option<String>>,
    sandbox: Option<String>,
    max_retries: Option<i64>,
    notify_on_change: Option<bool>,
    attach_proof: Option<bool>,
}

/// Distinguish "key absent" from "key present and null" for `skill`/`workflow_id`.
fn double_option<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::<String>::deserialize(de)?))
}

// --- Validation ------------------------------------------------------------

/// Accepted agent providers plus `shell`; a non-empty custom slug is also allowed
/// (a user-registered provider). Only an empty-but-required provider is rejected.
fn check_provider(p: &str) -> Result<(), ApiError> {
    let p = p.trim();
    if p.is_empty() {
        return Ok(()); // resolved to the default ("claude") downstream
    }
    // Slug-shaped (the provider registry key); reject obviously bad input.
    if p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid(format!("provider '{p}' is not a valid provider name"))))
    }
}

fn check_sandbox(s: &str) -> Result<(), ApiError> {
    match s {
        "none" | "worktree" => Ok(()),
        other => Err(ApiError(Error::Invalid(format!("sandbox must be none|worktree (got '{other}')")))),
    }
}

fn check_kind(k: &str) -> Result<(), ApiError> {
    match k {
        "agent_prompt" | "workflow" => Ok(()),
        other => Err(ApiError(Error::Invalid(format!("kind must be agent_prompt|workflow (got '{other}')")))),
    }
}

fn check_retries(n: i64) -> Result<(), ApiError> {
    if (0..=5).contains(&n) {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid("max_retries must be 0..=5".into())))
    }
}

/// Validate a timezone string (empty == UTC). Rejects an unknown IANA name.
fn check_timezone(tz: &str) -> Result<(), ApiError> {
    let t = tz.trim();
    if t.is_empty() || t.parse::<chrono_tz::Tz>().is_ok() {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid(format!("unknown timezone '{tz}'"))))
    }
}

fn validate_schedule(schedule: &Value) -> Result<(), ApiError> {
    cadence::validate(schedule).map_err(ApiError)
}

/// A `kind=workflow` task must name a workflow that exists in the same workspace.
async fn validate_workflow(
    ctx: &ServerCtx,
    ws_id: &str,
    workflow_id: Option<&str>,
) -> Result<(), ApiError> {
    let wf_id = workflow_id
        .ok_or_else(|| ApiError(Error::Invalid("kind=workflow requires a workflow_id".into())))?;
    let wf = otto_state::WorkflowsRepo::new(ctx.pool.clone())
        .get(&wf_id.to_string())
        .await
        .map_err(|_| ApiError(Error::Invalid("workflow_id not found".into())))?;
    if wf.workspace_id != ws_id {
        return Err(ApiError(Error::Invalid("workflow belongs to a different workspace".into())));
    }
    Ok(())
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
    let kind = req.kind.unwrap_or_else(|| "agent_prompt".into());
    check_kind(&kind)?;
    let sandbox = req.sandbox.unwrap_or_else(|| "none".into());
    check_sandbox(&sandbox)?;
    let max_retries = req.max_retries.unwrap_or(0);
    check_retries(max_retries)?;
    let timezone = req.timezone.unwrap_or_else(|| "UTC".into());
    check_timezone(&timezone)?;
    let schedule = req.schedule.unwrap_or_else(|| json!({"cadence":"interval","every_min":60}));
    validate_schedule(&schedule)?;
    let workflow_id = req.workflow_id.filter(|s| !s.is_empty());
    if kind == "workflow" {
        validate_workflow(&ctx, &ws_id, workflow_id.as_deref()).await?;
    }
    let destination = req.destination.unwrap_or_else(|| json!({"type":"none"}));
    let tz = cadence::task_tz(&timezone);
    let next = cadence::next_run(&schedule, chrono::Utc::now(), tz).map(|d| d.to_rfc3339());

    let task = ctx
        .scheduled_tasks
        .create(NewScheduledTask {
            kind,
            prompt: req.prompt,
            skill: req.skill.filter(|s| !s.is_empty()),
            provider,
            model: req.model.unwrap_or_default(),
            cwd: req.cwd.unwrap_or_default(),
            schedule,
            destination,
            enabled: req.enabled,
            created_by: Some(user.id.clone()),
            timezone,
            workflow_id,
            sandbox,
            max_retries,
            notify_on_change: req.notify_on_change.unwrap_or(false),
            attach_proof: req.attach_proof.unwrap_or(false),
            ..NewScheduledTask::defaults(ws_id.clone(), req.name.trim().to_string())
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
    if let Some(s) = req.sandbox.as_deref() {
        check_sandbox(s)?;
    }
    if let Some(n) = req.max_retries {
        check_retries(n)?;
    }
    if let Some(tz) = req.timezone.as_deref() {
        check_timezone(tz)?;
    }
    // `kind` is fixed at create; if this is a workflow task, the (possibly new)
    // workflow_id must still name a workflow in this workspace.
    if task.kind == "workflow" {
        let wf = req
            .workflow_id
            .clone()
            .flatten()
            .or_else(|| task.workflow_id.clone());
        validate_workflow(&ctx, &task.workspace_id, wf.as_deref()).await?;
    }
    let recompute_next = req.schedule.clone();
    let tz_changed = req.timezone.is_some();
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
                timezone: req.timezone,
                workflow_id: req.workflow_id,
                sandbox: req.sandbox,
                max_retries: req.max_retries,
                notify_on_change: req.notify_on_change,
                attach_proof: req.attach_proof,
            },
        )
        .await
        .map_err(ApiError)?;
    // If the cadence/timezone changed, refresh next_run_at for display.
    if recompute_next.is_some() || tz_changed {
        let tz = cadence::task_tz(&updated.timezone);
        let next = cadence::next_run(&updated.schedule, chrono::Utc::now(), tz).map(|d| d.to_rfc3339());
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
    vec![
        ScheduledTaskPreset {
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
        },
        ScheduledTaskPreset {
            id: "weekly-security-scan".into(),
            name: "Weekly security scan".into(),
            description: "Weekly: run a security review of the repository in an isolated \
                          worktree and report findings. Notifies only when findings change."
                .into(),
            kind: "agent_prompt".into(),
            prompt: "Perform a security review of the code in this repository. Look for \
                     injection, authn/authz gaps, SSRF, secret handling, unsafe deserialization, \
                     and path-traversal issues. Produce a Markdown report titled \"Security scan\" \
                     whose summary states the count of High/Medium/Low findings (or \"No new \
                     findings\"), then after a `---` rule, list each finding with file:line, \
                     severity, and a suggested fix. Do not modify any files."
                .into(),
            schedule: json!({"cadence": "cron", "expr": "0 8 * * 1"}),
            suggested_destination: json!({"type": "none"}),
            skill: Some("security-review".into()),
        },
        ScheduledTaskPreset {
            id: "weekly-code-review".into(),
            name: "Weekly code-quality review".into(),
            description: "Weekly: review recent changes for quality/maintainability in an \
                          isolated worktree and report. Attach a proof pack."
                .into(),
            kind: "agent_prompt".into(),
            prompt: "Review the repository's recent changes (last 7 days of commits) for code \
                     quality: correctness risks, missing tests, error handling, and \
                     maintainability. Produce a Markdown report titled \"Code-quality review\" \
                     whose summary lists the top issues by area, then after a `---` rule give \
                     per-issue detail with file:line and a concrete suggestion. Do not modify \
                     files."
                .into(),
            schedule: json!({"cadence": "weekly", "at": "08:00", "weekday": 0}),
            suggested_destination: json!({"type": "none"}),
            skill: Some("code-review".into()),
        },
        ScheduledTaskPreset {
            id: "weekly-dependency-scan".into(),
            name: "Weekly dependency / PR scan".into(),
            description: "Weekly: check for outdated or vulnerable dependencies and stale open \
                          PRs, and report what needs attention."
                .into(),
            kind: "agent_prompt".into(),
            prompt: "Inspect this repository's dependency manifests and lockfiles for outdated or \
                     known-vulnerable dependencies, and list any open pull requests that look \
                     stale or unmerged. Produce a Markdown report titled \"Dependency & PR scan\" \
                     whose summary lists counts (outdated, vulnerable, stale PRs), then after a \
                     `---` rule give the specifics with suggested upgrades. Do not modify files."
                .into(),
            schedule: json!({"cadence": "cron", "expr": "0 9 * * 1"}),
            suggested_destination: json!({"type": "none"}),
            skill: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_check_allows_known_and_custom_slugs() {
        for p in ["claude", "codex", "agy", "shell", "", "my-custom-agent"] {
            assert!(check_provider(p).is_ok(), "provider '{p}' should be accepted");
        }
        assert!(check_provider("bad provider!").is_err());
    }

    #[test]
    fn field_validators() {
        assert!(check_sandbox("none").is_ok());
        assert!(check_sandbox("worktree").is_ok());
        assert!(check_sandbox("vm").is_err());
        assert!(check_kind("agent_prompt").is_ok());
        assert!(check_kind("workflow").is_ok());
        assert!(check_kind("magic").is_err());
        assert!(check_retries(0).is_ok());
        assert!(check_retries(5).is_ok());
        assert!(check_retries(6).is_err());
        assert!(check_timezone("UTC").is_ok());
        assert!(check_timezone("Europe/London").is_ok());
        assert!(check_timezone("").is_ok());
        assert!(check_timezone("Mars/Phobos").is_err());
    }

    #[test]
    fn presets_present_and_shaped() {
        let p = builtin_presets();
        assert!(p.iter().any(|x| x.id == "ticket-followup-review"));
        assert!(p.iter().any(|x| x.id == "weekly-security-scan"));
        assert!(p.iter().any(|x| x.id == "weekly-code-review"));
        assert!(p.iter().all(|x| x.prompt.contains("---")));
        // cron preset is valid.
        let sec = p.iter().find(|x| x.id == "weekly-security-scan").unwrap();
        assert!(cadence::validate(&sec.schedule).is_ok());
    }
}
