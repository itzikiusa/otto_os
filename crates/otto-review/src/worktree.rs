//! Isolated PR-head checkouts for review runs: a throwaway linked worktree of
//! the PR's source branch (or the host's PR head ref for fork PRs), torn down
//! when the run ends.

use otto_core::Id;

/// A throwaway checkout of a PR's head, created for a review run.
pub struct PrWorktree {
    pub path: String,
    pub branch: String,
}

/// Materialize a PR's head into an isolated linked worktree (best-effort):
/// `origin/<source>` first (same-repo PRs), then the host's PR head ref /
/// the head sha via [`otto_git::LocalGit::fetch_pr_head`] (fork PRs, whose
/// branch is not on `origin`). `None` ⇒ no checkout could be built and the
/// caller must not claim one. `suffix` keeps concurrent retries apart.
/// `git_token` is the repo's git credential (otto-server resolves it), used
/// only so the fetch can reach a private origin.
pub async fn materialize_pr_worktree(
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    pr_number: u64,
    source: Option<&str>,
    head_sha: Option<&str>,
    suffix: &str,
    git_token: Option<String>,
) -> Option<PrWorktree> {
    let git = otto_git::LocalGit::new(&repo.path);
    if let Err(e) = git.fetch(git_token.clone()).await {
        tracing::warn!(review = %review_id, "fetch before review failed: {e}; using cached refs");
    }
    let wt_path = std::env::temp_dir()
        .join(format!("otto-review-wt-{review_id}{suffix}"))
        .to_string_lossy()
        .into_owned();
    let wt_branch = format!("otto-review-{review_id}{suffix}");
    let _ = git.worktree_remove(&wt_path).await; // clear any stale tree
    if let Some(src) = source.filter(|s| !s.is_empty()) {
        let base = format!("origin/{src}");
        match git.worktree_add(&wt_path, &wt_branch, &base).await {
            Ok(()) => {
                tracing::info!(review = %review_id, "reviewing source branch `{src}` in isolated worktree");
                return Some(PrWorktree {
                    path: wt_path,
                    branch: wt_branch,
                });
            }
            Err(e) => {
                tracing::warn!(review = %review_id, "worktree of {base} failed: {e}; trying the PR head ref")
            }
        }
    }
    let head = match git.fetch_pr_head(pr_number, head_sha, git_token).await {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!(review = %review_id, "PR head unavailable: {e}; reviewing in repo path (partial)");
            return None;
        }
    };
    match git.worktree_add(&wt_path, &wt_branch, &head).await {
        Ok(()) => {
            tracing::info!(review = %review_id, "reviewing PR #{pr_number} head ({head}) in isolated worktree");
            Some(PrWorktree {
                path: wt_path,
                branch: wt_branch,
            })
        }
        Err(e) => {
            tracing::warn!(review = %review_id, "worktree of PR head {head} failed: {e}; reviewing in repo path (partial)");
            None
        }
    }
}

/// Tear down a [`PrWorktree`] (best-effort; leaves no litter in the user's
/// branch list). The branch is review-only, so force-delete is safe.
pub async fn teardown_pr_worktree(repo_path: &str, wt: PrWorktree) {
    let git = otto_git::LocalGit::new(repo_path);
    let _ = git.worktree_remove(&wt.path).await;
    let _ = git.delete_branch(&wt.branch, true).await;
}
