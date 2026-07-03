//! Snips: one-gesture screenshot → annotate → clipboard.
//!
//! The daemon owns the whole loop so the flow works from the desktop app and a
//! plain browser alike: capture runs `/usr/sbin/screencapture -i` (the native
//! interactive crosshair; space toggles window mode, Esc cancels), storage is
//! file-backed under `data_dir/snips/` (`<id>.png` original,
//! `<id>.annotated.png` flattened annotation export, `<id>.json` metadata
//! sidecar — deliberately no SQLite table: snips are ephemeral media, pruned
//! after `RETENTION_DAYS`), and every create/annotate/copy writes the macOS
//! pasteboard via `osascript` («class PNGf») so the latest state is always
//! paste-ready.
//!
//! Test seams: `OTTO_SNIP_CAPTURE_CMD` replaces the screencapture invocation
//! (`sh -c "$CMD" sh <out>` — the output path arrives as `$1`), and under
//! `OTTO_E2E` the pasteboard write is skipped. Either way the bytes that
//! *would* be on the clipboard are mirrored to `snips/clipboard-last.png`,
//! which doubles as an observability artifact and the E2E assertion target.
//!
//! Uploads are base64 JSON like `product_media` (no multipart dep), PNG-only
//! (magic-byte sniff), 25 MB raw / 40 MB body. Ids are daemon-generated ULIDs
//! and every path param is re-validated as plain ASCII alphanumerics before
//! touching the filesystem, so traversal is structurally impossible.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path as AxPath, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::Utc;
use otto_core::Error;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Storage sub-path under `data_dir`.
const SNIPS_ROOT: &str = "snips";
/// Maximum raw (decoded) PNG size: 25 MB (the routes cap bodies at 40 MB to
/// bound the ~33% base64 inflation, mirroring `product_media`).
const MAX_RAW_BYTES: usize = 25 * 1024 * 1024;
/// How long the interactive capture may sit on screen before we give up.
const CAPTURE_TIMEOUT_SECS: u64 = 120;
/// Snips older than this are pruned opportunistically on each create.
const RETENTION_DAYS: i64 = 14;

/// Serialized single-flight for the interactive capture: two crosshair UIs at
/// once is never what the user meant.
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn snips_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/snips",
            post(upload_snip)
                .layer(DefaultBodyLimit::max(40 * 1024 * 1024))
                .get(list_snips),
        )
        .route("/snips/capture", post(capture_snip))
        .route("/snips/{id}", delete(delete_snip))
        .route("/snips/{id}/image", get(snip_image))
        .route(
            "/snips/{id}/annotated",
            post(save_annotated)
                .layer(DefaultBodyLimit::max(40 * 1024 * 1024))
                .get(snip_annotated),
        )
        .route("/snips/{id}/copy", post(copy_snip))
}

// ---------------------------------------------------------------------------
// DTOs + sidecar
// ---------------------------------------------------------------------------

/// Wire + sidecar shape. `has_annotated` is computed from the filesystem at
/// read time (never stored) so the sidecar can't drift from reality.
#[derive(Serialize, Deserialize, Clone)]
pub struct Snip {
    pub id: String,
    pub created_at: String,
    pub width: u32,
    pub height: u32,
    /// `"capture"` or `"upload"`.
    pub source: String,
    #[serde(default)]
    pub has_annotated: bool,
}

#[derive(Deserialize)]
pub struct UploadSnipReq {
    pub data_b64: String,
    /// Original filename, metadata only (the on-disk name is the snip id).
    #[serde(default)]
    pub filename: Option<String>,
}

#[derive(Deserialize)]
pub struct AnnotatedReq {
    pub data_b64: String,
}

#[derive(Serialize)]
pub struct SnipCopyResp {
    pub copied: bool,
}

