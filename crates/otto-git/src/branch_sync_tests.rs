//! Branch sync against a LOCAL bare "remote" — never the network. The shape of
//! the user report: a PR `x → y` where `y` (and `x`) exist only on the remote.

use super::*;
use std::path::{Path, PathBuf};

#[allow(clippy::disallowed_methods)] // fixture setup: blocking git is fine here
fn sh(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn identity(dir: &Path) {
    sh(dir, &["config", "user.email", "otto@test.local"]);
    sh(dir, &["config", "user.name", "Otto Test"]);
    sh(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit(dir: &Path, file: &str, msg: &str) -> String {
    std::fs::write(dir.join(file), msg).unwrap();
    sh(dir, &["add", "."]);
    sh(dir, &["commit", "-qm", msg]);
    sh(dir, &["rev-parse", "HEAD"])
}

/// `remote.git` (bare), `work` (the user's clone, only `main` fetched) and
/// `other` (a teammate's clone that pushes branches the work clone never saw).
struct Fx {
    _tmp: tempfile::TempDir,
    remote: PathBuf,
    work: PathBuf,
    other: PathBuf,
}

fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let remote = tmp.path().join("remote.git");
    let work = tmp.path().join("work");
    let other = tmp.path().join("other");
    sh(
        tmp.path(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    sh(
        tmp.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    identity(&work);
    commit(&work, "a.txt", "base");
    sh(&work, &["push", "-q", "-u", "origin", "main"]);
    sh(
        tmp.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    identity(&other);
    Fx {
        _tmp: tmp,
        remote,
        work,
        other,
    }
}

/// Push a new commit on `branch` from the teammate clone; returns its sha.
fn push_from_other(fx: &Fx, branch: &str, file: &str) -> String {
    let o = &fx.other;
    sh(o, &["fetch", "-q", "origin"]);
    let start = if sh(o, &["ls-remote", "--heads", "origin", branch]).is_empty() {
        "origin/main".to_string()
    } else {
        format!("origin/{branch}")
    };
    sh(o, &["checkout", "-q", "-B", branch, &start]);
    let sha = commit(o, file, &format!("{branch} {file}"));
    sh(o, &["push", "-q", "origin", branch]);
    sha
}

fn head_of(dir: &Path, r: &str) -> String {
    sh(dir, &["rev-parse", r])
}

#[tokio::test]
async fn pr_branches_only_on_the_remote_resolve_after_sync() {
    let fx = fixture();
    let y = push_from_other(&fx, "y", "y.txt");
    let x = push_from_other(&fx, "x", "x.txt");
    let git = LocalGit::new(&fx.work);
    // The bug: before any sync neither ref resolves in the user's clone.
    assert!(git.rev_parse("y").await.is_err());
    assert!(git.rev_parse("origin/x").await.is_err());

    let report = git
        .fetch_and_sync_branches(&["y", "x", " ", "y"], None)
        .await;
    assert_eq!(report.fetch_error, None);
    assert_eq!(report.branches.len(), 2, "blank + duplicate names skipped");
    for b in &report.branches {
        assert_eq!(b.outcome, BranchSyncOutcome::Created, "{b:?}");
    }
    assert_eq!(report.resolved("y"), Some("refs/heads/y"));
    assert_eq!(git.rev_parse("y").await.unwrap(), y);
    assert_eq!(git.rev_parse("x").await.unwrap(), x);
    assert_eq!(
        sh(&fx.work, &["rev-parse", "--abbrev-ref", "y@{upstream}"]),
        "origin/y",
        "created branch tracks the remote"
    );
}

#[tokio::test]
async fn stale_local_branch_not_checked_out_is_fast_forwarded() {
    let fx = fixture();
    push_from_other(&fx, "y", "y1.txt");
    sh(&fx.work, &["fetch", "-q", "origin"]);
    sh(&fx.work, &["branch", "--track", "y", "origin/y"]);
    let tip = push_from_other(&fx, "y", "y2.txt");
    let git = LocalGit::new(&fx.work);

    let report = git.fetch_and_sync_branches(&["origin/y"], None).await;
    assert_eq!(report.branches[0].outcome, BranchSyncOutcome::FastForwarded);
    assert_eq!(report.resolved("y"), Some("refs/heads/y"));
    assert_eq!(head_of(&fx.work, "refs/heads/y"), tip);
}

#[tokio::test]
async fn behind_branch_checked_out_is_not_moved() {
    let fx = fixture();
    let old = head_of(&fx.work, "main");
    let tip = push_from_other(&fx, "main", "m.txt");
    let git = LocalGit::new(&fx.work);

    let report = git.fetch_and_sync_branches(&["main"], None).await;
    let b = &report.branches[0];
    assert_eq!(b.outcome, BranchSyncOutcome::CheckedOutBehind);
    assert_eq!(b.resolved.as_deref(), Some("refs/remotes/origin/main"));
    // The user's checkout is untouched; the remote tip is still usable.
    assert_eq!(head_of(&fx.work, "main"), old);
    assert_eq!(head_of(&fx.work, "HEAD"), old);
    assert_eq!(head_of(&fx.work, "origin/main"), tip);
}

#[tokio::test]
async fn diverged_branch_keeps_local_commits_and_uses_the_remote_ref() {
    let fx = fixture();
    push_from_other(&fx, "y", "y1.txt");
    sh(&fx.work, &["fetch", "-q", "origin"]);
    sh(&fx.work, &["checkout", "-q", "-b", "y", "origin/y"]);
    let mine = commit(&fx.work, "local.txt", "local only");
    sh(&fx.work, &["checkout", "-q", "main"]);
    let theirs = push_from_other(&fx, "y", "y2.txt");
    let git = LocalGit::new(&fx.work);

    let report = git.fetch_and_sync_branches(&["y"], None).await;
    let b = &report.branches[0];
    assert_eq!(b.outcome, BranchSyncOutcome::Diverged);
    assert_eq!(b.resolved.as_deref(), Some("refs/remotes/origin/y"));
    assert_eq!(head_of(&fx.work, "refs/heads/y"), mine, "local commit kept");
    assert_eq!(head_of(&fx.work, "origin/y"), theirs);
}

#[tokio::test]
async fn local_ahead_and_local_only_branches_are_left_alone() {
    let fx = fixture();
    push_from_other(&fx, "y", "y1.txt");
    sh(&fx.work, &["fetch", "-q", "origin"]);
    sh(&fx.work, &["checkout", "-q", "-b", "y", "origin/y"]);
    let ahead = commit(&fx.work, "ahead.txt", "unpushed");
    sh(&fx.work, &["checkout", "-q", "-b", "scratch"]);
    sh(&fx.work, &["checkout", "-q", "main"]);
    let git = LocalGit::new(&fx.work);

    let report = git
        .fetch_and_sync_branches(&["y", "scratch", "nope"], None)
        .await;
    let by = |n: &str| report.branches.iter().find(|b| b.branch == n).unwrap();
    assert_eq!(by("y").outcome, BranchSyncOutcome::LocalAhead);
    assert_eq!(by("y").resolved.as_deref(), Some("refs/heads/y"));
    assert_eq!(head_of(&fx.work, "y"), ahead);
    assert_eq!(by("scratch").outcome, BranchSyncOutcome::LocalOnly);
    assert_eq!(by("nope").outcome, BranchSyncOutcome::Missing);
    assert_eq!(by("nope").resolved, None);
}

#[tokio::test]
async fn failed_fetch_is_reported_and_sync_uses_cached_refs() {
    let fx = fixture();
    push_from_other(&fx, "y", "y1.txt");
    sh(&fx.work, &["fetch", "-q", "origin"]);
    std::fs::remove_dir_all(&fx.remote).unwrap();
    let git = LocalGit::new(&fx.work);

    let report = git.fetch_and_sync_branches(&["y"], None).await;
    assert!(report.fetch_error.is_some());
    assert_eq!(report.branches[0].outcome, BranchSyncOutcome::Created);
}

#[tokio::test]
async fn unsafe_branch_names_are_refused_without_running_git_on_them() {
    let fx = fixture();
    let git = LocalGit::new(&fx.work);
    for bad in [
        "--upload-pack=x",
        "a..b",
        "refs/heads/main",
        "a b",
        "x:y",
        "+y",
        "y@{1}",
    ] {
        let b = git.sync_branch_with_origin(bad).await;
        assert_eq!(b.outcome, BranchSyncOutcome::Invalid, "{bad}");
        assert_eq!(b.resolved, None);
    }
    assert!(is_syncable_branch_name("feature/PROJ-1_x.y"));
}

#[tokio::test]
async fn resolve_base_fetches_a_base_that_only_exists_on_the_remote() {
    let fx = fixture();
    let y = push_from_other(&fx, "y", "y.txt");
    sh(&fx.work, &["checkout", "-q", "-b", "feature"]);
    commit(&fx.work, "f.txt", "feature work");
    let git = LocalGit::new(&fx.work);
    // Before the fix this silently fell back to the default branch (`main`).
    let rb = git.resolve_base(Some("y")).await.unwrap();
    assert_eq!(rb.branch, "y");
    assert_eq!(rb.diff_ref, "origin/y");
    assert_eq!(head_of(&fx.work, "origin/y"), y);
    // A sha-like or unknown base never fetches and still falls back.
    let rb = git.resolve_base(Some("nope")).await.unwrap();
    assert_eq!(rb.branch, "main");
    assert!(looks_like_sha("0123abc") && !looks_like_sha("develop"));
}
