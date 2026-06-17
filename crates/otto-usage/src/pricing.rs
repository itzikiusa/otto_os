//! Best-effort USD cost estimation from token counts.
//!
//! Used only when a recorder didn't supply an explicit `cost_usd`. Rates are
//! per 1M tokens (input, output) and approximate published list prices; the
//! result is an estimate, surfaced as such in the UI. Unknown models cost 0.

/// Estimate USD cost for a turn given the model id and token counts.
pub fn estimate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let m = model.to_lowercase();
    // (input $/1M, output $/1M)
    let (in_rate, out_rate) = if m.contains("opus") {
        (15.0, 75.0)
    } else if m.contains("haiku") {
        (0.80, 4.0)
    } else if m.contains("sonnet") {
        (3.0, 15.0)
    } else if m.contains("gpt-4o-mini") || m.contains("o4-mini") || m.contains("-mini") {
        (0.15, 0.60)
    } else if m.contains("gpt") || m.contains("o3") || m.contains("o1") || m.contains("codex") {
        (2.5, 10.0)
    } else {
        (0.0, 0.0)
    };
    (input_tokens as f64 / 1_000_000.0) * in_rate + (output_tokens as f64 / 1_000_000.0) * out_rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_costs_more_than_sonnet() {
        let opus = estimate_cost("claude-opus-4", 1_000_000, 1_000_000);
        let sonnet = estimate_cost("claude-sonnet-4", 1_000_000, 1_000_000);
        assert!(opus > sonnet);
        assert_eq!(opus, 90.0);
        assert_eq!(sonnet, 18.0);
    }

    #[test]
    fn unknown_model_is_free() {
        assert_eq!(estimate_cost("mystery", 1000, 1000), 0.0);
    }
}
