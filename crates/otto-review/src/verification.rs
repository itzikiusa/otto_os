//! What the final verifier established, claim by claim — and how a review that
//! did not finish is told apart from one that did.
//!
//! A review has three honest end states:
//! - **verified** (`done`): the verifier accounted for the reviewers' claims.
//!   Claims it left out are carried forward as UNVERIFIED comments (see
//!   [`UNVERIFIED_MARK`]) and still count, so omitting a claim can never make
//!   it disappear.
//! - **incomplete** (`error`, message starts with [`INCOMPLETE_PREFIX`]): the
//!   reviewers finished but the verifier did not run, accounted for no claim,
//!   or a reviewer is missing. Every reviewer finding is stored unverified —
//!   last night's runs lost 10–16 findings per lens to a summarizer that never
//!   started, and the PR author got a false "score 0".
//! - **error**: nothing usable.
//!
//! `ReviewStatus` has no `incomplete` variant (adding one is an otto-core +
//! otto-state + UI change), so the review row stays `error` and the prefix,
//! the marked comments and the coverage note carry the distinction.

use crate::engine::{raw_claim_id, DraftComment, NOTE_DISPOSITIONS};
use otto_core::domain::{Review, ReviewAgentState, ReviewFinding, ReviewStatus};
use std::collections::HashSet;
use std::time::Duration;

/// Body prefix of a stored draft comment the final verifier did NOT check. It
/// is a reviewer's claim, not a verdict: the UI badges it and a workflow never
/// posts it as verified (an incomplete review fails its step first).
pub const UNVERIFIED_MARK: &str = "[unverified] ";

/// The disposition carried-forward claims get — the verifier prompt's own word
/// ([`NOTE_DISPOSITIONS`]), so there is one list.
pub const UNVERIFIED: &str = NOTE_DISPOSITIONS[1];

/// Start of a review's error when it is not a verdict but its reviewers'
/// findings are stored (unverified) and a retry can finish it.
pub const INCOMPLETE_PREFIX: &str = "Review incomplete — not a verdict: ";

/// Start of the verification note that records claim coverage.
pub const COVERAGE_NOTE_PREFIX: &str = "Verification coverage: ";

/// The `resolve_blocked` reason a summarizer-only retry passes: it cannot
/// prove findings absent, but it is a full verification of stored claims.
pub const SUMMARIZER_RETRY_BLOCKER: &str = "summarizer retry";

/// Attempts the managed summarizer gets when it fails to START (reviewers get
/// three; the verifier used to get one).
pub const SUMMARIZER_ATTEMPTS: u32 = 2;

/// Pause before the summarizer's second attempt.
pub const SUMMARIZER_RETRY_BACKOFF: Duration = Duration::from_secs(5);

/// The plain-words cause of a review error written by [`incomplete_reason`]
/// (the stored text carries the error kind first: "invalid: Review …").
pub fn incomplete_cause(error: &str) -> Option<&str> {
    error.split_once(INCOMPLETE_PREFIX).map(|(_, why)| why)
}

pub fn is_unverified(body: &str) -> bool {
    body.starts_with(UNVERIFIED_MARK)
}

/// Whether a failed summarizer attempt never really started (the TUI never
/// drew, the paste failed, the session died before taking the prompt) or the
/// provider refused it — failures a fresh attempt cures. A cancellation, the
/// absolute deadline, or an answer with no comments is not retried.
pub fn summarizer_retryable(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    if e.contains("cancel") || e.contains("deadline") {
        return false;
    }
    [
        "never drew",
        "could not paste",
        "before the paste",
        "before accepting the prompt",
        "pty spawn",
        "failed to spawn",
        "overloaded",
        "rate limit",
        "529",
        "too many open files",
        "api error",
    ]
    .iter()
    .any(|m| e.contains(m))
}

/// The verifier's output, settled into what is stored.
pub struct Settled {
    /// Verified comments first, then every claim the verifier did not account
    /// for, deduped + ranked and marked [`UNVERIFIED_MARK`].
    pub comments: Vec<DraftComment>,
    /// The scope plan's verification notes (suppressed claims + coverage).
    pub notes: Vec<String>,
    /// Raw reviewer claims the verifier accounted for, of `total`.
    pub accounted: usize,
    pub total: usize,
    /// How many stored comments are unverified.
    pub unverified: usize,
    /// No verdict: the verifier did not run, or it accounted for no claim
    /// while there were claims to account for.
    pub incomplete: bool,
}

