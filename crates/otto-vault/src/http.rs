//! Vault v3 REST router. Paths are relative to the `/api/v1` mount point.
//! Reads require workspace `Viewer` (including the read-shaped POSTs `search`
//! and `okf/validate`, which policy.rs also gates View); mutations require
//! `Editor`.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};

use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{User, WorkspaceRole};
use otto_core::{Error, Id};

use crate::engine::VaultEngine;
use crate::types::*;

/// Host-application context required by the vault router.
pub trait VaultCtx: Clone + Send + Sync + 'static {
    fn vault(&self) -> &Arc<VaultEngine>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
}

struct ApiErr(Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
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
        let problem = Problem { code: self.0.code().to_string(), message: self.0.to_string() };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

async fn require<C: VaultCtx>(
    c: &C,
    user: &User,
    ws: &Id,
    min: WorkspaceRole,
) -> std::result::Result<(), ApiErr> {
    c.roles().check(user, ws, min).await.map_err(ApiErr)
}

#[derive(Deserialize)]
struct WsPath {
    ws: Id,
}

#[derive(Deserialize)]
struct WsVaultPath {
    ws: Id,
    id: i64,
}

#[derive(Deserialize)]
struct CreateVaultReq {
    name: String,
    #[serde(default)]
    root_path: Option<String>,
    #[serde(default)]
    okf: bool,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PatchVaultReq {
    name: Option<String>,
    okf: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PathQ {
    path: String,
}

#[derive(Deserialize)]
struct WriteNoteReq {
    path: String,
    content: String,
    #[serde(default)]
    if_hash: Option<String>,
}

#[derive(Deserialize)]
struct RenameReq {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct FolderReq {
    path: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SwitcherQ {
    q: String,
}

#[derive(Serialize)]
struct IndexesResp {
    written: usize,
}

async fn list_vaults<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
) -> ApiResult<Json<Vec<VaultRec>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().list(&ws).await?))
}

async fn create_vault<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<CreateVaultReq>,
) -> ApiResult<Json<VaultRec>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(c.vault().register(&ws, &req.name, req.root_path, req.okf).await?))
}

async fn patch_vault<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Json(req): Json<PatchVaultReq>,
) -> ApiResult<Json<VaultRec>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(c.vault().patch(&ws, id, req.name.as_deref(), req.okf).await?))
}

async fn delete_vault<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
) -> ApiResult<StatusCode> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    c.vault().unregister(&ws, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rescan<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
) -> ApiResult<Json<VaultStatus>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    c.vault().get_scoped(&ws, id).await?;
    c.vault().scan(id).await?;
    Ok(Json(c.vault().store().status(id).await?))
}

async fn status<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
) -> ApiResult<Json<VaultStatus>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().status(&ws, id).await?))
}

async fn dir<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(q): Query<PathQ>,
) -> ApiResult<Json<DirListing>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().dir(&ws, id, &q.path).await?))
}

async fn note<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(q): Query<PathQ>,
) -> ApiResult<Json<NoteFull>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().note(&ws, id, &q.path).await?))
}

async fn write_note<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Json(req): Json<WriteNoteReq>,
) -> ApiResult<Json<NoteMeta>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(
        c.vault()
            .write_note(&ws, id, &req.path, &req.content, req.if_hash.as_deref())
            .await?,
    ))
}

async fn delete_note<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(q): Query<PathQ>,
) -> ApiResult<StatusCode> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    c.vault().delete_note(&ws, id, &q.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Json(req): Json<RenameReq>,
) -> ApiResult<Json<RenameResult>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(c.vault().rename(&ws, id, &req.from, &req.to).await?))
}

async fn folder<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Json(req): Json<FolderReq>,
) -> ApiResult<StatusCode> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    c.vault().create_folder(&ws, id, &req.path).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn backlinks<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(q): Query<PathQ>,
) -> ApiResult<Json<Vec<Backlink>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().backlinks(&ws, id, &q.path).await?))
}

async fn search<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Json(req): Json<SearchReq>,
) -> ApiResult<Json<Vec<SearchHit>>> {
    // Read-shaped POST — Viewer (policy.rs gates it View as well).
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().search(&ws, id, &req).await?))
}

async fn switcher<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(q): Query<SwitcherQ>,
) -> ApiResult<Json<Vec<SwitchHit>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().switcher(&ws, id, &q.q).await?))
}

async fn tags<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
) -> ApiResult<Json<Vec<TagCount>>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().tags(&ws, id).await?))
}

async fn graph<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(o): Query<GraphOpts>,
) -> ApiResult<Json<GraphPayload>> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().graph(&ws, id, &o).await?))
}

async fn okf_validate<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
) -> ApiResult<Json<OkfReport>> {
    // Read-shaped POST — Viewer.
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(c.vault().okf_validate(&ws, id).await?))
}

async fn okf_indexes<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
) -> ApiResult<Json<IndexesResp>> {
    require(&c, &user, &ws, WorkspaceRole::Editor).await?;
    Ok(Json(IndexesResp { written: c.vault().okf_indexes(&ws, id).await? }))
}

/// Stream an attachment (image/pdf/…) with a best-effort content type.
async fn asset<C: VaultCtx>(
    State(c): State<C>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsVaultPath { ws, id }): Path<WsVaultPath>,
    Query(q): Query<PathQ>,
) -> ApiResult<Response> {
    require(&c, &user, &ws, WorkspaceRole::Viewer).await?;
    let abs = c.vault().asset_path(&ws, id, &q.path).await?;
    let bytes = tokio::fs::read(&abs)
        .await
        .map_err(|_| ApiErr(Error::NotFound(format!("asset {}", q.path))))?;
    let mime = mime_guess::from_path(&abs).first_or_octet_stream();
    Ok((
        [
            (header::CONTENT_TYPE, mime.essence_str().to_string()),
            (header::CACHE_CONTROL, "private, max-age=60".to_string()),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// Build the vault router. Paths are relative to the `/api/v1` mount point.
pub fn router<C: VaultCtx>() -> Router<C> {
    Router::new()
        .route("/workspaces/{ws}/vault/vaults", get(list_vaults::<C>).post(create_vault::<C>))
        .route(
            "/workspaces/{ws}/vault/vaults/{id}",
            axum::routing::patch(patch_vault::<C>).delete(delete_vault::<C>),
        )
        .route("/workspaces/{ws}/vault/vaults/{id}/rescan", post(rescan::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/status", get(status::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/dir", get(dir::<C>))
        .route(
            "/workspaces/{ws}/vault/vaults/{id}/note",
            get(note::<C>).put(write_note::<C>).delete(delete_note::<C>),
        )
        .route("/workspaces/{ws}/vault/vaults/{id}/rename", post(rename::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/folder", post(folder::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/backlinks", get(backlinks::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/search", post(search::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/switcher", get(switcher::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/tags", get(tags::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/graph", get(graph::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/okf/validate", post(okf_validate::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/okf/indexes", post(okf_indexes::<C>))
        .route("/workspaces/{ws}/vault/vaults/{id}/asset", get(asset::<C>))
}
