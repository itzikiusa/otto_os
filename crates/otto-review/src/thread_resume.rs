//! A review across a daemon restart (runtime.md §7.8, DR-006, DR-115.2):
//! the reviewer rows a restart left unfinished are settled without re-running
//! the ones whose threads survived in their holders.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_agent_run::agent_run::{FailReason, RunOutcome};
use otto_agent_run::step::StepTurn;
use otto_core::cancel::{CancelRegistry, CancelRegistryExt};
use otto_core::domain::{ReviewAgentState, ReviewFinding, User, Workspace};
use otto_sessions::SessionManager;
use otto_state::ReviewsRepo;

use crate::session::{
    findings_path, review_session_meta, run_agent_session_with_recovery, settle_run, SharedStates,
};
use crate::thread_run::{drive, fail_reason, result_text, Threads};

/// Everything a restart resume needs to re-enter one review's rows.
#[derive(Clone)]
pub struct ResumeCtx {
    pub manager: Arc<SessionManager>,
    pub threads: Threads,
    pub reviews: ReviewsRepo,
    pub ws: Workspace,
    pub user: User,
    /// The checkout the review ran in (kept: a held review is not swept).
    pub cwd: String,
    pub review_id: String,
    pub grace: Duration,
    pub skills_dir: Option<String>,
    pub cancel: Option<Arc<AtomicBool>>,
    /// Per-agent cancel registry (the fresh path's `review_agent_cancels`).
    pub agent_cancels: CancelRegistry,
}

/// Settle every reviewer row a restart left unfinished: a row whose thread
/// survived is awaited through the runner (`StepRunner::resume`) and its
/// findings file read as usual; any other row is re-run from its persisted
/// prompt with the review's retry rules. A summarizer thread still open is
/// closed (the caller summarizes afresh). Returns each reviewer row's
/// findings for the summarizer (empty for a row that did not finish).
pub async fn resume_reviewers(c: ResumeCtx) -> otto_core::Result<Vec<Vec<ReviewFinding>>> {
    let id: otto_core::Id = c.review_id.clone();
    let rows = c.reviews.get_review(&id).await?.agents;
    let n = rows.len().saturating_sub(1);
    let any_reviewer_unfinished = rows
        .iter()
        .take(n)
        .any(|r| matches!(r.status.as_str(), "pending" | "running" | "waiting"));
    // Close a stale summarizer thread only when reviewers still need to run;
    // when every reviewer already finished, keep the held summarizer open so
    // the caller can await it in its holder.
    if any_reviewer_unfinished {
        if let Some(sid) = rows
            .last()
            .filter(|s| n > 0 && s.thread)
            .and_then(|s| s.session_id.clone())
        {
            c.threads.close(&sid).await;
        }
    }
    let states: SharedStates = Arc::new(tokio::sync::Mutex::new(rows.clone()));
    let mut set = tokio::task::JoinSet::new();
    for (i, row) in rows.into_iter().enumerate().take(n) {
        if matches!(row.status.as_str(), "pending" | "running" | "waiting") {
            let guard = c
                .agent_cancels
                .register(&crate::engine::review_agent_cancel_key(&c.review_id, i));
            if c.cancel.as_ref().is_some_and(|f| f.load(Ordering::SeqCst)) {
                guard.flag().store(true, Ordering::SeqCst);
            }
            let (c, states) = (c.clone(), Arc::clone(&states));
            set.spawn(async move {
                resume_row(&c, &states, i, &row, guard.flag()).await;
                drop(guard);
            });
        }
    }
    while set.join_next().await.is_some() {}
    let rows = c.reviews.get_review(&id).await?.agents;
    Ok(rows
        .iter()
        .take(n)
        .map(|r| {
            if r.status == "done" {
                r.findings.clone()
            } else {
                Vec::new()
            }
        })
        .collect())
}

async fn resume_row(
    c: &ResumeCtx,
    states: &SharedStates,
    i: usize,
    row: &ReviewAgentState,
    cancel: &Arc<AtomicBool>,
) {
    let stopped = || {
        cancel.load(Ordering::SeqCst) || c.cancel.as_ref().is_some_and(|f| f.load(Ordering::SeqCst))
    };
    let settle = |outcome| {
        settle_run(
            states,
            &c.reviews,
            &c.review_id,
            i,
            &row.lens_slugs,
            outcome,
            false,
            None,
        )
    };
    if let (true, Some(thread)) = (row.thread, row.session_id.clone()) {
        let mut meta = review_session_meta(&c.review_id, i, !row.lens_slugs.is_empty());
        c.threads.meta_into(&mut meta);
        let findings = findings_path(&c.review_id, i);
        // Supply the persisted reviewer prompt so `StepRunner::resume` can send
        // it if the restart landed after `create` and before turn 1 went out.
        let prompt = c
            .reviews
            .get_agent_prompt(&c.review_id, i)
            .await
            .ok()
            .flatten()
            .map(|p| crate::session::typed_reviewer_prompt(&p, &findings.to_string_lossy(), None))
            .unwrap_or_default();
        let turn = StepTurn {
            user_id: c.user.id.clone(),
            workspace_id: c.ws.id.clone(),
            provider: row.provider.clone(),
            model: Some(row.model.clone()).filter(|m| !m.trim().is_empty()),
            effort: Some(row.effort.clone()).filter(|e| !e.trim().is_empty()),
            cwd: c.cwd.clone().into(),
            title: row.name.clone(),
            meta,
            prompt,
            continue_thread: None,
            handoff: Some(findings),
            read_only: true,
            timeout: None,
            no_progress: Some(c.grace),
        };
        let resumed = drive(
            &c.threads,
            turn,
            &c.reviews,
            states,
            &c.review_id,
            i,
            Some(cancel),
            Some(thread),
        )
        .await;
        if stopped() {
            settle(RunOutcome::failed(
                resumed.map(|o| o.thread_id),
                FailReason::Stopped,
            ))
            .await;
            return;
        }
        if let Some(o) = resumed {
            if let Some(raw) = result_text(&findings_path(&c.review_id, i), &o) {
                settle(RunOutcome::ok(raw, o.thread_id)).await;
                return;
            }
            if fail_reason(&o) == FailReason::Stopped {
                settle(RunOutcome::failed(Some(o.thread_id), FailReason::Stopped)).await;
                return;
            }
        }
    }
    if stopped() {
        settle(RunOutcome::failed(None, FailReason::Stopped)).await;
        return;
    }
    // The thread is gone (or never was one): re-run the row.
    let prompt = c.reviews.get_agent_prompt(&c.review_id, i).await;
    let Ok(Some(prompt)) = prompt else {
        settle(RunOutcome::failed(None, FailReason::CreateFailed)).await;
        return;
    };
    run_agent_session_with_recovery(
        &c.manager,
        &c.reviews,
        states,
        &c.ws,
        &c.user,
        &row.provider,
        &row.model,
        &c.cwd,
        &c.review_id,
        i,
        &prompt,
        c.grace,
        None,
        Some(cancel),
        c.skills_dir.as_deref(),
        &row.lens_slugs,
        Some(&c.threads),
    )
    .await;
}
