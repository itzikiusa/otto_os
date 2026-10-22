//! Per-agent Retry / Stop for a review's reviewer rows and the summarizer.

use super::*;

/// Re-materialize the ORIGINAL PR review revision for a retry. The stored
/// diff and findings are from that revision, even if the forge head has moved.
/// Missing evidence fails explicitly instead of substituting another checkout.
pub(crate) async fn pr_retry_worktree(
    ctx: &ServerCtx,
    _user: &otto_core::domain::User,
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    pr_number: u64,
    suffix: &str,
) -> Result<PrWorktree> {
    let recorded = ctx
        .reviews_store
        .get_run_context(review_id)
        .await?
        .ok_or_else(|| {
            Error::Conflict("review revision is unavailable — start a new review".into())
        })?;
    let head = recorded
        .head_sha
        .as_deref()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| {
            Error::Conflict("review revision is unavailable — start a new review".into())
        })?;
    materialize_pr_worktree(
        ctx,
        repo,
        review_id,
        pr_number,
        recorded.source_branch.as_deref(),
        Some(head),
        suffix,
    )
    .await
    .ok_or_else(|| {
        Error::Conflict(
            "the original reviewed revision cannot be checked out — start a new review".into(),
        )
    })
}

/// The checkout a BRANCH review's (pr #0) retry runs in: the run's recorded
/// checkout when it still exists at the recorded head, else a throwaway
/// worktree at that head (returned so the caller tears it down), else — no
/// context recorded (pre-context reviews) or the head is gone — the repo path,
/// as before. Never returns a tear-down handle for a checkout Otto didn't make.
pub(crate) async fn branch_retry_checkout(
    ctx: &ServerCtx,
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    suffix: &str,
) -> (String, Option<PrWorktree>) {
    let Some(rc) = ctx
        .reviews_store
        .get_run_context(review_id)
        .await
        .ok()
        .flatten()
    else {
        return (repo.path.clone(), None);
    };
    let head = rc.head_sha.as_deref().filter(|h| !h.is_empty());
    if let Some(cwd) = rc
        .cwd
        .as_deref()
        .filter(|c| std::path::Path::new(c).is_dir())
    {
        // A LOCAL review ran in the user's own checkout and diffed its working
        // tree (uncommitted + untracked included). Committing those changes
        // moves HEAD, and a worktree at the OLD head would not contain them
        // at all — the checkout itself is still where the reviewed code lives
        // (S2-310). Only Run/workflow worktrees fall back to the recorded head.
        if same_dir(cwd, &repo.path) {
            return (cwd.to_string(), None);
        }
        let at = otto_git::LocalGit::new(cwd).rev_parse("HEAD").await.ok();
        let same = match (head, at.as_deref()) {
            (None, _) => true,
            (Some(h), Some(a)) => otto_review::worktree::sha_matches(a, h),
            (Some(_), None) => false,
        };
        if same {
            return (cwd.to_string(), None);
        }
    }
    if let Some(h) = head {
        let git = otto_git::LocalGit::new(&repo.path);
        let path = std::env::temp_dir()
            .join(format!("otto-review-wt-{review_id}{suffix}"))
            .to_string_lossy()
            .into_owned();
        let branch = format!("otto-review-{review_id}{suffix}");
        let _ = git.worktree_remove(&path).await;
        match git.worktree_add(&path, &branch, h).await {
            Ok(()) => {
                return (
                    path.clone(),
                    Some(PrWorktree {
                        path,
                        branch,
                        review_ref: None,
                    }),
                )
            }
            Err(e) => {
                tracing::warn!(review = %review_id, "retry worktree at {h} failed: {e}; using repo path")
            }
        }
    }
    (repo.path.clone(), None)
}

/// Agent rows a Retry may restart: settled ones only (`pending` / `running` /
/// `waiting` still belong to a live recovery loop).
pub(crate) fn agent_retryable(status: &str) -> bool {
    matches!(status, "done" | "error" | "skipped")
}

/// Why a single-agent Retry is refused (`None` ⇒ allowed). Besides the agent
/// itself being settled, the REVIEW must be: while it is still Running, the
/// original run's end-of-review temp sweep deletes the retried agent's
/// re-materialised `.diff` / `.prompt` / findings file mid-read, and its
/// summarizer runs without the retried agent's findings (S2-303) — the same
/// rule the summarizer retry already applies.
pub(crate) fn agent_retry_block(
    review_status: ReviewStatus,
    index: usize,
    agent_status: &str,
) -> Option<String> {
    if !agent_retryable(agent_status) {
        return Some(format!(
            "agent {index} is still {agent_status} — stop it before retrying"
        ));
    }
    if review_status == ReviewStatus::Running {
        return Some(
            "the review is still running — retry this agent once the review has finished".into(),
        );
    }
    None
}

