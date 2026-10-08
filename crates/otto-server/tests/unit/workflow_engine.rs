#[test]
fn run_acts_as_its_starter_else_workflow_creator() {
    let wf_owner: Id = "root".into();
    assert_eq!(
        super::acting_user_id(Some("editor-b".into()), &wf_owner),
        "editor-b"
    );
    assert_eq!(super::acting_user_id(None, &wf_owner), "root");
    assert_eq!(
        super::acting_user_id(Some(String::new()), &wf_owner),
        "root"
    );
}

use super::*;
use otto_core::workflows::WorkflowEdge;

/// Migrated in-memory DB; FKs off so a run needs no seeded workspace.
async fn mem_repo() -> WorkflowsRepo {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("migrations");
    WorkflowsRepo::new(pool)
}

/// Perf W1/W12 budget: a running node with no cancel issues ZERO full
/// run reads (`SELECT *` / `nodes_json`) — only the first-tick and
/// every-CANCEL_SAFETY_TICKS status reads — and a cancel wakes it at once
/// via the announcement, not after a poll period.
#[tokio::test]
async fn cancel_watch_reads_status_only_and_wakes_on_announce() {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(false);
    let raw = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&raw)
        .await
        .unwrap();
    let pool = otto_state::DbPool::from(raw);
    let repo = WorkflowsRepo::new(pool.clone());
    let wf = repo
        .create(
            &"ws1".into(),
            "WF",
            "",
            "",
            &WorkflowGraph::default(),
            &"u1".into(),
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &wf.workspace_id, &Value::Null, None)
        .await
        .unwrap();
    let probe = pool.statement_probe();
    probe.reset();
    // 25 ticks of "running" (a scaled-down ~50 s of 2 s ticks), no cancel.
    let mut watch = CancelWatch::new(&run.id, Duration::from_millis(10));
    let idle = tokio::time::timeout(Duration::from_millis(255), watch.next(&repo, || false)).await;
    assert!(idle.is_err(), "nothing to wake for");
    let stmts = probe.take();
    assert!(
        stmts
            .iter()
            .all(|q| !q.contains("SELECT *") && !q.contains("nodes_json")),
        "no full-row reads while running: {stmts:?}"
    );
    assert!(
        (1..=2).contains(&stmts.len()),
        "status reads only on the safety ticks (1st + every {CANCEL_SAFETY_TICKS}): {stmts:?}"
    );
    // A cancel wakes a watch whose tick is far away, at once.
    let mut watch = CancelWatch::new(&run.id, Duration::from_secs(3600));
    let first = tokio::time::timeout(Duration::from_millis(100), watch.next(&repo, || false)).await;
    assert!(first.is_err(), "first tick read the status, not canceled");
    let t = Instant::now();
    let (wake, _) = tokio::join!(watch.next(&repo, || false), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        repo.request_cancel(&run.id).await.unwrap();
    });
    assert_eq!(wake, CancelWake::Canceled);
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "woken by the announcement"
    );
}

#[tokio::test]
async fn cancel_watch_skip_reports_the_approval_park() {
    let repo = mem_repo().await;
    let wf = repo
        .create(
            &"ws1".into(),
            "WF",
            "",
            "",
            &WorkflowGraph::default(),
            &"u1".into(),
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &wf.workspace_id, &Value::Null, None)
        .await
        .unwrap();
    let mut watch = CancelWatch::new(&run.id, Duration::from_millis(5));
    let once = std::sync::atomic::AtomicBool::new(true);
    let wake = watch
        .next(&repo, || {
            once.swap(false, std::sync::atomic::Ordering::SeqCst)
        })
        .await;
    assert_eq!(
        wake,
        CancelWake::Skip {
            parked_at_approval: false
        }
    );
}

#[tokio::test]
async fn retry_backoff_wakes_on_a_cancel() {
    let repo = mem_repo().await;
    let wf = repo
        .create(
            &"ws1".into(),
            "WF",
            "",
            "",
            &WorkflowGraph::default(),
            &"u1".into(),
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &wf.workspace_id, &Value::Null, None)
        .await
        .unwrap();
    // Not canceled: sleeps the (short) backoff out and reports false.
    assert!(!backoff_canceled(&repo, &run.id, Duration::from_millis(50)).await);
    // Canceled: true at once, not after the whole (long) backoff.
    repo.request_cancel(&run.id).await.unwrap();
    let t = Instant::now();
    assert!(backoff_canceled(&repo, &run.id, Duration::from_secs(60)).await);
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_canceled_finalize_never_overwrites_a_retry() {
    let repo = mem_repo().await;
    let wf = repo
        .create(
            &"ws1".into(),
            "WF",
            "",
            "",
            &WorkflowGraph::default(),
            &"u1".into(),
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &wf.workspace_id, &Value::Null, None)
        .await
        .unwrap();
    repo.update_run(&run.id, RunStatus::Running, &[], None, false)
        .await
        .unwrap();
    repo.request_cancel(&run.id).await.unwrap();
    // The user retried before the stale driver finalized its cancel.
    repo.prepare_retry(&run.id, &[], false, &Default::default())
        .await
        .unwrap();
    // The guard finalize uses: canceled ONLY → no write on the retry row.
    let wrote = repo
        .update_run_if(
            &run.id,
            &[RunStatus::Canceled],
            RunStatus::Canceled,
            &[],
            Some("canceled"),
            true,
        )
        .await
        .unwrap();
    assert!(wrote.is_none());
    assert_eq!(
        repo.get_run(&run.id).await.unwrap().status,
        RunStatus::Pending
    );
}

#[test]
fn driver_registry_counts_overlapping_drivers() {
    let id: Id = "run-driver-registry-test".into();
    assert!(!driver_alive(&id));
    let a = DriverGuard::register(&id);
    let b = DriverGuard::register(&id);
    assert!(driver_alive(&id));
    drop(a);
    assert!(driver_alive(&id), "the second driver is still running");
    drop(b);
    assert!(!driver_alive(&id));
}

fn node(id: &str, kind: &str) -> WorkflowNode {
    WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: String::new(),
        x: 0.0,
        y: 0.0,
        params: Value::Null,
        retry: None,
    }
}
fn edge(s: &str, t: &str) -> WorkflowEdge {
    WorkflowEdge {
        id: format!("{s}-{t}"),
        source: s.into(),
        target: t.into(),
        condition: None,
    }
}

/// The daemon-wide run gate: at most `max_parallel_runs()` (default 2)
/// workflow runs execute at once; `spawn_run` parks the rest FIFO, their
/// still-`pending` rows doubling as the persistent queue.
#[tokio::test]
async fn run_gate_caps_parallel_runs_at_two() {
    // The gate is sized once from the env; with an operator override this
    // test's permit arithmetic wouldn't apply — skip rather than mislead.
    if std::env::var("OTTO_WF_MAX_PARALLEL_RUNS").is_ok() {
        return;
    }
    assert_eq!(max_parallel_runs(), 2);
    let gate = run_gate();
    let a = gate.try_acquire().expect("first run starts");
    let b = gate.try_acquire().expect("second run starts");
    assert!(gate.try_acquire().is_err(), "third run must queue");
    drop(a);
    let c = gate
        .try_acquire()
        .expect("a slot frees when a run finishes");
    drop(b);
    drop(c);
}

#[test]
fn topo_orders_a_chain() {
    let g = WorkflowGraph {
        nodes: vec![
            node("c", "log"),
            node("a", "manual_trigger"),
            node("b", "log"),
        ],
        edges: vec![edge("a", "b"), edge("b", "c")],
    };
    assert_eq!(topo_order(&g).unwrap(), vec!["a", "b", "c"]);
}

