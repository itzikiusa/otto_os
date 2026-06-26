//! Embedder seam. The default is a deterministic hashed-feature stub (no deps,
//! no network) — real embedders (local fastembed / remote OpenAI/Voyage) plug in
//! behind this trait behind cargo features.

use std::time::Duration;

use async_trait::async_trait;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Outbound embed request timeout (matches the otto-mcp client posture).
const EMBED_TIMEOUT: Duration = Duration::from_secs(20);
/// Hard cap on an embeddings response body (defends against a hostile/buggy
/// endpoint streaming an unbounded body). 32 MiB is far above any real
/// embeddings batch (dim × count × ~8 bytes of JSON).
const MAX_EMBED_BODY: u64 = 32 * 1024 * 1024;

#[async_trait]
pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// Validate a remote embedder base URL against the outbound SSRF guard
/// (loopback / private / link-local / metadata addresses are refused). The
/// daemon calls this when a user configures a custom embedder endpoint so a
/// hostile URL is rejected before any request is ever made.
pub async fn validate_remote_url(url: &str) -> Result<()> {
    otto_netguard::check_url(url)
        .await
        .map_err(|e| Error::Invalid(format!("embedder url rejected: {e}")))
}

// ---------------------------------------------------------------------------
// Remote embedders (OpenAI / Voyage). The `base_url` is injectable so they can
// be pointed at a mock server in tests. API keys come from the keychain (passed
// in as a string ref-resolved value, never stored in the DB).
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct EmbedReq<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbedItem {
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbedResp {
    data: Vec<EmbedItem>,
}

/// Which remote provider to construct.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteProvider {
    Openai,
    Voyage,
}

/// OpenAI-/Voyage-compatible embeddings client (both speak `{model,input}` →
/// `{data:[{embedding}]}` at `<base>/embeddings`).
pub struct RemoteEmbedder {
    base_url: String,
    api_key: String,
    model: String,
    dim: usize,
    http: reqwest::Client,
}

impl RemoteEmbedder {
    pub fn new(provider: RemoteProvider, api_key: String) -> Self {
        match provider {
            RemoteProvider::Openai => Self::with(
                api_key,
                "https://api.openai.com/v1".into(),
                "text-embedding-3-small".into(),
                1536,
            ),
            RemoteProvider::Voyage => Self::with(
                api_key,
                "https://api.voyageai.com/v1".into(),
                "voyage-3".into(),
                1024,
            ),
        }
    }

    pub fn with(api_key: String, base_url: String, model: String, dim: usize) -> Self {
        // Bounded, redirect-guarded client: a timeout so a hung provider can't
        // stall ingest, and the netguard redirect policy so a 3xx can't bounce
        // the request to a private/loopback address.
        let http = reqwest::Client::builder()
            .timeout(EMBED_TIMEOUT)
            .redirect(otto_netguard::redirect_policy())
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model,
            dim,
            http,
        }
    }
}

#[async_trait]
impl Embedder for RemoteEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }
    fn dim(&self) -> usize {
        self.dim
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let resp = self
            .http
            .post(format!("{}/embeddings", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&EmbedReq {
                model: &self.model,
                input: texts,
            })
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("embed request: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "embed provider returned {}",
                resp.status()
            )));
        }
        // Reject an oversized body before buffering it.
        if let Some(len) = resp.content_length() {
            if len > MAX_EMBED_BODY {
                return Err(Error::Upstream(format!(
                    "embed response too large: {len} bytes"
                )));
            }
        }
        let body: EmbedResp = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("embed decode: {e}")))?;
        Ok(body.data.into_iter().map(|d| d.embedding).collect())
    }
}

/// Deterministic hashed bag-of-words embedder — unit-normalized. Shared tokens →
/// closer vectors, which is enough to exercise the whole semantic path in tests
/// and as a zero-dependency default. Not a substitute for a real model.
pub struct StubEmbedder {
    dim: usize,
}

impl StubEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

#[async_trait]
impl Embedder for StubEmbedder {
    fn model_id(&self) -> &str {
        "stub-v1"
    }
    fn dim(&self) -> usize {
        self.dim
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for tok in t
                    .to_lowercase()
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|s| !s.is_empty())
                {
                    // FNV-1a hash → feature bucket.
                    let mut h = 1469598103934665603u64;
                    for b in tok.bytes() {
                        h ^= b as u64;
                        h = h.wrapping_mul(1099511628211);
                    }
                    v[(h as usize) % self.dim] += 1.0;
                }
                let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
                for x in &mut v {
                    *x /= n;
                }
                v
            })
            .collect())
    }
}
