//! Shared HTTP layer for provider clients: one reqwest client with a 20s
//! timeout, a single retry on 429/5xx with backoff, and uniform error mapping
//! into `Error::Upstream` carrying the HTTP status plus the provider's
//! message field when parseable.
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

    /// Send with one retry on 429/5xx; returns the successful response.
    pub async fn send(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::Response> {
        let retry = rb.try_clone();
        let resp = rb
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?;

        let status = resp.status();
        let retryable = status.as_u16() == 429 || status.is_server_error();
        let resp = if retryable {
            if let Some(rb2) = retry {
                tokio::time::sleep(Duration::from_millis(700)).await;
                rb2.send()
                    .await
                    .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?
            } else {
                resp
            }
        } else {
            resp
        };

        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        Err(Error::Upstream(format!(
            "{} {}: {}",
            self.provider,
            status.as_u16(),
            extract_message(&body)
        )))
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
        let cached = {
            let guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
            guard.get(&key).map(|e| {
                (
                    e.etag.clone(),
                    e.body.clone(),
                    e.fetched_at,
                )
            })
        };

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

                let resp = rb2
                    .send()
                    .await
                    .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?;

                if resp.status().as_u16() == 304 {
                    // Not Modified: refresh fetched_at, return cached body.
                    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(entry) = guard.get_mut(&key) {
                        entry.fetched_at = Instant::now();
                    }
                    return Ok(body);
                }

                // 200 (or error) — fall through to the common store-or-error path.
                if resp.status().is_success() {
                    let new_etag = resp
                        .headers()
                        .get("etag")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    let new_body = resp
                        .text()
                        .await
                        .map_err(|e| Error::Upstream(format!("{}: body read: {e}", self.provider)))?;
                    let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
                    guard.insert(
                        key,
                        CachedGet {
                            etag: new_etag,
                            body: new_body.clone(),
                            fetched_at: Instant::now(),
                        },
                    );
                    return Ok(new_body);
                }

                let status = resp.status();
                let err_body = resp.text().await.unwrap_or_default();
                return Err(Error::Upstream(format!(
                    "{} {}: {}",
                    self.provider,
                    status.as_u16(),
                    extract_message(&err_body)
                )));
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

        let resp = rb_fresh
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("{}: {e}", self.provider)))?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(Error::Upstream(format!(
                "{} {}: {}",
                self.provider,
                status.as_u16(),
                extract_message(&err_body)
            )));
        }

        let new_etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let new_body = resp
            .text()
            .await
            .map_err(|e| Error::Upstream(format!("{}: body read: {e}", self.provider)))?;

        {
            let mut guard = get_cache().lock().unwrap_or_else(|p| p.into_inner());
            guard.insert(
                key,
                CachedGet {
                    etag: new_etag,
                    body: new_body.clone(),
                    fetched_at: Instant::now(),
                },
            );
        }

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
        let body = resp.text().await.unwrap_or_default();
        Err(Error::Upstream(format!(
            "{} {}: {}",
            self.provider,
            status.as_u16(),
            extract_message(&body)
        )))
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
    use super::{cache_key, extract_auth, SHORT_TTL};
    use std::time::Duration;

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
}
