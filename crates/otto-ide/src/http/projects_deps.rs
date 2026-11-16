//! HTTP handlers for Named Multi-Repo Projects (`/ide/projects/*`),
//! Dependency & Vulnerability Inspector (`/ide/deps/*`), and Maven/Gradle/Spring
//! Build Tools Model (`POST /ide/build-tools/model`).

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Path, State};
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Result};

use super::ApiResult;
use crate::target::enforce_non_root_path;
use crate::types::{
    IdeBuildToolsModelReq, IdeBuildToolsModelResp, IdeBulkFoldersReq, IdeBulkFoldersResp,
    IdeDepsActionReq, IdeDepsActionResp, IdeDepsInspectReq, IdeDepsInspectResp, IdeOkResp,
    IdeProjectsResp, IdeScanReposReq, IdeScanReposResp, IdeTargetKind, IdeTargetSpec,
    IdeUpsertProjectReq, IdeUpsertProjectResp,
};
use crate::IdeCtx;

// ---------------------------------------------------------------------------
// Named Multi-Repo IDE Projects (`/ide/projects/*`)
// ---------------------------------------------------------------------------

pub(super) async fn list_projects<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeProjectsResp>> {
    let projects = ctx
        .ide()
        .projects
        .list_for_user_with_ctx(&ctx, &user)
        .await?;
    Ok(Json(IdeProjectsResp { projects }))
}

pub(super) async fn upsert_project<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeUpsertProjectReq>,
) -> ApiResult<Json<IdeUpsertProjectResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    if !has_root_auth {
        let effective_primary = if !req.primary_root.trim().is_empty() {
            req.primary_root.clone()
        } else {
            req.folders
                .iter()
                .find(|f| !f.trim().is_empty())
                .cloned()
                .unwrap_or_default()
        };
        ctx.authorize_local_target(&user, &IdeTargetSpec::local(effective_primary), true)
            .await?;
        for folder in &req.folders {
            if !folder.trim().is_empty() {
                ctx.authorize_local_target(&user, &IdeTargetSpec::local(folder.clone()), true)
                    .await?;
            }
        }
    }
    let project = ctx
        .ide()
        .projects
        .upsert_for_user_with_ctx(&ctx, &user, req)
        .await?;

    let target = IdeTargetSpec::local(project.primary_root.clone());
    if project.folders.len() > 1
        && ctx
            .ide()
            .targets
            .resolve_write(&ctx, &user, &target)
            .await
            .is_ok()
    {
        if let Err(err) = ctx
            .ide()
            .environments
            .add_project_folders_bulk(&ctx.pool(), &target, &project.folders)
            .await
        {
            tracing::warn!(
                project_id = %project.id,
                error = %err,
                "failed to sync named project folders into ide_project_environments"
            );
            return Err(err.into());
        }
    }
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeUpsertProjectResp { project }))
}

pub(super) async fn delete_project<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide()
        .projects
        .delete_for_user_with_ctx(&ctx, &user, &id)
        .await?;
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeOkResp { ok: true }))
}

pub(super) async fn scan_repos<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeScanReposReq>,
) -> ApiResult<Json<IdeScanReposResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    if !has_root_auth {
        ctx.authorize_local_target(&user, &IdeTargetSpec::local(req.parent_path.clone()), false)
            .await?;
    }
    let resp = tokio::task::spawn_blocking(move || {
        crate::projects::scan_parent_repos(&req.parent_path, has_root_auth)
    })
    .await
    .map_err(|e| Error::Internal(format!("scan repos worker failed: {e}")))??;
    Ok(Json(resp))
}

pub(super) async fn add_project_folders_bulk<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeBulkFoldersReq>,
) -> ApiResult<Json<IdeBulkFoldersResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    let _ = ctx
        .ide()
        .targets
        .resolve_write(&ctx, &user, &req.target)
        .await?;
    if req.target.kind == IdeTargetKind::Local && !has_root_auth {
        for fp in &req.folder_paths {
            if !fp.trim().is_empty() {
                ctx.authorize_local_target(&user, &IdeTargetSpec::local(fp.clone()), true)
                    .await?;
            }
        }
    }
    let (env, _) = ctx
        .ide()
        .environments
        .add_project_folders_bulk(&ctx.pool(), &req.target, &req.folder_paths)
        .await?;
    ctx.ide().targets.invalidate_local_resolve_cache();
    Ok(Json(IdeBulkFoldersResp {
        additional_roots: env.additional_roots,
    }))
}

// ---------------------------------------------------------------------------
// Dependency & Vulnerability Inspector (`/ide/deps/*`)
// ---------------------------------------------------------------------------

