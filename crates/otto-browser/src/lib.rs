//! Browser engine abstraction: navigate a URL, extract markdown, and fall
//! back to a plain `reqwest` fetch (no JS execution — `degraded: true`) when
//! the primary engine (e.g. lightpanda) is unavailable or a host keeps
//! failing against it.
//!
//! Callers are responsible for netguard-checking any user-supplied URL
//! (`otto_netguard::is_blocked_ip` et al.) *before* it reaches
//! [`BrowserService::page`] or [`FallbackEngine`] — this crate does not
//! resolve/re-check hosts itself, to keep the SSRF policy defined exactly
//! once (see `otto-netguard`).

pub mod cdp;
pub mod engine;
pub mod extract;
pub mod lightpanda;

pub use cdp::LightpandaEngine;
pub use engine::{BrowserEngine, EngineError, MatchedNode, Page, PAGE_BYTE_CAP, PAGE_TIMEOUT_SECS};
pub use extract::{html_to_markdown, readability};
pub use lightpanda::Lightpanda;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use scraper::{Html, Selector};

/// Consecutive engine failures for a host before we stop trying it and go
/// straight to the fallback engine.
const DENYLIST_THRESHOLD: u32 = 3;

/// Plain-fetch backend: a bare `reqwest` GET, readability-cleaned and
/// converted to markdown. Never runs scripts, so every page it returns is
/// `degraded: true`.
pub struct FallbackEngine {
    client: reqwest::Client,
    /// Test-only canned body, set via [`FallbackEngine::from_static`]. When
    /// present, `fetch_page`/`query` skip the network entirely.
    static_body: Option<String>,
}

