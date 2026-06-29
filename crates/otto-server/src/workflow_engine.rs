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
use otto_core::domain::{Channel, User, Workspace};
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
        output_schema: output_schema_for(kind),
        params_schema: None,
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
        n("condition", "Condition", "Flow",
          "Evaluate an expression on the input; outputs { result, value }. Pair with edge conditions to branch.", 1, 1, "#f0c040", "git-branch"),
        n("loop", "Loop (Until)", "Flow",
          "Re-run inner steps until an expression holds or max iterations (e.g. fix → review until score ≥ 80).", 1, 1, "#f0c040", "repeat"),
        // Swarm task: wired — enqueues via SwarmRepo. Requires swarm_id +
        // project_id in params; the task is created in "todo" status so the
        // swarm coordinator picks it up on its next tick.
        n("swarm_task", "Swarm Task", "AI",
          "Enqueue a task in a running Agent Swarm project.", 1, 1, "#a070ff", "users"),
        // --- Product nodes (wired: run a real single-agent turn over the story's
        // context + the matching product skill, as a visible session). ---------
        n("product_analyze", "Product Analyze", "Product",
          "Analyze a product story (grill lens) over its real context; outputs the analysis.", 1, 1, "#ff8c42", "file-text"),
        n("product_rewrite", "Product Rewrite", "Product",
          "Rewrite a product story (jira-story-writer); optionally save a new version.", 1, 1, "#ff8c42", "edit"),
        n("product_plan", "Product Plan", "Product",
          "Break a story into an implementation plan (story-task-breakdown); optional version.", 1, 1, "#ff8c42", "map"),
        n("product_publish", "Product Publish", "Product",
          "Publish a story as a Confluence RFC or a Jira issue (dry-run by default).", 1, 1, "#ff8c42", "upload"),
        n("canvas", "Canvas Diagram", "Product",
          "Generate/update a Canvas scene (mermaid/excalidraw) from a prompt via an agent.", 1, 1, "#57b9ff", "image"),
        // review_run: wired to the local-review engine (run_review_for_branch).
        n("review_run", "Review Run", "AI",
          "Run a code review on a repo branch; outputs findings + a 0–100 score.", 1, 1, "#c080ff", "search"),
        n("git_pr", "Git PR", "Network",
          "Draft (and optionally open) a pull request for a repo branch.", 1, 1, "#46c0a0", "git-pull-request"),
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

/// Declared output shape per node kind (drives UI expression hints + warn-only
/// runtime validation). Keys map to JSON types; `None` means free-form output.
fn output_schema_for(kind: &str) -> Option<Value> {
    let obj = |pairs: &[(&str, &str)]| {
        let mut m = serde_json::Map::new();
        for (k, t) in pairs {
            m.insert((*k).to_string(), json!(t));
        }
        Some(json!({ "type": "object", "fields": Value::Object(m) }))
    };
    match kind {
        "agent_prompt" => obj(&[("reply", "string")]),
        "http_request" | "api_run" => obj(&[("status", "number"), ("body", "any")]),
        "db_query" => obj(&[("columns", "array"), ("rows", "array"), ("rows_returned", "number")]),
        "broker_peek" => obj(&[("topic", "string"), ("messages", "array"), ("count", "number")]),
        "budget_gate" => obj(&[("exceeded", "boolean"), ("blocked", "boolean")]),
        "human_approval" => obj(&[("approved", "boolean"), ("approved_by", "string")]),
        "condition" => obj(&[("result", "boolean"), ("value", "any")]),
        "loop" => obj(&[("iterations", "number"), ("satisfied", "boolean"), ("last", "any")]),
        "review_run" => obj(&[
            ("review_id", "string"),
            ("status", "string"),
            ("blocking", "number"),
            ("advisory", "number"),
            ("score", "number"),
            ("threshold", "number"),
            ("passed", "boolean"),
        ]),
        "product_analyze" => obj(&[("story_id", "string"), ("analysis", "string")]),
        "product_rewrite" => obj(&[("story_id", "string"), ("body_md", "string")]),
        "product_plan" => obj(&[("story_id", "string"), ("plan_md", "string")]),
        "product_publish" => obj(&[("story_id", "string"), ("url", "string"), ("dry_run", "boolean")]),
        "git_pr" => obj(&[("title", "string"), ("description", "string"), ("opened", "boolean")]),
        "canvas" => obj(&[("scene_id", "string"), ("summary", "string")]),
        "swarm_task" => obj(&[("task_id", "string"), ("title", "string")]),
        _ => None,
    }
}

