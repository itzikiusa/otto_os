//! Embedder seam. The default is a deterministic hashed-feature stub (no deps,
//! no network) — real embedders (local fastembed / remote OpenAI/Voyage) plug in
//! behind this trait behind cargo features.

use std::time::Duration;

use async_trait::async_trait;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Outbound embed request timeout for REMOTE providers (OpenAI/Voyage are fast).
const EMBED_TIMEOUT: Duration = Duration::from_secs(20);
/// Ollama runs a local neural model — a large one (e.g. qwen3-embedding:8b) needs
/// a slow first load + seconds per text, so a single (batched) call can take
/// minutes. A generous timeout prevents silently dropping embeddings.
const OLLAMA_TIMEOUT: Duration = Duration::from_secs(600);
/// Hard cap on an embeddings response body (defends against a hostile/buggy
/// endpoint streaming an unbounded body). 32 MiB is far above any real
/// embeddings batch (dim × count × ~8 bytes of JSON).
const MAX_EMBED_BODY: u64 = 32 * 1024 * 1024;

#[async_trait]
pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    /// Embed DOCUMENTS (stored content). Some models (nomic) need an asymmetric
    /// document prefix — impls apply it here.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// Embed a QUERY. Asymmetric models (nomic: `search_query:` vs
    /// `search_document:`) embed queries differently from documents — overriding
    /// this is what makes retrieval actually work. Default = treat as a document.
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self
            .embed(std::slice::from_ref(&text.to_string()))
            .await?
            .into_iter()
            .next()
            .unwrap_or_default())
    }
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

// ---------------------------------------------------------------------------
// Local code-aware embedder (Vault v2 default). Deterministic + offline (no
// model download, no native ONNX) but materially better than the FNV stub for
// CODE: it splits identifiers (camelCase / snake_case / dotted paths) into
// subtokens AND hashes character trigrams, so `GetLimits` and `GetUserLimits`
// share dimensions (subword overlap) and near-miss spellings still land close.
// This is the hermetic default E2E runs against; real *neural* local embeddings
// are available via the Ollama provider (see `OllamaEmbedder`), and remote
// neural via OpenAI/Voyage (`RemoteEmbedder`).
// ---------------------------------------------------------------------------

/// Split an identifier into lowercased subtokens on case + separator boundaries.
/// `GetUserLimits` → [getuserlimits, get, user, limits]; `get_limits.v2` →
/// [getlimits, get, limits, v2]. The whole joined token is kept too so exact
/// identifier matches still align.
pub(crate) fn code_subtokens(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in raw.split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        // camelCase / PascalCase / digit boundaries → parts.
        let mut parts: Vec<String> = Vec::new();
        let mut cur = String::new();
        let chars: Vec<char> = word.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let boundary = matches!(prev, Some(p)
                if (ch.is_uppercase() && p.is_lowercase())
                || (ch.is_ascii_digit() != p.is_ascii_digit()));
            if boundary && !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur.push(ch.to_ascii_lowercase());
        }
        if !cur.is_empty() {
            parts.push(cur);
        }
        let joined = word.to_lowercase();
        if parts.len() != 1 || parts.first().map(|p| p != &joined).unwrap_or(false) {
            out.push(joined);
        }
        out.extend(parts.into_iter().filter(|p| !p.is_empty()));
    }
    out
}

/// FNV-1a of a string → a feature bucket.
fn bucket(s: &str, dim: usize) -> usize {
    let mut h = 1469598103934665603u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    (h as usize) % dim
}

/// Deterministic, offline, code-aware local embedder. Default dim 512.
pub struct LocalCodeEmbedder {
    dim: usize,
}

impl LocalCodeEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Default for LocalCodeEmbedder {
    fn default() -> Self {
        Self { dim: 512 }
    }
}

#[async_trait]
impl Embedder for LocalCodeEmbedder {
    fn model_id(&self) -> &str {
        "local-code-v1"
    }
    fn dim(&self) -> usize {
        self.dim
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0f32; self.dim];
                for tok in code_subtokens(t) {
                    // Whole subtoken (full weight).
                    v[bucket(&tok, self.dim)] += 1.0;
                    // Character trigrams (subword/fuzzy signal, down-weighted).
                    if tok.len() >= 3 {
                        let chars: Vec<char> = tok.chars().collect();
                        for w in chars.windows(3) {
                            let tri: String = w.iter().collect();
                            v[bucket(&format!("#{tri}"), self.dim)] += 0.35;
                        }
                    }
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

// ---------------------------------------------------------------------------
// Ollama — real *local neural* embeddings via a localhost Ollama server (e.g.
// `nomic-embed-text`, 768-dim). Local by design, so it deliberately does NOT go
// through the SSRF guard (which blocks loopback); the URL is an admin setting.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct OllamaEmbedReq<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct OllamaEmbedResp {
    embeddings: Vec<Vec<f32>>,
}

/// Embeddings client for a local Ollama server (`POST <base>/api/embed`).
pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    model_id: String,
    dim: usize,
    /// Asymmetric task prefixes. nomic REQUIRES `search_document:` /
    /// `search_query:` — without them retrieval similarity collapses (~0.4) and
    /// ranking is poor. Empty for models that don't use prefixes.
    doc_prefix: String,
    query_prefix: String,
    http: reqwest::Client,
}

