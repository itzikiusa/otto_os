//! Axum router for swarm CRUD + board. Runtime endpoints (recruit, plan, run,
//! lifecycle, graph, board-ingest) live in otto-server where SessionManager and
//! the Coordinator are available. Paths are relative to `/api/v1`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;
use otto_state::{RunFilter, Swarm, SwarmAgent, SwarmMessage, SwarmProject, SwarmRun, SwarmTask};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::service::SwarmService;
use crate::types::*;

pub trait SwarmCtx: Clone + Send + Sync + 'static {
    fn swarm(&self) -> &Arc<SwarmService>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Providers (agent CLIs) currently installed/usable, for preset mapping and
    /// the recruiter. Best-effort; order = preference.
    fn available_providers(&self) -> Vec<String>;
    /// Product repository — used by the Swarm↔Product closure endpoint to read
    /// the Product story that originated a swarm task's project.  Returns `None`
    /// for host implementations that have no product layer.
    fn product_repo(&self) -> Option<&otto_state::ProductRepo> {
        None
    }
}

#[derive(Debug)]
pub(crate) struct ApiErr(pub Error);
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
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("internal error: {}", self.0);
        }
        (
            status,
            Json(otto_core::api::Problem {
                code: self.0.code().to_string(),
                message: self.0.to_string(),
            }),
        )
            .into_response()
    }
}
pub(crate) type ApiResult<T> = std::result::Result<T, ApiErr>;

pub fn router<S: SwarmCtx>() -> Router<S> {
    Router::new()
        // Swarms
        .route(
            "/workspaces/{id}/swarm/swarms",
            get(list_swarms::<S>).post(create_swarm::<S>),
        )
        .route(
            "/swarm/swarms/{sid}",
            // DELETE lives in `runtime::engine::routes`: it must halt the
            // swarm's live runs and sessions before the rows go.
            get(get_swarm::<S>).patch(update_swarm::<S>),
        )
        // Agents
        .route(
            "/swarm/swarms/{sid}/agents",
            get(list_agents::<S>).post(create_agent::<S>),
        )
        .route(
            "/swarm/agents/{aid}",
            axum::routing::patch(update_agent::<S>).delete(delete_agent::<S>),
        )
        // Projects
        .route(
            "/swarm/swarms/{sid}/projects",
            get(list_projects::<S>).post(create_project::<S>),
        )
        // DELETE: `runtime::engine::routes` (halts the project's runs first).
        .route(
            "/swarm/projects/{pid}",
            axum::routing::patch(update_project::<S>),
        )
        // Tasks
        .route(
            "/swarm/projects/{pid}/tasks",
            get(list_tasks::<S>).post(create_task::<S>),
        )
        // Every project's tasks in one read — the board's open path (perf §15 N5).
        .route("/swarm/swarms/{sid}/tasks", get(list_swarm_tasks::<S>))
        .route(
            "/swarm/tasks/{tid}",
            axum::routing::patch(update_task::<S>).delete(delete_task::<S>),
        )
        // Tasks — item GET + closure back-link to Product story
        .route("/swarm/tasks/{tid}/story", get(task_story_link::<S>))
        // Runs
        .route("/workspaces/{id}/swarm/runs", get(list_runs::<S>))
        .route("/swarm/runs/{rid}", get(get_run::<S>))
        // Board
        .route(
            "/swarm/swarms/{sid}/board",
            get(list_board::<S>).post(post_board::<S>),
        )
        // Graph
        .route("/swarm/swarms/{sid}/graph", get(graph::<S>))
        // Presets (static; no workspace scope needed)
        .route("/swarm/presets", get(list_presets::<S>))
}

async fn list_presets<S: SwarmCtx>(
    State(_s): State<S>,
    Extension(_user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<SwarmPreset>>> {
    Ok(Json(crate::presets::list_presets()))
}

// -- helpers ----------------------------------------------------------------

async fn check<S: SwarmCtx>(s: &S, user: &AuthUser, ws: &Id, role: WorkspaceRole) -> ApiResult<()> {
    s.roles().check(&user.0, ws, role).await?;
    Ok(())
}

// -- Swarms -----------------------------------------------------------------

async fn list_swarms<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
) -> ApiResult<Json<Vec<Swarm>>> {
    check(&s, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().list_swarms(&ws).await?))
}

