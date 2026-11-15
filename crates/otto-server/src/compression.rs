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
        return Response::from_parts(parts, Body::empty());
    };
    if (raw.len() as u64) < MIN_COMPRESS_BYTES {
        return Response::from_parts(parts, Body::from(raw));
    }

    let mut encoder = GzEncoder::new(Vec::with_capacity(raw.len() / 4), Compression::fast());
    if encoder.write_all(&raw).is_err() {
        return Response::from_parts(parts, Body::from(raw));
    }
    let Ok(compressed) = encoder.finish() else {
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
