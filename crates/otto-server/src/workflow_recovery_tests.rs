//! Recovery regressions exercise real node execution against isolated state.
use super::*;
use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};

#[tokio::test]
async fn failed_loop_iteration_cannot_satisfy_until_from_previous_output() {
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "recovery-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let user = root_user();
    let ws = otto_state::WorkspacesRepo::new(pool.clone())
        .get(&"recovery-ws".into())
        .await
        .unwrap();
    let node: WorkflowNode = serde_json::from_value(json!({"id":"loop","kind":"loop","params":{
        "max_iterations":2,"continue_on_error":true,"until":"last.passed",
        "steps":[{"kind":"agent_prompt","name":"broken","params":{"prompt":""},
            "retry":{"max_attempts":0}}]
    }}))
    .unwrap();
    let graph = WorkflowGraph {
        nodes: vec![node.clone()],
        edges: vec![],
    };
    let repo = WorkflowsRepo::new(pool);
    let wf = repo
        .create(&ws.id, "Recovery", "", "", &graph, &user.id)
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&user.id))
        .await
        .unwrap();
    let env = RunEnv {
        budget: budget::ActiveBudget::new(RUN_WALL_CLOCK_TIMEOUT),
        run_id: run.id.clone(),
        wf_name: wf.name.clone(),
        run_cwd: dir.path().to_string_lossy().into(),
        run_base: None,
        files: std::sync::Arc::new(crate::workflow_context::RunContextFiles::disabled(&run.id)),
        default_provider: "claude".into(),
        run_input: json!({}),
    };
    let (sessions, _sr) = tokio::sync::mpsc::unbounded_channel();
    let (logs, _lr) = tokio::sync::mpsc::unbounded_channel();
    let (activity, _ar) = tokio::sync::mpsc::unbounded_channel();
    let (out, _) = execute_node(
        &ctx,
        &ws,
        &user,
        &node,
        json!({"passed":true}),
        &env,
        &StepScope {
            step_no: 1,
            iter: None,
            inner_idx: None,
        },
        &sessions,
        &logs,
        &activity,
        &ProgressSink::disabled(),
    )
    .await
    .unwrap();
    assert_eq!(
        out["satisfied"], false,
        "a failed iteration must not inspect stale last.passed"
    );
    assert_eq!(out["iterations"], 2);
    assert!(out["_workflow_error"].as_str().is_some());
    assert!(out["history"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["error"].is_string()));
    run_workflow(
        ctx,
        ws,
        wf,
        run.id.clone(),
        json!({"passed":true}),
        RunScope::default(),
        None,
    )
    .await;
    let settled = repo.get_run(&run.id).await.unwrap();
    assert_eq!(settled.status, RunStatus::Error);
    assert_eq!(settled.nodes[0].status, NodeStatus::Error);
    assert!(settled.nodes[0].output.as_ref().unwrap()["_workflow_error"].is_string());
}

