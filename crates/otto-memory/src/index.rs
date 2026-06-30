//! Vector index seam. Two local implementations plug in behind one trait:
//! [`BruteForceIndex`] (exact cosine over the BLOB vectors — sub-millisecond at
//! single-user scale) and [`HnswIndex`] (an HNSW ANN graph for large
//! collections, with an exact fallback below a size threshold so small sets /
//! tests stay exact and deterministic). The remote [`crate::backends::QdrantIndex`]
//! implements the same trait for the optional Qdrant vector layer.

use std::collections::HashMap;
use std::sync::RwLock;

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

// ---------------------------------------------------------------------------
// HNSW ANN index. Below `exact_threshold` vectors we run exact cosine (cheap,
// correct, deterministic — what tests and typical single-user workspaces hit);
// above it we build an HNSW graph (instant-distance) per workspace and cache it,
// rebuilding only when the workspace's vector count changes. The graph maps a
// point's L2 distance back to a memory id; we return cosine similarity so the
// score is comparable to the brute-force path.
// ---------------------------------------------------------------------------

/// A vector point for instant-distance, carrying its own values + a normalized
/// copy so distance is a plain dot product (cosine for unit vectors).
#[derive(Clone)]
struct VecPoint(Vec<f32>);

impl instant_distance::Point for VecPoint {
    fn distance(&self, other: &Self) -> f32 {
        // Cosine distance (1 - cosine sim). Robust to non-normalized inputs.
        1.0 - cosine(&self.0, &other.0)
    }
}

struct CachedGraph {
    count: usize,
    map: instant_distance::HnswMap<VecPoint, String>,
}

pub struct HnswIndex {
    repo: MemoriesRepo,
    model: String,
    exact_threshold: usize,
    cache: RwLock<HashMap<String, CachedGraph>>,
}

impl HnswIndex {
    pub fn new(pool: SqlitePool, model: String) -> Self {
        Self {
            repo: MemoriesRepo::new(pool),
            model,
            exact_threshold: 1024,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Override the size below which exact search is used (mainly for tests).
    pub fn with_threshold(mut self, t: usize) -> Self {
        self.exact_threshold = t;
        self
    }

    async fn exact(&self, ws: &str, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
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

#[async_trait]
impl VectorIndex for HnswIndex {
    async fn knn(&self, ws: &str, query: &[f32], k: usize) -> Result<Vec<(String, f32)>> {
        let count = self.repo.count_vectors(ws, &self.model).await?;
        if count <= self.exact_threshold {
            return self.exact(ws, query, k).await;
        }
        // Rebuild the cached graph if the workspace's vector count changed.
        let need_build = {
            let guard = self.cache.read().ok();
            !guard
                .as_ref()
                .and_then(|g| g.get(ws))
                .map(|c| c.count == count)
                .unwrap_or(false)
        };
        if need_build {
            let rows = self.repo.all_vectors(ws, &self.model).await?;
            let (points, values): (Vec<VecPoint>, Vec<String>) =
                rows.into_iter().map(|(id, v)| (VecPoint(v), id)).unzip();
            let map = instant_distance::Builder::default().build(points, values);
            if let Ok(mut guard) = self.cache.write() {
                guard.insert(ws.to_string(), CachedGraph { count, map });
            }
        }
        // Query the cached graph inside a synchronous scope so the (non-Send)
        // read guard is never held across an await.
        let cached_result: Option<Vec<(String, f32)>> = {
            match self.cache.read() {
                Ok(guard) => guard.get(ws).map(|cached| {
                    let mut search = instant_distance::Search::default();
                    let qp = VecPoint(query.to_vec());
                    cached
                        .map
                        .search(&qp, &mut search)
                        .take(k)
                        .map(|item| (item.value.clone(), 1.0 - item.distance))
                        .collect()
                }),
                Err(_) => None,
            }
        };
        match cached_result {
            Some(r) => Ok(r),
            // Lost a race against an evict / poisoned lock: fall back to exact.
            None => self.exact(ws, query, k).await,
        }
    }
}
