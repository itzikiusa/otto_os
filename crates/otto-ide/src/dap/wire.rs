//! Hardened Debug Adapter Protocol (DAP) `Content-Length` wire framing.
//!
//! DAP frames UTF-8 JSON messages (`request`, `response`, `event`) using
//! HTTP-style headers on a byte stream (stdio or TCP):
//!
//! ```text
//! Content-Length: <N>\r\n
//! \r\n
//! <N bytes of UTF-8 JSON>
//! ```
//!
//! Security & reliability invariants:
//! - Header blocks are bounded to [`MAX_HEADER_BYTES`] (8 KiB) prior to line
//!   allocation so an unterminated line from a misbehaving child cannot OOM the
//!   daemon.
//! - Payloads exceeding [`MAX_MESSAGE_BYTES`] (16 MiB) are rejected *before*
//!   allocating the body buffer.
//! - Partial/fragmented reads across arbitrary pipe or TCP chunk boundaries are
//!   reassembled deterministically.
//! - Every frame is validated as UTF-8 and typed into [`DapMessage`].

use std::io::{self, ErrorKind};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};

/// Maximum total byte length of the HTTP-like header block preceding `\r\n\r\n`.
pub const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Maximum DAP JSON payload size (16 MiB).
pub const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// Top-level Debug Adapter Protocol message envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum DapMessage {
    Request(DapRequest),
    Response(DapResponse),
    Event(DapEvent),
}

impl DapMessage {
    /// Sequence number attached to the protocol message.
    pub fn seq(&self) -> i64 {
        match self {
            DapMessage::Request(r) => r.seq,
            DapMessage::Response(r) => r.seq,
            DapMessage::Event(e) => e.seq,
        }
    }
}

/// Client-to-adapter (or reverse adapter-to-client) DAP request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DapRequest {
    #[serde(default)]
    pub seq: i64,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

impl DapRequest {
    pub fn new(seq: i64, command: impl Into<String>, arguments: Option<Value>) -> Self {
        Self {
            seq,
            command: command.into(),
            arguments,
        }
    }
}

/// DAP response correlated to an earlier request via `request_seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DapResponse {
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub request_seq: i64,
    pub success: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl DapResponse {
    pub fn ok(seq: i64, request_seq: i64, command: impl Into<String>, body: Option<Value>) -> Self {
        Self {
            seq,
            request_seq,
            success: true,
            command: command.into(),
            message: None,
            body,
        }
    }

    pub fn err(
        seq: i64,
        request_seq: i64,
        command: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            request_seq,
            success: false,
            command: command.into(),
            message: Some(message.into()),
            body: None,
        }
    }
}

/// Asynchronous notification emitted by the debug adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DapEvent {
    #[serde(default)]
    pub seq: i64,
    pub event: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

impl DapEvent {
    pub fn new(seq: i64, event: impl Into<String>, body: Option<Value>) -> Self {
        Self {
            seq,
            event: event.into(),
            body,
        }
    }
}

fn invalid_data(msg: impl Into<String>) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, msg.into())
}

/// Write a single `Content-Length` framed UTF-8 JSON payload to `writer`.
pub async fn write_raw_frame<W>(writer: &mut W, payload: &[u8]) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(invalid_data(format!(
            "DAP outgoing payload ({} bytes) exceeds 16 MiB cap",
            payload.len()
        )));
    }
    std::str::from_utf8(payload)
        .map_err(|_| invalid_data("DAP outgoing payload is not valid UTF-8"))?;

    let header = format!("Content-Length: {}\r\n\r\n", payload.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await
}

/// Read a single `Content-Length` framed UTF-8 payload from `reader`.
///
/// Returns `Ok(None)` on clean EOF before a new header begins.
pub async fn read_raw_frame<R>(reader: &mut R) -> io::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0usize;

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
                    "truncated DAP headers before empty line",
                ))
            };
        }

        header_bytes += n;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(invalid_data("DAP headers exceed 8 KiB cap"));
        }
        if !line.ends_with(b"\n") {
            return Err(io::Error::new(
                ErrorKind::UnexpectedEof,
                "truncated DAP header line",
            ));
        }

        let line_str =
            std::str::from_utf8(&line).map_err(|_| invalid_data("non-UTF-8 DAP header line"))?;
        let trimmed = line_str.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            // Ignore stray leading blank lines before `Content-Length:` if no
            // headers have been seen yet; otherwise `\r\n\r\n` terminates headers.
            if content_length.is_none() && header_bytes == n {
                header_bytes = 0;
                continue;
            }
            break;
        }

        let (name, value) = trimmed
            .split_once(':')
            .ok_or_else(|| invalid_data(format!("malformed DAP header line: {trimmed}")))?;

        if name.trim().eq_ignore_ascii_case("Content-Length") {
            if content_length.is_some() {
                return Err(invalid_data("duplicate DAP Content-Length header"));
            }
            let len = value
                .trim()
                .parse::<usize>()
                .map_err(|_| invalid_data("invalid DAP Content-Length integer"))?;
            if len > MAX_MESSAGE_BYTES {
                return Err(invalid_data(format!(
                    "DAP message length ({len} bytes) exceeds 16 MiB cap"
                )));
            }
            content_length = Some(len);
        }
    }

    let len =
        content_length.ok_or_else(|| invalid_data("DAP frame missing Content-Length header"))?;

    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;

    String::from_utf8(body)
        .map(Some)
        .map_err(|_| invalid_data("DAP message body contains invalid UTF-8"))
}

