//! SPA serving with a history-API fallback to `index.html`.
//! The daemon supplies its embedded assets; this crate must not depend on
//! `ui/dist`, so a frontend edit does not recompile the entire server.
//! Without an asset loader, non-API paths serve a development placeholder.

use axum::http::{header, StatusCode, Uri};
use axum::response::Html;
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_core::api::Problem;
use std::borrow::Cow;

/// Asset lookup owned by the binary. A function pointer keeps router code
/// concrete, independent of the embedding implementation and generated data.
pub type AssetLoader = fn(&str) -> Option<Cow<'static, [u8]>>;

/// Root fallback handler: API/WS paths that reached here are true 404s;
/// everything else is the unembedded development placeholder.
pub async fn spa_fallback(uri: Uri) -> Response {
    spa_fallback_with_assets(uri, None).await
}

/// Serve the binary's assets without making them server compilation inputs.
pub async fn spa_fallback_with_assets(uri: Uri, assets: Option<AssetLoader>) -> Response {
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
    match assets {
        Some(load) => serve_spa(path, load),
        None => Html(PLACEHOLDER).into_response(),
    }
}

fn serve_spa(path: &str, load: AssetLoader) -> Response {
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    if let Some(data) = load(candidate) {
        let mime = mime_guess::from_path(candidate).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref().to_string())],
            data.into_owned(),
        )
            .into_response();
    }
    // History-API fallback: unknown non-asset paths get index.html.
    match load("index.html") {
        Some(index) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            index.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        http::Request,
        Router,
    };
    use std::borrow::Cow;
    use tower::ServiceExt;

    fn fixture(path: &str) -> Option<Cow<'static, [u8]>> {
        match path {
            "index.html" => Some(Cow::Borrowed(b"<main>Otto fixture</main>")),
            "assets/main.js" => Some(Cow::Borrowed(b"console.log('fixture')")),
            "assets/icon.png" => Some(Cow::Borrowed(b"\x89PNG")),
            _ => None,
        }
    }

    async fn request(path: &str, assets: Option<AssetLoader>) -> (StatusCode, String, Vec<u8>) {
        let app = Router::new().fallback(move |uri: Uri| spa_fallback_with_assets(uri, assets));
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap()
            .to_vec();
        (status, content_type, body)
    }

    #[tokio::test]
    async fn injected_assets_serve_root_and_deep_links() {
        for path in ["/", "/index.html", "/rooms/example?tab=chat"] {
            let (status, content_type, body) = request(path, Some(fixture)).await;
            assert_eq!(status, StatusCode::OK);
            assert!(content_type.starts_with("text/html"));
            assert_eq!(body, b"<main>Otto fixture</main>");
        }
    }

    #[tokio::test]
    async fn injected_assets_preserve_binary_bytes_and_mime() {
        let (status, content_type, body) = request("/assets/icon.png", Some(fixture)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "image/png");
        assert_eq!(body, b"\x89PNG");
        let (_, content_type, body) = request("/assets/main.js", Some(fixture)).await;
        assert!(content_type.contains("javascript"));
        assert_eq!(body, b"console.log('fixture')");
    }

    #[tokio::test]
    async fn missing_api_and_websocket_routes_never_load_spa_assets() {
        fn must_not_load(_: &str) -> Option<Cow<'static, [u8]>> {
            panic!("API and WebSocket misses must not resolve UI assets")
        }
        for path in ["/api", "/api/v1/missing", "/ws", "/ws/missing"] {
            let (status, content_type, body) = request(path, Some(must_not_load)).await;
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(content_type, "application/json");
            let problem: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(problem["code"], "not_found");
            assert_eq!(problem["message"], format!("no such route: {path}"));
        }
    }

    #[tokio::test]
    async fn unavailable_ui_and_missing_index_remain_distinct() {
        let (status, _, body) = request("/", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(String::from_utf8(body).unwrap().contains("UI not embedded"));
        let (status, _, body) = request("/deep/link", Some(|_| None)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, b"UI not built");
    }
}
