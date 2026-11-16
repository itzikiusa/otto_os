//! HTTP handlers for LSP Intelligence (`/ide/lsp/*`), Local History (`/ide/history/*`),
//! and `<15ms` `@` File Completion (`POST /ide/fs/complete`).

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Query, State};
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Result};

use super::ApiResult;
use crate::target::enforce_non_root_path;
use crate::types::{
    IdeCodeActionsReq, IdeCodeActionsResp, IdeDiagnosticsReq, IdeDiagnosticsResp, IdeFormatReq,
    IdeFormatResp, IdeFsCompleteReq, IdeFsCompleteResp, IdeHistoryEntryQuery, IdeHistoryEntryResp,
    IdeHistoryListQuery, IdeHistoryListResp, IdeHistoryRestoreReq, IdeHistoryRestoreResp,
    IdeHistorySnapshotReq, IdeHistorySnapshotResp, IdeHoverReq, IdeHoverResp, IdeLspStatusResp,
    IdeReferencesReq, IdeReferencesResp, IdeRenameReq, IdeRenameResp, IdeSymbolsReq,
    IdeSymbolsResp, IdeTargetSpec,
};
use crate::IdeCtx;

// ---------------------------------------------------------------------------
// Extended LSP & Fast Code Intelligence (`/ide/lsp/*`)
// ---------------------------------------------------------------------------

pub(super) async fn lsp_status<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
) -> ApiResult<Json<IdeLspStatusResp>> {
    let engine = ctx.ide().clone();
    let resp = tokio::task::spawn_blocking(move || {
        engine
            .lsp_pool
            .status_snapshot(Some(&user), Some(&engine.data_dir), Some(&engine.plugins))
    })
    .await
    .map_err(|e| Error::Internal(format!("lsp status worker failed: {e}")))?;
    Ok(Json(resp))
}

pub(super) async fn lsp_symbols<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeSymbolsReq>,
) -> ApiResult<Json<IdeSymbolsResp>> {
    if !otto_core::auth::root_authority(&user)
        && req.target.kind == crate::types::IdeTargetKind::Local
    {
        ctx.authorize_local_target(&user, &req.target, false)
            .await?;
    }
    Ok(Json(
        otto_telemetry::context::measure_result(
            "ide.lsp.request",
            crate::lsp_service::document_symbols(&ctx, &user, req),
        )
        .await?,
    ))
}

pub(super) async fn lsp_definition<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    Ok(Json(
        otto_telemetry::context::measure_result(
            "ide.lsp.request",
            crate::lsp_service::goto_definition(&ctx, &user, req),
        )
        .await?,
    ))
}

pub(super) async fn lsp_implementation<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    Ok(Json(
        otto_telemetry::context::measure_result(
            "ide.lsp.request",
            crate::lsp_service::goto_implementation(&ctx, &user, req),
        )
        .await?,
    ))
}

pub(super) async fn lsp_references<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    Ok(Json(
        otto_telemetry::context::measure_result(
            "ide.lsp.request",
            crate::lsp_service::find_references(&ctx, &user, req),
        )
        .await?,
    ))
}

