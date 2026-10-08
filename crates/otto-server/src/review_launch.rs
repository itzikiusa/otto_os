//! Durable review launch and workflow adoption.
use super::*;

/// Launch an AI review on a specific branch/worktree (Run with Otto's `reviewing`
/// stage and the workflow `review_run` node). Resolves `base` (an explicit
/// ref/SHA, or `None` ⇒ the repo's detected default branch — never a
/// fabricated `main`), diffs `worktree_path` against it, creates a
/// local-review row keyed to `repo_id`, and drives `run_review` in the
/// background. Returns the `review_id` plus the base actually used, so
/// callers publish the RESOLVED branch (a downstream PR must target what was
/// really reviewed); the caller polls `reviews_store.get_review` for
/// `Done`/`Error` and reads counts via [`review_findings_counts`].
///
/// The third return value is `no_changes`: the diff vs the resolved base was
/// EMPTY, so the review completed instantly with no reviewers and no findings.
/// Callers MUST NOT read that as a clean review — zero findings there means
/// nothing was looked at. Scoring it (100/PASS) is how a misconfigured base
/// silently green-lights an unreviewed PR.
///
/// `jira_context` / `run_context` are what the caller already knows about the
/// change — the ticket, and the briefs earlier steps produced. Both reach every
/// reviewer's prompt. Passing `None` makes the reviewers re-derive the system
/// from raw hunks, which is how documented, intentional behavior gets reported
/// as a defect of the change that happened to touch its line.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_review_for_branch(
    ctx: &ServerCtx,
    repo_id: &Id,
    worktree_path: &str,
    base: Option<&str>,
    cfg_override: Option<ReviewConfig>,
    jira_context: Option<String>,
    run_context: Option<String>,
    mode_override: Option<otto_core::domain::ReviewMode>,
    workflow: (&Id, otto_core::workflows::WorkflowCheckpoint),
) -> Result<(Id, otto_git::ResolvedBase, bool)> {
    start_review_for_branch(
        ctx,
        repo_id,
        worktree_path,
        base,
        cfg_override,
        jira_context,
        run_context,
        mode_override,
        None,
        Some(workflow),
    )
    .await
    .map(|(id, resolved, no_changes, _)| (id, resolved, no_changes))
}

