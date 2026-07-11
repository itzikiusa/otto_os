//! Pure retrieval helpers: light re-rank signals over the keyword ranking.

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
