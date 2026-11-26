//! A run's isolated worktrees under `<data_dir>/workflow-runs/<run_id>/`, with
//! no daemon dependency: create one per repo entry, list them, reap them
//! (sweeping leftovers into a commit first) and list the run directories the
//! startup sweep visits. Which repos, the reopen check and the run-status
//! lookup stay in otto-server (`workflow_engine.rs`).

use std::path::{Path, PathBuf};

use crate::run_refs::{release_pr_heads, resolve_wf_base};

/// Create the run's linked worktree of `repo_path` on a fresh
/// `otto-wf/<run_id>` branch at `base` (the run-start sync's `synced` ref
/// first, else [`resolve_wf_base`]). Returns the worktree path, or the message
/// that fails the run: an entry whose base cannot be resolved or whose
/// worktree cannot be created would otherwise run in the user's own checkout.
///
/// SAFETY (load-bearing): this only ever CREATES a new linked worktree + a
/// fresh branch under the data dir. It never checks out, resets, or switches
/// branches in the user's own repo, and never deletes.
pub async fn provision(
    data_dir: &Path,
    run_id: &str,
    repo_path: &str,
    repo_name: &str,
    base: &str,
    synced: Option<&str>,
) -> Result<String, String> {
    let git = otto_git::LocalGit::new(repo_path);
    let synced = match synced {
        Some(r) => git.rev_parse(r).await.ok(),
        None => None,
    };
    let resolved = match synced {
        Some(c) => Some(c),
        None => resolve_wf_base(&git, base).await,
    };
    let Some(base_commit) = resolved else {
        tracing::warn!(
            "wf worktree: base '{base}' not found in {repo_path} (locally or as origin/{base}) — refusing to run in the user's checkout"
        );
        return Err(format!(
            "base branch '{base}' was not found in {repo_name} (neither locally nor as origin/{base}) — fetch it or fix the run's base"
        ));
    };
    let wt_path = data_dir
        .join("workflow-runs")
        .join(run_id)
        .join(crate::context::slug(repo_name));
    let wt_str = wt_path.to_string_lossy().to_string();
    let branch = format!("otto-wf/{run_id}");
    match git
        .worktree_add_if_absent(&wt_str, &branch, &base_commit)
        .await
    {
        Ok(_) => {
            tracing::info!("wf worktree: {repo_name} @ {base} → {wt_str}");
            Ok(wt_str)
        }
        Err(err) => {
            tracing::warn!(
                "wf worktree provision failed for {repo_name} @ {base}: {err} — refusing to run in the user's checkout"
            );
            Err(format!(
                "could not create an isolated worktree of {repo_name} @ {base}: {err}"
            ))
        }
    }
}

/// The sub-directories of `dir` (a run's worktrees, or the run directories
/// under `workflow-runs/`); `None` when `dir` cannot be read.
pub async fn list_dirs(dir: &Path) -> Option<Vec<PathBuf>> {
    let dir = dir.to_path_buf();
    // Listing (and the per-entry `is_dir` stats) is sync fs work: blocking pool.
    #[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
    tokio::task::spawn_blocking(move || {
        std::fs::read_dir(&dir).ok().map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect::<Vec<_>>()
        })
    })
    .await
    .ok()
    .flatten()
}

/// Reap the run's `worktrees` (listed from `dir`) and, when safe, the matching
/// `otto-wf/<run_id>` branch in each owning repo; then `dir` itself.
///
/// Branch policy — never discard commits: the branch is deleted only when its
/// commits are reachable elsewhere (merged into HEAD) or pushed to origin;
/// otherwise the branch stays (cheap) and only the worktree directory goes
/// (`worktree_add_if_absent` re-attaches a surviving branch on a later run).
///
/// Work policy — never discard UNCOMMITTED work either: leftovers in a dirty
/// worktree are swept into a commit on the run branch BEFORE removal (2026-07-14:
/// a failed run's reap destroyed an implement step's uncommitted test suite).
/// If that sweep fails, the worktree directory is kept rather than destroyed.
pub async fn reap(data_dir: &Path, run_id: &str, dir: &Path, worktrees: Vec<PathBuf>) {
    release_pr_heads(data_dir, run_id).await;
    let branch = format!("otto-wf/{run_id}");
    let mut kept_any = false;
    for wt in worktrees {
        let wt_str = wt.to_string_lossy().to_string();
        // Owning repo root: the worktree's git-common-dir is `<root>/.git`.
        let wt_git = otto_git::LocalGit::new(&wt_str);
        // Not (or no longer) a readable git worktree: its content can't be
        // swept into a commit, so the directory cleanup below must not
        // bulldoze it either.
        let Ok(common) = wt_git
            .run(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .await
        else {
            tracing::warn!("wf reap: {wt_str} is not a readable git worktree — keeping it");
            kept_any = true;
            continue;
        };
        let Some(root) = Path::new(common.trim()).parent() else {
            kept_any = true;
            continue;
        };
        match wt_git
            .commit_all_if_dirty(&format!(
                "chore: preserve workflow leftovers (run {run_id})"
            ))
            .await
        {
            Ok(Some(sha)) => tracing::info!(
                "wf reap: committed leftover changes in {wt_str} onto {branch} ({})",
                &sha[..sha.len().min(9)]
            ),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    "wf reap: could not commit leftovers in {wt_str}: {e} — keeping the worktree"
                );
                kept_any = true;
                continue;
            }
        }
        let repo_git = otto_git::LocalGit::new(root);
        let _ = repo_git.worktree_remove(&wt_str).await;
        // Safe-delete the run branch: merged into HEAD OR pushed to origin.
        let merged = repo_git
            .run(&["branch", "--merged", "HEAD", "--list", &branch])
            .await
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false);
        // Quiet probe: a deleted remote ref is the common case, not an error.
        if merged || repo_git.origin_branch_exists(&branch).await {
            let _ = repo_git.run(&["branch", "-D", &branch]).await;
        } else {
            tracing::debug!(
                "wf reap: keeping branch {branch} in {} (unmerged, unpushed commits)",
                root.display()
            );
        }
    }
    // Only when every worktree was safely handled — a kept (unsweepable)
    // worktree must not be bulldozed by the directory cleanup.
    if !kept_any {
        let _ = tokio::fs::remove_dir_all(dir).await;
    }
}
