//! Axum REST & SSE routers for `otto-ide` (`/ide/*` and `/ide/collab/guest/*`).
//!
//! - [`api_router`]: protected `/ide/*` routes nested under `/api/v1` (enforced by
//!   `auth_middleware` + `otto-server/src/policy.rs`).
//! - [`public_router`]: capability-token `/ide/collab/guest/*` routes nested under
//!   `/api/v1` for "Share with Friends" live collaborative editing and co-debugging.

#[path = "http/collab_sse.rs"]
mod collab_sse;
#[path = "http/intelligence.rs"]
mod intelligence;
#[path = "http/projects_deps.rs"]
mod projects_deps;
#[path = "http/runtime_debug.rs"]
mod runtime_debug;

use std::path::Path as FsPath;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::{Error, Result};
use serde::Deserialize;

use self::collab_sse::{
    close_collab_room, collab_guest_cursor, collab_guest_debug_action, collab_guest_edit,
    collab_guest_events, collab_guest_join, collab_guest_state, create_collab_room, install_plugin,
    list_collab_rooms, list_plugins, set_collab_role, toggle_plugin, uninstall_plugin,
};
use self::intelligence::{
    fs_complete, history_entry, history_list, history_restore, history_snapshot, lsp_code_actions,
    lsp_definition, lsp_diagnostics, lsp_format, lsp_hover, lsp_implementation, lsp_references,
    lsp_rename, lsp_symbols,
};
use self::projects_deps::{
    add_project_folders_bulk, build_tools_model, delete_project, deps_action, deps_inspect,
    list_projects, scan_repos, upsert_project,
};
use self::runtime_debug::{
    create_python_venv, debug_command, debug_evaluate, debug_variables, delete_run_config,
    get_debug_session, get_environments, get_run, list_debug_sessions, put_environments,
    set_debug_breakpoints, start_debug_session, start_run, stop_run, stream_debug_events,
    upsert_run_config,
};
use crate::types::{
    IdeCreatePythonVenvReq, IdeDeleteLocalSdkQuery, IdeDeleteProjectFolderQuery,
    IdeEnvironmentResp, IdeFileQuery, IdeFileReadResp, IdeFileWriteReq, IdeFileWriteResp,
    IdeFsEntryKind, IdeFsMutateReq, IdeFsMutateResp, IdeFsTreeQuery, IdeFsTreeResp, IdeGutterReq,
    IdeGutterResp, IdeInstallJobResp, IdeInstallToolchainReq, IdeOkResp, IdeProjectFolderReq,
    IdePythonVenvInfo, IdeRegisterLocalSdkReq, IdeRegisterLocalSdkResp, IdeReplaceReq,
    IdeReplaceResp, IdeSearchReq, IdeSearchResp, IdeShellEnvResp, IdeTargetKind, IdeTargetQuery,
    IdeTargetSpec, IdeToolchainPythonVenvReq, IdeToolchainPythonVenvResp, IdeToolchainsResp,
};
use crate::IdeCtx;

pub(super) fn require_root_user(user: &otto_core::domain::User) -> Result<()> {
    if otto_core::auth::root_authority(user) {
        Ok(())
    } else {
        Err(Error::Forbidden("root authority required".into()))
    }
}

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
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("ide internal error: {}", self.0);
        }
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

pub(crate) type ApiResult<T> = std::result::Result<T, ApiErr>;

