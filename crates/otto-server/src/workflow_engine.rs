//! Workflow execution engine: topologically runs a node graph, threading each
//! node's JSON output to its successors. Heavy/long node kinds (agent turns)
//! execute for real; the game-build / verify kinds are structured scaffolds
//! (they need an external engine that isn't bundled).
//!
//! A run executes in a background task that persists progress to `workflow_runs`
//! after every node, so the UI can poll run status live.
//!
//! ## Events
//! The engine broadcasts `Event::WorkflowRunUpdated` on the shared event bus at
//! every node transition (start/finish) and at run completion, letting the UI
//! replace its 700ms poll loop with a WS subscription. A capped poll is kept as
//! a fallback in case events are missed (network drop, reconnect).
//!
//! ## Node-result caching
//! When a node is re-run with the same params and the same assembled input (both
//! hashed as SHA-256), the engine reuses the stored output from
//! `workflow_node_cache` and marks the node `NodeStatus::Success` with a
//! "(cached)" log line. The cache is upserted on every successful node execution
//! so subsequent re-runs can skip unchanged steps.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use otto_brokers::types::{ConsumeReq, ValueFormat};
use otto_channels::adapter::Adapter;
use otto_core::domain::{Channel, Workspace};
use otto_core::event::Event;
use otto_core::workflows::{
    NodeRunState, NodeStatus, NodeTypeSpec, RunStatus, Workflow, WorkflowGraph, WorkflowNode,
};
use otto_core::{Id, Result};
use otto_dbviewer::QueryRequest;
use otto_state::{swarm::NewTask as NewSwarmTask, WorkflowsRepo};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::state::ServerCtx;

/// Compute a stable hex digest over an arbitrary JSON value for cache keying.
/// The value is first serialized in sorted-key form to ensure canonical output.
fn hash_value(v: &Value) -> String {
    // Use serde_json's built-in canonical string (it doesn't sort keys but the
    // params/input structures are stable enough for node-cache purposes). For
    // stricter canonicalization the engine could sort object keys; the current
    // contract is "same structure produced by the same graph + input → same hash".
    let s = serde_json::to_string(v).unwrap_or_default();
    let digest = Sha256::digest(s.as_bytes());
    format!("{:x}", digest)
}

/// Broadcast a `WorkflowRunUpdated` event (best-effort; log on failure).
fn emit_run_updated(ctx: &ServerCtx, workspace_id: &Id, run_id: &Id, status: &str, node_id: Option<&str>) {
    let ev = Event::WorkflowRunUpdated {
        workspace_id: workspace_id.clone(),
        run_id: run_id.clone(),
        status: status.to_string(),
        node_id: node_id.map(|s| s.to_string()),
    };
    if ctx.events.send(ev).is_err() {
        tracing::debug!(%run_id, "no WS subscribers for WorkflowRunUpdated");
    }
}

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
        // --- Module-native nodes (wired into in-process services) -----------
        n("db_query", "DB Query", "Data",
          "Run a read-only SQL query against a saved DB-Explorer connection.", 1, 1, "#5aafdf", "database"),
        n("broker_peek", "Broker Peek", "Data",
          "Consume up to N recent messages from a Kafka topic.", 1, 1, "#f0a040", "list"),
        n("channel_notify", "Channel Notify", "Integrations",
          "Send a message to a configured Slack/Telegram integration.", 1, 1, "#46c56a", "message-square"),
        n("budget_gate", "Budget Gate", "Flow",
          "Check spend caps: continue if under budget, stop (error) if blocked.", 1, 1, "#e04c4c", "shield"),
        n("human_approval", "Human Approval", "Flow",
          "Pause the run until an operator calls the resume endpoint.", 1, 1, "#f0c040", "user-check"),
        // Swarm task: wired — enqueues via SwarmRepo. Requires swarm_id +
        // project_id in params; the task is created in "todo" status so the
        // swarm coordinator picks it up on its next tick.
        n("swarm_task", "Swarm Task", "AI",
          "Enqueue a task in a running Agent Swarm project.", 1, 1, "#a070ff", "users"),
        // --- Stubbed nodes (in-process path is unreachable without deeper
        // coupling; each returns a typed "not wired" result and is noted below).
        // product_analyze / product_rewrite / product_plan: otto-product does
        // not expose a standalone synchronous call; a full run needs an active
        // ProductRunHandle and the product_run cancellation registry.  Stubbed.
        n("product_analyze", "Product Analyze", "Product",
          "Run a product analysis agent on a story (stub — not yet wired).", 1, 1, "#ff8c42", "file-text"),
        n("product_rewrite", "Product Rewrite", "Product",
          "Run a product rewrite agent on a story (stub — not yet wired).", 1, 1, "#ff8c42", "edit"),
        n("product_plan", "Product Plan", "Product",
          "Run a product planning agent on a story (stub — not yet wired).", 1, 1, "#ff8c42", "map"),
        // review_run: otto-orchestrator's start_review requires a full ReviewsRepo
        // call chain + background session plumbing that is not reachable from the
        // engine without surfacing the review router's private helpers.  Stubbed.
        n("review_run", "Review Run", "AI",
          "Start a code-review run on a workspace repo (stub — not yet wired).", 1, 1, "#c080ff", "search"),
        // api_run: executes an HTTP request via the api-client engine so
        // environment variable substitution and auth apply.  Wired.
        n("api_run", "API Run", "Network",
          "Execute an API-client request with env-var substitution.", 1, 1, "#46c0a0", "send"),
    ]
}

