//! Best-effort USD cost estimation from token counts.
//!
//! Used only when a recorder didn't supply an explicit `cost_usd`. Rates are
//! per 1M tokens and track published list prices as of [`PRICED_AS_OF`]; the
//! result is an estimate, surfaced as such in the UI.
//!
//! Four token classes are priced independently:
//!   * **input** — uncached prompt tokens, at the model's base input rate.
//!   * **output** — generated tokens, at the model's output rate.
//!   * **cache read** — tokens served from the prompt cache, at ~0.1× input.
//!   * **cache write** — tokens written to the prompt cache, at ~1.25× input
//!     (the 5-minute-TTL rate the agent CLIs use; the 1-hour TTL is 2×, but the
//!     transcripts report a single `cache_creation` count with no TTL, so we
//!     price the common case).
//!
//! Unknown models fall back to a conservative non-zero tier (see
//! [`FALLBACK`]) rather than silently costing $0 — a brand-new model id must
//! never under-report spend. Such turns are tagged via [`is_priced`] so callers
//! can flag estimates made against the fallback.

/// Date the rate table below was last reconciled against published list prices.
/// Bump this whenever a rate changes. (Source: Anthropic pricing, claude-api
/// reference — Opus $5/$25, Sonnet $3/$15, Haiku $1/$5, Fable 5 $10/$50 per 1M.)
pub const PRICED_AS_OF: &str = "2026-06-19";

/// Cache-read tokens cost ~0.1× the base input rate.
const CACHE_READ_FACTOR: f64 = 0.1;
/// Cache-write (5m TTL) tokens cost ~1.25× the base input rate.
const CACHE_WRITE_FACTOR: f64 = 1.25;

/// Per-model rates, all per 1M tokens.
#[derive(Clone, Copy)]
struct Rates {
    /// Base input $/1M.
    input: f64,
    /// Output $/1M.
    output: f64,
}

impl Rates {
    const fn new(input: f64, output: f64) -> Self {
        Self { input, output }
    }

    /// Cache-read $/1M (derived from the base input rate).
    fn cache_read(&self) -> f64 {
        self.input * CACHE_READ_FACTOR
    }

    /// Cache-write $/1M (derived from the base input rate).
    fn cache_write(&self) -> f64 {
        self.input * CACHE_WRITE_FACTOR
    }
}

/// Conservative non-zero fallback for unrecognized / brand-new models. Priced
/// at the Opus tier so a new flagship model is over-estimated rather than
/// billed at $0. Turns priced with this should be flagged via [`is_priced`].
const FALLBACK: Rates = Rates::new(5.0, 25.0);

/// Resolve a model id to its rate card. Matching is case-insensitive and
/// substring-based but ordered most-specific-first so a known model is never
/// missed (e.g. "fable" / "mythos" before the generic Opus tier, "mini" before
/// the generic gpt tier). Returns `None` for unknown models so the caller can
/// apply the non-zero [`FALLBACK`] and flag the estimate.
fn lookup(model: &str) -> Option<Rates> {
    let m = model.to_lowercase();

    // ── Anthropic ──────────────────────────────────────────────────────────
    // Fable 5 / Mythos 5 — flagship tier. Match before "opus"/"sonnet" so the
    // family name wins even if a vendored id strings them together.
    if m.contains("fable") || m.contains("mythos") {
        return Some(Rates::new(10.0, 50.0));
    }
    if m.contains("haiku") {
        return Some(Rates::new(1.0, 5.0));
    }
    if m.contains("opus") {
        return Some(Rates::new(5.0, 25.0));
    }
    if m.contains("sonnet") {
        return Some(Rates::new(3.0, 15.0));
    }

    // ── OpenAI / Codex ─────────────────────────────────────────────────────
    // "-mini" tiers must be checked before the generic gpt/o-series tier.
    if m.contains("gpt-4o-mini") || m.contains("o4-mini") || m.contains("-mini") {
        return Some(Rates::new(0.15, 0.60));
    }
    if m.contains("gpt") || m.contains("codex") || m.contains("o3") || m.contains("o1") {
        return Some(Rates::new(2.5, 10.0));
    }

    None
}

