//! Isolated git worktree provisioning for Goal Loops.
//!
//! Every loop runs on its own branch `goal-loop/<id>` in a dedicated worktree
//! under the daemon data dir — NEVER the user's checkout. Branch names are
//! ULID-unique, so provisioning is a FRESH create (not the destructive
//! `-B --force` reuse the swarm uses for multi-turn agents): if the path already
//! exists and we have no record of it, we fail loudly rather than clobber.

use otto_core::domain::GoalLoop;
use otto_core::{Error, Result};

use crate::state::ServerCtx;

/// Directory holding a loop's worktree.
fn worktree_dir(ctx: &ServerCtx, loop_id: &str) -> std::path::PathBuf {
    ctx.data_dir
        .join("goal-loops")
        .join(loop_id)
        .join("work")
}

/// Ensure the loop has an isolated worktree + branch, returning
/// `(branch, worktree_path, base_commit)`.
///
/// Idempotent across start/resume: when the loop already recorded a worktree and
/// it still exists on disk, it is reused as-is (the loop's prior commits are
/// preserved). Otherwise a fresh worktree is created from the repo's current
/// HEAD. A pre-existing path with no record is an error (we never reuse foreign
/// or stale trees, and never force-reset a branch).
pub async fn provision_worktree(ctx: &ServerCtx, loop_: &GoalLoop) -> Result<(String, String, String)> {
    let git = otto_git::LocalGit::new(&loop_.repo_path);
    let path = worktree_dir(ctx, &loop_.id);
    let path_str = path.to_string_lossy().to_string();

    // Resume: reuse the loop's existing worktree if it's still registered.
    if let (Some(branch), Some(wt)) = (loop_.branch.clone(), loop_.worktree_path.clone()) {
        if git.worktree_exists(&wt).await {
            let base = loop_.base_commit.clone().unwrap_or_default();
            return Ok((branch, wt, base));
        }
    }

    let branch = format!("goal-loop/{}", loop_.id);

    if git.worktree_exists(&path_str).await {
        return Err(Error::Internal(format!(
            "goal-loop worktree path already registered (refusing to reuse): {path_str}"
        )));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // If the branch already exists (e.g. resuming a loop whose worktree was
    // removed), RE-ATTACH it non-destructively — never `-B`-reset it, which
    // would discard the loop's accumulated commits.
    if git.branch_exists(&branch).await {
        git.worktree_attach(&path_str, &branch)
            .await
            .map_err(|e| Error::Internal(format!("re-attach goal-loop worktree: {e}")))?;
        let base = loop_
            .base_commit
            .clone()
            .or_else(|| Some(branch.clone()))
            .unwrap_or_default();
        tracing::info!("goal-loop: re-attached worktree {path_str} on existing {branch}");
        return Ok((branch, path_str, base));
    }

    // True fresh launch. Capture the launch HEAD as the diff base.
    let base = match git.rev_parse("HEAD").await {
        Ok(sha) => sha,
        Err(_) => git
            .current_branch()
            .await
            .unwrap_or_else(|_| "HEAD".to_string()),
    };
    git.worktree_add(&path_str, &branch, &base)
        .await
        .map_err(|e| Error::Internal(format!("create goal-loop worktree: {e}")))?;
    tracing::info!("goal-loop: created worktree {path_str} on {branch} (base {base})");
    Ok((branch, path_str, base))
}

/// Best-effort: remove the loop's worktree (keeps the branch so the diff
/// survives). Called on finalize, delete, and boot cleanup.
pub async fn remove_worktree(ctx: &ServerCtx, loop_: &GoalLoop) {
    let git = otto_git::LocalGit::new(&loop_.repo_path);
    let path = loop_
        .worktree_path
        .clone()
        .unwrap_or_else(|| worktree_dir(ctx, &loop_.id).to_string_lossy().to_string());
    let _ = git.worktree_remove(&path).await;
}
