use super::*;
use otto_core::api::LineOrigin;

/// Run `git` synchronously for fixture setup.
fn sh_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

/// Repo with two commits, a staged rename, a staged add, an unstaged
/// modification and an untracked file.
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);

    write(&dir, "a.txt", "alpha line 1\nalpha line 2\nalpha line 3\n");
    write(
        &dir,
        "c.txt",
        "carrot content that is long enough to track renames\n",
    );
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "first commit"]);

    write(
        &dir,
        "a.txt",
        "alpha line 1\nalpha CHANGED 2\nalpha line 3\n",
    );
    sh_git(&dir, &["add", "a.txt"]);
    sh_git(&dir, &["commit", "-m", "second commit"]);

    // staged rename
    sh_git(&dir, &["mv", "c.txt", "d.txt"]);
    // staged new file
    write(&dir, "f.txt", "fresh\n");
    sh_git(&dir, &["add", "f.txt"]);
    // unstaged modification
    write(
        &dir,
        "a.txt",
        "alpha line 1\nalpha CHANGED 2\nalpha line 3\nappended\n",
    );
    // untracked
    write(&dir, "e.txt", "loose\n");

    (tmp, dir)
}

#[tokio::test]
async fn binary_and_marker_free_conflicts_preserve_exact_bytes() {
    for side in ["ours", "theirs", "keep", "delete"] {
        let (_tmp, dir) = fixture_on_branch("main");
        let git = LocalGit::new(&dir);
        std::fs::write(dir.join("blob"), b"base\0bytes").unwrap();
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "test: base binary"]);
        sh_git(&dir, &["checkout", "-b", "other"]);
        std::fs::write(dir.join("blob"), b"other\0bytes").unwrap();
        sh_git(&dir, &["commit", "-am", "test: other binary"]);
        sh_git(&dir, &["checkout", "main"]);
        std::fs::write(dir.join("blob"), b"ours\0bytes").unwrap();
        sh_git(&dir, &["commit", "-am", "test: our binary"]);
        let _ = git.run(&["merge", "other"]).await;
        let file = git.conflict_file("blob").await.unwrap();
        assert!(file.is_binary && file.ours_present && file.theirs_present);
        if side == "keep" {
            std::fs::write(dir.join("blob"), b"manual resolution").unwrap();
        }
        git.resolve_take_side("blob", side).await.unwrap();
        if side == "delete" {
            assert!(!dir.join("blob").exists());
        } else {
            let expected: &[u8] = match side {
                "theirs" => b"other\0bytes",
                "keep" => b"manual resolution",
                _ => b"ours\0bytes",
            };
            assert_eq!(std::fs::read(dir.join("blob")).unwrap(), expected);
        }
        assert!(git.conflicted_paths().await.unwrap().is_empty());
        assert!(
            git.resolve_take_side("blob", "delete").await.is_err(),
            "stale action cannot delete a resolved file"
        );
    }
}

/// S15-301: "Mark file resolved" on a binary conflict posted `""`; the
/// daemon truncated the blob and staged it. On a deleted-by-us conflict
/// it resurrected the file as empty. Both writes are refused now and
/// leave the conflict (and the bytes) untouched.
#[tokio::test]
async fn write_resolution_refuses_binary_and_absent_working_files() {
    let (_tmp, dir) = fixture_on_branch("main");
    let git = LocalGit::new(&dir);
    std::fs::write(dir.join("blob"), b"base\0bytes").unwrap();
    write(&dir, "gone.txt", "base\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "test: base"]);
    sh_git(&dir, &["checkout", "-b", "other"]);
    std::fs::write(dir.join("blob"), b"other\0bytes").unwrap();
    write(&dir, "gone.txt", "theirs changed\n");
    sh_git(&dir, &["commit", "-am", "test: other"]);
    sh_git(&dir, &["checkout", "main"]);
    std::fs::write(dir.join("blob"), b"ours\0bytes").unwrap();
    sh_git(&dir, &["rm", "-q", "gone.txt"]);
    sh_git(&dir, &["commit", "-am", "test: ours"]);
    let _ = git.run(&["merge", "other"]).await;

    let blob = git.conflict_file("blob").await.unwrap();
    assert!(blob.is_binary && blob.segments.is_empty());
    for body in ["", "text\n"] {
        let err = git.write_resolution("blob", body).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    }
    assert_eq!(std::fs::read(dir.join("blob")).unwrap(), b"ours\0bytes");

    // git leaves the modified side in the tree; the conflict this guards
    // is one whose working file is gone (removed by hand / a tool).
    let _ = std::fs::remove_file(dir.join("gone.txt"));
    let gone = git.conflict_file("gone.txt").await.unwrap();
    assert!(!gone.worktree_present && gone.segments.is_empty());
    let err = git.write_resolution("gone.txt", "").await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert!(!dir.join("gone.txt").exists());

    let conflicted = git.conflicted_paths().await.unwrap();
    assert_eq!(conflicted.len(), 2, "both stay conflicted: {conflicted:?}");
}

/// S15-28: the amend warning keyed on `upstream && ahead == 0` missed a
/// HEAD pushed under another name and a branch with no upstream.
#[tokio::test]
async fn head_remote_refs_lists_any_remote_ref_containing_head() {
    let (_tmp, dir) = fixture_on_branch("main");
    let git = LocalGit::new(&dir);
    assert!(git.head_remote_refs().await.unwrap().is_empty());
    // Pushed as `upstream/other-name`; the local branch has NO upstream.
    sh_git(
        &dir,
        &["update-ref", "refs/remotes/upstream/other-name", "HEAD"],
    );
    sh_git(
        &dir,
        &[
            "symbolic-ref",
            "refs/remotes/upstream/HEAD",
            "refs/remotes/upstream/other-name",
        ],
    );
    assert_eq!(
        git.head_remote_refs().await.unwrap(),
        vec!["upstream/other-name".to_string()],
        "the symbolic upstream/HEAD alias is not listed"
    );
    write(&dir, "new.txt", "unpushed\n");
    sh_git(&dir, &["add", "new.txt"]);
    sh_git(&dir, &["commit", "-m", "test: unpushed"]);
    assert!(git.head_remote_refs().await.unwrap().is_empty());
}

#[tokio::test]
async fn resolve_deleted_side_removes_file_and_index_conflict() {
    let (_tmp, dir) = fixture_on_branch("main");
    let git = LocalGit::new(&dir);
    write(&dir, "deleted.txt", "base\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "test: base"]);
    sh_git(&dir, &["checkout", "-b", "remove"]);
    sh_git(&dir, &["rm", "deleted.txt"]);
    sh_git(&dir, &["commit", "-m", "test: remove"]);
    sh_git(&dir, &["checkout", "main"]);
    write(&dir, "deleted.txt", "changed\n");
    sh_git(&dir, &["commit", "-am", "test: modify"]);
    let _ = git.run(&["merge", "remove"]).await;
    git.resolve_take_side("deleted.txt", "theirs")
        .await
        .unwrap();
    assert!(!dir.join("deleted.txt").exists());
    assert!(git.conflicted_paths().await.unwrap().is_empty());
}

#[tokio::test]
async fn unstage_rename_restores_both_index_paths() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    git.unstage(&["d.txt".into()]).await.unwrap();
    let staged = git.run(&["diff", "--cached", "--name-only"]).await.unwrap();
    assert_eq!(
        staged.trim(),
        "f.txt",
        "the unrelated addition stays staged; neither rename side does"
    );
    assert!(dir.join("d.txt").exists());
    assert!(!dir.join("c.txt").exists());
}

#[tokio::test]
async fn continuing_rebase_returns_successive_conflicts() {
    let (_tmp, dir) = fixture_on_branch("main");
    let git = LocalGit::new(&dir);
    write(&dir, "sequence.txt", "base\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "test: add base"]);
    sh_git(&dir, &["checkout", "-b", "topic"]);
    for content in ["first\n", "second\n"] {
        write(&dir, "sequence.txt", content);
        sh_git(&dir, &["commit", "-am", "test: change topic"]);
    }
    sh_git(&dir, &["checkout", "main"]);
    write(&dir, "sequence.txt", "upstream\n");
    sh_git(&dir, &["commit", "-am", "test: change upstream"]);
    sh_git(&dir, &["checkout", "topic"]);
    git.rebase("main", false).await.unwrap();
    git.write_resolution("sequence.txt", "resolved differently\n")
        .await
        .unwrap();
    let next = git.merge_commit(None).await.unwrap();
    assert_eq!(next.status, "conflicts");
    assert_eq!(next.conflicted_files, vec!["sequence.txt"]);
    git.merge_abort().await.unwrap();
}

/// Stash must take UNTRACKED files too — without `-u`, a tree whose only
/// changes were new files "stashed" successfully while stashing nothing
/// (the Stash button looked dead) — and a genuinely clean tree must error
/// loudly instead of toasting a success that did nothing.
#[tokio::test]
async fn stash_includes_untracked_and_clean_tree_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);
    write(&dir, "base.txt", "base\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "init"]);
    let git = LocalGit::new(&dir);

    // Untracked-only tree → the stash really takes it.
    write(&dir, "new.txt", "loose\n");
    git.stash_save().await.expect("untracked-only stash");
    assert!(
        git.status().await.unwrap().changes.is_empty(),
        "tree must be clean after stashing the untracked file"
    );
    assert_eq!(git.stash_list().await.unwrap().len(), 1);

    // Pop restores it.
    git.stash_pop().await.expect("pop");
    assert!(
        dir.join("new.txt").exists(),
        "pop must restore the untracked file"
    );

    // Clean tree → explicit error, not a silent no-op "success".
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "absorb"]);
    let err = git
        .stash_save()
        .await
        .expect_err("clean tree must not 'stash'");
    assert!(
        err.to_string().contains("nothing to stash"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn end_to_end_status_log_diff_commit() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);

    // status
    let st = git.status().await.unwrap();
    assert_eq!(st.branch, "main");
    let by_path = |p: &str| st.changes.iter().find(|c| c.path == p).cloned();
    let ren = by_path("d.txt").expect("rename present");
    assert_eq!(ren.kind, "renamed");
    assert_eq!(ren.orig_path.as_deref(), Some("c.txt"));
    assert!(ren.staged && !ren.unstaged);
    let add = by_path("f.txt").expect("added present");
    assert_eq!(add.kind, "added");
    assert!(add.staged);
    let m = by_path("a.txt").expect("modified present");
    assert_eq!(m.kind, "modified");
    assert!(!m.staged && m.unstaged);
    let unt = by_path("e.txt").expect("untracked present");
    assert_eq!(unt.kind, "untracked");

    // branches / current
    let branches = git.branches().await.unwrap();
    let main = branches.iter().find(|b| b.name == "main").unwrap();
    assert!(main.is_current);
    assert_eq!(git.current_branch().await.unwrap(), "main");

    // log
    let log = git.log(10, 0, false).await.unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].subject, "second commit");
    assert_eq!(log[1].subject, "first commit");
    assert_eq!(log[0].author, "Otto Test");
    let one = git.log(1, 1, false).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].subject, "first commit");

    // staged diff: rename detected, new file present
    let staged = git.diff(DiffTarget::Staged, None).await.unwrap();
    let dren = staged.files.iter().find(|f| f.path == "d.txt").unwrap();
    assert_eq!(dren.old_path.as_deref(), Some("c.txt"));
    assert!(staged.files.iter().any(|f| f.path == "f.txt"));

    // worktree diff: a.txt with one added line numbered 4 (untracked excluded)
    let wt = git.diff(DiffTarget::Worktree, None).await.unwrap();
    assert!(!wt.files.iter().any(|f| f.path == "e.txt"));
    let fa = wt.files.iter().find(|f| f.path == "a.txt").unwrap();
    let adds: Vec<_> = fa.hunks[0]
        .lines
        .iter()
        .filter(|l| l.origin == LineOrigin::Add)
        .collect();
    assert_eq!(adds.len(), 1);
    assert_eq!(adds[0].content, "appended");
    assert_eq!(adds[0].new_line, Some(4));

    // commit diff of HEAD (the a.txt change)
    let head = git.log(1, 0, false).await.unwrap()[0].sha.clone();
    let cd = git
        .diff(DiffTarget::Commit(head.clone()), None)
        .await
        .unwrap();
    assert_eq!(cd.files.len(), 1);
    assert_eq!(cd.files[0].path, "a.txt");

    // range diff
    let first = git.log(1, 1, false).await.unwrap()[0].sha.clone();
    let rd = git
        .diff(DiffTarget::Range(first.clone(), head.clone()), None)
        .await
        .unwrap();
    assert_eq!(rd.files.len(), 1);

    // stage the modification, commit, verify log grows and sha returned
    git.stage(&["a.txt".into()]).await.unwrap();
    let sha = git.commit("third commit", false).await.unwrap();
    assert_eq!(sha.len(), 40);
    let log = git.log(10, 0, false).await.unwrap();
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].sha, sha);

    // unstage works
    git.stage(&["e.txt".into()]).await.unwrap();
    git.unstage(&["e.txt".into()]).await.unwrap();
    let st = git.status().await.unwrap();
    assert_eq!(
        st.changes.iter().find(|c| c.path == "e.txt").unwrap().kind,
        "untracked"
    );

    // checkout -b
    git.checkout("feature/x", true).await.unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "feature/x");
    git.checkout("main", false).await.unwrap();

    // stash save/pop round-trip
    write(&dir, "a.txt", "stash me\n");
    git.stash_save().await.unwrap();
    let st = git.status().await.unwrap();
    assert!(!st.changes.iter().any(|c| c.path == "a.txt"));
    git.stash_pop().await.unwrap();
    let st = git.status().await.unwrap();
    assert!(st.changes.iter().any(|c| c.path == "a.txt"));
}

#[tokio::test]
async fn missing_repo_dir_is_not_found() {
    let git = LocalGit::new("/tmp/otto-definitely-not-a-repo-xyz");
    match git.status().await {
        Err(Error::NotFound(_)) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn checkout_of_unknown_branch_is_a_conflict_not_an_outage() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    // A bad ref is the caller's input — a 409 with git's own message, never
    // the 502 that raises the provider-outage banner.
    match git.checkout("no-such-branch", false).await {
        Err(Error::Conflict(msg)) => assert!(msg.contains("invalid reference"), "{msg}"),
        other => panic!("expected Conflict, got {other:?}"),
    }
}

/// Abort with NOTHING in progress must refuse — the old fallback ran
/// `reset --hard HEAD`, so a stale/double "Abort" click destroyed every
/// uncommitted change in the tree.
#[tokio::test]
async fn merge_abort_with_nothing_in_progress_refuses_and_keeps_work() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    match git.merge_abort().await {
        Err(Error::Conflict(_)) => {}
        other => panic!("expected Conflict, got {other:?}"),
    }
    // The fixture's dirty working tree survived untouched.
    let st = git.status().await.unwrap();
    assert!(st.changes.iter().any(|c| c.path == "a.txt" && c.unstaged));
    assert!(st.changes.iter().any(|c| c.path == "e.txt"));
}

