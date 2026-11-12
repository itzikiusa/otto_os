//! Pure completion/verification decisions, separate from scheduling and sessions.
use otto_core::domain::{GoalLoopDefinition, GoalLoopEvaluation, GoalLoopLedger};

/// Separates a criterion's evidence headline from the verify command's output
/// tail stored after it. The iteration's `evaluation_json` keeps the output so
/// a person can see why a check failed; ledgers and signatures keep only the
/// headline (see [`evidence_headline`]).
pub const VERIFY_OUTPUT_MARKER: &str = "\n--- verify output ---\n";
/// How much of a verify command's output an iteration keeps (its tail).
pub const VERIFY_OUTPUT_TAIL_BYTES: usize = 8 * 1024;
/// Evidence for an agent-judged criterion the evaluator called met without
/// saying why. Such a claim counts as not met.
pub const NO_EVIDENCE: &str =
    "Marked met without evidence, so it counts as not met. The evaluator must cite what it checked.";

/// `headline` plus the last [`VERIFY_OUTPUT_TAIL_BYTES`] of `output`.
pub fn with_output(headline: &str, output: &str) -> String {
    let output = output.trim_end();
    if output.trim().is_empty() {
        return headline.to_string();
    }
    let mut start = output.len().saturating_sub(VERIFY_OUTPUT_TAIL_BYTES);
    while !output.is_char_boundary(start) {
        start += 1;
    }
    let cut = if start > 0 {
        "… earlier output omitted\n"
    } else {
        ""
    };
    format!("{headline}{VERIFY_OUTPUT_MARKER}{cut}{}", &output[start..])
}

/// The evidence without any stored verify output.
pub fn evidence_headline(evidence: &str) -> &str {
    evidence
        .split_once(VERIFY_OUTPUT_MARKER)
        .map_or(evidence, |(head, _)| head)
}

/// A `met: true` with blank evidence is a claim, not a check: it counts as
/// not met and says why. Command and human criteria always carry evidence
/// (exit status / the person's record), so in practice this binds the
/// agent-judged ones.
pub fn require_evidence(eval: &mut GoalLoopEvaluation) {
    for c in eval.criteria.iter_mut() {
        if c.met && c.evidence.trim().is_empty() {
            c.met = false;
            c.evidence = NO_EVIDENCE.into();
        }
    }
}