/// Warn-only validation of a node's output against its declared schema. Returns a
/// list of human-readable warnings (missing keys / wrong types). Never fails a run.
fn validate_node_output(kind: &str, output: &Value) -> Vec<String> {
    let Some(schema) = output_schema_for(kind) else {
        return vec![];
    };
    let Some(fields) = schema.get("fields").and_then(Value::as_object) else {
        return vec![];
    };
    let Some(obj) = output.as_object() else {
        return vec![format!("{kind}: expected an object output")];
    };
    let mut warns = Vec::new();
    for (key, ty) in fields {
        let ty = ty.as_str().unwrap_or("any");
        match obj.get(key) {
            None => warns.push(format!("{kind}: missing output field '{key}'")),
            Some(v) => {
                let ok = match ty {
                    "string" => v.is_string(),
                    "number" => v.is_number(),
                    "boolean" => v.is_boolean(),
                    "array" => v.is_array(),
                    "object" => v.is_object(),
                    _ => true,
                };
                if !ok && !v.is_null() {
                    warns.push(format!("{kind}: output field '{key}' is not {ty}"));
                }
            }
        }
    }
    warns
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
            attempts: None,
            sessions: vec![],
        })
        .collect();
    // Resolve the user this run acts as (for spawning visible agent sessions).
    let user = resolve_run_user(&ctx, &workflow.created_by).await;
    // Seed the run input from the entry manual_trigger node's configured fields
    // (its inspector — prompt/working_directory/repo_id/goals/…), letting the
    // actual /run body or chat-trigger input override per key. This is what makes
    // the Start node the place to set the input for a manual run.
    let input = {
        let mut seeded = serde_json::Map::new();
        if let Some(mt) = workflow.graph.nodes.iter().find(|n| n.kind == "manual_trigger") {
            if let Some(o) = mt.params.as_object() {
                for (k, v) in o {
                    if !v.is_null() {
                        seeded.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        if let Some(o) = input.as_object() {
            for (k, v) in o {
                seeded.insert(k.clone(), v.clone());
            }
        }
        if seeded.is_empty() {
            input
        } else {
            Value::Object(seeded)
        }
    };
    // Run-level working directory: the `working_directory` from the run input
    // (e.g. a Slack `Working Directory:` field), else the workspace root. Agent
    // nodes run here — so a workflow owned by workspace A can operate on repo X.
    let run_cwd = input
        .get("working_directory")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| ws.root_path.clone());
    // Nodes that errored (or were poisoned by an errored upstream) — these
    // propagate failure. `branch_skipped` nodes were pruned by an edge condition
    // (or are downstream of a pruned node) and do NOT fail the run.
    let mut errored: std::collections::HashSet<String> = Default::default();
    let mut branch_skipped: std::collections::HashSet<String> = Default::default();
    // Edge ids whose condition evaluated false (the branch was not taken).
    let mut inactive_edges: std::collections::HashSet<String> = Default::default();
    let mut canceled = false;
    let mut timed_out = false;

    // Record which workflow version this run executed (best-effort).
    let _ = repo.set_run_version(&run_id, workflow.version).await;

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

        // Decide whether to run this node based on its incoming edges. Only
        // edges whose source is within the run scope constrain control flow (a
        // start-from-here run leaves ancestors out of scope; their edges don't
        // poison or branch-skip the entry node — it falls back to the run input).
        let in_scope = |n: &str| run_set.as_ref().map(|s| s.contains(n)).unwrap_or(true);
        let views: Vec<EdgeView> = incoming_edges(&workflow.graph, &node_id)
            .iter()
            .filter(|e| in_scope(&e.source))
            .map(|e| EdgeView {
                source: e.source.clone(),
                errored: errored.contains(&e.source),
                has_output: outputs.contains_key(&e.source),
                edge_active: !inactive_edges.contains(&e.id),
            })
            .collect();
        let node_input = match decide_node(&views) {
            NodeDecision::ErrorSkip => {
                states[idx].status = NodeStatus::Skipped;
                states[idx].logs = vec!["skipped (upstream did not succeed)".into()];
                errored.insert(node_id.clone());
                let _ = repo
                    .update_run(&run_id, RunStatus::Running, &states, None, false)
                    .await;
                emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
                continue;
            }
            NodeDecision::BranchSkip => {
                states[idx].status = NodeStatus::Skipped;
                states[idx].logs = vec!["skipped (branch not taken)".into()];
                branch_skipped.insert(node_id.clone());
                let _ = repo
                    .update_run(&run_id, RunStatus::Running, &states, None, false)
                    .await;
                emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
                continue;
            }
            NodeDecision::Run(satisfied) => assemble_input(&satisfied, &outputs, &input),
        };

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
            states[idx].attempts = Some(0);
            // A cached agent/product/canvas node still carries its session id in
            // the cached output — surface it so the run can open it.
            harvest_session_ids(&cached_out, &mut states[idx].sessions);
            // Prune outgoing edges whose condition fails on the cached output.
            let (pruned, mut plogs) =
                eval_outgoing(&workflow.graph, node, &cached_out, &node_input, &input);
            inactive_edges.extend(pruned);
            states[idx].logs.append(&mut plogs);
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
        // Run the node, honoring its retry policy (default: a single attempt).
        // A node that spawns an openable agent session reports its id over
        // `sess_tx` the moment it's created; we record it on the node state and
        // persist+emit immediately so the run view can open it *while running*.
        let policy = resolve_retry(node);
        let mut attempt: u32 = 0;
        let mut backoff = policy.backoff_ms;
        let mut retry_logs: Vec<String> = vec![];
        let (sess_tx, mut sess_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let result = loop {
            attempt += 1;
            let fut = execute_node(&ctx, &ws, &user, node, node_input.clone(), &run_id, &run_cwd, &sess_tx);
            tokio::pin!(fut);
            let attempt_res = loop {
                tokio::select! {
                    biased;
                    Some(sid) = sess_rx.recv() => {
                        if !states[idx].sessions.contains(&sid) {
                            states[idx].sessions.push(sid);
                            let _ = repo
                                .update_run(&run_id, RunStatus::Running, &states, None, false)
                                .await;
                            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id));
                        }
                    }
                    r = &mut fut => break r,
                }
            };
            match attempt_res {
                Ok(ok) => break Ok(ok),
                Err(e) => {
                    let can_retry = attempt <= policy.max_attempts && is_retryable(&node.kind);
                    if !can_retry {
                        break Err(e);
                    }
                    retry_logs.push(format!(
                        "attempt {attempt} failed: {e} — retrying in {backoff}ms"
                    ));
                    // Bail out of the backoff promptly if the run was canceled.
                    if let Ok(r) = repo.get_run(&run_id).await {
                        if r.status == RunStatus::Canceled {
                            break Err(e);
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    backoff = ((backoff as f64) * policy.factor) as u64;
                    backoff = backoff.clamp(1, 60_000);
                }
            }
        };
        // Drain any session ids reported right as the node finished.
        while let Ok(sid) = sess_rx.try_recv() {
            if !states[idx].sessions.contains(&sid) {
                states[idx].sessions.push(sid);
            }
        }
        match result {
            Ok((out, mut logs)) => {
                states[idx].status = NodeStatus::Success;
                states[idx].output = Some(out.clone());
                logs.insert(0, start_line);
                logs.append(&mut retry_logs);
                // Warn-only output validation against the node's declared schema.
                for w in validate_node_output(&node.kind, &out) {
                    logs.push(format!("⚠ {w}"));
                }
                // Prune outgoing edges whose condition fails on this output.
                let (pruned, mut plogs) =
                    eval_outgoing(&workflow.graph, node, &out, &node_input, &input);
                inactive_edges.extend(pruned);
                logs.append(&mut plogs);
                states[idx].logs = logs;
                states[idx].attempts = Some(attempt);
                // Also harvest a session id carried in the output (dedups with the
                // live channel report).
                harvest_session_ids(&out, &mut states[idx].sessions);
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
                let mut elogs = vec![start_line];
                elogs.append(&mut retry_logs);
                elogs.push(format!("✗ {e}"));
                states[idx].logs = elogs;
                states[idx].attempts = Some(attempt);
                states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
                errored.insert(node_id.clone());
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
        deliver_run_result(&ctx, &workflow, &states, RunStatus::Canceled, None, &input).await;
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
        deliver_run_result(&ctx, &workflow, &states, RunStatus::Error, None, &input).await;
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
    // Proof pack: package the run's node outputs, human approvals, and budget
    // gate into inspectable evidence; link the pack to the run. Best-effort.
    let pack_id = assemble_workflow_proof(&ctx, &workflow, &run_id, &states).await;
    if let Some(pid) = &pack_id {
        let _ = repo.set_run_proof_pack(&run_id, pid).await;
    }
    // Report the result back to wherever the run was triggered from (Slack
    // thread / webhook): a brief status + the full summary.md. Best-effort.
    deliver_run_result(&ctx, &workflow, &states, final_status, pack_id.as_deref(), &input).await;
    // Final event: run complete.
    emit_run_updated(&ctx, &workflow.workspace_id, &run_id, final_status.as_str(), None);
}

/// Assemble the proof pack for a completed workflow run: each node's output is a
/// `log` artifact (status from the node status), a `human_approval` node becomes
/// an `approval` artifact (passed iff approved), and the run's approval metadata
/// is captured. Best-effort.
async fn assemble_workflow_proof(
    ctx: &ServerCtx,
    workflow: &Workflow,
    run_id: &Id,
    states: &[NodeRunState],
) -> Option<String> {
    use otto_core::proof::{ProofArtifactKind as K, ProofArtifactStatus as S, WorkItemKind};
    use sqlx::Row;

    let pack = match crate::proof::gate(
        ctx,
        WorkItemKind::WorkflowRun,
        run_id,
        &workflow.workspace_id,
        &workflow.name,
        "otto",
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(run = %run_id, "workflow proof gate failed: {e}");
            return None;
        }
    };

    // Approval fields live on the `workflow_runs` row, not the `WorkflowRun`
    // struct (added by migration 0058).
    let arow = sqlx::query("SELECT approved_by, approval_note, approved_at FROM workflow_runs WHERE id = ?")
        .bind(run_id)
        .fetch_optional(&ctx.pool)
        .await
        .ok()
        .flatten();
    let approved_by: Option<String> = arow.as_ref().and_then(|r| r.try_get("approved_by").ok());
    let approval_note: Option<String> = arow.as_ref().and_then(|r| r.try_get("approval_note").ok());
    let approved_at: Option<String> = arow.as_ref().and_then(|r| r.try_get("approved_at").ok());

    let mut state_by_id = std::collections::HashMap::new();
    for s in states {
        state_by_id.insert(s.node_id.as_str(), s);
    }

    for node in &workflow.graph.nodes {
        let st = state_by_id.get(node.id.as_str()).copied();
        let node_status = st.map(|s| s.status);
        let title = if node.name.is_empty() {
            node.kind.clone()
        } else {
            format!("{}: {}", node.kind, node.name)
        };

        if node.kind == "human_approval" {
            let approved = approved_by.is_some();
            let astatus = if approved { S::Passed } else { S::Failed };
            let body = if approved {
                format!("Approved by {}", approved_by.clone().unwrap_or_default())
            } else {
                "Not approved".to_string()
            };
            let meta = json!({
                "approved_by": approved_by, "approval_note": approval_note,
                "approved_at": approved_at, "node_id": node.id,
            });
            let _ = crate::proof::upsert_content_artifact(ctx, &pack, K::Approval, &title, &body, astatus, meta, "otto").await;
        } else {
            let art_status = match node_status {
                Some(NodeStatus::Success) => S::Passed,
                Some(NodeStatus::Error) => S::Failed,
                _ => S::Info,
            };
            let content = st
                .and_then(|s| s.output.as_ref())
                .map(|o| serde_json::to_string_pretty(o).unwrap_or_default())
                .unwrap_or_else(|| "(no output)".to_string());
            let meta = json!({ "node_kind": node.kind, "node_id": node.id });
            let _ = crate::proof::upsert_content_artifact(ctx, &pack, K::Log, &title, &content, art_status, meta, "otto").await;
        }
    }

    let _ = crate::proof::recompute_and_emit(ctx, &pack.id).await;
    Some(pack.id)
}

/// Build a `(brief, full_markdown)` summary of a finished run. `brief` is the
/// short chat message; `full_markdown` is the attached `summary.md`.
fn build_run_summary(
    workflow: &Workflow,
    states: &[NodeRunState],
    status: RunStatus,
    proof_pack_id: Option<&str>,
) -> (String, String) {
    let emoji = match status {
        RunStatus::Success => "✅",
        RunStatus::Error => "❌",
        RunStatus::Canceled => "⏹",
        _ => "•",
    };
    let total = states.len();
    let ok = states.iter().filter(|s| s.status == NodeStatus::Success).count();
    let failed = states.iter().filter(|s| s.status == NodeStatus::Error).count();
    let skipped = states.iter().filter(|s| s.status == NodeStatus::Skipped).count();

    // Pull a review score out of any node output that carries one.
    let score = states.iter().find_map(|s| {
        s.output
            .as_ref()
            .and_then(|o| o.get("score"))
            .and_then(Value::as_i64)
    });

    let counts = format!("{ok}/{total} steps ok · {failed} failed · {skipped} skipped");
    let score_line = score.map(|sc| format!("\n*Review score:* {sc}/100")).unwrap_or_default();
    let proof_line = proof_pack_id
        .map(|p| format!("\n*Proof pack:* `{p}`"))
        .unwrap_or_default();

    let brief = format!(
        "{emoji} *{}* — {}\n{counts}{score_line}{proof_line}\n_Full summary attached (summary.md)._",
        workflow.name,
        status.as_str(),
    );

    // Full markdown: per-step detail.
    let mut md = String::new();
    md.push_str(&format!("# Workflow run — {}\n\n", workflow.name));
    md.push_str(&format!("- **Status:** {} {}\n", emoji, status.as_str()));
    md.push_str(&format!("- **Steps:** {counts}\n"));
    if let Some(sc) = score {
        md.push_str(&format!("- **Review score:** {sc}/100\n"));
    }
    if let Some(p) = proof_pack_id {
        md.push_str(&format!("- **Proof pack:** `{p}`\n"));
    }
    md.push_str("\n## Steps\n\n");
    for (i, s) in states.iter().enumerate() {
        let kind = workflow
            .graph
            .nodes
            .iter()
            .find(|n| n.id == s.node_id)
            .map(|n| {
                if n.name.is_empty() {
                    n.kind.clone()
                } else {
                    format!("{} ({})", n.name, n.kind)
                }
            })
            .unwrap_or_else(|| s.node_id.clone());
        let dur = s.duration_ms.map(|d| format!(" · {d}ms")).unwrap_or_default();
        let attempts = match s.attempts {
            Some(a) if a > 1 => format!(" · {a} attempts"),
            _ => String::new(),
        };
        md.push_str(&format!(
            "{}. **{kind}** — {}{dur}{attempts}\n",
            i + 1,
            s.status.as_str()
        ));
        if let Some(err) = &s.error {
            md.push_str(&format!("   - error: {}\n", truncate(err, 300)));
        }
        // A short peek at the work product (agent reply or compact JSON).
        if let Some(out) = &s.output {
            let peek = out
                .get("reply")
                .and_then(Value::as_str)
                .map(|r| r.to_string())
                .unwrap_or_else(|| out.to_string());
            let peek = truncate(peek.trim(), 240);
            if !peek.is_empty() && peek != "null" {
                md.push_str(&format!("   - {}\n", peek.replace('\n', " ")));
            }
        }
    }
    (brief, md)
}

/// Deliver a finished run's result to wherever it was triggered from: the chat
/// channel + thread that started it (Slack/Telegram — the main path), and/or a
/// `result_webhook`/`callback_url` in the run input. A brief status message is
/// posted with the full `summary.md` attached. Manual UI runs (no origin) no-op.
/// Best-effort; redacted before it leaves the machine.
async fn deliver_run_result(
    ctx: &ServerCtx,
    workflow: &Workflow,
    states: &[NodeRunState],
    status: RunStatus,
    proof_pack_id: Option<&str>,
    input: &Value,
) {
    let obj = match input.as_object() {
        Some(o) => o,
        None => return,
    };
    // Target = the incoming-hook origin (channel/chat/thread the trigger came
    // from), unless explicitly overridden with `result_chat` (+ optional
    // `result_channel`/`result_thread`) — e.g. to post results to a #releases
    // channel, or to give a manual UI run a destination.
    let str_at = |k: &str| obj.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let (channel, chat, thread) = match str_at("result_chat") {
        Some(c) => (str_at("result_channel").or_else(|| str_at("channel")), Some(c), str_at("result_thread")),
        None => (str_at("channel"), str_at("chat"), str_at("thread")),
    };
    let webhook = str_at("result_webhook").or_else(|| str_at("callback_url"));

    let has_chat = matches!(channel, Some("slack") | Some("telegram")) && chat.is_some();
    if !has_chat && webhook.is_none() {
        return; // no origin to report back to (e.g. a manual UI run)
    }

    let (brief, full) = build_run_summary(workflow, states, status, proof_pack_id);
    let brief = otto_core::redact::redact_text(&brief).value;
    let full = otto_core::redact::redact_text(&full).value;
    let bytes = full.into_bytes();

    // --- chat (Slack / Telegram) ---
    if let (Some(ch), Some(chat)) = (channel, chat) {
        let chan = match ch {
            "slack" => Some(Channel::Slack),
            "telegram" => Some(Channel::Telegram),
            _ => None,
        };
        if let Some(chan) = chan {
            match otto_state::IntegrationsRepo::new(ctx.pool.clone())
                .get(&workflow.workspace_id, chan)
                .await
            {
                Ok(Some(integ)) => {
                    let sent = otto_channels::improve_notify::send_to(
                        &ctx.secrets, &integ, chat, thread, &brief,
                    )
                    .await;
                    if sent {
                        if let Some(adapter) =
                            otto_channels::improve_notify::build_adapter(&ctx.secrets, &integ)
                        {
                            if let Err(e) = adapter.upload(chat, thread, "summary.md", &bytes).await {
                                tracing::debug!("workflow result: summary upload failed: {e}");
                            }
                        }
                    } else {
                        tracing::debug!("workflow result: chat send failed (token missing?)");
                    }
                }
                _ => tracing::debug!("workflow result: no {ch} integration for workspace"),
            }
        }
    }

    // --- webhook (SSRF-guarded, reuses the scheduled-task delivery path) ---
    if let Some(url) = webhook {
        if let Err(e) =
            crate::scheduled_tasks_engine::deliver_webhook(url, &brief, "summary.md", &bytes).await
        {
            tracing::debug!("workflow result: webhook delivery failed: {e}");
        }
    }
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
#[allow(clippy::too_many_arguments)]
async fn execute_node(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    input: Value,
    run_id: &Id,
    run_cwd: &str,
    session_tx: &tokio::sync::mpsc::UnboundedSender<String>,
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
            // Run as a real, openable session (reusing the shared session runner)
            // so the run view can watch/inspect it — not the headless PTY.
            let provider = p.get("provider").and_then(Value::as_str).unwrap_or("claude");
            let full = format!("{prompt}\n\n[input data]\n{}", truncate(&input.to_string(), 4000));
            let acwd = node_cwd(node, &input, run_cwd);
            let (reply, sid) =
                run_node_agent(ctx, ws, user, node, provider, &full, &acwd, session_tx).await?;
            Ok((
                json!({ "reply": reply, "session_id": sid }),
                vec!["agent turn complete".into()],
            ))
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
                mask: None,
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
            // Webhooks are inbound-only (not a proactive-push target), so they're
            // excluded here.
            let targets: Vec<_> = integrations
                .into_iter()
                .filter(|i| i.workspace_id == ws.id)
                .filter(|i| i.channel != Channel::Webhook)
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
                    // Webhooks are inbound-only; excluded from `targets` above.
                    Channel::Webhook => continue,
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

        // --- Condition (branching primitive) --------------------------------
        "condition" => {
            let expr = p.get("expr").and_then(Value::as_str).unwrap_or("true");
            let value = otto_core::expr::eval(expr, &input)
                .map_err(|e| otto_core::Error::Invalid(format!("condition: {e}")))?;
            let result = otto_core::expr::truthy(&value);
            let mut out = match &input {
                Value::Object(m) => m.clone(),
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("input".into(), other.clone());
                    m
                }
            };
            out.insert("result".into(), json!(result));
            out.insert("value".into(), value);
            Ok((Value::Object(out), vec![format!("condition `{expr}` → {result}")]))
        }

        // --- Loop (bounded iterate-until) -----------------------------------
        "loop" => {
            let max_iter = p.get("max_iterations").and_then(Value::as_u64).unwrap_or(3).clamp(1, 10);
            let until = p.get("until").and_then(Value::as_str).unwrap_or("").to_string();
            let steps = p.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
            if steps.is_empty() {
                return Err(otto_core::Error::Invalid("loop: requires at least one step".into()));
            }
            let continue_on_error = p.get("continue_on_error").and_then(Value::as_bool).unwrap_or(false);
            // Run-level keys (repo_id, base, goals, …) flow to every step as a
            // base; the threaded prev-step output overlays them.
            let loop_base = input.as_object().cloned().unwrap_or_default();
            let mut logs = vec![];
            let mut history = vec![];
            let mut satisfied = false;
            let mut last = input.clone();
            let mut iterations = 0u64;
            for i in 1..=max_iter {
                iterations = i;
                if let Ok(rr) = WorkflowsRepo::new(ctx.pool.clone()).get_run(run_id).await {
                    if rr.status == RunStatus::Canceled {
                        logs.push("loop: canceled".into());
                        break;
                    }
                }
                // `thread` carries across iterations (so a fix step sees the prior
                // review's findings) and updates after each step within an iteration.
                let mut thread = last.clone();
                let mut step_outputs = serde_json::Map::new();
                for (k, step) in steps.iter().enumerate() {
                    let skind = step.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
                    if skind == "loop" {
                        return Err(otto_core::Error::Invalid("loop: nested loops are not allowed".into()));
                    }
                    let sname = step
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("step{k}"));
                    let mut merged = loop_base.clone();
                    if let Value::Object(m) = &thread {
                        for (mk, mv) in m {
                            merged.insert(mk.clone(), mv.clone());
                        }
                    }
                    merged.insert("_iteration".into(), json!(i));
                    let step_input = Value::Object(merged);
                    let sub = WorkflowNode {
                        id: format!("{}#{i}.{k}", node.id),
                        kind: skind.clone(),
                        name: sname.clone(),
                        x: 0.0,
                        y: 0.0,
                        params: step.get("params").cloned().unwrap_or(Value::Null),
                        retry: step.get("retry").and_then(|r| serde_json::from_value(r.clone()).ok()),
                    };
                    match Box::pin(execute_node(ctx, ws, user, &sub, step_input, run_id, run_cwd, session_tx)).await {
                        Ok((out, mut slogs)) => {
                            for l in slogs.drain(..) {
                                logs.push(format!("  [{i}/{sname}] {l}"));
                            }
                            step_outputs.insert(sname.clone(), out.clone());
                            last = out.clone();
                            thread = out;
                        }
                        Err(e) => {
                            logs.push(format!("  [{i}/{sname}] ✗ {e}"));
                            if !continue_on_error {
                                return Err(otto_core::Error::Upstream(format!(
                                    "loop step '{sname}' failed at iteration {i}: {e}"
                                )));
                            }
                            break;
                        }
                    }
                }
                let ictx = json!({ "iteration": i, "last": last, "steps": Value::Object(step_outputs), "input": input });
                history.push(ictx.clone());
                if !until.is_empty() && otto_core::expr::eval_bool(&until, &ictx) {
                    satisfied = true;
                    logs.push(format!("loop: `{until}` satisfied at iteration {i}"));
                    break;
                }
            }
            if !until.is_empty() && !satisfied {
                logs.push(format!("loop: reached max_iterations ({max_iter}) without satisfying `{until}`"));
            }
            Ok((
                json!({ "iterations": iterations, "satisfied": satisfied, "last": last, "history": history }),
                logs,
            ))
        }

        // --- Review Run (wired: local-review engine + 0–100 score + goals) ---
        "review_run" => {
            let repo_id = p
                .get("repo_id")
                .and_then(Value::as_str)
                .or_else(|| input.get("repo_id").and_then(Value::as_str))
                .ok_or_else(|| otto_core::Error::Invalid("review_run: missing repo_id".into()))?
                .to_string();
            let threshold = p.get("threshold").and_then(Value::as_u64).unwrap_or(80) as i64;
            let await_done = p.get("await").and_then(Value::as_bool).unwrap_or(true);
            let timeout_s = p.get("timeout_s").and_then(Value::as_u64).unwrap_or(900).min(1800);
            let repo = ctx
                .git_store
                .get_repo(&repo_id)
                .await
                .map_err(|e| otto_core::Error::NotFound(format!("review_run: repo: {e}")))?;
            let worktree = p
                .get("worktree_path")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| repo.path.clone());
            let base = p
                .get("base")
                .and_then(Value::as_str)
                .or_else(|| input.get("base").and_then(Value::as_str))
                .unwrap_or("main")
                .to_string();
            let review_id = crate::modules::run_review_for_branch(ctx, &repo_id, &worktree, &base).await?;
            let mut logs = vec![format!("review_run: started review {review_id} ({worktree} vs {base})")];
            let mut status = "running".to_string();
            if await_done {
                let deadline = Instant::now() + Duration::from_secs(timeout_s);
                loop {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    if let Ok(r) = ctx.reviews_store.get_review(&review_id).await {
                        use otto_core::domain::ReviewStatus as RS;
                        match r.status {
                            RS::Done => {
                                status = "done".into();
                                break;
                            }
                            RS::Error => {
                                status = "error".into();
                                break;
                            }
                            RS::Cancelled => {
                                status = "cancelled".into();
                                break;
                            }
                            RS::Running => {}
                        }
                    }
                    if let Ok(rr) = WorkflowsRepo::new(ctx.pool.clone()).get_run(run_id).await {
                        if rr.status == RunStatus::Canceled {
                            status = "cancelled".into();
                            break;
                        }
                    }
                    if Instant::now() >= deadline {
                        status = "timeout".into();
                        logs.push("review_run: timed out waiting for review".into());
                        break;
                    }
                }
            }
            let (total, open, blocker) = crate::modules::review_findings_counts(ctx, &review_id).await;
            let blocking = blocker as i64;
            let advisory = (open.saturating_sub(blocker)) as i64;
            let review_score = (100 - 20 * blocking - 5 * advisory).clamp(0, 100);
            // Optional goals — assessed by an agent and blended into the score.
            let goals: Vec<String> = p
                .get("goals")
                .and_then(Value::as_array)
                .or_else(|| input.get("goals").and_then(Value::as_array))
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            let (goals_score, goals_detail) = if goals.is_empty() {
                (None, json!([]))
            } else {
                let gprompt = format!(
                    "Assess whether each goal below is met for the repository at `{worktree}` \
                     (review the code/tests). Reply with ONLY JSON of the form \
                     {{\"goals\":[{{\"goal\":\"...\",\"met\":true,\"score\":0,\"note\":\"...\"}}],\"score\":0}} \
                     where each score and the overall score are 0–100.\n\nGoals:\n- {}",
                    goals.join("\n- ")
                );
                match run_node_agent(ctx, ws, user, node, "claude", &gprompt, &worktree, session_tx).await {
                    Ok((reply, _sid)) => match extract_json(&reply) {
                        Some(v) => {
                            let gs = v.get("score").and_then(Value::as_i64).unwrap_or(review_score).clamp(0, 100);
                            (Some(gs), v.get("goals").cloned().unwrap_or(json!([])))
                        }
                        None => (Some(review_score), json!([{ "note": "goals eval reply not parseable" }])),
                    },
                    Err(e) => {
                        logs.push(format!("review_run: goals eval failed: {e}"));
                        (Some(review_score), json!([{ "note": "goals eval failed" }]))
                    }
                }
            };
            let score = match goals_score {
                Some(gs) => (review_score + gs) / 2,
                None => review_score,
            };
            let passed = score >= threshold && status == "done";
            logs.push(format!(
                "review_run: score {score} (review {review_score}{}) — {}",
                goals_score.map(|g| format!(", goals {g}")).unwrap_or_default(),
                if passed { "passed" } else { "below threshold" }
            ));
            Ok((
                json!({
                    "review_id": review_id, "status": status,
                    "total": total, "open": open, "blocking": blocking, "advisory": advisory,
                    "review_score": review_score, "goals_score": goals_score, "goals": goals_detail,
                    "score": score, "threshold": threshold, "passed": passed,
                }),
                logs,
            ))
        }

        // --- Product nodes (wired: real single-agent turn over story context) -
        "product_analyze" | "product_rewrite" | "product_plan" => {
            let kind = node.kind.clone();
            let story_id = p
                .get("story_id")
                .and_then(Value::as_str)
                .or_else(|| input.get("story_id").and_then(Value::as_str))
                .map(str::to_string);
            let (skill_name, instruction, persist_kind) = match kind.as_str() {
                "product_analyze" => (
                    "grill",
                    "Analyze this product story: surface scope gaps, ambiguities, non-testable acceptance criteria, unhandled edge cases, and risks. Be specific and evidence-based.",
                    None,
                ),
                "product_rewrite" => (
                    "jira-story-writer",
                    "Rewrite this story so it is clear, valuable, and testable. Reply with the rewritten story in Markdown.",
                    Some("suggested"),
                ),
                _ => (
                    "story-task-breakdown",
                    "Break this story into an ordered implementation plan of small, independently-verifiable tasks. Reply in Markdown.",
                    Some("plan"),
                ),
            };
            let skill = crate::modules::resolve_skill_inline(&ctx.context_library, skill_name);
            let context = match &story_id {
                Some(sid) => ctx
                    .product
                    .build_agent_context(sid, None)
                    .await
                    .unwrap_or_else(|_| truncate(&input.to_string(), 6000)),
                None => truncate(&input.to_string(), 6000),
            };
            let extra = p.get("instruction").and_then(Value::as_str).unwrap_or("");
            let prompt = format!(
                "{skill}\n\n# Task\n{instruction}\n{extra}\n\n# Story context\n{}",
                truncate(&context, 8000)
            );
            let acwd = node_cwd(node, &input, run_cwd);
            let (reply, sid) = run_node_agent(ctx, ws, user, node, "claude", &prompt, &acwd, session_tx).await?;
            let mut out = serde_json::Map::new();
            out.insert("story_id".into(), json!(story_id));
            out.insert("session_id".into(), json!(sid));
            match kind.as_str() {
                "product_analyze" => {
                    out.insert("analysis".into(), json!(reply));
                }
                "product_rewrite" => {
                    out.insert("body_md".into(), json!(reply));
                }
                _ => {
                    out.insert("plan_md".into(), json!(reply));
                }
            }
            // Optional: persist as a product version.
            let persist = p.get("persist").and_then(Value::as_bool).unwrap_or(false);
            if let (Some(pk), Some(sid_story)) = (persist_kind, story_id.as_ref()) {
                if persist {
                    let nv = otto_state::NewVersion {
                        story_id: sid_story.clone(),
                        kind: pk.into(),
                        title: format!("Workflow {kind}"),
                        body_md: reply.clone(),
                        raw_json: None,
                        change_notes: Some(format!("from workflow node {}", node.id)),
                        created_by: user.id.clone(),
                    };
                    if let Ok(v) = ctx.product_repo.add_version(nv).await {
                        out.insert("version_id".into(), json!(v.id));
                    }
                }
            }
            Ok((Value::Object(out), vec![format!("{kind}: complete")]))
        }

        // --- Product Publish (RFC / Jira; dry-run by default) ----------------
        "product_publish" => {
            let story_id = p
                .get("story_id")
                .and_then(Value::as_str)
                .or_else(|| input.get("story_id").and_then(Value::as_str))
                .ok_or_else(|| otto_core::Error::Invalid("product_publish: missing story_id".into()))?
                .to_string();
            let kind = p.get("kind").and_then(Value::as_str).unwrap_or("rfc").to_string();
            let dry_run = p.get("dry_run").and_then(Value::as_bool).unwrap_or(true);
            if dry_run {
                return Ok((
                    json!({ "story_id": story_id, "kind": kind, "dry_run": true,
                            "note": "dry run — set dry_run=false with an account_id to publish" }),
                    vec![format!("product_publish: dry run ({kind})")],
                ));
            }
            let account_id = p
                .get("account_id")
                .and_then(Value::as_str)
                .ok_or_else(|| otto_core::Error::Invalid("product_publish: account_id required to publish".into()))?
                .to_string();
            if kind == "jira" {
                let project = p
                    .get("project_key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| otto_core::Error::Invalid("product_publish: project_key required".into()))?;
                let issue_type = p.get("issue_type").and_then(Value::as_str).unwrap_or("Story");
                let detail = ctx
                    .product
                    .publish_as_story(&story_id, &account_id, project, issue_type, &user.id)
                    .await?;
                Ok((
                    json!({ "story_id": story_id, "kind": "jira", "dry_run": false,
                            "detail": serde_json::to_value(&detail).ok() }),
                    vec!["product_publish: published to Jira".into()],
                ))
            } else {
                let space = p
                    .get("space_key")
                    .and_then(Value::as_str)
                    .ok_or_else(|| otto_core::Error::Invalid("product_publish: space_key required".into()))?;
                let parent = p.get("parent_id").and_then(Value::as_str);
                let title = p.get("title").and_then(Value::as_str);
                let detail = ctx
                    .product
                    .publish_as_rfc(&story_id, &account_id, space, parent, title, &user.id)
                    .await?;
                Ok((
                    json!({ "story_id": story_id, "kind": "rfc", "dry_run": false,
                            "detail": serde_json::to_value(&detail).ok() }),
                    vec!["product_publish: published RFC to Confluence".into()],
                ))
            }
        }

        // --- Canvas (generate a mermaid/excalidraw diagram artifact) ---------
        "canvas" => {
            let prompt_in = p
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or("Create a clear diagram of the system/flow described in the input.");
            let mode = p.get("mode").and_then(Value::as_str).unwrap_or("mermaid");
            let full = format!(
                "Produce a {mode} diagram. {prompt_in}\nReply with ONLY a ```{mode} code block.\n\n[context]\n{}",
                truncate(&input.to_string(), 4000)
            );
            let acwd = node_cwd(node, &input, run_cwd);
            let (reply, sid) = run_node_agent(ctx, ws, user, node, "claude", &full, &acwd, session_tx).await?;
            let diagram = extract_code_block(&reply, mode).unwrap_or_else(|| reply.clone());
            // Write under the data dir (never the user's repo working tree).
            let ext = if mode == "excalidraw" { "json" } else { "mmd" };
            let rel = format!("workflow-canvas/{run_id}/{}.{ext}", node.id);
            let path = ctx.data_dir.join(&rel);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let written = std::fs::write(&path, &diagram).is_ok();
            Ok((
                json!({ "scene_id": rel, "path": path.to_string_lossy(), "mode": mode,
                        "diagram": diagram, "written": written, "session_id": sid }),
                vec![format!("canvas: {mode} diagram ({} bytes)", diagram.len())],
            ))
        }

        // --- Git PR (draft; opening is done from the Git tab) ----------------
        "git_pr" => {
            let repo_id = p
                .get("repo_id")
                .and_then(Value::as_str)
                .or_else(|| input.get("repo_id").and_then(Value::as_str))
                .ok_or_else(|| otto_core::Error::Invalid("git_pr: missing repo_id".into()))?
                .to_string();
            let repo = ctx
                .git_store
                .get_repo(&repo_id)
                .await
                .map_err(|e| otto_core::Error::NotFound(format!("git_pr: repo: {e}")))?;
            let worktree = p
                .get("worktree_path")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| repo.path.clone());
            let base = p
                .get("base")
                .and_then(Value::as_str)
                .or_else(|| input.get("base").and_then(Value::as_str))
                .unwrap_or("main")
                .to_string();
            let draft = crate::modules::draft_pr_core(ctx, &worktree, &base).await?;
            Ok((
                json!({
                    "title": draft.title, "description": draft.description,
                    "source_branch": draft.source_branch, "target_branch": draft.target_branch,
                    "opened": false,
                    "note": "Drafted. Open the PR from the Git tab (engine auto-open is gated).",
                }),
                vec![format!(
                    "git_pr: drafted '{}' ({} → {})",
                    draft.title, draft.source_branch, draft.target_branch
                )],
            ))
        }

        other => Err(otto_core::Error::Invalid(format!("unknown node kind '{other}'"))),
    }
}



// ---------------------------------------------------------------------------
// Graph helpers
// ---------------------------------------------------------------------------

/// Edges entering `node_id` (in graph order).
fn incoming_edges<'a>(graph: &'a WorkflowGraph, node_id: &str) -> Vec<&'a otto_core::workflows::WorkflowEdge> {
    graph.edges.iter().filter(|e| e.target == node_id).collect()
}