/// In a LINKED worktree `.git` is a file: the merge label must still come
/// from MERGE_MSG in the real git dir, not fall back to a bare sha.
#[tokio::test]
async fn merge_source_reads_merge_msg_in_a_linked_worktree() {
    let (tmp, dir) = fixture_n_commits(1);
    sh_git(&dir, &["checkout", "-b", "side"]);
    write(&dir, "f.txt", "side\n");
    sh_git(&dir, &["commit", "-am", "side"]);
    sh_git(&dir, &["checkout", "main"]);
    let wt = tmp.path().join("wt");
    sh_git(
        &dir,
        &["worktree", "add", "-b", "wtb", wt.to_str().unwrap(), "main"],
    );
    write(&wt, "f.txt", "wt\n");
    sh_git(&wt, &["commit", "-am", "wt"]);
    let merged = std::process::Command::new("git")
        .current_dir(&wt)
        .args(["merge", "side"])
        .output()
        .unwrap();
    assert!(!merged.status.success(), "the fixture must conflict");
    let ms = LocalGit::new(&wt).merge_status().await.unwrap();
    assert!(ms.merging);
    assert_eq!(ms.source.as_deref(), Some("Merge branch 'side' into wtb"));
}

/// A conflicting cherry-pick is a normal outcome: status reports the op +
/// conflicted files, merge/status opens the resolver, and abort uses
/// `cherry-pick --abort` (not a hard reset).
#[tokio::test]
async fn conflicting_cherry_pick_is_visible_and_abortable() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "clean slate"]);
    sh_git(&dir, &["checkout", "-b", "side"]);
    write(&dir, "a.txt", "side version\n");
    sh_git(&dir, &["commit", "-am", "side change"]);
    sh_git(&dir, &["checkout", "main"]);
    write(&dir, "a.txt", "main version\n");
    sh_git(&dir, &["commit", "-am", "main change"]);
    let side_sha = git.rev_parse("side").await.unwrap();

    git.cherry_pick(&side_sha).await.unwrap();
    let st = git.status().await.unwrap();
    assert_eq!(st.op_in_progress.as_deref(), Some("cherry_pick"));
    assert!(st.changes.iter().any(|c| c.kind == "conflicted"));
    let ms = git.merge_status().await.unwrap();
    assert!(ms.merging);
    assert_eq!(ms.op.as_deref(), Some("cherry_pick"));
    assert!(!ms.conflicted_files.is_empty());
    // diff3 markers: the resolver gets the BASE side too, not just
    // ours/theirs (only `merge_branch` used to set the style).
    let cf = git.conflict_file("a.txt").await.unwrap();
    let base_seen = cf.segments.iter().any(|seg| {
        matches!(seg, otto_core::api::ConflictSegment::Conflict { base, .. } if !base.is_empty())
    });
    assert!(base_seen, "cherry-pick conflict must carry a base: {cf:?}");

    let st = git.merge_abort().await.unwrap();
    assert_eq!(st.op_in_progress, None);
    assert!(st.changes.is_empty());
}

/// Starting a merge over an unfinished merge is refused with a 409 — the
/// old path checked out `target` and died with git's "resolve your current
/// index first" as a 502.
#[tokio::test]
async fn merge_over_an_unfinished_merge_is_refused() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "clean slate"]);
    sh_git(&dir, &["checkout", "-b", "side"]);
    write(&dir, "a.txt", "side version\n");
    sh_git(&dir, &["commit", "-am", "side change"]);
    sh_git(&dir, &["checkout", "main"]);
    write(&dir, "a.txt", "main version\n");
    sh_git(&dir, &["commit", "-am", "main change"]);

    let r = git
        .merge_branch("side", "main", LocalMergeStrategy::MergeCommit, false)
        .await
        .unwrap();
    assert_eq!(r.status, "conflicts");
    match git
        .merge_branch("side", "main", LocalMergeStrategy::MergeCommit, false)
        .await
    {
        Err(Error::Conflict(msg)) => assert!(msg.contains("already in progress")),
        other => panic!("expected Conflict, got {other:?}"),
    }
    git.merge_abort().await.unwrap();
}

/// A LOCAL refusal (dirty tree, no upstream, divergent branches) is a 409,
/// not the 502 that makes the UI announce a git-provider outage. A remote
/// failure — and anything unrecognised — still maps to Upstream.
#[test]
fn local_git_refusals_are_conflicts_not_upstream() {
    let local = [
        "error: Your local changes to the following files would be overwritten by merge:\n\tsrc/a.rs\nPlease commit your changes or stash them before you merge.",
        "fatal: Need to specify how to reconcile divergent branches",
        "fatal: There is no tracking information for the current branch.",
        "error: You have not concluded your merge (MERGE_HEAD exists).",
        "fatal: refusing to merge unrelated histories",
        "error: you need to resolve your current index first",
        "error: Merging is not possible because you have unmerged files.",
        "error: pathspec 'no-such-branch' did not match any file(s) known to git",
        "fatal: Unable to create '/r/.git/index.lock': File exists.\nAnother git process seems to be running",
        "fatal: You are not currently on a branch.\nTo push the history leading to the current (detached HEAD)",
        "error: src refspec main does not match any\nerror: failed to push some refs to 'origin'",
    ];
    for msg in local {
        match upstream_err(msg, "", Some(1)) {
            Error::Conflict(_) => {}
            other => panic!("expected Conflict for {msg:?}, got {other:?}"),
        }
    }
    let remote = [
        "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
        "ssh: connect to host github.com port 22: Connection timed out",
    ];
    for msg in remote {
        match upstream_err(msg, "", Some(1)) {
            Error::Upstream(_) => {}
            other => panic!("expected Upstream for {msg:?}, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn commit_all_if_dirty_sweeps_leftovers_but_not_artifacts() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    // The fixture starts dirty (staged rename/new, unstaged, untracked) —
    // the sweep commits all of it.
    assert!(git
        .commit_all_if_dirty("sweep fixture")
        .await
        .unwrap()
        .is_some());
    // Now-clean tree → no commit.
    assert!(git.commit_all_if_dirty("noop").await.unwrap().is_none());
    // Leftover agent work + runtime artifacts that must stay out.
    write(&dir, "work.txt", "agent forgot to commit me\n");
    write(&dir, ".mcp.json", "{}\n");
    write(&dir, ".env", "SECRET=1\n");
    let sha = git.commit_all_if_dirty("sweep").await.unwrap();
    assert!(sha.is_some());
    let st = git.status().await.unwrap();
    assert!(!st.changes.iter().any(|c| c.path == "work.txt"));
    assert!(st.changes.iter().any(|c| c.path == ".mcp.json"));
    assert!(st.changes.iter().any(|c| c.path == ".env"));
    // Nothing real left → None again.
    assert!(git.commit_all_if_dirty("noop").await.unwrap().is_none());
}

#[test]
fn askpass_username_follows_token_kind() {
    // Atlassian API tokens must authenticate under the magic username;
    // everything else uses x-token-auth (overriding inherited env).
    let api = AskPass::new(&TransportCredential {
        token: "ATATT3xFfGF0abc".into(),
        origins: vec!["https://bitbucket.org".into()],
    })
    .unwrap();
    assert!(api
        .envs()
        .iter()
        .any(|(k, v)| { k == "OTTO_GIT_USERNAME" && v == "x-bitbucket-api-token-auth" }));
    let pat = AskPass::new(&TransportCredential {
        token: "ghp_abc123".into(),
        origins: vec!["https://github.com".into()],
    })
    .unwrap();
    assert!(pat
        .envs()
        .iter()
        .any(|(k, v)| k == "OTTO_GIT_USERNAME" && v == "x-token-auth"));
    // Otto's credential must be authoritative: helpers reset for every
    // token kind (a stale osxkeychain app password otherwise wins and
    // draws a hard 410 before askpass is ever consulted).
    for a in [&api, &pat] {
        assert!(a
            .envs()
            .iter()
            .any(|(k, v)| k == "GIT_CONFIG_KEY_0" && v == "credential.helper"));
    }
}

/// `clone_repo` only ever receives a REMOTE url: a local path is a
/// registration (`POST /repos {path}`), and letting one through would make
/// the clone route an arbitrary-path reader. Nothing is spawned — the guard
/// fires before git.
#[tokio::test]
async fn clone_refuses_local_paths_and_option_like_urls() {
    let (_tmp, dir) = fixture();
    let dest_tmp = tempfile::tempdir().unwrap();
    let dest = dest_tmp.path().join("cloned");
    for url in [
        dir.to_str().unwrap(),
        "file:///tmp/x",
        "--upload-pack=touch /tmp/pwn",
    ] {
        let err = clone_repo(url, &dest, None, |_| {}).await.unwrap_err();
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("invalid remote url")),
            "{url:?} → {err:?}"
        );
    }
    assert!(!dest.exists(), "a refused clone creates nothing");
}

#[tokio::test]
async fn worktree_failed_branch_probe_never_resets_existing_branch() {
    let (tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    let head = git.rev_parse("HEAD").await.unwrap();
    sh_git(&dir, &["branch", "agent"]);
    let sh = shim(
        tmp.path(),
        "failed-branch-probe",
        "if [ \"$1\" = rev-parse ] && [ \"$4\" = refs/heads/agent ]; then exit 128; fi\nexec git \"$@\"",
    );
    let wt = tmp.path().join("agent-wt");
    let result = LocalGit::new(&dir)
        .with_git_bin(sh)
        .worktree_add_if_absent(wt.to_str().unwrap(), "agent", "HEAD~1")
        .await;
    assert!(
        result.is_err(),
        "failed probe must not authorize resetting a branch"
    );
    assert_eq!(git.rev_parse("agent").await.unwrap(), head);
    assert!(!wt.exists());
}

#[tokio::test]
async fn stash_apply_keeps_sha_identity_when_stack_is_renumbered() {
    let (tmp, dir) = fixture();
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "clean fixture"]);
    write(&dir, "a.txt", "requested stash\n");
    sh_git(&dir, &["stash", "push"]);
    let git = LocalGit::new(&dir);
    let sha = git.stash_list().await.unwrap()[0].sha.clone();
    let sh = shim(
        tmp.path(),
        "renumber-stash",
        "while [ \"$1\" = -c ]; do shift 2; done\nif [ \"$1\" = stash ] && [ \"$2\" = apply ]; then\nprintf 'other stash\\n' > a.txt\ngit stash push >/dev/null || exit 1\nfi\nexec git \"$@\"",
    );
    LocalGit::new(&dir)
        .with_git_bin(sh)
        .stash_apply(&sha)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "requested stash\n"
    );
    assert_eq!(
        git.stash_list().await.unwrap().len(),
        2,
        "apply keeps both stashes"
    );
}

#[tokio::test]
async fn stash_drop_keeps_sha_identity_when_stack_is_renumbered() {
    let (tmp, dir) = fixture();
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "clean fixture"]);
    write(&dir, "a.txt", "requested stash\n");
    sh_git(&dir, &["stash", "push", "-m", "requested"]);
    let git = LocalGit::new(&dir);
    let selected = git.stash_list().await.unwrap()[0].sha.clone();
    let sh = shim(
        tmp.path(),
        "renumber-before-stash-drop",
        "while [ \"$1\" = -c ]; do shift 2; done\nif [ \"$1\" = stash ] && [ \"$2\" = list ]; then\ngit \"$@\" || exit 1\nprintf 'concurrent stash\\n' > a.txt\ngit stash push -m concurrent >/dev/null || exit 1\nexit 0\nfi\nexec git \"$@\"",
    );
    LocalGit::new(&dir)
        .with_git_bin(sh)
        .stash_drop(&selected)
        .await
        .unwrap();
    let remaining = git.stash_list().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_ne!(
        remaining[0].sha, selected,
        "Drop must remove the selected object, not the concurrent arrival"
    );
    git.stash_apply(&remaining[0].sha).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "concurrent stash\n"
    );
}

#[tokio::test]
async fn automatic_stash_capture_and_restore_preserve_identity_and_backup() {
    let (tmp, dir) = fixture();
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "clean fixture"]);
    write(&dir, "a.txt", "requested staged contents\n");
    sh_git(&dir, &["add", "a.txt"]);
    let sh = shim(
        tmp.path(),
        "interleaved-stash-push",
        "if [ \"$1\" = stash ] && [ \"$2\" = push ]; then\ngit \"$@\" || exit 1\nprintf 'concurrent contents\\n' > a.txt\ngit stash push -m concurrent\nexit $?\nfi\nexec git \"$@\"",
    );
    let git = LocalGit::new(&dir).with_git_bin(sh);
    let sha = git.stash_save_sha().await.unwrap();
    let before = git.stash_list().await.unwrap();
    assert_eq!(before.len(), 2);
    assert_ne!(before[0].sha, sha, "our stash is no longer the top entry");
    git.pop_stash_sha(&sha).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "requested staged contents\n"
    );
    assert_eq!(
        git.run_read(&["show", ":a.txt"]).await.unwrap(),
        "requested staged contents\n"
    );
    let after = git.stash_list().await.unwrap();
    assert_eq!(
        after.iter().map(|s| &s.sha).collect::<Vec<_>>(),
        before.iter().map(|s| &s.sha).collect::<Vec<_>>()
    );
}

/// D1 regression: a worktree provisioned with `worktree_add_if_absent` must
/// be REUSED on the second call (not reset), so an agent's committed work
/// from a prior turn survives. The old unconditional `worktree_add`
/// (`-B`/`--force`) would discard it by resetting the branch to base.
#[tokio::test]
async fn worktree_add_if_absent_reuses_and_preserves_commits() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    let wt = dir.parent().unwrap().join("agent-wt");
    let wt_str = wt.to_str().unwrap().to_string();
    let branch = "swarm/s1/a1";

    // First turn: created from absent → true.
    assert!(!git.worktree_exists(&wt_str).await);
    let created = git
        .worktree_add_if_absent(&wt_str, branch, "HEAD")
        .await
        .unwrap();
    assert!(created, "first call should create the worktree");
    assert!(git.worktree_exists(&wt_str).await);

    // Agent does work IN the worktree and commits it (multi-turn progress).
    let wt_git = LocalGit::new(&wt);
    write(&wt, "agent_work.txt", "turn 1 output\n");
    wt_git.stage(&["agent_work.txt".into()]).await.unwrap();
    let sha = wt_git.commit("agent turn 1", false).await.unwrap();

    // Second turn: already exists → reuse (false), NO reset. The commit and
    // the file must still be there.
    let created2 = git
        .worktree_add_if_absent(&wt_str, branch, "HEAD")
        .await
        .unwrap();
    assert!(!created2, "second call should reuse, not recreate");
    assert_eq!(
        wt_git.current_branch().await.unwrap(),
        branch,
        "still on the agent's branch"
    );
    let head = wt_git.log(1, 0, false).await.unwrap();
    assert_eq!(head[0].sha, sha, "prior commit preserved");
    assert_eq!(head[0].subject, "agent turn 1");
    assert!(
        wt.join("agent_work.txt").exists(),
        "committed file preserved"
    );
}

