//! Agent UI control — HTTP surface (see `crate::ui_bridge`, docs/contracts/api.md
//! "Agent UI control").
//!
//! - `GET  /ui/commands/catalog` — the command catalog (`ui-commands.json`).
//! - `POST /ui/commands/{id}/result` — an Otto window reports a command's outcome.
//! - `POST /ui/commands/{id}/progress` — a note; `awaiting_human` extends the deadline.
//! - `POST /sessions/{id}/ui-control` — grant / revoke UI control for a session.
//!
//! The three POSTs are HUMAN-ONLY: an agent session's own credential (which is
//! the owner's full API token, and which the agent can `curl` with from its
//! shell) must never answer its own commands or grant itself control. Every
//! one rejects `managed_session_id` / `mcp_session_id` / `mcp_only` / share
//! tokens ([`crate::ui_bridge::is_human`]). The result/progress routes further
//! require the caller to be the command's target user AND the `X-Otto-Ui-Conn`
//! header to name the window connection the command was sent to.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::auth::AuthContext;
use otto_core::domain::Session;
use otto_core::{Error, Id};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::CurrentAuthContext;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use crate::ui_bridge::{self, UiError, UiReply};

/// The header naming the `/ws/events` connection (from `hello_ack`).
pub const CONN_HEADER: &str = "x-otto-ui-conn";

/// Largest accepted result body (a result page is capped at 200 rows anyway).
pub const MAX_RESULT_BYTES: usize = 1024 * 1024;

fn require_human(auth: &AuthContext, what: &str) -> ApiResult<()> {
    if ui_bridge::is_human(auth) {
        return Ok(());
    }
    Err(ApiError(Error::Forbidden(format!(
        "{what} needs a person's own Otto sign-in; an agent session's or MCP credential cannot do it"
    ))))
}

fn conn_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(CONN_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `GET /ui/commands/catalog` — the parsed catalog, verbatim.
pub async fn catalog() -> Response {
    match serde_json::from_str::<Value>(crate::ui_commands::CATALOG_JSON) {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError(Error::Internal(format!("ui-commands.json: {e}"))).into_response(),
    }
}

/// `POST /ui/commands/{id}/result`.
pub async fn post_result(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Json(reply): Json<UiReply>,
) -> ApiResult<StatusCode> {
    require_human(&auth, "reporting a UI command result")?;
    ctx.ui_bridge
        .complete(&id, &auth.effective_user.id, conn_header(&headers), reply)
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub struct ProgressReq {
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub awaiting_human: bool,
}

/// `POST /ui/commands/{id}/progress`.
pub async fn post_progress(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    headers: HeaderMap,
    Json(req): Json<ProgressReq>,
) -> ApiResult<StatusCode> {
    require_human(&auth, "reporting UI command progress")?;
    ctx.ui_bridge
        .progress(
            &id,
            &auth.effective_user.id,
            conn_header(&headers),
            &req.note,
            req.awaiting_human,
        )
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiControlReq {
    pub enabled: bool,
}

/// `POST /sessions/{id}/ui-control {enabled}` — the ONLY writer of
/// `session.meta.ui_control` (`PATCH /sessions/{id}` and session creation
/// strip it). Granting is the session OWNER's call, made in person (not while
/// impersonated): it is their window the agent will drive. Revoking — the
/// Stop button, a Deny — is open to the owner or a workspace admin / root,
/// and is accepted even when nothing was granted (a Deny before any grant).
/// Revoking ends the session's in-flight UI commands at once.
pub async fn set_ui_control(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Path(id): Path<Id>,
    Json(req): Json<UiControlReq>,
) -> ApiResult<Json<Session>> {
    require_human(&auth, "changing UI control")?;
    let user = &auth.effective_user;
    let session = ctx.manager.get(&id).await.map_err(ApiError)?;
    if req.enabled {
        if session.created_by != user.id {
            return Err(ApiError(Error::Forbidden(
                "only the session's owner can allow UI control — it drives their Otto window".into(),
            )));
        }
        if auth.real_user.id != user.id {
            return Err(ApiError(Error::Forbidden(
                "an impersonated sign-in cannot allow UI control".into(),
            )));
        }
    } else if !otto_core::auth::session_owner_or_admin(ctx.roles.as_ref(), user, &session).await {
        return Err(ApiError(Error::Forbidden(
            "only the session's owner or a workspace admin can stop UI control".into(),
        )));
    }
    let updated = ctx
        .manager
        .update_meta(&id, ui_bridge::grant_record(req.enabled, &user.id))
        .await
        .map_err(ApiError)?;
    if !req.enabled {
        ctx.ui_bridge.cancel_session(
            &id,
            "revoked",
            UiError::new("cancelled_by_user", "the user stopped UI control for this session"),
        );
    }
    ctx.ui_bridge.grant_changed();
    ctx.audit(otto_state::NewAuditEntry {
        user_id: Some(auth.real_user.id.clone()),
        action: if req.enabled {
            "ui_control.grant".into()
        } else {
            "ui_control.revoke".into()
        },
        target: Some(id.clone()),
        detail: Some(serde_json::json!({
            "session_owner": session.created_by,
            "workspace_id": session.workspace_id,
        })),
        ip: None,
    })
    .await;
    Ok(Json(updated))
}
