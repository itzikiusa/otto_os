//! HTTP handlers for LSP Intelligence (`/ide/lsp/*`), Local History (`/ide/history/*`),
//! and `<15ms` `@` File Completion (`POST /ide/fs/complete`).

use std::path::{Path as FsPath, PathBuf};

use axum::extract::{Query, State};
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::{Error, Result};

use super::ApiResult;
use crate::target::{canonicalize_local_root_dir, enforce_non_root_path};
use crate::types::{
    IdeCodeActionsReq, IdeCodeActionsResp, IdeDiagnosticsReq, IdeDiagnosticsResp, IdeFormatReq,
    IdeFormatResp, IdeFsCompleteReq, IdeFsCompleteResp, IdeHistoryEntryQuery, IdeHistoryEntryResp,
    IdeHistoryListQuery, IdeHistoryListResp, IdeHistoryRestoreReq, IdeHistoryRestoreResp,
    IdeHistorySnapshotReq, IdeHistorySnapshotResp, IdeHoverReq, IdeHoverResp, IdeReferencesReq,
    IdeReferencesResp, IdeRenameReq, IdeRenameResp, IdeSymbolsReq, IdeSymbolsResp, IdeTargetSpec,
};
use crate::IdeCtx;

// ---------------------------------------------------------------------------
// Extended LSP & Fast Code Intelligence (`/ide/lsp/*`)
// ---------------------------------------------------------------------------

pub(super) async fn lsp_symbols<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeSymbolsReq>,
) -> ApiResult<Json<IdeSymbolsResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    Ok(Json(
        crate::lsp_service::document_symbols(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_definition<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    Ok(Json(
        crate::lsp_service::goto_definition(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_implementation<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    Ok(Json(
        crate::lsp_service::goto_implementation(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_references<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeReferencesReq>,
) -> ApiResult<Json<IdeReferencesResp>> {
    req.target.absorb_project_folders(&req.project_folders);
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    Ok(Json(
        crate::lsp_service::find_references(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_rename<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeRenameReq>,
) -> ApiResult<Json<IdeRenameResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    Ok(Json(
        crate::lsp_service::rename_symbol(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_format<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeFormatReq>,
) -> ApiResult<Json<IdeFormatResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
    Ok(Json(
        crate::lsp_service::format_document(&ctx, &user, req).await?,
    ))
}

pub(super) async fn lsp_code_actions<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(mut req): Json<IdeCodeActionsReq>,
) -> ApiResult<Json<IdeCodeActionsResp>> {
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut req.target)
        .await;
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
        ctx.ide()
            .environments
            .enrich_target_roots(&ctx.pool(), target)
            .await;
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
        ctx.ide()
            .environments
            .enrich_target_roots(&ctx.pool(), target)
            .await;
    }
    Ok(Json(crate::lsp_service::hover(&ctx, &user, req).await?))
}

// ---------------------------------------------------------------------------
// `<15ms` `@` File & Path Completion (`POST /ide/fs/complete`)
// ---------------------------------------------------------------------------

pub(super) async fn fs_complete<S: IdeCtx>(
    State(_ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFsCompleteReq>,
) -> ApiResult<Json<IdeFsCompleteResp>> {
    let is_root = user.is_root;
    let resp = tokio::task::spawn_blocking(move || {
        crate::target::complete::complete_fs_paths(&req, is_root)
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
) -> Result<(PathBuf, Vec<PathBuf>)> {
    if raw_root.trim().is_empty() {
        return Err(Error::Invalid("missing 'root' parameter".into()));
    }
    let mut spec = IdeTargetSpec::local(raw_root.to_string());
    ctx.ide()
        .environments
        .enrich_target_roots(&ctx.pool(), &mut spec)
        .await;
    let _ = ctx.ide().targets.resolve(ctx, user, &spec).await?;
    let canon_root = canonicalize_local_root_dir(raw_root, user.is_root)?;
    let mut extra_roots = Vec::new();
    for er in spec.merged_extra_roots() {
        if let Ok(c) = canonicalize_local_root_dir(&er, user.is_root) {
            if c != canon_root && !extra_roots.contains(&c) {
                extra_roots.push(c);
            }
        }
    }
    Ok((canon_root, extra_roots))
}

pub(super) async fn history_list<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeHistoryListQuery>,
) -> ApiResult<Json<IdeHistoryListResp>> {
    let (canon_root, _) = authorize_history_root(&ctx, &user, &q.root).await?;
    let store = ctx.ide().history.clone();
    let resp = tokio::task::spawn_blocking(move || {
        store.list_entries(
            &canon_root,
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
    let (canon_root, _) = authorize_history_root(&ctx, &user, &q.root).await?;
    let store = ctx.ide().history.clone();
    let resp = tokio::task::spawn_blocking(move || store.get_entry(&canon_root, &q.id))
        .await
        .map_err(|e| Error::Internal(format!("history entry worker failed: {e}")))??;
    Ok(Json(resp))
}

pub(super) async fn history_restore<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeHistoryRestoreReq>,
) -> ApiResult<Json<IdeHistoryRestoreResp>> {
    let (canon_root, extra_roots) =
        authorize_history_root(&ctx, &user, req.effective_root()).await?;
    let store = ctx.ide().history.clone();
    let is_root = user.is_root;
    let resp = tokio::task::spawn_blocking(move || {
        store.restore_entry(&canon_root, &extra_roots, is_root, &req.id)
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
    let (canon_root, extra_roots) =
        authorize_history_root(&ctx, &user, req.effective_root()).await?;
    let store = ctx.ide().history.clone();
    let is_root = user.is_root;
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

        let allowed = std::iter::once(&canon_root)
            .chain(extra_roots.iter())
            .any(|r| resolved_path.starts_with(r));
        if !allowed {
            return Err(Error::Forbidden(format!(
                "snapshot path '{}' escapes workspace root",
                resolved_path.display()
            )));
        }
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
                .max_depth(Some(8))
                .build();
            for item in walker.flatten().take(200) {
                let p = item.path();
                if p.is_file() {
                    if let Some(rec) =
                        store.record_existing_file_sync(&canon_root, p, "manual", label, false)
                    {
                        if first.is_none() {
                            first = Some(rec);
                        }
                    }
                }
            }
            if let Some(e) = first {
                return Ok(e);
            }
            let marker = resolved_path.join(".otto-folder-snapshot");
            return store
                .record_bytes_sync(&canon_root, &marker, b"", "manual", label, false)?
                .ok_or_else(|| Error::Invalid("unable to record folder snapshot".into()));
        }

        let bytes = if req.content.is_empty() && resolved_path.is_file() {
            std::fs::read(&resolved_path).unwrap_or_default()
        } else {
            req.content.into_bytes()
        };
        store
            .record_bytes_sync(&canon_root, &resolved_path, &bytes, "manual", label, false)?
            .ok_or_else(|| {
                Error::Invalid("snapshot skipped (binary content or exceeds 2 MiB limit)".into())
            })
    })
    .await
    .map_err(|e| Error::Internal(format!("history snapshot worker failed: {e}")))??;

    Ok(Json(IdeHistorySnapshotResp { entry }))
}
