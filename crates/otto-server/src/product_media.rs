//! Story attachments + mockup pinned-annotation routes.
//!
//! Lives in `otto-server` (not `otto-product`) because attachment storage needs
//! `ServerCtx.data_dir` — `otto-product`'s `ProductCtx` exposes no filesystem.
//!
//! Attachment storage root is `data_dir/product/attachments/<story_id>/<id><ext>`;
//! the on-disk name is the daemon-generated attachment id (the original filename
//! is kept only as metadata). Uploads are base64 JSON (no multipart dep),
//! validated against a MIME allow-list **and** a magic-byte sniff of the decoded
//! bytes, capped at 25 MB raw (the upload route also caps the request body at
//! 40 MB to bound the ~33% base64 inflation). The serve handler canonicalizes the
//! resolved path and asserts containment under the attachments root before
//! reading, sets `Content-Type` from the stored mime, and sends
//! `X-Content-Type-Options: nosniff`.
//!
//! Design-arena artifacts ride the same table: `kind:'design'` rows whose mime is
//! one of the four `DesignFormat`s or an uploaded `model/gltf-binary` /
//! `model/gltf+json`. `PUT /product/attachments/{aid}/content` saves an edited
//! artifact from the UI (3D inspector, Excalidraw board, code view) through the
//! upload path's guards without ever changing the row's `mime`/`filename`, and
//! broadcasts `MockupUpdated` (text formats carry the source, binaries `null`).
//!
//! All routes follow the workspace-role pattern: mutations require Editor, reads
//! require Viewer. The workspace is resolved from the owning story (annotation
//! routes resolve the attachment → story → workspace).

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as AxPath, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{
    AnnotationPatch, AttachmentPatch, MockupAnnotation, NewAnnotation, NewAttachment,
    ProductAttachment,
};
use serde::Deserialize;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Maximum raw (decoded) attachment size: 25 MB.
pub(crate) const MAX_RAW_BYTES: usize = 25 * 1024 * 1024;
/// Storage sub-path under `data_dir` for story attachments.
pub(crate) const ATTACH_ROOT: &str = "product/attachments";

// ---------------------------------------------------------------------------
// Request / response bodies
// ---------------------------------------------------------------------------

/// `POST /product/stories/{sid}/attachments` body (base64 JSON, no multipart).
#[derive(Debug, Deserialize)]
pub struct UploadReq {
    pub filename: String,
    pub mime: String,
    #[serde(default)]
    pub kind: Option<String>,
    pub data_b64: String,
}

/// `PUT /product/attachments/{aid}/content` body — the new bytes, base64.
#[derive(Debug, Deserialize)]
pub struct ContentPutReq {
    pub data_b64: String,
    /// Optimistic concurrency: the `updated_at` the editor loaded (RFC 3339).
    /// When present and it no longer matches the row's `updated_at` (someone —
    /// the agent, another tab — saved in between) the save is a **409** and the
    /// client re-fetches instead of clobbering. Absent = unconditional save.
    #[serde(default)]
    pub base_updated_at: Option<String>,
}

/// `PATCH /product/attachments/{aid}` body.
#[derive(Debug, Default, Deserialize)]
pub struct AttachmentPatchReq {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub filename: Option<String>,
}

/// `POST /product/attachments/{aid}/annotations` body.
#[derive(Debug, Deserialize)]
pub struct AnnotationCreateReq {
    pub x_pct: f64,
    pub y_pct: f64,
    pub body: String,
}

/// `PATCH /product/annotations/{id}` body.
#[derive(Debug, Default, Deserialize)]
pub struct AnnotationPatchReq {
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub resolved: Option<bool>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// MIME allow-list for uploads. Executables/scripts are rejected by being
/// absent here (combined with the magic-byte sniff for the binary types). No
/// `text/x-python`: a Blender script is a server-side EXPORT of a validated
/// `scene3d` document, never an uploaded or agent-written file.
pub(crate) fn allowed_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/svg+xml"
            | "application/pdf"
            | "text/html"
            | "text/plain"
            | "text/markdown"
            | "text/vnd.mermaid"
            // Design arena.
            | "application/vnd.excalidraw+json"
            | "application/vnd.otto.scene3d+json"
            | "model/gltf-binary"
            | "model/gltf+json"
    )
}

