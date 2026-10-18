use super::*;
use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};

#[tokio::test]
async fn precancelled_review_never_seeds_or_launches_agents() {
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "cancel-review-ws").await;
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let workspace = ctx
        .workspaces
        .get(&"cancel-review-ws".into())
        .await
        .unwrap();
    let repo = ctx
        .git_store
        .create_repo(otto_state::git::NewRepo {
            workspace_id: workspace.id.clone(),
            name: "fixture".into(),
            path: dir.path().to_string_lossy().into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let review = ctx.reviews_store.create_review(&repo.id, 0).await.unwrap();
    ctx.reviews_store
        .set_status(&review.id, ReviewStatus::Cancelled, None)
        .await
        .unwrap();
    // No root user is inserted: the old path stops just after seeding rows,
    // before any provider session could run, and the assertion detects it.
    run_review(
        ctx.clone(),
        review.id.clone(),
        repo.path,
        "diff fixture".into(),
        None,
        None,
        workspace,
        repo.id,
        0,
        None,
        Some(default_review_config("claude")),
        None,
    )
    .await;
    let after = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert!(after.agents.is_empty());
    assert_eq!(after.status, ReviewStatus::Cancelled);
}

#[allow(clippy::disallowed_methods)]
fn revision_git(path: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}

#[tokio::test]
async fn pr_retry_uses_recorded_revision_without_current_forge_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "retry-revision-ws").await;
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let user = otto_state::UsersRepo::new(pool.clone())
        .create("reviewer", "x", "Reviewer", true)
        .await
        .unwrap();
    let path = dir.path().join("repo");
    std::fs::create_dir(&path).unwrap();
    revision_git(&path, &["init", "-q"]);
    revision_git(&path, &["config", "user.name", "Fixture"]);
    revision_git(&path, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(path.join("code"), "A").unwrap();
    revision_git(&path, &["add", "."]);
    revision_git(&path, &["commit", "-qm", "A"]);
    let a = revision_git(&path, &["rev-parse", "HEAD"]);
    let repo = ctx
        .git_store
        .create_repo(otto_state::git::NewRepo {
            workspace_id: "retry-revision-ws".into(),
            name: "fixture".into(),
            path: path.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let review = ctx.reviews_store.create_review(&repo.id, 7).await.unwrap();
    ctx.reviews_store
        .set_run_context(
            &review.id,
            &otto_state::ReviewRunContext {
                source_branch: Some("feature".into()),
                cwd: Some(repo.path.clone()),
                head_sha: Some(a.clone()),
            },
        )
        .await
        .unwrap();
    std::fs::write(path.join("code"), "B").unwrap();
    revision_git(&path, &["commit", "-qam", "B"]);
    let wt = pr_retry_worktree(&ctx, &user, &repo, &review.id, 7, "-revision-test")
        .await
        .unwrap();
    assert_eq!(
        revision_git(std::path::Path::new(&wt.path), &["rev-parse", "HEAD"]),
        a
    );
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&wt.path).join("code")).unwrap(),
        "A"
    );
    teardown_pr_worktree(&repo.path, wt).await;
    let unrecorded = ctx.reviews_store.create_review(&repo.id, 8).await.unwrap();
    assert!(
        pr_retry_worktree(&ctx, &user, &repo, &unrecorded.id, 8, "-missing-test")
            .await
            .is_err()
    );
}
