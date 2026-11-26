//! Pure graph helpers of the workflow engine: run order, the per-node
//! decision from incoming edges, input assembly and edge conditions.
//! Moved from otto-server's `workflow_engine.rs` (topo order, edge views,
//! `decide_node`, `assemble_input`, `eval_outgoing`) and `run/state.rs`
//! (`reconstruct_branches`); the parallel scheduler (`crate::schedule`) and
//! the engine share them.

use std::collections::{HashMap, HashSet};

use otto_core::workflows::{NodeRunState, NodeStatus, WorkflowEdge, WorkflowGraph, WorkflowNode};
use serde_json::{json, Value};

use crate::context::RepoEntry;
use crate::run_refs::RunPr;

/// Kahn topological sort with a FIFO queue, seeded with the in-degree-0
/// nodes in graph order, successors pushed in edge order. This order is the
/// run order: it decides step numbers, so it never changes. Errors on a cycle.
pub fn topo_order(graph: &WorkflowGraph) -> Result<Vec<String>, String> {
    let mut indeg: HashMap<&str, usize> = HashMap::new();
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for n in &graph.nodes {
        indeg.entry(&n.id).or_insert(0);
        adj.entry(&n.id).or_default();
    }
    for e in &graph.edges {
        if !indeg.contains_key(e.target.as_str()) || !indeg.contains_key(e.source.as_str()) {
            continue; // dangling edge — ignore
        }
        *indeg.entry(&e.target).or_insert(0) += 1;
        adj.entry(&e.source).or_default().push(&e.target);
    }
    let mut queue: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| indeg.get(n.id.as_str()).copied().unwrap_or(0) == 0)
        .map(|n| n.id.as_str())
        .collect();
    let mut i = 0;
    while i < queue.len() {
        let id = queue[i];
        i += 1;
        for s in adj.get(id).cloned().unwrap_or_default() {
            let d = indeg.get_mut(s).unwrap();
            *d -= 1;
            if *d == 0 {
                queue.push(s);
            }
        }
    }
    if queue.len() != graph.nodes.len() {
        return Err("workflow graph has a cycle".into());
    }
    Ok(queue.into_iter().map(str::to_string).collect())
}

/// `start` plus every node reachable from it via edges.
pub fn descendants_inclusive(graph: &WorkflowGraph, start: &str) -> HashSet<String> {
    reachable(graph, start, |e| (&e.source, &e.target))
}

/// Every node `node` is reachable from (its transitive predecessors), not
/// `node` itself. The step files a step is told to read are its settled
/// ancestors' (engine.md §5.2).
pub fn ancestors(graph: &WorkflowGraph, node: &str) -> HashSet<String> {
    let mut set = reachable(graph, node, |e| (&e.target, &e.source));
    set.remove(node);
    set
}

fn reachable(
    graph: &WorkflowGraph,
    start: &str,
    dir: impl Fn(&WorkflowEdge) -> (&String, &String),
) -> HashSet<String> {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    for e in &graph.edges {
        let (from, to) = dir(e);
        adj.entry(from).or_default().push(to);
    }
    let mut set = HashSet::new();
    let mut stack = vec![start];
    while let Some(n) = stack.pop() {
        if set.insert(n.to_string()) {
            stack.extend(adj.get(n).into_iter().flatten().copied());
        }
    }
    set
}

/// Edges entering `node_id` (in graph order).
pub fn incoming_edges<'a>(graph: &'a WorkflowGraph, node_id: &str) -> Vec<&'a WorkflowEdge> {
    graph.edges.iter().filter(|e| e.target == node_id).collect()
}

/// Edges leaving `node_id` (in graph order).
pub fn outgoing_edges<'a>(graph: &'a WorkflowGraph, node_id: &str) -> Vec<&'a WorkflowEdge> {
    graph.edges.iter().filter(|e| e.source == node_id).collect()
}

