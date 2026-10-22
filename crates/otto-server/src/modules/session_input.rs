//! Typing into sessions: the `/sessions/{id}/input` / `message` / `wait` routes,
//! agent-session open, and the broadcast / relay fan-out helpers they share.

use super::*;

/// The typist's freshly reloaded identity and workspace role — the part of
/// [`input_user`] that is the same for every session of one workspace, so a
/// fan-out (broadcast / relay) loads it ONCE per request instead of per target.
pub(crate) struct InputAuthority {
    pub(crate) user: User,
    pub(crate) role: Option<WorkspaceRole>,
}

impl InputAuthority {
    /// Reload `user_id` and require Editor+ in `ws_id`.
    pub(crate) async fn load(pool: &otto_state::DbPool, user_id: &Id, ws_id: &Id) -> Result<Self> {
        let user = otto_state::UsersRepo::new(pool.clone())
            .get(user_id)
            .await?;
        if user.disabled {
            return Err(Error::Forbidden("account disabled".into()));
        }
        let role = WorkspacesRepo::new(pool.clone())
            .role_of(&user, ws_id)
            .await?;
        if !matches!(role, Some(WorkspaceRole::Editor | WorkspaceRole::Admin)) {
            return Err(Error::Forbidden(
                "workspace Editor access is required for session input".into(),
            ));
        }
        Ok(Self { user, role })
    }

    /// A session owner — or root, or a workspace admin — may type into it.
    pub(crate) fn owner_or_admin(&self, session: &Session) -> Result<()> {
        if !self.user.is_root
            && self.user.id != session.created_by
            && self.role != Some(WorkspaceRole::Admin)
        {
            return Err(Error::Forbidden(
                "not the session owner or a workspace admin".into(),
            ));
        }
        Ok(())
    }
}

/// Reload identity and workspace authority at the actual input boundary. A
/// session owner still needs Editor membership to type into that workspace.
pub(crate) async fn input_user(
    pool: &otto_state::DbPool,
    user_id: &Id,
    session: &Session,
) -> Result<User> {
    let authority = InputAuthority::load(pool, user_id, &session.workspace_id).await?;
    authority.owner_or_admin(session)?;
    crate::resource_sessions::check_with_pool(pool, &authority.user, session).await?;
    Ok(authority.user)
}

pub(crate) async fn input_session(ctx: &ServerCtx, user_id: &Id, id: &Id) -> Result<Session> {
    let session = ctx.manager.get(id).await?;
    input_user(&ctx.pool, user_id, &session).await?;
    Ok(session)
}

/// Who is typing into a session through the REST fan-out routes: the person
/// the credential belongs to, and — when the credential is an agent
/// session's own token — that session (S11-305). An agent caller reaches
/// itself, the workers it opened and its sibling agent sessions
/// ([`agent_message_rule`]), and what it sends is recorded as
/// agent-originated, never as the person's message.
#[derive(Clone, Copy)]
pub(crate) struct Typist<'a> {
    pub(crate) user_id: &'a Id,
    pub(crate) agent: Option<&'a Id>,
}

impl<'a> Typist<'a> {
    pub(crate) fn new(user_id: &'a Id, auth: &'a otto_core::auth::AuthContext) -> Self {
        Self {
            user_id,
            agent: crate::feature_guard::agent_session_of(auth),
        }
    }
}

/// Send both the paste and its delayed submit through current authorization.
/// `session` is the row the caller already authorized for `who` (via
/// [`input_session`] or [`input_agents`]); the paste itself still re-checks
/// current authority inside `human_submit_text_checked`, so a grant revoked
/// in between is honoured.
pub(crate) async fn submit_session_text(
    ctx: &ServerCtx,
    who: Typist<'_>,
    session: &Session,
    text: &str,
) -> Result<()> {
    let user_id = who.user_id;
    let id = &session.id;
    agent_message_rule(who.agent, session)?;
    ctx.manager
        .human_submit_text_checked(id, user_id, false, text, || async {
            input_session(ctx, user_id, id).await.map(|_| ())
        })
        .await?;
    match who.agent {
        Some(from) => ctx.manager.record_agent_message(id, from, text).await,
        None => ctx.manager.record_user_message(id, text).await,
    }
    Ok(())
}

