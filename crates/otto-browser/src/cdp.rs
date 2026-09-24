//! Minimal CDP-over-WebSocket client for lightpanda, plus the
//! [`BrowserEngine`] that drives it. Only the handful of Chrome DevTools
//! Protocol calls `LightpandaEngine` needs (`Target.createTarget`,
//! `Target.attachToTarget`, `Page.enable`/`navigate`, `Page.loadEventFired`,
//! `Runtime.evaluate`, `Target.closeTarget`) — not a general CDP client, add
//! methods only when a real caller needs them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    /// A top-level document request (navigation or redirect hop) was refused
    /// by the SSRF guard. Carries the refused host, never the full URL.
    #[error("navigation to {0} blocked (SSRF guard)")]
    Blocked(String),
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
        self.create_target_in(url, None).await
    }

    /// `Target.createTarget`, inside `browser_context_id` when given.
    pub async fn create_target_in(
        &self,
        url: &str,
        browser_context_id: Option<&str>,
    ) -> Result<String, CdpError> {
        let mut params = json!({"url": url});
        if let Some(ctx) = browser_context_id {
            params["browserContextId"] = json!(ctx);
        }
        let result = self.call("Target.createTarget", params, None).await?;
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

    /// Turn on CDP request interception (`Fetch.enable`, request stage, every
    /// URL) for `session_id`: from here on every network request the page
    /// makes — the navigation, each redirect hop, subresources, XHR/fetch —
    /// is PAUSED until [`Self::pump_until_load`] answers it. Errors when the
    /// engine does not implement the `Fetch` domain.
    pub async fn enable_request_interception(&self, session_id: &str) -> Result<(), CdpError> {
        self.call(
            "Fetch.enable",
            json!({"patterns": [{"urlPattern": "*", "requestStage": "Request"}]}),
            Some(session_id),
        )
        .await?;
        Ok(())
    }

    /// Like [`Self::wait_for_load_event`], but also answers every
    /// `Fetch.requestPaused` event while waiting: when `guard` is set the
    /// request's URL is vetted through `otto-netguard` (see
    /// [`request_allowed`]) and continued or failed (`BlockedByClient`); a
    /// refused *document* request (the navigation itself or a redirect hop)
    /// ends the wait with [`CdpError::Blocked`]. With `guard` off paused
    /// requests are simply continued.
    pub async fn pump_until_load(
        &self,
        session_id: &str,
        timeout: Duration,
        guard: bool,
    ) -> Result<(), CdpError> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut events = self.events.lock().await;
        // Per-navigation verdict cache keyed by scheme://host:port, so a page
        // with 100 subresources on one CDN costs one DNS vetting, not 100.
        let mut verdicts: HashMap<String, bool> = HashMap::new();
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(CdpError::Timeout("Page.loadEventFired".into()));
            }
            match tokio::time::timeout(remaining, events.recv()).await {
                Ok(Some((method, _))) if method == "Page.loadEventFired" => return Ok(()),
                Ok(Some((method, params))) if method == "Fetch.requestPaused" => {
                    self.answer_paused(session_id, &params, guard, &mut verdicts)
                        .await?;
                }
                Ok(Some(_)) => continue,
                Ok(None) => return Err(CdpError::Closed),
                Err(_) => return Err(CdpError::Timeout("Page.loadEventFired".into())),
            }
        }
    }

    async fn answer_paused(
        &self,
        session_id: &str,
        params: &Value,
        guard: bool,
        verdicts: &mut HashMap<String, bool>,
    ) -> Result<(), CdpError> {
        let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
            return Ok(());
        };
        let url = params
            .pointer("/request/url")
            .and_then(Value::as_str)
            .unwrap_or("");
        let allowed = if !guard {
            true
        } else {
            let key = origin_key(url);
            match verdicts.get(&key) {
                Some(v) => *v,
                None => {
                    let v = request_allowed(url).await;
                    verdicts.insert(key, v);
                    v
                }
            }
        };
        if allowed {
            self.call(
                "Fetch.continueRequest",
                json!({"requestId": request_id}),
                Some(session_id),
            )
            .await?;
            return Ok(());
        }
        let _ = self
            .call(
                "Fetch.failRequest",
                json!({"requestId": request_id, "errorReason": "BlockedByClient"}),
                Some(session_id),
            )
            .await;
        // A refused document (navigation / redirect hop) ends the page. An
        // engine that omits `resourceType` is treated the same (fail closed).
        if params
            .get("resourceType")
            .and_then(Value::as_str)
            .is_none_or(|t| t == "Document")
        {
            return Err(CdpError::Blocked(host_for_error(url)));
        }
        Ok(())
    }

    pub async fn evaluate_outer_html(&self, session_id: &str) -> Result<String, CdpError> {
        self.evaluate(session_id, "document.documentElement.outerHTML")
            .await?
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| CdpError::Protocol("Runtime.evaluate: no string value".into()))
    }

    /// Run `expression` in the page and return its JSON-encoded result
    /// (`Runtime.evaluate` with `returnByValue: true`). Generalizes
    /// `evaluate_outer_html` for callers that need something other than a
    /// bare string (e.g. a boolean login-success check).
    pub async fn evaluate(&self, session_id: &str, expression: &str) -> Result<Value, CdpError> {
        let result = self
            .call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
                Some(session_id),
            )
            .await?;
        if let Some(details) = result.get("exceptionDetails") {
            return Err(CdpError::Protocol(format!(
                "Runtime.evaluate threw: {details}"
            )));
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// A fresh, isolated browser context (own cookie jar / storage).
    pub async fn create_browser_context(&self) -> Result<String, CdpError> {
        let result = self
            .call("Target.createBrowserContext", json!({}), None)
            .await?;
        result
            .get("browserContextId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                CdpError::Protocol("Target.createBrowserContext: no browserContextId".into())
            })
    }

    pub async fn dispose_browser_context(&self, id: &str) -> Result<(), CdpError> {
        self.call(
            "Target.disposeBrowserContext",
            json!({"browserContextId": id}),
            None,
        )
        .await?;
        Ok(())
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
        e @ CdpError::Blocked(_) => EngineError::Nav(e.to_string()),
    }
}

