//! `/runtime-sessions/*` REST router (contract: `docs/contracts/api.md`
//! "Structured agent runtime"). Page-level authorization is the server's
//! policy table (`Agents` View for reads, Edit for writes); the handlers add:
//!
//! * workspace `viewer`+ membership, then owner-or-admin on the session (the
//!   same rule as PTY sessions' `require_session_owner_or_admin`);
//! * every write requires a PERSON's credential — an agent session's token,
//!   an MCP or share credential may not open sessions, drive them, or answer
//!   permission prompts (an agent approving its own tool call is exactly
//!   what the prompt exists to prevent).

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use otto_core::agent_runtime::{
    CreateRuntimeSessionReq, RuntimePromptReq, RuntimePromptResp, RuntimeReplay,
    RuntimeRequestRespondReq, RuntimeSession, UpdateRuntimeSessionReq,
};
use otto_core::api::Problem;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};
use serde::Deserialize;

use crate::RuntimeCtx;

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
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// REST routes; the server nests this under `/api/v1` and supplies the state.
pub fn api_router<S: RuntimeCtx>() -> Router<S> {
    Router::new()
        .route(
            "/workspaces/{id}/runtime-sessions",
            get(list::<S>).post(create::<S>),
        )
        .route(
            "/runtime-sessions/{id}",
            get(get_one::<S>).patch(rename::<S>),
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
        .route("/runtime-sessions/{id}/events", get(events::<S>))
        .route(
            "/runtime-sessions/{id}/attachments/{attachment_id}",
            get(attachment::<S>),
        )
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

/// GET /workspaces/{id}/runtime-sessions — the caller's own (all, for an admin).
async fn list<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(ws): Path<Id>,
) -> ApiResult<Json<Vec<RuntimeSession>>> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Viewer).await?;
    let owner = (!is_ws_admin(&ctx, &user, &ws).await).then_some(&user.id);
    Ok(Json(ctx.agent_runtime().list(&ws, owner).await?))
}

/// POST /workspaces/{id}/runtime-sessions — create + spawn (editor+).
async fn create<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws): Path<Id>,
    Json(req): Json<CreateRuntimeSessionReq>,
) -> ApiResult<(StatusCode, Json<RuntimeSession>)> {
    require_human(&ctx, &auth)?;
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    let s = ctx.agent_runtime().create(&user.id, &ws, req).await?;
    Ok((StatusCode::CREATED, Json(s)))
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
    let r = ctx.agent_runtime().prompt_with(&id, &req).await?;
    Ok((StatusCode::ACCEPTED, Json(r)))
}

#[derive(serde::Serialize)]
struct InterruptResp {
    interrupted: bool,
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
    Ok(Json(InterruptResp { interrupted }))
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
    ctx.agent_runtime()
        .respond(&id, &request_id, req.decision)
        .await?;
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
    authorized(&ctx, &user, &id).await?;
    Ok(Json(ctx.agent_runtime().rename(&id, &req.title).await?))
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

#[derive(Deserialize)]
struct EventsQuery {
    #[serde(default)]
    after_sequence: u64,
}

/// GET /runtime-sessions/{id}/events?after_sequence= — events or a snapshot.
async fn events<S: RuntimeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<Json<RuntimeReplay>> {
    authorized(&ctx, &user, &id).await?;
    Ok(Json(
        ctx.agent_runtime().replay(&id, q.after_sequence).await?,
    ))
}
