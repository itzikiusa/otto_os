//! A review across a daemon restart (runtime.md §7.8, DR-006, DR-115.2):
//! the reviewer rows a restart left unfinished are settled without re-running
//! the ones whose threads survived in their holders.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use otto_agent_run::agent_run::{FailReason, RunOutcome};
use otto_agent_run::step::StepTurn;
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
}

/// Settle every reviewer row a restart left unfinished: a row whose thread
/// survived is awaited through the runner (`StepRunner::resume`) and its
/// findings file read as usual; any other row is re-run from its persisted
/// prompt with the review's retry rules. A summarizer thread still open is
/// closed (the caller summarizes afresh). Returns each reviewer row's
/// findings for the summarizer (empty for a row that did not finish).
pub async fn resume_reviewers(c: ResumeCtx) -> otto_core::Result<Vec<Vec<ReviewFinding>>> {
    let id: otto_core::Id = c.review_id.clone().into();
    let rows = c.reviews.get_review(&id).await?.agents;
    let n = rows.len().saturating_sub(1);
    if let Some(sid) = rows
        .last()
        .filter(|s| n > 0 && s.thread)
        .and_then(|s| s.session_id.clone())
    {
        c.threads.close(&sid.into()).await;
    }
    let states: SharedStates = Arc::new(tokio::sync::Mutex::new(rows.clone()));
    let mut set = tokio::task::JoinSet::new();
    for (i, row) in rows.into_iter().enumerate().take(n) {
        if matches!(row.status.as_str(), "pending" | "running" | "waiting") {
            let (c, states) = (c.clone(), Arc::clone(&states));
            set.spawn(async move { resume_row(&c, &states, i, &row).await });
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

async fn resume_row(c: &ResumeCtx, states: &SharedStates, i: usize, row: &ReviewAgentState) {
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
        let turn = StepTurn {
            user_id: c.user.id.clone(),
            workspace_id: c.ws.id.clone(),
            provider: row.provider.clone(),
            model: Some(row.model.clone()).filter(|m| !m.trim().is_empty()),
            effort: Some(row.effort.clone()).filter(|e| !e.trim().is_empty()),
            cwd: c.cwd.clone().into(),
            title: row.name.clone(),
            meta,
            // A held thread is awaited; an unheld one is told Otto restarted.
            prompt: String::new(),
            continue_thread: None,
            handoff: Some(findings_path(&c.review_id, i)),
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
            c.cancel.as_ref(),
            Some(thread.into()),
        )
        .await;
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
    // The thread is gone (or never was one): re-run the row.
    let prompt = c
        .reviews
        .get_agent_prompt(&c.review_id.clone().into(), i)
        .await;
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
        c.cancel.as_ref(),
        c.skills_dir.as_deref(),
        &row.lens_slugs,
        Some(&c.threads),
    )
    .await;
}
