//! Host context + HTTP plumbing for the agent-assist endpoints ([`crate::assist`]).
//!
//! The assist turn runs an agent session, which only the host daemon can do
//! (session manager + turn runner live in otto-server), so the host hands that
//! in through [`CanvasAssistCtx`]. The error type and the `CurrentUser`
//! extractor mirror otto-server's so the moved handlers answer byte-for-byte
//! the same (401 when unauthenticated, 500s logged).

use std::future::Future;
use std::path::Path;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::{User, Workspace, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::WorkspacesRepo;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::http::CanvasCtx;

/// One agent turn the host runs for an assist request.
pub struct AgentTurn<'a> {
    pub ws: &'a Workspace,
    pub user: &'a User,
    /// The scene's session to RESUME (`None` → spawn a fresh one).
    pub existing: Option<&'a Id>,
    pub title: &'a str,
    pub cwd: &'a str,
    pub provider: &'a str,
    pub meta: Value,
    pub prompt: &'a str,
}

/// What the canvas assist endpoints need from the host beyond [`CanvasCtx`].
pub trait CanvasAssistCtx: CanvasCtx {
    fn workspaces(&self) -> &WorkspacesRepo;
    fn events(&self) -> &broadcast::Sender<Event>;
    /// Otto's data dir (scene scratch dirs live under `<data>/canvas/<id>`).
    fn data_dir(&self) -> &Path;
    /// Workspace → global default agent when `requested` is empty.
    fn resolve_provider(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
    ) -> impl Future<Output = otto_core::Result<String>> + Send;
    /// Pre-trust `cwd` for `provider` so the PTY doesn't stall on a first-run
    /// trust prompt.
    fn ensure_trusted(&self, provider: &str, cwd: &str);
    /// Run ONE interactive agent turn (the long idle backstop) and return
    /// `(reply_text, session_id)`. `on_ready` fires the moment the session exists.
    fn run_agent_turn<F: FnOnce(&Id) + Send>(
        &self,
        turn: AgentTurn<'_>,
        on_ready: F,
    ) -> impl Future<Output = otto_core::Result<(String, Id)>> + Send;
    fn kill_session(&self, sid: &Id) -> impl Future<Output = otto_core::Result<()>> + Send;
}

/// Require a workspace role (root short-circuits inside the checker).
pub(crate) async fn require_ws_role<C: CanvasCtx>(
    ctx: &C,
    user: &User,
    ws_id: &Id,
    min: WorkspaceRole,
) -> Result<(), ApiError> {
    ctx.roles().check(user, ws_id, min).await.map_err(ApiError)
}

/// The authenticated (effective) user; 401 when the auth middleware left none.
pub struct CurrentUser(pub User);

impl<S: Send + Sync> FromRequestParts<S> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .map(|a| CurrentUser(a.0.clone()))
            .ok_or(ApiError(Error::Unauthorized))
    }
}

#[derive(Debug)]
pub struct ApiError(pub Error);

pub type ApiResult<T> = std::result::Result<T, ApiError>;

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
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
            tracing::error!("internal error: {}", self.0);
        }
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}
