//! Shared HTTP layer for provider clients: one reqwest client with a 20s
//! timeout, a single retry on a rate limit / 5xx, and uniform error mapping
//! into `Error::Upstream` carrying the HTTP status plus the provider's
//! message field when parseable.
//!
//! # Rate limits
//!
//! A forge refusal for *quota* (429, or a 403 carrying
//! `x-ratelimit-remaining: 0` / `retry-after`) is not the same failure as a bad
//! token, and it tells us exactly how long to wait. [`rate_limit_wait`] reads
//! that wait from `retry-after` (seconds or HTTP-date) or `x-ratelimit-reset`;
//! a short one is slept off and the request retried once, a long one becomes
//! `Error::Upstream("<provider> rate limited — retry in Ns")` immediately
//! rather than parking the caller's request for minutes.
//!
//! # ETag / short-TTL GET cache
//!
//! `get_cached` wraps GET requests with a process-wide in-memory cache keyed by
//! `sha256(method + "\0" + url + "\0" + auth_header_value)`.  Including the
//! auth header in the key means two accounts hitting the same URL never share a
//! cached body — the only cross-account leak vector is eliminated.
//!
//! Behaviour on a repeat call to the same (url, auth) pair:
//! - Still within the short TTL (60 s) and no ETag stored → return cached body.
//! - ETag stored → send `If-None-Match`; on `304 Not Modified` refresh
//!   `fetched_at` and return cached body; on `200` store the new etag+body.
//! - TTL elapsed and no ETag → fall through to a normal GET and refresh the
//!   entry.
//!
//! The map is bounded: past [`CACHE_MAX_ENTRIES`] the oldest fetch is evicted,
//! and an entry older than [`CACHE_STALE`] reads as a miss — a process-wide
//! cache keyed by (url, credential) must not grow for the life of the daemon.
//!
//! Callers that need pagination or mutation continue to use `send` / `json` /
//! `text` / `ok` directly; those paths are unaffected.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use otto_core::{Error, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Process-wide GET cache
// ---------------------------------------------------------------------------

/// A single cached GET response.
struct CachedGet {
    /// ETag value returned by the server, if any (`ETag` response header).
    etag: Option<String>,
    /// Decoded response body.
    body: String,
    /// When the body was last fetched from the network.
    fetched_at: Instant,
}

/// Fallback TTL: if a response has no ETag we still serve the cached copy
/// for this long before issuing a fresh unconditional GET.
const SHORT_TTL: Duration = Duration::from_secs(60);

/// Hard cap on cached entries. The cache is process-wide and keyed by
/// (url, credential), so a long-lived daemon talking to many repos/accounts
/// would otherwise grow it without bound; past this the oldest *fetch* is
/// evicted (not the least recently read — `fetched_at` is what we track).
const CACHE_MAX_ENTRIES: usize = 512;

/// Age past which an entry is dropped on read. [`SHORT_TTL`] governs only
/// *blind* reuse; this is the point where even revalidating an hour-old body
/// is not worth the memory it occupies.
const CACHE_STALE: Duration = Duration::from_secs(3600);

static GET_CACHE: OnceLock<Mutex<HashMap<String, CachedGet>>> = OnceLock::new();

fn get_cache() -> &'static Mutex<HashMap<String, CachedGet>> {
    GET_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Build an auth-scoped, URL-keyed cache key.
///
/// The key is `sha256(url + "\0" + auth_header_value)` returned as a hex
/// string.  The auth value is whatever `Authorization` or `PRIVATE-TOKEN`
/// header the caller attaches — different tokens produce different keys, so
/// one account can never receive another's cached body.
fn cache_key(url: &str, auth_value: &str) -> String {
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    h.update(b"\0");
    h.update(auth_value.as_bytes());
    hex::encode(h.finalize())
}

/// Store `entry` under `key`, evicting the oldest fetch when the cache is full.
fn insert_cached(key: String, entry: CachedGet) {
    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    insert_into(&mut guard, key, entry);
}

/// Read `key`, treating an entry older than [`CACHE_STALE`] as a miss (and
/// dropping it). Returns `(etag, body, fetched_at)`.
fn read_cached(key: &str) -> Option<(Option<String>, String, Instant)> {
    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    read_from(&mut guard, key)
}

