//! Axum REST & SSE routers for `otto-ide` (`/ide/*` and `/ide/collab/guest/*`).
//!
//! - [`api_router`]: protected `/ide/*` routes nested under `/api/v1` (enforced by
//!   `auth_middleware` + `otto-server/src/policy.rs`).
//! - [`public_router`]: capability-token `/ide/collab/guest/*` routes nested under
//!   `/api/v1` for "Share with Friends" live collaborative editing and co-debugging.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use bytes::Bytes;
use futures_util::stream;
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::dap::DapStartContext;
use crate::types::{
    IdeCodeActionsReq, IdeCodeActionsResp, IdeCollabCursorReq, IdeCollabDebugActionReq,
    IdeCollabEditReq, IdeCollabEditResp, IdeCollabEvent, IdeCollabJoinReq, IdeCollabJoinResp,
    IdeCollabRoleReq, IdeCollabRoom, IdeCollabRoomResp, IdeCollabRoomsResp, IdeCollabStateResp,
    IdeCreateCollabRoomReq, IdeCreateCollabRoomResp, IdeCreatePythonVenvReq,
    IdeCreatePythonVenvResp, IdeDebugAction, IdeDebugCommandReq, IdeDebugEvaluateReq,
    IdeDebugEvaluateResp, IdeDebugSessionResp, IdeDebugSessionSnapshot, IdeDebugSessionsResp,
    IdeDebugVariablesReq, IdeDebugVariablesResp, IdeEnvironmentResp, IdeEnvironmentsResp,
    IdeFileQuery, IdeFileReadResp, IdeFileWriteReq, IdeFileWriteResp, IdeFormatReq, IdeFormatResp,
    IdeFsEntryKind, IdeFsMutateReq, IdeFsMutateResp, IdeFsTreeQuery, IdeFsTreeResp, IdeGutterReq,
    IdeGutterResp, IdeInstallJobResp, IdeInstallPluginReq, IdeInstallToolchainReq, IdeOkResp,
    IdePluginResp, IdePluginsResp, IdeReferencesReq, IdeReferencesResp, IdeRenameReq,
    IdeRenameResp, IdeReplaceReq, IdeReplaceResp, IdeRunConfigResp, IdeRunResp, IdeSearchReq,
    IdeSearchResp, IdeSetBreakpointsReq, IdeSshSpec, IdeStartDebugReq, IdeStartRunReq,
    IdeSymbolsReq, IdeSymbolsResp, IdeTargetKind, IdeTargetQuery, IdeTargetSpec,
    IdeTogglePluginReq, IdeToolchainsResp, IdeUpdateEnvironmentReq, IdeUpsertRunConfigReq,
};
use crate::IdeCtx;

const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Problem-details error wrapper mapping [`otto_core::Error`] to HTTP status codes.
pub(crate) struct ApiErr(pub Error);

