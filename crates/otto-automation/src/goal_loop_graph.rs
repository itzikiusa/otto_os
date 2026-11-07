//! Deterministic task admission and evidence gates for the existing Goal Loop.
use std::collections::HashSet;

use otto_core::domain::{
    EvalCriterion, GoalGraphNode, GoalGraphNodeState, GoalLoopConfig, GoalLoopDefinition,
    GoalLoopEvaluation, GoalLoopGraph, GoalLoopLedger,
};
use otto_core::{Error, Result};

pub fn validate(config: &GoalLoopConfig, definition: &GoalLoopDefinition) -> Result<()> {
    let Some(graph) = &config.graph else {
        return Ok(());
    };
    if graph.nodes.is_empty() || graph.nodes.len() > 16 {
        return Err(Error::Invalid("a task graph needs 1–16 nodes".into()));
    }
    let ids: HashSet<_> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let criteria: HashSet<_> = definition
        .acceptance_criteria
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let mut covered = HashSet::new();
    if ids.len() != graph.nodes.len() {
        return Err(Error::Invalid("task ids must be unique".into()));
    }
    for node in &graph.nodes {
        if node.id.trim().is_empty()
            || node.title.trim().is_empty()
            || node.instructions.trim().is_empty()
            || node.criterion_ids.is_empty()
        {
            return Err(Error::Invalid(
                "each task needs an id, title, instructions and acceptance criteria".into(),
            ));
        }
        let deps: HashSet<_> = node.depends_on.iter().map(String::as_str).collect();
        let refs: HashSet<_> = node.criterion_ids.iter().map(String::as_str).collect();
        if deps.len() != node.depends_on.len()
            || deps.contains(node.id.as_str())
            || !deps.is_subset(&ids)
        {
            return Err(Error::Invalid(
                "task dependencies must be unique known task ids, excluding itself".into(),
            ));
        }
        if refs.len() != node.criterion_ids.len() || !refs.is_subset(&criteria) {
            return Err(Error::Invalid(
                "task criteria must be unique known acceptance criterion ids".into(),
            ));
        }
        if definition
            .acceptance_criteria
            .iter()
            .any(|c| c.verify_kind == "human" && refs.contains(c.id.as_str()))
            && graph
                .nodes
                .iter()
                .any(|other| other.depends_on.contains(&node.id))
        {
            return Err(Error::Invalid("Move human acceptance to a final task: human-verification tasks cannot have dependents because later mutations invalidate approval".into()));
        }
        covered.extend(refs);
    }
    if covered != criteria {
        return Err(Error::Invalid(
            "assign every acceptance criterion to a graph task".into(),
        ));
    }
    if order(graph).len() != graph.nodes.len() {
        return Err(Error::Invalid(
            "task dependencies must not contain a cycle".into(),
        ));
    }
    Ok(())
}

fn order(graph: &GoalLoopGraph) -> Vec<usize> {
    let mut done = HashSet::new();
    let mut result = Vec::new();
    while let Some((i, node)) = graph.nodes.iter().enumerate().find(|(_, n)| {
        !done.contains(n.id.as_str()) && n.depends_on.iter().all(|d| done.contains(d.as_str()))
    }) {
        done.insert(node.id.as_str());
        result.push(i);
    }
    result
}

pub fn initialize(graph: &GoalLoopGraph, ledger: &mut GoalLoopLedger) {
    ledger
        .graph_nodes
        .retain(|s| graph.nodes.iter().any(|n| n.id == s.node_id));
    for node in &graph.nodes {
        if !ledger.graph_nodes.iter().any(|s| s.node_id == node.id) {
            ledger.graph_nodes.push(GoalGraphNodeState {
                node_id: node.id.clone(),
                status: "pending".into(),
                iteration: 0,
                evidence: "Not executed".into(),
                criteria: Vec::new(),
            });
        }
    }
}

fn passed(ledger: &GoalLoopLedger, id: &str) -> bool {
    ledger
        .graph_nodes
        .iter()
        .any(|s| s.node_id == id && s.status == "passed")
}

/// Only previously passed nodes or the task executed this iteration may pass.
/// Topological order invalidates every descendant of a regressed prerequisite.
pub fn reconcile(
    graph: &GoalLoopGraph,
    ledger: &mut GoalLoopLedger,
    eval: &GoalLoopEvaluation,
    selected: Option<&str>,
) {
    initialize(graph, ledger);
    for i in order(graph) {
        let node = &graph.nodes[i];
        let deps_passed = node.depends_on.iter().all(|d| passed(ledger, d));
        let state = ledger
            .graph_nodes
            .iter_mut()
            .find(|s| s.node_id == node.id)
            .unwrap();
        let eligible = needs_refresh(state) || selected == Some(node.id.as_str());
        if !deps_passed {
            state.status = "pending".into();
            state.evidence =
                "Prerequisite evidence is missing or regressed; this task must run again".into();
            state.criteria.clear();
        } else if eligible {
            state.criteria = node
                .criterion_ids
                .iter()
                .map(|id| {
                    eval.criteria
                        .iter()
                        .find(|c| c.id == *id)
                        .cloned()
                        .unwrap_or(EvalCriterion {
                            id: id.clone(),
                            met: false,
                            evidence: "Not assessed".into(),
                        })
                })
                .collect();
            let verified = state.iteration > 0
                && state
                    .criteria
                    .iter()
                    .all(|c| c.met && !c.evidence.trim().is_empty());
            state.status = if verified { "passed" } else { "blocked" }.into();
            state.evidence = state
                .criteria
                .iter()
                .map(|c| format!("{}: {}", c.id, c.evidence))
                .collect::<Vec<_>>()
                .join("\n");
        }
    }
}