/// Serialize and write a typed [`DapMessage`] to `writer`.
pub async fn write_dap_message<W>(writer: &mut W, msg: &DapMessage) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let json =
        serde_json::to_vec(msg).map_err(|e| invalid_data(format!("serialize DAP message: {e}")))?;
    write_raw_frame(writer, &json).await
}

/// Read and deserialize a typed [`DapMessage`] from `reader`.
pub async fn read_dap_message<R>(reader: &mut R) -> io::Result<Option<DapMessage>>
where
    R: AsyncBufRead + Unpin,
{
    let Some(raw) = read_raw_frame(reader).await? else {
        return Ok(None);
    };
    let msg: DapMessage = serde_json::from_str(&raw)
        .map_err(|e| invalid_data(format!("invalid DAP JSON message: {e}")))?;
    Ok(Some(msg))
}

/// Buffered async DAP wire reader over any [`AsyncRead`].
pub struct DapReader<R> {
    inner: BufReader<R>,
}

impl<R: AsyncRead + Unpin> DapReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner: BufReader::new(inner),
        }
    }

    pub fn with_capacity(capacity: usize, inner: R) -> Self {
        Self {
            inner: BufReader::with_capacity(capacity, inner),
        }
    }

    pub async fn next_message(&mut self) -> io::Result<Option<DapMessage>> {
        read_dap_message(&mut self.inner).await
    }

    pub async fn next_raw_frame(&mut self) -> io::Result<Option<String>> {
        read_raw_frame(&mut self.inner).await
    }
}

/// Monotonically sequenced async DAP wire writer over any [`AsyncWrite`].
pub struct DapWriter<W> {
    inner: W,
    next_seq: i64,
}

