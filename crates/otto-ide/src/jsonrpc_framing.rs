//! Shared `Content-Length` wire framing for LSP and DAP JSON-RPC transports,
//! plus shared hex SHA-256 hashing.

use std::io::{self, ErrorKind};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Maximum total byte length of the HTTP-like header block preceding `\r\n\r\n`.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Maximum JSON-RPC payload size (16 MiB).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Largest over-cap body [`read_frame_or_skip`] streams past (never buffers)
/// instead of ending the stream; anything bigger is treated as a broken peer.
pub const MAX_SKIPPABLE_BYTES: usize = 1024 * 1024 * 1024;

/// Bytes kept from the start of a skipped body, enough to find its request id.
pub const SKIPPED_HEAD_BYTES: usize = 4096;

/// One frame from [`read_frame_or_skip`].
#[derive(Debug, PartialEq, Eq)]
pub enum Frame {
    Body(Vec<u8>),
    /// A body over [`MAX_MESSAGE_BYTES`] that was read and discarded; `head` is
    /// its first [`SKIPPED_HEAD_BYTES`] bytes.
    Oversized {
        len: usize,
        head: Vec<u8>,
    },
}

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
    let Some(len) = read_frame_header(reader, MAX_MESSAGE_BYTES).await? else {
        return Ok(None);
    };
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// Like [`read_frame_bytes`], but a body over [`MAX_MESSAGE_BYTES`] (up to
/// [`MAX_SKIPPABLE_BYTES`]) is streamed past and reported as
/// [`Frame::Oversized`] so the caller can fail that one request and keep the
/// session — one huge `variables` reply must not kill a debug session, nor one
/// huge LSP response the shared server for every editor.
pub async fn read_frame_or_skip<R>(reader: &mut R) -> io::Result<Option<Frame>>
where
    R: AsyncBufRead + Unpin,
{
    let Some(len) = read_frame_header(reader, MAX_SKIPPABLE_BYTES).await? else {
        return Ok(None);
    };
    if len <= MAX_MESSAGE_BYTES {
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).await?;
        return Ok(Some(Frame::Body(body)));
    }
    let mut head = vec![0u8; SKIPPED_HEAD_BYTES.min(len)];
    reader.read_exact(&mut head).await?;
    let rest = (len - head.len()) as u64;
    let skipped = tokio::io::copy(&mut (&mut *reader).take(rest), &mut tokio::io::sink()).await?;
    if skipped < rest {
        return Err(io::Error::new(
            ErrorKind::UnexpectedEof,
            "truncated oversized JSON-RPC body",
        ));
    }
    Ok(Some(Frame::Oversized { len, head }))
}

/// Parse a frame's headers and return its `Content-Length`, rejecting one
/// over `max_len` before any of the body is read.
async fn read_frame_header<R>(reader: &mut R, max_len: usize) -> io::Result<Option<usize>>
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
            if len > max_len {
                let cap = if max_len == MAX_MESSAGE_BYTES {
                    "16 MiB".to_string()
                } else {
                    format!("{max_len}-byte")
                };
                return Err(invalid_data(format!(
                    "JSON-RPC message length ({len} bytes) exceeds {cap} cap"
                )));
            }
            content_length = Some(len);
        }
    }

    content_length
        .map(Some)
        .ok_or_else(|| invalid_data("JSON-RPC frame missing Content-Length header"))
}

/// Compute lowercase hex SHA-256 digest of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod skip_tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn oversized_body_is_skipped_and_the_next_frame_still_reads() {
        let len = MAX_MESSAGE_BYTES + 10;
        let mut input = format!("Content-Length: {len}\r\n\r\n").into_bytes();
        input.extend_from_slice(br#"{"request_seq":7,"type":"response""#);
        input.resize(input.len() + len - 34, b' ');
        input.extend_from_slice(b"Content-Length: 2\r\n\r\n{}");
        let mut reader = BufReader::new(&input[..]);
        match read_frame_or_skip(&mut reader).await.unwrap() {
            Some(Frame::Oversized { len: got, head }) => {
                assert_eq!(got, len);
                assert_eq!(head.len(), SKIPPED_HEAD_BYTES);
                assert!(head.starts_with(br#"{"request_seq":7"#));
            }
            other => panic!("expected an oversized frame, got {other:?}"),
        }
        assert_eq!(
            read_frame_or_skip(&mut reader).await.unwrap(),
            Some(Frame::Body(b"{}".to_vec()))
        );
        assert_eq!(read_frame_or_skip(&mut reader).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_body_past_the_skippable_cap_still_errors() {
        let input = format!("Content-Length: {}\r\n\r\n", MAX_SKIPPABLE_BYTES + 1);
        let err = read_frame_or_skip(&mut BufReader::new(input.as_bytes()))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }
}