/// D2 regression: if the worktree was REMOVED between turns (the branch is
/// kept by `worktree_remove`), a later `worktree_add_if_absent` must
/// RE-ATTACH the surviving branch — never reset it to `base` with `-B`. The
/// old destructive `-B` path discarded the agent's committed work whenever
/// its worktree had been pruned (idle cleanup / restart), which is exactly
/// the "destructive `-B --force` branch reuse" hazard.
#[tokio::test]
async fn worktree_add_if_absent_reattaches_branch_after_worktree_removed() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    let wt = dir.parent().unwrap().join("reattach-wt");
    let wt_str = wt.to_str().unwrap().to_string();
    let branch = "swarm/s2/a2";

    // base = the FIRST commit (≠ HEAD/second commit) so a reset-to-base is
    // observable as a different sha than the agent's own commit.
    let base = git.log(1, 1, false).await.unwrap()[0].sha.clone();

    // Turn 1: create on the branch from HEAD, then commit agent work.
    git.worktree_add_if_absent(&wt_str, branch, "HEAD")
        .await
        .unwrap();
    let wt_git = LocalGit::new(&wt);
    write(&wt, "agent_work.txt", "turn 1 output\n");
    wt_git.stage(&["agent_work.txt".into()]).await.unwrap();
    let sha = wt_git.commit("agent turn 1", false).await.unwrap();

    // The worktree is pruned, but the branch + its commit must live on.
    git.worktree_remove(&wt_str).await.unwrap();
    assert!(!git.worktree_exists(&wt_str).await);
    assert!(
        git.branch_exists(branch).await,
        "branch survives worktree removal"
    );

    // Turn 2: re-provision with a DIFFERENT base. It must RE-ATTACH the
    // existing branch (preserving the agent commit), not reset to base.
    let created = git
        .worktree_add_if_absent(&wt_str, branch, &base)
        .await
        .unwrap();
    assert!(
        created,
        "re-provisioning a pruned worktree counts as created"
    );
    let head = wt_git.log(1, 0, false).await.unwrap();
    assert_eq!(
        head[0].sha, sha,
        "branch must NOT be reset to base; agent commit preserved"
    );
    assert_eq!(head[0].subject, "agent turn 1");
    assert!(
        wt.join("agent_work.txt").exists(),
        "committed file restored on re-attach"
    );
}

/// `changed_files` lists what a worktree branch changed vs its base — the
/// signal the swarm uses to detect two agents touching the same shared files.
#[tokio::test]
async fn changed_files_lists_branch_changes() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    let wt = dir.parent().unwrap().join("cf-wt");
    let wt_str = wt.to_str().unwrap().to_string();
    git.worktree_add_if_absent(&wt_str, "swarm/s1/a1", "main")
        .await
        .unwrap();

    let wt_git = LocalGit::new(&wt);
    // No commits on the branch yet → no changes vs base.
    assert!(wt_git.changed_files("main").await.unwrap().is_empty());

    // Commit a new file + modify an existing one.
    write(&wt, "shared.txt", "agent A\n");
    write(&wt, "a.txt", "alpha line 1\nalpha line 2\nalpha A\n");
    wt_git
        .stage(&["shared.txt".into(), "a.txt".into()])
        .await
        .unwrap();
    wt_git.commit("agent A work", false).await.unwrap();

    let mut files = wt_git.changed_files("main").await.unwrap();
    files.sort();
    assert_eq!(files, vec!["a.txt".to_string(), "shared.txt".to_string()]);
}

/// `worktree_exists` is path-aware: false for an unrelated path, true once
/// registered (even via a non-canonical path with a trailing component).
#[tokio::test]
async fn worktree_exists_tracks_registration() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    let wt = dir.parent().unwrap().join("wt2");
    let wt_str = wt.to_str().unwrap().to_string();

    assert!(!git.worktree_exists(&wt_str).await);
    git.worktree_add(&wt_str, "swarm/s/b", "HEAD")
        .await
        .unwrap();
    assert!(git.worktree_exists(&wt_str).await);
    // An unrelated path is not a worktree.
    assert!(
        !git.worktree_exists("/tmp/definitely-not-a-worktree-xyz")
            .await
    );
}

/// Local-only graph context-menu ops: branch create/rename/delete, tag
/// create (lightweight + annotated) / delete, cherry-pick and revert. Remote
/// ops (`delete_remote_*`, `push_tag`) need a remote and are exercised via
/// the live verification, not here.
#[tokio::test]
async fn graph_context_ops_local() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);

    // Commit the pending fixture changes so HEAD is clean for picks/reverts.
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "baseline"]);
    let head = git.log(1, 0, false).await.unwrap()[0].sha.clone();

    // create_branch in place (no checkout) from a start_point.
    git.create_branch("feat/a", Some(&head), false)
        .await
        .unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "main");
    assert!(git
        .refs()
        .await
        .unwrap()
        .local
        .iter()
        .any(|b| b.name == "feat/a"));

    // create_branch + checkout from HEAD.
    git.create_branch("feat/b", None, true).await.unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "feat/b");

    // rename it, then go back to main.
    git.rename_branch("feat/b", "feat/b2").await.unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "feat/b2");
    git.checkout("main", false).await.unwrap();

    // delete branches (force for the unmerged renamed one).
    git.delete_branch("feat/a", false).await.unwrap();
    git.delete_branch("feat/b2", true).await.unwrap();
    let locals = git.refs().await.unwrap().local;
    assert!(!locals
        .iter()
        .any(|b| b.name == "feat/a" || b.name == "feat/b2"));

    // lightweight + annotated tags, then list + delete.
    git.create_tag("v1", &head, None).await.unwrap();
    git.create_tag("v2", &head, Some("release two"))
        .await
        .unwrap();
    let tags = git.refs().await.unwrap().tags;
    assert!(tags.iter().any(|t| t.name == "v1"));
    assert!(tags.iter().any(|t| t.name == "v2"));
    git.delete_tag("v1").await.unwrap();
    assert!(!git
        .refs()
        .await
        .unwrap()
        .tags
        .iter()
        .any(|t| t.name == "v1"));

    // cherry-pick: make a commit on a side branch, pick it onto main.
    git.create_branch("side", Some(&head), true).await.unwrap();
    write(&dir, "picked.txt", "from side\n");
    git.stage(&["picked.txt".into()]).await.unwrap();
    let side_sha = git.commit("side change", false).await.unwrap();
    git.checkout("main", false).await.unwrap();
    git.cherry_pick(&side_sha).await.unwrap();
    assert!(dir.join("picked.txt").exists());
    assert_eq!(
        git.log(1, 0, false).await.unwrap()[0].subject,
        "side change"
    );

    // revert the cherry-picked commit → file removed again.
    let picked = git.log(1, 0, false).await.unwrap()[0].sha.clone();
    git.revert(&picked).await.unwrap();
    assert!(!dir.join("picked.txt").exists());

    // a bad cherry-pick ref surfaces as an Upstream error.
    assert!(git.cherry_pick("deadbeefdeadbeef").await.is_err());
}

#[test]
fn diff_target_parse() {
    assert_eq!(DiffTarget::parse("worktree").unwrap(), DiffTarget::Worktree);
    assert_eq!(DiffTarget::parse("staged").unwrap(), DiffTarget::Staged);
    assert_eq!(
        DiffTarget::parse("commit:abc").unwrap(),
        DiffTarget::Commit("abc".into())
    );
    assert_eq!(
        DiffTarget::parse("range:a1..b2").unwrap(),
        DiffTarget::Range("a1".into(), "b2".into())
    );
    assert!(DiffTarget::parse("bogus").is_err());
    assert!(DiffTarget::parse("range:onlyone").is_err());
    assert!(DiffTarget::parse("commit:").is_err());
}

/// MergeCommit must "always create a merge commit" — even when <source> is an
/// ancestor of <target> (the "close the already-merged release into develop"
/// case), where plain `merge --no-ff` would just say "Already up to date".
#[tokio::test]
async fn merge_commit_forces_a_commit_when_up_to_date() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);
    // Commit the fixture's dirty state so the working tree is clean to merge.
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "tidy"]);
    let main_head = git
        .run(&["rev-parse", "HEAD"])
        .await
        .unwrap()
        .trim()
        .to_string();
    // `rel` points at an ANCESTOR of main's tip → already contained, so a
    // plain merge would be "Already up to date" with no commit.
    let ancestor = git
        .run(&["rev-parse", "HEAD~1"])
        .await
        .unwrap()
        .trim()
        .to_string();
    sh_git(&dir, &["branch", "rel", &ancestor]);

    let res = git
        .merge_branch("rel", "main", LocalMergeStrategy::MergeCommit, false)
        .await
        .unwrap();

    assert_eq!(
        res.status, "merged",
        "forced a merge even though up to date"
    );
    let new_head = res.commit.expect("a merge commit sha");
    assert_ne!(
        new_head, main_head,
        "main advanced onto the new merge commit"
    );
    // …a real 2-parent merge (target tip + the source).
    let parents = git
        .run(&["rev-list", "--parents", "-n", "1", "HEAD"])
        .await
        .unwrap();
    assert_eq!(
        parents.split_whitespace().count() - 1,
        2,
        "forced merge commit has two parents"
    );
    // No working-tree churn — the merge tree equals main's tree.
    assert!(
        git.status().await.unwrap().changes.is_empty(),
        "working tree stays clean"
    );
}

/// Deleting a branch on the remote must also drop the LOCAL remote-tracking
/// ref, so the UI's REMOTE list reflects it immediately (no pull needed).
#[tokio::test]
async fn delete_remote_branch_prunes_local_tracking_ref() {
    let (_tmp, dir) = fixture();
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "tidy"]);
    // A bare repo plays the role of `origin`.
    let parent = dir.parent().unwrap();
    sh_git(parent, &["init", "--bare", "origin.git"]);
    let bare = parent.join("origin.git");
    sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sh_git(&dir, &["push", "origin", "main"]);
    sh_git(&dir, &["branch", "tmp"]);
    sh_git(&dir, &["push", "origin", "tmp"]);
    sh_git(&dir, &["fetch", "origin"]);

    let git = LocalGit::new(&dir);
    let before = git.refs().await.unwrap().remote;
    assert!(
        before.iter().any(|b| b.name == "origin/tmp"),
        "origin/tmp present before delete"
    );

    git.delete_remote_branch("tmp", None).await.unwrap();

    // Pruned locally → gone from /refs WITHOUT a fetch.
    let after = git.refs().await.unwrap().remote;
    assert!(
        !after.iter().any(|b| b.name == "origin/tmp"),
        "origin/tmp pruned from local tracking refs after delete"
    );
}

/// S15-01: a remote-ref row on a NON-origin remote (`upstream/tmp`) must
/// delete `tmp` on `upstream` — origin's same-named branch is untouched —
/// and an unknown remote name is refused before any push.
#[tokio::test]
async fn delete_remote_branch_on_targets_the_named_remote_only() {
    let (_tmp, dir) = fixture();
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "tidy"]);
    let parent = dir.parent().unwrap();
    sh_git(parent, &["init", "--bare", "origin.git"]);
    sh_git(parent, &["init", "--bare", "upstream.git"]);
    let origin = parent.join("origin.git");
    let upstream = parent.join("upstream.git");
    sh_git(&dir, &["remote", "add", "origin", origin.to_str().unwrap()]);
    sh_git(
        &dir,
        &["remote", "add", "upstream", upstream.to_str().unwrap()],
    );
    sh_git(&dir, &["branch", "tmp"]);
    sh_git(&dir, &["push", "origin", "main", "tmp"]);
    sh_git(&dir, &["push", "upstream", "main", "tmp"]);
    sh_git(&dir, &["fetch", "--all"]);

    let git = LocalGit::new(&dir);
    let err = git
        .delete_remote_branch_on("nope", "tmp", None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");

    git.delete_remote_branch_on("upstream", "tmp", None)
        .await
        .unwrap();
    let remote = git.refs().await.unwrap().remote;
    assert!(!remote.iter().any(|b| b.name == "upstream/tmp"));
    assert!(
        remote.iter().any(|b| b.name == "origin/tmp"),
        "origin/tmp must survive a delete aimed at upstream"
    );
    // And on the remotes themselves, not just the tracking refs.
    let on = |bare: &std::path::Path| {
        std::process::Command::new("git")
            .args([
                "--git-dir",
                bare.to_str().unwrap(),
                "branch",
                "--list",
                "tmp",
            ])
            .output()
            .unwrap()
            .stdout
    };
    assert!(on(&upstream).is_empty(), "tmp deleted on upstream");
    assert!(!on(&origin).is_empty(), "tmp still on origin");
}

/// `repo` (from [`fixture`], committed) pushed to a bare `origin.git`, plus
/// a second clone `other` that can push behind `repo`'s back.
fn pushed_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let (tmp, dir) = fixture();
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "tidy"]);
    let parent = dir.parent().unwrap().to_path_buf();
    sh_git(&parent, &["init", "--bare", "-b", "main", "origin.git"]);
    let bare = parent.join("origin.git");
    sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sh_git(&dir, &["push", "-u", "origin", "main"]);
    sh_git(&parent, &["clone", bare.to_str().unwrap(), "other"]);
    let other = parent.join("other");
    sh_git(&other, &["config", "user.email", "o@test.local"]);
    sh_git(&other, &["config", "user.name", "Other"]);
    (tmp, dir, other)
}