pub fn needs_refresh(state: &GoalGraphNodeState) -> bool {
    state.status == "passed"
        || (state.status == "blocked"
            && !state.criteria.is_empty()
            && state.criteria.iter().any(|c| !c.met)
            && state
                .criteria
                .iter()
                .all(|c| c.met || c.evidence == "Awaiting human verification"))
}

fn waiting_for_human(node: &GoalGraphNode, ledger: &GoalLoopLedger) -> bool {
    ledger
        .graph_nodes
        .iter()
        .find(|s| s.node_id == node.id)
        .is_some_and(|s| {
            s.status == "blocked"
                && !s.criteria.is_empty()
                && s.criteria.iter().any(|c| !c.met)
                && s.criteria
                    .iter()
                    .all(|c| c.met || c.evidence == "Awaiting human verification")
        })
}

pub fn next_node<'a>(
    graph: &'a GoalLoopGraph,
    ledger: &GoalLoopLedger,
) -> Option<&'a GoalGraphNode> {
    graph.nodes.iter().find(|node| {
        !passed(ledger, &node.id)
            && !waiting_for_human(node, ledger)
            && node.depends_on.iter().all(|d| passed(ledger, d))
    })
}

pub fn all_passed(graph: &GoalLoopGraph, ledger: &GoalLoopLedger) -> bool {
    !graph.nodes.is_empty() && graph.nodes.iter().all(|n| passed(ledger, &n.id))
}

/// A failed whole-goal review needs actual repair work on Resume, not another
/// identical review of an already-passed graph. Reopen the final terminal task.
pub fn reopen_completion(graph: &GoalLoopGraph, ledger: &mut GoalLoopLedger) {
    if let Some(node) = graph.nodes.iter().rev().find(|node| {
        !graph
            .nodes
            .iter()
            .any(|other| other.depends_on.contains(&node.id))
    }) {
        if let Some(state) = ledger.graph_nodes.iter_mut().find(|s| s.node_id == node.id) {
            state.status = "blocked".into();
            state.evidence =
                "Completion review requires repair; see the persisted review summary".into();
            state.criteria.clear();
        }
    }
}

