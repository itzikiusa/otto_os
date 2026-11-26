//! The numbering proof: the start rule gives every node the serial engine's
//! decision and step number — on random graphs (with failures, branch and
//! chat skips, adoption on restart and retry) and on the five exported
//! graphs against the Phase-0 goldens.

use std::collections::{HashMap, HashSet};

use otto_core::workflows::{NodeRunState, NodeStatus, WorkflowEdge, WorkflowGraph, WorkflowNode};
use serde_json::{json, Value};

use super::*;
use crate::graph::{
    assemble_input, decide_node, descendants_inclusive, edge_views, eval_outgoing,
    reconstruct_branches, run_data, topo_order,
};

/// xorshift64* — deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() % 10_000) as f64 / 10_000.0 < p
    }
}

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

/// How a node that runs ends, in the model.
#[derive(Debug, Clone)]
enum Fate {
    Ok(Value),
    /// An output carrying an outcome error (`outcome_error`).
    FailWith(Value),
    Fail,
    /// Skipped through chat while running: marker output.
    ChatSkip,
}

/// The node's status, output and scheduler outcome for a fate — the same
/// settle rules for the serial model and the scheduler.
fn apply(
    graph: &WorkflowGraph,
    n: &WorkflowNode,
    fate: &Fate,
    input: &Value,
) -> (NodeStatus, Option<Value>, Outcome) {
    let run = run_data(&Value::Null, &[], &[]);
    let pruned = |out: &Value| eval_outgoing(graph, n, out, input, &run).0;
    match fate {
        Fate::Ok(out) => (
            NodeStatus::Success,
            Some(out.clone()),
            Outcome {
                errored: false,
                has_output: true,
                inactive_edges: pruned(out),
            },
        ),
        Fate::FailWith(out) => (
            NodeStatus::Error,
            Some(out.clone()),
            Outcome {
                errored: true,
                has_output: true,
                inactive_edges: pruned(out),
            },
        ),
        Fate::Fail => (
            NodeStatus::Error,
            None,
            Outcome {
                errored: true,
                has_output: false,
                inactive_edges: vec![],
            },
        ),
        Fate::ChatSkip => (
            NodeStatus::Skipped,
            Some(json!({ "skipped": true, "note": "skipped via chat command" })),
            Outcome {
                errored: false,
                has_output: true,
                inactive_edges: chat_skip_inactive_edges(graph, &n.id),
            },
        ),
    }
}

struct Serial {
    numbers: HashMap<String, u32>,
    states: Vec<NodeRunState>,
}

/// Today's serial engine (run/schedule.rs `step`), one node at a time in
/// run order — the reference the scheduler must equal.
fn serial(graph: &WorkflowGraph, fates: &HashMap<String, Fate>, run_input: &Value) -> Serial {
    let mut states: Vec<NodeRunState> = graph
        .nodes
        .iter()
        .map(|n| NodeRunState {
            node_id: n.id.clone(),
            ..Default::default()
        })
        .collect();
    let (mut outputs, mut errored, mut inactive) = (HashMap::new(), HashSet::new(), HashSet::new());
    let mut numbers = HashMap::new();
    let mut counter = 0;
    for id in topo_order(graph).unwrap() {
        let idx = graph.nodes.iter().position(|n| n.id == id).unwrap();
        let views = edge_views(
            graph,
            &id,
            |_| true,
            &errored,
            |s| outputs.contains_key(s),
            &inactive,
        );
        match decide_node(&views) {
            NodeDecision::ErrorSkip => {
                errored.insert(id.clone());
                states[idx].status = NodeStatus::Skipped;
            }
            NodeDecision::BranchSkip => states[idx].status = NodeStatus::Skipped,
            NodeDecision::Run(sources) => {
                counter += 1;
                numbers.insert(id.clone(), counter);
                let input = assemble_input(&sources, &outputs, run_input);
                let (status, output, outcome) =
                    apply(graph, &graph.nodes[idx], &fates[&id], &input);
                let st = &mut states[idx];
                st.status = status;
                st.step_no = Some(counter);
                st.started_at = Some(chrono::Utc::now());
                st.output = output.clone();
                if let Some(o) = output {
                    outputs.insert(id.clone(), o);
                }
                if outcome.errored {
                    errored.insert(id.clone());
                }
                inactive.extend(outcome.inactive_edges);
            }
        }
    }
    Serial { numbers, states }
}

