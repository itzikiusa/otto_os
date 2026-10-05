//! SPA serving with a history-API fallback to `index.html`.
//! The daemon supplies its embedded assets; this crate must not depend on
//! `ui/dist`, so a frontend edit does not recompile the entire server.
//! Without an asset loader, non-API paths serve a development placeholder.

use axum::http::{header, HeaderValue, StatusCode, Uri};
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

/// Hashed build output: the name changes whenever the bytes do, so a client
/// may keep it forever. Documents (pop-outs, side panes, remote clients)
/// then reuse chunks instead of refetching them per window.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// The document names the current chunk hashes; it must be revalidated so a
/// deploy is picked up on the next load rather than after a stale cache hit.
const REVALIDATE: &str = "no-cache";

/// CSP for the UI when ottod itself serves it (remote/LAN/Tailscale clients,
/// `npm run preview` against the daemon). Mirrors the desktop app's CSP in
/// `apps/desktop/src-tauri/tauri.conf.json` minus the Tauri-only sources
/// (`ipc:` / `http://ipc.localhost`). `'self'` is the daemon origin, which —
/// per CSP3 — also matches same-host `ws:`/`wss:` for the event/terminal
/// sockets. `frame-ancestors 'self'` stops another site from framing the UI
/// (clickjacking); the same-origin side pane (`?embed=1`) still works.
///
/// Two deliberately broad sources (S8-12 / S11-13), pinned by
/// `csp_broad_sources_are_the_documented_ones`:
/// - `'unsafe-eval'`: the D2 diagram renderer (`@terrastruct/d2`) loads ELK and
///   its setup script with `new Function(...)` inside a `blob:` worker, and a
///   blob worker inherits the DOCUMENT's policy — without it every D2 diagram
///   fails to render. Removing it needs D2 served as a same-origin worker
///   script with its own CSP header; until then any change must keep D2 working.
/// - `frame-src https: http:`: the Browser panel's direct mode frames the
///   site the user typed (`BrowserPanel.svelte`; X-Frame-Options sites fall
///   back to the daemon proxy) and plugin panes frame their own local origin.
///   Every such frame is cross-origin to the UI, so it gets no access to the
///   app's DOM or storage.
pub const SPA_CSP: &str = "default-src 'self'; \
    script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'; \
    style-src 'self' 'unsafe-inline'; \
    img-src 'self' data: blob: https: http://127.0.0.1:7700; \
    font-src 'self' data:; \
    connect-src 'self' http://127.0.0.1:7700 ws://127.0.0.1:7700 https://127.0.0.1:7700 \
    wss://127.0.0.1:7700 http://localhost:7700 ws://localhost:7700 blob: data:; \
    worker-src 'self' blob:; \
    frame-src 'self' blob: data: https: http:; \
    media-src 'self' blob: data: https://github.com https://objects.githubusercontent.com \
    https://release-assets.githubusercontent.com; \
    object-src 'none'; \
    base-uri 'self'; \
    frame-ancestors 'self'";

/// Stamp the document-hardening headers on every SPA response.
fn harden(mut response: Response) -> Response {
    let h = response.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(SPA_CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response
}

fn serve_spa(path: &str, load: AssetLoader) -> Response {
    harden(serve_spa_inner(path, load))
}

fn serve_spa_inner(path: &str, load: AssetLoader) -> Response {
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        "index.html"
    } else {
        trimmed
    };

    if let Some(data) = load(candidate) {
        let mime = mime_guess::from_path(candidate).first_or_octet_stream();
        let mut response = (
            [(header::CONTENT_TYPE, mime.as_ref().to_string())],
            data.into_owned(),
        )
            .into_response();
        let cache = if candidate == "index.html" {
            Some(REVALIDATE)
        } else if is_hashed_asset(candidate) {
            Some(IMMUTABLE)
        } else {
            None
        };
        if let Some(cache) = cache {
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
        }
        return response;
    }
    // Build output is never a client route. A chunk missing after a deploy
    // must fail as a 404, not as index.html served under a script's URL
    // (a MIME error that hides the real cause).
    if candidate.starts_with("assets/") {
        return (StatusCode::NOT_FOUND, "asset not found").into_response();
    }
    // History-API fallback: unknown non-asset paths get index.html.
    match load("index.html") {
        Some(index) => (
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8"),
                (header::CACHE_CONTROL, REVALIDATE),
            ],
            index.into_owned(),
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
    }
}