/// Whether an incoming edge from `source` constrains control flow: its
/// source is in the run scope, or it already ran in the prior run. A
/// start-from-here run leaves ancestors out of scope; their edges neither
/// poison nor branch-skip the entry node, which falls back to the run input.
pub fn dependency_is_relevant(
    source: &str,
    run_set: Option<&HashSet<String>>,
    prior: Option<&[NodeRunState]>,
) -> bool {
    run_set.is_none_or(|set| set.contains(source))
        || prior.is_some_and(|nodes| {
            nodes
                .iter()
                .any(|node| node.node_id == source && node.status != NodeStatus::Pending)
        })
}

/// A reduced view of one in-scope incoming edge, for the branching decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeView {
    pub source: String,
    /// The source node errored (or was poisoned).
    pub errored: bool,
    /// The source produced an output (ran successfully, or a chat skip's marker).
    pub has_output: bool,
    /// This edge's condition is satisfied (true / absent).
    pub edge_active: bool,
}

/// The views of `node_id`'s relevant incoming edges, from what the run knows.
pub fn edge_views(
    graph: &WorkflowGraph,
    node_id: &str,
    relevant: impl Fn(&str) -> bool,
    errored: &HashSet<String>,
    has_output: impl Fn(&str) -> bool,
    inactive_edges: &HashSet<String>,
) -> Vec<EdgeView> {
    incoming_edges(graph, node_id)
        .into_iter()
        .filter(|e| relevant(&e.source))
        .map(|e| EdgeView {
            source: e.source.clone(),
            errored: errored.contains(&e.source),
            has_output: has_output(&e.source),
            edge_active: !inactive_edges.contains(&e.id),
        })
        .collect()
}

/// The control-flow decision for a node, derived purely from its incoming edges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeDecision {
    /// Run, assembling input from these satisfied source ids.
    Run(Vec<String>),
    /// Skip + propagate failure: an active-path predecessor errored.
    ErrorSkip,
    /// Skip without failure: the node has in-scope predecessors but no satisfied
    /// edge (every branch into it was pruned, or upstream was branch-skipped).
    BranchSkip,
}

/// Decide whether a node runs.
///
/// - An errored in-scope predecessor poisons the node (ErrorSkip).
/// - Otherwise, "satisfied" sources are those that produced output via an active
///   edge; if there are in-scope incoming edges but none satisfied, the node is
///   BranchSkip; else Run with the satisfied sources (empty ⇒ an entry node that
///   falls back to the run input).
pub fn decide_node(views: &[EdgeView]) -> NodeDecision {
    if views.iter().any(|v| v.errored) {
        return NodeDecision::ErrorSkip;
    }
    let satisfied: Vec<String> = views
        .iter()
        .filter(|v| v.has_output && v.edge_active)
        .map(|v| v.source.clone())
        .collect();
    if !views.is_empty() && satisfied.is_empty() {
        return NodeDecision::BranchSkip;
    }
    NodeDecision::Run(satisfied)
}

/// Build a node's input from its predecessors' outputs: the lone predecessor's
/// output when there's exactly one, an object keyed by source id when several,
/// or the run input for source nodes.
///
/// Only predecessors that actually produced an output count. When a node is the
/// entry point of a start-from-here run, its predecessors were skipped (no
/// output), so it falls back to the run `input`.
pub fn assemble_input(
    upstream: &[String],
    outputs: &HashMap<String, Value>,
    run_input: &Value,
) -> Value {
    let present: Vec<&String> = upstream
        .iter()
        .filter(|p| outputs.contains_key(*p))
        .collect();
    match present.len() {
        0 => run_input.clone(),
        1 => outputs.get(present[0]).cloned().unwrap_or(Value::Null),
        _ => Value::Object(
            present
                .into_iter()
                .map(|p| (p.clone(), outputs.get(p).cloned().unwrap_or(Value::Null)))
                .collect(),
        ),
    }
}

/// The `run` object conditions see: the run input, the current `repos.json`
/// registry and the run's PRs (`prs.json`). `len(run.repos) > 1` is "more
/// than one repo". Read when the condition is evaluated (its source settles),
/// so a repos writer must not run beside a node whose condition reads it
/// (validation warns, engine.md §3.3).
pub fn run_data(input: &Value, repos: &[RepoEntry], prs: &[RunPr]) -> Value {
    json!({ "input": input, "repos": repos, "prs": prs })
}

