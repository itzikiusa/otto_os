//! The one Atlassian HTTP transport, shared by [`crate::jira::JiraClient`] and
//! [`crate::confluence::ConfluenceClient`]: both authenticate with the same
//! `email:token` Basic credential from the same `IssueAccount`, so every
//! transport rule lives here ONCE — the bounded client cache, the cleartext
//! refusal and the status → `Error::Upstream` mapping. Jira used to keep its
//! own copy without the TLS guard Confluence had, so an `http://` account sent
//! the API token in cleartext on every Jira call (architecture-3-02).
//!
//! The clients keep only their path builders and parsers; every request they
//! build starts from [`AtlassianTransport::http`], which refuses a base URL
//! that would carry the Basic header in cleartext (plain http stays allowed
//! for loopback dev servers and the test fixtures).

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use otto_core::{Error, Result};

/// How long to wait for the TCP/TLS connection to an endpoint.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Overall per-request deadline so a hung endpoint can't block the caller.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Most cached clients (one per site + credential). A rotated token or an
/// edited base URL makes a new key; past this the oldest entry goes, so the
/// cache can't grow for the daemon's lifetime.
const CLIENT_CACHE_MAX: usize = 32;
/// Upstream error bodies are cut to this many chars in the error text.
const BODY_SNIPPET_MAX: usize = 500;

struct CachedClient {
    client: reqwest::Client,
    used: u64,
}

/// key hash → client. The key (site + credential) is hashed so the cache
/// holds no extra copy of a secret.
static CLIENT_CACHE: OnceLock<Mutex<(u64, HashMap<u64, CachedClient>)>> = OnceLock::new();

fn build_client(https_only: bool) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .https_only(https_only)
        .build()
        .unwrap_or_default()
}

/// A shared `reqwest::Client` for `key`, built on first use and kept in a
/// bounded (least-recently-used) cache so its connection pool is reused.
/// Falls back to a fresh client when the lock is poisoned (callers stay
/// infallible). Linear uses it with a fixed key.
pub(crate) fn cached_client(key: &str) -> reqwest::Client {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    let k = h.finish();
    let cache = CLIENT_CACHE.get_or_init(|| Mutex::new((0, HashMap::new())));
    let Ok(mut guard) = cache.lock() else {
        return build_client(false);
    };
    let (clock, map) = &mut *guard;
    *clock += 1;
    if let Some(c) = map.get_mut(&k) {
        c.used = *clock;
        return c.client.clone();
    }
    while map.len() >= CLIENT_CACHE_MAX {
        let oldest = map.iter().min_by_key(|(_, c)| c.used).map(|(k, _)| *k);
        match oldest {
            Some(o) => map.remove(&o),
            None => break,
        };
    }
    let client = build_client(false);
    map.insert(
        k,
        CachedClient {
            client: client.clone(),
            used: *clock,
        },
    );
    client
}

/// The mapped error for a non-2xx answer: `<what> failed (<status>): <body…>`.
pub(crate) fn upstream_status(what: &str, status: reqwest::StatusCode, body: &str) -> Error {
    let body = body.trim();
    let snippet: String = body.chars().take(BODY_SNIPPET_MAX).collect();
    let more = if body.chars().count() > BODY_SNIPPET_MAX {
        "…"
    } else {
        ""
    };
    Error::Upstream(format!("{what} failed ({status}): {snippet}{more}"))
}

/// One Atlassian site + credential: the base URL, the Basic header, the
/// shared client, and why the site is refused (cleartext), if it is.
pub(crate) struct AtlassianTransport {
    /// Site base without a trailing `/` (Jira: as entered; Confluence: with
    /// `/wiki` stripped).
    pub(crate) base: String,
    /// `Basic base64(email:token)`.
    pub(crate) auth_header: String,
    http: reqwest::Client,
    /// `Some` when the base would carry the header over plain http to a
    /// non-loopback host — every request is refused before it is built.
    transport_err: Option<String>,
    /// "jira" / "confluence" — the error-text prefix.
    product: &'static str,
}

impl AtlassianTransport {
    pub(crate) fn new(product: &'static str, base: String, email: &str, token: &str) -> Self {
        let auth_header = format!(
            "Basic {}",
            B64.encode(format!("{email}:{token}").as_bytes())
        );
        let transport_err = otto_netguard::require_tls_or_loopback(&base).err();
        // Refused sites get an https-only client too: a backstop should any
        // path ever reach the client without `http()`.
        let http = if transport_err.is_some() {
            build_client(true)
        } else {
            cached_client(&format!("{base}\x00{auth_header}"))
        };
        Self {
            base,
            auth_header,
            http,
            transport_err,
            product,
        }
    }

    /// Fail before sending when the base URL would carry the auth header in
    /// cleartext.
    pub(crate) fn ensure_tls(&self) -> Result<()> {
        match &self.transport_err {
            None => Ok(()),
            Some(e) => Err(Error::Invalid(format!("{} base url: {e}", self.product))),
        }
    }

    /// The client to build a request with — refused for a cleartext site.
    /// EVERY request starts here, so the TLS rule can't be skipped.
    pub(crate) fn http(&self) -> Result<&reqwest::Client> {
        self.ensure_tls()?;
        Ok(&self.http)
    }

    /// Send `req`, mapping a transport failure and a non-2xx status to
    /// `Error::Upstream` (`what` names the call, e.g. "jira get transitions").
    pub(crate) async fn send(
        &self,
        req: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response> {
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("{what} request: {e}")))?;
        if resp.status().is_success() {
            return Ok(resp);
        }
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(upstream_status(what, status, &body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleartext_sites_are_refused_loopback_and_https_are_not() {
        let t = AtlassianTransport::new("jira", "http://example.com".into(), "e", "t");
        let err = t.http().expect_err("http:// is refused");
        assert!(err.to_string().contains("jira base url"), "{err}");
        assert!(
            AtlassianTransport::new("jira", "https://x.atlassian.net".into(), "e", "t")
                .http()
                .is_ok()
        );
        assert!(
            AtlassianTransport::new("jira", "http://127.0.0.1:9".into(), "e", "t")
                .http()
                .is_ok()
        );
    }

    #[test]
    fn the_client_cache_is_bounded() {
        for i in 0..(CLIENT_CACHE_MAX + 10) {
            let _ = cached_client(&format!("bounded-test-{i}"));
        }
        let len = CLIENT_CACHE.get().unwrap().lock().unwrap().1.len();
        assert!(len <= CLIENT_CACHE_MAX, "{len}");
    }

    #[test]
    fn upstream_errors_carry_a_bounded_body_snippet() {
        let long = "x".repeat(BODY_SNIPPET_MAX + 50);
        let e = upstream_status("jira get", reqwest::StatusCode::BAD_GATEWAY, &long).to_string();
        assert!(e.contains("jira get failed (502 Bad Gateway)"), "{e}");
        assert!(e.ends_with('…'));
        assert!(e.len() < long.len());
    }
}
