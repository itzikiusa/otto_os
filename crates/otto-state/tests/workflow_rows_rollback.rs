//! Rollback with the run-view fields (test-plan X4, DR-014): rows the new
//! build writes — nodes with every new field, a loop checkpoint with banked
//! active time, a workflow with an origin, a held run with limits and an
//! overview — still decode into the frozen 0.6 types and keep the 0.6
//! statuses, so a rolled-back binary reads, lists and cancels them.

// Only the node and checkpoint types are needed here.
#[allow(dead_code)]
#[path = "fixtures/workflow_types_0_6.rs"]
mod old;

use chrono::{TimeZone, Utc};
use otto_core::agent_runtime::{Attention, AttentionKind, TranscriptDigest};
use otto_core::workflows::{
    ChildThread, Deliverable, DeliverableState, HumanEntry, NodeRunState, NodeStatus, PhaseMark,
    RunOverview, RunState, RunStatus, StepPhase, StepThread, WorkflowCheckpoint, WorkflowGraph,
};
use otto_state::WorkflowsRepo;
use serde_json::json;

fn t(s: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, s).unwrap()
}

/// A running step that uses every field this build adds.
fn full_node() -> NodeRunState {
    NodeRunState {
        node_id: "n2".into(),
        status: NodeStatus::Running,
        logs: vec!["agent turn started".into()],
        started_at: Some(t(1)),
        attempts: Some(1),
        step_no: Some(2),
        step_file: Some("step2-review".into()),
        phase: Some(StepPhase::NeedsYou),
        phases: vec![PhaseMark {
            phase: StepPhase::Queued,
            at: t(1),
            note: Some("attempt 2 of 3".into()),
        }],
        wait: None,
        attention: Some(Attention::new(
            AttentionKind::Question,
            "Which base branch?",
            t(2),
        )),
        error_class: Some("timeout".into()),
        thread: Some(StepThread {
            thread_id: "t1".into(),
            runner: "threads".into(),
            provider: "claude".into(),
            model: Some("m".into()),
            effort: Some("low".into()),
            continued_from: Some("n1".into()),
            start_ordinal: Some(3),
            turns: 2,
            usage: Default::default(),
            digest: Some(TranscriptDigest {
                turns: 2,
                tool_calls: 5,
                ..Default::default()
            }),
            closed_at: None,
            available: None,
        }),
        children: vec![ChildThread {
            id: "r1".into(),
            thread_id: Some("t2".into()),
            label: "Correctness · Codex".into(),
            provider: "codex".into(),
            model: None,
            effort: None,
            status: "needs_you".into(),
            started_at: Some(t(2)),
            ended_at: None,
            note: Some("12 findings".into()),
            usage: Default::default(),
        }],
        deliverables: vec![Deliverable {
            label: "PR comments".into(),
            state: DeliverableState::Partial,
            detail: Some("3 of 4 posted".into()),
            url: None,
        }],
        human_log: vec![HumanEntry {
            kind: "answer".into(),
            ask: Some("Which base branch?".into()),
            text: "main".into(),
            met: None,
            revision: None,
            iteration: Some(1),
            by: "A person".into(),
            at: t(3),
        }],
        brief: Some("waiting for an answer".into()),
        ..Default::default()
    }
}

/// proves: X4 — a rolled-back binary decodes everything this build writes.
#[tokio::test]
async fn rows_with_run_view_fields_decode_in_old_types() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u', 'u', 'hash', 'User', 1, '2026-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w', 'Workspace', '/tmp', '2026-01-01T00:00:00Z')")
        .execute(&pool).await.unwrap();
    let repo = WorkflowsRepo::new(pool.clone());
    let wf = repo
        .create(
            &"w".into(),
            "Goal",
            "",
            "",
            &WorkflowGraph::default(),
            &"u".into(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE workflows SET origin = 'goal', origin_ref = 'mcp' WHERE id = ?")
        .bind(&wf.id)
        .execute(&pool)
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &wf.workspace_id, &json!({}), None)
        .await
        .unwrap();
    repo.update_run(&run.id, RunStatus::Running, &[], None, false)
        .await
        .unwrap();
    let nodes = vec![full_node()];
    let ov = RunOverview {
        state: RunState::NeedsYou,
        headline: "Which base branch?".into(),
        ..Default::default()
    };
    repo.update_run_progress_with(&run.id, &nodes, Some(&ov))
        .await
        .unwrap();
    let cp = WorkflowCheckpoint {
        node_id: "loop#0.0".into(),
        loop_id: "loop".into(),
        iteration: 0,
        step_index: 0,
        kind: "agent_prompt".into(),
        name: "Work".into(),
        status: NodeStatus::Success,
        attempts: 1,
        input: json!({}),
        output: None,
        error: None,
        logs: vec![],
        updated_at: t(4),
        active_ms: 61_000,
        thread_id: Some("t1".into()),
        start_ordinal: Some(3),
    };
    repo.save_checkpoint(&run.id, &cp).await.unwrap();

    let (status, nodes_json, progress_json): (String, String, String) =
        sqlx::query_as("SELECT status, nodes_json, progress_json FROM workflow_runs WHERE id = ?")
            .bind(&run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "running", "the stored status stays a 0.6 value");
    let old_nodes: Vec<old::NodeRunState> = serde_json::from_str(&nodes_json).unwrap();
    assert_eq!(old_nodes[0].node_id, "n2");
    assert_eq!(old_nodes[0].status, old::NodeStatus::Running);
    assert_eq!(old_nodes[0].logs, nodes[0].logs);
    assert_eq!(
        old_nodes[0].sessions,
        Vec::<String>::new(),
        "thread ids never land in sessions"
    );
    // The old build's progress reader serves the projection as JSON.
    let projection: Vec<serde_json::Value> = serde_json::from_str(&progress_json).unwrap();
    assert_eq!(projection[0]["status"], "running");
    let (cp_json,): (String,) =
        sqlx::query_as("SELECT checkpoint_json FROM workflow_checkpoints WHERE run_id = ?")
            .bind(&run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let old_cp: old::WorkflowCheckpoint = serde_json::from_str(&cp_json).unwrap();
    assert_eq!((old_cp.node_id.as_str(), old_cp.attempts), ("loop#0.0", 1));

    // A held goal run: settled with a 0.6 status, its hold in its own column.
    repo.update_run(
        &run.id,
        RunStatus::Error,
        &nodes,
        Some("Blocked: tests fail"),
        true,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE workflow_runs SET hold_json = ?, limits_json = ? WHERE id = ?")
        .bind(r#"{"kind":"blocked","reason":"tests fail","since":"2026-01-01T00:00:05Z","node_ids":["loop"]}"#)
        .bind(r#"{"loops":{"loop":{"max_iterations":9}}}"#)
        .bind(&run.id)
        .execute(&pool)
        .await
        .unwrap();
    let (status,): (String,) = sqlx::query_as("SELECT status FROM workflow_runs WHERE id = ?")
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(["pending", "running", "success", "error", "canceled"].contains(&status.as_str()));
    // Today's reader sees the hold the old one never names.
    let back = repo.get_run(&run.id).await.unwrap();
    assert_eq!(back.status, RunStatus::Error);
    assert!(back.hold.is_some() && back.limits.is_some());
}
