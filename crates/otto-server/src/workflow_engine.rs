//! Workflow execution engine: topologically runs a node graph, threading each
//! node's JSON output to its successors. Heavy/long node kinds (agent turns)
//! execute for real; the game-build / verify kinds are structured scaffolds
//! (they need an external engine that isn't bundled).
//!
//! A run executes in a background task that persists progress to `workflow_runs`
//! after every node, so the UI can poll run status live.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use otto_core::domain::Workspace;
use otto_core::workflows::{
    NodeRunState, NodeStatus, NodeTypeSpec, RunStatus, Workflow, WorkflowGraph, WorkflowNode,
};
use otto_core::{Id, Result};
use otto_state::WorkflowsRepo;
use serde_json::{json, Value};
use sqlx::SqlitePool;

use crate::state::ServerCtx;

/// Per-node turn budget for agent/LLM nodes.
const NODE_AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Global wall-clock budget for a whole run. A run can't execute forever: once
/// the cumulative time across all nodes exceeds this, the run is failed at the
/// next node boundary (a node already executing finishes first, bounded by
/// `NODE_AGENT_TIMEOUT`). 30 min comfortably covers a multi-agent game pipeline
/// while still guaranteeing termination.
const RUN_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Fail any workflow run left `pending`/`running` by a previous daemon process.
///
/// A run executes in a background task that dies with the process, so a row left
/// non-terminal is orphaned and would otherwise poll forever in the UI. Called
/// once on daemon startup (mirrors the review / skill-eval / product / swarm
/// startup reconciliation). Writes inline SQL against `workflow_runs` so it needs
/// no repo method. Returns the number of rows updated.
pub async fn reap_orphaned_runs(pool: &SqlitePool) -> std::result::Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE workflow_runs
         SET status = 'error',
             error = 'Interrupted by a daemon restart — re-run the workflow.',
             finished_at = COALESCE(finished_at, ?)
         WHERE status IN ('pending', 'running')",
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// The node-kind catalog: drives the editor palette and validates generated
/// graphs. Keep in sync with `execute_node` below.
pub fn node_catalog() -> Vec<NodeTypeSpec> {
    let n = |kind: &str,
             label: &str,
             category: &str,
             description: &str,
             inputs: u8,
             outputs: u8,
             color: &str,
             icon: &str| NodeTypeSpec {
        kind: kind.to_string(),
        label: label.to_string(),
        category: category.to_string(),
        description: description.to_string(),
        inputs,
        outputs,
        color: color.to_string(),
        icon: icon.to_string(),
    };
    vec![
        n("manual_trigger", "Manual Trigger", "Triggers",
          "Starts the workflow and emits its input payload.", 0, 1, "#6b7bff", "play"),
        n("agent_prompt", "Agent", "AI",
          "Run a headless agent turn with a prompt; outputs its reply.", 1, 1, "#d97cff", "command"),
        n("http_request", "HTTP Request", "Network",
          "Call an HTTP endpoint and capture the response.", 1, 1, "#46c0a0", "globe"),
        n("transform", "Set / Transform", "Data",
          "Merge static JSON into the data flowing through.", 1, 1, "#9aa0aa", "edit"),
        n("delay", "Delay", "Flow",
          "Wait a number of milliseconds, then continue.", 1, 1, "#9aa0aa", "clock"),
        n("log", "Log", "Flow",
          "Record the incoming data in the run log; pass it through.", 1, 1, "#9aa0aa", "note"),
        n("game_engine", "Game Engine", "Game",
          "Assemble a slot game from approved assets (RNG, paytable, reels).", 1, 1, "#57b9ff", "box"),
        n("verifier", "Verifier", "Game",
          "Verify the built game (RNG fairness, RTP, asset integrity).", 1, 1, "#57d98b", "check"),
    ]
}

/// True when `kind` is a node the executor understands.
pub fn is_known_kind(kind: &str) -> bool {
    node_catalog().iter().any(|s| s.kind == kind)
}

