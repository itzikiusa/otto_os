//! Content-Length framing helpers for the LSP stdio transport.
//!
//! LSP messages on stdin/stdout use HTTP-style headers:
//!   `Content-Length: <n>\r\n\r\n<n bytes of JSON>`
//!
//! `write_message` frames a JSON body and writes it.
//! `read_message`  reads exactly one framed message, returning the raw JSON
//! body bytes.

use tokio::io::{AsyncBufRead, AsyncWrite};

/// Write one Content-Length-framed message to `writer`.
pub async fn write_message<W>(writer: &mut W, body: &[u8]) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    crate::jsonrpc_framing::write_frame_bytes(writer, body).await
}

/// Read one Content-Length-framed message from `reader`.
///
/// Returns `None` on clean EOF (the server exited).
/// Returns an error on malformed framing.
pub async fn read_message<R>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>>
where
    R: AsyncBufRead + Unpin,
{
    crate::jsonrpc_framing::read_frame_bytes(reader).await
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
