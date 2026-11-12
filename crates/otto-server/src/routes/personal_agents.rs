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

use otto_assistant::personal_agents_engine::{agent_ownership, AgentOwnership};
use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::Error;
use otto_state::{
    AgentRoom, AgentRoomMessage, AgentRoomsRepo, AgentSchedulePatch, NewPersonalAgent,
    NewRoomMessage, PersonalAgent, PersonalAgentPatch, PersonalAgentRun, PersonalAgentSchedule,
    PersonalAgentsRepo, SettingsRepo,
};

use crate::auth::{require_ws_role, CurrentAuthContext, CurrentUser};
use crate::cadence;
use crate::error::{ApiError, ApiResult};
use crate::personal_agents_engine;
use crate::state::ServerCtx;

/// Max bytes for one room message (agent AND user posts).
mod autonomy;
mod own;
mod team;

pub const MAX_ROOM_POST_BYTES: usize = 16 * 1024;
/// Max characters in a room name (it is shown in lists, headers and every
/// member agent's instructions).
pub const MAX_ROOM_NAME_CHARS: usize = 120;

/// Trimmed, non-empty, length-capped room name.
fn room_name(raw: &str) -> Result<&str, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError(Error::Invalid("name is required".into())));
    }
    if name.chars().count() > MAX_ROOM_NAME_CHARS {
        return Err(ApiError(Error::Invalid(format!(
            "room name is longer than {MAX_ROOM_NAME_CHARS} characters"
        ))));
    }
    Ok(name)
}

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
        .route(
            "/personal-agents/{id}/memory",
            get(read_memory).put(save_memory),
        )
        .route(
            "/personal-agents/{id}/context",
            get(read_context).put(save_context),
        )
        .route("/personal-agents/{id}/run", post(run_now))
        .route("/personal-agents/{id}/runs", get(list_runs))
        .route("/personal-agents/runs/{run_id}/report", get(report))
        .route("/personal-agents/runs/{run_id}/cancel", post(cancel_run))
        .route("/personal-agents/{id}/chat-session", post(chat_session))
        .route(
            "/workspaces/{id}/agent-rooms",
            get(list_rooms).post(create_room),
        )
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
        .merge(autonomy::routes())
        .merge(own::routes())
        .merge(team::routes())
}

fn agents(ctx: &ServerCtx) -> PersonalAgentsRepo {
    PersonalAgentsRepo::new(ctx.pool.clone())
}

fn rooms(ctx: &ServerCtx) -> AgentRoomsRepo {
    AgentRoomsRepo::new(ctx.pool.clone())
}

// --- Request bodies --------------------------------------------------------

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

#[derive(Deserialize, Default)]
struct UpdateScheduleReq {
    schedule: Option<Value>,
    timezone: Option<String>,
    directive: Option<String>,
    enabled: Option<bool>,
    permission: Option<String>,
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
    /// Page BACKWARDS: the `limit` messages before this id (oldest first).
    /// Ignored when `after` is set.
    #[serde(default)]
    before: Option<String>,
    /// `tail=true` with no cursor: the room's newest `limit` messages.
    #[serde(default)]
    tail: Option<bool>,
    #[serde(default)]
    limit: Option<i64>,
    /// Same session→agent resolution as posting: an agent read is
    /// membership-checked; a plain user read needs only Viewer.
    #[serde(default)]
    session_id: Option<String>,
}

// Validation lives in `otto_assistant::personal_agent_admin` (shared with
// the governed agent-admin tools).
use otto_assistant::personal_agent_admin::{
    check_delivery, check_permission, check_provider, check_timezone,
};

// --- Owner-only changes (S3-05 for personal agents) ------------------------

