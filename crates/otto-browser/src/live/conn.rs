//! CDP connection over Chromium's `--remote-debugging-pipe` transport: JSON
//! messages separated by a NUL byte, read from the browser's fd 4 and written
//! to its fd 3. Unlike the reader engine's short-lived WebSocket client
//! (`crate::cdp::CdpClient`), this connection lives as long as the browser
//! process and multiplexes many page sessions: command replies are matched by
//! `id`, and events are routed by their flat-mode `sessionId` to that session's
//! channel (browser-level events — no `sessionId` — go to the process loop).
//!
//! Generic over the byte streams so tests drive it with `tokio::io::duplex`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::cdp::CdpError;

/// One inbound CDP message is capped (a full-page PNG screenshot comes back
/// base64-encoded in a single reply).
pub const MAX_MESSAGE_BYTES: usize = 96 * 1024 * 1024;

/// Default per-command timeout.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// A CDP event.
#[derive(Debug, Clone)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>;
type Routes = Arc<Mutex<HashMap<String, mpsc::UnboundedSender<CdpEvent>>>>;

pub struct CdpConn {
    next_id: AtomicU64,
    pending: Pending,
    routes: Routes,
    out: mpsc::UnboundedSender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    closed_rx: watch::Receiver<bool>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl CdpConn {
    /// Start the reader/writer tasks. Returns the connection and the receiver
    /// of browser-level events (and of events for sessions nobody routed).
    pub fn start<R, W>(read: R, write: W) -> (Arc<Self>, mpsc::UnboundedReceiver<CdpEvent>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (browser_tx, browser_rx) = mpsc::unbounded_channel::<CdpEvent>();
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let (closed_tx, closed_rx) = watch::channel(false);

        let mut write = write;
        let writer = tokio::spawn(async move {
            while let Some(mut msg) = out_rx.recv().await {
                msg.push(0);
                if write.write_all(&msg).await.is_err() {
                    break;
                }
                if write.flush().await.is_err() {
                    break;
                }
            }
        });

        let pending_r = pending.clone();
        let routes_r = routes.clone();
        let closed_r = closed.clone();
        let reader = tokio::spawn(async move {
            let mut buf_reader = BufReader::with_capacity(1 << 16, read);
            let mut buf: Vec<u8> = Vec::new();
            loop {
                buf.clear();
                match read_message(&mut buf_reader, &mut buf).await {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
                }
                let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
                    continue;
                };
                dispatch(v, &pending_r, &routes_r, &browser_tx);
            }
            // EOF / error: the browser is gone. Fail every waiter.
            closed_r.store(true, Ordering::SeqCst);
            let waiters: Vec<_> = pending_r
                .lock()
                .map(|mut p| p.drain().map(|(_, tx)| tx).collect())
                .unwrap_or_default();
            for tx in waiters {
                let _ = tx.send(Err(CdpError::Closed));
            }
            if let Ok(mut r) = routes_r.lock() {
                r.clear(); // drops the senders → session loops see the end
            }
            let _ = closed_tx.send(true);
        });

        (
            Arc::new(Self {
                next_id: AtomicU64::new(1),
                pending,
                routes,
                out: out_tx,
                closed,
                closed_rx,
                reader,
                writer,
            }),
            browser_rx,
        )
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Resolves once the browser end of the pipe is gone.
    pub async fn closed(&self) {
        let mut rx = self.closed_rx.clone();
        while !*rx.borrow() {
            if rx.changed().await.is_err() {
                return;
            }
        }
    }

    /// Route every event carrying `session_id` to `tx` from now on.
    pub fn route(&self, session_id: &str, tx: mpsc::UnboundedSender<CdpEvent>) {
        if let Ok(mut r) = self.routes.lock() {
            r.insert(session_id.to_string(), tx);
        }
    }

    pub fn unroute(&self, session_id: &str) {
        if let Ok(mut r) = self.routes.lock() {
            r.remove(session_id);
        }
    }

    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, CdpError> {
        self.call_with_timeout(method, params, session_id, CALL_TIMEOUT)
            .await
    }

    pub async fn call_with_timeout(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        if self.is_closed() {
            return Err(CdpError::Closed);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        if let Ok(mut p) = self.pending.lock() {
            p.insert(id, tx);
        }
        let frame = build_command(id, method, params, session_id);
        if self.out.send(frame).is_err() {
            self.forget(id);
            return Err(CdpError::Closed);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(CdpError::Closed),
            Err(_) => {
                self.forget(id);
                Err(CdpError::Timeout(method.to_string()))
            }
        }
    }

    /// Fire-and-forget (acks, `Fetch.continueRequest` on hot paths): the
    /// reply is dropped when it arrives.
    pub fn send(&self, method: &str, params: Value, session_id: Option<&str>) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let _ = self.out.send(build_command(id, method, params, session_id));
    }

    fn forget(&self, id: u64) {
        if let Ok(mut p) = self.pending.lock() {
            p.remove(&id);
        }
    }

    /// Stop both tasks (the browser process itself is owned elsewhere).
    pub fn shutdown(&self) {
        self.reader.abort();
        self.writer.abort();
        self.closed.store(true, Ordering::SeqCst);
    }
}

impl Drop for CdpConn {
    fn drop(&mut self) {
        self.reader.abort();
        self.writer.abort();
    }
}

/// Serialize one command frame (without the NUL terminator).
pub fn build_command(id: u64, method: &str, params: Value, session_id: Option<&str>) -> Vec<u8> {
    let mut frame = json!({"id": id, "method": method, "params": params});
    if let Some(sid) = session_id {
        frame["sessionId"] = json!(sid);
    }
    serde_json::to_vec(&frame).unwrap_or_default()
}

/// Read one NUL-terminated message into `buf`. `Ok(false)` on clean EOF; an
/// error when a message exceeds [`MAX_MESSAGE_BYTES`].
async fn read_message<R: AsyncRead + Unpin>(
    r: &mut BufReader<R>,
    buf: &mut Vec<u8>,
) -> std::io::Result<bool> {
    loop {
        let available = r.fill_buf().await?;
        if available.is_empty() {
            return Ok(false);
        }
        if let Some(pos) = available.iter().position(|b| *b == 0) {
            buf.extend_from_slice(&available[..pos]);
            r.consume(pos + 1);
            return Ok(true);
        }
        let n = available.len();
        buf.extend_from_slice(available);
        r.consume(n);
        if buf.len() > MAX_MESSAGE_BYTES {
            return Err(std::io::Error::other("cdp message too large"));
        }
    }
}

fn dispatch(
    v: Value,
    pending: &Pending,
    routes: &Routes,
    browser_tx: &mpsc::UnboundedSender<CdpEvent>,
) {
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        let tx = pending.lock().ok().and_then(|mut p| p.remove(&id));
        if let Some(tx) = tx {
            let result = match v.get("error") {
                Some(err) => Err(CdpError::Protocol(err.to_string())),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(result);
        }
        return;
    }
    let Some(method) = v.get("method").and_then(Value::as_str) else {
        return;
    };
    let session_id = v
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ev = CdpEvent {
        method: method.to_string(),
        params: v.get("params").cloned().unwrap_or(Value::Null),
        session_id: session_id.clone(),
    };
    if let Some(sid) = &session_id {
        let route = routes.lock().ok().and_then(|r| r.get(sid).cloned());
        if let Some(tx) = route {
            // A closed receiver means the session is gone: drop its late events.
            let _ = tx.send(ev);
            return;
        }
    }
    let _ = browser_tx.send(ev);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// A fake browser end: reads NUL-framed commands, lets the test answer.
    async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Value {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            r.read_exact(&mut byte).await.unwrap();
            if byte[0] == 0 {
                break;
            }
            buf.push(byte[0]);
        }
        serde_json::from_slice(&buf).unwrap()
    }