/// The workspace's live agent sessions `who` may type into — the same rules
/// as [`input_session`] per row, with the user + role loaded once and the
/// resource-bound rows decided in one batch. Archived rows and connection
/// sessions are filtered in SQL (it used to decode the workspace's whole
/// session history, then re-authorize each live agent from scratch).
pub(crate) async fn input_agents(
    ctx: &ServerCtx,
    who: Typist<'_>,
    ws_id: &Id,
) -> Result<Vec<Session>> {
    let Ok(authority) = InputAuthority::load(&ctx.pool, who.user_id, ws_id).await else {
        // Not an Editor here (or disabled): no target is reachable.
        return Ok(Vec::new());
    };
    let live: Vec<Session> = ctx
        .manager
        .list_filtered(
            &[otto_state::SessionScope {
                workspace_id: ws_id.clone(),
                owner: None,
            }],
            &otto_state::SessionListFilter {
                archived: Some(false),
                kind: Some("agent".into()),
                ..Default::default()
            },
        )
        .await?
        .into_iter()
        .filter(|session| {
            session.kind == SessionKind::Agent
                && matches!(
                    session.status,
                    otto_core::domain::SessionStatus::Running
                        | otto_core::domain::SessionStatus::Working
                        | otto_core::domain::SessionStatus::Idle
                )
                && agent_message_rule(who.agent, session).is_ok()
                && authority.owner_or_admin(session).is_ok()
        })
        .collect();
    Ok(crate::resource_sessions::check_many(ctx, &authority.user, live).await)
}

pub(crate) async fn broadcast_sessions(
    ctx: &ServerCtx,
    who: Typist<'_>,
    ws_id: &Id,
    text: &str,
    targets: Option<&[Id]>,
) -> Result<Vec<Id>> {
    let mut sent = Vec::new();
    for session in input_agents(ctx, who, ws_id).await? {
        if targets.is_none_or(|ids| ids.contains(&session.id))
            && submit_session_text(ctx, who, &session, text).await.is_ok()
        {
            sent.push(session.id);
        }
    }
    Ok(sent)
}

pub(crate) fn delay_session_input(
    ctx: &ServerCtx,
    user: &User,
    session_id: &Id,
    text: String,
    delay: Duration,
    token: String,
) {
    let ctx = ctx.clone();
    let user_id = user.id.clone();
    let session_id = session_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let result: Result<()> = async {
            let auth = ctx.authenticator.authenticate(&token).await?;
            if auth.effective_user.id != user_id || auth.mcp_only || auth.scope.is_some() {
                return Err(Error::Unauthorized);
            }
            input_session(&ctx, &user_id, &session_id).await?;
            ctx.manager
                .human_input(
                    &session_id,
                    &user_id,
                    false,
                    true,
                    format!("{text}\n").as_bytes(),
                )
                .await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(session = %session_id, "delayed session input denied or failed: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// Router ctx trait impls
// ---------------------------------------------------------------------------

/// Routes: POST /sessions/{id}/input, POST /sessions/{id}/message,
/// GET /sessions/{id}/wait
pub fn session_input_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/sessions/{id}/input", post(send_input))
        .route("/sessions/{id}/message", post(session_message))
        .route("/sessions/{id}/wait", get(wait_session))
}

/// Longest a `/sessions/{id}/wait` call may block. Kept under the 30 s the
/// MCP self-call client allows so a governed `otto.wait_session` never times
/// out on the transport; callers loop for longer waits.
pub(crate) const MAX_WAIT_SESSION_SECS: u64 = 25;

/// `POST /workspaces/{id}/sessions/open` — open an agent session for a
/// delegating lead and queue its opening prompt. The prompt goes through
/// [`crate::review_session::submit_prompt`] (wait for the TUI, paste, verify
/// the echo, Enter) on a background task, so the caller gets the session row
/// back immediately and polls `/sessions/{id}/wait` for the outcome. Editor.
pub(crate) async fn open_agent_session(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::OpenAgentSessionReq>,
) -> ApiResult<Json<otto_core::api::OpenAgentSessionResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let provider = req.provider.trim().to_string();
    if provider.is_empty() {
        return Err(ApiError(Error::Invalid("provider is required".into())));
    }
    // Stamp the work origin so usage attribution can tell a delegated worker
    // from a hand-opened tab; a caller-supplied `work` ref wins.
    let mut meta = req.meta.unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        return Err(ApiError(Error::Invalid("meta must be an object".into())));
    }
    if meta.get("work").is_none() {
        meta["work"] = serde_json::json!({ "origin": "delegation" });
    }
    // The delegating lead, stamped by the server from the credential (never
    // the body). `/sessions/{id}/message` lets an agent's own token reach any
    // non-shell agent session that this user could message, including a
    // sibling it did not open. A shell or SSH sibling stays refused unless
    // `delegated_by` names this agent (S11-02).
    match crate::feature_guard::agent_session_of(&auth) {
        Some(lead) => meta[DELEGATED_BY_META] = serde_json::json!(lead),
        None => {
            if let Some(m) = meta.as_object_mut() {
                m.remove(DELEGATED_BY_META);
            }
        }
    }
    let create = otto_core::api::CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider),
        title: req.title.filter(|t| !t.trim().is_empty()),
        cwd: req.cwd.filter(|c| !c.trim().is_empty()),
        connection_id: None,
        meta: Some(meta),
        model: req.model,
    };
    let ws = WorkspacesRepo::new(ctx.pool.clone())
        .get(&ws_id)
        .await
        .map_err(ApiError)?;
    // S11-302: an agent's own credential opens workers only inside the
    // workspace, its own folder, a registered repo or a worktree of those.
    if let Some(lead) = crate::feature_guard::agent_session_of(&auth) {
        ctx.manager
            .check_agent_cwd(&ws, lead, create.cwd.as_deref())
            .await
            .map_err(ApiError)?;
    }
    let session = ctx
        .manager
        .create(&ws, &user.id, create, None)
        .await
        .map_err(ApiError)?;

    let prompt = req
        .prompt
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let prompt_dispatch = match prompt {
        Some(prompt) => {
            let manager = ctx.manager.clone();
            let sid = session.id.clone();
            tokio::spawn(async move {
                if crate::review_session::submit_prompt(&manager, &sid, &prompt).await {
                    manager.record_user_message(&sid, &prompt).await;
                } else {
                    tracing::warn!(session = %sid, "delegation: session ended before its opening prompt was sent");
                }
            });
            "queued"
        }
        None => "none",
    };
    Ok(Json(otto_core::api::OpenAgentSessionResp {
        session,
        prompt_dispatch: prompt_dispatch.into(),
    }))
}