async fn create_swarm<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Json(req): Json<CreateSwarmReq>,
) -> ApiResult<Json<SwarmDetail>> {
    check(&s, &user, &ws, WorkspaceRole::Editor).await?;
    // Resolve the default agent provider for this workspace up front: the
    // workspace's configured `default_provider`, else the first available
    // provider, else "claude". Drives both the swarm's default `config.provider`
    // and preset-agent instantiation. (The global default lives behind the
    // settings repo, which this crate can't reach; the server resolves that layer
    // for its own meta-agents.)
    let providers = s.available_providers();
    let ws_default = s
        .workspaces()
        .get(&ws)
        .await
        .ok()
        .map(|w| otto_core::provider::workspace_default(&w.settings).to_string())
        .unwrap_or_default();
    let default = otto_core::provider::resolve_provider(&[
        ws_default.as_str(),
        providers.first().map(String::as_str).unwrap_or(""),
    ]);
    // Preset instantiation (Phase 5) maps providers to installed CLIs; for a blank
    // create or when the preset is unknown we just create the swarm row.
    let preset = req.preset_slug.clone();
    let swarm = s
        .swarm()
        .create_swarm(&ws, &user.0.id, req, &default)
        .await?;
    if let Some(slug) = preset {
        // Best-effort: ignore unknown presets (blank swarm remains usable).
        let _ = crate::presets::instantiate(
            &s.swarm().repo,
            &swarm,
            &user.0.id,
            &slug,
            &providers,
            &default,
        )
        .await;
    }
    Ok(Json(s.swarm().detail(&swarm.id).await?))
}

async fn get_swarm<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<SwarmDetail>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().detail(&sid).await?))
}

async fn update_swarm<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<UpdateSwarmReq>,
) -> ApiResult<Json<Swarm>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    Ok(Json(s.swarm().update_swarm(&sid, req).await?))
}

// -- Agents -----------------------------------------------------------------

async fn list_agents<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmAgent>>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().list_agents(&sid).await?))
}

async fn create_agent<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<CreateAgentReq>,
) -> ApiResult<Json<SwarmAgent>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    Ok(Json(s.swarm().create_agent(&swarm, &user.0.id, req).await?))
}

async fn update_agent<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(aid): Path<Id>,
    Json(req): Json<UpdateAgentReq>,
) -> ApiResult<Json<SwarmAgent>> {
    let agent = s.swarm().get_agent(&aid).await?;
    check(&s, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    // PC-2: what an agent IS — its prompt (scope/specialization), soul,
    // provider, model and cwd — is the swarm creator's, a ws Admin's or
    // root's to change. Other fields (name, title, skills, schedule, status,
    // order, reporting line) keep the ws Editor rule. A full-form save that
    // leaves those fields as they are passes.
    let changed = identity_changes(&agent, &req);
    if !changed.is_empty() {
        let swarm = s.swarm().get_swarm(&agent.swarm_id).await?;
        let admin = s
            .roles()
            .check(&user.0, &agent.workspace_id, WorkspaceRole::Admin)
            .await
            .is_ok();
        if !(admin || swarm.created_by == user.0.id) {
            return Err(ApiErr(Error::Forbidden(format!(
                "only the swarm's creator or a workspace admin can change an agent's {}",
                changed.join(", ")
            ))));
        }
    }
    Ok(Json(s.swarm().update_agent(&aid, req).await?))
}

/// The identity fields `req` would change on `agent` (an absent field keeps
/// its value; an empty model / soul / cwd mode equals an unset one).
fn identity_changes(agent: &SwarmAgent, req: &UpdateAgentReq) -> Vec<&'static str> {
    fn norm(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    let differs = |new: Option<&String>, cur: Option<&str>| {
        new.is_some_and(|n| norm(Some(n.as_str())) != norm(cur))
    };
    let mut out = Vec::new();
    if differs(req.scope_md.as_ref(), Some(&agent.scope_md))
        || differs(req.specialization.as_ref(), Some(&agent.specialization))
    {
        out.push("prompt");
    }
    if differs(req.soul_md.as_ref(), agent.soul_md.as_deref())
        || differs(req.soul_name.as_ref(), agent.soul_name.as_deref())
    {
        out.push("soul");
    }
    if differs(req.provider.as_ref(), Some(&agent.provider)) {
        out.push("provider");
    }
    if differs(req.model.as_ref(), agent.model.as_deref()) {
        out.push("model");
    }
    if differs(req.cwd_mode.as_ref(), agent.cwd_mode.as_deref()) {
        out.push("cwd");
    }
    // access-control-5-02: a scheduled turn has no task, so it acts as the
    // swarm's creator — and its prompt IS the schedule's directive. Writing a
    // new directive is the creator's (or an Admin's) call, the same rule as a
    // personal agent's schedule (S3-05). Cadence, `at` and `enabled` stay
    // Editor-level; clearing the schedule or the directive adds no prompt.
    let directive = |s: Option<&serde_json::Value>| {
        s.and_then(|v| v.get("directive"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
    };
    if let Some(next) = &req.schedule {
        let new = directive(next.as_ref());
        if new.is_some() && new != directive(agent.schedule.as_ref()) {
            out.push("schedule directive");
        }
    }
    out
}

async fn delete_agent<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(aid): Path<Id>,
) -> ApiResult<StatusCode> {
    let agent = s.swarm().get_agent(&aid).await?;
    check(&s, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    s.swarm().delete_agent(&aid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// -- Projects ---------------------------------------------------------------

async fn list_projects<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmProject>>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().list_projects(&sid).await?))
}

async fn create_project<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<CreateProjectReq>,
) -> ApiResult<Json<SwarmProject>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    Ok(Json(
        s.swarm().create_project(&swarm, &user.0.id, req).await?,
    ))
}