/// A push the remote rejects because it has commits we don't is a 409
/// naming the fix (pull), never a 502 "provider unavailable"; and a
/// force-with-lease never overwrites commits we haven't seen/integrated.
#[tokio::test]
async fn diverged_push_is_a_conflict_and_lease_protects_unseen_commits() {
    let (_tmp, dir, other) = pushed_fixture();
    write(&other, "theirs.txt", "theirs\n");
    sh_git(&other, &["add", "-A"]);
    sh_git(&other, &["commit", "-m", "theirs"]);
    sh_git(&other, &["push"]);
    write(&dir, "mine.txt", "mine\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "mine"]);

    let git = LocalGit::new(&dir);
    match git.push(None).await {
        Err(Error::Conflict(m)) => assert!(m.starts_with(PUSH_REJECTED), "{m}"),
        other => panic!("expected a 409 push rejection, got {other:?}"),
    }
    // Lease against a stale tracking ref: refused, remote untouched.
    let remote = dir.parent().unwrap().join("origin.git");
    let theirs = rev(&remote, "main");
    match git.push_with(None, None, true).await {
        Err(Error::Conflict(m)) => assert!(m.starts_with(PUSH_LEASE_REFUSED), "{m}"),
        other => panic!("expected a refused lease, got {other:?}"),
    }
    // Fetched but NOT integrated: --force-if-includes still refuses.
    sh_git(&dir, &["fetch", "origin"]);
    match git.push_with(None, None, true).await {
        Err(Error::Conflict(m)) => assert!(m.starts_with(PUSH_LEASE_REFUSED), "{m}"),
        other => panic!("expected --force-if-includes to refuse, got {other:?}"),
    }
    assert_eq!(rev(&remote, "main"), theirs, "the remote kept their commit");
}

/// Rewriting our OWN pushed commit (amend/rebase) is the case force-with-
/// lease exists for: the plain push is rejected, the lease goes through.
#[tokio::test]
async fn force_with_lease_replaces_our_own_rewritten_history() {
    let (_tmp, dir, _other) = pushed_fixture();
    sh_git(&dir, &["commit", "--amend", "-m", "tidy (reworded)"]);
    let git = LocalGit::new(&dir);
    assert!(matches!(git.push(None).await, Err(Error::Conflict(_))));
    git.push_with(None, None, true).await.unwrap();
    let remote = dir.parent().unwrap().join("origin.git");
    assert_eq!(rev(&remote, "main"), rev(&dir, "HEAD"));
    // An explicit branch takes the same path.
    sh_git(&dir, &["commit", "--amend", "-m", "tidy (again)"]);
    git.push_with(None, Some("main"), true).await.unwrap();
    assert_eq!(rev(&remote, "main"), rev(&dir, "HEAD"));
}

/// Pushing a NAMED fresh branch (the Create-PR flow) publishes it WITH
/// tracking — `push origin b` alone succeeds without setting one.
#[tokio::test]
async fn push_named_branch_sets_its_upstream() {
    let (_tmp, dir, _other) = pushed_fixture();
    sh_git(&dir, &["branch", "feat"]);
    // A same-named tag must not make the refspec ambiguous.
    sh_git(&dir, &["tag", "feat"]);
    let git = LocalGit::new(&dir);
    git.push_branch(None, Some("feat")).await.unwrap();
    assert_eq!(rev(&dir, "feat@{upstream}"), rev(&dir, "feat"));
}

/// `commit_only` commits exactly the named paths; the user's other staged
/// work stays staged and out of the commit.
#[tokio::test]
async fn commit_only_leaves_other_staged_work_alone() {
    let (_tmp, dir) = fixture();
    // fixture: staged rename c→d and staged new f.txt.
    write(&dir, "collections/x.json", "{}\n");
    let git = LocalGit::new(&dir);
    let p = vec!["collections/x.json".to_string()];
    git.stage(&p).await.unwrap();
    git.commit_only("collections", &p).await.unwrap();
    let files = String::from_utf8(
        std::process::Command::new("git")
            .current_dir(&dir)
            .args(["show", "--name-only", "--format=", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    assert_eq!(files.trim(), "collections/x.json");
    let st = git.status().await.unwrap();
    assert!(st.changes.iter().any(|c| c.path == "f.txt" && c.staged));
    assert!(st.changes.iter().any(|c| c.path == "d.txt" && c.staged));
}

/// Fresh repo, no commit yet: the graph's log is empty, not an error.
#[tokio::test]
async fn log_of_an_unborn_branch_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    sh_git(tmp.path(), &["init", "-b", "main"]);
    let git = LocalGit::new(tmp.path());
    assert!(git.log(50, 0, false).await.unwrap().is_empty());
    assert!(git.log(50, 0, true).await.unwrap().is_empty());
}

/// `log.showSignature=true` in the user's config must not leak gpg
/// output into the parsed records.
#[tokio::test]
async fn log_ignores_show_signature_config() {
    let (tmp, dir) = fixture();
    // An SSH-signed commit: with showSignature git prints "No signature"
    // (no allowedSigners) to stdout ahead of its record.
    let key = tmp.path().join("sigkey");
    let made = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output();
    if !made.is_ok_and(|o| o.status.success()) {
        eprintln!("ssh-keygen unavailable — skipping");
        return;
    }
    sh_git(&dir, &["config", "gpg.format", "ssh"]);
    sh_git(&dir, &["config", "user.signingkey", key.to_str().unwrap()]);
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-S", "-m", "signed"]);
    sh_git(&dir, &["config", "log.showSignature", "true"]);
    let git = LocalGit::new(&dir);
    let log = git.log(10, 0, false).await.unwrap();
    assert_eq!(log.len(), 3);
    assert!(
        log.iter()
            .all(|c| c.sha.len() == 40 && !c.sha.contains('\n')),
        "{log:?}"
    );
    assert_eq!(log[0].parents, vec![log[1].sha.clone()]);
    assert_eq!(log[0].sha, rev(&dir, "HEAD"));
}

#[tokio::test]
async fn force_push_refuses_a_detached_head() {
    let (_tmp, dir, _other) = pushed_fixture();
    sh_git(&dir, &["checkout", "--detach"]);
    let git = LocalGit::new(&dir);
    assert!(matches!(
        git.push_with(None, None, true).await,
        Err(Error::Conflict(m)) if m.contains("detached")
    ));
}

#[test]
fn auth_failures_name_where_to_fix_them() {
    let e = upstream_err(
        "fatal: Authentication failed for 'https://github.com/o/r.git/'",
        "",
        Some(128),
    );
    assert!(
        matches!(&e, Error::Upstream(m) if m.contains("Git Accounts")),
        "{e:?}"
    );
    let e = upstream_err(
        "git@github.com: Permission denied (publickey).",
        "",
        Some(128),
    );
    assert!(
        matches!(&e, Error::Upstream(m) if m.contains("SSH agent")),
        "{e:?}"
    );
    let e = upstream_err(
        "fatal: unable to access: Could not resolve host",
        "",
        Some(128),
    );
    assert!(
        matches!(&e, Error::Upstream(m) if !m.contains(" — ")),
        "{e:?}"
    );
    // A protected-branch hook refusal is not a non-fast-forward rejection.
    let e = push_err(
        " ! [remote rejected] main -> main (protected branch hook declined)",
        "",
        Some(1),
    );
    assert!(matches!(e, Error::Upstream(_)), "{e:?}");
}

/// Creating a branch whose name exists on origin must start AT the remote
/// tip and track it — a bare `checkout -b` from a stale HEAD makes an
/// upstream-less branch whose first `pull` dies with "no tracking
/// information" (the koala-bigdaddy `develop` incident).
#[tokio::test]
async fn create_named_like_remote_branch_tracks_remote_tip() {
    let (_tmp, dir) = fixture();
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "tidy"]);
    let parent = dir.parent().unwrap();
    sh_git(parent, &["init", "--bare", "origin.git"]);
    let bare = parent.join("origin.git");
    sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sh_git(&dir, &["push", "origin", "main"]);
    // `dev` exists on origin one commit AHEAD of local main (the "stale
    // HEAD" scenario), and only as a remote-tracking ref locally.
    sh_git(&dir, &["checkout", "-b", "dev"]);
    write(&dir, "remote-only.txt", "ahead\n");
    sh_git(&dir, &["add", "remote-only.txt"]);
    sh_git(&dir, &["commit", "-m", "remote ahead"]);
    sh_git(&dir, &["push", "origin", "dev"]);
    sh_git(&dir, &["checkout", "main"]);
    sh_git(&dir, &["branch", "-D", "dev"]);
    sh_git(&dir, &["fetch", "origin"]);

    let git = LocalGit::new(&dir);
    git.checkout("dev", true).await.unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "dev");
    let head = git.run(&["rev-parse", "HEAD"]).await.unwrap();
    let remote_tip = git.run(&["rev-parse", "origin/dev"]).await.unwrap();
    assert_eq!(
        head, remote_tip,
        "branch starts at the remote tip, not stale HEAD"
    );
    let upstream = git
        .run(&["rev-parse", "--abbrev-ref", "dev@{upstream}"])
        .await
        .unwrap();
    assert_eq!(upstream.trim(), "origin/dev", "upstream configured");

    // Same guarantee for the create-in-place path.
    sh_git(&dir, &["checkout", "main"]);
    sh_git(&dir, &["branch", "-D", "dev"]);
    git.create_branch("dev", None, false).await.unwrap();
    let upstream = git
        .run(&["rev-parse", "--abbrev-ref", "dev@{upstream}"])
        .await
        .unwrap();
    assert_eq!(upstream.trim(), "origin/dev", "create_branch tracks too");

    // A genuinely new name still branches from HEAD (no upstream to wire).
    git.checkout("feature/fresh", true).await.unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "feature/fresh");
    // An explicit start_point is respected verbatim (no remote override).
    let base = git.run(&["rev-parse", "main~1"]).await.unwrap();
    git.create_branch("dev2", Some(base.trim()), false)
        .await
        .unwrap();
    let dev2 = git.run(&["rev-parse", "dev2"]).await.unwrap();
    assert_eq!(dev2, base, "explicit start point wins");
}

/// A stale local remote-tracking ref — the branch was deleted on origin
/// elsewhere, without a local `fetch --prune` — must not make a remote
/// delete fail. The UI trusts that tracking ref and offers the delete, but
/// `git push --delete` rejects an already-absent ref ("remote ref does not
/// exist"). The desired end state (no such branch on origin) already holds,
/// so the op must succeed AND prune the phantom ref so the REMOTE list
/// clears it instead of looping on a doomed delete.
#[tokio::test]
async fn delete_remote_branch_tolerates_already_absent_ref() {
    let (_tmp, dir) = fixture();
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "tidy"]);
    let parent = dir.parent().unwrap();
    sh_git(parent, &["init", "--bare", "origin.git"]);
    let bare = parent.join("origin.git");
    sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
    sh_git(&dir, &["push", "origin", "main"]);
    sh_git(&dir, &["branch", "tmp"]);
    sh_git(&dir, &["push", "origin", "tmp"]);
    sh_git(&dir, &["fetch", "origin"]);

    // Someone else deletes `tmp` on origin WITHOUT pruning our tracking ref:
    // origin no longer has it, but `origin/tmp` lingers locally (stale).
    sh_git(&bare, &["update-ref", "-d", "refs/heads/tmp"]);

    let git = LocalGit::new(&dir);
    assert!(
        git.refs()
            .await
            .unwrap()
            .remote
            .iter()
            .any(|b| b.name == "origin/tmp"),
        "stale origin/tmp present before delete"
    );

    // Must NOT error even though origin has no such ref anymore.
    git.delete_remote_branch("tmp", None)
        .await
        .expect("deleting an already-absent remote branch is a no-op success");

    // …and the stale tracking ref is pruned, clearing the UI.
    assert!(
        !git.refs()
            .await
            .unwrap()
            .remote
            .iter()
            .any(|b| b.name == "origin/tmp"),
        "stale origin/tmp pruned after no-op remote delete"
    );
}

/// Minimal repo whose initial (and only) branch is `branch` — the
/// master-only/develop-only shape that used to make `git diff main` exit 128.
fn fixture_on_branch(branch: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", branch]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);
    write(&dir, "a.txt", "hello\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "init"]);
    (tmp, dir)
}

/// A pull whose merge conflicts is an OUTCOME, not a failure: `pull_outcome`
/// returns Ok with the unmerged paths (so the UI can open the resolver),
/// while the strict `pull` still errors — callers that keep mutating the
/// tree afterwards (an auto-stash pull's pop) must not run on a
/// half-merged tree.
#[tokio::test]
async fn conflicting_pull_reports_conflicts_instead_of_failing() {
    let tmp = tempfile::tempdir().unwrap();
    // Bare "origin" + a clone that publishes the first commit.
    let origin = tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    sh_git(&origin, &["init", "--bare", "-b", "main"]);

    let seed = tmp.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    sh_git(&seed, &["init", "-b", "main"]);
    sh_git(&seed, &["config", "user.email", "otto@test.local"]);
    sh_git(&seed, &["config", "user.name", "Otto Test"]);
    sh_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "shared.txt", "line1\nline2\nline3\n");
    sh_git(&seed, &["add", "."]);
    sh_git(&seed, &["commit", "-m", "init"]);
    sh_git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    sh_git(&seed, &["push", "-u", "origin", "main"]);

    // The user's clone.
    let dir = tmp.path().join("work");
    sh_git(
        tmp.path(),
        &["clone", origin.to_str().unwrap(), dir.to_str().unwrap()],
    );
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);

    // Upstream moves (adds a file + edits the shared line)…
    write(&seed, "shared.txt", "line1\nUPSTREAM\nline3\n");
    write(&seed, "pulled_only.txt", "from upstream\n");
    sh_git(&seed, &["add", "-A"]);
    sh_git(&seed, &["commit", "-m", "upstream work"]);
    sh_git(&seed, &["push", "origin", "main"]);

    // …and the user edits the SAME line locally → the pull's merge conflicts.
    write(&dir, "shared.txt", "line1\nLOCAL\nline3\n");
    sh_git(&dir, &["commit", "-am", "local work"]);

    let git = LocalGit::new(&dir);
    let outcome = git
        .pull_outcome(None)
        .await
        .expect("a conflicting pull is an outcome, not an error");
    assert_eq!(outcome.conflicted_files, vec!["shared.txt".to_string()]);
    assert!(git.is_merging().await, "the merge is left in progress");

    // The status the endpoint returns carries the conflict, so the UI can
    // route to the resolver: the unmerged path is kind="conflicted" and the
    // cleanly-merged incoming file shows up staged (this is what looked like
    // mystery WIP).
    let st = git.status().await.unwrap();
    assert!(st
        .changes
        .iter()
        .any(|c| c.path == "shared.txt" && c.kind == "conflicted"));
    assert!(st.changes.iter().any(|c| c.path == "pulled_only.txt"));

    // The strict wrapper still fails so stash-popping callers stay safe.
    assert!(git.pull(None).await.is_err());
}

#[tokio::test]
async fn default_branch_probes_master_when_no_main() {
    let (_tmp, dir) = fixture_on_branch("master");
    let git = LocalGit::new(&dir);
    assert_eq!(git.default_branch().await.as_deref(), Some("master"));
}

#[tokio::test]
async fn resolve_base_explicit_hit_fallback_and_none() {
    let (_tmp, dir) = fixture_on_branch("master");
    let git = LocalGit::new(&dir);
    // Explicit existing ref wins as-is.
    let r = git.resolve_base(Some("master")).await.unwrap();
    assert_eq!(
        (r.diff_ref.as_str(), r.branch.as_str()),
        ("master", "master")
    );
    // A missing explicit ref falls back to the detected default — the exact
    // production failure ("main" on a master-only repo) becomes a success.
    let r = git.resolve_base(Some("main")).await.unwrap();
    assert_eq!(r.branch, "master");
    // want=None resolves the default directly.
    let r = git.resolve_base(None).await.unwrap();
    assert_eq!(r.branch, "master");
    // A SHA verifies too (run-engine passes base commits).
    let sha = git.rev_parse("HEAD").await.unwrap();
    let r = git.resolve_base(Some(&sha)).await.unwrap();
    assert_eq!(r.diff_ref, sha);
}