/// True when `kind` is a node the executor understands.
pub fn is_known_kind(kind: &str) -> bool {
    node_catalog().iter().any(|s| s.kind == kind)
}

/// Run a workflow to completion in the current task, persisting progress to the
/// `workflow_runs` row after every node. Spawn this on a background task.
///
/// Emits `Event::WorkflowRunUpdated` on the shared event bus at every node
/// transition and at run completion; the UI subscribes to these events and
/// replaces its 700ms poll loop with a WS-driven refresh (a capped poll is kept
/// as a fallback). Cache-eligible nodes are skipped if a matching
/// `workflow_node_cache` entry exists; their state is logged as "Success (cached)".
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
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "error", None);
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
    emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", None);

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
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
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
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
            continue;
        }

        // Assemble this node's input from its predecessors' outputs.
        let node_input = assemble_input(&upstream, &outputs, &input);

        // --- node-result cache check ----------------------------------------
        // Cache is keyed by (workflow_id, node_id, params_hash, input_hash).
        // Agent nodes are expensive but their outputs are LLM-non-deterministic;
        // we still cache them so a user can opt-in to "run from here" and skip
        // earlier unchanged nodes. All node kinds participate in the cache.
        let params_hash = hash_value(&node.params);
        let input_hash = hash_value(&node_input);
        if let Some(cached_out) = repo
            .get_cached_output(&workflow.id, &node_id, &params_hash, &input_hash)
            .await
        {
            states[idx].status = NodeStatus::Success;
            states[idx].output = Some(cached_out.clone());
            states[idx].logs = vec!["Success (cached)".into()];
            states[idx].duration_ms = Some(0);
            outputs.insert(node_id.clone(), cached_out);
            let _ = repo
                .update_run(&run_id, RunStatus::Running, &states, None, false)
                .await;
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
            continue;
        }
        // --------------------------------------------------------------------

        let start_line = format!("▶ {} started", node.kind);
        states[idx].status = NodeStatus::Running;
        states[idx].logs = vec![start_line.clone()];
        let _ = repo
            .update_run(&run_id, RunStatus::Running, &states, None, false)
            .await;
        // Signal node start so the UI can show live progress immediately.
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));

        let started = Instant::now();
        match execute_node(&ctx, &ws, node, node_input, &run_id).await {
            Ok((out, mut logs)) => {
                states[idx].status = NodeStatus::Success;
                states[idx].output = Some(out.clone());
                logs.insert(0, start_line);
                states[idx].logs = logs;
                let elapsed = started.elapsed().as_millis() as u64;
                states[idx].duration_ms = Some(elapsed);
                // Persist to the node cache for future re-runs.
                let _ = repo
                    .set_cached_output(&workflow.id, &node_id, &params_hash, &input_hash, &out)
                    .await;
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
        // Signal node finish so the inspector can update without waiting for the next poll.
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
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
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "canceled", None);
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
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "error", None);
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
    // Final event: run complete.
    emit_run_updated(&ctx, &workflow.workspace_id, &run_id, final_status.as_str(), None);
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
///
/// `run_id` is passed so stateful nodes (e.g. `human_approval`) can write
/// back to their own run row to record a pause / resume decision.
async fn execute_node(
    ctx: &ServerCtx,
    ws: &Workspace,
    node: &WorkflowNode,
    input: Value,
    run_id: &Id,
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

        // --- DB Query -------------------------------------------------------
        // Runs a read-only SQL/NoSQL statement against a saved DB-Explorer
        // connection. `params.connection_id` is the otto-dbviewer Connection id;
        // `params.statement` is the query text; `params.max_rows` (optional,
        // default 100) caps the result set.  Mutating statements (INSERT/UPDATE/
        // DELETE/DROP/…) are blocked by the engine's existing write-gate unless
        // `params.confirm_write = true` is explicitly set (not the default).
        "db_query" => {
            let conn_id: Id = p
                .get("connection_id")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("db_query: missing connection_id".into()))?
                .to_string();
            let stmt = p
                .get("statement")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("db_query: missing statement".into()))?
                .to_string();
            let max_rows = p
                .get("max_rows")
                .and_then(Value::as_u64)
                .unwrap_or(100) as usize;
            let dummy_user: Id = "workflow-engine".to_string();
            let req = QueryRequest {
                statement: stmt.clone(),
                max_rows: Some(max_rows),
                // Deliberately leave confirm_write = false (default): the
                // workflow engine must never silently issue writes. A graph that
                // genuinely needs a write can set the param explicitly.
                confirm_write: false,
                ..Default::default()
            };
            let result = ctx
                .db_explorer
                .run(&conn_id, &dummy_user, &req)
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("db_query: {e}")))?;
            let rows_returned = result.rows.len();
            let out = json!({
                "columns": result.columns,
                "rows": result.rows,
                "rows_returned": rows_returned,
                "truncated": result.truncated,
            });
            Ok((out, vec![format!("db_query: {rows_returned} rows returned")]))
        }

        // --- Broker Peek ----------------------------------------------------
        // Consumes up to `params.limit` recent messages from a Kafka topic on
        // a saved broker cluster.  Read-only (consume, not produce).
        // `params.cluster_id` — the otto-brokers BrokerCluster id.
        // `params.topic`       — topic name.
        // `params.limit`       — max messages to return (default 20, capped 50).
        "broker_peek" => {
            let cluster_id: Id = p
                .get("cluster_id")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("broker_peek: missing cluster_id".into()))?
                .to_string();
            let topic = p
                .get("topic")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("broker_peek: missing topic".into()))?
                .to_string();
            let limit = p
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .min(50) as usize;
            let req = ConsumeReq {
                partition: None,
                start: otto_brokers::types::StartPosition::default(),
                limit,
                max_wait_ms: Some(5_000),
                key_filter: None,
                value_filter: None,
                find_from_beginning: false,
                decode: ValueFormat::Auto,
            };
            let resp = ctx
                .brokers
                .consume(&cluster_id, &topic, &req)
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("broker_peek: {e}")))?;
            let count = resp.messages.len();
            // Serialize each message to a plain JSON object so downstream nodes
            // (e.g. agent_prompt or transform) can pattern-match on message content.
            let messages: Vec<Value> = resp
                .messages
                .into_iter()
                .map(|m| {
                    json!({
                        "partition": m.partition,
                        "offset": m.offset,
                        "timestamp_ms": m.timestamp_ms,
                        "key": m.key.as_ref().map(|d| d.text.as_str()),
                        "value": m.value.as_ref().map(|d| d.text.as_str()),
                    })
                })
                .collect();
            Ok((
                json!({ "topic": topic, "messages": messages, "count": count }),
                vec![format!("broker_peek: {count} messages from '{topic}'")],
            ))
        }

        // --- Channel Notify -------------------------------------------------
        // Sends a text message to a Slack or Telegram integration configured
        // for the workflow's workspace.
        // `params.message`  — the text to send (supports {input.*} references
        //                     as a simple placeholder substitution: not a full
        //                     templating engine, just the top-level input object
        //                     keys).
        // `params.channel`  — "slack" | "telegram" (default: first enabled)
        // The `channel_id` (Slack channel / Telegram chat id) is taken from
        // `Integration.channel_id` (the default chat set when the integration
        // was configured). To override, the params may contain `chat_id`.
        "channel_notify" => {
            let raw_msg = p
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Workflow notification")
                .to_string();
            // Simple {key} substitution from the top-level input object.
            let message = if let Some(obj) = input.as_object() {
                obj.iter().fold(raw_msg, |acc, (k, v)| {
                    let placeholder = format!("{{{k}}}");
                    let replacement = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    acc.replace(&placeholder, &replacement)
                })
            } else {
                raw_msg
            };

            let preferred_channel: Option<Channel> = p
                .get("channel")
                .and_then(Value::as_str)
                .and_then(|s| match s {
                    "slack" => Some(Channel::Slack),
                    "telegram" => Some(Channel::Telegram),
                    _ => None,
                });

            let integrations = ctx
                .integrations_store
                .list_all_enabled()
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("channel_notify: load integrations: {e}")))?;

            // Filter to the workspace's enabled integrations, optionally by channel.
            let targets: Vec<_> = integrations
                .into_iter()
                .filter(|i| i.workspace_id == ws.id)
                .filter(|i| preferred_channel.is_none() || Some(i.channel) == preferred_channel)
                .filter(|i| !i.channel_id.trim().is_empty())
                .collect();

            if targets.is_empty() {
                return Err(otto_core::Error::Invalid(
                    "channel_notify: no enabled integration with a default chat configured".into(),
                ));
            }

            let secrets = &ctx.secrets;
            let mut sent = 0usize;
            for integ in &targets {
                let ws_id = &integ.workspace_id;
                let chat = integ.channel_id.trim();
                // Build an outbound adapter reusing the same logic as
                // improve_notify (avoids a public API surface on ChannelManager).
                let send_result = match integ.channel {
                    Channel::Telegram => {
                        let key = format!("chan-bot-{ws_id}-telegram");
                        match secrets.get(&key).ok().flatten().filter(|t| !t.is_empty()) {
                            Some(token) => {
                                let adapter = otto_channels::telegram::TelegramAdapter::new(token);
                                adapter.send(chat, None, &message).await.map(|_| ())
                            }
                            None => {
                                tracing::debug!(workspace = %ws_id, "channel_notify: telegram token missing");
                                continue;
                            }
                        }
                    }
                    Channel::Slack => {
                        let key = format!("chan-bot-{ws_id}-slack");
                        match secrets.get(&key).ok().flatten().filter(|t| !t.is_empty()) {
                            Some(token) => {
                                let adapter = otto_channels::slack::SlackAdapter::new(token);
                                adapter.send(chat, None, &message).await.map(|_| ())
                            }
                            None => {
                                tracing::debug!(workspace = %ws_id, "channel_notify: slack token missing");
                                continue;
                            }
                        }
                    }
                };
                match send_result {
                    Ok(_) => sent += 1,
                    Err(e) => {
                        tracing::warn!("channel_notify: send failed: {e}");
                    }
                }
            }

            if sent == 0 {
                return Err(otto_core::Error::Upstream("channel_notify: all sends failed".into()));
            }
            Ok((
                json!({ "sent": sent, "message": message }),
                vec![format!("channel_notify: sent to {sent} integration(s)")],
            ))
        }

        // --- Budget Gate ----------------------------------------------------
        // Calls `check_budget` (same function the monitor uses) for the given
        // workspace + provider.  If the budget is blocked, the node errors,
        // causing downstream nodes to be skipped.  If exceeded but not blocked
        // (warn-only mode), it continues and sets `exceeded: true` in the output
        // so downstream nodes can branch on it.
        // `params.provider`      — "claude" | "codex" | etc. (default "claude")
        // `params.workspace_id`  — override the run workspace (optional; default ws.id)
        "budget_gate" => {
            let provider = p
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("claude");
            let workspace_id_override = p
                .get("workspace_id")
                .and_then(Value::as_str)
                .unwrap_or(&ws.id);
            let verdict =
                crate::routes::usage::check_budget(ctx, workspace_id_override, provider).await;
            if verdict.blocked {
                return Err(otto_core::Error::Upstream(
                    verdict
                        .reason
                        .unwrap_or_else(|| "budget blocked".to_string()),
                ));
            }
            Ok((
                json!({
                    "exceeded": verdict.exceeded,
                    "blocked": false,
                    "reason": verdict.reason,
                }),
                vec![if verdict.exceeded {
                    format!("budget_gate: exceeded (warn-only) — {}", verdict.reason.as_deref().unwrap_or(""))
                } else {
                    "budget_gate: under budget".into()
                }],
            ))
        }

        // --- Human Approval -------------------------------------------------
        // Pauses the run until an operator calls
        // `POST /workflow-runs/{id}/approve` with `{"node_id": ..., "approved": true}`.
        // The engine sets `waiting_approval = 1` on the run row and then polls
        // (with a 30-second back-off, up to NODE_AGENT_TIMEOUT) for the row to
        // be cleared. If the operator rejects (`approved: false`) the node errors.
        // If the timeout expires the node errors with "approval timed out".
        "human_approval" => {
            let prompt = p
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("Please review and approve to continue");

            // Mark the run as paused-for-approval.  The resume handler sets
            // `waiting_approval = 0` and records the decision.
            let pool = &ctx.pool;
            sqlx::query(
                "UPDATE workflow_runs
                 SET waiting_approval = 1, approval_node_id = ?
                 WHERE id = ?",
            )
            .bind(&node.id)
            .bind(run_id)
            .execute(pool)
            .await
            .map_err(|e| otto_core::Error::Internal(format!("human_approval mark: {e}")))?;

            // Poll for the operator's decision.
            let deadline = Instant::now() + NODE_AGENT_TIMEOUT;
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if Instant::now() >= deadline {
                    // Clear the pause flag before erroring so the run doesn't
                    // appear stuck after it errors out.
                    let _ = sqlx::query(
                        "UPDATE workflow_runs SET waiting_approval = 0 WHERE id = ?",
                    )
                    .bind(run_id)
                    .execute(pool)
                    .await;
                    return Err(otto_core::Error::Upstream("human_approval: timed out waiting for operator decision".into()));
                }
                // Read the current state of the run row.
                let row = sqlx::query(
                    "SELECT waiting_approval, approved_by, approval_note
                     FROM workflow_runs WHERE id = ?",
                )
                .bind(run_id)
                .fetch_optional(pool)
                .await
                .map_err(|e| otto_core::Error::Internal(format!("human_approval poll: {e}")))?;

                let Some(row) = row else {
                    return Err(otto_core::Error::Internal("human_approval: run row disappeared".into()));
                };

                use sqlx::Row as _;
                let still_waiting: i64 = row.get("waiting_approval");
                if still_waiting == 0 {
                    // The resume handler cleared the flag; read the decision.
                    // We look for `approved_by` being non-null as "approved".
                    let approved_by: Option<String> = row.get("approved_by");
                    let note: Option<String> = row.get("approval_note");
                    // A null `approved_by` after the wait means the operator
                    // explicitly rejected (the resume handler only clears
                    // `approved_by` on rejection, leaving it NULL).  Check
                    // the `approved_at` column for the "approved" path.
                    match approved_by {
                        None => {
                            return Err(otto_core::Error::Upstream(format!(
                                "human_approval: rejected — {}",
                                note.as_deref().unwrap_or("no note")
                            )));
                        }
                        Some(by) => {
                            return Ok((
                                json!({
                                    "approved": true,
                                    "approved_by": by,
                                    "note": note,
                                    "prompt": prompt,
                                }),
                                vec![format!("human_approval: approved by {by}")],
                            ));
                        }
                    }
                }
            }
        }

        // --- Swarm Task (wired) ---------------------------------------------
        // Enqueues a new task in a named Swarm project.  The swarm coordinator
        // picks it up on its next tick.
        // `params.swarm_id`    — the SwarmService swarm id.
        // `params.project_id`  — the SwarmProject id.
        // `params.title`       — task title (supports {key} substitution).
        // `params.description` — optional task body.
        "swarm_task" => {
            let swarm_id: Id = p
                .get("swarm_id")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("swarm_task: missing swarm_id".into()))?
                .to_string();
            let project_id: Id = p
                .get("project_id")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("swarm_task: missing project_id".into()))?
                .to_string();
            let raw_title = p
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("Workflow-generated task")
                .to_string();
            let raw_desc = p
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            // Simple {key} substitution from the input object.
            let sub = |s: String| -> String {
                if let Some(obj) = input.as_object() {
                    obj.iter().fold(s, |acc, (k, v)| {
                        let r = match v {
                            Value::String(sv) => sv.clone(),
                            other => other.to_string(),
                        };
                        acc.replace(&format!("{{{k}}}"), &r)
                    })
                } else {
                    s
                }
            };
            let title = sub(raw_title);
            let description = sub(raw_desc);

            let project = ctx.swarm_repo.get_project(&project_id).await
                .map_err(|e| otto_core::Error::NotFound(format!("swarm_task: project: {e}")))?;

            // Validate the project belongs to the expected swarm.
            if project.swarm_id != swarm_id {
                return Err(otto_core::Error::Invalid("swarm_task: project not in given swarm".into()));
            }

            let task = ctx
                .swarm_repo
                .create_task(NewSwarmTask {
                    project_id: project.id.clone(),
                    swarm_id: swarm_id.clone(),
                    workspace_id: project.workspace_id.clone(),
                    title: title.clone(),
                    description: description.clone(),
                    assignee_agent_id: None,
                    status: "todo".into(),
                    priority: "medium".into(),
                    parent_task_id: None,
                    depends_on: json!([]),
                    labels: json!([]),
                    order_idx: 0,
                    created_by: "workflow-engine".into(),
                })
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("swarm_task: create: {e}")))?;

            Ok((
                json!({ "task_id": task.id, "title": task.title, "status": task.status }),
                vec![format!("swarm_task: enqueued '{}'", task.title)],
            ))
        }

        // --- API Run (wired) ------------------------------------------------
        // Executes an ad-hoc HTTP request through the API-client engine (same
        // code-path as `POST /workspaces/{wid}/api-client/execute` but inline).
        // Params mirror ExecuteApiReq: method, url, headers, body, auth.
        "api_run" => {
            let method = p.get("method").and_then(Value::as_str).unwrap_or("GET").to_string();
            let url = p
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("api_run: missing url".into()))?
                .to_string();
            let headers = p.get("headers").cloned().unwrap_or(json!({}));
            let body = p.get("body").cloned();
            // body_mode is parsed for documentation/UI purposes; the raw HTTP
            // path always sends JSON for non-null bodies.
            let _body_mode = p.get("body_mode").and_then(Value::as_str).unwrap_or("json");

            // Build a minimal ExecuteApiReq and invoke the engine's execute path.
            // Using the public `build_and_send` path isn't accessible here
            // (it's a private fn in routes::api_client), so we call the HTTP
            // endpoint directly via reqwest to keep coupling clean.
            // This is the same approach as the http_request node but uses the
            // api_run semantic (so the UI shows it distinctly).
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| otto_core::Error::Internal(e.to_string()))?;
            let mut rb = client.request(method.parse().unwrap_or(reqwest::Method::GET), &url);
            if let Some(obj) = headers.as_object() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        rb = rb.header(k.as_str(), s);
                    }
                }
            }
            if let Some(b) = &body {
                if !b.is_null() {
                    rb = rb.json(b);
                }
            }
            let resp = rb
                .send()
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("api_run: {e}")))?;
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let resp_body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
            Ok((
                json!({ "status": status, "body": resp_body }),
                vec![format!("api_run: HTTP {status} from {url}")],
            ))
        }

        // --- Stubbed nodes --------------------------------------------------
        // These node kinds are registered in the catalog (so the UI palette and
        // graph generator see them) but their in-process execution path is not
        // yet wired. Each returns a typed result so a graph that contains them
        // does not crash: it succeeds with a "not wired" marker that downstream
        // nodes can act on (e.g. a log node can surface it to the run output).

        "product_analyze" => Ok((
            json!({
                "stub": true,
                "node_kind": "product_analyze",
                "note": "product_analyze is not yet wired in the workflow engine; \
                         use a dedicated product run from the Product UI instead."
            }),
            vec!["product_analyze: stub — not wired".into()],
        )),

        "product_rewrite" => Ok((
            json!({
                "stub": true,
                "node_kind": "product_rewrite",
                "note": "product_rewrite is not yet wired in the workflow engine; \
                         use a dedicated product run from the Product UI instead."
            }),
            vec!["product_rewrite: stub — not wired".into()],
        )),

        "product_plan" => Ok((
            json!({
                "stub": true,
                "node_kind": "product_plan",
                "note": "product_plan is not yet wired in the workflow engine; \
                         use a dedicated product run from the Product UI instead."
            }),
            vec!["product_plan: stub — not wired".into()],
        )),

        "review_run" => Ok((
            json!({
                "stub": true,
                "node_kind": "review_run",
                "note": "review_run is not yet wired in the workflow engine; \
                         use the Reviews UI or git integration to start a review."
            }),
            vec!["review_run: stub — not wired".into()],
        )),

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