    #[tokio::test]
    async fn replies_are_matched_by_id_and_events_routed_by_session() {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (our_r, our_w) = tokio::io::split(ours);
        let (mut their_r, mut their_w) = tokio::io::split(theirs);
        let (conn, mut browser_events) = CdpConn::start(our_r, our_w);

        let (tx, mut session_events) = mpsc::unbounded_channel();
        conn.route("S1", tx);

        let c2 = conn.clone();
        let call = tokio::spawn(async move {
            c2.call(
                "Page.navigate",
                json!({"url": "https://example.com/"}),
                Some("S1"),
            )
            .await
        });
        let cmd = read_frame(&mut their_r).await;
        assert_eq!(cmd["method"], "Page.navigate");
        assert_eq!(cmd["sessionId"], "S1");
        let id = cmd["id"].as_u64().unwrap();

        // An event for S1, a browser-level event, then the reply.
        their_w
            .write_all(b"{\"method\":\"Page.frameNavigated\",\"sessionId\":\"S1\",\"params\":{\"frame\":{}}}\0")
            .await
            .unwrap();
        their_w
            .write_all(b"{\"method\":\"Target.targetCrashed\",\"params\":{\"targetId\":\"T\"}}\0")
            .await
            .unwrap();
        their_w
            .write_all(format!("{{\"id\":{id},\"result\":{{\"frameId\":\"F\"}}}}\0").as_bytes())
            .await
            .unwrap();

        let r = call.await.unwrap().unwrap();
        assert_eq!(r["frameId"], "F");
        let ev = session_events.recv().await.unwrap();
        assert_eq!(ev.method, "Page.frameNavigated");
        let ev = browser_events.recv().await.unwrap();
        assert_eq!(ev.method, "Target.targetCrashed");
    }

