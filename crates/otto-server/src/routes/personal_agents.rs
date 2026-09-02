//! Personal Agents REST endpoints (+ agent rooms).
//!
//! Two-axis RBAC, same shape as scheduled tasks: `feature_guard` maps every
//! path here to `Feature::ScheduledTasks` (View for GET, Edit for writes) via
//! `policy.rs`; every handler *additionally* enforces the workspace-role axis
//! with `require_ws_role`. Flat by-id routes load the agent (or run / room)
//! first and check the role on its `workspace_id` — the IDOR guard.
//!
//! The report route serves a run's stored Markdown by **run id** (never an
//! arbitrary path) and canonicalizes against the personal reports root.
//!
//! Room messages: a post with a `session_id` whose session carries
//! `meta.personal_agent` is an **agent** post (membership-checked, 16 KB cap);
//! a post without one is a **user** post — the user may post into any room in a
//! workspace where they hold Editor. Both are persisted + broadcast
//! (`AgentRoomMessage`); there is no other agent-to-agent transport.

use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::Error;
use otto_state::{
    AgentRoom, AgentRoomMessage, AgentRoomsRepo, AgentSchedulePatch, NewAgentSchedule,
    NewPersonalAgent, NewRoomMessage, PersonalAgent, PersonalAgentPatch, PersonalAgentRun,
    PersonalAgentSchedule, PersonalAgentsRepo, SettingsRepo,
};

use crate::auth::{require_ws_role, CurrentUser};
use crate::cadence;
use crate::error::{ApiError, ApiResult};
use crate::personal_agents_engine;
use crate::state::ServerCtx;

/// Max bytes for one room message (agent AND user posts).
pub const MAX_ROOM_POST_BYTES: usize = 16 * 1024;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/personal-agents", get(list).post(create))
        .route(
            "/personal-agents/{id}",
            get(get_one).patch(update).delete(remove),
        )
        .route(
            "/personal-agents/{id}/schedules",
            get(list_schedules).post(create_schedule),
        )
        .route(
            "/personal-agents/schedules/{schedule_id}",
            axum::routing::patch(update_schedule).delete(delete_schedule),
        )
        .route("/personal-agents/{id}/run", post(run_now))
        .route("/personal-agents/{id}/runs", get(list_runs))
        .route("/personal-agents/runs/{run_id}/report", get(report))
        .route("/personal-agents/{id}/chat-session", post(chat_session))
        .route("/workspaces/{id}/agent-rooms", get(list_rooms).post(create_room))
        .route(
            "/agent-rooms/{id}",
            get(get_room).patch(update_room).delete(delete_room),
        )
        .route("/agent-rooms/{id}/members", post(add_member))
        .route(
            "/agent-rooms/{id}/members/{agent_id}",
            axum::routing::delete(remove_member),
        )
        .route(
            "/agent-rooms/{id}/messages",
            get(list_messages).post(post_message),
        )
}

fn agents(ctx: &ServerCtx) -> PersonalAgentsRepo {
    PersonalAgentsRepo::new(ctx.pool.clone())
}

fn rooms(ctx: &ServerCtx) -> AgentRoomsRepo {
    AgentRoomsRepo::new(ctx.pool.clone())
}

// --- Request bodies --------------------------------------------------------

#[derive(Deserialize)]
struct CreateAgentReq {
    name: String,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    soul_md: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    browser: Option<bool>,
    #[serde(default)]
    delivery: Option<Value>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize, Default)]
struct UpdateAgentReq {
    name: Option<String>,
    avatar: Option<String>,
    soul_md: Option<String>,
    provider: Option<String>,
    model: Option<String>,
    cwd: Option<String>,
    browser: Option<bool>,
    delivery: Option<Value>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
struct CreateScheduleReq {
    schedule: Value,
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    directive: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize, Default)]
struct UpdateScheduleReq {
    schedule: Option<Value>,
    timezone: Option<String>,
    directive: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize, Default)]
struct RunReq {
    #[serde(default)]
    schedule_id: Option<String>,
}

#[derive(Deserialize)]
struct RoomReq {
    name: String,
}

#[derive(Deserialize)]
struct MemberReq {
    agent_id: String,
}

