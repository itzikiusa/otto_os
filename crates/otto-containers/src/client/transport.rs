//! Unix-socket HTTP/1.1 transport, bounded body reads, and NDJSON streaming.

use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1;
use hyper::Method;
use hyper_util::rt::TokioIo;
use otto_core::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::net::UnixStream;

use super::{DockerClient, BACKGROUND_HEADER, KIRI_BACKGROUND_HEADER};

pub(super) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
pub(super) const PULL_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) const MAX_JSON_BODY: usize = 16 * 1024 * 1024;
pub(super) const MAX_LOGS_BODY: usize = 4 * 1024 * 1024;
const MAX_NDJSON_LINE_BYTES: usize = 256 * 1024;
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

/// Raw HTTP response collected from the Docker daemon Unix socket.
#[derive(Debug)]
pub(super) struct RawResp {
    pub status: u16,
    pub body: Vec<u8>,
    pub truncated: bool,
}

impl DockerClient {
    async fn send_unix_request(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Vec<u8>>,
        background: bool,
    ) -> Result<hyper::Response<hyper::body::Incoming>> {
        let stream = UnixStream::connect(&self.socket_path).await.map_err(|e| {
            Error::Upstream(format!(
                "Docker daemon unreachable at {}: {e}",
                self.socket_path.display()
            ))
        })?;
        let (mut sender, conn) = http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|e| Error::Upstream(format!("Docker socket HTTP handshake failed: {e}")))?;
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let has_body = body.is_some();
        let mut builder = hyper::Request::builder()
            .method(method)
            .uri(path_and_query)
            .header(hyper::header::HOST, "localhost");
        if background {
            builder = builder
                .header(BACKGROUND_HEADER, "1")
                .header(KIRI_BACKGROUND_HEADER, "1");
        }
        if has_body {
            builder = builder.header(hyper::header::CONTENT_TYPE, "application/json");
        }
        let req = builder
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|e| Error::Invalid(format!("build Docker request: {e}")))?;

        sender
            .send_request(req)
            .await
            .map_err(|e| Error::Upstream(format!("Docker daemon request failed: {e}")))
    }

    pub(super) async fn request(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Vec<u8>>,
        background: bool,
        timeout: Duration,
        max_body: usize,
    ) -> Result<RawResp> {
        tokio::time::timeout(
            timeout,
            self.request_inner(method, path_and_query, body, background, max_body),
        )
        .await
        .map_err(|_| {
            Error::Upstream(format!(
                "Docker daemon request timed out after {}s ({})",
                timeout.as_secs(),
                self.socket_path.display()
            ))
        })?
    }

    async fn request_inner(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<Vec<u8>>,
        background: bool,
        max_body: usize,
    ) -> Result<RawResp> {
        let resp = self
            .send_unix_request(method, path_and_query, body, background)
            .await?;
        let status = resp.status().as_u16();
        let mut incoming = resp.into_body();
        let mut buf = Vec::new();
        let mut truncated = false;
        while let Some(frame) = incoming.frame().await {
            let frame =
                frame.map_err(|e| Error::Upstream(format!("read Docker response body: {e}")))?;
            if let Some(chunk) = frame.data_ref() {
                let remaining = max_body.saturating_sub(buf.len());
                if chunk.len() > remaining {
                    buf.extend_from_slice(&chunk[..remaining]);
                    truncated = true;
                    break;
                }
                buf.extend_from_slice(chunk);
            }
        }
        Ok(RawResp {
            status,
            body: buf,
            truncated,
        })
    }

    /// Stream an NDJSON response line-by-line with bounded per-line memory so
    /// large multi-layer image pulls never fail or falsely succeed on truncation.
    pub(super) async fn stream_ndjson<F>(
        &self,
        method: Method,
        path_and_query: &str,
        background: bool,
        timeout: Duration,
        mut on_line: F,
    ) -> Result<()>
    where
        F: FnMut(&str) -> Result<()>,
    {
        tokio::time::timeout(timeout, async move {
            let resp = self
                .send_unix_request(method, path_and_query, None, background)
                .await?;
            let status = resp.status().as_u16();
            let mut incoming = resp.into_body();

            if !(200..300).contains(&status) {
                let mut err_buf = Vec::new();
                while let Some(frame) = incoming.frame().await {
                    let frame = frame
                        .map_err(|e| Error::Upstream(format!("read Docker error body: {e}")))?;
                    if let Some(chunk) = frame.data_ref() {
                        let remaining = MAX_ERROR_BODY_BYTES.saturating_sub(err_buf.len());
                        err_buf.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                        if err_buf.len() >= MAX_ERROR_BODY_BYTES {
                            break;
                        }
                    }
                }
                return check_status(&RawResp {
                    status,
                    body: err_buf,
                    truncated: false,
                });
            }

            let mut line_buf: Vec<u8> = Vec::new();
            while let Some(frame) = incoming.frame().await {
                let frame = frame
                    .map_err(|e| Error::Upstream(format!("read Docker NDJSON stream: {e}")))?;
                let Some(chunk) = frame.data_ref() else {
                    continue;
                };
                let mut slice = chunk.as_ref();
                while let Some(nl) = slice.iter().position(|&b| b == b'\n') {
                    let piece = &slice[..nl];
                    if line_buf.len().saturating_add(piece.len()) > MAX_NDJSON_LINE_BYTES {
                        return Err(Error::Upstream(
                            "Docker NDJSON progress line exceeded 256 KiB safety limit".into(),
                        ));
                    }
                    line_buf.extend_from_slice(piece);
                    let line = String::from_utf8_lossy(&line_buf);
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        on_line(trimmed)?;
                    }
                    line_buf.clear();
                    slice = &slice[nl + 1..];
                }
                if !slice.is_empty() {
                    if line_buf.len().saturating_add(slice.len()) > MAX_NDJSON_LINE_BYTES {
                        return Err(Error::Upstream(
                            "Docker NDJSON progress line exceeded 256 KiB safety limit".into(),
                        ));
                    }
                    line_buf.extend_from_slice(slice);
                }
            }
            if !line_buf.is_empty() {
                let line = String::from_utf8_lossy(&line_buf);
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    on_line(trimmed)?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|_| {
            Error::Upstream(format!(
                "Docker daemon request timed out after {}s ({})",
                timeout.as_secs(),
                self.socket_path.display()
            ))
        })?
    }

    pub(super) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        background: bool,
        timeout: Duration,
    ) -> Result<T> {
        let resp = self
            .request(Method::GET, path, None, background, timeout, MAX_JSON_BODY)
            .await?;
        check_status(&resp)?;
        serde_json::from_slice(&resp.body)
            .map_err(|e| Error::Upstream(format!("parse Docker JSON from {path}: {e}")))
    }
}

pub(super) fn check_status(resp: &RawResp) -> Result<()> {
    if (200..300).contains(&resp.status) {
        return Ok(());
    }
    let msg = extract_docker_error(&resp.body)
        .unwrap_or_else(|| format!("Docker daemon returned HTTP {}", resp.status));
    match resp.status {
        400 => Err(Error::Invalid(msg)),
        404 => Err(Error::NotFound(msg)),
        409 => Err(Error::Conflict(msg)),
        _ => Err(Error::Upstream(msg)),
    }
}

fn extract_docker_error(body: &[u8]) -> Option<String> {
    if let Ok(v) = serde_json::from_slice::<Value>(body) {
        if let Some(msg) = v.get("message").and_then(|m| m.as_str()) {
            let trimmed = msg.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    let s = String::from_utf8_lossy(body);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.chars().take(400).collect())
    }
}

/// RFC 3986 percent-encoding for URL query/path components (zero extra deps).
pub fn encode_query_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(*b, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(*b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(&mut out, "%{b:02X}");
        }
    }
    out
}