/// Magic-byte sniff of the decoded bytes against the declared MIME. Binary types
/// (png/jpeg/gif/webp/pdf/glb) are checked by signature; text-ish types
/// (svg/html/plain/markdown/mermaid) are only required to be valid UTF-8; JSON
/// types (excalidraw/scene3d/gltf) must be UTF-8 opening with `{` after optional
/// whitespace (a full parse happens where the schema is known — `scene3d` is
/// validated by `design_scene3d::validate` on save).
pub(crate) fn sniff_ok(declared: &str, bytes: &[u8]) -> bool {
    match declared {
        "image/png" => bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        // RIFF....WEBP — 4-byte "RIFF", 4-byte size, then "WEBP".
        "image/webp" => bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "application/pdf" => bytes.starts_with(b"%PDF-"),
        // GLB: magic "glTF", u32 version, u32 length.
        "model/gltf-binary" => bytes.len() >= 12 && &bytes[0..4] == b"glTF",
        // Text-ish formats: accept any valid UTF-8 payload.
        "image/svg+xml" | "text/html" | "text/plain" | "text/markdown" | "text/vnd.mermaid" => {
            std::str::from_utf8(bytes).is_ok()
        }
        // JSON documents: UTF-8 and an object at the top.
        "application/vnd.excalidraw+json" | "application/vnd.otto.scene3d+json" | "model/gltf+json" => {
            std::str::from_utf8(bytes)
                .map(|s| s.trim_start().starts_with('{'))
                .unwrap_or(false)
        }
        _ => false,
    }
}

/// Storage extension for a MIME (matches `allowed_mime`).
pub(crate) fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/gif" => ".gif",
        "image/webp" => ".webp",
        "image/svg+xml" => ".svg",
        "application/pdf" => ".pdf",
        "text/html" => ".html",
        "text/markdown" => ".md",
        "text/vnd.mermaid" => ".mmd",
        "application/vnd.excalidraw+json" => ".excalidraw",
        "application/vnd.otto.scene3d+json" => ".json",
        "model/gltf-binary" => ".glb",
        "model/gltf+json" => ".gltf",
        _ => ".bin",
    }
}

/// Is this a text artifact whose source rides on `MockupUpdated.content`
/// (vs. a binary the client re-fetches)? Payloads above `MAX_EVENT_CONTENT`
/// are sent as `null` too so a giant board doesn't flood every WS client.
pub(crate) fn event_content(mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.len() > MAX_EVENT_CONTENT {
        return None;
    }
    let text = crate::design_format::DesignFormat::from_mime(mime).is_some()
        || matches!(mime, "text/plain" | "text/markdown" | "image/svg+xml" | "model/gltf+json");
    if text {
        String::from_utf8(bytes.to_vec()).ok()
    } else {
        None
    }
}

/// Above this the WS event carries `content: null` and clients re-fetch.
const MAX_EVENT_CONTENT: usize = 4 * 1024 * 1024;

/// Canonicalized-containment check: is `candidate` inside `root`?
///
/// Canonicalizes whatever prefix of each path exists on disk, then resolves any
/// `..`/`.` components lexically so the check still rejects traversal even when
/// the candidate (or some leading component) does not yet exist. Returns false if
/// `root` itself cannot be canonicalized.
fn path_within(root: &Path, candidate: &Path) -> bool {
    let root = match canonicalize_lexical(root) {
        Some(p) => p,
        None => return false,
    };
    match canonicalize_lexical(candidate) {
        Some(c) => c.starts_with(&root),
        None => false,
    }
}