/// Session meta key naming the agent session that opened this one through
/// `POST /workspaces/{id}/sessions/open` (server-owned: PATCH cannot set it).
pub const DELEGATED_BY_META: &str = otto_sessions::http::DELEGATED_BY_META;

/// Confinement of an agent session's OWN credential on the REST twins of the
/// terminal (S11-02 / S1-11) and on broadcast/relay (S11-305 / S1-303). The
/// one rule lives with the sessions router, which applies it to the lifecycle
/// routes (PATCH, restart, kill, archive, …) too.
pub use otto_sessions::http::{agent_input_rule, agent_message_rule};

/// `POST /sessions/{id}/message` — deliver ONE message to ONE live agent
/// session as if typed + Enter: the targeted counterpart of
/// `/workspaces/{id}/broadcast`, for a lead driving a single worker. Same
/// authorization as `/input` (Editor + owner-or-admin, resource re-check).
pub(crate) async fn session_message(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::SessionMessageReq>,
) -> ApiResult<Json<otto_core::api::SessionMessageResp>> {
    let text = req.text.trim();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("message text is empty".into())));
    }
    let session = input_session(&ctx, &user.id, &session_id)
        .await
        .map_err(ApiError)?;
    agent_message_rule(crate::feature_guard::agent_session_of(&auth), &session)
        .map_err(ApiError)?;
    if session.kind != SessionKind::Agent {
        return Err(ApiError(Error::Invalid(
            "messages can only be sent to agent sessions".into(),
        )));
    }
    if !matches!(
        session.status,
        otto_core::domain::SessionStatus::Running
            | otto_core::domain::SessionStatus::Working
            | otto_core::domain::SessionStatus::Idle
    ) {
        return Err(ApiError(Error::Conflict(format!(
            "session is {}, not live",
            session.status.as_str()
        ))));
    }
    submit_session_text(&ctx, Typist::new(&user.id, &auth), &session, text)
        .await
        .map_err(ApiError)?;
    Ok(Json(otto_core::api::SessionMessageResp {
        session_id,
        delivered: true,
    }))
}

