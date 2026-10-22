//! The one Atlassian HTTP transport, shared by [`crate::jira::JiraClient`] and
//! [`crate::confluence::ConfluenceClient`]: both authenticate with the same
//! `email:token` Basic credential from the same `IssueAccount`, so every
//! transport rule lives here ONCE — the bounded client cache, the cleartext
//! refusal, the one-retry rate-limit handling and the status → error mapping
//! (429 → "rate limited — retry in Ns", 401/403 → `Forbidden` with a token
//! hint, anything else → a bounded `Error::Upstream`). Jira used to keep its
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
pub(crate) const BODY_SNIPPET_MAX: usize = 500;
/// Longest rate-limit wait a READ absorbs (once) before the error names the
/// wait instead — the same bound the git providers' client retries within.
const RATE_LIMIT_RETRY_MAX: Duration = Duration::from_secs(10);

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
    let snippet = otto_core::text::clip_chars(body.trim(), BODY_SNIPPET_MAX);
    Error::Upstream(format!("{what} failed ({status}): {snippet}"))
}

/// `Some(wait)` when the answer is a rate-limit refusal: 429, or 503 with a
/// `Retry-After`. `wait` comes from `Retry-After` (delta-seconds or an
/// HTTP-date); missing / unparsable → 1 s.
///
/// The same rule as the git providers' `rate_limit_wait`
/// (`otto-git/src/providers/client.rs`), kept local because otto-issues does
/// not depend on otto-git — a candidate for a shared otto-core helper.
pub(crate) fn rate_limit_wait(
    status: u16,
    headers: &reqwest::header::HeaderMap,
) -> Option<Duration> {
    let retry_after = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);
    if status != 429 && !(status == 503 && retry_after.is_some()) {
        return None;
    }
    if let Some(v) = retry_after {
        if let Ok(secs) = v.parse::<u64>() {
            return Some(Duration::from_secs(secs));
        }
        if let Ok(when) = chrono::DateTime::parse_from_rfc2822(v) {
            let secs = (when.timestamp() - chrono::Utc::now().timestamp()).max(0);
            return Some(Duration::from_secs(secs as u64));
        }
    }
    Some(Duration::from_secs(1))
}