/// Where a scheduled run starts from (defaults: a fresh run).
#[derive(Default)]
struct Begin<'a> {
    states: Option<Vec<NodeRunState>>,
    run_set: Option<HashSet<String>>,
    prior: Option<&'a [NodeRunState]>,
    continue_unfinished: bool,
}

/// Drive the scheduler like the engine does, settling a random running node
/// each time; returns the step number of every node that ran or was adopted.
fn drive(
    graph: &WorkflowGraph,
    fates: &HashMap<String, Fate>,
    run_input: &Value,
    from: Begin<'_>,
    cap: usize,
    rng: &mut Rng,
) -> HashMap<String, u32> {
    let order = topo_order(graph).unwrap();
    let states = from.states.unwrap_or_else(|| {
        graph
            .nodes
            .iter()
            .map(|n| NodeRunState {
                node_id: n.id.clone(),
                ..Default::default()
            })
            .collect()
    });
    // adopt_prior + reconstruct_branches, as the engine does before the walk.
    let mut outputs: HashMap<String, Value> = states
        .iter()
        .filter(|s| s.status != NodeStatus::Pending)
        .filter_map(|s| s.output.clone().map(|o| (s.node_id.clone(), o)))
        .collect();
    let mut errored: HashSet<String> = states
        .iter()
        .filter(|s| s.status == NodeStatus::Error)
        .map(|s| s.node_id.clone())
        .collect();
    let mut inactive = HashSet::new();
    let run = run_data(run_input, &[], &[]);
    reconstruct_branches(
        &order,
        graph,
        &run,
        &states,
        &outputs,
        &mut inactive,
        &mut errored,
    );
    let has_output: HashSet<String> = outputs.keys().cloned().collect();
    let mut sched = Scheduler::new(
        graph,
        &order,
        Init {
            states: &states,
            has_output: &has_output,
            errored: &errored,
            inactive_edges: &inactive,
            run_set: from.run_set.as_ref(),
            prior: from.prior,
            continue_unfinished: from.continue_unfinished,
        },
        cap,
    );
    let mut numbers = HashMap::new();
    let mut queued: Option<(usize, u32)> = None;
    let mut running: Vec<(usize, Value)> = Vec::new();
    loop {
        for start in sched.startable() {
            match start {
                Start::Adopt { idx, step_no } => {
                    if let Some(n) = step_no {
                        numbers.insert(graph.nodes[idx].id.clone(), n);
                    }
                }
                Start::Skip { .. } => {}
                Start::Queue { idx, step_no, .. } => {
                    assert!(queued.is_none(), "one node waits for a slot at a time");
                    assert!(takes_step_slot(&graph.nodes[idx]));
                    queued = Some((idx, step_no));
                }
                Start::Run {
                    idx,
                    step_no,
                    sources,
                } => {
                    if let Some(q) = queued.take_if(|q| q.0 == idx) {
                        assert_eq!(q.1, step_no, "a held node keeps its number");
                    }
                    numbers.insert(graph.nodes[idx].id.clone(), step_no);
                    running.push((idx, assemble_input(&sources, &outputs, run_input)));
                }
            }
        }
        let slotted = running
            .iter()
            .filter(|(i, _)| takes_step_slot(&graph.nodes[*i]))
            .count();
        assert!(
            slotted <= cap.max(1),
            "{slotted} slotted steps beside a cap of {cap}"
        );
        assert_eq!(sched.slots().0, slotted);
        if running.is_empty() {
            assert!(
                sched.is_finished(),
                "nothing runs, so every node is decided"
            );
            return numbers;
        }
        assert!(!sched.is_finished());
        let (idx, input) = running.swap_remove(rng.below(running.len()));
        let n = &graph.nodes[idx];
        let (_, output, outcome) = apply(graph, n, &fates[&n.id], &input);
        if let Some(o) = output {
            outputs.insert(n.id.clone(), o);
        }
        sched.settle(idx, outcome);
    }
}

