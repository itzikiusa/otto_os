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

/// What [`settle_after_restart`] left for the daemon to drive.
#[derive(Debug, Default, PartialEq)]
pub struct Settled {
    /// Settled as `error` with [`INTERRUPTED_SUMMARIZING`]: re-run only the
    /// summarizer (the caller's rerun claim takes it).
    pub summarizer_resume: Vec<Id>,
    /// Still `running`: some reviewers survived in their holders. A driver
    /// awaits them, re-runs the ones that did not, then summarizes; their
    /// worktrees are not swept.
    pub resume_held: Vec<Id>,
}

/// True when the review's summarizer row is a thread whose id survived in a
/// holder (`held_threads`: `None` trusts `held` for the review itself).
fn summarizer_is_held_thread(
    review: &Review,
    held_threads: Option<&std::collections::HashSet<Id>>,
) -> bool {
    let Some(sum) = review.agents.last().filter(|_| review.agents.len() >= 2) else {
        return false;
    };
    if !sum.thread {
        return false;
    }
    let Some(sid) = sum.session_id.as_ref() else {
        return false;
    };
    held_threads.is_none_or(|threads| threads.contains(sid))
}

/// True when unfinished reviewers can all be resumed or started from persisted
/// prompts (e.g. finished reviewers closed their threads and the remaining ones
/// were still queued behind the admission ceiling with no live holder sockets).
async fn has_persisted_prompts_for_unfinished(
    reviews: &otto_state::ReviewsRepo,
    review: &Review,
) -> bool {
    let n = review.agents.len().saturating_sub(1);
    if n == 0 {
        return false;
    }
    let mut any_unfinished = false;
    for (i, a) in review.agents.iter().take(n).enumerate() {
        if matches!(a.status.as_str(), "pending" | "running" | "waiting") {
            any_unfinished = true;
            if !(a.thread && a.session_id.is_some()) {
                match reviews.get_agent_prompt(&review.id, i).await {
                    Ok(Some(_)) => {}
                    _ => return false,
                }
            }
        }
    }
    any_unfinished
}

/// Settle what a dead daemon left `running`: a review with an agent in
/// `held` (its holder survived the restart — reviewer or summarizer) or with
/// persisted reviewer prompts for every unfinished reviewer stays running for a
/// driver ([`Settled::resume_held`], DR-006); any other either gets its
/// summarizer resumed or fails with the reason. Both resumes count against
/// [`MAX_RESTART_RESUMES`]. Live agent rows of settled reviews are failed too.
/// One review's failed write is logged and skipped; the closing sweep still
/// fails it, so nothing else is left `running`.
pub async fn settle_after_restart(
    reviews: &otto_state::ReviewsRepo,
    held: &std::collections::HashSet<Id>,
) -> Result<Settled> {
    settle_after_restart_with_threads(reviews, held, None).await
}

