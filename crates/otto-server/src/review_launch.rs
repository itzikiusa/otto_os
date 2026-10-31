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
        Some(workflow),
    )
    .await
    .map(|(id, resolved, no_changes, _)| (id, resolved, no_changes))
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
                        if review_adoptable(&review) {
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
            let _ = transition(
                ctx,
                &workspace.id,
                &review_id,
                ReviewStatus::Error,
                Some("workflow association could not be saved"),
            )
            .await;
            return Err(error);
        }
        if store.is_canceled(run_id).await {
            let _ = transition(
                ctx,
                &workspace.id,
                &review_id,
                ReviewStatus::Cancelled,
                None,
            )
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
        transition(ctx, &workspace.id, &review_id, ReviewStatus::Done, None).await?;
    } else {
        announce(ctx, &workspace.id, &review_id, ReviewStatus::Running);
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

/// Whether a workflow step may ADOPT the review its checkpoint names instead
/// of launching a new one: a review still running (restart adoption), or a
/// finished one whose reviewers all completed. A finished review with a
/// failed or partial reviewer is not adopted — "Retry step" on a review step
/// that failed `require_pass` for missing coverage re-scored the same
/// incomplete review on every retry and could never pass.
fn review_adoptable(review: &otto_core::domain::Review) -> bool {
    match review.status {
        ReviewStatus::Running => true,
        // A summary that fell back to the deterministic merge counts as
        // incomplete here too: a step that refuses fallbacks would re-adopt
        // it on every retry and never pass.
        ReviewStatus::Done => otto_review::engine::review_run_complete(
            &review.agents,
            review.summary_fallback.unwrap_or(false),
        ),
        _ => false,
    }
}

#[cfg(test)]
#[path = "../tests/unit/review_adoption.rs"]
mod adoption_tests;