#[derive(Deserialize)]
struct PostMessageReq {
    text: String,
    /// A personal-agent session posting via the room MCP tools; resolved to
    /// the agent through the session's `meta.personal_agent`.
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct MessagesQuery {
    #[serde(default)]
    after: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    /// Same session→agent resolution as posting: an agent read is
    /// membership-checked; a plain user read needs only Viewer.
    #[serde(default)]
    session_id: Option<String>,
}

// --- Validation ------------------------------------------------------------

/// Same slug rule as scheduled tasks — a non-empty custom provider is allowed.
fn check_provider(p: &str) -> Result<(), ApiError> {
    let p = p.trim();
    if p.is_empty() || p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid(format!("provider '{p}' is not a valid provider name"))))
    }
}

fn check_timezone(tz: &str) -> Result<(), ApiError> {
    let t = tz.trim();
    if t.is_empty() || t.parse::<chrono_tz::Tz>().is_ok() {
        Ok(())
    } else {
        Err(ApiError(Error::Invalid(format!("unknown timezone '{tz}'"))))
    }
}

fn check_delivery(d: &Value) -> Result<(), ApiError> {
    match d.get("type").and_then(Value::as_str).unwrap_or("none") {
        "none" | "slack" | "telegram" | "email" | "webhook" => Ok(()),
        other => Err(ApiError(Error::Invalid(format!(
            "delivery type must be none|slack|telegram|email|webhook (got '{other}')"
        )))),
    }
}

// --- Agent handlers --------------------------------------------------------

/// `GET /workspaces/{id}/personal-agents` — seeds the example presets (once per
/// workspace, disabled + fully editable) before the first listing.
async fn list(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<PersonalAgent>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    seed_presets(&ctx, &ws_id, &user).await;
    Ok(Json(agents(&ctx).list_by_workspace(&ws_id).await.map_err(ApiError)?))
}

/// `POST /workspaces/{id}/personal-agents`
async fn create(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateAgentReq>,
) -> ApiResult<Json<PersonalAgent>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError(Error::Invalid("name is required".into())));
    }
    let provider = req.provider.unwrap_or_default();
    check_provider(&provider)?;
    let delivery = req.delivery.unwrap_or_else(|| json!({"type":"none"}));
    check_delivery(&delivery)?;
    let agent = agents(&ctx)
        .create(NewPersonalAgent {
            avatar: req.avatar.unwrap_or_default(),
            soul_md: req.soul_md.unwrap_or_default(),
            provider: if provider.trim().is_empty() { "claude".into() } else { provider },
            model: req.model.unwrap_or_default(),
            cwd: req.cwd.unwrap_or_default(),
            browser: req.browser.unwrap_or(false),
            delivery,
            enabled: req.enabled,
            created_by: Some(user.id.clone()),
            ..NewPersonalAgent::defaults(ws_id, req.name.trim().to_string())
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(agent))
}

/// `GET /personal-agents/{id}`
async fn get_one(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<PersonalAgent>> {
    let agent = agents(&ctx).get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(agent))
}

/// `PATCH /personal-agents/{id}`
async fn update(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateAgentReq>,
) -> ApiResult<Json<PersonalAgent>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    if let Some(p) = req.provider.as_deref() {
        check_provider(p)?;
    }
    if let Some(d) = &req.delivery {
        check_delivery(d)?;
    }
    let updated = repo
        .update(
            &id,
            PersonalAgentPatch {
                name: req.name,
                avatar: req.avatar,
                soul_md: req.soul_md,
                provider: req.provider,
                model: req.model,
                cwd: req.cwd,
                browser: req.browser,
                delivery: req.delivery,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

/// `DELETE /personal-agents/{id}` — schedules, runs and room memberships
/// cascade; room messages remain (the transcript stays user-visible).
async fn remove(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    repo.delete(&id).await.map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

// --- Schedule handlers ------------------------------------------------------

/// `GET /personal-agents/{id}/schedules`
async fn list_schedules(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<PersonalAgentSchedule>>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo.list_schedules(&id).await.map_err(ApiError)?))
}

/// `POST /personal-agents/{id}/schedules`
async fn create_schedule(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateScheduleReq>,
) -> ApiResult<Json<PersonalAgentSchedule>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    cadence::validate(&req.schedule).map_err(ApiError)?;
    let timezone = req.timezone.unwrap_or_else(|| "UTC".into());
    check_timezone(&timezone)?;
    let schedule = repo
        .create_schedule(NewAgentSchedule {
            agent_id: id,
            schedule: req.schedule,
            timezone,
            directive: req.directive,
            enabled: req.enabled,
        })
        .await
        .map_err(ApiError)?;
    refresh_next_run(&repo, &schedule).await;
    repo.get_schedule(&schedule.id).await.map(Json).map_err(ApiError)
}