/// Protected `/ide/*` REST & SSE routes (nested under `/api/v1`).
pub fn api_router<S: IdeCtx>() -> Router<S> {
    Router::new()
        // A. Target Filesystem, Search, Gutter, Completion & Extended LSP
        .route("/ide/fs/tree", get(fs_tree::<S>))
        .route(
            "/ide/fs/file",
            get(fs_read_file::<S>).put(fs_write_file::<S>),
        )
        .route("/ide/fs/mutate", post(fs_mutate::<S>))
        .route("/ide/fs/complete", post(fs_complete::<S>))
        .route("/ide/search", post(search_workspace::<S>))
        .route("/ide/replace", post(replace_workspace::<S>))
        .route("/ide/gutter", post(detect_gutter::<S>))
        .route("/ide/lsp/symbols", post(lsp_symbols::<S>))
        .route("/ide/lsp/definition", post(lsp_definition::<S>))
        .route("/ide/lsp/implementation", post(lsp_implementation::<S>))
        .route("/ide/lsp/references", post(lsp_references::<S>))
        .route("/ide/lsp/rename", post(lsp_rename::<S>))
        .route("/ide/lsp/format", post(lsp_format::<S>))
        .route("/ide/lsp/code-actions", post(lsp_code_actions::<S>))
        .route("/ide/lsp/diagnostics", post(lsp_diagnostics::<S>))
        .route("/ide/lsp/hover", post(lsp_hover::<S>))
        // A2. Backend Local History & Deleted-File Recovery
        .route("/ide/history/list", get(history_list::<S>))
        .route("/ide/history/entry", get(history_entry::<S>))
        .route("/ide/history/restore", post(history_restore::<S>))
        .route("/ide/history/snapshot", post(history_snapshot::<S>))
        // B. OS/Arch Toolchains, Detected SDKs, Custom Local SDKs & SDK Downloader
        .route("/ide/toolchains", get(list_toolchains::<S>))
        .route("/ide/toolchains/install", post(install_toolchain::<S>))
        .route(
            "/ide/toolchains/local",
            post(register_local_sdk::<S>).delete(delete_local_sdk::<S>),
        )
        .route("/ide/toolchains/jobs/{job_id}", get(get_toolchain_job::<S>))
        .route(
            "/ide/toolchains/python-venv",
            post(toolchain_python_venv::<S>),
        )
        // C. Shell Env, Named Projects, Dependencies, Build Tools & Run Configs
        .route("/ide/shell-env", get(get_shell_env::<S>))
        .route("/ide/shell-env/reload", post(reload_shell_env::<S>))
        .route(
            "/ide/projects",
            get(list_projects::<S>).post(upsert_project::<S>),
        )
        .route("/ide/projects/{id}", delete(delete_project::<S>))
        .route("/ide/projects/scan-repos", post(scan_repos::<S>))
        .route(
            "/ide/projects/folders",
            post(add_project_folder::<S>).delete(remove_project_folder::<S>),
        )
        .route(
            "/ide/projects/folders/bulk",
            post(add_project_folders_bulk::<S>),
        )
        .route("/ide/deps/inspect", post(deps_inspect::<S>))
        .route("/ide/deps/action", post(deps_action::<S>))
        .route("/ide/build-tools/model", post(build_tools_model::<S>))
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

// ===========================================================================
// A. Target Filesystem, Search & Gutter Handlers
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
    let resolved = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
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
    let resolved = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
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
    let resolved = if req.dry_run {
        ctx.ide().targets.resolve(&ctx, &user, &req.target).await?
    } else {
        ctx.ide()
            .targets
            .resolve_write(&ctx, &user, &req.target)
            .await?
    };
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

// ===========================================================================
// B. OS/Arch Toolchains, Custom Local SDKs & SDK Downloader Handlers
// ===========================================================================

async fn list_toolchains<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeToolchainsResp>> {
    let include_catalog = q.include_catalog;
    let target = q.into_target_spec()?;
    let ssh = ctx
        .ide()
        .targets
        .resolve(&ctx, &user, &target)
        .await?
        .into_ssh_spec();
    let active_env = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), &target)
        .await
        .ok();
    let resp = ctx
        .ide()
        .toolchains
        .list_toolchains_with_opts(
            Some(&ctx.pool()),
            &ctx.ide().targets,
            &target,
            ssh.as_ref(),
            active_env.as_ref(),
            include_catalog,
        )
        .await?;
    Ok(Json(resp))
}

async fn install_toolchain<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeInstallToolchainReq>,
) -> ApiResult<Json<IdeInstallJobResp>> {
    require_root_user(&user)?;
    let ssh = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?
        .into_ssh_spec();
    let job = ctx
        .ide()
        .toolchains
        .start_install(&ctx.ide().targets, req, ssh)
        .await?;
    Ok(Json(IdeInstallJobResp { job }))
}