#[test]
fn topo_detects_cycle() {
    let g = WorkflowGraph {
        nodes: vec![node("a", "log"), node("b", "log")],
        edges: vec![edge("a", "b"), edge("b", "a")],
    };
    assert!(topo_order(&g).is_err());
}

fn nstate(id: &str, status: NodeStatus) -> NodeRunState {
    NodeRunState {
        node_id: id.into(),
        status,
        output: None,
        error: None,
        logs: vec![],
        started_at: None,
        duration_ms: None,
        attempts: None,
        sessions: vec![],
        review_ids: Vec::new(),
        activity: None,
    }
}

fn mk_run(status: RunStatus, nodes: Vec<NodeRunState>) -> WorkflowRun {
    WorkflowRun {
        checkpoints: vec![],
        id: "r1".into(),
        workflow_id: "wf1".into(),
        workspace_id: "ws1".into(),
        status,
        input: Value::Null,
        nodes,
        error: None,
        started_at: chrono::Utc::now(),
        finished_at: None,
        rev: 0,
        waiting_approval: false,
        approval_node_id: None,
        approved_by: None,
        created_by: None,
        approval_note: None,
        approved_at: None,
        workflow_version: None,
        proof_pack_id: None,
        resume_attempts: 0,
        context_dir: None,
    }
}

/// A restart that caught an idempotent step mid-flight resumes AT it:
/// the step flips back to Pending, finished siblings keep their states.
#[test]
fn classify_resume_reenters_at_interrupted_idempotent_step() {
    let g = WorkflowGraph {
        nodes: vec![
            node("a", "manual_trigger"),
            node("b", "agent_prompt"),
            node("c", "git_pr"),
        ],
        edges: vec![edge("a", "b"), edge("b", "c")],
    };
    let run = mk_run(
        RunStatus::Running,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("b", NodeStatus::Running),
            nstate("c", NodeStatus::Pending),
        ],
    );
    match classify_resume(&g, &run, None) {
        ResumeDecision::Resume { scope, nodes } => {
            assert_eq!(scope.start_node, None);
            assert!(scope.continue_unfinished);
            assert!(!scope.only_node && !scope.adopt_start);
            assert_eq!(
                nodes[1].status,
                NodeStatus::Pending,
                "interrupted step re-runs"
            );
            assert_eq!(
                nodes[0].status,
                NodeStatus::Success,
                "finished step adopted"
            );
        }
        other => panic!("expected Resume, got {other:?}"),
    }
}

#[test]
fn retry_consumer_receives_adopted_output_but_fresh_partial_run_uses_input() {
    let scope = std::collections::HashSet::from(["consumer".to_string()]);
    let prior = vec![nstate("fetch", NodeStatus::Success)];
    let outputs = HashMap::from([("fetch".to_string(), json!({"fetched":42}))]);
    assert!(dependency_is_relevant("fetch", Some(&scope), Some(&prior)));
    assert!(!dependency_is_relevant("fetch", Some(&scope), None));
    assert_eq!(
        assemble_input(&["fetch".into()], &outputs, &json!({"seed":1})),
        json!({"fetched":42})
    );
    assert_eq!(
        assemble_input(&[], &outputs, &json!({"seed":1})),
        json!({"seed":1})
    );
}

#[test]
fn restart_keeps_pending_sibling_in_original_scope() {
    let g = WorkflowGraph {
        nodes: vec![
            node("start", "manual_trigger"),
            node("a", "delay"),
            node("b", "log"),
        ],
        edges: vec![edge("start", "a"), edge("start", "b")],
    };
    let run = mk_run(
        RunStatus::Running,
        vec![
            nstate("start", NodeStatus::Success),
            nstate("a", NodeStatus::Running),
            nstate("b", NodeStatus::Pending),
        ],
    );
    match classify_resume(&g, &run, None) {
        ResumeDecision::Resume { scope, .. } => assert_eq!(
            scope.start_node, None,
            "restart must preserve whole-graph scope, including pending sibling b"
        ),
        other => panic!("expected continuation, got {other:?}"),
    }
}

#[test]
fn interrupted_loop_requires_checkpoint_before_replay() {
    let mut loop_node = node("loop", "loop");
    loop_node.params =
        json!({"steps":[{"kind":"channel_notify","params":{"text":"once"}}, {"kind":"delay"}]});
    let graph = WorkflowGraph {
        nodes: vec![loop_node],
        edges: vec![],
    };
    let run = mk_run(
        RunStatus::Running,
        vec![nstate("loop", NodeStatus::Running)],
    );
    assert!(
        matches!(
            classify_resume(&graph, &run, None),
            ResumeDecision::Fail { .. }
        ),
        "a legacy loop has no checkpoint proving which external actions completed"
    );
}

#[test]
fn checkpointed_loop_resumes_safe_inner_work_and_preserves_siblings() {
    let loop_node = node("loop", "loop");
    let graph = WorkflowGraph {
        nodes: vec![loop_node.clone(), node("sibling", "log")],
        edges: vec![],
    };
    let mut run = mk_run(
        RunStatus::Running,
        vec![
            nstate("loop", NodeStatus::Running),
            nstate("sibling", NodeStatus::Pending),
        ],
    );
    let mut root = loop_checkpoint(&loop_node, "loop", 0, 0, json!({}));
    root.status = NodeStatus::Running;
    let mut sent = loop_checkpoint(&node("loop#1.0", "http_request"), "loop", 1, 0, json!({}));
    sent.status = NodeStatus::Success;
    let mut waiting = loop_checkpoint(&node("loop#1.1", "delay"), "loop", 1, 1, json!({}));
    waiting.status = NodeStatus::Running;
    run.checkpoints = vec![root, sent, waiting];
    match classify_resume(&graph, &run, None) {
        ResumeDecision::Resume { scope, nodes } => {
            assert!(scope.continue_unfinished);
            assert_eq!(scope.start_node, None);
            assert_eq!(nodes[1].status, NodeStatus::Pending);
        }
        other => panic!("expected safe continuation, got {other:?}"),
    }
    run.checkpoints[1].status = NodeStatus::Running;
    assert!(
        matches!(
            classify_resume(&graph, &run, None),
            ResumeDecision::Fail { .. }
        ),
        "unknown external outcome still prevents automatic replay"
    );
}

