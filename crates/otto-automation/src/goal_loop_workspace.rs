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
    let base = match git
        .rev_parse(loop_.base_commit.as_deref().unwrap_or("HEAD"))
        .await
    {
        Ok(sha) => sha,
        Err(e) if loop_.base_commit.is_some() => {
            return Err(Error::Invalid(format!(
                "Reviewed commit is unavailable: {e}"
            )))
        }
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
    async fn git(
        cwd: &str,
        prefix: &[String],
        args: &[&str],
        cap: usize,
    ) -> Result<(bool, Vec<u8>)> {
        use tokio::io::AsyncReadExt;
        let mut cmd = otto_git::hardened_command();
        cmd.args(prefix)
            .args(args)
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
    // Worktree `.gitattributes` is not a source of programs. `--attr-source`
    // reads attributes from the empty tree, and every configured clean/smudge
    // filter command is blanked so `$GIT_DIR/info/attributes` cannot name one
    // that still runs. `filter.use` is not a git key.
    let safe = diff_safety_args(cwd).await;
    let left = |text: &str| CAPTURE_CAP.saturating_sub(text.len());
    let (ok, tracked) = git(
        cwd,
        &safe,
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
            &safe,
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
            &safe,
            &["diff", "--no-ext-diff", "--no-textconv", "--no-color", "--"],
            left(&t),
        )
        .await?;
        t.push_str(&String::from_utf8_lossy(&unstaged));
        t
    };
    let (_, untracked) = git(
        cwd,
        &safe,
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
            &safe,
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

/// Inspect a loop worktree's current `git diff --stat`, capped unified patch,
/// and untracked files for `GET /api/v1/goal-loops/{id}/diff`. Returns
/// `available: false` cleanly when the loop has no git worktree yet (e.g.
/// draft or research mode).
pub async fn worktree_diff(
    loop_: &GoalLoop,
    max_patch_bytes: usize,
) -> Result<crate::goal_loop_steering::GoalLoopDiff> {
    let empty = || crate::goal_loop_steering::GoalLoopDiff {
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
    Ok(crate::goal_loop_steering::GoalLoopDiff {
        loop_id: loop_.id.clone(),
        available: true,
        branch: loop_.branch.clone(),
        stat: String::from_utf8_lossy(&stat_bytes).trim_end().to_string(),
        patch: String::from_utf8_lossy(&patch_bytes).into_owned(),
        truncated,
        untracked,
    })
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

    /// A clean filter named from worktree attributes or info/attributes must
    /// not run while the capture reads the working tree. The diff still shows
    /// the bytes on disk.
    #[cfg(unix)]
    #[tokio::test]
    async fn capture_does_not_run_a_clean_filter() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("filter-ran");
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
        assert!(git(&["config", "user.name", "t"]).status.success());
        std::fs::write(dir.path().join("f.txt"), "hi\n").unwrap();
        assert!(git(&["add", "f.txt"]).status.success());
        assert!(git(&["commit", "-qm", "init"]).status.success());
        let script = dir.path().join("filt.sh");
        std::fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut mode = std::fs::metadata(&script).unwrap().permissions();
        mode.set_mode(0o755);
        std::fs::set_permissions(&script, mode).unwrap();
        assert!(
            git(&["config", "filter.secret.clean", script.to_str().unwrap()])
                .status
                .success()
        );
        std::fs::write(dir.path().join(".gitattributes"), "* filter=secret\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".git/info")).unwrap();
        std::fs::write(dir.path().join(".git/info/attributes"), "* filter=secret\n").unwrap();
        std::fs::write(dir.path().join("f.txt"), "secret\n").unwrap();
        let diff = super::capture_work(dir.path().to_str().unwrap(), None)
            .await
            .unwrap();
        assert!(!marker.exists(), "clean filter ran during capture");
        assert!(diff.contains("secret"));
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
}