async fn register_local_sdk<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeRegisterLocalSdkReq>,
) -> ApiResult<Json<IdeRegisterLocalSdkResp>> {
    require_root_user(&user)?;
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    let sdk = ctx
        .ide()
        .toolchains
        .register_local_sdk(&ctx.pool(), req)
        .await?;
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeRegisterLocalSdkResp { sdk }))
}

async fn delete_local_sdk<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeDeleteLocalSdkQuery>,
) -> ApiResult<Json<IdeOkResp>> {
    require_root_user(&user)?;
    let target = if q.root.as_deref().is_some_and(|r| !r.trim().is_empty()) {
        let spec = IdeTargetSpec {
            kind: q.kind.unwrap_or_default(),
            root: q.root.clone().unwrap_or_default(),
            workspace_id: None,
            connection_id: q.connection_id.clone(),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        let _ = ctx.ide().targets.resolve_write(&ctx, &user, &spec).await?;
        spec
    } else {
        IdeTargetSpec {
            kind: q.kind.unwrap_or_default(),
            root: "/".into(),
            workspace_id: None,
            connection_id: q.connection_id.clone(),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        }
    };
    ctx.ide()
        .toolchains
        .remove_local_sdk(&ctx.pool(), &target, q.id.as_deref(), q.path.as_deref())
        .await?;
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeOkResp { ok: true }))
}

async fn get_toolchain_job<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(job_id): Path<String>,
) -> ApiResult<Json<IdeInstallJobResp>> {
    require_root_user(&user)?;
    let job = ctx.ide().toolchains.get_job(&job_id)?;
    Ok(Json(IdeInstallJobResp { job }))
}

async fn toolchain_python_venv<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeToolchainPythonVenvReq>,
) -> ApiResult<Json<IdeToolchainPythonVenvResp>> {
    let mut target = match req.target {
        Some(t) if !t.root.trim().is_empty() => t,
        _ => IdeTargetSpec::local(req.root.clone().unwrap_or_default()),
    };
    let mut venv_dir = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(".venv")
        .to_string();
    if FsPath::new(&venv_dir).is_absolute() {
        let vp = FsPath::new(&venv_dir);
        let base_for_strip = req
            .root
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| target.root.trim());
        if !base_for_strip.is_empty() {
            if let Ok(rel) = vp.strip_prefix(base_for_strip) {
                let rel_s = rel.to_string_lossy().into_owned();
                if !rel_s.is_empty() {
                    venv_dir = rel_s;
                }
            }
        } else if let (Some(parent), Some(fname)) =
            (vp.parent(), vp.file_name().and_then(|n| n.to_str()))
        {
            target.root = parent.to_string_lossy().into_owned();
            venv_dir = fname.to_string();
        }
    }

    let ssh = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &target)
        .await?
        .into_ssh_spec();
    if let Some(explicit_root) = req
        .root
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != target.root)
    {
        let mut sub_target = target.clone();
        sub_target.root = explicit_root.to_string();
        let _ = ctx
            .ide()
            .targets
            .resolve_write(&ctx, &user, &sub_target)
            .await?;
    }
    let created = ctx
        .ide()
        .environments
        .create_python_venv(
            &ctx.pool(),
            &ctx.ide().targets,
            IdeCreatePythonVenvReq {
                target,
                root: req.root,
                venv_dir: Some(venv_dir),
                python_version: req.base_python,
            },
            ssh.as_ref(),
        )
        .await?;

    Ok(Json(IdeToolchainPythonVenvResp {
        ok: true,
        venv: IdePythonVenvInfo {
            path: created.path,
            version: created.version,
            kind: "venv".into(),
        },
        interpreter_path: created.interpreter_path,
    }))
}

// ===========================================================================
// C. Shell Env, Environments & Run Config Handlers
// ===========================================================================