/// The PATCH fields that change WHAT the agent runs or WHERE its report goes
/// and actually differ from the stored agent (a full-form PATCH re-sending
/// unchanged values is not a change). Same rule as
/// `scheduled_tasks::owner_only_changes`.
fn owner_only_changes(agent: &PersonalAgent, req: &UpdateAgentReq) -> Vec<&'static str> {
    let mut out = Vec::new();
    let diff = |new: Option<&str>, old: &str| new.is_some_and(|n| n != old);
    if diff(req.soul_md.as_deref(), &agent.soul_md) {
        out.push("instructions");
    }
    if diff(req.provider.as_deref(), &agent.provider) {
        out.push("provider");
    }
    if diff(req.model.as_deref(), &agent.model) {
        out.push("model");
    }
    if diff(req.cwd.as_deref(), &agent.cwd) {
        out.push("folder");
    }
    if req.browser.is_some_and(|b| b != agent.browser) {
        out.push("browser access");
    }
    if req.delivery.as_ref().is_some_and(|d| d != &agent.delivery) {
        out.push("delivery");
    }
    out
}

/// A personal agent runs AS `created_by` (its session, git / Jira accounts,
/// connection grants and delivery), so what it runs and where its report goes
/// is the owner's call: another Editor changing them would make A's agent run
/// B's instructions as A and send the result to B (S3-05). The owner, a
/// workspace Admin or root may (`roles.check` passes root). An agent with no
/// owner (a seeded preset) is adopted by the first Editor who changes one of
/// these fields — it then runs as them, never as somebody else. An agent's
/// own credential acts AS its owner, so it is refused outright.
async fn require_agent_owner(
    ctx: &ServerCtx,
    auth: &otto_core::auth::AuthContext,
    agent: &PersonalAgent,
    changed: &[&str],
) -> Result<(), ApiError> {
    personal_agents_engine::person_only_change(crate::ui_bridge::is_human(auth), changed)?;
    let user = &auth.effective_user;
    let ownership = agent_ownership(agent, &user.id);
    if changed.is_empty() || ownership == AgentOwnership::Owner {
        return Ok(());
    }
    if require_ws_role(ctx, user, &agent.workspace_id, WorkspaceRole::Admin)
        .await
        .is_ok()
    {
        return Ok(());
    }
    if ownership == AgentOwnership::Ownerless && adopt(ctx, &agent.id, &user.id).await {
        return Ok(());
    }
    Err(ApiError(Error::Forbidden(format!(
        "This agent runs as its owner; only the owner or a workspace admin can change its {}.",
        changed.join(", ")
    ))))
}

/// Claim an ownerless agent for `user_id`. Conditional on the row still having
/// no owner, so of two Editors racing on a preset exactly one adopts it.
async fn adopt(ctx: &ServerCtx, agent_id: &str, user_id: &str) -> bool {
    sqlx::query(
        "UPDATE personal_agents SET created_by = ? \
         WHERE id = ? AND (created_by IS NULL OR created_by = '')",
    )
    .bind(user_id)
    .bind(agent_id)
    .execute(&ctx.pool)
    .await
    .is_ok_and(|r| r.rows_affected() == 1)
}

// --- Agent handlers --------------------------------------------------------

/// `GET /workspaces/{id}/personal-agents` — seeds the main agent (once per
/// workspace, active) before the first listing.
async fn list(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<PersonalAgent>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    seed_presets(&ctx, &ws_id, &user).await;
    Ok(Json(
        agents(&ctx)
            .list_by_workspace(&ws_id)
            .await
            .map_err(ApiError)?,
    ))
}

