//! Workflow rows across builds (test-plan X3, X4). The frozen 0.6 serde types
//! in `fixtures/workflow_types_0_6.rs` stand for the old binary: rows they
//! wrote must decode into today's types (an upgrade with runs on disk), and
//! rows today's repo writes must decode into them (a rollback to the old
//! binary on the new schema). Values are compared as JSON: every field the
//! old side holds must arrive unchanged; a field the new side adds is fine.

#[path = "fixtures/workflow_types_0_6.rs"]
mod old;

use chrono::{TimeZone, Utc};
use otto_core::workflows::{NodeRunState, NodeStatus, RunScope, WorkflowCheckpoint, WorkflowGraph};
use otto_state::{DbPool, WorkflowsRepo};
use serde::Serialize;
use serde_json::{json, Value};

async fn fixture() -> (tempfile::TempDir, DbPool) {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u', 'u', 'hash', 'User', 1, '2026-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w', 'Workspace', '/tmp', '2026-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    (dir, pool)
}

/// `old` ⊆ `new`: every key of `old` is in `new` with an equal value.
fn assert_kept(old: &Value, new: &Value, at: &str) {
    match (old, new) {
        (Value::Object(o), Value::Object(n)) => {
            for (k, v) in o {
                let got = n.get(k).unwrap_or_else(|| panic!("{at}.{k} dropped"));
                assert_kept(v, got, &format!("{at}.{k}"));
            }
        }
        (Value::Array(o), Value::Array(n)) => {
            assert_eq!(o.len(), n.len(), "{at}: length");
            for (i, (a, b)) in o.iter().zip(n).enumerate() {
                assert_kept(a, b, &format!("{at}[{i}]"));
            }
        }
        _ => assert_eq!(old, new, "{at}"),
    }
}

fn v(x: &impl Serialize) -> Value {
    serde_json::to_value(x).unwrap()
}

fn t(s: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, s).unwrap()
}

/// A graph using every field the type has (retry, edge condition, positions).
fn old_graph() -> old::WorkflowGraph {
    old::WorkflowGraph {
        nodes: vec![
            old::WorkflowNode {
                id: "n1".into(),
                kind: "manual_trigger".into(),
                name: "Start".into(),
                x: 40.0,
                y: 30.0,
                params: json!({}),
                retry: None,
            },
            old::WorkflowNode {
                id: "n2".into(),
                kind: "agent_prompt".into(),
                name: "Agent step 2".into(),
                x: 300.0,
                y: 30.0,
                params: json!({"provider": "claude", "skills": ["okf-authoring"], "prompt": "do it"}),
                retry: Some(old::RetryPolicy {
                    max_attempts: 2,
                    backoff_ms: 2000,
                    factor: 2.0,
                }),
            },
            old::WorkflowNode {
                id: "n3".into(),
                kind: "condition".into(),
                name: String::new(),
                x: 0.0,
                y: 0.0,
                params: json!({"expr": "passed == true"}),
                retry: None,
            },
        ],
        edges: vec![
            old::WorkflowEdge {
                id: "e1".into(),
                source: "n1".into(),
                target: "n2".into(),
                condition: None,
            },
            old::WorkflowEdge {
                id: "e2".into(),
                source: "n2".into(),
                target: "n3".into(),
                condition: Some("output.result == true".into()),
            },
        ],
    }
}