async fn resolve_manifest_target<S: IdeCtx>(
    ctx: &S,
    user: &User,
    target_opt: Option<IdeTargetSpec>,
    raw_path: &str,
    write: bool,
) -> Result<(IdeTargetSpec, PathBuf, PathBuf, Vec<PathBuf>)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("manifest 'path' is required".into()));
    }
    let mut target = match target_opt {
        Some(t) if !t.root.trim().is_empty() => t,
        _ => {
            let p = FsPath::new(trimmed);
            let dir = p
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or_else(|| FsPath::new("."));
            IdeTargetSpec::local(dir.to_string_lossy().into_owned())
        }
    };
    let resolved = if write {
        ctx.ide().targets.resolve_write(ctx, user, &target).await?
    } else {
        ctx.ide().targets.resolve(ctx, user, &target).await?
    };
    ctx.ide()
        .environments
        .enrich_target_roots_authorized(ctx, user, &mut target, write)
        .await;
    let crate::target::ResolvedTarget::Local {
        root: canon_root,
        additional_roots,
        ..
    } = resolved
    else {
        return Err(Error::Invalid(
            "dependency inspector requires a local target".into(),
        ));
    };

    let cand = if trimmed.starts_with('~') {
        otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?
    } else if FsPath::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        canon_root.join(trimmed)
    };
    let trimmed_owned = trimmed.to_string();
    let canon_manifest = tokio::task::spawn_blocking(move || {
        #[allow(clippy::disallowed_methods)]
        let resolved_p = std::fs::canonicalize(&cand)
            .ok()
            .or_else(|| otto_core::secret_paths::canonicalize_prospective(&cand));
        resolved_p.ok_or_else(|| Error::NotFound(format!("manifest '{trimmed_owned}' not found")))
    })
    .await
    .map_err(|e| Error::Internal(format!("manifest canonicalize worker failed: {e}")))??;

    let mut allowed_roots = vec![canon_root.clone()];
    allowed_roots.extend(additional_roots);
    let in_roots = allowed_roots.iter().any(|r| canon_manifest.starts_with(r));
    if !in_roots {
        return Err(Error::Forbidden(format!(
            "manifest '{}' escapes workspace root '{}'",
            canon_manifest.display(),
            canon_root.display()
        )));
    }
    if !otto_core::auth::root_authority(user) {
        enforce_non_root_path(&canon_manifest)?;
    }
    Ok((target, canon_root, canon_manifest, allowed_roots))
}

pub(super) async fn deps_inspect<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeDepsInspectReq>,
) -> ApiResult<Json<IdeDepsInspectResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    let (target, _, manifest_path, allowed_roots) =
        resolve_manifest_target(&ctx, &user, req.target, &req.path, false).await?;
    ctx.ide().environments.ensure_hydrated(&ctx.pool()).await;
    let env_map = ctx
        .ide()
        .environments
        .cached_probe_env_for_local_root_as(&target.root, has_root_auth);
    let resp = crate::deps_inspector::inspect_dependencies_scoped(
        &manifest_path,
        req.content.as_deref(),
        &env_map,
        &allowed_roots,
        has_root_auth,
    )
    .await;
    Ok(Json(resp))
}

pub(super) async fn deps_action<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeDepsActionReq>,
) -> ApiResult<Json<IdeDepsActionResp>> {
    super::require_root_user(&user)?;
    let has_root_auth = otto_core::auth::root_authority(&user);
    let (target, canon_root, manifest_path, allowed_roots) =
        resolve_manifest_target(&ctx, &user, req.target.clone(), &req.path, true).await?;
    let env_map = ctx
        .ide()
        .environments
        .build_effective_env_for_run_as(&ctx.pool(), ctx.secrets(), &target, None, has_root_auth)
        .await
        .map_err(|err| {
            tracing::warn!(
                target = %target.project_key(),
                error = %err,
                "failed to resolve effective environment for dependency action"
            );
            err
        })?;
    let resp = crate::deps_inspector::run_deps_action_scoped(
        &canon_root,
        &allowed_roots,
        &manifest_path,
        &req,
        &env_map,
        &ctx.ide().history,
        has_root_auth,
    )
    .await?;
    if resp.ok {
        let lsp_pool = &ctx.ide().lsp_pool;
        lsp_pool.notify_watched_file_if_build_manifest(&manifest_path, 2);
        if let Some(dir) = manifest_path.parent() {
            for sibling in [
                "go.sum",
                "Cargo.lock",
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
                "uv.lock",
                "poetry.lock",
            ] {
                let lock_path = dir.join(sibling);
                if lock_path != manifest_path && lock_path.is_file() {
                    lsp_pool.notify_watched_file_if_build_manifest(&lock_path, 2);
                }
            }
        }
    }
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Auto-Detected Maven / Gradle / Spring Boot Model (`POST /ide/build-tools/model`)
// ---------------------------------------------------------------------------

pub(super) async fn build_tools_model<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeBuildToolsModelReq>,
) -> ApiResult<Json<IdeBuildToolsModelResp>> {
    let mut authorized_any = false;
    if let Some(target) = &mut req.target {
        if !target.root.trim().is_empty() {
            let _ = ctx.ide().targets.resolve(&ctx, &user, target).await?;
            ctx.ide()
                .environments
                .enrich_target_roots_authorized(&ctx, &user, target, false)
                .await;
            authorized_any = true;
        }
    }
    let has_root_auth = otto_core::auth::root_authority(&user);
    if !has_root_auth {
        for r in &req.roots {
            if !r.trim().is_empty() {
                ctx.authorize_local_target(&user, &IdeTargetSpec::local(r.clone()), false)
                    .await?;
                authorized_any = true;
            }
        }
        if !authorized_any {
            return Err(Error::Forbidden(
                "workspace root is required for build tools model".into(),
            )
            .into());
        }
    }
    let resp = tokio::task::spawn_blocking(move || {
        crate::build_tools::build_tools_model(&req, has_root_auth)
    })
    .await
    .map_err(|e| Error::Internal(format!("build tools model worker failed: {e}")))?;
    Ok(Json(resp))
}