pub fn reconcile_verdict(
    graph: &GoalLoopGraph,
    ledger: &GoalLoopLedger,
    eval: &mut GoalLoopEvaluation,
) {
    if all_passed(graph, ledger) {
        return;
    }
    let ready = next_node(graph, ledger);
    if eval.verdict == "achieved"
        || (eval.verdict == "blocked" && eval.feedback.starts_with("Work is ready for human"))
    {
        eval.verdict = "continue".into();
    }
    if let Some(node) = ready {
        eval.feedback = format!("Next task: {}. {}", node.title, eval.feedback);
    } else {
        eval.verdict = "blocked".into();
        eval.feedback = "Work is ready for human verification. Verify the task criteria awaiting human evidence, then Resume.".into();
    }
    // A global evaluator's optimistic score must not present unfinished tasks as done.
    eval.progress_pct = eval.progress_pct.min(
        (graph.nodes.iter().filter(|n| passed(ledger, &n.id)).count() * 100 / graph.nodes.len())
            as u32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (GoalLoopConfig, GoalLoopDefinition) {
        let cfg = GoalLoopConfig { graph: Some(serde_json::from_value(serde_json::json!({"nodes":[
            {"id":"a","title":"A","instructions":"First","depends_on":[],"criterion_ids":["ca"]},
            {"id":"b","title":"B","instructions":"Second","depends_on":["a"],"criterion_ids":["cb"]},
            {"id":"c","title":"C","instructions":"Third","depends_on":["b"],"criterion_ids":["cc"]}
        ]})).unwrap()), ..Default::default() };
        let d = serde_json::from_value(serde_json::json!({"title":"Goal","acceptance_criteria":[
            {"id":"ca","text":"A","verify":"inspect"},{"id":"cb","text":"B","verify":"inspect"},{"id":"cc","text":"C","verify":"inspect"}
        ]})).unwrap();
        (cfg, d)
    }
    fn eval() -> GoalLoopEvaluation {
        serde_json::from_value(serde_json::json!({"progress_pct":100,"verdict":"achieved","criteria":[
            {"id":"ca","met":true,"evidence":"checked A"},{"id":"cb","met":true,"evidence":"checked B"},{"id":"cc","met":true,"evidence":"checked C"}
        ]})).unwrap()
    }
    #[test]
    fn rejects_cycles_unknown_references_and_uncovered_criteria() {
        let (mut cfg, d) = fixture();
        assert!(validate(&cfg, &d).is_ok());
        cfg.graph.as_mut().unwrap().nodes[0]
            .depends_on
            .push("c".into());
        assert!(validate(&cfg, &d).is_err());
        cfg.graph.as_mut().unwrap().nodes[0].depends_on = vec!["missing".into()];
        assert!(validate(&cfg, &d).is_err());
        cfg.graph.as_mut().unwrap().nodes[0].depends_on.clear();
        cfg.graph.as_mut().unwrap().nodes[0].criterion_ids = vec!["cb".into()];
        assert!(validate(&cfg, &d).is_err());
    }
    #[test]
    fn human_acceptance_must_be_terminal_so_mutations_cannot_deadlock_dependencies() {
        let (cfg, mut definition) = fixture();
        definition.acceptance_criteria[0].verify_kind = "human".into();
        assert!(validate(&cfg, &definition)
            .unwrap_err()
            .to_string()
            .contains("final task"));
        definition.acceptance_criteria[0].verify_kind = "agent".into();
        definition.acceptance_criteria[2].verify_kind = "human".into();
        assert!(validate(&cfg, &definition).is_ok());
    }

    #[test]
    fn optimistic_evaluation_cannot_skip_execution_or_dependency_gate() {
        let (cfg, _) = fixture();
        let g = cfg.graph.unwrap();
        let mut l = GoalLoopLedger::default();
        initialize(&g, &mut l);
        reconcile(&g, &mut l, &eval(), None);
        assert_eq!(next_node(&g, &l).unwrap().id, "a");
        assert!(!all_passed(&g, &l));
        l.graph_nodes[0].iteration = 1;
        reconcile(&g, &mut l, &eval(), Some("a"));
        assert_eq!(next_node(&g, &l).unwrap().id, "b");
        assert_eq!(l.graph_nodes[1].status, "pending");
        let mut e = eval();
        reconcile_verdict(&g, &l, &mut e);
        assert_eq!(e.verdict, "continue");
        assert_eq!(e.progress_pct, 33);
    }
    #[test]
    fn regression_invalidates_transitive_dependents_and_blank_evidence_never_passes() {
        let (cfg, _) = fixture();
        let g = cfg.graph.unwrap();
        let mut l = GoalLoopLedger::default();
        initialize(&g, &mut l);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            l.graph_nodes[i].iteration = i as u32 + 1;
            reconcile(&g, &mut l, &eval(), Some(id));
        }
        assert!(all_passed(&g, &l));
        let mut e = eval();
        e.criteria[0].evidence.clear();
        reconcile(&g, &mut l, &e, None);
        assert_eq!(l.graph_nodes[0].status, "blocked");
        assert_eq!(l.graph_nodes[1].status, "pending");
        assert_eq!(l.graph_nodes[2].status, "pending");
        reconcile(&g, &mut l, &eval(), Some("a"));
        assert_eq!(next_node(&g, &l).unwrap().id, "b");
    }
    #[test]
    fn rejected_completion_review_reopens_actual_terminal_work() {
        let (cfg, _) = fixture();
        let g = cfg.graph.unwrap();
        let mut l = GoalLoopLedger::default();
        initialize(&g, &mut l);
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            l.graph_nodes[i].iteration = i as u32 + 1;
            reconcile(&g, &mut l, &eval(), Some(id));
        }
        assert!(all_passed(&g, &l));
        reopen_completion(&g, &mut l);
        assert!(!all_passed(&g, &l));
        reconcile(&g, &mut l, &eval(), None);
        assert_eq!(next_node(&g, &l).unwrap().id, "c");
        assert_eq!(l.graph_nodes[0].status, "passed");
    }

    #[test]
    fn pending_human_task_does_not_block_independent_work() {
        let (mut cfg, _) = fixture();
        let g = cfg.graph.as_mut().unwrap();
        g.nodes[1].depends_on.clear();
        let mut l = GoalLoopLedger::default();
        initialize(g, &mut l);
        l.graph_nodes[0].iteration = 1;
        let mut e = eval();
        e.criteria[0].met = false;
        e.criteria[0].evidence = "Awaiting human verification".into();
        reconcile(g, &mut l, &e, Some("a"));
        assert_eq!(next_node(g, &l).unwrap().id, "b");
        e.verdict = "blocked".into();
        e.feedback = "Work is ready for human verification".into();
        reconcile_verdict(g, &l, &mut e);
        assert_eq!(e.verdict, "continue");
    }
}
