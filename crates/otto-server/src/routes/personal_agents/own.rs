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

/// A routine body, refused when an AGENT credential sets, changes or (`stored`:
/// the body with the stored destination filled in) edits one posting outside
/// the chat: that goes through `otto.routine_*`, whose approval card a person
/// decides and whose approved call replays as them. Same rule as the gate.
fn routine_body(
    auth: &otto_core::auth::AuthContext,
    bare: &str,
    body: Value,
    stored: Option<Value>,
) -> ApiResult<RoutineReq> {
    let probe = stored.as_ref().unwrap_or(&body);
    if !crate::ui_bridge::is_human(auth) && own::outward_delivery_reason(bare, probe).is_some() {
        return Err(ApiError(Error::Forbidden(format!(
            "a routine that posts outside the agent's chat needs a person's approval — call \
             otto.{bare} (it asks for it); an agent's own token cannot set one up directly"
        ))));
    }
    serde_json::from_value(body)
        .map_err(|e| ApiError(Error::Invalid(format!("invalid routine: {e}"))))
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
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let req = routine_body(&auth, "routine_create", body, None)?;
    let me = caller(&ctx, &user, &auth, req.session_id.as_deref(), Editor).await?;
    let mut agent = own::target(&ctx, &me, req.agent.as_deref()).await?;
    // An ownerless (seeded) agent has no one to run as, so its routine would
    // run headless and never reach the chat. The person whose chat set it up
    // adopts it.
    if agent.created_by.as_deref().is_none_or(str::is_empty)
        && super::adopt(&ctx, &agent.id, &user.id).await
    {
        agent.created_by = Some(user.id.clone());
    }
    Ok(Json(own::create(&ctx, &agent, req.routine).await?))
}

/// `PATCH /personal-agents/routines/{schedule_id}` (pause = `enabled: false`).
async fn update(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    let stored = own::with_stored_deliver(&ctx, "routine_update", Some(id.as_str()), &body).await;
    let req = routine_body(&auth, "routine_update", body, stored)?;
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
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<UserPersonalityReq>,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &auth.effective_user, &id, Editor).await?;
    if req.note.is_some() {
        super::require_agent_owner(&ctx, &auth, &agent, &["personality note"]).await?;
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
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ApplyReq>,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &auth.effective_user, &id, Editor).await?;
    super::require_agent_owner(&ctx, &auth, &agent, &["instructions"]).await?;
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
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &auth.effective_user, &id, Editor).await?;
    super::require_agent_owner(&ctx, &auth, &agent, &["instructions"]).await?;
    let prev = own::undo_instructions(&ctx, &agent).await?;
    Ok(Json(json!({"ok": true, "soul_md": prev})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auth(managed: Option<&str>) -> otto_core::auth::AuthContext {
        let user: User = serde_json::from_value(json!({
            "id": "u1", "username": "u", "display_name": "U", "is_root": false,
            "disabled": false, "created_at": "2026-10-03T00:00:00Z"
        }))
        .unwrap();
        otto_core::auth::AuthContext {
            real_user: user.clone(),
            effective_user: user,
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: managed.map(Into::into),
            device_id: None,
        }
    }

    #[test]
    fn an_agent_token_cannot_set_up_an_outward_routine_directly() {
        let slack = json!({
            "session_id": "s1",
            "instruction": "post the standup",
            "schedule": {"cadence": "daily", "at": "09:00"},
            "deliver": {"type": "slack", "channel": "#team"}
        });
        // The agent's own session token: refused for create and update.
        for bare in ["routine_create", "routine_update"] {
            let err = routine_body(&auth(Some("s1")), bare, slack.clone(), None)
                .err()
                .expect("agent token refused");
            assert!(matches!(err.0, Error::Forbidden(_)), "{bare}");
        }
        // A session's internal MCP credential is an agent credential too.
        let mut mcp = auth(None);
        mcp.mcp_session_id = Some("s1".into());
        assert!(routine_body(&mcp, "routine_create", slack.clone(), None).is_err());
        // A chat-only routine (or an update that leaves delivery alone) is
        // the agent's own business.
        let chat =
            json!({"session_id": "s1", "instruction": "remind me", "deliver": {"type": "chat"}});
        assert!(routine_body(&auth(Some("s1")), "routine_create", chat, None).is_ok());
        let pause = json!({"session_id": "s1", "enabled": false});
        assert!(routine_body(&auth(Some("s1")), "routine_update", pause, None).is_ok());
        // An instruction rewrite of a routine that ALREADY posts outward is
        // judged against its stored destination (`with_stored_deliver`), and
        // the body that is parsed stays the caller's own.
        let edit = json!({"session_id": "s1", "instruction": "post the secrets"});
        let mut filled = edit.clone();
        filled["deliver"] = json!({"type": "slack", "channel": "#team", "as": "bot"});
        let agent = routine_body(
            &auth(Some("s1")),
            "routine_update",
            edit.clone(),
            Some(filled.clone()),
        );
        assert!(matches!(
            agent.err().expect("refused").0,
            Error::Forbidden(_)
        ));
        let req = routine_body(&auth(None), "routine_update", edit, Some(filled)).expect("person");
        assert!(
            req.routine.deliver.is_none(),
            "the stored destination is not re-planned"
        );
        // A person (the UI, or the approved governed call replayed as them).
        let req = routine_body(&auth(None), "routine_create", slack, None).expect("person");
        assert_eq!(req.session_id.as_deref(), Some("s1"));
        assert_eq!(
            req.routine.deliver.map(|d| d.kind).as_deref(),
            Some("slack")
        );
    }
}