/// SSRF verdict for one request the page makes (navigation, redirect hop,
/// subresource, XHR). Non-network schemes (`data:`/`blob:`/`about:`) pass;
/// everything else must satisfy `otto_netguard::check_url` — http(s)/ws(s)
/// only, and no loopback / private / link-local / metadata address.
pub(crate) async fn request_allowed(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if matches!(parsed.scheme(), "data" | "blob" | "about") {
        return true;
    }
    if !otto_netguard::check_url_literal(&parsed) {
        return false;
    }
    otto_netguard::check_url(url).await.is_ok()
}

/// Cache key for [`request_allowed`] verdicts: `scheme://host:port`.
fn origin_key(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(u) => format!(
            "{}://{}:{}",
            u.scheme(),
            u.host_str().unwrap_or(""),
            u.port_or_known_default().unwrap_or(0)
        ),
        Err(_) => url.to_string(),
    }
}

/// Host of a refused URL for an error message (never the path/query, which
/// may carry tokens).
fn host_for_error(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "an unparseable url".into())
}

/// Drives a lightpanda sidecar over CDP: navigate, wait for load, snapshot
/// the DOM. Runs JS, so pages it returns are never `degraded`.
///
/// SSRF: the route layer netguard-checks the URL it hands us, but the engine
/// then follows redirects and loads subresources/XHR on its own. A guarded
/// engine ([`Self::new`]) therefore intercepts EVERY request the page makes
/// (`Fetch.enable`) and vets it through `otto-netguard` before letting it go,
/// and re-checks the settled `location.href`. An engine build that cannot
/// intercept is refused (reported `Unavailable`, so [`crate::BrowserService`]
/// degrades to the guarded plain fetch) rather than browsing unguarded.
/// Residual: lightpanda resolves hostnames itself, so a request is vetted on
/// our resolution of the name, not pinned to it.
pub struct LightpandaEngine {
    cdp_url: String,
    /// Vet every request through the SSRF guard. Only fixtures pointing the
    /// engine at a loopback test server turn this off ([`Self::new_unguarded`]).
    guard: bool,
    /// Latched when the engine rejected `Fetch.enable`: every later call is
    /// refused up front instead of re-probing per page.
    interception_unsupported: AtomicBool,
}