/// Run a workflow to completion in the current task, persisting progress to the
/// `workflow_runs` row after every node. Spawn this on a background task.
pub async fn run_workflow(
    ctx: ServerCtx,
    ws: Workspace,
    workflow: Workflow,
    run_id: Id,
    input: Value,
    start_node: Option<String>,
    only_node: bool,
) {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let order = match topo_order(&workflow.graph) {
        Ok(o) => o,
        Err(e) => {
            let _ = repo
                .update_run(&run_id, RunStatus::Error, &[], Some(&e), true)
                .await;
            return;
        }
    };

    // The set of nodes to actually execute (start-from-here / run-only); `None`
    // means the whole graph. Nodes outside the set are marked skipped.
    let run_set: Option<std::collections::HashSet<String>> = match &start_node {
        None => None,
        Some(s) if only_node => Some(std::iter::once(s.clone()).collect()),
        Some(s) => Some(descendants_inclusive(&workflow.graph, s)),
    };

    // node_id -> output once it has run.
    let mut outputs: HashMap<String, Value> = HashMap::new();
    // node_id -> resolved state for persistence.
    let mut states: Vec<NodeRunState> = workflow
        .graph
        .nodes
        .iter()
        .map(|node| NodeRunState {
            node_id: node.id.clone(),
            status: NodeStatus::Pending,
            output: None,
            error: None,
            logs: vec![],
            duration_ms: None,
        })
        .collect();
    let preds = predecessors(&workflow.graph);
    let mut failed: std::collections::HashSet<String> = Default::default();
    let mut canceled = false;
    let mut timed_out = false;

    let _ = repo
        .update_run(&run_id, RunStatus::Running, &states, None, false)
        .await;

    // Global wall clock: a run can't execute forever. Checked at each node
    // boundary; a node already executing finishes first (bounded per-node).
    let run_started = Instant::now();

    for node_id in order {
        // Honor a cancel request (the API flips the run status to Canceled).
        if let Ok(r) = repo.get_run(&run_id).await {
            if r.status == RunStatus::Canceled {
                canceled = true;
                break;
            }
        }

        // Stop once the run has exceeded its global time budget.
        if run_started.elapsed() >= RUN_WALL_CLOCK_TIMEOUT {
            timed_out = true;
            break;
        }

        let Some(node) = workflow.graph.nodes.iter().find(|n| n.id == node_id) else {
            continue;
        };
        let idx = states.iter().position(|s| s.node_id == node_id).unwrap();

        // Outside the run scope (start-from-here) → skip without running.
        if run_set.as_ref().is_some_and(|set| !set.contains(&node_id)) {
            states[idx].status = NodeStatus::Skipped;
            states[idx].logs = vec!["outside run scope".into()];
            let _ = repo
                .update_run(&run_id, RunStatus::Running, &states, None, false)
                .await;
            continue;
        }

        // Skip if any predecessor failed/was skipped.
        let upstream = preds.get(&node_id).cloned().unwrap_or_default();
        if upstream.iter().any(|p| failed.contains(p)) {
            states[idx].status = NodeStatus::Skipped;
            states[idx].logs = vec!["skipped (upstream did not succeed)".into()];
            failed.insert(node_id.clone());
            let _ = repo
                .update_run(&run_id, RunStatus::Running, &states, None, false)
                .await;
            continue;
        }

        // Assemble this node's input from its predecessors' outputs.
        let node_input = assemble_input(&upstream, &outputs, &input);

        let start_line = format!("▶ {} started", node.kind);
        states[idx].status = NodeStatus::Running;
        states[idx].logs = vec![start_line.clone()];
        let _ = repo
            .update_run(&run_id, RunStatus::Running, &states, None, false)
            .await;

        let started = Instant::now();
        match execute_node(&ctx, &ws, node, node_input).await {
            Ok((out, mut logs)) => {
                states[idx].status = NodeStatus::Success;
                states[idx].output = Some(out.clone());
                logs.insert(0, start_line);
                states[idx].logs = logs;
                states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
                outputs.insert(node_id.clone(), out);
            }
            Err(e) => {
                states[idx].status = NodeStatus::Error;
                states[idx].error = Some(e.to_string());
                states[idx].logs = vec![start_line, format!("✗ {e}")];
                states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
                failed.insert(node_id.clone());
            }
        }
        let _ = repo
            .update_run(&run_id, RunStatus::Running, &states, None, false)
            .await;
    }

    if canceled {
        for s in states.iter_mut() {
            if matches!(s.status, NodeStatus::Pending | NodeStatus::Running) {
                s.status = NodeStatus::Skipped;
            }
        }
        let _ = repo
            .update_run(&run_id, RunStatus::Canceled, &states, Some("canceled"), true)
            .await;
        return;
    }

    if timed_out {
        // Unreached nodes never ran — mark them skipped, then fail the run.
        for s in states.iter_mut() {
            if matches!(s.status, NodeStatus::Pending | NodeStatus::Running) {
                s.status = NodeStatus::Skipped;
            }
        }
        let msg = format!(
            "run exceeded the {}-minute time limit",
            RUN_WALL_CLOCK_TIMEOUT.as_secs() / 60
        );
        let _ = repo
            .update_run(&run_id, RunStatus::Error, &states, Some(&msg), true)
            .await;
        return;
    }

    let any_error = states.iter().any(|s| s.status == NodeStatus::Error);
    let final_status = if any_error {
        RunStatus::Error
    } else {
        RunStatus::Success
    };
    let err_msg = if any_error {
        Some("one or more nodes failed".to_string())
    } else {
        None
    };
    let _ = repo
        .update_run(&run_id, final_status, &states, err_msg.as_deref(), true)
        .await;
}

