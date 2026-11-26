//! The driver of a review whose agents survived a daemon restart in their
//! holders (runtime.md §7.8): it holds the review's cancel flag (so a
//! workflow's `review_run` stall check sees a live runner), lets
//! `otto_review::thread_resume` settle the reviewer rows, then summarizes.

use super::*;

/// `live`: the review's cancel flag, registered by boot before this task
/// runs, so a resumed workflow step sees a live review at once (CR1-028).
pub(crate) async fn resume_held_review(
    ctx: ServerCtx,
    review_id: Id,
    live: otto_core::cancel::CancelGuard,
) {
    let cancel = Arc::clone(live.flag());
    let review = match ctx.reviews_store.get_review(&review_id).await {
        Ok(r) => r,
        Err(e) => return tracing::warn!(review = %review_id, "held review resume: {e}"),
    };
    let Ok(repo) = ctx.git_store.get_repo(&review.repo_id).await else {
        return;
    };
    let run = ctx
        .reviews_store
        .get_run_context(&review_id)
        .await
        .ok()
        .flatten();
    let cwd = run.and_then(|r| r.cwd).unwrap_or_else(|| repo.path.clone());
    let result = async {
        let ws = ctx.workspaces.get(&repo.workspace_id).await?;
        let user = otto_review::restart::review_user(&otto_state::UsersRepo::new(ctx.pool.clone()))
            .await?;
        let cfg = load_review_config_for_repo(&ctx, &repo.id).await;
        let Some(threads) = review_threads(&ctx, Some(&cfg)).await else {
            // Switched to the terminal runner meanwhile: the held agents stop.
            for agent in &review.agents {
                end_review_agent(&ctx, agent).await;
            }
            return Err(Error::Invalid(otto_review::restart::INTERRUPTED.into()));
        };
        let diff = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
        let diff_len = std::fs::metadata(&diff)
            .map(|m| m.len() as usize)
            .unwrap_or(0);
        let resume = otto_review::thread_resume::ResumeCtx {
            manager: Arc::clone(&ctx.manager),
            threads,
            reviews: ctx.reviews_store.clone(),
            ws: ws.clone(),
            user: user.clone(),
            cwd: cwd.clone(),
            review_id: review_id.to_string(),
            grace: review_agent_timeout(diff_len, cfg.timeout_secs),
            skills_dir: stage_all_review_skills(&ctx.context_library).await,
            cancel: Some(Arc::clone(&cancel)),
        };
        let keep = resume.threads.clone();
        let findings = otto_review::thread_resume::resume_reviewers(resume).await?;
        let blocker = Some(otto_review::verification::SUMMARIZER_RETRY_BLOCKER);
        let pr = review.pr_number;
        summarize_and_persist(
            &ctx, &review_id, &cwd, &ws, &user, &repo.id, pr, &cfg, &findings, "", blocker,
        )
        .await?;
        // A finished review's threads are kept for the success period.
        if let Ok((_, rows)) = ctx.reviews_store.review_agents(&review_id).await {
            keep.finished(&rows).await;
        }
        Ok::<(), Error>(())
    }
    .await;
    remove_review_temp_files(&review_id).await;
    // The PR checkout the review ran in goes with it, as after a live review.
    let own = format!("otto-review-wt-{review_id}");
    if std::path::Path::new(&cwd)
        .file_name()
        .is_some_and(|n| *n == *own)
    {
        let branch = format!("otto-review-{review_id}");
        let wt = PrWorktree {
            path: cwd.clone(),
            branch,
            review_ref: None,
        };
        teardown_pr_worktree(&repo.path, wt).await;
    }
    if review_is_cancelled(&ctx, &review_id).await {
        return;
    }
    let (status, error) = match &result {
        Ok(()) => (ReviewStatus::Done, None),
        Err(e) => (ReviewStatus::Error, Some(e.to_string())),
    };
    let _ = transition(
        &ctx,
        &repo.workspace_id,
        &review_id,
        status,
        error.as_deref(),
    )
    .await;
    drop(live);
}
