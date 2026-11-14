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

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Result};

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
    IdeReplaceResp, IdeSearchReq, IdeSearchResp, IdeShellEnvResp, IdeSshSpec, IdeTargetKind,
    IdeTargetQuery, IdeTargetSpec, IdeToolchainPythonVenvReq, IdeToolchainPythonVenvResp,
    IdeToolchainsResp,
};
use crate::IdeCtx;

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

pub(crate) async fn resolve_ssh_opt<S: IdeCtx>(
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
// A. Target Filesystem, Search & Gutter Handlers
// ===========================================================================

async fn fs_tree<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeFsTreeQuery>,
) -> ApiResult<Json<IdeFsTreeResp>> {
    let mut spec = q.target_spec()?;
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut spec)
        .await;
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
    let mut spec = q.target_spec()?;
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut spec)
        .await;
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
    Json(mut req): Json<IdeFileWriteReq>,
) -> ApiResult<Json<IdeFileWriteResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
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
    Json(mut req): Json<IdeFsMutateReq>,
) -> ApiResult<Json<IdeFsMutateResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
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
    Json(mut req): Json<IdeSearchReq>,
) -> ApiResult<Json<IdeSearchResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx.ide().targets.search(&resolved, &req).await?;
    Ok(Json(resp))
}

async fn replace_workspace<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReplaceReq>,
) -> ApiResult<Json<IdeReplaceResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let resp = ctx.ide().targets.replace(&resolved, &req).await?;
    Ok(Json(resp))
}

async fn detect_gutter<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeGutterReq>,
) -> ApiResult<Json<IdeGutterResp>> {
    req.target.validate()?;
    let content = match req.content {
        Some(c) => c,
        None => {
            ctx.ide()
                .environments
                .enrich_target_roots(&ctx.pool(), &mut req.target)
                .await;
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
    let mut target = q.into_target_spec()?;
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut target)
        .await;
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
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &req.target).await?;
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
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let sdk = ctx
        .ide()
        .toolchains
        .register_local_sdk(&ctx.pool(), req)
        .await?;
    Ok(Json(IdeRegisterLocalSdkResp { sdk }))
}

async fn delete_local_sdk<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeDeleteLocalSdkQuery>,
) -> ApiResult<Json<IdeOkResp>> {
    let target = if q.root.as_deref().is_some_and(|r| !r.trim().is_empty()) {
        let spec = IdeTargetSpec {
            kind: q.kind.unwrap_or_default(),
            workspace_id: q.workspace_id.clone(),
            root: q.root.clone().unwrap_or_default(),
            connection_id: q.connection_id.clone(),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        let _ = ctx.ide().targets.resolve(&ctx, &user, &spec).await?;
        spec
    } else {
        IdeTargetSpec {
            kind: q.kind.unwrap_or_default(),
            workspace_id: q.workspace_id.clone(),
            root: "/".into(),
            connection_id: q.connection_id.clone(),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        }
    };
    ctx.ide()
        .toolchains
        .remove_local_sdk(&ctx.pool(), &target, q.id.as_deref(), q.path.as_deref())
        .await?;
    Ok(Json(IdeOkResp { ok: true }))
}

async fn get_toolchain_job<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
    Path(job_id): Path<String>,
) -> ApiResult<Json<IdeInstallJobResp>> {
    let job = ctx.ide().toolchains.get_job(&job_id)?;
    Ok(Json(IdeInstallJobResp { job }))
}

async fn toolchain_python_venv<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeToolchainPythonVenvReq>,
) -> ApiResult<Json<IdeToolchainPythonVenvResp>> {
    let mut target = req
        .target
        .unwrap_or_else(|| IdeTargetSpec::local(req.root.clone().unwrap_or_default()));
    if let Some(explicit_root) = req.root.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        target.root = explicit_root.to_string();
    }
    let mut venv_dir = req
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(".venv")
        .to_string();
    if FsPath::new(&venv_dir).is_absolute() {
        let vp = FsPath::new(&venv_dir);
        if !target.root.trim().is_empty() {
            if let Ok(rel) = vp.strip_prefix(&target.root) {
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

    let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    let ssh = resolve_ssh_opt(&ctx, &user, &target).await?;
    let created = ctx
        .ide()
        .environments
        .create_python_venv(
            &ctx.pool(),
            &ctx.ide().targets,
            IdeCreatePythonVenvReq {
                target,
                root: None,
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

async fn get_shell_env<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeShellEnvResp>> {
    Ok(Json(ctx.ide().environments.get_shell_env().await))
}

async fn reload_shell_env<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(_user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeShellEnvResp>> {
    Ok(Json(ctx.ide().environments.reload_shell_env().await))
}

async fn add_project_folder<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeProjectFolderReq>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let _ = ctx.ide().targets.resolve(&ctx, &user, &req.target).await?;
    let env = ctx
        .ide()
        .environments
        .add_project_folder(&ctx.pool(), &req.target, &req.folder_path)
        .await?;
    Ok(Json(IdeEnvironmentResp { env }))
}

async fn remove_project_folder<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeDeleteProjectFolderQuery>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let target = q.target_spec()?;
    let _ = ctx.ide().targets.resolve(&ctx, &user, &target).await?;
    let env = ctx
        .ide()
        .environments
        .remove_project_folder(&ctx.pool(), &target, &q.folder_path)
        .await?;
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