/// A random DAG of 3–16 nodes, edge density 0.08–0.3, about a third of the
/// edges conditional, nodes listed in a shuffled graph order; and a fate per
/// node (failures, chat skips, the branch each condition takes).
fn random_case(rng: &mut Rng) -> (WorkflowGraph, HashMap<String, Fate>) {
    let n = 3 + rng.below(14);
    let density = 0.08 + rng.below(23) as f64 / 100.0;
    let ids: Vec<String> = (0..n).map(|i| format!("n{i}")).collect();
    let mut edges = Vec::new();
    for i in 0..n {
        for j in i + 1..n {
            if rng.chance(density) {
                let mut e = edge(&ids[i], &ids[j]);
                if rng.chance(0.33) {
                    e.condition = Some(format!("output.{} == true", e.id.replace('-', "_")));
                }
                edges.push(e);
            }
        }
    }
    let mut nodes: Vec<WorkflowNode> = ids
        .iter()
        .map(|id| {
            node(
                id,
                if rng.chance(0.5) {
                    "agent_prompt"
                } else {
                    "log"
                },
            )
        })
        .collect();
    for i in (1..nodes.len()).rev() {
        nodes.swap(i, rng.below(i + 1));
    }
    let mut fates = HashMap::new();
    for id in &ids {
        let flags: serde_json::Map<String, Value> = edges
            .iter()
            .filter(|e| &e.source == id)
            .map(|e| (e.id.replace('-', "_"), Value::Bool(rng.chance(0.6))))
            .collect();
        let out = Value::Object(flags);
        let fate = match rng.below(100) {
            0..=5 => Fate::Fail,
            6..=9 => Fate::FailWith(out),
            10..=19 => Fate::ChatSkip,
            _ => Fate::Ok(out),
        };
        fates.insert(id.clone(), fate);
    }
    (WorkflowGraph { nodes, edges }, fates)
}

#[test]
fn start_rule_numbering_equals_serial() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for case in 0..7_000 {
        let (graph, fates) = random_case(&mut rng);
        let input = json!({"seed": case});
        let reference = serial(&graph, &fates, &input);
        let cap = 1 + rng.below(4);

        // A fresh run, any finish order and step-slot cap.
        let got = drive(&graph, &fates, &input, Begin::default(), cap, &mut rng);
        assert_eq!(
            got, reference.numbers,
            "case {case}: fresh run, cap {cap}: {graph:?}"
        );

        // A restart after the first p nodes of run order settled: the rest
        // re-runs and every node keeps its number — on new rows (step_no) and
        // on old rows (none; a chat-skipped node counts by started_at).
        let order = topo_order(&graph).unwrap();
        let p = rng.below(order.len() + 1);
        let old_rows = rng.chance(0.5);
        let states: Vec<NodeRunState> = reference
            .states
            .iter()
            .map(|s| {
                if order[..p].contains(&s.node_id) {
                    let mut s = s.clone();
                    if old_rows {
                        s.step_no = None;
                    }
                    s
                } else {
                    NodeRunState {
                        node_id: s.node_id.clone(),
                        ..Default::default()
                    }
                }
            })
            .collect();
        let restart = Begin {
            states: Some(states.clone()),
            prior: Some(&states),
            continue_unfinished: true,
            ..Default::default()
        };
        let got = drive(&graph, &fates, &input, restart, cap, &mut rng);
        assert_eq!(
            got, reference.numbers,
            "case {case}: restart after {p} (old rows: {old_rows}): {graph:?}"
        );

        // Retry from a node: it and its descendants re-run, the rest is adopted.
        let start = &graph.nodes[rng.below(graph.nodes.len())].id;
        let run_set = descendants_inclusive(&graph, start);
        let states: Vec<NodeRunState> = reference
            .states
            .iter()
            .map(|s| {
                if run_set.contains(&s.node_id) {
                    NodeRunState {
                        node_id: s.node_id.clone(),
                        ..Default::default()
                    }
                } else {
                    let mut s = s.clone();
                    if old_rows {
                        s.step_no = None;
                    }
                    s
                }
            })
            .collect();
        let retry = Begin {
            states: Some(states),
            run_set: Some(run_set),
            prior: Some(&reference.states),
            continue_unfinished: false,
        };
        let got = drive(&graph, &fates, &input, retry, cap, &mut rng);
        assert_eq!(
            got, reference.numbers,
            "case {case}: retry from {start}: {graph:?}"
        );
    }
}

