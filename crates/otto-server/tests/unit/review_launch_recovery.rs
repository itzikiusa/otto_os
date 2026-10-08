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