/// Whether `model` resolves to a known rate card. When `false`, [`estimate_cost`]
/// still returns a non-zero estimate (via [`FALLBACK`]) but the caller should
/// treat the figure as a flagged guess for a new/unknown model.
pub fn is_priced(model: &str) -> bool {
    lookup(model).is_some()
}

/// Estimate USD cost for a turn given the model id and token counts, pricing
/// input, output, cache-read, and cache-write tokens at the per-model rates.
/// Unknown models use the conservative non-zero [`FALLBACK`] rather than $0.
pub fn estimate_cost(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
) -> f64 {
    let r = lookup(model).unwrap_or(FALLBACK);
    let per_m = |tokens: u64, rate: f64| (tokens as f64 / 1_000_000.0) * rate;
    per_m(input_tokens, r.input)
        + per_m(output_tokens, r.output)
        + per_m(cache_read_tokens, r.cache_read())
        + per_m(cache_write_tokens, r.cache_write())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1M tokens of each class makes the per-class rates easy to read off.
    fn cost(model: &str) -> f64 {
        estimate_cost(model, 1_000_000, 1_000_000, 1_000_000, 1_000_000)
    }

    #[test]
    fn opus_costs_more_than_sonnet_more_than_haiku() {
        assert!(cost("claude-opus-4-8") > cost("claude-sonnet-4-6"));
        assert!(cost("claude-sonnet-4-6") > cost("claude-haiku-4-5"));
    }

    #[test]
    fn opus_rates_are_current_and_order_of_magnitude() {
        // Opus: $5 in + $25 out + $0.50 cache-read (0.1×5) + $6.25 cache-write (1.25×5)
        let c = cost("claude-opus-4-8");
        assert!((c - (5.0 + 25.0 + 0.5 + 6.25)).abs() < 1e-9, "got {c}");
        // Sanity: nowhere near the old stale $15/$75 over-bill.
        assert!(c < 40.0, "opus over-billing regression: {c}");
    }

    #[test]
    fn sonnet_and_haiku_rates() {
        // Sonnet: $3 + $15 + $0.30 + $3.75
        assert!((cost("claude-sonnet-4-6") - (3.0 + 15.0 + 0.3 + 3.75)).abs() < 1e-9);
        // Haiku: $1 + $5 + $0.10 + $1.25
        assert!((cost("claude-haiku-4-5") - (1.0 + 5.0 + 0.1 + 1.25)).abs() < 1e-9);
    }

    #[test]
    fn fable_5_resolves_to_flagship_tier() {
        assert!(is_priced("claude-fable-5"));
        assert!(is_priced("claude-mythos-5"));
        // Fable: $10 + $50 + $1.00 + $12.50 — and dearer than Opus.
        assert!((cost("claude-fable-5") - (10.0 + 50.0 + 1.0 + 12.5)).abs() < 1e-9);
        assert!(cost("claude-fable-5") > cost("claude-opus-4-8"));
    }

    #[test]
    fn cache_tokens_are_priced() {
        // Cache-read and cache-write must contribute non-zero cost.
        let read_only = estimate_cost("claude-opus-4-8", 0, 0, 1_000_000, 0);
        let write_only = estimate_cost("claude-opus-4-8", 0, 0, 0, 1_000_000);
        assert!((read_only - 0.5).abs() < 1e-9, "cache read unpriced: {read_only}");
        assert!((write_only - 6.25).abs() < 1e-9, "cache write unpriced: {write_only}");
        // Cache write is the dearer of the two (1.25× vs 0.1× input).
        assert!(write_only > read_only);
    }

    #[test]
    fn unknown_model_hits_nonzero_fallback() {
        assert!(!is_priced("some-future-model-9"));
        // Fallback prices input+output at the Opus tier — definitely not $0.
        let c = estimate_cost("some-future-model-9", 1_000_000, 1_000_000, 0, 0);
        assert!(c > 0.0, "unknown model silently billed $0");
        assert!((c - 30.0).abs() < 1e-9, "fallback not at Opus tier: {c}");
    }

    #[test]
    fn codex_and_mini_tiers_still_resolve() {
        assert!(is_priced("codex"));
        assert!(is_priced("gpt-4o-mini"));
        // -mini must beat the generic gpt tier.
        assert!(cost("gpt-4o-mini") < cost("gpt-4o"));
    }
}
