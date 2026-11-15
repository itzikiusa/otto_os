//! HTTP handlers for OS/Arch Toolchains (`/ide/toolchains/*`), Cached Shell
//! Environment (`/ide/shell-env*`), and Project Folder Attachment (`/ide/projects/folders`).

use std::path::Path as FsPath;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::Error;
use serde::Deserialize;

use super::{require_root_user, ApiResult};
use crate::types::{
    IdeCreatePythonVenvReq, IdeDeleteLocalSdkQuery, IdeDeleteProjectFolderQuery,
    IdeEnvironmentResp, IdeInstallJobResp, IdeInstallToolchainReq, IdeOkResp, IdeProjectFolderReq,
    IdePythonVenvInfo, IdeRegisterLocalSdkReq, IdeRegisterLocalSdkResp, IdeShellEnvResp,
    IdeTargetKind, IdeTargetQuery, IdeTargetSpec, IdeToolchainPythonVenvReq,
    IdeToolchainPythonVenvResp, IdeToolchainsResp,
};
use crate::IdeCtx;

pub(super) async fn list_toolchains<S: IdeCtx>(
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

pub(super) async fn install_toolchain<S: IdeCtx>(
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

pub(super) async fn register_local_sdk<S: IdeCtx>(
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

pub(super) async fn delete_local_sdk<S: IdeCtx>(
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

pub(super) async fn get_toolchain_job<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(job_id): Path<String>,
) -> ApiResult<Json<IdeInstallJobResp>> {
    require_root_user(&user)?;
    let job = ctx.ide().toolchains.get_job(&job_id)?;
    Ok(Json(IdeInstallJobResp { job }))
}

pub(super) async fn toolchain_python_venv<S: IdeCtx>(
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

    let created = super::runtime_debug::execute_create_python_venv(
        &ctx,
        &user,
        IdeCreatePythonVenvReq {
            target,
            root: req.root,
            venv_dir: Some(venv_dir),
            python_version: req.base_python,
        },
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

#[derive(Debug, Default, Deserialize)]
pub(super) struct IdeShellEnvQuery {
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

pub(super) async fn get_shell_env<S: IdeCtx>(
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

pub(super) async fn reload_shell_env<S: IdeCtx>(
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

pub(super) async fn add_project_folder<S: IdeCtx>(
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
        ctx.authorize_local_target(&user, &IdeTargetSpec::local(req.folder_path.clone()), true)
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

pub(super) async fn remove_project_folder<S: IdeCtx>(
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
