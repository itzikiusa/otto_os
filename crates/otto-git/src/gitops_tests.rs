use super::*;
use std::process::Command as StdCommand;

fn sh(dir: &Path, args: &[&str]) -> String {
    let out = StdCommand::new("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn commit(dir: &Path, file: &str, body: &str, msg: &str) {
    std::fs::write(dir.join(file), body).unwrap();
    sh(dir, &["add", file]);
    sh(dir, &["commit", "-qm", msg]);
}

/// `<tmp>/repo` (main, pushed to the bare `<tmp>/origin.git`).
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let origin = root.join("origin.git");
    let repo = root.join("repo");
    std::fs::create_dir(&repo).unwrap();
    sh(
        &root,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            origin.to_str().unwrap(),
        ],
    );
    sh(&repo, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.email", "otto@test.local"),
        ("user.name", "Otto Test"),
        ("commit.gpgsign", "false"),
    ] {
        sh(&repo, &["config", k, v]);
    }
    commit(&repo, "a.txt", "a\n", "init");
    sh(
        &repo,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    sh(&repo, &["push", "-q", "-u", "origin", "main"]);
    sh(&repo, &["remote", "set-head", "origin", "main"]);
    (tmp, repo)
}

fn add_wt(repo: &Path, name: &str, branch: &str) -> PathBuf {
    let wt = repo.parent().unwrap().join(name);
    sh(
        repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            wt.to_str().unwrap(),
            "main",
        ],
    );
    wt
}

fn find<'a>(rows: &'a [WorktreeInfo], path: &Path) -> &'a WorktreeInfo {
    rows.iter()
        .find(|w| Path::new(&w.path) == path)
        .unwrap_or_else(|| panic!("{} not listed", path.display()))
}

#[test]
fn porcelain_counts_split_tracked_and_untracked() {
    let out = b" M a.txt\nM  b.txt\n?? new.txt\n?? dir/\nR  c -> d\n";
    assert_eq!(porcelain_counts(out), (3, 2));
    assert_eq!(porcelain_counts(b""), (0, 0));
}

#[test]
fn parse_track_reads_ahead_behind_and_gone() {
    assert_eq!(parse_track("ahead 2, behind 1"), (2, 1));
    assert_eq!(parse_track("behind 4"), (0, 4));
    assert_eq!(parse_track("gone"), (0, 0));
    assert_eq!(parse_track(""), (0, 0));
}

#[test]
fn risks_are_empty_for_main_and_stale_entries() {
    let mut w = WorktreeInfo {
        is_main: true,
        changes: 3,
        ..WorktreeInfo::default()
    };
    assert!(removal_risks(&w).is_empty());
    w.is_main = false;
    w.prunable = true;
    assert!(removal_risks(&w).is_empty());
}

#[tokio::test]
async fn detailed_list_reports_changes_untracked_and_unpushed_commits() {
    let (_tmp, repo) = fixture();
    let dirty = add_wt(&repo, "wt-dirty", "feat/dirty");
    commit(&dirty, "b.txt", "b\n", "local only");
    std::fs::write(dirty.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dirty.join("new.txt"), "n\n").unwrap();
    let pushed = add_wt(&repo, "wt-pushed", "feat/pushed");
    commit(&pushed, "c.txt", "c\n", "pushed");
    sh(&pushed, &["push", "-q", "-u", "origin", "feat/pushed"]);

    let git = LocalGit::new(&repo);
    let rows = git.worktree_list_detailed().await.unwrap();
    let d = find(&rows, &dirty);
    assert!(d.dirty_known);
    assert_eq!((d.changes, d.untracked), (1, 1));
    assert_eq!(d.unpushed, Some(1));
    assert_eq!(d.unmerged, Some(1));
    assert_eq!(d.base.as_deref(), Some("origin/main"));
    assert!(!d.on_remote && d.upstream.is_none());
    assert_eq!(d.risks.len(), 3, "{:?}", d.risks);
    assert!(d
        .risks
        .iter()
        .any(|r| r.contains("not pushed to any remote")));

    let p = find(&rows, &pushed);
    assert!(p.on_remote);
    assert_eq!(p.upstream.as_deref(), Some("origin/feat/pushed"));
    assert_eq!((p.ahead, p.behind), (0, 0));
    assert_eq!(p.unpushed, Some(0));
    assert_eq!(p.unmerged, Some(1), "pushed but not merged into main");
    assert!(p.risks.is_empty(), "{:?}", p.risks);
    assert!(find(&rows, &repo).is_main);
}