/// Resolve a path as far as the filesystem allows (canonicalizing the existing
/// prefix to follow symlinks), then fold the remaining components lexically —
/// collapsing `.`/`..` — so traversal is normalized away even for not-yet-created
/// paths.
fn canonicalize_lexical(p: &Path) -> Option<PathBuf> {
    // Fast path: a fully-existing path canonicalizes directly.
    if let Ok(c) = p.canonicalize() {
        return Some(c);
    }
    // Otherwise canonicalize the longest existing ancestor, then append the rest
    // with `.`/`..` folded in. `tail` holds owned segments (innermost first):
    // `None` marks a `..` (pop), `Some(name)` marks a normal component (push).
    let mut existing = p.to_path_buf();
    let mut tail: Vec<Option<std::ffi::OsString>> = Vec::new();
    loop {
        if let Ok(c) = existing.canonicalize() {
            let mut out = c;
            for seg in tail.iter().rev() {
                match seg {
                    None => {
                        out.pop();
                    }
                    Some(name) => out.push(name),
                }
            }
            return Some(out);
        }
        // Pop the last component into `tail` (as an owned segment) and retry on
        // the parent. `.`-components are dropped; `..` becomes a pop marker.
        let seg = match existing.components().next_back()? {
            Component::ParentDir => Some(None),
            Component::CurDir => None,
            other => Some(Some(other.as_os_str().to_os_string())),
        };
        if let Some(seg) = seg {
            tail.push(seg);
        }
        if !existing.pop() {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Attachment handlers
// ---------------------------------------------------------------------------

/// `POST /product/stories/{sid}/attachments` — Editor. Base64 JSON upload.
pub async fn upload_attachment(
    AxPath(sid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<UploadReq>,
) -> ApiResult<Json<ProductAttachment>> {
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Editor).await?;

    // Validate the declared MIME against the allow-list.
    if !allowed_mime(&req.mime) {
        return Err(ApiError(Error::Invalid(format!(
            "disallowed content type: {}",
            req.mime
        ))));
    }
    // Decode the base64 payload.
    let bytes = B64
        .decode(req.data_b64.trim())
        .map_err(|e| ApiError(Error::Invalid(format!("invalid base64: {e}"))))?;
    if bytes.is_empty() {
        return Err(ApiError(Error::Invalid("empty attachment".into())));
    }
    if bytes.len() > MAX_RAW_BYTES {
        return Err(ApiError(Error::Invalid(format!(
            "attachment exceeds {} MB cap",
            MAX_RAW_BYTES / (1024 * 1024)
        ))));
    }
    // Magic-byte sniff: declared MIME must match the decoded bytes.
    if !sniff_ok(&req.mime, &bytes) {
        return Err(ApiError(Error::Invalid(
            "file contents do not match the declared type".into(),
        )));
    }

    // Daemon-generated on-disk name (the attachment id); original name is metadata.
    let id = otto_core::new_id();
    let ext = ext_for_mime(&req.mime);
    let rel = format!("{ATTACH_ROOT}/{sid}/{id}{ext}");
    // `sid` is a route param — confine both joins under the attachments root so
    // a hostile id can't steer the writes (rust/path-injection).
    let dir = otto_core::paths::confine_join(&ctx.data_dir.join(ATTACH_ROOT), &sid)
        .ok_or_else(|| ApiError(Error::Invalid(format!("unsafe story id {sid}"))))?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("create attachment dir: {e}"))))?;
    let full = otto_core::paths::confine_join(&ctx.data_dir, &rel)
        .ok_or_else(|| ApiError(Error::Invalid(format!("unsafe story id {sid}"))))?;
    tokio::fs::write(&full, &bytes)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("write attachment: {e}"))))?;

    let size_bytes = bytes.len() as i64;
    let kind = req
        .kind
        .filter(|k| !k.trim().is_empty())
        .unwrap_or_else(|| default_kind_for_mime(&req.mime));
    let att = ctx
        .attachment_repo
        .create(NewAttachment {
            story_id: story.id.clone(),
            workspace_id: story.workspace_id.clone(),
            filename: sanitize_filename(&req.filename),
            mime: req.mime.clone(),
            size_bytes,
            sha256: None,
            storage_path: rel,
            kind,
            source: "user".into(),
            meta_json: None,
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(att))
}

