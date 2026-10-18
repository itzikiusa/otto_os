//! Run with Otto never works in a repo its launcher cannot reach (S15-05):
//! launch only checks the caller's role on the TARGET workspace, so an
//! explicit `repo_id` from a workspace they cannot edit is refused — as a 404,
//! exactly like a missing repo — before any run row, worktree or agent
//! exists. A repo another workspace registered is fine when they CAN edit
//! that workspace (sweep 2026-10-11: the same lift #102 gave workflow runs).
use otto_core::domain::{Capability, Feature, WorkspaceRole};
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
async fn launching_a_run_on_a_repo_the_launcher_cannot_edit_is_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let member = users
        .create("member", "unused", "Member", false)
        .await
        .unwrap();
    otto_state::GrantsRepo::new(pool.clone())
        .set_grants(&member.id, &[(Feature::RunWithOtto, Capability::Edit)])
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&member.id)
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
    ctx.workspaces
        .set_member(&ws_b.id, &member.id, WorkspaceRole::Editor)
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

    for repo_id in [repo_a.id.as_str(), "no-such-repo"] {
        let r = client
            .post(format!("{origin}/workspaces/{}/runs", ws_b.id))
            .bearer_auth(&token)
            .json(&json!({ "seed_text": "fix it", "repo_id": repo_id }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{repo_id}: a repo the member cannot edit");
        // The handler's NotFound (not an unmounted route): a JSON problem body.
        let body: serde_json::Value = r.json().await.unwrap();
        assert_eq!(body["code"], "not_found", "{body}");
        assert!(
            body["message"].as_str().unwrap_or("").contains("repo"),
            "{body}"
        );
    }
    assert!(
        ctx.runs
            .list_by_workspace(&ws_b.id, 10)
            .await
            .unwrap()
            .is_empty(),
        "no run row is created"
    );
    // Once the member may edit A as well, B's run may work in A's repo. The
    // repo-scope check passes; the launch then goes on to the source checks.
    ctx.workspaces
        .set_member(&ws_a.id, &member.id, WorkspaceRole::Viewer)
        .await
        .unwrap();
    let r = client
        .post(format!("{origin}/workspaces/{}/runs", ws_b.id))
        .bearer_auth(&token)
        .json(&json!({ "seed_text": "fix it", "repo_id": repo_a.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        404,
        "a viewer of A still cannot drive its checkout"
    );
    ctx.workspaces
        .set_member(&ws_a.id, &member.id, WorkspaceRole::Editor)
        .await
        .unwrap();
    let r = client
        .post(format!("{origin}/workspaces/{}/runs", ws_b.id))
        .bearer_auth(&token)
        .json(&json!({ "seed_text": "fix it", "repo_id": repo_a.id }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "an editor of both workspaces: {}",
        r.text().await.unwrap_or_default()
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

/// S3-302: a manual workflow run may name ANOTHER workspace's registered repo
/// (one shared workflow, e.g. a PR reviewer, serves repos registered
/// elsewhere); an id that is not a registered repo is refused (400) before any
/// run row exists.
#[tokio::test]
async fn manual_workflow_run_takes_any_registered_repo_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, owner, token, ws_b, repo_a) = two_workspaces_with_repo(&tmp).await;
    let graph: otto_core::workflows::WorkflowGraph = serde_json::from_value(json!({
        "nodes": [{"id": "n", "kind": "log", "params": {}}], "edges": []
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
        .json(&json!({ "input": { "repo_id": "no-such-repo", "base": "main" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400, "an unregistered repo id");
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(
        body["message"]
            .as_str()
            .unwrap_or("")
            .contains("registered"),
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
    let r = reqwest::Client::new()
        .post(format!("{origin}/workflows/{}/run", wf.id))
        .bearer_auth(&token)
        .json(&json!({ "input": { "repo_id": repo_a.id, "base": "main" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        200,
        "a repo of workspace A from B's workflow: {}",
        r.text().await.unwrap_or_default()
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

/// Fixture for the run-LOCATION tests (S3-302): workspace B rooted at its own
/// folder with a repo registered OUTSIDE that root, and a sibling folder that
/// belongs to neither. Returns (ctx, pool, owner, token, ws_b, b_repo dir,
/// foreign dir, workflow in B).
async fn located_workspace(
    tmp: &tempfile::TempDir,
) -> (
    ServerCtx,
    DbPool,
    String,
    otto_core::domain::Workspace,
    std::path::PathBuf,
    std::path::PathBuf,
    otto_core::workflows::Workflow,
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
    let root = tmp.path().join("b-root");
    let b_repo = tmp.path().join("b-repo");
    let foreign = tmp.path().join("elsewhere");
    for d in [&root, &root.join("sub"), &b_repo, &foreign] {
        std::fs::create_dir_all(d).unwrap();
    }
    let ws_b = ctx
        .workspaces
        .create("B", root.to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    ctx.git_store
        .create_repo(NewRepo {
            workspace_id: ws_b.id.clone(),
            name: "b-repo".into(),
            path: b_repo.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let graph: otto_core::workflows::WorkflowGraph = serde_json::from_value(json!({
        "nodes": [{"id": "n", "kind": "log", "params": {}}], "edges": []
    }))
    .unwrap();
    let wf = otto_state::WorkflowsRepo::new(pool.clone())
        .create(&ws_b.id, "Locate", "", "", &graph, &owner.id)
        .await
        .unwrap();
    (ctx, pool, token, ws_b, b_repo, foreign, wf)
}

/// S3-302 (route level): a manual run's `working_directory` / `repos` may name
/// the workflow's workspace root or a registered repo (of ANY workspace), and
/// paths inside them. The caller here is root, so an existing absolute folder
/// outside those roots is also accepted. Relative paths, `..`, `/`, a missing
/// path, a file, and an unknown repo name stay 400, before any run row.
#[tokio::test]
async fn manual_run_location_is_limited_to_registered_repos() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, token, ws_b, b_repo, foreign, wf) = located_workspace(&tmp).await;
    let (origin, server) = serve(&ctx).await;
    let client = reqwest::Client::new();
    let run = |input: serde_json::Value| {
        client
            .post(format!("{origin}/workflows/{}/run", wf.id))
            .bearer_auth(&token)
            .json(&json!({ "input": input }))
            .send()
    };
    let escape = format!("{}/../elsewhere", ws_b.root_path);
    let missing = foreign.join("no-such-folder");
    let climb = foreign.join("missing-child/../still-missing");
    let file = foreign.join("notes.txt");
    std::fs::write(&file, "x").unwrap();
    let refused = [
        json!({"working_directory": "relative/dir"}),
        json!({"working_directory": climb.to_string_lossy()}),
        json!({"working_directory": escape}),
        json!({"working_directory": "/"}),
        json!({"working_directory": missing.to_string_lossy()}),
        json!({"working_directory": file.to_string_lossy()}),
        json!({"repos": [{"repo": "repo-of-another-ws", "name": "main", "type": "branch"}]}),
        json!({"repos": [{"repo": "./relative", "name": "main", "type": "branch"}]}),
    ];
    for input in refused {
        let r = run(input.clone()).await.unwrap();
        assert_eq!(r.status(), 400, "{input}");
        let body: serde_json::Value = r.json().await.unwrap();
        let msg = body["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("registered") || msg.contains("absolute") || msg.contains("'..'"),
            "{input}: {body}"
        );
        // A human root caller is allowed an existing host directory, so a path
        // that still fails says what that exception requires.
        if msg.contains("existing host directory") {
            assert!(
                msg.contains("already exist")
                    && msg.contains("be a directory")
                    && msg.contains("no '..'"),
                "{input}: {body}"
            );
        }
    }
    // Every human-root refusal appends the same host-directory rule. The
    // specific reason depends on which check failed: a relative path is not
    // described as an existing host directory.
    let explained = [
        (
            json!({"working_directory": "relative/dir"}),
            "absolute path",
        ),
        (
            json!({"working_directory": climb.to_string_lossy()}),
            "may not contain '..'",
        ),
        (
            json!({"working_directory": escape}),
            "existing host directory",
        ),
        (
            json!({"working_directory": missing.to_string_lossy()}),
            "existing host directory",
        ),
        (
            json!({"working_directory": file.to_string_lossy()}),
            "existing host directory",
        ),
        (json!({"working_directory": "/"}), "existing host directory"),
    ];
    for (input, reason) in explained {
        let r = run(input.clone()).await.unwrap();
        let body: serde_json::Value = r.json().await.unwrap();
        let msg = body["message"].as_str().unwrap_or("");
        assert!(
            msg.contains(reason)
                && msg.contains("already exist")
                && msg.contains("be a directory")
                && msg.contains("no '..'")
                && msg.contains("filesystem root"),
            "root failure must explain the host-directory rule ({reason}): {input}: {body}"
        );
    }
    // Sweep 2026-10-11 A2: an unregistered `org/name` is a repo NAME, so the
    // refusal says it is not registered (it used to say "must be an absolute
    // path", treating the slash as a path).
    let r = run(json!({"repos": [{"repo": "org/name", "name": "main", "type": "branch"}]}))
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let body: serde_json::Value = r.json().await.unwrap();
    let msg = body["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("'org/name' is not a registered repo"),
        "{body}"
    );
    assert!(
        otto_state::WorkflowsRepo::new(pool.clone())
            .list_runs(&wf.id)
            .await
            .unwrap()
            .is_empty(),
        "no run row is created for a refused location"
    );
    // Inside the root, a registered repo, a path inside one, and — because
    // this caller is root — an existing unregistered directory.
    for wd in [
        ws_b.root_path.clone(),
        format!("{}/sub", ws_b.root_path),
        b_repo.to_string_lossy().into_owned(),
        format!("{}/src/new", b_repo.display()),
        foreign.to_string_lossy().into_owned(),
        format!("{}, {}", b_repo.display(), foreign.display()),
    ] {
        let r = run(json!({"working_directory": wd})).await.unwrap();
        assert_eq!(
            r.status(),
            200,
            "{wd}: {}",
            r.text().await.unwrap_or_default()
        );
    }
    let r = run(json!({"repos": [{"repo": "b-repo", "name": "main", "type": "branch"}]}))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = run(json!({
        "repos": [{"repo": foreign.to_string_lossy(), "name": foreign.to_string_lossy()}]
    }))
    .await
    .unwrap();
    assert_eq!(
        r.status(),
        200,
        "root manual run: {}",
        r.text().await.unwrap_or_default()
    );
    // A repo registered in ANOTHER workspace: accepted by path, name and id.
    let a_repo = tmp.path().join("a-repo");
    std::fs::create_dir_all(&a_repo).unwrap();
    let ws_a = ctx
        .workspaces
        .create(
            "A",
            tmp.path().join("a-root").to_str().unwrap(),
            &wf.created_by,
        )
        .await
        .unwrap();
    let a = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws_a.id.clone(),
            name: "a-repo".into(),
            path: a_repo.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    for input in [
        json!({"working_directory": a_repo.to_string_lossy()}),
        json!({"repos": [{"repo": "a-repo", "name": "main", "type": "branch"}]}),
        json!({"repo_id": a.id}),
    ] {
        let r = run(input.clone()).await.unwrap();
        assert_eq!(
            r.status(),
            200,
            "{input}: {}",
            r.text().await.unwrap_or_default()
        );
    }
    server.abort();
}

/// The host-folder exception is the root user on a manual run. A workspace
/// Editor and a workspace Admin, both with Workflows Edit, still get 400 for
/// an existing folder that is not a registered repo. Their own workspace root
/// still runs, so the refusal is the location rule.
#[tokio::test]
async fn manual_run_outside_registered_repos_stays_refused_for_non_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, _token, ws_b, _b_repo, foreign, wf) = located_workspace(&tmp).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let grants = otto_state::GrantsRepo::new(pool.clone());
    let editor = users
        .create("editor", "unused", "Editor", false)
        .await
        .unwrap();
    let admin = users
        .create("ws-admin", "unused", "Workspace Admin", false)
        .await
        .unwrap();
    for user in [&editor, &admin] {
        grants
            .set_grants(&user.id, &[(Feature::Workflows, Capability::Edit)])
            .await
            .unwrap();
    }
    ctx.workspaces
        .set_member(&ws_b.id, &editor.id, WorkspaceRole::Editor)
        .await
        .unwrap();
    ctx.workspaces
        .set_member(&ws_b.id, &admin.id, WorkspaceRole::Admin)
        .await
        .unwrap();
    let editor_token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&editor.id)
        .await
        .unwrap();
    let admin_token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&admin.id)
        .await
        .unwrap();
    let (origin, server) = serve(&ctx).await;
    let client = reqwest::Client::new();
    for token in [editor_token, admin_token] {
        let foreign_run = client
            .post(format!("{origin}/workflows/{}/run", wf.id))
            .bearer_auth(&token)
            .json(&json!({ "input": {"working_directory": foreign.to_string_lossy()} }))
            .send()
            .await
            .unwrap();
        assert_eq!(foreign_run.status(), 400);
        let body: serde_json::Value = foreign_run.json().await.unwrap();
        let msg = body["message"].as_str().unwrap_or("");
        assert!(
            msg.contains("not a registered repo") && !msg.contains("existing host directory"),
            "non-root refusal stays the registered-repo rule: {body}"
        );
        let inside = client
            .post(format!("{origin}/workflows/{}/run", wf.id))
            .bearer_auth(&token)
            .json(&json!({ "input": {"working_directory": ws_b.root_path} }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            inside.status(),
            200,
            "{}",
            inside.text().await.unwrap_or_default()
        );
    }
    server.abort();
}

/// A root user's agent session token is a different credential class from the
/// person's login. It may still run inside the workspace root, and an existing
/// folder outside registered repos stays 400 with the registered-repo wording.
#[tokio::test]
async fn root_agent_token_does_not_get_the_host_folder_exception() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, _token, ws_b, _b_repo, foreign, wf) = located_workspace(&tmp).await;
    let now = chrono::Utc::now().to_rfc3339();
    let session_id = "root-agent-session";
    sqlx::query(
        "INSERT INTO sessions \
         (id, workspace_id, kind, provider, title, status, cwd, created_by, created_at, last_active_at) \
         VALUES (?, ?, 'agent', 'claude', 'root agent', 'idle', ?, ?, ?, ?)",
    )
    .bind(session_id)
    .bind(&ws_b.id)
    .bind(&ws_b.root_path)
    .bind(&wf.created_by)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    let (agent_token, _) = otto_rbac::AuthRepo::new(pool.clone())
        .issue_session_api_token(&wf.created_by, &session_id.to_string())
        .await
        .unwrap();
    let authed = otto_rbac::AuthRepo::new(pool.clone())
        .authenticate(&agent_token)
        .await
        .unwrap();
    assert_eq!(authed.managed_session_id.as_deref(), Some(session_id));
    assert!(!otto_server::ui_bridge::is_human(&authed));

    let (origin, server) = serve(&ctx).await;
    let client = reqwest::Client::new();
    let foreign_run = client
        .post(format!("{origin}/workflows/{}/run", wf.id))
        .bearer_auth(&agent_token)
        .json(&json!({ "input": {"working_directory": foreign.to_string_lossy()} }))
        .send()
        .await
        .unwrap();
    assert_eq!(foreign_run.status(), 400);
    let body: serde_json::Value = foreign_run.json().await.unwrap();
    let msg = body["message"].as_str().unwrap_or("");
    assert!(
        msg.contains("not a registered repo") && !msg.contains("existing host directory"),
        "agent credential of root stays on the registered-repo rule: {body}"
    );
    let inside = client
        .post(format!("{origin}/workflows/{}/run", wf.id))
        .bearer_auth(&agent_token)
        .json(&json!({ "input": {"working_directory": ws_b.root_path} }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        inside.status(),
        200,
        "{}",
        inside.text().await.unwrap_or_default()
    );
    server.abort();
}

/// S3-302 (chat path): a chat `Working Directory:` outside every registered
/// repo starts nothing and the reply in the thread says why. The workflow
/// owner is root; that does not widen an external trigger.
#[tokio::test]
async fn chat_working_directory_outside_the_workspace_is_refused_in_the_thread() {
    use otto_channels::workflow_trigger::WorkflowChatTrigger;
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, _token, ws_b, _b_repo, foreign, wf) = located_workspace(&tmp).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .get(&wf.created_by)
        .await
        .unwrap();
    assert!(
        owner.is_root,
        "this workflow's owner is root; chat must still refuse the foreign folder"
    );
    let trigger = otto_workflows::chat::WorkflowChatTriggerImpl { ctx: ctx.clone() };
    let text = format!(
        "@otto\nAction: Workflow\nName: Locate\nMsg: do it\nWorking Directory: {}\n",
        foreign.display()
    );
    let ack = trigger
        .try_start(&ws_b.id, "slack", "C1", Some("t1"), "U1", &text)
        .await
        .expect("the thread gets a reply");
    assert!(ack.reply.contains("Can't start"), "{}", ack.reply);
    assert!(ack.reply.contains("not a registered repo"), "{}", ack.reply);
    assert!(
        !ack.reply.contains("existing host directory"),
        "an external trigger does not offer the root host-directory rule: {}",
        ack.reply
    );
    assert!(
        otto_state::WorkflowsRepo::new(pool.clone())
            .list_runs(&wf.id)
            .await
            .unwrap()
            .is_empty(),
        "no run row is created"
    );
}

/// A channel binding matches every message — a reply in the thread of the
/// run it already started must not launch a parallel copy of the workflow.
#[tokio::test]
async fn a_reply_in_a_bound_runs_thread_does_not_start_a_second_run() {
    use otto_channels::workflow_trigger::WorkflowChatTrigger;
    let tmp = tempfile::TempDir::new().unwrap();
    let (ctx, pool, _token, ws_b, _b_repo, _foreign, wf) = located_workspace(&tmp).await;
    otto_state::TriggersRepo::new(pool.clone())
        .create(otto_state::NewWorkflowTrigger {
            workflow_id: wf.id.clone(),
            kind: "chat".into(),
            spec: json!({"channel": "slack", "chat": "C1"}),
            enabled: true,
        })
        .await
        .unwrap();
    let runs = otto_state::WorkflowsRepo::new(pool.clone());
    let active = runs
        .create_run(
            &wf.id,
            &ws_b.id,
            &json!({"trigger": "chat", "channel": "slack", "chat": "C1", "thread": "t1",
                    "origin_workspace_id": ws_b.id}),
            None,
        )
        .await
        .unwrap();
    let trigger = otto_workflows::chat::WorkflowChatTriggerImpl { ctx: ctx.clone() };
    let ack = trigger
        .try_start(
            &ws_b.id,
            "slack",
            "C1",
            Some("t1"),
            "U1",
            "also check the docs",
        )
        .await
        .expect("the thread gets a reply");
    assert!(ack.reply.contains("already running"), "{}", ack.reply);
    assert!(ack.reply.contains(&active.id), "{}", ack.reply);
    assert_eq!(runs.list_runs(&wf.id).await.unwrap().len(), 1);
}
