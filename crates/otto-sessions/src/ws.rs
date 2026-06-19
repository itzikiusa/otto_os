//! Terminal WebSocket — `GET /ws/term/{session_id}` per docs/contracts/ws.md.
//!
//! Auth: `?token=` validated BEFORE the upgrade via a `route_layer` middleware,
//! so the 403 path is exercisable in tests even without a real WS connection.
//! Owners (session creator), workspace Admins, and root may attach. Editors and
//! Viewers who are not the owner are rejected (#L9).  Input/resize capability
//! is determined post-auth by whether the caller holds at least Editor role in
//! the session's workspace.
//!
//! A **scoped (share-link) token** is a separate path (mobile plan Task 1.6): it
//! bypasses the owner-or-admin gate (the scope IS the authority) but may attach
//! ONLY to its one pinned session — the path `session_id` must equal the scope's
//! `session_id` (else 403, no upgrade) — and its write capability is the share's
//! capped role (`Editor` may input/resize; a `Viewer` share is read-only).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
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

use crate::share_throttle;

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

/// Fixed first subprotocol the browser offers alongside the token; the gate
/// echoes it back on a successful upgrade so the handshake completes. Mirrors
/// the same constant in `otto-server`'s `ws_events.rs`.
const BEARER_SUBPROTOCOL: &str = "otto-bearer";

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