#[derive(Debug, Default, Deserialize)]
struct IdeShellEnvQuery {
    #[serde(default)]
    kind: Option<IdeTargetKind>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct IdeReloadShellEnvReq {
    #[serde(default)]
    target: Option<IdeTargetSpec>,
    #[serde(default)]
    kind: Option<IdeTargetKind>,
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
}

fn ssh_shell_env_spec_from_parts(
    kind: Option<IdeTargetKind>,
    root: Option<String>,
    connection_id: Option<String>,
) -> Option<IdeTargetSpec> {
    if kind == Some(IdeTargetKind::Ssh)
        || connection_id
            .as_deref()
            .is_some_and(|c| !c.trim().is_empty())
    {
        let root_val = root
            .filter(|r| !r.trim().is_empty())
            .unwrap_or_else(|| "~".to_string());
        Some(IdeTargetSpec {
            kind: IdeTargetKind::Ssh,
            root: root_val,
            workspace_id: None,
            connection_id,
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        })
    } else {
        None
    }
}

async fn get_shell_env<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeShellEnvQuery>,
) -> ApiResult<Json<IdeShellEnvResp>> {
    require_root_user(&user)?;
    if let Some(spec) = ssh_shell_env_spec_from_parts(q.kind, q.root, q.connection_id) {
        let ssh = ctx
            .ide()
            .targets
            .resolve(&ctx, &user, &spec)
            .await?
            .into_ssh_spec()
            .ok_or_else(|| Error::Invalid("expected SSH target".into()))?;
        let resp =
            crate::environments::probe_remote_shell_env(&ctx.ide().targets, &spec, &ssh).await?;
        return Ok(Json(resp));
    }
    Ok(Json(ctx.ide().environments.get_shell_env().await))
}

async fn reload_shell_env<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeShellEnvQuery>,
    body: Bytes,
) -> ApiResult<Json<IdeShellEnvResp>> {
    require_root_user(&user)?;
    let parsed_body = if body.is_empty() {
        IdeReloadShellEnvReq::default()
    } else {
        serde_json::from_slice::<IdeReloadShellEnvReq>(&body)
            .map_err(|e| Error::Invalid(format!("invalid reload request body: {e}")))?
    };
    let ssh_spec_opt = parsed_body
        .target
        .filter(|t| t.kind == IdeTargetKind::Ssh)
        .or_else(|| {
            ssh_shell_env_spec_from_parts(
                parsed_body.kind.or(q.kind),
                parsed_body.root.or(q.root),
                parsed_body.connection_id.or(q.connection_id),
            )
        });
    if let Some(mut spec) = ssh_spec_opt {
        if spec.root.trim().is_empty() {
            spec.root = "~".to_string();
        }
        let ssh = ctx
            .ide()
            .targets
            .resolve(&ctx, &user, &spec)
            .await?
            .into_ssh_spec()
            .ok_or_else(|| Error::Invalid("expected SSH target".into()))?;
        let resp =
            crate::environments::probe_remote_shell_env(&ctx.ide().targets, &spec, &ssh).await?;
        return Ok(Json(resp));
    }
    Ok(Json(ctx.ide().environments.reload_shell_env().await))
}

async fn add_project_folder<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeProjectFolderReq>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    if req.target.kind == IdeTargetKind::Local && !user.is_root {
        ctx.authorize_local_target(&user, &IdeTargetSpec::local(req.folder_path.clone()), false)
            .await?;
    }
    let env = ctx
        .ide()
        .environments
        .add_project_folder(&ctx.pool(), &req.target, &req.folder_path)
        .await?;
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeEnvironmentResp { env }))
}

async fn remove_project_folder<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeDeleteProjectFolderQuery>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let target = q.target_spec()?;
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &target)
        .await?;
    let env = ctx
        .ide()
        .environments
        .remove_project_folder(&ctx.pool(), &target, &q.folder_path)
        .await?;
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeEnvironmentResp { env }))
}

#[cfg(test)]
mod tests {
    use super::collab_sse::{extract_collab_token, format_collab_event_frame};
    use super::*;
    use crate::types::{IdeCollabEvent, IdeSelectionRange};
    use axum::http::{header, HeaderMap, HeaderValue};
    use serde_json::Value;

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
