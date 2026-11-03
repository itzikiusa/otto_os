//! Run-start branch sync (user report: a Slack-triggered workflow for a PR
//! `x → y` failed because `y` existed only on the remote). A LOCAL bare repo
//! stands in for the remote — no network.
use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
use crate::workflow_engine::*;
use std::path::Path;

#[allow(clippy::disallowed_methods)] // test fixture setup: a short sync git call
fn git(dir: &Path, args: &[&str]) -> String {
    let out = otto_git::hardened_std_command()
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn identity(dir: &Path) {
    git(dir, &["config", "user.email", "t@example.invalid"]);
    git(dir, &["config", "user.name", "T"]);
    git(dir, &["config", "commit.gpgsign", "false"]);
}

fn commit(dir: &Path, file: &str) -> String {
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", "."]);
    git(dir, &["commit", "-q", "-m", file]);
    git(dir, &["rev-parse", "HEAD"])
}

#[tokio::test]
async fn run_start_sync_resolves_remote_only_pr_branches_and_fast_forwards_a_stale_base() {
    let dir = tempfile::tempdir().unwrap();
    let (remote, checkout, other) = (
        dir.path().join("remote.git"),
        dir.path().join("checkout"),
        dir.path().join("other"),
    );
    let s = |p: &Path| p.to_string_lossy().into_owned();
    git(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", &s(&remote)],
    );
    git(dir.path(), &["clone", "-q", &s(&remote), &s(&checkout)]);
    identity(&checkout);
    commit(&checkout, "base.txt");
    git(&checkout, &["push", "-q", "-u", "origin", "main"]);
    // A teammate publishes the base `y` (which the user's clone has a STALE
    // copy of) and then the PR head `x`, which the clone never saw.
    git(dir.path(), &["clone", "-q", &s(&remote), &s(&other)]);
    identity(&other);
    git(&other, &["checkout", "-q", "-b", "y"]);
    commit(&other, "y1.txt");
    git(&other, &["push", "-q", "origin", "y"]);
    git(&checkout, &["fetch", "-q", "origin"]);
    git(&checkout, &["branch", "--track", "y", "origin/y"]);
    let y_tip = commit(&other, "y2.txt");
    git(&other, &["push", "-q", "origin", "y"]);
    git(&other, &["checkout", "-q", "-b", "x"]);
    let x_tip = commit(&other, "x.txt");
    git(&other, &["push", "-q", "origin", "x"]);

    let pool = mem_pool().await;
    seed_workspace(&pool, "sync-ws").await;
    let ctx = test_ctx(&pool, dir.path().join("data")).await;
    let repo = ctx
        .git_store
        .create_repo(otto_state::NewRepo {
            workspace_id: "sync-ws".into(),
            name: "checkout".into(),
            path: s(&checkout),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let mut entries = vec![crate::workflow_context::RepoEntry {
        repo: repo.path.clone(),
        repo_id: Some(repo.id.clone()),
        repo_name: Some(repo.name.clone()),
        kind: "worktree".into(),
        name: repo.path.clone(),
        source: None,
        worktree: Some(repo.path.clone()),
        base: Some("y".into()),
        error: None,
    }];

    let synced = sync_run_branches(&ctx, &entries, Some("x")).await;
    let report = &synced[&repo.id];
    assert_eq!(report.fetch_error, None);
    assert_eq!(report.resolved("y"), Some("refs/heads/y"));
    assert_eq!(report.resolved("x"), Some("refs/heads/x"));
    // Both PR branches now resolve by name in the user's clone…
    assert_eq!(
        git(&checkout, &["rev-parse", "y"]),
        y_tip,
        "stale base fast-forwarded"
    );
    assert_eq!(
        git(&checkout, &["rev-parse", "x"]),
        x_tip,
        "remote-only head created"
    );
    // …and the user's own checkout was not touched.
    assert_eq!(git(&checkout, &["branch", "--show-current"]), "main");

    let run_id: Id = "run-sync-1".into();
    let (primary, failures) = provision_wf_worktrees(&ctx, &run_id, &mut entries, &synced).await;
    assert!(failures.is_empty(), "{failures:?}");
    let wt = primary.expect("isolated worktree provisioned");
    // The run's worktree is cut from the CURRENT remote base, not the stale copy.
    assert_eq!(git(Path::new(&wt), &["rev-parse", "HEAD"]), y_tip);
}
