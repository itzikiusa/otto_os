//! Eval-lab scoring math — pure, deterministic, and the single source of truth for
//! the composite score and the promote gate. No I/O: `otto-server` runs the
//! commands and assembles the proof pack; this module turns the collected signals
//! into a number and a gate decision.
//!
//! The composite is a weighted mean over the signals that **ran**. A signal that
//! didn't run (e.g. no lint command, no human rating) is excluded from the
//! denominator, so a repo without a lint step isn't silently penalized.

use crate::domain::{DiffScore, EvalScore, HumanScore, PromoteGate, SignalScore};

/// A test/lint command result → a 0/100 gate signal (binary: it passed or it
/// didn't). `ran=false` when no command was configured.
pub fn signal_from_cmd(ran: bool, success: bool, detail: impl Into<String>) -> SignalScore {
    SignalScore {
        ran,
        score: if ran && success { 100.0 } else { 0.0 },
        detail: detail.into(),
    }
}

/// A review-findings score (0–100) that already came from `score_findings`.
pub fn signal_score(ran: bool, score: f64, detail: impl Into<String>) -> SignalScore {
    SignalScore { ran, score: score.clamp(0.0, 100.0), detail: detail.into() }
}

/// The diff-quality signal: `100 − min(80, risk)` — small, focused, low-risk diffs
/// score high; sprawling / migration-touching diffs score lower. `risk` must be
/// computed over the diff artifact ALONE (one-element slice) so the failing-test /
/// review penalties inside `compute_risk` don't double-count the separate signals.
pub fn diff_score(files: u32, additions: u32, deletions: u32, risky: u32, risk: u8) -> DiffScore {
    let score = 100.0 - (risk.min(80) as f64);
    DiffScore {
        ran: files > 0,
        files_changed: files,
        additions,
        deletions,
        risky,
        score,
        detail: format!("{files} file(s), +{additions}/-{deletions}, risk {risk}"),
    }
}

/// The human-rating signal (0–5 → 0–100). `ran` is implied by `rating.is_some()`.
pub fn human_score(rating: Option<u8>, note: impl Into<String>, rater: impl Into<String>) -> HumanScore {
    HumanScore {
        rating,
        note: note.into(),
        rater: rater.into(),
        score: rating.map(|r| (r.min(5) as f64) / 5.0 * 100.0).unwrap_or(0.0),
    }
}

/// Weighted mean over the signals that ran, renormalized to that present subset,
/// clamped 0–100. Returns 0.0 when nothing ran.
pub fn compute_composite(s: &EvalScore) -> f64 {
    let w = &s.weights;
    let mut num = 0.0;
    let mut den = 0.0;
    let mut add = |ran: bool, weight: f64, score: f64| {
        if ran && weight > 0.0 {
            num += weight * score;
            den += weight;
        }
    };
    add(s.tests.ran, w.tests, s.tests.score);
    add(s.lint.ran, w.lint, s.lint.score);
    add(s.diff.ran, w.diff, s.diff.score);
    add(s.review.ran, w.review, s.review.score);
    add(s.human.rating.is_some(), w.human, s.human.score);
    if den <= 0.0 {
        0.0
    } else {
        (num / den).clamp(0.0, 100.0)
    }
}