impl FallbackEngine {
    /// Caller must netguard-check `url` first — see crate docs.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            static_body: None,
        }
    }

    /// Test constructor: serve `body` for every URL instead of fetching.
    pub fn from_static(body: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            static_body: Some(body.to_string()),
        }
    }

    /// Caller must netguard-check `url` first — see crate docs.
    ///
    /// Streams the response body and aborts as soon as the running byte
    /// count passes [`PAGE_BYTE_CAP`], so peak memory is actually bounded —
    /// a hostile/huge response never gets fully buffered before we notice.
    async fn raw_html(&self, url: &str) -> Result<String, EngineError> {
        if let Some(body) = &self.static_body {
            return Ok(body.clone());
        }
        let send = self.client.get(url).send();
        let resp = tokio::time::timeout(Duration::from_secs(PAGE_TIMEOUT_SECS), send)
            .await
            .map_err(|_| EngineError::Timeout(PAGE_TIMEOUT_SECS))?
            .map_err(|e| EngineError::Nav(e.to_string()))?;

        let mut buf: Vec<u8> = Vec::new();
        let mut stream = resp.bytes_stream();
        let deadline = Duration::from_secs(PAGE_TIMEOUT_SECS);
        loop {
            let next = tokio::time::timeout(deadline, stream.next())
                .await
                .map_err(|_| EngineError::Timeout(PAGE_TIMEOUT_SECS))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| EngineError::Nav(e.to_string()))?;
            if buf.len() + chunk.len() > PAGE_BYTE_CAP {
                return Err(EngineError::TooLarge(PAGE_BYTE_CAP));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
}

impl Default for FallbackEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl BrowserEngine for FallbackEngine {
    /// Caller must netguard-check `url` first — see crate docs.
    async fn fetch_page(&self, url: &str) -> Result<Page, EngineError> {
        let html = self.raw_html(url).await?;
        if html.len() > PAGE_BYTE_CAP {
            return Err(EngineError::TooLarge(PAGE_BYTE_CAP));
        }
        let title = extract_title(&html);
        let cleaned = readability(&html);
        let markdown = html_to_markdown(&cleaned);
        Ok(Page {
            url: url.to_string(),
            title,
            html,
            markdown,
            degraded: true,
            engine: self.name().to_string(),
        })
    }

    async fn query(&self, url: &str, selector: &str) -> Result<Vec<MatchedNode>, EngineError> {
        let html = self.raw_html(url).await?;
        let document = Html::parse_document(&html);
        let sel = Selector::parse(selector)
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
        "fallback"
    }
}

pub(crate) fn extract_title(html: &str) -> String {
    let document = Html::parse_document(html);
    let sel = Selector::parse("title").expect("static selector");
    document
        .select(&sel)
        .next()
        .map(|el| el.text().collect::<String>())
        .unwrap_or_default()
}

/// Fronts a primary [`BrowserEngine`] with the plain-fetch [`FallbackEngine`].
/// Falls back immediately on `EngineError::Unavailable`, and after
/// [`DENYLIST_THRESHOLD`] consecutive `Unavailable` failures for a host,
/// skips the primary engine entirely for that host until it succeeds again.
pub struct BrowserService {
    engine: Arc<dyn BrowserEngine>,
    fallback: FallbackEngine,
    denylist: Mutex<HashMap<String, u32>>,
}

impl BrowserService {
    pub fn with_engines(engine: Arc<dyn BrowserEngine>, fallback: FallbackEngine) -> Self {
        Self {
            engine,
            fallback,
            denylist: Mutex::new(HashMap::new()),
        }
    }

    /// Locate + start a lightpanda sidecar and use it as the primary engine;
    /// falls back to plain-fetch-only (as both primary and fallback) when no
    /// binary is found or the sidecar fails to start. Never errors — a
    /// missing/broken lightpanda degrades gracefully rather than blocking
    /// startup.
    pub async fn autodetect(configured_bin: Option<&str>, data_dir: std::path::PathBuf) -> Self {
        if let Some(bin) = Lightpanda::locate(configured_bin) {
            match Lightpanda::start(bin, data_dir).await {
                Ok(sidecar) => {
                    let engine = LightpandaEngine::new(sidecar.cdp_url());
                    return Self::with_engines(
                        Arc::new(SidecarBackedEngine {
                            engine,
                            _sidecar: sidecar,
                        }),
                        FallbackEngine::new(),
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        "browser: lightpanda sidecar failed to start, using plain fetch: {e}"
                    );
                }
            }
        } else {
            tracing::info!("browser: no lightpanda binary found, using plain fetch");
        }
        Self::with_engines(Arc::new(FallbackEngine::new()), FallbackEngine::new())
    }

    /// Navigate and return the settled page, falling back to plain-fetch
    /// when the primary engine is unavailable (or the host is denylisted).
    ///
    /// Caller must netguard-check `url` first — see crate docs.
    pub async fn page(&self, url: &str) -> Result<Page, EngineError> {
        let host = host_of(url);
        if self.is_denylisted(&host) {
            return self.fallback.fetch_page(url).await;
        }
        match self.engine.fetch_page(url).await {
            Ok(page) => {
                self.clear_failures(&host);
                Ok(page)
            }
            Err(EngineError::Unavailable(_)) => {
                self.record_failure(&host);
                self.fallback.fetch_page(url).await
            }
            Err(e) => Err(e),
        }
    }

    fn is_denylisted(&self, host: &str) -> bool {
        let denylist = self.denylist.lock().expect("denylist mutex poisoned");
        denylist.get(host).copied().unwrap_or(0) >= DENYLIST_THRESHOLD
    }

    fn record_failure(&self, host: &str) {
        let mut denylist = self.denylist.lock().expect("denylist mutex poisoned");
        *denylist.entry(host.to_string()).or_insert(0) += 1;
    }

    fn clear_failures(&self, host: &str) {
        let mut denylist = self.denylist.lock().expect("denylist mutex poisoned");
        denylist.remove(host);
    }
}

/// Pairs a [`LightpandaEngine`] with the [`Lightpanda`] sidecar it talks to,
/// so the sidecar (and its restart supervisor) stays alive for as long as
/// the engine — and only that long.
struct SidecarBackedEngine {
    engine: LightpandaEngine,
    _sidecar: Lightpanda,
}

#[async_trait::async_trait]
impl BrowserEngine for SidecarBackedEngine {
    async fn fetch_page(&self, url: &str) -> Result<Page, EngineError> {
        self.engine.fetch_page(url).await
    }

    async fn query(&self, url: &str, selector: &str) -> Result<Vec<MatchedNode>, EngineError> {
        self.engine.query(url, selector).await
    }

    fn name(&self) -> &'static str {
        self.engine.name()
    }
}

