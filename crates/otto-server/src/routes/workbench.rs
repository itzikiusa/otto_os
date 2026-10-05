//! Workbench: per-user scratch files with a FULL, append-only edit history.
//!
//! Thin HTTP layer over `otto_state::WorkbenchRepo` (which owns the history
//! rules: coalesced autosave bursts, checkpoints, content-addressed blobs,
//! trash → explicit permanent delete). Every route is workspace-scoped AND
//! owner-scoped: a doc is only ever visible to the user who created it, so the
//! repo is always called with `(workspace, caller)`.
//!
//!   GET    /workspaces/{ws}/workbench/docs[?trash=true]          → [WorkbenchDoc]
//!   POST   /workspaces/{ws}/workbench/docs                       → 201 WorkbenchDocFull
//!   GET    /workspaces/{ws}/workbench/docs/{id}                  → WorkbenchDocFull
//!   PATCH  /workspaces/{ws}/workbench/docs/{id}                  → WorkbenchDoc
//!   DELETE /workspaces/{ws}/workbench/docs/{id}[?permanent=true] → WorkbenchDoc | 204
//!   POST   /workspaces/{ws}/workbench/docs/{id}/restore          → WorkbenchDoc
//!   GET    /workspaces/{ws}/workbench/docs/{id}/revisions        → [WorkbenchRevision]
//!   GET    /workspaces/{ws}/workbench/docs/{id}/revisions/{seq}  → WorkbenchRevisionDetail
//!   POST   /workspaces/{ws}/workbench/docs/{id}/revisions/{seq}/restore → WorkbenchDocFull
//!   GET    /workspaces/{ws}/workbench/docs/{id}/diff?from=&to=   → WorkbenchDiff
//!   POST   /workspaces/{ws}/workbench/assets (raw image body)    → 201 WorkbenchAsset
//!   GET    /workspaces/{ws}/workbench/assets/{id}                → image bytes
//!
//! Every mutation publishes `workbench_doc_changed` (owner-only) so the
//! caller's other windows refresh.

use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::workbench::MAX_ASSET_BYTES;
use otto_state::{
    NewWorkbenchDoc, WorkbenchAsset, WorkbenchDiff, WorkbenchDoc, WorkbenchDocFull, WorkbenchPatch,
    WorkbenchRepo, WorkbenchRevision, WorkbenchRevisionDetail,
};
use serde::Deserialize;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

pub fn workbench_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/workspaces/{ws}/workbench/docs",
            get(list_docs)
                .post(create_doc)
                .layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route(
            "/workspaces/{ws}/workbench/docs/{id}",
            get(get_doc)
                .patch(patch_doc)
                .delete(delete_doc)
                .layer(DefaultBodyLimit::max(8 * 1024 * 1024)),
        )
        .route(
            "/workspaces/{ws}/workbench/docs/{id}/restore",
            post(restore_doc),
        )
        .route(
            "/workspaces/{ws}/workbench/docs/{id}/revisions",
            get(list_revisions),
        )
        .route(
            "/workspaces/{ws}/workbench/docs/{id}/revisions/{seq}",
            get(get_revision),
        )
        .route(
            "/workspaces/{ws}/workbench/docs/{id}/revisions/{seq}/restore",
            post(restore_revision),
        )
        .route("/workspaces/{ws}/workbench/docs/{id}/diff", get(diff))
        .route(
            "/workspaces/{ws}/workbench/assets",
            post(upload_asset).layer(DefaultBodyLimit::max(MAX_ASSET_BYTES + 1024)),
        )
        .route("/workspaces/{ws}/workbench/assets/{id}", get(get_asset))
}

fn repo(ctx: &ServerCtx) -> WorkbenchRepo {
    WorkbenchRepo::new(ctx.pool.clone())
}

/// Publish the owner-only invalidation cue (best effort: no subscribers is fine).
fn changed(ctx: &ServerCtx, doc: &WorkbenchDoc, action: &str, client_id: Option<String>) {
    let _ = ctx.events.send(Event::WorkbenchDocChanged {
        workspace_id: doc.workspace_id.clone(),
        user_id: doc.owner_id.clone(),
        doc_id: doc.id.clone(),
        action: action.to_string(),
        rev: doc.rev,
        content_hash: doc.content_hash.clone(),
        updated_at: doc.updated_at.to_rfc3339(),
        client_id,
    });
}

#[derive(Deserialize)]
pub struct ListQ {
    #[serde(default)]
    trash: Option<bool>,
}

async fn list_docs(
    Path(ws): Path<Id>,
    Query(q): Query<ListQ>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<WorkbenchDoc>>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list(&ws, &user.id, q.trash.unwrap_or(false))
            .await?,
    ))
}

async fn create_doc(
    Path(ws): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<NewWorkbenchDoc>,
) -> ApiResult<(StatusCode, Json<WorkbenchDocFull>)> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let doc = repo(&ctx).create(&ws, &user.id, req).await?;
    changed(&ctx, &doc.doc, "created", None);
    Ok((StatusCode::CREATED, Json(doc)))
}

