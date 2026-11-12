//! Which earlier findings a finished run may close as fixed.
//!
//! Closing by ABSENCE ("an earlier finding this run did not report is fixed")
//! was switched off on purpose (3193fad39 forced every run's resolve blocker
//! to "absence is not verification"; 84a3feccc then removed the dead call)
//! and stays off. A run not reporting a finding no longer means anything:
//! - the scope assessment picks a different set of lenses for every run, so
//!   the lens that raised a finding may simply not have run this time;
//! - the final verifier drops claims that are pre-existing or outside the
//!   PR's causal diff (`not_relevant`), and those are not fixed;
//! - the verifier re-words titles between runs, so the same defect can come
//!   back under a new fingerprint and the old row would read "fixed".
//!
//! Instead every prior finding is put to this run explicitly: the scope
//! assessment gives it a disposition with evidence (`scope.rs`), and the
//! final verifier re-checks it as a claim of its own (`history::prior_claims`,
//! always the LAST batch). A prior finding is closed only when BOTH say it is
//! gone, and only when the run can vouch for the code it read.

use crate::engine::{raw_claim_id, DraftComment};
use crate::verification::SUMMARIZER_RETRY_BLOCKER;
use otto_core::domain::ReviewScopePlan;

/// The verifier's word for a prior claim it no longer finds in the code.
const GONE: &str = "not_relevant";

/// Finding ids of the plan's prior findings this run confirmed FIXED.
///
/// `prior_batch` is the batch index `history::prior_claims` went in as (the
/// reviewer batches come first); `parsed` is the verifier's raw output, before
/// [`crate::verification::settle`]. Empty unless the run can vouch for what it
/// read: the managed verifier ran (`fallback_cause` is `None`) and nothing
/// blocks resolution except a summarizer-only retry, which re-verifies the
/// same stored claims against a rebuilt checkout of the reviewed head.
///
/// Per prior finding, all of:
/// - the scope assessment said `fixed` with evidence (a disposition the plan
///   filled in itself — [`crate::scope::NOT_ASSESSED`] or a fallback plan's
///   "Not assessed …" — is always `still_open`, so it never qualifies);
/// - the verifier accounted for its claim with an evidenced `not_relevant`;
/// - no verifier item kept it (`verified`) or folded it into a kept fix
///   (`duplicate`) or left it open (`unverified`).
pub fn confirmed_fixed(
    plan: Option<&ReviewScopePlan>,
    prior_batch: usize,
    parsed: &[DraftComment],
    fallback_cause: Option<&str>,
    resolve_blocked: Option<&str>,
) -> Vec<String> {
    let Some(plan) = plan else {
        return Vec::new();
    };
    if fallback_cause.is_some() || resolve_blocked.is_some_and(|b| b != SUMMARIZER_RETRY_BLOCKER) {
        return Vec::new();
    }
    plan.prior_findings
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.disposition.trim() == "fixed"
                && !p.evidence.trim().is_empty()
                && p.evidence.trim() != crate::scope::NOT_ASSESSED
        })
        .filter(|(j, _)| {
            let id = raw_claim_id(prior_batch, *j);
            let mut covering = parsed
                .iter()
                .filter(|c| c.source_ids.contains(&id))
                .peekable();
            covering.peek().is_some()
                && covering
                    .all(|c| c.verification_complete() && c.disposition.as_deref() == Some(GONE))
        })
        .map(|(_, p)| p.finding_id.clone())
        .collect()
}

