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

/// Shared fixture for the workflow repo-scope tests: root user + token, two
/// workspaces, and a repo registered in workspace A.
async fn two_workspaces_with_repo(
    tmp: &tempfile::TempDir,
) -> (
    ServerCtx,
    DbPool,
    String,
    String,
    otto_core::domain::Workspace,
    otto_core::domain::Repo,
) {
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
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
    (ctx, pool, owner.id, token, ws_b, repo_a)
}

async fn serve(ctx: &ServerCtx) -> (String, tokio::task::JoinHandle<()>) {
    let (api_extras, root_extras) = otto_server::modules::module_routers(ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (origin, server)
}

/// S3-302: a manual workflow run whose input names ANOTHER workspace's repo is
/// refused (400) before any run row exists — it would otherwise drive that
/// repo's checkout and git account from a `git_pr` / `review_run` step.
#[tokio::test]
async fn manual_workflow_run_with_a_foreign_repo_id_is_refused() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, owner, token, ws_b, repo_a) = two_workspaces_with_repo(&tmp).await;
    let graph: otto_core::workflows::WorkflowGraph = serde_json::from_value(json!({
        "nodes": [{"id": "n", "kind": "agent", "params": {"prompt": "x"}}], "edges": []
    }))
    .unwrap();
    let wf = otto_state::WorkflowsRepo::new(pool.clone())
        .create(&ws_b.id, "wf", "", "", &graph, &owner)
        .await
        .unwrap();
    let (origin, server) = serve(&ctx).await;
    let r = reqwest::Client::new()
        .post(format!("{origin}/workflows/{}/run", wf.id))
        .bearer_auth(&token)
        .json(&json!({ "input": { "repo_id": repo_a.id, "base": "main" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "a repo of workspace A from B's workflow");
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body["message"].as_str().unwrap_or("").contains("workspace"),
        "{body}"
    );
    assert!(
        otto_state::WorkflowsRepo::new(pool.clone())
            .list_runs(&wf.id)
            .await
            .unwrap()
            .is_empty(),
        "no run row is created"
    );
    server.abort();
}

/// S3-02 / S3-302 (route level): a token-only webhook body can set NONE of the
/// reserved run-input keys — chat origin, result destinations, run location or
/// git target. The stored `input` carries only the ordinary keys plus the
/// trigger SPEC's own defaults.
#[tokio::test]
async fn webhook_body_cannot_set_reserved_run_input_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, owner, _token, ws_b, repo_a) = two_workspaces_with_repo(&tmp).await;
    // No nodes: the spawned run has nothing to execute.
    let graph: otto_core::workflows::WorkflowGraph =
        serde_json::from_value(json!({ "nodes": [], "edges": [] })).unwrap();
    let wf = otto_state::WorkflowsRepo::new(pool.clone())
        .create(&ws_b.id, "wf", "", "", &graph, &owner)
        .await
        .unwrap();
    let hook = "a".repeat(64);
    otto_state::TriggersRepo::new(pool.clone())
        .create(otto_state::NewWorkflowTrigger {
            workflow_id: wf.id.clone(),
            kind: "webhook".into(),
            spec: json!({ "token": hook, "base": "develop", "result_webhook": "https://spec.example/hook" }),
            enabled: true,
        })
        .await
        .unwrap();
    let (origin, server) = serve(&ctx).await;
    let r = reqwest::Client::new()
        .post(format!("{origin}/workflows/{}/webhook/{hook}", wf.id))
        .json(&json!({
            "prompt": "summarize",
            "origin_workspace_id": "other-ws", "origin_user": "x",
            "channel": "slack", "chat": "C-attacker", "thread": "1",
            "result_chat": "C-attacker", "result_channel": "slack", "result_thread": "2",
            "result_webhook": "https://evil.example", "callback_url": "https://evil.example",
            "working_directory": "/", "repos": [{"repo": "repo-a"}], "cwd": "/",
            "worktree": "/x", "worktree_path": "/y",
            "repo_id": repo_a.id, "base": "main", "pr": 7, "pr_branch": "evil",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let run: serde_json::Value = r.json().await.unwrap();
    let stored = otto_state::WorkflowsRepo::new(pool.clone())
        .get_run(&run["id"].as_str().unwrap().to_string())
        .await
        .unwrap();
    let input = stored.input.as_object().expect("object input");
    assert_eq!(input.get("prompt"), Some(&json!("summarize")));
    // Spec defaults replace the body's values.
    assert_eq!(input.get("base"), Some(&json!("develop")), "{input:?}");
    assert_eq!(
        input.get("result_webhook"),
        Some(&json!("https://spec.example/hook"))
    );
    for k in [
        "origin_workspace_id",
        "origin_user",
        "channel",
        "chat",
        "thread",
        "result_chat",
        "result_channel",
        "result_thread",
        "callback_url",
        "working_directory",
        "repos",
        "cwd",
        "worktree",
        "worktree_path",
        "repo_id",
        "pr",
        "pr_branch",
    ] {
        assert!(
            !input.contains_key(k),
            "webhook body set reserved {k}: {input:?}"
        );
    }
    server.abort();
}