/// A restart that caught a SIDE-EFFECT step mid-flight must NOT replay it —
/// the step is marked unknown-outcome and the run fails.
#[test]
fn classify_resume_fails_on_interrupted_side_effect_step() {
    let g = WorkflowGraph {
        nodes: vec![
            node("a", "agent_prompt"),
            node("b", "git_pr"),
            node("c", "log"),
        ],
        edges: vec![edge("a", "b"), edge("b", "c")],
    };
    let run = mk_run(
        RunStatus::Running,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("b", NodeStatus::Running),
            nstate("c", NodeStatus::Pending),
        ],
    );
    match classify_resume(&g, &run, None) {
        ResumeDecision::Fail { nodes, error } => {
            assert!(error.contains("outcome is unknown"), "{error}");
            assert_eq!(nodes[1].status, NodeStatus::Error);
            assert_eq!(nodes[2].status, NodeStatus::Skipped);
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

/// A run paused at a human approval resumes AT the approval node — the
/// cheapest case: nothing but the wait is lost.
#[test]
fn classify_resume_reenters_at_waiting_approval_node() {
    let g = WorkflowGraph {
        nodes: vec![
            node("a", "log"),
            node("gate", "human_approval"),
            node("b", "channel_notify"),
        ],
        edges: vec![edge("a", "gate"), edge("gate", "b")],
    };
    let mut run = mk_run(
        RunStatus::Running,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("gate", NodeStatus::Running),
            nstate("b", NodeStatus::Pending),
        ],
    );
    run.waiting_approval = true;
    run.approval_node_id = Some("gate".into());
    match classify_resume(&g, &run, None) {
        ResumeDecision::Resume { scope, .. } => {
            assert_eq!(scope.start_node, None);
            assert!(scope.continue_unfinished);
        }
        other => panic!("expected Resume, got {other:?}"),
    }
}

/// A STALE approval flag (the run had moved past its gate) must not mask
/// a side-effect step caught mid-flight: that step's outcome is unknown,
/// so the run fails instead of replaying it as a "resumable approval".
#[test]
fn classify_resume_stale_approval_flag_never_masks_a_side_effect_step() {
    let g = WorkflowGraph {
        nodes: vec![
            node("gate", "human_approval"),
            node("pr", "git_pr"),
            node("c", "log"),
        ],
        edges: vec![edge("gate", "pr"), edge("pr", "c")],
    };
    let mut run = mk_run(
        RunStatus::Running,
        vec![
            nstate("gate", NodeStatus::Success),
            nstate("pr", NodeStatus::Running),
            nstate("c", NodeStatus::Pending),
        ],
    );
    run.waiting_approval = true;
    run.approval_node_id = Some("gate".into());
    match classify_resume(&g, &run, None) {
        ResumeDecision::Fail { nodes, error } => {
            assert!(error.contains("'pr'"), "{error}");
            assert_eq!(nodes[1].status, NodeStatus::Error);
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

/// S3-07: a live run row with no driver (its terminal write failed, its
/// driver panicked) is errored by the runtime sweep on its SECOND
/// driverless sighting — never on the first, and never while a driver
/// (even a queued one) holds it.
#[tokio::test]
async fn orphan_sweep_errors_driverless_runs_after_two_passes() {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "orphan-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('u','u','x','U',0,?)")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let wf = repo
        .create(
            &"orphan-ws".into(),
            "WF",
            "",
            "",
            &WorkflowGraph::default(),
            &"u".into(),
        )
        .await
        .unwrap();
    let orphan = repo
        .create_run(&wf.id, &"orphan-ws".into(), &json!({}), None)
        .await
        .unwrap();
    let driven = repo
        .create_run(&wf.id, &"orphan-ws".into(), &json!({}), None)
        .await
        .unwrap();
    let _driver = DriverGuard::register(&driven.id);
    let first = sweep_orphaned_runs(&ctx, &HashSet::new()).await;
    assert!(first.contains(&orphan.id) && !first.contains(&driven.id));
    assert!(!repo.is_canceled(&orphan.id).await);
    assert_eq!(
        repo.get_run(&orphan.id).await.unwrap().status,
        RunStatus::Pending
    );
    // S3-305: the run's provisioned dir (no worktrees left in it) is
    // reaped by the sweep, as a canceled run's would be.
    let run_dir = ctx.data_dir.join("workflow-runs").join(&orphan.id);
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(run_dir.join("run-brief.md"), "x").unwrap();
    sweep_orphaned_runs(&ctx, &first).await;
    assert!(!run_dir.exists(), "orphan's run dir is reaped");
    let swept = repo.get_run(&orphan.id).await.unwrap();
    assert_eq!(swept.status, RunStatus::Error);
    assert!(swept
        .error
        .unwrap_or_default()
        .contains("lost its engine driver"));
    assert_eq!(
        repo.get_run(&driven.id).await.unwrap().status,
        RunStatus::Pending
    );
    // The workflow is admissible again.
    assert!(
        repo.admit_run_if_idle(&wf.id, &"orphan-ws".into(), &json!({}), None)
            .await
            .unwrap()
            .is_none(),
        "the driven run still holds the slot"
    );
}

#[test]
fn settle_interrupted_nodes_leaves_nothing_running() {
    let nodes = settle_interrupted_nodes(vec![
        nstate("a", NodeStatus::Success),
        nstate("b", NodeStatus::Running),
        nstate("c", NodeStatus::Pending),
        nstate("d", NodeStatus::Error),
    ]);
    let st: Vec<NodeStatus> = nodes.iter().map(|n| n.status).collect();
    assert_eq!(
        st,
        vec![
            NodeStatus::Success,
            NodeStatus::Error,
            NodeStatus::Skipped,
            NodeStatus::Error
        ]
    );
    assert!(nodes[1].error.as_deref().unwrap().contains("interrupted"));
}

#[test]
fn skip_marker_key_is_per_run_and_step() {
    assert_eq!(skip_marker_key("r1", "n2"), "r1/n2");
    assert_ne!(skip_marker_key("r1", "n2"), skip_marker_key("r1", "n3"));
}

/// A re-queued retry-a-step (pending with progress) resumes its PERSISTED
/// scope verbatim; without one (pre-0108 row) it fails like before.
#[test]
fn classify_resume_honors_persisted_retry_scope() {
    let g = WorkflowGraph {
        nodes: vec![node("a", "log"), node("b", "agent_prompt")],
        edges: vec![edge("a", "b")],
    };
    let run = mk_run(
        RunStatus::Pending,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("b", NodeStatus::Error),
        ],
    );
    let scope = RunScope {
        continue_unfinished: false,
        start_node: Some("b".into()),
        only_node: true,
        adopt_start: false,
    };
    match classify_resume(&g, &run, Some(scope.clone())) {
        ResumeDecision::Resume { scope: got, .. } => assert_eq!(got, scope),
        other => panic!("expected Resume, got {other:?}"),
    }
    match classify_resume(&g, &run, None) {
        ResumeDecision::Fail { error, .. } => assert!(error.contains("retry scope was lost")),
        other => panic!("expected Fail, got {other:?}"),
    }
}

/// Died between the last node and the finalize write → just stamp the
/// terminal status the settled nodes imply.
#[test]
fn classify_resume_finishes_a_fully_settled_run() {
    let g = WorkflowGraph {
        nodes: vec![node("a", "log"), node("b", "log")],
        edges: vec![edge("a", "b")],
    };
    let run = mk_run(
        RunStatus::Running,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("b", NodeStatus::Success),
        ],
    );
    match classify_resume(&g, &run, None) {
        ResumeDecision::Finish { status, error, .. } => {
            assert_eq!(status, RunStatus::Success);
            assert!(error.is_none());
        }
        other => panic!("expected Finish, got {other:?}"),
    }
    // …and Error when a settled node failed.
    let run = mk_run(
        RunStatus::Running,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("b", NodeStatus::Error),
        ],
    );
    match classify_resume(&g, &run, None) {
        ResumeDecision::Finish { status, .. } => assert_eq!(status, RunStatus::Error),
        other => panic!("expected Finish, got {other:?}"),
    }
}

/// Died between node boundaries with work remaining → resume at the first
/// still-pending node in topo order.
#[test]
fn classify_resume_resumes_at_first_pending_between_boundaries() {
    let g = WorkflowGraph {
        nodes: vec![node("a", "log"), node("b", "log"), node("c", "log")],
        edges: vec![edge("a", "b"), edge("b", "c")],
    };
    let run = mk_run(
        RunStatus::Running,
        vec![
            nstate("a", NodeStatus::Success),
            nstate("b", NodeStatus::Pending),
            nstate("c", NodeStatus::Pending),
        ],
    );
    match classify_resume(&g, &run, None) {
        ResumeDecision::Resume { scope, .. } => {
            assert_eq!(scope.start_node, None);
            assert!(scope.continue_unfinished);
        }
        other => panic!("expected Resume, got {other:?}"),
    }
}