/// `POST /workspaces/{id}/personal-agents` — optionally from a `template`,
/// with its `schedules` and `primary` in the same call
/// (`otto_assistant::personal_agent_admin::create`).
async fn create(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<otto_assistant::personal_agent_admin::CreateReq>,
) -> ApiResult<Json<Value>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let made = otto_assistant::personal_agent_admin::create(&ctx, &ws_id, &user.id, req).await?;
    if let Some(id) = made["id"].as_str() {
        changed(&ctx, &ws_id, id, "agent");
    }
    Ok(Json(made))
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
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<UpdateAgentReq>,
) -> ApiResult<Json<PersonalAgent>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    // Name, avatar and enable/pause stay open to Editors; what runs and where
    // the report goes is the owner's (S3-05).
    require_agent_owner(&ctx, &auth, &agent, &owner_only_changes(&agent, &req)).await?;
    if let Some(p) = req.provider.as_deref() {
        check_provider(p)?;
    }
    if let Some(d) = &req.delivery {
        check_delivery(d)?;
    }
    if let Some(c) = req.cwd.as_deref() {
        personal_agents_engine::validate_agent_cwd(c).map_err(ApiError)?;
    }
    // Resuming the agent resumes all its schedules: re-arm them so what they
    // missed while it was paused is not fired (and delivered) at once.
    let resumed = !agent.enabled && req.enabled == Some(true);
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
    if resumed {
        let _ = repo
            .rearm_agent_schedules(&id, &chrono::Utc::now().to_rfc3339())
            .await;
        if let Ok(schedules) = repo.list_schedules(&id).await {
            for s in &schedules {
                refresh_next_run(&repo, s).await;
            }
        }
    }
    changed(&ctx, &updated.workspace_id, &id, "agent");
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
    // Stop a run still going first: the cascade removed its row, but the
    // engine still delivered its report and left its session running.
    let running = repo
        .list_runs(&id, 1)
        .await
        .map_err(ApiError)?
        .into_iter()
        .find(|r| r.status == "running");
    if let Some(run) = running {
        if !personal_agents_engine::cancel_run(&run.id) {
            return Err(ApiError(Error::Conflict(
                "the agent is finishing a run (saving and delivering its report) — delete it once \
                 that run has finished"
                    .into(),
            )));
        }
        let waiter = repo.clone();
        crate::api_helpers::wait_until_settled(|| {
            let repo = waiter.clone();
            let run_id = run.id.clone();
            async move {
                repo.get_run(&run_id)
                    .await
                    .map(|r| r.status != "running")
                    .unwrap_or(true)
            }
        })
        .await;
    }
    repo.delete(&id).await.map_err(ApiError)?;
    changed(&ctx, &agent.workspace_id, &id, "deleted");
    Ok(Json(json!({"ok": true})))
}

/// `personal_agent_changed` — every agent / routine write announces itself,
/// whichever path made it (the UI, an agent's MCP admin or team tools).
pub(crate) fn changed(ctx: &ServerCtx, ws: &str, agent_id: &str, what: &str) {
    personal_agents_engine::emit_changed(ctx, ws, agent_id, what);
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
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_assistant::personal_agent_admin::ScheduleIn>,
) -> ApiResult<Json<PersonalAgentSchedule>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    // A schedule's directive is the prompt its runs execute and its
    // permission what they may do — the owner's call. A cadence with the
    // default directive and permission stays open to Editors.
    let mut changed = Vec::new();
    if !req.directive.trim().is_empty() {
        changed.push("schedule directive");
    }
    if req.permission.as_deref().is_some_and(|p| p != "directed") {
        changed.push("schedule permission");
    }
    require_agent_owner(&ctx, &auth, &agent, &changed).await?;
    let made = otto_assistant::personal_agent_admin::add_schedule::<ServerCtx>(&repo, &id, req)
        .await
        .map_err(ApiError)?;
    if made.enabled
        && otto_assistant::personal_agent_admin::resume_for_routine(&repo, &agent)
            .await
            .map_err(ApiError)?
    {
        self::changed(&ctx, &agent.workspace_id, &id, "agent");
    }
    self::changed(&ctx, &agent.workspace_id, &id, "schedules");
    Ok(Json(made))
}

