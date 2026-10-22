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
    let branch = format!("otto-run/{}", run.id);
    let path = ctx.data_dir.join("otto-runs").join(&run.id).join("work");
    let path_str = path.to_string_lossy().to_string();
    let durable = ctx.runs.get(&run.id).await?;
    let base_commit = match durable.base_commit.filter(|b| !b.is_empty()) {
        Some(base) => base,
        None => {
            // A legacy interrupted provision has no trustworthy origin. Do not
            // substitute today's checkout HEAD for an existing branch's base.
            if git.branch_exists(&branch).await || git.worktree_exists(&path_str).await {
                return Err(otto_core::Error::Conflict(
                    "existing run worktree has no recorded base — relaunch the run".into(),
                ));
            }
            let base = git
                .rev_parse(run.base_branch.as_deref().unwrap_or("HEAD"))
                .await?;
            ctx.runs
                .set_fields(
                    &run.id,
                    &otto_state::runs::RunPatch {
                        base_commit: Some(base.clone()),
                        branch: Some(branch.clone()),
                        worktree_path: Some(path_str.clone()),
                        ..Default::default()
                    },
                )
                .await?;
            base
        }
    };
    add_worktree_guarded(&run.id, git, &path_str, &branch, &base_commit).await?;
    Ok((branch, path_str, base_commit))
}

/// Runs whose `git worktree add` is in flight. The add is a `LocalWrite`, which
/// git runs detached: a Stop that drops the provisioning stage stops only the
/// WAIT, and the checkout runs on under git's "initializing" lock. Cancel
/// awaits this (bounded, [`await_provisioning`]) before it removes the tree,
/// or the removal lost to the lock and the finished checkout leaked.
static PROVISIONING: otto_core::keyed_lock::KeyedLocks<String> =
    otto_core::keyed_lock::KeyedLocks::new();

/// [`LocalGit::worktree_add_if_absent`] in its own task that holds the run's
/// provisioning guard until git is done, even when the caller is dropped.
async fn add_worktree_guarded(
    run_id: &str,
    git: LocalGit,
    path: &str,
    branch: &str,
    base: &str,
) -> Result<bool> {
    let guard = PROVISIONING.lock(run_id).await;
    let (path, branch, base) = (path.to_string(), branch.to_string(), base.to_string());
    tokio::spawn(async move {
        let _guard = guard;
        git.worktree_add_if_absent(&path, &branch, &base).await
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("worktree provisioning task: {e}")))?
}

/// Wait (up to `budget`) for an in-flight provisioning of `run_id` to finish.
/// False when it is still running at the deadline.
pub(crate) async fn await_provisioning(run_id: &str, budget: std::time::Duration) -> bool {
    tokio::time::timeout(budget, PROVISIONING.lock(run_id))
        .await
        .is_ok()
}

/// Best-effort worktree removal (keeps the branch so its commits survive for the
/// diff / PR draft). Called on cancel/delete and after completion.
pub(crate) async fn remove_worktree(_ctx: &ServerCtx, run: &OttoRun) {
    if let (Some(repo_path), Some(wt)) = (run.repo_path.as_deref(), run.worktree_path.as_deref()) {
        let git = LocalGit::new(repo_path);
        let _ = git.worktree_remove(wt).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// concurrency-async-3-02: Stop during a slow `git worktree add` — the
    /// stage future is dropped mid-checkout, cancel waits for the detached
    /// add, and the removal then leaves nothing registered or on disk.
    #[tokio::test]
    async fn cancel_during_a_slow_worktree_add_leaves_no_worktree() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        // Synchronous fixture setup; never called by production handlers.
        #[allow(clippy::disallowed_methods)]
        let sh = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        sh(&["init", "-q"]);
        sh(&["config", "user.name", "Fixture"]);
        sh(&["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join("a.txt"), "x\n").unwrap();
        sh(&["add", "a.txt"]);
        sh(&["commit", "-qm", "init"]);
        // A git whose `worktree add` takes a while (a large checkout).
        let shim = tmp.path().join("slow-git");
        std::fs::write(
            &shim,
            "#!/bin/sh\nfor a in \"$@\"; do [ \"$a\" = add ] && sleep 1; done\nexec git \"$@\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let wt = tmp.path().join("otto-runs/run-x/work");
        let wt_str = wt.to_string_lossy().to_string();
        let slow = LocalGit::new(&repo).with_git_bin(&shim);
        let base = LocalGit::new(&repo).rev_parse("HEAD").await.unwrap();
        // The stage is dropped (Stop) while the add is still running.
        let stage = add_worktree_guarded("run-x", slow, &wt_str, "otto-run/run-x", &base);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), stage)
                .await
                .is_err(),
            "the slow add is still in flight"
        );
        assert!(await_provisioning("run-x", std::time::Duration::from_secs(20)).await);
        LocalGit::new(&repo).worktree_remove(&wt_str).await.unwrap();
        assert!(
            !wt.exists(),
            "the late checkout must not survive the cancel"
        );
        assert!(!LocalGit::new(&repo).worktree_exists(&wt_str).await);
    }
}