/// [`settle_after_restart`] with the exact set of adopted runtime thread ids
/// (`adopted.handled_ids`), so a held summarizer is recognized only when its
/// own thread survived.
pub async fn settle_after_restart_with_threads(
    reviews: &otto_state::ReviewsRepo,
    held: &std::collections::HashSet<Id>,
    held_threads: Option<&std::collections::HashSet<Id>>,
) -> Result<Settled> {
    let mut resume = Vec::new();
    let mut resume_held = Vec::new();
    let mut repo_paths = Vec::new();
    for review in reviews.list_running().await? {
        let id = review.id.clone();
        if let Ok(Some(ctx)) = reviews.get_run_context(&id).await {
            if let Some(cwd) = ctx.cwd.as_deref().filter(|s| !s.is_empty()) {
                let p = std::path::PathBuf::from(cwd);
                if let Some(parent_repo) = crate::worktree::repo_path_from_worktree(&p).await {
                    repo_paths.push(parent_repo);
                } else if tokio::fs::metadata(&p).await.is_ok_and(|m| m.is_dir()) {
                    repo_paths.push(p);
                }
            }
        }
        let sum_only = summarizer_only_left(&review);
        let held_sum =
            held.contains(&id) && sum_only && summarizer_is_held_thread(&review, held_threads);
        let resumable_reviewers = !sum_only
            && (held.contains(&id) || has_persisted_prompts_for_unfinished(reviews, &review).await);
        let awaitable = held_sum || resumable_reviewers;
        if let Some(mut plan) = review
            .scope_plan
            .clone()
            .filter(|p| awaitable && restart_resumes(p) < MAX_RESTART_RESUMES)
        {
            let n = restart_resumes(&plan) + 1;
            let what = if held_sum {
                "its summarizer was awaited in its holder."
            } else {
                "its reviewers were awaited in their holders."
            };
            plan.verification_notes.push(format!(
                "{RESUME_NOTE} ({n} of {MAX_RESTART_RESUMES}); {what}"
            ));
            match reviews.set_scope_plan(&id, &plan).await {
                Ok(()) => {
                    resume_held.push(id);
                    continue;
                }
                Err(e) => tracing::warn!(review = %id, "restart resume note: {e}"),
            }
        }
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
    for r in reviews.list_running().await? {
        if !resume_held.contains(&r.id) {
            reviews
                .set_status(&r.id, ReviewStatus::Error, Some(INTERRUPTED))
                .await?;
        }
    }
    reviews
        .fail_orphaned_agents("Interrupted by a daemon restart")
        .await?;

    // Sweep orphaned `$TMPDIR/otto-review-wt-*` directories and stale
    // `otto-review-*` branches / `refs/otto/pr-review/*` refs left behind by
    // the previous daemon life (never touching any still-active review).
    let active: std::collections::HashSet<String> = reviews
        .list_running()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.id)
        .collect();
    let swept = crate::worktree::sweep_orphaned_worktrees(&active, &repo_paths).await;
    if swept.worktrees_removed > 0 || swept.branches_deleted > 0 || swept.refs_deleted > 0 {
        tracing::info!(
            worktrees = swept.worktrees_removed,
            branches = swept.branches_deleted,
            refs = swept.refs_deleted,
            "review recovery: swept orphaned PR worktrees and refs"
        );
    }
    Ok(Settled {
        summarizer_resume: resume,
        resume_held,
    })
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

/// The review's error note when its diff file can't be written for the
/// reviewers (a new run settles with it; a retry refuses with it).
pub fn diff_write_failure(cause: &dyn std::fmt::Display) -> String {
    format!("Couldn't write the diff for the reviewers: {cause}")
}

/// Write the diff to `$TMPDIR/otto-review-<id>.diff` and persist its durable copy.
pub async fn materialize_review_diff(
    reviews: &otto_state::ReviewsRepo,
    review_id: &Id,
    diff_text: &str,
) -> Result<std::path::PathBuf> {
    let diff_path = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
    if let Err(e) = tokio::fs::write(&diff_path, diff_text).await {
        tracing::warn!(review = %review_id, "could not write review diff file: {e}");
        return Err(Error::Internal(diff_write_failure(&e)));
    }
    if let Err(e) = reviews.set_diff(review_id, diff_text).await {
        tracing::warn!(review = %review_id, "could not persist review diff: {e}");
    }
    Ok(diff_path)
}

/// Remove matching `$TMPDIR/otto-review-<id>*` files (FILES only, never worktrees).
#[allow(clippy::disallowed_methods)]
pub fn remove_review_temp_files_in(dir: &std::path::Path, prefix: &str) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if entry.file_name().to_string_lossy().starts_with(prefix)
                && entry.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Ensure `$TMPDIR/otto-review-<review_id>.diff` exists on disk (restoring it
/// from the durable `reviews` diff column if a reboot or temp sweep removed it)
/// and return its text. Fails loudly if no durable diff exists so queued
/// reviewers never start against a missing file (DATA-2).
pub async fn ensure_review_diff(
    reviews: &otto_state::ReviewsRepo,
    review_id: &Id,
) -> Result<String> {
    let diff_file = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
    if let Ok(existing) = tokio::fs::read_to_string(&diff_file).await {
        if !existing.is_empty() {
            return Ok(existing);
        }
    }
    match reviews.get_diff(review_id).await {
        Ok(Some(diff)) if !diff.is_empty() => {
            tokio::fs::write(&diff_file, &diff).await.map_err(|e| {
                Error::Internal(format!("Couldn't write the diff for the reviewers: {e}"))
            })?;
            Ok(diff)
        }
        Ok(_) => Err(Error::Conflict(
            "this review has no stored diff to retry against; start a new review".into(),
        )),
        Err(e) => Err(Error::Internal(format!(
            "Couldn't write the diff for the reviewers: {e}"
        ))),
    }
}

/// Blocker passed to `summarize_and_persist` when resuming a review after a
/// restart: preserves the fresh path's partial-diff incompleteness check so a
/// resumed review whose persisted diff omitted files can never pass clean
/// (BUG-11), otherwise uses `SUMMARIZER_RETRY_BLOCKER`.
pub fn resumed_resolve_blocker(diff_text: Option<&str>) -> Option<&'static str> {
    if diff_text.is_some_and(crate::engine::diff_is_partial) {
        return Some("partial diff (files omitted)");
    }
    Some(crate::verification::SUMMARIZER_RETRY_BLOCKER)
}

#[cfg(test)]
#[path = "restart_tests.rs"]
mod tests;