    #[tokio::test]
    async fn protocol_errors_and_eof_fail_the_caller() {
        let (ours, theirs) = tokio::io::duplex(1 << 16);
        let (our_r, our_w) = tokio::io::split(ours);
        let (mut their_r, mut their_w) = tokio::io::split(theirs);
        let (conn, _browser_events) = CdpConn::start(our_r, our_w);

        let c2 = conn.clone();
        let call = tokio::spawn(async move { c2.call("Fetch.enable", json!({}), None).await });
        let cmd = read_frame(&mut their_r).await;
        let id = cmd["id"].as_u64().unwrap();
        their_w
            .write_all(
                format!("{{\"id\":{id},\"error\":{{\"code\":-32601,\"message\":\"nope\"}}}}\0")
                    .as_bytes(),
            )
            .await
            .unwrap();
        assert!(matches!(call.await.unwrap(), Err(CdpError::Protocol(_))));

        // Browser goes away mid-call.
        let c3 = conn.clone();
        let call = tokio::spawn(async move { c3.call("Page.reload", json!({}), None).await });
        let _ = read_frame(&mut their_r).await;
        drop(their_w);
        drop(their_r);
        assert!(matches!(call.await.unwrap(), Err(CdpError::Closed)));
        conn.closed().await;
        assert!(conn.is_closed());
        assert!(matches!(
            conn.call("Page.reload", json!({}), None).await,
            Err(CdpError::Closed)
        ));
    }

    #[test]
    fn command_frames_carry_the_flat_session_id() {
        let f = build_command(3, "Input.insertText", json!({"text": "x"}), Some("S"));
        let v: Value = serde_json::from_slice(&f).unwrap();
        assert_eq!(v["id"], 3);
        assert_eq!(v["sessionId"], "S");
        let f = build_command(4, "Browser.getVersion", json!({}), None);
        let v: Value = serde_json::from_slice(&f).unwrap();
        assert!(v.get("sessionId").is_none());
    }
}