/// Settle the summarizer's comments against the raw reviewer batches.
/// `fallback_cause` is `Some` when the managed verifier failed and `parsed`
/// is the deterministic floor (deduped + ranked, never verified).
///
/// Coverage is exactly-once per known id: an unknown id cannot launder an
/// omission, a repeated id counts once, and only an evidence-backed
/// disposition counts as accounting for a claim.
pub fn settle(
    parsed: Vec<DraftComment>,
    batches: &[Vec<ReviewFinding>],
    fallback_cause: Option<&str>,
) -> Settled {
    let total: usize = batches.iter().map(Vec::len).sum();
    let (unreadable, parsed): (Vec<DraftComment>, Vec<DraftComment>) =
        parsed.into_iter().partition(DraftComment::unreadable);
    let skipped_note = unreadable_note(&unreadable, parsed.len() + unreadable.len());
    if let Some(cause) = fallback_cause {
        let why = format!("the final verifier did not run ({cause})");
        let comments: Vec<_> = parsed.into_iter().map(|c| mark(c, &why)).collect();
        return Settled {
            unverified: comments.len(),
            notes: vec![format!(
                "{COVERAGE_NOTE_PREFIX}0 of {total} reviewer claims verified — {why}; every reviewer finding is stored unverified."
            )],
            comments,
            accounted: 0,
            total,
            incomplete: true,
        };
    }
    let expected: HashSet<String> = batches
        .iter()
        .enumerate()
        .flat_map(|(b, f)| (0..f.len()).map(move |j| raw_claim_id(b, j)))
        .collect();
    let covered: HashSet<&String> = parsed
        .iter()
        .filter(|c| c.verification_complete())
        .flat_map(|c| c.source_ids.iter())
        .filter(|id| expected.contains(*id))
        .collect();
    let accounted = covered.len();
    let mut notes: Vec<String> = parsed
        .iter()
        .filter(|c| c.suppressed())
        .map(|c| {
            format!(
                "{}: {} — {}",
                c.disposition.as_deref().unwrap_or(UNVERIFIED),
                c.body,
                c.evidence.as_deref().unwrap_or_default()
            )
        })
        .collect();
    let uncovered: Vec<Vec<ReviewFinding>> = batches
        .iter()
        .enumerate()
        .map(|(b, f)| {
            f.iter()
                .enumerate()
                .filter(|(j, _)| !covered.contains(&raw_claim_id(b, *j)))
                .map(|(_, f)| f.clone())
                .collect()
        })
        .collect();
    let why = "the final verifier did not account for this claim";
    let carried: Vec<DraftComment> = crate::engine::draft_comments_from_json(
        &crate::fallback::deterministic_summary(&uncovered),
    )
    .unwrap_or_default()
    .into_iter()
    .map(|c| mark(c, why))
    .collect();
    let mut comments: Vec<DraftComment> = parsed.into_iter().filter(|c| !c.suppressed()).collect();
    if total > 0 {
        notes.push(format!(
            "{COVERAGE_NOTE_PREFIX}{accounted} of {total} reviewer claims verified{}",
            match carried.len() {
                0 => String::new(),
                n => format!("; {n} carried forward unverified"),
            }
        ));
    }
    notes.extend(skipped_note);
    let unverified = carried.len();
    comments.extend(carried);
    Settled {
        comments,
        notes,
        accounted,
        total,
        unverified,
        incomplete: total > 0 && accounted == 0,
    }
}

/// Start of the note that counts the verifier items no reader could use.
pub const UNREADABLE_NOTE_PREFIX: &str = "Verifier output: ";

/// "Verifier output: 2 of 14 items could not be read and were skipped (item
/// 3: not a JSON object; item 9: no text `body`); …" — `None` when every item
/// was read.
fn unreadable_note(unreadable: &[DraftComment], items: usize) -> Option<String> {
    if unreadable.is_empty() {
        return None;
    }
    let which: Vec<&str> = unreadable.iter().map(|c| c.body.as_str()).collect();
    let which = otto_core::text::clip_chars(&which.join("; "), 300);
    Some(format!(
        "{UNREADABLE_NOTE_PREFIX}{} of {items} items could not be read and were skipped ({which}); the claims they covered are carried forward unverified.",
        unreadable.len()
    ))
}

