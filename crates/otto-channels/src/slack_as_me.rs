//! The "act as me in Slack" gate and a personal agent's opt-in.
//!
//! [`caller_scope`] decides which of a person's connected Slack teams a
//! credential may act in as them: a person, or a session they run directly
//! (terminal, Threads, scheduled task, workflow, an external MCP client),
//! every team; a **personal agent** only the teams its owner ticked
//! (`AgentAutonomy.slack_as_me_teams`, default none); a share link none.
//! The calling session is read the way otto-server's
//! `personal_agent_policy::SessionBinding` reads it (`meta.personal_agent`).
//!
//! `GET / PUT /personal-agents/{id}/slack-as-me` read and set that opt-in.
//! Setting it is the agent **owner's** call only (it lends the owner's own
//! Slack identity) and only from a person's credential — never an agent's.

use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Extension, Json, Router};
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Result};
use otto_state::DbPool;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::ChannelsCtx;
use crate::slack_user_tokens::{self as store, AsMe, SlackMeView};

/// The agent session a credential is bound to, if any.
fn bound_session(auth: &AuthContext) -> Option<&otto_core::Id> {
    auth.mcp_session_id
        .as_ref()
        .or(auth.managed_session_id.as_ref())
}

/// A person's own credential (not an agent's, an MCP token's or a share link).
fn is_human(auth: &AuthContext) -> bool {
    bound_session(auth).is_none() && !auth.mcp_only && !auth.is_scoped()
}

/// The personal agent a session's server-owned meta runs as — read the way
/// otto-server's `SessionBinding` reads it (any string, so an odd value
/// narrows to that "agent's" empty grant rather than widening to `All`).
fn meta_agent(meta: &Value) -> Option<String> {
    meta.get("personal_agent")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Which connected Slack teams `auth` may act in as its user.
pub async fn caller_scope(pool: &DbPool, auth: &AuthContext) -> Result<AsMe> {
    if auth.is_scoped() {
        return Ok(AsMe::Teams(vec![]));
    }
    let Some(sid) = bound_session(auth) else {
        return Ok(AsMe::All);
    };
    let agent = match otto_state::SessionsRepo::new(pool.clone()).get(sid).await {
        Ok(s) => meta_agent(&s.meta),
        // A Threads thread has no `sessions` row. A personal agent's chat,
        // run or delegated work IS one, carrying the same server-owned
        // `meta.personal_agent` (`otto_assistant::personal_agent_threads`);
        // a plain thread has none. A session that resolves nowhere gets
        // nothing.
        Err(_) => match otto_state::AgentRuntimeRepo::new(pool.clone())
            .get(sid)
            .await
        {
            Ok(t) => t.session.meta.as_ref().and_then(meta_agent),
            Err(_) => return Ok(AsMe::Teams(vec![])),
        },
    };
    match agent {
        None => Ok(AsMe::All),
        Some(a) => Ok(AsMe::Teams(
            otto_state::PersonalAgentsRepo::new(pool.clone())
                .autonomy(&a)
                .await?
                .slack_as_me_teams,
        )),
    }
}

pub(crate) fn routes<S: ChannelsCtx>(r: Router<S>) -> Router<S> {
    r.route(
        "/personal-agents/{id}/slack-as-me",
        get(read_grant::<S>).put(save_grant::<S>),
    )
}

/// `GET /personal-agents/{id}/slack-as-me`.
#[derive(Debug, Serialize)]
pub struct GrantView {
    /// Team ids the agent may act in as its owner.
    pub teams: Vec<String>,
    /// The caller is the agent's owner (only they may change it).
    pub is_owner: bool,
    /// The caller's connected teams to choose from (only for the owner).
    pub available: Vec<SlackMeView>,
}

#[derive(Debug, Deserialize)]
pub struct GrantReq {
    pub teams: Vec<String>,
}

async fn agent_for<S: ChannelsCtx>(
    s: &S,
    user: &AuthUser,
    id: &str,
    role: WorkspaceRole,
) -> std::result::Result<otto_state::PersonalAgent, Error> {
    let agent = otto_state::PersonalAgentsRepo::new(s.integrations().pool())
        .get(id)
        .await?;
    s.roles().check(&user.0, &agent.workspace_id, role).await?;
    Ok(agent)
}

async fn view<S: ChannelsCtx>(
    s: &S,
    user: &AuthUser,
    agent: &otto_state::PersonalAgent,
) -> Result<GrantView> {
    let cfg = otto_state::PersonalAgentsRepo::new(s.integrations().pool())
        .autonomy(&agent.id)
        .await?;
    let is_owner = agent.created_by.as_deref() == Some(user.0.id.as_str());
    let available = if is_owner {
        store::list(s.secrets(), &user.0.id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect()
    } else {
        Vec::new()
    };
    Ok(GrantView {
        teams: cfg.slack_as_me_teams,
        is_owner,
        available,
    })
}

async fn read_grant<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<String>,
) -> std::result::Result<Json<GrantView>, crate::slack_tools::ApiErr> {
    let agent = agent_for(&s, &user, &id, WorkspaceRole::Viewer).await?;
    Ok(Json(view(&s, &user, &agent).await?))
}

/// `PUT /personal-agents/{id}/slack-as-me` — `{teams}`: the owner's own
/// connected team ids (others are refused); `[]` turns it off.
async fn save_grant<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<String>,
    Json(req): Json<GrantReq>,
) -> std::result::Result<Json<GrantView>, crate::slack_tools::ApiErr> {
    store::refuse_impersonation(&auth)?;
    if !is_human(&auth) {
        return Err(
            Error::Forbidden("only a person can let an agent act as them in Slack".into()).into(),
        );
    }
    let agent = agent_for(&s, &user, &id, WorkspaceRole::Editor).await?;
    if agent.created_by.as_deref() != Some(user.0.id.as_str()) {
        return Err(Error::Forbidden(
            "only this agent's owner can let it act as them in Slack".into(),
        )
        .into());
    }
    let mine = store::list(s.secrets(), &user.0.id).await?;
    let mut teams: Vec<String> = Vec::new();
    for t in req.teams.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if !mine.iter().any(|m| m.team_id == t) {
            return Err(Error::Invalid(format!(
                "'{t}' is not one of your connected Slack workspaces"
            ))
            .into());
        }
        if !teams.iter().any(|x| x == t) {
            teams.push(t.to_string());
        }
    }
    let repo = otto_state::PersonalAgentsRepo::new(s.integrations().pool());
    let mut cfg = repo.autonomy(&agent.id).await?;
    cfg.slack_as_me_teams = teams;
    repo.save_autonomy(&agent.id, &cfg).await?;
    Ok(Json(view(&s, &user, &agent).await?))
}

#[cfg(test)]
#[path = "slack_as_me_tests.rs"]
mod tests;