/// Classify a non-2xx Atlassian answer instead of collapsing everything into
/// `Error::Upstream` (a 502 the UI reads as "the site is down"):
/// - a rate-limit refusal names the wait (`<product> rate limited — retry in Ns`);
/// - 401/403 is `Forbidden` with a token hint (an expired or revoked API
///   token, or an account without access), so the UI can say "reconnect";
/// - anything else stays `Upstream` with a bounded body snippet.
pub(crate) fn status_error(
    product: &str,
    what: &str,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> Error {
    if let Some(wait) = rate_limit_wait(status.as_u16(), headers) {
        return Error::Upstream(format!(
            "{product} rate limited — retry in {}s",
            wait.as_secs().max(1)
        ));
    }
    match status.as_u16() {
        401 | 403 => {
            let snippet = otto_core::text::clip_chars(body.trim(), BODY_SNIPPET_MAX);
            Error::Forbidden(format!(
                "{what} was refused ({status}): {snippet} — the {product} account's API token was \
                 rejected or lacks access; update the token on the issue account"
            ))
        }
        _ => upstream_status(what, status, body),
    }
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

    /// Send `req`, mapping a transport failure to `Error::Upstream` (`what`
    /// names the call, e.g. "jira get transitions"), and return the answer
    /// whatever its status — for callers that branch on one (a 404 endpoint
    /// fallback, a 409 version conflict) before [`Self::check`]. A READ
    /// (GET/HEAD) refused with a short rate-limit wait (≤ 10 s) is re-sent
    /// once after that wait; writes are never re-sent.
    pub(crate) async fn send_raw(
        &self,
        req: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response> {
        let (client, request) = req.build_split();
        let request = request.map_err(|e| Error::Upstream(format!("{what} request: {e}")))?;
        let method = request.method();
        let read = *method == reqwest::Method::GET || *method == reqwest::Method::HEAD;
        let again = if read { request.try_clone() } else { None };
        let resp = client
            .execute(request)
            .await
            .map_err(|e| Error::Upstream(format!("{what} request: {e}")))?;
        let wait = rate_limit_wait(resp.status().as_u16(), resp.headers());
        match (again, wait) {
            (Some(again), Some(wait)) if wait <= RATE_LIMIT_RETRY_MAX => {
                tracing::debug!(
                    what,
                    wait_secs = wait.as_secs(),
                    "atlassian: rate limited, retrying once"
                );
                tokio::time::sleep(wait).await;
                client
                    .execute(again)
                    .await
                    .map_err(|e| Error::Upstream(format!("{what} request: {e}")))
            }
            _ => Ok(resp),
        }
    }

    /// `resp` when it is 2xx, else the classified error ([`status_error`]).
    pub(crate) async fn check(
        &self,
        resp: reqwest::Response,
        what: &str,
    ) -> Result<reqwest::Response> {
        if resp.status().is_success() {
            return Ok(resp);
        }
        Err(self.fail(resp, what).await)
    }

    /// The classified error for a non-2xx `resp` ([`status_error`]).
    pub(crate) async fn fail(&self, resp: reqwest::Response, what: &str) -> Error {
        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.text().await.unwrap_or_default();
        status_error(self.product, what, status, &headers, &body)
    }

    /// [`Self::send_raw`] + [`Self::check`]: the one way a Jira/Confluence
    /// call is sent when no status needs special handling.
    pub(crate) async fn send(
        &self,
        req: reqwest::RequestBuilder,
        what: &str,
    ) -> Result<reqwest::Response> {
        let resp = self.send_raw(req, what).await?;
        self.check(resp, what).await
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

    fn headers(pairs: &[(&'static str, &str)]) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    /// integrations-4-06: 429 (and 503 + Retry-After) read the wait; other
    /// statuses are not rate limits.
    #[test]
    fn rate_limit_waits_come_from_retry_after() {
        let none = headers(&[]);
        assert_eq!(
            rate_limit_wait(429, &headers(&[("retry-after", "7")])),
            Some(Duration::from_secs(7))
        );
        assert_eq!(rate_limit_wait(429, &none), Some(Duration::from_secs(1)));
        assert_eq!(
            rate_limit_wait(503, &headers(&[("retry-after", "3")])),
            Some(Duration::from_secs(3))
        );
        assert_eq!(rate_limit_wait(503, &none), None);
        assert_eq!(
            rate_limit_wait(500, &headers(&[("retry-after", "3")])),
            None
        );
    }

    /// 429 names the wait, 401/403 is Forbidden with a token hint, the rest
    /// stays Upstream — every body bounded.
    #[test]
    fn statuses_are_classified() {
        use reqwest::StatusCode as S;
        let none = headers(&[]);
        let rl = status_error(
            "jira",
            "jira search",
            S::TOO_MANY_REQUESTS,
            &headers(&[("retry-after", "42")]),
            "",
        );
        assert!(
            matches!(&rl, Error::Upstream(m) if m == "jira rate limited — retry in 42s"),
            "{rl:?}"
        );
        for st in [S::UNAUTHORIZED, S::FORBIDDEN] {
            let e = status_error(
                "jira",
                "jira search",
                st,
                &none,
                &"x".repeat(BODY_SNIPPET_MAX * 3),
            );
            match &e {
                Error::Forbidden(m) => {
                    assert!(m.contains("update the token"), "{m}");
                    assert!(m.len() < BODY_SNIPPET_MAX * 2, "body is capped");
                }
                other => panic!("{other:?}"),
            }
        }
        let e = status_error(
            "confluence",
            "confluence get_page",
            S::BAD_GATEWAY,
            &none,
            "down",
        );
        assert!(
            matches!(&e, Error::Upstream(m) if m.contains("confluence get_page failed (502")),
            "{e:?}"
        );
    }

    /// A short rate-limit wait on a read is absorbed by one retry; a write is
    /// never re-sent.
    #[tokio::test]
    async fn a_rate_limited_read_is_retried_once_and_a_write_is_not() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let served = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let n = served.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                // Every other answer is a 429 with a zero-second wait.
                let reply = if n.is_multiple_of(2) {
                    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                };
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        let t = AtlassianTransport::new("jira", format!("http://{addr}"), "e", "t");
        let url = format!("http://{addr}/x");
        let resp = t
            .send(t.http().unwrap().get(&url), "jira read")
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), "ok");
        assert_eq!(hits.load(Ordering::SeqCst), 2, "one 429 + one retry");
        // hits is even again → the POST meets a 429 and is not re-sent.
        let err = t
            .send(t.http().unwrap().post(&url), "jira write")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("jira rate limited — retry in 1s"),
            "{err}"
        );
        assert_eq!(hits.load(Ordering::SeqCst), 3);
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