/// Store a settled summary's notes on the scope plan: the scope step's own
/// notes (how the lenses were chosen, what its answer lacked) stay, every
/// earlier summary's notes and restart-resume marks are replaced.
pub fn set_notes(plan: &mut otto_core::domain::ReviewScopePlan, settled: &[String]) {
    plan.verification_notes
        .retain(|n| n.starts_with(crate::scope::SCOPE_NOTE_PREFIX));
    plan.verification_notes.extend_from_slice(settled);
}

fn mark(mut c: DraftComment, why: &str) -> DraftComment {
    if !is_unverified(&c.body) {
        c.body = format!("{UNVERIFIED_MARK}{}", c.body);
    }
    c.disposition = Some(UNVERIFIED.into());
    c.reasoning = Some(match c.reasoning.take().filter(|r| !r.trim().is_empty()) {
        Some(r) => format!("unverified: {why} ({r})"),
        None => format!("unverified: {why}"),
    });
    c
}

/// Reviewer rows (pass the rows BEFORE the summarizer's) that did not
/// finish: not `done`/`skipped`, or an orchestrator adopted `partial` — the
/// same rule as [`crate::engine::review_run_complete`].
pub fn missing_lenses(reviewers: &[ReviewAgentState]) -> Vec<String> {
    reviewers
        .iter()
        .filter(|a| {
            !matches!(a.status.as_str(), "done" | "skipped") || a.note.starts_with("partial")
        })
        .map(|a| a.name.clone())
        .collect()
}

/// Start of the cause when no reviewer finished: no score of any kind.
pub const NO_REVIEWER_FINISHED: &str = "no reviewer finished";

/// Why a settled summary is not a verdict — `None` when it is one. The text
/// becomes the review's error and the workflow's reason, so it says which
/// step failed and why in plain words.
pub fn incomplete_reason(
    s: &Settled,
    fallback_cause: Option<&str>,
    resolve_blocked: Option<&str>,
    reviewers: &[ReviewAgentState],
) -> Option<String> {
    let stored = match s.unverified {
        1 => "1 reviewer finding is stored unverified".to_string(),
        n => format!("{n} reviewer findings are stored unverified"),
    };
    let missing = missing_lenses(reviewers);
    let why = if !reviewers.is_empty() && missing.len() == reviewers.len() {
        format!(
            "{NO_REVIEWER_FINISHED}: {}; nothing was reviewed, so there is no score. Retry the failed reviewer(s) or re-run the review.",
            missing.join(", ")
        )
    } else if let Some(cause) = fallback_cause.filter(|_| s.incomplete) {
        format!("the final verifier agent failed ({cause}); {stored}. Retry the summarizer to verify them.")
    } else if s.incomplete {
        format!(
            "the final verifier accounted for none of the {} reviewer claims; {stored}. Retry the summarizer.",
            s.total
        )
    } else if let Some(r) = resolve_blocked.filter(|r| *r != SUMMARIZER_RETRY_BLOCKER) {
        format!("the reviewed change is not fully covered ({r}); findings were verified but the result is partial. Re-run the review.")
    } else {
        if missing.is_empty() {
            return None;
        }
        format!(
            "reviewer(s) did not finish: {}; the findings of the other lenses were verified, so the score is partial. Retry the failed reviewer(s).",
            missing.join(", ")
        )
    };
    Some(format!("{INCOMPLETE_PREFIX}{why}"))
}

/// What a workflow step reports about one settled review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewReport {
    /// `"verified"` | `"partial"` (some claims carried unverified) |
    /// `"incomplete"` (no verdict, reviewer findings stored) | `"none"`
    /// (nothing usable).
    pub verification: &'static str,
    /// Stored comments the verifier did not check.
    pub unverified: usize,
    /// The coverage note ("31 of 34 reviewer claims verified; …").
    pub coverage: Option<String>,
    pub missing_lenses: Vec<String>,
    /// Plain-words cause when the review did not complete; empty otherwise.
    pub cause: String,
    /// Every reviewer finished and only the verifier failed: the stored
    /// findings are the reviewers' full result, just unverified.
    pub verifier_failed: bool,
    /// Not a verdict, but the verifier DID run: what it verified stands and
    /// a score over it is meaningful ("partial"), e.g. one lens is missing.
    pub partial: bool,
}

