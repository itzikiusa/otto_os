//! Pure retrieval helpers: Reciprocal Rank Fusion + light re-rank signals.

use std::collections::HashMap;

/// Reciprocal Rank Fusion over two id-rankings (best-first). Returns (id, score)
/// descending. `k0` is the standard RRF damping constant (≈60).
pub fn rrf_fuse(keyword: &[String], semantic: &[String], k0: f32) -> Vec<(String, f32)> {
    let mut acc: HashMap<String, f32> = HashMap::new();
    for (rank, id) in keyword.iter().enumerate() {
        *acc.entry(id.clone()).or_default() += 1.0 / (k0 + rank as f32 + 1.0);
    }
    for (rank, id) in semantic.iter().enumerate() {
        *acc.entry(id.clone()).or_default() += 1.0 / (k0 + rank as f32 + 1.0);
    }
    let mut v: Vec<(String, f32)> = acc.into_iter().collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v
}

pub struct RerankSignals {
    pub recency_days: f32,
    pub access_count: i64,
    pub confidence: f32,
    pub salience: f32,
    pub scope_match: bool,
}

/// Apply light priors on top of the fused base score.
pub fn rerank_score(base: f32, s: &RerankSignals, half_life_days: f32) -> f32 {
    let recency = 0.5f32.powf(s.recency_days / half_life_days.max(0.1));
    let usage = (1.0 + s.access_count as f32).ln();
    let scope = if s.scope_match { 0.15 } else { 0.0 };
    base * (1.0 + 0.3 * recency + 0.05 * usage + 0.2 * (s.confidence * s.salience)) + scope
}
