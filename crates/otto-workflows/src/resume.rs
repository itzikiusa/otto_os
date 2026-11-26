//! Restart policy: which steps a restarted daemon may re-enter, and what
//! happens to a run a previous process left in flight. Pure, so the policy is
//! unit-testable without a daemon. Moved from otto-server's
//! `workflow_engine.rs` (`is_restart_resumable_kind`, `classify_resume`,
//! `settle_interrupted_nodes`); `classify_resume` now checks every running
//! node, since steps run side by side (engine.md §11.3).

use otto_core::workflows::{
    NodeRunState, NodeStatus, RunScope, RunStatus, WorkflowGraph, WorkflowNode, WorkflowRun,
};

/// Node kinds the startup reconciler may re-execute after a restart caught
/// them MID-FLIGHT. The allowlist is deliberately explicit: an unlisted kind
/// interrupted mid-step has an UNKNOWN outcome (`git_pr` may have opened the
/// PR, `channel_notify` may have posted, `api_run`/`http_request` may have
/// fired) — replaying it silently would double its side effect, so such runs
/// fail with a pointer at the manual retry-a-step flow instead. Listed kinds
/// are safe to re-run: pure/deterministic nodes, read-only fetches,
/// agent-backed steps (a held thread re-enters through the runner's resume,
/// else a fresh session in the run's still-alive worktree), `git_checkout`
/// (detach, merge-base and the registry rewrite repeat), `post_review_comments`
/// (every comment post is claimed atomically) and `check`. A `script` is
/// resumable only with `idempotent: true` — see [`is_restart_resumable`].
pub fn is_restart_resumable_kind(kind: &str) -> bool {
    matches!(
        kind,
        "manual_trigger"
            | "agent_prompt"
            | "prepare_context"
            | "transform"
            | "delay"
            | "log"
            | "condition"
            | "budget_gate"
            | "human_approval"
            | "review_run"
            | "db_query"
            | "broker_peek"
            | "game_engine"
            | "verifier"
            | "canvas"
            | "product_analyze"
            | "git_checkout"
            | "post_review_comments"
            | "check"
    )
}

/// [`is_restart_resumable_kind`] for a concrete node: a `script` depends on
/// its `idempotent` param (default false).
pub fn is_restart_resumable(node: &WorkflowNode) -> bool {
    match node.kind.as_str() {
        "script" => node.params.get("idempotent").and_then(|v| v.as_bool()) == Some(true),
        kind => is_restart_resumable_kind(kind),
    }
}

/// What the startup reconciler decided for one interrupted run.
#[derive(Debug)]
pub enum ResumeDecision {
    /// Re-enter the run: persist `nodes` + `scope` and spawn it.
    Resume {
        scope: RunScope,
        nodes: Vec<NodeRunState>,
    },
    /// The run cannot be safely resumed — fail it with `error` (nodes carry
    /// any per-node annotation, e.g. the unknown-outcome step marked Error).
    Fail {
        nodes: Vec<NodeRunState>,
        error: String,
    },
    /// Every node already settled (the daemon died between the last step and
    /// the finalize write) — just stamp the terminal status.
    Finish {
        status: RunStatus,
        nodes: Vec<NodeRunState>,
        error: Option<String>,
    },
}

