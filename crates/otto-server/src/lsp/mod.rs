//! LSP backend: server registry, capabilities detection, WebSocket↔stdio
//! bridge, and language-server install helper.
//!
//! Routes exposed:
//!   GET  /api/v1/lsp/capabilities              → LspCapabilities (authed)
//!   POST /api/v1/workspaces/{id}/lsp/install   → Session (Editor role)
//!   GET  /ws/lsp?lang=&root=                   → WebSocket bridge (`otto-bearer` subprotocol auth)
//!
//! The bridge does NOT spawn a server per socket: sockets attach to the shared
//! [`pool::LspPool`] (one process per `(lang, root)`, ref-counted, reaped
//! [`pool::IDLE_TIMEOUT`] after its last socket leaves).

pub mod framing;
pub mod pool;
pub mod servers;

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::api::{CreateSessionReq, Problem};
use otto_core::auth::TokenAuthenticator;
use otto_core::domain::{SessionKind, WorkspaceRole};
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// API types
// ---------------------------------------------------------------------------

/// One entry in the capabilities response.
#[derive(Debug, Clone, Serialize)]
pub struct LspServerStatus {
    pub lang: String,
    pub available: bool,
    /// Resolved executable path, or the primary name when unavailable.
    pub command: String,
    /// Shell command the UI can run to install this server; null when unknown.
    pub install_command: Option<String>,
}

/// `GET /api/v1/lsp/capabilities`
#[derive(Debug, Clone, Serialize)]
pub struct LspCapabilities {
    pub servers: Vec<LspServerStatus>,
}

// ---------------------------------------------------------------------------
// Capabilities route
// ---------------------------------------------------------------------------

/// `GET /api/v1/lsp/capabilities` — available to any authenticated user.
async fn capabilities(
    State(_ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<LspCapabilities>> {
    let resolved = servers::detect_all();
    let servers = resolved
        .into_iter()
        .map(|r| LspServerStatus {
            lang: r.lang,
            available: r.available,
            command: r.command,
            install_command: r.install_command,
        })
        .collect();
    Ok(Json(LspCapabilities { servers }))
}

// ---------------------------------------------------------------------------
// Install route
// ---------------------------------------------------------------------------

/// Request body for `POST /api/v1/workspaces/{id}/lsp/install`.
#[derive(Debug, Deserialize)]
pub struct InstallLspReq {
    /// Which languages to install. When absent → install all unavailable ones
    /// that have an `install_command`.
    #[serde(default)]
    pub langs: Option<Vec<String>>,
}

/// `POST /api/v1/workspaces/{id}/lsp/install` — Editor role required.
///
/// Spawns ONE shell agent session that runs all requested install commands
/// sequentially, mirroring the `providers/update` handler pattern exactly.
async fn install_servers(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<InstallLspReq>>,
) -> ApiResult<Json<otto_core::domain::Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;

    let req_langs = body.and_then(|b| b.0.langs);

    // Collect (lang, install_command) pairs.
    let resolved = servers::detect_all();
    let pairs: Vec<String> = match req_langs {
        Some(ref langs) => {
            // Caller specified which langs to install.
            let mut cmds = Vec::new();
            for lang in langs {
                let entry = resolved.iter().find(|r| &r.lang == lang);
                match entry {
                    None => {
                        return Err(ApiError(Error::Invalid(format!(
                            "unknown language: '{lang}'"
                        ))));
                    }
                    Some(r) => match &r.install_command {
                        None => {
                            return Err(ApiError(Error::Invalid(format!(
                                "no install command for '{lang}' (manual install required)"
                            ))));
                        }
                        Some(cmd) => cmds.push(cmd.clone()),
                    },
                }
            }
            cmds
        }
        None => {
            // Install all unavailable languages that have an install command.
            resolved
                .iter()
                .filter(|r| !r.available)
                .filter_map(|r| r.install_command.clone())
                .collect()
        }
    };

    if pairs.is_empty() {
        return Err(ApiError(Error::Invalid(
            "no languages need installation (all available, or no install commands)".into(),
        )));
    }

    // Deduplicate: same install command can cover multiple langs (e.g.
    // vscode-langservers-extracted covers css, html, json).
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = pairs
        .into_iter()
        .filter(|cmd| seen.insert(cmd.clone()))
        .collect();

    // Build the compound shell command, mirroring providers/update exactly.
    let compound = unique.join("; echo; ");

    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;

    let session_req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("shell".to_string()),
        title: Some("Install language servers".to_string()),
        cwd: None,
        connection_id: None,
        model: None,
        meta: None,
    };

    let session = ctx
        .manager
        .create(&ws, &user.id, session_req, None)
        .await
        .map_err(ApiError)?;

    // Write the compound command into the PTY ~800 ms after spawn, exactly as
    // the providers/update handler does.
    let manager = Arc::clone(&ctx.manager);
    let session_id = session.id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        if let Err(e) = manager
            .input(&session_id, format!("{compound}\n").as_bytes())
            .await
        {
            tracing::warn!(session = %session_id, "lsp install command write failed: {e}");
        }
    });

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// WebSocket bridge
// ---------------------------------------------------------------------------