impl From<Error> for ApiErr {
    fn from(err: Error) -> Self {
        Self(err)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

/// Protected `/ide/*` REST & SSE routes (nested under `/api/v1`).
pub fn api_router<S: IdeCtx>() -> Router<S> {
    Router::new()
        // A. Target Filesystem, Search, Gutter & Extended LSP
        .route("/ide/fs/tree", get(fs_tree::<S>))
        .route(
            "/ide/fs/file",
            get(fs_read_file::<S>).put(fs_write_file::<S>),
        )
        .route("/ide/fs/mutate", post(fs_mutate::<S>))
        .route("/ide/search", post(search_workspace::<S>))
        .route("/ide/replace", post(replace_workspace::<S>))
        .route("/ide/gutter", post(detect_gutter::<S>))
        .route("/ide/lsp/symbols", post(lsp_symbols::<S>))
        .route("/ide/lsp/references", post(lsp_references::<S>))
        .route("/ide/lsp/rename", post(lsp_rename::<S>))
        .route("/ide/lsp/format", post(lsp_format::<S>))
        .route("/ide/lsp/code-actions", post(lsp_code_actions::<S>))
        // B. OS/Arch Toolchains & SDK Downloader
        .route("/ide/toolchains", get(list_toolchains::<S>))
        .route("/ide/toolchains/install", post(install_toolchain::<S>))
        .route("/ide/toolchains/jobs/{job_id}", get(get_toolchain_job::<S>))
        // C. Environments, Python Venvs, Run Configurations & Test Runner
        .route(
            "/ide/environments",
            get(get_environments::<S>).put(put_environments::<S>),
        )
        .route(
            "/ide/environments/python-venv",
            post(create_python_venv::<S>),
        )
        .route("/ide/run-configs", post(upsert_run_config::<S>))
        .route("/ide/run-configs/{id}", delete(delete_run_config::<S>))
        .route("/ide/runs", post(start_run::<S>))
        .route("/ide/runs/{run_id}", get(get_run::<S>))
        .route("/ide/runs/{run_id}/stop", post(stop_run::<S>))
        // D. Full DAP Debugger Engine
        .route(
            "/ide/debug/sessions",
            get(list_debug_sessions::<S>).post(start_debug_session::<S>),
        )
        .route("/ide/debug/sessions/{id}", get(get_debug_session::<S>))
        .route(
            "/ide/debug/sessions/{id}/events",
            get(stream_debug_events::<S>),
        )
        .route(
            "/ide/debug/sessions/{id}/breakpoints",
            put(set_debug_breakpoints::<S>),
        )
        .route("/ide/debug/sessions/{id}/command", post(debug_command::<S>))
        .route(
            "/ide/debug/sessions/{id}/variables",
            post(debug_variables::<S>),
        )
        .route(
            "/ide/debug/sessions/{id}/evaluate",
            post(debug_evaluate::<S>),
        )
        // E. "Share with Friends" Host Routes
        .route(
            "/ide/collab/rooms",
            get(list_collab_rooms::<S>).post(create_collab_room::<S>),
        )
        .route(
            "/ide/collab/rooms/{room_id}/role",
            post(set_collab_role::<S>),
        )
        .route(
            "/ide/collab/rooms/{room_id}",
            delete(close_collab_room::<S>),
        )
        // F. Runtime IDE Plugins
        .route("/ide/plugins", get(list_plugins::<S>))
        .route("/ide/plugins/install", post(install_plugin::<S>))
        .route("/ide/plugins/{slug}/toggle", post(toggle_plugin::<S>))
        .route("/ide/plugins/{slug}", delete(uninstall_plugin::<S>))
}

/// Public capability-token `/ide/collab/guest/*` routes (nested under `/api/v1`).
pub fn public_router<S: IdeCtx>() -> Router<S> {
    Router::new()
        .route(
            "/ide/collab/guest/{room_id}/join",
            post(collab_guest_join::<S>),
        )
        .route(
            "/ide/collab/guest/{room_id}/state",
            get(collab_guest_state::<S>),
        )
        .route(
            "/ide/collab/guest/{room_id}/events",
            get(collab_guest_events::<S>),
        )
        .route(
            "/ide/collab/guest/{room_id}/cursor",
            post(collab_guest_cursor::<S>),
        )
        .route(
            "/ide/collab/guest/{room_id}/edit",
            post(collab_guest_edit::<S>),
        )
        .route(
            "/ide/collab/guest/{room_id}/debug-action",
            post(collab_guest_debug_action::<S>),
        )
}

async fn resolve_ssh_opt<S: IdeCtx>(
    ctx: &S,
    user: &User,
    target: &IdeTargetSpec,
) -> Result<Option<IdeSshSpec>> {
    if target.kind == IdeTargetKind::Ssh {
        let conn_id = target
            .connection_id
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::Invalid("connection_id is required for SSH targets".into()))?;
        Ok(Some(ctx.resolve_ssh_connection(user, conn_id).await?))
    } else {
        Ok(None)
    }
}

// ===========================================================================
// A. Target Filesystem, Search, Gutter & Extended LSP Handlers
// ===========================================================================

async fn fs_tree<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeFsTreeQuery>,
) -> ApiResult<Json<IdeFsTreeResp>> {
    let spec = q.target_spec()?;
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &spec).await?;
    let mut resp = ctx
        .ide()
        .targets
        .list_tree(&resolved, q.path.as_deref(), q.show_hidden)
        .await?;
    for entry in &mut resp.entries {
        if entry.kind == IdeFsEntryKind::File && entry.language.is_none() {
            if let Some(plugin_lang) = ctx.ide().plugins.detect_language_for_path(&entry.path) {
                entry.language = Some(plugin_lang.id);
            }
        }
    }
    Ok(Json(resp))
}