impl LightpandaEngine {
    /// A guarded engine — what the daemon uses.
    pub fn new(cdp_url: String) -> Self {
        Self {
            cdp_url,
            guard: true,
            interception_unsupported: AtomicBool::new(false),
        }
    }

    /// An engine WITHOUT request interception / SSRF vetting. For tests that
    /// drive a real sidecar against a loopback fixture server only — never
    /// for user-supplied URLs.
    pub fn new_unguarded(cdp_url: String) -> Self {
        Self {
            cdp_url,
            guard: false,
            interception_unsupported: AtomicBool::new(false),
        }
    }

    /// `false` once this (guarded) engine found it cannot intercept requests.
    pub fn interception_ok(&self) -> bool {
        !(self.guard && self.interception_unsupported.load(Ordering::Relaxed))
    }

    /// Guarded navigation: enable interception (when guarding), then run
    /// `Page.navigate` CONCURRENTLY with the event pump — with interception on,
    /// the navigation request itself is paused until the pump continues it, so
    /// awaiting `navigate` first could stall. Finally re-vet the settled
    /// `location.href` (belt and braces for engines that don't pause redirect
    /// hops).
    async fn navigate_guarded(
        &self,
        client: &CdpClient,
        session_id: &str,
        url: &str,
    ) -> Result<(), EngineError> {
        if self.guard {
            if !self.interception_ok() {
                return Err(unsupported_interception());
            }
            match client.enable_request_interception(session_id).await {
                Ok(()) => {}
                // The engine answered and said no: it lacks the Fetch domain.
                // Latch, so later pages skip straight to the plain fetch.
                Err(CdpError::Protocol(_)) => {
                    self.interception_unsupported.store(true, Ordering::Relaxed);
                    tracing::warn!(
                        "browser: lightpanda does not support CDP request interception \
                         (Fetch.enable); refusing JS browsing and using the guarded plain fetch"
                    );
                    return Err(unsupported_interception());
                }
                Err(e) => return Err(cdp_err(e)),
            }
        }
        let nav = client.navigate(session_id, url);
        let load = client.pump_until_load(
            session_id,
            Duration::from_secs(PAGE_TIMEOUT_SECS),
            self.guard,
        );
        tokio::pin!(nav);
        tokio::pin!(load);
        tokio::select! {
            r = &mut load => {
                r.map_err(cdp_err)?;
                nav.await.map_err(cdp_err)?;
            }
            r = &mut nav => {
                r.map_err(cdp_err)?;
                load.await.map_err(cdp_err)?;
            }
        }
        if self.guard {
            let href = client
                .evaluate(session_id, "location.href")
                .await
                .map_err(cdp_err)?;
            let href = href.as_str().unwrap_or("");
            if !request_allowed(href).await {
                return Err(cdp_err(CdpError::Blocked(host_for_error(href))));
            }
        }
        Ok(())
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

    /// Every page load runs in its OWN browser context, disposed afterwards:
    /// before this, `createTarget` never passed a `browserContextId`, so
    /// cookies a page (or a `login()`) set landed in the sidecar's shared
    /// default context — potentially visible to another workspace's/user's
    /// later fetches. Reads fall back to the connection's default context
    /// when the engine refuses `Target.createBrowserContext` (logged);
    /// `login()` never does (see [`Self::login_flow_inner`]).
    async fn drive(&self, client: &CdpClient, url: &str) -> Result<String, EngineError> {
        let ctx = match client.create_browser_context().await {
            Ok(id) => Some(id),
            Err(CdpError::Protocol(e)) => {
                tracing::warn!(
                    "browser: lightpanda refused Target.createBrowserContext ({e}); \
                     reading in the connection's default context"
                );
                None
            }
            Err(e) => return Err(cdp_err(e)),
        };
        let result = self.drive_in(client, url, ctx.as_deref()).await;
        if let Some(id) = &ctx {
            let _ = client.dispose_browser_context(id).await;
        }
        result
    }

    async fn drive_in(
        &self,
        client: &CdpClient,
        url: &str,
        ctx: Option<&str>,
    ) -> Result<String, EngineError> {
        let target_id = client
            .create_target_in("about:blank", ctx)
            .await
            .map_err(cdp_err)?;
        let session_id = client.attach_to_target(&target_id).await.map_err(cdp_err)?;
        client.enable_page(&session_id).await.map_err(cdp_err)?;
        if let Err(e) = self.navigate_guarded(client, &session_id, url).await {
            let _ = client.close_target(&target_id).await;
            return Err(e);
        }
        let html = client
            .evaluate_outer_html(&session_id)
            .await
            .map_err(cdp_err)?;
        let _ = client.close_target(&target_id).await;
        Ok(html)
    }

    /// Caller must netguard-check `url` first — see crate docs.
    ///
    /// Bounded by one `PAGE_TIMEOUT_SECS` timeout, same contract as
    /// [`Self::navigate_and_snapshot`]. `username`/`password` are only ever
    /// interpolated into the fill-and-submit JS expression run in the
    /// target page's own context — never logged, never returned in an
    /// `EngineError`.
    async fn login_flow(
        &self,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<bool, EngineError> {
        tokio::time::timeout(
            Duration::from_secs(PAGE_TIMEOUT_SECS),
            self.login_flow_inner(url, username, password),
        )
        .await
        .unwrap_or(Err(EngineError::Timeout(PAGE_TIMEOUT_SECS)))
    }

    async fn login_flow_inner(
        &self,
        url: &str,
        username: &str,
        password: &str,
    ) -> Result<bool, EngineError> {
        let client = CdpClient::connect(&self.cdp_url).await.map_err(cdp_err)?;
        // A credential is only ever typed into a throwaway, isolated context —
        // never the sidecar's shared default one (whose cookies another
        // workspace's fetch could later see). No isolation → no login.
        let ctx = match client.create_browser_context().await {
            Ok(id) => id,
            Err(e) => {
                client.close().await;
                return Err(EngineError::Unavailable(format!(
                    "lightpanda cannot create an isolated browser context for sign-in: {e}"
                )));
            }
        };
        let result = self
            .drive_login(&client, url, username, password, &ctx)
            .await;
        let _ = client.dispose_browser_context(&ctx).await;
        // Deterministic cleanup on every path, same as `navigate_and_snapshot_inner`.
        client.close().await;
        result
    }

    async fn drive_login(
        &self,
        client: &CdpClient,
        url: &str,
        username: &str,
        password: &str,
        ctx: &str,
    ) -> Result<bool, EngineError> {
        let target_id = client
            .create_target_in("about:blank", Some(ctx))
            .await
            .map_err(cdp_err)?;
        let session_id = client.attach_to_target(&target_id).await.map_err(cdp_err)?;
        client.enable_page(&session_id).await.map_err(cdp_err)?;
        if let Err(e) = self.navigate_guarded(client, &session_id, url).await {
            let _ = client.close_target(&target_id).await;
            return Err(e);
        }

        // The credential belongs to the REQUESTED site: the fill script
        // refuses (before touching any field) when the page settled on
        // another site — an open redirect, a lapsed domain or a takeover must
        // not receive the password — or when the form would post it off-site
        // or downgrade it to plain http.
        let Some(expected) = LoginOrigin::of(url) else {
            let _ = client.close_target(&target_id).await;
            return Err(EngineError::Nav("login url has no host".into()));
        };

        // `serde_json::to_string` on a `&str` produces a properly-escaped
        // JSON string literal, which is also a valid JS string literal — the
        // one safe way to splice an arbitrary username/password into a JS
        // expression string without risking injection into the surrounding
        // script (see `fill_and_submit_expr`'s doc comment).
        let outcome = client
            .evaluate(
                &session_id,
                &fill_and_submit_expr(username, password, &expected),
            )
            .await
            .map_err(cdp_err)?;
        let refusal = match outcome.as_str() {
            Some("no-password-field") => Some("no password field found on login page"),
            Some("wrong-origin") => Some(
                "login page is served from a different site than the credential's domain; \
                 refusing to fill credentials",
            ),
            Some("wrong-form-action") => Some(
                "login form would submit to a different site (or over plain http); \
                 refusing to fill credentials",
            ),
            _ => None,
        };
        if let Some(message) = refusal {
            let _ = client.close_target(&target_id).await;
            return Err(EngineError::Nav(message.into()));
        }

        // Best-effort settle: a classic form submit fires a real navigation
        // (Page.loadEventFired); a JS/SPA login (fetch/XHR, no navigation)
        // never will, so a timeout here is expected, not an error — it just
        // means "check the DOM as it stands now" instead of "reload happened".
        // The pump keeps answering intercepted requests (the submit POST and
        // whatever it loads); a refused document hop is a hard error.
        if let Err(e @ CdpError::Blocked(_)) = client
            .pump_until_load(&session_id, Duration::from_secs(5), self.guard)
            .await
        {
            let _ = client.close_target(&target_id).await;
            return Err(cdp_err(e));
        }

        let still_present = client
            .evaluate(
                &session_id,
                "!!document.querySelector('input[type=\"password\"]')",
            )
            .await
            .map_err(cdp_err)?;
        let _ = client.close_target(&target_id).await;
        Ok(!still_present.as_bool().unwrap_or(true))
    }
}

fn unsupported_interception() -> EngineError {
    EngineError::Unavailable(
        "lightpanda cannot intercept requests (CDP Fetch domain), so its redirects and \
         subresources can't be SSRF-checked"
            .into(),
    )
}

/// The site a stored browser credential may be typed into, derived from the
/// login URL the route built (`https://{domain}/`).
struct LoginOrigin {
    /// Lower-cased host, trailing dot stripped.
    host: String,
    /// The login URL was https — the page and the form must stay https.
    require_https: bool,
}

impl LoginOrigin {
    fn of(url: &str) -> Option<Self> {
        let parsed = reqwest::Url::parse(url).ok()?;
        let host = parsed
            .host_str()?
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }
        Some(Self {
            host,
            require_https: parsed.scheme() == "https",
        })
    }

    /// Rust mirror of the in-page `okSite` check in [`fill_and_submit_expr`]
    /// (kept in lockstep; unit-tested here since the JS can't be).
    #[cfg(test)]
    fn allows(&self, protocol: &str, host: &str) -> bool {
        let h = host.trim_end_matches('.').to_ascii_lowercase();
        if self.require_https && protocol != "https:" {
            return false;
        }
        h == self.host
            || (h.len() > self.host.len() + 1 && h.ends_with(&format!(".{}", self.host)))
            || self.host == format!("www.{h}")
    }
}

/// Builds the JS expression `drive_login` evaluates in the target page to
/// fill and submit a login form. `username`/`password` are embedded via
/// `serde_json::to_string` (JSON string-literal escaping, which is also
/// valid JS string-literal escaping for the characters that matter here —
/// quotes, backslashes, control characters) rather than any hand-rolled
/// escaping, so a credential containing `"`, `\`, or a newline can't break
/// out of the string literal into surrounding script.
///
/// Before touching any field it checks, in the page, that the document it
/// settled on belongs to `expected` (same host, a subdomain of it, or the bare
/// domain of a `www.` login host — the same rule as [`LoginOrigin::allows`])
/// and, for an https login, is still https; and that the enclosing form's
/// `action` satisfies the same rule. Returns `'wrong-origin'` /
/// `'wrong-form-action'` instead of filling when it doesn't.
fn fill_and_submit_expr(username: &str, password: &str, expected: &LoginOrigin) -> String {
    let user_js = serde_json::to_string(username).unwrap_or_else(|_| "\"\"".to_string());
    let pass_js = serde_json::to_string(password).unwrap_or_else(|_| "\"\"".to_string());
    let host_js = serde_json::to_string(&expected.host).unwrap_or_else(|_| "\"\"".to_string());
    let https_js = if expected.require_https { "true" } else { "false" };
    format!(
        "(function(){{\
           var EXP = {host_js};\
           var REQ_HTTPS = {https_js};\
           var okSite = function(proto, h) {{\
             h = String(h || '').toLowerCase();\
             if (h.charAt(h.length - 1) === '.') h = h.slice(0, -1);\
             if (REQ_HTTPS && proto !== 'https:') return false;\
             return h === EXP || \
                    (h.length > EXP.length + 1 && h.slice(-(EXP.length + 1)) === '.' + EXP) || \
                    ('www.' + h) === EXP;\
           }};\
           if (!okSite(location.protocol, location.hostname)) return 'wrong-origin';\
           var pwd = document.querySelector('input[type=\"password\"]');\
           if (!pwd) return 'no-password-field';\
           var owner = pwd.closest('form');\
           if (owner) {{\
             var act;\
             try {{ act = new URL(owner.getAttribute('action') || location.href, location.href); }}\
             catch (e) {{ return 'wrong-form-action'; }}\
             if (!okSite(act.protocol, act.hostname)) return 'wrong-form-action';\
           }}\
           var user = document.querySelector('input[type=\"email\"]') || \
                      document.querySelector('input[autocomplete=\"username\"]') || \
                      document.querySelector('input[type=\"text\"]');\
           if (user) {{\
             user.focus();\
             user.value = {user_js};\
             user.dispatchEvent(new Event('input', {{bubbles: true}}));\
             user.dispatchEvent(new Event('change', {{bubbles: true}}));\
           }}\
           pwd.focus();\
           pwd.value = {pass_js};\
           pwd.dispatchEvent(new Event('input', {{bubbles: true}}));\
           pwd.dispatchEvent(new Event('change', {{bubbles: true}}));\
           var form = pwd.closest('form');\
           if (form) {{\
             if (typeof form.requestSubmit === 'function') form.requestSubmit();\
             else form.submit();\
           }} else {{\
             var btn = document.querySelector('button[type=\"submit\"]') || \
                       document.querySelector('input[type=\"submit\"]');\
             if (btn) btn.click();\
           }}\
           return 'submitted';\
         }})()"
    )
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

    /// Caller must netguard-check `url` first — see crate docs.
    async fn login(&self, url: &str, username: &str, password: &str) -> Result<bool, EngineError> {
        self.login_flow(url, username, password).await
    }

    fn name(&self) -> &'static str {
        "lightpanda"
    }