/// Vite emits build output as `assets/<name>-<hash>.<ext>`, the hash being
/// 8 base64url chars (which may themselves contain `-`/`_`). Anything else
/// under `assets/` keeps default caching, so an unhashed file is never
/// pinned for a year.
fn is_hashed_asset(path: &str) -> bool {
    const HASH_LEN: usize = 8;
    if let Some(font) = path.strip_prefix("assets/excalidraw/fonts/") {
        return is_excalidraw_font(font);
    }
    let Some(file) = path.strip_prefix("assets/") else {
        return false;
    };
    if file.contains('/') {
        return false;
    }
    let Some((stem, _ext)) = file.rsplit_once('.') else {
        return false;
    };
    let bytes = stem.as_bytes();
    // `<at least one name char>-<HASH_LEN hash chars>`
    if bytes.len() < HASH_LEN + 2 {
        return false;
    }
    let (head, hash) = bytes.split_at(bytes.len() - HASH_LEN);
    head.ends_with(b"-")
        && hash
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

/// Excalidraw's own font files, copied into `assets/excalidraw/fonts/` by the
/// UI build (`vite.config.ts` `excalidrawFonts`): `<Family>/<name>-<32 hex>.woff2`
/// — the package content-hashes every file, so they are immutable too.
fn is_excalidraw_font(rel: &str) -> bool {
    let Some((family, file)) = rel.split_once('/') else {
        return false;
    };
    if family.is_empty() || file.contains('/') || !family.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return false;
    }
    let Some(stem) = file.strip_suffix(".woff2") else {
        return false;
    };
    let Some((name, hash)) = stem.rsplit_once('-') else {
        return false;
    };
    !name.is_empty() && hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit())
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
            "assets/App-Dx-3l_Gc.js" | "assets/mermaid.core-B7Hx2kzQ.js" => {
                Some(Cow::Borrowed(b"export {}"))
            }
            "sw.js" => Some(Cow::Borrowed(b"self")),
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

    /// S8-12 / S11-13: the only broad sources are the two documented next to
    /// [`SPA_CSP`]; anything new (or a wider script/frame source) must be a
    /// deliberate, reviewed change to this test.
    #[test]
    fn csp_broad_sources_are_the_documented_ones() {
        let directive = |name: &str| {
            SPA_CSP
                .split(';')
                .map(str::trim)
                .find(|d| d.starts_with(name))
                .unwrap_or_else(|| panic!("missing {name}"))
                .to_string()
        };
        assert_eq!(
            directive("script-src "),
            "script-src 'self' 'wasm-unsafe-eval' 'unsafe-eval'",
            "'unsafe-eval' is only for the D2 worker; never add 'unsafe-inline'"
        );
        assert_eq!(directive("frame-src "), "frame-src 'self' blob: data: https: http:");
        assert_eq!(directive("object-src "), "object-src 'none'");
        assert_eq!(directive("frame-ancestors "), "frame-ancestors 'self'");
        assert!(!SPA_CSP.contains('*'), "no wildcard sources");
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

    async fn cache_control(path: &str) -> (StatusCode, Option<String>) {
        let app =
            Router::new().fallback(move |uri: Uri| spa_fallback_with_assets(uri, Some(fixture)));
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let cache = response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_owned());
        (response.status(), cache)
    }

    #[tokio::test]
    async fn missing_build_assets_are_404_not_the_spa_document() {
        for path in [
            "/assets/App-Zz9_x-Q1.js",
            "/assets/gone.css",
            "/assets/nested/chunk-AbCdEfGh.js",
        ] {
            let (status, content_type, body) = request(path, Some(fixture)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(!content_type.starts_with("text/html"), "{path}");
            assert_ne!(body, b"<main>Otto fixture</main>", "{path}");
        }
        // A client route that merely contains "assets" still gets the SPA.
        let (status, content_type, _) = request("/vault/assets/x", Some(fixture)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(content_type.starts_with("text/html"));
    }

    #[tokio::test]
    async fn hashed_assets_are_immutable_and_the_document_revalidates() {
        let (status, cache) = cache_control("/assets/App-Dx-3l_Gc.js").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache.as_deref(), Some(IMMUTABLE));
        let (_, cache) = cache_control("/assets/mermaid.core-B7Hx2kzQ.js").await;
        assert_eq!(cache.as_deref(), Some(IMMUTABLE));
        // The document names the current hashes, directly or via fallback.
        for path in ["/", "/index.html", "/git/repo?x=1"] {
            let (status, cache) = cache_control(path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(cache.as_deref(), Some(REVALIDATE), "{path}");
        }
        // Unhashed files keep default caching, never a year-long pin.
        for path in ["/assets/main.js", "/sw.js"] {
            let (_, cache) = cache_control(path).await;
            assert_eq!(cache, None, "{path}");
        }
    }

    #[tokio::test]
    async fn spa_responses_carry_csp_and_anti_framing() {
        for path in [
            "/",
            "/rooms/x",
            "/assets/main.js",
            "/assets/gone-AbCdEfGh.js",
        ] {
            let app = Router::new()
                .fallback(move |uri: Uri| spa_fallback_with_assets(uri, Some(fixture)));
            let response = app
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let h = response.headers();
            let csp = h[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
            assert!(csp.contains("frame-ancestors 'self'"), "{path}: {csp}");
            assert!(csp.contains("object-src 'none'"), "{path}");
            assert!(!csp.contains("ipc:"), "tauri-only source leaked: {csp}");
            assert!(!csp.contains("  "), "no stray whitespace from line joins");
            assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff", "{path}");
            assert_eq!(h[header::X_FRAME_OPTIONS], "SAMEORIGIN", "{path}");
        }
    }

    #[test]
    fn hashed_asset_names_follow_vites_pattern() {
        for yes in [
            "assets/index-B7Hx2kzQ.js",
            "assets/index-DT-3lsGc.css",
            "assets/d2-a_b-c_d-.wasm",
            "assets/mermaid.core-AbCdEfGh.js",
            "assets/excalidraw/fonts/Excalifont/Excalifont-Regular-349fac6ca4700ffec595a7150a0d1e1d.woff2",
        ] {
            assert!(is_hashed_asset(yes), "{yes}");
        }
        for no in [
            "index.html",
            "sw.js",
            "assets/main.js",
            "assets/-AbCdEfGh.js",
            "assets/app-short.js",
            "assets/app-AbCdEfG!.js",
            "assets/sub/app-AbCdEfGh.js",
            "assets/app-AbCdEfGh",
            "assets/excalidraw/fonts/Excalifont/Excalifont-Regular.woff2",
            "assets/excalidraw/fonts/Excalifont/Excalifont-Regular-349fac6c.woff2",
            "assets/excalidraw/fonts/../x-349fac6ca4700ffec595a7150a0d1e1d.woff2",
            "assets/excalidraw/fonts/A/b/c-349fac6ca4700ffec595a7150a0d1e1d.woff2",
        ] {
            assert!(!is_hashed_asset(no), "{no}");
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