async fn fs_read_file<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeFileQuery>,
) -> ApiResult<Json<IdeFileReadResp>> {
    let spec = q.target_spec()?;
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &spec).await?;
    let mut resp = ctx.ide().targets.read_file(&resolved, &q.path).await?;
    if resp.language == "plaintext" {
        if let Some(plugin_lang) = ctx.ide().plugins.detect_language_for_path(&resp.path) {
            resp.language = plugin_lang.id;
        }
    }
    Ok(Json(resp))
}

async fn fs_write_file<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFileWriteReq>,
) -> ApiResult<Json<IdeFileWriteResp>> {
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx
        .ide()
        .targets
        .write_file(
            &resolved,
            &req.path,
            &req.content,
            req.expected_sha256.as_deref(),
            req.create_dirs,
        )
        .await?;
    Ok(Json(resp))
}

async fn fs_mutate<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFsMutateReq>,
) -> ApiResult<Json<IdeFsMutateResp>> {
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx
        .ide()
        .targets
        .mutate_fs(&resolved, req.op, &req.path, req.dest_path.as_deref())
        .await?;
    Ok(Json(resp))
}

async fn search_workspace<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeSearchReq>,
) -> ApiResult<Json<IdeSearchResp>> {
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx.ide().targets.search(&resolved, &req).await?;
    Ok(Json(resp))
}

async fn replace_workspace<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeReplaceReq>,
) -> ApiResult<Json<IdeReplaceResp>> {
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx.ide().targets.replace(&resolved, &req).await?;
    Ok(Json(resp))
}

async fn detect_gutter<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeGutterReq>,
) -> ApiResult<Json<IdeGutterResp>> {
    req.target.validate()?;
    let content = match req.content {
        Some(c) => c,
        None => {
            let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
            ctx.ide()
                .targets
                .read_file(&resolved, &req.path)
                .await?
                .content
        }
    };
    let extra = ctx.ide().plugins.active_gutter_detectors();
    let items = crate::gutter::detect_gutter_items(&req.path, &content, &extra);
    Ok(Json(IdeGutterResp { items }))
}

async fn lsp_symbols<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeSymbolsReq>,
) -> ApiResult<Json<IdeSymbolsResp>> {
    Ok(Json(
        crate::lsp_service::document_symbols(&ctx, &user, req).await?,
    ))
}

async fn lsp_references<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    Ok(Json(
        crate::lsp_service::find_references(&ctx, &user, req).await?,
    ))
}

async fn lsp_rename<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeRenameReq>,
) -> ApiResult<Json<IdeRenameResp>> {
    Ok(Json(
        crate::lsp_service::rename_symbol(&ctx, &user, req).await?,
    ))
}

async fn lsp_format<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFormatReq>,
) -> ApiResult<Json<IdeFormatResp>> {
    Ok(Json(
        crate::lsp_service::format_document(&ctx, &user, req).await?,
    ))
}

async fn lsp_code_actions<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCodeActionsReq>,
) -> ApiResult<Json<IdeCodeActionsResp>> {
    Ok(Json(
        crate::lsp_service::code_actions(&ctx, &user, req).await?,
    ))
}

