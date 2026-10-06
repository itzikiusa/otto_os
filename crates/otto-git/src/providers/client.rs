//! Shared HTTP layer for provider clients: one reqwest client with a 20s
//! timeout, a single retry on a rate limit / 5xx (reads only — a write that
//! drew a 5xx may already have landed), and uniform error mapping
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
//! Concurrent misses on one key share a single request ([`page_flights`]):
//! N windows opening one PR on a cold cache cost one GET, not N.
//!
//! Our own writes clear their repository's entries ([`invalidate_scope`]) and
//! bump its write *generation* ([`write_generation`]). A read captures the
//! generation before it goes to the network and stores its result only if the
//! generation is unchanged — a read that left before a comment was posted
//! must not put the pre-comment body back after the write cleared it — and
//! the generation is part of the single-flight key, so a read issued after a
//! write never joins a request that left before it.
//!
//! Callers that need pagination or mutation continue to use `send` / `json` /
//! `text` / `ok` directly; those paths are unaffected.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
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
    /// The request URL — what [`invalidate_scope`] matches a write against
    /// (the map key is a hash, so it can't be prefix-matched).
    url: String,
    /// `Link: rel="next"` of this page, so a cached page still paginates.
    next: Option<String>,
}

/// Fallback TTL: if a response has no ETag we still serve the cached copy
/// for this long before issuing a fresh unconditional GET.
/// Short on purpose: Bitbucket sends no ETag on PR reads, so this is how long
/// a colleague's new comment can stay invisible there. It exists to collapse
/// a burst (N windows, a remount, the detail + its sub-lists) into one fetch;
/// our OWN writes never wait it out — they clear the repo's entries
/// ([`invalidate_scope`]).
const SHORT_TTL: Duration = Duration::from_secs(15);

/// Byte budget across all cached bodies, and the largest body worth keeping
/// (a 20-page comment list is ~1 MB; a bigger one is simply re-fetched).
const CACHE_MAX_BYTES: usize = 32 << 20;
const CACHE_MAX_BODY: usize = 2 << 20;

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

/// Store `entry` under `key`, evicting the oldest fetch when the cache is
/// full — unless a write to its scope landed since the read captured
/// `generation` (then the body is pre-write and storing it would undo the
/// write's [`invalidate_scope`]). Checked under the cache lock, which
/// [`invalidate_scope`] also holds while it bumps, so no bump slips between
/// the check and the insert.
fn insert_cached(key: String, entry: CachedGet, generation: u64) {
    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    if write_generation(&entry.url) != generation {
        return;
    }
    insert_into(&mut guard, key, entry);
}

// ---------------------------------------------------------------------------
// Write generations
// ---------------------------------------------------------------------------

/// Per-scope write generations (scope = what [`invalidate_scope`] clears: a
/// repo prefix, or a host origin for a URL outside any repository). Values
/// come from one process-wide counter, so a bump is always larger than every
/// generation handed out before it.
struct Generations {
    by_scope: HashMap<String, u64>,
    /// The generation of every scope not in `by_scope`: the counter value at
    /// the last prune. A pruned scope reads as at least its old value (and a
    /// read that captured it before the prune merely skips its store) — the
    /// map is bounded without a generation ever going backwards.
    floor: u64,
}

/// Scopes kept before [`Generations`] is pruned to its floor.
const GENERATIONS_MAX: usize = 1024;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

fn generations() -> &'static Mutex<Generations> {
    static G: OnceLock<Mutex<Generations>> = OnceLock::new();
    G.get_or_init(|| {
        Mutex::new(Generations {
            by_scope: HashMap::new(),
            floor: 0,
        })
    })
}

impl Generations {
    fn get(&self, scope: &str) -> u64 {
        self.by_scope.get(scope).copied().unwrap_or(self.floor)
    }

    fn bump(&mut self, scope: String) {
        let g = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
        if self.by_scope.len() >= GENERATIONS_MAX && !self.by_scope.contains_key(&scope) {
            self.by_scope.clear();
            self.floor = g;
        }
        self.by_scope.insert(scope, g);
    }
}

