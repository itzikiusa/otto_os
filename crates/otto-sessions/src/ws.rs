//! Terminal WebSocket — `GET /ws/term/{session_id}` per docs/contracts/ws.md.
//!
//! Auth: `?token=` validated BEFORE the upgrade via a `route_layer` middleware,
//! so the 403 path is exercisable in tests even without a real WS connection.
//! Owners (session creator), workspace Admins, and root may attach. Editors and
//! Viewers who are not the owner are rejected (#L9).  Input/resize capability
//! is determined post-auth by whether the caller holds at least Editor role in
//! the session's workspace.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use bytes::Bytes;
use otto_core::api::Problem;
use otto_core::auth::{session_owner_or_admin, AuthUser, TokenAuthenticator};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_pty::PtyHandle;
use serde::Deserialize;
use tokio::sync::{broadcast, watch};

use crate::http::SessionsCtx;

/// Interval for server-initiated pings (keeps idle sockets alive).
const PING_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct WsState<S> {
    auth: Arc<dyn TokenAuthenticator>,
    ctx: S,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

/// Client → server control frames.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Input {
        data: String,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    // Request a history-inclusive snapshot: up to `lines` rows of scrollback
    // history (rows that scrolled off above the visible screen) followed by a
    // coherent current-screen frame. Honors the requested `lines`.
    Scrollback {
        lines: usize,
    },
}

/// Default scrollback depth used for the initial on-attach snapshot when the
/// client has not yet asked for a specific amount. Capped well under the
/// emulator's 1000-line history so reconnecting restores ample context.
const DEFAULT_ATTACH_HISTORY_LINES: usize = 1000;

/// Build the standalone terminal-WS router (carries its own state).
///
/// The route is layered with [`ws_auth_gate`]: token validation, session
/// lookup, and owner-or-admin check all run BEFORE axum attempts the WS
/// upgrade, so a forbidden caller receives a plain 403 JSON response without
/// the connection ever being promoted to WebSocket.
pub fn ws_router<S: SessionsCtx>(authenticator: Arc<dyn TokenAuthenticator>, ctx: S) -> Router {
    let state = WsState {
        auth: authenticator,
        ctx,
    };
    Router::new()
        .route("/ws/term/{session_id}", get(term_ws::<S>))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            ws_auth_gate::<S>,
        ))
        .with_state(state)
}

fn problem(status: StatusCode, e: &Error) -> Response {
    let body = Problem {
        code: e.code().to_string(),
        message: e.to_string(),
    };
    (status, Json(body)).into_response()
}

/// Route-layer middleware: authenticate the `?token=` query param, look up the
/// session, and enforce the owner-or-admin gate (#L9).
///
/// On success, inserts [`AuthUser`] and [`CanInput`] extensions so `term_ws`
/// can read them without repeating the DB round-trips. On failure, returns a
/// 401/403/404 JSON problem BEFORE the WebSocket upgrade extractor runs — this
/// is the property the isolation tests rely on.
async fn ws_auth_gate<S: SessionsCtx>(
    State(st): State<WsState<S>>,
    Path(session_id): Path<Id>,
    Query(q): Query<TokenQuery>,
    mut req: Request,
    next: Next,
) -> Response {
    // 1. Token auth.
    let token = match q.token {
        Some(t) => t,
        None => return problem(StatusCode::UNAUTHORIZED, &Error::Unauthorized),
    };
    let user = match st.auth.authenticate(&token).await {
        Ok(u) => u,
        Err(_) => return problem(StatusCode::UNAUTHORIZED, &Error::Unauthorized),
    };

    // 2. Session lookup.
    let session = match st.ctx.manager().get(&session_id).await {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::NOT_FOUND, &e),
    };

    // 3. Owner-or-admin gate (#L9): only the creator, a workspace Admin, or
    //    root may attach. Workspace Viewers/Editors who are not the owner are
    //    refused here, before the WS upgrade.
    if !session_owner_or_admin(st.ctx.roles().as_ref(), &user, &session).await {
        return problem(
            StatusCode::FORBIDDEN,
            &Error::Forbidden("not the session owner or a workspace admin".into()),
        );
    }

    // 4. Determine write capability (owner/ws-admin/root all pass Editor check;
    //    a viewer-owner would fail it and stay read-only — unchanged behaviour).
    let can_input = st
        .ctx
        .roles()
        .check(&user, &session.workspace_id, WorkspaceRole::Editor)
        .await
        .is_ok();

    // Propagate auth results to the handler via extensions.
    req.extensions_mut().insert(AuthUser(user));
    req.extensions_mut().insert(CanInput(can_input));

    next.run(req).await
}

/// Newtype extension carrying the write-capability flag set by [`ws_auth_gate`].
#[derive(Clone, Copy)]
struct CanInput(bool);

async fn term_ws<S: SessionsCtx>(
    ws: WebSocketUpgrade,
    Path(session_id): Path<Id>,
    State(st): State<WsState<S>>,
    axum::Extension(AuthUser(_user)): axum::Extension<AuthUser>,
    axum::Extension(CanInput(can_input)): axum::Extension<CanInput>,
) -> Response {
    // Auth and owner-gate already enforced by ws_auth_gate middleware.
    let session = match st.ctx.manager().get(&session_id).await {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::NOT_FOUND, &e),
    };
    let initial_status = session.status;
    ws.on_upgrade(move |socket| async move {
        serve_terminal(socket, st.ctx, session_id, initial_status, can_input).await;
    })
}