#[tokio::test]
async fn remove_refuses_a_risky_worktree_until_forced_and_keeps_the_branch() {
    let (_tmp, repo) = fixture();
    let wt = add_wt(&repo, "wt", "feat/wt");
    commit(&wt, "b.txt", "b\n", "unpushed");
    std::fs::write(wt.join("scratch.txt"), "s\n").unwrap();
    let git = LocalGit::new(&repo);
    let path = wt.to_str().unwrap();

    let err = git.worktree_remove_safe(path, false).await.unwrap_err();
    let Error::Conflict(msg) = err else {
        panic!("expected a conflict, got {err:?}")
    };
    assert!(
        msg.contains("1 untracked file") && msg.contains("1 commit"),
        "{msg}"
    );
    assert!(
        wt.join("scratch.txt").exists(),
        "nothing deleted without force"
    );

    let outcome = git.worktree_remove_safe(path, true).await.unwrap();
    assert!(
        matches!(outcome, WorktreeRemoval::Detached { .. }),
        "{outcome:?}"
    );
    assert!(!wt.exists(), "the worktree path is free immediately");
    let list = sh(&repo, &["worktree", "list", "--porcelain"]);
    assert!(!list.contains(path), "unregistered: {list}");
    sh(&repo, &["rev-parse", "--verify", "refs/heads/feat/wt"]);
}

#[tokio::test]
async fn remove_takes_a_clean_worktree_with_ignored_bulk_without_force() {
    let (_tmp, repo) = fixture();
    std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
    sh(&repo, &["add", ".gitignore"]);
    sh(&repo, &["commit", "-qm", "ignore"]);
    sh(&repo, &["push", "-q", "origin", "main"]);
    let wt = add_wt(&repo, "wt-clean", "feat/clean");
    std::fs::create_dir_all(wt.join("target/debug")).unwrap();
    std::fs::write(wt.join("target/debug/big.bin"), vec![0u8; 4096]).unwrap();
    let git = LocalGit::new(&repo);
    let (w, admin) = git.worktree_inspect(wt.to_str().unwrap()).await.unwrap();
    assert!(admin.is_some() && w.risks.is_empty(), "{:?}", w.risks);
    git.worktree_remove_safe(wt.to_str().unwrap(), false)
        .await
        .unwrap();
    assert!(!wt.exists());
    sh(&repo, &["rev-parse", "--verify", "refs/heads/feat/clean"]);
}

#[tokio::test]
async fn locked_and_detached_orphans_are_risks() {
    let (_tmp, repo) = fixture();
    let locked = add_wt(&repo, "wt-locked", "feat/locked");
    sh(
        &repo,
        &[
            "worktree",
            "lock",
            "--reason",
            "agent busy",
            locked.to_str().unwrap(),
        ],
    );
    let detached = repo.parent().unwrap().join("wt-detached");
    sh(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            detached.to_str().unwrap(),
            "main",
        ],
    );
    commit(&detached, "x.txt", "x\n", "orphan");
    let git = LocalGit::new(&repo);
    let rows = git.worktree_list_detailed().await.unwrap();
    let l = find(&rows, &locked);
    assert_eq!(l.risks, vec!["it is locked (agent busy)".to_string()]);
    let d = find(&rows, &detached);
    assert_eq!(d.unpushed, Some(1));
    assert!(
        d.risks.iter().any(|r| r.contains("orphans them")),
        "{:?}",
        d.risks
    );
    assert!(git
        .worktree_remove_safe(locked.to_str().unwrap(), false)
        .await
        .is_err());
    git.worktree_remove_safe(locked.to_str().unwrap(), true)
        .await
        .unwrap();
    assert!(!locked.exists());
}

#[tokio::test]
async fn remove_never_deletes_a_tree_whose_git_link_points_elsewhere() {
    let (_tmp, repo) = fixture();
    let wt = add_wt(&repo, "wt-tampered", "feat/t");
    let other = repo.parent().unwrap().join("other");
    std::fs::create_dir(&other).unwrap();
    sh(&other, &["init", "-q"]);
    let foreign = other.join(".git");
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", foreign.display())).unwrap();
    std::fs::write(wt.join("keep.txt"), "keep\n").unwrap();
    let git = LocalGit::new(&repo);
    let err = git
        .worktree_remove_safe(wt.to_str().unwrap(), true)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert!(wt.join("keep.txt").exists());
}

