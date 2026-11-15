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
    IdeEnvironmentResp, IdeEnvironmentsResp, IdeInstallJobResp, IdeInstallToolchainReq,
    IdeNetBridgeStatusResp, IdeNetBridgeTestReq, IdeNetworkBridgeConfig, IdeOkResp,
    IdeProjectFolderReq, IdePythonVenvInfo, IdeRegisterLocalSdkReq, IdeRegisterLocalSdkResp,
    IdeShellEnvResp, IdeTargetKind, IdeTargetQuery, IdeTargetSpec, IdeToolchainPythonVenvReq,
    IdeToolchainPythonVenvResp, IdeToolchainsResp, IdeUpdateEnvironmentReq,
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

pub(super) async fn get_environments<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeTargetQuery>,
) -> ApiResult<Json<IdeEnvironmentsResp>> {
    let mut target = q.into_target_spec()?;
    let ssh = ctx
        .ide()
        .targets
        .resolve(&ctx, &user, &target)
        .await?
        .into_ssh_spec();
    ctx.ide()
        .environments
        .enrich_target_roots_authorized(&ctx, &user, &mut target, false)
        .await;
    let resp = ctx
        .ide()
        .environments
        .get_environments_for_user(
            &ctx.pool(),
            ctx.secrets(),
            &ctx.ide().targets,
            &target,
            ssh.as_ref(),
            user.is_root,
        )
        .await?;
    Ok(Json(resp))
}

pub(super) async fn put_environments<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeUpdateEnvironmentReq>,
) -> ApiResult<Json<IdeEnvironmentResp>> {
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    if let Some(ref extra_roots) = req.additional_roots {
        if req.target.kind == IdeTargetKind::Local && !user.is_root {
            for r in extra_roots {
                if !r.trim().is_empty() {
                    ctx.authorize_local_target(&user, &IdeTargetSpec::local(r.clone()), true)
                        .await?;
                }
            }
        }
    }
    ctx.ide()
        .environments
        .enrich_target_roots_authorized(&ctx, &user, &mut req.target, true)
        .await;
    let prev_nb = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), &req.target)
        .await
        .ok()
        .and_then(|p| p.network_bridge);
    let mut should_evict_nb = false;
    if let Some(ref raw_nb) = req.network_bridge {
        let clean_nb_opt = crate::netbridge::sanitize_network_bridge(Some(raw_nb.clone()))?;
        if let Some(ref clean_nb) = clean_nb_opt {
            if clean_nb.enabled {
                let same_prod_targets = prev_nb.as_ref().is_some_and(|p| {
                    p.enabled
                        && p.k8s_cluster_id == clean_nb.k8s_cluster_id
                        && p.k8s_namespace == clean_nb.k8s_namespace
                        && p.ssh_tunnel_connection_id == clean_nb.ssh_tunnel_connection_id
                });
                let effective_confirm = req.confirm_prod_targets || same_prod_targets;
                ctx.ide()
                    .netbridge
                    .authorize_config_with_confirm(&ctx, &user, clean_nb, effective_confirm)
                    .await?;
            } else {
                let prev_k8s = prev_nb.as_ref().and_then(|p| p.k8s_cluster_id.as_ref());
                let prev_ns = prev_nb.as_ref().and_then(|p| p.k8s_namespace.as_deref());
                let prev_ssh = prev_nb
                    .as_ref()
                    .and_then(|p| p.ssh_tunnel_connection_id.as_ref());
                let k8s_changed = clean_nb.k8s_cluster_id.is_some()
                    && (clean_nb.k8s_cluster_id.as_ref() != prev_k8s
                        || clean_nb.k8s_namespace.as_deref() != prev_ns);
                let ssh_changed = clean_nb.ssh_tunnel_connection_id.is_some()
                    && clean_nb.ssh_tunnel_connection_id.as_ref() != prev_ssh;
                if k8s_changed || ssh_changed {
                    let check_nb = IdeNetworkBridgeConfig {
                        k8s_cluster_id: if k8s_changed {
                            clean_nb.k8s_cluster_id.clone()
                        } else {
                            None
                        },
                        k8s_namespace: clean_nb.k8s_namespace.clone(),
                        ssh_tunnel_connection_id: if ssh_changed {
                            clean_nb.ssh_tunnel_connection_id.clone()
                        } else {
                            None
                        },
                        ..Default::default()
                    };
                    ctx.ide()
                        .netbridge
                        .authorize_config(&ctx, &user, &check_nb)
                        .await?;
                }
            }
        }
        should_evict_nb = clean_nb_opt != prev_nb
            || (prev_nb.is_some() && clean_nb_opt.as_ref().is_none_or(|nb| !nb.enabled));
    }
    let req_target = req.target.clone();
    let go_tags_updated = req.go_build_tags.is_some() || req.module_go_build_tags.is_some();
    let env = ctx
        .ide()
        .environments
        .update_environment_for_user(&ctx.pool(), ctx.secrets(), req, user.is_root)
        .await?;
    if go_tags_updated && req_target.kind == IdeTargetKind::Local {
        ctx.ide()
            .lsp_pool
            .sync_project_go_build_tags(&req_target.root, &env);
    }
    if should_evict_nb {
        ctx.ide().netbridge.evict_user(&user.id);
    }
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeEnvironmentResp { env }))
}

pub(super) async fn test_network_bridge<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeNetBridgeTestReq>,
) -> ApiResult<Json<IdeNetBridgeStatusResp>> {
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    let resp = ctx.ide().netbridge.probe_status(&ctx, &user, &req).await?;
    Ok(Json(resp))
}
