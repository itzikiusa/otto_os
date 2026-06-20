//! Shared HTTP layer for provider clients: one reqwest client with a 20s
//! timeout, a single retry on 429/5xx with backoff, and uniform error mapping
//! into `Error::Upstream` carrying the HTTP status plus the provider's
//! message field when parseable.

use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::Value;

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
