//! Content-Length framing helpers for the LSP stdio transport.
//!
//! LSP messages on stdin/stdout use HTTP-style headers:
//!   `Content-Length: <n>\r\n\r\n<n bytes of JSON>`
//!
//! `write_message` frames a JSON body and writes it.
//! `read_message`  reads exactly one framed message, returning the raw JSON
//! body bytes.

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Write one Content-Length-framed message to `writer`.
pub async fn write_message<W>(writer: &mut W, body: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(body).await?;
    writer.flush().await
}

/// Read one Content-Length-framed message from `reader`.
///
/// Returns `None` on clean EOF (the server exited).
/// Returns an error on malformed framing.
pub async fn read_message<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    // Read headers until the blank line (\r\n\r\n separator).
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            // EOF
            return Ok(None);
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // Blank line — end of headers.
            break;
        }
        // Parse Content-Length header (case-insensitive prefix match).
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            let val: usize = rest.trim().parse().map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid Content-Length value: '{}'", rest.trim()),
                )
            })?;
            content_length = Some(val);
        }
        // Other headers (e.g. Content-Type) are silently ignored.
    }

    let len = content_length.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "LSP message missing Content-Length header",
        )
    })?;

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(Some(buf))
}