/// Re-run a single review agent (e.g. one that never received its prompt). Uses
/// the prompt persisted when the review started, kills the agent's old (stuck)
/// session, and spawns a fresh one in the background.
pub(crate) async fn retry_review_agent(
    Path((review_id, index)): Path<(Id, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    // The id is a route param used as a temp-file name component below
    // (`otto-review-<id>.diff` + the legacy prompt file) — reject traversal
    // before it can reach a path join.
    if otto_core::paths::safe_component(&review_id).is_none() {
        return Err(crate::error::ApiError(Error::NotFound("review".into())));
    }
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    let workspace = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    if index >= review.agents.len() {
        return Err(crate::error::ApiError(Error::Invalid(format!(
            "no review agent at index {index}"
        ))));
    }
    // Only a SETTLED agent is retried. Retrying a live one archived its session
    // and replaced its cancel flag: the original recovery loop then respawned
    // into the same index + findings file, and its orphaned flag left neither
    // Stop nor Cancel able to reach it. A stuck agent is Stopped first.
    if let Some(why) = agent_retry_block(review.status, index, &review.agents[index].status) {
        return Err(crate::error::ApiError(Error::Conflict(why)));
    }
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;
    // DB-first (durable across reboot / temp-sweep / redeploy); the temp file
    // is the legacy fallback for reviews that predate migration 0100.
    let prompt = match ctx.reviews_store.get_agent_prompt(&review_id, index).await {
        Ok(Some(p)) => p,
        _ => std::fs::read_to_string(crate::review_session::prompt_path(&review_id, index))
            .map_err(|_| {
                crate::error::ApiError(Error::Invalid(
                    "cannot retry: this agent's prompt is no longer available — re-run the review"
                        .into(),
                ))
            })?,
    };

    let review_user = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .ok()
        .and_then(|us| us.into_iter().find(|u| u.is_root))
        .ok_or_else(|| crate::error::ApiError(Error::Internal("no root user".into())))?;

    // Mark the agent pending again so the UI reflects the retry immediately.
    // Write only this agent's element (atomic per-index) so we never revert the
    // other agents' live rows.
    let states: crate::review_session::SharedStates =
        Arc::new(tokio::sync::Mutex::new(review.agents.clone()));
    {
        let row = {
            let mut g = states.lock().await;
            g.get_mut(index).map(|s| {
                s.status = "pending".into();
                s.note = "retrying…".into();
                s.session_id = None;
                s.findings = Vec::new();
                s.comment_count = 0;
                s.clone()
            })
        };
        if let Some(row) = row {
            // Claim, don't just write: a concurrent retry of the same agent
            // loses here (409) instead of spawning a second agent.
            if !ctx
                .reviews_store
                .claim_agent_retry(&review_id, index, &row)
                .await
                .map_err(crate::error::ApiError)?
            {
                return Err(crate::error::ApiError(Error::Conflict(format!(
                    "agent {index} is already being retried"
                ))));
            }
        }
    }
    // Kill the old (likely stuck) session so it doesn't linger.
    if let Some(sid) = review.agents[index].session_id.clone() {
        let _ = ctx.manager.archive(&sid).await;
    }

    let provider = review.agents[index].provider.clone();
    // An orchestrator row re-spawns as an orchestrator (sub-agent tool + the
    // per-lens findings files its persisted prompt names) — S2-304.
    let lens_slugs = review.agents[index].lens_slugs.clone();
    // Retry this reviewer with the SAME model it was configured with (empty →
    // provider default) so the retry matches the original run.
    let model = review.agents[index].model.clone();
    let pr_number = review.pr_number;
    // The prompt references the diff via an absolute temp path — re-materialize
    // it from the durable row if a reboot/temp-sweep removed it (pre-0100
    // reviews have no stored diff and need their legacy temp file). The
    // restored length also feeds the grace-period heuristic, which previously
    // collapsed to the 10-minute floor whenever the temp file was gone.
    // Register a FRESH per-agent cancel flag (replacing any tripped one from a
    // prior Stop) so the retried agent is stoppable exactly like the original.
    // Registered BEFORE the diff is re-materialised: a sibling retry's
    // end-of-retry temp sweep (below) skips while any agent of this review is
    // registered, so it can never delete the file this retry is about to use.
    let agent_registration =
        register_review_agent_cancel(&ctx.review_agent_cancels, &review_id, index);
    let agent_cancel = Arc::clone(agent_registration.flag());
    let diff_file = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
    let mut diff_len = std::fs::metadata(&diff_file)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    // A diff that can't be re-materialised refuses the retry instead of
    // launching an agent at a missing file (it would read nothing and could
    // report a clean pass).
    if diff_len == 0 {
        let diff = match ctx.reviews_store.get_diff(&review_id).await {
            Ok(Some(diff)) => diff,
            Ok(None) => {
                return Err(crate::error::ApiError(Error::Conflict(
                    "this review has no stored diff to retry against; start a new review".into(),
                )))
            }
            Err(e) => {
                return Err(crate::error::ApiError(Error::Internal(diff_write_failure(
                    &e,
                ))))
            }
        };
        diff_len = diff.len();
        if let Err(e) = std::fs::write(&diff_file, diff) {
            return Err(crate::error::ApiError(Error::Internal(diff_write_failure(
                &e,
            ))));
        }
    }
    let timeout = review_agent_timeout(diff_len, None);
    let manager = Arc::clone(&ctx.manager);
    let reviews = ctx.reviews_store.clone();
    let review_id_bg = review_id.clone();
    // Re-stage the FULL skill library (idempotent, shared bundle) so a retried
    // CLAUDE agent can invoke `Skill(<lens>)` exactly like the original run —
    // which also stages the whole library (a review config usually carries the
    // lens in the agent NAME with `skill` empty, so filtering on `skill` here
    // would leave the common case with an empty bundle). codex/agy ignore the
    // bundle and re-read the lens method inline from the persisted prompt.
    let review_skills_dir = stage_all_review_skills(&ctx.context_library).await;
    let agent_cancels_reg = ctx.review_agent_cancels.clone();
    let slots = reviewer_slots();
    let ctx_bg = ctx.clone();
    tokio::spawn(async move {
        let _slot = slots.acquire_owned().await.ok(); // SI-11 reviewer cap
                                                      // A PR reviewer must verify against the PR head, not the user's
                                                      // checkout (the original run's worktree is gone) — rebuild one, unique
                                                      // per retry so concurrent retries never tear down each other's tree.
        let suffix = format!("-a{index}-{}", chrono::Utc::now().timestamp_millis());
        let (cwd, pr_wt) = if pr_number != 0 {
            let wt =
                match pr_retry_worktree(&ctx_bg, &user, &repo, &review_id_bg, pr_number, &suffix)
                    .await
                {
                    Ok(wt) => wt,
                    Err(error) => {
                        let mut row = states.lock().await[index].clone();
                        row.status = "error".into();
                        row.note = error.to_string();
                        let _ = reviews.set_agent_at(&review_id_bg, index, &row).await;
                        drop(agent_registration);
                        return;
                    }
                };
            (wt.path.clone(), Some(wt))
        } else {
            // A branch review re-enters the checkout it ran in (S2-06), not
            // the user's main checkout on whatever branch it is on now.
            branch_retry_checkout(&ctx_bg, &repo, &review_id_bg, &suffix).await
        };
        crate::review_session::run_agent_session_with_recovery(
            &manager,
            &reviews,
            &states,
            &workspace,
            &review_user,
            &provider,
            &model,
            &cwd,
            &review_id_bg,
            index,
            &prompt,
            timeout,
            None,
            Some(&agent_cancel), // per-agent Stop works on retried agents too
            review_skills_dir.as_deref(),
            &lens_slugs,
        )
        .await;
        if let Some(wt) = pr_wt {
            teardown_pr_worktree(&repo.path, wt).await;
        }
        drop(agent_registration);
        // The last retry of this (settled) review out sweeps the temp files it
        // re-materialised — only `run_review` swept them before (S2-310).
        let () = crate::offload::blocking(move || {
            sweep_review_temp_files_if_idle(&agent_cancels_reg, &review_id_bg)
        })
        .await;
    });

    Ok(Json(
        ctx.reviews_store
            .get_review(&review_id)
            .await
            .map_err(crate::error::ApiError)?,
    ))
}

/// `POST /reviews/{review_id}/summarizer/retry` — re-run ONLY the summarize +
/// persist stage from the STORED per-agent findings, replacing the review's
/// unposted draft comments. Cheap compared to re-running every reviewer: the
/// findings already exist; only the final dedupe/rank turn is repeated.
pub(crate) async fn retry_summarizer(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    let workspace = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    if review.status == ReviewStatus::Running {
        return Err(crate::error::ApiError(Error::Invalid(
            "review is still running — wait for it to finish first".into(),
        )));
    }
    if review.agents.len() < 2 {
        return Err(crate::error::ApiError(Error::Invalid(
            "this review has no summarizer stage to retry".into(),
        )));
    }
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    // The reviewers' findings are durable on the agent rows — the summarizer is
    // always the last row and contributes none.
    let agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> = review.agents
        [..review.agents.len() - 1]
        .iter()
        .map(|a| a.findings.clone())
        .collect();
    // Only findings with actual content count — a run whose findings were
    // gutted (empty bodies) has nothing to summarize and needs a re-run.
    if !agent_findings
        .iter()
        .flatten()
        .any(|f| !f.body.trim().is_empty())
    {
        return Err(crate::error::ApiError(Error::Invalid(
            "no stored agent findings with content to summarize — re-run the review instead".into(),
        )));
    }

    // Flip the run back to running so the UI tracks the re-summarize live —
    // atomically: the status check above is a fast-path, this is the guard.
    // Two quick clicks must not both re-summarize (each would persist a full
    // set of drafts).
    if !ctx
        .reviews_store
        .try_begin_rerun(&review_id)
        .await
        .map_err(crate::error::ApiError)?
    {
        return Err(crate::error::ApiError(Error::Invalid(
            "review is still running — wait for it to finish first".into(),
        )));
    }
    // The claim excludes live reviewer retries. Reload AFTER claiming: a reviewer
    // may have completed between the handler's initial read and atomic admission.
    let settled = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> = settled
        .agents
        .iter()
        .take(settled.agents.len().saturating_sub(1))
        .map(|a| a.findings.clone())
        .collect();
    // Each retry has its own cancellation flag; a cancelled earlier attempt
    // must not poison the new session, and Cancel must cover this task too
    // (registered only once this attempt owns the review).
    let attempt_registration = ctx.review_cancels.register(&review_id);
    let attempt_cancel = Arc::clone(attempt_registration.flag());
    announce(&ctx, &workspace.id, &review_id, ReviewStatus::Running);

    let ctx_bg = ctx.clone();
    let review_id_bg = review_id.clone();
    let repo_id = review.repo_id.clone();
    let pr_number = review.pr_number;
    tokio::spawn(async move {
        let _cancel_guard = attempt_registration;
        // Fingerprints anchor on the flagged line's TEXT; a PR review's lines
        // are in the PR head, not the user's checkout. Rebuild a head worktree
        // for anchoring so the retry re-keys onto the original findings.
        let (repo_path, pr_wt) = if pr_number != 0 {
            let wt =
                match pr_retry_worktree(&ctx_bg, &user, &repo, &review_id_bg, pr_number, "-sum")
                    .await
                {
                    Ok(wt) => wt,
                    Err(error) => {
                        if attempt_cancel.load(std::sync::atomic::Ordering::SeqCst)
                            || review_is_cancelled(&ctx_bg, &review_id_bg).await
                        {
                            return;
                        }
                        let error = error.to_string();
                        let _ = transition(
                            &ctx_bg,
                            &workspace.id,
                            &review_id_bg,
                            ReviewStatus::Error,
                            Some(&error),
                        )
                        .await;
                        return;
                    }
                };
            (wt.path.clone(), Some(wt))
        } else {
            // Anchor on the branch review's own checkout/head (S2-06): the
            // user's checkout holds other line text and would re-key nothing.
            branch_retry_checkout(&ctx_bg, &repo, &review_id_bg, "-sum").await
        };
        // The summarizer follows the repo's EFFECTIVE config (per-repo binding
        // resolution included), same as a fresh run would.
        let cfg = load_review_config_for_repo(&ctx_bg, &repo_id).await;
        // A re-run replaces the previous draft comments; posted/approved/
        // declined ones are user decisions and stay untouched.
        match ctx_bg
            .reviews_store
            .delete_draft_comments(&review_id_bg)
            .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(review = %review_id_bg, "summarizer retry: cleared {n} draft comments")
            }
            Err(e) => {
                tracing::warn!(review = %review_id_bg, "summarizer retry: clear drafts failed: {e}")
            }
            _ => {}
        }
        let result = summarize_and_persist(
            &ctx_bg,
            &review_id_bg,
            &repo_path,
            &workspace,
            &user,
            &repo_id,
            pr_number,
            &cfg.summarizer,
            &agent_findings,
            // A retry re-runs ONLY this stage, from findings stored earlier; the
            // run that produced them is long gone, so there is no context to
            // re-render. The findings themselves are unchanged either way.
            "",
            // The original run's cwd/head is gone (a PR worktree is torn down
            // after the run; the head may have moved since), so this attempt
            // cannot prove a finding absent — it only re-summarizes.
            Some("summarizer retry"),
        )
        .await;
        if let Some(wt) = pr_wt {
            teardown_pr_worktree(&repo.path, wt).await;
        }
        if attempt_cancel.load(std::sync::atomic::Ordering::SeqCst)
            || review_is_cancelled(&ctx_bg, &review_id_bg).await
        {
            return;
        }
        let status = match result {
            Ok(()) => {
                tracing::info!(review = %review_id_bg, "summarizer retry complete");
                ReviewStatus::Done
            }
            Err(e) => {
                tracing::warn!(review = %review_id_bg, "summarizer retry failed: {e}");
                let error = e.to_string();
                let _ = transition(
                    &ctx_bg,
                    &workspace.id,
                    &review_id_bg,
                    ReviewStatus::Error,
                    Some(&error),
                )
                .await;
                return;
            }
        };
        let _ = transition(&ctx_bg, &workspace.id, &review_id_bg, status, None).await;
    });

    Ok(Json(
        ctx.reviews_store
            .get_review(&review_id)
            .await
            .map_err(crate::error::ApiError)?,
    ))
}