#[test]
fn side_effect_kinds_are_not_restart_resumable() {
    for k in [
        "git_pr",
        "channel_notify",
        "swarm_task",
        "product_publish",
        "api_run",
        "http_request",
        "self_improve",
        "product_rewrite",
        "product_plan",
    ] {
        assert!(!is_restart_resumable_kind(k), "{k} must not auto-replay");
    }
    for k in [
        "agent_prompt",
        "transform",
        "condition",
        "delay",
        "log",
        "human_approval",
        "review_run",
        "prepare_context",
    ] {
        assert!(is_restart_resumable_kind(k), "{k} should be restart-safe");
    }
}

#[test]
fn canvas_node_ext_matches_mode() {
    assert_eq!(canvas_node_ext("excalidraw"), "json");
    assert_eq!(canvas_node_ext("d2"), "d2");
    assert_eq!(canvas_node_ext("mermaid"), "mmd");
    assert_eq!(
        canvas_node_ext("sequence"),
        "mmd",
        "unrecognized modes default to mermaid"
    );
}

#[test]
fn descendants_scope_is_self_plus_downstream() {
    let g = WorkflowGraph {
        nodes: vec![
            node("a", "log"),
            node("b", "log"),
            node("c", "log"),
            node("d", "log"),
        ],
        edges: vec![edge("a", "b"), edge("b", "c"), edge("a", "d")],
    };
    let set = descendants_inclusive(&g, "b");
    assert!(set.contains("b") && set.contains("c"), "self + downstream");
    assert!(
        !set.contains("a") && !set.contains("d"),
        "not upstream/siblings"
    );
}

fn view(source: &str, errored: bool, has_output: bool, edge_active: bool) -> EdgeView {
    EdgeView {
        source: source.into(),
        errored,
        has_output,
        edge_active,
    }
}

#[test]
fn decide_entry_node_runs_with_no_sources() {
    assert_eq!(decide_node(&[]), NodeDecision::Run(vec![]));
}

#[test]
fn decide_errored_predecessor_poisons() {
    let v = vec![view("a", true, false, true)];
    assert_eq!(decide_node(&v), NodeDecision::ErrorSkip);
    // error wins even if a sibling succeeded
    let v = vec![view("a", true, false, true), view("b", false, true, true)];
    assert_eq!(decide_node(&v), NodeDecision::ErrorSkip);
}

#[test]
fn decide_active_branch_runs() {
    let v = vec![view("a", false, true, true)];
    assert_eq!(decide_node(&v), NodeDecision::Run(vec!["a".into()]));
}

#[test]
fn decide_inactive_only_branch_skips() {
    // condition pruned the only incoming edge
    let v = vec![view("a", false, true, false)];
    assert_eq!(decide_node(&v), NodeDecision::BranchSkip);
    // upstream was branch-skipped (no output, not errored)
    let v = vec![view("a", false, false, true)];
    assert_eq!(decide_node(&v), NodeDecision::BranchSkip);
}

#[test]
fn decide_join_runs_from_active_side_only() {
    // if/else join: a=true branch produced output (active), b=false branch pruned
    let v = vec![view("a", false, true, true), view("b", false, true, false)];
    assert_eq!(decide_node(&v), NodeDecision::Run(vec!["a".into()]));
    // and the other way
    let v = vec![view("a", false, true, false), view("b", false, true, true)];
    assert_eq!(decide_node(&v), NodeDecision::Run(vec!["b".into()]));
}

#[test]
fn eval_outgoing_prunes_false_edges() {
    let mut g = WorkflowGraph {
        nodes: vec![node("c", "condition"), node("t", "log"), node("f", "log")],
        edges: vec![
            WorkflowEdge {
                id: "c-t".into(),
                source: "c".into(),
                target: "t".into(),
                condition: Some("output.result == true".into()),
            },
            WorkflowEdge {
                id: "c-f".into(),
                source: "c".into(),
                target: "f".into(),
                condition: Some("output.result == false".into()),
            },
        ],
    };
    let cnode = g.nodes[0].clone();
    let out = json!({ "result": true });
    let (inactive, _logs) = eval_outgoing(&g, &cnode, &out, &Value::Null, &Value::Null);
    assert_eq!(inactive, vec!["c-f".to_string()], "false branch pruned");
    // flip
    let out = json!({ "result": false });
    let (inactive, _) = eval_outgoing(&g, &cnode, &out, &Value::Null, &Value::Null);
    assert_eq!(inactive, vec!["c-t".to_string()]);
    g.edges.clear();
    let (inactive, _) = eval_outgoing(&g, &cnode, &out, &Value::Null, &Value::Null);
    assert!(inactive.is_empty(), "no edges → nothing pruned");
}

#[test]
fn explicit_zero_disables_even_transient_retries() {
    let mut node = node("agent", "agent_prompt");
    node.retry = Some(otto_core::workflows::RetryPolicy::default());
    let policy = resolve_retry(&node);
    assert_eq!(policy.max_attempts, 0);
    assert!(retry_backoff("529 overloaded", &policy, 1, 0, 0).is_none());
}

#[test]
fn retry_policy_resolution_and_clamps() {
    let mut n = node("a", "agent_prompt");
    // R5: agent steps default to a small retry budget (2 retries = 3 attempts)
    // so a stuck no-op spawn is re-attempted with a fresh session.
    assert_eq!(
        resolve_retry(&n).max_attempts,
        2,
        "agent_prompt default 2 retries"
    );
    // Non-agent kinds keep the no-retry default.
    assert_eq!(
        resolve_retry(&node("b", "log")).max_attempts,
        0,
        "non-agent no retry"
    );
    n.params = json!({ "retry": { "max_attempts": 99, "backoff_ms": 999999 } });
    let p = resolve_retry(&n);
    assert_eq!(p.max_attempts, 5, "clamped to 5");
    assert_eq!(p.backoff_ms, 60_000, "clamped to 60s");
    assert!(is_retryable("agent_prompt"));
    assert!(!is_retryable("human_approval"));
    assert!(!is_retryable("manual_trigger"));
}

#[test]
fn prepare_context_gets_agent_retry_budget_only_with_a_prompt() {
    // No prompt (pure Jira-fetch step) → default no-retry, like any other kind.
    let no_prompt = node("p", "prepare_context");
    assert_eq!(
        resolve_retry(&no_prompt).max_attempts,
        0,
        "no agent phase → no retry"
    );
    // A blank/whitespace prompt doesn't count as an agent phase either.
    let mut blank_prompt = node("p", "prepare_context");
    blank_prompt.params = json!({ "prompt": "   " });
    assert_eq!(resolve_retry(&blank_prompt).max_attempts, 0);
    // Non-empty prompt → same retry budget as agent_prompt.
    let mut with_prompt = node("p", "prepare_context");
    with_prompt.params = json!({ "prompt": "analyze the ticket" });
    assert_eq!(
        resolve_retry(&with_prompt).max_attempts,
        2,
        "agent phase → agent_prompt budget"
    );
    assert!(is_retryable("prepare_context"));
}

#[test]
fn run_summary_has_status_steps_and_score() {
    let wf = Workflow {
        id: "w".into(),
        workspace_id: "ws".into(),
        name: "Write tests".into(),
        description: String::new(),
        instructions: String::new(),
        graph: WorkflowGraph {
            nodes: vec![node("a", "agent_prompt"), node("b", "review_run")],
            edges: vec![],
        },
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        version: 1,
        on_restart: "resume".into(),
    };
    let mk = |id: &str, status: NodeStatus, out: Value| NodeRunState {
        node_id: id.into(),
        status,
        output: Some(out),
        error: None,
        logs: vec![],
        started_at: None,
        duration_ms: Some(10),
        attempts: Some(1),
        sessions: vec![],
        review_ids: Vec::new(),
        activity: None,
    };
    let states = vec![
        mk(
            "a",
            NodeStatus::Success,
            json!({ "reply": "implemented the tests" }),
        ),
        mk(
            "b",
            NodeStatus::Success,
            json!({ "score": 92, "passed": true }),
        ),
    ];
    let (brief, full) = build_run_summary(&wf, &states, RunStatus::Success, Some("pack1"));
    assert!(brief.contains("Write tests"));
    assert!(brief.contains("2/2 steps ok"));
    assert!(brief.contains("92/100"), "brief shows the review score");
    assert!(brief.contains("summary.md"));
    assert!(full.contains("## Steps"));
    assert!(full.contains("review_run"));
    assert!(full.contains("pack1"), "full summary names the proof pack");
}

