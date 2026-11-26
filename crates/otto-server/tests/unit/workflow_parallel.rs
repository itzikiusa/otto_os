//! The scheduler driver on real nodes (engine.md §3.6, test plan §14.2):
//! nodes run side by side, a failing sibling does not stop the other, and a
//! cancel stops every running node at once.

use super::super::super::*;
use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
use otto_core::workflows::WorkflowEdge;

fn node(id: &str, kind: &str, params: Value) -> WorkflowNode {
    WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: String::new(),
        x: 0.0,
        y: 0.0,
        params,
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

/// Create the workflow and a run of it; returns what `run_workflow` takes.
async fn setup(
    ws_id: &str,
    graph: WorkflowGraph,
) -> (
    ServerCtx,
    Workspace,
    Workflow,
    Id,
    WorkflowsRepo,
    tempfile::TempDir,
) {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, ws_id).await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let ws = ctx.workspaces.get(&ws_id.into()).await.unwrap();
    let repo = WorkflowsRepo::new(pool);
    let wf = repo
        .create(&ws.id, "Parallel", "", "", &graph, &"root".into())
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&"root".into()))
        .await
        .unwrap();
    (ctx, ws, wf, run.id, repo, tmp)
}

fn state<'a>(run: &'a WorkflowRun, id: &str) -> &'a NodeRunState {
    run.nodes
        .iter()
        .find(|n| n.node_id == id)
        .unwrap_or_else(|| panic!("no {id}: {:?} {:?}", run.status, run.error))
}

/// A sibling failing while another runs: the other runs to its end, the
/// failed one's dependents error-skip, the run ends `error` only then, and
/// the step numbers are the serial engine's.
#[tokio::test]
async fn a_failing_sibling_lets_the_other_finish_and_poisons_only_its_own_branch() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("slow", "delay", json!({"ms": 400})),
            node("bad", "budget_gate", json!({})),
            node("after_slow", "log", json!({})),
            node("after_bad", "log", json!({})),
        ],
        edges: vec![
            edge("start", "slow"),
            edge("start", "bad"),
            edge("slow", "after_slow"),
            edge("bad", "after_bad"),
        ],
    };
    let (ctx, ws, wf, run_id, repo, _tmp) = setup("par-ws", graph).await;
    // An unreadable budget config fails the gate closed.
    otto_state::SettingsRepo::new(ctx.pool.clone())
        .put(
            crate::budget::BUDGETS_KEY,
            &json!({"enforce": "yes please"}),
        )
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(20),
        run_workflow(
            ctx,
            ws,
            wf,
            run_id.clone(),
            json!({}),
            RunScope::default(),
            None,
        ),
    )
    .await
    .expect("the run settles");
    let run = repo.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Error);
    let (slow, bad) = (state(&run, "slow"), state(&run, "bad"));
    assert_eq!(
        slow.status,
        NodeStatus::Success,
        "the sibling ran to its end"
    );
    assert_eq!(bad.status, NodeStatus::Error);
    assert_eq!(state(&run, "after_slow").status, NodeStatus::Success);
    assert_eq!(state(&run, "after_bad").status, NodeStatus::Skipped);
    // Run order start, slow, bad, after_slow, after_bad; the skipped one has no number.
    let numbers: Vec<_> = ["start", "slow", "bad", "after_slow", "after_bad"]
        .iter()
        .map(|id| state(&run, id).step_no)
        .collect();
    assert_eq!(numbers, [Some(1), Some(2), Some(3), Some(4), None]);
    // The failure was decided while the slow sibling still ran.
    assert!(
        bad.started_at.unwrap() < slow.started_at.unwrap() + chrono::Duration::milliseconds(400),
        "bad started only after slow finished"
    );
}

/// Three nodes running at once all stop at a cancel; nothing settles as a
/// success afterwards and the run is `canceled`.
#[tokio::test]
async fn a_cancel_stops_every_running_node_at_once() {
    let delay = || json!({"ms": 8000});
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("a", "delay", delay()),
            node("b", "delay", delay()),
            node("c", "delay", delay()),
        ],
        edges: vec![edge("start", "a"), edge("start", "b"), edge("start", "c")],
    };
    let (ctx, ws, wf, run_id, repo, _tmp) = setup("cancel-ws", graph).await;
    let driver = tokio::spawn(run_workflow(
        ctx,
        ws,
        wf,
        run_id.clone(),
        json!({}),
        RunScope::default(),
        None,
    ));
    // Wait until all three run, then cancel.
    let started = Instant::now();
    loop {
        let run = repo.get_run(&run_id).await.unwrap();
        let running = run
            .nodes
            .iter()
            .filter(|n| n.node_id != "start" && n.status == NodeStatus::Running)
            .count();
        if running == 3 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "three nodes run at once"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    repo.request_cancel(&run_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .expect("the cancel stops the run long before the delays end")
        .unwrap();
    let run = repo.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Canceled);
    for id in ["a", "b", "c"] {
        assert_ne!(state(&run, id).status, NodeStatus::Success, "{id}");
    }
}
