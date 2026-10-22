//! Merge a swarm agent's task branch into the project's DEDICATED integration
//! branch — inside the integration worktree, NEVER the user's primary checkout
//! (AGENTS.md "Do NOT damage user work"; review M3). Concurrent merges into the
//! same integration branch are serialized by a per-branch lock (review M4).
//! Conflicts are never auto-resolved — they're returned for the leader to turn
//! into a fix task.

use std::sync::Arc;

use otto_core::api::LocalMergeStrategy;
use otto_state::{Swarm, SwarmProject};

use crate::runtime::host::SwarmRt;

#[derive(Debug, Clone)]
pub struct MergeOutcome {
    /// "merged" | "conflicts" | "up_to_date" | "error"
    pub status: String,
    pub conflicted_files: Vec<String>,
    pub integration_branch: String,
    pub note: Option<String>,
}

impl MergeOutcome {
    fn err(integration_branch: String, note: impl Into<String>) -> Self {
        Self {
            status: "error".into(),
            conflicted_files: Vec::new(),
            integration_branch,
            note: Some(note.into()),
        }
    }
}

/// Per-(repo::branch) async lock so two task merges into the same integration
/// branch never run concurrently.
static LOCKS: otto_core::keyed_lock::KeyedLocks<String> = otto_core::keyed_lock::KeyedLocks::new();

fn branch_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    LOCKS.handle(key)
}

/// Merge `agent_branch` into the project's integration branch, in the integration
/// worktree. Returns the outcome (merged / conflicts / up_to_date / error).
pub async fn merge_task_branch(
    ctx: &SwarmRt,
    swarm: &Swarm,
    project: &SwarmProject,
    agent_branch: &str,
) -> MergeOutcome {
    let (int_wt, integration_branch) =
        match crate::runtime::workspace::ensure_integration_worktree(ctx, swarm, project).await {
            Ok(v) => v,
            Err(e) => {
                return MergeOutcome::err(
                    crate::runtime::workspace::integration_branch_name(swarm, project),
                    format!("integration worktree: {e}"),
                )
            }
        };

    let repo_key = format!(
        "{}::{}",
        project.repo_path.clone().unwrap_or_default(),
        integration_branch
    );
    let lock = branch_lock(&repo_key);
    let _guard = lock.lock().await;

    let git = otto_git::LocalGit::new(&int_wt);

    // Pre-flight: don't try to merge a branch already contained in the target.
    match git.merge_preview(agent_branch, &integration_branch).await {
        Ok(p) if p.up_to_date => {
            return MergeOutcome {
                status: "up_to_date".into(),
                conflicted_files: Vec::new(),
                integration_branch,
                note: None,
            };
        }
        _ => {}
    }

    // The integration worktree is Otto-owned and kept clean, so auto_stash:false.
    match merge_or_back_out(&git, agent_branch, &integration_branch).await {
        Ok(res) => {
            if res.status == "merged" {
                // The branch's work is integrated — stop tracking its files for the
                // shared-files detector.
                crate::runtime::run::forget_branch_files(&swarm.id, agent_branch);
            }
            MergeOutcome {
                status: res.status,
                conflicted_files: res.conflicted_files,
                integration_branch,
                note: res.note,
            }
        }
        Err(e) => MergeOutcome::err(integration_branch, format!("merge: {e}")),
    }
}

/// Merge `source` into `target` in the integration worktree; on conflicts,
/// back the merge out again. Nobody resolves conflicts HERE — the fix task
/// works in the dev's own worktree — so a merge left half-done made every
/// later merge into the integration branch fail with "a merge is already in
/// progress" (each finished task then blocked) until someone found the
/// hidden worktree by hand.
async fn merge_or_back_out(
    git: &otto_git::LocalGit,
    source: &str,
    target: &str,
) -> otto_core::Result<otto_core::api::MergeResult> {
    let res = git
        .merge_branch(source, target, LocalMergeStrategy::MergeCommit, false)
        .await?;
    if res.status == "conflicts" {
        if let Err(e) = git.merge_abort().await {
            tracing::warn!("swarm merge: abort conflicted merge of {source} into {target}: {e}");
        }
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::disallowed_methods)] // test: blocking git setup
    fn run_git(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }

    fn commit(dir: &std::path::Path, file: &str, body: &str, msg: &str) {
        std::fs::write(dir.join(file), body).unwrap();
        run_git(dir, &["add", "."]);
        run_git(dir, &["commit", "-qm", msg]);
    }

    /// A conflicted merge is backed out, so the next task's merge still lands.
    #[tokio::test]
    async fn a_conflict_does_not_wedge_later_merges() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = tmp.path();
        run_git(d, &["init", "-q", "-b", "integration"]);
        run_git(d, &["config", "user.email", "t@example.com"]);
        run_git(d, &["config", "user.name", "t"]);
        commit(d, "a.txt", "base\n", "base");
        run_git(d, &["checkout", "-qb", "conflicting"]);
        commit(d, "a.txt", "theirs\n", "theirs");
        run_git(d, &["checkout", "-q", "integration"]);
        run_git(d, &["checkout", "-qb", "clean"]);
        commit(d, "b.txt", "new\n", "clean");
        run_git(d, &["checkout", "-q", "integration"]);
        commit(d, "a.txt", "ours\n", "ours");

        let lg = otto_git::LocalGit::new(d);
        let first = merge_or_back_out(&lg, "conflicting", "integration")
            .await
            .unwrap();
        assert_eq!(first.status, "conflicts");
        assert_eq!(first.conflicted_files, vec!["a.txt".to_string()]);
        let second = merge_or_back_out(&lg, "clean", "integration")
            .await
            .unwrap();
        assert_eq!(second.status, "merged");
    }
}