/// `GET /product/stories/{sid}/attachments` — Viewer.
pub async fn list_attachments(
    AxPath(sid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ProductAttachment>>> {
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Viewer).await?;
    let atts = ctx
        .attachment_repo
        .list_for_story(&story.id)
        .await
        .map_err(ApiError)?;
    Ok(Json(atts))
}

/// `GET /product/attachments/{aid}` — Viewer. Serves the bytes with the stored
/// content-type, `Content-Disposition: inline`, and `X-Content-Type-Options:
/// nosniff`. Canonicalizes the resolved path and asserts containment under the
/// attachments root before reading (defense in depth — paths are daemon-managed).
pub async fn serve_attachment(
    AxPath(aid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    let att = load_attachment(&ctx, &aid).await?;
    crate::auth::require_ws_role(&ctx, &user, &att.workspace_id, WorkspaceRole::Viewer).await?;

    let root = ctx.data_dir.join(ATTACH_ROOT);
    // Confine the stored path's join under the data dir (rust/path-injection)…
    let full = otto_core::paths::confine_join(&ctx.data_dir, &att.storage_path)
        .ok_or_else(|| ApiError(Error::Forbidden("attachment path escapes the data dir".into())))?;
    // …then path-sandbox: the resolved file MUST live under the attachments root.
    if !path_within(&root, &full) {
        return Err(ApiError(Error::Forbidden(
            "attachment path escapes the attachments root".into(),
        )));
    }
    let bytes = tokio::fs::read(&full)
        .await
        .map_err(|_| ApiError(Error::NotFound(format!("attachment file {aid}"))))?;

    let disposition = format!(
        "inline; filename=\"{}\"",
        att.filename.replace('"', "").replace(['\r', '\n'], "")
    );
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, att.mime.as_str())
        .header(header::CONTENT_DISPOSITION, disposition)
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| ApiError(Error::Internal(format!("build response: {e}"))))?;
    Ok(resp)
}

/// `PATCH /product/attachments/{aid}` — Editor. Update `kind`/`filename`.
pub async fn patch_attachment(
    AxPath(aid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AttachmentPatchReq>,
) -> ApiResult<Json<ProductAttachment>> {
    let att = load_attachment(&ctx, &aid).await?;
    crate::auth::require_ws_role(&ctx, &user, &att.workspace_id, WorkspaceRole::Editor).await?;
    let updated = ctx
        .attachment_repo
        .update(
            &aid,
            AttachmentPatch {
                kind: req.kind.filter(|k| !k.trim().is_empty()),
                filename: req.filename.map(|f| sanitize_filename(&f)),
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

/// `PUT /product/attachments/{aid}/content` — Editor. Replace an artifact's bytes
/// from the UI editor (3D inspector / Excalidraw board / code view). Reuses the
/// upload guards (allow-list, sniff, raw cap, confined path); the row's `mime`
/// and `filename` never change here, so extension and served content-type stay
/// consistent. A `scene3d` payload is additionally schema-validated (400 on a
/// bad document). Persists via `set_assist_result` (size + `updated_at`, meta
/// untouched) and broadcasts `MockupUpdated` — `content` is the source for text
/// formats and `null` for binaries / oversized payloads.
pub async fn put_attachment_content(
    AxPath(aid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ContentPutReq>,
) -> ApiResult<Json<ProductAttachment>> {
    let att = load_attachment(&ctx, &aid).await?;
    crate::auth::require_ws_role(&ctx, &user, &att.workspace_id, WorkspaceRole::Editor).await?;
    check_base_updated_at(req.base_updated_at.as_deref(), &att.updated_at).map_err(ApiError)?;
    let bytes = decode_content(&att.mime, &req.data_b64).map_err(ApiError)?;

    let root = ctx.data_dir.join(ATTACH_ROOT);
    let full = otto_core::paths::confine_join(&ctx.data_dir, &att.storage_path)
        .ok_or_else(|| ApiError(Error::Forbidden("attachment path escapes the data dir".into())))?;
    if !path_within(&root, &full) {
        return Err(ApiError(Error::Forbidden(
            "attachment path escapes the attachments root".into(),
        )));
    }
    if let Some(parent) = full.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError(Error::Internal(format!("create attachment dir: {e}"))))?;
    }
    tokio::fs::write(&full, &bytes)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("write attachment: {e}"))))?;

    let updated = ctx
        .attachment_repo
        .set_assist_result(&aid, bytes.len() as i64, None, att.meta_json.clone())
        .await
        .map_err(ApiError)?;

    let _ = ctx.events.send(Event::MockupUpdated {
        workspace_id: att.workspace_id.clone(),
        story_id: att.story_id.clone(),
        attachment_id: aid,
        format: crate::design_format::DesignFormat::from_mime(&att.mime)
            .map(|f| f.as_str().to_string())
            .unwrap_or_else(|| att.mime.clone()),
        content: event_content(&att.mime, &bytes),
    });
    Ok(Json(updated))
}

