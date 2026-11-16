//! Shared `Content-Length` wire framing for LSP and DAP JSON-RPC transports,
//! plus shared hex SHA-256 hashing.

use std::io::{self, ErrorKind};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum total byte length of the HTTP-like header block preceding `\r\n\r\n`.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Maximum JSON-RPC payload size (16 MiB).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

fn invalid_data(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, msg.into())
}

/// Write a single `Content-Length` framed payload to `writer`, enforcing the
/// 16 MiB outgoing cap.
pub async fn write_frame_bytes<W>(writer: &mut W, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(invalid_data(format!(
            "JSON-RPC outgoing payload ({} bytes) exceeds 16 MiB cap",
            payload.len()
        )));
    }
    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

/// Read a single `Content-Length` framed byte payload from `reader`.
///
/// Returns `Ok(None)` on clean EOF before a frame begins. Tolerates stray
/// leading blank lines up to [`MAX_HEADER_BYTES`], trims header names, and
/// rejects headers > 8 KiB or bodies > 16 MiB before allocating the body.
pub async fn read_frame_bytes<R>(reader: &mut R) -> io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0usize;
    let mut leading_blank_bytes = 0usize;

    loop {
        let mut line = Vec::new();
        let remaining_cap = (MAX_HEADER_BYTES - header_bytes + 1) as u64;
        let n = reader
            .take(remaining_cap)
            .read_until(b'\n', &mut line)
            .await?;

        if n == 0 {
            return if header_bytes == 0 {
                Ok(None)
            } else {
                Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "truncated JSON-RPC headers before empty line",
                ))
            };
        }

        header_bytes += n;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(invalid_data("JSON-RPC headers exceed 8 KiB cap"));
        }
        if !line.ends_with(b"\n") {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "truncated JSON-RPC header line",
            ));
        }

        let line_str = std::str::from_utf8(&line)
            .map_err(|_| invalid_data("non-UTF-8 JSON-RPC header line"))?;
        let trimmed = line_str.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if content_length.is_none() && header_bytes == n {
                leading_blank_bytes += n;
                if leading_blank_bytes > MAX_HEADER_BYTES {
                    return Err(invalid_data(
                        "JSON-RPC leading blank lines exceed 8 KiB cap",
                    ));
                }
                header_bytes = 0;
                continue;
            }
            break;
        }

        let (name, value) = trimmed
            .split_once(':')
            .ok_or_else(|| invalid_data(format!("malformed JSON-RPC header line: {trimmed}")))?;

        if name.trim().eq_ignore_ascii_case("Content-Length") {
            if content_length.is_some() {
                return Err(invalid_data("duplicate JSON-RPC Content-Length header"));
            }
            let len = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid_data("invalid JSON-RPC Content-Length integer"))?;
            if len > MAX_MESSAGE_BYTES {
                return Err(invalid_data(format!(
                    "JSON-RPC message length ({len} bytes) exceeds 16 MiB cap"
                )));
            }
            content_length = Some(len);
        }
    }

    let len = content_length
        .ok_or_else(|| invalid_data("JSON-RPC frame missing Content-Length header"))?;

    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Compute lowercase hex SHA-256 digest of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