#[tokio::test]
async fn resolve_base_skips_a_base_that_is_head() {
    // The PR-Reviewer failure: the run declares the PR's OWN branch as the
    // base and that branch is what's checked out, so `git diff base HEAD`
    // is empty and the review passes 100 without running a reviewer.
    let (_tmp, dir) = fixture_on_branch("develop");
    let git = LocalGit::new(&dir);
    sh_git(&dir, &["checkout", "-b", "feature/X-1"]);
    write(&dir, "b.txt", "change\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "work"]);

    // Attached: base == the current branch → fall through to `develop`.
    let r = git.resolve_base(Some("feature/X-1")).await.unwrap();
    assert_eq!(r.branch, "develop", "base equal to HEAD must not win");
    assert!(!git
        .diff_text_against(&r.diff_ref)
        .await
        .unwrap()
        .trim()
        .is_empty());

    // Detached at the same commit — how the PR-Reviewer checks a PR out.
    let sha = git.rev_parse("HEAD").await.unwrap();
    sh_git(&dir, &["checkout", "--detach", &sha]);
    let r = git.resolve_base(Some("feature/X-1")).await.unwrap();
    assert_eq!(r.branch, "develop");

    // A branch genuinely identical to its base still resolves (nothing
    // narrows): every candidate is HEAD, so the HEAD-equal one is kept.
    sh_git(&dir, &["checkout", "develop"]);
    let r = git.resolve_base(Some("develop")).await.unwrap();
    assert_eq!(r.branch, "develop");
}

/// `main` moved on after the branch point: the review/PR diff must hold
/// ONLY the branch's own change (merge-base), not main's later commits as
/// reverse hunks; the review variant also carries untracked new files.
/// S2-19: the PR review ref is keyed by PR + head, and only a "missing
/// ref" fetch failure is treated as "try the next provider ref".
#[test]
fn pr_review_ref_is_keyed_by_head_and_missing_ref_is_classified() {
    assert_eq!(pr_review_ref(7, None), "refs/otto/pr-review/7");
    assert_eq!(
        pr_review_ref(7, Some("ABCDEF0123456789abcdef")),
        "refs/otto/pr-review/7-abcdef012345"
    );
    assert_ne!(
        pr_review_ref(7, Some("aaaa")),
        pr_review_ref(7, Some("bbbb"))
    );
    assert!(is_missing_remote_ref(
        "fatal: couldn't find remote ref refs/pull/7/head"
    ));
    assert!(!is_missing_remote_ref(
        "remote: Invalid username or token.\nfatal: Authentication failed for 'https://x'"
    ));
}

/// S2-19: an unreachable origin surfaces the real fetch error, not the
/// "no pull/merge-request ref" text.
#[tokio::test]
async fn fetch_pr_head_reports_the_real_fetch_error() {
    let (tmp, dir) = bare_main_repo();
    let missing = tmp.path().join("no-such-remote.git");
    sh_git(
        &dir,
        &["remote", "add", "origin", missing.to_str().unwrap()],
    );
    let err = LocalGit::new(&dir)
        .fetch_pr_head(3, None, None)
        .await
        .unwrap_err()
        .to_string();
    assert!(!err.contains("no pull/merge-request ref"), "{err}");
}

fn bare_main_repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);
    write(&dir, "seed.txt", "seed\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "init"]);
    (tmp, dir)
}

/// S2-09: ONE oversized untracked file is clipped at the per-file cap and
/// marked `(diff omitted: …)` (it used to be appended whole).
#[tokio::test]
async fn review_diff_clips_an_oversized_untracked_file_with_a_marker() {
    let (_tmp, dir) = bare_main_repo();
    let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n";
    write(&dir, "dump.txt", &line.repeat(48 * 1024)); // ~3 MiB
    write(&dir, "small.rs", "fn small() {}\n");
    let r = LocalGit::new(&dir).review_diff_text("main").await.unwrap();
    assert!(r.len() < 2 * 1024 * 1024, "clipped: {} bytes", r.len());
    assert!(r.contains("+fn small() {}"), "small file still reviewed");
    assert!(
        r.lines()
            .any(|l| l.starts_with(UNTRACKED_OMITTED_MARKER) && l.contains("dump.txt")),
        "clipped file is marked"
    );
}

/// S2-09: past the total budget, the remaining untracked files are NOT
/// dropped silently — each gets a marker line.
#[tokio::test]
async fn review_diff_marks_untracked_files_past_the_budget() {
    let (_tmp, dir) = bare_main_repo();
    let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde\n";
    let big = line.repeat(17 * 1024); // ~1.06 MiB → clipped to the 1 MiB cap
    for i in 0..10 {
        write(&dir, &format!("f{i:02}.txt"), &big);
    }
    let r = LocalGit::new(&dir).review_diff_text("main").await.unwrap();
    assert!(r.len() <= 9 * 1024 * 1024, "{} bytes", r.len());
    let skipped = r
        .lines()
        .filter(|l| l.starts_with(UNTRACKED_OMITTED_MARKER) && l.contains("size cap reached"))
        .count();
    assert!(skipped >= 1, "over-budget files are marked");
    assert!(r.contains("+++ b/f09.txt"), "the last file is named");
}

#[tokio::test]
async fn diff_text_against_uses_merge_base_and_review_adds_untracked() {
    let (_tmp, dir) = fixture_on_branch("main");
    let git = LocalGit::new(&dir);
    sh_git(&dir, &["checkout", "-b", "feature"]);
    write(&dir, "feature.txt", "mine\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "feature work"]);
    // main advances after the branch point.
    sh_git(&dir, &["checkout", "main"]);
    write(&dir, "main_only.txt", "landed on main later\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "main moved"]);
    sh_git(&dir, &["checkout", "feature"]);

    let d = git.diff_text_against("main").await.unwrap();
    assert!(d.contains("feature.txt"), "{d}");
    assert!(
        !d.contains("main_only.txt"),
        "main's later commit leaked: {d}"
    );
    let (capped, _) = git.diff_text_capped(Some("main"), 1 << 20).await.unwrap();
    assert!(!capped.contains("main_only.txt"), "{capped}");

    write(&dir, "brand_new.rs", "fn new() {}\n");
    assert!(!git
        .diff_text_against("main")
        .await
        .unwrap()
        .contains("brand_new.rs"));
    let r = git.review_diff_text("main").await.unwrap();
    assert!(
        r.contains("brand_new.rs") && r.contains("+fn new() {}"),
        "{r}"
    );
    assert!(!r.contains("main_only.txt"));
}

#[tokio::test]
async fn resolve_base_error_lists_candidates() {
    // A repo with no commits has no branches at all — nothing can resolve.
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    let git = LocalGit::new(&dir);
    let err = git
        .resolve_base(Some("develop"))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("develop"), "error names what was tried: {err}");
}

#[tokio::test]
async fn resolve_base_rejects_option_injection() {
    let (_tmp, dir) = fixture_on_branch("master");
    let git = LocalGit::new(&dir);
    // An option-looking "ref" from untrusted input must not be treated as a
    // git flag; it fails verification and detection falls back to master.
    let r = git.resolve_base(Some("--output=/tmp/pwn")).await.unwrap();
    assert_eq!(r.branch, "master");
    assert!(!std::path::Path::new("/tmp/pwn").exists());
}

#[tokio::test]
async fn worktree_for_branch_finds_checkout() {
    let (tmp, dir) = fixture_on_branch("master");
    let git = LocalGit::new(&dir);
    let wt = tmp.path().join("wt-feature");
    sh_git(
        &dir,
        &["worktree", "add", "-b", "feature/x", wt.to_str().unwrap()],
    );
    let found = git.worktree_for_branch("feature/x").await.unwrap();
    assert_eq!(
        std::fs::canonicalize(&found).unwrap(),
        std::fs::canonicalize(&wt).unwrap()
    );
    // The main checkout itself is a worktree entry too.
    let main_wt = git.worktree_for_branch("master").await.unwrap();
    assert_eq!(
        std::fs::canonicalize(&main_wt).unwrap(),
        std::fs::canonicalize(&dir).unwrap()
    );
    assert!(git.worktree_for_branch("nope").await.is_none());
}

/// Regression: discarding a STAGED RENAME must restore the file at its
/// original path — restoring only the new name removed the file entirely
/// (absent at HEAD) and left the old name staged-deleted.
#[tokio::test]
async fn discard_staged_rename_restores_original_file() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);

    git.discard(&["d.txt".to_string()]).await.unwrap();

    let original = "carrot content that is long enough to track renames\n";
    assert_eq!(
        std::fs::read_to_string(dir.join("c.txt")).unwrap(),
        original,
        "file restored at its ORIGINAL path with original content"
    );
    assert!(!dir.join("d.txt").exists(), "new name gone after discard");
    let st = git.status().await.unwrap();
    assert!(
        !st.changes
            .iter()
            .any(|c| c.path == "c.txt" || c.path == "d.txt"),
        "rename fully undone — no residual staged entries: {:?}",
        st.changes
    );
}

/// The Unstaged list's Discard (`keep_staged`) reverts only the worktree
/// side: staged hunks of a partially staged file, a staged new file's
/// staged content and a staged-only file all survive; untracked files go.
#[tokio::test]
async fn discard_keep_staged_only_reverts_the_worktree_side() {
    let (_tmp, dir) = fixture();
    // a.txt: stage the "appended" edit, then edit it again (partial, MM).
    sh_git(&dir, &["add", "a.txt"]);
    let staged_a = "alpha line 1\nalpha CHANGED 2\nalpha line 3\nappended\n";
    write(&dir, "a.txt", &format!("{staged_a}scratch\n"));
    // f.txt: staged new file with an unstaged edit on top (AM).
    write(&dir, "f.txt", "fresh\nscratch\n");
    let git = LocalGit::new(&dir);
    let all: Vec<String> = ["a.txt", "f.txt", "d.txt", "e.txt"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    git.discard_with(&all, true).await.unwrap();

    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        staged_a
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("f.txt")).unwrap(),
        "fresh\n"
    );
    assert!(dir.join("d.txt").exists(), "staged-only rename untouched");
    assert!(!dir.join("e.txt").exists(), "untracked file removed");
    let st = git.status().await.unwrap();
    let by = |p: &str| st.changes.iter().find(|c| c.path == p).cloned();
    let a = by("a.txt").expect("a.txt still staged");
    assert!(a.staged && !a.unstaged, "{a:?}");
    let f = by("f.txt").expect("f.txt still staged");
    assert!(f.staged && !f.unstaged, "{f:?}");
    assert!(by("d.txt").is_some_and(|c| c.staged));

    // Nothing unstaged left among these → a 409, not a silent success.
    assert!(matches!(
        git.discard_with(&["a.txt".to_string()], true).await,
        Err(Error::Conflict(_))
    ));
}

/// Amend with an EMPTY message folds staged changes into HEAD and keeps
/// the previous commit message (`--amend --no-edit`).
#[tokio::test]
async fn amend_empty_message_keeps_previous_message() {
    let (_tmp, dir) = fixture();
    let git = LocalGit::new(&dir);

    // Fold the staged rename + add into "second commit".
    let before = git.log(1, 0, false).await.unwrap()[0].clone();
    let sha = git.commit("", true).await.unwrap();
    assert_ne!(sha, before.sha, "amend rewrote HEAD");
    let after = git.log(1, 0, false).await.unwrap()[0].clone();
    assert_eq!(after.subject, "second commit", "message preserved");
    // Empty message WITHOUT amend still rejects.
    assert!(git.commit("", false).await.is_err());
}

/// A merge commit's diff must show the changes it brought in (first-parent
/// diff); the combined default rendered an empty/garbled diff.
#[tokio::test]
async fn merge_commit_diff_shows_first_parent_changes() {
    let (_tmp, dir) = fixture();
    // Clean the dirty fixture state so branching is simple.
    sh_git(&dir, &["checkout", "--", "."]);
    sh_git(&dir, &["stash", "--include-untracked"]);
    sh_git(&dir, &["checkout", "-b", "feature"]);
    write(&dir, "merged.txt", "from the feature branch\n");
    sh_git(&dir, &["add", "merged.txt"]);
    sh_git(&dir, &["commit", "-m", "feature work"]);
    sh_git(&dir, &["checkout", "main"]);
    sh_git(&dir, &["merge", "--no-ff", "--no-edit", "feature"]);

    let git = LocalGit::new(&dir);
    let head = git.log(1, 0, false).await.unwrap()[0].clone();
    assert_eq!(head.parents.len(), 2, "fixture produced a merge commit");
    let diff = git
        .diff(DiffTarget::Commit(head.sha.clone()), None)
        .await
        .unwrap();
    assert!(
        diff.files.iter().any(|f| f.path == "merged.txt"),
        "merge diff lists the merged file: {:?}",
        diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

/// `refs_with_base` flags a branch merged into the base and leaves an
/// unmerged one alone, while never flagging the base branch or the current
/// branch itself.
#[tokio::test]
async fn refs_flag_branches_merged_into_base() {
    let (_tmp, dir) = fixture();
    // `main` has two commits from the fixture. A `merged` branch forked and
    // folded straight back (fast-forward → contained in main); a `feature`
    // branch adds its own commit and stays ahead.
    sh_git(&dir, &["checkout", "-b", "merged"]);
    // No new commit: `merged` tip == main tip → contained in main.
    sh_git(&dir, &["checkout", "main"]);
    sh_git(&dir, &["checkout", "-b", "feature"]);
    write(&dir, "feat.txt", "feature work\n");
    sh_git(&dir, &["add", "feat.txt"]);
    sh_git(&dir, &["commit", "-m", "feature commit"]);
    sh_git(&dir, &["checkout", "main"]);

    let git = LocalGit::new(&dir);
    let refs = git.refs_with_base(Some("main")).await.unwrap();
    assert_eq!(refs.base_branch.as_deref(), Some("main"));
    let by = |name: &str| {
        refs.local
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("branch {name} present"))
            .merged_into_base
    };
    assert!(by("merged"), "a branch contained in base is flagged");
    assert!(!by("feature"), "a branch ahead of base is not flagged");
    assert!(!by("main"), "the base branch is never flagged");
}

/// Git-flow: `origin/HEAD` (the cleanup base) is `main`, but work lands in
/// `develop` first. `merged_into` names EVERY integration branch holding the
/// tip, so a release merged into develop still shows where it landed even
/// though `merged_into_base` (main) is false. The integration branches never
/// flag each other (main ⊂ develop is noise, not a cleanup hint).
#[tokio::test]
async fn refs_list_every_integration_branch_holding_a_tip() {
    let (_tmp, dir) = fixture();
    sh_git(&dir, &["branch", "old"]); // at main's tip → in main AND develop
    sh_git(&dir, &["checkout", "-b", "develop"]);
    write(&dir, "dev.txt", "dev\n");
    sh_git(&dir, &["add", "dev.txt"]);
    sh_git(&dir, &["commit", "-m", "develop commit"]);
    sh_git(&dir, &["checkout", "-b", "release/1.0"]);
    write(&dir, "rel.txt", "release\n");
    sh_git(&dir, &["add", "rel.txt"]);
    sh_git(&dir, &["commit", "-m", "release commit"]);
    sh_git(&dir, &["checkout", "develop"]);
    sh_git(&dir, &["merge", "--no-ff", "--no-edit", "release/1.0"]);
    sh_git(&dir, &["checkout", "-b", "feature/x"]);
    write(&dir, "x.txt", "x\n");
    sh_git(&dir, &["add", "x.txt"]);
    sh_git(&dir, &["commit", "-m", "feature commit"]);
    sh_git(&dir, &["checkout", "develop"]);
    // A remote twin of the release, so remote rows get the same answer.
    sh_git(
        &dir,
        &[
            "update-ref",
            "refs/remotes/origin/release/1.0",
            "release/1.0",
        ],
    );
    sh_git(
        &dir,
        &["update-ref", "refs/remotes/origin/develop", "develop"],
    );

    let git = LocalGit::new(&dir);
    let refs = git.refs_with_base(Some("main")).await.unwrap();
    let landed = |list: &[RefBranch], name: &str| {
        let mut v = list
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("branch {name} present"))
            .merged_into
            .clone();
        v.sort();
        v
    };
    assert_eq!(landed(&refs.local, "release/1.0"), ["develop"]);
    assert_eq!(landed(&refs.local, "old"), ["develop", "main"]);
    assert!(
        landed(&refs.local, "feature/x").is_empty(),
        "ahead of every base"
    );
    assert!(
        landed(&refs.local, "develop").is_empty(),
        "integration branch"
    );
    assert!(landed(&refs.local, "main").is_empty(), "integration branch");
    assert_eq!(landed(&refs.remote, "origin/release/1.0"), ["develop"]);
    assert!(
        landed(&refs.remote, "origin/develop").is_empty(),
        "integration twin"
    );
    let release = refs.local.iter().find(|b| b.name == "release/1.0").unwrap();
    assert!(
        !release.merged_into_base,
        "base (main) does not hold the release"
    );
}