#[test]
fn skill_names_parse_skill_and_skills_deduped() {
    let p = json!({ "skill": "golang-feature-implementation",
                    "skills": ["correctness-review", " test-review ", "correctness-review"] });
    assert_eq!(
        node_skill_names(&p),
        vec![
            "golang-feature-implementation",
            "correctness-review",
            "test-review"
        ]
    );
    assert!(node_skill_names(&json!({})).is_empty());
    // `lenses` is also folded in (review nodes carry lenses).
    assert_eq!(
        node_skill_names(&json!({ "lenses": ["security-review"] })),
        vec!["security-review"]
    );
}

#[test]
fn param_str_list_accepts_array_or_csv() {
    assert_eq!(
        param_str_list(&json!({ "providers": ["claude", "codex"] }), "providers"),
        vec!["claude", "codex"]
    );
    assert_eq!(
        param_str_list(&json!({ "providers": "claude, codex ,  " }), "providers"),
        vec!["claude", "codex"]
    );
    assert!(param_str_list(&json!({}), "providers").is_empty());
}

#[test]
fn brief_summary_collapses_and_truncates() {
    // Short reply passes through, whitespace-collapsed.
    let s = brief_summary(&json!({ "reply": "Did   the\n\nthing." })).unwrap();
    assert_eq!(s, "Did the thing.");
    // Long text is cut to a sentence boundary with an ellipsis.
    let long = "First sentence. ".repeat(80);
    let out = brief_summary(&json!({ "reply": long })).unwrap();
    assert!(out.chars().count() <= 701);
    assert!(out.ends_with('…'));
    // Nothing to summarize → None.
    assert!(brief_summary(&json!({ "score": 5 })).is_none());
}

#[test]
fn reportable_skips_structural_and_review() {
    assert!(is_reportable("agent_prompt"));
    assert!(is_reportable("loop"));
    assert!(is_reportable("git_pr"));
    assert!(is_reportable("prepare_context"));
    // review_run self-reports; structural kinds stay quiet.
    assert!(!is_reportable("review_run"));
    assert!(!is_reportable("log"));
    assert!(!is_reportable("condition"));
    assert!(!is_reportable("manual_trigger"));
}

#[test]
fn chat_target_resolves_origin_and_override() {
    let wf = Workflow {
        id: "w".into(),
        workspace_id: "wf-ws".into(),
        name: "x".into(),
        description: String::new(),
        instructions: String::new(),
        graph: WorkflowGraph {
            nodes: vec![],
            edges: vec![],
        },
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        version: 1,
        on_restart: "resume".into(),
    };
    // Slack origin from a chat trigger.
    let t = resolve_chat_target(
        &wf,
        &json!({ "channel": "slack", "chat": "C123", "thread": "169.1",
                 "origin_workspace_id": "trigger-ws" }),
    )
    .expect("slack target");
    assert!(matches!(t.channel, Channel::Slack));
    assert_eq!(t.chat, "C123");
    assert_eq!(t.thread.as_deref(), Some("169.1"));
    assert_eq!(
        t.ws, "wf-ws",
        "S3-02: a foreign origin_workspace_id never borrows another workspace's bot"
    );
    // Explicit override wins.
    let t = resolve_chat_target(
        &wf,
        &json!({ "channel": "slack", "chat": "C1", "result_chat": "C2", "result_channel": "telegram" }),
    )
    .unwrap();
    assert!(matches!(t.channel, Channel::Telegram));
    assert_eq!(t.chat, "C2");
    assert_eq!(t.ws, "wf-ws", "no origin_workspace_id → workflow's own ws");
    // A manual UI run (no chat) → no target → disabled progress.
    assert!(resolve_chat_target(&wf, &json!({ "repo_id": "r" })).is_none());
    assert!(resolve_chat_target(&wf, &json!({ "channel": "webhook", "chat": "x" })).is_none());
}

// --- repo_id resolution (design §B) ------------------------------------

#[test]
fn match_repo_path_exact_subdir_and_sibling() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    let sub = repo.join("pkg/inner");
    let sibling = root.path().join("repo_wt"); // shares the "repo" name prefix
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let pairs = vec![("R".to_string(), repo.to_string_lossy().into_owned())];

    // Exact path → match.
    assert_eq!(
        match_repo_path(repo.to_string_lossy().as_ref(), &pairs).as_deref(),
        Some("R")
    );
    // A nested subdir → match (working dir inside the repo).
    assert_eq!(
        match_repo_path(sub.to_string_lossy().as_ref(), &pairs).as_deref(),
        Some("R")
    );
    // A sibling whose name shares a prefix must NOT match (component-wise).
    assert_eq!(
        match_repo_path(sibling.to_string_lossy().as_ref(), &pairs),
        None
    );
    // An unrelated ancestor must not match.
    assert_eq!(
        match_repo_path(root.path().to_string_lossy().as_ref(), &pairs),
        None
    );
}

#[test]
fn match_repo_path_deepest_repo_wins() {
    let root = tempfile::tempdir().unwrap();
    let outer = root.path().join("outer");
    let inner = outer.join("inner");
    let target = inner.join("x");
    std::fs::create_dir_all(&target).unwrap();
    let pairs = vec![
        ("OUTER".to_string(), outer.to_string_lossy().into_owned()),
        ("INNER".to_string(), inner.to_string_lossy().into_owned()),
    ];
    assert_eq!(
        match_repo_path(target.to_string_lossy().as_ref(), &pairs).as_deref(),
        Some("INNER")
    );
}

#[tokio::test]
async fn git_main_worktree_maps_linked_worktree_to_origin() {
    // Skip cleanly when git isn't available in the environment.
    let has_git = std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_git {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("origin");
    std::fs::create_dir_all(&repo).unwrap();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f.txt"), "hi").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "c"]);
    let wt = root.path().join("linked_wt");
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["worktree", "add", "-q"])
        .arg(&wt)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let main = git_main_worktree(wt.to_string_lossy().as_ref())
        .await
        .expect("worktree resolves to origin");
    let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap();
    assert_eq!(canon(std::path::Path::new(&main)), canon(&repo));
}

