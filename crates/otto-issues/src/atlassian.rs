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
use std::net::IpAddr;
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
/// Cap on bytes buffered from an error response body before clipping to
/// [`BODY_SNIPPET_MAX`], so an oversized HTML/dump error page cannot bloat RAM.
pub(crate) const ERROR_BODY_CAP: usize = 64 * 1024;
/// Longest rate-limit wait a READ absorbs (once) before the error names the
/// wait instead — the same bound the git providers' client retries within.
pub(crate) const RATE_LIMIT_RETRY_MAX: Duration = Duration::from_secs(10);

/// Stream at most `max_bytes` from `resp` and decode lossily as UTF-8.
pub(crate) async fn read_text_capped(mut resp: reqwest::Response, max_bytes: usize) -> String {
    let mut buf = Vec::new();
    while buf.len() < max_bytes {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                let remaining = max_bytes - buf.len();
                let take = chunk.len().min(remaining);
                buf.extend_from_slice(&chunk[..take]);
            }
            Ok(None) | Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

struct CachedClient {
    client: reqwest::Client,
    used: u64,
}

/// key hash → client. The key (site + credential) is hashed so the cache
/// holds no extra copy of a secret.
static CLIENT_CACHE: OnceLock<Mutex<(u64, HashMap<u64, CachedClient>)>> = OnceLock::new();

/// Validate an issue-tracker account `base_url` against TLS and SSRF rules.
///
/// Returns `Ok(is_loopback)`:
/// - Only `http` and `https` schemes are accepted.
/// - Cloud-metadata hostnames and Otto's own daemon port are always refused.
/// - Explicit loopback hosts (`localhost`, `127.0.0.1`, `[::1]`) are allowed on
///   plain `http` or `https` (for local dev servers and test fixtures) so long
///   as the port is not Otto's daemon port.
/// - Every non-loopback site MUST use `https` and MUST NOT be a blocked IP
///   literal (RFC1918, link-local/IMDS, CGNAT, unspecified, multicast, etc.).
///   Non-loopback hostnames are additionally vetted at connect time by
///   `otto_netguard::GuardedResolver` so DNS rebinding cannot pivot inward.
pub(crate) fn validate_account_base_url(base_url: &str) -> std::result::Result<bool, String> {
    let trimmed = base_url.trim();
    let parsed = reqwest::Url::parse(trimmed).map_err(|e| format!("invalid url: {e}"))?;
    match parsed.scheme() {
        "https" | "http" => {}
        other => return Err(format!("blocked url scheme: {other}")),
    }
    let host = parsed
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| "url has no host".to_string())?;
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if otto_netguard::is_metadata_hostname(bare) {
        return Err(format!(
            "blocked address {bare}: cloud-metadata hosts are always blocked (SSRF guard)"
        ));
    }
    let loopback = bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);
    if loopback {
        let port = parsed.port_or_known_default().unwrap_or(80);
        if otto_netguard::is_daemon_port(port) {
            return Err(format!(
                "blocked address {bare}:{port}: port {port} is Otto's own daemon, always blocked \
                 (SSRF guard)"
            ));
        }
        return Ok(true);
    }
    if parsed.scheme() != "https" {
        otto_netguard::require_tls_or_loopback(trimmed)?;
    }
    if !otto_netguard::check_url_literal(&parsed) {
        return Err(format!("blocked address {bare} (SSRF guard)"));
    }
    Ok(false)
}

/// Same scheme, ASCII-lowercased host, and effective port.
pub(crate) fn same_origin(a: &reqwest::Url, b: &reqwest::Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
        && a.port_or_known_default() == b.port_or_known_default()
}

