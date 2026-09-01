//! Minimal CDP-over-WebSocket client for lightpanda, plus the
//! [`BrowserEngine`] that drives it. Only the handful of Chrome DevTools
//! Protocol calls `LightpandaEngine` needs (`Target.createTarget`,
//! `Target.attachToTarget`, `Page.enable`/`navigate`, `Page.loadEventFired`,
//! `Runtime.evaluate`, `Target.closeTarget`) — not a general CDP client, add
//! methods only when a real caller needs them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use crate::engine::{
    BrowserEngine, EngineError, MatchedNode, Page, PAGE_BYTE_CAP, PAGE_TIMEOUT_SECS,
};

#[derive(Debug, thiserror::Error)]
pub enum CdpError {
    #[error("websocket connect failed: {0}")]
    Connect(String),
    #[error("websocket closed")]
    Closed,
    #[error("cdp protocol error: {0}")]
    Protocol(String),
    #[error("timed out waiting for {0}")]
    Timeout(String),
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Value>>>>;

/// One WebSocket connection to a lightpanda CDP endpoint, with a background
/// reader task that demuxes command replies (by `id`) from events.
///
/// Owns the reader/writer task handles and aborts both on drop (or via the
/// explicit [`CdpClient::close`]) — without that, each connection would leak
/// its WebSocket task pair, since dropping just the `out` sender doesn't
/// tear down the reader half sharing the split socket.
pub struct CdpClient {
    next_id: AtomicU64,
    pending: PendingMap,
    out: mpsc::UnboundedSender<Message>,
    /// Raw CDP events (`method`, `params`), single-consumer — fine, this
    /// client drives exactly one page fetch at a time.
    events: Mutex<mpsc::UnboundedReceiver<(String, Value)>>,
    reader: JoinHandle<()>,
    writer: JoinHandle<()>,
}

impl CdpClient {
    /// `PAGE_TIMEOUT_SECS`-bounded so a sidecar that accepts the TCP
    /// connection but never completes the WS upgrade can't hang forever.
    pub async fn connect(ws_url: &str) -> Result<Self, CdpError> {
        let (ws, _) = tokio::time::timeout(
            Duration::from_secs(PAGE_TIMEOUT_SECS),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .map_err(|_| CdpError::Timeout("websocket handshake".into()))?
        .map_err(|e| CdpError::Connect(e.to_string()))?;
        let (mut write, mut read) = ws.split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();
        let writer = tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let (events_tx, events_rx) = mpsc::unbounded_channel::<(String, Value)>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let pending_reader = pending.clone();
        let reader = tokio::spawn(async move {
            while let Some(Ok(msg)) = read.next().await {
                let Message::Text(text) = msg else { continue };
                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                if let Some(id) = v.get("id").and_then(Value::as_u64) {
                    if let Some(tx) = pending_reader.lock().await.remove(&id) {
                        let _ = tx.send(v);
                    }
                } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                    let params = v.get("params").cloned().unwrap_or(Value::Null);
                    let _ = events_tx.send((method.to_string(), params));
                }
            }
        });

        Ok(Self {
            next_id: AtomicU64::new(1),
            pending,
            out: out_tx,
            events: Mutex::new(events_rx),
            reader,
            writer,
        })
    }

    /// Best-effort graceful shutdown: send a WS close frame, then abort both
    /// background tasks. Safe to call more than once, and safe to skip —
    /// `Drop` aborts both tasks unconditionally as a safety net either way.
    pub async fn close(&self) {
        let _ = self.out.send(Message::Close(None));
        self.reader.abort();
        self.writer.abort();
    }

    async fn call(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, CdpError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let mut frame = json!({"id": id, "method": method, "params": params});
        if let Some(sid) = session_id {
            frame["sessionId"] = json!(sid);
        }
        self.out
            .send(Message::Text(frame.to_string()))
            .map_err(|_| CdpError::Closed)?;

        let resp = tokio::time::timeout(Duration::from_secs(PAGE_TIMEOUT_SECS), rx)
            .await
            .map_err(|_| CdpError::Timeout(method.to_string()))?
            .map_err(|_| CdpError::Closed)?;
        if let Some(err) = resp.get("error") {
            return Err(CdpError::Protocol(err.to_string()));
        }
        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn create_target(&self, url: &str) -> Result<String, CdpError> {
        let result = self
            .call("Target.createTarget", json!({"url": url}), None)
            .await?;
        result
            .get("targetId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CdpError::Protocol("Target.createTarget: no targetId".into()))
    }

    pub async fn attach_to_target(&self, target_id: &str) -> Result<String, CdpError> {
        let result = self
            .call(
                "Target.attachToTarget",
                json!({"targetId": target_id, "flatten": true}),
                None,
            )
            .await?;
        result
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CdpError::Protocol("Target.attachToTarget: no sessionId".into()))
    }

    pub async fn enable_page(&self, session_id: &str) -> Result<(), CdpError> {
        self.call("Page.enable", json!({}), Some(session_id))
            .await?;
        Ok(())
    }

    pub async fn navigate(&self, session_id: &str, url: &str) -> Result<(), CdpError> {
        self.call("Page.navigate", json!({"url": url}), Some(session_id))
            .await?;
        Ok(())
    }

    /// Drain events until `Page.loadEventFired` arrives, or `timeout` elapses.
    pub async fn wait_for_load_event(&self, timeout: Duration) -> Result<(), CdpError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut events = self.events.lock().await;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(CdpError::Timeout("Page.loadEventFired".into()));
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Some((method, _))) if method == "Page.loadEventFired" => return Ok(()),
                Ok(Some(_)) => continue,
                Ok(None) => return Err(CdpError::Closed),
                Err(_) => return Err(CdpError::Timeout("Page.loadEventFired".into())),
            }
        }
    }

    pub async fn evaluate_outer_html(&self, session_id: &str) -> Result<String, CdpError> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({"expression": "document.documentElement.outerHTML", "returnByValue": true}),
                Some(session_id),
            )
            .await?;
        if let Some(details) = result.get("exceptionDetails") {
            return Err(CdpError::Protocol(format!(
                "Runtime.evaluate threw: {details}"
            )));
        }
        result
            .get("result")
            .and_then(|r| r.get("value"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| CdpError::Protocol("Runtime.evaluate: no string value".into()))
    }

    pub async fn close_target(&self, target_id: &str) -> Result<(), CdpError> {
        self.call("Target.closeTarget", json!({"targetId": target_id}), None)
            .await?;
        Ok(())
    }
}

