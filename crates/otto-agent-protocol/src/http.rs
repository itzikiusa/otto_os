//! `/runtime-sessions/*` REST router (contract: `docs/contracts/api.md`).
//! Enforces workspace `viewer`+ / session owner-or-admin RBAC and requires a
//! human credential for writes (except `/peers`, where a session's own agent
//! token may open/message/read same-owner, same-workspace linked threads at no
//! wider access mode).

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::agent_runtime::{
    validate_thread_meta, AskChildReq, AskChildResp, CouncilResp, CreateCouncilReq,
    CreateRuntimeSessionBody, OpenPeerReq, PeerInfo, PeerMessageReq, PeerReply, QueuedDeliverReq,
    RuntimePromptReq, RuntimePromptResp, RuntimeReplay, RuntimeRequestRespondReq, RuntimeSession,
    SynthesizeCouncilReq, TurnItem, UnqueuePromptResp, UpdateRuntimeSessionReq,
};
use otto_core::api::Problem;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use serde::Deserialize;

use crate::RuntimeCtx;

pub(crate) mod drafts;
mod tasks;

pub(crate) struct ApiErr(pub Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
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
            tracing::error!(error = %self.0, "runtime-sessions internal error");
        }
        let message = match &self.0 {
            Error::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        };
        let problem = Problem {
            code: self.0.code().to_string(),
            message,
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: RuntimeCtx>() -> Router<S> {
    Router::new()
        // The caller's threads across every workspace (a thread started on
        // another device lands in whatever workspace that device had open).
        .route("/runtime-sessions", get(list_all::<S>))
        .route(
            "/workspaces/{id}/runtime-sessions",
            get(list::<S>).post(create::<S>),
        )
        // One prompt to 2–4 agents; inline images ride in the body.
        .route(
            "/workspaces/{id}/runtime-sessions/council",
            post(create_council::<S>).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route(
            "/runtime-sessions/{id}/council/synthesize",
            post(synthesize_council::<S>),
        )
        .route(
            "/runtime-sessions/{id}",
            get(get_one::<S>).patch(rename::<S>).delete(delete_one::<S>),
        )
        .route(
            "/runtime-sessions/{id}/draft",
            get(drafts::get::<S>)
                .put(drafts::put::<S>)
                .layer(DefaultBodyLimit::max(24 * 1024 * 1024)),
        )
        // Inline base64 images ride in the body (axum's default cap is 2 MB).
        .route(
            "/runtime-sessions/{id}/prompt",
            post(prompt::<S>).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route("/runtime-sessions/{id}/interrupt", post(interrupt::<S>))
        .route(
            "/runtime-sessions/{id}/requests/{request_id}",
            post(answer::<S>),
        )
        .route("/runtime-sessions/{id}/close", post(close::<S>))
        .route("/runtime-sessions/{id}/reopen", post(reopen::<S>))
        .route("/runtime-sessions/{id}/resume-at", post(resume_at::<S>))
        .route(
            "/runtime-sessions/{id}/queue/{queued_id}",
            post(deliver_queued::<S>).delete(unqueue::<S>),
        )
        .route("/runtime-sessions/{id}/events", get(events::<S>))
        .route("/runtime-sessions/{id}/items/{item_id}", get(get_item::<S>))
        .route(
            "/runtime-sessions/{id}/attachments/{attachment_id}",
            get(attachment::<S>),
        )
        .route(
            "/runtime-sessions/{id}/items/{item_id}/images/{n}",
            get(tool_image::<S>),
        )
        // Work the agent runs off its protocol stream (Agents & tasks).
        .route(
            "/runtime-sessions/{id}/tasks/{task_id}/output",
            get(task_output::<S>),
        )
        // A person stops background work (the turn's Stop does not).
        .route(
            "/runtime-sessions/{id}/tasks/stop",
            post(tasks::stop_all::<S>),
        )
        .route(
            "/runtime-sessions/{id}/tasks/{task_id}/stop",
            post(tasks::stop_one::<S>),
        )
        .route("/runtime-sessions/{id}/swarm", get(swarm::<S>))
        .route(
            "/runtime-sessions/{id}/peers",
            get(peers::<S>).post(open_peer::<S>),
        )
        .route(
            "/runtime-sessions/{id}/peers/{peer}/message",
            post(message_peer::<S>),
        )
        .route(
            "/runtime-sessions/{id}/peers/{peer}/reply",
            get(peer_reply::<S>),
        )
        .route(
            "/runtime-sessions/{id}/peers/{peer}/close",
            post(close_peer::<S>),
        )
        // "Ask another agent" from a thread's composer; images ride in the body.
        .route(
            "/runtime-sessions/{id}/children",
            post(ask_children::<S>).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
}

/// POST /runtime-sessions/{id}/children — a PERSON sends one message to 1–4
/// new threads linked to `{id}` (editor+, the thread's owner or an admin).
async fn ask_children<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<AskChildReq>,
) -> ApiResult<(StatusCode, Json<AskChildResp>)> {
    require_human(&ctx, &auth)?;
    let s = authorized(&ctx, &user, &id).await?;
    ctx.roles()
        .check(&user, &s.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.agent_runtime().person_prompted(&id).await;
    let children = ctx.agent_runtime().ask_children(&id, req).await?;
    Ok((StatusCode::CREATED, Json(AskChildResp { children })))
}

/// A linked-thread call AS thread `id`: its owner (a person) or that
/// thread's own agent — never another session's credential, an external MCP
/// or a share token.
async fn as_thread<S: RuntimeCtx>(
    ctx: &S,
    user: &User,
    auth: &AuthContext,
    id: &Id,
) -> ApiResult<RuntimeSession> {
    let own_agent = auth.managed_session_id.as_ref() == Some(id) && !auth.is_scoped();
    if !own_agent {
        require_human(ctx, auth)?;
    }
    let s = authorized(ctx, user, id).await?;
    ctx.roles()
        .check(user, &s.workspace_id, WorkspaceRole::Editor)
        .await?;
    Ok(s)
}

/// GET /runtime-sessions/{id}/peers — the linked group.
async fn peers<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<Json<Vec<PeerInfo>>> {
    as_thread(&ctx, &user, &auth, &id).await?;
    Ok(Json(ctx.agent_runtime().peers(&id).await?))
}

/// POST /runtime-sessions/{id}/peers — open a linked thread.
async fn open_peer<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<OpenPeerReq>,
) -> ApiResult<(StatusCode, Json<RuntimeSession>)> {
    as_thread(&ctx, &user, &auth, &id).await?;
    let by_person = ctx.is_human(&auth);
    let s = ctx
        .agent_runtime()
        .open_peer_by(&id, req, by_person)
        .await?;
    if by_person {
        ctx.agent_runtime().person_prompted(&id).await;
    }
    Ok((StatusCode::CREATED, Json(s)))
}

/// POST /runtime-sessions/{id}/peers/{peer}/message — the peer's next turn.
async fn message_peer<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, peer)): Path<(Id, Id)>,
    Json(req): Json<PeerMessageReq>,
) -> ApiResult<(StatusCode, Json<RuntimePromptResp>)> {
    as_thread(&ctx, &user, &auth, &id).await?;
    let r = ctx
        .agent_runtime()
        .message_peer_by(&id, &peer, &req.text, ctx.is_human(&auth))
        .await?;
    Ok((StatusCode::ACCEPTED, Json(r)))
}

