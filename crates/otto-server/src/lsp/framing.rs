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
    const MAX_HEADER_BYTES: usize = 8 * 1024;
    const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;
    let invalid = |message| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0;
    loop {
        let mut line = Vec::new();
        // Bound each read before allocating: read_line can grow without limit
        // when a broken language server emits an unterminated header.
        let n = reader
            .take((MAX_HEADER_BYTES - header_bytes + 1) as u64)
            .read_until(b'\n', &mut line)
            .await?;
        if n == 0 {
            return if header_bytes == 0 {
                Ok(None)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "truncated LSP headers",
                ))
            };
        }
        header_bytes += n;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(invalid("LSP headers exceed 8 KiB"));
        }
        if !line.ends_with(b"\n") {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "truncated LSP header line",
            ));
        }
        let line =
            std::str::from_utf8(&line).map_err(|_| invalid("invalid LSP header encoding"))?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.eq_ignore_ascii_case("Content-Length") {
                if content_length.is_some() {
                    return Err(invalid("duplicate LSP Content-Length"));
                }
                let value = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| invalid("invalid LSP Content-Length"))?;
                if value > MAX_BODY_BYTES {
                    return Err(invalid("LSP message exceeds 16 MiB"));
                }
                content_length = Some(value);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn rejects_oversized_body_before_reading_or_allocating_it() {
        let input = b"Content-Length: 16777217\r\n\r\n";
        let err = read_message(&mut BufReader::new(&input[..]))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn bounds_header_line_and_total_headers() {
        for input in [
            format!("X-Extra: {}\r\nContent-Length: 0\r\n\r\n", "x".repeat(8192)),
            format!("{}Content-Length: 0\r\n\r\n", "X: value\r\n".repeat(1000)),
        ] {
            let err = read_message(&mut BufReader::new(input.as_bytes()))
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidData);
        }
    }

    #[tokio::test]
    async fn rejects_duplicate_length_and_truncated_headers() {
        let duplicate = b"Content-Length: 0\r\nContent-Length: 1\r\n\r\nx";
        assert_eq!(
            read_message(&mut BufReader::new(&duplicate[..]))
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidData
        );
        let incomplete = b"Content-Length: 4\r\n";
        assert_eq!(
            read_message(&mut BufReader::new(&incomplete[..]))
                .await
                .unwrap_err()
                .kind(),
            ErrorKind::UnexpectedEof
        );
    }

    #[tokio::test]
    async fn accepts_fragmented_frames_empty_body_and_clean_eof() {
        let input = b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: 2\r\n\r\n{}Content-Length: 0\r\n\r\n";
        let mut reader = BufReader::with_capacity(1, &input[..]);
        assert_eq!(
            read_message(&mut reader).await.unwrap(),
            Some(b"{}".to_vec())
        );
        assert_eq!(read_message(&mut reader).await.unwrap(), Some(vec![]));
        assert_eq!(read_message(&mut reader).await.unwrap(), None);
    }
}