pub(super) async fn lsp_rename<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeRenameReq>,
) -> ApiResult<Json<IdeRenameResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    if req.apply {
        let _ = ctx
            .ide()
            .targets
            .resolve_write(&ctx, &user, &req.target)
            .await?;
    }
    Ok(Json(
        crate::lsp_service::rename_symbol(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_format<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFormatReq>,
) -> ApiResult<Json<IdeFormatResp>> {
    Ok(Json(
        crate::lsp_service::format_document(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_code_actions<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeCodeActionsReq>,
) -> ApiResult<Json<IdeCodeActionsResp>> {
    if !otto_core::auth::root_authority(&user)
        && req.target.kind == crate::types::IdeTargetKind::Local
    {
        ctx.authorize_local_target(&user, &req.target, false)
            .await?;
    }
    Ok(Json(
        crate::lsp_service::code_actions(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_diagnostics<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeDiagnosticsReq>,
) -> ApiResult<Json<IdeDiagnosticsResp>> {
    if let Some(target) = &mut req.target {
        target.absorb_project_folders(&req.project_folders);
    }
    Ok(Json(
        crate::lsp_service::diagnostics(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_hover<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeHoverReq>,
) -> ApiResult<Json<IdeHoverResp>> {
    if let Some(target) = &mut req.target {
        target.absorb_project_folders(&req.project_folders);
    }
    Ok(Json(crate::lsp_service::hover(&ctx, &user, req).await?))
}

// ---------------------------------------------------------------------------
// `<15ms` `@` File & Path Completion (`POST /ide/fs/complete`)
// ---------------------------------------------------------------------------

fn push_canon_dir(out: &mut Vec<PathBuf>, raw: &str) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let expanded =
        otto_core::paths::expand_home_strict(trimmed).unwrap_or_else(|_| PathBuf::from(trimmed));
    if let Ok(canon) = std::fs::canonicalize(&expanded) {
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
}

pub(super) async fn fs_complete<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFsCompleteReq>,
) -> ApiResult<Json<IdeFsCompleteResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    let auth_roots = if !has_root_auth {
        let mut authorized_any = false;
        let mut allowed_dirs: Vec<PathBuf> = Vec::new();
        if let Some(ref t) = req.target {
            if !t.root.trim().is_empty() {
                let resolved = ctx.ide().targets.resolve(&ctx, &user, t).await?;
                authorized_any = true;
                if let crate::target::ResolvedTarget::Local {
                    root,
                    additional_roots,
                    ..
                } = resolved
                {
                    if !allowed_dirs.contains(&root) {
                        allowed_dirs.push(root);
                    }
                    for ar in additional_roots {
                        if !allowed_dirs.contains(&ar) {
                            allowed_dirs.push(ar);
                        }
                    }
                }
            }
        }
        for r in &req.roots {
            if !r.trim().is_empty() {
                ctx.authorize_local_target(&user, &IdeTargetSpec::local(r.clone()), false)
                    .await?;
                push_canon_dir(&mut allowed_dirs, r);
                authorized_any = true;
            }
        }
        if let Some(cwd) = req.cwd.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            ctx.authorize_local_target(&user, &IdeTargetSpec::local(cwd.to_string()), false)
                .await?;
            push_canon_dir(&mut allowed_dirs, cwd);
            authorized_any = true;
        }
        if !authorized_any {
            return Err(
                Error::Forbidden("workspace root is required for path completion".into()).into(),
            );
        }
        for alias_val in req.aliases.values() {
            let trimmed = alias_val.trim();
            if !trimmed.is_empty()
                && ctx
                    .authorize_local_target(
                        &user,
                        &IdeTargetSpec::local(trimmed.to_string()),
                        false,
                    )
                    .await
                    .is_ok()
            {
                push_canon_dir(&mut allowed_dirs, trimmed);
            }
        }
        Some(allowed_dirs)
    } else {
        None
    };

    let resp = tokio::task::spawn_blocking(move || {
        crate::target::complete::complete_fs_paths_for_roots(
            &req,
            has_root_auth,
            auth_roots.as_deref(),
        )
    })
    .await
    .map_err(|e| Error::Internal(format!("fs complete worker failed: {e}")))?;
    Ok(Json(resp))
}

// ---------------------------------------------------------------------------
// Backend Local History & Deleted-File Recovery (`/ide/history/*`)
// ---------------------------------------------------------------------------

async fn authorize_history_root<S: IdeCtx>(
    ctx: &S,
    user: &User,
    raw_root: &str,
    write: bool,
) -> Result<(PathBuf, Vec<PathBuf>)> {
    if raw_root.trim().is_empty() {
        return Err(Error::Invalid("missing 'root' parameter".into()));
    }
    let spec = IdeTargetSpec::local(raw_root.to_string());
    let resolved = if write {
        ctx.ide().targets.resolve_write(ctx, user, &spec).await?
    } else {
        ctx.ide().targets.resolve(ctx, user, &spec).await?
    };
    match resolved {
        crate::target::ResolvedTarget::Local {
            root,
            additional_roots,
            ..
        } => Ok((root, additional_roots)),
        crate::target::ResolvedTarget::Ssh { .. } => Err(Error::Invalid(
            "local history requires a local target".into(),
        )),
    }
}

pub(super) async fn history_list<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeHistoryListQuery>,
) -> ApiResult<Json<IdeHistoryListResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    let (canon_root, extra_roots) = authorize_history_root(&ctx, &user, &q.root, false).await?;
    let store = ctx.ide().history.clone();
    let resp = tokio::task::spawn_blocking(move || {
        store.list_entries(
            &canon_root,
            &extra_roots,
            has_root_auth,
            q.path.as_deref(),
            q.include_deleted,
            q.limit,
            q.q.as_deref(),
        )
    })
    .await
    .map_err(|e| Error::Internal(format!("history list worker failed: {e}")))??;
    Ok(Json(resp))
}

pub(super) async fn history_entry<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeHistoryEntryQuery>,
) -> ApiResult<Json<IdeHistoryEntryResp>> {
    let has_root_auth = otto_core::auth::root_authority(&user);
    let (canon_root, extra_roots) = authorize_history_root(&ctx, &user, &q.root, false).await?;
    let store = ctx.ide().history.clone();
    let resp = tokio::task::spawn_blocking(move || {
        store.get_entry(&canon_root, &extra_roots, has_root_auth, &q.id)
    })
    .await
    .map_err(|e| Error::Internal(format!("history entry worker failed: {e}")))??;
    Ok(Json(resp))
}

pub(super) async fn history_restore<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeHistoryRestoreReq>,
) -> ApiResult<Json<IdeHistoryRestoreResp>> {
    if req
        .target
        .as_ref()
        .is_some_and(|t| t.kind == crate::types::IdeTargetKind::Ssh)
    {
        return Err(Error::Invalid("local history requires a local target".into()).into());
    }
    let has_root_auth = otto_core::auth::root_authority(&user);
    let (canon_root, extra_roots) =
        authorize_history_root(&ctx, &user, req.effective_root(), true).await?;
    let store = ctx.ide().history.clone();
    let resp = tokio::task::spawn_blocking(move || {
        store.restore_entry(&canon_root, &extra_roots, has_root_auth, &req.id)
    })
    .await
    .map_err(|e| Error::Internal(format!("history restore worker failed: {e}")))??;
    Ok(Json(resp))
}

pub(super) async fn history_snapshot<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeHistorySnapshotReq>,
) -> ApiResult<Json<IdeHistorySnapshotResp>> {
    if req
        .target
        .as_ref()
        .is_some_and(|t| t.kind == crate::types::IdeTargetKind::Ssh)
    {
        return Err(Error::Invalid("local history requires a local target".into()).into());
    }
    if !req.local_history_enabled {
        return Err(Error::Invalid("local history is disabled in preferences".into()).into());
    }
    let retention_days = req.local_history_retention_days;
    let has_root_auth = otto_core::auth::root_authority(&user);
    let (canon_root, extra_roots) =
        authorize_history_root(&ctx, &user, req.effective_root(), true).await?;
    let store = ctx.ide().history.clone();
    let is_root = has_root_auth;
    let entry = tokio::task::spawn_blocking(move || -> Result<_> {
        let raw_path = req.path.trim();
        let target_candidate = if raw_path.is_empty() || raw_path == "." {
            canon_root.clone()
        } else if raw_path.starts_with('~') {
            otto_core::paths::expand_home_strict(raw_path).map_err(Error::Invalid)?
        } else if FsPath::new(raw_path).is_absolute() {
            PathBuf::from(raw_path)
        } else {
            canon_root.join(raw_path)
        };
        let resolved_path = std::fs::canonicalize(&target_candidate)
            .ok()
            .or_else(|| otto_core::secret_paths::canonicalize_prospective(&target_candidate))
            .ok_or_else(|| Error::Invalid("invalid snapshot path".into()))?;

        let matched_root = std::iter::once(&canon_root)
            .chain(extra_roots.iter())
            .find(|r| resolved_path.starts_with(r))
            .ok_or_else(|| {
                Error::Forbidden(format!(
                    "snapshot path '{}' escapes workspace root",
                    resolved_path.display()
                ))
            })?;
        if !is_root {
            enforce_non_root_path(&resolved_path)?;
        }

        let label = req
            .label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Manual Snapshot");

        if resolved_path.is_dir() {
            let mut first = None;
            let walker = ignore::WalkBuilder::new(&resolved_path)
                .hidden(false)
                .git_ignore(false)
                .follow_links(false)
                .max_depth(Some(8))
                .build();
            for item in walker.flatten().take(200) {
                if !item.file_type().is_some_and(|ft| ft.is_file()) {
                    continue;
                }
                let p = item.path();
                let Ok(canon_p) = std::fs::canonicalize(p) else {
                    continue;
                };
                let Some(entry_root) = std::iter::once(&canon_root)
                    .chain(extra_roots.iter())
                    .find(|r| canon_p.starts_with(r))
                else {
                    continue;
                };
                if !is_root && enforce_non_root_path(&canon_p).is_err() {
                    continue;
                }
                if let Some(rec) = store.record_existing_file_with_retention_sync(
                    entry_root,
                    &canon_p,
                    "manual",
                    label,
                    false,
                    retention_days,
                ) {
                    if first.is_none() {
                        first = Some(rec);
                    }
                }
            }
            if let Some(e) = first {
                return Ok(e);
            }
            let marker = resolved_path.join(".otto-folder-snapshot");
            return store
                .record_bytes_with_retention_sync(
                    matched_root,
                    &marker,
                    b"",
                    "manual",
                    label,
                    false,
                    retention_days,
                )?
                .ok_or_else(|| Error::Invalid("unable to record folder snapshot".into()));
        }

        let bytes = match req.content {
            Some(buf) => buf.into_bytes(),
            None if resolved_path.is_file() => {
                std::fs::read(&resolved_path).map_err(|e| match e.kind() {
                    std::io::ErrorKind::NotFound => Error::NotFound(format!(
                        "snapshot target '{}' not found",
                        resolved_path.display()
                    )),
                    std::io::ErrorKind::PermissionDenied => Error::Forbidden(format!(
                        "permission denied reading '{}'",
                        resolved_path.display()
                    )),
                    _ => Error::Internal(format!(
                        "failed to read '{}': {e}",
                        resolved_path.display()
                    )),
                })?
            }
            None => Vec::new(),
        };
        // The editor's idle "Change" snapshot often lands right after the save already logged
        // the same bytes: skip it then. A labelled snapshot of the same bytes is still kept.
        store
            .record_bytes_with_retention_sync(
                matched_root,
                &resolved_path,
                &bytes,
                "manual",
                label,
                label == "Change",
                retention_days,
            )?
            .ok_or_else(|| {
                Error::Invalid("snapshot skipped (binary content or exceeds 2 MiB limit)".into())
            })
    })
    .await
    .map_err(|e| Error::Internal(format!("history snapshot worker failed: {e}")))??;

    Ok(Json(IdeHistorySnapshotResp { entry }))
}