/// Pure classification of one interrupted run: where to re-enter, or why not.
///
/// `prior_scope` is the persisted `resume_scope_json` (the scope the dead
/// process was executing). The caller has already applied the workflow-level
/// gates (`on_restart = 'fail'`, resume-attempt cap, missing workflow) — this
/// only reasons about the run's node states vs the graph:
///   * nodes mid-flight (several run side by side) → re-enter when EVERY one
///     of them is restart-safe ([`is_restart_resumable`], or a loop whose
///     checkpoints prove its inner work is), else fail with each unsafe step
///     marked "outcome unknown". The re-entered nodes go back to `pending`
///     with everything else on their state kept (`thread`, `sessions`,
///     `phases`, `step_no` …), so a held thread re-enters through the
///     runner's resume instead of a second agent (DR-006);
///   * paused at `human_approval` with nothing mid-flight → re-enter (it
///     re-parks; side-effect free by construction). A stale pause flag never
///     masks a side-effect step caught mid-flight: that step is checked like
///     any other;
///   * no node mid-flight → resume the persisted scope (a re-queued retry that
///     never started), else the first still-pending node, else finish the run
///     with the status its settled nodes imply.
pub fn classify_resume(
    graph: &WorkflowGraph,
    run: &WorkflowRun,
    prior_scope: Option<RunScope>,
) -> ResumeDecision {
    let mut nodes = run.nodes.clone();
    let node_of = |id: &str| graph.nodes.iter().find(|n| n.id == id);
    let running_ids: Vec<String> = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Running)
        .map(|n| n.node_id.clone())
        .collect();
    // A run paused at an approval re-enters at the gate too (it re-parks).
    let mut entries = running_ids.clone();
    if let Some(gate) = run
        .approval_node_id
        .as_ref()
        .filter(|_| run.waiting_approval)
    {
        if !entries.contains(gate) {
            entries.push(gate.clone());
        }
    }

    if !entries.is_empty() {
        if let Some(id) = entries.iter().find(|id| node_of(id).is_none()) {
            return ResumeDecision::Fail {
                nodes,
                error: format!(
                    "Interrupted by a daemon restart at unknown step '{id}' — re-run the workflow."
                ),
            };
        }
        let unsafe_nodes: Vec<&WorkflowNode> = entries
            .iter()
            .filter_map(|id| node_of(id))
            .filter(|n| !is_restart_resumable(n) && !checkpointed_loop(run, n))
            .collect();
        if !unsafe_nodes.is_empty() {
            // Unknown outcome: mark the steps themselves, settle the rest.
            for n in nodes.iter_mut() {
                if unsafe_nodes.iter().any(|u| u.id == n.node_id) {
                    n.status = NodeStatus::Error;
                    n.error = Some(
                        "interrupted mid-step by a daemon restart — outcome unknown (this step has external side effects)".into(),
                    );
                    n.logs
                        .push("✗ interrupted mid-step by a daemon restart".into());
                } else if matches!(n.status, NodeStatus::Pending | NodeStatus::Running) {
                    n.status = NodeStatus::Skipped;
                }
            }
            let steps = unsafe_nodes
                .iter()
                .map(|n| format!("'{}' ({})", n.id, n.kind))
                .collect::<Vec<_>>()
                .join(", ");
            return ResumeDecision::Fail {
                nodes,
                error: format!(
                    "Interrupted by a daemon restart during {steps} — its outcome is unknown, so it was not replayed. Retry the step (or \"re-run from here\") once you've checked what landed."
                ),
            };
        }
        for n in nodes.iter_mut() {
            if running_ids.contains(&n.node_id) {
                n.status = NodeStatus::Pending;
                n.error = None;
            }
        }
        // Preserve a single-step retry scope when the restart caught exactly
        // that step; otherwise re-enter and run the rest.
        let mut scope = prior_scope.unwrap_or_default();
        scope.continue_unfinished = true;
        return ResumeDecision::Resume { scope, nodes };
    }

    // No node mid-flight. A re-queued scoped re-entry that never started
    // (pending with progress) resumes its persisted scope verbatim.
    if run.status == RunStatus::Pending {
        return match prior_scope {
            Some(scope) => ResumeDecision::Resume { scope, nodes },
            None => ResumeDecision::Fail {
                nodes,
                error: "Interrupted by a daemon restart — the retry scope was lost (pre-resume run); use \"retry from this step\" again.".into(),
            },
        };
    }

    // Executing, but between node boundaries: resume while a node is still
    // pending, or finish the run if none remain.
    if crate::graph::topo_order(graph).is_ok()
        && nodes.iter().any(|n| {
            n.status == NodeStatus::Pending && graph.nodes.iter().any(|g| g.id == n.node_id)
        })
    {
        let mut scope = prior_scope.unwrap_or_default();
        scope.continue_unfinished = true;
        return ResumeDecision::Resume { scope, nodes };
    }
    let any_error = nodes.iter().any(|n| n.status == NodeStatus::Error);
    ResumeDecision::Finish {
        status: if any_error {
            RunStatus::Error
        } else {
            RunStatus::Success
        },
        nodes,
        error: any_error.then(|| "one or more nodes failed".to_string()),
    }
}

