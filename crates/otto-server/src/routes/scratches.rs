//! Scratches routes: thin handlers over [`otto_ide::scratches`] (the folder,
//! the one-time Workbench export, path rules, read / write).
//!
//!   GET /scratches[?query=]                 → { root, files: \[ScratchFile\] }
//!   GET /scratches/file?path=<rel>          → ScratchFile + { content }
//!   PUT /scratches/file { path, content }   → ScratchFile + { created }

use std::path::PathBuf;

use axum::extract::{DefaultBodyLimit, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use otto_core::domain::User;
use otto_core::Result;
use otto_ide::scratches::{self, ScratchFileFull, ScratchList, WriteResp};
use serde::Deserialize;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

pub use otto_ide::scratches::{default_home, user_root, MAX_FILE_BYTES};

pub fn scratches_routes() -> Router<ServerCtx> {
    Router::new().route("/scratches", get(list)).route(
        "/scratches/file",
        get(read_file)
            .put(write_file)
            .layer(DefaultBodyLimit::max(MAX_FILE_BYTES + 64 * 1024)),
    )
}

/// The caller's folder, created on first use, with any not-yet-copied
/// Workbench docs exported into it. Returns its canonical path.
pub async fn ensure_root(ctx: &ServerCtx, user: &User) -> Result<PathBuf> {
    scratches::ensure_root(&ctx.pool, &ctx.scratches_home, &user.id).await
}

#[derive(Deserialize)]
pub struct ListQ {
    #[serde(default)]
    query: Option<String>,
}

async fn list(
    Query(q): Query<ListQ>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ScratchList>> {
    let root = ensure_root(&ctx, &user).await?;
    let files = scratches::list(&root, q.query.as_deref()).await?;
    Ok(Json(ScratchList {
        root: root.to_string_lossy().into_owned(),
        files,
    }))
}

#[derive(Deserialize)]
pub struct FileQ {
    path: String,
}

async fn read_file(
    Query(q): Query<FileQ>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ScratchFileFull>> {
    let root = ensure_root(&ctx, &user).await?;
    Ok(Json(scratches::read(root, q.path).await?))
}

#[derive(Deserialize)]
pub struct WriteReq {
    path: String,
    content: String,
}

/// Create or replace a scratch file (see [`scratches::write`]).
async fn write_file(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<WriteReq>,
) -> ApiResult<Json<WriteResp>> {
    if req.content.len() > MAX_FILE_BYTES {
        return Err(otto_core::Error::Invalid("content is larger than 5 MB".into()).into());
    }
    let root = ensure_root(&ctx, &user).await?;
    let history = ctx.ide.history.clone();
    Ok(Json(
        scratches::write(history, root, req.path, req.content).await?,
    ))
}
