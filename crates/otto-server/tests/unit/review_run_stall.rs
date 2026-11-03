//! A review whose runner died after its reviewers finished used to hold the
//! workflow step "Running" until the 5 h timeout while every agent row read
//! Done (the user's "it says running, but all is done").
use super::*;
use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
use otto_core::cancel::CancelRegistryExt;
use otto_core::domain::{ReviewAgentState, ReviewStatus};

fn row(name: &str, status: &str) -> ReviewAgentState {
    serde_json::from_value(json!({
        "name": name, "provider": "claude", "model": "",
        "status": status, "note": "", "comment_count": 0,
    }))
    .unwrap()
}

async fn fixture(agents: &[ReviewAgentState]) -> (tempfile::TempDir, ServerCtx, Workspace, Id, Id) {
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "stall-ws").await;
    let ctx = test_ctx(&pool, dir.path().join("data")).await;
    let ws = otto_state::WorkspacesRepo::new(pool.clone())
        .get(&"stall-ws".into())
        .await
        .unwrap();
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let graph: WorkflowGraph = serde_json::from_value(json!({"nodes":[],"edges":[]})).unwrap();
    let runs = WorkflowsRepo::new(pool);
    let wf = runs
        .create(&ws.id, "PR Reviewer", "", "", &graph, &root_user().id)
        .await
        .unwrap();
    let run = runs
        .create_run(&wf.id, &ws.id, &Value::Null, Some(&root_user().id))
        .await
        .unwrap();
    let review = ctx
        .reviews_store
        .create_review(&"repo".to_string(), 0)
        .await
        .unwrap();
    ctx.reviews_store
        .set_agents(&review.id, agents)
        .await
        .unwrap();
    (dir, ctx, ws, review.id, run.id)
}

#[tokio::test]
async fn a_review_with_settled_agents_and_no_runner_settles_as_error() {
    // Reviewers done, the summarizer never claimed its row, no runner task.
    let agents = [
        row("correctness", "done"),
        row("security", "done"),
        row("Summarizer", "pending"),
    ];
    let (_d, ctx, ws, review_id, run_id) = fixture(&agents).await;
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut logs = Vec::new();
    let status = tokio::time::timeout(
        Duration::from_secs(10),
        await_review(
            &ctx,
            &ws,
            &run_id,
            &review_id,
            Some(3600),
            Duration::ZERO,
            &tx,
            "",
            &mut logs,
        ),
    )
    .await
    .expect("must not wait for the step timeout")
    .unwrap();
    assert_eq!(status, "error");
    assert!(
        logs.iter().any(|l| l.contains("review stalled")),
        "{logs:?}"
    );
    let (st, _) = ctx.reviews_store.review_agents(&review_id).await.unwrap();
    assert_eq!(st, ReviewStatus::Error);
}

#[tokio::test]
async fn a_live_runner_or_a_running_agent_is_never_called_stalled() {
    for (agents, live) in [
        (vec![row("a", "done"), row("Summarizer", "pending")], true),
        (
            vec![row("a", "running"), row("Summarizer", "pending")],
            false,
        ),
    ] {
        let (_d, ctx, ws, review_id, run_id) = fixture(&agents).await;
        let _guard = live.then(|| ctx.review_cancels.register(&review_id));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut logs = Vec::new();
        let waited = tokio::time::timeout(
            Duration::from_millis(300),
            await_review(
                &ctx,
                &ws,
                &run_id,
                &review_id,
                Some(3600),
                Duration::ZERO,
                &tx,
                "",
                &mut logs,
            ),
        )
        .await;
        assert!(
            waited.is_err(),
            "still waiting (live={live}): {waited:?} {logs:?}"
        );
        let (st, _) = ctx.reviews_store.review_agents(&review_id).await.unwrap();
        assert_eq!(st, ReviewStatus::Running);
    }
}