/// Receive the next live output chunk, pending forever without a handle.
async fn next_output(
    rx: &mut Option<broadcast::Receiver<Bytes>>,
) -> Result<Bytes, broadcast::error::RecvError> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// Wait for the child exit code, pending forever without a handle.
async fn next_exit(rx: &mut Option<watch::Receiver<Option<i32>>>) -> i32 {
    match rx {
        Some(r) => match crate::manager::wait_exit_code(r).await {
            Some(code) => code,
            None => std::future::pending().await,
        },
        None => std::future::pending().await,
    }
}

async fn serve_terminal<S: SessionsCtx>(
    mut socket: WebSocket,
    ctx: S,
    session_id: Id,
    initial_status: otto_core::domain::SessionStatus,
    can_input: bool,
) {
    // Track this viewer for the whole connection. The guard decrements the
    // session's attached-viewer count on EVERY return path below (clean close,
    // send error, recv end, drop), so the idle-suspend sweep never frees the
    // PTY of a session someone is actively watching.
    let _attach = ctx.manager().attach(&session_id);

    // Auto-resume: if the session is an exited-but-resumable agent session,
    // spawn it now so the reconnect yields a live terminal instead of a black
    // screen.  Errors are logged and ignored — the WS stays open (no handle).
    if let Err(e) = ctx.manager().ensure_live(&session_id).await {
        tracing::warn!(session = %session_id, "ensure_live on ws attach: {e}");
    }

    // Re-read the current status (it may have changed from Exited → Running
    // if ensure_live just resumed the session).
    let current_status = ctx
        .manager()
        .get(&session_id)
        .await
        .map(|s| s.status)
        .unwrap_or(initial_status);

    // On attach: current status first.
    let status_frame = format!(
        r#"{{"type":"status","status":"{}"}}"#,
        current_status.as_str()
    );
    if socket
        .send(Message::Text(status_frame.into()))
        .await
        .is_err()
    {
        return;
    }

    let handle: Option<Arc<PtyHandle>> = ctx.manager().live_handle(&session_id);
    let mut out_rx = handle.as_ref().map(|h| h.subscribe());
    let mut exit_rx = handle.as_ref().map(|h| h.on_exit());
    let mut warned_forbidden = false;
    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ping.reset(); // skip the immediate first tick

    loop {
        tokio::select! {
            // Live PTY output → binary frames.
            chunk = next_output(&mut out_rx) => {
                match chunk {
                    Ok(bytes) => {
                        if socket.send(Message::Binary(bytes)).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(session = %session_id, "terminal ws lagged by {n} chunks");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        out_rx = None;
                    }
                }
            }
            // Child exit → {"type":"exit","code":N}; socket stays open.
            code = next_exit(&mut exit_rx) => {
                exit_rx = None;
                let frame = format!(r#"{{"type":"exit","code":{code}}}"#);
                if socket.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
            }
            _ = ping.tick() => {
                if socket.send(Message::Ping(Bytes::new())).await.is_err() {
                    return;
                }
            }
            // Client control frames.
            msg = socket.recv() => {
                let Some(Ok(msg)) = msg else { return };
                let Message::Text(text) = msg else {
                    if matches!(msg, Message::Close(_)) { return; }
                    continue;
                };
                let Ok(frame) = serde_json::from_str::<ClientFrame>(text.as_str()) else {
                    continue;
                };
                match frame {
                    ClientFrame::Input { data } => {
                        if !can_input {
                            if !warned_forbidden {
                                warned_forbidden = true;
                                let err = r#"{"type":"error","code":"forbidden","message":"viewers cannot send input"}"#;
                                if socket.send(Message::Text(err.into())).await.is_err() {
                                    return;
                                }
                            }
                            continue;
                        }
                        if let Ok(bytes) = B64.decode(data.as_bytes()) {
                            let _ = ctx.manager().input(&session_id, &bytes).await;
                        }
                    }
                    ClientFrame::Resize { cols, rows } => {
                        if can_input {
                            let _ = ctx.manager().resize(&session_id, cols, rows).await;
                        }
                    }
                    ClientFrame::Scrollback { lines } => {
                        // Reproduce the live screen as one coherent frame (what
                        // tmux does on attach) PRECEDED by up to `lines` rows of
                        // scrollback history, so reconnecting restores history
                        // above the viewport instead of losing it. A `lines` of
                        // 0 falls back to the bare current-screen snapshot.
                        let want = if lines == 0 {
                            DEFAULT_ATTACH_HISTORY_LINES
                        } else {
                            lines
                        };
                        let data = handle
                            .as_ref()
                            .map(|h| h.snapshot_with_history(want))
                            .unwrap_or_default();
                        let frame = format!(
                            r#"{{"type":"scrollback","data":"{}"}}"#,
                            B64.encode(&data)
                        );
                        // Sent inline, i.e. before any subsequent live bytes.
                        if socket.send(Message::Text(frame.into())).await.is_err() {
                            return;
                        }
                    }
                }
            }
        }
    }
}
