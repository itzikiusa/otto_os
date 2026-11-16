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
use tokio::io::{AsyncBufRead, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::mpsc;

pub use crate::jsonrpc_framing::{MAX_HEADER_BYTES, MAX_MESSAGE_BYTES};

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
    std::str::from_utf8(payload)
        .map_err(|_| invalid_data("DAP outgoing payload is not valid UTF-8"))?;
    crate::jsonrpc_framing::write_frame_bytes(writer, payload).await
}

/// Read a single `Content-Length` framed UTF-8 payload from `reader`.
///
/// Returns `Ok(None)` on clean EOF before a new header begins.
pub async fn read_raw_frame<R>(reader: &mut R) -> io::Result<Option<String>>
where
    R: AsyncBufRead + Unpin,
{
    let Some(body) = crate::jsonrpc_framing::read_frame_bytes(reader).await? else {
        return Ok(None);
    };
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

    /// Reads messages on their own task, for a `select!` loop to poll. Reading
    /// in the loop is not cancel-safe: a frame's header and body are separate
    /// writes, and a read cancelled between them drops the header, desyncing
    /// the stream so the session hangs. Yields the closing `Ok(None)` or error.
    pub fn into_channel(mut self) -> mpsc::UnboundedReceiver<io::Result<Option<DapMessage>>>
    where
        R: Send + 'static,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let msg = self.next_message().await;
                let more = matches!(msg, Ok(Some(_)));
                if tx.send(msg).is_err() || !more {
                    break;
                }
            }
        });
        rx
    }

    pub async fn next_raw_frame(&mut self) -> io::Result<Option<String>> {
        read_raw_frame(&mut self.inner).await
    }

    /// Next message, or a [`DapReadItem::Skipped`] frame (over the 16 MiB cap,
    /// or not a DAP message) that leaves the stream usable: the caller fails
    /// just the request it answered instead of ending the session.
    pub async fn next_item(&mut self) -> io::Result<Option<DapReadItem>> {
        use crate::jsonrpc_framing::{read_frame_or_skip, Frame, SKIPPED_HEAD_BYTES};
        let item = match read_frame_or_skip(&mut self.inner).await? {
            None => return Ok(None),
            Some(Frame::Body(body)) => match serde_json::from_slice::<DapMessage>(&body) {
                Ok(msg) => DapReadItem::Message(msg),
                Err(e) => DapReadItem::Skipped {
                    len: body.len(),
                    request_seq: response_request_seq(&body[..body.len().min(SKIPPED_HEAD_BYTES)]),
                    reason: format!("invalid DAP message: {e}"),
                },
            },
            Some(Frame::Oversized { len, head }) => DapReadItem::Skipped {
                len,
                request_seq: response_request_seq(&head),
                reason: format!("{len} bytes, over the 16 MiB message cap"),
            },
        };
        Ok(Some(item))
    }
}

/// One item read by [`DapReader::next_item`].
#[derive(Debug)]
pub enum DapReadItem {
    Message(DapMessage),
    /// A frame that was read past; `request_seq` names the request it answered
    /// when its head says it is a response.
    Skipped {
        len: usize,
        request_seq: Option<i64>,
        reason: String,
    },
}

/// `request_seq` of a response, from the first bytes of its JSON.
fn response_request_seq(head: &[u8]) -> Option<i64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let text = String::from_utf8_lossy(head);
    if !text.contains("\"response\"") {
        return None;
    }
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#""request_seq"\s*:\s*(\d+)"#).expect("static request_seq regex")
    });
    re.captures(&text)?.get(1)?.as_str().parse().ok()
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
    use tokio::io::AsyncWriteExt;

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

    /// Bridges poll DAP inside `select!`, so a frame whose body arrives after its
    /// header must survive the poll being cancelled between the two writes.
    #[tokio::test]
    async fn into_channel_survives_cancellation_mid_frame() {
        let (mut tx, rx) = tokio::io::duplex(1024);
        let mut msgs = DapReader::new(rx).into_channel();
        let req = DapMessage::Request(DapRequest::new(1, "configurationDone", None));
        let body = serde_json::to_vec(&req).unwrap();
        tx.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        tokio::select! {
            got = msgs.recv() => panic!("a header alone must not yield: {got:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
        }
        tx.write_all(&body).await.unwrap();
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), msgs.recv())
            .await
            .expect("frame completed after the cancelled poll")
            .unwrap()
            .unwrap();
        assert_eq!(got, Some(req));
    }
}