/// POST /runtime-sessions/{id}/peers/{peer}/close — close a linked thread.
async fn close_peer<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, peer)): Path<(Id, Id)>,
) -> ApiResult<Json<RuntimeSession>> {
    as_thread(&ctx, &user, &auth, &id).await?;
    Ok(Json(ctx.agent_runtime().close_peer(&id, &peer).await?))
}

#[derive(Deserialize)]
struct ReplyQuery {
    /// Seconds to wait for a running turn to settle, 0..=25 (default 20).
    #[serde(default)]
    wait_secs: Option<u64>,
}

/// GET /runtime-sessions/{id}/peers/{peer}/reply?wait_secs= — its last reply.
async fn peer_reply<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, peer)): Path<(Id, Id)>,
    Query(q): Query<ReplyQuery>,
) -> ApiResult<Json<PeerReply>> {
    as_thread(&ctx, &user, &auth, &id).await?;
    let wait = std::time::Duration::from_secs(q.wait_secs.unwrap_or(20).min(25));
    Ok(Json(
        ctx.agent_runtime().peer_reply(&id, &peer, wait).await?,
    ))
}

fn require_human<S: RuntimeCtx>(ctx: &S, auth: &AuthContext) -> ApiResult<()> {
    if ctx.is_human(auth) {
        Ok(())
    } else {
        Err(ApiErr(Error::Forbidden(
            "a person signed in to Otto must do this — an agent session's, MCP or share \
             credential cannot"
                .into(),
        )))
    }
}

async fn is_ws_admin<S: RuntimeCtx>(ctx: &S, user: &User, ws: &Id) -> bool {
    user.is_root
        || ctx
            .roles()
            .check(user, ws, WorkspaceRole::Admin)
            .await
            .is_ok()
}

/// The session, if `user` may see it: workspace viewer+, then owner / admin / root.
async fn authorized<S: RuntimeCtx>(ctx: &S, user: &User, id: &Id) -> ApiResult<RuntimeSession> {
    let s = ctx.agent_runtime().get(id).await?;
    ctx.roles()
        .check(user, &s.workspace_id, WorkspaceRole::Viewer)
        .await?;
    if s.created_by == user.id || is_ws_admin(ctx, user, &s.workspace_id).await {
        Ok(s)
    } else {
        Err(ApiErr(Error::Forbidden(
            "not the session owner or a workspace admin".into(),
        )))
    }
}

