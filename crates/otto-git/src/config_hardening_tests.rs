//! The daemon's own git never runs a program a repo's `.git/config` names
//! (sec-agent P1). A sandboxed agent can write its repo's `.git`; the daemon
//! polls `status` / fetches that repo UNconfined, so an agent-planted
//! `core.fsmonitor` or hook must not execute there — while a person's own
//! commit keeps the repo's hooks.

use std::path::Path;

use crate::local::{hardened_config_for, LocalGit, HARDENED_GIT_CONFIG, HOOKED_GIT_CONFIG};

fn sh(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// An executable script that records it ran by creating `marker`.
fn script(dir: &Path, name: &str, marker: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let p = dir.join(name);
    std::fs::write(
        &p,
        format!("#!/bin/sh\ntouch '{}'\nexit 0\n", marker.display()),
    )
    .unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}

fn repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@example.com"]);
    sh(&repo, &["config", "user.name", "t"]);
    sh(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    (tmp, repo)
}

#[test]
fn hooks_are_kept_only_for_deliberate_verbs() {
    assert_eq!(hardened_config_for("status"), HARDENED_GIT_CONFIG);
    assert_eq!(hardened_config_for("fetch"), HARDENED_GIT_CONFIG);
    assert_eq!(hardened_config_for("update-ref"), HARDENED_GIT_CONFIG);
    assert_eq!(hardened_config_for("commit"), HOOKED_GIT_CONFIG);
    assert_eq!(hardened_config_for("push"), HOOKED_GIT_CONFIG);
    // fsmonitor is off on every path, hooks only on the hooked one.
    assert!(HARDENED_GIT_CONFIG.contains("'core.fsmonitor=false'"));
    assert!(HARDENED_GIT_CONFIG.contains("'core.hooksPath=/dev/null'"));
    assert!(HOOKED_GIT_CONFIG.contains("'core.fsmonitor=false'"));
    assert!(!HOOKED_GIT_CONFIG.contains("hooksPath"));
}

#[test]
fn base_cmd_carries_the_hardened_config() {
    let git = LocalGit::new("/tmp");
    let cmd = git.base_cmd();
    let envs: Vec<_> = cmd.as_std().get_envs().collect();
    let get = |k: &str| {
        envs.iter()
            .find(|(n, _)| *n == k)
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    };
    assert_eq!(
        get("GIT_CONFIG_PARAMETERS").as_deref(),
        Some(HARDENED_GIT_CONFIG)
    );
    assert_eq!(get("GIT_PAGER").as_deref(), Some("cat"));
    // argv keeps its shape: no `-c` spliced in front of the verb.
    assert_eq!(cmd.as_std().get_args().count(), 0);
}

#[tokio::test]
async fn an_agent_planted_fsmonitor_or_hook_never_runs_on_daemon_reads() {
    let (tmp, repo) = repo();
    let ran = tmp.path().join("fsmonitor-ran");
    let fsm = script(tmp.path(), "fsm.sh", &ran);
    sh(&repo, &["config", "core.fsmonitor", fsm.to_str().unwrap()]);
    let hook_ran = tmp.path().join("hook-ran");
    let hooks = repo.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::copy(
        script(tmp.path(), "rt.sh", &hook_ran),
        hooks.join("reference-transaction"),
    )
    .unwrap();
    std::fs::write(repo.join("f.txt"), "x").unwrap();

    let git = LocalGit::new(&repo);
    git.status().await.expect("status");
    git.run(&["update-ref", "refs/heads/side", "HEAD"])
        .await
        .expect("update-ref");
    assert!(!ran.exists(), "daemon status must not run core.fsmonitor");
    assert!(
        !hook_ran.exists(),
        "a background ref update must not run the repo's hooks"
    );
}

#[tokio::test]
async fn a_person_s_commit_still_runs_the_repo_hooks() {
    let (tmp, repo) = repo();
    let ran = tmp.path().join("pre-commit-ran");
    std::fs::create_dir_all(repo.join(".git/hooks")).unwrap();
    std::fs::copy(
        script(tmp.path(), "pc.sh", &ran),
        repo.join(".git/hooks/pre-commit"),
    )
    .unwrap();
    std::fs::write(repo.join("f.txt"), "x").unwrap();
    let git = LocalGit::new(&repo);
    git.run(&["add", "f.txt"]).await.expect("add");
    git.run(&["commit", "-m", "c"]).await.expect("commit");
    assert!(
        ran.exists(),
        "the user's pre-commit hook runs on their commit"
    );
}

/// S2-02(a): an agent plants an embedded repo whose OWN config names a clean
/// filter, then `git add sub`. Deciding whether that gitlink's work tree is
/// dirty means running git inside it — with its filters. Daemon status and
/// diffs skip submodule work-tree dirt, so the filter never runs.
#[tokio::test]
async fn an_embedded_submodule_filter_never_runs_on_daemon_status_or_diff() {
    let (tmp, repo) = repo();
    let ran = tmp.path().join("filter-ran");
    let filt = script(tmp.path(), "clean.sh", &ran);
    let sub = repo.join("sub");
    std::fs::create_dir(&sub).unwrap();
    sh(&sub, &["init", "-q", "-b", "main"]);
    std::fs::write(sub.join("f"), "a").unwrap();
    sh(&sub, &["add", "f"]);
    sh(
        &sub,
        &[
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "s",
        ],
    );
    sh(&sub, &["config", "filter.x.clean", filt.to_str().unwrap()]);
    std::fs::write(sub.join(".gitattributes"), "* filter=x\n").unwrap();
    std::fs::write(sub.join("f"), "b").unwrap();
    sh(&repo, &["add", "sub"]);
    let _ = std::fs::remove_file(&ran);

    let git = LocalGit::new(&repo);
    let st = git.status().await.expect("status");
    git.status_full().await.expect("status_full");
    git.working_diff_text().await.expect("working diff");
    assert!(
        !ran.exists(),
        "daemon status ran the embedded repo's filter"
    );
    // The gitlink itself is still reported (only its work-tree dirt is not).
    assert!(
        format!("{st:?}").contains("sub"),
        "the staged gitlink must still show: {st:?}"
    );
    assert!(crate::local::DIFF_FORMAT.contains(&"--ignore-submodules=dirty"));
}

#[test]
fn hooks_path_inside_resolves_relative_absolute_and_escapes() {
    use crate::local::hooks_path_inside;
    let tmp = tempfile::tempdir().unwrap();
    let top = std::fs::canonicalize(tmp.path()).unwrap();
    let t = top.to_str().unwrap();
    assert!(hooks_path_inside(".husky/_", Some(t)));
    assert!(hooks_path_inside(&format!("{t}/.husky"), Some(t)));
    assert!(!hooks_path_inside("../outside/hooks", Some(t)));
    assert!(!hooks_path_inside("/usr/share/hooks", Some(t)));
    assert!(!hooks_path_inside(".husky/_", None));
}

/// S11-07: husky-style `core.hooksPath=.husky/_` lives in the work tree, so an
/// agent there can rewrite the hook. A daemon-initiated commit (workflow,
/// Run with Otto, swarm) runs without it; a person's commit through the git
/// routes (`person_initiated`) still runs it.
#[tokio::test]
async fn an_in_worktree_hooks_path_runs_only_for_a_person_s_commit() {
    let (tmp, repo) = repo();
    let ran = tmp.path().join("husky-ran");
    let hooks = repo.join(".husky/_");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::copy(script(tmp.path(), "pc.sh", &ran), hooks.join("pre-commit")).unwrap();
    sh(&repo, &["config", "core.hooksPath", ".husky/_"]);

    std::fs::write(repo.join("a.txt"), "x").unwrap();
    let daemon = LocalGit::new(&repo);
    daemon.run(&["add", "a.txt"]).await.expect("add");
    daemon.run(&["commit", "-m", "auto"]).await.expect("commit");
    assert!(
        !ran.exists(),
        "a daemon-initiated commit ran an in-worktree hook"
    );

    std::fs::write(repo.join("b.txt"), "x").unwrap();
    let person = LocalGit::new(&repo).person_initiated();
    person.run(&["add", "b.txt"]).await.expect("add");
    person.run(&["commit", "-m", "mine"]).await.expect("commit");
    assert!(
        ran.exists(),
        "the person's own commit keeps their husky hook"
    );
}

/// S2-02(b): the worktree probe only runs `git status` in trees that
/// round-trip to this repo — not in one whose admin `gitdir` (or `.git`
/// file) was re-pointed at an agent-built repo.
#[test]
fn worktree_round_trip_rejects_forged_pointers() {
    use crate::local::worktree_round_trips;
    let (tmp, repo) = repo();
    let common = repo.join(".git");
    let wt = tmp.path().join("wt");
    sh(
        &repo,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "side"],
    );
    let wt = std::fs::canonicalize(&wt).unwrap();
    assert!(worktree_round_trips(&repo, &common), "main tree");
    assert!(worktree_round_trips(&wt, &common), "linked tree");

    // A foreign repo the pointer is aimed at.
    let evil = tmp.path().join("evil");
    std::fs::create_dir(&evil).unwrap();
    sh(&evil, &["init", "-q"]);
    assert!(!worktree_round_trips(&evil, &common), "foreign repo");
    // A tree whose `.git` file names some other admin dir.
    let fake = tmp.path().join("fake");
    std::fs::create_dir(&fake).unwrap();
    std::fs::write(
        fake.join(".git"),
        format!("gitdir: {}\n", evil.join(".git").display()),
    )
    .unwrap();
    assert!(!worktree_round_trips(&fake, &common), "forged .git file");
    // The admin `gitdir` re-pointed elsewhere: the real tree no longer
    // round-trips.
    let admin = std::fs::read_dir(common.join("worktrees"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", fake.join(".git").display()),
    )
    .unwrap();
    assert!(
        !worktree_round_trips(&wt, &common),
        "re-pointed admin gitdir"
    );
}