/// Read a review row for the workflow step. `status` is the step's settled
/// view (`done` / `error` / `timeout` / `cancelled` / `running`).
pub fn report(review: Option<&Review>, status: &str) -> ReviewReport {
    let Some(r) = review else {
        return ReviewReport {
            verification: "none",
            unverified: 0,
            coverage: None,
            missing_lenses: Vec::new(),
            cause: format!("the review could not be read (status: {status})"),
            verifier_failed: false,
            partial: false,
        };
    };
    let unverified = r.comments.iter().filter(|c| is_unverified(&c.body)).count();
    let coverage = r.scope_plan.as_ref().and_then(|p| {
        p.verification_notes
            .iter()
            .find_map(|n| n.strip_prefix(COVERAGE_NOTE_PREFIX))
            .map(str::to_string)
    });
    // The summarizer is always the last row.
    let reviewers = &r.agents[..r.agents.len().saturating_sub(1)];
    let missing_lenses = missing_lenses(reviewers);
    // Not one lens finished: a "partial" score would rate an empty review.
    let nothing_reviewed = !reviewers.is_empty() && missing_lenses.len() == reviewers.len();
    let error = r.error.as_deref().unwrap_or_default();
    let incomplete = incomplete_cause(error);
    let cause = match (status, incomplete) {
        ("done", _) if missing_lenses.is_empty() => String::new(),
        ("done", _) => format!("reviewer(s) did not finish: {}", missing_lenses.join(", ")),
        (_, Some(why)) => why.to_string(),
        ("timeout", _) => "the review did not finish before the step's timeout".into(),
        ("running", _) => "the review is still running (the step does not wait for it)".into(),
        ("cancelled", _) => "the review was cancelled".into(),
        _ if !error.is_empty() => error.to_string(),
        _ => format!("the review ended with status {status} and no reason"),
    };
    let by_verifier = incomplete.is_some_and(|w| w.starts_with("the final verifier"));
    let verifier_failed =
        r.status == ReviewStatus::Error && by_verifier && missing_lenses.is_empty();
    let verification = match status {
        "done" if unverified == 0 => "verified",
        "done" => "partial",
        _ if incomplete.is_some() => "incomplete",
        _ => "none",
    };
    ReviewReport {
        verification,
        unverified,
        coverage,
        missing_lenses,
        cause,
        verifier_failed,
        partial: incomplete.is_some() && !by_verifier && !nothing_reviewed,
    }
}

/// The step's chat message for a review that is not a verdict: what failed
/// and why, the stored-but-unverified findings, the lens scores, and how to
/// retry. `partial_score` is the score over what WAS verified, when any.
pub fn incomplete_chat(
    head: &str,
    r: &ReviewReport,
    partial_score: Option<i64>,
    lens_scores: &[serde_json::Value],
) -> String {
    let mut msg = format!(
        "{head} — ⚠️ Review incomplete — not a verdict\nWhy: {}",
        r.cause
    );
    if let Some(c) = &r.coverage {
        msg.push_str(&format!("\nVerification: {c}"));
    }
    if r.unverified > 0 {
        msg.push_str(&format!(
            "\n{} reviewer finding(s) stored unverified — not posted to the PR.",
            r.unverified
        ));
    }
    if !r.missing_lenses.is_empty() {
        msg.push_str(&format!(
            "\nLenses that did not finish: {}",
            r.missing_lenses.join(", ")
        ));
    }
    if let Some(s) = partial_score {
        msg.push_str(&format!("\nPartial score: {s}/100 over what was verified"));
    }
    msg.push_str(&crate::prompts::lens_scores_chat(lens_scores));
    msg.push_str(
        "\nTo finish it: retry this step (it re-runs only the summarizer when every reviewer finished), or open the review and retry the summarizer or the failed reviewer. Re-running the workflow starts a full new review.",
    );
    msg
}

