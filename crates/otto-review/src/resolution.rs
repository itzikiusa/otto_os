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

/// The verifier's answer for a prior-finding claim it finds FIXED in the
/// current code — the verifier prompt names exactly this
/// (`prompts::prior_finding_rules`). One of the note dispositions, so the
/// completeness check accepts it and it is never published as a comment.
pub const FIXED_PRIOR_DISPOSITION: &str = crate::engine::NOTE_DISPOSITIONS[0];

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
            p.disposition.trim() == crate::contract::PRIOR_SCOPE_DISPOSITIONS[0]
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
                && covering.all(|c| {
                    c.verification_complete()
                        && c.disposition.as_deref() == Some(FIXED_PRIOR_DISPOSITION)
                })
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

/// After a summarizer re-run of the SAME review (Retry summarizer, a restart
/// resume, a workflow "Retry step"): close the findings an earlier attempt
/// promoted from `[unverified]` drafts that this attempt did not raise again.
/// The re-run already replaced those undecided drafts; their findings would
/// otherwise linger open beside the verified outcome. Only when this attempt's
/// managed verifier ran and accounted for the claims (`verified`) — then every
/// stored claim was kept, merged under a new anchor, or rejected, never
/// silently dropped. Within one review, by id: a finding a later sighting
/// moved to another review, one linked to a comment the user decided on, and
/// the human `status` are all left alone. A fresh run's first attempt is a
/// no-op (everything this review raised, it raised just now). Best-effort.
pub async fn close_superseded_unverified(
    stores: (
        &otto_state::ReviewsRepo,
        &otto_state::ReviewFindingsRepo,
        &otto_state::FindingEventsRepo,
    ),
    review_id: &str,
    seen: &[String],
    verified: bool,
) {
    if !verified {
        return;
    }
    let (reviews, findings, events) = stores;
    let closed = async {
        let review = reviews.get_review(&review_id.to_string()).await?;
        let (_, links) = crate::triage::review_links(findings, events, &review).await?;
        let linked: Vec<&str> = links.values().map(String::as_str).collect();
        let seen: Vec<&str> = seen.iter().map(String::as_str).collect();
        findings
            .close_superseded(
                review_id,
                &seen,
                &linked,
                crate::verification::UNVERIFIED_MARK,
            )
            .await
    };
    match closed.await {
        Ok(0) => {}
        Ok(n) => tracing::info!(review = %review_id, "closed {n} superseded unverified finding(s)"),
        Err(e) => {
            tracing::warn!(review = %review_id, "closing superseded unverified findings: {e}")
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

    /// The prompt's own FIXED example, as the verifier would answer it, goes
    /// through the real parser and the real rule — and closes the finding.
    /// Contract: the prompt and `confirmed_fixed` cannot drift apart.
    #[test]
    fn contract_the_prompts_fixed_prior_example_resolves_the_finding() {
        let rules = crate::prompts::prior_finding_rules();
        let marker = "Fixed in the current code: ";
        let start = rules.find(marker).expect("fixed case") + marker.len();
        let example = &rules[start..][..rules[start..].find('}').expect("object") + 1];
        let answered = example.replace("<its source_id>", &id(0));
        let parsed =
            crate::engine::draft_comments_from_json(&format!("[{answered}]")).expect("parses");
        assert!(parsed[0].verification_complete(), "the check accepts it");
        assert!(parsed[0].suppressed(), "never published as a comment");
        let p = plan(&[("F0", "fixed", "auth.rs:9 now checks the role")]);
        assert_eq!(
            confirmed_fixed(Some(&p), 2, &parsed, None, None),
            vec!["F0"]
        );
        // String level: the prompt names the rule's disposition, the other two
        // cases name `verified` / `unverified`, and every word is one the
        // checker knows.
        assert!(example.contains(&format!("\"disposition\":\"{FIXED_PRIOR_DISPOSITION}\"")));
        assert!(crate::engine::NOTE_DISPOSITIONS.contains(&FIXED_PRIOR_DISPOSITION));
        assert!(rules.contains("Still present: disposition=verified"));
        assert!(rules.contains("Cannot tell: disposition=unverified"));
        // The lens the prompt names is the one prior claims are labelled with.
        assert!(include_str!("history.rs").contains(crate::prompts::PRIOR_FINDING_LENS));
        let cfg = crate::engine::default_review_config("claude").summarizer;
        assert!(crate::prompts::summarizer_prompt("", &cfg, &[]).contains(&rules));
    }
}