/// Optimistic-concurrency check for the content PUT: an unparseable
/// `base_updated_at` is a 400; a parseable one that differs from the row's
/// `updated_at` (compared as instants, so formatting differences don't matter)
/// is a 409.
pub(crate) fn check_base_updated_at(
    base: Option<&str>,
    current: &chrono::DateTime<chrono::Utc>,
) -> Result<(), Error> {
    let Some(base) = base.map(str::trim).filter(|b| !b.is_empty()) else {
        return Ok(());
    };
    let base = chrono::DateTime::parse_from_rfc3339(base)
        .map_err(|e| Error::Invalid(format!("base_updated_at is not RFC 3339: {e}")))?
        .with_timezone(&chrono::Utc);
    if base != *current {
        return Err(Error::Conflict(format!(
            "attachment changed since {} (now {}); reload before saving",
            base.to_rfc3339(),
            current.to_rfc3339()
        )));
    }
    Ok(())
}

/// The content PUT's guards, pure so they unit-test without a daemon: the row's
/// mime must be allowed, the base64 must decode to a non-empty payload under the
/// raw cap that sniffs as the declared type, and a `scene3d` document must
/// validate against its schema.
pub(crate) fn decode_content(mime: &str, data_b64: &str) -> Result<Vec<u8>, Error> {
    if !allowed_mime(mime) {
        return Err(Error::Invalid(format!("attachment type {mime} is not editable")));
    }
    let bytes = B64
        .decode(data_b64.trim())
        .map_err(|e| Error::Invalid(format!("invalid base64: {e}")))?;
    if bytes.is_empty() {
        return Err(Error::Invalid("empty content".into()));
    }
    if bytes.len() > MAX_RAW_BYTES {
        return Err(Error::Invalid(format!(
            "content exceeds {} MB cap",
            MAX_RAW_BYTES / (1024 * 1024)
        )));
    }
    if !sniff_ok(mime, &bytes) {
        return Err(Error::Invalid(
            "content does not match the attachment's type".into(),
        ));
    }
    if mime == crate::design_format::DesignFormat::Scene3d.mime() {
        crate::design_scene3d::validate_bytes(&bytes)?;
    }
    Ok(bytes)
}