#[tokio::test]
async fn resolve_wf_base_fetches_a_branch_created_after_the_clone() {
    let has_git = std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_git {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let upstream = root.path().join("upstream");
    let clone = root.path().join("clone");
    std::fs::create_dir_all(&upstream).unwrap();
    let git = |dir: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&upstream, &["init", "-q", "-b", "main"]);
    git(&upstream, &["config", "user.email", "t@t"]);
    git(&upstream, &["config", "user.name", "t"]);
    std::fs::write(upstream.join("f.txt"), "hi").unwrap();
    git(&upstream, &["add", "-A"]);
    git(&upstream, &["commit", "-q", "-m", "c"]);
    git(
        root.path(),
        &[
            "clone",
            "-q",
            upstream.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    // The PR's target branch appears on the remote only after the clone.
    git(&upstream, &["branch", "hotfix/5.02.40-HF1"]);
    let want = git(&upstream, &["rev-parse", "hotfix/5.02.40-HF1"]);

    let local = otto_git::LocalGit::new(clone.to_str().unwrap());
    assert_eq!(
        resolve_wf_base(&local, "hotfix/5.02.40-HF1")
            .await
            .as_deref(),
        Some(want.as_str())
    );
    // Only the remote-tracking ref was written — no local branch appeared.
    assert!(git(&clone, &["branch", "--list", "hotfix/*"]).is_empty());
    // A base that exists nowhere still resolves to nothing.
    assert_eq!(resolve_wf_base(&local, "no/such-branch").await, None);
}

#[test]
fn fetchable_branch_name_rejects_refspec_injection() {
    assert!(fetchable_branch_name("hotfix/5.02.40-HF1"));
    assert!(fetchable_branch_name("develop"));
    for bad in [
        "",
        "-u",
        "--upload-pack=x",
        "a:b",
        "a b",
        "a..b",
        "*",
        "+x",
        "/x",
    ] {
        assert!(!fetchable_branch_name(bad), "{bad:?} must be rejected");
    }
}

// --- git_pr multi-target collection (design: PR opens one per changed repo) -

fn repo_ids(targets: &[Value]) -> Vec<String> {
    targets
        .iter()
        .filter_map(|t| t.get("repo_id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[test]
fn collect_pr_targets_single_review_reference() {
    // A direct review→git_pr: the review output carries one reference.
    let input = json!({ "repo_id": "R1", "base": "develop", "worktree": "/w/r1", "passed": true });
    let got = collect_pr_targets(&json!({}), &input);
    assert_eq!(repo_ids(&got), vec!["R1"]);
    assert_eq!(got[0].get("base").and_then(Value::as_str), Some("develop"));
}

#[test]
fn collect_pr_targets_fan_in_multiple_repos() {
    // Two review branches fanned into one git_pr (keyed by node id).
    let input = json!({
        "revA": { "repo_id": "A", "base": "main", "worktree": "/w/a" },
        "revB": { "repo_id": "B", "base": "release", "worktree": "/w/b" },
    });
    let mut ids = repo_ids(&collect_pr_targets(&json!({}), &input));
    ids.sort();
    assert_eq!(ids, vec!["A", "B"]);
}

#[test]
fn collect_pr_targets_loop_repos_and_explicit_params() {
    // A multi-repo loop publishes `repos[]`; explicit params.repos add more.
    let input = json!({ "repos": [{ "repo_id": "L1", "base": "dev", "worktree": "/w/l1" }] });
    let p = json!({ "repos": [{ "repo_id": "P1", "worktree": "/w/p1" }] });
    let mut ids = repo_ids(&collect_pr_targets(&p, &input));
    ids.sort();
    assert_eq!(ids, vec!["L1", "P1"]);
}

#[test]
fn collect_pr_targets_empty_when_only_working_directory() {
    // A plain run input with only a working_directory carries NO explicit
    // reference → caller resolves a single implicit target instead.
    let input = json!({ "working_directory": "/w/x", "goals": ["g"] });
    assert!(collect_pr_targets(&json!({}), &input).is_empty());
}

// --- repos declarations → run-input seeding ----------------------------------

fn entry(
    repo_id: &str,
    worktree: Option<&str>,
    base: Option<&str>,
    error: Option<&str>,
) -> crate::workflow_context::RepoEntry {
    crate::workflow_context::RepoEntry {
        repo: repo_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        repo_name: None,
        kind: "branch".into(),
        name: "feat/x".into(),
        source: base.map(str::to_string),
        worktree: worktree.map(str::to_string),
        base: base.map(str::to_string),
        error: error.map(str::to_string),
    }
}

#[test]
fn seed_input_fills_blanks_from_first_valid_entry() {
    let entries = vec![
        entry("BAD", None, None, Some("no repo")), // errored — skipped
        entry("R1", Some("/w/r1"), Some("develop"), None),
        entry("R2", Some("/w/r2"), Some("master"), None),
    ];
    let out = seed_input_from_entries(json!({ "msg": "do it" }), &entries);
    assert_eq!(
        out.get("working_directory").and_then(Value::as_str),
        Some("/w/r1")
    );
    assert_eq!(out.get("base").and_then(Value::as_str), Some("develop"));
    assert_eq!(out.get("repo_id").and_then(Value::as_str), Some("R1"));
    // Normalized targets exclude the errored entry and feed straight into
    // collect_pr_targets (git_pr's fan-out shape). The seeded top-level
    // repo_id ALSO matches as a single reference — dedup by repo_id is the
    // caller's job (git_pr's `seen` set), so assert the SET here.
    let targets = collect_pr_targets(&json!({}), &out);
    let ids: std::collections::BTreeSet<&str> = targets
        .iter()
        .filter_map(|t| t.get("repo_id").and_then(Value::as_str))
        .collect();
    assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec!["R1", "R2"]);
}

#[test]
fn seed_input_explicit_keys_win() {
    let entries = vec![entry("R1", Some("/w/r1"), Some("develop"), None)];
    let out = seed_input_from_entries(
        json!({ "working_directory": "/explicit", "base": "release", "repo_id": "X" }),
        &entries,
    );
    assert_eq!(
        out.get("working_directory").and_then(Value::as_str),
        Some("/explicit")
    );
    assert_eq!(out.get("base").and_then(Value::as_str), Some("release"));
    assert_eq!(out.get("repo_id").and_then(Value::as_str), Some("X"));
}

#[test]
fn seed_input_no_valid_entries_is_identity() {
    let entries = vec![entry("BAD", None, None, Some("no repo"))];
    let input = json!({ "msg": "hi" });
    assert_eq!(seed_input_from_entries(input.clone(), &entries), input);
    assert_eq!(seed_input_from_entries(Value::Null, &[]), Value::Null);
}

#[test]
fn normalize_prompt_fills_from_msg_only() {
    let v = normalize_prompt(json!({"msg": "hello"}));
    assert_eq!(v["prompt"], "hello");
    let v = normalize_prompt(json!({"prompt": "p", "msg": "m"}));
    assert_eq!(v["prompt"], "p"); // never overwritten
    let v = normalize_prompt(json!({"prompt": "  ", "msg": "m"}));
    assert_eq!(v["prompt"], "m"); // blank counts as absent
    let v = normalize_prompt(json!("scalar"));
    assert_eq!(v, json!("scalar")); // non-object untouched
}

// --- reviewer checks (commands delegated to the review agent) ---------------

#[test]
fn parse_checks_strings_and_objects() {
    let v = json!([
        "go test -tags=component ./...",
        { "name": "integration", "cmd": "go test -tags=integration ./..." },
        { "cmd": "" },     // dropped (empty)
        { "name": "x" },   // dropped (no cmd)
        "   ",             // dropped (blank)
    ]);
    let got = parse_checks(Some(&v));
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].1, "go test -tags=component ./...");
    assert_eq!(
        got[1],
        (
            "integration".to_string(),
            "go test -tags=integration ./...".to_string()
        )
    );
    assert!(parse_checks(None).is_empty());
    assert!(parse_checks(Some(&json!("not an array"))).is_empty());
}

#[test]
fn checks_review_agent_runs_and_flags_failures_as_bugs() {
    let checks = vec![
        (
            "component".to_string(),
            "go test -tags=component ./...".to_string(),
        ),
        (
            "integration".to_string(),
            "go test -tags=integration ./...".to_string(),
        ),
    ];
    let a = checks_review_agent("claude", &checks);
    assert_eq!(a.providers, vec!["claude"]);
    // Each command is named in the prompt, and failures are reported as bugs.
    assert!(a.prompt.contains("go test -tags=component ./..."));
    assert!(a.prompt.contains("go test -tags=integration ./..."));
    assert!(a.prompt.contains("\"severity\":\"bug\""));
    assert!(a.prompt.to_lowercase().contains("run each command"));
}

// --- R1 / R5: turn-oracle step plumbing -------------------------------------

#[test]
fn wf_step_rules_no_longer_forbid_subagents() {
    // R1.3: the block is a COMPLETION PROTOCOL now, not a prohibition —
    // orchestrator review mode (R2) delegates lenses to sub-agents.
    assert!(!WF_STEP_RULES.contains("Do NOT spawn"));
    assert!(WF_STEP_RULES.contains("Write your handoff file LAST"));
    assert!(WF_STEP_RULES.contains("You MAY delegate to sub-agents"));
}

#[test]
fn e2e_subagents_parses_count_and_hold() {
    assert_eq!(
        e2e_subagents("do the thing\nOTTO_E2E_SUBAGENTS: 2 hold_ms=5000\nthanks"),
        Some((2, Duration::from_millis(5000)))
    );
    // No hold token → the env default (4000 ms unless OTTO_E2E_STEP_HOLD_MS).
    let (n, hold) = e2e_subagents("OTTO_E2E_SUBAGENTS: 3").expect("parsed");
    assert_eq!(n, 3);
    assert!(hold >= Duration::from_millis(1000));
    // Out-of-range counts and a missing sentinel are ignored entirely.
    assert_eq!(e2e_subagents("OTTO_E2E_SUBAGENTS: 0"), None);
    assert_eq!(e2e_subagents("OTTO_E2E_SUBAGENTS: 41"), None);
    assert_eq!(e2e_subagents("OTTO_E2E_SUBAGENTS: many"), None);
    assert_eq!(e2e_subagents("a normal step prompt"), None);
    // An out-of-range hold falls back to the default rather than being used.
    let (_, hold) = e2e_subagents("OTTO_E2E_SUBAGENTS: 1 hold_ms=99").expect("parsed");
    assert!(hold >= Duration::from_millis(1000));
}

#[test]
fn keep_session_param_defaults_false() {
    assert!(stop_step_sessions_wanted("agent_prompt", &json!({})));
    assert!(stop_step_sessions_wanted(
        "agent_prompt",
        &json!({ "keep_session": false })
    ));
    assert!(!stop_step_sessions_wanted(
        "agent_prompt",
        &json!({ "keep_session": true })
    ));
    // A non-bool value is not an opt-out.
    assert!(stop_step_sessions_wanted(
        "agent_prompt",
        &json!({ "keep_session": "yes" })
    ));
}

#[test]
fn review_association_survives_completed_async_steps_and_retry_sessions() {
    let mut state = nstate("review", NodeStatus::Running);
    assert!(record_association(
        &mut state,
        AgentAssociation::Review("review-id".into())
    ));
    assert!(record_association(
        &mut state,
        AgentAssociation::Session("first".into())
    ));
    assert!(!record_association(
        &mut state,
        AgentAssociation::Review("review-id".into())
    ));
    state.status = NodeStatus::Success; // await:false has already returned.
    let mut restored: NodeRunState =
        serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
    assert_eq!(restored.review_ids, ["review-id"]);
    record_association(&mut restored, AgentAssociation::Session("retry".into()));
    assert_eq!(restored.sessions, ["first", "retry"]);
}

#[test]
fn success_suspends_step_session_unless_keep_session() {
    // A resumable provider is SUSPENDED (the row stays Reconnectable, so
    // "Open session" on the finished step still resumes the transcript);
    // one that cannot resume is killed.
    assert_eq!(stop_action(true), "suspend");
    assert_eq!(stop_action(false), "kill");
    // …and the review_run arm is exempt (its sessions belong to the review).
    assert!(!stop_step_sessions_wanted("review_run", &json!({})));
    assert!(stop_step_sessions_wanted("prepare_context", &json!({})));
}

/// SD-01: 1 000 streamed lines (one per ms) on a 500-node run used to be
/// 1 000 whole-run writes. Coalesced, they are ≤ one write per 20 lines /
/// 250 ms, a lone line after a quiet spell is still written at once, and
/// the trailing flush delivers the last lines.
#[test]
fn log_persist_coalesces_bursts_and_flushes_the_tail() {
    let t0 = Instant::now();
    let mut lp = LogPersist::default();
    let mut writes = 0usize;
    let mut buffered = 0usize;
    let mut delivered = 0usize;
    for i in 0..1000u64 {
        let now = t0 + Duration::from_millis(i);
        // A due trailing flush fires before the next line is taken.
        if lp.flush_at.is_some_and(|at| at <= now) {
            lp.wrote_log(now);
            writes += 1;
            delivered += std::mem::take(&mut buffered);
        }
        buffered += 1;
        if lp.on_line(now) {
            writes += 1;
            delivered += std::mem::take(&mut buffered);
        }
    }
    if let Some(at) = lp.flush_at {
        lp.wrote_log(at);
        writes += 1;
        delivered += std::mem::take(&mut buffered);
    }
    assert_eq!(delivered, 1000, "every line is written");
    assert!(writes <= 1000 / LOG_PERSIST_LINES + 1, "{writes} writes");
    assert!(lp.flush_at.is_none() && lp.unpersisted == 0);
    // Quiet spell → the next line is written immediately.
    let later = t0 + Duration::from_secs(10);
    assert!(lp.on_line(later));
    // A second line right after waits for the trailing flush…
    assert!(!lp.on_line(later + Duration::from_millis(1)));
    assert_eq!(lp.flush_at, Some(later + LOG_PERSIST_EVERY));
    // …unless another write of the run carried it.
    lp.carried();
    assert!(lp.flush_at.is_none() && lp.unpersisted == 0);
    // The live cap evicts only phase lines, oldest first, like the end cap.
    let mut logs: Vec<String> = (0..NODE_LIVE_LOG_CAP + 50)
        .map(|i| {
            if i % 2 == 0 {
                format!("⏳ phase {i}")
            } else {
                format!("▶ line {i}")
            }
        })
        .collect();
    cap_node_logs(&mut logs, NODE_LIVE_LOG_CAP);
    assert_eq!(logs.len(), NODE_LIVE_LOG_CAP);
    assert!(logs.iter().filter(|l| l.starts_with('▶')).count() == (NODE_LIVE_LOG_CAP + 50) / 2);
    // Capping while running changes nothing about the node-end result.
    let lines: Vec<String> = (0..3000u32)
        .map(|i| match i.wrapping_mul(2_654_435_761) % 7 {
            0 => format!("✓ decision {i}"),
            1 | 2 => format!("🧩 sub-agents {i}"),
            _ => format!("⏳ working {i}"),
        })
        .collect();
    let mut batch = lines.clone();
    cap_node_logs(&mut batch, NODE_LOG_CAP);
    let mut live = Vec::new();
    for l in &lines {
        live.push(l.clone());
        if live.len() > NODE_LIVE_LOG_CAP {
            cap_node_logs(&mut live, NODE_LIVE_LOG_CAP);
        }
    }
    cap_node_logs(&mut live, NODE_LOG_CAP);
    assert_eq!(live, batch);
}

#[test]
fn success_path_keeps_phase_lines_in_order_and_caps_at_200() {
    // The success path's assembled vector: the live phase lines (kept now)
    // followed by the returned/persist/edge lines.
    let mut logs: Vec<String> = vec![
        "▶ agent_prompt started".into(),
        "⏳ starting claude session".into(),
    ];
    for i in 0..300 {
        logs.push(format!("🧩 sub-agents: {i} running · 0 done"));
    }
    logs.push("⏸ agent idle — confirming completion (20s)".into());
    logs.push("📄 handoff file written".into());
    logs.push("✓ step complete (handoff + idle turn)".into());
    logs.push("⚠ handoff written but 3 tasks still pending — waiting (up to 15m)".into());
    logs.push("↻ retry 2/5 in 23s (provider overloaded: 529)".into());
    logs.push("agent turn complete".into());
    logs.push("edge → n3 not taken (output.score < 80)".into());
    let before = logs.len();
    cap_node_logs(&mut logs, NODE_LOG_CAP);
    assert!(before > NODE_LOG_CAP);
    assert_eq!(logs.len(), NODE_LOG_CAP);
    // Order preserved, and NOTHING that records a decision was evicted.
    assert_eq!(logs[0], "▶ agent_prompt started");
    for keep in [
        "✓ step complete (handoff + idle turn)",
        "⚠ handoff written but 3 tasks still pending — waiting (up to 15m)",
        "↻ retry 2/5 in 23s (provider overloaded: 529)",
        "agent turn complete",
        "edge → n3 not taken (output.score < 80)",
    ] {
        assert!(logs.iter().any(|l| l == keep), "evicted: {keep}");
    }
    // The oldest phase lines went first.
    assert!(!logs
        .iter()
        .any(|l| l == "🧩 sub-agents: 0 running · 0 done"));
    assert!(logs
        .iter()
        .any(|l| l == "🧩 sub-agents: 299 running · 0 done"));
}

#[test]
fn activity_snapshots_are_rate_limited_and_bounded() {
    // 41 sub-agents with oversized ids/descriptions → 40 rows, clamped.
    let subs: Vec<SubagentActivity> = (0..41)
        .map(|i| SubagentActivity {
            id: format!("{i}-{}", "i".repeat(200)),
            description: "d".repeat(200),
            status: "running".into(),
            started_at: Some(chrono::Utc::now()),
            finished_at: None,
        })
        .collect();
    let a = bound_activity(NodeActivity {
        phase: "sub-agents: 41 running · 0 done".into(),
        updated_at: chrono::Utc::now(),
        last_progress_at: Some(chrono::Utc::now()),
        pending_tasks: 41,
        subagents: subs,
        hold_reason: None,
    });
    assert_eq!(a.subagents.len(), 40);
    assert!(a.subagents.iter().all(|s| s.id.chars().count() <= 64));
    assert!(a
        .subagents
        .iter()
        .all(|s| s.description.chars().count() <= 80));
    // The snapshot's OWN contribution is what this batch adds to the row —
    // a 40-agent sweep costs under 12 KiB, so `activity` can never be the
    // thing that blows the frame.
    let abytes = serde_json::to_string(&a).unwrap().len();
    assert!(abytes < 12 * 1024, "activity was {abytes} bytes");
    // And a real node row — 200 kept phase lines at their documented texts
    // plus the 40-agent snapshot — stays under NODE_EVENT_MAX_BYTES, so the
    // WS frame is never dropped. (A node whose LOGS alone exceed the cap
    // still falls back to the UI's rev-guarded refetch, as it did before.)
    let state = NodeRunState {
        node_id: "n2".into(),
        status: NodeStatus::Running,
        output: None,
        error: None,
        logs: (0..200)
            .map(|i| format!("🧩 sub-agents: {i} running · 0 done (Sweep diff chunk aa ✓)"))
            .collect(),
        started_at: Some(chrono::Utc::now()),
        duration_ms: None,
        attempts: Some(1),
        sessions: vec!["01ABCDEF".into()],
        review_ids: Vec::new(),
        activity: Some(a),
    };
    let n = serde_json::to_string(&state).unwrap().len();
    assert!(n < NODE_EVENT_MAX_BYTES, "node row was {n} bytes");
}

#[test]
fn phase_text_strips_the_glyph() {
    assert_eq!(
        phase_text("🧩 sub-agents: 2 running · 1 done"),
        "sub-agents: 2 running · 1 done"
    );
    assert_eq!(
        phase_text("⏸ agent idle — confirming completion (20s)"),
        "agent idle — confirming completion (20s)"
    );
    assert_eq!(
        phase_text("📄 handoff file written"),
        "handoff file written"
    );
}

#[test]
fn completion_lines_are_the_documented_texts() {
    let feed = PhaseFeed::new("claude");
    assert_eq!(
        feed.completion_line(CompleteVia::HandoffAndIdleTurn),
        "✓ step complete (handoff + idle turn)"
    );
    assert_eq!(
        feed.completion_line(CompleteVia::CodexTaskComplete),
        "✓ step complete (codex task_complete + handoff)"
    );
    assert_eq!(
        feed.completion_line(CompleteVia::IdleTurnNoHandoff),
        "⚠ handoff file missing — accepted the agent's final reply after 90s idle"
    );
    assert_eq!(
        feed.completion_line(CompleteVia::QuietFallback),
        "⚠ legacy completion record: silence fallback (no longer accepted)"
    );
    // Counted lines take the last phase's pending count (singular/plural).
    let mut feed = PhaseFeed::new("claude");
    feed.pending = 1;
    assert_eq!(
        feed.completion_line(CompleteVia::HandoffLingerCap),
        "⚠ handoff written; 1 sub-agent never reported after 15m — moving on"
    );
    feed.pending = 3;
    assert_eq!(
        feed.completion_line(CompleteVia::HandoffLingerCap),
        "⚠ handoff written; 3 sub-agents never reported after 15m — moving on"
    );
    feed.running_id = Some("b5gvqf675".into());
    assert_eq!(
        feed.completion_line(CompleteVia::BashLingerCap),
        "⚠ background task b5gvqf675 never finished after 15m — moving on"
    );
}

// --- R2: review_run execution-mode resolution -------------------------------

#[test]
fn review_run_mode_precedence_run_input_over_node_param() {
    use otto_core::domain::ReviewMode;
    // The RUN's override beats a node that sets its own mode.
    assert_eq!(
        resolve_review_mode_source(
            &json!({ "review_mode": "fan_out" }),
            &json!({ "mode": "orchestrator" })
        ),
        Some((ReviewMode::FanOut, "run override"))
    );
    // No run override → the node's param.
    assert_eq!(
        resolve_review_mode_source(&json!({}), &json!({ "mode": "orchestrator" })),
        Some((ReviewMode::Orchestrator, "node"))
    );
    // Neither → the caller falls back to the stored config / default.
    assert_eq!(resolve_review_mode_source(&json!({}), &json!({})), None);
    // Garbage is treated as absent, never an error.
    assert_eq!(
        resolve_review_mode_source(&json!({ "review_mode": "nope" }), &json!({})),
        None
    );
    assert_eq!(
        resolve_review_mode_source(&json!({}), &json!({ "mode": 7 })),
        None
    );
    // …and garbage in the run input still lets the node's value win.
    assert_eq!(
        resolve_review_mode_source(
            &json!({ "review_mode": "nope" }),
            &json!({ "mode": "fan_out" })
        ),
        Some((ReviewMode::FanOut, "node"))
    );
}

#[test]
fn review_run_reads_mode_from_run_env_not_node_input() {
    // The resolver takes the RUN input (`RunEnv.run_input`) and the node's
    // params — it has no node-input parameter at all, so a `review_mode`
    // riding a node input (which loses run-level keys hop by hop through
    // `assemble_input`) can never be picked up.
    let node_input = json!({ "review_mode": "orchestrator", "repo_id": "r1" });
    assert_eq!(resolve_review_mode_source(&json!({}), &json!({})), None);
    // Passing it as the run input DOES resolve — proving the difference is
    // the argument, not the value.
    assert_eq!(
        resolve_review_mode_source(&node_input, &json!({})),
        Some((otto_core::domain::ReviewMode::Orchestrator, "run override"))
    );
}
