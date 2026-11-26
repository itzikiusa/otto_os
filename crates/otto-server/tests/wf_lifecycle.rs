//! Workflow run lifecycle across a daemon restart (test-plan §5.1, guard G1).
//!
//! Phase 0 pins TODAY's classification: for every kind in the node catalog,
//! what the startup reconciler's `classify_resume` decides when a restart
//! caught a node of that kind mid-flight. The table below is the single place
//! a kind's restart behaviour is decided; adding a catalog kind without a row
//! fails `restart_expectation_for_every_kind` (G1). The mid-node restart
//! harness (drop the engine, boot on the same data dir) lands with each kind
//! ported to the new runner (test-plan Phase 4) and reads the same table.

use otto_core::workflows::{NodeStatus, WorkflowGraph, WorkflowRun};
use otto_server::workflow_engine::{classify_resume, ResumeDecision};
use serde_json::{json, Value};

/// What a restart does with a run whose node of this kind was mid-flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Restart {
    /// Re-entered at the node (pure, read-only, or a fresh agent session in
    /// the run's worktree).
    Resume,
    /// Failed with the step marked "outcome unknown": it has external side
    /// effects and is never replayed.
    FailOutcomeUnknown,
    /// Resumed only from its durable checkpoint (and only when the inner step
    /// caught mid-flight is itself resumable); failed without one.
    ResumeFromCheckpoint,
}

/// Today's restart classification for every catalog kind (test-plan §5.1).
const RESTART: &[(&str, Restart)] = &[
    ("manual_trigger", Restart::Resume),
    ("agent_prompt", Restart::Resume),
    ("prepare_context", Restart::Resume),
    ("http_request", Restart::FailOutcomeUnknown),
    ("transform", Restart::Resume),
    ("delay", Restart::Resume),
    ("log", Restart::Resume),
    ("game_engine", Restart::Resume),
    ("verifier", Restart::Resume),
    ("db_query", Restart::Resume),
    ("broker_peek", Restart::Resume),
    ("channel_notify", Restart::FailOutcomeUnknown),
    ("budget_gate", Restart::Resume),
    ("human_approval", Restart::Resume),
    ("condition", Restart::Resume),
    ("loop", Restart::ResumeFromCheckpoint),
    ("product_analyze", Restart::Resume),
    ("product_rewrite", Restart::FailOutcomeUnknown),
    ("product_plan", Restart::FailOutcomeUnknown),
    ("product_publish", Restart::FailOutcomeUnknown),
    ("canvas", Restart::Resume),
    ("review_run", Restart::Resume),
    ("git_pr", Restart::FailOutcomeUnknown),
    ("api_run", Restart::FailOutcomeUnknown),
    ("self_improve", Restart::FailOutcomeUnknown),
];

fn graph(kind: &str) -> WorkflowGraph {
    // Built from variables: otto-core's background-source guard reads every
    // literal string under a "source" key in the workspace as a session source.
    let edge = |id: &str, from: &str, to: &str| json!({"id": id, "source": from, "target": to});
    serde_json::from_value(json!({
        "nodes": [
            {"id": "n1", "kind": "manual_trigger", "name": "Start 1"},
            {"id": "n2", "kind": kind, "name": "Step 2"},
            {"id": "n3", "kind": "log", "name": "Log 3"},
        ],
        "edges": [
            edge("e1", "n1", "n2"),
            edge("e2", "n2", "n3"),
        ],
    }))
    .unwrap()
}

/// A run a dead daemon left with n2 mid-flight.
fn interrupted(extra: Value) -> WorkflowRun {
    let mut run = json!({
        "id": "run-1", "workflow_id": "wf-1", "workspace_id": "ws-1", "status": "running",
        "started_at": "2026-01-01T00:00:00Z",
        "nodes": [
            {"node_id": "n1", "status": "success", "output": {}},
            {"node_id": "n2", "status": "running", "sessions": ["session-1"]},
            {"node_id": "n3", "status": "pending"},
        ],
    });
    for (k, v) in extra.as_object().unwrap() {
        run[k] = v.clone();
    }
    serde_json::from_value(run).unwrap()
}

fn checkpoint(node_id: &str, kind: &str, status: &str) -> Value {
    json!({"node_id": node_id, "loop_id": "n2", "iteration": 1, "step_index": 0, "kind": kind,
           "name": "inner", "status": status, "attempts": 1, "input": {}, "output": null,
           "error": null, "updated_at": "2026-01-01T00:00:01Z"})
}