/// `PATCH /personal-agents/schedules/{schedule_id}`
async fn update_schedule(
    Path(schedule_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UpdateScheduleReq>,
) -> ApiResult<Json<PersonalAgentSchedule>> {
    let repo = agents(&ctx);
    let schedule = repo.get_schedule(&schedule_id).await.map_err(ApiError)?;
    let agent = repo.get(&schedule.agent_id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    if let Some(s) = &req.schedule {
        cadence::validate(s).map_err(ApiError)?;
    }
    if let Some(tz) = req.timezone.as_deref() {
        check_timezone(tz)?;
    }
    let cadence_changed = req.schedule.is_some() || req.timezone.is_some();
    let updated = repo
        .update_schedule(
            &schedule_id,
            AgentSchedulePatch {
                schedule: req.schedule,
                timezone: req.timezone,
                directive: req.directive,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(ApiError)?;
    if cadence_changed {
        refresh_next_run(&repo, &updated).await;
    }
    repo.get_schedule(&schedule_id).await.map(Json).map_err(ApiError)
}

/// `DELETE /personal-agents/schedules/{schedule_id}`
async fn delete_schedule(
    Path(schedule_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = agents(&ctx);
    let schedule = repo.get_schedule(&schedule_id).await.map_err(ApiError)?;
    let agent = repo.get(&schedule.agent_id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    repo.delete_schedule(&schedule_id).await.map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

/// Recompute `next_run_at` for display (best-effort; cursor untouched).
async fn refresh_next_run(repo: &PersonalAgentsRepo, schedule: &PersonalAgentSchedule) {
    let tz = cadence::task_tz(&schedule.timezone);
    let next = cadence::next_run(&schedule.schedule, chrono::Utc::now(), tz).map(|d| d.to_rfc3339());
    let _ = repo.set_schedule_runtime(&schedule.id, None, next.as_deref()).await;
}

// --- Run handlers -----------------------------------------------------------

/// `POST /personal-agents/{id}/run` — run now (manual; never moves a schedule
/// cursor). Optional `schedule_id` picks which directive to run.
async fn run_now(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<RunReq>>,
) -> ApiResult<Json<PersonalAgentRun>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    let schedule = match body.and_then(|b| b.0.schedule_id) {
        Some(sid) => {
            let s = repo.get_schedule(&sid).await.map_err(ApiError)?;
            if s.agent_id != agent.id {
                return Err(ApiError(Error::Invalid("schedule belongs to a different agent".into())));
            }
            Some(s)
        }
        None => None,
    };
    let run_id = personal_agents_engine::run_agent(&ctx, &agent, schedule.as_ref(), "manual")
        .await
        .map_err(ApiError)?;
    repo.get_run(&run_id).await.map(Json).map_err(ApiError)
}

/// `GET /personal-agents/{id}/runs`
async fn list_runs(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<PersonalAgentRun>>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo.list_runs(&id, 100).await.map_err(ApiError)?))
}

/// `GET /personal-agents/runs/{run_id}/report` — the stored Markdown report.
async fn report(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<axum::response::Response> {
    let run = agents(&ctx).get_run(&run_id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Viewer).await?;
    let rel = run
        .report_rel
        .ok_or_else(|| ApiError(Error::NotFound("no report for this run".into())))?;
    let root = ctx.data_dir.join("personal");
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

// --- Chat -------------------------------------------------------------------

/// `POST /personal-agents/{id}/chat-session` — return (creating if absent or
/// dead) the agent's SINGLE interactive chat session: kind Agent, pinned
/// provider / `meta.model` / persona cwd, tagged `meta.personal_agent`. The UI
/// attaches a terminal pane to the returned id.
async fn chat_session(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;

    // Reuse the pinned session if it still exists.
    if let Some(sid) = agent.chat_session_id.as_deref().filter(|s| !s.is_empty()) {
        if let Ok(session) = ctx.manager.get(&sid.to_string()).await {
            return Ok(Json(json!({"session_id": session.id, "created": false})));
        }
    }

    let cwd = personal_agents_engine::ensure_agent_workspace(&ctx, &agent)
        .await
        .map_err(ApiError)?;
    let mut meta = json!({
        "source": "personal_agent",
        "personal_agent": agent.id,
        "personal_agent_chat": true,
        "browser": agent.browser,
    });
    if !agent.model.trim().is_empty() {
        meta["model"] = json!(agent.model.trim());
    }
    let ws = ctx.workspaces.get(&agent.workspace_id).await.map_err(ApiError)?;
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(agent.provider.clone()),
        title: Some(format!("Chat: {}", agent.name)),
        cwd: Some(cwd),
        connection_id: None,
        model: None,
        meta: Some(meta),
    };
    let session = ctx
        .manager
        .create(&ws, &user.id, req, None)
        .await
        .map_err(ApiError)?;
    repo.set_chat_session(&agent.id, Some(&session.id)).await.map_err(ApiError)?;
    Ok(Json(json!({"session_id": session.id, "created": true})))
}

// --- Rooms ------------------------------------------------------------------

/// `GET /workspaces/{id}/agent-rooms` — each room with its member agent ids.
async fn list_rooms(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Value>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let repo = rooms(&ctx);
    let mut out = Vec::new();
    for room in repo.list_by_workspace(&ws_id).await.map_err(ApiError)? {
        let members = repo.list_members(&room.id).await.unwrap_or_default();
        out.push(json!({"room": room, "members": members}));
    }
    Ok(Json(out))
}

/// `POST /workspaces/{id}/agent-rooms`
async fn create_room(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RoomReq>,
) -> ApiResult<Json<AgentRoom>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError(Error::Invalid("name is required".into())));
    }
    rooms(&ctx)
        .create(&ws_id, req.name.trim(), Some(&user.id))
        .await
        .map(Json)
        .map_err(ApiError)
}

/// `GET /agent-rooms/{id}`
async fn get_room(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Viewer).await?;
    let members = repo.list_members(&id).await.map_err(ApiError)?;
    Ok(Json(json!({"room": room, "members": members})))
}

/// `PATCH /agent-rooms/{id}` (rename)
async fn update_room(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<RoomReq>,
) -> ApiResult<Json<AgentRoom>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Editor).await?;
    if req.name.trim().is_empty() {
        return Err(ApiError(Error::Invalid("name is required".into())));
    }
    repo.rename(&id, req.name.trim()).await.map(Json).map_err(ApiError)
}