impl<W: AsyncWrite + Unpin> DapWriter<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, next_seq: 1 }
    }

    pub fn alloc_seq(&mut self) -> i64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        seq
    }

    pub async fn write_message(&mut self, msg: &DapMessage) -> io::Result<()> {
        write_dap_message(&mut self.inner, msg).await
    }

    pub async fn send_request(
        &mut self,
        command: impl Into<String>,
        arguments: Option<Value>,
    ) -> io::Result<i64> {
        let seq = self.alloc_seq();
        let msg = DapMessage::Request(DapRequest::new(seq, command, arguments));
        self.write_message(&msg).await?;
        Ok(seq)
    }

    pub async fn send_response(
        &mut self,
        request_seq: i64,
        command: impl Into<String>,
        result: Result<Option<Value>, String>,
    ) -> io::Result<i64> {
        let seq = self.alloc_seq();
        let resp = match result {
            Ok(body) => DapResponse::ok(seq, request_seq, command, body),
            Err(err) => DapResponse::err(seq, request_seq, command, err),
        };
        self.write_message(&DapMessage::Response(resp)).await?;
        Ok(seq)
    }

    pub async fn send_event(
        &mut self,
        event: impl Into<String>,
        body: Option<Value>,
    ) -> io::Result<i64> {
        let seq = self.alloc_seq();
        let msg = DapMessage::Event(DapEvent::new(seq, event, body));
        self.write_message(&msg).await?;
        Ok(seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn duplex_request_response_event_round_trip() {
        let (client_io, server_io) = tokio::io::duplex(4096);
        let (client_r, client_w) = tokio::io::split(client_io);
        let (server_r, server_w) = tokio::io::split(server_io);

        let mut c_reader = DapReader::new(client_r);
        let mut c_writer = DapWriter::new(client_w);
        let mut s_reader = DapReader::new(server_r);
        let mut s_writer = DapWriter::new(server_w);

        let server_task = tokio::spawn(async move {
            let Some(DapMessage::Request(req)) = s_reader.next_message().await.unwrap() else {
                panic!("expected request");
            };
            assert_eq!(req.seq, 1);
            assert_eq!(req.command, "initialize");
            assert_eq!(req.arguments, Some(json!({ "adapterID": "go" })));

            s_writer
                .send_response(
                    req.seq,
                    &req.command,
                    Ok(Some(json!({ "supportsConfigurationDoneRequest": true }))),
                )
                .await
                .unwrap();
            s_writer
                .send_event("initialized", Some(json!({})))
                .await
                .unwrap();
        });

        let seq = c_writer
            .send_request("initialize", Some(json!({ "adapterID": "go" })))
            .await
            .unwrap();
        assert_eq!(seq, 1);

        let Some(DapMessage::Response(resp)) = c_reader.next_message().await.unwrap() else {
            panic!("expected initialize response");
        };
        assert_eq!(resp.seq, 1);
        assert_eq!(resp.request_seq, 1);
        assert!(resp.success);
        assert_eq!(
            resp.body,
            Some(json!({ "supportsConfigurationDoneRequest": true }))
        );

        let Some(DapMessage::Event(ev)) = c_reader.next_message().await.unwrap() else {
            panic!("expected initialized event");
        };
        assert_eq!(ev.seq, 2);
        assert_eq!(ev.event, "initialized");

        server_task.await.unwrap();
        // Server writer dropped -> clean EOF on client reader.
        assert_eq!(c_reader.next_message().await.unwrap(), None);
    }

    #[tokio::test]
    async fn reassembles_heavily_fragmented_partial_reads_and_unicode() {
        // Tiny 5-byte duplex ring forces partial reads across header and UTF-8 body boundaries.
        let (mut tx, rx) = tokio::io::duplex(5);
        let mut reader = DapReader::with_capacity(3, rx);

        let event = DapMessage::Event(DapEvent::new(
            7,
            "output",
            Some(json!({
                "category": "stdout",
                "output": "שלום עולם · 🐞 debugging café\n"
            })),
        ));
        let body = serde_json::to_vec(&event).unwrap();
        let wire = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            std::str::from_utf8(&body).unwrap()
        );

        let writer_task = tokio::spawn(async move {
            for chunk in wire.as_bytes().chunks(2) {
                tx.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
            tx.shutdown().await.unwrap();
        });

        let decoded = reader.next_message().await.unwrap().expect("message");
        assert_eq!(decoded, event);
        assert_eq!(reader.next_message().await.unwrap(), None);
        writer_task.await.unwrap();
    }

    #[tokio::test]
    async fn enforces_16_mib_cap_before_allocating_body() {
        let (mut tx, rx) = tokio::io::duplex(256);
        let mut reader = DapReader::new(rx);

        let over = MAX_MESSAGE_BYTES + 1;
        tx.write_all(format!("Content-Length: {over}\r\n\r\n").as_bytes())
            .await
            .unwrap();

        let err = reader.next_message().await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().contains("16 MiB"));

        // Outgoing cap is enforced too.
        let (mut out_tx, _out_rx) = tokio::io::duplex(64);
        let huge = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let write_err = write_raw_frame(&mut out_tx, &huge).await.unwrap_err();
        assert_eq!(write_err.kind(), ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn rejects_oversized_headers_duplicate_length_missing_length_and_bad_utf8() {
        // 1. Oversized header line > 8 KiB
        {
            let (mut tx, rx) = tokio::io::duplex(16 * 1024);
            let mut reader = DapReader::new(rx);
            let big = format!(
                "X-Long: {}\r\nContent-Length: 2\r\n\r\n{{}}",
                "x".repeat(9000)
            );
            tx.write_all(big.as_bytes()).await.unwrap();
            assert_eq!(
                reader.next_message().await.unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        // 2. Duplicate Content-Length
        {
            let (mut tx, rx) = tokio::io::duplex(256);
            let mut reader = DapReader::new(rx);
            tx.write_all(b"Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}")
                .await
                .unwrap();
            assert_eq!(
                reader.next_message().await.unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        // 3. Missing Content-Length
        {
            let (mut tx, rx) = tokio::io::duplex(256);
            let mut reader = DapReader::new(rx);
            tx.write_all(b"X-Header: only\r\n\r\n{}").await.unwrap();
            assert_eq!(
                reader.next_message().await.unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        // 4. Invalid UTF-8 in payload
        {
            let (mut tx, rx) = tokio::io::duplex(256);
            let mut reader = DapReader::new(rx);
            tx.write_all(b"Content-Length: 4\r\n\r\n\xff\xfe\xfd\xfc")
                .await
                .unwrap();
            assert_eq!(
                reader.next_message().await.unwrap_err().kind(),
                ErrorKind::InvalidData
            );
        }

        // 5. Truncated body (EOF mid-frame)
        {
            let (mut tx, rx) = tokio::io::duplex(256);
            let mut reader = DapReader::new(rx);
            tx.write_all(b"Content-Length: 20\r\n\r\n{\"seq\":1")
                .await
                .unwrap();
            drop(tx);
            assert_eq!(
                reader.next_message().await.unwrap_err().kind(),
                ErrorKind::UnexpectedEof
            );
        }
    }
}