// ===========================================================================
// B. OS/Arch Toolchains & SDK Downloader Handlers
// ===========================================================================

async fn list_toolchains<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeToolchainsResp>> {
    let target = q.into_target_spec()?;
    let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &target).await?;
    let active_env = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), &target)
        .await
        .ok();
    let resp = ctx
        .ide()
        .toolchains
        .list_toolchains(
            &ctx.ide().targets,
            &target,
            ssh.as_ref(),
            active_env.as_ref(),
        )
        .await?;
    Ok(Json(resp))
}

async fn install_toolchain<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeInstallToolchainReq>,
) -> ApiResult<Json<IdeInstallJobResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let job = ctx
        .ide()
        .toolchains
        .start_install(&ctx.ide().targets, req, ssh)
        .await?;
    Ok(Json(IdeInstallJobResp { job }))
}

async fn get_toolchain_job<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(job_id): Path<String>,
) -> ApiResult<Json<IdeInstallJobResp>> {
    let job = ctx.ide().toolchains.get_job(&job_id)?;
    Ok(Json(IdeInstallJobResp { job }))
}

// ===========================================================================
// C. Project Environments, Python Venvs, Run Configs & Test Runner Handlers
// ===========================================================================

async fn get_environments<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeEnvironmentsResp>> {
    let target = q.into_target_spec()?;
    let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &target).await?;
    let resp = ctx
        .ide()
        .environments
        .get_environments(
            &ctx.pool(),
            ctx.secrets(),
            &ctx.ide().targets,
            &target,
            ssh.as_ref(),
        )
        .await?;
    Ok(Json(resp))
}

async fn put_environments<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeUpdateEnvironmentReq>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let env = ctx
        .ide()
        .environments
        .update_environment(&ctx.pool(), ctx.secrets(), req)
        .await?;
    Ok(Json(IdeEnvironmentResp { env }))
}

async fn create_python_venv<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCreatePythonVenvReq>,
) -> ApiResult<Json<IdeCreatePythonVenvResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let resp = ctx
        .ide()
        .environments
        .create_python_venv(&ctx.pool(), &ctx.ide().targets, req, ssh.as_ref())
        .await?;
    Ok(Json(resp))
}

async fn upsert_run_config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeUpsertRunConfigReq>,
) -> ApiResult<Json<IdeRunConfigResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let config = ctx
        .ide()
        .environments
        .upsert_run_config(&ctx.pool(), req)
        .await?;
    Ok(Json(IdeRunConfigResp { config }))
}

async fn delete_run_config<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeOkResp>> {
    if q.root.as_deref().is_some_and(|r| !r.trim().is_empty()) {
        let target = q.into_target_spec()?;
        let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    }
    ctx.ide()
        .environments
        .delete_run_config(&ctx.pool(), &id)
        .await?;
    Ok(Json(IdeOkResp { ok: true }))
}

async fn start_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeStartRunReq>,
) -> ApiResult<Json<IdeRunResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let run = ctx
        .ide()
        .environments
        .start_run(&ctx.pool(), ctx.secrets(), &ctx.ide().targets, req, ssh)
        .await?;
    Ok(Json(IdeRunResp { run }))
}

async fn get_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<IdeRunResp>> {
    let run = ctx.ide().environments.get_run(&run_id)?;
    Ok(Json(IdeRunResp { run }))
}

async fn stop_run<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<IdeRunResp>> {
    let run = ctx.ide().environments.stop_run(&run_id).await?;
    Ok(Json(IdeRunResp { run }))
}

// ===========================================================================
// D. Full DAP Debugger Engine Handlers + SSE Event Stream
// ===========================================================================

