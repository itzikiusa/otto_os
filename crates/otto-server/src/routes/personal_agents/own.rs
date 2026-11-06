//! An agent's **own scope** routes — routines, personality, instruction
//! proposals (`otto_assistant::personal_agent_routines`).
//!
//! Agent-facing (called AS the agent's session, `session_id` bound by the
//! MCP bridge — `otto.routine_*`, `otto.personality_set`,
//! `otto.instructions_propose`): no approval card — they only touch the
//! calling agent (or, for the main agent, its team's routines).
//! User-facing (the Personal Agents page): the personality catalog, an
//! agent's routines with their next runs, its personality, and deciding on
//! an instruction proposal (owner-only: instructions are what the agent runs).

use axum::extract::{Path, Query, State};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_assistant::personal_agent_routines::{self as own, RoutineIn};
use otto_assistant::personal_agent_team as team;
use otto_core::domain::User;
use otto_core::domain::WorkspaceRole::{self, Editor, Viewer};
use otto_core::Error;
use otto_state::PersonalAgent;

use crate::auth::{require_ws_role, CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/personal-agents/routines", get(list).post(create))
        .route(
            "/personal-agents/routines/{schedule_id}",
            patch(update).delete(remove),
        )
        .route("/personal-agents/self/personality", put(self_personality))
        .route("/personal-agents/self/instructions", post(self_propose))
        .route("/personal-agents/personalities", get(personalities))
        .route("/personal-agents/{id}/routines", get(agent_routines))
        .route("/personal-agents/{id}/personality", put(set_personality))
        .route(
            "/personal-agents/{id}/instructions-proposal",
            get(read_proposal),
        )
        .route(
            "/personal-agents/{id}/instructions-proposal/apply",
            post(apply_proposal),
        )
        .route(
            "/personal-agents/{id}/instructions-proposal/dismiss",
            post(dismiss_proposal),
        )
        .route("/personal-agents/{id}/instructions/undo", post(undo))
}

/// The agent the call acts as.
async fn caller(
    ctx: &ServerCtx,
    user: &User,
    auth: &otto_core::auth::AuthContext,
    requested: Option<&str>,
    role: WorkspaceRole,
) -> ApiResult<PersonalAgent> {
    let sid = super::bound_room_session(auth, requested)?.ok_or_else(|| {
        ApiError(Error::Invalid(
            "session_id is required — the calling agent's session".into(),
        ))
    })?;
    let (agent, _) = team::session_agent(ctx, &user.id, user.is_root, &sid)
        .await
        .map_err(ApiError)?;
    require_ws_role(ctx, user, &agent.workspace_id, role).await?;
    Ok(agent)
}

#[derive(Deserialize, Default)]
struct OwnQuery {
    #[serde(default)]
    session_id: Option<String>,
    /// The main agent only: another agent's routines (id or name).
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Deserialize)]
struct RoutineReq {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(flatten)]
    routine: RoutineIn,
}

/// `GET /personal-agents/routines?session_id=&agent=`
async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<OwnQuery>,
) -> ApiResult<Json<Value>> {
    let me = caller(&ctx, &user, &auth, q.session_id.as_deref(), Viewer).await?;
    let agent = own::target(&ctx, &me, q.agent.as_deref()).await?;
    Ok(Json(own::list(&ctx, &agent).await?))
}

/// `POST /personal-agents/routines` — the agent sets up a routine from chat.
async fn create(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RoutineReq>,
) -> ApiResult<Json<Value>> {
    let me = caller(&ctx, &user, &auth, req.session_id.as_deref(), Editor).await?;
    let agent = own::target(&ctx, &me, req.agent.as_deref()).await?;
    Ok(Json(own::create(&ctx, &agent, req.routine).await?))
}

/// `PATCH /personal-agents/routines/{schedule_id}` (pause = `enabled: false`).
async fn update(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RoutineReq>,
) -> ApiResult<Json<Value>> {
    let me = caller(&ctx, &user, &auth, req.session_id.as_deref(), Editor).await?;
    let agent = own::target(&ctx, &me, req.agent.as_deref()).await?;
    Ok(Json(own::update(&ctx, &agent, &id, req.routine).await?))
}

