//! Merge a swarm agent's task branch into the project's DEDICATED integration
//! branch — inside the integration worktree, NEVER the user's primary checkout
//! (AGENTS.md "Do NOT damage user work"; review M3). Concurrent merges into the
//! same integration branch are serialized by a per-branch lock (review M4).
//! Conflicts are never auto-resolved — they're returned for the leader to turn
//! into a fix task.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use otto_core::api::LocalMergeStrategy;
use otto_state::{Swarm, SwarmProject};

use crate::state::ServerCtx;

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
        Self { status: "error".into(), conflicted_files: Vec::new(), integration_branch, note: Some(note.into()) }
    }
}

/// Per-(repo::branch) async lock so two task merges into the same integration
/// branch never run concurrently.
static LOCKS: OnceLock<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    OnceLock::new();

fn branch_lock(key: &str) -> Arc<tokio::sync::Mutex<()>> {
    let map = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut g = map.lock().unwrap();
    g.entry(key.to_string())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}

/// Merge `agent_branch` into the project's integration branch, in the integration
/// worktree. Returns the outcome (merged / conflicts / up_to_date / error).
pub async fn merge_task_branch(
    ctx: &ServerCtx,
    swarm: &Swarm,
    project: &SwarmProject,
    agent_branch: &str,
) -> MergeOutcome {
    let (int_wt, integration_branch) =
        match crate::swarm_workspace::ensure_integration_worktree(ctx, swarm, project).await {
            Ok(v) => v,
            Err(e) => {
                return MergeOutcome::err(
                    crate::swarm_workspace::integration_branch_name(swarm, project),
                    format!("integration worktree: {e}"),
                )
            }
        };

    let repo_key = format!("{}::{}", project.repo_path.clone().unwrap_or_default(), integration_branch);
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
    match git
        .merge_branch(agent_branch, &integration_branch, LocalMergeStrategy::MergeCommit, false)
        .await
    {
        Ok(res) => {
            if res.status == "merged" {
                // The branch's work is integrated — stop tracking its files for the
                // shared-files detector.
                crate::swarm_run::forget_branch_files(&swarm.id, agent_branch);
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