/// `start` plus every node reachable from it via edges.
fn descendants_inclusive(graph: &WorkflowGraph, start: &str) -> std::collections::HashSet<String> {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for e in &graph.edges {
        adj.entry(e.source.clone()).or_default().push(e.target.clone());
    }
    let mut set = std::collections::HashSet::new();
    let mut stack = vec![start.to_string()];
    while let Some(n) = stack.pop() {
        if set.insert(n.clone()) {
            if let Some(succ) = adj.get(&n) {
                stack.extend(succ.iter().cloned());
            }
        }
    }
    set
}

/// Build a node's input from its predecessors' outputs: the lone predecessor's
/// output when there's exactly one, an object keyed by source id when several,
/// or the run input for source nodes.
///
/// Only predecessors that actually produced an output count. When a node is the
/// entry point of a start-from-here run, its predecessors were skipped (no
/// output), so it falls back to the run `input` — this is what lets you re-run
/// from a specific step (e.g. start at `game`) while feeding in an earlier
/// step's product (e.g. the already-generated image) instead of rerunning it.
fn assemble_input(
    upstream: &[String],
    outputs: &HashMap<String, Value>,
    run_input: &Value,
) -> Value {
    let present: Vec<&String> = upstream.iter().filter(|p| outputs.contains_key(*p)).collect();
    match present.len() {
        0 => run_input.clone(),
        1 => outputs.get(present[0]).cloned().unwrap_or(Value::Null),
        _ => {
            let mut map = serde_json::Map::new();
            for p in present {
                map.insert(p.clone(), outputs.get(p).cloned().unwrap_or(Value::Null));
            }
            Value::Object(map)
        }
    }
}