#[tokio::test]
async fn completed_review_gate_failure_retains_result_and_reuses_review_on_reentry() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    ] {
        let output = otto_git::hardened_command()
            .args(args)
            .current_dir(dir.path())
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let pool = mem_pool().await;
    seed_workspace(&pool, "recovery-review-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let user = root_user();
    let ws = otto_state::WorkspacesRepo::new(pool.clone())
        .get(&"recovery-review-ws".into())
        .await
        .unwrap();
    let git_repo = ctx
        .git_store
        .create_repo(otto_state::git::NewRepo {
            workspace_id: ws.id.clone(),
            name: "Review fixture".into(),
            path: dir.path().to_string_lossy().into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let node: WorkflowNode = serde_json::from_value(json!({"id":"review","kind":"review_run",
        "params":{"repo_id":git_repo.id,"worktree":git_repo.path,"base":"main","require_pass":true},
        "retry":{"max_attempts":3,"backoff_ms":1}}))
    .unwrap();
    let repo = WorkflowsRepo::new(pool);
    let wf = repo
        .create(
            &ws.id,
            "Review recovery",
            "",
            "",
            &WorkflowGraph {
                nodes: vec![node.clone()],
                edges: vec![],
            },
            &user.id,
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&user.id))
        .await
        .unwrap();
    let env = RunEnv {
        budget: budget::ActiveBudget::new(RUN_WALL_CLOCK_TIMEOUT),
        run_id: run.id.clone(),
        wf_name: wf.name,
        run_cwd: git_repo.path,
        run_base: None,
        files: std::sync::Arc::new(crate::workflow_context::RunContextFiles::disabled(&run.id)),
        default_provider: "claude".into(),
        run_input: json!({}),
    };
    let (sessions, _sr) = tokio::sync::mpsc::unbounded_channel();
    let (logs, _lr) = tokio::sync::mpsc::unbounded_channel();
    let (activity, _ar) = tokio::sync::mpsc::unbounded_channel();
    let mut first_id = None;
    for _ in 0..2 {
        let (out, _) = execute_node(
            &ctx,
            &ws,
            &user,
            &node,
            json!({}),
            &env,
            &StepScope {
                step_no: 1,
                iter: None,
                inner_idx: None,
            },
            &sessions,
            &logs,
            &activity,
            &ProgressSink::disabled(),
        )
        .await
        .unwrap();
        assert_eq!(out["passed"], false);
        assert_eq!(out["no_changes"], true);
        assert!(out["_workflow_error"]
            .as_str()
            .unwrap()
            .contains("empty diff"));
        let id = out["review_id"].as_str().unwrap().to_owned();
        if let Some(first_id) = &first_id {
            assert_eq!(&id, first_id);
        } else {
            first_id = Some(id);
        }
    }
}

#[test]
fn review_gate_requires_coverage_and_explicit_fallback_policy() {
    let mut review: otto_core::domain::Review = serde_json::from_value(json!({
        "id":"r","repo_id":"repo","pr_number":0,"status":"done","error":null,
        "comments":[],"created_at":"2026-10-08T00:00:00Z","agents":[
            {"name":"correctness","provider":"claude","model":"","status":"error","note":"timeout","comment_count":0},
            {"name":"summary","provider":"claude","model":"","status":"done","note":"clean","comment_count":0}
        ]
    })).unwrap();
    assert!(!review_coverage_complete(&review, false));
    review.agents[0].status = "skipped".into();
    review.agents[0].note = "sibling covered lens".into();
    assert!(review_coverage_complete(&review, false));
    review.agents[0].status = "done".into();
    review.agents[0].note = "partial: one lens missing".into();
    assert!(!review_coverage_complete(&review, true));
    review.agents[0].note = "complete".into();
    review.summary_fallback = Some(true);
    assert!(!review_coverage_complete(&review, false));
    assert!(review_coverage_complete(&review, true));
}

#[tokio::test]
async fn active_budget_interrupts_inflight_node_and_retires_durable_child_review() {
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "budget-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let ws = ctx.workspaces.get(&"budget-ws".into()).await.unwrap();
    let node: WorkflowNode =
        serde_json::from_value(json!({"id":"slow","kind":"delay","params":{"ms":10000}})).unwrap();
    let graph = WorkflowGraph {
        nodes: vec![node.clone()],
        edges: vec![],
    };
    let repo = WorkflowsRepo::new(pool);
    let wf = repo
        .create(&ws.id, "Budget", "", "", &graph, &"root".into())
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&"root".into()))
        .await
        .unwrap();
    let git_repo = ctx
        .git_store
        .create_repo(otto_state::git::NewRepo {
            workspace_id: ws.id.clone(),
            name: "fixture".into(),
            path: dir.path().to_string_lossy().into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let review = ctx
        .reviews_store
        .create_review(&git_repo.id, 0)
        .await
        .unwrap();
    let mut checkpoint = loop_checkpoint(&node, "slow", 1, 0, json!({}));
    checkpoint.node_id = "slow#review.0".into();
    checkpoint.kind = "review_run".into();
    checkpoint.status = NodeStatus::Success;
    checkpoint.output = Some(json!({"review_id":review.id}));
    repo.save_checkpoint(&run.id, &checkpoint).await.unwrap();
    // The child association has not reached node.review_ids yet. Cleanup must
    // use the durable launch record and interrupt the active node, not wait for
    // its ten-second delay to return to the between-node boundary.
    tokio::time::timeout(
        Duration::from_secs(3),
        run_workflow_with_budget(
            ctx.clone(),
            ws,
            wf,
            run.id.clone(),
            json!({}),
            RunScope::default(),
            None,
            Duration::from_millis(100),
        ),
    )
    .await
    .expect("active budget must interrupt the in-flight delay");
    let after = repo.get_run(&run.id).await.unwrap();
    assert_eq!(after.status, RunStatus::Error);
    assert_eq!(after.nodes[0].status, NodeStatus::Error);
    assert_eq!(
        after.nodes[0].attempts,
        Some(1),
        "the delay must have entered execution before timing out"
    );
    assert!(after.nodes[0].error.as_deref().unwrap().contains("budget"));
    assert_eq!(
        ctx.reviews_store.review_status(&review.id).await.unwrap(),
        otto_core::domain::ReviewStatus::Cancelled
    );
}

#[tokio::test]
async fn inner_retry_waits_for_owned_session_and_review_cleanup() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "inner-cleanup-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let ws = ctx
        .workspaces
        .get(&"inner-cleanup-ws".into())
        .await
        .unwrap();
    let node: WorkflowNode = serde_json::from_value(
        json!({"id":"inner","kind":"agent_prompt","params":{"prompt":"fixture"}}),
    )
    .unwrap();
    let repo = WorkflowsRepo::new(pool.clone());
    let wf = repo
        .create(
            &ws.id,
            "Cleanup",
            "",
            "",
            &WorkflowGraph {
                nodes: vec![node.clone()],
                edges: vec![],
            },
            &"root".into(),
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&"root".into()))
        .await
        .unwrap();
    // Persist a session without spawning any process. Manager cleanup still
    // exercises the real ownership check and terminal state transition.
    let session = otto_state::SessionsRepo::new(pool)
        .create(otto_state::sessions::NewSession {
            workspace_id: ws.id.clone(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "fixture".into(),
            title: "attempt".into(),
            cwd: dir.path().to_string_lossy().into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "root".into(),
            meta: json!({"source":"workflow","run_id":run.id}),
        })
        .await
        .unwrap();
    let git_repo = ctx
        .git_store
        .create_repo(otto_state::git::NewRepo {
            workspace_id: ws.id.clone(),
            name: "fixture".into(),
            path: dir.path().to_string_lossy().into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let review = ctx
        .reviews_store
        .create_review(&git_repo.id, 0)
        .await
        .unwrap();
    let env = RunEnv {
        budget: budget::ActiveBudget::new(RUN_WALL_CLOCK_TIMEOUT),
        run_id: run.id.clone(),
        wf_name: wf.name,
        run_cwd: dir.path().to_string_lossy().into(),
        run_base: None,
        files: std::sync::Arc::new(crate::workflow_context::RunContextFiles::disabled(&run.id)),
        default_provider: "fixture".into(),
        run_input: json!({}),
    };
    let (sessions, mut observed) = tokio::sync::mpsc::unbounded_channel();
    let (logs, _log_rx) = tokio::sync::mpsc::unbounded_channel();
    let attempts = AtomicUsize::new(0);
    let ctx_ref = &ctx;
    let sid = &session.id;
    let review_id = &review.id;
    let result = crate::workflow_checkpoint::execute(
        &repo,
        &run.id,
        loop_checkpoint(&node, "loop", 1, 0, json!({})),
        &otto_core::workflows::RetryPolicy {
            max_attempts: 1,
            backoff_ms: 0,
            factor: 1.0,
        },
        true,
        || {
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            run_owned_inner_attempt(
                &ctx,
                &ws,
                &node,
                &env,
                &sessions,
                &logs,
                move |tx| async move {
                    if attempt == 0 {
                        tx.send(AgentAssociation::Session(sid.clone())).unwrap();
                        tx.send(AgentAssociation::Review(review_id.clone()))
                            .unwrap();
                        return Err(otto_core::Error::Upstream("fixture failure".into()));
                    }
                    assert_eq!(
                        ctx_ref.manager.get(sid).await.unwrap().status,
                        otto_core::domain::SessionStatus::Exited
                    );
                    assert_eq!(
                        ctx_ref
                            .reviews_store
                            .review_status(review_id)
                            .await
                            .unwrap(),
                        otto_core::domain::ReviewStatus::Cancelled
                    );
                    Ok((json!({"recovered":true}), vec![]))
                },
            )
        },
    )
    .await
    .unwrap();
    assert_eq!(result.0["recovered"], true);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert!(matches!(observed.try_recv().unwrap(),AgentAssociation::Session(id) if id==session.id));
    assert!(matches!(observed.try_recv().unwrap(),AgentAssociation::Review(id) if id==review.id));
}
