//! Pure review scoring and completion gates shared by workflow execution.
use serde_json::{json, Value};

/// Per-severity deductions (percent off 100) over the OPEN findings —
/// `scoring: { bug, warn, info }`. Defaults (20/5/0) prioritize must-fix
/// and should-fix defects; optional observations do not lower the score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreWeights {
    pub bug: i64,
    pub warn: i64,
    pub info: i64,
}

impl Default for ScoreWeights {
    fn default() -> Self {
        Self {
            bug: 20,
            warn: 5,
            info: 0,
        }
    }
}

impl ScoreWeights {
    /// The node's `scoring` overrides over the defaults. A missing or
    /// non-integer weight keeps its default.
    pub fn from_params(p: &Value) -> Self {
        let d = Self::default();
        let weight = |key: &str, default: i64| -> i64 {
            p.get("scoring")
                .and_then(|s| s.get(key))
                .and_then(Value::as_i64)
                .unwrap_or(default)
        };
        Self {
            bug: weight("bug", d.bug),
            warn: weight("warn", d.warn),
            info: weight("info", d.info),
        }
    }
}

/// The review's own score: 100 minus the weighted OPEN findings, clamped to
/// 0..=100. An EMPTY diff scores 0, not 100: the formula starts at a perfect
/// 100 and subtracts findings, so "nothing was reviewed" and "reviewed,
/// spotless" would produce the same number. Only one of them earned it.
pub fn review_score(bugs: u64, warns: u64, infos: u64, w: ScoreWeights, no_changes: bool) -> i64 {
    if no_changes {
        return 0;
    }
    (100 - bugs as i64 * w.bug - warns as i64 * w.warn - infos as i64 * w.info).clamp(0, 100)
}

/// The gate's score for one target: the review's own score, averaged with
/// the goals grade when the node has goals (integer mean, rounding down).
pub fn blended_score(review: i64, goals: Option<i64>) -> i64 {
    match goals {
        Some(gs) => (review + gs) / 2,
        None => review,
    }
}

/// Whether one target's review passed, and why (not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewVerdict {
    /// Completed (status `done`) with full reviewer coverage.
    pub complete: bool,
    pub passed: bool,
    pub reason: String,
}

/// A review must COMPLETE (status "done" with full coverage), have had
/// something to review, have no open must-fix findings, AND clear the threshold to pass. The reasons stay
/// distinct: an incomplete review (error / timeout / still running) and an
/// empty diff BOTH produce zero findings, so reporting either as "below
/// threshold" is nonsense — the score was never the problem.
pub fn review_verdict(
    status: &str,
    coverage_complete: bool,
    no_changes: bool,
    must_fix: u64,
    score: i64,
    threshold: i64,
    base: &str,
) -> ReviewVerdict {
    let complete = status == "done" && coverage_complete;
    let passed = complete && !no_changes && must_fix == 0 && score >= threshold;
    let reason = if passed {
        "passed".to_string()
    } else if !complete {
        if status == "done" {
            "review coverage incomplete or summary fallback not allowed".into()
        } else {
            format!("review did not complete (status: {status})")
        }
    } else if no_changes {
        format!("empty diff vs {base} — nothing was reviewed")
    } else if must_fix > 0 {
        format!("{must_fix} must-fix finding(s) remain open")
    } else {
        format!("below threshold ({score} < {threshold})")
    };
    ReviewVerdict {
        complete,
        passed,
        reason,
    }
}

/// Read the goals agent's reply: `(goals_score, goals_detail)`.
///
/// The reply is searched for the object the prompt asked for — one with an
/// overall `score` or a `goals` array — so a quoted `{"name":…}` from a
/// `package.json` ahead of it can't shadow it. A reply with no such object,
/// or one without an overall score, is NOT parseable: it keeps the review
/// score (the goals can't be graded) and says so in the detail, instead of
/// silently reporting `goals: []` as if no goals were configured.
pub fn goals_from_reply(reply: &str, review_score: i64) -> (Option<i64>, Value) {
    let found = otto_core::text::extract_json_where(reply, |v| {
        v.get("score").and_then(Value::as_i64).is_some()
            || v.get("goals").is_some_and(Value::is_array)
    });
    match found {
        Some(v) => match v.get("score").and_then(Value::as_i64) {
            Some(gs) => (
                Some(gs.clamp(0, 100)),
                v.get("goals").cloned().unwrap_or(json!([])),
            ),
            None => {
                let mut detail = v
                    .get("goals")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                detail.push(json!({ "note": "goals eval reply had no overall score" }));
                (Some(review_score), Value::Array(detail))
            }
        },
        None => (
            Some(review_score),
            json!([{ "note": "goals eval reply not parseable" }]),
        ),
    }
}

