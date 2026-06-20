//! Context-packet routes — B2a.
//!
//! Lets an operator send an API response / DB result / broker message to a
//! focused agent session as context, with a mandatory redaction preview step
//! before any data reaches the agent's PTY.
//!
//! # Endpoints
//!
//! ```text
//! POST /workspaces/{wid}/agents/{sid}/context-packet/preview
//!   body  { kind: "api"|"db"|"broker", payload: <json> }
//!   →     { redacted: <json>, redactions: [{kind,count}], size_bytes }
//!
//! POST /workspaces/{wid}/agents/{sid}/context-packet/send
//!   body  { kind, payload }
//!   →     { ok: true, size_bytes, redactions: [{kind,count}] }
//! ```
//!
//! The `/preview` endpoint builds the packet (a titled markdown/JSON context
//! block), runs it through [`otto_core::redact::redact_json`], and returns the
//! redacted view + hit summary **without injecting anything into the session**.
//! Only `/send` pushes the *already-redacted* packet into the PTY.
//!
//! Auth: both endpoints require that the caller either owns the target session
//! or is a workspace admin (`require_session_owner_or_admin`).

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use otto_core::redact::{redact_json, RedactionHit};
use otto_core::{Error, Id};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Kind of data source the packet originates from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketKind {
    /// An HTTP API client response (status + headers + body).
    Api,
    /// A database query result set (rows + columns).
    Db,
    /// A Kafka broker message (key + value + metadata).
    Broker,
}

impl PacketKind {
    fn label(&self) -> &'static str {
        match self {
            PacketKind::Api => "API Response",
            PacketKind::Db => "DB Result",
            PacketKind::Broker => "Broker Message",
        }
    }
}

/// Request body for both `/preview` and `/send`.
#[derive(Debug, Deserialize)]
pub struct ContextPacketReq {
    pub kind: PacketKind,
    pub payload: Value,
}

/// Response from `/preview`.
#[derive(Debug, Serialize)]
pub struct PreviewResp {
    /// The redacted JSON value (same shape as `payload` but with secrets removed).
    pub redacted: Value,
    /// Per-kind redaction tally.
    pub redactions: Vec<RedactionHit>,
    /// Byte length of the formatted packet that will be injected on `/send`.
    pub size_bytes: usize,
}

/// Response from `/send`.
#[derive(Debug, Serialize)]
pub struct SendResp {
    pub ok: bool,
    pub size_bytes: usize,
    pub redactions: Vec<RedactionHit>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the markdown/text packet block that is injected into the agent PTY.
/// The block is source-labelled so the agent understands what it received.
fn build_packet_text(kind: &PacketKind, redacted_payload: &Value) -> String {
    let label = kind.label();
    let body = match serde_json::to_string_pretty(redacted_payload) {
        Ok(s) => s,
        Err(_) => redacted_payload.to_string(),
    };
    format!(
        "```context:{kind}\n{body}\n```",
        kind = label.to_lowercase().replace(' ', "_")
    )
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /workspaces/{wid}/agents/{sid}/context-packet/preview`
///
/// Redacts the payload and returns the cleaned view + hit summary without
/// injecting anything. The UI MUST show this to the operator before sending.
async fn preview(
    Path((ws_id, session_id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ContextPacketReq>,
) -> ApiResult<Json<PreviewResp>> {
    // Resolve the session and verify ownership/admin.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    if session.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "session not found in workspace".into(),
        )));
    }
    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;

    // Redact, build packet text, compute byte length.
    let redacted_result = redact_json(&req.payload);
    let packet_text = build_packet_text(&req.kind, &redacted_result.value);

    Ok(Json(PreviewResp {
        redacted: redacted_result.value,
        redactions: redacted_result.hits,
        size_bytes: packet_text.len(),
    }))
}

/// `POST /workspaces/{wid}/agents/{sid}/context-packet/send`
///
/// Redacts the payload (same pass as `/preview`), formats it as a context
/// block, and injects it into the agent session's PTY via the session manager.
/// The caller must own the session or be a workspace admin.
async fn send(
    Path((ws_id, session_id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ContextPacketReq>,
) -> ApiResult<Json<SendResp>> {
    // Resolve the session and verify ownership/admin.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    if session.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "session not found in workspace".into(),
        )));
    }
    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;

    // Redact, build packet text.
    let redacted_result = redact_json(&req.payload);
    let packet_text = build_packet_text(&req.kind, &redacted_result.value);
    let size_bytes = packet_text.len();
    let redactions = redacted_result.hits;

    // Inject the redacted packet into the agent PTY. Reuses the same
    // `manager.input` path as `session_input_routes` and `attach_product_story`
    // (the canonical write path for all session context injection).
    let manager = Arc::clone(&ctx.manager);
    let sid = session_id.clone();
    let payload_bytes = format!("{packet_text}\n").into_bytes();
    tokio::spawn(async move {
        // Short settle so the agent's prompt is ready before the packet arrives
        // (mirrors the 2-second delay used by `attach_product_story`).
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if let Err(e) = manager.input(&sid, &payload_bytes).await {
            tracing::warn!(session = %sid, "context-packet write failed: {e}");
        }
    });

    Ok(Json(SendResp {
        ok: true,
        size_bytes,
        redactions,
    }))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Routes for the context-packet endpoints. Merged into the protected API
/// router via `module_routers` in `crates/otto-server/src/modules.rs`.
pub fn context_packet_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{wid}/agents/{sid}/context-packet/preview",
            post(preview),
        )
        .route(
            "/workspaces/{wid}/agents/{sid}/context-packet/send",
            post(send),
        )
}
