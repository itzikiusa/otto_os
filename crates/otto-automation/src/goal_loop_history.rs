//! Older goal loops (goal.md §5): what their read-only history page and the
//! boot sweep still need after the controller was folded into Workflows —
//! the kept worktree's diff, the steering notes a loop had queued, and the
//! clean-up of executor sessions that survived a restart (DR-108).

use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::domain::{GoalLoop, GoalLoopStatus};
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use crate::AutomationCtx;

/// Default cap on unified diff bytes returned by `GET /goal-loops/{id}/diff`.
pub const DEFAULT_DIFF_PATCH_CAP: usize = 256 * 1024;

/// Response body for `GET /api/v1/goal-loops/{id}/diff`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct GoalLoopDiff {
    pub loop_id: String,
    pub available: bool,
    pub branch: Option<String>,
    pub stat: String,
    pub patch: String,
    pub truncated: bool,
    pub untracked: Vec<String>,
}

fn steering_path(data_dir: &Path, loop_id: &str) -> Result<PathBuf> {
    let safe = otto_core::paths::safe_component(loop_id)
        .ok_or_else(|| Error::Invalid("invalid goal loop id".into()))?;
    Ok(data_dir.join("goal-loops").join(safe).join("steering.json"))
}

/// Read currently queued steering notes without consuming them.
pub async fn peek_notes(data_dir: &Path, loop_id: &str) -> Vec<String> {
    let Ok(path) = steering_path(data_dir, loop_id) else {
        return Vec::new();
    };
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<String>>(&bytes).unwrap_or_default()
}

/// Per-git budget: a capture never hangs the loop (a FIFO, a stuck NFS path).
const CAPTURE_GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Git's well-known empty tree. `--attr-source` reads attributes from it, so a
/// worktree `.gitattributes` is not consulted.
const EMPTY_ATTR_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

/// Args that keep `git diff` from running a clean filter. The empty attribute
/// tree drops worktree `.gitattributes`. Blanking each configured
/// `filter.<name>.{clean,smudge,process}` drops a driver named from
/// `info/attributes` or the global file. Driver names that are not a config
/// token are skipped rather than interpolated into `-c`.
async fn diff_safety_args(cwd: &str) -> Vec<String> {
    let mut cmd = otto_git::hardened_command();
    cmd.args(["config", "--get-regexp", "^filter\\."])
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let listed = cmd.output().await.ok().map(|out| out.stdout);
    let mut names = Vec::new();
    if let Some(stdout) = listed {
        for line in String::from_utf8_lossy(&stdout).lines() {
            let key = line.split_whitespace().next().unwrap_or("");
            let Some(rest) = key.strip_prefix("filter.") else {
                continue;
            };
            let name = match rest.rsplit_once('.') {
                Some((name, "clean" | "smudge" | "process" | "required")) => name,
                _ => rest,
            };
            let token = name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                && !name.is_empty();
            if token {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    let mut args = vec![format!("--attr-source={EMPTY_ATTR_TREE}")];
    for name in names {
        for kind in ["clean", "smudge", "process"] {
            args.push("-c".into());
            args.push(format!("filter.{name}.{kind}="));
        }
    }
    args
}

/// Inspect a loop worktree's current `git diff --stat`, capped unified patch,
/// and untracked files for `GET /api/v1/goal-loops/{id}/diff`. Returns
/// `available: false` cleanly when the loop has no git worktree yet (e.g.
/// draft or research mode).
pub async fn worktree_diff(loop_: &GoalLoop, max_patch_bytes: usize) -> Result<GoalLoopDiff> {
    let empty = || GoalLoopDiff {
        loop_id: loop_.id.clone(),
        available: false,
        branch: loop_.branch.clone(),
        stat: String::new(),
        patch: String::new(),
        truncated: false,
        untracked: Vec::new(),
    };
    if loop_.config.mode == "research" {
        return Ok(empty());
    }
    let Some(wt) = loop_.worktree_path.as_deref().filter(|s| !s.is_empty()) else {
        return Ok(empty());
    };
    if !tokio::fs::try_exists(wt).await.unwrap_or(false) {
        return Ok(empty());
    }
    let base = loop_
        .base_commit
        .as_deref()
        .filter(|b| !b.is_empty() && !b.starts_with('-'))
        .unwrap_or("HEAD");
    let safe = diff_safety_args(wt).await;

    async fn run_git(cwd: &str, prefix: &[String], args: &[&str], cap: usize) -> (Vec<u8>, bool) {
        use tokio::io::AsyncReadExt;
        let mut cmd = otto_git::hardened_command();
        cmd.args(prefix)
            .args(args)
            .current_dir(cwd)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let fut = async {
            let mut child = cmd.spawn().ok()?;
            let mut out = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                let _ = stdout
                    .take((cap as u64).saturating_add(1))
                    .read_to_end(&mut out)
                    .await;
            }
            let cut = out.len() > cap;
            if cut {
                out.truncate(cap);
                let _ = child.start_kill();
            }
            let _ = child.wait().await;
            Some((out, cut))
        };
        tokio::time::timeout(CAPTURE_GIT_TIMEOUT, fut)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| (Vec::new(), false))
    }

    let (stat_bytes, _) = run_git(
        wt,
        &safe,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--stat",
            base,
            "--",
        ],
        64 * 1024,
    )
    .await;
    let (patch_bytes, truncated) = run_git(
        wt,
        &safe,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            base,
            "--",
        ],
        max_patch_bytes,
    )
    .await;
    let (untracked_bytes, _) = run_git(
        wt,
        &[],
        &["ls-files", "--others", "--exclude-standard"],
        64 * 1024,
    )
    .await;
    let untracked = String::from_utf8_lossy(&untracked_bytes)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(500)
        .map(str::to_string)
        .collect();
    Ok(GoalLoopDiff {
        loop_id: loop_.id.clone(),
        available: true,
        branch: loop_.branch.clone(),
        stat: String::from_utf8_lossy(&stat_bytes).trim_end().to_string(),
        patch: String::from_utf8_lossy(&patch_bytes).into_owned(),
        truncated,
        untracked,
    })
}