/// Query of `GET /sessions/{id}/wait`.
#[derive(Deserialize)]
pub(crate) struct WaitSessionQuery {
    /// Comma-separated statuses to wait for (default `idle,exited`).
    pub(crate) status: Option<String>,
    /// Seconds to block at most (default 20, capped at [`MAX_WAIT_SESSION_SECS`]).
    pub(crate) timeout_secs: Option<u64>,
}

/// `GET /sessions/{id}/wait` — block until the session's status is one of the
/// awaited set or the deadline passes, then return the session as observed.
/// `idle` is "the agent's turn ended" for a delegating lead. Owner-or-admin.
pub(crate) async fn wait_session(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WaitSessionQuery>,
) -> ApiResult<Json<otto_core::api::WaitSessionResp>> {
    // Event-driven: subscribe BEFORE the first read so a transition between
    // the read and the wait is never missed, then re-read the row only when
    // THIS session's status event arrives (or every
    // [`WAIT_SESSION_SAFETY_RECHECK`], or after a lagged bus). It used to
    // re-read and decode the full row every 500 ms per waiter — orchestrators
    // long-poll this for every worker they supervise.
    let mut rx = ctx.events.subscribe();
    let mut session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;
    let wanted: Vec<String> = q
        .status
        .as_deref()
        .unwrap_or("idle,exited")
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if wanted.is_empty() {
        return Err(ApiError(Error::Invalid("status list is empty".into())));
    }
    let timeout = Duration::from_secs(q.timeout_secs.unwrap_or(20).min(MAX_WAIT_SESSION_SECS));
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if wanted.iter().any(|w| w == session.status.as_str()) {
            return Ok(Json(otto_core::api::WaitSessionResp {
                session,
                reached: true,
            }));
        }
        // A daemon shutdown answers the long-poll now (callers loop on
        // `reached: false`) instead of holding the HTTP drain open past
        // launchd's exit timeout.
        if tokio::time::Instant::now() >= deadline || crate::shutdown::is_shutting_down() {
            return Ok(Json(otto_core::api::WaitSessionResp {
                session,
                reached: false,
            }));
        }
        let wake_at = (tokio::time::Instant::now() + WAIT_SESSION_SAFETY_RECHECK).min(deadline);
        tokio::select! {
            () = wait_for_session_event(&mut rx, &session_id, wake_at) => {}
            _ = crate::shutdown::cancelled() => {}
        }
        session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    }
}

/// Slowest re-read of a waited-on session when no event for it arrives (the
/// bus is best-effort: a missed send must not strand a waiter until timeout).
pub(crate) const WAIT_SESSION_SAFETY_RECHECK: Duration = Duration::from_secs(5);

/// Block until `session_id`'s status changes or it is removed (the manager
/// writes the row BEFORE it broadcasts), the bus lags (ours may have been
/// dropped), or `wake_at` passes. Other sessions' events are skipped.
pub(crate) async fn wait_for_session_event(
    rx: &mut tokio::sync::broadcast::Receiver<Event>,
    session_id: &Id,
    wake_at: tokio::time::Instant,
) {
    crate::workflow_engine::wake_on_event(rx, |ev| is_session_change(ev, session_id), wake_at)
        .await;
}

/// Whether `ev` is a status change or removal of `session_id`.
pub(crate) fn is_session_change(ev: &Event, session_id: &Id) -> bool {
    matches!(ev,
        Event::SessionStatus { session_id: id, .. } | Event::SessionRemoved { session_id: id, .. }
            if id == session_id)
}