/// `DELETE /agent-rooms/{id}`
async fn delete_room(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Editor).await?;
    repo.delete(&id).await.map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

/// `POST /agent-rooms/{id}/members` — add a personal agent (same workspace only).
async fn add_member(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<MemberReq>,
) -> ApiResult<Json<Value>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Editor).await?;
    let agent = agents(&ctx).get(&req.agent_id).await.map_err(ApiError)?;
    if agent.workspace_id != room.workspace_id {
        return Err(ApiError(Error::Invalid("agent belongs to a different workspace".into())));
    }
    repo.add_member(&id, &agent.id).await.map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

/// `DELETE /agent-rooms/{id}/members/{agent_id}`
async fn remove_member(
    Path((id, agent_id)): Path<(String, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Editor).await?;
    repo.remove_member(&id, &agent_id).await.map_err(ApiError)?;
    Ok(Json(json!({"ok": true})))
}

/// Resolve a `session_id` to the personal agent it runs as: the session must
/// exist, belong to the caller (root exempt), and carry `meta.personal_agent`.
/// This is the room-tool identity — an agent can only speak as itself.
async fn resolve_session_agent(
    ctx: &ServerCtx,
    user: &User,
    session_id: &str,
) -> Result<PersonalAgent, ApiError> {
    let session = ctx
        .manager
        .get(&session_id.to_string())
        .await
        .map_err(|_| ApiError(Error::Invalid("unknown session_id".into())))?;
    if !user.is_root && session.created_by != user.id {
        return Err(ApiError(Error::Forbidden("session belongs to another user".into())));
    }
    let agent_id = session
        .meta
        .get("personal_agent")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError(Error::Invalid("session is not a personal-agent session".into())))?;
    agents(ctx).get(agent_id).await.map_err(ApiError)
}

