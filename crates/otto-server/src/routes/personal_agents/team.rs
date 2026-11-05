//! The **main agent's team** routes — what `otto.personal_agent_list` /
//! `_create` / `_delegate` call AS the calling agent session (an agent
//! credential is bound to its own session; a person names one with
//! `session_id`). The logic is `otto_assistant::personal_agent_team`; these
//! handlers resolve the caller and check its workspace role.

use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::Value;

use otto_assistant::personal_agent_team::{self as team, NewMember};
use otto_core::agent_runtime::RuntimeSession;
use otto_core::domain::{User, WorkspaceRole};
use otto_core::Error;
use otto_state::PersonalAgent;

use crate::auth::{require_ws_role, CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/personal-agents/team", get(list).post(create))
        .route("/personal-agents/team/delegate", post(delegate))
}

#[derive(Deserialize, Default)]
struct TeamQuery {
    #[serde(default)]
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct CreateReq {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(flatten)]
    member: NewMember,
}

#[derive(Deserialize)]
struct DelegateReq {
    #[serde(default)]
    session_id: Option<String>,
    /// The agent's id or name.
    agent: String,
    task: String,
}

/// The personal agent the call acts as (+ its thread, when it is one), with
/// `role` on its workspace.
async fn caller(
    ctx: &ServerCtx,
    user: &User,
    auth: &otto_core::auth::AuthContext,
    requested: Option<&str>,
    role: WorkspaceRole,
) -> ApiResult<(PersonalAgent, Option<RuntimeSession>)> {
    let sid = super::bound_room_session(auth, requested)?.ok_or_else(|| {
        ApiError(Error::Invalid(
            "session_id is required — the calling agent's session".into(),
        ))
    })?;
    let (agent, thread) = team::session_agent(ctx, &user.id, user.is_root, &sid)
        .await
        .map_err(ApiError)?;
    require_ws_role(ctx, user, &agent.workspace_id, role).await?;
    Ok((agent, thread))
}

/// `GET /personal-agents/team?session_id=` — the caller and its workspace's
/// other agents.
async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Query(q): Query<TeamQuery>,
) -> ApiResult<Json<Value>> {
    let (lead, _) = caller(
        &ctx,
        &user,
        &auth,
        q.session_id.as_deref(),
        WorkspaceRole::Viewer,
    )
    .await?;
    team::team(&ctx, &lead).await.map(Json).map_err(ApiError)
}

/// `POST /personal-agents/team` — the main agent creates an agent (owned by
/// the caller's user, attributed to the main agent).
async fn create(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<CreateReq>,
) -> ApiResult<Json<PersonalAgent>> {
    let (lead, _) = caller(
        &ctx,
        &user,
        &auth,
        req.session_id.as_deref(),
        WorkspaceRole::Editor,
    )
    .await?;
    team::create_member(&ctx, &lead, &user.id, req.member)
        .await
        .map(Json)
        .map_err(ApiError)
}

/// `POST /personal-agents/team/delegate` — the main agent's chat thread opens
/// `agent` as a linked thread with `task`; its report comes back as the
/// lead's next turn.
async fn delegate(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<DelegateReq>,
) -> ApiResult<Json<RuntimeSession>> {
    let (lead, thread) = caller(
        &ctx,
        &user,
        &auth,
        req.session_id.as_deref(),
        WorkspaceRole::Editor,
    )
    .await?;
    let thread = thread.ok_or_else(|| {
        ApiError(Error::Invalid(
            "delegation runs from the main agent's Threads chat — this is a terminal session"
                .into(),
        ))
    })?;
    team::delegate(&ctx, &lead, &thread, &req.agent, &req.task)
        .await
        .map(Json)
        .map_err(ApiError)
}