/// Untracked files with non-ASCII names must appear in the Working diff —
/// quotePath escaping made `--no-index` miss them silently.
#[tokio::test]
async fn untracked_non_ascii_filename_appears_in_working_diff() {
    let (_tmp, dir) = fixture();
    write(&dir, "café.txt", "accented\n");
    let git = LocalGit::new(&dir);
    let diff = git.diff(DiffTarget::Working, None).await.unwrap();
    assert!(
        diff.files.iter().any(|f| f.path.contains("café")),
        "non-ASCII untracked file present: {:?}",
        diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

/// A repo with `n` linear commits ("c1".."cn") on `main`.
fn fixture_n_commits(n: usize) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);
    for i in 1..=n {
        write(&dir, "f.txt", &format!("line {i}\n"));
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", &format!("c{i}")]);
    }
    (tmp, dir)
}

/// `limit = 0` means the WHOLE history — the graph must be able to walk back
/// to the root commit, which a hard `-n` cap silently prevented.
#[tokio::test]
async fn log_limit_zero_returns_full_history() {
    let (_tmp, dir) = fixture_n_commits(30);
    let git = LocalGit::new(&dir);

    let all = git.log(0, 0, false).await.unwrap();
    assert_eq!(all.len(), 30, "limit=0 returns every commit");
    assert_eq!(all[0].subject, "c30", "newest first");
    assert_eq!(all[29].subject, "c1", "reaches the root commit");
}

/// Paging with skip/limit must tile history exactly once — no gap, no
/// overlap — since the graph appends each page onto the previous one.
#[tokio::test]
async fn log_paging_tiles_history_without_gaps() {
    let (_tmp, dir) = fixture_n_commits(25);
    let git = LocalGit::new(&dir);

    let mut paged = Vec::new();
    let mut skip = 0u32;
    loop {
        let page = git.log(10, skip, true).await.unwrap();
        if page.is_empty() {
            break;
        }
        skip += page.len() as u32;
        paged.extend(page);
    }

    let full = git.log(0, 0, true).await.unwrap();
    assert_eq!(paged.len(), 25, "paging reaches every commit");
    assert_eq!(
        paged.iter().map(|c| &c.sha).collect::<Vec<_>>(),
        full.iter().map(|c| &c.sha).collect::<Vec<_>>(),
        "paged order matches an unpaged read"
    );
}

/// Branch/tag rows carry the sha they point at so the UI can jump to a ref
/// whose commit is NOT in the currently loaded page. Annotated tags must
/// report the dereferenced COMMIT, never the tag object's own sha.
#[tokio::test]
async fn refs_carry_commit_shas_for_branches_and_tags() {
    let (_tmp, dir) = fixture_n_commits(3);
    let git = LocalGit::new(&dir);

    // A lightweight tag and an annotated tag on the FIRST commit.
    let head = git.log(0, 0, false).await.unwrap();
    let root = head.last().unwrap().sha.clone();
    sh_git(&dir, &["tag", "light", &root]);
    sh_git(&dir, &["tag", "-a", "annotated", "-m", "ann", &root]);

    let refs = git.refs().await.unwrap();

    let main = refs.local.iter().find(|b| b.name == "main").unwrap();
    assert_eq!(main.sha, head[0].sha, "branch sha is its tip commit");

    for name in ["light", "annotated"] {
        let t = refs
            .tags
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("tag {name} present"));
        assert_eq!(t.sha, root, "tag {name} resolves to the tagged COMMIT");
    }
}

/// A branch sharing its name with a tag: `%(refname:short)` disambiguates
/// it to `heads/v1.0`, which the PR form then sent as the head branch.
#[tokio::test]
async fn branches_keep_plain_names_when_a_tag_shares_one() {
    let (_tmp, dir) = fixture_n_commits(1);
    sh_git(&dir, &["branch", "v1.0"]);
    sh_git(&dir, &["tag", "v1.0"]);
    let names: Vec<String> = LocalGit::new(&dir)
        .branches()
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.contains(&"v1.0".to_string()), "{names:?}");
    assert!(!names.iter().any(|n| n.starts_with("heads/")), "{names:?}");
}

#[tokio::test]
async fn refs_keep_local_names_when_remote_names_collide() {
    let (_tmp, dir) = fixture_n_commits(2);
    sh_git(&dir, &["branch", "origin/collision"]);
    sh_git(
        &dir,
        &["update-ref", "refs/remotes/origin/collision", "HEAD~1"],
    );
    let git = LocalGit::new(&dir);
    let refs = git.refs().await.unwrap();
    let local = refs
        .local
        .iter()
        .find(|b| b.name == "origin/collision")
        .expect("local branch keeps its usable checkout name");
    let remote = refs
        .remote
        .iter()
        .find(|b| b.name == "origin/collision")
        .unwrap();
    assert!(!local.remote);
    assert!(remote.remote);
    assert_ne!(local.sha, remote.sha);
    assert!(
        local.merged_into_base,
        "normalized names must still match cleanup membership"
    );
}

// ── R0: a branch switch never pulls ─────────────────────────────────────

/// Bare origin + a clone whose `develop` is BOTH ahead of and behind
/// `origin/develop`, with a dirty overlapping file. The user's bug: this is
/// exactly the shape where `pull --no-rebase` created an unasked-for merge
/// commit during a plain switch.
/// A clone has the symbolic `refs/remotes/origin/HEAD`; `%(refname:short)`
/// abbreviates it to bare "origin", which used to slip past the "/HEAD"
/// filter and render as a stray "origin" leaf under REMOTE.
#[tokio::test]
async fn refs_skip_the_symbolic_remote_head() {
    let (_tmp, dir) = diverged_fixture();
    // Fails the test if the fixture lacks origin/HEAD (proves nothing then).
    sh_git(&dir, &["symbolic-ref", "refs/remotes/origin/HEAD"]);
    let names: Vec<String> = LocalGit::new(&dir)
        .refs()
        .await
        .unwrap()
        .remote
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert!(names.contains(&"origin/main".to_string()), "{names:?}");
    assert!(
        !names.iter().any(|n| n == "origin" || n.ends_with("/HEAD")),
        "no HEAD pseudo-branch in {names:?}"
    );
}

fn diverged_fixture() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    std::fs::create_dir(&origin).unwrap();
    sh_git(&origin, &["init", "--bare", "-b", "main"]);

    let seed = tmp.path().join("seed");
    std::fs::create_dir(&seed).unwrap();
    sh_git(&seed, &["init", "-b", "main"]);
    sh_git(&seed, &["config", "user.email", "otto@test.local"]);
    sh_git(&seed, &["config", "user.name", "Otto Test"]);
    sh_git(&seed, &["config", "commit.gpgsign", "false"]);
    write(&seed, "shared.txt", "line1\nline2\nline3\n");
    sh_git(&seed, &["add", "."]);
    sh_git(&seed, &["commit", "-m", "init"]);
    sh_git(&seed, &["branch", "develop"]);
    sh_git(
        &seed,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    sh_git(&seed, &["push", "-u", "origin", "main", "develop"]);

    let dir = tmp.path().join("work");
    sh_git(
        tmp.path(),
        &["clone", origin.to_str().unwrap(), dir.to_str().unwrap()],
    );
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);

    // Local `develop` gets a commit of its own, then we go back to main.
    sh_git(&dir, &["checkout", "-q", "develop"]);
    write(&dir, "local_only.txt", "mine\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "local develop work"]);
    sh_git(&dir, &["checkout", "-q", "main"]);

    // …and origin/develop moves on, so the branch is behind too.
    sh_git(&seed, &["checkout", "-q", "develop"]);
    write(&seed, "upstream_only.txt", "theirs\n");
    sh_git(&seed, &["add", "-A"]);
    sh_git(&seed, &["commit", "-m", "upstream develop work"]);
    sh_git(&seed, &["push", "origin", "develop"]);
    sh_git(&dir, &["fetch", "-q", "origin"]);

    (tmp, dir)
}

fn rev(dir: &Path, spec: &str) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", spec])
        .output()
        .expect("spawn git rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn count(dir: &Path, range: &str) -> u32 {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["rev-list", "--count", range])
        .output()
        .expect("spawn git rev-list");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0)
}

#[tokio::test]
async fn checkout_autostash_never_pulls() {
    let (_tmp, dir) = diverged_fixture();
    let git = LocalGit::new(&dir);
    let develop_before = rev(&dir, "refs/heads/develop");
    let behind_before = count(&dir, "develop..origin/develop");
    assert!(behind_before > 0, "fixture: develop must start behind");

    // Dirty the file that DIFFERS across the switch so git itself would
    // refuse the plain checkout.
    write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");

    let outcome = git.checkout_autostash("develop", false).await.unwrap();
    assert!(outcome.stashed, "a dirty tree must be stashed");
    assert!(!outcome.pop_conflicted);

    assert_eq!(git.current_branch().await.unwrap(), "develop");
    assert_eq!(
        rev(&dir, "HEAD"),
        develop_before,
        "the switch must not move develop — nothing was pulled or merged"
    );
    assert_eq!(
        count(&dir, "develop..origin/develop"),
        behind_before,
        "still behind origin/develop: a switch never fetches"
    );
    assert!(!dir.join(".git/MERGE_HEAD").exists(), "no merge started");
    assert!(git.op_in_progress().await.is_none());
    assert_eq!(
        std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
        "line1\nDIRTY\nline3\n",
        "the stashed change is restored on the new branch"
    );
    assert_eq!(
        git.stash_list().await.unwrap().len(),
        1,
        "restoration retains its recovery backup"
    );
}

#[tokio::test]
async fn checkout_autostash_creates_tracking_branch_when_remote_only() {
    let (_tmp, dir) = diverged_fixture();
    // A branch that exists ONLY on origin.
    sh_git(
        &dir,
        &[
            "update-ref",
            "refs/remotes/origin/feature",
            "origin/develop",
        ],
    );
    write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");

    let git = LocalGit::new(&dir);
    let outcome = git.checkout_autostash("feature", true).await.unwrap();
    assert!(outcome.stashed);
    assert_eq!(git.current_branch().await.unwrap(), "feature");

    let upstream = git
        .run(&["rev-parse", "--abbrev-ref", "feature@{u}"])
        .await
        .unwrap();
    assert_eq!(
        upstream.trim(),
        "origin/feature",
        "created branch tracks origin"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
        "line1\nDIRTY\nline3\n"
    );
}

#[tokio::test]
async fn checkout_autostash_index_conflict_retains_staged_recovery() {
    let (_tmp, dir) = diverged_fixture();
    sh_git(&dir, &["checkout", "-q", "develop"]);
    write(&dir, "shared.txt", "line1\nDEVELOP\nline3\n");
    sh_git(&dir, &["commit", "-am", "develop changes staged line"]);
    sh_git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "shared.txt", "line1\nSTAGED\nline3\n");
    sh_git(&dir, &["add", "shared.txt"]);
    let git = LocalGit::new(&dir);
    let result = git.checkout_autostash("develop", false).await;
    match result {
        Err(Error::Conflict(message)) => assert!(message.contains("git stash pop --index")),
        Ok(outcome) => assert!(outcome.pop_conflicted),
        other => panic!("expected retained-stash recovery: {other:?}"),
    }
    assert_eq!(git.stash_list().await.unwrap().len(), 1);
    assert!(git
        .run_read(&["show", "stash@{0}^2:shared.txt"])
        .await
        .unwrap()
        .contains("STAGED"));
}

#[tokio::test]
async fn checkout_autostash_preserves_index_on_success_and_rollback() {
    for branch in ["same-head", "missing-branch"] {
        let (_tmp, dir) = diverged_fixture();
        sh_git(&dir, &["branch", "same-head"]);
        write(&dir, "shared.txt", "line1\nSTAGED\nline3\n");
        sh_git(&dir, &["add", "shared.txt"]);
        write(&dir, "shared.txt", "line1\nUNSTAGED\nline3\n");
        let git = LocalGit::new(&dir);
        let cached = git.run_read(&["diff", "--cached"]).await.unwrap();
        let working = git.run_read(&["diff"]).await.unwrap();
        let result = git.checkout_autostash(branch, false).await;
        assert_eq!(result.is_ok(), branch == "same-head");
        assert_eq!(git.run_read(&["diff", "--cached"]).await.unwrap(), cached);
        assert_eq!(git.run_read(&["diff"]).await.unwrap(), working);
        assert_eq!(
            git.stash_list().await.unwrap().len(),
            1,
            "restoration retains its recovery backup"
        );
    }
}

#[tokio::test]
async fn checkout_autostash_restores_stash_when_switch_fails() {
    let (_tmp, dir) = diverged_fixture();
    write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");
    let git = LocalGit::new(&dir);

    let err = git
        .checkout_autostash("no-such-branch", false)
        .await
        .unwrap_err();
    assert!(
        !matches!(err, Error::Invalid(_)),
        "git's own refusal: {err:?}"
    );

    assert_eq!(
        git.current_branch().await.unwrap(),
        "main",
        "tree untouched"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
        "line1\nDIRTY\nline3\n",
        "a failed switch pops the stash back"
    );
    assert_eq!(
        git.stash_list().await.unwrap().len(),
        1,
        "restoration retains its recovery backup"
    );
}