#[derive(Deserialize)]
struct ListQuery {
    /// Only threads whose `meta.source` is this (an embedded page's lookup).
    #[serde(default)]
    source: Option<String>,
    /// Only threads whose `meta.origin.id` is this.
    #[serde(default)]
    origin_id: Option<String>,
}

/// GET /workspaces/{id}/runtime-sessions[?source=&origin_id=] — the caller's
/// own (all, for an admin); the filters narrow that same set, closed
/// threads included.
async fn list<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws): Path<Id>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<RuntimeSession>>> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Viewer).await?;
    let owner = (!is_ws_admin(&ctx, &user, &ws).await).then_some(&user.id);
    let source = q.source.as_deref().filter(|s| !s.is_empty());
    let origin_id = q.origin_id.as_deref().filter(|s| !s.is_empty());
    Ok(Json(
        ctx.agent_runtime()
            .list_filtered(&ws, owner, source, origin_id)
            .await?,
    ))
}

/// GET /runtime-sessions — the caller's own sessions in every workspace they
/// can view (root: everyone's), newest activity first, max 500.
async fn list_all<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<Vec<RuntimeSession>>> {
    let owner = (!user.is_root).then_some(&user.id);
    let rows = ctx.agent_runtime().list_all(owner).await?;
    let mut viewable = std::collections::HashMap::<Id, bool>::new();
    let mut out = Vec::with_capacity(rows.len());
    for s in rows {
        let ok = match viewable.get(&s.workspace_id) {
            Some(&ok) => ok,
            None => {
                let ok = ctx
                    .roles()
                    .check(&user, &s.workspace_id, WorkspaceRole::Viewer)
                    .await
                    .is_ok();
                viewable.insert(s.workspace_id.clone(), ok);
                ok
            }
        };
        if ok {
            out.push(s);
        }
    }
    Ok(Json(out))
}

/// POST /workspaces/{id}/runtime-sessions — create + spawn (editor+).
async fn create<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws): Path<Id>,
    Json(body): Json<CreateRuntimeSessionBody>,
) -> ApiResult<(StatusCode, Json<RuntimeSession>)> {
    require_human(&ctx, &auth)?;
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    let rt = ctx.agent_runtime();
    let effort = body.effort.clone();
    let effort = effort.as_deref().map(str::trim).filter(|e| !e.is_empty());
    let req = body.into_req_with_resolver(|slug| rt.resolve_runtime_provider(slug))?;
    // Checked before the thread exists: a bad effort creates nothing.
    if let (Some(e), Some(p)) = (effort, req.provider) {
        let caps = otto_core::agent_runtime::RuntimeCapabilities::for_provider(p);
        crate::runtime::check_effort(&caps, p.as_str(), e)?;
    }
    let mut s = rt.create(&user.id, &ws, req).await?;
    if effort.is_some() {
        s = rt.set_effort(&s.id, effort).await?;
    }
    Ok((StatusCode::CREATED, Json(s)))
}

/// POST /workspaces/{id}/runtime-sessions/council — 2–4 seats, created all
/// or none, each sent the prompt (editor+, a person).
async fn create_council<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws): Path<Id>,
    Json(req): Json<CreateCouncilReq>,
) -> ApiResult<(StatusCode, Json<CouncilResp>)> {
    require_human(&ctx, &auth)?;
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    let seats = ctx
        .agent_runtime()
        .create_council(&user.id, &ws, req)
        .await?;
    Ok((StatusCode::CREATED, Json(CouncilResp { seats })))
}

/// POST /runtime-sessions/{id}/council/synthesize — open the council's judge
/// (`{id}` = any of its threads); 409 while a seat still works.
async fn synthesize_council<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<SynthesizeCouncilReq>,
) -> ApiResult<(StatusCode, Json<RuntimeSession>)> {
    require_human(&ctx, &auth)?;
    let s = authorized(&ctx, &user, &id).await?;
    ctx.roles()
        .check(&user, &s.workspace_id, WorkspaceRole::Editor)
        .await?;
    let judge = ctx.agent_runtime().synthesize_council(&id, req).await?;
    Ok((StatusCode::CREATED, Json(judge)))
}

async fn get_one<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RuntimeSession>> {
    Ok(Json(authorized(&ctx, &user, &id).await?))
}

async fn prompt<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<RuntimePromptReq>,
) -> ApiResult<(StatusCode, Json<RuntimePromptResp>)> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    ctx.agent_runtime().person_prompted(&id).await;
    let r = ctx.agent_runtime().prompt_with(&id, &req).await?;
    Ok((StatusCode::ACCEPTED, Json(r)))
}

#[derive(serde::Serialize)]
struct InterruptResp {
    interrupted: bool,
    /// Background work still running: the turn's Stop leaves it alone
    /// (`POST …/tasks/stop` stops it).
    background_tasks: usize,
}

async fn interrupt<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<Json<InterruptResp>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    let interrupted = ctx.agent_runtime().interrupt(&id).await?;
    let background_tasks = ctx.agent_runtime().background_count(&id).await?;
    Ok(Json(InterruptResp {
        interrupted,
        background_tasks,
    }))
}