/// Lock-free body of [`insert_cached`] so the bound is unit-testable without
/// touching the process-wide cache (tests run in parallel in one binary).
fn insert_into(map: &mut HashMap<String, CachedGet>, key: String, entry: CachedGet) {
    if map.len() >= CACHE_MAX_ENTRIES && !map.contains_key(&key) {
        let oldest = map
            .iter()
            .min_by_key(|(_, e)| e.fetched_at)
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest {
            map.remove(&k);
        }
    }
    map.insert(key, entry);
}

/// Lock-free body of [`read_cached`] (see [`insert_into`]).
fn read_from(
    map: &mut HashMap<String, CachedGet>,
    key: &str,
) -> Option<(Option<String>, String, Instant)> {
    if map.get(key)?.fetched_at.elapsed() > CACHE_STALE {
        map.remove(key);
        return None;
    }
    let e = map.get(key)?;
    Some((e.etag.clone(), e.body.clone(), e.fetched_at))
}

/// Extract the value of whichever auth header is present on the request.
/// GitHub uses `Authorization: Bearer …`, GitLab uses `PRIVATE-TOKEN: …`,
/// Bitbucket uses `Authorization: Basic …`.  We just need *something* that
/// identifies the credential; the exact string is never stored or logged.
fn extract_auth(headers: &reqwest::header::HeaderMap) -> String {
    if let Some(v) = headers
        .get("authorization")
        .or_else(|| headers.get("PRIVATE-TOKEN"))
        .or_else(|| headers.get("private-token"))
    {
        return v.to_str().unwrap_or("").to_string();
    }
    String::new()
}

// ---------------------------------------------------------------------------
// Rate limits
// ---------------------------------------------------------------------------

/// Longest wait we are willing to absorb inside a request. Anything longer is
/// handed back to the caller as an error naming the real wait.
const RATE_LIMIT_MAX_WAIT: Duration = Duration::from_secs(30);