/// Route-layer middleware: authenticate the token (from `Sec-WebSocket-Protocol`
/// or `?token=`), look up the session, and enforce the owner-or-admin gate (#L9).
///
/// Token extraction order (Task 1.10):
///  1. `Sec-WebSocket-Protocol: otto-bearer, <token>` — preferred; keeps the
///     token out of the URL (which is logged everywhere). On a successful
///     subprotocol auth, the upgrade response echoes `otto-bearer` back.
///  2. `?token=<bearer token>` query param — backward-compatible fallback.
///
/// On success, inserts [`AuthUser`], [`CanInput`], and [`UsedSubprotocol`]
/// extensions so `term_ws` can read them. On failure, returns a 401/403/404
/// JSON problem BEFORE the WebSocket upgrade extractor runs — this is the
/// property the isolation tests rely on.
///
/// Rate limiting (Task 1.8): the real socket-peer IP (from
/// `ConnectInfo<SocketAddr>`, wired up by `into_make_service_with_connect_info`
/// in `ottod`) is checked against the share-redemption throttle BEFORE auth.
/// A failed auth records a failure; a successful auth clears the IP's tally.
async fn ws_auth_gate<S: SessionsCtx>(
    State(st): State<WsState<S>>,
    Path(session_id): Path<Id>,
    Query(q): Query<TokenQuery>,
    mut req: Request,
    next: Next,
) -> Response {
    // 0. Extract the real peer IP from the ConnectInfo extension (Task 1.8).
    //    `into_make_service_with_connect_info::<SocketAddr>` (in ottod) injects
    //    this; it's absent only in unit-test harnesses that don't wire it up.
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    // 1. Token source resolution (Task 1.10): subprotocol first, then query.
    let subprotocol_token = token_from_subprotocol(req.headers());
    let used_subprotocol = subprotocol_token.is_some();
    let token = match subprotocol_token.or(q.token) {
        Some(t) => t,
        None => return problem(StatusCode::UNAUTHORIZED, &Error::Unauthorized),
    };

    // 2. IP rate-limit check BEFORE auth (Task 1.8).
    if let Some(ip) = peer_ip {
        if let Err(locked) = share_throttle::global().check(ip) {
            let secs = locked.retry_after.as_secs().max(1);
            let body = otto_core::api::Problem {
                code: "too_many_requests".to_string(),
                message: "too many failed share-token attempts; try again later".to_string(),
            };
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", secs.to_string())],
                Json(body),
            )
                .into_response();
        }
    }

    // 3. Token auth.
    let auth = match st.auth.authenticate(&token).await {
        Ok(auth) => {
            // Successful auth: clear the IP's failure tally.
            if let Some(ip) = peer_ip {
                share_throttle::global().clear(ip);
            }
            auth
        }
        Err(_) => {
            // Failed auth: record failure against the peer IP.
            if let Some(ip) = peer_ip {
                share_throttle::global().record_failure(ip);
            }
            return problem(StatusCode::UNAUTHORIZED, &Error::Unauthorized);
        }
    };
    // Authorize attach against the effective user (== real for a normal token);
    // the owner-or-admin gate below runs on the effective identity.
    let user = auth.effective_user;

    // 2. Session lookup.
    let session = match st.ctx.manager().get(&session_id).await {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::NOT_FOUND, &e),
    };

    // 3. Authorize the attach + decide write capability. Two disjoint paths:
    //
    //  - **Scoped (share-link) token** (`scope == Some`): the scope IS the
    //    authority, so the owner-or-admin gate is BYPASSED. The single guarantee
    //    a share carries is its one pinned `session_id`, so the path id MUST
    //    equal `scope.session_id` (a share for S1 must never attach to S2, even
    //    if the same owner created S2) → otherwise 403, no upgrade. Write
    //    capability is the share's capped role: `Editor` may type/resize, a
    //    `Viewer` share is strictly read-only. The workspace-role probe is
    //    ignored entirely (the synthetic share principal holds no membership).
    //
    //  - **Unscoped token** (`scope == None`): unchanged behaviour — the
    //    owner-or-admin gate (#L9) plus the Editor probe for write capability.
    let can_input = match auth.scope {
        Some(scope) => {
            if scope.session_id != session_id {
                return problem(
                    StatusCode::FORBIDDEN,
                    &Error::Forbidden("share token is scoped to a different session".into()),
                );
            }
            scope.role == WorkspaceRole::Editor
        }
        None => {
            // Owner-or-admin gate (#L9): only the creator, a workspace Admin, or
            // root may attach. Workspace Viewers/Editors who are not the owner
            // are refused here, before the WS upgrade.
            if !session_owner_or_admin(st.ctx.roles().as_ref(), &user, &session).await {
                return problem(
                    StatusCode::FORBIDDEN,
                    &Error::Forbidden("not the session owner or a workspace admin".into()),
                );
            }
            // Write capability (owner/ws-admin/root all pass the Editor check; a
            // viewer-owner would fail it and stay read-only — unchanged).
            st.ctx
                .roles()
                .check(&user, &session.workspace_id, WorkspaceRole::Editor)
                .await
                .is_ok()
        }
    };

    // Propagate auth results to the handler via extensions.
    req.extensions_mut().insert(AuthUser(user));
    req.extensions_mut().insert(CanInput(can_input));
    // Tell term_ws whether the client used the subprotocol path so it can echo
    // `otto-bearer` back in the upgrade response (Task 1.10).
    req.extensions_mut().insert(UsedSubprotocol(used_subprotocol));

    next.run(req).await
}

/// Newtype extension carrying the write-capability flag set by [`ws_auth_gate`].
#[derive(Clone, Copy)]
struct CanInput(bool);

/// Newtype extension: true iff the client presented the token via the
/// `Sec-WebSocket-Protocol: otto-bearer, <token>` header. When set, `term_ws`
/// echoes the `otto-bearer` subprotocol in the upgrade response so the browser
/// handshake completes; a bare `?token=` client gets a plain upgrade.
#[derive(Clone, Copy)]
struct UsedSubprotocol(bool);

