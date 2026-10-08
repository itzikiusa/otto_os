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
use tokio::sync::{mpsc, oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

use crate::cdp::CdpError;

/// One inbound CDP message is capped (a full-page PNG screenshot comes back
/// base64-encoded in a single reply).
pub const MAX_MESSAGE_BYTES: usize = 96 * 1024 * 1024;

/// Default per-command timeout.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Count and conservative byte charges both bound each transport direction.
/// Events share one budget across all page routes and browser-level events.
pub const QUEUE_MESSAGES: usize = 256;
const QUEUE_BYTES: usize = 32 * 1024 * 1024;

/// A CDP event.
#[derive(Debug)]
pub struct CdpEvent {
    pub method: String,
    pub params: Value,
    pub session_id: Option<String>,
    _charge: OwnedSemaphorePermit,
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>;
type Routes = Arc<Mutex<HashMap<String, mpsc::Sender<CdpEvent>>>>;

struct PendingCall<'a> {
    pending: &'a Pending,
    id: u64,
}

impl Drop for PendingCall<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.id);
        }
    }
}

pub struct CdpConn {
    next_id: AtomicU64,
    pending: Pending,
    routes: Routes,
    out: mpsc::Sender<(Vec<u8>, OwnedSemaphorePermit)>,
    out_budget: Arc<Semaphore>,
    closed_tx: watch::Sender<bool>,
    closed: Arc<AtomicBool>,
    closed_rx: watch::Receiver<bool>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl CdpConn {
    /// Start the reader/writer tasks. Returns the connection and the receiver
    /// of browser-level events (and of events for sessions nobody routed).
    pub fn start<R, W>(read: R, write: W) -> (Arc<Self>, mpsc::Receiver<CdpEvent>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, mut out_rx) = mpsc::channel::<(Vec<u8>, OwnedSemaphorePermit)>(QUEUE_MESSAGES);
        let (browser_tx, browser_rx) = mpsc::channel::<CdpEvent>(QUEUE_MESSAGES);
        let event_budget = Arc::new(Semaphore::new(QUEUE_BYTES));
        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let routes: Routes = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));
        let (closed_tx, closed_rx) = watch::channel(false);

        let writer_closed = closed_tx.clone();
        let mut write = write;
        let writer = tokio::spawn(async move {
            while let Some((mut msg, _charge)) = out_rx.recv().await {
                msg.push(0);
                if write.write_all(&msg).await.is_err() {
                    break;
                }
                if write.flush().await.is_err() {
                    break;
                }
            }
            let _ = writer_closed.send(true);
        });

        let pending_r = pending.clone();
        let routes_r = routes.clone();
        let closed_r = closed.clone();
        let reader_closed = closed_tx.clone();
        let mut stop = closed_rx.clone();
        let stop_writer = writer.abort_handle();
        let reader = tokio::spawn(async move {
            let mut buf_reader = BufReader::with_capacity(1 << 16, read);
            let mut buf: Vec<u8> = Vec::new();
            loop {
                buf.clear();
                let read = tokio::select! {
                    biased;
                    _ = stop.changed() => break,
                    result = read_message(&mut buf_reader, &mut buf) => result,
                };
                match read {
                    Ok(true) => {}
                    Ok(false) | Err(_) => break,
                }
                let Ok(v) = serde_json::from_slice::<Value>(&buf) else {
                    continue;
                };
                if !dispatch(
                    v,
                    buf.len(),
                    &event_budget,
                    &pending_r,
                    &routes_r,
                    &browser_tx,
                ) {
                    break; // Critical events cannot be dropped: fail the connection closed.
                }
            }
            stop_writer.abort();
            close_transport(&closed_r, &reader_closed, &pending_r, &routes_r);
        });

        (
            Arc::new(Self {
                next_id: AtomicU64::new(1),
                pending,
                routes,
                out: out_tx,
                out_budget: Arc::new(Semaphore::new(QUEUE_BYTES)),
                closed_tx,
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
    pub fn route(&self, session_id: &str, tx: mpsc::Sender<CdpEvent>) {
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
            if self.is_closed() {
                return Err(CdpError::Closed);
            }
            // Output can drain while the browser stops answering. Bound the
            // reply waiters independently of the serialized command queue.
            if p.len() >= QUEUE_MESSAGES {
                return Err(CdpError::Protocol(
                    "browser command capacity exceeded; retry shortly".into(),
                ));
            }
            p.insert(id, tx);
        }
        // An HTTP/socket caller can disappear before a reply or timeout.
        // Keep its waiter scoped to this future even on cancellation.
        let _pending = PendingCall {
            pending: &self.pending,
            id,
        };
        let frame = build_command(id, method, params, session_id);
        if !self.enqueue(frame) {
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
        if !self.is_closed() {
            self.enqueue(build_command(id, method, params, session_id));
        }
    }

    fn enqueue(&self, frame: Vec<u8>) -> bool {
        let charge = frame.len().saturating_add(64);
        let permit = u32::try_from(charge)
            .ok()
            .and_then(|n| self.out_budget.clone().try_acquire_many_owned(n).ok());
        if let Some(permit) = permit {
            if self.out.try_send((frame, permit)).is_ok() {
                return true;
            }
        }
        self.shutdown();
        false
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
        close_transport(&self.closed, &self.closed_tx, &self.pending, &self.routes);
    }
}

fn close_transport(
    closed: &AtomicBool,
    signal: &watch::Sender<bool>,
    pending: &Pending,
    routes: &Routes,
) {
    closed.store(true, Ordering::SeqCst);
    if let Ok(mut p) = pending.lock() {
        for (_, tx) in p.drain() {
            let _ = tx.send(Err(CdpError::Closed));
        }
    }
    if let Ok(mut r) = routes.lock() {
        r.clear();
    }
    signal.send_replace(true);
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
            if buf.len().saturating_add(pos) > MAX_MESSAGE_BYTES {
                return Err(std::io::Error::other("cdp message too large"));
            }
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
    mut v: Value,
    wire_bytes: usize,
    budget: &Arc<Semaphore>,
    pending: &Pending,
    routes: &Routes,
    browser_tx: &mpsc::Sender<CdpEvent>,
) -> bool {
    if let Some(id) = v.get("id").and_then(Value::as_u64) {
        let tx = pending.lock().ok().and_then(|mut p| p.remove(&id));
        if let Some(tx) = tx {
            let result = match v.get("error") {
                Some(err) => Err(CdpError::Protocol(err.to_string())),
                None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
            };
            let _ = tx.send(result);
        }
        return true;
    }
    let Some(method) = v.get("method").and_then(Value::as_str) else {
        return true;
    };
    let method = method.to_string();
    let charge = wire_bytes.saturating_mul(4).saturating_add(512);
    let Some(permit) = u32::try_from(charge)
        .ok()
        .and_then(|n| budget.clone().try_acquire_many_owned(n).ok())
    else {
        return false;
    };
    let session_id = v
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let ev = CdpEvent {
        method,
        params: v.get_mut("params").map(Value::take).unwrap_or(Value::Null),
        _charge: permit,
        session_id: session_id.clone(),
    };
    if let Some(sid) = &session_id {
        let route = routes.lock().ok().and_then(|r| r.get(sid).cloned());
        if let Some(tx) = route {
            // A closed receiver means the session is gone: drop its late events.
            return !matches!(tx.try_send(ev), Err(mpsc::error::TrySendError::Full(_)));
        }
    }
    browser_tx.try_send(ev).is_ok()
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

        let (tx, mut session_events) = mpsc::channel(QUEUE_MESSAGES);
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

    #[tokio::test]
    async fn cancelled_live_call_removes_its_pending_reply() {
        let (ours, mut theirs) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(ours);
        let (conn, _events) = CdpConn::start(read, write);
        let caller = conn.clone();
        let call = tokio::spawn(async move {
            caller
                .call("Page.captureScreenshot", json!({}), Some("S"))
                .await
        });
        let _ = read_frame(&mut theirs).await;
        assert_eq!(conn.pending.lock().unwrap().len(), 1);
        call.abort();
        assert!(call.await.unwrap_err().is_cancelled());
        assert!(conn.pending.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn output_flood_closes_connection_instead_of_retaining_commands() {
        let (ours, _stalled_browser) = tokio::io::duplex(64);
        let (read, write) = tokio::io::split(ours);
        let (conn, _events) = CdpConn::start(read, write);
        for _ in 0..4096 {
            conn.send(
                "Input.insertText",
                json!({"text": "x".repeat(256)}),
                Some("S"),
            );
        }
        assert!(
            conn.is_closed(),
            "stalled output must have bounded admission"
        );
        tokio::time::timeout(Duration::from_secs(1), conn.closed())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn event_flood_closes_connection_and_fails_pending_calls() {
        let (ours, mut browser) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(ours);
        let (conn, _stalled_events) = CdpConn::start(read, write);
        let caller = conn.clone();
        let pending =
            tokio::spawn(async move { caller.call("Page.reload", json!({}), None).await });
        let _ = read_frame(&mut browser).await;
        for _ in 0..4096 {
            if browser
                .write_all(b"{\"method\":\"Fetch.requestPaused\",\"params\":{}}\0")
                .await
                .is_err()
            {
                break;
            }
        }
        assert!(
            tokio::time::timeout(Duration::from_secs(1), conn.closed())
                .await
                .is_ok(),
            "stalled event consumer must terminate the transport at its bound"
        );
        assert!(matches!(pending.await.unwrap(), Err(CdpError::Closed)));
    }
    #[tokio::test]
    async fn unanswered_calls_have_bounded_admission_even_when_output_drains() {
        let (ours, mut browser) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(ours);
        let (conn, _events) = CdpConn::start(read, write);
        let drain = tokio::spawn(async move {
            let mut buf = [0; 4096];
            while browser.read(&mut buf).await.unwrap_or(0) != 0 {}
        });
        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..1024 {
            let conn = conn.clone();
            calls.spawn(async move { conn.call("Page.reload", json!({}), None).await });
            tokio::task::yield_now().await;
        }
        assert!(
            conn.pending.lock().unwrap().len() <= 256,
            "a draining browser that never replies must not accumulate unbounded callers"
        );
        calls.abort_all();
        while calls.join_next().await.is_some() {}
        assert!(conn.pending.lock().unwrap().is_empty());
        conn.shutdown();
        drain.abort();
    }

    #[tokio::test]
    async fn event_byte_budget_closes_before_message_count_limit() {
        let (ours, mut browser) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(ours);
        let (conn, _events) = CdpConn::start(read, write);
        let mut event = serde_json::to_vec(&json!({"method": "Page.javascriptDialogOpening", "params": {"message": "x".repeat(1024 * 1024)}})).unwrap();
        event.push(0);
        // Sixteen events are far below the message-count bound, but their
        // aggregate decoded-payload charge must exceed the byte bound.
        for _ in 0..16 {
            if browser.write_all(&event).await.is_err() {
                break;
            }
        }
        tokio::time::timeout(Duration::from_secs(2), conn.closed())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn output_byte_budget_closes_before_message_count_limit() {
        let (ours, _browser) = tokio::io::duplex(64);
        let (read, write) = tokio::io::split(ours);
        let (conn, _events) = CdpConn::start(read, write);
        for _ in 0..40 {
            conn.send(
                "Input.insertText",
                json!({"text": "x".repeat(1024 * 1024)}),
                None,
            );
            if conn.is_closed() {
                break;
            }
        }
        assert!(
            conn.is_closed(),
            "a few large commands must also hit the byte limit"
        );
    }

    #[tokio::test]
    async fn sustained_event_drain_releases_budget_and_preserves_reply_routing() {
        let started = std::time::Instant::now();
        let (ours, mut browser) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(ours);
        let (conn, mut events) = CdpConn::start(read, write);
        let mut event = serde_json::to_vec(
            &json!({"method": "Target.targetInfoChanged", "params": {"title": "x".repeat(4096)}}),
        )
        .unwrap();
        event.push(0);
        // 10k real pipe events / ~40 MiB source; would exhaust the byte budget
        // if consumer completion failed to release permits.
        for _ in 0..100 {
            for _ in 0..100 {
                browser.write_all(&event).await.unwrap();
            }
            for _ in 0..100 {
                drop(events.recv().await.unwrap());
            }
        }
        assert!(!conn.is_closed());
        let caller = conn.clone();
        let call =
            tokio::spawn(async move { caller.call("Browser.getVersion", json!({}), None).await });
        let cmd = read_frame(&mut browser).await;
        browser
            .write_all(
                format!(
                    "{{\"id\":{},\"result\":{{\"product\":\"fixture\"}}}}\0",
                    cmd["id"]
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(call.await.unwrap().unwrap()["product"], "fixture");
        println!(
            "10,000 CDP events (40 MiB) drained in {:?}; subsequent reply routed",
            started.elapsed()
        );
        conn.shutdown();
    }
}
