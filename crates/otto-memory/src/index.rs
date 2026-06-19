//! Vector index seam. The default is brute-force cosine over the BLOB vectors in
//! SQLite (sub-millisecond at single-user scale). An ANN impl (in-Rust HNSW /
//! sqlite-vec) plugs in behind this same trait for large collections.

use async_trait::async_trait;
use otto_core::Result;
use otto_state::MemoriesRepo;
use sqlx::SqlitePool;

#[async_trait]
pub trait VectorIndex: Send + Sync {
    /// Nearest neighbors by cosine similarity (higher = closer), best-first.
    async fn knn(&self, ws: &str, query: &[f32], k: usize) -> Result<Vec<(String, f32)>>;
}

pub struct BruteForceIndex {
    repo: MemoriesRepo,
    model: String,
}

impl BruteForceIndex {
    pub fn new(pool: SqlitePool, model: String) -> Self {
        Self {
            repo: MemoriesRepo::new(pool),
            model,
        }
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (mut d, mut na, mut nb) = (0f32, 0f32, 0f32);
    for i in 0..n {
        d += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        d / (na.sqrt() * nb.sqrt())
    }
}

#[async_trait]
impl VectorIndex for BruteForceIndex {
    async fn knn(&self, ws: &str, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        let mut scored: Vec<(String, f32)> = self
            .repo
            .all_vectors(ws, &self.model)
            .await?
            .into_iter()
            .map(|(id, v)| (id, cosine(query, &v)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(k);
        Ok(scored)
    }
}