/// `GET /agent-rooms/{id}/messages?after=&limit=&session_id=`
async fn list_messages(
    Path(id): Path<String>,
    Query(q): Query<MessagesQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<AgentRoomMessage>>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Viewer).await?;
    // An agent read (over the room MCP tools) is membership-checked: an agent
    // may only read rooms it belongs to, even though its owner could see more.
    if let Some(sid) = q.session_id.as_deref().filter(|s| !s.is_empty()) {
        let agent = resolve_session_agent(&ctx, &user, sid).await?;
        if !repo.is_member(&id, &agent.id).await.map_err(ApiError)? {
            return Err(ApiError(Error::Forbidden("agent is not a member of this room".into())));
        }
    }
    repo.list_messages(&id, q.after.as_deref(), q.limit.unwrap_or(100))
        .await
        .map(Json)
        .map_err(ApiError)
}

/// `POST /agent-rooms/{id}/messages` — a user post (`author_kind=user`), or an
/// agent post when `session_id` names a personal-agent session
/// (membership-checked). Size-capped; persisted; broadcast.
async fn post_message(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<PostMessageReq>,
) -> ApiResult<Json<AgentRoomMessage>> {
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Editor).await?;
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("text is required".into())));
    }
    if text.len() > MAX_ROOM_POST_BYTES {
        return Err(ApiError(Error::Invalid(format!(
            "message exceeds the {MAX_ROOM_POST_BYTES}-byte room post cap"
        ))));
    }

    let (author_kind, author_id) = match req.session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(sid) => {
            let agent = resolve_session_agent(&ctx, &user, sid).await?;
            if agent.workspace_id != room.workspace_id {
                return Err(ApiError(Error::Forbidden("agent belongs to a different workspace".into())));
            }
            if !repo.is_member(&id, &agent.id).await.map_err(ApiError)? {
                return Err(ApiError(Error::Forbidden("agent is not a member of this room".into())));
            }
            ("agent".to_string(), agent.id)
        }
        None => ("user".to_string(), user.id.clone()),
    };

    let msg = repo
        .add_message(NewRoomMessage {
            room_id: id,
            author_kind,
            author_id,
            text,
        })
        .await
        .map_err(ApiError)?;
    let _ = ctx.events.send(Event::AgentRoomMessage {
        workspace_id: room.workspace_id.clone(),
        room_id: msg.room_id.clone(),
        message_id: msg.id.clone(),
        author_kind: msg.author_kind.clone(),
        author_id: msg.author_id.clone(),
    });
    Ok(Json(msg))
}

// --- Seed presets -----------------------------------------------------------

/// Seed the four example agents once per workspace (guarded by a settings
/// marker so deleting them doesn't resurrect them). All DISABLED, all normal
/// fully-editable rows; model empty = provider default.
async fn seed_presets(ctx: &ServerCtx, ws_id: &str, user: &User) {
    let settings = SettingsRepo::new(ctx.pool.clone());
    let marker = format!("personal_agents_seeded_{ws_id}");
    if matches!(settings.get(&marker).await, Ok(Some(_))) {
        return;
    }
    let repo = agents(ctx);
    match repo.list_by_workspace(ws_id).await {
        Ok(existing) if existing.is_empty() => {}
        _ => {
            let _ = settings.put(&marker, &json!(true)).await;
            return;
        }
    }
    for (agent, schedules) in preset_agents(ws_id, &user.id) {
        match repo.create(agent).await {
            Ok(created) => {
                for mut s in schedules {
                    s.agent_id = created.id.clone();
                    let _ = repo.create_schedule(s).await;
                }
            }
            Err(e) => tracing::warn!("personal agents: seed preset failed: {e}"),
        }
    }
    let _ = settings.put(&marker, &json!(true)).await;
}

