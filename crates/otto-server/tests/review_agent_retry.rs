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

/// A fix launch must be a completed, pinned review and must reuse its existing
/// paused goal. Exercise the HTTP boundary without starting a real provider.
#[tokio::test]
async fn fix_goal_checks_review_identity_and_reuses_the_existing_loop() {
    use otto_core::domain::{GoalLoopPhase, GoalLoopStatus, ReviewScopePlan, ReviewStatus};
    let tmp = tempfile::tempdir().unwrap();
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
    let ws = ctx
        .workspaces
        .create("Fix goal", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let repo = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws.id.clone(),
            name: "repo".into(),
            path: tmp.path().to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let review = ctx.reviews_store.create_review(&repo.id, 7).await.unwrap();
    let (api, root) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api, root);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/api/v1/reviews/{}/fix-goal",
        listener.local_addr().unwrap(),
        review.id
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::new();
    let request = || {
        http.post(&endpoint)
            .bearer_auth(&token)
            .json(&serde_json::json!({"target_score":9.5}))
    };
    let viewer = otto_state::UsersRepo::new(pool.clone())
        .create("viewer", "unused", "Viewer", false)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, ?, 'viewer')",
    )
    .bind(&ws.id)
    .bind(&viewer.id)
    .execute(&pool)
    .await
    .unwrap();
    otto_state::GrantsRepo::new(pool.clone())
        .set_grants(
            &viewer.id,
            &[(
                otto_core::domain::Feature::Git,
                otto_core::domain::Capability::Edit,
            )],
        )
        .await
        .unwrap();
    let viewer_token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&viewer.id)
        .await
        .unwrap();
    assert_eq!(
        http.post(&endpoint)
            .bearer_auth(viewer_token)
            .json(&serde_json::json!({"target_score":9.5}))
            .send()
            .await
            .unwrap()
            .status(),
        403,
        "feature access cannot bypass workspace Editor requirement"
    );
    assert_eq!(
        request().send().await.unwrap().status(),
        400,
        "running review cannot launch fixes"
    );
    let head = "a".repeat(40);
    let mut scope: ReviewScopePlan = serde_json::from_value(serde_json::json!({
        "head_sha":"b".repeat(40), "base_sha":"c".repeat(40), "summary":"Permission change",
        "related_modules":[], "risks":[], "lenses":[]
    }))
    .unwrap();
    ctx.reviews_store
        .set_scope_plan(&review.id, &scope)
        .await
        .unwrap();
    ctx.reviews_store
        .set_run_context(
            &review.id,
            &otto_state::reviews::ReviewRunContext {
                head_sha: Some(head.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ctx.reviews_store
        .set_status(&review.id, ReviewStatus::Done, None)
        .await
        .unwrap();
    let rejected = request().send().await.unwrap();
    assert_eq!(rejected.status(), 400);
    assert!(rejected
        .text()
        .await
        .unwrap()
        .contains("scope and checkout differ"));
    assert!(ctx
        .goal_loops_repo
        .list_by_workspace(&ws.id)
        .await
        .unwrap()
        .is_empty());

    ctx.reviews_store
        .set_status(&review.id, ReviewStatus::Running, None)
        .await
        .unwrap();
    scope.head_sha = head;
    ctx.reviews_store
        .set_scope_plan(&review.id, &scope)
        .await
        .unwrap();
    ctx.reviews_store
        .set_status(&review.id, ReviewStatus::Done, None)
        .await
        .unwrap();
    let checked = ctx.reviews_store.get_review(&review.id).await.unwrap();
    let (definition, config) = otto_review::fix_goal::build(&checked, 9.5, "codex").unwrap();
    let goal = ctx
        .goal_loops_repo
        .create(otto_state::NewGoalLoop {
            workspace_id: ws.id.clone(),
            name: definition.title.clone(),
            repo_path: repo.path,
            definition,
            config,
            limits: Default::default(),
            created_by: owner.id,
        })
        .await
        .unwrap();
    ctx.goal_loops_repo
        .update_runtime(
            &goal.id,
            GoalLoopStatus::Paused,
            GoalLoopPhase::Planning,
            0,
            0,
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let response = request().send().await.unwrap();
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["id"], goal.id);
        assert_eq!(
            body["status"], "paused",
            "existing work must not silently resume"
        );
    }
    assert_eq!(
        ctx.goal_loops_repo
            .list_by_workspace(&ws.id)
            .await
            .unwrap()
            .len(),
        1
    );
    server.abort();
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

    // backend-correctness-3-01: a retry that fails AFTER its claim (no stored
    // diff to re-materialise) puts the settled row back instead of leaving it
    // `pending` for good.
    ctx.reviews_store
        .set_status(&review.id, otto_core::domain::ReviewStatus::Done, None)
        .await
        .unwrap();
    let rows = vec![
        agent("error"),
        agent("pending"),
        agent("done"),
        agent("done"),
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();
    ctx.reviews_store
        .set_agent_prompt(&review.id, 0, "review the diff")
        .await
        .unwrap();
    let r = client
        .post(format!("{origin}/reviews/{}/agents/0/retry", review.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "no stored diff");
    let body: serde_json::Value = r.json().await.unwrap();
    assert!(body.to_string().contains("no stored diff"), "{body}");
    let after = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(
        after.agents[0].status, "error",
        "the claimed row is restored"
    );
    assert_eq!(after.agents[0].session_id.as_deref(), Some("sess-live"));

    // An orphaned `pending` row (no cancel flag, settled review) is stoppable,
    // so Retry and the summarizer re-run are reachable again.
    let r = client
        .post(format!("{origin}/reviews/{}/agents/1/stop", review.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202, "stop settles an orphaned pending row");
    let after = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(after.agents[1].status, "error");
    assert_eq!(after.agents[1].note, "stopped by user");
    // A settled row still is not stoppable.
    let r = client
        .post(format!("{origin}/reviews/{}/agents/2/stop", review.id))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409, "a done agent is not stopped");
    server.abort();
}

/// concurrency-async-3-01: `submit_prompt` (paste → echo wait → Enter) holds
/// the session's submit lock, so a `submit_text` racing into the same session
/// lands as its own paste + Enter instead of between the prompt's paste and
/// its Enter (one merged prompt plus an empty one).
#[tokio::test]
async fn submit_prompt_and_submit_text_do_not_interleave() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Submit lock", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    // The line discipline echoes what is typed (so the paste "echoes" on the
    // screen) while `cat` records exactly what reached the PTY; each Enter
    // ends one recorded line.
    let sink = tmp.path().join("pty-input");
    let session = ctx
        .manager
        .create(
            &ws,
            &owner.id,
            otto_core::api::CreateSessionReq {
                kind: otto_core::domain::SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("submit lock".into()),
                cwd: None,
                connection_id: None,
                meta: None,
                model: None,
            },
            Some(otto_pty::CommandSpec {
                program: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    format!("echo ready; exec cat > '{}'", sink.display()),
                ],
                cwd: Some(tmp.path().to_string_lossy().into()),
                env: vec![],
            }),
        )
        .await
        .unwrap();
    let sid = session.id.clone();
    let prompt = tokio::spawn({
        let manager = ctx.manager.clone();
        let sid = sid.clone();
        async move {
            otto_sessions::submit::submit_prompt(&manager, &sid, "from-prompt please review").await
        }
    });
    // Race in the moment the prompt's paste is on screen, before its Enter.
    let mut echoed = false;
    for _ in 0..200 {
        if let Some(h) = ctx.manager.live_handle(&sid) {
            if String::from_utf8_lossy(&h.scrollback(50)).contains("from-prompt") {
                echoed = true;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(echoed, "the prompt's paste never echoed");
    ctx.manager.submit_text(&sid, "from-text").await.unwrap();
    assert!(
        prompt.await.unwrap(),
        "submit_prompt reported a dead session"
    );
    let mut lines = Vec::new();
    for _ in 0..100 {
        let got = std::fs::read_to_string(&sink).unwrap_or_default();
        lines = got
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if lines.len() >= 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    lines.sort();
    assert_eq!(
        lines,
        vec![
            "\x1b[200~from-prompt please review\x1b[201~".to_string(),
            "\x1b[200~from-text\x1b[201~".to_string(),
        ],
        "each submission must be its own paste + Enter"
    );
    let _ = ctx.manager.kill_session(&sid).await;
}