/// `Some(wait)` when the response is a rate-limit refusal: 429, or 403 with
/// `x-ratelimit-remaining: 0` / a `retry-after` header. `wait` comes from
/// `retry-after` (seconds or HTTP-date) or `x-ratelimit-reset − now`, clamped
/// to [`RATE_LIMIT_MAX_WAIT`] for retry purposes — the value itself is exact so
/// the error text can quote it. Missing / unparsable → 1 s.
fn rate_limit_wait(status: u16, headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let hdr = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let retry_after = hdr("retry-after");
    let limited = status == 429
        || (status == 403
            && (hdr("x-ratelimit-remaining") == Some("0") || retry_after.is_some()));
    if !limited {
        return None;
    }
    // `Retry-After` is either delta-seconds or an HTTP-date (RFC 2822 shape).
    if let Some(v) = retry_after {
        if let Ok(secs) = v.parse::<u64>() {
            return Some(Duration::from_secs(secs));
        }
        if let Ok(when) = chrono::DateTime::parse_from_rfc2822(v) {
            return Some(Duration::from_secs(
                (when.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64,
            ));
        }
    }
    // GitHub/GitLab budget reset: an absolute unix timestamp.
    if let Some(reset) = hdr("x-ratelimit-reset").and_then(|v| v.parse::<i64>().ok()) {
        return Some(Duration::from_secs(
            (reset - chrono::Utc::now().timestamp()).max(0) as u64,
        ));
    }
    Some(Duration::from_secs(1))
}

/// The one error text for a quota refusal — kept in one place so the handler,
/// the docs and the troubleshooting table all say the same thing.
fn rate_limited_err(provider: &str, wait: Duration) -> Error {
    Error::Upstream(format!(
        "{provider} rate limited — retry in {}s",
        wait.as_secs()
    ))
}

// ---------------------------------------------------------------------------
// Http helper
// ---------------------------------------------------------------------------

pub struct Http {
    client: reqwest::Client,
    provider: &'static str,
}

impl Http {
    pub fn new(provider: &'static str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("otto-ade/0.1")
            .build()
            .expect("reqwest client");
        Self { client, provider }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    /// Send with one retry on a short rate limit / 5xx; returns the successful
    /// response.
    pub async fn send(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        self.send_classified(rb, false).await
    }

    /// As [`send`](Self::send), but a `304 Not Modified` comes back as `Ok` so a
    /// conditional GET can inspect it. Callers MUST check `status() == 304`
    /// before reading the body.
    async fn send_checked(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        self.send_classified(rb, true).await
    }

    /// Shared body of [`send`](Self::send) / [`send_checked`](Self::send_checked):
    /// one retry (the forge's own `retry-after` for a quota refusal, a flat
    /// 700 ms for a 5xx), then status classification.
    async fn send_classified(
        &self,
        rb: reqwest::RequestBuilder,
        allow_304: bool,
    ) -> Result<reqwest::Response> {
        let retry = rb.try_clone();
        let resp = rb
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?;

        let status = resp.status();
        // Read the verdict off the headers BEFORE the match: the arms move
        // `resp`, so no borrow of it may still be live.
        let limit_wait = rate_limit_wait(status.as_u16(), resp.headers());
        let resp = match limit_wait {
            // Quota refusal: wait exactly as long as the forge asked, but only
            // when that is short enough to hold a request open for.
            Some(wait) => {
                if wait > RATE_LIMIT_MAX_WAIT {
                    return Err(rate_limited_err(self.provider, wait));
                }
                match retry {
                    Some(rb2) => {
                        tokio::time::sleep(wait).await;
                        rb2.send()
                            .await
                            .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?
                    }
                    None => resp,
                }
            }
            None if status.is_server_error() => match retry {
                Some(rb2) => {
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    rb2.send()
                        .await
                        .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?
                }
                None => resp,
            },
            None => resp,
        };

        let status = resp.status();
        if status.is_success() || (allow_304 && status.as_u16() == 304) {
            return Ok(resp);
        }
        let headers = resp.headers().clone();
        let body = resp.text().await.unwrap_or_default();
        Err(provider_status_err(self.provider, status, &headers, &body))
    }

    /// Send and parse a JSON body.
    pub async fn json(&self, rb: reqwest::RequestBuilder) -> Result<Value> {
        let resp = self.send(rb).await?;
        resp.json::<Value>()
            .await
            .map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))
    }

    /// Send and return the raw text body.
    pub async fn text(&self, rb: reqwest::RequestBuilder) -> Result<String> {
        let resp = self.send(rb).await?;
        resp.text()
            .await
            .map_err(|e| Error::Upstream(format!("{}: body read: {e}", self.provider)))
    }

    /// Send and discard the body.
    pub async fn ok(&self, rb: reqwest::RequestBuilder) -> Result<()> {
        self.send(rb).await.map(|_| ())
    }

    /// Perform a GET with ETag / short-TTL caching.
    ///
    /// The cache key includes the auth header value, so responses are scoped to
    /// the account — different credentials for the same URL never share a body.
    ///
    /// - If the cached entry is fresh (within [`SHORT_TTL`]) → return cached.
    /// - If there is a stored ETag → send `If-None-Match`; `304` refreshes
    ///   `fetched_at` and returns cached; `200` updates the entry.
    /// - If TTL elapsed and no ETag → unconditional GET, update entry.
    ///
    /// The `rb` parameter **must** be a GET request with the auth header(s)
    /// already attached.  Non-2xx responses are returned as `Error::Upstream`.
    pub async fn get_cached(&self, rb: reqwest::RequestBuilder) -> Result<String> {
        // Build the request so we can inspect its headers (url + auth) for the
        // cache key, then convert back to a builder for sending.
        let req = rb
            .build()
            .map_err(|e| Error::Upstream(format!("{}: build req: {e}", self.provider)))?;

        let url = req.url().to_string();
        let auth = extract_auth(req.headers());
        let key = cache_key(&url, &auth);

        // -- Read the cache (lock scope: just the lookup) -------------------
        let cached = read_cached(&key);

        if let Some((etag, body, fetched_at)) = cached {
            let age = fetched_at.elapsed();

            // Still within TTL → return without hitting the network.
            if age < SHORT_TTL && etag.is_none() {
                return Ok(body);
            }

            // We have an ETag → send conditional GET.
            if let Some(ref tag) = etag {
                let mut rb2 = self
                    .client
                    .get(&url)
                    .header("If-None-Match", tag);
                // Re-attach the auth header by copying from the original req.
                for (name, value) in req.headers() {
                    rb2 = rb2.header(name.clone(), value.clone());
                }

                // Same retry / rate-limit classification as `send`, with 304
                // surfaced as success.
                let resp = self.send_checked(rb2).await?;

                if resp.status().as_u16() == 304 {
                    // Not Modified: refresh fetched_at, return cached body.
                    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(entry) = guard.get_mut(&key) {
                        entry.fetched_at = Instant::now();
                    }
                    return Ok(body);
                }

                // 200 — the resource changed; replace the entry.
                let new_etag = resp
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let new_body = resp
                    .text()
                    .await
                    .map_err(|e| Error::Upstream(format!("{}: body read: {e}", self.provider)))?;
                insert_cached(
                    key,
                    CachedGet {
                        etag: new_etag,
                        body: new_body.clone(),
                        fetched_at: Instant::now(),
                    },
                );
                return Ok(new_body);
            }

            // TTL elapsed, no ETag → unconditional GET (fall through).
        }

        // -- No usable cache entry: unconditional GET -----------------------
        // Re-create the builder from the already-built request by cloning its
        // headers into a fresh GET for the same URL.
        let mut rb_fresh = self.client.get(&url);
        for (name, value) in req.headers() {
            rb_fresh = rb_fresh.header(name.clone(), value.clone());
        }

        // No validator is sent here, so `send_checked` can only return a 2xx.
        let resp = self.send_checked(rb_fresh).await?;

        let new_etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let new_body = resp
            .text()
            .await
            .map_err(|e| Error::Upstream(format!("{}: body read: {e}", self.provider)))?;

        insert_cached(
            key,
            CachedGet {
                etag: new_etag,
                body: new_body.clone(),
                fetched_at: Instant::now(),
            },
        );

        Ok(new_body)
    }

    /// Fetch all pages of a JSON array endpoint by following GitHub-style
    /// `Link: <url>; rel="next"` headers. Collects into a flat `Vec<Value>`.
    /// Stops after 20 pages as a safety guard against runaway pagination.
    pub async fn paginate_json(
        &self,
        first_rb: reqwest::RequestBuilder,
        client: &reqwest::Client,
        auth_header: (&'static str, String),
    ) -> Result<Vec<Value>> {
        const MAX_PAGES: usize = 20;
        let mut all: Vec<Value> = Vec::new();
        let resp = self.send(first_rb).await?;
        let next = parse_next_link(resp.headers());
        let page: Value = resp.json().await.map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))?;
        if let Some(arr) = page.as_array() { all.extend_from_slice(arr); }
        let mut next_url = next;
        let mut pages = 1usize;
        while let Some(url) = next_url {
            if pages >= MAX_PAGES { break; }
            let rb = client.get(&url).header(auth_header.0, &auth_header.1);
            let resp = self.send(rb).await?;
            let nxt = parse_next_link(resp.headers());
            let page: Value = resp.json().await.map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))?;
            if let Some(arr) = page.as_array() { all.extend_from_slice(arr); }
            next_url = nxt;
            pages += 1;
        }
        Ok(all)
    }

    /// Send WITHOUT erroring on non-2xx so callers can inspect the status.
    pub async fn send_raw(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        rb.send()
            .await
            .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))
    }

    /// Map a response to Ok/Err based on its HTTP status, extracting the
    /// provider error message on failure.
    pub async fn into_result(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let headers = resp.headers().clone();
        let body = resp.text().await.unwrap_or_default();
        Err(provider_status_err(self.provider, status, &headers, &body))
    }
}