async fn start_debug_session<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeStartDebugReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
    let effective_env = ctx
        .ide()
        .environments
        .build_effective_env(&ctx.pool(), ctx.secrets(), &req.target)
        .await
        .unwrap_or_default();
    let project_env = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), &req.target)
        .await
        .ok();
    let run_config = match req.config_id.as_deref().filter(|s| !s.is_empty()) {
        Some(cid) => ctx
            .ide()
            .environments
            .get_run_config(&ctx.pool(), cid)
            .await
            .ok(),
        None => None,
    };
    let plugin_adapters = ctx.ide().plugins.active_dap_adapters();
    let start_ctx = DapStartContext::new()
        .with_ssh(ssh)
        .with_effective_env(effective_env)
        .with_project_env(project_env)
        .with_run_config(run_config)
        .with_plugin_adapters(plugin_adapters)
        .with_caller_is_root(user.is_root);

    let target_for_collab = req.target.clone();
    let session = ctx.ide().dap.start_session(req, start_ctx).await?;
    ctx.ide()
        .collab
        .publish_debug_snapshot(Some(&target_for_collab), &session);

    // Forward asynchronous DAP events (breakpoint stops, console output, termination)
    // to any active Share-with-Friends room bound to this target.
    if let Ok(mut rx) = ctx.ide().dap.subscribe_events(&session.id) {
        let engine = ctx.ide().clone();
        let target_clone = target_for_collab;
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(snap) => {
                        engine
                            .collab
                            .publish_debug_snapshot(Some(&target_clone), &snap);
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    Ok(Json(IdeDebugSessionResp { session }))
}

async fn list_debug_sessions<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeDebugSessionsResp>> {
    Ok(Json(IdeDebugSessionsResp {
        sessions: ctx.ide().dap.list_sessions(),
    }))
}

async fn get_debug_session<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let session = ctx.ide().dap.get_session(&id)?;
    Ok(Json(IdeDebugSessionResp { session }))
}

async fn stream_debug_events<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Response> {
    let initial = ctx.ide().dap.get_session(&id)?;
    let rx = ctx.ide().dap.subscribe_events(&id)?;
    Ok(build_debug_sse_response(initial, rx))
}

async fn set_debug_breakpoints<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeSetBreakpointsReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let session = ctx.ide().dap.set_breakpoints(&id, req).await?;
    ctx.ide().collab.publish_debug_snapshot(None, &session);
    Ok(Json(IdeDebugSessionResp { session }))
}

async fn debug_command<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugCommandReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let session = ctx.ide().dap.command(&id, req).await?;
    ctx.ide().collab.publish_debug_snapshot(None, &session);
    Ok(Json(IdeDebugSessionResp { session }))
}

async fn debug_variables<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugVariablesReq>,
) -> ApiResult<Json<IdeDebugVariablesResp>> {
    let variables = ctx
        .ide()
        .dap
        .expand_variables(&id, req.variables_reference)
        .await?;
    Ok(Json(IdeDebugVariablesResp { variables }))
}

async fn debug_evaluate<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDebugEvaluateReq>,
) -> ApiResult<Json<IdeDebugEvaluateResp>> {
    let resp = ctx.ide().dap.evaluate(&id, req).await?;
    Ok(Json(resp))
}

// ===========================================================================
// E. "Share with Friends" Host Routes
// ===========================================================================

async fn create_collab_room<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCreateCollabRoomReq>,
) -> ApiResult<Json<IdeCreateCollabRoomResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx.ide().collab.create_room(&user, req)?;
    Ok(Json(resp))
}

async fn list_collab_rooms<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeCollabRoomsResp>> {
    let rooms = ctx.ide().collab.list_rooms(&user)?;
    Ok(Json(IdeCollabRoomsResp { rooms }))
}

async fn set_collab_role<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabRoleReq>,
) -> ApiResult<Json<IdeCollabRoomResp>> {
    let room = ctx
        .ide()
        .collab
        .set_peer_role(&user, &room_id, &req.peer_id, req.role)?;
    Ok(Json(IdeCollabRoomResp { room }))
}

async fn close_collab_room<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(room_id): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().collab.close_room(&user, &room_id)?;
    Ok(Json(IdeOkResp { ok: true }))
}