#[tokio::test]
async fn checkout_autostash_pop_conflict_is_ok() {
    let (_tmp, dir) = diverged_fixture();
    // develop changes the same line the user is editing → the pop conflicts.
    sh_git(&dir, &["checkout", "-q", "develop"]);
    write(&dir, "shared.txt", "line1\nDEVELOP\nline3\n");
    sh_git(&dir, &["commit", "-am", "develop edits the shared line"]);
    sh_git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");

    let git = LocalGit::new(&dir);
    let outcome = git.checkout_autostash("develop", false).await.unwrap();
    assert!(outcome.stashed && outcome.pop_conflicted, "{outcome:?}");

    let st = git.status().await.unwrap();
    assert!(
        st.changes
            .iter()
            .any(|c| c.path == "shared.txt" && c.kind == "conflicted"),
        "the conflicted pop is visible in the status: {:?}",
        st.changes
    );
    assert!(
        st.op_in_progress.is_none(),
        "a stash pop is not a merge/rebase — no op to abort"
    );
}

#[tokio::test]
async fn checkout_autostash_untracked_collision_is_409_and_keeps_stash() {
    let (_tmp, dir) = diverged_fixture();
    // `develop` TRACKS a file the user has as an untracked local file: the
    // switch works, the pop refuses to clobber it.
    sh_git(&dir, &["checkout", "-q", "develop"]);
    write(&dir, "collide.txt", "tracked on develop\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-m", "develop adds collide.txt"]);
    sh_git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "collide.txt", "untracked locally\n");

    let git = LocalGit::new(&dir);
    let err = git.checkout_autostash("develop", false).await.unwrap_err();
    match &err {
        Error::Conflict(m) => {
            assert!(m.contains("switched to develop"), "{m}");
            assert!(m.contains("git stash pop"), "{m}");
        }
        other => panic!("expected a 409 Conflict, got {other:?}"),
    }
    assert_eq!(git.current_branch().await.unwrap(), "develop");
    assert_eq!(
        git.stash_list().await.unwrap().len(),
        1,
        "the stash entry is kept so nothing is lost"
    );
}

/// `git stash push` exits 0 with "No local changes to save" when porcelain
/// is non-empty but nothing is stashable (a submodule with a dirty work
/// tree is the real-world shape). Nothing of OURS is on the stack, so the
/// switch must not pop — popping would take the user's own `stash@{0}`.
#[tokio::test]
async fn checkout_autostash_does_not_pop_when_nothing_was_stashed() {
    let (tmp, dir) = diverged_fixture();
    // The user's own stash, the entry a bogus pop would steal.
    write(&dir, "shared.txt", "line1\nUSER STASH\nline3\n");
    sh_git(&dir, &["stash", "push", "-m", "user"]);
    // A dirty-but-(pretend)-unstashable tree: porcelain is non-empty, so
    // `checkout_autostash` takes the stash branch.
    write(&dir, "scratch.txt", "mine\n");

    // A real submodule fixture reproduces this without a shim, but needs
    // `protocol.file.allow=always` and a nested clone; forcing the ONE
    // outcome that matters is deterministic and git-version independent.
    let sh = shim(
        tmp.path(),
        "nostash-git",
        "if [ \"$1\" = stash ] && [ \"$2\" = push ]; then\n\
         echo 'No local changes to save'\n\
         exit 0\n\
         fi\n\
         exec git \"$@\"",
    );
    let git = LocalGit::new(&dir).with_git_bin(&sh);

    let outcome = git.checkout_autostash("develop", false).await.unwrap();
    assert_eq!(git.current_branch().await.unwrap(), "develop");

    let stashes = git.stash_list().await.unwrap();
    assert_eq!(
        stashes.len(),
        1,
        "the user's own stash must survive the switch untouched: {stashes:?}"
    );
    assert!(stashes[0].message.contains("user"), "{:?}", stashes[0]);
    assert_eq!(
        std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
        "line1\nline2\nline3\n",
        "the user's stash was not applied onto the new branch"
    );
    assert!(
        !outcome.stashed,
        "nothing was stashed, so nothing may be reported as stashed: {outcome:?}"
    );
    assert!(!outcome.pop_conflicted);
    let st = git.status().await.unwrap();
    assert!(
        !st.changes.iter().any(|c| c.kind == "conflicted"),
        "a stash we never pushed cannot conflict: {:?}",
        st.changes
    );
}

// ── R1: hardening ───────────────────────────────────────────────────────

#[tokio::test]
async fn diff_target_refuses_option_like_revs() {
    assert!(matches!(
        DiffTarget::parse("commit:--output=/tmp/pwn"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        DiffTarget::parse("range:--upload-pack=x..HEAD"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        DiffTarget::parse("range:HEAD..--x"),
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        DiffTarget::parse("range:a b..HEAD"),
        Err(Error::Invalid(_))
    ));

    // A real sha still parses AND still diffs.
    let (_tmp, dir) = fixture_n_commits(2);
    let git = LocalGit::new(&dir);
    let sha = git.log(1, 0, false).await.unwrap()[0].sha.clone();
    let target = DiffTarget::parse(&format!("commit:{sha}")).unwrap();
    assert_eq!(target, DiffTarget::Commit(sha));
    assert!(!git.diff(target, None).await.unwrap().files.is_empty());
}

#[test]
fn clone_url_validation() {
    for ok in [
        "https://h/o/r.git",
        "http://h/o/r",
        "ssh://git@h/o/r",
        "git://h/o/r.git",
        "git@h:o/r.git",
        "git@github.com:otto/otto_os.git",
    ] {
        assert!(validate_remote_url(ok).is_ok(), "should accept {ok}");
    }
    for bad in [
        "",
        "   ",
        "-c",
        "--upload-pack=x",
        "file:///tmp/x",
        "/tmp/x",
        "https://h/o r",
        "a\nb",
        "https://h",
        "https://h/",
        "ssh://",
        "ftp://h/o/r",
        "git@h:",
        "git@h:-x",
    ] {
        assert!(validate_remote_url(bad).is_err(), "should refuse {bad:?}");
    }
}

#[tokio::test]
async fn worktree_probes_distinguish_known_clean_and_dirty() {
    let (tmp, dir) = fixture_on_branch("main");
    let git = LocalGit::new(&dir);
    let wt = tmp.path().join("linked");
    git.worktree_add(wt.to_str().unwrap(), "other", "HEAD")
        .await
        .unwrap();
    write(&wt, "untracked.txt", "changes");
    let rows = git.worktree_list().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(!rows[0].dirty);
    assert!(rows[1].dirty);
    assert_eq!(serde_json::to_value(&rows[0]).unwrap()["dirty_known"], true);
    assert_eq!(serde_json::to_value(&rows[1]).unwrap()["dirty_known"], true);
}

#[tokio::test]
async fn worktree_probe_failure_is_unknown_and_uses_configured_git() {
    let (tmp, dir) = fixture_on_branch("main");
    let bin = shim(
        tmp.path(),
        "probe-git",
        "if [ \"$1\" = status ]; then exit 1; fi\nexec git \"$@\"",
    );
    let rows = LocalGit::new(&dir)
        .with_git_bin(bin)
        .worktree_list()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].dirty);
    assert_eq!(
        serde_json::to_value(&rows[0]).unwrap()["dirty_known"],
        false
    );
}

/// Write an executable `sh` shim and return its path. `body` runs with the
/// repo as cwd (`base_cmd` sets `current_dir`).
fn shim(tmp: &Path, name: &str, body: &str) -> PathBuf {
    let p = tmp.join(name);
    std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    p
}

/// Budget for the two shim tests. Generous on purpose: the whole suite
/// spawns git in parallel, and a tight budget occasionally SIGTERMed the
/// shim's process group before `sh` had even recorded its pids — a flake
/// that looks exactly like a broken group kill.
const SHIM_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

/// Read the pids a shim recorded, with a message that names the real cause
/// if it never got that far.
fn shim_pids(path: &Path) -> Vec<libc::pid_t> {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "shim never recorded its pids ({e}) — the {SHIM_BUDGET:?} budget \
             beat its own startup; raise SHIM_BUDGET"
        )
    });
    raw.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only probes for the process's existence.
    unsafe { libc::kill(pid, 0) == 0 }
}

async fn wait_dead(pids: &[libc::pid_t], within: std::time::Duration) {
    let deadline = std::time::Instant::now() + within;
    loop {
        if pids.iter().all(|p| !alive(*p)) {
            return;
        }
        if std::time::Instant::now() >= deadline {
            let left: Vec<_> = pids.iter().filter(|p| alive(**p)).collect();
            panic!("still alive after {within:?}: {left:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn worktree_probe_timeout_keeps_cleanup_after_response() {
    let (tmp, dir) = fixture_on_branch("main");
    let bin = shim(
        tmp.path(),
        "slow-status",
        "if [ \"$1\" != status ]; then exec git \"$@\"; fi\ntrap '' TERM\nsleep 30 &\necho $! > shim-pids\necho $$ >> shim-pids\nwait",
    );
    let rows = LocalGit::new(&dir)
        .with_git_bin(bin)
        .worktree_list_with_budgets(SHIM_BUDGET, SHIM_BUDGET + std::time::Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].dirty_known && !rows[0].dirty);
    let pids = shim_pids(&dir.join("shim-pids"));
    assert_eq!(pids.len(), 2);
    wait_dead(&pids, std::time::Duration::from_secs(5)).await;
}

/// A spawn that overruns its budget is SIGTERMed as a GROUP, so the helper
/// processes git forked (`ssh`, `git-remote-https`) die with it — a plain
/// `kill(pid)` would leave them holding the network connection.
#[tokio::test]
async fn spawn_timeout_terminates_group_then_kills() {
    let (tmp, dir) = fixture_n_commits(1);
    let sh = shim(
        tmp.path(),
        "slow-git",
        "sleep 30 &\necho $! > shim-pids\necho $$ >> shim-pids\nsleep 30",
    );
    let git = LocalGit::new(&dir)
        .with_git_bin(&sh)
        .with_budget(SHIM_BUDGET);

    let started = std::time::Instant::now();
    let err = git.run(&["status"]).await.unwrap_err();
    assert!(
        started.elapsed() < SHIM_BUDGET + std::time::Duration::from_secs(6),
        "the timeout must not wait for the process: {:?}",
        started.elapsed()
    );
    match &err {
        Error::Upstream(m) => {
            assert!(m.contains("timed out"), "{m}");
            assert!(m.contains("index.lock"), "the text names the leftover: {m}");
        }
        other => panic!("expected Upstream, got {other:?}"),
    }

    let pids = shim_pids(&dir.join("shim-pids"));
    assert_eq!(pids.len(), 2, "shim recorded its own pid and its child's");
    wait_dead(&pids, std::time::Duration::from_secs(5)).await;
}

/// A process that IGNORES SIGTERM (git never does, but a hung helper might)
/// is escalated to SIGKILL 2 s later — the budget is a real bound.
#[tokio::test]
async fn spawn_timeout_escalates_to_sigkill() {
    let (tmp, dir) = fixture_n_commits(1);
    // `trap '' TERM` BEFORE forking `sleep`: an ignored disposition is
    // inherited, so neither process dies until the SIGKILL.
    let sh = shim(
        tmp.path(),
        "stubborn-git",
        "trap '' TERM\necho $$ > shim-pid\nsleep 30",
    );
    let git = LocalGit::new(&dir)
        .with_git_bin(&sh)
        .with_budget(SHIM_BUDGET);

    let started = std::time::Instant::now();
    let err = git.run(&["status"]).await.unwrap_err();
    assert!(
        matches!(&err, Error::Upstream(m) if m.contains("timed out")),
        "{err:?}"
    );
    assert!(
        started.elapsed() < SHIM_BUDGET + std::time::Duration::from_secs(6),
        "elapsed {:?}",
        started.elapsed()
    );
    let pids = shim_pids(&dir.join("shim-pid"));
    assert_eq!(pids.len(), 1, "shim recorded its pid");
    wait_dead(&pids, std::time::Duration::from_secs(5)).await;
}

/// The reason `LocalWrite` is detached: the UI aborts in-flight requests, and
/// hyper drops the handler future with them. A commit that is already writing
/// the index must survive that, or the repo is left with `index.lock`.
#[tokio::test]
async fn mutating_spawn_survives_request_drop() {
    let (_tmp, dir) = fixture_n_commits(1);
    let hooks = dir.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nsleep 0.4\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    write(&dir, "late.txt", "committed under a dropped request\n");
    sh_git(&dir, &["add", "-A"]);

    let git = LocalGit::new(&dir);
    let dropped = tokio::time::timeout(
        std::time::Duration::from_millis(10),
        git.commit("survives the drop", false),
    )
    .await;
    assert!(
        dropped.is_err(),
        "the caller future must be dropped mid-commit"
    );

    // Poll (bounded) instead of one fixed sleep: under a loaded test run
    // the detached commit (hook included) can need more than 1.5 s.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let subject = loop {
        let s = LocalGit::new(&dir)
            .run(&["log", "-1", "--format=%s"])
            .await
            .unwrap();
        if s.trim() == "survives the drop" || std::time::Instant::now() >= deadline {
            break s;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    // Let the finished git process release its locks.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(subject.trim(), "survives the drop");
    assert!(
        !dir.join(".git/index.lock").exists(),
        "a SIGKILLed git would have left index.lock behind"
    );
}

/// Same contract for the `Remote` class: a dropped request must not abort a
/// fetch mid-write (`FETCH_HEAD.lock` / `refs/remotes/*.lock`).
#[tokio::test]
async fn remote_class_survives_request_drop() {
    let (tmp, dir) = diverged_fixture();
    // Advance origin/main so the fetch has something to do.
    let seed = tmp.path().join("seed");
    sh_git(&seed, &["checkout", "-q", "main"]);
    write(&seed, "fetched.txt", "after\n");
    sh_git(&seed, &["add", "-A"]);
    sh_git(&seed, &["commit", "-m", "advance main"]);
    sh_git(&seed, &["push", "origin", "main"]);
    let origin_tip = rev(&tmp.path().join("origin.git"), "refs/heads/main");

    let sh = shim(tmp.path(), "slow-fetch-git", "sleep 0.4\nexec git \"$@\"");
    let git = LocalGit::new(&dir).with_git_bin(&sh);
    let dropped = tokio::time::timeout(std::time::Duration::from_millis(10), git.fetch(None)).await;
    assert!(
        dropped.is_err(),
        "the caller future must be dropped mid-fetch"
    );

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert_eq!(
        rev(&dir, "refs/remotes/origin/main"),
        origin_tip,
        "the detached fetch finished after the request was dropped"
    );
}

/// An agent's concurrent git holding `index.lock` is transient — staging
/// retries instead of surfacing a bogus failure.
#[tokio::test]
async fn stage_retries_transient_index_lock() {
    let (_tmp, dir) = fixture_n_commits(1);
    write(&dir, "staged.txt", "one\n");
    let lock = dir.join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();

    let unlock = lock.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        let _ = std::fs::remove_file(&unlock);
    });

    let git = LocalGit::new(&dir);
    git.stage(&["staged.txt".to_string()]).await.unwrap();
    let st = git.status().await.unwrap();
    assert!(st
        .changes
        .iter()
        .any(|c| c.path == "staged.txt" && c.staged));
}

/// G-1: every path is a LITERAL name. As a pathspec `app/[id]/page.tsx`
/// also matched `app/d/page.tsx` (restore reverted its edits) and
/// `app/i/page.tsx` (clean deleted it) — silently, with a success reply.
#[tokio::test]
async fn discard_and_stage_treat_bracketed_paths_literally() {
    let (_tmp, dir) = fixture_on_branch("main");
    write(&dir, "app/[id]/page.tsx", "id\n");
    write(&dir, "app/d/page.tsx", "d\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "routes"]);
    write(&dir, "app/[id]/page.tsx", "id EDIT\n");
    write(&dir, "app/d/page.tsx", "d EDIT\n");
    write(&dir, "app/i/page.tsx", "untracked\n");
    let git = LocalGit::new(&dir);

    git.discard(&["app/[id]/page.tsx".into()]).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("app/[id]/page.tsx")).unwrap(),
        "id\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("app/d/page.tsx")).unwrap(),
        "d EDIT\n",
        "a glob match must not revert a sibling"
    );
    assert!(dir.join("app/i/page.tsx").exists(), "…nor clean one");

    write(&dir, "app/[id]/page.tsx", "id EDIT\n");
    git.stage(&["app/[id]/page.tsx".into()]).await.unwrap();
    let staged = git.run(&["diff", "--cached", "--name-only"]).await.unwrap();
    assert_eq!(staged.trim(), "app/[id]/page.tsx");
}