/// The write scope of `url` ([`repo_scope`]) and its host scope.
fn write_scopes(url: &str) -> (Option<String>, Option<String>) {
    let host = reqwest::Url::parse(url)
        .ok()
        .map(|u| u.origin().ascii_serialization() + "/");
    (repo_scope(url), host)
}

/// The write generation a read of `url` must still see when it stores its
/// result: the newer of its repository's and its host's (a write outside any
/// repository clears the whole host). Changes iff one of OUR writes cleared
/// `url`'s scope since — capture it BEFORE the request goes out.
pub(crate) fn write_generation(url: &str) -> u64 {
    let (repo, host) = write_scopes(url);
    let g = generations().lock().unwrap_or_else(|p| p.into_inner());
    let repo = repo.map_or(0, |s| g.get(&s));
    let host = host.map_or(0, |s| g.get(&s));
    repo.max(host)
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
    map.remove(&key);
    if entry.body.len() > CACHE_MAX_BODY {
        return;
    }
    let mut total: usize = map.values().map(|e| e.body.len()).sum();
    while !map.is_empty()
        && (map.len() >= CACHE_MAX_ENTRIES || total + entry.body.len() > CACHE_MAX_BYTES)
    {
        let oldest = map
            .iter()
            .min_by_key(|(_, e)| e.fetched_at)
            .map(|(k, _)| k.clone());
        match oldest.and_then(|k| map.remove(&k)) {
            Some(e) => total -= e.body.len(),
            None => break,
        }
    }
    map.insert(key, entry);
}

/// The repository a forge API URL belongs to, as a URL prefix:
/// `…/repos/{owner}/{repo}/` (GitHub), `…/repositories/{ws}/{repo}/`
/// (Bitbucket), `…/projects/{id}/` (GitLab — the id is one encoded segment).
/// `None` for URLs outside a repository (`/user`, `/graphql`).
fn repo_scope(url: &str) -> Option<String> {
    let u = reqwest::Url::parse(url).ok()?;
    let segs: Vec<&str> = u.path_segments()?.collect();
    let i = segs
        .iter()
        .position(|s| *s == "repos" || *s == "repositories" || *s == "projects")?;
    let take = if segs[i] == "projects" { 1 } else { 2 };
    if segs.len() < i + 1 + take {
        return None;
    }
    let mut scope = u.origin().ascii_serialization();
    for s in &segs[..=i + take] {
        scope.push('/');
        scope.push_str(s);
    }
    scope.push('/');
    Some(scope)
}

/// Drop every cached GET of the repository `url` belongs to (or, for a URL
/// outside any repository, every entry on its host). Run after each
/// successful write, so a comment/approve/merge is visible on the very next
/// read instead of after [`SHORT_TTL`].
pub(crate) fn invalidate_scope(url: &str) {
    // GitHub's GraphQL endpoint is POSTed for READS too (review-thread
    // state on every PR open); nothing it touches is in this cache, and a
    // host-wide clear per PR open would void the cache entirely.
    if url
        .split('?')
        .next()
        .is_some_and(|p| p.ends_with("/graphql"))
    {
        return;
    }
    let scope = repo_scope(url).or_else(|| {
        reqwest::Url::parse(url)
            .ok()
            .map(|u| u.origin().ascii_serialization() + "/")
    });
    let Some(scope) = scope else { return };
    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    // Bump under the cache lock: a read that left before this write now
    // fails its store check, and later reads start a new flight.
    generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .bump(scope.clone());
    // A repo's own URL without the trailing slash (`GET /repos/o/r`) too.
    let bare = scope.trim_end_matches('/');
    guard.retain(|_, e| !(e.url.starts_with(&scope) || e.url == bare));
}