#[tokio::test]
async fn stash_counts_detail_and_pop_by_sha() {
    let (_tmp, repo) = fixture();
    std::fs::write(repo.join("a.txt"), "a\nmore\n").unwrap();
    std::fs::write(repo.join("u.txt"), "untracked\n").unwrap();
    sh(&repo, &["stash", "push", "-q", "-u", "-m", "older"]);
    std::fs::write(repo.join("a.txt"), "a\nnewer\n").unwrap();
    sh(&repo, &["stash", "push", "-q", "-m", "newer"]);
    let git = LocalGit::new(&repo);

    let list = git.stash_list_counted().await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].files, Some(1), "newer: a.txt");
    assert_eq!(list[1].files, Some(2), "older: a.txt + untracked u.txt");

    let older = list[1].sha.clone();
    let d = git.stash_detail(&older).await.unwrap();
    let kinds: Vec<(&str, &str)> = d
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status.as_str()))
        .collect();
    assert_eq!(kinds, vec![("a.txt", "modified"), ("u.txt", "untracked")]);
    assert_eq!(d.files[0].additions, Some(1));
    assert!(
        d.patch.contains("+more") && d.patch.contains("+untracked"),
        "{}",
        d.patch
    );
    assert!(!d.truncated);

    // Pop the OLDER stash (index 1), not the top of the stack.
    git.stash_pop_sha(&older).await.unwrap();
    let left = git.stash_list().await.unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].message.ends_with("newer"));
    assert_eq!(
        std::fs::read_to_string(repo.join("u.txt")).unwrap(),
        "untracked\n"
    );

    assert!(matches!(
        git.stash_detail(&older).await,
        Err(Error::Invalid(_))
    ));
}

/// Regression (the "can't remove a worktree" report): removal used to be
/// `git worktree remove` under the 30 s local budget, and git deletes ignored
/// bulk (`target/`, `node_modules`) too — big trees were killed mid-delete.
/// The tree is now renamed aside and unregistered without git deleting it:
/// a git shim that fails `worktree remove` must not matter.
#[tokio::test]
async fn removal_never_waits_on_git_deleting_the_tree() {
    use std::os::unix::fs::PermissionsExt;
    let (tmp, repo) = fixture();
    let wt = add_wt(&repo, "wt-big", "feat/big");
    let shim = tmp.path().join("git-shim");
    let real =
        String::from_utf8(StdCommand::new("which").arg("git").output().unwrap().stdout).unwrap();
    std::fs::write(
        &shim,
        format!(
            "#!/bin/sh\ncase \" $* \" in *\" worktree remove \"*) echo 'killed: budget' >&2; exit 137;; esac\nexec {} \"$@\"\n",
            real.trim()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
    let git = LocalGit::new(&repo).with_git_bin(&shim);
    let outcome = git
        .worktree_remove_safe(wt.to_str().unwrap(), false)
        .await
        .unwrap();
    assert!(matches!(outcome, WorktreeRemoval::Detached { .. }));
    assert!(!wt.exists());
    assert!(!sh(&repo, &["worktree", "list"]).contains("wt-big"));
}

#[test]
fn sibling_worktree_path_slugs_the_branch_next_to_the_repo() {
    let p = sibling_worktree_path(Path::new("/w/claude_ade"), "feat/side chat");
    assert_eq!(p, PathBuf::from("/w/claude_ade-feat-side-chat"));
}

#[tokio::test]
async fn create_adds_a_new_branch_worktree_beside_the_repo_and_refuses_reuse() {
    let (_tmp, repo) = fixture();
    let git = LocalGit::new(&repo);

    let w = git.worktree_create("feat/new", None).await.unwrap();
    let expected = repo.parent().unwrap().join("repo-feat-new");
    assert_eq!(Path::new(&w.path), expected.as_path());
    assert_eq!(w.branch.as_deref(), Some("feat/new"));
    assert!(!w.is_main);
    // Same commit as main: created from HEAD.
    assert_eq!(
        sh(&expected, &["rev-parse", "HEAD"]),
        sh(&repo, &["rev-parse", "HEAD"])
    );

    let err = git.worktree_create("feat/new", None).await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    let err = git.worktree_create("bad..name", None).await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let err = git.worktree_create("--force", None).await.unwrap_err();
    assert!(!matches!(err, Error::Conflict(_)), "{err:?}");
}
