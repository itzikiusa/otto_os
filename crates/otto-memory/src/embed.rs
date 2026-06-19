//! Embedder seam. The default is a deterministic hashed-feature stub (no deps,
//! no network) — real embedders (local fastembed / remote OpenAI/Voyage) plug in
//! behind this trait behind cargo features.

use async_trait::async_trait;
use otto_core::Result;

#[async_trait]
pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
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