async fn answer<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, request_id)): Path<(Id, String)>,
    Json(req): Json<RuntimeRequestRespondReq>,
) -> ApiResult<StatusCode> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    match req {
        RuntimeRequestRespondReq::Approval { decision } => {
            ctx.agent_runtime()
                .respond(&id, &request_id, decision)
                .await?
        }
        RuntimeRequestRespondReq::DismissQuestion { question_action } => {
            ctx.agent_runtime()
                .dismiss_question(&id, &request_id, question_action)
                .await?
        }
        RuntimeRequestRespondReq::Question { answers } => {
            ctx.agent_runtime()
                .respond_question(&id, &request_id, answers)
                .await?
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn rename<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateRuntimeSessionReq>,
) -> ApiResult<Json<RuntimeSession>> {
    require_human(&ctx, &auth)?;
    let s = authorized(&ctx, &user, &id).await?;
    if let Some(meta) = &req.meta {
        // Client-owned thread keys only (pinned / History / seen at); writing
        // them is an editor's act, like any other change to the session.
        validate_thread_meta(meta).map_err(|e| ApiErr(Error::Invalid(e)))?;
        ctx.roles()
            .check(&user, &s.workspace_id, WorkspaceRole::Editor)
            .await?;
    }
    let renamed = match &req.title {
        Some(title) => Some(ctx.agent_runtime().rename(&id, title).await?),
        None => None,
    };
    match req.meta {
        Some(meta) => Ok(Json(
            ctx.agent_runtime().update_thread_meta(&id, meta).await?,
        )),
        None => renamed
            .map(Json)
            .ok_or_else(|| ApiErr(Error::Invalid("give a title and/or meta".into()))),
    }
}

async fn close<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RuntimeSession>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    Ok(Json(ctx.agent_runtime().close(&id).await?))
}

/// POST /runtime-sessions/{id}/reopen — a closed thread takes prompts again
/// (its agent respawns immediately; MCP/resume readiness completes asynchronously).
async fn reopen<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<Json<RuntimeSession>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    Ok(Json(ctx.agent_runtime().reopen_fast(&id).await?))
}

/// Body of `POST /runtime-sessions/{id}/resume-at`.
#[derive(Deserialize)]
struct ResumeAtReq {
    /// When the usage limit resets; `null` cancels the armed resume.
    at: Option<chrono::DateTime<chrono::Utc>>,
}

/// POST /runtime-sessions/{id}/resume-at — arm the thread's ONE resume when
/// its provider's usage limit resets (kept in its meta, so a restart keeps
/// it), or cancel it. At most a week ahead.
async fn resume_at<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
    Json(req): Json<ResumeAtReq>,
) -> ApiResult<Json<RuntimeSession>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    if req
        .at
        .is_some_and(|t| t > chrono::Utc::now() + chrono::Duration::days(7))
    {
        return Err(Error::Invalid("a resume can be at most a week ahead".into()).into());
    }
    Ok(Json(ctx.agent_runtime().set_resume_at(&id, req.at).await?))
}

/// DELETE /runtime-sessions/{id} — stop its agent and remove it with its history.
async fn delete_one<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    ctx.agent_runtime().delete(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /runtime-sessions/{id}/queue/{queued_id} — deliver a queued message
/// now (`steer` into the running turn, or `interrupt` it and send next).
async fn deliver_queued<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, queued_id)): Path<(Id, String)>,
    Json(req): Json<QueuedDeliverReq>,
) -> ApiResult<(StatusCode, Json<RuntimePromptResp>)> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    ctx.agent_runtime().person_prompted(&id).await;
    let r = ctx
        .agent_runtime()
        .deliver_queued(&id, &queued_id, req.delivery)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(r)))
}

/// DELETE /runtime-sessions/{id}/queue/{queued_id} — drop a queued message
/// and return its text + attachments so Edit can restore them into the composer.
async fn unqueue<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path((id, queued_id)): Path<(Id, String)>,
) -> ApiResult<Json<UnqueuePromptResp>> {
    require_human(&ctx, &auth)?;
    authorized(&ctx, &user, &id).await?;
    if let Some(req) = ctx.agent_runtime().unqueue_prompt(&id, &queued_id).await? {
        Ok(Json(UnqueuePromptResp {
            ok: true,
            text: Some(req.text),
            images: req.images,
            files: req.files,
        }))
    } else {
        Err(ApiErr(Error::NotFound(
            "that message is no longer queued".into(),
        )))
    }
}