/// `DELETE /product/attachments/{aid}` — Editor. Removes the DB row + the file
/// (best effort).
pub async fn delete_attachment(
    AxPath(aid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let att = load_attachment(&ctx, &aid).await?;
    crate::auth::require_ws_role(&ctx, &user, &att.workspace_id, WorkspaceRole::Editor).await?;
    ctx.attachment_repo.delete(&aid).await.map_err(ApiError)?;
    // Best-effort file removal (DB row is the source of truth).
    // Path-sandbox: only unlink if the confined (rust/path-injection) resolved
    // path is within the attachments root.
    let root = ctx.data_dir.join(ATTACH_ROOT);
    if let Some(full) = otto_core::paths::confine_join(&ctx.data_dir, &att.storage_path) {
        if path_within(&root, &full) {
            let _ = tokio::fs::remove_file(&full).await;
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Annotation handlers
// ---------------------------------------------------------------------------

/// `GET /product/attachments/{aid}/annotations` — Viewer.
pub async fn list_annotations(
    AxPath(aid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<MockupAnnotation>>> {
    let att = load_attachment(&ctx, &aid).await?;
    crate::auth::require_ws_role(&ctx, &user, &att.workspace_id, WorkspaceRole::Viewer).await?;
    let list = ctx
        .mockup_repo
        .list_for_attachment(&aid)
        .await
        .map_err(ApiError)?;
    Ok(Json(list))
}

/// `POST /product/attachments/{aid}/annotations` — Editor. Resolves the
/// attachment → story/workspace for the new row.
pub async fn create_annotation(
    AxPath(aid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AnnotationCreateReq>,
) -> ApiResult<Json<MockupAnnotation>> {
    let att = load_attachment(&ctx, &aid).await?;
    crate::auth::require_ws_role(&ctx, &user, &att.workspace_id, WorkspaceRole::Editor).await?;
    let ann = ctx
        .mockup_repo
        .create(NewAnnotation {
            attachment_id: aid,
            story_id: att.story_id.clone(),
            workspace_id: att.workspace_id.clone(),
            x_pct: req.x_pct,
            y_pct: req.y_pct,
            body: req.body,
            author_id: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;
    Ok(Json(ann))
}

/// `PATCH /product/annotations/{id}` — Editor.
pub async fn patch_annotation(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AnnotationPatchReq>,
) -> ApiResult<Json<MockupAnnotation>> {
    let ann = ctx.mockup_repo.get(&id).await.map_err(ApiError)?;
    let ann = ann.ok_or_else(|| ApiError(Error::NotFound(format!("annotation {id}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &ann.workspace_id, WorkspaceRole::Editor).await?;
    let updated = ctx
        .mockup_repo
        .update(
            &id,
            AnnotationPatch {
                body: req.body,
                resolved: req.resolved,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

/// `DELETE /product/annotations/{id}` — Editor.
pub async fn delete_annotation(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let ann = ctx.mockup_repo.get(&id).await.map_err(ApiError)?;
    let ann = ann.ok_or_else(|| ApiError(Error::NotFound(format!("annotation {id}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &ann.workspace_id, WorkspaceRole::Editor).await?;
    ctx.mockup_repo.delete(&id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Load an attachment by id, mapping absence to a 404.
async fn load_attachment(ctx: &ServerCtx, aid: &Id) -> ApiResult<ProductAttachment> {
    ctx.attachment_repo
        .get(aid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("attachment {aid}"))))
}

/// Default `kind` for a freshly-uploaded attachment from its MIME: images get
/// `image`, the Design-arena formats (Excalidraw / scene3d / glTF) get `design`
/// so the arena lists them, everything else gets `file`. ("mockup" is set
/// explicitly via PATCH.)
pub(crate) fn default_kind_for_mime(mime: &str) -> String {
    if mime.starts_with("image/") {
        "image".into()
    } else if matches!(
        mime,
        "application/vnd.excalidraw+json"
            | "application/vnd.otto.scene3d+json"
            | "model/gltf-binary"
            | "model/gltf+json"
    ) {
        "design".into()
    } else {
        "file".into()
    }
}

/// Strip any directory components from a user-supplied filename, keeping only the
/// final path segment for display (the on-disk name is the attachment id).
pub(crate) fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    if base.is_empty() || base == "." || base == ".." {
        "attachment".into()
    } else {
        base.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_disallowed_mime() {
        assert!(!allowed_mime("application/x-sh"));
        assert!(!allowed_mime("application/octet-stream"));
        assert!(allowed_mime("image/png"));
        assert!(allowed_mime("application/pdf"));
        assert!(allowed_mime("text/markdown"));
    }

    #[test]
    fn sniff_detects_png_mismatch() {
        let png = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        assert!(sniff_ok("image/png", &png));
        assert!(!sniff_ok("image/png", b"not a png"));
    }

    #[test]
    fn sniff_binary_and_text_types() {
        assert!(sniff_ok("image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(sniff_ok("image/gif", b"GIF89a...."));
        assert!(sniff_ok("application/pdf", b"%PDF-1.7\n..."));
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&[0, 0, 0, 0]);
        webp.extend_from_slice(b"WEBP");
        assert!(sniff_ok("image/webp", &webp));
        // Text-ish: any valid UTF-8 passes.
        assert!(sniff_ok("text/html", b"<html></html>"));
        assert!(sniff_ok("image/svg+xml", b"<svg/>"));
        // Invalid UTF-8 fails the text check.
        assert!(!sniff_ok("text/plain", &[0xFF, 0xFE, 0x00]));
    }

    #[test]
    fn ext_matches_mime() {
        assert_eq!(ext_for_mime("image/png"), ".png");
        assert_eq!(ext_for_mime("application/pdf"), ".pdf");
        assert_eq!(ext_for_mime("text/html"), ".html");
        assert_eq!(ext_for_mime("text/markdown"), ".md");
        assert_eq!(ext_for_mime("application/vnd.excalidraw+json"), ".excalidraw");
        assert_eq!(ext_for_mime("application/vnd.otto.scene3d+json"), ".json");
        assert_eq!(ext_for_mime("model/gltf-binary"), ".glb");
        assert_eq!(ext_for_mime("model/gltf+json"), ".gltf");
        assert_eq!(ext_for_mime("application/x-unknown"), ".bin");
    }

    #[test]
    fn design_mimes_are_allowed_sniffed_and_kind_design() {
        for m in [
            "application/vnd.excalidraw+json",
            "application/vnd.otto.scene3d+json",
            "model/gltf-binary",
            "model/gltf+json",
        ] {
            assert!(allowed_mime(m), "{m}");
            assert_eq!(default_kind_for_mime(m), "design", "{m}");
        }
        // Never Python: the Blender script is a server-side export.
        assert!(!allowed_mime("text/x-python"));
        assert_eq!(default_kind_for_mime("image/png"), "image");
        assert_eq!(default_kind_for_mime("application/pdf"), "file");

        // glTF magic header.
        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&[2, 0, 0, 0, 12, 0, 0, 0]);
        assert!(sniff_ok("model/gltf-binary", &glb));
        assert!(!sniff_ok("model/gltf-binary", b"{\"not\": \"glb\"}"));
        // JSON case: `{` after whitespace.
        assert!(sniff_ok("application/vnd.excalidraw+json", b"  \n{\"type\":\"excalidraw\"}"));
        assert!(sniff_ok("model/gltf+json", b"{\"asset\":{}}"));
        assert!(!sniff_ok("application/vnd.otto.scene3d+json", b"[1,2]"));
        assert!(!sniff_ok("application/vnd.otto.scene3d+json", b"<html>"));
    }

    #[test]
    fn content_put_guards() {
        let b64 = |b: &[u8]| B64.encode(b);
        // Happy path: html source.
        assert_eq!(decode_content("text/html", &b64(b"<p>x</p>")).unwrap(), b"<p>x</p>");
        // Disallowed row type (an executable somehow) → Invalid.
        assert!(decode_content("application/x-sh", &b64(b"echo")).is_err());
        // Bad base64 / empty payload.
        assert!(decode_content("text/html", "!!not-b64!!").is_err());
        assert!(decode_content("text/html", "").is_err());
        // Sniff mismatch: a PNG row fed HTML.
        assert!(decode_content("image/png", &b64(b"<html>")).is_err());
        // scene3d must validate: wrong type key → 400; a good doc passes.
        let bad = b64(br#"{"type":"nope","version":1}"#);
        assert!(decode_content("application/vnd.otto.scene3d+json", &bad).is_err());
        let good = crate::design_scene3d::empty_scene_json("T");
        assert!(decode_content("application/vnd.otto.scene3d+json", &b64(good.as_bytes())).is_ok());
        // Over the raw cap.
        let huge = vec![b'a'; MAX_RAW_BYTES + 1];
        assert!(decode_content("text/plain", &b64(&huge)).is_err());
    }

    #[test]
    fn base_updated_at_guard() {
        use chrono::{TimeZone, Utc};
        let now = Utc.with_ymd_and_hms(2026, 9, 5, 12, 0, 0).unwrap();
        // Absent / blank → unconditional.
        assert!(check_base_updated_at(None, &now).is_ok());
        assert!(check_base_updated_at(Some("  "), &now).is_ok());
        // Same instant, any RFC 3339 spelling → ok.
        assert!(check_base_updated_at(Some("2026-09-05T12:00:00Z"), &now).is_ok());
        assert!(check_base_updated_at(Some("2026-09-05T14:00:00+02:00"), &now).is_ok());
        // Stale → Conflict; garbage → Invalid.
        assert!(matches!(
            check_base_updated_at(Some("2026-09-05T11:59:59Z"), &now),
            Err(Error::Conflict(_))
        ));
        assert!(matches!(
            check_base_updated_at(Some("yesterday"), &now),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn event_content_is_source_for_text_and_null_for_binaries() {
        assert_eq!(event_content("text/html", b"<p>").as_deref(), Some("<p>"));
        assert_eq!(event_content("application/vnd.otto.scene3d+json", b"{}").as_deref(), Some("{}"));
        assert_eq!(event_content("model/gltf-binary", b"glTF...."), None);
        assert_eq!(event_content("image/png", b"\x89PNG"), None);
        let big = vec![b'x'; MAX_EVENT_CONTENT + 1];
        assert_eq!(event_content("text/html", &big), None);
    }

    #[test]
    fn path_within_blocks_escape() {
        let root = std::env::temp_dir();
        assert!(!path_within(&root, &root.join("../etc/passwd")));
    }

    #[test]
    fn path_within_allows_contained() {
        let root = std::env::temp_dir();
        assert!(path_within(
            &root,
            &root.join("product/attachments/s1/a1.png")
        ));
    }

    #[test]
    fn sanitize_strips_path_components() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("C:\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(sanitize_filename("  plain.png  "), "plain.png");
        assert_eq!(sanitize_filename(".."), "attachment");
    }
}