/// What a `condition` node's `expr` is evaluated against (DR-005): its
/// input at the root, as always (`passed == true` keeps working), plus a
/// `run` key with [`run_data`] — unless the input already has its own
/// `run` key, which then stays. A non-object input is used as it is.
pub fn condition_root(input: &Value, run: &Value) -> Value {
    let mut root = input.clone();
    if let Some(obj) = root.as_object_mut() {
        obj.entry("run").or_insert_with(|| run.clone());
    }
    root
}

/// Evaluate the conditions on a node's outgoing edges against its output.
/// Returns `(inactive_edge_ids, log_lines)`. The context is `{output, input,
/// node, run}` with `run` from [`run_data`]. An edge with no condition is
/// always active; a condition that fails to parse/evaluate is treated as
/// *not taken* (and logged).
pub fn eval_outgoing(
    graph: &WorkflowGraph,
    node: &WorkflowNode,
    output: &Value,
    node_input: &Value,
    run: &Value,
) -> (Vec<String>, Vec<String>) {
    let mut inactive = Vec::new();
    let mut logs = Vec::new();
    let ctx = json!({
        "output": output,
        "input": node_input,
        "node": { "id": node.id, "kind": node.kind, "name": node.name },
        "run": run,
    });
    for e in outgoing_edges(graph, &node.id) {
        if let Some(cond) = &e.condition {
            if !otto_core::expr::eval_bool(cond, &ctx) {
                inactive.push(e.id.clone());
                logs.push(format!("edge → {} not taken ({cond})", e.target));
            }
        }
    }
    (inactive, logs)
}