async fn update_project<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
    Json(req): Json<UpdateProjectReq>,
) -> ApiResult<Json<SwarmProject>> {
    let project = s.swarm().get_project(&pid).await?;
    check(&s, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    Ok(Json(s.swarm().update_project(&pid, req).await?))
}

// -- Tasks ------------------------------------------------------------------

async fn list_tasks<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmTask>>> {
    let project = s.swarm().get_project(&pid).await?;
    check(&s, &user, &project.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().list_tasks(&pid).await?))
}

/// `GET /swarm/swarms/{sid}/tasks` — every task of the swarm (all projects),
/// ordered like the per-project list; the board groups them by `project_id`.
async fn list_swarm_tasks<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<Vec<SwarmTask>>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().list_tasks_for_swarm(&sid).await?))
}

async fn create_task<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
    Json(req): Json<CreateTaskReq>,
) -> ApiResult<Json<SwarmTask>> {
    let project = s.swarm().get_project(&pid).await?;
    check(&s, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    let task = s.swarm().create_task(&project, &user.0.id, req).await?;
    emit_task(&s, &task);
    Ok(Json(task))
}

async fn update_task<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
    Json(req): Json<UpdateTaskReq>,
) -> ApiResult<Json<SwarmTask>> {
    let task = s.swarm().get_task(&tid).await?;
    check(&s, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    let task = s.swarm().update_task(&tid, req, &user.0.id).await?;
    emit_task(&s, &task);
    Ok(Json(task))
}

async fn delete_task<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<StatusCode> {
    let task = s.swarm().get_task(&tid).await?;
    check(&s, &user, &task.workspace_id, WorkspaceRole::Editor).await?;
    s.swarm().delete_task(&tid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// -- Runs -------------------------------------------------------------------

#[derive(Deserialize)]
struct RunsQuery {
    swarm_id: Option<Id>,
    project_id: Option<Id>,
    agent_id: Option<Id>,
    status: Option<String>,
    /// `lite=true`: omit each run's `result` (except `kind='recruit'`); read
    /// one run's result via `GET /swarm/runs/{rid}`.
    #[serde(default)]
    lite: bool,
}

async fn list_runs<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Vec<SwarmRun>>> {
    check(&s, &user, &ws, WorkspaceRole::Viewer).await?;
    let f = RunFilter {
        // The role check above is for THIS workspace — the list must not
        // span every workspace's runs (session ids, summaries, cost).
        workspace_id: Some(ws),
        swarm_id: q.swarm_id,
        project_id: q.project_id,
        agent_id: q.agent_id,
        status: q.status,
    };
    if q.lite {
        return Ok(Json(s.swarm().list_runs_lite(&f).await?));
    }
    Ok(Json(s.swarm().list_runs(&f).await?))
}

async fn get_run<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(rid): Path<Id>,
) -> ApiResult<Json<SwarmRun>> {
    let run = s.swarm().get_run(&rid).await?;
    check(&s, &user, &run.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(run))
}

// -- Board ------------------------------------------------------------------

#[derive(Deserialize)]
struct BoardQuery {
    project_id: Option<Id>,
    task_id: Option<Id>,
}

async fn list_board<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Query(q): Query<BoardQuery>,
) -> ApiResult<Json<Vec<SwarmMessage>>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(
        s.swarm()
            .list_board(&sid, q.project_id.as_ref(), q.task_id.as_ref())
            .await?,
    ))
}

