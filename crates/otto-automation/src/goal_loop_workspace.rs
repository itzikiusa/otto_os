//! Isolated git worktree provisioning for Goal Loops.
//!
//! Every loop runs on its own branch `goal-loop/<id>` in a dedicated worktree
//! under the daemon data dir — NEVER the user's checkout. Branch names are
//! ULID-unique, so provisioning is a FRESH create (not the destructive
//! `-B --force` reuse the swarm uses for multi-turn agents): if the path already
//! exists and we have no record of it, we fail loudly rather than clobber.

use otto_core::domain::GoalLoop;
use otto_core::{Error, Result};

use crate::AutomationCtx;

/// Directory holding a loop's worktree. Ids are daemon-generated ULIDs, but
/// re-validate before joining under the data dir so a hostile id fails closed
/// to a never-existing name instead of escaping it.
fn worktree_dir(ctx: &impl AutomationCtx, loop_id: &str) -> std::path::PathBuf {
    let id = otto_core::paths::safe_component(loop_id).unwrap_or("invalid");
    ctx.data_dir().join("goal-loops").join(id).join("work")
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
    ctx: &impl AutomationCtx,
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

/// Ceiling on the evidence one capture keeps (the whole patch text). An
/// agent's tree can hold a multi-GB generated file; past this the capture is
/// cut and says so instead of growing the daemon.
const CAPTURE_CAP: usize = 8 * 1024 * 1024;
/// Per-git budget: a capture never hangs the loop (a FIFO, a stuck NFS path).
const CAPTURE_GIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Capture the actual working contents, including untracked files. No staging,
/// temporary commits or mutation of the user's index is needed for evidence.
///
/// Every git here is the hardened one (`otto_git::hardened_command`: repo
/// config can't run fsmonitor / hooks), reads at most what is left of
/// [`CAPTURE_CAP`] (then is killed), and is bounded by
/// [`CAPTURE_GIT_TIMEOUT`]. Untracked entries that aren't regular files
/// (FIFOs, sockets, devices — `diff --no-index` would block reading one) are
/// skipped.
pub async fn capture_work(cwd: &str, base: Option<&str>) -> Result<String> {
    /// `(succeeded-or-cut, stdout ≤ cap)`.
    async fn git(cwd: &str, args: &[&str], cap: usize) -> Result<(bool, Vec<u8>)> {
        use tokio::io::AsyncReadExt;
        let mut cmd = otto_git::hardened_command();
        cmd.args(args)
            .current_dir(cwd)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let run = async {
            let mut child = cmd.spawn()?;
            let mut out = Vec::new();
            if let Some(stdout) = child.stdout.take() {
                stdout.take(cap as u64).read_to_end(&mut out).await?;
            }
            let cut = out.len() >= cap;
            if cut {
                let _ = child.start_kill();
            }
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((status.success() || cut, out))
        };
        match tokio::time::timeout(CAPTURE_GIT_TIMEOUT, run).await {
            Ok(r) => r.map_err(|e| Error::Internal(format!("capture goal work: {e}"))),
            Err(_) => Err(Error::Internal(format!(
                "capture goal work: git {} timed out",
                args.first().copied().unwrap_or("")
            ))),
        }
    }
    let base = base.filter(|b| !b.is_empty()).unwrap_or("HEAD");
    if base.starts_with('-') {
        return Err(Error::Invalid("invalid goal base".into()));
    }
    let left = |text: &str| CAPTURE_CAP.saturating_sub(text.len());
    let (ok, tracked) = git(
        cwd,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            base,
            "--",
        ],
        CAPTURE_CAP,
    )
    .await?;
    let mut text = if ok {
        String::from_utf8_lossy(&tracked).into_owned()
    } else {
        // Unborn repositories in research/fixtures have no HEAD.
        let (_, staged) = git(
            cwd,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--cached",
                "--",
            ],
            CAPTURE_CAP,
        )
        .await?;
        let mut t = String::from_utf8_lossy(&staged).into_owned();
        let (_, unstaged) = git(
            cwd,
            &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--"],
            left(&t),
        )
        .await?;
        t.push_str(&String::from_utf8_lossy(&unstaged));
        t
    };
    let (_, untracked) = git(
        cwd,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        CAPTURE_CAP,
    )
    .await?;
    for path in untracked.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        if left(&text) == 0 {
            break;
        }
        let path = String::from_utf8_lossy(path);
        let regular = std::fs::symlink_metadata(std::path::Path::new(cwd).join(&*path))
            .is_ok_and(|m| m.file_type().is_file());
        if !regular {
            continue;
        }
        // -- prevents option-like filenames being interpreted as git flags.
        let (_, patch) = git(
            cwd,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-index",
                "--",
                "/dev/null",
                &path,
            ],
            left(&text),
        )
        .await?;
        text.push_str(&String::from_utf8_lossy(&patch));
    }
    if text.len() >= CAPTURE_CAP {
        text.push_str("\n[otto: evidence capture truncated]\n");
    }
    Ok(text)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine
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

    /// S2-11: an untracked FIFO used to block `diff --no-index` forever, and
    /// the planted fsmonitor ran on `ls-files`; neither happens now.
    #[cfg(unix)]
    #[tokio::test]
    async fn capture_skips_fifos_and_never_runs_repo_fsmonitor() {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
        };
        assert!(git(&["init", "-q"]).status.success());
        let marker = dir.path().join("fsmonitor-ran");
        let hook = dir.path().join("fsm.sh");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(git(&["config", "core.fsmonitor", hook.to_str().unwrap()])
            .status
            .success());
        let fifo = dir.path().join("pipe");
        assert!(std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success());
        std::fs::write(dir.path().join("note.txt"), "evidence\n").unwrap();
        let diff = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            super::capture_work(dir.path().to_str().unwrap(), None),
        )
        .await
        .expect("capture must not block on a FIFO")
        .unwrap();
        assert!(diff.contains("evidence"));
        assert!(!marker.exists(), "capture ran the repo's fsmonitor");
    }
}