/// One node in every status, the running one with a full activity snapshot.
fn old_nodes() -> Vec<old::NodeRunState> {
    let bare = |id: &str, status| old::NodeRunState {
        node_id: id.into(),
        status,
        output: None,
        error: None,
        logs: vec![],
        started_at: None,
        duration_ms: None,
        attempts: None,
        sessions: vec![],
        review_ids: vec![],
        activity: None,
    };
    vec![
        old::NodeRunState {
            output: Some(json!({"reply": "ok", "session_id": "s1", "working_directory": "/w"})),
            logs: vec!["agent turn complete".into()],
            started_at: Some(t(1)),
            duration_ms: Some(1500),
            attempts: Some(2),
            sessions: vec!["s0".into(), "s1".into()],
            review_ids: vec!["r1".into()],
            ..bare("n1", old::NodeStatus::Success)
        },
        old::NodeRunState {
            started_at: Some(t(2)),
            sessions: vec!["s2".into()],
            activity: Some(old::NodeActivity {
                phase: "waiting on 1 sub-agent".into(),
                updated_at: t(3),
                last_progress_at: Some(t(3)),
                pending_tasks: 1,
                subagents: vec![old::SubagentActivity {
                    id: "a1".into(),
                    description: "scan".into(),
                    status: "running".into(),
                    started_at: Some(t(2)),
                    finished_at: None,
                }],
                hold_reason: Some("sub-agents pending".into()),
            }),
            ..bare("n2", old::NodeStatus::Running)
        },
        old::NodeRunState {
            error: Some("boom".into()),
            ..bare("n3", old::NodeStatus::Error)
        },
        bare("n4", old::NodeStatus::Skipped),
        bare("n5", old::NodeStatus::Pending),
    ]
}

fn old_checkpoint() -> old::WorkflowCheckpoint {
    old::WorkflowCheckpoint {
        node_id: "n2#0.1".into(),
        loop_id: "n2".into(),
        iteration: 1,
        step_index: 0,
        kind: "agent_prompt".into(),
        name: "fix".into(),
        status: old::NodeStatus::Success,
        attempts: 1,
        input: json!({"x": 1}),
        output: Some(json!({"reply": "done"})),
        error: None,
        logs: vec!["agent turn complete".into()],
        updated_at: t(4),
    }
}

/// proves: X3 — rows written by the 0.6 types (definition, pinned version,
/// run nodes in every status, input, resume scope, loop checkpoint) decode
/// into today's types with every field kept, through the repo readers the run
/// view, retry and the restart reconciler use.
#[tokio::test]
async fn old_rows_decode() {
    let (_dir, pool) = fixture().await;
    let graph = serde_json::to_string(&old_graph()).unwrap();
    let nodes = serde_json::to_string(&old_nodes()).unwrap();
    let scope = old::RunScope {
        continue_unfinished: true,
        start_node: Some("n2".into()),
        only_node: true,
        adopt_start: false,
    };
    let input = json!({"trigger": "chat", "prompt": "go", "goals": ["a"], "pr": "7"});
    sqlx::query("INSERT INTO workflows (id, workspace_id, name, description, graph_json, created_by, created_at, updated_at, version, on_restart, instructions) VALUES ('wf', 'w', 'Old', 'd', ?, 'u', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 3, 'resume', 'rules')")
        .bind(&graph).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workflow_versions (id, workflow_id, version, name, description, graph_json, created_by, created_at, instructions, on_restart) VALUES ('v2', 'wf', 2, 'Old', 'd', ?, 'u', '2026-01-01T00:00:00Z', 'rules', 'fail')")
        .bind(&graph).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workflow_runs (id, workflow_id, workspace_id, status, input_json, nodes_json, started_at, workflow_version, resume_scope_json, resume_attempts) VALUES ('run', 'wf', 'w', 'running', ?, ?, '2026-01-01T00:00:00Z', 2, ?, 1)")
        .bind(input.to_string()).bind(&nodes).bind(serde_json::to_string(&scope).unwrap())
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workflow_checkpoints (run_id, node_id, checkpoint_json) VALUES ('run', 'n2#0.1', ?)")
        .bind(serde_json::to_string(&old_checkpoint()).unwrap()).execute(&pool).await.unwrap();

    let repo = WorkflowsRepo::new(pool);
    let wf = repo.get(&"wf".into()).await.unwrap();
    assert_kept(&v(&old_graph()), &v(&wf.graph), "graph");
    assert_eq!(
        (wf.version, wf.on_restart.as_str(), wf.instructions.as_str()),
        (3, "resume", "rules")
    );
    let run = repo.get_run(&"run".into()).await.unwrap();
    assert_kept(&v(&old_nodes()), &v(&run.nodes), "nodes");
    assert_eq!(run.input, input);
    assert_eq!((run.workflow_version, run.resume_attempts), (Some(2), 1));
    let pinned = repo.definition_for_run(&run).await.unwrap();
    assert_kept(&v(&old_graph()), &v(&pinned.graph), "pinned graph");
    assert_eq!(
        pinned.on_restart, "fail",
        "the run keeps its pinned version's policy"
    );
    assert_kept(
        &v(&scope),
        &v(&repo.run_scope(&"run".into()).await.unwrap()),
        "scope",
    );
    let cps = repo.checkpoints(&"run".into()).await.unwrap();
    assert_kept(&v(&vec![old_checkpoint()]), &v(&cps), "checkpoints");
    let interrupted = repo.interrupted_runs().await.unwrap();
    assert_eq!(interrupted.len(), 1);
    assert_kept(
        &v(&old_nodes()),
        &v(&interrupted[0].0.nodes),
        "interrupted nodes",
    );
}