pub(crate) async fn send_input(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::SendInputReq>,
) -> ApiResult<axum::http::StatusCode> {
    // Resolve the session and check that the caller has Editor access to the
    // workspace that owns it — then owner-or-admin: typing into another user's
    // live agent is the same capability as attaching to its terminal (the WS
    // gate), so it gets the same confinement.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Editor).await?;
    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;
    agent_input_rule(
        crate::feature_guard::agent_session_of(&auth),
        &session.id,
        &session.meta,
        false,
    )
    .map_err(ApiError)?;

    // `submit` omitted/true: paste + a real Enter via `submit_text` — writing
    // `"{text}\n"` in one burst makes bracketed-paste TUIs (Claude Code, Codex)
    // treat the newline as pasted content, so the message lands but is never
    // sent (docs/design/conversation-view.md §1). `submit: false`: the text
    // verbatim, for the user to inspect/edit before pressing Enter.
    let bracketed_tui = session.kind == otto_core::domain::SessionKind::Agent
        && matches!(session.provider.as_str(), "claude" | "codex");
    if req.submit.unwrap_or(true) && bracketed_tui {
        ctx.manager
            .human_submit_text(&session_id, &user.id, auth.scope.is_some(), &req.text)
            .await
            .map_err(ApiError)?;
    } else if req.submit.unwrap_or(true) {
        // Shells / connections / bridges: a plain line + newline runs it.
        ctx.manager
            .human_input(
                &session_id,
                &user.id,
                auth.scope.is_some(),
                true,
                format!("{}\n", req.text).as_bytes(),
            )
            .await
            .map_err(ApiError)?;
    } else {
        ctx.manager
            .human_input(
                &session_id,
                &user.id,
                auth.scope.is_some(),
                true,
                req.text.as_bytes(),
            )
            .await
            .map_err(ApiError)?;
    }

    Ok(axum::http::StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Browser proxy route  (GET /browser/proxy?url=…&ticket=…)
// ---------------------------------------------------------------------------
//
// The take-over iframe can't send an `Authorization` header, so the proxy used
// to accept the owner's bearer as `?token=` — which leaked it into the iframe
// URL (history, Referer, the proxied page's `location.href`) and ran arbitrary
// third-party HTML on the daemon origin, where it could read that token back.
// Now the UI mints a single-use, short-lived ticket bound to ONE url via an
// authed POST (`/api/v1/browser/proxy-ticket`), and every proxy response is
// served with `Content-Security-Policy: sandbox allow-scripts` (no
// `allow-same-origin`): the proxied page runs in an opaque origin and can
// neither read the daemon's storage nor make credentialed same-origin calls.

#[cfg(test)]
mod terminal_input_access_tests {
    use super::*;
    use otto_core::access::{AccessActor, ResourceKind};

    #[tokio::test]
    async fn alternate_input_checks_current_resource_page_owner_and_workspace() {
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let users = otto_state::UsersRepo::new(pool.clone());
        let owner = users.create("owner", "hash", "Owner", false).await.unwrap();
        let outsider = users
            .create("outsider", "hash", "Outsider", false)
            .await
            .unwrap();
        let ws = WorkspacesRepo::new(pool.clone())
            .create("Workspace", "/tmp", &owner.id)
            .await
            .unwrap();
        WorkspacesRepo::new(pool.clone())
            .set_member(&ws.id, &outsider.id, WorkspaceRole::Editor)
            .await
            .unwrap();
        let conn_id = otto_core::new_id();
        sqlx::query("INSERT INTO connections (id,name,kind,params_json,created_by,created_at) VALUES (?,'SSH','ssh','{}',?,'2026-09-05T00:00:00Z')").bind(&conn_id).bind(&owner.id).execute(&pool).await.unwrap();
        for user in [&owner, &outsider] {
            sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'connections','view')").bind(&user.id).execute(&pool).await.unwrap();
        }
        let repo = otto_state::ResourceAccessRepo::new(pool.clone());
        repo.initialize_owner_policy(
            ResourceKind::Connection,
            &conn_id,
            &owner.id,
            &["discover".into(), "shell".into()],
            &[],
            &AccessActor {
                real_user_id: owner.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        let mut shared = repo
            .get_policy(ResourceKind::Connection, &conn_id)
            .await
            .unwrap();
        let mut outsider_rule = shared.rules[0].clone();
        outsider_rule.id = otto_core::new_id();
        outsider_rule.subject_id = outsider.id.clone();
        shared.rules.push(outsider_rule);
        repo.put_policy(
            &shared,
            shared.revision,
            &AccessActor {
                real_user_id: owner.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        let session = otto_state::SessionsRepo::new(pool.clone())
            .create(otto_state::NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Connection,
                provider: "ssh".into(),
                title: "SSH".into(),
                cwd: "/tmp".into(),
                provider_session_id: None,
                connection_id: Some(conn_id.clone()),
                created_by: owner.id.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        assert!(input_user(&pool, &owner.id, &session).await.is_ok());
        assert!(
            input_user(&pool, &outsider.id, &session).await.is_err(),
            "workspace Editor cannot inject into another user's terminal"
        );
        sqlx::query("DELETE FROM user_feature_grants WHERE user_id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "page revocation applies at delayed input"
        );
        sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'connections','view')").bind(&owner.id).execute(&pool).await.unwrap();
        WorkspacesRepo::new(pool.clone())
            .set_member(&ws.id, &owner.id, WorkspaceRole::Viewer)
            .await
            .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "ownership cannot override revoked Editor role"
        );
        WorkspacesRepo::new(pool.clone())
            .set_member(&ws.id, &owner.id, WorkspaceRole::Admin)
            .await
            .unwrap();
        assert!(input_user(&pool, &owner.id, &session).await.is_ok());
        sqlx::query("UPDATE users SET disabled=1 WHERE id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "disabled user cannot supply delayed input"
        );
        sqlx::query("UPDATE users SET disabled=0 WHERE id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        let mut policy = repo
            .get_policy(ResourceKind::Connection, &conn_id)
            .await
            .unwrap();
        policy.rules.clear();
        repo.put_policy(
            &policy,
            policy.revision,
            &AccessActor {
                real_user_id: owner.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "resource revoke applies even with workspace Admin"
        );
    }
}

#[cfg(test)]
mod wait_session_tests {
    use super::{wait_for_session_event, Event};
    use otto_core::domain::SessionStatus;
    use otto_core::Id;
    use std::time::Duration;

    fn status(id: &str) -> Event {
        Event::SessionStatus {
            session_id: Id::from(id),
            workspace_id: Id::from("ws"),
            status: SessionStatus::Idle,
        }
    }

    /// The long-poll wakes on ITS session's status change (not another's)
    /// well before the safety re-read, and on a lagged bus.
    #[tokio::test]
    async fn wakes_only_for_its_own_session() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(4);
        let me = Id::from("me");
        let far = tokio::time::Instant::now() + Duration::from_secs(30);
        tx.send(status("other")).unwrap();
        tx.send(status("me")).unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_session_event(&mut rx, &me, far),
        )
        .await
        .expect("woke on its own status event");
        // Only someone else's event: sleeps until the safety deadline.
        tx.send(status("other")).unwrap();
        let soon = tokio::time::Instant::now() + Duration::from_millis(50);
        let started = tokio::time::Instant::now();
        wait_for_session_event(&mut rx, &me, soon).await;
        assert!(started.elapsed() >= Duration::from_millis(45));
        // A removal wakes too.
        tx.send(Event::SessionRemoved {
            session_id: me.clone(),
            workspace_id: Id::from("ws"),
        })
        .unwrap();
        tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_session_event(&mut rx, &me, far),
        )
        .await
        .expect("woke on removal");
        // A lagged receiver re-reads (ours may have been dropped).
        for _ in 0..6 {
            tx.send(status("other")).unwrap();
        }
        tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_session_event(&mut rx, &me, far),
        )
        .await
        .expect("woke on lag");
    }
}

#[cfg(test)]
mod agent_input_rule_tests {
    use super::{agent_input_rule, DELEGATED_BY_META};
    use otto_core::Id;
    use serde_json::json;

    /// S11-02 / S1-11: the REST twins of `/ws/term` confine an agent
    /// session's own token like the WS gate does.
    #[test]
    fn agent_token_reaches_only_its_own_session_and_its_workers() {
        let me = Id::from("lead");
        let sibling = Id::from("sibling-shell");
        let worker = Id::from("worker");
        let worker_meta = json!({ DELEGATED_BY_META: "lead" });
        let other_worker_meta = json!({ DELEGATED_BY_META: "someone-else" });
        // A person's credential is unaffected.
        assert!(agent_input_rule(None, &sibling, &json!({}), false).is_ok());
        // Own session: /input and /message.
        assert!(agent_input_rule(Some(&me), &me, &json!({}), false).is_ok());
        assert!(agent_input_rule(Some(&me), &me, &json!({}), true).is_ok());
        // A sibling (the owner's unsandboxed shell): never.
        assert!(agent_input_rule(Some(&me), &sibling, &json!({}), false).is_err());
        assert!(agent_input_rule(Some(&me), &sibling, &json!({}), true).is_err());
        // Its own worker: /message yes (otto_send_message), raw /input no.
        assert!(agent_input_rule(Some(&me), &worker, &worker_meta, true).is_ok());
        assert!(agent_input_rule(Some(&me), &worker, &worker_meta, false).is_err());
        // Another lead's worker: no.
        assert!(agent_input_rule(Some(&me), &worker, &other_worker_meta, true).is_err());
    }
}