/// State carried into the WS upgrade handler (owns the token authenticator).
#[derive(Clone)]
struct LspWsState {
    auth: Arc<dyn TokenAuthenticator>,
    ctx: ServerCtx,
    /// Shared servers, one per `(user, lang, root)`.
    pool: Arc<pool::LspPool>,
}

#[derive(Deserialize)]
struct LspWsQuery {
    lang: Option<String>,
    root: Option<String>,
}

fn ws_problem(status: StatusCode, code: &str, message: &str) -> Response {
    let body = Problem {
        code: code.to_string(),
        message: message.to_string(),
    };
    (status, Json(body)).into_response()
}

/// `GET /ws/lsp?lang=<lang>&root=<path>` (bearer in `Sec-WebSocket-Protocol`)
async fn lsp_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<LspWsQuery>,
    State(st): State<LspWsState>,
    headers: axum::http::HeaderMap,
) -> Response {
    // 1. Token auth before upgrade (mirrors term_ws). The bearer travels only
    //    in the `otto-bearer` subprotocol — never the URL, which lands in
    //    trace spans and tunnel/proxy access logs (S11-11); the legacy
    //    `?token=` fallback is gone (S11-312).
    let ws = ws.protocols([crate::ws_events::BEARER_SUBPROTOCOL]);
    let token = match crate::ws_events::token_from_subprotocol(&headers) {
        Some(t) => t,
        None => {
            return ws_problem(StatusCode::UNAUTHORIZED, "unauthorized", "missing token");
        }
    };
    let auth = match st.auth.authenticate(&token).await {
        Ok(a) => a,
        Err(_) => {
            return ws_problem(StatusCode::UNAUTHORIZED, "unauthorized", "invalid token");
        }
    };
    // Same credential rules as the Files routes (`/fs/read`, `/fs/browse`):
    // share-link (scoped), MCP-restricted and agent-session tokens never reach
    // host files (an unconfined language server reads the whole root).
    if crate::feature_guard::root_route_gate(
        crate::feature_guard::RootRoute::Lsp,
        &auth,
        Some(&st.ctx.pool),
    )
    .await
    .is_err()
    {
        return ws_problem(
            StatusCode::FORBIDDEN,
            "forbidden",
            "this credential cannot open a language server",
        );
    }
    let user = auth.effective_user.clone();

    // 2. Validate lang parameter.
    let lang = match q.lang {
        Some(l) if !l.is_empty() => l,
        _ => {
            return ws_problem(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "missing ?lang= parameter",
            );
        }
    };

    // 3. Validate and canonicalize root directory.
    let root = match q.root {
        Some(r) if !r.is_empty() => r,
        _ => {
            return ws_problem(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "missing ?root= parameter",
            );
        }
    };
    let root_path = std::path::Path::new(&root);
    if !root_path.exists() || !root_path.is_dir() {
        return ws_problem(
            StatusCode::BAD_REQUEST,
            "bad_request",
            &format!("root does not exist or is not a directory: {root}"),
        );
    }

    // 4. Resolve the language server.
    let resolved = match servers::resolve_lang(&lang) {
        Some(r) => r,
        None => {
            return ws_problem(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("unknown language: '{lang}'"),
            );
        }
    };
    if !resolved.available {
        return ws_problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_unavailable",
            &format!(
                "no language server found for '{lang}' ('{}')",
                resolved.command
            ),
        );
    }

    // One pooled server per (user, lang, canonical root): `/a/../a` and `/a`
    // share; two users never do (r3-10-08).
    let root = std::fs::canonicalize(root_path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(root);
    // 5. The caller must be able to see a workspace containing `root` (the
    //    server indexes — and answers queries about — everything under it).
    //    Root users own the host, like the Files routes.
    if !user.is_root {
        match root_visible_to(&st.ctx, &user, std::path::Path::new(&root)).await {
            Ok(true) => {}
            Ok(false) => {
                return ws_problem(
                    StatusCode::FORBIDDEN,
                    "forbidden",
                    "root is not inside a workspace you can access",
                );
            }
            Err(e) => {
                tracing::warn!("lsp: workspace lookup failed: {e}");
                return ws_problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "workspace lookup failed",
                );
            }
        }
    }
    let key = pool::PoolKey {
        owner: user.id.to_string(),
        lang,
        root,
    };
    let cmd = resolved.command.clone();
    let args = resolved.args.clone();
    let pool = Arc::clone(&st.pool);

    ws.on_upgrade(move |socket| async move {
        serve_lsp(socket, pool, key, cmd, args).await;
    })
}