#[derive(Debug, PartialEq, Eq)]
enum RedirectDecision {
    Follow,
    Stop,
    Error(&'static str),
}

fn classify_redirect(
    previous: &[reqwest::Url],
    target: &reqwest::Url,
    allow_loopback: bool,
) -> RedirectDecision {
    if previous.len() >= otto_netguard::MAX_REDIRECTS {
        return RedirectDecision::Error("too many redirects");
    }
    if !matches!(target.scheme(), "http" | "https") {
        return RedirectDecision::Error("blocked redirect scheme");
    }
    if previous.iter().any(|u| u.scheme() == "https") && target.scheme() != "https" {
        return RedirectDecision::Error("blocked HTTPS to HTTP redirect downgrade");
    }
    if let Some(host) = target.host_str() {
        let bare = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        if otto_netguard::is_metadata_hostname(bare) {
            return RedirectDecision::Error("blocked redirect target (SSRF guard)");
        }
    }
    let ssrf_ok = if allow_loopback {
        otto_netguard::check_url_literal_allow_local(target)
    } else {
        otto_netguard::check_url_literal(target)
    };
    if !ssrf_ok {
        return RedirectDecision::Error("blocked redirect target (SSRF guard)");
    }
    let Some(origin) = previous.first() else {
        return RedirectDecision::Stop;
    };
    if same_origin(origin, target) {
        RedirectDecision::Follow
    } else {
        RedirectDecision::Stop
    }
}

/// Redirect policy for authenticated issue-tracker API calls:
/// - caps hops at [`otto_netguard::MAX_REDIRECTS`];
/// - allows only `http`/`https` schemes;
/// - **blocks HTTPS→HTTP downgrades** with an explicit error;
/// - vets every hop against SSRF rules (metadata hostnames, blocked IP literals,
///   daemon ports);
/// - **stops on cross-origin redirects** so credentials and request bodies are
///   never sent to another host (attachment downloads handle cross-host CDN
///   redirects explicitly without `Authorization`).
fn atlassian_redirect_policy(allow_loopback: bool) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        match classify_redirect(attempt.previous(), attempt.url(), allow_loopback) {
            RedirectDecision::Follow => attempt.follow(),
            RedirectDecision::Stop => attempt.stop(),
            RedirectDecision::Error(msg) => attempt.error(msg),
        }
    })
}

fn build_client_for(https_only: bool, allow_loopback: bool) -> reqwest::Client {
    let builder = if allow_loopback {
        reqwest::Client::builder().no_proxy()
    } else {
        otto_netguard::guarded_client_builder()
    };
    builder
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .https_only(https_only)
        .redirect(atlassian_redirect_policy(allow_loopback))
        .build()
        .unwrap_or_default()
}