#[derive(Serialize)]
pub struct CaptureSnipResp {
    pub cancelled: bool,
    pub snip: Option<Snip>,
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

fn snips_dir(ctx: &ServerCtx) -> PathBuf {
    ctx.data_dir.join(SNIPS_ROOT)
}

/// Ids are daemon-generated ULIDs; anything else (traversal, spaces, too
/// short/long) is treated as "no such snip". This is the only line between a
/// path param and the filesystem, so it stays deliberately strict.
fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn png_path(ctx: &ServerCtx, id: &str) -> PathBuf {
    snips_dir(ctx).join(format!("{id}.png"))
}

fn annotated_path(ctx: &ServerCtx, id: &str) -> PathBuf {
    snips_dir(ctx).join(format!("{id}.annotated.png"))
}

fn sidecar_path(ctx: &ServerCtx, id: &str) -> PathBuf {
    snips_dir(ctx).join(format!("{id}.json"))
}

/// Parse PNG dimensions straight from the IHDR chunk (signature + first chunk
/// header + 8 bytes) — no image-crate dependency for two u32 reads.
fn png_dims(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// Decode + validate a base64 PNG payload (shared by upload + annotated save).
fn decode_png(data_b64: &str) -> Result<Vec<u8>, Error> {
    let bytes = B64
        .decode(data_b64.trim())
        .map_err(|e| Error::Invalid(format!("invalid base64: {e}")))?;
    if bytes.len() > MAX_RAW_BYTES {
        return Err(Error::Invalid(format!(
            "image exceeds {} MB cap",
            MAX_RAW_BYTES / (1024 * 1024)
        )));
    }
    if png_dims(&bytes).is_none() {
        return Err(Error::Invalid("payload is not a PNG image".into()));
    }
    Ok(bytes)
}

async fn load_snip(ctx: &ServerCtx, id: &str) -> Result<Snip, Error> {
    if !valid_id(id) {
        return Err(Error::NotFound(format!("snip {id}")));
    }
    let raw = tokio::fs::read(sidecar_path(ctx, id))
        .await
        .map_err(|_| Error::NotFound(format!("snip {id}")))?;
    let mut snip: Snip =
        serde_json::from_slice(&raw).map_err(|e| Error::Internal(format!("snip sidecar: {e}")))?;
    snip.has_annotated = annotated_path(ctx, id).exists();
    Ok(snip)
}

async fn store_snip(ctx: &ServerCtx, bytes: &[u8], source: &str) -> Result<Snip, Error> {
    let (width, height) =
        png_dims(bytes).ok_or_else(|| Error::Invalid("payload is not a PNG image".into()))?;
    let id = otto_core::new_id();
    let dir = snips_dir(ctx);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| Error::Internal(format!("create snips dir: {e}")))?;
    tokio::fs::write(png_path(ctx, &id), bytes)
        .await
        .map_err(|e| Error::Internal(format!("write snip: {e}")))?;
    let snip = Snip {
        id: id.clone(),
        created_at: Utc::now().to_rfc3339(),
        width,
        height,
        source: source.into(),
        has_annotated: false,
    };
    let sidecar = serde_json::to_vec(&snip)
        .map_err(|e| Error::Internal(format!("encode snip sidecar: {e}")))?;
    tokio::fs::write(sidecar_path(ctx, &id), sidecar)
        .await
        .map_err(|e| Error::Internal(format!("write snip sidecar: {e}")))?;
    prune_old(ctx).await;
    Ok(snip)
}

/// Best-effort retention sweep: drop snip file sets older than `RETENTION_DAYS`.
async fn prune_old(ctx: &ServerCtx) {
    let cutoff = Utc::now() - chrono::Duration::days(RETENTION_DAYS);
    let Ok(mut entries) = tokio::fs::read_dir(snips_dir(ctx)).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
            continue;
        };
        let id = id.to_string();
        let Ok(raw) = tokio::fs::read(entry.path()).await else {
            continue;
        };
        let Ok(snip) = serde_json::from_slice::<Snip>(&raw) else {
            continue;
        };
        let Ok(created) = chrono::DateTime::parse_from_rfc3339(&snip.created_at) else {
            continue;
        };
        if created.with_timezone(&Utc) < cutoff {
            let _ = tokio::fs::remove_file(png_path(ctx, &id)).await;
            let _ = tokio::fs::remove_file(annotated_path(ctx, &id)).await;
            let _ = tokio::fs::remove_file(sidecar_path(ctx, &id)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

/// Put a PNG on the macOS pasteboard. The bytes are always mirrored to
/// `snips/clipboard-last.png` (observability + the E2E assertion target); the
/// real `osascript` write is skipped under `OTTO_E2E`. Returns whether the
/// clipboard now (logically) holds the image — callers treat `false` as a
/// degraded success, never a request failure: the snip itself is saved.
async fn copy_png_to_clipboard(ctx: &ServerCtx, png: &Path) -> bool {
    if let Err(e) = tokio::fs::copy(png, snips_dir(ctx).join("clipboard-last.png")).await {
        tracing::warn!("snips: clipboard sink write failed: {e}");
    }
    if matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true")) {
        return true;
    }
    // AppleScript string literal: escape backslashes then quotes. The path is
    // daemon-controlled but may contain spaces ("Application Support").
    let esc = png.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("set the clipboard to (read (POSIX file \"{esc}\") as \u{ab}class PNGf\u{bb})");
    let run = tokio::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output();
    match tokio::time::timeout(Duration::from_secs(10), run).await {
        Ok(Ok(out)) if out.status.success() => true,
        Ok(Ok(out)) => {
            tracing::warn!(
                "snips: pasteboard write failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            false
        }
        Ok(Err(e)) => {
            tracing::warn!("snips: osascript spawn failed: {e}");
            false
        }
        Err(_) => {
            tracing::warn!("snips: pasteboard write timed out");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// RAII reset for the single-flight flag (capture can exit via `?`).
struct CaptureGuard;
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURE_ACTIVE.store(false, Ordering::SeqCst);
    }
}

enum CaptureOutcome {
    Captured(Vec<u8>),
    Cancelled,
}

/// Run the interactive capture into `out`. Esc (or any silent non-zero exit
/// with no file) is a clean cancel; a TCC/permission complaint on stderr is
/// surfaced as an actionable error.
async fn run_capture(out: &Path) -> Result<CaptureOutcome, Error> {
    let mut cmd = match std::env::var("OTTO_SNIP_CAPTURE_CMD") {
        // Test seam: `sh -c "$CMD" sh <out>` — the script sees the target as $1.
        Ok(script) => {
            let mut c = tokio::process::Command::new("/bin/sh");
            c.arg("-c").arg(script).arg("sh").arg(out);
            c
        }
        Err(_) => {
            let mut c = tokio::process::Command::new("/usr/sbin/screencapture");
            c.arg("-i").arg("-t").arg("png").arg(out);
            c
        }
    };
    let output = tokio::time::timeout(Duration::from_secs(CAPTURE_TIMEOUT_SECS), cmd.output())
        .await
        .map_err(|_| Error::Internal("screen capture timed out".into()))?
        .map_err(|e| Error::Internal(format!("launch screen capture: {e}")))?;

    let bytes = tokio::fs::read(out).await.unwrap_or_default();
    let _ = tokio::fs::remove_file(out).await;
    if !bytes.is_empty() && png_dims(&bytes).is_some() {
        return Ok(CaptureOutcome::Captured(bytes));
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    if stderr.contains("authoriz") || stderr.contains("permission") || stderr.contains("declined")
    {
        return Err(Error::Internal(
            "screen capture was blocked by macOS. Grant Screen Recording to \"ottod\" in \
             System Settings → Privacy & Security → Screen Recording, then retry."
                .into(),
        ));
    }
    Ok(CaptureOutcome::Cancelled)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /snips/capture` — interactive region/window capture. Long-polls while
/// the crosshair is on screen; Esc → `{cancelled:true}`. On success the
/// original is already on the clipboard when the response lands (R2).
pub async fn capture_snip(State(ctx): State<ServerCtx>) -> ApiResult<Json<CaptureSnipResp>> {
    if CAPTURE_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(ApiError(Error::Conflict(
            "a screen capture is already in progress".into(),
        )));
    }
    let _guard = CaptureGuard;

    let dir = snips_dir(&ctx);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("create snips dir: {e}"))))?;
    let out = dir.join(format!("capture-{}.pending.png", otto_core::new_id()));
    match run_capture(&out).await.map_err(ApiError)? {
        CaptureOutcome::Cancelled => Ok(Json(CaptureSnipResp {
            cancelled: true,
            snip: None,
        })),
        CaptureOutcome::Captured(bytes) => {
            let snip = store_snip(&ctx, &bytes, "capture").await.map_err(ApiError)?;
            copy_png_to_clipboard(&ctx, &png_path(&ctx, &snip.id)).await;
            Ok(Json(CaptureSnipResp {
                cancelled: false,
                snip: Some(snip),
            }))
        }
    }
}

/// `POST /snips` — base64 PNG upload ("annotate an existing image", and the
/// E2E seed path). Auto-copies like a capture.
pub async fn upload_snip(
    State(ctx): State<ServerCtx>,
    Json(req): Json<UploadSnipReq>,
) -> ApiResult<Json<Snip>> {
    let _ = req.filename; // metadata-only today; the on-disk name is the id
    let bytes = decode_png(&req.data_b64).map_err(ApiError)?;
    let snip = store_snip(&ctx, &bytes, "upload").await.map_err(ApiError)?;
    copy_png_to_clipboard(&ctx, &png_path(&ctx, &snip.id)).await;
    Ok(Json(snip))
}

/// `GET /snips` — newest first, capped at 100.
pub async fn list_snips(State(ctx): State<ServerCtx>) -> ApiResult<Json<Vec<Snip>>> {
    let mut snips = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(snips_dir(&ctx)).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            if let Ok(snip) = load_snip(&ctx, id).await {
                snips.push(snip);
            }
        }
    }
    snips.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    snips.truncate(100);
    Ok(Json(snips))
}

fn serve_png(bytes: Vec<u8>, name: &str) -> ApiResult<Response> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CONTENT_DISPOSITION, format!("inline; filename=\"{name}\""))
        .header("x-content-type-options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| ApiError(Error::Internal(format!("build response: {e}"))))
}

/// `GET /snips/{id}/image` — the original capture.
pub async fn snip_image(
    AxPath(id): AxPath<String>,
    State(ctx): State<ServerCtx>,
) -> ApiResult<Response> {
    let snip = load_snip(&ctx, &id).await.map_err(ApiError)?;
    let bytes = tokio::fs::read(png_path(&ctx, &snip.id))
        .await
        .map_err(|_| ApiError(Error::NotFound(format!("snip {id}"))))?;
    serve_png(bytes, &format!("{id}.png"))
}

/// `GET /snips/{id}/annotated` — the flattened annotated export (404 until the
/// first annotated save).
pub async fn snip_annotated(
    AxPath(id): AxPath<String>,
    State(ctx): State<ServerCtx>,
) -> ApiResult<Response> {
    let snip = load_snip(&ctx, &id).await.map_err(ApiError)?;
    let bytes = tokio::fs::read(annotated_path(&ctx, &snip.id))
        .await
        .map_err(|_| ApiError(Error::NotFound(format!("snip {id} has no annotated image"))))?;
    serve_png(bytes, &format!("{id}.annotated.png"))
}

/// `POST /snips/{id}/annotated` — save the flattened annotation export and put
/// it on the clipboard (the editor's debounced auto-copy target, R4).
pub async fn save_annotated(
    AxPath(id): AxPath<String>,
    State(ctx): State<ServerCtx>,
    Json(req): Json<AnnotatedReq>,
) -> ApiResult<Json<SnipCopyResp>> {
    let snip = load_snip(&ctx, &id).await.map_err(ApiError)?;
    let bytes = decode_png(&req.data_b64).map_err(ApiError)?;
    let path = annotated_path(&ctx, &snip.id);
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("write annotated snip: {e}"))))?;
    let copied = copy_png_to_clipboard(&ctx, &path).await;
    Ok(Json(SnipCopyResp { copied }))
}

/// `POST /snips/{id}/copy` — re-copy without editing: annotated if present,
/// else the original.
pub async fn copy_snip(
    AxPath(id): AxPath<String>,
    State(ctx): State<ServerCtx>,
) -> ApiResult<Json<SnipCopyResp>> {
    let snip = load_snip(&ctx, &id).await.map_err(ApiError)?;
    let path = if snip.has_annotated {
        annotated_path(&ctx, &snip.id)
    } else {
        png_path(&ctx, &snip.id)
    };
    let copied = copy_png_to_clipboard(&ctx, &path).await;
    Ok(Json(SnipCopyResp { copied }))
}

/// `DELETE /snips/{id}` — remove the snip's file set.
pub async fn delete_snip(
    AxPath(id): AxPath<String>,
    State(ctx): State<ServerCtx>,
) -> ApiResult<StatusCode> {
    let snip = load_snip(&ctx, &id).await.map_err(ApiError)?;
    let _ = tokio::fs::remove_file(png_path(&ctx, &snip.id)).await;
    let _ = tokio::fs::remove_file(annotated_path(&ctx, &snip.id)).await;
    tokio::fs::remove_file(sidecar_path(&ctx, &snip.id))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("delete snip: {e}"))))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_dims_parses_ihdr() {
        // 8-byte signature + IHDR length/type + 60x40.
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&60u32.to_be_bytes());
        bytes.extend_from_slice(&40u32.to_be_bytes());
        assert_eq!(png_dims(&bytes), Some((60, 40)));
        assert_eq!(png_dims(b"\xFF\xD8\xFF\xE0 not a png"), None);
        assert_eq!(png_dims(b""), None);
    }

    #[test]
    fn valid_id_rejects_traversal_shapes() {
        assert!(valid_id("01J8ZX0N7Q2R4T6V8X0Z2B4D6F"));
        assert!(valid_id("0123456789abcdef"));
        assert!(!valid_id("../../etc/passwd"));
        assert!(!valid_id("..%2F..%2Fetc"));
        assert!(!valid_id("a b"));
        assert!(!valid_id("short"));
        assert!(!valid_id(&"x".repeat(65)));
    }

    #[test]
    fn applescript_path_escaping() {
        let p = Path::new("/Users/x/Application Support/we\"ird\\name.png");
        let esc = p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
        assert_eq!(esc, "/Users/x/Application Support/we\\\"ird\\\\name.png");
    }
}
