//! Host context + HTTP plumbing for the Design Hall assist engine.
//!
//! The artifact graph comes from [`DesignCtx`] (service handle, roles, the
//! host's deep format check); the agent turn itself runs in the host
//! (otto-server owns the session manager + turn runner) behind
//! [`DesignAssistCtx::run_agent_turn`]. The error type and the `CurrentUser`
//! extractor mirror otto-server's so the moved handlers answer byte-for-byte
//! the same (401 when unauthenticated, 500s logged).

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
use otto_design::DesignCtx;
use otto_improve::ImprovementEngine;
use otto_memory::MemoryService;
use otto_state::{ProductRepo, WorkspacesRepo};
use serde_json::Value;
use tokio::sync::broadcast;

/// One agent turn the host runs for a design assist / variant.
pub struct AgentTurn<'a> {
    pub ws: &'a Workspace,
    pub user: &'a User,
    /// The artifact's session to RESUME (`None` → spawn a fresh one).
    pub existing: Option<&'a Id>,
    pub title: &'a str,
    pub cwd: &'a str,
    pub provider: &'a str,
    pub meta: Value,
    pub prompt: &'a str,
    /// No-output idle trip.
    pub stuck_after: Duration,
    /// Completion marker file: the turn is done once it holds non-empty text.
    pub done_file: Option<PathBuf>,
    /// Quiet fallback for non-transcript providers (PTY silence = done).
    pub quiet_done: Option<Duration>,
}

/// What the design assist engine needs from the host beyond [`DesignCtx`].
pub trait DesignAssistCtx: DesignCtx {
    fn events(&self) -> &broadcast::Sender<Event>;
    fn workspaces(&self) -> &WorkspacesRepo;
    /// Stories (acceptance criteria) for the context brief.
    fn product_repo(&self) -> &ProductRepo;
    /// Team rules (`design-team-style`) + the learned-rule proposals.
    fn improve_engine(&self) -> &Arc<ImprovementEngine>;
    /// The workspace `design` memory collection for the context brief.
    fn memory(&self) -> &Arc<MemoryService>;
    /// Requested → workspace → global default agent; never fails (logs + falls
    /// back), tagged with `site` in the log.
    fn resolve_provider_or_fallback(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
        site: &'static str,
    ) -> impl Future<Output = String> + Send;
    /// Run ONE agent turn and return `(reply_text, session_id)`. `on_ready`
    /// fires the moment the session exists.
    fn run_agent_turn<F: FnOnce(&Id) + Send>(
        &self,
        turn: AgentTurn<'_>,
        on_ready: F,
    ) -> impl Future<Output = otto_core::Result<(String, Id)>> + Send;
    fn kill_session(&self, sid: &Id) -> impl Future<Output = otto_core::Result<()>> + Send;
}

/// Require a workspace role (root short-circuits inside the checker).
pub(crate) async fn require_ws_role<C: DesignCtx>(
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