/// A `loop` caught mid-flight is resumable when it has a checkpoint and none
/// of its inner steps that were running is a side-effect kind.
fn checkpointed_loop(run: &WorkflowRun, node: &WorkflowNode) -> bool {
    let id = &node.id;
    node.kind == "loop"
        && run.checkpoints.iter().any(|cp| &cp.node_id == id)
        && run
            .checkpoints
            .iter()
            .filter(|cp| &cp.node_id == id || cp.node_id.starts_with(&format!("{id}#")))
            .all(|cp| {
                cp.status != NodeStatus::Running
                    || cp.kind == "loop"
                    || is_restart_resumable_kind(&cp.kind)
            })
}

/// Settle a run's node states for a terminal FAIL after a restart: a step
/// still `running` becomes `error` ("interrupted") — retryable, and no
/// ever-growing timer — and never-reached `pending` steps become `skipped`.
/// Settled states pass through untouched.
pub fn settle_interrupted_nodes(mut nodes: Vec<NodeRunState>) -> Vec<NodeRunState> {
    for n in nodes.iter_mut() {
        match n.status {
            NodeStatus::Running => {
                n.status = NodeStatus::Error;
                if n.error.is_none() {
                    n.error = Some("interrupted by a daemon restart".into());
                }
                n.activity = None;
            }
            NodeStatus::Pending => n.status = NodeStatus::Skipped,
            NodeStatus::Success | NodeStatus::Error | NodeStatus::Skipped => {}
        }
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::workflows::{StepThread, WorkflowCheckpoint, WorkflowEdge};
    use serde_json::{json, Value};

    fn node(id: &str, kind: &str) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            kind: kind.into(),
            name: id.into(),
            x: 0.0,
            y: 0.0,
            params: Value::Null,
            retry: None,
        }
    }

    fn edge(a: &str, b: &str) -> WorkflowEdge {
        WorkflowEdge {
            id: format!("{a}-{b}"),
            source: a.into(),
            target: b.into(),
            condition: None,
        }
    }

    fn nstate(id: &str, status: NodeStatus) -> NodeRunState {
        NodeRunState {
            node_id: id.into(),
            status,
            ..Default::default()
        }
    }

    fn mk_run(status: RunStatus, nodes: Vec<NodeRunState>) -> WorkflowRun {
        serde_json::from_value(json!({
            "id": "r1", "workflow_id": "wf1", "workspace_id": "ws1",
            "status": status, "input": null, "nodes": nodes,
            "started_at": "2026-11-26T10:00:00Z", "rev": 0,
        }))
        .expect("a minimal run decodes")
    }

    fn checkpoint(node_id: &str, kind: &str, status: NodeStatus) -> WorkflowCheckpoint {
        WorkflowCheckpoint {
            node_id: node_id.into(),
            kind: kind.into(),
            status,
            ..Default::default()
        }
    }

    /// A restart that caught an idempotent step mid-flight resumes AT it:
    /// the step flips back to Pending, finished siblings keep their states.
    #[test]
    fn reenters_at_interrupted_idempotent_step() {
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

    /// Steps run side by side now: EVERY running node is checked, not the first.
    #[test]
    fn a_side_effect_step_beside_a_resumable_one_fails_the_run() {
        let g = WorkflowGraph {
            nodes: vec![
                node("start", "manual_trigger"),
                node("vault", "agent_prompt"),
                node("notify", "channel_notify"),
                node("tail", "log"),
            ],
            edges: vec![
                edge("start", "vault"),
                edge("start", "notify"),
                edge("vault", "tail"),
            ],
        };
        let run = mk_run(
            RunStatus::Running,
            vec![
                nstate("start", NodeStatus::Success),
                nstate("vault", NodeStatus::Running),
                nstate("notify", NodeStatus::Running),
                nstate("tail", NodeStatus::Pending),
            ],
        );
        match classify_resume(&g, &run, None) {
            ResumeDecision::Fail { nodes, error } => {
                assert!(error.contains("'notify' (channel_notify)"), "{error}");
                assert_eq!(nodes[2].status, NodeStatus::Error);
                assert!(nodes[2]
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("outcome unknown"));
                assert_eq!(
                    nodes[1].status,
                    NodeStatus::Skipped,
                    "the resumable sibling settles"
                );
                assert_eq!(nodes[3].status, NodeStatus::Skipped);
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn several_resumable_steps_all_reenter_and_keep_their_thread() {
        let g = WorkflowGraph {
            nodes: vec![
                node("s", "manual_trigger"),
                node("a", "agent_prompt"),
                node("b", "review_run"),
            ],
            edges: vec![edge("s", "a"), edge("s", "b")],
        };
        let mut a = nstate("a", NodeStatus::Running);
        a.step_no = Some(2);
        a.thread = Some(StepThread {
            thread_id: "t-1".into(),
            runner: "threads".into(),
            provider: "claude".into(),
            model: None,
            effort: None,
            continued_from: None,
            start_ordinal: Some(3),
            turns: 1,
            usage: Default::default(),
            digest: None,
            closed_at: None,
            available: None,
        });
        let run = mk_run(
            RunStatus::Running,
            vec![
                nstate("s", NodeStatus::Success),
                a,
                nstate("b", NodeStatus::Running),
            ],
        );
        match classify_resume(&g, &run, None) {
            ResumeDecision::Resume { nodes, .. } => {
                assert_eq!(nodes[1].status, NodeStatus::Pending);
                assert_eq!(nodes[2].status, NodeStatus::Pending);
                assert_eq!(nodes[1].thread.as_ref().unwrap().thread_id, "t-1", "DR-006");
                assert_eq!(nodes[1].step_no, Some(2));
            }
            other => panic!("expected Resume, got {other:?}"),
        }
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
            ResumeDecision::Resume { scope, .. } => assert_eq!(scope.start_node, None),
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
        let graph = WorkflowGraph {
            nodes: vec![node("loop", "loop"), node("sibling", "log")],
            edges: vec![],
        };
        let mut run = mk_run(
            RunStatus::Running,
            vec![
                nstate("loop", NodeStatus::Running),
                nstate("sibling", NodeStatus::Pending),
            ],
        );
        run.checkpoints = vec![
            checkpoint("loop", "loop", NodeStatus::Running),
            checkpoint("loop#1.0", "http_request", NodeStatus::Success),
            checkpoint("loop#1.1", "delay", NodeStatus::Running),
        ];
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

    /// A restart that caught a SIDE-EFFECT step mid-flight must NOT replay it.
    #[test]
    fn fails_on_interrupted_side_effect_step() {
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

    #[test]
    fn reenters_at_waiting_approval_node() {
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
            ResumeDecision::Resume { scope, nodes } => {
                assert_eq!(scope.start_node, None);
                assert!(scope.continue_unfinished);
                assert_eq!(nodes[1].status, NodeStatus::Pending);
            }
            other => panic!("expected Resume, got {other:?}"),
        }
    }

    /// A STALE approval flag (the run had moved past its gate) must not mask
    /// a side-effect step caught mid-flight.
    #[test]
    fn stale_approval_flag_never_masks_a_side_effect_step() {
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

    #[test]
    fn unknown_running_step_fails() {
        let g = WorkflowGraph::default();
        let run = mk_run(
            RunStatus::Running,
            vec![nstate("ghost", NodeStatus::Running)],
        );
        match classify_resume(&g, &run, None) {
            ResumeDecision::Fail { error, .. } => assert!(error.contains("unknown step 'ghost'")),
            other => panic!("expected Fail, got {other:?}"),
        }
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

    /// A re-queued retry-a-step (pending with progress) resumes its PERSISTED
    /// scope verbatim; without one (pre-0108 row) it fails like before.
    #[test]
    fn honors_persisted_retry_scope() {
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

    #[test]
    fn finishes_a_fully_settled_run() {
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

    #[test]
    fn resumes_at_first_pending_between_boundaries() {
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
            "product_publish",
            "api_run",
            "http_request",
            "self_improve",
            "product_rewrite",
            "product_plan",
            "swarm_task",
            "script",
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
            "git_checkout",
            "post_review_comments",
            "check",
        ] {
            assert!(is_restart_resumable_kind(k), "{k} should be restart-safe");
        }
    }

    #[test]
    fn a_script_is_resumable_only_when_declared_idempotent() {
        let mut s = node("gates", "script");
        assert!(!is_restart_resumable(&s));
        s.params = json!({"idempotent": false});
        assert!(!is_restart_resumable(&s));
        s.params = json!({"idempotent": true});
        assert!(is_restart_resumable(&s));
        assert!(is_restart_resumable(&node("a", "agent_prompt")));
        assert!(!is_restart_resumable(&node("n", "channel_notify")));
    }
}