const CLEANUP_GRACE: Duration = Duration::from_secs(10);

/// List + kill all executor sessions tagged for this loop.
pub async fn cleanup_executor_sessions(ctx: &impl AutomationCtx, ws_id: &Id, loop_id: &Id) {
    let _ = tokio::time::timeout(
        CLEANUP_GRACE,
        cleanup_executor_sessions_inner(ctx, ws_id, loop_id),
    )
    .await;
}

async fn cleanup_executor_sessions_inner(ctx: &impl AutomationCtx, ws_id: &Id, loop_id: &Id) {
    // `meta.source` narrows in SQL (indexed); only the loop id is checked here
    // — the workspace's whole session history is never decoded.
    let scope = otto_state::SessionScope {
        workspace_id: ws_id.clone(),
        owner: None,
    };
    let filter = otto_state::SessionListFilter {
        source: Some("goal_loop".into()),
        ..Default::default()
    };
    let sessions = match ctx.manager().list_filtered(&[scope], &filter).await {
        Ok(s) => s,
        Err(_) => return,
    };
    for s in sessions {
        let is_ours = s.meta.get("loop_id").and_then(|v| v.as_str()) == Some(loop_id.as_str());
        if is_ours {
            let _ = ctx.manager().kill_session(&s.id).await;
        }
    }
}

/// The `error` boot recovery stores on a loop it paused. The UI recognises a
/// restart pause by this text on a Paused loop (a person's Pause never sets
/// an error, and Resume clears it).
pub const RESTART_PAUSE_NOTE: &str =
    "Interrupted by a daemon restart — continue it as a goal; its work is kept.";

/// The notice streak kind. Separate from `goal_loop` (blocked / exhausted /
/// …) so a restart notice neither swallows nor is swallowed by those.
const RESTART_NOTICE_KIND: &str = "goal_loop_restart";

/// Kill the swept loops' surviving executor sessions (DR-108: a held
/// `pty-holder` executor outlives the restart); for each loop the restart
/// paused, post one notice that opens it.
/// `swept` is what `fail_running` returned (their pre-sweep rows).
pub async fn announce_restart_pause(ctx: &impl AutomationCtx, swept: &[GoalLoop]) {
    for l in swept {
        cleanup_executor_sessions(ctx, &l.workspace_id, &l.id).await;
        if !paused_by_restart(l.status) {
            continue;
        }
        let (title, body) = restart_notice(l);
        ctx.run_notice(
            RESTART_NOTICE_KIND,
            &l.id,
            Some((title, body)),
            format!("loops/{}", l.id),
            Some(l.workspace_id.clone()),
            Some(l.created_by.clone()),
        )
        .await;
    }
}

/// The sweep keeps a Blocked loop Blocked (its question already raised a
/// notice); everything else it returns was running and is now Paused.
fn paused_by_restart(pre_sweep: GoalLoopStatus) -> bool {
    pre_sweep != GoalLoopStatus::Blocked
}

