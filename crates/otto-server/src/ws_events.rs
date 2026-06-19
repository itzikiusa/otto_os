//! `GET /ws/events` — event stream WebSocket (see docs/contracts/ws.md).
//!
//! Auth: the bearer token is read from the `Sec-WebSocket-Protocol` header
//! (the browser sends `["otto-bearer", "<token>"]`; we validate the token and
//! echo back the `otto-bearer` subprotocol). This keeps the token out of the
//! request URL, which is logged everywhere. A legacy `?token=` query parameter
//! is still accepted as a fallback. Validation happens BEFORE the upgrade (401
//! otherwise). Session-scoped events are delivered only to members (viewer+) of
//! the session's workspace; `Notice` events go to every authenticated client;
//! `Notification` events are delivered per the notice's owner. A Ping frame is
//! sent every 30 seconds.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::error::ApiError;
use crate::state::ServerCtx;

/// Fixed first subprotocol the browser offers alongside the token; echoed back
/// on a successful upgrade so the handshake completes.
const BEARER_SUBPROTOCOL: &str = "otto-bearer";

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub async fn events_ws(
    ws: WebSocketUpgrade,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    State(ctx): State<ServerCtx>,
) -> Response {
    // Prefer the bearer subprotocol; fall back to the legacy `?token=` query.
    let subprotocol_token = token_from_subprotocol(&headers);
    let used_subprotocol = subprotocol_token.is_some();
    let Some(token) = subprotocol_token.or(query.token) else {
        return ApiError(Error::Unauthorized).into_response();
    };
    match ctx.authenticator.authenticate(&token).await {
        Ok(user) => {
            // Echo `otto-bearer` only when the client used the subprotocol path,
            // otherwise the browser would reject an unsolicited subprotocol.
            if used_subprotocol {
                ws.protocols([BEARER_SUBPROTOCOL])
                    .on_upgrade(move |socket| handle_events(socket, ctx, user))
            } else {
                ws.on_upgrade(move |socket| handle_events(socket, ctx, user))
            }
        }
        Err(_) => ApiError(Error::Unauthorized).into_response(),
    }
}

/// Extract the bearer token from a `Sec-WebSocket-Protocol: otto-bearer, <token>`
/// request header. Returns `None` when the header is absent or not in that form.
fn token_from_subprotocol(headers: &HeaderMap) -> Option<String> {
    let raw = headers
        .get(axum::http::header::SEC_WEBSOCKET_PROTOCOL)?
        .to_str()
        .ok()?;
    // The header is a comma-separated list; the browser sends the fixed marker
    // first and the token second.
    let mut parts = raw.split(',').map(str::trim);
    if parts.next()? != BEARER_SUBPROTOCOL {
        return None;
    }
    let token = parts.next()?;
    (!token.is_empty()).then(|| token.to_string())
}

async fn handle_events(socket: WebSocket, ctx: ServerCtx, user: User) {
    let mut events = ctx.events.subscribe();
    let (mut sink, mut stream) = socket.split();
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await; // consume the immediate first tick

    // Role-check results cached per workspace for this connection's lifetime.
    let mut role_cache: HashMap<Id, bool> = HashMap::new();

    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if !allowed(&ctx, &user, &event, &mut role_cache).await {
                        continue;
                    }
                    let Ok(text) = serde_json::to_string(&event) else { continue };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!("events ws lagged, skipped {skipped} events");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => match incoming {
                // Client→server messages on this socket are ignored.
                Some(Ok(_)) => continue,
                _ => break,
            },
        }
    }
}

/// Notice → everyone; session-scoped events → workspace members (viewer+).
async fn allowed(
    ctx: &ServerCtx,
    user: &User,
    event: &Event,
    cache: &mut HashMap<Id, bool>,
) -> bool {
    let workspace_id = match event {
        // Free-form toasts go to every authenticated client.
        Event::Notice { .. } => return true,
        // Persisted notifications are per-user scoped: a global notice
        // (`user_id == None`) goes to everyone; an owned notice is delivered
        // only to that user's connections (root sees all, mirroring the REST
        // `NoticeAccess` policy in routes/notifications.rs).
        Event::Notification { user_id, .. } => {
            return match user_id {
                None => true,
                Some(target) => user.is_root || &user.id == target,
            };
        }
        Event::SessionStatus { workspace_id, .. } | Event::SessionRemoved { workspace_id, .. } => {
            workspace_id
        }
        Event::SessionCreated { session } => &session.workspace_id,
        Event::SessionMetaUpdated { workspace_id, .. } => workspace_id,
        // Improvement (self-reflection) events are workspace-scoped — gate them
        // on viewer access to that workspace, like session events.
        Event::ImprovementRunStarted { workspace_id, .. }
        | Event::ImprovementRunFinished { workspace_id, .. }
        | Event::ImprovementEditApplied { workspace_id, .. }
        | Event::ImprovementApprovalPending { workspace_id, .. }
        // Session activity-trail / task events are workspace-scoped, like the
        // session status events above.
        | Event::TrailAppended { workspace_id, .. }
        | Event::TasksUpdated { workspace_id, .. }
        // Agent-swarm events are workspace-scoped, like session events.
        | Event::SwarmRunUpdated { workspace_id, .. }
        | Event::SwarmTaskUpdated { workspace_id, .. }
        | Event::SwarmMessagePosted { workspace_id, .. }
        | Event::SwarmStatus { workspace_id, .. } => workspace_id,
    };
    if let Some(&ok) = cache.get(workspace_id) {
        return ok;
    }
    let ok = ctx
        .roles
        .check(user, workspace_id, WorkspaceRole::Viewer)
        .await
        .is_ok();
    cache.insert(workspace_id.clone(), ok);
    ok
}