/// GET /runtime-sessions/{id}/attachments/{attachment_id} — the raw bytes.
async fn attachment<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, attachment_id)): Path<(Id, String)>,
) -> ApiResult<Response> {
    authorized(&ctx, &user, &id).await?;
    let (mime, bytes) = ctx.agent_runtime().attachment(&id, &attachment_id).await?;
    use axum::http::header;
    // The mime is the uploader's (`image/svg+xml` can carry script): never
    // run it on the daemon's origin.
    let headers = [
        (header::CONTENT_TYPE, mime),
        (header::CONTENT_SECURITY_POLICY, "sandbox".into()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
    ];
    Ok((headers, bytes).into_response())
}

/// GET /runtime-sessions/{id}/items/{item_id}/images/{n} — an image a tool
/// left on disk (Codex `imageView` / `imageGeneration`), read-only.
async fn tool_image<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, item_id, n)): Path<(Id, String, usize)>,
) -> ApiResult<Response> {
    authorized(&ctx, &user, &id).await?;
    let (mime, bytes) = ctx.agent_runtime().tool_image(&id, &item_id, n).await?;
    use axum::http::header;
    let headers = [
        (header::CONTENT_TYPE, mime),
        (header::CONTENT_SECURITY_POLICY, "sandbox".into()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
    ];
    Ok((headers, bytes).into_response())
}

/// GET /runtime-sessions/{id}/tasks/{task_id}/output — a Claude background
/// command's output so far (`{text, truncated}`); 404 for other agents, an
/// unknown task, or output Claude no longer keeps.
async fn task_output<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, task_id)): Path<(Id, String)>,
) -> ApiResult<Json<crate::native_work::TaskOutput>> {
    let session = authorized(&ctx, &user, &id).await?;
    let gone = || ApiErr(Error::NotFound("no output for that task".into()));
    let native = session
        .native_thread_id
        .filter(|_| session.provider == otto_core::agent_runtime::RuntimeProvider::Claude)
        .ok_or_else(gone)?;
    let tmp = std::env::var_os("CLAUDE_CODE_TMPDIR")
        .map_or_else(|| "/tmp".into(), std::path::PathBuf::from);
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    tokio::task::spawn_blocking(move || {
        crate::native_work::claude_task_output(&tmp, uid, &native, &task_id)
    })
    .await
    .ok()
    .flatten()
    .map(Json)
    .ok_or_else(gone)
}

#[derive(Deserialize)]
struct SwarmQuery {
    member: Option<String>,
}

/// GET /runtime-sessions/{id}/swarm?member= — an Antigravity thread's swarm
/// (members, their board posts, `member`'s own steps); 404 without one.
async fn swarm<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<SwarmQuery>,
) -> ApiResult<Json<crate::native_work::Swarm>> {
    let session = authorized(&ctx, &user, &id).await?;
    let gone = || ApiErr(Error::NotFound("this thread has no swarm".into()));
    let native = session
        .native_thread_id
        .filter(|_| session.provider == otto_core::agent_runtime::RuntimeProvider::Agy)
        .ok_or_else(gone)?;
    let brain =
        std::path::PathBuf::from(crate::acp_sideband::default_agy_brain_dir().ok_or_else(gone)?);
    tokio::task::spawn_blocking(move || {
        crate::native_work::agy_swarm(&brain, &native, q.member.as_deref())
    })
    .await
    .ok()
    .flatten()
    .map(Json)
    .ok_or_else(gone)
}

#[derive(Default, Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after_sequence: u64,
    #[serde(default)]
    tail_turns: Option<usize>,
    #[serde(default)]
    before_turn_ordinal: Option<u32>,
    #[serde(default)]
    limit_turns: Option<usize>,
    #[serde(default)]
    max_lines: Option<usize>,
    #[serde(default)]
    compact: Option<String>,
}

/// GET /runtime-sessions/{id}/events?after_sequence= — events, tail snapshot, or older turn page.
async fn events<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<Json<RuntimeReplay>> {
    authorized(&ctx, &user, &id).await?;
    let compact = q
        .compact
        .as_deref()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    Ok(Json(
        ctx.agent_runtime()
            .replay_paged_opts(
                &id,
                q.after_sequence,
                q.tail_turns,
                q.before_turn_ordinal,
                q.limit_turns,
                compact,
                q.max_lines,
            )
            .await?,
    ))
}