impl Drop for CdpClient {
    fn drop(&mut self) {
        // Safety net if `close()` wasn't called (e.g. an early `?` return) —
        // stops the reader/writer tasks so a connection never outlives the
        // client that owns it.
        self.reader.abort();
        self.writer.abort();
    }
}

fn cdp_err(e: CdpError) -> EngineError {
    match e {
        CdpError::Connect(msg) => EngineError::Unavailable(msg),
        CdpError::Closed => EngineError::Unavailable("cdp websocket closed".into()),
        CdpError::Protocol(msg) => EngineError::Nav(msg),
        CdpError::Timeout(_) => EngineError::Timeout(PAGE_TIMEOUT_SECS),
    }
}

/// Drives a lightpanda sidecar over CDP: navigate, wait for load, snapshot
/// the DOM. Runs JS, so pages it returns are never `degraded`.
pub struct LightpandaEngine {
    cdp_url: String,
}

impl LightpandaEngine {
    pub fn new(cdp_url: String) -> Self {
        Self { cdp_url }
    }

    /// Caller must netguard-check `url` first — see crate docs.
    ///
    /// The ENTIRE navigate-and-snapshot flow is bounded by one
    /// `PAGE_TIMEOUT_SECS` timeout — matching `PAGE_TIMEOUT_SECS`'s contract
    /// as the wall-clock budget for a single `fetch_page`/`query` call,
    /// rather than letting each of the ~6 CDP round trips inside it spend up
    /// to `PAGE_TIMEOUT_SECS` on its own.
    async fn navigate_and_snapshot(&self, url: &str) -> Result<String, EngineError> {
        tokio::time::timeout(
            Duration::from_secs(PAGE_TIMEOUT_SECS),
            self.navigate_and_snapshot_inner(url),
        )
        .await
        .unwrap_or(Err(EngineError::Timeout(PAGE_TIMEOUT_SECS)))
    }

    async fn navigate_and_snapshot_inner(&self, url: &str) -> Result<String, EngineError> {
        let client = CdpClient::connect(&self.cdp_url).await.map_err(cdp_err)?;
        let html = self.drive(&client, url).await;
        // Deterministic cleanup on every path — success or the `?` bailouts
        // inside `drive` — `Drop` is the safety net if this is ever skipped.
        client.close().await;
        let html = html?;
        if html.len() > PAGE_BYTE_CAP {
            return Err(EngineError::TooLarge(PAGE_BYTE_CAP));
        }
        Ok(html)
    }

    async fn drive(&self, client: &CdpClient, url: &str) -> Result<String, EngineError> {
        let target_id = client.create_target("about:blank").await.map_err(cdp_err)?;
        let session_id = client.attach_to_target(&target_id).await.map_err(cdp_err)?;
        client.enable_page(&session_id).await.map_err(cdp_err)?;
        client.navigate(&session_id, url).await.map_err(cdp_err)?;
        client
            .wait_for_load_event(Duration::from_secs(PAGE_TIMEOUT_SECS))
            .await
            .map_err(cdp_err)?;
        let html = client
            .evaluate_outer_html(&session_id)
            .await
            .map_err(cdp_err)?;
        let _ = client.close_target(&target_id).await;
        Ok(html)
    }
}

#[async_trait::async_trait]
impl BrowserEngine for LightpandaEngine {
    /// Caller must netguard-check `url` first — see crate docs.
    async fn fetch_page(&self, url: &str) -> Result<Page, EngineError> {
        let html = self.navigate_and_snapshot(url).await?;
        let title = crate::extract_title(&html);
        let cleaned = crate::readability(&html);
        let markdown = crate::html_to_markdown(&cleaned);
        Ok(Page {
            url: url.to_string(),
            title,
            html,
            markdown,
            degraded: false,
            engine: self.name().to_string(),
        })
    }

    /// Caller must netguard-check `url` first — see crate docs.
    async fn query(&self, url: &str, selector: &str) -> Result<Vec<MatchedNode>, EngineError> {
        let html = self.navigate_and_snapshot(url).await?;
        let document = scraper::Html::parse_document(&html);
        let sel = scraper::Selector::parse(selector)
            .map_err(|e| EngineError::Nav(format!("bad selector {selector:?}: {e:?}")))?;
        Ok(document
            .select(&sel)
            .map(|el| MatchedNode {
                selector: selector.to_string(),
                outer_html: el.html(),
                text: el.text().collect::<Vec<_>>().join(" "),
            })
            .collect())
    }

    fn name(&self) -> &'static str {
        "lightpanda"
    }
}