fn restart_notice(l: &GoalLoop) -> (String, String) {
    (
        format!("Goal loop “{}” was paused by the update", l.name),
        format!(
            "Otto restarted while this loop was running (iteration {}, {}% done). \
             Its work is kept: open it and continue it as a goal.",
            l.current_iteration, l.progress_pct
        ),
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process is fine
mod tests {
    use super::*;
    use chrono::Utc;

    #[tokio::test]
    async fn worktree_diff_returns_stat_patch_untracked_and_respects_cap() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        assert!(git(&["config", "user.email", "t@example.invalid"])
            .status
            .success());
        assert!(git(&["config", "user.name", "Test"]).status.success());
        std::fs::write(dir.path().join("a.txt"), "hello\n").unwrap();
        assert!(git(&["add", "a.txt"]).status.success());
        assert!(git(&["commit", "-q", "-m", "init"]).status.success());
        std::fs::write(dir.path().join("a.txt"), "hello\nworld\n").unwrap();
        std::fs::write(dir.path().join("new.txt"), "untracked\n").unwrap();

        let now = chrono::Utc::now();
        let loop_ = otto_core::domain::GoalLoop {
            id: "gl-1".into(),
            workspace_id: "ws-1".into(),
            name: "test".into(),
            repo_path: dir.path().to_string_lossy().into_owned(),
            definition: otto_core::domain::GoalLoopDefinition {
                title: "g".into(),
                summary: String::new(),
                objectives: vec![],
                acceptance_criteria: vec![],
                constraints: vec![],
                out_of_scope: vec![],
                success_signal: String::new(),
            },
            limits: otto_core::domain::GoalLoopLimits::default(),
            config: otto_core::domain::GoalLoopConfig::default(),
            status: otto_core::domain::GoalLoopStatus::Running,
            phase: otto_core::domain::GoalLoopPhase::Executing,
            iterations_started: 1,
            current_iteration: 1,
            progress_pct: 0,
            context_digest: String::new(),
            ledger: Default::default(),
            branch: Some("goal-loop/gl-1".into()),
            worktree_path: Some(dir.path().to_string_lossy().into_owned()),
            base_commit: Some("HEAD".into()),
            summary: None,
            error: None,
            run_started_at: Some(now),
            elapsed_secs: 0,
            cost_usd: 0.0,
            created_by: "u-1".into(),
            created_at: now,
            updated_at: now,
            finished_at: None,
        };
        let d = super::worktree_diff(&loop_, 64 * 1024).await.unwrap();
        assert!(d.available);
        assert!(d.stat.contains("a.txt"));
        assert!(d.patch.contains("+world"));
        assert!(!d.truncated);
        assert_eq!(d.untracked, vec!["new.txt"]);

        let capped = super::worktree_diff(&loop_, 16).await.unwrap();
        assert!(capped.truncated);
        assert!(capped.patch.len() <= 16);
    }

    fn swept(status: &str) -> GoalLoop {
        serde_json::from_value(serde_json::json!({
            "id":"g", "workspace_id":"w", "name":"Nightly fix", "repo_path":"/tmp/isolated",
            "definition":{"title":"Goal","acceptance_criteria":[]},
            "config":otto_core::domain::GoalLoopConfig::default(),
            "limits":otto_core::domain::GoalLoopLimits::default(),
            "status":status, "phase":"executing", "iterations_started":2, "current_iteration":2,
            "progress_pct":40, "elapsed_secs":0, "cost_usd":0, "created_by":"u",
            "created_at":Utc::now(), "updated_at":Utc::now()
        }))
        .unwrap()
    }

    #[test]
    fn only_loops_that_were_running_are_announced_as_paused_by_restart() {
        assert!(paused_by_restart(swept("running").status));
        assert!(paused_by_restart(swept("paused").status));
        assert!(!paused_by_restart(swept("blocked").status));
    }

    #[test]
    fn the_restart_notice_names_the_loop_and_says_how_to_continue() {
        let (title, body) = restart_notice(&swept("running"));
        assert!(title.contains("Nightly fix"), "{title}");
        assert!(title.contains("paused by the update"), "{title}");
        assert!(body.contains("iteration 2, 40% done"), "{body}");
        assert!(body.contains("continue it as a goal"), "{body}");
        assert!(RESTART_PAUSE_NOTE.contains("daemon restart"));
    }
}
