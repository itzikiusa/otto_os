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
