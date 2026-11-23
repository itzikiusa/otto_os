//! Bounded payload helpers for workflow nodes (`http_request`, `db_query`).

use otto_core::{Error, Result};

/// Default response body cap for `http_request` workflow nodes (4 MiB).
pub const HTTP_RESPONSE_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Maximum row limit for `db_query` workflow nodes.
pub const DB_QUERY_MAX_ROWS: u64 = 5_000;

/// Clamp a `db_query` node's `max_rows` parameter to `[1, DB_QUERY_MAX_ROWS]`
/// (defaulting to `100` when omitted).
pub fn clamp_db_max_rows(raw: Option<u64>) -> usize {
    raw.unwrap_or(100).clamp(1, DB_QUERY_MAX_ROWS) as usize
}

/// Stream a `reqwest::Response` body into UTF-8 text while enforcing a hard
/// byte cap (`max_bytes`). Rejects responses that declare or stream more than
/// `max_bytes` bytes with [`Error::Upstream`], preventing unbounded buffering.
pub async fn read_text_capped(mut resp: reqwest::Response, max_bytes: usize) -> Result<String> {
    if let Some(len) = resp.content_length() {
        if len > max_bytes as u64 {
            return Err(Error::Upstream(format!(
                "http: response body exceeds {max_bytes}-byte limit ({len} bytes)"
            )));
        }
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| Error::Upstream(format!("http: read response body: {e}")))?
    {
        if buf.len().saturating_add(chunk.len()) > max_bytes {
            return Err(Error::Upstream(format!(
                "http: response body exceeds {max_bytes}-byte limit"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn spawn_one_shot_http(raw_response: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut req_buf = [0u8; 1024];
                let _ = sock.read(&mut req_buf).await;
                let _ = sock.write_all(raw_response).await;
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}/")
    }

    #[test]
    fn db_max_rows_is_clamped_to_safe_bounds() {
        assert_eq!(clamp_db_max_rows(None), 100);
        assert_eq!(clamp_db_max_rows(Some(0)), 1);
        assert_eq!(clamp_db_max_rows(Some(250)), 250);
        assert_eq!(clamp_db_max_rows(Some(5_000)), 5_000);
        assert_eq!(clamp_db_max_rows(Some(100_000)), 5_000);
        assert_eq!(clamp_db_max_rows(Some(u64::MAX)), 5_000);
    }

    #[tokio::test]
    async fn read_text_capped_allows_bodies_within_limit_and_rejects_oversized() {
        let client = reqwest::Client::new();

        // Exact cap passes.
        let url_ok = spawn_one_shot_http(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
        )
        .await;
        let resp = client.get(&url_ok).send().await.unwrap();
        assert_eq!(read_text_capped(resp, 5).await.unwrap(), "hello");

        // Declared Content-Length over cap is rejected.
        let url_cl = spawn_one_shot_http(
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789",
        )
        .await;
        let resp = client.get(&url_cl).send().await.unwrap();
        let err = read_text_capped(resp, 5).await.unwrap_err();
        assert!(matches!(err, Error::Upstream(ref m) if m.contains("exceeds 5-byte limit")));

        // Chunked response (no Content-Length) over cap is rejected while streaming.
        let url_chunked = spawn_one_shot_http(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n4\r\nefgh\r\n0\r\n\r\n",
        )
        .await;
        let resp = client.get(&url_chunked).send().await.unwrap();
        let err = read_text_capped(resp, 5).await.unwrap_err();
        assert!(matches!(err, Error::Upstream(ref m) if m.contains("exceeds 5-byte limit")));
    }
}
