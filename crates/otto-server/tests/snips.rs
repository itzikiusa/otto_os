//! Integration tests for the snips feature (screenshot → annotate → clipboard):
//!   POST   /api/v1/snips             (base64 PNG upload; doubles as the E2E seed path)
//!   POST   /api/v1/snips/capture     (interactive `screencapture -i`; test seam via
//!                                     `OTTO_SNIP_CAPTURE_CMD`)
//!   GET    /api/v1/snips
//!   GET    /api/v1/snips/{id}/image
//!   GET    /api/v1/snips/{id}/annotated
//!   POST   /api/v1/snips/{id}/annotated
//!   POST   /api/v1/snips/{id}/copy
//!   DELETE /api/v1/snips/{id}
//!
//! Uses the same "real minimal ServerCtx" harness as `canvas_refs_api.rs`
//! (stub secrets/spawner, in-memory sqlite, `tower::ServiceExt::oneshot` with
//! the `AuthUser` extension injected as the production auth middleware does),
//! with a per-test temp `data_dir` so the file-backed snip store and the
//! `clipboard-last.png` sink can be asserted on disk. `OTTO_E2E=1` is set so
//! the clipboard writer never touches the real macOS pasteboard; the two
//! capture tests serialize `OTTO_SNIP_CAPTURE_CMD` mutation behind a Mutex
//! (env vars are process-global).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::Router;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use chrono::Utc;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_rbac::RbacRoleChecker;
use otto_server::ServerCtx;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, DbPool, GitStore, IntegrationsRepo,
    IssuesRepo, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SwarmRepo, WorkspacesRepo,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tower::ServiceExt; // for `oneshot`

/// 60×40 solid-red PNG (132 bytes) — the shared fixture for upload/capture tests.
const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAADwAAAAoCAYAAACiu5n/AAAAS0lEQVR4nO3PQQ0AIBDAMIThXwVeQAbJro/913X2vpNavweAgYGBgYGB5wRcD7gecD3gesD1gOsB1wOuB1wPuB5wPeB6wPWA6wHXe+cRy1yXJ5HmAAAAAElFTkSuQmCC";
/// Same dimensions, solid blue — a byte-distinct valid PNG for the annotated slot.
const PNG2_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAADwAAAAoCAYAAACiu5n/AAAAS0lEQVR4nO3PMQ0AIADAMOSgCe14ARkko8f+dcy1z0+N1wPAwMDAwMDA/wRcD7gecD3gesD1gOsB1wOuB1wPuB5wPeB6wPWA6wHXuyw3KSsam61wAAAAAElFTkSuQmCC";

fn fixture_png() -> Vec<u8> {
    B64.decode(PNG_B64).expect("decode fixture")
}

fn fixture_png2() -> Vec<u8> {
    B64.decode(PNG2_B64).expect("decode fixture2")
}

/// Env-mutation guard: capture tests set `OTTO_SNIP_CAPTURE_CMD` process-wide.
/// tokio's Mutex so the guard may be held across the request `.await`s.
static CAPTURE_ENV: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
fn capture_env_lock() -> &'static tokio::sync::Mutex<()> {
    CAPTURE_ENV.get_or_init(|| tokio::sync::Mutex::new(()))
}

// ---------------------------------------------------------------------------
// Stubs (mirrors canvas_refs_api.rs)
// ---------------------------------------------------------------------------

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

fn user(id: &str) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root: false,
        disabled: false,
        created_at: Utc::now(),
    }
}

async fn test_ctx(pool: &DbPool, data_dir: PathBuf) -> ServerCtx {
    ServerCtx::for_tests(pool, data_dir).await
}

/// Build a minimal router exposing the snips endpoints under production paths.
fn snips_router(ctx: ServerCtx) -> Router {
    Router::new()
        .merge(otto_server::routes::snips::snips_routes())
        .with_state(ctx)
}

/// One-stop test app: temp data dir + router. Sets `OTTO_E2E=1` so clipboard
/// writes hit only the file sink (never the real pasteboard).
async fn test_app() -> (TempDir, PathBuf, Router) {
    std::env::set_var("OTTO_E2E", "1");
    let tmp = TempDir::new().expect("tempdir");
    let data_dir = tmp.path().to_path_buf();
    let pool = mem_pool().await;
    let ctx = test_ctx(&pool, data_dir.clone()).await;
    let app = snips_router(ctx);
    (tmp, data_dir, app)
}

async fn send(
    app: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Vec<u8>) {
    let b = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => b
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    };
    let mut req = req;
    req.extensions_mut().insert(AuthUser(user("alice")));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, body)
}

fn json(body: &[u8]) -> serde_json::Value {
    serde_json::from_slice(body).unwrap_or(serde_json::Value::Null)
}

