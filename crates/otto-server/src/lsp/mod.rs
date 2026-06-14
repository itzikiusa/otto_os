//! LSP backend: server registry, capabilities detection, WebSocket↔stdio
//! bridge, and language-server install helper.
//!
//! Routes exposed:
//!   GET  /api/v1/lsp/capabilities              → LspCapabilities (authed)
//!   POST /api/v1/workspaces/{id}/lsp/install   → Session (Editor role)
//!   GET  /ws/lsp?lang=&root=&token=            → WebSocket bridge (token auth)

pub mod framing;
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
use tokio::io::BufReader;
use tokio::process::Command;

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
    #[allow(dead_code)]
    ctx: ServerCtx,
}

#[derive(Deserialize)]
struct LspWsQuery {
    lang: Option<String>,
    root: Option<String>,
    token: Option<String>,
}

fn ws_problem(status: StatusCode, code: &str, message: &str) -> Response {
    let body = Problem {
        code: code.to_string(),
        message: message.to_string(),
    };
    (status, Json(body)).into_response()
}

/// `GET /ws/lsp?lang=<lang>&root=<path>&token=<token>`
async fn lsp_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<LspWsQuery>,
    State(st): State<LspWsState>,
) -> Response {
    // 1. Token auth before upgrade (mirrors term_ws).
    let token = match q.token {
        Some(t) => t,
        None => {
            return ws_problem(StatusCode::UNAUTHORIZED, "unauthorized", "missing token");
        }
    };
    if let Err(_) = st.auth.authenticate(&token).await {
        return ws_problem(StatusCode::UNAUTHORIZED, "unauthorized", "invalid token");
    }

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

    let cmd = resolved.command.clone();
    let args = resolved.args.clone();
    let root_owned = root.clone();
    let lang_owned = lang.clone();

    ws.on_upgrade(move |socket| async move {
        serve_lsp(socket, cmd, args, root_owned, lang_owned).await;
    })
}

/// Drive the WS↔stdio relay for one LSP session.
async fn serve_lsp(
    mut socket: WebSocket,
    cmd: String,
    args: Vec<String>,
    root: String,
    lang: String,
) {
    // Spawn the language server.
    let mut child = match Command::new(&cmd)
        .args(&args)
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(lang, cmd, "failed to spawn language server: {e}");
            let err = format!(
                r#"{{"type":"error","code":"spawn_failed","message":"{}"}}"#,
                e
            );
            let _ = socket.send(Message::Text(err.into())).await;
            return;
        }
    };

    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let mut stdout_reader = BufReader::new(stdout);

    // Drain stderr to debug logs in a background task.
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(lang = lang.as_str(), "lsp stderr: {line}");
        }
    });

    // Main relay loop.
    loop {
        tokio::select! {
            // Server stdout → WS text frame (strip Content-Length framing).
            msg = framing::read_message(&mut stdout_reader) => {
                match msg {
                    Ok(Some(body)) => {
                        match String::from_utf8(body) {
                            Ok(text) => {
                                if socket.send(Message::Text(text.into())).await.is_err() {
                                    break;
                                }
                            }
                            Err(_) => {
                                tracing::debug!("lsp: server sent non-UTF-8 body, dropping");
                            }
                        }
                    }
                    Ok(None) => {
                        // Server stdout closed (process exited).
                        tracing::debug!("lsp: server stdout closed");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!("lsp: stdout framing error: {e}");
                        break;
                    }
                }
            }

            // WS text frame → server stdin (add Content-Length framing).
            ws_msg = socket.recv() => {
                let Some(Ok(ws_msg)) = ws_msg else {
                    // WS closed or error.
                    break;
                };
                match ws_msg {
                    Message::Text(text) => {
                        let bytes = text.as_bytes();
                        if let Err(e) = framing::write_message(&mut stdin, bytes).await {
                            tracing::warn!("lsp: stdin write error: {e}");
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

    // Kill the server and close the socket.
    let _ = child.kill().await;
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

/// Root-level WS router (self-authenticates via `?token=`).
pub fn ws_router(authenticator: Arc<dyn TokenAuthenticator>, ctx: ServerCtx) -> Router {
    Router::new()
        .route("/ws/lsp", get(lsp_ws))
        .with_state(LspWsState {
            auth: authenticator,
            ctx,
        })
}
