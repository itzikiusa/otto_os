//! LSP backend Axum shell: capabilities, language-server install session
//! launcher, `/ws/lsp` WebSocket↔stdio relay, and workspace-root RBAC glue.
//!
//! Framing, server registry, path/secret confinement, Go build-tag syncing, and
//! the multiplexed server pool live in [`otto_ide::lsp_host`].
//!
//! Routes exposed:
//!   GET  /api/v1/lsp/capabilities                     → LspCapabilities (authed)
//!   POST /api/v1/workspaces/{id}/lsp/install          → Session (Editor role)
//!   GET  /ws/lsp?lang=&root=&connection_id=           → WebSocket bridge (`otto-bearer` subprotocol auth;
//!                                                       a Local target is root-only: 403 `lsp_requires_root`)

pub use otto_ide::lsp_host::ws_bridge::{InstallLspReq, LspCapabilities, LspServerStatus};
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
use otto_core::secret_paths::subtree_root_denial;
use otto_core::Id;
use otto_ide::lsp_host::ws_bridge::canonical_of;
use otto_ide::lsp_host::ws_bridge::{
    attach_lsp_ws_session, canonical_or_ancestor, detect_lsp_capabilities,
    plan_lsp_install_compound, prepare_lsp_ws_session, root_under_any, secret_root_denial,
    LspWsQuery, PreparedLspSession,
};

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// `GET /api/v1/lsp/capabilities` — available to any authenticated user
/// (gated by `Feature::Agents` View in `policy_for`).
async fn capabilities(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> ApiResult<Json<LspCapabilities>> {
    let caps = detect_lsp_capabilities(&ctx.ide).await.map_err(ApiError)?;
    Ok(Json(caps))
}

/// `POST /api/v1/workspaces/{id}/lsp/install` — Editor role required.
async fn install_servers(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<InstallLspReq>>,
) -> ApiResult<Json<otto_core::domain::Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let compound = plan_lsp_install_compound(&ctx.ide, body.and_then(|b| b.0.langs))
        .await
        .map_err(ApiError)?;

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
    let ide_plugins = ctx.ide.plugins.clone();
    tokio::spawn(async move {
        let _ = &ide_plugins;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        if let Err(e) = manager
            .input(&session_id, format!("{compound}\n").as_bytes())
            .await
        {
            tracing::warn!(session = %session_id, "lsp install command write failed: {e}");
        }
        servers::invalidate_detect_cache();
    });

    Ok(Json(session))
}

#[derive(Clone)]
struct LspWsState {
    auth: Arc<dyn TokenAuthenticator>,
    ctx: ServerCtx,
    pool: Arc<pool::LspPool>,
}

fn ws_problem(status: StatusCode, code: &str, message: &str) -> Response {
    let body = Problem {
        code: code.to_string(),
        message: message.to_string(),
    };
    (status, Json(body)).into_response()
}

/// `GET /ws/lsp?lang=<lang>&root=<path>[&file=<path>][&connection_id=<ssh_conn_id>]`
async fn lsp_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<LspWsQuery>,
    State(st): State<LspWsState>,
    headers: axum::http::HeaderMap,
) -> Response {
    let ws = ws.protocols([crate::ws_events::BEARER_SUBPROTOCOL]);
    let Some(token) = crate::ws_events::token_from_subprotocol(&headers) else {
        return ws_problem(StatusCode::UNAUTHORIZED, "unauthorized", "missing token");
    };
    let Ok(auth) = st.auth.authenticate(&token).await else {
        return ws_problem(StatusCode::UNAUTHORIZED, "unauthorized", "invalid token");
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
    let prepared = match prepare_lsp_ws_session(&st.ctx, &user, q).await {
        Ok(s) => s,
        Err(err) => return ws_problem(err.status, err.code, &err.message),
    };

    let pool = Arc::clone(&st.pool);
    ws.on_upgrade(move |socket| async move {
        serve_lsp(socket, pool, prepared).await;
    })
}

/// Authorize `root` against live workspace RBAC (`Editor` for writes, `Viewer`
/// for reads). Non-root users must hold explicit membership on a covering
/// workspace (no implicit `SCRATCH_WORKSPACE_ID` bypass).
pub async fn ide_local_root_authorized(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    root: &std::path::Path,
    write: bool,
) -> otto_core::Result<bool> {
    let has_root = if write {
        otto_core::auth::root_authority(user)
    } else {
        user.is_root
    };
    if has_root {
        return Ok(true);
    }
    let canon_root = tokio::task::spawn_blocking({
        let p = root.to_path_buf();
        move || canonical_or_ancestor(&p)
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("ide root check: {e}")))?;

    if !write && crate::routes::session_files::lsp_root_in_own_session(ctx, user, &canon_root).await
    {
        return Ok(true);
    }
    // `/`, `$HOME` or an ancestor of a protected dir is never a non-root IDE
    // root, nor does a workspace rooted there (any user can create one)
    // authorize what lies under it — `~/.zshrc` would be one save away.
    if secret_root_denial(&canon_root).is_some() || subtree_root_denial(&canon_root).is_some() {
        return Ok(false);
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
    tokio::task::spawn_blocking(move || {
        member_roots.retain(|r| canonical_of(r).is_some_and(|c| subtree_root_denial(&c).is_none()));
        root_under_any(&canon_root, &member_roots)
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("ide root check: {e}")))
}

async fn serve_lsp(mut socket: WebSocket, pool: Arc<pool::LspPool>, prepared: PreparedLspSession) {
    let (mut attached, attached_notice) = match attach_lsp_ws_session(pool, prepared).await {
        Ok(pair) => pair,
        Err(Some(err_json)) => {
            let _ = socket.send(Message::Text(err_json.into())).await;
            return;
        }
        Err(None) => return,
    };

    if socket
        .send(Message::Text(attached_notice.into()))
        .await
        .is_err()
    {
        return;
    }

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