/// The verdict fields of one target's `review_run` step output.
pub struct StepScore {
    /// `passed` | `failed` | `incomplete` (not a verdict) | `pending` (the
    /// step does not wait for the review).
    pub outcome: &'static str,
    /// `null` unless the review is a verdict: a 0 read as a FAILED review in
    /// notify templates, a 100 as a clean one.
    pub score: serde_json::Value,
    pub review_score: serde_json::Value,
    /// "72/100", "partial — 72/100", "incomplete" or "pending".
    pub score_text: String,
    /// The score over what WAS verified when the verifier ran but the
    /// review is partial (e.g. one lens missing).
    pub partial_score: Option<i64>,
    pub reason: String,
}

impl ReviewReport {
    /// Fold the gate's verdict into the step's score fields. `scores` is
    /// `(blended score, review score)`.
    pub fn step(
        &self,
        passed: bool,
        complete: bool,
        no_changes: bool,
        status: &str,
        (score, review_score): (i64, i64),
        reason: String,
    ) -> StepScore {
        use serde_json::Value;
        let outcome = match (passed, complete || no_changes, status) {
            (true, ..) => "passed",
            (_, true, _) => "failed",
            (_, _, "running") => "pending",
            _ => "incomplete",
        };
        let partial_score = (!complete && self.partial).then_some(score);
        let (score_v, review_v, score_text) = match (outcome, partial_score) {
            ("passed" | "failed", _) => (score.into(), review_score.into(), format!("{score}/100")),
            (_, Some(s)) => (Value::Null, Value::Null, format!("partial — {s}/100")),
            (o, None) => (Value::Null, Value::Null, o.to_string()),
        };
        StepScore {
            outcome,
            score: score_v,
            review_score: review_v,
            score_text,
            partial_score,
            reason: if complete || no_changes {
                reason
            } else {
                format!("review incomplete — not a verdict: {}", self.cause)
            },
        }
    }

    /// The status the step's gate judges: a review whose ONLY failure is the
    /// verifier counts as `done` when the node opts in
    /// (`allow_summary_fallback: true`) — its unverified findings are scored.
    pub fn gate_status<'a>(&self, status: &'a str, allow_summary_fallback: bool) -> &'a str {
        if allow_summary_fallback && self.verifier_failed {
            "done"
        } else {
            status
        }
    }

    /// The step output's `verification` object.
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.verification, "unverified": self.unverified,
            "coverage": self.coverage, "missing_lenses": self.missing_lenses,
        })
    }
}

/// The step's chat message, read from the target's step output (`out`) so
/// the message and what downstream steps see cannot disagree. A review that
/// did not complete gets [`incomplete_chat`]; "Findings: none 🎉" is only
/// ever said of a review that finished with none.
pub fn step_chat(
    head: &str,
    out: &serde_json::Map<String, serde_json::Value>,
    r: &ReviewReport,
) -> String {
    use serde_json::Value;
    let s = |k: &str| out.get(k).and_then(Value::as_str).unwrap_or_default();
    let n = |k: &str| out.get(k).and_then(Value::as_i64).unwrap_or_default();
    let list = |k: &str| {
        out.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let no_changes = out.get("no_changes").and_then(Value::as_bool) == Some(true);
    let lens_scores = list("lens_scores");
    if !no_changes && !matches!(s("outcome"), "passed" | "failed") {
        let partial = out.get("partial_score").and_then(Value::as_i64);
        return incomplete_chat(head, r, partial, &lens_scores);
    }
    let mut msg = if no_changes {
        format!(
            "{head} — ⚠️ empty diff vs `{}` — nothing was reviewed",
            s("base")
        )
    } else {
        let verdict = match s("outcome") {
            "passed" => "✅ passed".to_string(),
            _ => format!("⚠️ {}", s("reason")),
        };
        format!(
            "{head} done — score *{}/100* (pass ≥ {}) — {verdict}",
            n("score"),
            n("threshold")
        )
    };
    msg.push_str(&crate::prompts::lens_scores_chat(&lens_scores));
    let findings = list("findings");
    if no_changes {
        msg.push_str("\nNo reviewers ran. Check the base branch this run declared.");
    } else if findings.is_empty() {
        msg.push_str("\nFindings: none 🎉");
    } else {
        msg.push_str(&format!("\nFindings ({} open):", n("open")));
        for b in findings.iter().filter_map(Value::as_str) {
            msg.push_str(&format!("\n • {b}"));
        }
    }
    msg
}

#[cfg(test)]
#[path = "verification_tests.rs"]
mod tests;