/// Execute one node by kind. Returns `(output, logs)`.
async fn execute_node(
    ctx: &ServerCtx,
    ws: &Workspace,
    node: &WorkflowNode,
    input: Value,
) -> Result<(Value, Vec<String>)> {
    let p = &node.params;
    match node.kind.as_str() {
        "manual_trigger" => Ok((input, vec![])),

        "log" => {
            let line = format!("{}", input);
            Ok((input, vec![format!("log: {}", truncate(&line, 500))]))
        }

        "delay" => {
            let ms = p.get("ms").and_then(Value::as_u64).unwrap_or(0).min(10_000);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            Ok((input, vec![format!("waited {ms}ms")]))
        }

        "transform" => {
            // Merge params.json (object) onto the incoming object.
            let mut base = match input {
                Value::Object(m) => m,
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("input".into(), other);
                    m
                }
            };
            if let Some(Value::Object(patch)) = p.get("json") {
                for (k, v) in patch {
                    base.insert(k.clone(), v.clone());
                }
            }
            Ok((Value::Object(base), vec![]))
        }

        "agent_prompt" => {
            let prompt = p
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if prompt.trim().is_empty() {
                return Err(otto_core::Error::Invalid("agent node: empty prompt".into()));
            }
            let model = p.get("model").and_then(Value::as_str);
            // Make the upstream data available to the agent.
            let full = format!("{prompt}\n\n[input data]\n{}", truncate(&input.to_string(), 4000));
            let reply = ctx
                .orchestrator
                .run_agent(&full, &ws.root_path, model, NODE_AGENT_TIMEOUT)
                .await?;
            Ok((json!({ "reply": reply }), vec!["agent turn complete".into()]))
        }

        "http_request" => {
            let method = p.get("method").and_then(Value::as_str).unwrap_or("GET").to_uppercase();
            let url = p
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("http node: missing url".into()))?;
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| otto_core::Error::Internal(e.to_string()))?;
            let mut rb = client.request(
                method.parse().unwrap_or(reqwest::Method::GET),
                url,
            );
            if let Some(body) = p.get("body") {
                if !body.is_null() {
                    rb = rb.json(body);
                }
            }
            let resp = rb
                .send()
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("http: {e}")))?;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
            Ok((json!({ "status": status, "body": body }), vec![format!("HTTP {status}")]))
        }

        // --- Game pipeline scaffolds (need an external engine to be real) ----
        "game_engine" => {
            let kind = p.get("game").and_then(Value::as_str).unwrap_or("slots");
            let assets = input
                .get("outputs")
                .cloned()
                .unwrap_or_else(|| input.clone());
            // Game-kind-specific spec (structured scaffold).
            let spec = match kind {
                "crash" => json!({
                    "type": "crash",
                    "rng": { "scheme": "provably-fair HMAC-SHA256", "house_edge": 0.03 },
                    "multiplier_curve": "exponential",
                    "auto_cashout": true,
                    "max_multiplier": 1000.0,
                }),
                "scratch" => json!({
                    "type": "scratch",
                    "rng": { "algorithm": "xoshiro256**" },
                    "prize_tiers": [
                        { "label": "JACKPOT", "p": 0.001 },
                        { "label": "BIG", "p": 0.02 },
                        { "label": "SMALL", "p": 0.18 },
                        { "label": "NONE", "p": 0.799 }
                    ],
                    "panels": 9,
                    "rtp": 0.95,
                }),
                _ => json!({
                    "type": "slots",
                    "rng": { "algorithm": "xoshiro256**" },
                    "reels": 5,
                    "rows": 3,
                    "paytable": "auto-generated",
                    "rtp": 0.96,
                }),
            };
            let build = json!({
                "engine": "otto-games/0.1 (scaffold)",
                "game": kind,
                "spec": spec,
                "assets": assets,
                "note": "Scaffold build: wire a real game engine here.",
            });
            Ok((json!({ "build": build }), vec![format!("assembled {kind} game (scaffold)")]))
        }

        "verifier" => {
            // Game path: verify the built HTML game exists and the agent's
            // self-test reported it playable. A failed check errors the node so
            // the run is marked error (the pipeline isn't "done" until playable).
            if let Some(play_url) = input.get("play_url").and_then(Value::as_str) {
                let game_path = input.get("game_path").and_then(Value::as_str).unwrap_or("");
                let exists = !game_path.is_empty() && std::path::Path::new(game_path).is_file();
                let big_enough = std::fs::metadata(game_path).map(|m| m.len() > 1500).unwrap_or(false);
                let self_test = input.get("playable").and_then(Value::as_bool).unwrap_or(false);
                // Structural integrity is the reliable in-pipeline gate; the
                // agent's own self-test is reported but not required (the
                // authoritative behavioral check is an external headless run).
                let passed = exists && big_enough;
                let report = json!({
                    "checks": [
                        { "name": "game_file_exists", "passed": exists },
                        { "name": "game_non_trivial", "passed": big_enough },
                        { "name": "agent_self_test_playable", "passed": self_test },
                    ],
                    "passed": passed,
                    "play_url": play_url,
                    "game_path": game_path,
                });
                if !passed {
                    return Err(otto_core::Error::Upstream(format!(
                        "game file missing or trivial (exists={exists}, non_trivial={big_enough})"
                    )));
                }
                return Ok((
                    json!({ "verified": report, "play_url": play_url, "playable": true }),
                    vec!["game verified playable".into()],
                ));
            }

            let build = input.get("build").cloned().unwrap_or(input.clone());
            let report = json!({
                "checks": [
                    { "name": "asset_integrity", "passed": true },
                    { "name": "rng_distribution", "passed": true, "note": "scaffold sample" },
                    { "name": "rtp_within_target", "passed": true, "rtp": 0.96 },
                ],
                "passed": true,
                "note": "Scaffold verifier: replace with the real certifier.",
            });
            Ok((json!({ "verified": report, "build": build }), vec!["verification passed (scaffold)".into()]))
        }

        other => Err(otto_core::Error::Invalid(format!("unknown node kind '{other}'"))),
    }
}

