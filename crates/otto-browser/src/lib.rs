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

pub mod engine;
pub mod extract;

pub use engine::{BrowserEngine, EngineError, MatchedNode, Page, PAGE_BYTE_CAP, PAGE_TIMEOUT_SECS};
pub use extract::{html_to_markdown, readability};

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

    async fn raw_html(&self, url: &str) -> Result<String, EngineError> {
        if let Some(body) = &self.static_body {
            return Ok(body.clone());
        }
        let send = self.client.get(url).send();
        let resp = tokio::time::timeout(Duration::from_secs(PAGE_TIMEOUT_SECS), send)
            .await
            .map_err(|_| EngineError::Timeout(PAGE_TIMEOUT_SECS))?
            .map_err(|e| EngineError::Nav(e.to_string()))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| EngineError::Nav(e.to_string()))?;
        if bytes.len() > PAGE_BYTE_CAP {
            return Err(EngineError::TooLarge(PAGE_BYTE_CAP));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

impl Default for FallbackEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl BrowserEngine for FallbackEngine {
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

fn extract_title(html: &str) -> String {
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

    /// Navigate and return the settled page, falling back to plain-fetch
    /// when the primary engine is unavailable (or the host is denylisted).
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
        let svc = BrowserService::with_engines(Arc::new(Down), FallbackEngine::from_static("<h1>Hi</h1>"));
        let page = svc.page("https://example.com").await.unwrap();
        assert!(page.degraded);
        assert_eq!(page.engine, "fallback");
        assert!(page.markdown.contains("Hi"));
    }

    #[tokio::test]
    async fn service_propagates_non_unavailable_errors() {
        let svc = BrowserService::with_engines(Arc::new(AlwaysNav), FallbackEngine::from_static("<h1>Hi</h1>"));
        let err = svc.page("https://example.com").await.unwrap_err();
        assert!(matches!(err, EngineError::Nav(_)));
    }

    #[tokio::test]
    async fn host_is_denylisted_after_three_failures() {
        let svc = BrowserService::with_engines(Arc::new(Down), FallbackEngine::from_static("<h1>Hi</h1>"));
        for _ in 0..DENYLIST_THRESHOLD {
            svc.page("https://flaky.example.com").await.unwrap();
        }
        assert!(svc.is_denylisted("flaky.example.com"));
        // Still resolves fine — straight to fallback, no engine call needed.
        let page = svc.page("https://flaky.example.com").await.unwrap();
        assert_eq!(page.engine, "fallback");
    }
}