/// `PATCH /personal-agents/schedules/{schedule_id}`
async fn update_schedule(
    Path(schedule_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<UpdateScheduleReq>,
) -> ApiResult<Json<PersonalAgentSchedule>> {
    let repo = agents(&ctx);
    let schedule = repo.get_schedule(&schedule_id).await.map_err(ApiError)?;
    let agent = repo.get(&schedule.agent_id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    // Cadence, timezone and enabled stay open; directive and permission are
    // the owner's (S3-05).
    let differs = |new: Option<&str>, old: &str| new.is_some_and(|n| n != old);
    let mut changed = Vec::new();
    if differs(req.directive.as_deref(), &schedule.directive) {
        changed.push("schedule directive");
    }
    if differs(req.permission.as_deref(), &schedule.permission) {
        changed.push("schedule permission");
    }
    require_agent_owner(&ctx, &auth, &agent, &changed).await?;
    if let Some(s) = &req.schedule {
        cadence::validate(s).map_err(ApiError)?;
    }
    if let Some(tz) = req.timezone.as_deref() {
        check_timezone(tz)?;
    }
    check_permission(req.permission.as_deref())?;
    if let Some(p) = req.permission.as_deref() {
        repo.set_schedule_permission(&schedule_id, p)
            .await
            .map_err(ApiError)?;
    }
    let cadence_changed = req.schedule.is_some() || req.timezone.is_some();
    // Resumed, or a really new cadence/timezone → re-arm; a re-timed `once`
    // forgets it fired (the engine disables a spent once — editing its time
    // and turning it back on must make it fire again).
    let rearm = cadence::rearms(
        schedule.enabled,
        req.enabled,
        &schedule.schedule,
        req.schedule.as_ref(),
        &schedule.timezone,
        req.timezone.as_deref(),
    );
    let reset_once = cadence::rearms_once(&schedule.schedule, req.schedule.as_ref());
    let updated = repo
        .update_schedule(
            &schedule_id,
            AgentSchedulePatch {
                schedule: req.schedule,
                timezone: req.timezone,
                directive: req.directive,
                enabled: req.enabled,
                // One write with the edit, so no tick sees the new cadence
                // against the old cursor.
                rearm_at: (rearm || reset_once).then(|| chrono::Utc::now().to_rfc3339()),
                reset_once,
            },
        )
        .await
        .map_err(ApiError)?;
    if cadence_changed || rearm {
        refresh_next_run(&repo, &updated).await;
    }
    let out = repo.get_schedule(&schedule_id).await.map_err(ApiError)?;
    self::changed(&ctx, &agent.workspace_id, &agent.id, "schedules");
    Ok(Json(out))
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
    changed(&ctx, &agent.workspace_id, &agent.id, "schedules");
    Ok(Json(json!({"ok": true})))
}

/// Recompute `next_run_at` for display (best-effort; cursor untouched).
async fn refresh_next_run(repo: &PersonalAgentsRepo, schedule: &PersonalAgentSchedule) {
    otto_assistant::personal_agent_admin::refresh_next_run::<ServerCtx>(repo, schedule).await;
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
                return Err(ApiError(Error::Invalid(
                    "schedule belongs to a different agent".into(),
                )));
            }
            Some(s)
        }
        None => None,
    };
    // Background run: the `running` row comes back at once (completion arrives
    // as a run-updated event); 409 while one is already in progress.
    personal_agents_engine::spawn_agent_run(&ctx, &agent, schedule.as_ref(), "manual")
        .await
        .map(Json)
        .map_err(ApiError)
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
/// `POST /personal-agents/runs/{run_id}/cancel` — stop a running run (its
/// session is killed, no retry); it settles as `canceled`. 409 once finished.
async fn cancel_run(
    Path(run_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let run = agents(&ctx).get_run(&run_id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Editor).await?;
    if run.status != "running" || !personal_agents_engine::cancel_run(&run_id) {
        return Err(ApiError(Error::Conflict("the run is not running".into())));
    }
    Ok(Json(json!({"ok": true})))
}

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
        return Err(ApiError(Error::Forbidden(
            "report path escapes the reports root".into(),
        )));
    }
    let body = tokio::fs::read_to_string(&canon)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("read report: {e}"))))?;
    Ok((
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        body,
    )
        .into_response())
}

// --- User-editable documents ------------------------------------------------

#[derive(Deserialize)]
struct SaveDocumentReq {
    content: String,
    version: String,
}