async fn get_doc(
    Path((ws, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkbenchDocFull>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(repo(&ctx).get(&ws, &user.id, &id).await?))
}

async fn patch_doc(
    Path((ws, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(patch): Json<WorkbenchPatch>,
) -> ApiResult<Json<WorkbenchDoc>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let client_id = patch.client_id.clone();
    let doc = repo(&ctx).update(&ws, &user.id, &id, patch).await?;
    changed(&ctx, &doc, "updated", client_id);
    Ok(Json(doc))
}

#[derive(Deserialize)]
pub struct DeleteQ {
    #[serde(default)]
    permanent: Option<bool>,
}

/// Soft delete (→ trash, `200` + the doc) or, with `?permanent=true`, the
/// irreversible purge of a TRASHED doc and its whole history (`204`; a live
/// doc answers `409`).
async fn delete_doc(
    Path((ws, id)): Path<(Id, Id)>,
    Query(q): Query<DeleteQ>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let r = repo(&ctx);
    if q.permanent.unwrap_or(false) {
        let doc = r.get_meta(&ws, &user.id, &id).await?;
        r.purge(&ws, &user.id, &id).await?;
        changed(&ctx, &doc, "deleted", None);
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let doc = r.trash(&ws, &user.id, &id).await?;
    changed(&ctx, &doc, "trashed", None);
    Ok(Json(doc).into_response())
}

async fn restore_doc(
    Path((ws, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkbenchDoc>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let doc = repo(&ctx).restore(&ws, &user.id, &id).await?;
    changed(&ctx, &doc, "restored", None);
    Ok(Json(doc))
}

#[derive(Deserialize)]
struct RevisionQuery {
    limit: Option<i64>,
    before_seq: Option<i64>,
}

async fn list_revisions(
    Path((ws, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(query): Query<RevisionQuery>,
) -> ApiResult<Json<Vec<WorkbenchRevision>>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx)
            .list_revisions_page(
                &ws,
                &user.id,
                &id,
                query.limit.unwrap_or(100),
                query.before_seq,
            )
            .await?,
    ))
}

async fn get_revision(
    Path((ws, id, seq)): Path<(Id, Id, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkbenchRevisionDetail>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(
        repo(&ctx).get_revision(&ws, &user.id, &id, seq).await?,
    ))
}

async fn restore_revision(
    Path((ws, id, seq)): Path<(Id, Id, i64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkbenchDocFull>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let doc = repo(&ctx).restore_revision(&ws, &user.id, &id, seq).await?;
    changed(&ctx, &doc.doc, "updated", None);
    Ok(Json(doc))
}

#[derive(Deserialize)]
pub struct DiffQ {
    from: i64,
    /// A revision seq, or `current` / absent for the current content.
    #[serde(default)]
    to: Option<String>,
}

async fn diff(
    Path((ws, id)): Path<(Id, Id)>,
    Query(q): Query<DiffQ>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<WorkbenchDiff>> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let to = match q.to.as_deref().map(str::trim) {
        None | Some("") | Some("current") => None,
        Some(s) => Some(s.parse::<i64>().map_err(|_| {
            ApiError(Error::Invalid(
                "`to` must be a revision seq or `current`".into(),
            ))
        })?),
    };
    Ok(Json(repo(&ctx).diff(&ws, &user.id, &id, q.from, to).await?))
}

/// Sniff the image type from magic bytes; SVG is accepted only when declared
/// (it is text) and is served sandboxed. Anything else is refused.
fn sniff_image(bytes: &[u8], declared: &str) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if declared.starts_with("image/svg+xml")
        && std::str::from_utf8(&bytes[..bytes.len().min(4096)])
            .map(|s| s.contains("<svg"))
            .unwrap_or(false)
    {
        Some("image/svg+xml")
    } else {
        None
    }
}

async fn upload_asset(
    Path(ws): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<WorkbenchAsset>)> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let declared = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mime = sniff_image(&body, &declared).ok_or_else(|| {
        ApiError(Error::Invalid(
            "unsupported image (PNG, JPEG, GIF, WebP or SVG)".into(),
        ))
    })?;
    let asset = repo(&ctx).put_asset(&ws, &user.id, mime, &body).await?;
    Ok((StatusCode::CREATED, Json(asset)))
}

async fn get_asset(
    Path((ws, id)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    let (meta, bytes) = repo(&ctx).get_asset(&ws, &user.id, &id).await?;
    let mut resp = Response::builder()
        .header(header::CONTENT_TYPE, meta.mime.as_str())
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            header::CACHE_CONTROL,
            "private, max-age=31536000, immutable",
        );
    if meta.mime == "image/svg+xml" {
        // An SVG can carry script: never let it run with the daemon's origin.
        resp = resp.header(header::CONTENT_SECURITY_POLICY, "sandbox");
    }
    resp.body(Body::from(bytes))
        .map_err(|e| ApiError(Error::Internal(format!("asset response: {e}"))))
}

#[cfg(test)]
mod tests {
    use super::sniff_image;

    #[test]
    fn sniffs_images_and_rejects_others() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nxx", ""), Some("image/png"));
        assert_eq!(
            sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0], ""),
            Some("image/jpeg")
        );
        assert_eq!(sniff_image(b"GIF89a..", ""), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 ", ""), Some("image/webp"));
        assert_eq!(
            sniff_image(
                b"<svg xmlns='http://www.w3.org/2000/svg'/>",
                "image/svg+xml"
            ),
            Some("image/svg+xml")
        );
        // SVG must be declared; HTML never passes.
        assert_eq!(sniff_image(b"<svg/>", "text/plain"), None);
        assert_eq!(sniff_image(b"<html><script>", "image/png"), None);
    }
}
