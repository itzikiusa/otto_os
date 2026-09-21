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

/// Directory holding a loop's worktree. Ids are daemon-generated ULIDs, but
/// re-validate before joining under the data dir so a hostile id fails closed
/// to a never-existing name instead of escaping it.
fn worktree_dir(ctx: &ServerCtx, loop_id: &str) -> std::path::PathBuf {
    let id = otto_core::paths::safe_component(loop_id).unwrap_or("invalid");
    ctx.data_dir.join("goal-loops").join(id).join("work")
}

/// Ensure the loop has an isolated worktree + branch, returning
/// `(branch, worktree_path, base_commit)`.
///
/// Idempotent across start/resume: when the loop already recorded a worktree and
/// it still exists on disk, it is reused as-is (the loop's prior commits are
/// preserved). Otherwise a fresh worktree is created from the repo's current
/// HEAD. A pre-existing path with no record is an error (we never reuse foreign
/// or stale trees, and never force-reset a branch).
pub async fn provision_worktree(
    ctx: &ServerCtx,
    loop_: &GoalLoop,
) -> Result<(String, String, String)> {
    if loop_.config.mode == "research" {
        let path = worktree_dir(ctx, &loop_.id);
        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|e| Error::Internal(e.to_string()))?;
        return Ok((
            String::new(),
            path.to_string_lossy().into_owned(),
            String::new(),
        ));
    }
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

/// Capture the actual working contents, including untracked files. No staging,
/// temporary commits or mutation of the user's index is needed for evidence.
pub async fn capture_work(cwd: &str, base: Option<&str>) -> Result<String> {
    async fn git(cwd: &str, args: &[&str]) -> Result<std::process::Output> {
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| Error::Internal(format!("capture goal work: {e}")))
    }
    let base = base.filter(|b| !b.is_empty()).unwrap_or("HEAD");
    if base.starts_with('-') {
        return Err(Error::Invalid("invalid goal base".into()));
    }
    let tracked = git(cwd, &["diff", "--no-ext-diff", "--no-color", base, "--"]).await?;
    let mut text = if tracked.status.success() {
        String::from_utf8_lossy(&tracked.stdout).into_owned()
    } else {
        // Unborn repositories in research/fixtures have no HEAD.
        let staged = git(
            cwd,
            &["diff", "--no-ext-diff", "--no-color", "--cached", "--"],
        )
        .await?;
        let unstaged = git(cwd, &["diff", "--no-ext-diff", "--no-color", "--"]).await?;
        format!(
            "{}{}",
            String::from_utf8_lossy(&staged.stdout),
            String::from_utf8_lossy(&unstaged.stdout)
        )
    };
    let untracked = git(cwd, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    for path in untracked
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
    {
        let path = String::from_utf8_lossy(path);
        // -- prevents option-like filenames being interpreted as git flags.
        let patch = git(
            cwd,
            &[
                "diff",
                "--no-ext-diff",
                "--no-color",
                "--no-index",
                "--",
                "/dev/null",
                &path,
            ],
        )
        .await?;
        text.push_str(&String::from_utf8_lossy(&patch.stdout));
    }
    Ok(text)
}

#[cfg(test)]
mod evidence_tests {
    #[tokio::test]
    async fn captures_staged_and_untracked_without_commits() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        std::fs::write(dir.path().join("tracked.txt"), "staged\n").unwrap();
        assert!(git(&["add", "tracked.txt"]).status.success());
        std::fs::write(dir.path().join("untracked.txt"), "new evidence\n").unwrap();
        let diff = super::capture_work(dir.path().to_str().unwrap(), None)
            .await
            .unwrap();
        assert!(diff.contains("staged"));
        assert!(diff.contains("new evidence"));
        assert!(dir.path().join("untracked.txt").exists());
    }
}
