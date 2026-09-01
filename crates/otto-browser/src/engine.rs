//! The `BrowserEngine` trait every navigation backend implements (a real
//! headless engine like lightpanda, the plain-fetch `FallbackEngine`, or a
//! test `MockEngine`), plus the page/error types they exchange.

/// Hard cap on page bytes any engine will hold in memory — a runaway page
/// (infinite scroll, a multi-GB response) must not OOM the daemon.
pub const PAGE_BYTE_CAP: usize = 2 * 1024 * 1024;

/// Wall-clock budget for a single `fetch_page`/`query` call.
pub const PAGE_TIMEOUT_SECS: u64 = 30;

/// A settled page: raw + extracted content, plus which engine produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub html: String,
    pub markdown: String,
    /// `true` when the engine could not run scripts the page needed (the
    /// plain-fetch fallback always sets this — it never executes JS).
    pub degraded: bool,
    /// Name of the engine that produced this page (`BrowserEngine::name()`).
    pub engine: String,
}

/// One selector match returned by `BrowserEngine::query`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchedNode {
    pub selector: String,
    pub outer_html: String,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine unavailable: {0}")]
    Unavailable(String),
    #[error("navigation failed: {0}")]
    Nav(String),
    #[error("page too large (cap {0} bytes)")]
    TooLarge(usize),
    #[error("timed out after {0}s")]
    Timeout(u64),
}

/// A navigation backend. Implementors fetch/render a URL and answer selector
/// queries against the settled DOM.
#[async_trait::async_trait]
pub trait BrowserEngine: Send + Sync {
    /// Navigate and return the settled page.
    ///
    /// Caller must netguard-check `url` first — see crate docs.
    async fn fetch_page(&self, url: &str) -> Result<Page, EngineError>;
    async fn query(&self, url: &str, selector: &str) -> Result<Vec<MatchedNode>, EngineError>;
    /// Stable engine identifier: `"lightpanda"` | `"fallback"` | `"mock"`.
    fn name(&self) -> &'static str;
}