/// Classify a forge's non-2xx by status instead of collapsing everything into
/// `Error::Upstream`/502 (which the UI reads as "the provider is DOWN" and
/// turns into a global outage banner — untrue for a revoked token or a 404):
/// 401/403 → 403 with a check-your-token hint, 404 → 404, and the
/// state-conflict family (405/409/422, e.g. "PR is not mergeable") → 409.
/// Real 5xx/429/transport failures stay Upstream.
///
/// A quota refusal is classified FIRST: a 403 that is really "you have spent
/// your hourly budget" must not read as "your token was rejected".
fn provider_status_err(
    provider: &str,
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    body: &str,
) -> Error {
    if let Some(wait) = rate_limit_wait(status.as_u16(), headers) {
        return rate_limited_err(provider, wait);
    }
    let msg = format!("{} {}: {}", provider, status.as_u16(), extract_message(body));
    match status.as_u16() {
        401 | 403 => Error::Forbidden(format!(
            "{msg} — the {provider} credential was rejected; check the git account's token/scopes"
        )),
        404 => Error::NotFound(msg),
        405 | 409 | 422 => Error::Conflict(msg),
        _ => Error::Upstream(msg),
    }
}

/// Best-effort extraction of the human message from a provider error body.
/// GitHub: {"message": "..."}; GitLab: {"message": ... (string|array|object)};
/// Bitbucket: {"error": {"message": "..."}}.
fn extract_message(body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    if let Some(v) = parsed {
        if let Some(m) = v.get("message") {
            return value_to_msg(m);
        }
        if let Some(e) = v.get("error") {
            if let Some(m) = e.get("message") {
                return value_to_msg(m);
            }
            return value_to_msg(e);
        }
        if let Some(errs) = v.get("errors").and_then(Value::as_array) {
            if let Some(first) = errs.first() {
                if let Some(m) = first.get("message") {
                    return value_to_msg(m);
                }
            }
        }
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "no error body".to_string()
    } else {
        trimmed.chars().take(200).collect()
    }
}

