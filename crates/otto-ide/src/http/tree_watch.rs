//! `POST /ide/fs/watch` — the live project-tree lease (`crate::fs_watch`).

use axum::extract::State;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use serde::{Deserialize, Serialize};

use super::ApiResult;
use crate::target::ResolvedTarget;
use crate::types::IdeTargetSpec;
use crate::IdeCtx;

/// One IDE window's lease: the local project roots it shows and the folders it
/// has expanded (absolute). Re-posted every minute; empty `roots` releases it.
#[derive(Debug, Deserialize)]
pub struct IdeFsWatchReq {
    pub client_id: String,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub dirs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IdeFsWatchResp {
    /// The requested roots that now have a live watcher.
    pub watched: Vec<String>,
}

pub(super) async fn fs_watch<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeFsWatchReq>,
) -> ApiResult<Json<IdeFsWatchResp>> {
    let Some(events) = ctx.events().cloned() else {
        return Ok(Json(IdeFsWatchResp { watched: vec![] }));
    };
    // Each root goes through the same authorization as listing it; one the
    // caller may not read is simply not watched.
    let mut roots = Vec::new();
    for root in &req.roots {
        let spec = IdeTargetSpec::local(root);
        if let Ok(ResolvedTarget::Local { root: path, .. }) =
            ctx.ide().targets.resolve(&ctx, &user, &spec).await
        {
            roots.push((root.clone(), path));
        }
    }
    let ide = ctx.ide().clone();
    let watched = tokio::task::spawn_blocking(move || {
        ide.fs_watch
            .set_lease(&user.id, &req.client_id, roots, &req.dirs, events)
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("fs watch: {e}")))?;
    Ok(Json(IdeFsWatchResp { watched }))
}