/// The five exported graphs (scrubbed clones) with the outcomes the Phase-0
/// goldens recorded: the scheduler numbers every step as today's engine did,
/// whatever the finish order and the slot cap.
#[test]
fn clones_keep_the_phase0_step_numbers() {
    let clones = [
        (
            "a",
            include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/a.json"),
        ),
        (
            "b",
            include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/b.json"),
        ),
        (
            "c",
            include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/c.json"),
        ),
        (
            "d",
            include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/d.json"),
        ),
        (
            "e",
            include_str!("../../../otto-server/tests/fixtures/workflows/compat/clones/e.json"),
        ),
    ];
    let goldens = [
        ("a", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/a/passed.old.json")),
        ("a", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/a/failed.old.json")),
        ("b", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/b/passed.old.json")),
        ("b", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/b/failed.old.json")),
        ("c", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/c/passed.old.json")),
        ("c", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/c/failed.old.json")),
        ("d", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/d/passed.old.json")),
        ("d", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/d/failed.old.json")),
        ("e", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/e/no-ticket.old.json")),
        ("e", include_str!("../../../otto-server/tests/fixtures/workflows/compat/golden/e/ticket-unreadable.old.json")),
    ];
    let mut rng = Rng(42);
    for (clone, text) in goldens {
        let golden: Value = serde_json::from_str(text).unwrap();
        let def: Value =
            serde_json::from_str(clones.iter().find(|c| c.0 == clone).unwrap().1).unwrap();
        let graph: WorkflowGraph = serde_json::from_value(def["graph"].clone()).unwrap();
        let scenario = golden["scenario"].as_str().unwrap();
        let expected: HashMap<String, u32> = golden["step_numbers"]
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(id, n)| n.as_u64().map(|n| (id.clone(), n as u32)))
            .collect();
        let mut fates = HashMap::new();
        for n in golden["nodes"].as_array().unwrap() {
            let output = n.get("output").cloned().filter(|o| !o.is_null());
            let fate = match (n["status"].as_str().unwrap(), output) {
                ("success", out) => Fate::Ok(out.unwrap_or(Value::Null)),
                ("error", Some(out)) => Fate::FailWith(out),
                ("error", None) => Fate::Fail,
                // Never ran: if the scheduler runs it, the numbers differ.
                _ => Fate::Ok(Value::Null),
            };
            fates.insert(n["id"].as_str().unwrap().to_string(), fate);
        }
        let input = golden["nodes"][0]["output"].clone();
        assert_eq!(
            serial(&graph, &fates, &input).numbers,
            expected,
            "{scenario}: serial model"
        );
        for cap in 1..=4 {
            for _ in 0..25 {
                let got = drive(&graph, &fates, &input, Begin::default(), cap, &mut rng);
                assert_eq!(got, expected, "{scenario}: cap {cap}");
            }
        }
    }
}

fn graph_of(nodes: &[(&str, &str)], edges: &[(&str, &str)]) -> WorkflowGraph {
    WorkflowGraph {
        nodes: nodes.iter().map(|(id, k)| node(id, k)).collect(),
        edges: edges.iter().map(|(a, b)| edge(a, b)).collect(),
    }
}

fn fresh(graph: &WorkflowGraph, cap: usize) -> Scheduler {
    let states: Vec<NodeRunState> = graph
        .nodes
        .iter()
        .map(|n| NodeRunState {
            node_id: n.id.clone(),
            ..Default::default()
        })
        .collect();
    let none = HashSet::new();
    Scheduler::new(
        graph,
        &topo_order(graph).unwrap(),
        Init {
            states: &states,
            has_output: &none,
            errored: &none,
            inactive_edges: &none,
            run_set: None,
            prior: None,
            continue_unfinished: false,
        },
        cap,
    )
}

fn ok() -> Outcome {
    Outcome {
        errored: false,
        has_output: true,
        inactive_edges: vec![],
    }
}

fn runs(starts: &[Start]) -> Vec<(usize, u32)> {
    starts
        .iter()
        .filter_map(|s| match s {
            Start::Run { idx, step_no, .. } => Some((*idx, *step_no)),
            _ => None,
        })
        .collect()
}

/// A1→A2→A3 beside B1→B2 (order s, A1, B1, A2, B2, A3): B2 is decided only
/// after A2 was, and A2 only after A1 finished — so B2 waits for A1 to finish
/// even when B1 finished long before (DR-013 item 3).
#[test]
fn a_ready_node_waits_for_the_earlier_node_in_run_order() {
    let g = graph_of(
        &[
            ("s", "log"),
            ("a1", "log"),
            ("b1", "log"),
            ("a2", "log"),
            ("b2", "log"),
            ("a3", "log"),
        ],
        &[
            ("s", "a1"),
            ("s", "b1"),
            ("a1", "a2"),
            ("b1", "b2"),
            ("a2", "a3"),
        ],
    );
    let mut s = fresh(&g, 4);
    assert_eq!(runs(&s.startable()), [(0, 1)]);
    s.settle(0, ok());
    assert_eq!(
        runs(&s.startable()),
        [(1, 2), (2, 3)],
        "the fork runs side by side"
    );
    s.settle(2, ok()); // b1 finishes first
    assert!(
        s.startable().is_empty(),
        "b2 waits: a2 is earlier in run order"
    );
    assert_eq!(
        s.run_order_wait(),
        Some((3, vec![4])),
        "b2 waits for a2 to start"
    );
    s.settle(1, ok());
    assert_eq!(runs(&s.startable()), [(3, 4), (4, 5)]);
    s.settle(4, ok());
    s.settle(3, ok());
    assert_eq!(runs(&s.startable()), [(5, 6)]);
    s.settle(5, ok());
    assert!(s.startable().is_empty() && s.is_finished());
}

#[test]
fn a_full_slot_numbers_the_node_holds_it_and_stops_the_walk() {
    let g = graph_of(
        &[
            ("s", "log"),
            ("a", "agent_prompt"),
            ("b", "agent_prompt"),
            ("c", "log"),
        ],
        &[("s", "a"), ("s", "b"), ("s", "c")],
    );
    let mut s = fresh(&g, 1);
    s.startable();
    s.settle(0, ok());
    let starts = s.startable();
    assert_eq!(
        starts,
        [
            Start::Run {
                idx: 1,
                step_no: 2,
                sources: vec!["s".into()]
            },
            Start::Queue {
                idx: 2,
                step_no: 3,
                sources: vec!["s".into()]
            },
        ],
        "c (pure, later in run order) does not overtake the held b"
    );
    assert_eq!(s.slots(), (1, 1));
    assert_eq!(s.run_order_wait(), Some((2, vec![3])));
    s.settle(1, ok());
    assert_eq!(
        s.startable(),
        [
            Start::Run {
                idx: 2,
                step_no: 3,
                sources: vec!["s".into()]
            },
            Start::Run {
                idx: 3,
                step_no: 4,
                sources: vec!["s".into()]
            },
        ]
    );
    assert_eq!(s.slots(), (1, 1), "pure kinds take no slot");
}

#[test]
fn skips_are_decided_at_once_and_poison_propagates() {
    let mut g = graph_of(
        &[
            ("s", "log"),
            ("bad", "log"),
            ("t", "log"),
            ("f", "log"),
            ("after_bad", "log"),
        ],
        &[("s", "t"), ("s", "f"), ("s", "bad"), ("bad", "after_bad")],
    );
    g.edges[1].condition = Some("output.go == true".into());
    let mut s = fresh(&g, 4);
    s.startable();
    s.settle(
        0,
        Outcome {
            errored: false,
            has_output: true,
            inactive_edges: vec!["s-f".into()],
        },
    );
    let starts = s.startable();
    assert!(
        starts.contains(&Start::Skip {
            idx: 3,
            why: SkipWhy::Branch
        }),
        "{starts:?}"
    );
    // Run order s, t, f, bad, after_bad (Kahn over the edges' order).
    assert_eq!(runs(&starts), [(2, 2), (1, 3)]);
    s.settle(
        1,
        Outcome {
            errored: true,
            has_output: false,
            inactive_edges: vec![],
        },
    );
    assert_eq!(
        s.startable(),
        [Start::Skip {
            idx: 4,
            why: SkipWhy::Upstream
        }]
    );
    assert_eq!(
        SkipWhy::Upstream.log_line(),
        "skipped (upstream did not succeed)"
    );
    s.settle(2, ok());
    assert!(s.is_finished());
}

#[test]
fn adoption_consumes_persisted_numbers_and_chat_skips_on_old_rows() {
    let g = graph_of(
        &[
            ("a", "log"),
            ("skipped", "agent_prompt"),
            ("branch", "log"),
            ("c", "log"),
        ],
        &[("a", "skipped"), ("a", "branch"), ("skipped", "c")],
    );
    let at = Some(chrono::Utc::now());
    let st = |id: &str, status, step_no, started_at| NodeRunState {
        node_id: id.into(),
        status,
        step_no,
        started_at,
        ..Default::default()
    };
    // Old rows: no step_no. The chat-skipped node started, so it consumes a
    // number; the branch-skipped one never started and consumes none.
    let states = vec![
        st("a", NodeStatus::Success, None, at),
        st("skipped", NodeStatus::Skipped, None, at),
        st("branch", NodeStatus::Skipped, None, None),
        st("c", NodeStatus::Pending, None, None),
    ];
    let outputs = HashSet::from(["a".to_string(), "skipped".to_string()]);
    let none = HashSet::new();
    let mut s = Scheduler::new(
        &g,
        &topo_order(&g).unwrap(),
        Init {
            states: &states,
            has_output: &outputs,
            errored: &none,
            inactive_edges: &none,
            run_set: None,
            prior: Some(&states),
            continue_unfinished: true,
        },
        4,
    );
    assert_eq!(
        s.startable(),
        [
            Start::Adopt {
                idx: 0,
                step_no: Some(1)
            },
            Start::Adopt {
                idx: 1,
                step_no: Some(2)
            },
            Start::Adopt {
                idx: 2,
                step_no: None
            },
            Start::Run {
                idx: 3,
                step_no: 3,
                sources: vec!["skipped".into()]
            },
        ]
    );
    // New rows: the persisted number wins.
    let states = vec![
        st("a", NodeStatus::Success, Some(1), at),
        st("skipped", NodeStatus::Skipped, Some(2), at),
        st("branch", NodeStatus::Skipped, None, None),
        st("c", NodeStatus::Pending, None, None),
    ];
    let mut s = Scheduler::new(
        &g,
        &topo_order(&g).unwrap(),
        Init {
            states: &states,
            has_output: &outputs,
            errored: &none,
            inactive_edges: &none,
            run_set: None,
            prior: Some(&states),
            continue_unfinished: true,
        },
        4,
    );
    assert_eq!(runs(&s.startable()), [(3, 3)]);
}

#[test]
fn outside_the_run_scope_is_skipped_and_out_of_scope_edges_do_not_constrain() {
    let g = graph_of(
        &[("a", "log"), ("b", "log"), ("c", "log")],
        &[("a", "b"), ("b", "c")],
    );
    let states: Vec<NodeRunState> = g
        .nodes
        .iter()
        .map(|n| NodeRunState {
            node_id: n.id.clone(),
            ..Default::default()
        })
        .collect();
    let run_set = HashSet::from(["b".to_string(), "c".to_string()]);
    let none = HashSet::new();
    let mut s = Scheduler::new(
        &g,
        &topo_order(&g).unwrap(),
        Init {
            states: &states,
            has_output: &none,
            errored: &none,
            inactive_edges: &none,
            run_set: Some(&run_set),
            prior: None,
            continue_unfinished: false,
        },
        4,
    );
    assert_eq!(
        s.startable(),
        [
            Start::Skip {
                idx: 0,
                why: SkipWhy::OutsideScope
            },
            Start::Run {
                idx: 1,
                step_no: 1,
                sources: vec![]
            },
        ],
        "a start-from-here entry falls back to the run input"
    );
}

#[test]
fn slot_kinds() {
    for k in [
        "agent_prompt",
        "review_run",
        "script",
        "git_checkout",
        "post_review_comments",
        "loop",
        "check",
        "canvas",
        "git_pr",
        "product_plan",
    ] {
        assert!(takes_step_slot(&node("x", k)), "{k}");
    }
    for k in [
        "manual_trigger",
        "log",
        "condition",
        "transform",
        "channel_notify",
        "delay",
        "human_approval",
    ] {
        assert!(!takes_step_slot(&node("x", k)), "{k}");
    }
    let mut prep = node("p", "prepare_context");
    assert!(!takes_step_slot(&prep), "a ticket fetch alone is pure");
    prep.params = json!({"prompt": "analyse"});
    assert!(takes_step_slot(&prep));
}