// ===========================================================================
// F. Runtime IDE Plugins Routes
// ===========================================================================

async fn list_plugins<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<IdePluginsResp>> {
    let (plugins, catalog) = ctx.ide().plugins.list();
    Ok(Json(IdePluginsResp { plugins, catalog }))
}

async fn install_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Json(req): Json<IdeInstallPluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    let plugin = ctx.ide().plugins.install(&req.source).await?;
    Ok(Json(IdePluginResp { plugin }))
}

async fn toggle_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(slug): Path<String>,
    Json(req): Json<IdeTogglePluginReq>,
) -> ApiResult<Json<IdePluginResp>> {
    let plugin = ctx.ide().plugins.toggle(&slug, req.enabled)?;
    Ok(Json(IdePluginResp { plugin }))
}

async fn uninstall_plugin<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(slug): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().plugins.uninstall(&slug)?;
    Ok(Json(IdeOkResp { ok: true }))
}

// ===========================================================================
// Public Capability-Token Guest Collaboration Routes
// ===========================================================================

#[derive(Debug, Default, Deserialize)]
struct CollabGuestQuery {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    peer_id: Option<String>,
}

fn extract_collab_token(headers: &HeaderMap, query_token: Option<&str>) -> Result<String> {
    if let Some(tok) = query_token.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(tok.to_string());
    }
    if let Some(hv) = headers
        .get("x-ide-collab-token")
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = hv.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = auth
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Ok(tok.to_string());
        }
    }
    Err(Error::Unauthorized)
}

async fn collab_guest_join<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Json(mut req): Json<IdeCollabJoinReq>,
) -> ApiResult<Json<IdeCollabJoinResp>> {
    if req.token.trim().is_empty() {
        req.token = extract_collab_token(&headers, None)?;
    }
    let resp = ctx.ide().collab.join_room(&room_id, req)?;
    Ok(Json(resp))
}

async fn collab_guest_state<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<CollabGuestQuery>,
) -> ApiResult<Json<IdeCollabStateResp>> {
    let token = extract_collab_token(&headers, q.token.as_deref())?;
    let mut state = ctx.ide().collab.get_state(&room_id, &token)?;
    if let Some(debug_id) = state.room.active_debug_id.as_deref() {
        if let Ok(fresh) = ctx.ide().dap.get_session(debug_id) {
            state.active_debug = Some(fresh);
        }
    }
    Ok(Json(state))
}

async fn collab_guest_events<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<CollabGuestQuery>,
) -> ApiResult<Response> {
    let _ = &q.peer_id;
    let token = extract_collab_token(&headers, q.token.as_deref())?;
    let (room, rx) = ctx.ide().collab.subscribe(&room_id, &token)?;
    Ok(build_collab_sse_response(room, rx))
}

async fn collab_guest_cursor<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabCursorReq>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().collab.update_cursor(&room_id, req)?;
    Ok(Json(IdeOkResp { ok: true }))
}

async fn collab_guest_edit<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabEditReq>,
) -> ApiResult<Json<IdeCollabEditResp>> {
    let (target, _host_user_id) =
        ctx.ide()
            .collab
            .authorize_editor(&room_id, &req.peer_id, &req.peer_secret)?;
    let mut saved_to_target = false;
    if req.save && target.kind == IdeTargetKind::Local {
        let resolved = ctx.ide().targets.resolve_local(&target, false)?;
        ctx.ide()
            .targets
            .write_file(&resolved, &req.path, &req.content, None, false)
            .await?;
        saved_to_target = true;
    }
    let version = ctx
        .ide()
        .collab
        .apply_edit(&room_id, &req, saved_to_target)?;
    Ok(Json(IdeCollabEditResp { version }))
}