/// Is canonical `root` equal to — or inside — the canonical root of a
/// workspace `user` is a member of? The scratch workspace (every user is an
/// implicit Editor; its root is the daemon user's HOME) matches only EXACTLY,
/// so it cannot turn into "anything under ~".
async fn root_visible_to(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    root: &std::path::Path,
) -> otto_core::Result<bool> {
    let member_roots: Vec<String> = ctx
        .workspaces
        .list_for_user(&user.id)
        .await?
        .into_iter()
        .map(|(w, _)| w.root_path)
        .collect();
    let scratch_root = ctx
        .workspaces
        .get(&otto_core::domain::SCRATCH_WORKSPACE_ID.to_string())
        .await
        .ok()
        .map(|w| w.root_path);
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        root_under_any(&root, &member_roots)
            || scratch_root.is_some_and(|s| canonical_eq(&root, &s))
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("lsp root check: {e}")))
}

fn canonical_of(p: &str) -> Option<std::path::PathBuf> {
    if p.trim().is_empty() {
        return None;
    }
    let p = std::path::Path::new(p);
    Some(std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
}

fn canonical_eq(root: &std::path::Path, other: &str) -> bool {
    canonical_of(other).is_some_and(|c| root == c)
}

/// Containment check: canonicalizes each workspace root (a symlinked
/// workspace path still matches). Empty roots never match.
fn root_under_any(root: &std::path::Path, workspace_roots: &[String]) -> bool {
    workspace_roots
        .iter()
        .filter_map(|w| canonical_of(w))
        .any(|c| root.starts_with(&c))
}

/// Relay one socket to its pooled server until either side goes away.
async fn serve_lsp(
    mut socket: WebSocket,
    pool: Arc<pool::LspPool>,
    key: pool::PoolKey,
    cmd: String,
    args: Vec<String>,
) {
    let mut attached = match pool.attach(key.clone(), &cmd, &args) {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(
                lang = key.lang.as_str(),
                cmd,
                "failed to spawn language server: {e}"
            );
            let err = serde_json::json!({
                "type": "error",
                "code": "spawn_failed",
                "message": e.to_string(),
            });
            let _ = socket.send(Message::Text(err.to_string().into())).await;
            return;
        }
    };

    loop {
        tokio::select! {
            // Pooled server → this socket (already routed + id-restored).
            out = attached.rx.recv() => {
                let Some(text) = out else { break };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
            // This socket → pooled server.
            ws_msg = socket.recv() => {
                let Some(Ok(ws_msg)) = ws_msg else {
                    break;
                };
                match ws_msg {
                    Message::Text(text) => {
                        if !attached.send(text.to_string()).await {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    // Ping/pong/binary: silently ignore.
                    _ => {}
                }
            }
        }
    }
    // Dropping `attached` detaches: its docs close, and the server is reaped
    // once no socket is left for IDLE_TIMEOUT.
}

// ---------------------------------------------------------------------------
// Router constructors
// ---------------------------------------------------------------------------

/// API routes (mounted under `/api/v1` with bearer auth): capabilities + install.
pub fn api_router() -> Router<ServerCtx> {
    Router::new()
        .route("/lsp/capabilities", get(capabilities))
        .route("/workspaces/{id}/lsp/install", post(install_servers))
}

/// Root-level WS router (self-authenticates via the `otto-bearer` subprotocol).
pub fn ws_router(authenticator: Arc<dyn TokenAuthenticator>, ctx: ServerCtx) -> Router {
    Router::new()
        .route("/ws/lsp", get(lsp_ws))
        .with_state(LspWsState {
            auth: authenticator,
            ctx,
            pool: pool::LspPool::new(pool::process_spawner(), pool::IDLE_TIMEOUT),
        })
}

#[cfg(test)]
mod root_tests {
    use super::*;

    #[test]
    fn lsp_root_must_sit_inside_a_member_workspace() {
        let d = tempfile::tempdir().unwrap();
        let ws = d.path().join("repo");
        let other = d.path().join("elsewhere");
        std::fs::create_dir_all(ws.join("sub")).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap();
        let roots = vec![ws.to_string_lossy().into_owned(), String::new()];
        assert!(root_under_any(&canon(&ws), &roots));
        assert!(root_under_any(&canon(&ws.join("sub")), &roots));
        assert!(!root_under_any(&canon(&other), &roots));
        assert!(
            !root_under_any(&canon(d.path()), &roots),
            "a parent is not inside"
        );
        // `repo-evil` shares a string prefix with `repo` but is not inside it.
        let evil = d.path().join("repo-evil");
        std::fs::create_dir_all(&evil).unwrap();
        assert!(!root_under_any(&canon(&evil), &roots));
        // Scratch (HOME) matches exactly, never its subtree.
        let home = d.path().to_string_lossy().into_owned();
        assert!(canonical_eq(&canon(d.path()), &home));
        assert!(!canonical_eq(&canon(&other), &home));
    }
}
