//! SPA serving. With the `embed-ui` feature the contents of `ui/dist` are
//! embedded (rust-embed) and served with a history-API fallback to
//! `index.html`; without it a minimal placeholder page is served at every
//! non-API path so the daemon builds independently of the UI.

use axum::http::{header, StatusCode, Uri};
#[cfg(not(feature = "embed-ui"))]
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::api::Problem;

/// Root fallback handler: API/WS paths that reached here are true 404s;
/// everything else is the SPA (or placeholder).
pub async fn spa_fallback(uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") || path.starts_with("/ws/") || path == "/api" || path == "/ws" {
        return (
            StatusCode::NOT_FOUND,
            Json(Problem {
                code: "not_found".into(),
                message: format!("no such route: {path}"),
            }),
        )
            .into_response();
    }
    serve_spa(path)
}

#[cfg(feature = "embed-ui")]
#[derive(rust_embed::RustEmbed)]
#[folder = "../../ui/dist"]
struct UiAssets;

#[cfg(feature = "embed-ui")]
fn serve_spa(path: &str) -> Response {
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    if let Some(file) = UiAssets::get(candidate) {
        let mime = mime_guess::from_path(candidate).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref().to_string())],
            file.data.into_owned(),
        )
            .into_response();
    }
    // History-API fallback: unknown non-asset paths get index.html.
    match UiAssets::get("index.html") {
        Some(index) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            index.data.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}

#[cfg(not(feature = "embed-ui"))]
fn serve_spa(_path: &str) -> Response {
    let _ = header::CONTENT_TYPE; // keep import shared across cfg variants
    Html(PLACEHOLDER).into_response()
}

#[cfg(not(feature = "embed-ui"))]
const PLACEHOLDER: &str = r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Otto</title>
<style>
  body { font-family: -apple-system, system-ui, sans-serif; display: grid;
         place-items: center; min-height: 100vh; margin: 0; background: #111;
         color: #eee; }
  main { text-align: center; }
  code { background: #222; padding: 2px 6px; border-radius: 4px; }
</style></head>
<body><main>
  <h1>Otto daemon running</h1>
  <p>UI not embedded — build with the <code>embed-ui</code> feature, or use the API at <code>/api/v1</code>.</p>
</main></body>
</html>
"#;