fn host_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Down;

    #[async_trait::async_trait]
    impl BrowserEngine for Down {
        async fn fetch_page(&self, _: &str) -> Result<Page, EngineError> {
            Err(EngineError::Unavailable("down".into()))
        }
        async fn query(&self, _: &str, _: &str) -> Result<Vec<MatchedNode>, EngineError> {
            Err(EngineError::Unavailable("down".into()))
        }
        fn name(&self) -> &'static str {
            "mock"
        }
    }

    struct AlwaysNav;

    #[async_trait::async_trait]
    impl BrowserEngine for AlwaysNav {
        async fn fetch_page(&self, _: &str) -> Result<Page, EngineError> {
            Err(EngineError::Nav("boom".into()))
        }
        async fn query(&self, _: &str, _: &str) -> Result<Vec<MatchedNode>, EngineError> {
            Err(EngineError::Nav("boom".into()))
        }
        fn name(&self) -> &'static str {
            "mock"
        }
    }

    #[tokio::test]
    async fn service_falls_back_when_engine_unavailable() {
        let svc = BrowserService::with_engines(
            Arc::new(Down),
            FallbackEngine::from_static("<h1>Hi</h1>"),
        );
        let page = svc.page("https://example.com").await.unwrap();
        assert!(page.degraded);
        assert_eq!(page.engine, "fallback");
        assert!(page.markdown.contains("Hi"));
    }

    #[tokio::test]
    async fn service_propagates_non_unavailable_errors() {
        let svc = BrowserService::with_engines(
            Arc::new(AlwaysNav),
            FallbackEngine::from_static("<h1>Hi</h1>"),
        );
        let err = svc.page("https://example.com").await.unwrap_err();
        assert!(matches!(err, EngineError::Nav(_)));
    }

    /// The 2 MB cap must be enforced while STREAMING the body, not only
    /// after buffering it whole — spin up a tiny raw TCP/HTTP server that
    /// chunks out well over the cap and confirm `FallbackEngine` aborts with
    /// `TooLarge` (rather than OOMing on a fully-buffered multi-MB body).
    #[tokio::test]
    async fn raw_html_aborts_when_response_exceeds_byte_cap() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await; // drain the request, don't care about it

            let header =
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
            if socket.write_all(header.as_bytes()).await.is_err() {
                return;
            }

            // Stream well past PAGE_BYTE_CAP in small chunks so the server
            // keeps writing after the client is expected to have aborted.
            let chunk = vec![b'a'; 64 * 1024];
            let mut sent = 0usize;
            while sent < PAGE_BYTE_CAP * 2 {
                let size_line = format!("{:x}\r\n", chunk.len());
                if socket.write_all(size_line.as_bytes()).await.is_err()
                    || socket.write_all(&chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return; // client dropped the connection — expected once it aborts
                }
                sent += chunk.len();
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });

        let engine = FallbackEngine::new();
        let url = format!("http://{addr}/huge");
        let err = engine.fetch_page(&url).await.unwrap_err();
        assert!(matches!(err, EngineError::TooLarge(cap) if cap == PAGE_BYTE_CAP));
    }

    #[tokio::test]
    async fn autodetect_falls_back_to_plain_fetch_without_a_lightpanda_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = BrowserService::autodetect(
            Some("/definitely/not/a/real/lightpanda/binary"),
            tmp.path().into(),
        )
        .await;
        let page = svc
            .page("https://example.com/nonexistent-host-for-test")
            .await;
        // Either the plain fetch fails on DNS (fine — no network in CI) or it
        // succeeds; either way the primary engine must be "fallback", never a
        // lightpanda engine we never actually started.
        match page {
            Ok(p) => assert_eq!(p.engine, "fallback"),
            Err(e) => assert!(matches!(e, EngineError::Nav(_) | EngineError::Timeout(_))),
        }
    }

    #[tokio::test]
    async fn host_is_denylisted_after_three_failures() {
        let svc = BrowserService::with_engines(
            Arc::new(Down),
            FallbackEngine::from_static("<h1>Hi</h1>"),
        );
        for _ in 0..DENYLIST_THRESHOLD {
            svc.page("https://flaky.example.com").await.unwrap();
        }
        assert!(svc.is_denylisted("flaky.example.com"));
        // Still resolves fine — straight to fallback, no engine call needed.
        let page = svc.page("https://flaky.example.com").await.unwrap();
        assert_eq!(page.engine, "fallback");
    }
}