/// Reconstruct adopted branch decisions without rerunning their nodes: for
/// each adopted node with an output, re-evaluate its out-edges (against
/// today's `run` data, so a retry sees the same registry, DR-013) and add
/// the untaken ones to `inactive_edges`; an adopted Skipped node below an
/// errored source is poison and joins `errored`.
pub fn reconstruct_branches(
    order: &[String],
    graph: &WorkflowGraph,
    run: &Value,
    states: &[NodeRunState],
    outputs: &HashMap<String, Value>,
    inactive_edges: &mut HashSet<String>,
    errored: &mut HashSet<String>,
) {
    let run_input = run.get("input").cloned().unwrap_or(Value::Null);
    for id in order {
        let Some(node) = graph.nodes.iter().find(|n| &n.id == id) else {
            continue;
        };
        if let Some(output) = outputs.get(id) {
            let sources: Vec<String> = incoming_edges(graph, id)
                .iter()
                .filter(|e| outputs.contains_key(&e.source) && !inactive_edges.contains(&e.id))
                .map(|e| e.source.clone())
                .collect();
            let adopted_input = assemble_input(&sources, outputs, &run_input);
            let (pruned, _) = eval_outgoing(graph, node, output, &adopted_input, run);
            inactive_edges.extend(pruned);
        } else if states
            .iter()
            .any(|s| &s.node_id == id && s.status == NodeStatus::Skipped)
            && incoming_edges(graph, id)
                .iter()
                .any(|e| errored.contains(&e.source))
        {
            // Skipped below an errored source is poison; otherwise the
            // branch simply wasn't taken.
            errored.insert(id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn node(id: &str, kind: &str) -> WorkflowNode {
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

    pub(crate) fn edge(source: &str, target: &str) -> WorkflowEdge {
        WorkflowEdge {
            id: format!("{source}-{target}"),
            source: source.into(),
            target: target.into(),
            condition: None,
        }
    }

    fn st(id: &str, status: NodeStatus) -> NodeRunState {
        NodeRunState {
            node_id: id.into(),
            status,
            ..Default::default()
        }
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
    fn topo_is_breadth_first_in_graph_then_edge_order() {
        // A1→A2→A3 beside B1→B2 (engine.md §3.2's example): FIFO Kahn.
        let g = WorkflowGraph {
            nodes: ["s", "a1", "b1", "a2", "b2", "a3"]
                .map(|i| node(i, "log"))
                .to_vec(),
            edges: vec![
                edge("s", "a1"),
                edge("s", "b1"),
                edge("a1", "a2"),
                edge("b1", "b2"),
                edge("a2", "a3"),
            ],
        };
        assert_eq!(topo_order(&g).unwrap(), ["s", "a1", "b1", "a2", "b2", "a3"]);
    }

    #[test]
    fn topo_detects_cycle_and_ignores_dangling_edges() {
        let g = WorkflowGraph {
            nodes: vec![node("a", "log"), node("b", "log")],
            edges: vec![edge("a", "b"), edge("b", "a")],
        };
        assert!(topo_order(&g).is_err());
        let g = WorkflowGraph {
            nodes: vec![node("a", "log")],
            edges: vec![edge("a", "gone")],
        };
        assert_eq!(topo_order(&g).unwrap(), ["a"]);
    }

    #[test]
    fn descendants_scope_is_self_plus_downstream_and_ancestors_the_reverse() {
        let g = WorkflowGraph {
            nodes: ["a", "b", "c", "d"].map(|i| node(i, "log")).to_vec(),
            edges: vec![edge("a", "b"), edge("b", "c"), edge("a", "d")],
        };
        let set = descendants_inclusive(&g, "b");
        assert!(set.contains("b") && set.contains("c"), "self + downstream");
        assert!(
            !set.contains("a") && !set.contains("d"),
            "not upstream/siblings"
        );
        let up = ancestors(&g, "c");
        assert_eq!(up, HashSet::from(["a".to_string(), "b".to_string()]));
        assert!(ancestors(&g, "a").is_empty());
    }

    #[test]
    fn retry_consumer_receives_adopted_output_but_fresh_partial_run_uses_input() {
        let scope = HashSet::from(["consumer".to_string()]);
        let prior = vec![st("fetch", NodeStatus::Success)];
        let outputs = HashMap::from([("fetch".to_string(), json!({"fetched":42}))]);
        assert!(dependency_is_relevant("fetch", Some(&scope), Some(&prior)));
        assert!(!dependency_is_relevant("fetch", Some(&scope), None));
        assert!(dependency_is_relevant("fetch", None, None));
        assert_eq!(
            assemble_input(&["fetch".into()], &outputs, &json!({"seed":1})),
            json!({"fetched":42})
        );
        assert_eq!(
            assemble_input(&[], &outputs, &json!({"seed":1})),
            json!({"seed":1})
        );
        let outputs = HashMap::from([("a".to_string(), json!(1)), ("b".to_string(), json!(2))]);
        assert_eq!(
            assemble_input(
                &["a".into(), "b".into(), "gone".into()],
                &outputs,
                &Value::Null
            ),
            json!({"a":1, "b":2}),
            "several sources key by id; one without output drops out"
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
        let v = vec![view("a", false, true, true), view("b", false, true, false)];
        assert_eq!(decide_node(&v), NodeDecision::Run(vec!["a".into()]));
        let v = vec![view("a", false, true, false), view("b", false, true, true)];
        assert_eq!(decide_node(&v), NodeDecision::Run(vec!["b".into()]));
    }

    #[test]
    fn edge_views_keep_only_relevant_sources() {
        let g = WorkflowGraph {
            nodes: ["a", "b", "c"].map(|i| node(i, "log")).to_vec(),
            edges: vec![edge("a", "c"), edge("b", "c")],
        };
        let errored = HashSet::from(["b".to_string()]);
        let inactive = HashSet::from(["a-c".to_string()]);
        let v = edge_views(&g, "c", |s| s == "a", &errored, |_| true, &inactive);
        assert_eq!(v, vec![view("a", false, true, false)]);
    }

    fn branch_graph() -> WorkflowGraph {
        let cond = |id: &str, target: &str, c: &str| WorkflowEdge {
            id: id.into(),
            source: "c".into(),
            target: target.into(),
            condition: Some(c.into()),
        };
        WorkflowGraph {
            nodes: vec![node("c", "condition"), node("t", "log"), node("f", "log")],
            edges: vec![
                cond("c-t", "t", "output.result == true"),
                cond("c-f", "f", "output.result == false"),
            ],
        }
    }

    #[test]
    fn eval_outgoing_prunes_false_edges() {
        let mut g = branch_graph();
        let cnode = g.nodes[0].clone();
        let run = run_data(&Value::Null, &[], &[]);
        let (inactive, logs) =
            eval_outgoing(&g, &cnode, &json!({ "result": true }), &Value::Null, &run);
        assert_eq!(inactive, vec!["c-f".to_string()], "false branch pruned");
        assert_eq!(logs, ["edge → f not taken (output.result == false)"]);
        let (inactive, _) =
            eval_outgoing(&g, &cnode, &json!({ "result": false }), &Value::Null, &run);
        assert_eq!(inactive, vec!["c-t".to_string()]);
        g.edges.clear();
        let (inactive, _) = eval_outgoing(&g, &cnode, &json!({}), &Value::Null, &run);
        assert!(inactive.is_empty(), "no edges → nothing pruned");
    }

    fn repo(name: &str) -> RepoEntry {
        RepoEntry {
            repo: name.into(),
            kind: "worktree".into(),
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn edge_conditions_see_run_repos_prs_and_input() {
        let mut g = branch_graph();
        g.edges[0].condition = Some("len(run.repos) > 1".into());
        g.edges[1].condition = Some("run.input.mode == \"x\" && len(run.prs) == 0".into());
        let cnode = g.nodes[0].clone();
        let one = run_data(&json!({"mode": "x"}), &[repo("a")], &[]);
        let (inactive, _) = eval_outgoing(&g, &cnode, &json!({}), &Value::Null, &one);
        assert_eq!(
            inactive,
            ["c-t"],
            "one repo: the cross-repo edge is not taken"
        );
        let two = run_data(&json!({"mode": "y"}), &[repo("a"), repo("b")], &[]);
        let (inactive, _) = eval_outgoing(&g, &cnode, &json!({}), &Value::Null, &two);
        assert_eq!(inactive, ["c-f"]);
    }

    #[test]
    fn condition_expr_keeps_its_root_and_gains_run_unless_input_has_one() {
        let run = run_data(&json!({"k": 1}), &[repo("a"), repo("b")], &[]);
        let root = condition_root(&json!({"passed": true}), &run);
        assert!(
            otto_core::expr::eval_bool("passed == true", &root),
            "DR-005"
        );
        assert!(otto_core::expr::eval_bool("len(run.repos) == 2", &root));
        assert!(otto_core::expr::eval_bool("run.input.k == 1", &root));
        let own = condition_root(&json!({"run": {"repos": []}}), &run);
        assert!(
            otto_core::expr::eval_bool("len(run.repos) == 0", &own),
            "an input's own run key wins"
        );
        assert_eq!(condition_root(&json!(5), &run), json!(5));
    }

    #[test]
    fn reconstruct_branches_restores_pruned_edges_and_poison() {
        let mut g = branch_graph();
        g.nodes.push(node("x", "log"));
        g.nodes.push(node("y", "log"));
        g.edges.push(edge("x", "y"));
        let order = topo_order(&g).unwrap();
        let states = vec![
            st("c", NodeStatus::Success),
            st("t", NodeStatus::Pending),
            st("f", NodeStatus::Skipped),
            st("x", NodeStatus::Error),
            st("y", NodeStatus::Skipped),
        ];
        let outputs = HashMap::from([("c".to_string(), json!({"result": true}))]);
        let mut inactive = HashSet::new();
        let mut errored = HashSet::from(["x".to_string()]);
        let run = run_data(&Value::Null, &[], &[]);
        reconstruct_branches(
            &order,
            &g,
            &run,
            &states,
            &outputs,
            &mut inactive,
            &mut errored,
        );
        assert_eq!(inactive, HashSet::from(["c-f".to_string()]));
        assert!(errored.contains("y"), "skipped below an error is poison");
        assert!(!errored.contains("f"), "an untaken branch is not");
    }
}