// ---------------------------------------------------------------------------
// Upload + retrieval + clipboard sink
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upload_roundtrip_and_clipboard_sink() {
    let (_tmp, data_dir, app) = test_app().await;

    let (status, body) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": PNG_B64, "filename": "shot.png" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "upload: {}",
        String::from_utf8_lossy(&body)
    );
    let snip = json(&body);
    let id = snip["id"].as_str().expect("id").to_string();
    assert_eq!(snip["width"], 60);
    assert_eq!(snip["height"], 40);
    assert_eq!(snip["source"], "upload");
    assert_eq!(snip["has_annotated"], false);

    // R2: the upload was automatically "copied" — sink file byte-equals the fixture.
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink written");
    assert_eq!(sink, fixture_png());

    // Original PNG served back verbatim with the right headers.
    let (status, body) = send(&app, Method::GET, &format!("/snips/{id}/image"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, fixture_png());

    // Listed.
    let (status, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(status, StatusCode::OK);
    let list = json(&body);
    assert_eq!(list.as_array().map(|a| a.len()), Some(1));
    assert_eq!(list[0]["id"], id.as_str());
}

#[tokio::test]
async fn upload_rejects_non_png_and_empty() {
    let (_tmp, data_dir, app) = test_app().await;

    // JPEG magic bytes → 400.
    let jpeg = B64.encode([0xFFu8, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let (status, _) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": jpeg })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Garbage base64 → 400.
    let (status, _) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": "!!!not-base64!!!" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Nothing was copied to the clipboard sink.
    assert!(!data_dir.join("snips/clipboard-last.png").exists());
}

#[tokio::test]
async fn invalid_ids_are_not_found_and_do_not_traverse() {
    let (_tmp, _data_dir, app) = test_app().await;
    // Encoded traversal (axum rejects raw `../` in paths at the routing layer;
    // the encoded form reaches the handler and must fail the id check).
    for bad in [
        "..%2F..%2Fetc%2Fpasswd",
        "AB",
        "a%20b",
        "x".repeat(65).as_str(),
    ] {
        let (status, _) = send(&app, Method::GET, &format!("/snips/{bad}/image"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "id {bad:?} must 404");
    }
    // Well-formed but unknown id → 404.
    let (status, _) = send(&app, Method::GET, "/snips/0123456789abcdef/image", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Annotated save + copy preference + delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn annotated_save_updates_clipboard_and_copy_prefers_annotated() {
    let (_tmp, data_dir, app) = test_app().await;

    let (_, body) = send(
        &app,
        Method::POST,
        "/snips",
        Some(serde_json::json!({ "data_b64": PNG_B64 })),
    )
    .await;
    let id = json(&body)["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &app,
        Method::POST,
        &format!("/snips/{id}/annotated"),
        Some(serde_json::json!({ "data_b64": PNG2_B64 })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "save annotated: {}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(json(&body)["copied"], true);

    // R4: the clipboard sink now holds the ANNOTATED bytes, not the original.
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png2());

    // Annotated file exists and is served.
    let (status, got) = send(&app, Method::GET, &format!("/snips/{id}/annotated"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, fixture_png2());

    // List reflects has_annotated.
    let (_, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(json(&body)[0]["has_annotated"], true);

    // Copy prefers the annotated file: wipe the sink, re-copy, sink reappears.
    std::fs::remove_file(data_dir.join("snips/clipboard-last.png")).unwrap();
    let (status, body) = send(&app, Method::POST, &format!("/snips/{id}/copy"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["copied"], true);
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png2(), "copy must prefer the annotated bytes");

    // Delete removes everything.
    let (status, _) = send(&app, Method::DELETE, &format!("/snips/{id}"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = send(&app, Method::GET, &format!("/snips/{id}/image"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(json(&body).as_array().map(|a| a.len()), Some(0));
}

#[tokio::test]
async fn annotated_rejects_unknown_snip() {
    let (_tmp, _data_dir, app) = test_app().await;
    let (status, _) = send(
        &app,
        Method::POST,
        "/snips/0123456789abcdef/annotated",
        Some(serde_json::json!({ "data_b64": PNG_B64 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Capture (via the OTTO_SNIP_CAPTURE_CMD test seam)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn capture_success_creates_snip_and_copies() {
    let _guard = capture_env_lock().lock().await;
    let (_tmp, data_dir, app) = test_app().await;

    // Fixture source the fake "screencapture" copies from.
    let src = data_dir.join("fixture.png");
    std::fs::write(&src, fixture_png()).unwrap();
    std::env::set_var(
        "OTTO_SNIP_CAPTURE_CMD",
        format!("cp {} \"$1\"", src.display()),
    );

    let (status, body) = send(
        &app,
        Method::POST,
        "/snips/capture",
        Some(serde_json::json!({})),
    )
    .await;
    std::env::remove_var("OTTO_SNIP_CAPTURE_CMD");

    assert_eq!(
        status,
        StatusCode::OK,
        "capture: {}",
        String::from_utf8_lossy(&body)
    );
    let resp = json(&body);
    assert_eq!(resp["cancelled"], false);
    let snip = &resp["snip"];
    assert_eq!(snip["source"], "capture");
    assert_eq!(snip["width"], 60);
    assert_eq!(snip["height"], 40);

    // R2: capture auto-copied.
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png());
}

#[tokio::test]
async fn capture_cancel_reports_cancelled() {
    let _guard = capture_env_lock().lock().await;
    let (_tmp, data_dir, app) = test_app().await;

    // Fake screencapture that writes nothing and exits 1 (Esc behavior).
    std::env::set_var("OTTO_SNIP_CAPTURE_CMD", "exit 1");
    let (status, body) = send(
        &app,
        Method::POST,
        "/snips/capture",
        Some(serde_json::json!({})),
    )
    .await;
    std::env::remove_var("OTTO_SNIP_CAPTURE_CMD");

    assert_eq!(status, StatusCode::OK);
    let resp = json(&body);
    assert_eq!(resp["cancelled"], true);
    assert!(resp["snip"].is_null());
    assert!(!data_dir.join("snips/clipboard-last.png").exists());

    // Nothing listed.
    let (_, body) = send(&app, Method::GET, "/snips", None).await;
    assert_eq!(json(&body).as_array().map(|a| a.len()), Some(0));
}

// ---------------------------------------------------------------------------
// Raw image/png bodies + HTTP caching (perf: no base64 inflation per edit)
// ---------------------------------------------------------------------------

async fn send_raw(
    app: &Router,
    method: Method,
    uri: &str,
    ct: &str,
    body: Vec<u8>,
    if_none_match: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", ct);
    if let Some(t) = if_none_match {
        b = b.header("if-none-match", t);
    }
    let mut req = b.body(Body::from(body)).unwrap();
    req.extensions_mut().insert(AuthUser(user("alice")));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

#[tokio::test]
async fn raw_png_upload_and_annotated_save_with_cache_validators() {
    let (_tmp, data_dir, app) = test_app().await;

    // Raw upload (no JSON, no base64).
    let (status, _, body) = send_raw(
        &app,
        Method::POST,
        "/snips",
        "image/png",
        fixture_png(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let id = json(&body)["id"].as_str().unwrap().to_string();
    assert_eq!(json(&body)["width"], 60);

    // Raw non-PNG is refused by the header sniff.
    let (status, _, _) = send_raw(
        &app,
        Method::POST,
        "/snips",
        "image/png",
        vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0],
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Raw annotated save lands on the clipboard sink byte-exactly.
    let uri = format!("/snips/{id}/annotated");
    let (status, _, body) =
        send_raw(&app, Method::POST, &uri, "image/png", fixture_png2(), None).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink, fixture_png2());

    // The original is immutable; the annotated export revalidates via ETag.
    let img = format!("/snips/{id}/image");
    let (status, h, got) = send_raw(&app, Method::GET, &img, "", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, fixture_png());
    assert!(h["cache-control"].to_str().unwrap().contains("immutable"));
    let (status, h, got) = send_raw(&app, Method::GET, &uri, "", vec![], None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(got, fixture_png2());
    let tag = h["etag"].to_str().unwrap().to_string();
    let (status, _, got) = send_raw(&app, Method::GET, &uri, "", vec![], Some(&tag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(got.is_empty());
}

/// R4 budget: a 15 MB raw `image/png` upload and a 15 MB raw annotated save
/// each finish well under 300 ms (header sniff only — no base64, no JSON, no
/// decode), and the bytes land intact.
#[tokio::test]
async fn raw_15mb_upload_and_annotated_save_within_budget() {
    let (_tmp, data_dir, app) = test_app().await;
    // A valid PNG header (the real fixture) padded to 15 MB — the server only
    // sniffs the IHDR, so this is what a large screen capture costs it.
    let mut big = fixture_png();
    big.resize(15 * 1024 * 1024, 0x5A);

    let t = std::time::Instant::now();
    let (status, _, body) =
        send_raw(&app, Method::POST, "/snips", "image/png", big.clone(), None).await;
    let upload_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let id = json(&body)["id"].as_str().unwrap().to_string();

    let uri = format!("/snips/{id}/annotated");
    let t = std::time::Instant::now();
    let (status, _, body) =
        send_raw(&app, Method::POST, &uri, "image/png", big.clone(), None).await;
    let save_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let sink = std::fs::read(data_dir.join("snips/clipboard-last.png")).expect("sink");
    assert_eq!(sink.len(), big.len());

    eprintln!("snips 15 MB raw: upload {upload_ms:.1} ms, annotated save {save_ms:.1} ms");
    assert!(upload_ms < 300.0, "15 MB upload took {upload_ms:.1} ms");
    assert!(save_ms < 300.0, "15 MB annotated save took {save_ms:.1} ms");
}
