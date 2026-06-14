//! `GET /ws/events` — event stream WebSocket (see docs/contracts/ws.md).
//!
//! Auth: `?token=` validated BEFORE the upgrade (401 otherwise). Session-
//! scoped events are delivered only to members (viewer+) of the session's
//! workspace; `Notice` events go to every authenticated client. A Ping frame
//! is sent every 30 seconds.

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::error::ApiError;
use crate::state::ServerCtx;

#[derive(Debug, Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub async fn events_ws(
    ws: WebSocketUpgrade,
    Query(query): Query<TokenQuery>,
    State(ctx): State<ServerCtx>,
) -> Response {
    let Some(token) = query.token else {
        return ApiError(Error::Unauthorized).into_response();
    };
    match ctx.authenticator.authenticate(&token).await {
        Ok(user) => ws.on_upgrade(move |socket| handle_events(socket, ctx, user)),
        Err(_) => ApiError(Error::Unauthorized).into_response(),
    }
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
        // Daemon-wide notices/notifications go to every authenticated client.
        Event::Notice { .. } | Event::Notification { .. } => return true,
        Event::SessionStatus { workspace_id, .. } | Event::SessionRemoved { workspace_id, .. } => {
            workspace_id
        }
        Event::SessionCreated { session } => &session.workspace_id,
        // Improvement (self-reflection) events are workspace-scoped — gate them
        // on viewer access to that workspace, like session events.
        Event::ImprovementRunStarted { workspace_id, .. }
        | Event::ImprovementRunFinished { workspace_id, .. }
        | Event::ImprovementEditApplied { workspace_id, .. }
        | Event::ImprovementApprovalPending { workspace_id, .. } => workspace_id,
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
