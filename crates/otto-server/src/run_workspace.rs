//! Run with Otto — isolated branch/worktree provisioning.
//!
//! A single-agent run works on a dedicated `otto-run/<id>` branch in a linked git
//! worktree under `<data_dir>/otto-runs/<id>/work`, so the agent's edits never
//! touch the user's checkout or branch. Provisioning is idempotent
//! (`worktree_add_if_absent`), which is also what makes the `provisioning` stage
//! safe to re-drive on a daemon restart. (Goal-loop mode provisions its own
//! `goal-loop/<id>` worktree; this is only used by single-agent mode.)

use otto_core::domain::Repo;
use otto_core::run::OttoRun;
use otto_core::Result;
use otto_git::LocalGit;

use crate::state::ServerCtx;

/// Ensure the run has an isolated worktree + branch. Returns
/// `(branch, worktree_path, base_commit)`. Idempotent across re-drives.
pub(crate) async fn provision_worktree(
    ctx: &ServerCtx,
    run: &OttoRun,
    repo: &Repo,
) -> Result<(String, String, String)> {
    let git = LocalGit::new(&repo.path);
    let base_commit = git.rev_parse("HEAD").await?;
    let branch = format!("otto-run/{}", run.id);
    let path = ctx
        .data_dir
        .join("otto-runs")
        .join(&run.id)
        .join("work");
    let path_str = path.to_string_lossy().to_string();
    git.worktree_add_if_absent(&path_str, &branch, &base_commit)
        .await?;
    Ok((branch, path_str, base_commit))
}

/// Best-effort worktree removal (keeps the branch so its commits survive for the
/// diff / PR draft). Called on cancel/delete and after completion.
pub(crate) async fn remove_worktree(_ctx: &ServerCtx, run: &OttoRun) {
    if let (Some(repo_path), Some(wt)) = (run.repo_path.as_deref(), run.worktree_path.as_deref()) {
        let git = LocalGit::new(repo_path);
        let _ = git.worktree_remove(wt).await;
    }
}