async fn post_board<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
    Json(req): Json<PostMessageReq>,
) -> ApiResult<Json<SwarmMessage>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    let msg = s
        .swarm()
        .post_human_message(&swarm, &user.0.id, req)
        .await?;
    let _ = s.events().send(Event::SwarmMessagePosted {
        workspace_id: swarm.workspace_id.clone(),
        swarm_id: swarm.id.clone(),
        message: serde_json::to_value(&msg).unwrap_or_default(),
    });
    Ok(Json(msg))
}

// -- Graph ------------------------------------------------------------------

async fn graph<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<Json<SwarmGraph>> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(s.swarm().graph(&sid).await?))
}

// -- Swarm↔Product closure  (GET /swarm/tasks/{tid}/story) ------------------

/// `GET /swarm/tasks/{tid}/story` — The Product story that originated this
/// swarm task, if any.  Traversal: task → project (via `project_id`) → story
/// (via `swarm_projects.story_id`) → product story row.
///
/// Read-only join; requires workspace Viewer. Returns `TaskStoryLink` with
/// `story: null` when no story is linked.  Always 200 — never 404 for a
/// "no story" case; callers can distinguish by the `story` field.
async fn task_story_link<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(tid): Path<Id>,
) -> ApiResult<Json<TaskStoryLink>> {
    let task = s.swarm().get_task(&tid).await?;
    check(&s, &user, &task.workspace_id, WorkspaceRole::Viewer).await?;

    let Some(product_repo) = s.product_repo() else {
        // Host application has no product layer; return empty link.
        return Ok(Json(TaskStoryLink {
            story: None,
            acceptance: None,
        }));
    };

    // Find the swarm project for this task, then look up its source story.
    let project = s.swarm().repo.get_project(&task.project_id).await.ok();
    let story_id = project.as_ref().and_then(|p| p.story_id.clone());
    let story = match story_id {
        Some(ref sid) => product_repo.get_story(sid).await.ok(),
        None => None,
    };

    // Acceptance criteria: the task description often holds structured ACs
    // (especially when generated from a plan); surface as a convenience field.
    let acceptance = if task.description.trim().is_empty() {
        None
    } else {
        Some(task.description.clone())
    };

    Ok(Json(TaskStoryLink { story, acceptance }))
}

// -- event helpers ----------------------------------------------------------

fn emit_task<S: SwarmCtx>(s: &S, task: &SwarmTask) {
    let _ = s.events().send(Event::SwarmTaskUpdated {
        workspace_id: task.workspace_id.clone(),
        swarm_id: task.swarm_id.clone(),
        project_id: task.project_id.clone(),
        task: serde_json::to_value(task).unwrap_or_default(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> SwarmAgent {
        SwarmAgent {
            id: "a".into(),
            swarm_id: "s".into(),
            workspace_id: "w".into(),
            name: "Dev".into(),
            title: "Developer".into(),
            reports_to: None,
            provider: "claude".into(),
            model: None,
            soul_name: None,
            soul_md: Some("calm".into()),
            specialization: "rust".into(),
            scope_md: "ship it".into(),
            skills: serde_json::json!([]),
            schedule: None,
            cwd_mode: Some("worktree".into()),
            avatar: String::new(),
            status: "active".into(),
            order_idx: 0,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn req(v: serde_json::Value) -> UpdateAgentReq {
        serde_json::from_value(v).unwrap()
    }

    /// PC-2: a full-form save that keeps the identity fields needs no more
    /// than Editor; changing any of them is flagged, by name.
    #[test]
    fn identity_changes_flags_only_real_identity_edits() {
        let a = agent();
        let same = req(serde_json::json!({
            "name": "Renamed", "title": "Lead", "provider": "claude", "model": "",
            "soul_md": "calm", "specialization": "rust", "scope_md": "ship it",
            "cwd_mode": "worktree", "status": "paused", "skills": [],
        }));
        assert!(identity_changes(&a, &same).is_empty());
        assert!(identity_changes(&a, &req(serde_json::json!({}))).is_empty());
        let all = req(serde_json::json!({
            "scope_md": "exfiltrate", "soul_md": "evil", "provider": "codex",
            "model": "o3", "cwd_mode": "repo",
        }));
        assert_eq!(
            identity_changes(&a, &all),
            ["prompt", "soul", "provider", "model", "cwd"]
        );
        assert_eq!(
            identity_changes(&a, &req(serde_json::json!({"specialization": "go"}))),
            ["prompt"]
        );
    }
}
