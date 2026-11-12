//! What becomes of an unfinished review when the daemon restarts, or when a
//! workflow step re-enters a review it already launched ("Retry step", or a
//! run resumed after a restart).
//!
//! A review's reviewers are the expensive part (30–60 minutes); their
//! findings are durable on the agent rows (`agents_json`). When every
//! reviewer finished and only the summarizer (the final verifier) was left —
//! or failed — the honest continuation is to run ONLY the summarizer over the
//! stored findings, the same path as `POST /reviews/{id}/summarizer/retry`.
//! Anything less finished is re-run from scratch, as before.

use crate::verification::{incomplete_cause, missing_lenses};
use otto_core::domain::{Review, ReviewAgentState, ReviewScopePlan, ReviewStatus, User};
use otto_core::{Error, Id, Result};

/// The error of a review a restart interrupted before its reviewers finished.
pub const INTERRUPTED: &str = "Interrupted by a daemon restart — re-run to review again.";

/// The error of a review the restart interrupted while summarizing, set just
/// before the summarizer is resumed (it stays if the resume cannot start).
pub const INTERRUPTED_SUMMARIZING: &str =
    "Interrupted by a daemon restart while summarizing — retry the summarizer.";

/// The error once the summarizer was resumed [`MAX_RESTART_RESUMES`] times
/// and a restart interrupted it again: a crash loop is not retried for ever.
pub const RESUMES_EXHAUSTED: &str = "Interrupted by daemon restarts while summarizing, after 2 automatic resumes — retry the summarizer or re-run the review.";

/// How many times a restart resumes the same review's summarizer.
pub const MAX_RESTART_RESUMES: usize = 2;

/// Verification note recording one restart resume. Kept in the scope plan
/// (durable, and shown with the review's verification notes); the count
/// survives the restarts it guards. A summary that settles replaces the
/// notes, which resets it — only an interrupted summarizer keeps counting.
pub const RESUME_NOTE: &str = "Summarizer resumed after a daemon restart";

/// Why the summarizer cannot be re-run on `review`, or `None` when it can:
/// it is settled, has a summarizer row, and its reviewers stored findings.
pub fn rerun_block(review: &Review) -> Option<&'static str> {
    if review.status == ReviewStatus::Running {
        Some("review is still running — wait for it to finish first")
    } else if review.agents.len() < 2 {
        Some("this review has no summarizer stage to retry")
    } else if !has_findings(reviewers(review)) {
        Some("no stored agent findings with content to summarize — re-run the review instead")
    } else {
        None
    }
}

/// The reviewer rows: every row but the summarizer (always the last).
fn reviewers(review: &Review) -> &[ReviewAgentState] {
    &review.agents[..review.agents.len().saturating_sub(1)]
}

fn has_findings(rows: &[ReviewAgentState]) -> bool {
    rows.iter()
        .flat_map(|a| &a.findings)
        .any(|f| !f.body.trim().is_empty())
}

/// Every reviewer finished (done / skipped, none partial) and stored
/// findings — so the summarizer alone can finish the review.
fn summarizer_only_left(review: &Review) -> bool {
    review.agents.len() >= 2
        && missing_lenses(reviewers(review)).is_empty()
        && has_findings(reviewers(review))
}

/// Restart resumes recorded so far (see [`RESUME_NOTE`]).
pub fn restart_resumes(plan: &ReviewScopePlan) -> usize {
    plan.verification_notes
        .iter()
        .filter(|n| n.starts_with(RESUME_NOTE))
        .count()
}

/// What boot recovery does with a review the previous daemon left `running`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartAction {
    /// Record the resume in the scope plan and run only the summarizer.
    ResumeSummarizer,
    /// Settle as `error` with this message.
    Fail(&'static str),
}

pub fn on_restart(review: &Review) -> RestartAction {
    if !summarizer_only_left(review) {
        return RestartAction::Fail(INTERRUPTED);
    }
    match &review.scope_plan {
        Some(plan) if restart_resumes(plan) < MAX_RESTART_RESUMES => {
            RestartAction::ResumeSummarizer
        }
        // No plan to count resumes in (an older review): never loop.
        Some(_) | None => RestartAction::Fail(RESUMES_EXHAUSTED),
    }
}

/// What a workflow step does with the review its checkpoint names when the
/// step's reviewed input (diff, head, config) is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adoption {
    /// Await / score it as it is: still running, or finished complete.
    Adopt,
    /// Every reviewer finished; only the summarizer failed (or never ran):
    /// re-run ONLY the summarizer on it, then await and score it.
    RerunSummarizer,
    /// Launch a new review (a reviewer failed, the review was cancelled, …).
    Launch,
}