/// Part of X4 (the decode half of a rollback): what today's repo writes (definition, progress,
/// resume scope, checkpoint) decodes into the 0.6 types, so a rolled-back
/// binary still reads the rows.
#[tokio::test]
async fn new_rows_decode_in_old_types() {
    let (_dir, pool) = fixture().await;
    let repo = WorkflowsRepo::new(pool.clone());
    let graph: WorkflowGraph = serde_json::from_value(v(&old_graph())).unwrap();
    let wf = repo
        .create(&"w".into(), "New", "d", "rules", &graph, &"u".into())
        .await
        .unwrap();
    let run = repo
        .create_run(
            &wf.id,
            &wf.workspace_id,
            &json!({"prompt": "go"}),
            Some(&"u".into()),
        )
        .await
        .unwrap();
    let nodes: Vec<NodeRunState> = serde_json::from_value(v(&old_nodes())).unwrap();
    repo.update_run_progress(&run.id, &nodes).await.unwrap();
    let scope = RunScope {
        continue_unfinished: true,
        start_node: Some("n2".into()),
        only_node: false,
        adopt_start: true,
    };
    repo.set_run_resume_scope(&run.id, Some(&serde_json::to_string(&scope).unwrap()))
        .await
        .unwrap();
    let cp: WorkflowCheckpoint = serde_json::from_value(v(&old_checkpoint())).unwrap();
    assert_eq!(cp.status, NodeStatus::Success);
    repo.save_checkpoint(&run.id, &cp).await.unwrap();

    let (graph_json,): (String,) = sqlx::query_as("SELECT graph_json FROM workflows WHERE id = ?")
        .bind(&wf.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let (nodes_json, scope_json): (String, String) =
        sqlx::query_as("SELECT nodes_json, resume_scope_json FROM workflow_runs WHERE id = ?")
            .bind(&run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (cp_json,): (String,) =
        sqlx::query_as("SELECT checkpoint_json FROM workflow_checkpoints WHERE run_id = ?")
            .bind(&run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let old_g: old::WorkflowGraph = serde_json::from_str(&graph_json).unwrap();
    let old_n: Vec<old::NodeRunState> = serde_json::from_str(&nodes_json).unwrap();
    let old_s: old::RunScope = serde_json::from_str(&scope_json).unwrap();
    let old_c: old::WorkflowCheckpoint = serde_json::from_str(&cp_json).unwrap();
    assert_kept(&v(&old_g), &v(&graph), "graph");
    assert_kept(&v(&old_n), &v(&nodes), "nodes");
    assert_kept(&v(&old_s), &v(&scope), "scope");
    assert_kept(&v(&old_c), &v(&cp), "checkpoint");
}