fn status_of(nodes: &[otto_core::workflows::NodeRunState], id: &str) -> NodeStatus {
    nodes.iter().find(|n| n.node_id == id).unwrap().status
}

/// What `classify_resume` decided, checked down to the node states a person
/// sees in the run view afterwards.
fn observed(kind: &str, run: &WorkflowRun) -> Restart {
    match classify_resume(&graph(kind), run, None) {
        ResumeDecision::Resume { scope, nodes } => {
            assert!(
                scope.continue_unfinished,
                "{kind}: re-entry continues the run"
            );
            assert_eq!(
                status_of(&nodes, "n2"),
                NodeStatus::Pending,
                "{kind}: re-run from pending"
            );
            assert_eq!(
                status_of(&nodes, "n1"),
                NodeStatus::Success,
                "{kind}: settled step kept"
            );
            Restart::Resume
        }
        ResumeDecision::Fail { nodes, error } => {
            let n2 = nodes.iter().find(|n| n.node_id == "n2").unwrap();
            assert_eq!(n2.status, NodeStatus::Error, "{kind}");
            assert!(
                n2.error
                    .as_deref()
                    .unwrap_or_default()
                    .contains("outcome unknown"),
                "{kind}: {:?}",
                n2.error
            );
            assert_eq!(
                status_of(&nodes, "n3"),
                NodeStatus::Skipped,
                "{kind}: nothing after it runs"
            );
            assert!(error.contains("not replayed"), "{kind}: {error}");
            Restart::FailOutcomeUnknown
        }
        other => panic!("{kind}: unexpected {other:?}"),
    }
}

/// G1: the restart table covers the node catalog exactly — a new kind fails
/// CI until its restart behaviour is decided here.
#[test]
fn restart_expectation_for_every_kind() {
    let catalog: Vec<String> = otto_workflows::catalog::node_catalog()
        .into_iter()
        .map(|s| s.kind)
        .collect();
    let missing: Vec<&String> = catalog
        .iter()
        .filter(|k| !RESTART.iter().any(|(r, _)| r == k))
        .collect();
    assert!(
        missing.is_empty(),
        "catalog kinds with no restart row (test-plan §5.1): {missing:?}"
    );
    let stale: Vec<&str> = RESTART
        .iter()
        .map(|(k, _)| *k)
        .filter(|k| !catalog.iter().any(|c| c == k))
        .collect();
    assert!(
        stale.is_empty(),
        "restart rows for kinds not in the catalog: {stale:?}"
    );
    let mut kinds: Vec<&str> = RESTART.iter().map(|(k, _)| *k).collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), RESTART.len(), "one row per kind");
}

/// Part of X5: today's classification of a node caught mid-flight, per kind.
#[test]
fn restart_mid_node_classification_today() {
    for (kind, expected) in RESTART {
        let run = interrupted(json!({}));
        let got = observed(kind, &run);
        let want = if *expected == Restart::ResumeFromCheckpoint {
            Restart::FailOutcomeUnknown
        } else {
            *expected
        };
        assert_eq!(got, want, "{kind} mid-flight, no checkpoint");
    }
}

/// Part of X5: a loop resumes from its durable checkpoint only when the
/// inner step caught mid-flight is itself restart-safe.
#[test]
fn restart_mid_loop_resumes_only_from_a_safe_checkpoint() {
    let safe = interrupted(json!({"checkpoints": [
        checkpoint("n2", "loop", "running"),
        checkpoint("n2#1.0", "agent_prompt", "running"),
    ]}));
    assert_eq!(observed("loop", &safe), Restart::Resume);
    let side_effect = interrupted(json!({"checkpoints": [
        checkpoint("n2", "loop", "running"),
        checkpoint("n2#1.0", "channel_notify", "running"),
    ]}));
    assert_eq!(observed("loop", &side_effect), Restart::FailOutcomeUnknown);
}

/// Part of X5: a run parked at an approval re-parks; a stale pause flag never
/// makes a side-effect step caught mid-flight look resumable.
#[test]
fn restart_at_an_approval_reparks_but_never_replays_a_side_effect() {
    let parked = interrupted(json!({"waiting_approval": true, "approval_node_id": "n2"}));
    assert_eq!(observed("human_approval", &parked), Restart::Resume);
    let stale = interrupted(json!({"waiting_approval": true, "approval_node_id": "n1"}));
    assert_eq!(
        observed("channel_notify", &stale),
        Restart::FailOutcomeUnknown
    );
}
