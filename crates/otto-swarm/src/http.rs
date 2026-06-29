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
use otto_state::{RunFilter, Swarm, SwarmAgent, SwarmMessage, SwarmProject, SwarmRun, SwarmTask};
use otto_state::WorkspacesRepo;
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
            get(get_swarm::<S>).patch(update_swarm::<S>).delete(delete_swarm::<S>),
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
        .route(
            "/swarm/projects/{pid}",
            axum::routing::patch(update_project::<S>).delete(delete_project::<S>),
        )
        // Tasks
        .route(
            "/swarm/projects/{pid}/tasks",
            get(list_tasks::<S>).post(create_task::<S>),
        )
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
    // Preset instantiation (Phase 5) maps providers to installed CLIs; for a blank
    // create or when the preset is unknown we just create the swarm row.
    let preset = req.preset_slug.clone();
    let swarm = s.swarm().create_swarm(&ws, &user.0.id, req).await?;
    if let Some(slug) = preset {
        let providers = s.available_providers();
        let default = providers.first().cloned().unwrap_or_else(|| "claude".into());
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

async fn delete_swarm<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(sid): Path<Id>,
) -> ApiResult<StatusCode> {
    let swarm = s.swarm().get_swarm(&sid).await?;
    check(&s, &user, &swarm.workspace_id, WorkspaceRole::Editor).await?;
    s.swarm().delete_swarm(&sid).await?;
    Ok(StatusCode::NO_CONTENT)
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
    Ok(Json(s.swarm().update_agent(&aid, req).await?))
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
    Ok(Json(s.swarm().create_project(&swarm, &user.0.id, req).await?))
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

async fn delete_project<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(pid): Path<Id>,
) -> ApiResult<StatusCode> {
    let project = s.swarm().get_project(&pid).await?;
    check(&s, &user, &project.workspace_id, WorkspaceRole::Editor).await?;
    s.swarm().delete_project(&pid).await?;
    Ok(StatusCode::NO_CONTENT)
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
    let task = s.swarm().update_task(&tid, req).await?;
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
}

async fn list_runs<S: SwarmCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Query(q): Query<RunsQuery>,
) -> ApiResult<Json<Vec<SwarmRun>>> {
    check(&s, &user, &ws, WorkspaceRole::Viewer).await?;
    let f = RunFilter {
        swarm_id: q.swarm_id,
        project_id: q.project_id,
        agent_id: q.agent_id,
        status: q.status,
    };
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
    let msg = s.swarm().post_human_message(&swarm, &user.0.id, req).await?;
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
        return Ok(Json(TaskStoryLink { story: None, acceptance: None }));
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