/// SH-08: "Stage all" / "Unstage all" / "Discard all" over a list whose
/// argv would exceed ARG_MAX (1 MB on macOS; ~1.3 MB here) — the names go
/// over stdin (`clean` in bounded argv batches) instead of dying E2BIG.
/// The byte count is what matters, not the file count: ~725-byte paths
/// (three long components, still under macOS's 1024-byte PATH_MAX with the
/// temp-dir prefix) reach it with 1,800 files instead of 14,000 short ones.
#[tokio::test]
async fn stage_unstage_discard_a_path_list_past_arg_max() {
    let (_tmp, dir) = fixture_on_branch("main");
    let pad = "x".repeat(235);
    let paths: Vec<String> = (0..1_800)
        .map(|i| format!("gen/{:02}/{pad}/{pad}/{pad}_{i:05}.txt", i % 50))
        .collect();
    for p in &paths {
        write(&dir, p, "g\n");
    }
    assert!(paths.iter().map(|p| p.len() + 1).sum::<usize>() > 1 << 20);
    let git = LocalGit::new(&dir);

    git.stage(&paths).await.unwrap();
    let staged = git
        .run(&["diff", "--cached", "--name-only", "-z"])
        .await
        .unwrap();
    assert_eq!(nul_records(&staged).count(), paths.len());

    git.unstage(&paths).await.unwrap();
    let staged = git
        .run(&["diff", "--cached", "--name-only", "-z"])
        .await
        .unwrap();
    assert_eq!(nul_records(&staged).count(), 0);

    git.discard(&paths).await.unwrap();
    assert!(
        paths.iter().all(|p| !dir.join(p).exists()),
        "every untracked file cleaned"
    );
}

#[test]
fn argv_batches_split_by_bytes_and_keep_order() {
    let paths: Vec<String> = ["aaaa", "bb", "cccccc", "d"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let b = argv_batches(&paths, 8);
    assert_eq!(b, vec![&paths[0..2], &paths[2..3], &paths[3..4]]);
    // A single name over the budget still gets a batch of its own.
    let b = argv_batches(&paths, 2);
    assert_eq!(b.len(), 4);
    assert!(argv_batches(&[], 8).is_empty());
}

/// G-3: status is read with `-z`, so a non-ASCII or `"`-carrying name is
/// the REAL name — stage/unstage/discard find the file, and a discard
/// with nothing left to do is a 409 instead of a silent "Discarded".
#[tokio::test]
async fn quoted_and_non_ascii_names_stage_and_discard() {
    let (_tmp, dir) = fixture_on_branch("main");
    write(&dir, "caf\u{e9}.txt", "accent\n");
    write(&dir, "q\"uote.txt", "quote\n");
    let git = LocalGit::new(&dir);
    let st = git.status().await.unwrap();
    let names: Vec<&str> = st.changes.iter().map(|c| c.path.as_str()).collect();
    assert!(names.contains(&"caf\u{e9}.txt"), "{names:?}");
    assert!(names.contains(&"q\"uote.txt"), "{names:?}");

    let both: Vec<String> = vec!["caf\u{e9}.txt".into(), "q\"uote.txt".into()];
    git.stage(&both).await.unwrap();
    let st = git.status().await.unwrap();
    assert!(st.changes.iter().all(|c| c.staged), "{:?}", st.changes);
    git.unstage(&both).await.unwrap();

    git.discard(&["caf\u{e9}.txt".into()]).await.unwrap();
    assert!(
        !dir.join("caf\u{e9}.txt").exists(),
        "the untracked file is gone"
    );
    assert!(dir.join("q\"uote.txt").exists());
    assert!(matches!(
        git.discard(&["caf\u{e9}.txt".into()]).await,
        Err(Error::Conflict(_))
    ));
}

/// G-9: before the first commit there is no HEAD to restore the index
/// from; unstage still works and the branch name still reads.
#[tokio::test]
async fn unstage_and_branch_name_before_the_first_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    write(&dir, "a.txt", "new\n");
    sh_git(&dir, &["add", "a.txt"]);
    let git = LocalGit::new(&dir);
    assert_eq!(git.current_branch().await.unwrap(), "main");
    git.unstage(&["a.txt".into()]).await.unwrap();
    let st = git.status().await.unwrap();
    assert_eq!(st.changes.len(), 1);
    assert_eq!(st.changes[0].kind, "untracked");
}

/// The preview lists conflicted FILES only — not merge-tree's
/// "Auto-merging …" / "CONFLICT (content): …" message lines.
#[tokio::test]
async fn merge_preview_lists_only_conflicted_file_names() {
    let (_tmp, dir) = conflicted_fixture("c.txt");
    let git = LocalGit::new(&dir);
    let p = git.merge_preview("side", "main").await.unwrap();
    assert!(p.conflicts);
    assert_eq!(p.conflicted_files, vec!["c.txt".to_string()]);
}

/// Two branches that change the same line of `rel` → `merge side`
/// conflicts on it. Returns (tmp, repo).
fn conflicted_fixture(rel: &str) -> (tempfile::TempDir, PathBuf) {
    let (tmp, dir) = fixture_on_branch("main");
    write(&dir, rel, "base\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "base"]);
    sh_git(&dir, &["checkout", "-q", "-b", "side"]);
    write(&dir, rel, "side\n");
    sh_git(&dir, &["commit", "-qam", "side"]);
    sh_git(&dir, &["checkout", "-q", "main"]);
    write(&dir, rel, "main\n");
    sh_git(&dir, &["commit", "-qam", "main"]);
    (tmp, dir)
}

/// G-4: the resolver writes ONLY a currently-conflicted file, inside the
/// working tree — never `.git/config` (fsmonitor = code execution), never
/// through a symlink or a symlinked parent.
#[tokio::test]
async fn write_resolution_is_confined_to_conflicted_worktree_files() {
    let (tmp, dir) = conflicted_fixture("sub/c.txt");
    let git = LocalGit::new(&dir);
    let _ = git.run(&["merge", "side"]).await;
    assert_eq!(git.conflicted_paths().await.unwrap(), vec!["sub/c.txt"]);

    let config_before = std::fs::read(dir.join(".git/config")).unwrap();
    let payload = "[core]\n\tfsmonitor = /tmp/otto-pwn.sh\n";
    for bad in [
        ".git/config",
        ".GIT/config",
        "a.txt",
        "sub",
        "sub/*",
        "../outside.txt",
        "/etc/hosts",
    ] {
        assert!(git.write_resolution(bad, payload).await.is_err(), "{bad}");
    }
    assert_eq!(
        std::fs::read(dir.join(".git/config")).unwrap(),
        config_before
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "hello\n"
    );

    // The conflicted file itself swapped for a symlink to a file outside.
    let outside = tmp.path().join("outside.txt");
    std::fs::write(&outside, "keep\n").unwrap();
    let conflicted = dir.join("sub/c.txt");
    let marked = std::fs::read(&conflicted).unwrap();
    std::fs::remove_file(&conflicted).unwrap();
    std::os::unix::fs::symlink(&outside, &conflicted).unwrap();
    assert!(matches!(
        git.write_resolution("sub/c.txt", "pwned\n").await,
        Err(Error::Invalid(_))
    ));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep\n");
    std::fs::remove_file(&conflicted).unwrap();

    // …and a symlinked PARENT directory that leads out of the tree.
    let outside_dir = tmp.path().join("outside_dir");
    std::fs::create_dir(&outside_dir).unwrap();
    std::fs::write(outside_dir.join("c.txt"), "keep\n").unwrap();
    std::fs::rename(dir.join("sub"), tmp.path().join("sub_parked")).unwrap();
    std::os::unix::fs::symlink(&outside_dir, dir.join("sub")).unwrap();
    assert!(git.write_resolution("sub/c.txt", "pwned\n").await.is_err());
    assert_eq!(
        std::fs::read_to_string(outside_dir.join("c.txt")).unwrap(),
        "keep\n"
    );
    std::fs::remove_file(dir.join("sub")).unwrap();
    std::fs::rename(tmp.path().join("sub_parked"), dir.join("sub")).unwrap();
    std::fs::write(&conflicted, marked).unwrap();

    // The legitimate write still resolves and stages the file.
    git.write_resolution("sub/c.txt", "resolved\n")
        .await
        .unwrap();
    assert!(git.conflicted_paths().await.unwrap().is_empty());
    assert_eq!(std::fs::read_to_string(&conflicted).unwrap(), "resolved\n");
}

/// G-5: a name that is a DIRECTORY, not a branch, must never be read as a
/// path — `checkout docs` restored `docs/` from the index (exit 0) and the
/// user's uncommitted edits were gone.
#[tokio::test]
async fn checkout_of_a_directory_name_keeps_uncommitted_work() {
    let (_tmp, dir) = fixture_on_branch("main");
    write(&dir, "docs/a.md", "committed\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "docs"]);
    write(&dir, "docs/a.md", "committed\nUNCOMMITTED EDIT\n");
    let git = LocalGit::new(&dir);
    match git.checkout("docs", false).await {
        Err(Error::Conflict(msg)) => assert!(msg.contains("invalid reference"), "{msg}"),
        other => panic!("expected Conflict, got {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("docs/a.md")).unwrap(),
        "committed\nUNCOMMITTED EDIT\n",
        "the edit survives"
    );
    assert_eq!(git.current_branch().await.unwrap(), "main");
    // A tag / SHA still checks out (detached) through the same path.
    sh_git(&dir, &["stash", "push", "-m", "park"]);
    sh_git(&dir, &["tag", "v1"]);
    git.checkout("v1", false).await.unwrap();
    git.checkout("main", false).await.unwrap();
}

/// G-6: when the switch to `target` fails after the auto-stash (target is
/// checked out in an agent's worktree), the user's tracked AND untracked
/// changes come back on the branch they were on — nothing is stranded.
#[tokio::test]
async fn merge_autostash_restores_changes_when_the_switch_fails() {
    let (tmp, dir) = fixture_on_branch("main");
    sh_git(&dir, &["branch", "develop"]);
    sh_git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "f.txt", "feature\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "feature"]);
    let wt = tmp.path().join("wt");
    sh_git(&dir, &["worktree", "add", wt.to_str().unwrap(), "develop"]);
    write(&dir, "a.txt", "hello\nDIRTY\n");
    write(&dir, "new.txt", "untracked\n");

    let git = LocalGit::new(&dir);
    let err = git
        .merge_branch("feature", "develop", LocalMergeStrategy::MergeCommit, true)
        .await
        .unwrap_err();
    assert_eq!(git.current_branch().await.unwrap(), "feature", "{err:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "hello\nDIRTY\n"
    );
    assert!(dir.join("new.txt").exists(), "untracked file restored");
    assert_eq!(git.stash_list().await.unwrap().len(), 1, "{err:?}");
}

/// `merge --squash` abort drops the squash but KEEPS unrelated edits
/// (`reset --merge`); `reset --hard` wiped them.
#[tokio::test]
async fn squash_abort_keeps_unrelated_uncommitted_edits() {
    let (_tmp, dir) = fixture_on_branch("main");
    sh_git(&dir, &["checkout", "-q", "-b", "side"]);
    write(&dir, "s.txt", "side\n");
    sh_git(&dir, &["add", "."]);
    sh_git(&dir, &["commit", "-m", "side"]);
    sh_git(&dir, &["checkout", "-q", "main"]);
    let git = LocalGit::new(&dir);
    git.merge_branch("side", "main", LocalMergeStrategy::Squash, false)
        .await
        .unwrap();
    write(&dir, "a.txt", "hello\nUNRELATED\n");
    git.merge_abort().await.unwrap();
    assert!(!dir.join("s.txt").exists(), "the squash is gone");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "hello\nUNRELATED\n"
    );
}

/// EH-6-07: the `Working` diff falls back to cached+worktree only for an
/// unborn HEAD. A HEAD naming a missing object used to come back as a partial
/// "everything is new" diff (or an empty one) instead of an error.
#[tokio::test]
async fn working_diff_errors_on_a_broken_head_and_still_handles_an_unborn_one() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-b", "main"]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);
    write(&dir, "a.txt", "fresh\n");
    sh_git(&dir, &["add", "a.txt"]);
    let git = LocalGit::new(&dir);
    // Unborn: the staged file is the whole diff, with and without a path.
    let unborn = git.diff(DiffTarget::Working, None).await.unwrap();
    assert!(unborn.files.iter().any(|f| f.path == "a.txt"), "{unborn:?}");
    let scoped = git.diff(DiffTarget::Working, Some("a.txt")).await.unwrap();
    assert!(scoped.files.iter().any(|f| f.path == "a.txt"));

    sh_git(&dir, &["commit", "-m", "first"]);
    write(&dir, "a.txt", "changed\n");
    write(
        &dir,
        ".git/refs/heads/main",
        "1111111111111111111111111111111111111111\n",
    );
    let git = LocalGit::new(&dir);
    let err = git.diff(DiffTarget::Working, None).await.unwrap_err();
    assert!(err.to_string().contains("bad object"), "{err}");
}
