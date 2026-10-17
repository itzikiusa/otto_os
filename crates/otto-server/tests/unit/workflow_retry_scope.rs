use super::retry_includes_downstream;

#[test]
fn retrying_a_failed_middle_step_also_runs_what_it_blocked() {
    let graph = serde_json::from_value(serde_json::json!({
        "nodes": [
            {"id": "implement", "kind": "agent"},
            {"id": "review", "kind": "review_run"},
            {"id": "pr", "kind": "git_pr"}
        ],
        "edges": [
            {"id": "e1", "source": "implement", "target": "review"},
            {"id": "e2", "source": "review", "target": "pr"}
        ]
    }))
    .unwrap();
    let nodes = |states: [(&str, &str); 3]| -> Vec<otto_core::workflows::NodeRunState> {
        states
            .iter()
            .map(|(id, st)| {
                let logs = if *st == "skipped" {
                    vec![crate::workflow_engine::SKIPPED_UPSTREAM]
                } else {
                    vec![]
                };
                serde_json::from_value(
                    serde_json::json!({"node_id": id, "status": st, "logs": logs}),
                )
                .unwrap()
            })
            .collect()
    };
    let failed_first = nodes([
        ("implement", "error"),
        ("review", "skipped"),
        ("pr", "skipped"),
    ]);
    assert!(retry_includes_downstream(
        &graph,
        &failed_first,
        "implement",
        false
    ));
    // The last step (nothing downstream) stays a single-step retry.
    let failed_last = nodes([
        ("implement", "success"),
        ("review", "success"),
        ("pr", "error"),
    ]);
    assert!(!retry_includes_downstream(
        &graph,
        &failed_last,
        "pr",
        false
    ));
    assert!(retry_includes_downstream(&graph, &failed_last, "pr", true));
    // The upgraded single-step retry re-runs the blocked steps but does
    // NOT replay (drop) the target's checkpoints; "re-run from here" does.
    let (nodes_in, scope, replay) = super::retry_plan(&graph, &failed_first, "implement", false);
    assert_eq!(nodes_in.len(), 3);
    assert!(!scope.only_node);
    assert!(!replay, "a loop keeps its successful iterations");
    assert!(super::retry_plan(&graph, &failed_first, "implement", true).2);
    // Steps skipped as OUTSIDE the run's scope (a run-only-this-node run)
    // keep the retry single-step.
    let mut scoped = failed_first.clone();
    for n in scoped.iter_mut().skip(1) {
        n.logs = vec!["outside run scope".into()];
    }
    assert!(!retry_includes_downstream(
        &graph,
        &scoped,
        "implement",
        false
    ));
}