/// Build a single-hop (no auto-redirect) client for following verified
/// attachment CDN redirects without leaking `Authorization`.
pub(crate) fn build_attachment_hop_client(
    https_only: bool,
    allow_loopback: bool,
) -> reqwest::Client {
    let builder = if allow_loopback {
        reqwest::Client::builder().no_proxy()
    } else {
        otto_netguard::guarded_client_builder()
    };
    builder
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

/// A shared `reqwest::Client` for `key`, built on first use and kept in a
/// bounded (least-recently-used) cache so its connection pool is reused.
/// Falls back to a fresh client when the lock is poisoned (callers stay
/// infallible).
pub(crate) fn cached_client_for(
    key: &str,
    https_only: bool,
    allow_loopback: bool,
) -> reqwest::Client {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    https_only.hash(&mut h);
    allow_loopback.hash(&mut h);
    let k = h.finish();
    let cache = CLIENT_CACHE.get_or_init(|| Mutex::new((0, HashMap::new())));
    let Ok(mut guard) = cache.lock() else {
        return build_client_for(https_only, allow_loopback);
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
    let client = build_client_for(https_only, allow_loopback);
    map.insert(
        k,
        CachedClient {
            client: client.clone(),
            used: *clock,
        },
    );
    client
}

#[cfg(test)]
pub(crate) fn cached_client(key: &str) -> reqwest::Client {
    cached_client_for(key, true, false)
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
    /// `true` when the base URL is an explicit loopback dev/test target.
    pub(crate) allow_loopback: bool,
    /// `Some` when the base would carry the header over plain http to a
    /// non-loopback host or targets an SSRF-blocked address — every request is
    /// refused before it is built.
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
        let (allow_loopback, transport_err) = match validate_account_base_url(&base) {
            Ok(is_loopback) => (is_loopback, None),
            Err(e) => (false, Some(e)),
        };
        let https_only = !allow_loopback || base.trim_start().starts_with("https://");
        // Refused sites get a strict guarded https-only client too: a backstop
        // should any path ever reach the client without `http()`.
        let http = if transport_err.is_some() {
            build_client_for(true, false)
        } else {
            cached_client_for(
                &format!("{base}\x00{auth_header}"),
                https_only,
                allow_loopback,
            )
        };
        Self {
            base,
            auth_header,
            http,
            allow_loopback,
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
        let body = read_text_capped(resp, ERROR_BODY_CAP).await;
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

    #[test]
    fn base_url_validation_blocks_ssrf_and_cleartext_and_daemon_port() {
        for bad in [
            "http://example.com",
            "https://169.254.169.254",
            "https://10.0.0.1",
            "https://192.168.1.10",
            "https://172.16.0.1",
            "https://0.0.0.0",
            "https://[::ffff:169.254.169.254]",
            "https://[::ffff:127.0.0.1]",
            "https://metadata.google.internal",
            "http://127.0.0.1:7700",
            "http://localhost:7700",
            "ws://example.com",
            "file:///etc/passwd",
        ] {
            assert!(
                validate_account_base_url(bad).is_err(),
                "expected {bad} to be rejected"
            );
        }
        assert_eq!(
            validate_account_base_url("https://acme.atlassian.net"),
            Ok(false)
        );
        assert_eq!(validate_account_base_url("http://127.0.0.1:9"), Ok(true));
        assert_eq!(validate_account_base_url("http://localhost:8080"), Ok(true));
    }

    #[test]
    fn redirect_policy_blocks_https_downgrade_ssrf_and_cross_origin() {
        let u = |s: &str| reqwest::Url::parse(s).unwrap();
        let origin = [u("https://acme.atlassian.net/rest/api/3/issue/A-1")];

        // Same-origin HTTPS redirect is allowed.
        assert_eq!(
            classify_redirect(
                &origin,
                &u("https://acme.atlassian.net/rest/api/3/issue/A-2"),
                false
            ),
            RedirectDecision::Follow
        );
        // HTTPS -> HTTP downgrade is blocked with explicit error.
        assert_eq!(
            classify_redirect(
                &origin,
                &u("http://acme.atlassian.net/rest/api/3/issue/A-2"),
                false
            ),
            RedirectDecision::Error("blocked HTTPS to HTTP redirect downgrade")
        );
        // Cross-origin redirect stops so credentials are never leaked.
        assert_eq!(
            classify_redirect(&origin, &u("https://evil.example/steal"), false),
            RedirectDecision::Stop
        );
        // SSRF targets (IMDS, private IP, loopback, metadata host) are blocked.
        for ssrf in [
            "https://169.254.169.254/latest/meta-data/",
            "https://127.0.0.1/secret",
            "https://10.0.0.1/internal",
            "https://metadata.google.internal/computeMetadata/v1/",
        ] {
            assert_eq!(
                classify_redirect(&origin, &u(ssrf), false),
                RedirectDecision::Error("blocked redirect target (SSRF guard)"),
                "{ssrf}"
            );
        }
    }

    #[tokio::test]
    async fn cross_origin_redirect_never_leaks_basic_credentials() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let evil_hit = Arc::new(AtomicBool::new(false));
        let evil_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let evil_addr = evil_listener.local_addr().unwrap();
        let evil_flag = Arc::clone(&evil_hit);
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = evil_listener.accept().await {
                evil_flag.store(true, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });

        let site_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let site_addr = site_listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = site_listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 302 Found\r\nLocation: http://{evil_addr}/steal\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });

        let t = AtlassianTransport::new("jira", format!("http://{site_addr}"), "e", "secret-tok");
        let url = format!("http://{site_addr}/rest/api/3/myself");
        let err = t
            .send(
                t.http()
                    .unwrap()
                    .get(&url)
                    .header("Authorization", &t.auth_header),
                "jira myself",
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("302"), "{err}");
        assert!(
            !evil_hit.load(Ordering::SeqCst),
            "cross-origin redirect target must never be contacted"
        );
    }

    #[tokio::test]
    async fn read_text_capped_bounds_oversized_error_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let huge = "X".repeat(256 * 1024);
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    huge.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(huge.as_bytes()).await;
            }
        });

        let client = build_client_for(false, true);
        let resp = client
            .get(format!("http://{addr}/err"))
            .send()
            .await
            .unwrap();
        let text = read_text_capped(resp, 1024).await;
        assert_eq!(text.len(), 1024);
    }
}