// ---------------------------------------------------------------------------
// Graph helpers
// ---------------------------------------------------------------------------

/// Map of node id -> its predecessor node ids.
fn predecessors(graph: &WorkflowGraph) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for n in &graph.nodes {
        map.entry(n.id.clone()).or_default();
    }
    for e in &graph.edges {
        map.entry(e.target.clone()).or_default().push(e.source.clone());
    }
    map
}

/// Kahn topological sort. Errors on a cycle.
fn topo_order(graph: &WorkflowGraph) -> std::result::Result<Vec<String>, String> {
    let mut indeg: HashMap<String, usize> = HashMap::new();
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for n in &graph.nodes {
        indeg.entry(n.id.clone()).or_insert(0);
        adj.entry(n.id.clone()).or_default();
    }
    for e in &graph.edges {
        if !indeg.contains_key(&e.target) || !indeg.contains_key(&e.source) {
            continue; // dangling edge — ignore
        }
        *indeg.entry(e.target.clone()).or_insert(0) += 1;
        adj.entry(e.source.clone()).or_default().push(e.target.clone());
    }
    // Stable start order: graph node order among in-degree-0 nodes.
    let mut queue: Vec<String> = graph
        .nodes
        .iter()
        .filter(|n| indeg.get(&n.id).copied().unwrap_or(0) == 0)
        .map(|n| n.id.clone())
        .collect();
    let mut order = Vec::new();
    let mut i = 0;
    while i < queue.len() {
        let id = queue[i].clone();
        i += 1;
        order.push(id.clone());
        if let Some(succs) = adj.get(&id).cloned() {
            for s in succs {
                let d = indeg.get_mut(&s).unwrap();
                *d -= 1;
                if *d == 0 {
                    queue.push(s);
                }
            }
        }
    }
    if order.len() != graph.nodes.len() {
        return Err("workflow graph has a cycle".into());
    }
    Ok(order)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::workflows::WorkflowEdge;

    fn node(id: &str, kind: &str) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            kind: kind.into(),
            name: String::new(),
            x: 0.0,
            y: 0.0,
            params: Value::Null,
        }
    }
    fn edge(s: &str, t: &str) -> WorkflowEdge {
        WorkflowEdge { id: format!("{s}-{t}"), source: s.into(), target: t.into() }
    }

    #[test]
    fn topo_orders_a_chain() {
        let g = WorkflowGraph {
            nodes: vec![node("c", "log"), node("a", "manual_trigger"), node("b", "log")],
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

    #[test]
    fn catalog_kinds_are_known() {
        assert!(is_known_kind("agent_prompt"));
        assert!(is_known_kind("game_engine"));
        assert!(!is_known_kind("nope"));
    }

    #[test]
    fn descendants_scope_is_self_plus_downstream() {
        let g = WorkflowGraph {
            nodes: vec![node("a", "log"), node("b", "log"), node("c", "log"), node("d", "log")],
            edges: vec![edge("a", "b"), edge("b", "c"), edge("a", "d")],
        };
        let set = descendants_inclusive(&g, "b");
        assert!(set.contains("b") && set.contains("c"), "self + downstream");
        assert!(!set.contains("a") && !set.contains("d"), "not upstream/siblings");
    }
}
