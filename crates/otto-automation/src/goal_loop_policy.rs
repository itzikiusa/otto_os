//! Pure completion/verification decisions, separate from scheduling and sessions.
use otto_core::domain::{GoalLoopDefinition, GoalLoopEvaluation, GoalLoopLedger};

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
        .map(|c| (&c.id, c.evidence.trim()))
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

pub fn missing_required_proof(
    required: bool,
    status: Option<otto_core::proof::ProofStatus>,
) -> bool {
    required && status != Some(otto_core::proof::ProofStatus::Passed)
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
    fn required_proof_has_no_budget_exhaustion_exception() {
        use otto_core::proof::ProofStatus;
        for status in [None, Some(ProofStatus::Partial), Some(ProofStatus::Failed)] {
            assert!(missing_required_proof(true, status));
        }
        assert!(!missing_required_proof(true, Some(ProofStatus::Passed)));
        assert!(!missing_required_proof(false, None));
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
}