/// Close the confirmed-fixed findings (`state` → `resolved`; the workflow
/// `status` people own is untouched). Best-effort: a failed write leaves the
/// findings open, which is the safe side.
pub async fn resolve_confirmed(
    findings: &otto_state::ReviewFindingsRepo,
    review_id: &str,
    ids: &[String],
) {
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    match findings.resolve_confirmed(&refs, review_id).await {
        Ok(0) => {}
        Ok(n) => {
            tracing::info!(review = %review_id, "resolved {n} prior finding(s) confirmed fixed")
        }
        Err(e) => {
            tracing::warn!(review = %review_id, "resolving confirmed-fixed findings failed: {e}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::ReviewFindingDisposition;

    fn plan(priors: &[(&str, &str, &str)]) -> ReviewScopePlan {
        let mut p = crate::scope::documentation_plan("--- a/docs/a.md\n+++ b/docs/a.md\n+x", "h")
            .expect("plan");
        p.prior_findings = priors
            .iter()
            .map(|(id, d, e)| ReviewFindingDisposition {
                finding_id: (*id).into(),
                disposition: (*d).into(),
                evidence: (*e).into(),
            })
            .collect();
        p
    }

    fn item(ids: &[String], disposition: &str, evidence: &str) -> DraftComment {
        serde_json::from_value(serde_json::json!({
            "source_ids": ids, "disposition": disposition, "evidence": evidence,
            "body": "prior claim", "severity": "bug",
        }))
        .expect("draft")
    }

    /// Prior batch 2 (after two reviewer batches): claim j ↔ prior j.
    fn id(j: usize) -> String {
        raw_claim_id(2, j)
    }

    #[test]
    fn a_prior_finding_closes_only_when_scope_and_verifier_both_say_fixed() {
        let p = plan(&[
            ("F0", "fixed", "auth.rs:9 now checks the role"),
            ("F1", "fixed", "api.rs:3 guards it"),
            ("F2", "still_open", "auth.rs:2 unchanged"),
            ("F3", "not_relevant", "predates the PR"),
        ]);
        let parsed = vec![
            item(
                &[id(0)],
                "not_relevant",
                "auth.rs:9 rejects anonymous callers",
            ),
            // The verifier disagrees with scope: still there.
            item(
                &[id(1)],
                "verified",
                "api.rs:3 still reachable from api.rs:8",
            ),
            item(&[id(2)], "not_relevant", "fixed after all"),
            item(&[id(3)], "not_relevant", "predates the PR"),
        ];
        assert_eq!(
            confirmed_fixed(Some(&p), 2, &parsed, None, None),
            vec!["F0"]
        );
    }

    #[test]
    fn filled_in_or_unconfirmed_dispositions_never_close() {
        let p = plan(&[
            ("F0", "still_open", crate::scope::NOT_ASSESSED),
            ("F1", "fixed", crate::scope::NOT_ASSESSED),
            ("F2", "fixed", "x.rs:1"),
            ("F3", "fixed", "y.rs:1"),
            ("F4", "fixed", "z.rs:1"),
        ]);
        let parsed = vec![
            item(&[id(0)], "not_relevant", "gone"),
            item(&[id(1)], "not_relevant", "gone"),
            // Not accounted for by the verifier at all: F2.
            item(&[id(3)], "duplicate", "same fix as the kept comment"),
            // Evidence-less `not_relevant` does not account for a claim.
            item(&[id(4)], "not_relevant", " "),
        ];
        assert!(confirmed_fixed(Some(&p), 2, &parsed, None, None).is_empty());
        // A second item keeping the same claim open vetoes the close.
        let p = plan(&[("F0", "fixed", "a.rs:1")]);
        let both = vec![
            item(&[id(0)], "not_relevant", "gone"),
            item(&[id(0)], "unverified", "could not inspect b.rs"),
        ];
        assert!(confirmed_fixed(Some(&p), 2, &both, None, None).is_empty());
    }

    #[test]
    fn a_run_that_cannot_vouch_closes_nothing() {
        let p = plan(&[("F0", "fixed", "auth.rs:9")]);
        let parsed = vec![item(&[id(0)], "not_relevant", "gone")];
        let ok = |cause, blocked| confirmed_fixed(Some(&p), 2, &parsed, cause, blocked);
        assert!(
            ok(Some("summarizer timed out"), None).is_empty(),
            "fallback"
        );
        assert!(ok(None, Some("partial diff (files omitted)")).is_empty());
        assert!(ok(None, Some("PR source not checked out")).is_empty());
        // A summarizer-only retry re-verifies the same claims: it may close.
        assert_eq!(ok(None, Some(SUMMARIZER_RETRY_BLOCKER)), vec!["F0"]);
        assert!(confirmed_fixed(None, 2, &parsed, None, None).is_empty());
        // The wrong batch index matches nothing.
        assert!(confirmed_fixed(Some(&p), 0, &parsed, None, None).is_empty());
    }
}
