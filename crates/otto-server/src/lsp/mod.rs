//! LSP backend: server registry, capabilities detection, WebSocket↔stdio
//! bridge (Local + Remote SSH), and language-server install helper.
//!
//! Pure framing, registry (with `<data_dir>/toolchains/**/bin` and runtime IDE
//! plugin discovery), and the multiplexed server pool live in
//! [`otto_ide::lsp_host`] and are re-exported here.
//!
//! Routes exposed:
//!   GET  /api/v1/lsp/capabilities                     → LspCapabilities (authed)
//!   POST /api/v1/workspaces/{id}/lsp/install          → Session (Editor role)
//!   GET  /ws/lsp?lang=&root=&connection_id=           → WebSocket bridge (`otto-bearer` subprotocol auth)

pub use otto_ide::lsp_host::{framing, pool, servers};

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
/// Includes binaries under `<data_dir>/toolchains/**/bin` and active IDE plugins.
async fn capabilities(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<LspCapabilities>> {
    let resolved = servers::detect_all_with_ide(&ctx.data_dir, &ctx.ide.plugins);
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
async fn install_servers(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<InstallLspReq>>,
) -> ApiResult<Json<otto_core::domain::Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;

    let req_langs = body.and_then(|b| b.0.langs);
    let resolved = servers::detect_all_with_ide(&ctx.data_dir, &ctx.ide.plugins);
    let pairs: Vec<String> = match req_langs {
        Some(ref langs) => {
            let mut cmds = Vec::new();
            for lang in langs {
                let norm = servers::normalize_lsp_lang(lang);
                let entry = resolved.iter().find(|r| r.lang == norm);
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
        None => resolved
            .iter()
            .filter(|r| !r.available)
            .filter_map(|r| r.install_command.clone())
            .collect(),
    };

    if pairs.is_empty() {
        return Err(ApiError(Error::Invalid(
            "no languages need installation (all available, or no install commands)".into(),
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = pairs
        .into_iter()
        .filter(|cmd| seen.insert(cmd.clone()))
        .collect();
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

#[derive(Clone)]
struct LspWsState {
    auth: Arc<dyn TokenAuthenticator>,
    ctx: ServerCtx,
    /// Shared servers, one per `(user, lang, root, ssh)`.
    pool: Arc<pool::LspPool>,
}

#[derive(Deserialize)]
struct LspWsQuery {
    lang: Option<String>,
    root: Option<String>,
    /// Optional SSH connection ID when editing a Remote SSH workspace.
    #[serde(default)]
    connection_id: Option<String>,
}

fn ws_problem(status: StatusCode, code: &str, message: &str) -> Response {
    let body = Problem {
        code: code.to_string(),
        message: message.to_string(),
    };
    (status, Json(body)).into_response()
}

/// `GET /ws/lsp?lang=<lang>&root=<path>[&connection_id=<ssh_conn_id>]`
/// (bearer in `Sec-WebSocket-Protocol`)
async fn lsp_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<LspWsQuery>,
    State(st): State<LspWsState>,
    headers: axum::http::HeaderMap,
) -> Response {
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
    if root.chars().any(char::is_control) {
        return ws_problem(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "root contains invalid control characters",
        );
    }

    let ssh_conn_id = q
        .connection_id
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty());

    st.ctx.ide.environments.ensure_hydrated(&st.ctx.pool).await;
    let _ = st.ctx.ide.projects.ensure_hydrated(&st.ctx.pool).await;

    let (key, launch) = if let Some(conn_id) = ssh_conn_id {
        if !root.starts_with('/') {
            return ws_problem(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "remote SSH root must be an absolute path",
            );
        }
        let ssh_spec =
            match otto_ide::IdeCtx::resolve_ssh_connection(&st.ctx, &user, &conn_id).await {
                Ok(spec) => spec,
                Err(Error::NotFound(m)) => {
                    return ws_problem(StatusCode::NOT_FOUND, "not_found", &m);
                }
                Err(Error::Forbidden(m)) => {
                    return ws_problem(StatusCode::FORBIDDEN, "forbidden", &m);
                }
                Err(Error::Invalid(m)) => {
                    return ws_problem(StatusCode::BAD_REQUEST, "bad_request", &m);
                }
                Err(e) => {
                    tracing::warn!(connection_id = %conn_id, "lsp ssh lookup failed: {e}");
                    return ws_problem(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal",
                        "SSH connection lookup failed",
                    );
                }
            };
        let remote = match servers::resolve_remote_lang(&lang, &st.ctx.ide.plugins) {
            Some(r) => r,
            None => {
                return ws_problem(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    &format!("unknown language: '{lang}'"),
                );
            }
        };
        let norm_root = {
            let trimmed = root.trim_end_matches('/');
            if trimmed.is_empty() {
                "/".to_string()
            } else {
                trimmed.to_string()
            }
        };
        let key = pool::PoolKey {
            owner: user.id.to_string(),
            lang: remote.lang,
            root: norm_root,
            ssh: Some(ssh_spec),
        };
        let launch = pool::SpawnLaunch {
            cmd: remote.primary_cmd,
            args: remote.primary_args,
            fallback_cmd: remote.fallback_cmd,
            fallback_args: remote.fallback_args,
        };
        (key, launch)
    } else {
        let root_path = std::path::Path::new(&root);
        if !root_path.exists() || !root_path.is_dir() {
            return ws_problem(
                StatusCode::BAD_REQUEST,
                "bad_request",
                &format!("root does not exist or is not a directory: {root}"),
            );
        }
        let root = std::fs::canonicalize(root_path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(root);

        if !user.is_root {
            let root_ref = std::path::Path::new(&root);
            let visible =
                if crate::routes::session_files::lsp_root_in_own_session(&st.ctx, &user, root_ref)
                    .await
                {
                    Ok(true)
                } else if let Some(why) = secret_root_denial(root_ref) {
                    return ws_problem(StatusCode::FORBIDDEN, "forbidden", &why);
                } else {
                    root_visible_to(&st.ctx, &user, root_ref).await
                };
            match visible {
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

        let resolved =
            match servers::resolve_lang_with_ide(&lang, &st.ctx.data_dir, &st.ctx.ide.plugins) {
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
        let key = pool::PoolKey {
            owner: user.id.to_string(),
            lang: resolved.lang,
            root,
            ssh: None,
        };
        let launch = pool::SpawnLaunch::primary(resolved.command, resolved.args);
        (key, launch)
    };

    let pool = Arc::clone(&st.pool);
    ws.on_upgrade(move |socket| async move {
        serve_lsp(socket, pool, key, launch).await;
    })
}

pub(crate) fn canonical_or_ancestor(p: &std::path::Path) -> std::path::PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let mut cur = p;
    let mut tail = Vec::new();
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_os_string());
        }
        if let Ok(mut base) = std::fs::canonicalize(parent) {
            for comp in tail.into_iter().rev() {
                base.push(comp);
            }
            return base;
        }
        cur = parent;
    }
    p.to_path_buf()
}

/// Authorize `root` against live workspace RBAC (`Editor` for writes, `Viewer`
/// for reads). Saved IDE project attachments and cached environment metadata
/// are user preferences rather than access grants, so every primary and
/// additional root must sit under a currently granted workspace or registered
/// workspace repository.
pub async fn ide_local_root_authorized(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    root: &std::path::Path,
    write: bool,
) -> otto_core::Result<bool> {
    if user.is_root {
        return Ok(true);
    }
    let canon_root = tokio::task::spawn_blocking({
        let p = root.to_path_buf();
        move || canonical_or_ancestor(&p)
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("ide root check: {e}")))?;

    if secret_root_denial(&canon_root).is_some() {
        return Ok(false);
    }
    if !write && crate::routes::session_files::lsp_root_in_own_session(ctx, user, &canon_root).await
    {
        return Ok(true);
    }

    let min_role = if write {
        WorkspaceRole::Editor
    } else {
        WorkspaceRole::Viewer
    };
    let workspaces = ctx.workspaces.list_for_user(&user.id).await?;
    let git = otto_state::GitStore::new(ctx.pool.clone());
    let mut member_roots: Vec<String> = Vec::with_capacity(workspaces.len() * 2);
    for (w, _) in workspaces {
        if ctx.roles.check(user, &w.id, min_role).await.is_err() {
            continue;
        }
        match git.list_repos(&w.id).await {
            Ok(repos) => member_roots.extend(repos.into_iter().map(|r| r.path)),
            Err(e) => tracing::warn!(workspace = %w.id, "ide root check: list repos: {e}"),
        }
        member_roots.push(w.root_path);
    }
    let scratch_root = if write {
        None
    } else {
        ctx.workspaces
            .get(&otto_core::domain::SCRATCH_WORKSPACE_ID.to_string())
            .await
            .ok()
            .map(|w| w.root_path)
    };
    tokio::task::spawn_blocking(move || {
        root_under_any(&canon_root, &member_roots)
            || scratch_root.is_some_and(|s| canonical_eq(&canon_root, &s))
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("ide root check: {e}")))
}

async fn root_visible_to(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    root: &std::path::Path,
) -> otto_core::Result<bool> {
    ide_local_root_authorized(ctx, user, root, false).await
}

fn secret_root_denial(canonical: &std::path::Path) -> Option<String> {
    use otto_core::secret_paths as sp;
    (sp::in_protected_dir(canonical) || sp::is_denied_file(canonical)).then(|| {
        format!(
            "{} holds credentials or Otto's own state; only the root user can open it",
            canonical.display()
        )
    })
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
    launch: pool::SpawnLaunch,
) {
    let attach_res = {
        let pool = Arc::clone(&pool);
        let key_cl = key.clone();
        let launch_cl = launch.clone();
        tokio::task::spawn_blocking(move || pool.attach_launch(key_cl, &launch_cl)).await
    };
    let mut attached = match attach_res {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            tracing::error!(
                lang = key.lang.as_str(),
                cmd = launch.cmd.as_str(),
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
        Err(join_err) => {
            tracing::error!(
                lang = key.lang.as_str(),
                "lsp attach join error: {join_err}"
            );
            return;
        }
    };

    loop {
        tokio::select! {
            out = attached.rx.recv() => {
                let Some(text) = out else { break };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
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
                    _ => {}
                }
            }
        }
    }
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
    let pool = Arc::clone(&ctx.ide.lsp_pool);
    Router::new()
        .route("/ws/lsp", get(lsp_ws))
        .with_state(LspWsState {
            auth: authenticator,
            ctx,
            pool,
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
        let evil = d.path().join("repo-evil");
        std::fs::create_dir_all(&evil).unwrap();
        assert!(!root_under_any(&canon(&evil), &roots));
        let repo = d.path().join("outside-repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let with_repo = vec![
            ws.to_string_lossy().into_owned(),
            repo.to_string_lossy().into_owned(),
        ];
        assert!(!root_under_any(&canon(&repo), &roots));
        assert!(root_under_any(&canon(&repo), &with_repo));
        assert!(root_under_any(&canon(&repo.join("src")), &with_repo));
        assert!(!root_under_any(&canon(&other), &with_repo));
        let home = d.path().to_string_lossy().into_owned();
        assert!(canonical_eq(&canon(d.path()), &home));
        assert!(!canonical_eq(&canon(&other), &home));
    }

    #[test]
    fn lsp_root_keeps_the_fs_secret_deny() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let home = std::path::PathBuf::from(home);
        assert!(secret_root_denial(&home.join(".ssh")).is_some());
        assert!(secret_root_denial(&home.join(".ssh/keys")).is_some());
        let d = tempfile::tempdir().unwrap();
        let plain = std::fs::canonicalize(d.path()).unwrap();
        assert!(secret_root_denial(&plain).is_none());
    }
}
