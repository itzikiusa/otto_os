//! `/secret-requests` routes: thin handlers over the runtime's private
//! secret requests (`otto_agent_protocol::runtime` `secret_request`,
//! T3 Code #15907). The value is never logged, stored or put in an event.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use otto_agent_protocol::{answer_secret_request, pending_secret_requests};
use otto_core::{Error, Id};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::CurrentAuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// `GET /secret-requests` — the caller's pending requests.
pub async fn list(CurrentAuthContext(auth): CurrentAuthContext) -> ApiResult<Json<Value>> {
    crate::auth::require_human(&auth)?;
    let items = pending_secret_requests(&auth.effective_user.id);
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
pub struct AnswerReq {
    /// The secret; absent (or `decline: true`) declines.
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    decline: bool,
}

/// `POST /secret-requests/{id}` — the thread's owner answers, with their own
/// sign-in (not an agent's, not an impersonation). **404** once settled.
pub async fn answer(
    State(_ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<AnswerReq>,
) -> ApiResult<StatusCode> {
    crate::auth::require_human(&auth)?;
    let value =
        if req.decline {
            None
        } else {
            Some(req.value.filter(|v| !v.trim().is_empty()).ok_or_else(|| {
                ApiError(Error::Invalid("give `value`, or `decline: true`".into()))
            })?)
        };
    answer_secret_request(&id, &auth.effective_user.id, &auth.real_user.id, value)?;
    Ok(StatusCode::NO_CONTENT)
}