/// The network leg of a cached GET, shared by concurrent misses on one cache
/// key (url + credential) at one [`write_generation`]. Abort-safe: when every
/// waiter is gone the request is dropped.
fn page_flights() -> &'static crate::diff_cache::SingleFlight<(String, Option<String>)> {
    static F: OnceLock<crate::diff_cache::SingleFlight<(String, Option<String>)>> = OnceLock::new();
    F.get_or_init(crate::diff_cache::SingleFlight::new)
}

/// A read that is not a GET (GitHub's GraphQL review-thread probe), memoised
/// in the GET cache under a synthetic repo-scoped `url`, so it shares the
/// byte budget, the [`SHORT_TTL`] and — through [`invalidate_scope`] — every
/// write to that repository. `None` when absent, older than the TTL, or the
/// cache is off.
pub(crate) fn memo_read(url: &str, auth: &str) -> Option<String> {
    if !cache_enabled() {
        return None;
    }
    let (_, body, at) = read_cached(&cache_key(url, auth))?;
    (at.elapsed() < SHORT_TTL).then_some(body)
}

/// Store a [`memo_read`] body, fetched after [`write_generation`] of `url`
/// returned `generation` (a write since then drops the store — the body may
/// predate it).
pub(crate) fn memo_store(url: &str, auth: &str, body: String, generation: u64) {
    if !cache_enabled() {
        return;
    }
    insert_cached(
        cache_key(url, auth),
        CachedGet {
            etag: None,
            body,
            fetched_at: Instant::now(),
            url: url.to_string(),
            next: None,
        },
        generation,
    );
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

/// A GET builder equivalent to the already-built `req`.
fn rb_from(client: &reqwest::Client, req: &reqwest::Request) -> reqwest::RequestBuilder {
    let mut b = client.get(req.url().clone());
    for (name, value) in req.headers() {
        b = b.header(name.clone(), value.clone());
    }
    b
}

#[cfg(test)]
thread_local! {
    /// Unit tests share one process and wiremock POOLS its servers (a later
    /// test gets an earlier one's port, so the same URL) — the process-wide
    /// cache would serve one test another's body. Off unless a test opts in
    /// ([`enable_cache_for_tests`]); `#[tokio::test]` is single-threaded, so
    /// the flag covers everything the test awaits.
    static TEST_CACHE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn cache_enabled() -> bool {
    #[cfg(test)]
    {
        TEST_CACHE.with(|c| c.get())
    }
    #[cfg(not(test))]
    {
        true
    }
}

/// Opt this test (thread) into the GET cache, starting from no entries for
/// `server` (a pooled server may carry another test's).
#[cfg(test)]
pub(crate) fn enable_cache_for_tests(server: &str) {
    TEST_CACHE.with(|c| c.set(true));
    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    guard.retain(|_, e| !e.url.starts_with(server));
}

/// Test hook: age every entry under `prefix` past [`SHORT_TTL`] so the next
/// read revalidates (wiremock servers have unique ports, so a prefix never
/// reaches another test's entries).
#[cfg(test)]
pub(crate) fn expire_cached_for_tests(prefix: &str) {
    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    for e in guard.values_mut() {
        if e.url.starts_with(prefix) {
            e.fetched_at = Instant::now() - SHORT_TTL - Duration::from_secs(1);
        }
    }
}

/// GitLab's `x-next-page: <n>` (empty on the last page) as the URL of that
/// page — instances behind some proxies send it without a `Link` header.
fn x_next_page_url(url: &str, headers: &reqwest::header::HeaderMap) -> Option<String> {
    let n = headers
        .get("x-next-page")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit()))?;
    let mut u = reqwest::Url::parse(url).ok()?;
    let rest: Vec<(String, String)> = u
        .query_pairs()
        .filter(|(k, _)| k != "page")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    u.query_pairs_mut()
        .clear()
        .extend_pairs(rest)
        .append_pair("page", n);
    Some(u.to_string())
}

