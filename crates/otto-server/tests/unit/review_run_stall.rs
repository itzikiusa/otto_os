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
            Some(Instant::now() + Duration::from_secs(3_600)),
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
                Some(Instant::now() + Duration::from_secs(3_600)),
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

/// BUG-14: in a workspace whose resolved workflow default is `codex` while the
/// global `default_provider` setting is `claude`, both `grade_goals` (checks +
/// goals with no explicit providers/summarizer) and `plan_review` (explicit
/// lenses with no provider) must use `env.default_provider` (`codex`), never
/// fall back to the global `claude` setting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspace_default_provider_wins_over_global_claude_for_goals_and_explicit_lenses() {
    use otto_agent_protocol::{scripted, AgentRuntime, RuntimeConfig, SessionManagerHost};
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "codex-ws").await;
    sqlx::query("UPDATE workspaces SET settings_json = ? WHERE id = 'codex-ws'")
        .bind(json!({"default_provider": "codex"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    otto_state::SettingsRepo::new(pool.clone())
        .put("default_provider", &json!("claude"))
        .await
        .unwrap();
    otto_state::SettingsRepo::new(pool.clone())
        .put("agent_admission_enabled", &json!(false))
        .await
        .unwrap();
    let mut ctx = test_ctx(&pool, dir.path().join("data")).await;
    let host = SessionManagerHost {
        manager: std::sync::Arc::clone(&ctx.manager),
        workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
        publish: Box::new(|_| {}),
        notices: otto_agent_protocol::attention::AttentionNotices::new(
            otto_state::NotificationsRepo::new(pool.clone()),
            std::sync::Arc::new(|_| {}),
        ),
    };
    ctx.agent_runtime = AgentRuntime::new(
        otto_state::AgentRuntimeRepo::new(pool.clone()),
        std::sync::Arc::new(host),
        scripted::drivers(),
        RuntimeConfig {
            attachments_dir: dir.path().join("attachments"),
            transport: Some(scripted::transport(Vec::new())),
            ..RuntimeConfig::default()
        },
    );
    let ws = otto_state::WorkspacesRepo::new(pool.clone())
        .get(&"codex-ws".into())
        .await
        .unwrap();
    let user = root_user();
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    let graph: WorkflowGraph = serde_json::from_value(json!({"nodes":[],"edges":[]})).unwrap();
    let runs = WorkflowsRepo::new(pool.clone());
    let wf = runs
        .create(&ws.id, "CodexGoals", "", "", &graph, &user.id)
        .await
        .unwrap();
    let run = runs
        .create_run(&wf.id, &ws.id, &Value::Null, Some(&user.id))
        .await
        .unwrap();
    let files = std::sync::Arc::new(crate::workflow_context::RunContextFiles::create(
        &ctx.data_dir,
        &run.id,
    ));
    let env = RunEnv {
        budget: budget::ActiveBudget::new(Duration::from_secs(60)),
        run_id: run.id.clone(),
        wf_name: wf.name.clone(),
        run_cwd: dir.path().to_string_lossy().into_owned(),
        run_base: None,
        files,
        default_provider: "codex".into(),
        run_input: json!({}),
    };
    let scope = StepScope {
        step_no: 1,
        iter: None,
        inner_idx: None,
    };
    let (session_tx, _session_rx) = tokio::sync::mpsc::unbounded_channel();
    let (log_tx, _log_rx) = tokio::sync::mpsc::unbounded_channel();
    let (activity_tx, _activity_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = ProgressSink::disabled();

    // Case 1: explicit lens list with no provider must inherit env.default_provider ("codex").
    let lens_node: WorkflowNode = serde_json::from_value(json!({
        "id": "rev-lenses",
        "kind": "review_run",
        "name": "Review lenses",
        "params": {"lenses": ["correctness"]},
    }))
    .unwrap();
    let lens_call = NodeCall {
        ctx: &ctx,
        ws: &ws,
        user: &user,
        node: &lens_node,
        input: json!({}),
        env: &env,
        scope: &scope,
        session_tx: &session_tx,
        log_tx: &log_tx,
        activity_tx: &activity_tx,
        progress: &progress,
    };
    let (lens_plan, _) = plan_review(
        &lens_call,
        otto_core::domain::ReviewMode::FanOut,
        1,
        Vec::new(),
    )
    .await;
    let lens_cfg = lens_plan
        .cfg_override
        .as_ref()
        .expect("explicit lenses build a base override");
    assert_eq!(
        lens_cfg.agents[0].provider, "codex",
        "explicit lens without provider must use env.default_provider, not global claude"
    );
    assert_eq!(
        lens_cfg.summarizer.provider, "codex",
        "explicit lens summarizer must use env.default_provider, not global claude"
    );

    // Case 2: required checks + goals with no explicit providers/summarizer must grade
    // goals on env.default_provider ("codex"), never global "claude".
    let checks_node: WorkflowNode = serde_json::from_value(json!({
        "id": "rev-checks",
        "kind": "review_run",
        "name": "Review checks",
        "params": {
            "checks": [{"name": "unit", "cmd": "cargo test"}],
            "goals": ["tests pass"],
        },
    }))
    .unwrap();
    let checks_call = NodeCall {
        ctx: &ctx,
        ws: &ws,
        user: &user,
        node: &checks_node,
        input: json!({}),
        env: &env,
        scope: &scope,
        session_tx: &session_tx,
        log_tx: &log_tx,
        activity_tx: &activity_tx,
        progress: &progress,
    };
    let (checks_plan, _) = plan_review(
        &checks_call,
        otto_core::domain::ReviewMode::FanOut,
        1,
        Vec::new(),
    )
    .await;
    assert!(checks_plan.cfg_override.is_none());
    let launched = Launched {
        repo_id: "r1".into(),
        worktree: dir.path().to_string_lossy().into_owned(),
        review_id: "rv-1".into(),
        base: "main".into(),
        no_changes: false,
        tag: String::new(),
    };
    let mut logs = Vec::new();
    let _ = grade_goals(&checks_plan, &launched, 90, &mut logs).await;
    let sessions = ctx
        .agent_runtime
        .list_filtered(&ws.id, None, Some("workflow"), None)
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1, "one goal-grading session must run");
    assert_eq!(
        sessions[0].provider,
        otto_core::agent_runtime::RuntimeProvider::Codex,
        "goal grading must run on workspace default codex, not global claude"
    );
}