/// The built-in example presets: (agent, its schedules). `agent_id` on each
/// schedule is filled in after the agent row is created.
fn preset_agents(ws_id: &str, user_id: &str) -> Vec<(NewPersonalAgent, Vec<NewAgentSchedule>)> {
    let base = |name: &str, avatar: &str, soul: &str| NewPersonalAgent {
        avatar: avatar.into(),
        soul_md: soul.into(),
        enabled: false,
        created_by: Some(user_id.to_string()),
        ..NewPersonalAgent::defaults(ws_id.to_string(), name.to_string())
    };
    let sched = |spec: Value, directive: &str| NewAgentSchedule {
        agent_id: String::new(), // filled after the agent row exists
        schedule: spec,
        timezone: "UTC".into(),
        directive: directive.into(),
        enabled: true,
    };
    vec![
        (
            base(
                "Personal Assistant",
                "🧑‍💼",
                "You are a calm, resourceful personal assistant. You keep track of what your \
                 user is working on, answer questions directly, and prepare concise briefings. \
                 You prefer short, actionable answers over long explanations.",
            ),
            vec![],
        ),
        (
            base(
                "Daily Recap",
                "📰",
                "You are a diligent chronicler. You summarize what happened — commits, sessions, \
                 tickets — into a crisp daily recap, and you watch for anything that needs \
                 attention between recaps. You never exaggerate; if nothing happened, say so.",
            ),
            vec![
                sched(
                    json!({"cadence": "daily", "at": "09:00"}),
                    "Produce the daily recap: summarize the last 24 hours of activity (commits, \
                     sessions, open work) into a short report with a highlights summary.",
                ),
                sched(
                    json!({"cadence": "interval", "every_min": 15}),
                    "Quick check: is there anything that needs the user's attention right now \
                     (failing runs, stuck sessions, urgent tickets)? If nothing needs attention, \
                     report exactly that in one line.",
                ),
            ],
        ),
        (
            base(
                "Casino Reviewer",
                "🎰",
                "You are a meticulous casino site reviewer working WITHOUT logging in. You browse \
                 public pages only, checking content, offers, and page health, and produce a \
                 structured review. You never attempt to authenticate or bypass anything.",
            ),
            vec![sched(
                json!({"cadence": "daily", "at": "08:00"}),
                "Review the configured casino sites' public pages: availability, visible offers, \
                 and anything broken or changed since your last notes. Batch the findings into \
                 one structured report.",
            )],
        ),
        (
            base(
                "Casino Reviewer Player",
                "🃏",
                "You are a casino site reviewer who checks the logged-in player experience. \
                 IMPORTANT: login credentials are provided to you at runtime via the machine's \
                 Keychain-backed configuration — they are NEVER written in this persona, in \
                 prompts, in reports, or in room messages, and you must never echo them anywhere.",
            ),
            vec![sched(
                json!({"cadence": "daily", "at": "08:30"}),
                "Using the runtime-provided credentials (never printed), review the logged-in \
                 player experience: login flow health, balance/bonus visibility, and any \
                 regressions since your last notes. Report findings without including any \
                 credential material.",
            )],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_and_delivery_validators() {
        assert!(check_provider("claude").is_ok());
        assert!(check_provider("my-custom_1").is_ok());
        assert!(check_provider("").is_ok());
        assert!(check_provider("bad provider!").is_err());
        assert!(check_delivery(&json!({"type":"slack","chat_id":"C1"})).is_ok());
        assert!(check_delivery(&json!({})).is_ok()); // defaults to none
        assert!(check_delivery(&json!({"type":"carrier-pigeon"})).is_err());
        assert!(check_timezone("Europe/London").is_ok());
        assert!(check_timezone("Mars/Phobos").is_err());
    }

    #[test]
    fn presets_are_disabled_editable_rows() {
        let presets = preset_agents("ws1", "u1");
        assert_eq!(presets.len(), 4);
        assert!(presets.iter().all(|(a, _)| !a.enabled), "presets seed disabled");
        assert!(presets.iter().all(|(a, _)| a.model.is_empty()), "model empty = provider default");
        // Daily Recap carries TWO schedules (daily + 15-min needs-attention).
        let recap = presets.iter().find(|(a, _)| a.name == "Daily Recap").unwrap();
        assert_eq!(recap.1.len(), 2);
        assert_eq!(recap.1[0].schedule["cadence"], "daily");
        assert_eq!(recap.1[1].schedule["every_min"], 15);
        // The player reviewer's persona references Keychain, never credentials.
        let player = presets.iter().find(|(a, _)| a.name == "Casino Reviewer Player").unwrap();
        assert!(player.0.soul_md.contains("Keychain"));
        for (_, schedules) in &presets {
            for s in schedules {
                assert!(crate::cadence::validate(&s.schedule).is_ok());
            }
        }
    }

    #[test]
    fn room_post_cap_is_16k() {
        assert_eq!(MAX_ROOM_POST_BYTES, 16 * 1024);
    }
}