async fn document_agent(
    ctx: &ServerCtx,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> ApiResult<PersonalAgent> {
    let agent = agents(ctx).get(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &agent.workspace_id, role).await?;
    Ok(agent)
}

async fn read_memory(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<crate::personal_agent_documents::AgentDocument>> {
    let agent = document_agent(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let root = personal_agents_engine::agent_directory(&ctx, &agent).map_err(ApiError)?;
    crate::personal_agent_documents::read_memory(&root)
        .await
        .map(Json)
        .map_err(ApiError)
}

async fn save_memory(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<SaveDocumentReq>,
) -> ApiResult<Json<crate::personal_agent_documents::AgentDocument>> {
    let agent = document_agent(&ctx, &auth.effective_user, &id, WorkspaceRole::Editor).await?;
    // Memory is injected into every run, which runs as the owner.
    require_agent_owner(&ctx, &auth, &agent, &["memory"]).await?;
    let root = personal_agents_engine::agent_directory(&ctx, &agent).map_err(ApiError)?;
    crate::personal_agent_documents::save_memory(&root, &req.version, &req.content)
        .await
        .map(Json)
        .map_err(ApiError)
}

async fn read_context(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<crate::personal_agent_documents::AgentDocument>> {
    document_agent(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let (content, version) = agents(&ctx).context(&id).await.map_err(ApiError)?;
    Ok(Json(crate::personal_agent_documents::AgentDocument {
        exists: version != "missing",
        content,
        version,
        path: None,
    }))
}

async fn save_context(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<SaveDocumentReq>,
) -> ApiResult<Json<crate::personal_agent_documents::AgentDocument>> {
    let agent = document_agent(&ctx, &auth.effective_user, &id, WorkspaceRole::Editor).await?;
    // The context is injected into every run, which runs as the owner.
    require_agent_owner(&ctx, &auth, &agent, &["context"]).await?;
    let version = agents(&ctx)
        .save_context(&id, &req.version, &req.content)
        .await
        .map_err(ApiError)?;
    Ok(Json(crate::personal_agent_documents::AgentDocument {
        exists: true,
        content: req.content,
        version,
        path: None,
    }))
}

// --- Chat -------------------------------------------------------------------

/// One async lock per agent for the chat get-or-create. The handler checks the
/// pinned session, creates one, submits context, THEN pins it — seconds apart.
/// Two overlapping calls (Chat → Runs → Chat during the first create) both saw
/// "no session" and spawned two Claude sessions, orphaning one that kept
/// burning a PTY and tokens. Serialised per agent, the second call waits and
/// then reuses the first one's pinned session.
// (Shared with the engine's chat feed, which opens the same chat to post a
// run's result into it.)
use otto_assistant::personal_agent_feed::CHAT_LOCKS;

/// `POST /personal-agents/{id}/chat-session` — return (creating if absent or
/// dead) the agent's SINGLE interactive chat: a Threads thread for a runtime
/// provider (`runtime: true` — the UI renders the Threads conversation), else
/// a terminal session (kind Agent, pinned provider / `meta.model` / persona
/// cwd, tagged `meta.personal_agent`) the UI attaches a terminal pane to.
async fn chat_session(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let repo = agents(&ctx);
    let agent = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &agent.workspace_id, WorkspaceRole::Editor).await?;
    let _guard = CHAT_LOCKS.lock(agent.id.as_str()).await;
    // Re-read under the lock: a concurrent call may have just pinned a chat.
    let agent = repo.get(&id).await.map_err(ApiError)?;
    let thread = otto_assistant::personal_agent_threads::chat_thread(&ctx, &agent, &user.id).await;
    if let Some(t) = thread.map_err(ApiError)? {
        return Ok(Json(
            json!({"session_id": t.session_id, "created": t.created, "runtime": true}),
        ));
    }

    // Reuse the pinned session if it still exists AND is the caller's: a chat
    // is opened under its creator, so another member's pinned chat is one
    // the caller can't attach to (and must not take over).
    let pinned = match agent.chat_session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(sid) => ctx.manager.get(&sid.to_string()).await.ok(),
        None => None,
    };
    if let Some(session) = pinned.as_ref().filter(|s| s.created_by == user.id) {
        return Ok(Json(json!({"session_id": session.id, "created": false})));
    }
    // Someone else's chat is pinned: reuse the caller's own live chat with
    // this agent, so chats stay one per user without a second pin column.
    if pinned.is_some() {
        let own = ctx
            .manager
            .list_live_by_meta(&agent.workspace_id, None, "personal_agent", &agent.id)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|s| {
                s.created_by == user.id
                    && s.meta.get("personal_agent_chat").and_then(Value::as_bool) == Some(true)
            })
            .max_by(|a, b| a.created_at.cmp(&b.created_at));
        if let Some(session) = own {
            return Ok(Json(json!({"session_id": session.id, "created": false})));
        }
    }

    // Snapshot once, before creating the session. Reusing an existing chat above
    // never injects or rewrites its context behind the user's back.
    let (user_context, context_version) = repo.context(&agent.id).await.map_err(ApiError)?;
    let cwd = personal_agents_engine::ensure_agent_workspace(&ctx, &agent)
        .await
        .map_err(ApiError)?;
    let mut meta = json!({
        "source": "personal_agent",
        "personal_agent": agent.id,
        "personal_agent_chat": true,
        "agent_mode": "directed",
        "personal_agent_context_version": context_version,
        "browser": agent.browser,
    });
    if !agent.model.trim().is_empty() {
        meta["model"] = json!(agent.model.trim());
    }
    let ws = ctx
        .workspaces
        .get(&agent.workspace_id)
        .await
        .map_err(ApiError)?;
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
    if !user_context.trim().is_empty() {
        let initial = personal_agents_engine::with_user_context(
            "Use this context for our conversation. Briefly acknowledge that you are ready, then wait for my request.",
            &user_context,
        );
        let _hold = ctx.manager.hold_for_turn(&session.id);
        if !otto_sessions::submit::submit_prompt(&ctx.manager, &session.id, &initial).await {
            otto_review::session::stop_review_sessions(
                &ctx.manager,
                std::slice::from_ref(&session.id),
            )
            .await;
            return Err(ApiError(Error::Upstream(
                "The chat session did not accept its context. Try opening it again.".into(),
            )));
        }
    }
    // Pin only into a free slot: another member's live pin stays theirs.
    if pinned.is_none() {
        repo.set_chat_session(&agent.id, Some(&session.id))
            .await
            .map_err(ApiError)?;
    }
    Ok(Json(json!({"session_id": session.id, "created": true})))
}

// --- Rooms ------------------------------------------------------------------

/// `GET /workspaces/{id}/agent-rooms` — each room with its member agent ids
/// and its activity (`message_count`, `last_message_at`). Three queries for
/// the whole list (rooms, members, activity), not one per room; the activity
/// is denormalized on `agent_rooms`, so no query scans the messages.
async fn list_rooms(
    Path(ws_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Value>>> {
    require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;
    let repo = rooms(&ctx);
    let list = repo.list_by_workspace(&ws_id).await.map_err(ApiError)?;
    let mut members = repo.members_by_workspace(&ws_id).await.map_err(ApiError)?;
    let activity = repo.activity_by_workspace(&ws_id).await.map_err(ApiError)?;
    let out = list
        .into_iter()
        .map(|room| {
            let act = activity.get(&room.id).cloned().unwrap_or_default();
            json!({
                "members": members.remove(&room.id).unwrap_or_default(),
                "message_count": act.message_count,
                "last_message_at": act.last_message_at,
                "room": room,
            })
        })
        .collect();
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
    let name = room_name(&req.name)?;
    rooms(&ctx)
        .create(&ws_id, name, Some(&user.id))
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
    let name = room_name(&req.name)?;
    repo.rename(&id, name).await.map(Json).map_err(ApiError)
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
        return Err(ApiError(Error::Invalid(
            "agent belongs to a different workspace".into(),
        )));
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

/// Resolve a `session_id` (a terminal session or a Threads thread) to the
/// personal agent it runs as: it must belong to the caller (root exempt) and
/// carry `meta.personal_agent`. The room / team tools' identity — an agent
/// can only speak as itself.
async fn resolve_session_agent(
    ctx: &ServerCtx,
    user: &User,
    session_id: &str,
) -> Result<PersonalAgent, ApiError> {
    otto_assistant::personal_agent_team::session_agent(ctx, &user.id, user.is_root, session_id)
        .await
        .map(|(agent, _)| agent)
        .map_err(ApiError)
}

/// The session a room read/post acts as. An agent-session credential (an
/// Otto-issued API token or a per-session MCP token) is bound to ITS session:
/// it cannot omit `session_id` to read without the membership check or post
/// as the human, and cannot name another session. A person's own credential
/// keeps the request's `session_id` (none = the user).
fn bound_room_session(
    auth: &otto_core::auth::AuthContext,
    requested: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let requested = requested.filter(|s| !s.is_empty());
    match (crate::ui_bridge::calling_session(auth), requested) {
        (Some(own), Some(other)) if own.as_str() != other => Err(ApiError(Error::Forbidden(
            "an agent session can only read or post in rooms as itself".into(),
        ))),
        (Some(own), _) => Ok(Some(own.clone())),
        (None, r) => Ok(r.map(str::to_string)),
    }
}

/// `GET /agent-rooms/{id}/messages?after=&limit=&session_id=`
async fn list_messages(
    Path(id): Path<String>,
    Query(mut q): Query<MessagesQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Vec<AgentRoomMessage>>> {
    q.session_id = bound_room_session(&auth, q.session_id.as_deref())?;
    let repo = rooms(&ctx);
    let room = repo.get(&id).await.map_err(ApiError)?;
    require_ws_role(&ctx, &user, &room.workspace_id, WorkspaceRole::Viewer).await?;
    // An agent read (over the room MCP tools) is membership-checked: an agent
    // may only read rooms it belongs to, even though its owner could see more.
    if let Some(sid) = q.session_id.as_deref().filter(|s| !s.is_empty()) {
        let agent = resolve_session_agent(&ctx, &user, sid).await?;
        if !repo.is_member(&id, &agent.id).await.map_err(ApiError)? {
            return Err(ApiError(Error::Forbidden(
                "agent is not a member of this room".into(),
            )));
        }
    }
    let limit = q.limit.unwrap_or(100);
    let backwards = q.after.is_none() && (q.before.is_some() || q.tail == Some(true));
    if backwards {
        return repo
            .list_messages_before(&id, q.before.as_deref(), limit)
            .await
            .map(Json)
            .map_err(ApiError);
    }
    repo.list_messages(&id, q.after.as_deref(), limit)
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
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(mut req): Json<PostMessageReq>,
) -> ApiResult<Json<AgentRoomMessage>> {
    req.session_id = bound_room_session(&auth, req.session_id.as_deref())?;
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
                return Err(ApiError(Error::Forbidden(
                    "agent belongs to a different workspace".into(),
                )));
            }
            if !repo.is_member(&id, &agent.id).await.map_err(ApiError)? {
                return Err(ApiError(Error::Forbidden(
                    "agent is not a member of this room".into(),
                )));
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
        text: msg.text.clone(),
        created_at: msg.created_at.clone(),
    });
    Ok(Json(msg))
}

// --- Seed presets -----------------------------------------------------------

/// Seed the workspace's main agent once (guarded by a settings marker so
/// deleting it doesn't resurrect it). It is ACTIVE: it is the agent the user
/// talks to, and a routine set up from its chat must fire — a paused seed
/// silently swallowed "every weekday at 9" routines. It is owned by the
/// Editor whose listing seeded it (runs act as that person); a Viewer's
/// listing seeds it ownerless, adopted by the first Editor who changes it.
/// Other agents come from templates (`+ New agent`) or from the main agent.
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
    let owner = require_ws_role(ctx, user, &ws_id.to_string(), WorkspaceRole::Editor)
        .await
        .is_ok()
        .then(|| user.id.clone());
    match repo.create(main_agent_preset(ws_id, owner)).await {
        Ok(created) => {
            let _ = otto_assistant::personal_agent_admin::make_primary(&repo, &created).await;
        }
        Err(e) => tracing::warn!("personal agents: seed main agent failed: {e}"),
    }
    let _ = settings.put(&marker, &json!(true)).await;
}

/// The seeded main agent; model empty = provider default.
fn main_agent_preset(ws_id: &str, owner: Option<String>) -> NewPersonalAgent {
    NewPersonalAgent {
        avatar: "char:chief".into(),
        soul_md: otto_assistant::personal_agent_admin::MAIN_AGENT_PERSONA.into(),
        enabled: true,
        created_by: owner,
        ..NewPersonalAgent::defaults(ws_id.to_string(), "Personal Assistant".to_string())
    }
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
    fn owner_only_changes_ignore_unchanged_and_open_fields() {
        let mut agent = main_agent_preset("ws1", None);
        agent.delivery = json!({"type":"none"});
        let stored = PersonalAgent {
            admission_generation: 0,
            id: "a1".into(),
            workspace_id: "ws1".into(),
            name: agent.name.clone(),
            avatar: agent.avatar.clone(),
            soul_md: agent.soul_md.clone(),
            provider: agent.provider.clone(),
            model: agent.model.clone(),
            cwd: agent.cwd.clone(),
            browser: agent.browser,
            delivery: agent.delivery.clone(),
            enabled: false,
            chat_session_id: None,
            created_by: Some("alice".into()),
            created_by_agent: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        // A full-form save re-sending unchanged values, plus name / enable.
        let same = UpdateAgentReq {
            name: Some("Renamed".into()),
            avatar: Some("x".into()),
            soul_md: Some(stored.soul_md.clone()),
            delivery: Some(stored.delivery.clone()),
            browser: Some(stored.browser),
            enabled: Some(true),
            ..Default::default()
        };
        assert!(owner_only_changes(&stored, &same).is_empty());
        let evil = UpdateAgentReq {
            soul_md: Some("leak".into()),
            delivery: Some(json!({"type":"webhook","url":"https://evil.test"})),
            ..Default::default()
        };
        assert_eq!(
            owner_only_changes(&stored, &evil),
            vec!["instructions", "delivery"]
        );
    }

    #[test]
    fn the_seeded_main_agent_is_active_and_owned_by_the_seeder() {
        let a = main_agent_preset("ws1", Some("u1".into()));
        assert!(a.enabled, "a paused main agent never fires its routines");
        assert_eq!(a.created_by.as_deref(), Some("u1"));
        assert!(a.model.is_empty(), "model empty = provider default");
        assert!(main_agent_preset("ws1", None).created_by.is_none());
    }

    #[test]
    fn agent_credentials_are_bound_to_their_own_room_session() {
        let user: User = serde_json::from_value(json!({
            "id": "u1", "username": "u", "display_name": "U", "is_root": false,
            "disabled": false, "created_at": "2026-10-03T00:00:00Z"
        }))
        .unwrap();
        let mut auth = otto_core::auth::AuthContext {
            real_user: user.clone(),
            effective_user: user,
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: None,
            device_id: None,
        };
        // A person: the request decides (none = post as the user).
        assert_eq!(bound_room_session(&auth, None).unwrap(), None);
        assert_eq!(
            bound_room_session(&auth, Some("s9")).unwrap().as_deref(),
            Some("s9")
        );
        // An Otto-issued session API token (managed only): never the user.
        auth.managed_session_id = Some("s1".into());
        assert_eq!(
            bound_room_session(&auth, None).unwrap().as_deref(),
            Some("s1")
        );
        assert_eq!(
            bound_room_session(&auth, Some("")).unwrap().as_deref(),
            Some("s1")
        );
        assert!(bound_room_session(&auth, Some("s2")).is_err());
        // A per-session MCP credential binds the same way.
        auth.managed_session_id = None;
        auth.mcp_session_id = Some("s3".into());
        assert_eq!(
            bound_room_session(&auth, Some("s3")).unwrap().as_deref(),
            Some("s3")
        );
        assert!(bound_room_session(&auth, Some("s1")).is_err());
    }

    #[test]
    fn room_post_cap_is_16k() {
        assert_eq!(MAX_ROOM_POST_BYTES, 16 * 1024);
    }

    #[test]
    fn room_names_are_trimmed_required_and_capped() {
        assert_eq!(room_name("  Standup  ").unwrap(), "Standup");
        assert!(room_name("   ").is_err());
        let max = "é".repeat(MAX_ROOM_NAME_CHARS);
        assert_eq!(room_name(&max).unwrap(), max, "the cap counts characters");
        assert!(room_name(&format!("{max}x")).is_err());
    }
}

#[cfg(test)]
mod chat_lock_tests {
    use super::*;

    /// S17-13: concurrent get-or-create calls for ONE agent share a lock (the
    /// second waits for the first to pin its session); other agents don't.
    #[tokio::test]
    async fn chat_lock_serialises_per_agent() {
        let held = CHAT_LOCKS.lock("agent-a").await;
        assert!(
            CHAT_LOCKS.try_lock("agent-a").is_none(),
            "a second chat create must wait"
        );
        assert!(
            CHAT_LOCKS.try_lock("agent-b").is_some(),
            "another agent is not blocked"
        );
        drop(held);
        assert!(CHAT_LOCKS.try_lock("agent-a").is_some());
    }
}