async fn collab_guest_debug_action<S: IdeCtx>(
    State(ctx): State<S>,
    Path(room_id): Path<String>,
    Json(req): Json<IdeCollabDebugActionReq>,
) -> ApiResult<Json<IdeDebugSessionResp>> {
    let (target, _host_user_id) = ctx.ide().collab.authorize_debug_action(&room_id, &req)?;

    let mut latest_snap = None;
    if let Some(breakpoints) = req.breakpoints.clone() {
        let updated = ctx
            .ide()
            .dap
            .set_breakpoints(
                &req.debug_id,
                IdeSetBreakpointsReq {
                    breakpoints,
                    break_on_exceptions: false,
                },
            )
            .await?;
        latest_snap = Some(updated);
    }

    let skip_cmd = latest_snap.is_some()
        && req.action == IdeDebugAction::SelectFrame
        && req.frame_id.is_none();
    let session = if skip_cmd {
        latest_snap.unwrap()
    } else {
        ctx.ide()
            .dap
            .command(
                &req.debug_id,
                IdeDebugCommandReq {
                    action: req.action,
                    thread_id: req.thread_id,
                    frame_id: req.frame_id,
                    path: req.path,
                    line: req.line,
                },
            )
            .await?
    };

    ctx.ide()
        .collab
        .publish_debug_snapshot(Some(&target), &session);
    Ok(Json(IdeDebugSessionResp { session }))
}

// ===========================================================================
// Server-Sent Events (`text/event-stream`) Framing Helpers
// ===========================================================================

fn sse_headers_response(body: Body) -> Response {
    let mut resp = Response::new(body);
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/event-stream; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        HeaderName::from_static("x-accel-buffering"),
        HeaderValue::from_static("no"),
    );
    resp
}

fn format_sse_data(value: &Value) -> Bytes {
    let payload = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    Bytes::from(format!("data: {payload}\n\n"))
}

fn build_debug_sse_response(
    initial: IdeDebugSessionSnapshot,
    rx: broadcast::Receiver<IdeDebugSessionSnapshot>,
) -> Response {
    struct State {
        initial: Option<IdeDebugSessionSnapshot>,
        rx: broadcast::Receiver<IdeDebugSessionSnapshot>,
    }

    let stream = stream::unfold(
        State {
            initial: Some(initial),
            rx,
        },
        |mut st| async move {
            if let Some(first) = st.initial.take() {
                let val = serde_json::to_value(&first).unwrap_or_else(|_| json!({}));
                return Some((Ok::<Bytes, Infallible>(format_sse_data(&val)), st));
            }
            match tokio::time::timeout(SSE_KEEPALIVE_INTERVAL, st.rx.recv()).await {
                Ok(Ok(snap)) => {
                    let val = serde_json::to_value(&snap).unwrap_or_else(|_| json!({}));
                    Some((Ok(format_sse_data(&val)), st))
                }
                Ok(Err(RecvError::Lagged(_))) => {
                    Some((Ok(Bytes::from_static(b": lagged\n\n")), st))
                }
                Ok(Err(RecvError::Closed)) => None,
                Err(_) => Some((Ok(Bytes::from_static(b": keepalive\n\n")), st)),
            }
        },
    );

    sse_headers_response(Body::from_stream(stream))
}

fn format_collab_event_frame(ev: &IdeCollabEvent) -> Bytes {
    let mut val = serde_json::to_value(ev).unwrap_or_else(|_| json!({}));
    if let Value::Object(ref mut map) = val {
        match ev {
            IdeCollabEvent::CursorMoved { .. } => {
                map.insert("kind".into(), Value::String("cursor".into()));
            }
            IdeCollabEvent::BufferUpdated { .. } => {
                map.insert("kind".into(), Value::String("edit".into()));
            }
            _ => {}
        }
    }
    format_sse_data(&val)
}