/// Per-model-family asymmetric task prefixes, returned as `(doc_prefix,
/// query_prefix)`. Instruction-tuned retrieval models each have their OWN
/// convention — applying the wrong one (or none) collapses retrieval quality:
///
/// - **nomic** (`nomic-embed-text`, `-v2-moe`): `search_document:` / `search_query:`
///   on BOTH sides — asymmetric, both prefixed.
/// - **qwen3-embedding**: documents are embedded RAW; only the QUERY is wrapped in
///   Qwen's instruct format `Instruct: {task}\nQuery: {q}`. Using nomic's prefix
///   here would actively hurt it.
/// - **mxbai-embed-large** / **bge-*-en**: query-only retrieval instruction.
/// - **bge-m3**, **all-minilm**, and anything unknown: NO prefix (symmetric) —
///   the conservative default, never worse than the model's own baseline.
///
/// Matching is on the bare model name (lowercased, `ollama:`/tag stripped) so
/// `qwen3-embedding:8b`, `qwen3-embedding`, etc. all resolve the same.
pub(crate) fn embed_prefixes_for(model: &str) -> (String, String) {
    let m = model
        .to_lowercase()
        .trim_start_matches("ollama:")
        .to_string();
    if m.contains("nomic") {
        // Asymmetric: both sides prefixed.
        ("search_document: ".to_string(), "search_query: ".to_string())
    } else if m.contains("qwen") {
        // Qwen3-Embedding: raw docs, instruct-wrapped queries.
        (
            String::new(),
            "Instruct: Given a search query, retrieve relevant code and documentation that answers the query\nQuery: "
                .to_string(),
        )
    } else if m.contains("mxbai") || (m.contains("bge") && !m.contains("m3")) {
        // Query-only retrieval instruction; documents raw.
        (
            String::new(),
            "Represent this sentence for searching relevant passages: ".to_string(),
        )
    } else {
        // bge-m3, all-minilm, snowflake/arctic, and unknown models: symmetric.
        (String::new(), String::new())
    }
}

impl OllamaEmbedder {
    /// Default local Ollama on `127.0.0.1:11434` with `nomic-embed-text` (768).
    pub fn new(model: Option<String>, dim: Option<usize>, base_url: Option<String>) -> Self {
        let model = model.unwrap_or_else(|| "nomic-embed-text".to_string());
        // Each model family has its OWN prefix convention — see embed_prefixes_for.
        let (doc_prefix, query_prefix) = embed_prefixes_for(&model);
        Self {
            base_url: base_url
                .unwrap_or_else(|| "http://127.0.0.1:11434".to_string())
                .trim_end_matches('/')
                .to_string(),
            model_id: format!("ollama:{model}"),
            dim: dim.unwrap_or(768),
            model,
            doc_prefix,
            query_prefix,
            http: reqwest::Client::builder()
                .timeout(OLLAMA_TIMEOUT)
                .build()
                .unwrap_or_default(),
        }
    }

    /// Raw embed call against Ollama for already-prefixed inputs.
    async fn embed_raw(&self, input: &[String]) -> Result<Vec<Vec<f32>>> {
        let resp = self
            .http
            .post(format!("{}/api/embed", self.base_url))
            .json(&OllamaEmbedReq {
                model: &self.model,
                input,
            })
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("ollama embed request: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Upstream(format!(
                "ollama returned {} (is the model pulled? `ollama pull {}`)",
                resp.status(),
                self.model
            )));
        }
        if let Some(len) = resp.content_length() {
            if len > MAX_EMBED_BODY {
                return Err(Error::Upstream(format!("ollama response too large: {len} bytes")));
            }
        }
        let body: OllamaEmbedResp = resp
            .json()
            .await
            .map_err(|e| Error::Upstream(format!("ollama decode: {e}")))?;
        Ok(body.embeddings)
    }
}

#[async_trait]
impl Embedder for OllamaEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }
    fn dim(&self) -> usize {
        self.dim
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Documents get the document prefix (for asymmetric models like nomic).
        let prefixed: Vec<String> = if self.doc_prefix.is_empty() {
            texts.to_vec()
        } else {
            texts.iter().map(|t| format!("{}{t}", self.doc_prefix)).collect()
        };
        self.embed_raw(&prefixed).await
    }
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let q = format!("{}{text}", self.query_prefix);
        Ok(self.embed_raw(std::slice::from_ref(&q)).await?.into_iter().next().unwrap_or_default())
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

#[cfg(test)]
mod prefix_tests {
    use super::embed_prefixes_for;

    #[test]
    fn nomic_is_asymmetric_both_sides() {
        for m in ["nomic-embed-text", "nomic-embed-text-v2-moe", "ollama:nomic-embed-text"] {
            let (doc, query) = embed_prefixes_for(m);
            assert_eq!(doc, "search_document: ", "doc prefix for {m}");
            assert_eq!(query, "search_query: ", "query prefix for {m}");
        }
    }

    #[test]
    fn qwen_docs_raw_query_instruct() {
        for m in ["qwen3-embedding:0.6b", "qwen3-embedding:8b", "qwen3-embedding"] {
            let (doc, query) = embed_prefixes_for(m);
            assert!(doc.is_empty(), "qwen docs must be raw for {m}");
            assert!(query.starts_with("Instruct:"), "qwen query must be instruct-wrapped for {m}");
            assert!(query.ends_with("Query: "), "qwen query must end with the Query: tag for {m}");
        }
    }

    #[test]
    fn mxbai_and_bge_en_are_query_only() {
        for m in ["mxbai-embed-large", "bge-large-en-v1.5"] {
            let (doc, query) = embed_prefixes_for(m);
            assert!(doc.is_empty(), "docs raw for {m}");
            assert!(query.contains("Represent this sentence"), "retrieval instruction for {m}");
        }
    }

    #[test]
    fn bge_m3_and_minilm_and_unknown_are_symmetric() {
        for m in ["bge-m3", "all-minilm", "snowflake-arctic-embed", "some-future-model"] {
            let (doc, query) = embed_prefixes_for(m);
            assert!(doc.is_empty() && query.is_empty(), "{m} should be symmetric (no prefix)");
        }
    }
}