pub fn adoption(review: &Review) -> Adoption {
    match review.status {
        ReviewStatus::Running => Adoption::Adopt,
        // A summary that fell back to the deterministic merge counts as
        // incomplete here too: a step that refuses fallbacks would re-adopt
        // it on every retry and never pass.
        ReviewStatus::Done
            if crate::engine::review_run_complete(
                &review.agents,
                review.summary_fallback.unwrap_or(false),
            ) =>
        {
            Adoption::Adopt
        }
        ReviewStatus::Error if summarizer_only_left(review) && summarizer_failed(review) => {
            Adoption::RerunSummarizer
        }
        _ => Adoption::Launch,
    }
}

/// The review's error says the SUMMARIZER is what did not finish: the final
/// verifier failed or accounted for nothing, its runner vanished, or a
/// restart interrupted it. Not after [`RESUMES_EXHAUSTED`] (a crash loop),
/// and not for a partial diff or checkout — a summarizer re-run cannot
/// widen what the reviewers saw.
fn summarizer_failed(review: &Review) -> bool {
    let error = review.error.as_deref().unwrap_or_default();
    incomplete_cause(error).is_some_and(|why| why.starts_with("the final verifier"))
        || error.contains(crate::engine::REVIEW_STALLED_MSG)
        || error.contains(INTERRUPTED_SUMMARIZING)
}

/// Settle what a dead daemon left `running`: each review either gets its
/// summarizer resumed (returned, already settled as `error` with
/// [`INTERRUPTED_SUMMARIZING`] so the caller's rerun claim can take it) or
/// fails with the reason. Live agent rows of settled reviews are failed too.
/// One review's failed write is logged and skipped; the closing sweep still
/// fails it, so nothing is left `running`.
pub async fn settle_after_restart(reviews: &otto_state::ReviewsRepo) -> Result<Vec<Id>> {
    let mut resume = Vec::new();
    for review in reviews.list_running().await? {
        let id = review.id.clone();
        let settled = match (on_restart(&review), review.scope_plan) {
            (RestartAction::ResumeSummarizer, Some(mut plan)) => {
                let n = restart_resumes(&plan) + 1;
                plan.verification_notes
                    .push(format!("{RESUME_NOTE} ({n} of {MAX_RESTART_RESUMES})."));
                // Written while still `running` (the plan write requires it).
                match reviews.set_scope_plan(&id, &plan).await {
                    Ok(()) => resume.push(id.clone()),
                    Err(e) => tracing::warn!(review = %id, "restart resume note: {e}"),
                }
                reviews.set_status(&id, ReviewStatus::Error, Some(INTERRUPTED_SUMMARIZING))
            }
            (RestartAction::Fail(why), _) => {
                reviews.set_status(&id, ReviewStatus::Error, Some(why))
            }
            // `on_restart` resumes only a review that has a plan.
            (RestartAction::ResumeSummarizer, None) => {
                reviews.set_status(&id, ReviewStatus::Error, Some(RESUMES_EXHAUSTED))
            }
        };
        if let Err(e) = settled.await {
            tracing::warn!(review = %id, "restart recovery: {e}");
            resume.retain(|r| *r != id);
        }
    }
    // Anything left (a failed write above, a row that turned `running`
    // meanwhile), then the live agent rows of every settled review.
    reviews.fail_running(INTERRUPTED).await?;
    reviews
        .fail_orphaned_agents("Interrupted by a daemon restart")
        .await?;
    Ok(resume)
}

/// The user review sessions run as when no person started them (a restart
/// resume, a workflow step): the root user, as for every review agent.
pub async fn review_user(users: &otto_state::UsersRepo) -> Result<User> {
    users
        .list()
        .await?
        .into_iter()
        .find(|u| u.is_root)
        .ok_or_else(|| Error::Internal("no root user to run review agents".into()))
}

/// The reviewer rows, each one that did not finish named with WHY (its row
/// note: the failure reason the session recorded — `AgentRunResult::reason`
/// persisted). Fed to `verification::incomplete_reason`, so the incomplete
/// message says "Security (timed out with no result)", not just "Security".
/// Read from the rows, not the run's results, so a summarizer retry or a
/// restart resume (which have no results) says the same.
pub fn with_failure_reasons(rows: &[ReviewAgentState]) -> Vec<ReviewAgentState> {
    rows.iter()
        .map(|a| {
            let mut row = a.clone();
            let why = a.note.trim();
            if !why.is_empty() && !missing_lenses(std::slice::from_ref(a)).is_empty() {
                row.name = format!("{} ({})", a.name, otto_core::text::clip_chars(why, 160));
            }
            row
        })
        .collect()
}

#[cfg(test)]
#[path = "restart_tests.rs"]
mod tests;