/// The cached page's `Link: rel="next"` (only meaningful right after a hit).
fn cached_next(key: &str) -> Option<String> {
    let guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
    guard.get(key).and_then(|e| e.next.clone())
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
    let hdr = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    let retry_after = hdr("retry-after");
    let limited = status == 429
        || (status == 403 && (hdr("x-ratelimit-remaining") == Some("0") || retry_after.is_some()));
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

/// True for the methods a 5xx may re-send: reads, which cannot duplicate or
/// half-apply anything at the forge.
fn is_idempotent_read(m: &reqwest::Method) -> bool {
    *m == reqwest::Method::GET || *m == reqwest::Method::HEAD
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

#[derive(Clone)]
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

    /// Send with one retry on a short rate limit / failed connect, or a 5xx
    /// on a READ; returns the successful response.
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
    /// 700 ms for a 5xx or a failed connect), then status classification.
    ///
    /// A 5xx is retried for READS only (GET/HEAD). For a write it does not
    /// mean "not done": GitHub commits a comment / merge / new PR and THEN
    /// answers 502, so a re-send posted the comment twice, turned a merge
    /// that landed into a reported failure (405 "not mergeable") and a
    /// created PR into 422 "already exists" (skipping the after-create
    /// steps). A write is re-sent only when the forge provably did nothing:
    /// a quota refusal (429 / quota-403) or a connection that never opened.
    ///
    /// A WRITE invalidates its repo's cached reads whatever its outcome: a
    /// 5xx / read timeout is exactly the "created it anyway" case, and a
    /// cached comment list without that comment made the duplicate-post
    /// guard answer "no copy on the PR" for the next ~15 s (S15-302).
    async fn send_classified(
        &self,
        rb: reqwest::RequestBuilder,
        allow_304: bool,
    ) -> Result<reqwest::Response> {
        let write_url = rb
            .try_clone()
            .and_then(|c| c.build().ok())
            .filter(|req| !is_idempotent_read(req.method()))
            .map(|req| req.url().to_string());
        let out = self.send_classified_once(rb, allow_304).await;
        if let (Err(_), Some(url)) = (&out, &write_url) {
            invalidate_scope(url);
        }
        out
    }

    async fn send_classified_once(
        &self,
        rb: reqwest::RequestBuilder,
        allow_304: bool,
    ) -> Result<reqwest::Response> {
        let mut retry = rb.try_clone();
        let idempotent = rb
            .try_clone()
            .and_then(|c| c.build().ok())
            .is_some_and(|req| is_idempotent_read(req.method()));
        let resp = match rb.send().await {
            Ok(resp) => resp,
            // Never connected ⇒ nothing reached the forge: safe for any method.
            Err(e) if e.is_connect() => match retry.take() {
                Some(rb2) => {
                    tokio::time::sleep(Duration::from_millis(700)).await;
                    rb2.send()
                        .await
                        .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?
                }
                None => return Err(Error::Upstream(format!("{}: {e}", self.provider))),
            },
            Err(e) => return Err(Error::Upstream(format!("{}: {e}", self.provider))),
        };

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
            None if status.is_server_error() && idempotent => match retry {
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
            if !idempotent {
                // A write landed: the repo's cached reads are stale now.
                invalidate_scope(resp.url().as_str());
            }
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
        self.get_cached_page(rb).await.map(|(body, _)| body)
    }

    /// [`get_cached`](Self::get_cached), parsed as JSON. The read path for
    /// provider GETs whose result the UI re-asks for on every mount (PR list,
    /// PR detail and its comment/review lists, CI): a GitHub `304` costs no
    /// rate-limit budget, and a burst within [`SHORT_TTL`] costs nothing.
    pub async fn get_cached_json(&self, rb: reqwest::RequestBuilder) -> Result<Value> {
        let body = self.get_cached(rb).await?;
        serde_json::from_str(&body)
            .map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))
    }

    /// The cached GET plus the page's `Link: rel="next"` (cached with it, so a
    /// 304 or a TTL hit still knows whether there is a next page).
    pub async fn get_cached_page(
        &self,
        rb: reqwest::RequestBuilder,
    ) -> Result<(String, Option<String>)> {
        // Build the request so we can inspect its headers (url + auth) for the
        // cache key, then convert back to a builder for sending.
        let req = rb
            .build()
            .map_err(|e| Error::Upstream(format!("{}: build req: {e}", self.provider)))?;

        let url = req.url().to_string();
        let auth = extract_auth(req.headers());
        let key = cache_key(&url, &auth);
        if !cache_enabled() {
            let resp = self.send(rb_from(&self.client, &req)).await?;
            let next =
                parse_next_link(resp.headers()).or_else(|| x_next_page_url(&url, resp.headers()));
            let body = resp
                .text()
                .await
                .map_err(|e| Error::Upstream(format!("{}: body read: {e}", self.provider)))?;
            return Ok((body, next));
        }
        // -- Read the cache (lock scope: just the lookup) -------------------
        // Still within TTL → return without hitting the network. With an
        // ETag the revalidation is cheap (and free of rate-limit cost on
        // GitHub), but a burst inside the TTL still needs no request.
        let cached = read_cached(&key);
        if let Some((_, body, fetched_at)) = &cached {
            if fetched_at.elapsed() < SHORT_TTL {
                return Ok((body.clone(), cached_next(&key)));
            }
        }
        // -- Miss / stale: one network leg per key, however many callers ----
        // ...at one write generation: a read issued after our own write must
        // not join (or be overwritten by) a request that left before it.
        let generation = write_generation(&url);
        let this = self.clone();
        let flight_key = format!("{key}@{generation}");
        page_flights()
            .run(&flight_key, move || async move {
                this.fetch_page(req, url, key, cached, generation).await
            })
            .await
    }

    /// The network leg of [`get_cached_page`](Self::get_cached_page): a
    /// conditional GET when the stale entry has an ETag (`304` → the cached
    /// body), otherwise a plain one; the result is stored unless one of our
    /// writes cleared the scope after `generation` was captured.
    async fn fetch_page(
        &self,
        req: reqwest::Request,
        url: String,
        key: String,
        cached: Option<(Option<String>, String, Instant)>,
        generation: u64,
    ) -> Result<(String, Option<String>)> {
        let rebuild = |validator: Option<&str>| {
            let mut b = self.client.get(&url);
            for (name, value) in req.headers() {
                b = b.header(name.clone(), value.clone());
            }
            if let Some(tag) = validator {
                b = b.header("If-None-Match", tag);
            }
            b
        };

        let mut validator = None;
        if let Some((Some(tag), body, _)) = cached {
            // Same retry / rate-limit classification as `send`, with 304
            // surfaced as success.
            let resp = self.send_checked(rebuild(Some(&tag))).await?;
            if resp.status().as_u16() == 304 {
                // Not Modified: refresh fetched_at, return cached body. (A
                // write since `generation` already removed the entry, and a
                // newer read's entry is not ours to vouch for.)
                let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                let current = write_generation(&url) == generation;
                let next = guard.get_mut(&key).filter(|_| current).and_then(|entry| {
                    entry.fetched_at = Instant::now();
                    entry.next.clone()
                });
                return Ok((body, next));
            }
            validator = Some(resp);
        }
        // TTL elapsed and no ETag, or no usable entry → unconditional GET.

        // -- A 200 from the conditional GET, or no usable entry -------------
        // No validator is sent on the fresh GET, so `send_checked` can only
        // return a 2xx there.
        let resp = match validator {
            Some(resp) => resp,
            None => self.send_checked(rebuild(None)).await?,
        };
        let new_etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let next =
            parse_next_link(resp.headers()).or_else(|| x_next_page_url(&url, resp.headers()));
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
                url,
                next: next.clone(),
            },
            generation,
        );
        Ok((new_body, next))
    }

    /// [`paginate_json`](Self::paginate_json) with every page read through the
    /// cached GET: each page revalidates on its own (a new comment lands on the
    /// LAST page of an ascending list, so page 1's 304 can't vouch for the
    /// rest), at no rate-limit cost on GitHub, and a repeat within the TTL
    /// makes no request at all. Same 20-page cap and same-origin rule.
    pub async fn paginate_json_cached(
        &self,
        first_rb: reqwest::RequestBuilder,
        client: &reqwest::Client,
        auth_header: (&'static str, String),
    ) -> Result<Vec<Value>> {
        const MAX_PAGES: usize = 20;
        let origin = first_rb
            .try_clone()
            .and_then(|b| b.build().ok())
            .map(|r| r.url().clone());
        let mut all: Vec<Value> = Vec::new();
        let mut rb = Some(first_rb);
        let mut pages = 0usize;
        while let Some(b) = rb.take() {
            let (body, next) = self.get_cached_page(b).await?;
            let page: Value = serde_json::from_str(&body)
                .map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))?;
            if let Some(arr) = page.as_array() {
                all.extend_from_slice(arr);
            }
            pages += 1;
            let Some(url) = next else { break };
            if pages >= MAX_PAGES {
                break;
            }
            if !same_origin(origin.as_ref(), &url) {
                tracing::warn!(
                    provider = self.provider,
                    "pagination link leaves the API origin — not followed"
                );
                break;
            }
            rb = Some(client.get(&url).header(auth_header.0, &auth_header.1));
        }
        Ok(all)
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
        // Where the first page came from: a `Link: rel="next"` pointing at
        // ANY other origin is not followed — the auth header rides on every
        // page, and a redirect-to-elsewhere link must not carry the token.
        let origin = first_rb
            .try_clone()
            .and_then(|b| b.build().ok())
            .map(|r| r.url().clone());
        let resp = self.send(first_rb).await?;
        let next = parse_next_link(resp.headers());
        let page: Value = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))?;
        if let Some(arr) = page.as_array() {
            all.extend_from_slice(arr);
        }
        let mut next_url = next;
        let mut pages = 1usize;
        while let Some(url) = next_url {
            if pages >= MAX_PAGES {
                break;
            }
            if !same_origin(origin.as_ref(), &url) {
                tracing::warn!(
                    provider = self.provider,
                    "pagination link leaves the API origin — not followed"
                );
                break;
            }
            let rb = client.get(&url).header(auth_header.0, &auth_header.1);
            let resp = self.send(rb).await?;
            let nxt = parse_next_link(resp.headers());
            let page: Value = resp
                .json()
                .await
                .map_err(|e| Error::Upstream(format!("{}: bad json: {e}", self.provider)))?;
            if let Some(arr) = page.as_array() {
                all.extend_from_slice(arr);
            }
            next_url = nxt;
            pages += 1;
        }
        Ok(all)
    }

    /// Send WITHOUT erroring on non-2xx so callers can inspect the status.
    pub async fn send_raw(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let write = rb
            .try_clone()
            .and_then(|c| c.build().ok())
            .is_some_and(|req| !is_idempotent_read(req.method()));
        let resp = rb
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?;
        if write && resp.status().is_success() {
            invalidate_scope(resp.url().as_str());
        }
        Ok(resp)
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
    let msg = format!(
        "{} {}: {}",
        provider,
        status.as_u16(),
        extract_message(body)
    );
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
            let url = part
                .split(';')
                .next()?
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>');
            return Some(url.to_string());
        }
    }
    None
}

