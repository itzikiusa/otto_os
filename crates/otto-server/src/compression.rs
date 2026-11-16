//! Fast gzip compression middleware for bounded JSON, HTML, JS, CSS, and SVG
//! HTTP responses (`>= 1 KiB`), leaving SSE (`text/event-stream`), WebSockets,
//! and binary responses untouched.

use std::io::Write;

use axum::body::{to_bytes, Body, HttpBody};
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use flate2::write::GzEncoder;
use flate2::Compression;

const MIN_COMPRESS_BYTES: u64 = 1_024;
const BLOCKING_COMPRESS_BYTES: usize = 64 * 1024;
const MAX_COMPRESS_BYTES: usize = 32 * 1024 * 1024;

fn accepts_gzip(req: &Request) -> bool {
    req.headers()
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(','))
        .any(|part| {
            let mut segs = part.trim().split(';');
            let Some(coding) = segs.next() else {
                return false;
            };
            if !coding.trim().eq_ignore_ascii_case("gzip") {
                return false;
            }
            !segs.any(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            })
        })
}

fn is_compressible_mime(ct: &str) -> bool {
    let media = ct.split(';').next().unwrap_or("").trim();
    media.eq_ignore_ascii_case("application/json")
        || media.eq_ignore_ascii_case("application/problem+json")
        || media.eq_ignore_ascii_case("application/javascript")
        || media.eq_ignore_ascii_case("text/javascript")
        || media.eq_ignore_ascii_case("text/html")
        || media.eq_ignore_ascii_case("text/css")
        || media.eq_ignore_ascii_case("text/plain")
        || media.eq_ignore_ascii_case("image/svg+xml")
}

pub async fn gzip_responses(req: Request, next: Next) -> Response {
    let can_gzip = req.method() != Method::HEAD && accepts_gzip(&req);
    let res = next.run(req).await;
    if !can_gzip {
        return res;
    }
    let status = res.status();
    if status == StatusCode::SWITCHING_PROTOCOLS
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED
        || res.headers().contains_key(header::CONTENT_ENCODING)
    {
        return res;
    }
    let compressible = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(is_compressible_mime);
    if !compressible {
        return res;
    }
    let Some(upper) = res.body().size_hint().upper() else {
        return res;
    };
    if upper < MIN_COMPRESS_BYTES || upper > MAX_COMPRESS_BYTES as u64 {
        return res;
    }

    let (mut parts, body) = res.into_parts();
    let Ok(raw) = to_bytes(body, MAX_COMPRESS_BYTES).await else {
        parts.headers.remove(header::CONTENT_LENGTH);
        parts.headers.remove(header::CONTENT_ENCODING);
        parts.status = StatusCode::PAYLOAD_TOO_LARGE;
        return Response::from_parts(parts, Body::empty());
    };
    if (raw.len() as u64) < MIN_COMPRESS_BYTES {
        return Response::from_parts(parts, Body::from(raw));
    }

    let (raw, compressed) = if raw.len() >= BLOCKING_COMPRESS_BYTES {
        let raw_for_task = raw.clone();
        match tokio::task::spawn_blocking(move || gzip_bytes(&raw_for_task)).await {
            Ok(Some(c)) => (raw, Some(c)),
            _ => (raw, None),
        }
    } else {
        let c = gzip_bytes(&raw);
        (raw, c)
    };
    let Some(compressed) = compressed else {
        return Response::from_parts(parts, Body::from(raw));
    };
    if compressed.len() >= raw.len() {
        return Response::from_parts(parts, Body::from(raw));
    }

    parts
        .headers
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    if let Ok(len_val) = HeaderValue::from_str(&compressed.len().to_string()) {
        parts.headers.insert(header::CONTENT_LENGTH, len_val);
    }
    parts
        .headers
        .append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    Response::from_parts(parts, Body::from(compressed))
}

fn gzip_bytes(raw: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(raw.len() / 4), Compression::fast());
    encoder.write_all(raw).ok()?;
    encoder.finish().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use axum::Router;
    use flate2::read::GzDecoder;
    use std::io::Read;
    use tower::ServiceExt;

    #[tokio::test]
    async fn compresses_small_and_large_json_and_leaves_tiny_uncompressed() {
        let app = Router::new()
            .route(
                "/tiny",
                get(|| async { ([("content-type", "application/json")], "{\"ok\":true}") }),
            )
            .route(
                "/medium",
                get(|| async {
                    let body = format!("{{\"items\":\"{}\"}}", "abcd".repeat(1_024));
                    ([("content-type", "application/json")], body)
                }),
            )
            .route(
                "/large",
                get(|| async {
                    let body = format!("{{\"items\":\"{}\"}}", "abcdefgh".repeat(16_384));
                    ([("content-type", "application/json")], body)
                }),
            )
            .layer(axum::middleware::from_fn(gzip_responses));

        let tiny = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/tiny")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(tiny.headers().get(header::CONTENT_ENCODING).is_none());

        for uri in ["/medium", "/large"] {
            let res = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header(header::ACCEPT_ENCODING, "gzip")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(res.status(), StatusCode::OK);
            assert_eq!(
                res.headers()
                    .get(header::CONTENT_ENCODING)
                    .and_then(|v| v.to_str().ok()),
                Some("gzip")
            );
            let bytes = to_bytes(res.into_body(), MAX_COMPRESS_BYTES).await.unwrap();
            let mut decoded = String::new();
            GzDecoder::new(&bytes[..])
                .read_to_string(&mut decoded)
                .unwrap();
            assert!(decoded.starts_with("{\"items\":\""));
        }
    }

    #[tokio::test]
    async fn body_read_failure_clears_content_length_and_sets_error_status() {
        let app = Router::new()
            .route(
                "/broken",
                get(|| async {
                    let failing = http_body_util::Limited::new(
                        http_body_util::Full::new(axum::body::Bytes::from(vec![b'x'; 4_096])),
                        1_024,
                    );
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::CONTENT_LENGTH, "4096")
                        .body(Body::new(failing))
                        .unwrap()
                }),
            )
            .layer(axum::middleware::from_fn(gzip_responses));

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/broken")
                    .header(header::ACCEPT_ENCODING, "gzip")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(res
            .headers()
            .get(header::CONTENT_LENGTH)
            .is_none_or(|v| v == "0"));
        assert!(res.headers().get(header::CONTENT_ENCODING).is_none());
    }
}