/// `POST /reviews/{review_id}/agents/{index}/stop` — stop one running/waiting
/// review agent without touching the rest of the run. Trips the agent's cancel
/// flag FIRST (so the recovery loop sees the kill as intentional, not a failure
/// to auto-retry), kills its live session, and marks the row `error` /
/// "stopped by user" — deliberately NOT a new status value, so no contract enum
/// change and the existing retry path (error rows are retryable) immediately
/// applies. The parent run rolls up exactly as an agent error does today: the
/// stopped agent leaves the pending set and the summarizer proceeds with the
/// remaining findings. 409 unless the row is running/waiting; the trailing
/// summarizer row (no recovery loop of its own) is never stoppable.
pub(crate) async fn stop_review_agent(
    Path((review_id, index)): Path<(Id, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<(StatusCode, Json<Review>)> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    if index >= review.agents.len() {
        return Err(crate::error::ApiError(Error::Invalid(format!(
            "no review agent at index {index}"
        ))));
    }
    if index == review.agents.len() - 1 {
        return Err(crate::error::ApiError(Error::Conflict(
            "the summarizer cannot be stopped".into(),
        )));
    }
    let status = review.agents[index].status.clone();
    if status != "running" && status != "waiting" {
        return Err(crate::error::ApiError(Error::Conflict(format!(
            "agent is {status} — only a running or waiting agent can be stopped"
        ))));
    }

    // Signal the recovery loop FIRST so the kill below reads as intentional
    // (Stopped, not SessionGone-then-auto-retry) — same ordering as Product's
    // per-agent stop.
    signal_review_agent_cancel(&ctx.review_agent_cancels, &review_id, index);
    if let Some(sid) = review.agents[index].session_id.as_ref() {
        let _ = ctx.manager.kill_session(sid).await;
    }
    // Persist the terminal row immediately so the UI reflects the stop without
    // waiting for the recovery loop to unwind (it re-persists the same
    // status/note via review_error_note(Stopped)).
    {
        let mut row = review.agents[index].clone();
        row.status = "error".into();
        row.note = "stopped by user".into();
        let _ = ctx
            .reviews_store
            .set_agent_at(&review_id, index, &row)
            .await;
    }
    // Reuse the existing review WS family so open panels refresh.
    announce(&ctx, &repo.workspace_id, &review_id, ReviewStatus::Running);

    Ok((
        StatusCode::ACCEPTED,
        Json(
            ctx.reviews_store
                .get_review(&review_id)
                .await
                .map_err(crate::error::ApiError)?,
        ),
    ))
}

#[cfg(test)]
mod review_retry_tests {
    use super::{agent_retry_block, same_dir, sweep_review_temp_files_if_idle};
    use otto_core::domain::{ReviewAgentState, ReviewStatus};

    /// S2-303: a settled agent is retryable only once the review has settled.
    #[test]
    fn agent_retry_needs_a_settled_agent_and_a_settled_review() {
        assert!(agent_retry_block(ReviewStatus::Done, 0, "done").is_none());
        assert!(agent_retry_block(ReviewStatus::Error, 0, "error").is_none());
        assert!(agent_retry_block(ReviewStatus::Cancelled, 0, "skipped").is_none());
        let why = agent_retry_block(ReviewStatus::Running, 0, "done").unwrap();
        assert!(why.contains("still running"), "{why}");
        let why = agent_retry_block(ReviewStatus::Done, 2, "running").unwrap();
        assert!(why.contains("agent 2 is still running"), "{why}");
    }

    /// S2-304: an orchestrator row carries its lens slugs through agents_json,
    /// so the retry route re-spawns it WITH its sub-agent tool; fan-out rows
    /// (and pre-field rows) stay without one.
    #[test]
    fn orchestrator_row_keeps_its_lens_slugs_for_retry() {
        let row: ReviewAgentState = serde_json::from_value(serde_json::json!({
            "name": "orchestrator", "provider": "claude", "model": "",
            "status": "error", "note": "", "comment_count": 0,
            "lens_slugs": ["correctness", "security"],
        }))
        .unwrap();
        let back = serde_json::to_value(&row).unwrap();
        assert_eq!(
            back["lens_slugs"],
            serde_json::json!(["correctness", "security"])
        );
        let meta = crate::review_session::review_session_meta("r1", 0, !row.lens_slugs.is_empty());
        assert_eq!(meta["allow_subagents"], true);

        let fan_out: ReviewAgentState = serde_json::from_value(serde_json::json!({
            "name": "correctness", "provider": "claude", "model": "",
            "status": "error", "note": "", "comment_count": 0,
        }))
        .unwrap();
        assert!(fan_out.lens_slugs.is_empty());
        assert!(serde_json::to_value(&fan_out)
            .unwrap()
            .get("lens_slugs")
            .is_none());
        let meta = crate::review_session::review_session_meta("r1", 0, false);
        assert!(meta.get("allow_subagents").is_none());
    }

    /// S2-310: a local review's recorded cwd IS the repo checkout — matched
    /// through symlinks / a trailing slash — so its retry stays there even
    /// after HEAD moved; a Run worktree elsewhere is not the repo.
    #[test]
    fn local_review_checkout_is_recognised_as_the_repo() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&repo, &link).unwrap();
        let r = repo.to_string_lossy().into_owned();
        assert!(same_dir(&r, &format!("{r}/")));
        assert!(same_dir(&link.to_string_lossy(), &r));
        let wt = tmp.path().join("otto-run-1");
        std::fs::create_dir_all(&wt).unwrap();
        assert!(!same_dir(&wt.to_string_lossy(), &r));
    }

    /// S2-310: a retry's temp files are swept once no agent of the review is
    /// registered — never while a sibling retry still is.
    #[test]
    fn retry_sweeps_temp_files_only_when_no_agent_is_left() {
        let reg: crate::skill_eval::CancelRegistry = Default::default();
        let rid = format!("t{}", otto_core::new_id());
        let diff = std::env::temp_dir().join(format!("otto-review-{rid}.diff"));
        std::fs::write(&diff, "d").unwrap();
        let registration = super::register_review_agent_cancel(&reg, &rid, 1);
        sweep_review_temp_files_if_idle(&reg, &rid);
        assert!(diff.exists(), "a sibling retry is still registered");
        drop(registration);
        sweep_review_temp_files_if_idle(&reg, &rid);
        assert!(!diff.exists(), "the last retry out sweeps");
    }

    /// S15-307: an empty draft on the checked-out branch with uncommitted work
    /// says to commit first; otherwise the plain "no changes".
    #[test]
    fn empty_pr_draft_names_uncommitted_work() {
        let msg = super::no_pr_changes_message("feat", "main", true);
        assert!(msg.contains("Commit your changes first"), "{msg}");
        assert!(msg.contains("'feat'") && msg.contains("'main'"), "{msg}");
        assert_eq!(
            super::no_pr_changes_message("feat", "main", false),
            "no changes between 'feat' and 'main'"
        );
    }
}