/// True when `next` has the same scheme, host and port as the first page.
/// An unknown origin (the first request couldn't be inspected) or an
/// unparsable link follows nothing.
fn same_origin(origin: Option<&reqwest::Url>, next: &str) -> bool {
    let (Some(o), Ok(n)) = (origin, reqwest::Url::parse(next)) else {
        return false;
    };
    o.scheme() == n.scheme()
        && o.host_str() == n.host_str()
        && o.port_or_known_default() == n.port_or_known_default()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::{
        cache_key, extract_auth, insert_into, rate_limit_wait, read_from, repo_scope, same_origin,
        x_next_page_url, CachedGet, Generations, Http, CACHE_MAX_BODY, CACHE_MAX_BYTES,
        CACHE_MAX_ENTRIES, GENERATIONS_MAX, SHORT_TTL,
    };

    /// The scope map is bounded, yet no scope's generation ever goes back to
    /// a value a pre-write read could still hold (that read would then store
    /// its stale body).
    #[test]
    fn write_generations_stay_monotonic_across_a_prune() {
        let mut g = Generations {
            by_scope: HashMap::new(),
            floor: 0,
        };
        g.bump("a/".into());
        let a = g.get("a/");
        assert!(a > 0);
        assert_eq!(g.get("unwritten/"), 0);
        for i in 0..GENERATIONS_MAX {
            g.bump(format!("s{i}/"));
        }
        assert!(g.by_scope.len() <= GENERATIONS_MAX, "pruned");
        assert!(
            g.get("a/") > a,
            "a pruned scope reads as the floor, never lower"
        );
        let before = g.get("unwritten/");
        g.bump("b/".into());
        assert!(g.get("b/") > before, "a bump always moves past the floor");
    }

    #[test]
    fn pagination_follows_only_same_origin_links() {
        let o = reqwest::Url::parse("https://api.github.com/repos/o/r/pulls?per_page=100").unwrap();
        assert!(same_origin(
            Some(&o),
            "https://api.github.com/repositories/1/pulls?page=2"
        ));
        assert!(same_origin(Some(&o), "https://api.github.com:443/x?page=2"));
        assert!(!same_origin(Some(&o), "https://evil.example/x?page=2"));
        assert!(!same_origin(Some(&o), "http://api.github.com/x?page=2"));
        assert!(!same_origin(Some(&o), "https://api.github.com:8443/x"));
        assert!(!same_origin(Some(&o), "not a url"));
        assert!(!same_origin(None, "https://api.github.com/x"));
    }
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn entry(body: &str, fetched_at: Instant) -> CachedGet {
        CachedGet {
            etag: None,
            body: body.to_string(),
            fetched_at,
            url: String::new(),
            next: None,
        }
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
    fn repo_scope_per_forge() {
        assert_eq!(
            repo_scope("https://api.github.com/repos/acme/app/pulls/7/reviews").as_deref(),
            Some("https://api.github.com/repos/acme/app/")
        );
        assert_eq!(
            repo_scope("https://api.bitbucket.org/2.0/repositories/ws/app/pullrequests/3")
                .as_deref(),
            Some("https://api.bitbucket.org/2.0/repositories/ws/app/")
        );
        assert_eq!(
            repo_scope("https://gl.example/api/v4/projects/acme%2Fapp/merge_requests/1/notes")
                .as_deref(),
            Some("https://gl.example/api/v4/projects/acme%2Fapp/")
        );
        assert_eq!(repo_scope("https://api.github.com/user"), None);
        assert_eq!(repo_scope("https://api.github.com/repos/acme"), None);
    }

    #[test]
    fn insert_respects_byte_budget_and_body_cap() {
        let mut m = HashMap::new();
        let big = "x".repeat(CACHE_MAX_BODY + 1);
        insert_into(&mut m, "k".into(), entry(&big, Instant::now()));
        assert!(m.is_empty(), "an oversized body is never cached");
        let chunk = "y".repeat(CACHE_MAX_BODY);
        for i in 0..(CACHE_MAX_BYTES / CACHE_MAX_BODY + 4) {
            insert_into(&mut m, format!("k{i}"), entry(&chunk, Instant::now()));
        }
        let total: usize = m.values().map(|e| e.body.len()).sum();
        assert!(total <= CACHE_MAX_BYTES, "total {total}");
    }

    #[test]
    fn x_next_page_becomes_a_page_url() {
        let h = headers(&[("x-next-page", "3")]);
        assert_eq!(
            x_next_page_url(
                "https://gl.example/api/v4/projects/1/merge_requests?per_page=50&page=2",
                &h
            )
            .as_deref(),
            Some("https://gl.example/api/v4/projects/1/merge_requests?per_page=50&page=3")
        );
        assert_eq!(
            x_next_page_url("https://x/y", &headers(&[("x-next-page", "")])),
            None
        );
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
    fn short_ttl_is_fifteen_seconds() {
        assert_eq!(SHORT_TTL, Duration::from_secs(15));
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
            insert_into(
                &mut map,
                format!("k{i}"),
                entry("body", base + Duration::from_millis(i as u64)),
            );
        }
        // Refreshing an existing key is not a new entry — nothing may be dropped.
        insert_into(
            &mut map,
            "k0".to_string(),
            entry("fresh", base + Duration::from_secs(10)),
        );
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
        insert_into(
            &mut map,
            "fresh".to_string(),
            entry("new body", Instant::now()),
        );

        assert!(
            read_from(&mut map, "stale").is_none(),
            "an hour-old entry is a miss"
        );
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
        let secs = rate_limit_wait(429, &headers(&[("retry-after", &when.to_rfc2822())]))
            .expect("date retry-after is a rate limit")
            .as_secs();
        assert!((58..=61).contains(&secs), "got {secs}s");
        // x-ratelimit-reset (absolute unix ts) on a 403 with the budget spent.
        let reset = chrono::Utc::now().timestamp() + 120;
        let secs = rate_limit_wait(
            403,
            &headers(&[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", &reset.to_string()),
            ]),
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

    // -- 5xx retry policy --------------------------------------------------

    /// A write that drew a 502 may already have landed (GitHub commits, then
    /// fails the response): re-sending posted comments twice and reported
    /// landed merges as failures. Reads keep their single retry.
    #[tokio::test]
    async fn server_error_retries_reads_but_never_writes() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/comment"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/merge"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/read"))
            .respond_with(ResponseTemplate::new(502))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/read"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;

        let http = Http::new("github");
        let post = http
            .client()
            .post(format!("{}/comment", server.uri()))
            .json(&serde_json::json!({ "body": "hi" }));
        assert!(http.send(post).await.is_err());
        let put = http
            .client()
            .put(format!("{}/merge", server.uri()))
            .json(&serde_json::json!({ "merge_method": "merge" }));
        assert!(http.send(put).await.is_err());
        let get = http.client().get(format!("{}/read", server.uri()));
        assert_eq!(http.text(get).await.unwrap(), "ok");

        let reqs = server.received_requests().await.unwrap();
        let count = |m: &str| reqs.iter().filter(|r| r.method.as_str() == m).count();
        assert_eq!(count("POST"), 1, "a 5xx write is never re-sent");
        assert_eq!(count("PUT"), 1, "a 5xx write is never re-sent");
        assert_eq!(count("GET"), 2, "a 5xx read is retried exactly once");
    }

    /// S15-302: a comment POST answered 502 may have landed. The repo's
    /// cached comment list must not survive it, or the duplicate-post lookup
    /// reads the stale list and reposts.
    #[tokio::test]
    async fn failed_write_still_invalidates_the_repo_cache() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        super::enable_cache_for_tests(&server.uri());
        let list = "/repos/acme/app/pulls/7/comments";
        Mock::given(method("GET"))
            .and(path(list))
            .respond_with(ResponseTemplate::new(200).set_body_string("[]"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(list))
            .respond_with(ResponseTemplate::new(200).set_body_string("[{\"id\":1}]"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(list))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;

        let http = Http::new("github");
        let url = format!("{}{list}", server.uri());
        assert_eq!(http.get_cached(http.client().get(&url)).await.unwrap(), "[]");
        let post = http.client().post(&url).json(&serde_json::json!({ "body": "hi" }));
        assert!(http.send(post).await.is_err());
        assert_eq!(
            http.get_cached(http.client().get(&url)).await.unwrap(),
            "[{\"id\":1}]",
            "the read after a failed write goes to the forge, not the cache"
        );
    }
}