    fn is_usable(&self) -> bool {
        self.interception_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_origin_binds_to_the_credential_site() {
        let o = LoginOrigin::of("https://Example.com./").unwrap();
        assert_eq!(o.host, "example.com");
        assert!(o.allows("https:", "example.com"));
        assert!(o.allows("https:", "login.example.com"));
        assert!(o.allows("https:", "EXAMPLE.COM."));
        // Off-site redirects (open redirect, lapsed domain, look-alikes).
        assert!(!o.allows("https:", "evil.com"));
        assert!(!o.allows("https:", "example.com.evil.com"));
        assert!(!o.allows("https:", "notexample.com"));
        // Downgrade to plain http.
        assert!(!o.allows("http:", "example.com"));
        // A www. login host may settle on its bare domain, not vice versa.
        let w = LoginOrigin::of("https://www.example.com/").unwrap();
        assert!(w.allows("https:", "example.com"));
        assert!(!o.allows("https:", "com"));
        // An http login (fixtures only) doesn't demand https.
        let h = LoginOrigin::of("http://127.0.0.1:8080/login").unwrap();
        assert!(h.allows("http:", "127.0.0.1"));
        assert!(!h.allows("http:", "127.0.0.2"));
    }

    #[test]
    fn fill_script_embeds_the_expected_site_before_any_field() {
        let o = LoginOrigin::of("https://example.com/").unwrap();
        let js = fill_and_submit_expr("u", "p\"w", &o);
        let origin_check = js.find("'wrong-origin'").unwrap();
        let fill = js.find("pwd.value").unwrap();
        assert!(origin_check < fill, "origin must be checked before filling");
        assert!(js.contains("var EXP = \"example.com\""));
        assert!(js.contains("var REQ_HTTPS = true"));
        assert!(js.contains("'wrong-form-action'"));
        // Credentials stay JSON-escaped.
        assert!(js.contains("\"p\\\"w\""));
    }

    #[tokio::test]
    async fn request_vetting_blocks_internal_targets() {
        assert!(!request_allowed("http://127.0.0.1:7700/api/v1/sessions").await);
        assert!(!request_allowed("http://169.254.169.254/latest/meta-data/").await);
        assert!(!request_allowed("http://[::1]/").await);
        assert!(!request_allowed("file:///etc/passwd").await);
        assert!(!request_allowed("not a url").await);
        assert!(request_allowed("data:text/plain,hi").await);
        assert!(request_allowed("about:blank").await);
        assert!(request_allowed("https://8.8.8.8/").await);
    }

    #[test]
    fn engine_usability_latches_only_for_guarded_engines() {
        let guarded = LightpandaEngine::new("ws://127.0.0.1:1".into());
        assert!(guarded.is_usable());
        guarded
            .interception_unsupported
            .store(true, Ordering::Relaxed);
        assert!(!guarded.is_usable());
        let fixture = LightpandaEngine::new_unguarded("ws://127.0.0.1:1".into());
        fixture
            .interception_unsupported
            .store(true, Ordering::Relaxed);
        assert!(fixture.is_usable());
    }

    /// A scripted lightpanda stand-in on a loopback WebSocket: answers the
    /// handful of CDP calls the engine makes and records every method (and
    /// the `createTarget` params), so the per-call context isolation is
    /// asserted without a real sidecar.
    async fn fake_lightpanda(
        support_contexts: bool,
    ) -> (String, Arc<std::sync::Mutex<Vec<(String, Value)>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log: Arc<std::sync::Mutex<Vec<(String, Value)>>> = Arc::default();
        let log2 = log.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let log = log2.clone();
                tokio::spawn(async move {
                    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                    while let Some(Ok(Message::Text(t))) = ws.next().await {
                        let v: Value = serde_json::from_str(&t).unwrap();
                        let method = v["method"].as_str().unwrap_or("").to_string();
                        log.lock().unwrap().push((method.clone(), v["params"].clone()));
                        let id = v["id"].clone();
                        let reply = match method.as_str() {
                            "Target.createBrowserContext" if !support_contexts => json!({
                                "id": id, "error": {"code": -32601, "message": "unsupported"}
                            }),
                            "Target.createBrowserContext" => {
                                json!({"id": id, "result": {"browserContextId": "CTX1"}})
                            }
                            "Target.createTarget" => json!({"id": id, "result": {"targetId": "T1"}}),
                            "Target.attachToTarget" => json!({"id": id, "result": {"sessionId": "S1"}}),
                            "Runtime.evaluate" => {
                                let expr = v["params"]["expression"].as_str().unwrap_or("");
                                let value = if expr == "location.href" {
                                    json!("https://8.8.8.8/")
                                } else {
                                    json!("<html><head><title>ok</title></head><body>hi</body></html>")
                                };
                                json!({"id": id, "result": {"result": {"value": value}}})
                            }
                            _ => json!({"id": id, "result": {}}),
                        };
                        ws.send(Message::Text(reply.to_string())).await.unwrap();
                        if method == "Page.navigate" {
                            let ev = json!({"method": "Page.loadEventFired", "sessionId": "S1", "params": {}});
                            ws.send(Message::Text(ev.to_string())).await.unwrap();
                        }
                    }
                });
            }
        });
        (format!("ws://127.0.0.1:{port}"), log)
    }

    #[tokio::test]
    async fn every_fetch_runs_in_its_own_disposed_browser_context() {
        let (url, log) = fake_lightpanda(true).await;
        let engine = LightpandaEngine::new(url);
        let page = engine.fetch_page("https://8.8.8.8/").await.unwrap();
        assert_eq!(page.title, "ok");
        let log = log.lock().unwrap().clone();
        let pos = |m: &str| log.iter().position(|(x, _)| x == m);
        let created = pos("Target.createBrowserContext").expect("context created");
        let target = pos("Target.createTarget").expect("target created");
        assert!(created < target);
        assert_eq!(log[target].1["browserContextId"], "CTX1");
        let disposed = log
            .iter()
            .find(|(m, _)| m == "Target.disposeBrowserContext")
            .expect("context disposed");
        assert_eq!(disposed.1["browserContextId"], "CTX1");
    }

    #[tokio::test]
    async fn reads_fall_back_but_login_refuses_without_context_isolation() {
        let (url, log) = fake_lightpanda(false).await;
        let engine = LightpandaEngine::new(url.clone());
        // A read still works in the connection's default context.
        assert!(engine.fetch_page("https://8.8.8.8/").await.is_ok());
        let target = log
            .lock()
            .unwrap()
            .iter()
            .find(|(m, _)| m == "Target.createTarget")
            .cloned()
            .unwrap();
        assert!(target.1.get("browserContextId").is_none());
        // A credential is never typed into a shared context.
        let engine = LightpandaEngine::new(url);
        let err = engine
            .login("https://8.8.8.8/", "user", "secret")
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::Unavailable(_)));
        assert!(!err.to_string().contains("secret"));
    }
}