/// Every criterion met with evidence (command criteria only count as met when
/// their command exited 0) means the goal is done, whatever the evaluator
/// would still like to do. Its wish to continue is kept in the rationale.
/// A `blocked` verdict is left alone: it asks a person for something.
/// Returns whether the verdict was changed.
pub fn complete_when_all_met(eval: &mut GoalLoopEvaluation) -> bool {
    let all_met = !eval.criteria.is_empty()
        && eval
            .criteria
            .iter()
            .all(|c| c.met && !c.evidence.trim().is_empty());
    if eval.verdict != "continue" || !all_met {
        return false;
    }
    let wish = [eval.feedback.trim(), eval.rationale.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("no reason given");
    eval.verdict = "achieved".into();
    eval.rationale = format!(
        "Every acceptance criterion is met with evidence, so the goal is done. The evaluator asked to continue: {wish}"
    );
    true
}

pub fn reconcile_human(
    definition: &GoalLoopDefinition,
    ledger: &GoalLoopLedger,
    eval: &mut GoalLoopEvaluation,
) {
    // Keep only the defined criterion ids and exactly one assessment per id.
    let old = std::mem::take(&mut eval.criteria);
    eval.criteria = definition
        .acceptance_criteria
        .iter()
        .map(|criterion| {
            let mut c = old
                .iter()
                .find(|c| c.id == criterion.id)
                .cloned()
                .unwrap_or(otto_core::domain::EvalCriterion {
                    id: criterion.id.clone(),
                    met: false,
                    evidence: "not assessed".into(),
                });
            if criterion.verify_kind == "human" {
                if let Some(approval) = ledger.verification(criterion) {
                    c.met = true;
                    c.evidence = format!(
                        "Verified by {}: {}",
                        approval.verified_by, approval.evidence
                    );
                } else {
                    c.met = false;
                    c.evidence = "Awaiting human verification".into();
                }
            }
            c
        })
        .collect();
    let human_pending = definition
        .acceptance_criteria
        .iter()
        .any(|c| c.verify_kind == "human" && ledger.verification(c).is_none());
    let automated_met = definition
        .acceptance_criteria
        .iter()
        .filter(|c| c.verify_kind != "human")
        .all(|c| eval.criteria.iter().any(|e| e.id == c.id && e.met));
    if human_pending && automated_met {
        eval.verdict = "blocked".into();
        eval.feedback = "Work is ready for human verification. Record evidence for the human criteria, then Resume.".into();
    } else if eval.verdict == "achieved" && eval.criteria.iter().any(|c| !c.met) {
        eval.verdict = "continue".into();
    }
    let count = eval.criteria.len();
    eval.progress_pct = (eval.criteria.iter().filter(|c| c.met).count() * 100)
        .checked_div(count)
        .unwrap_or(0) as u32;
}

pub fn record_progress(ledger: &mut GoalLoopLedger, eval: &GoalLoopEvaluation) -> bool {
    ledger.next_action = eval.feedback.clone();
    let mut unmet: Vec<_> = eval
        .criteria
        .iter()
        .filter(|c| !c.met)
        // The headline only: a verify command's output tail (timings, paths)
        // differs on every run and would hide a repeated failure.
        .map(|c| (&c.id, evidence_headline(&c.evidence).trim()))
        .collect();
    unmet.sort();
    let signature = serde_json::to_string(&unmet).unwrap_or_default();
    if unmet.is_empty() {
        ledger.repeated_failures = 0;
    } else if ledger.last_failure_signature == signature {
        ledger.repeated_failures += 1;
    } else {
        ledger.repeated_failures = 1;
    }
    ledger.last_failure_signature = signature;
    ledger.repeated_failures >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::*;
    fn definition() -> GoalLoopDefinition {
        serde_json::from_value(serde_json::json!({"title":"Goal", "acceptance_criteria":[
            {"id":"h", "text":"Human check", "verify":"Inspect output", "verify_kind":"human"}
        ]}))
        .unwrap()
    }
    fn eval() -> GoalLoopEvaluation {
        serde_json::from_value(serde_json::json!({"progress_pct":100,"verdict":"achieved","criteria":[{"id":"h","met":true,"evidence":"model claims done"}]})).unwrap()
    }
    #[test]
    fn model_cannot_satisfy_human_criterion() {
        let mut e = eval();
        reconcile_human(&definition(), &GoalLoopLedger::default(), &mut e);
        assert!(!e.criteria[0].met);
        assert_eq!(e.verdict, "blocked");
        assert_eq!(e.progress_pct, 0);
    }
    #[test]
    fn revised_criteria_invalidate_human_approval() {
        let mut d = definition();
        let mut ledger = GoalLoopLedger::default();
        ledger.verifications.push(GoalHumanVerification {
            criterion_id: "h".into(),
            criterion_revision: d.acceptance_criteria[0].revision(),
            verified_by: "human-id".into(),
            evidence: "Saw it".into(),
            verified_at: chrono::Utc::now(),
        });
        let mut e = eval();
        reconcile_human(&d, &ledger, &mut e);
        assert!(e.criteria[0].met);
        d.acceptance_criteria[0].text = "Different check".into();
        reconcile_human(&d, &ledger, &mut e);
        assert!(!e.criteria[0].met);
    }
    #[test]
    fn repeated_identical_failure_blocks_but_new_evidence_resets() {
        let mut ledger = GoalLoopLedger::default();
        let mut e = eval();
        e.criteria[0].met = false;
        assert!(!record_progress(&mut ledger, &e));
        assert!(record_progress(&mut ledger, &e));
        e.criteria[0].evidence = "different result".into();
        assert!(!record_progress(&mut ledger, &e));
    }

    fn agent_eval(verdict: &str, criteria: serde_json::Value) -> GoalLoopEvaluation {
        serde_json::from_value(serde_json::json!({
            "progress_pct": 100, "verdict": verdict, "criteria": criteria,
            "feedback": "polish the docs", "rationale": "more could be done"
        }))
        .unwrap()
    }

    #[test]
    fn a_met_claim_without_evidence_counts_as_not_met() {
        let mut e = agent_eval(
            "achieved",
            serde_json::json!([
                {"id":"a","met":true,"evidence":"  "},
                {"id":"b","met":true},
                {"id":"c","met":true,"evidence":"read src/lib.rs: handler returns 409"}
            ]),
        );
        require_evidence(&mut e);
        assert!(!e.criteria[0].met);
        assert_eq!(e.criteria[0].evidence, NO_EVIDENCE);
        assert!(!e.criteria[1].met, "a missing evidence field is blank too");
        assert!(e.criteria[2].met, "cited evidence is kept");
        // The definition-driven reconcile then refuses the achieved verdict.
        let d: GoalLoopDefinition = serde_json::from_value(serde_json::json!({
            "title":"Goal","acceptance_criteria":[
                {"id":"a","text":"A","verify":"inspect","verify_kind":"agent"},
                {"id":"b","text":"B","verify":"inspect","verify_kind":"agent"},
                {"id":"c","text":"C","verify":"inspect","verify_kind":"agent"}
            ]
        }))
        .unwrap();
        reconcile_human(&d, &GoalLoopLedger::default(), &mut e);
        assert_eq!(e.verdict, "continue");
        assert_eq!(e.progress_pct, 33);
    }

    #[test]
    fn all_criteria_met_with_evidence_completes_despite_continue() {
        let mut e = agent_eval(
            "continue",
            serde_json::json!([
                {"id":"a","met":true,"evidence":"verify command exited 0"},
                {"id":"b","met":true,"evidence":"checked the error message"}
            ]),
        );
        assert!(complete_when_all_met(&mut e));
        assert_eq!(e.verdict, "achieved");
        assert!(
            e.rationale.contains("polish the docs"),
            "the evaluator's wish is recorded: {}",
            e.rationale
        );
    }

    #[test]
    fn completion_override_needs_every_criterion_and_respects_blocked() {
        let unmet = serde_json::json!([
            {"id":"a","met":true,"evidence":"ok"},
            {"id":"b","met":false,"evidence":"verify command failed (exit 1)"}
        ]);
        let mut e = agent_eval("continue", unmet);
        assert!(!complete_when_all_met(&mut e));
        assert_eq!(e.verdict, "continue");

        let met = serde_json::json!([{"id":"a","met":true,"evidence":"ok"}]);
        let mut e = agent_eval("blocked", met);
        assert!(!complete_when_all_met(&mut e), "blocked asks a person");
        assert_eq!(e.verdict, "blocked");

        let mut e = agent_eval("continue", serde_json::json!([]));
        assert!(!complete_when_all_met(&mut e), "no criteria is not done");

        let mut e = agent_eval(
            "continue",
            serde_json::json!([{"id":"a","met":true,"evidence":""}]),
        );
        assert!(!complete_when_all_met(&mut e), "met without evidence");
    }

    #[test]
    fn verify_output_keeps_a_bounded_tail_after_the_headline() {
        assert_eq!(
            with_output("verify command exited 0", " \n"),
            "verify command exited 0"
        );
        let short = with_output("verify command failed (exit 1)", "assertion failed\n");
        assert_eq!(evidence_headline(&short), "verify command failed (exit 1)");
        assert!(short.ends_with("assertion failed"));
        // Multi-byte output cut on a char boundary, newest bytes kept.
        let long = format!("{}TAIL", "é".repeat(VERIFY_OUTPUT_TAIL_BYTES));
        let kept = with_output("verify command failed (exit 2)", &long);
        let (_, out) = kept.split_once(VERIFY_OUTPUT_MARKER).unwrap();
        assert!(out.starts_with("… earlier output omitted"));
        assert!(out.ends_with("TAIL"));
        assert!(out.len() <= VERIFY_OUTPUT_TAIL_BYTES + 64, "{}", out.len());
        assert_eq!(evidence_headline("plain evidence"), "plain evidence");
    }

    #[test]
    fn repeated_failure_ignores_a_changing_output_tail() {
        let mut ledger = GoalLoopLedger::default();
        let mut e = eval();
        e.criteria[0].met = false;
        e.criteria[0].evidence = with_output("verify command failed (exit 1)", "took 1.2s");
        assert!(!record_progress(&mut ledger, &e));
        e.criteria[0].evidence = with_output("verify command failed (exit 1)", "took 3.4s");
        assert!(
            record_progress(&mut ledger, &e),
            "the same failing check is stagnation even when its timings differ"
        );
    }
}
