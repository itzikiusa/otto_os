//! Review-agent Retry is refused while the agent is live (S2-07): retrying a
//! running/waiting/pending row used to archive its session and replace its
//! cancel flag, leaving two loops on one index that neither Stop nor Cancel
//! could reach. Only settled rows (done / error / skipped) are retried.
use otto_core::domain::ReviewAgentState;
use otto_server::ServerCtx;
use otto_state::{DbPool, NewRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

fn agent(status: &str) -> ReviewAgentState {
    serde_json::from_value(serde_json::json!({
        "name": "correctness", "provider": "claude", "model": "",
        "status": status, "note": "", "comment_count": 0,
        "session_id": "sess-live",
    }))
    .unwrap()
}

#[tokio::test]
async fn retrying_a_live_review_agent_is_a_conflict() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Retry gate", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let checkout = tmp.path().join("repo");
    std::fs::create_dir_all(&checkout).unwrap();
    let repo = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws.id.clone(),
            name: "repo".into(),
            path: checkout.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let review = ctx.reviews_store.create_review(&repo.id, 0).await.unwrap();
    // Row 0..2 are live, row 3 is the summarizer slot.
    let rows = vec![
        agent("running"),
        agent("waiting"),
        agent("pending"),
        agent("pending"),
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();

    // The review/run routes live in the module routers, not the core router.
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    for (index, row) in rows.iter().enumerate().take(3) {
        let r = client
            .post(format!(
                "{origin}/reviews/{}/agents/{index}/retry",
                review.id
            ))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 409, "retry of a {} agent", row.status);
        let body: serde_json::Value = r.json().await.unwrap();
        assert_eq!(body["code"], "conflict", "{body}");
    }
    // Nothing was reset: the live rows keep their status + session.
    let after = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(after.agents[0].status, "running");
    assert_eq!(after.agents[0].session_id.as_deref(), Some("sess-live"));

    // S2-303: a SETTLED agent of a review that is still Running is refused
    // too — the original run's end-of-review sweep would delete the retried
    // agent's diff/prompt/findings files mid-read.
    assert_eq!(after.status, otto_core::domain::ReviewStatus::Running);
    let rows = vec![
        agent("done"),
        agent("running"),
        agent("pending"),
        agent("pending"),
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();
    let r = client
        .post(format!("{origin}/reviews/{}/agents/0/retry", review.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "retry of a done agent in a running review");
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body["message"]
            .as_str()
            .unwrap_or_default()
            .contains("still running")
            || body.to_string().contains("still running"),
        "{body}"
    );
    let after = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(after.agents[0].status, "done", "nothing was reset");
    server.abort();
}