/// GET /runtime-sessions/{id}/items/{item_id} — one full (uncompacted) TurnItem on demand.
async fn get_item<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path((id, item_id)): Path<(Id, Id)>,
) -> ApiResult<Json<TurnItem>> {
    authorized(&ctx, &user, &id).await?;
    Ok(Json(ctx.agent_runtime().get_item(&id, &item_id).await?))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{
        AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig,
        RuntimeHost,
    };
    use chrono::Utc;
    use otto_core::agent_runtime::{RuntimeAccessMode, RuntimeEvent, RuntimeProvider};
    use otto_core::auth::{BoxFuture, RoleChecker};
    use otto_core::domain::Workspace;
    use otto_state::{AgentRuntimeRepo, DbPool, NewRuntimeSession, UsersRepo, WorkspacesRepo};

    struct MemberNotAdminRoles;
    impl RoleChecker for MemberNotAdminRoles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _ws: &'a Id,
            min_role: WorkspaceRole,
        ) -> BoxFuture<'a, otto_core::Result<()>> {
            Box::pin(async move {
                if min_role == WorkspaceRole::Admin {
                    Err(Error::Forbidden("not a workspace admin".into()))
                } else {
                    Ok(())
                }
            })
        }
    }

    struct DummyHost(String);
    #[async_trait::async_trait]
    impl RuntimeHost for DummyHost {
        async fn workspace(&self, id: &Id) -> otto_core::Result<Workspace> {
            Ok(Workspace {
                id: id.clone(),
                name: "W".into(),
                root_path: self.0.clone(),
                settings: serde_json::json!({}),
                archived: false,
                created_at: Utc::now(),
            })
        }
        async fn sandboxed(&self, _s: &RuntimeSession) -> bool {
            false
        }
        async fn prepare_spawn(
            &self,
            _ws: &Workspace,
            _s: &RuntimeSession,
            _spec: &mut LaunchSpec,
            _mcp: McpDelivery,
        ) -> otto_core::Result<Vec<McpServerDef>> {
            Ok(Vec::new())
        }
        fn publish(&self, _s: &RuntimeSession, _event: Arc<RuntimeEvent>) {}
    }

    #[derive(Clone)]
    struct TestRuntimeCtx {
        rt: AgentRuntime,
        roles: Arc<dyn RoleChecker>,
    }

    impl RuntimeCtx for TestRuntimeCtx {
        fn agent_runtime(&self) -> &AgentRuntime {
            &self.rt
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn is_human(&self, auth: &AuthContext) -> bool {
            !auth.is_scoped()
        }
    }

    /// Editor (or only Viewer) everywhere, never Admin.
    struct Roles {
        editor: bool,
    }
    impl RoleChecker for Roles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _ws: &'a Id,
            min_role: WorkspaceRole,
        ) -> BoxFuture<'a, otto_core::Result<()>> {
            let ok = match min_role {
                WorkspaceRole::Admin => false,
                WorkspaceRole::Editor => self.editor,
                _ => true,
            };
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(Error::Forbidden("role".into()))
                }
            })
        }
    }

    /// A host that keeps what the runtime published (the WS fan-out).
    struct RecHost(String, std::sync::Mutex<Vec<Arc<RuntimeEvent>>>);
    #[async_trait::async_trait]
    impl RuntimeHost for RecHost {
        async fn workspace(&self, id: &Id) -> otto_core::Result<Workspace> {
            DummyHost(self.0.clone()).workspace(id).await
        }
        async fn sandboxed(&self, _s: &RuntimeSession) -> bool {
            false
        }
        async fn prepare_spawn(
            &self,
            _ws: &Workspace,
            _s: &RuntimeSession,
            _spec: &mut LaunchSpec,
            _mcp: McpDelivery,
        ) -> otto_core::Result<Vec<McpServerDef>> {
            Ok(Vec::new())
        }
        fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
            self.1.lock().unwrap().push(event);
        }
    }

    fn auth(user: &User, scoped: bool) -> AuthContext {
        let mut a = AuthContext::automation(user, "test");
        if scoped {
            a.scope = Some(otto_core::auth::SessionScope {
                session_id: "s".into(),
                role: WorkspaceRole::Editor,
                otp_pending: false,
            });
        }
        a
    }

    async fn patch(
        ctx: &TestRuntimeCtx,
        user: &User,
        scoped: bool,
        id: &Id,
        body: serde_json::Value,
    ) -> Result<RuntimeSession, StatusCode> {
        rename(
            State(ctx.clone()),
            Extension(AuthUser(user.clone())),
            Extension(auth(user, scoped)),
            Path(id.clone()),
            Json(serde_json::from_value(body).unwrap()),
        )
        .await
        .map(|Json(s)| s)
        .map_err(|e| e.into_response().status())
    }

    #[tokio::test]
    async fn thread_meta_patch_allow_list_rbac_and_live_event() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap().to_string();
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let raw_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&raw_pool)
            .await
            .unwrap();
        let pool: DbPool = raw_pool.into();
        let users = UsersRepo::new(pool.clone());
        let owner = users.create("owner", "Owner", "h", false).await.unwrap();
        let other = users.create("other", "Other", "h", false).await.unwrap();
        let ws = WorkspacesRepo::new(pool.clone())
            .create("ws", &cwd, &owner.id)
            .await
            .unwrap();
        let host = Arc::new(RecHost(cwd.clone(), Default::default()));
        let drivers: Arc<dyn DriverFactory> = Arc::new(|_| None);
        let rt = AgentRuntime::new(
            AgentRuntimeRepo::new(pool.clone()),
            host.clone(),
            drivers,
            RuntimeConfig {
                attachments_dir: dir.path().join("att"),
                ..RuntimeConfig::default()
            },
        );
        let s = rt
            .repo()
            .create(NewRuntimeSession {
                id: otto_core::new_id(),
                workspace_id: ws.id,
                provider: RuntimeProvider::Claude,
                title: "t".into(),
                cwd,
                model: None,
                access_mode: RuntimeAccessMode::FullAccess,
                created_by: owner.id.clone(),
                parent_id: None,
                meta: Some(serde_json::json!({"source": "pr-agent"})),
            })
            .await
            .unwrap()
            .session;
        let ctx = TestRuntimeCtx {
            rt,
            roles: Arc::new(Roles { editor: true }),
        };

        // Only the client-owned keys, with their types: 400 before anything is written.
        for bad in [
            serde_json::json!({"meta": {"source": "connection-diagnose"}}),
            serde_json::json!({"meta": {"thread_pinned": "yes"}}),
            serde_json::json!({"meta": {"thread_seen_at": "yesterday"}}),
            serde_json::json!({"meta": {}}),
            serde_json::json!({}),
        ] {
            assert_eq!(
                patch(&ctx, &owner, false, &s.id, bad).await.unwrap_err(),
                StatusCode::BAD_REQUEST
            );
        }
        // Not the owner, a share / guest credential, a viewer: no.
        let pin = serde_json::json!({"meta": {"thread_pinned": true}});
        assert_eq!(
            patch(&ctx, &other, false, &s.id, pin.clone())
                .await
                .unwrap_err(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            patch(&ctx, &owner, true, &s.id, pin.clone())
                .await
                .unwrap_err(),
            StatusCode::FORBIDDEN
        );
        let viewer = TestRuntimeCtx {
            rt: ctx.rt.clone(),
            roles: Arc::new(Roles { editor: false }),
        };
        assert_eq!(
            patch(&viewer, &owner, false, &s.id, pin.clone())
                .await
                .unwrap_err(),
            StatusCode::FORBIDDEN
        );
        assert!(host.1.lock().unwrap().is_empty());

        // The owner: merged beside the server-owned meta, published live, persisted.
        let at = "2026-11-23T10:00:00Z";
        let got = patch(
            &ctx,
            &owner,
            false,
            &s.id,
            serde_json::json!({"meta": {"thread_pinned": true, "thread_seen_at": at}}),
        )
        .await
        .unwrap();
        assert_eq!(
            got.meta,
            Some(
                serde_json::json!({"source": "pr-agent", "thread_pinned": true, "thread_seen_at": at})
            )
        );
        let thread_of = |e: &RuntimeEvent| match &e.body {
            otto_core::agent_runtime::RuntimeEventBody::ProviderSessionUpdated {
                provider_session,
            } => provider_session.thread.clone(),
            _ => None,
        };
        let last = host.1.lock().unwrap().last().cloned().expect("an event");
        assert_eq!(
            thread_of(&last),
            Some(serde_json::json!({"thread_pinned": true, "thread_seen_at": at}))
        );
        // null clears a key; a title and meta together both land.
        let got = patch(&ctx, &owner, false, &s.id, serde_json::json!({"title": "Renamed", "meta": {"thread_pinned": null, "thread_history": true}})).await.unwrap();
        assert_eq!(got.title, "Renamed");
        assert_eq!(
            got.meta,
            Some(
                serde_json::json!({"source": "pr-agent", "thread_history": true, "thread_seen_at": at})
            )
        );
        let row = AgentRuntimeRepo::new(pool)
            .get(&s.id)
            .await
            .unwrap()
            .session;
        assert_eq!(row.meta, got.meta);
    }

    #[tokio::test]
    async fn resume_at_arms_cancels_and_a_restart_re_arms_it() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap().to_string();
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let raw_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&raw_pool)
            .await
            .unwrap();
        let pool: DbPool = raw_pool.into();
        let users = UsersRepo::new(pool.clone());
        let owner = users.create("owner", "Owner", "h", false).await.unwrap();
        let other = users.create("other", "Other", "h", false).await.unwrap();
        let ws = WorkspacesRepo::new(pool.clone())
            .create("ws", &cwd, &owner.id)
            .await
            .unwrap();
        let host = Arc::new(RecHost(cwd.clone(), Default::default()));
        let drivers: Arc<dyn DriverFactory> = Arc::new(|_| None);
        let rt = AgentRuntime::new(
            AgentRuntimeRepo::new(pool.clone()),
            host.clone(),
            drivers,
            RuntimeConfig {
                attachments_dir: dir.path().join("att"),
                ..RuntimeConfig::default()
            },
        );
        let s = rt
            .repo()
            .create(NewRuntimeSession {
                id: otto_core::new_id(),
                workspace_id: ws.id,
                provider: RuntimeProvider::Codex,
                title: "t".into(),
                cwd,
                model: None,
                access_mode: RuntimeAccessMode::FullAccess,
                created_by: owner.id.clone(),
                parent_id: None,
                meta: None,
            })
            .await
            .unwrap()
            .session;
        let ctx = TestRuntimeCtx {
            rt: rt.clone(),
            roles: Arc::new(Roles { editor: true }),
        };
        let call = |user: &User, at: Option<chrono::DateTime<chrono::Utc>>| {
            resume_at(
                State(ctx.clone()),
                Extension(AuthUser(user.clone())),
                Extension(auth(user, false)),
                Path(s.id.clone()),
                Json(ResumeAtReq { at }),
            )
        };
        let soon = chrono::Utc::now() + chrono::Duration::hours(2);
        let err = |r: ApiResult<Json<RuntimeSession>>| r.err().map(|e| e.into_response().status());
        assert_eq!(
            err(call(&other, Some(soon)).await),
            Some(StatusCode::FORBIDDEN)
        );
        let far = chrono::Utc::now() + chrono::Duration::days(8);
        assert_eq!(
            err(call(&owner, Some(far)).await),
            Some(StatusCode::BAD_REQUEST)
        );

        // Armed: on the meta, and sent to clients with the thread keys.
        let Json(got) = call(&owner, Some(soon)).await.ok().unwrap();
        let stored = got
            .meta
            .as_ref()
            .and_then(|m| m.get(otto_core::agent_runtime::RESUME_AT))
            .and_then(|v| v.as_str())
            .map(|v| chrono::DateTime::parse_from_rfc3339(v).unwrap().timestamp());
        assert_eq!(stored, Some(soon.timestamp()));
        let last = host.1.lock().unwrap().last().cloned().expect("an event");
        let thread = match &last.body {
            otto_core::agent_runtime::RuntimeEventBody::ProviderSessionUpdated {
                provider_session,
            } => provider_session.thread.clone(),
            _ => None,
        };
        assert!(thread.and_then(|t| t.get("resume_at").cloned()).is_some());
        // Cancelled: gone.
        let Json(got) = call(&owner, None).await.ok().unwrap();
        assert_eq!(got.meta, None);

        // A resume already due when the daemon starts fires at boot: this
        // thread's last turn did not end on the limit, so it is just cleared.
        let past = (chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339();
        rt.repo()
            .patch_meta(&s.id, &serde_json::json!({ "resume_at": past }))
            .await
            .unwrap();
        rt.recover().await.unwrap();
        for _ in 0..100 {
            let meta = rt.repo().get(&s.id).await.unwrap().session.meta;
            if meta.as_ref().and_then(|m| m.get("resume_at")).is_none() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("the due resume was not re-armed after recover");
    }

    #[tokio::test]
    async fn get_item_and_events_enforce_session_owner_or_admin() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_str().unwrap().to_string();
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let raw_pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&raw_pool)
            .await
            .unwrap();
        let pool: DbPool = raw_pool.into();
        let users = UsersRepo::new(pool.clone());
        let owner = users.create("owner", "Owner", "h", false).await.unwrap();
        let other = users.create("other", "Other", "h", false).await.unwrap();
        let ws = WorkspacesRepo::new(pool.clone())
            .create("ws", &cwd, &owner.id)
            .await
            .unwrap();
        let drivers: Arc<dyn DriverFactory> = Arc::new(|_| None);
        let rt = AgentRuntime::new(
            AgentRuntimeRepo::new(pool),
            Arc::new(DummyHost(cwd.clone())),
            drivers,
            RuntimeConfig {
                attachments_dir: dir.path().join("att"),
                ..RuntimeConfig::default()
            },
        );
        let s = rt
            .repo()
            .create(NewRuntimeSession {
                id: otto_core::new_id(),
                workspace_id: ws.id,
                provider: RuntimeProvider::Claude,
                title: "t".into(),
                cwd,
                model: None,
                access_mode: RuntimeAccessMode::FullAccess,
                created_by: owner.id.clone(),
                parent_id: None,
                meta: None,
            })
            .await
            .unwrap()
            .session;
        let ctx = TestRuntimeCtx {
            rt,
            roles: Arc::new(MemberNotAdminRoles),
        };

        // Non-owner workspace member is rejected with 403 Forbidden before item lookup.
        let err = match get_item(
            State(ctx.clone()),
            Extension(AuthUser(other.clone())),
            Path((s.id.clone(), "any-item".into())),
        )
        .await
        {
            Ok(_) => panic!("expected Forbidden for non-owner"),
            Err(e) => e,
        };
        assert_eq!(err.into_response().status(), StatusCode::FORBIDDEN);

        let ev_err = match events(
            State(ctx.clone()),
            Extension(AuthUser(other)),
            Path(s.id.clone()),
            Query(EventsQuery {
                compact: Some("1".into()),
                max_lines: Some(10_000),
                ..EventsQuery::default()
            }),
        )
        .await
        {
            Ok(_) => panic!("expected Forbidden for non-owner events"),
            Err(e) => e,
        };
        assert_eq!(ev_err.into_response().status(), StatusCode::FORBIDDEN);

        // Owner passes authz and receives 404 NotFound for an unknown item ID.
        let miss = match get_item(
            State(ctx),
            Extension(AuthUser(owner)),
            Path((s.id, "missing-item".into())),
        )
        .await
        {
            Ok(_) => panic!("expected NotFound for missing item"),
            Err(e) => e,
        };
        assert_eq!(miss.into_response().status(), StatusCode::NOT_FOUND);
    }
}