fn value_to_msg(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string().chars().take(200).collect(),
    }
}

/// Extract the `rel="next"` URL from a GitHub/GitLab Link response header, if present.
/// Format: `<https://…>; rel="next", <https://…>; rel="last"`
pub fn parse_next_link(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get("link")?.to_str().ok()?;
    for part in link.split(',') {
        let part = part.trim();
        if part.contains(r#"rel="next""#) {
            let url = part.split(';').next()?.trim().trim_start_matches('<').trim_end_matches('>');
            return Some(url.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        cache_key, extract_auth, insert_into, rate_limit_wait, read_from, CachedGet,
        CACHE_MAX_ENTRIES, SHORT_TTL,
    };
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn entry(body: &str, fetched_at: Instant) -> CachedGet {
        CachedGet { etag: None, body: body.to_string(), fetched_at }
    }

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                reqwest::header::HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn cache_key_differs_by_auth() {
        let url = "https://api.github.com/repos/acme/app/pulls";
        let key1 = cache_key(url, "Bearer token-alice");
        let key2 = cache_key(url, "Bearer token-bob");
        assert_ne!(key1, key2, "different tokens must produce different keys");
    }

    #[test]
    fn cache_key_same_for_same_auth() {
        let url = "https://api.github.com/repos/acme/app/pulls";
        let k1 = cache_key(url, "Bearer token-alice");
        let k2 = cache_key(url, "Bearer token-alice");
        assert_eq!(k1, k2);
    }

    #[test]
    fn cache_key_differs_by_url() {
        let auth = "Bearer shared-token";
        let k1 = cache_key("https://api.github.com/repos/a/b/pulls", auth);
        let k2 = cache_key("https://api.github.com/repos/a/c/pulls", auth);
        assert_ne!(k1, k2);
    }

    #[test]
    fn empty_auth_still_produces_a_key() {
        let k = cache_key("https://example.com/path", "");
        assert!(!k.is_empty());
    }

    #[test]
    fn short_ttl_is_sixty_seconds() {
        assert_eq!(SHORT_TTL, Duration::from_secs(60));
    }

    #[test]
    fn extract_auth_prefers_authorization() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_static("Bearer tok"),
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("private-token"),
            reqwest::header::HeaderValue::from_static("glpat-xyz"),
        );
        // Authorization should win.
        assert_eq!(extract_auth(&headers), "Bearer tok");
    }

    #[test]
    fn extract_auth_falls_back_to_private_token() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::HeaderName::from_static("private-token"),
            reqwest::header::HeaderValue::from_static("glpat-xyz"),
        );
        assert_eq!(extract_auth(&headers), "glpat-xyz");
    }

    #[test]
    fn extract_auth_empty_when_no_header() {
        let headers = reqwest::header::HeaderMap::new();
        assert_eq!(extract_auth(&headers), "");
    }

    // -- cache bounds ------------------------------------------------------

    #[test]
    fn cache_evicts_oldest_past_512() {
        let mut map: HashMap<String, CachedGet> = HashMap::new();
        let base = Instant::now();
        // Oldest first so the eviction victim is unambiguous.
        for i in 0..CACHE_MAX_ENTRIES {
            let at = base + Duration::from_millis(i as u64);
            insert_into(&mut map, format!("k{i}"), entry("body", at));
        }
        assert_eq!(map.len(), CACHE_MAX_ENTRIES);
        insert_into(
            &mut map,
            "k-new".to_string(),
            entry("body", base + Duration::from_secs(10)),
        );
        assert_eq!(map.len(), CACHE_MAX_ENTRIES, "cache must stay bounded");
        assert!(!map.contains_key("k0"), "the oldest fetch is evicted");
        assert!(map.contains_key("k-new"));
        assert!(map.contains_key("k1"), "only ONE entry is evicted");
    }

    #[test]
    fn cache_overwrite_does_not_evict() {
        let mut map: HashMap<String, CachedGet> = HashMap::new();
        let base = Instant::now();
        for i in 0..CACHE_MAX_ENTRIES {
            insert_into(&mut map, format!("k{i}"), entry("body", base + Duration::from_millis(i as u64)));
        }
        // Refreshing an existing key is not a new entry — nothing may be dropped.
        insert_into(&mut map, "k0".to_string(), entry("fresh", base + Duration::from_secs(10)));
        assert_eq!(map.len(), CACHE_MAX_ENTRIES);
        assert_eq!(map.get("k0").map(|e| e.body.as_str()), Some("fresh"));
    }

    #[test]
    fn cache_entry_older_than_an_hour_is_a_miss() {
        // `Instant` is monotonic-since-boot: on a machine up for less than an
        // hour there is no such instant to construct, and nothing to assert.
        let Some(stale_at) = Instant::now().checked_sub(Duration::from_secs(3700)) else {
            return;
        };
        let mut map: HashMap<String, CachedGet> = HashMap::new();
        insert_into(&mut map, "stale".to_string(), entry("old body", stale_at));
        insert_into(&mut map, "fresh".to_string(), entry("new body", Instant::now()));

        assert!(read_from(&mut map, "stale").is_none(), "an hour-old entry is a miss");
        assert!(!map.contains_key("stale"), "and it is dropped, not kept");
        assert_eq!(
            read_from(&mut map, "fresh").map(|(_, b, _)| b),
            Some("new body".to_string())
        );
    }

    // -- rate limits -------------------------------------------------------

    #[test]
    fn rate_limit_wait_parses_seconds_date_and_reset() {
        // delta-seconds
        assert_eq!(
            rate_limit_wait(429, &headers(&[("retry-after", "45")])),
            Some(Duration::from_secs(45))
        );
        // HTTP-date, ~60 s out (allow a second of slack for the clock read).
        let when = chrono::Utc::now() + chrono::Duration::seconds(60);
        let secs = rate_limit_wait(
            429,
            &headers(&[("retry-after", &when.to_rfc2822())]),
        )
        .expect("date retry-after is a rate limit")
        .as_secs();
        assert!((58..=61).contains(&secs), "got {secs}s");
        // x-ratelimit-reset (absolute unix ts) on a 403 with the budget spent.
        let reset = chrono::Utc::now().timestamp() + 120;
        let secs = rate_limit_wait(
            403,
            &headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", &reset.to_string())]),
        )
        .expect("spent budget is a rate limit")
        .as_secs();
        assert!((118..=121).contains(&secs), "got {secs}s");
        // Rate-limited but no usable hint → the 1 s floor.
        assert_eq!(
            rate_limit_wait(429, &reqwest::header::HeaderMap::new()),
            Some(Duration::from_secs(1))
        );
        assert_eq!(
            rate_limit_wait(429, &headers(&[("retry-after", "soon")])),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn rate_limit_wait_ignores_ordinary_failures() {
        // A plain 403 is a token problem, not a quota one.
        assert!(rate_limit_wait(403, &headers(&[("x-ratelimit-remaining", "4999")])).is_none());
        assert!(rate_limit_wait(403, &reqwest::header::HeaderMap::new()).is_none());
        assert!(rate_limit_wait(404, &headers(&[("retry-after", "5")])).is_none());
        assert!(rate_limit_wait(200, &reqwest::header::HeaderMap::new()).is_none());
    }
}