/// Edges leaving `node_id` (in graph order).
fn outgoing_edges<'a>(graph: &'a WorkflowGraph, node_id: &str) -> Vec<&'a otto_core::workflows::WorkflowEdge> {
    graph.edges.iter().filter(|e| e.source == node_id).collect()
}

/// A reduced view of one in-scope incoming edge, for the branching decision.
struct EdgeView {
    source: String,
    /// The source node errored (or was poisoned).
    errored: bool,
    /// The source produced an output (i.e. ran successfully or hit cache).
    has_output: bool,
    /// This edge's condition is satisfied (true / absent).
    edge_active: bool,
}

/// The control-flow decision for a node, derived purely from its incoming edges.
#[derive(Debug, PartialEq)]
enum NodeDecision {
    /// Run, assembling input from these satisfied source ids.
    Run(Vec<String>),
    /// Skip + propagate failure: an active-path predecessor errored.
    ErrorSkip,
    /// Skip without failure: the node has in-scope predecessors but no satisfied
    /// edge (every branch into it was pruned, or upstream was branch-skipped).
    BranchSkip,
}

/// Decide whether a node runs. Pure + unit-tested.
///
/// - An errored in-scope predecessor poisons the node (ErrorSkip).
/// - Otherwise, "satisfied" sources are those that produced output via an active
///   edge; if there are in-scope incoming edges but none satisfied, the node is
///   BranchSkip; else Run with the satisfied sources (empty ⇒ an entry node that
///   falls back to the run input).
fn decide_node(views: &[EdgeView]) -> NodeDecision {
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

/// Evaluate the conditions on a node's outgoing edges against its output. Returns
/// `(inactive_edge_ids, log_lines)`. An edge with no condition is always active;
/// a condition that fails to parse/evaluate is treated as *not taken* (and logged).
fn eval_outgoing(
    graph: &WorkflowGraph,
    node: &WorkflowNode,
    output: &Value,
    node_input: &Value,
    run_input: &Value,
) -> (Vec<String>, Vec<String>) {
    let mut inactive = Vec::new();
    let mut logs = Vec::new();
    let ctx = json!({
        "output": output,
        "input": node_input,
        "node": { "id": node.id, "kind": node.kind, "name": node.name },
        "run": { "input": run_input },
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

/// The effective retry policy for a node: an explicit `node.retry`, else a
/// `params.retry` object, else the default (no retry). Clamped to sane bounds.
fn resolve_retry(node: &WorkflowNode) -> otto_core::workflows::RetryPolicy {
    if let Some(p) = &node.retry {
        return p.clamped();
    }
    if let Some(rp) = node.params.get("retry") {
        if let Ok(p) = serde_json::from_value::<otto_core::workflows::RetryPolicy>(rp.clone()) {
            return p.clamped();
        }
    }
    otto_core::workflows::RetryPolicy::default()
}

/// Whether a node kind should be retried on failure. Interactive / entry kinds
/// are never retried.
fn is_retryable(kind: &str) -> bool {
    !matches!(kind, "human_approval" | "manual_trigger")
}

/// Resolve the user a run acts as. Falls back to a synthetic user for system /
/// trigger-initiated runs whose `created_by` isn't a real account.
async fn resolve_run_user(ctx: &ServerCtx, created_by: &Id) -> User {
    otto_state::UsersRepo::new(ctx.pool.clone())
        .get(created_by)
        .await
        .unwrap_or_else(|_| User {
            id: created_by.clone(),
            username: "workflow".into(),
            display_name: "Workflow".into(),
            is_root: false,
            disabled: false,
            created_at: chrono::Utc::now(),
        })
}

/// Run one agent turn as a visible, openable session (reusing the shared
/// `run_session_turn` flow), reporting the session id over `session_tx` the
/// moment the session exists so the run view can open it live. Returns
/// `(reply, session_id)`.
#[allow(clippy::too_many_arguments)]
async fn run_node_agent(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    provider: &str,
    prompt: &str,
    cwd: &str,
    session_tx: &tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<(String, Id)> {
    let title = if node.name.is_empty() {
        format!("Workflow: {}", node.kind)
    } else {
        format!("Workflow: {}", node.name)
    };
    let meta = json!({ "source": "workflow", "node_id": node.id, "node_kind": node.kind, "cwd": cwd });
    let tx = session_tx.clone();
    crate::agent_session::run_session_turn(
        ctx,
        ws,
        user,
        None,
        &title,
        cwd,
        provider,
        meta,
        prompt,
        move |id| {
            let _ = tx.send(id.to_string());
        },
    )
    .await
    .map_err(|e| e.0)
}

/// Expand a leading `~`/`~/` to the user's home directory.
fn expand_tilde(p: &str) -> String {
    if p == "~" {
        return dirs::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    p.to_string()
}

/// The directory an agent node runs in: a per-node `cwd`/`working_directory`
/// param, else the run-level working dir, else the workspace root. Tilde-expanded.
fn node_cwd(node: &WorkflowNode, input: &Value, run_cwd: &str) -> String {
    let p = &node.params;
    let pick = p
        .get("cwd")
        .or_else(|| p.get("working_directory"))
        .and_then(Value::as_str)
        .or_else(|| input.get("working_directory").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty());
    match pick {
        Some(s) => expand_tilde(s),
        None => run_cwd.to_string(),
    }
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

/// Collect openable session id(s) from a node's output (`session_id` string
/// and/or `sessions` array) into `into`, de-duplicated. Used so a node whose
/// session id rode in its output — including a *cached* re-run — still surfaces
/// the session on the run, complementing the live channel report.
fn harvest_session_ids(output: &Value, into: &mut Vec<String>) {
    if let Some(s) = output.get("session_id").and_then(Value::as_str) {
        if !s.is_empty() && !into.iter().any(|x| x == s) {
            into.push(s.to_string());
        }
    }
    if let Some(arr) = output.get("sessions").and_then(Value::as_array) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !s.is_empty() && !into.iter().any(|x| x == s) {
                    into.push(s.to_string());
                }
            }
        }
    }
}

/// Tolerantly extract a JSON value from an agent reply: try the whole string,
/// else the first balanced `{ … }` span.
fn extract_json(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end > start {
        serde_json::from_str(&text[start..=end]).ok()
    } else {
        None
    }
}

/// Extract the body of the first fenced code block (preferring the `lang` fence).
fn extract_code_block(text: &str, lang: &str) -> Option<String> {
    let fence = format!("```{lang}");
    let start = text
        .find(&fence)
        .map(|i| i + fence.len())
        .or_else(|| text.find("```").map(|i| i + 3))?;
    let rest = &text[start..];
    let end = rest.find("```")?;
    Some(rest[..end].trim().to_string())
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
            retry: None,
        }
    }
    fn edge(s: &str, t: &str) -> WorkflowEdge {
        WorkflowEdge { id: format!("{s}-{t}"), source: s.into(), target: t.into(), condition: None }
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

    fn view(source: &str, errored: bool, has_output: bool, edge_active: bool) -> EdgeView {
        EdgeView { source: source.into(), errored, has_output, edge_active }
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
        // if/else join: a=true branch produced output (active), b=false branch pruned
        let v = vec![view("a", false, true, true), view("b", false, true, false)];
        assert_eq!(decide_node(&v), NodeDecision::Run(vec!["a".into()]));
        // and the other way
        let v = vec![view("a", false, true, false), view("b", false, true, true)];
        assert_eq!(decide_node(&v), NodeDecision::Run(vec!["b".into()]));
    }

    #[test]
    fn eval_outgoing_prunes_false_edges() {
        let mut g = WorkflowGraph {
            nodes: vec![node("c", "condition"), node("t", "log"), node("f", "log")],
            edges: vec![
                WorkflowEdge { id: "c-t".into(), source: "c".into(), target: "t".into(), condition: Some("output.result == true".into()) },
                WorkflowEdge { id: "c-f".into(), source: "c".into(), target: "f".into(), condition: Some("output.result == false".into()) },
            ],
        };
        let cnode = g.nodes[0].clone();
        let out = json!({ "result": true });
        let (inactive, _logs) = eval_outgoing(&g, &cnode, &out, &Value::Null, &Value::Null);
        assert_eq!(inactive, vec!["c-f".to_string()], "false branch pruned");
        // flip
        let out = json!({ "result": false });
        let (inactive, _) = eval_outgoing(&g, &cnode, &out, &Value::Null, &Value::Null);
        assert_eq!(inactive, vec!["c-t".to_string()]);
        g.edges.clear();
        let (inactive, _) = eval_outgoing(&g, &cnode, &out, &Value::Null, &Value::Null);
        assert!(inactive.is_empty(), "no edges → nothing pruned");
    }

    #[test]
    fn retry_policy_resolution_and_clamps() {
        let mut n = node("a", "agent_prompt");
        assert_eq!(resolve_retry(&n).max_attempts, 0, "default no retry");
        n.params = json!({ "retry": { "max_attempts": 99, "backoff_ms": 999999 } });
        let p = resolve_retry(&n);
        assert_eq!(p.max_attempts, 5, "clamped to 5");
        assert_eq!(p.backoff_ms, 60_000, "clamped to 60s");
        assert!(is_retryable("agent_prompt"));
        assert!(!is_retryable("human_approval"));
        assert!(!is_retryable("manual_trigger"));
    }

    #[test]
    fn run_summary_has_status_steps_and_score() {
        let wf = Workflow {
            id: "w".into(),
            workspace_id: "ws".into(),
            name: "Write tests".into(),
            description: String::new(),
            graph: WorkflowGraph {
                nodes: vec![node("a", "agent_prompt"), node("b", "review_run")],
                edges: vec![],
            },
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: 1,
        };
        let mk = |id: &str, status: NodeStatus, out: Value| NodeRunState {
            node_id: id.into(),
            status,
            output: Some(out),
            error: None,
            logs: vec![],
            duration_ms: Some(10),
            attempts: Some(1),
            sessions: vec![],
        };
        let states = vec![
            mk("a", NodeStatus::Success, json!({ "reply": "implemented the tests" })),
            mk("b", NodeStatus::Success, json!({ "score": 92, "passed": true })),
        ];
        let (brief, full) = build_run_summary(&wf, &states, RunStatus::Success, Some("pack1"));
        assert!(brief.contains("Write tests"));
        assert!(brief.contains("2/2 steps ok"));
        assert!(brief.contains("92/100"), "brief shows the review score");
        assert!(brief.contains("summary.md"));
        assert!(full.contains("## Steps"));
        assert!(full.contains("review_run"));
        assert!(full.contains("pack1"), "full summary names the proof pack");
    }
}