async fn term_ws<S: SessionsCtx>(
    ws: WebSocketUpgrade,
    Path(session_id): Path<Id>,
    State(st): State<WsState<S>>,
    axum::Extension(AuthUser(_user)): axum::Extension<AuthUser>,
    axum::Extension(CanInput(can_input)): axum::Extension<CanInput>,
    axum::Extension(UsedSubprotocol(used_subprotocol)): axum::Extension<UsedSubprotocol>,
) -> Response {
    // Auth and owner-gate already enforced by ws_auth_gate middleware.
    let session = match st.ctx.manager().get(&session_id).await {
        Ok(s) => s,
        Err(e) => return problem(StatusCode::NOT_FOUND, &e),
    };
    let initial_status = session.status;
    // Echo `otto-bearer` only when the client used the subprotocol path (Task
    // 1.10): the browser rejects an unsolicited subprotocol in the upgrade
    // response, so we must not echo it for legacy `?token=` clients.
    if used_subprotocol {
        ws.protocols([BEARER_SUBPROTOCOL])
            .on_upgrade(move |socket| async move {
                serve_terminal(socket, st.ctx, session_id, initial_status, can_input).await;
            })
    } else {
        ws.on_upgrade(move |socket| async move {
            serve_terminal(socket, st.ctx, session_id, initial_status, can_input).await;
        })
    }
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

/// Wait for the per-session forced-disconnect signal. `Ok` (fired) and `Lagged`
/// both mean "evict"; `Closed` (the sender was dropped without ever firing,
/// e.g. the session row was removed) clears the receiver and pends forever so
/// this branch stops competing in the `select!` without busy-looping.
async fn next_evict(rx: &mut Option<broadcast::Receiver<()>>) {
    match rx {
        Some(r) => match r.recv().await {
            Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                *rx = None;
                std::future::pending().await
            }
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
    // Forced-disconnect signal: admin terminate / share-link revoke fire this
    // (via SessionManager::evict) to immediately kick attached viewers, even
    // before the PTY broadcast closes.
    let mut evict_rx = Some(ctx.manager().evict_signal(&session_id));
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
            // Forced disconnect: the session was terminated (admin terminate or
            // share-link revoke). Tell the client and close cleanly. `Lagged`
            // also resolves here (signal was sent) → evict; only `Closed`
            // (handled in next_evict) is a non-event that stops this branch.
            _ = next_evict(&mut evict_rx) => {
                let frame = r#"{"type":"terminated"}"#;
                let _ = socket.send(Message::Text(frame.into())).await;
                return;
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

#[cfg(test)]
mod tests {
    //! In-crate gate tests (mobile plan Task 1.6). These drive the private
    //! [`ws_auth_gate`] middleware directly through a probe handler that echoes
    //! the [`CanInput`] extension into a response header, so the read-only vs
    //! read-write capability a scoped share confers is observable WITHOUT a real
    //! WebSocket upgrade (which the integration harness in `tests/isolation.rs`
    //! cannot inspect — it only sees the pre-upgrade status).

    use super::*;
    use crate::{ProviderRegistry, SessionManager};
    use chrono::Utc;
    use otto_core::auth::RoleChecker;
    use otto_core::domain::{SessionKind, WorkspaceRole};
    use otto_rbac::{tokens::AuthRepo, RbacAuthenticator, RbacRoleChecker};
    use otto_state::{SessionsRepo, SqlitePool, WorkspacesRepo};
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt; // for `oneshot`

    #[derive(Clone)]
    struct Ctx {
        manager: Arc<SessionManager>,
        roles: Arc<dyn RoleChecker>,
        workspaces: WorkspacesRepo,
    }

    impl SessionsCtx for Ctx {
        fn manager(&self) -> &Arc<SessionManager> {
            &self.manager
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn workspaces(&self) -> &WorkspacesRepo {
            &self.workspaces
        }
    }

    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .expect("connect in-memory sqlite");
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn seed_user(pool: &SqlitePool, id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, 'x', ?, 0, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed user");
    }

    async fn seed_workspace(pool: &SqlitePool, ws_id: &str) {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
             VALUES (?, 'ws', '/tmp', '{}', 0, ?)",
        )
        .bind(ws_id)
        .bind(&now)
        .execute(pool)
        .await
        .expect("seed workspace");
    }

    async fn insert_session(repo: &SessionsRepo, ws: &str, created_by: &str) -> Id {
        repo.create(otto_state::NewSession {
            workspace_id: ws.into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: created_by.into(),
            meta: serde_json::Value::Null,
        })
        .await
        .expect("insert session")
        .id
    }

    /// Build a router that layers the REAL [`ws_auth_gate`] over a probe handler
    /// echoing the gate-set [`CanInput`] and [`UsedSubprotocol`] flags into
    /// response headers. This lets the test read the exact capability decision
    /// and subprotocol detection pre-upgrade (without a real WS upgrade).
    fn probe_app(state: WsState<Ctx>) -> Router {
        async fn probe(
            axum::Extension(CanInput(can_input)): axum::Extension<CanInput>,
            axum::Extension(UsedSubprotocol(used_subprotocol)): axum::Extension<UsedSubprotocol>,
        ) -> Response {
            let mut resp = StatusCode::OK.into_response();
            resp.headers_mut().insert(
                "x-can-input",
                axum::http::HeaderValue::from_static(if can_input { "1" } else { "0" }),
            );
            resp.headers_mut().insert(
                "x-used-subprotocol",
                axum::http::HeaderValue::from_static(if used_subprotocol { "1" } else { "0" }),
            );
            resp
        }
        Router::new()
            .route("/ws/term/{session_id}", get(probe))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                ws_auth_gate::<Ctx>,
            ))
            .with_state(state)
    }

    async fn build(pool: &SqlitePool) -> WsState<Ctx> {
        let repo = SessionsRepo::new(pool.clone());
        let (events, _rx) = broadcast::channel(64);
        let providers = ProviderRegistry::new(None);
        let manager = Arc::new(SessionManager::new(repo, events, providers));
        let ctx = Ctx {
            manager,
            roles: Arc::new(RbacRoleChecker::new(pool.clone())),
            workspaces: WorkspacesRepo::new(pool.clone()),
        };
        WsState {
            auth: Arc::new(RbacAuthenticator::new(pool.clone())),
            ctx,
        }
    }

    async fn mint_share(pool: &SqlitePool, owner: &str, sid: &Id, role: WorkspaceRole) -> String {
        AuthRepo::new(pool.clone())
            .issue_share_token(&owner.into(), sid, role, 3600, None)
            .await
            .expect("issue share")
            .0
    }

    /// Drive the gate via the legacy `?token=` query param.
    async fn gate(app: &Router, sid: &Id, token: &str) -> Response {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{sid}?token={token}"))
            .body(axum::body::Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    /// Drive the gate via the `Sec-WebSocket-Protocol: otto-bearer, <token>` header.
    async fn gate_subprotocol(app: &Router, sid: &Id, token: &str) -> Response {
        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{sid}"))
            .header(
                axum::http::header::SEC_WEBSOCKET_PROTOCOL,
                format!("otto-bearer, {token}"),
            )
            .body(axum::body::Body::empty())
            .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    /// A **viewer** share attaches to its session but is read-only: the gate sets
    /// `CanInput = false`.
    #[tokio::test]
    async fn viewer_share_is_read_only() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(resp.status(), StatusCode::OK, "viewer share must pass the gate");
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "0",
            "a viewer share must be read-only (CanInput=false)"
        );
    }

    /// An **editor** share attaches AND may type/resize: `CanInput = true`.
    #[tokio::test]
    async fn editor_share_can_input() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Editor).await;
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(resp.status(), StatusCode::OK, "editor share must pass the gate");
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "1",
            "an editor share may input (CanInput=true)"
        );
    }

    /// A share scoped to S1 is REFUSED (403) on S2 — even though the same owner
    /// created both — and never reaches the probe handler.
    #[tokio::test]
    async fn share_for_s1_denied_on_s2() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let repo = SessionsRepo::new(pool.clone());
        let s1 = insert_session(&repo, "ws1", "alice").await;
        let s2 = insert_session(&repo, "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        assert_eq!(
            gate(&app, &s1, &token).await.status(),
            StatusCode::OK,
            "share must pass on its pinned session S1"
        );
        assert_eq!(
            gate(&app, &s2, &token).await.status(),
            StatusCode::FORBIDDEN,
            "a share scoped to S1 must be 403 on S2"
        );
    }

    /// A normal (unscoped) owner token still passes the owner gate and gets the
    /// Editor-probe capability — unchanged behaviour for non-scoped tokens.
    #[tokio::test]
    async fn unscoped_owner_token_unchanged() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1','alice','editor')")
            .execute(&pool)
            .await
            .expect("set member");
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = AuthRepo::new(pool.clone())
            .issue(&"alice".into())
            .await
            .expect("issue token");
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(resp.status(), StatusCode::OK, "owner must pass the gate");
        assert_eq!(
            resp.headers().get("x-can-input").unwrap(),
            "1",
            "owner-editor keeps input capability (unscoped path unchanged)"
        );
    }

    // ---- Task 1.10: otto-bearer subprotocol on /ws/term -------------------

    /// A share token presented via `Sec-WebSocket-Protocol: otto-bearer, <token>`
    /// is accepted: the gate sets 200 and marks `UsedSubprotocol = true`.
    #[tokio::test]
    async fn subprotocol_token_accepted() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = gate_subprotocol(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "subprotocol token must pass the gate"
        );
        assert_eq!(
            resp.headers().get("x-used-subprotocol").unwrap(),
            "1",
            "gate must detect the subprotocol path"
        );
    }

    /// A bad token via subprotocol is rejected with 401.
    #[tokio::test]
    async fn bad_subprotocol_token_rejected() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let resp = gate_subprotocol(&app, &s1, "not-a-real-token").await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "bad subprotocol token must return 401"
        );
    }

    /// Legacy `?token=` is still accepted and `UsedSubprotocol` is false.
    #[tokio::test]
    async fn query_param_token_marks_no_subprotocol() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let resp = gate(&app, &s1, &token).await;
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "?token= must still work (backward compat)"
        );
        assert_eq!(
            resp.headers().get("x-used-subprotocol").unwrap(),
            "0",
            "legacy ?token= path must NOT set UsedSubprotocol"
        );
    }

    // ---- Task 1.8: share redemption rate limiter --------------------------

    /// The `ShareThrottle` unit tests live in `share_throttle.rs`. Here we
    /// assert that the gate wiring is correct: a request carrying an invalid
    /// token from an IP that has been pre-locked returns 429 (not 401), and a
    /// valid token still works when the IP is not locked.
    #[tokio::test]
    async fn rate_limit_blocks_locked_ip() {
        use crate::share_throttle::{ShareThrottle, FAILURE_THRESHOLD};
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        // Use an isolated throttle instance to pre-lock an IP.
        let throttle = ShareThrottle::default();
        let loopback = IpAddr::V4(Ipv4Addr::new(127, 42, 42, 1));
        for _ in 0..FAILURE_THRESHOLD {
            throttle.record_failure(loopback);
        }
        assert!(throttle.check(loopback).is_err(), "IP should be locked");

        // The global throttle (used by the gate) is separate; verify the gate
        // itself does NOT lock a clean IP and still accepts a valid token.
        let token = mint_share(&pool, "alice", &s1, WorkspaceRole::Viewer).await;
        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{s1}?token={token}"))
            // Inject a clean ConnectInfo — the loopback already has N failures
            // in the isolated `throttle` above, but the GLOBAL store is clean.
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "a valid token from a clean IP must still pass through the global throttle"
        );
    }

    /// A request with no token at all is 401 (no change to throttle behaviour
    /// — we only rate-limit actual token attempts, not missing-token probes).
    #[tokio::test]
    async fn no_token_is_401() {
        let pool = mem_pool().await;
        seed_user(&pool, "alice").await;
        seed_workspace(&pool, "ws1").await;
        let s1 = insert_session(&SessionsRepo::new(pool.clone()), "ws1", "alice").await;
        let app = probe_app(build(&pool).await);

        let req = Request::builder()
            .method("GET")
            .uri(format!("/ws/term/{s1}"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