/// [`run_review_for_branch`] that also returns the reviewed diff's byte
/// length — the caller's wait budget is sized off it, and recomputing the
/// whole review diff (one git spawn per untracked file) just for its length
/// doubled the stage's git work.
///
/// `for_run`: a Run-with-Otto run to record the new `review_id` on BEFORE the
/// background review is spawned. A run cancel drops the stage future at any
/// await; recording it afterwards (in the stage) left a window where the
/// review was running but the run did not name it, so cancel could not stop
/// its reviewers (S2-302 / S15-306).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_review_for_branch_sized(
    ctx: &ServerCtx,
    repo_id: &Id,
    worktree_path: &str,
    base: Option<&str>,
    cfg_override: Option<ReviewConfig>,
    jira_context: Option<String>,
    run_context: Option<String>,
    mode_override: Option<otto_core::domain::ReviewMode>,
    for_run: Option<&Id>,
) -> Result<(Id, otto_git::ResolvedBase, bool, usize)> {
    start_review_for_branch(
        ctx,
        repo_id,
        worktree_path,
        base,
        cfg_override,
        jira_context,
        run_context,
        mode_override,
        for_run,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn start_review_for_branch(
    ctx: &ServerCtx,
    repo_id: &Id,
    worktree_path: &str,
    base: Option<&str>,
    mut cfg_override: Option<ReviewConfig>,
    jira_context: Option<String>,
    run_context: Option<String>,
    mode_override: Option<otto_core::domain::ReviewMode>,
    for_run: Option<&Id>,
    mut workflow: Option<(&Id, otto_core::workflows::WorkflowCheckpoint)>,
) -> Result<(Id, otto_git::ResolvedBase, bool, usize)> {
    let repo = ctx.git_store.get_repo(repo_id).await?;
    let workspace = ctx.workspaces.get(&repo.workspace_id).await?;
    let git = otto_git::LocalGit::new(worktree_path);
    let resolved = git.resolve_base(base).await?;
    let diff_text = git.review_diff_text(&resolved.diff_ref).await?;
    if let Some((run_id, checkpoint)) = workflow.as_mut() {
        use sha2::{Digest, Sha256};
        // Freeze the actual reviewed content and effective configuration. A node
        // id alone cannot distinguish a later loop iteration or changed target.
        let config = match cfg_override.take() {
            Some(config) => config,
            None => load_review_config_for_repo(ctx, repo_id).await,
        };
        checkpoint.input = serde_json::json!({
            "request": checkpoint.input, "repo_id":repo_id, "worktree":worktree_path,
            "base":resolved.diff_ref, "head":git.rev_parse("HEAD").await?,
            "diff_sha256":hex::encode(Sha256::digest(diff_text.as_bytes())),
            "config":config, "mode":mode_override, "jira_context":jira_context, "run_context":run_context,
        });
        cfg_override = Some(config);
        let store = otto_state::WorkflowsRepo::new(ctx.pool.clone());
        if store.is_canceled(run_id).await {
            return Err(Error::Conflict(
                "workflow canceled before review launch".into(),
            ));
        }
        if let Some(prior) = store.checkpoint(run_id, &checkpoint.node_id).await? {
            if prior.input == checkpoint.input {
                if let Some(id) = prior
                    .output
                    .as_ref()
                    .and_then(|out| out.get("review_id"))
                    .and_then(serde_json::Value::as_str)
                {
                    if let Ok(review) = ctx.reviews_store.get_review(&id.to_string()).await {
                        if matches!(review.status, ReviewStatus::Done | ReviewStatus::Running) {
                            return Ok((
                                review.id,
                                resolved,
                                diff_text.trim().is_empty(),
                                diff_text.len(),
                            ));
                        }
                    }
                }
            }
        }
        if store.is_canceled(run_id).await {
            return Err(Error::Conflict(
                "workflow canceled before review launch".into(),
            ));
        }
    }
    // Adopting completed work consumes no review budget.
    review_budget_gate(ctx, &repo.workspace_id).await?;
    let review = ctx
        .reviews_store
        .create_review(repo_id, LOCAL_REVIEW_PR_NUMBER)
        .await?;
    let review_id = review.id.clone();
    if let Some(run_id) = for_run {
        ctx.runs
            .set_fields(
                run_id,
                &otto_state::runs::RunPatch {
                    review_id: Some(review_id.clone()),
                    ..Default::default()
                },
            )
            .await?;
    }
    let no_changes = diff_text.trim().is_empty();
    let diff_len = diff_text.len();
    if let Some((run_id, mut checkpoint)) = workflow {
        checkpoint.status = otto_core::workflows::NodeStatus::Success;
        checkpoint.attempts = 1;
        checkpoint.output = Some(
            serde_json::json!({"review_id":review_id,"base":resolved.branch,"no_changes":no_changes}),
        );
        checkpoint.updated_at = chrono::Utc::now();
        // Save before spawning: even cancellation between this call and the
        // workflow's live channel sees the durable child association.
        let store = otto_state::WorkflowsRepo::new(ctx.pool.clone());
        if let Err(error) = store.save_checkpoint(run_id, &checkpoint).await {
            let _ = ctx
                .reviews_store
                .set_status(
                    &review_id,
                    ReviewStatus::Error,
                    Some("workflow association could not be saved"),
                )
                .await;
            return Err(error);
        }
        if store.is_canceled(run_id).await {
            let _ = ctx
                .reviews_store
                .set_status(&review_id, ReviewStatus::Cancelled, None)
                .await;
            return Err(Error::Conflict(
                "workflow canceled before review launch".into(),
            ));
        }
    }

    if no_changes {
        // No changes vs base — complete immediately with no findings. Loud,
        // because "0 findings" here is indistinguishable from a clean review in
        // every downstream count; the caller gets `no_changes` to tell them apart.
        tracing::warn!(
            review = %review_id, worktree = %worktree_path, base = %resolved.diff_ref,
            "review: EMPTY diff vs base — no reviewers ran, no findings are possible"
        );
        ctx.reviews_store
            .set_status(&review_id, ReviewStatus::Done, None)
            .await?;
        let _ = ctx.events.send(Event::ReviewChanged {
            workspace_id: workspace.id.clone(),
            session_id: None,
            review_id: review_id.clone(),
            status: ReviewStatus::Done.as_str().to_string(),
        });
    } else {
        let _ = ctx.events.send(Event::ReviewChanged {
            workspace_id: workspace.id.clone(),
            session_id: None,
            review_id: review_id.clone(),
            status: ReviewStatus::Running.as_str().to_string(),
        });
        let ctx_bg = ctx.clone();
        let rid = review_id.clone();
        let wt = worktree_path.to_string();
        let repo_id_bg = repo_id.clone();
        // The worktree already holds the branch's real code; name both sides
        // for the reviewers. A SHA base shows as itself — still meaningful.
        let dest = resolved.branch.clone();
        let branches = git
            .current_branch()
            .await
            .ok()
            .filter(|c| !c.is_empty())
            .map(|source| ReviewBranches {
                source,
                dest,
                checked_out: true,
            });
        tokio::spawn(async move {
            run_review(
                ctx_bg,
                rid,
                wt,
                diff_text,
                jira_context,
                run_context,
                workspace,
                repo_id_bg,
                0,
                branches,
                cfg_override,
                mode_override,
            )
            .await;
        });
    }
    Ok((review_id, resolved, no_changes, diff_len))
}
