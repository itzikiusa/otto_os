//! Host context + HTTP plumbing for the insights engine.
//!
//! The engine spawns a headless agent session and pastes the skill prompt into
//! its TUI; the session manager is a narrow handle, the TUI-readiness/paste
//! dance stays in the host (otto-server's `review_session`) behind
//! [`InsightsCtx::submit_prompt`]. The error type and the `CurrentUser`
//! extractor mirror otto-server's so the moved handlers answer byte-for-byte
//! the same (401 when unauthenticated, 500s logged).

use std::future::Future;
use std::sync::Arc;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::{User, Workspace};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_sessions::SessionManager;
use otto_state::WorkspacesRepo;
use tokio::sync::broadcast;

/// What the insights engine needs from the host daemon.
pub trait InsightsCtx: Clone + Send + Sync + 'static {
    /// The skill Library — its root's parent is the Otto data dir (where the
    /// `insights/` tree lives); `skill_path` says whether the skill is installed.
    fn library(&self) -> &otto_context::Library;
    fn manager(&self) -> &Arc<SessionManager>;
    fn events(&self) -> &broadcast::Sender<Event>;
    fn workspaces(&self) -> &WorkspacesRepo;
    /// Workspace → global default agent when `requested` is empty.
    fn resolve_provider(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
    ) -> impl Future<Output = otto_core::Result<String>> + Send;
    /// Wait for the session's TUI to draw + settle, paste `prompt` and press
    /// Enter. `false` when the TUI never became ready (nothing was sent).
    fn submit_prompt(&self, sid: &Id, prompt: &str) -> impl Future<Output = bool> + Send;
}

/// Run synchronous std::fs work on the blocking pool, re-raising a panic on
/// the caller (the one shared copy lives in otto-agent-run).
pub(crate) use otto_agent_run::offload::blocking;

/// Require the global root role.
pub(crate) fn require_root(user: &User) -> Result<(), ApiError> {
    if otto_core::auth::root_authority(user) {
        Ok(())
    } else {
        Err(ApiError(otto_core::auth::root_refusal()))
    }
}

/// The authenticated (effective) user; 401 when the auth middleware left none.
pub(crate) struct CurrentUser(pub User);

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
pub(crate) struct ApiError(pub Error);

pub(crate) type ApiResult<T> = std::result::Result<T, ApiError>;

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