fn build_collab_sse_response(
    room: IdeCollabRoom,
    rx: broadcast::Receiver<IdeCollabEvent>,
) -> Response {
    struct State {
        initial_room: Option<IdeCollabRoom>,
        rx: broadcast::Receiver<IdeCollabEvent>,
        done: bool,
    }

    let stream = stream::unfold(
        State {
            initial_room: Some(room),
            rx,
            done: false,
        },
        |mut st| async move {
            if st.done {
                return None;
            }
            if let Some(r) = st.initial_room.take() {
                let val = json!({ "type": "room_state", "room": r });
                return Some((Ok::<Bytes, Infallible>(format_sse_data(&val)), st));
            }
            match tokio::time::timeout(SSE_KEEPALIVE_INTERVAL, st.rx.recv()).await {
                Ok(Ok(ev)) => {
                    if matches!(ev, IdeCollabEvent::RoomClosed { .. }) {
                        st.done = true;
                    }
                    let frame = format_collab_event_frame(&ev);
                    Some((Ok(frame), st))
                }
                Ok(Err(RecvError::Lagged(_))) => {
                    Some((Ok(Bytes::from_static(b": lagged\n\n")), st))
                }
                Ok(Err(RecvError::Closed)) => None,
                Err(_) => Some((Ok(Bytes::from_static(b": keepalive\n\n")), st)),
            }
        },
    );

    sse_headers_response(Body::from_stream(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IdeSelectionRange;

    #[test]
    fn collab_token_extraction_checks_query_header_and_bearer() {
        let mut headers = HeaderMap::new();
        assert!(matches!(
            extract_collab_token(&headers, None),
            Err(Error::Unauthorized)
        ));
        assert_eq!(
            extract_collab_token(&headers, Some("  tok_q  ")).unwrap(),
            "tok_q"
        );

        headers.insert("x-ide-collab-token", HeaderValue::from_static("tok_hdr"));
        assert_eq!(extract_collab_token(&headers, None).unwrap(), "tok_hdr");

        let mut bearer_headers = HeaderMap::new();
        bearer_headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer tok_bearer"),
        );
        assert_eq!(
            extract_collab_token(&bearer_headers, None).unwrap(),
            "tok_bearer"
        );
    }

    #[test]
    fn collab_sse_framing_includes_canonical_type_and_ui_kind_tags() {
        let cursor_ev = IdeCollabEvent::CursorMoved {
            peer_id: "peer_1".into(),
            display_name: "Alice".into(),
            color: "hsl(212, 88%, 56%)".into(),
            path: "src/main.rs".into(),
            selections: vec![IdeSelectionRange { anchor: 4, head: 9 }],
            active_line: 3,
        };
        let raw = String::from_utf8(format_collab_event_frame(&cursor_ev).to_vec()).unwrap();
        assert!(raw.starts_with("data: "));
        assert!(raw.ends_with("\n\n"));
        let parsed: Value = serde_json::from_str(raw.trim_start_matches("data: ").trim()).unwrap();
        assert_eq!(parsed["type"], "cursor_moved");
        assert_eq!(parsed["kind"], "cursor");
        assert_eq!(parsed["active_line"], 3);

        let edit_ev = IdeCollabEvent::BufferUpdated {
            peer_id: "peer_1".into(),
            path: "src/main.rs".into(),
            content: "fn main() {}\n".into(),
            version: 2,
            saved: false,
        };
        let edit_raw = String::from_utf8(format_collab_event_frame(&edit_ev).to_vec()).unwrap();
        let edit_val: Value =
            serde_json::from_str(edit_raw.trim_start_matches("data: ").trim()).unwrap();
        assert_eq!(edit_val["type"], "buffer_updated");
        assert_eq!(edit_val["kind"], "edit");
        assert_eq!(edit_val["version"], 2);
    }

    #[test]
    fn api_err_maps_domain_errors_to_expected_http_statuses() {
        assert_eq!(
            ApiErr(Error::NotFound("x".into())).into_response().status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ApiErr(Error::Unauthorized).into_response().status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            ApiErr(Error::Forbidden("no".into()))
                .into_response()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            ApiErr(Error::Conflict("sha".into()))
                .into_response()
                .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiErr(Error::Invalid("bad".into()))
                .into_response()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}