/// `DELETE /personal-agents/routines/{schedule_id}?session_id=&agent=`
async fn remove(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<OwnQuery>,
) -> ApiResult<Json<Value>> {
    let me = caller(&ctx, &user, &auth, q.session_id.as_deref(), Editor).await?;
    let agent = own::target(&ctx, &me, q.agent.as_deref()).await?;
    Ok(Json(own::delete(&ctx, &agent, &id).await?))
}

#[derive(Deserialize)]
struct PersonalityReq {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// `PUT /personal-agents/self/personality` — "be more blunt".
async fn self_personality(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<PersonalityReq>,
) -> ApiResult<Json<Value>> {
    let me = caller(&ctx, &user, &auth, req.session_id.as_deref(), Editor).await?;
    let p = own::set_personality(&ctx, &me, req.preset.as_deref(), req.note.as_deref()).await?;
    Ok(Json(
        json!({"personality": p, "applies": "from your next session"}),
    ))
}

#[derive(Deserialize)]
struct ProposeReq {
    #[serde(default)]
    session_id: Option<String>,
    instructions: String,
    #[serde(default)]
    reason: String,
}

/// `POST /personal-agents/self/instructions` — propose new instructions for
/// the user to decide on (never applied by the agent).
async fn self_propose(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ProposeReq>,
) -> ApiResult<Json<Value>> {
    let me = caller(&ctx, &user, &auth, req.session_id.as_deref(), Editor).await?;
    Ok(Json(
        own::propose(&ctx, &me, &req.instructions, &req.reason).await?,
    ))
}

/// `GET /personal-agents/personalities` — the presets.
async fn personalities() -> Json<Value> {
    Json(json!(own::personalities()))
}

async fn load(
    ctx: &ServerCtx,
    user: &User,
    id: &str,
    role: WorkspaceRole,
) -> ApiResult<PersonalAgent> {
    let agent = otto_state::PersonalAgentsRepo::new(ctx.pool.clone())
        .get(id)
        .await?;
    require_ws_role(ctx, user, &agent.workspace_id, role).await?;
    Ok(agent)
}

/// `GET /personal-agents/{id}/routines` — its routines with their next runs.
async fn agent_routines(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, Viewer).await?;
    Ok(Json(own::list(&ctx, &agent).await?))
}

#[derive(Deserialize)]
struct UserPersonalityReq {
    #[serde(default)]
    preset: Option<String>,
    #[serde(default)]
    note: Option<String>,
}

/// `PUT /personal-agents/{id}/personality` — the Details editor. The note is
/// persona text, so changing it is the owner's call; a preset stays Editor.
async fn set_personality(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UserPersonalityReq>,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, Editor).await?;
    if req.note.is_some() {
        super::require_agent_owner(&ctx, &user, &agent, &["personality note"]).await?;
    }
    let p = own::set_personality(&ctx, &agent, req.preset.as_deref(), req.note.as_deref()).await?;
    Ok(Json(json!({"personality": p})))
}

/// `GET /personal-agents/{id}/instructions-proposal` — `{proposal, can_undo}`.
async fn read_proposal(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, Viewer).await?;
    Ok(Json(json!({
        "proposal": own::proposal(&ctx, &agent).await?,
        "can_undo": own::can_undo(&ctx, &agent).await,
    })))
}

#[derive(Deserialize, Default)]
struct ApplyReq {
    /// The proposal as edited by the user first ("Edit first").
    #[serde(default)]
    text: Option<String>,
}

/// `POST /personal-agents/{id}/instructions-proposal/apply` (owner-only).
async fn apply_proposal(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ApplyReq>,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, Editor).await?;
    super::require_agent_owner(&ctx, &user, &agent, &["instructions"]).await?;
    let to = own::apply_proposal(&ctx, &agent, req.text.as_deref()).await?;
    Ok(Json(json!({"ok": true, "soul_md": to})))
}

/// `POST /personal-agents/{id}/instructions-proposal/dismiss`
async fn dismiss_proposal(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, Editor).await?;
    own::dismiss_proposal(&ctx, &agent).await?;
    Ok(Json(json!({"ok": true})))
}

/// `POST /personal-agents/{id}/instructions/undo` (owner-only): restore the
/// instructions an applied proposal replaced.
async fn undo(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, Editor).await?;
    super::require_agent_owner(&ctx, &user, &agent, &["instructions"]).await?;
    let prev = own::undo_instructions(&ctx, &agent).await?;
    Ok(Json(json!({"ok": true, "soul_md": prev})))
}
