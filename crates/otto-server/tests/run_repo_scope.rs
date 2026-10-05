//! Run with Otto never works in another workspace's repo (S15-05): launch only
//! checks the caller's role on the TARGET workspace, so an explicit `repo_id`
//! from a different workspace must be refused — as a 404, exactly like a
//! missing repo — before any run row, worktree or agent exists.
use otto_server::ServerCtx;
use otto_state::{DbPool, NewRepo};
use serde_json::json;
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

#[tokio::test]
async fn launching_a_run_on_another_workspaces_repo_is_not_found() {
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
    let ws_a = ctx
        .workspaces
        .create("A", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let ws_b = ctx
        .workspaces
        .create("B", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let checkout = tmp.path().join("repo-a");
    std::fs::create_dir_all(&checkout).unwrap();
    let repo_a = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws_a.id.clone(),
            name: "repo-a".into(),
            path: checkout.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();

    // The review/run routes live in the module routers, not the core router.
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();

    let r = client
        .post(format!("{origin}/workspaces/{}/runs", ws_b.id))
        .bearer_auth(&token)
        .json(&json!({ "seed_text": "fix it", "repo_id": repo_a.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404, "a repo of workspace A launched from B");
    // The handler's NotFound (not an unmounted route): a JSON problem body.
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["code"], "not_found", "{body}");
    assert!(
        body["message"].as_str().unwrap_or("").contains("repo"),
        "{body}"
    );
    assert!(
        ctx.runs
            .list_by_workspace(&ws_b.id, 10)
            .await
            .unwrap()
            .is_empty(),
        "no run row is created"
    );
    server.abort();
}