/// Decide whether a run's winning skill may be promoted. `proof_status` is the best
/// iteration's proof-pack status. "passed" or "waived" satisfy the proof clause.
pub fn promote_gate(
    best_score: Option<f64>,
    proof_status: &str,
    min_score: f64,
    require_proof: bool,
) -> PromoteGate {
    let score = best_score.unwrap_or(0.0);
    let score_ok = best_score.is_some() && score >= min_score;
    let proof_ok = !require_proof || matches!(proof_status, "passed" | "waived");
    let mut reasons = Vec::new();
    if !score_ok {
        reasons.push(format!(
            "composite score {:.0} is below the {:.0} threshold",
            score, min_score
        ));
    }
    if !proof_ok {
        reasons.push(format!(
            "proof pack is '{}' (a code change needs a passing recognized test command to pass proof)",
            if proof_status.is_empty() { "missing" } else { proof_status }
        ));
    }
    PromoteGate {
        allowed: score_ok && proof_ok,
        score,
        threshold: min_score,
        proof_status: proof_status.to_string(),
        require_proof,
        score_ok,
        proof_ok,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ScoreWeights;

    fn score(tests: SignalScore, lint: SignalScore, diff: DiffScore, review: SignalScore, human: HumanScore) -> EvalScore {
        let mut s = EvalScore {
            tests,
            lint,
            diff,
            review,
            human,
            weights: ScoreWeights::default(),
            composite: 0.0,
            proof_status: String::new(),
            done_score: 0,
        };
        s.composite = compute_composite(&s);
        s
    }

    #[test]
    fn composite_renormalizes_over_present_signals() {
        // Only tests + diff ran; lint/review/human absent → composite is the
        // weight-weighted mean of just those two, not diluted by absent signals.
        let s = score(
            signal_from_cmd(true, true, "pass"),       // 100, w 0.35
            SignalScore::default(),                      // absent
            diff_score(1, 5, 0, 0, 0),                   // 100, w 0.15
            SignalScore::default(),                      // absent
            human_score(None, "", ""),                   // absent
        );
        assert!((s.composite - 100.0).abs() < 1e-9, "got {}", s.composite);
    }

    #[test]
    fn composite_blends_pass_and_fail() {
        // tests pass (100, w .35), review weak (40, w .25) → 100*.35+40*.25 over .60.
        let s = score(
            signal_from_cmd(true, true, ""),
            SignalScore::default(),
            DiffScore::default(),
            signal_score(true, 40.0, ""),
            human_score(None, "", ""),
        );
        let expected = (100.0 * 0.35 + 40.0 * 0.25) / (0.35 + 0.25);
        assert!((s.composite - expected).abs() < 1e-9, "got {}", s.composite);
    }

    #[test]
    fn composite_zero_when_nothing_ran() {
        let s = score(
            SignalScore::default(),
            SignalScore::default(),
            DiffScore::default(),
            SignalScore::default(),
            human_score(None, "", ""),
        );
        assert_eq!(s.composite, 0.0);
    }

    #[test]
    fn human_rating_maps_to_percent() {
        assert_eq!(human_score(Some(5), "", "").score, 100.0);
        assert_eq!(human_score(Some(0), "", "").score, 0.0);
        assert!((human_score(Some(3), "", "").score - 60.0).abs() < 1e-9);
    }

    #[test]
    fn diff_score_penalizes_risk() {
        assert_eq!(diff_score(1, 10, 0, 0, 0).score, 100.0);
        assert_eq!(diff_score(1, 10, 0, 1, 50).score, 50.0);
        assert_eq!(diff_score(1, 10, 0, 5, 90).score, 20.0); // capped at -80
    }

    #[test]
    fn gate_blocks_low_score() {
        let g = promote_gate(Some(60.0), "passed", 80.0, true);
        assert!(!g.allowed && !g.score_ok && g.proof_ok);
        assert!(g.reasons.iter().any(|r| r.contains("below")));
    }

    #[test]
    fn gate_blocks_unpassed_proof() {
        let g = promote_gate(Some(95.0), "partial", 80.0, true);
        assert!(!g.allowed && g.score_ok && !g.proof_ok);
        assert!(g.reasons.iter().any(|r| r.contains("proof pack")));
    }

    #[test]
    fn gate_allows_when_both_pass() {
        let g = promote_gate(Some(85.0), "passed", 80.0, true);
        assert!(g.allowed && g.reasons.is_empty());
    }

    #[test]
    fn gate_waived_proof_counts_as_ok() {
        let g = promote_gate(Some(85.0), "waived", 80.0, true);
        assert!(g.proof_ok && g.allowed);
    }

    #[test]
    fn gate_ignores_proof_when_not_required() {
        let g = promote_gate(Some(85.0), "partial", 80.0, false);
        assert!(g.allowed);
    }

    #[test]
    fn gate_none_score_blocks() {
        let g = promote_gate(None, "passed", 80.0, true);
        assert!(!g.allowed && !g.score_ok);
    }
}