/// Fold the per-target outputs into the step output: `(output, passed)`.
///
/// Single target: its output as is. Several: the FIRST reviewed repo's fields
/// stay mirrored at the top level (back-compat for single-repo consumers);
/// the strict union gates multi-repo runs — worst score, pass only when EVERY
/// target was reviewed and passed — with per-repo detail under `reviews[]`
/// and every reviewed reference under `repos[]`. `outs` is non-empty.
pub fn aggregate(
    outs: Vec<serde_json::Map<String, Value>>,
    n_targets: usize,
) -> (serde_json::Map<String, Value>, bool) {
    let mut out = outs.first().cloned().unwrap_or_default();
    if n_targets <= 1 {
        let passed = out.get("passed").and_then(Value::as_bool).unwrap_or(false);
        return (out, passed);
    }
    let agg_score = outs
        .iter()
        .filter_map(|o| o.get("score").and_then(Value::as_i64))
        .min()
        .unwrap_or(0);
    let agg_passed = outs.len() == n_targets
        && outs
            .iter()
            .all(|o| o.get("passed").and_then(Value::as_bool).unwrap_or(false));
    // One target that did not complete makes the whole step not a verdict:
    // no score (the min of the others would read as the result) and
    // `outcome: incomplete`, which fails the step.
    let incomplete = outs
        .iter()
        .any(|o| o.get("outcome").and_then(Value::as_str) == Some("incomplete"));
    let outcome = match (agg_passed, incomplete) {
        (true, _) => "passed",
        (_, true) => "incomplete",
        _ => "failed",
    };
    out.insert(
        "score".into(),
        if incomplete {
            Value::Null
        } else {
            json!(agg_score)
        },
    );
    out.insert("passed".into(), json!(agg_passed));
    out.insert("outcome".into(), json!(outcome));
    // EVERY reviewed reference, in the `repos[]` shape collect_pr_targets
    // consumes — without this a downstream git_pr would see only the first
    // repo mirrored at top level and never open the other entries' PRs.
    let repo_targets: Vec<Value> = outs
        .iter()
        .map(|o| {
            let mut m = serde_json::Map::new();
            for k in ["repo_id", "worktree", "base"] {
                if let Some(v) = o.get(k) {
                    m.insert(k.to_string(), v.clone());
                }
            }
            Value::Object(m)
        })
        .collect();
    out.insert("repos".into(), Value::Array(repo_targets));
    out.insert(
        "reviews".into(),
        Value::Array(outs.into_iter().map(Value::Object).collect()),
    );
    (out, agg_passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-12: a target whose verifier never started read as "score 0,
    /// FAILED". Across repos it must stay "incomplete" with no score.
    #[test]
    fn one_incomplete_target_makes_the_step_incomplete_with_no_score() {
        let t = |score: Value, passed: bool, outcome: &str| {
            json!({"score": score, "passed": passed, "outcome": outcome, "repo_id": outcome})
                .as_object()
                .cloned()
                .unwrap()
        };
        let (out, passed) = aggregate(
            vec![
                t(json!(95), true, "passed"),
                t(Value::Null, false, "incomplete"),
            ],
            2,
        );
        assert!(!passed);
        assert_eq!(out["outcome"], "incomplete");
        assert!(out["score"].is_null());
        let (out, _) = aggregate(
            vec![t(json!(95), true, "passed"), t(json!(60), false, "failed")],
            2,
        );
        assert_eq!(
            (out["outcome"].clone(), out["score"].clone()),
            (json!("failed"), json!(60))
        );
        let (out, passed) = aggregate(vec![t(json!(90), true, "passed")], 1);
        assert!(passed);
        assert_eq!(out["outcome"], "passed");
    }

    #[test]
    fn must_fix_blocks_even_when_weight_or_goals_allow_a_passing_score() {
        for weights in [
            ScoreWeights::default(),
            ScoreWeights {
                bug: 0,
                ..ScoreWeights::default()
            },
        ] {
            let score = blended_score(review_score(1, 0, 0, weights, false), Some(100));
            assert!(score >= 80);
            let blocked = review_verdict("done", true, false, 1, score, 80, "main");
            assert!(blocked.complete);
            assert!(!blocked.passed);
            assert_eq!(blocked.reason, "1 must-fix finding(s) remain open");
            assert!(review_verdict("done", true, false, 0, score, 80, "main").passed);
        }
    }
}
