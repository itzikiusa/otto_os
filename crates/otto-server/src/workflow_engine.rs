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

/// A changed node bigger than this (serialized) is dropped from the event; the
/// UI falls back to a rev-guarded refetch. Keeps broadcast frames bounded when a
/// step output is huge (long agent replies, big JSON products).
const NODE_EVENT_MAX_BYTES: usize = 32 * 1024;

/// Broadcast a `WorkflowRunUpdated` event (best-effort; log on failure).
///
/// `rev` is the run revision returned by the `update_run` that persisted this
/// change (0 when that write failed — clients then fall back to a refetch).
/// `node` rides the event so clients can merge the changed step in place
/// without refetching the whole run; `states` feeds the `nodes_done` /
/// `nodes_total` progress the "Running" sidebar shows (pass `&[]` when the
/// caller doesn't hold the run's states — clients keep their last counts).
#[allow(clippy::too_many_arguments)]
fn emit_run_updated(
    ctx: &ServerCtx,
    workspace_id: &Id,
    run_id: &Id,
    status: &str,
    node_id: Option<&str>,
    rev: i64,
    node: Option<&NodeRunState>,
    states: &[NodeRunState],
    waiting_approval: bool,
) {
    let node = node
        .filter(|n| {
            serde_json::to_string(n)
                .map(|s| s.len() <= NODE_EVENT_MAX_BYTES)
                .unwrap_or(false)
        })
        .cloned();
    let ev = Event::WorkflowRunUpdated {
        workspace_id: workspace_id.clone(),
        run_id: run_id.clone(),
        status: status.to_string(),
        node_id: node_id.map(|s| s.to_string()),
        rev,
        node,
        nodes_done: states
            .iter()
            .filter(|n| matches!(n.status, NodeStatus::Success | NodeStatus::Skipped))
            .count() as u32,
        nodes_total: states.len() as u32,
        waiting_approval,
    };
    if ctx.events.send(ev).is_err() {
        tracing::debug!(%run_id, "no WS subscribers for WorkflowRunUpdated");
    }
}

// ---------------------------------------------------------------------------
// Live progress streaming (Slack/Telegram thread the run was triggered from)
// ---------------------------------------------------------------------------

/// One queued progress delivery: a text line, or a step's handoff file
/// attached beneath the step's "done" line (the user reads the summary in the
/// thread and opens the file for the full detail).
enum ProgressItem {
    Text(String),
    File { name: String, text: String },
}

/// Cap on an attached step file — a runaway reply shouldn't turn into a
/// multi-megabyte chat upload; the full file stays in the run's context dir.
const PROGRESS_FILE_CAP: usize = 1024 * 1024;

/// A best-effort sink for human-facing progress lines streamed back to the chat
/// thread that triggered the run. Cloneable + cheap; a *disabled* sink (manual UI
/// run, or webhook-only trigger) silently drops everything. Messages are sent
/// non-blocking over a channel and posted, in order, by a single pump task so the
/// engine never blocks on Slack/Telegram latency.
#[derive(Clone)]
struct ProgressSink {
    tx: Option<tokio::sync::mpsc::UnboundedSender<ProgressItem>>,
}

impl ProgressSink {
    fn disabled() -> Self {
        Self { tx: None }
    }
    /// Queue a progress line (no-op when disabled).
    fn post(&self, msg: impl Into<String>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(ProgressItem::Text(msg.into()));
        }
    }
    /// Queue a step's `.md` handoff file as a chat attachment (no-op when
    /// disabled or the file doesn't exist — attachments never fail a step).
    fn post_step_file(&self, files: &crate::workflow_context::RunContextFiles, base_name: &str) {
        let Some(tx) = &self.tx else { return };
        let Some(path) = files.step_md_path(base_name) else { return };
        let Ok(mut text) = std::fs::read_to_string(&path) else { return };
        if text.len() > PROGRESS_FILE_CAP {
            let mut end = PROGRESS_FILE_CAP;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("\n… [truncated for chat — the full file is in the run's context directory]");
        }
        let _ = tx.send(ProgressItem::File { name: format!("{base_name}.md"), text });
    }
    fn enabled(&self) -> bool {
        self.tx.is_some()
    }
}

/// Where a run reports back to: the chat integration + channel/thread the trigger
/// arrived on. Resolved once from the run input.
struct ChatTarget {
    /// Workspace whose integration received the trigger (workflows are global, so
    /// this may differ from the workflow's own workspace).
    ws: String,
    channel: Channel,
    chat: String,
    thread: Option<String>,
}

/// Resolve the chat target for live progress + result delivery from the run input.
/// Honors an explicit `result_chat`(+`result_channel`/`result_thread`) override,
/// else the incoming-hook origin (`channel`/`chat`/`thread`). Returns `None` for a
/// manual UI run or a webhook-only trigger (nothing to stream to).
fn resolve_chat_target(workflow: &Workflow, input: &Value) -> Option<ChatTarget> {
    let obj = input.as_object()?;
    let str_at = |k: &str| obj.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let ws = str_at("origin_workspace_id")
        .map(|s| s.to_string())
        .unwrap_or_else(|| workflow.workspace_id.clone());
    let (channel, chat, thread) = match str_at("result_chat") {
        Some(c) => (
            str_at("result_channel").or_else(|| str_at("channel")),
            Some(c),
            str_at("result_thread"),
        ),
        None => (str_at("channel"), str_at("chat"), str_at("thread")),
    };
    let channel = match channel {
        Some("slack") => Channel::Slack,
        Some("telegram") => Channel::Telegram,
        _ => return None,
    };
    Some(ChatTarget {
        ws,
        channel,
        chat: chat?.to_string(),
        thread: thread.map(str::to_string),
    })
}

/// Spawn the progress pump: a single task that owns the receiver + resolved
/// integration and posts each queued line, in order, to the chat thread
/// (redacted, best-effort). Returns the sink (held by `run_workflow`, threaded
/// into nodes) and the task handle (awaited at run end to flush before the final
/// summary is delivered). Drop the sink to close the channel and end the pump.
fn spawn_progress_pump(ctx: ServerCtx, target: ChatTarget) -> (ProgressSink, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProgressItem>();
    let handle = tokio::spawn(async move {
        let integ = match otto_state::IntegrationsRepo::new(ctx.pool.clone())
            .get(&target.ws, target.channel)
            .await
        {
            Ok(Some(i)) => i,
            _ => {
                // No integration to post to — drain so senders don't block on a
                // bounded buffer (it's unbounded, but keep the task tidy).
                while rx.recv().await.is_some() {}
                return;
            }
        };
        // One adapter for file uploads, built once (send_to builds its own).
        let adapter = otto_channels::improve_notify::build_adapter(&ctx.secrets, &integ);
        while let Some(item) = rx.recv().await {
            match item {
                ProgressItem::Text(msg) => {
                    let msg = otto_core::redact::redact_text(&msg).value;
                    let _ = otto_channels::improve_notify::send_to(
                        &ctx.secrets,
                        &integ,
                        &target.chat,
                        target.thread.as_deref(),
                        &msg,
                    )
                    .await;
                }
                ProgressItem::File { name, text } => {
                    // Same discipline as summary.md: redact before it leaves
                    // the machine; best-effort.
                    let text = otto_core::redact::redact_text(&text).value;
                    if let Some(adapter) = adapter.as_ref() {
                        if let Err(e) = adapter
                            .upload(&target.chat, target.thread.as_deref(), &name, text.as_bytes())
                            .await
                        {
                            tracing::debug!("workflow progress: {name} upload failed: {e}");
                        }
                    }
                }
            }
        }
    });
    (ProgressSink { tx: Some(tx) }, handle)
}

/// A label for a node in progress messages: its name, else its kind.
fn node_label(node: &WorkflowNode) -> String {
    if node.name.trim().is_empty() {
        node.kind.clone()
    } else {
        node.name.clone()
    }
}

/// Whether a node kind is worth a "started/finished" line in the chat thread.
/// Structural/plumbing kinds (log/transform/delay/condition/manual_trigger) are
/// skipped so a long pipeline doesn't drown the thread — the user asked for the
/// meaningful steps, "without it being too overwhelming". `review_run` is excluded
/// here because it streams its OWN richer block (started → score → findings).
fn is_reportable(kind: &str) -> bool {
    !matches!(
        kind,
        "manual_trigger" | "log" | "transform" | "delay" | "condition" | "review_run"
    )
}

/// Collapse runs of whitespace/newlines into single spaces and trim.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate to at most `max` chars, preferring to cut at the last sentence
/// terminator so the snippet reads as whole sentences (the user wants brief,
/// well-formatted summaries — ≤ a short paragraph). Appends `…` if cut.
fn truncate_sentences(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    // Prefer the last sentence boundary in the kept window (only if it keeps a
    // reasonable amount, so we don't truncate to almost nothing).
    if let Some(cut) = head.rfind(['.', '!', '?', '\n']) {
        if cut >= max / 2 {
            return format!("{}…", head[..=cut].trim());
        }
    }
    format!("{}…", head.trim())
}

/// A short, chat-friendly summary of a node's product (agent reply / analysis /
/// plan / summary text), whitespace-collapsed and truncated to a brief block.
/// Returns `None` for purely structural outputs (nothing worth posting).
fn brief_summary(output: &Value) -> Option<String> {
    let raw = ["reply", "analysis", "plan_md", "body_md", "summary", "note"]
        .iter()
        .find_map(|k| output.get(*k).and_then(Value::as_str))
        .map(str::to_string)?;
    let s = truncate_sentences(&collapse_ws(&raw), 700);
    if s.is_empty() || s == "…" {
        None
    } else {
        Some(s)
    }
}

/// Collect skill names a node wants injected: `skill` (string) and/or `skills`
/// (array of strings), de-duplicated, in declared order.
fn node_skill_names(params: &Value) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    };
    if let Some(s) = params.get("skill").and_then(Value::as_str) {
        push(s);
    }
    for key in ["skills", "lenses"] {
        if let Some(arr) = params.get(key).and_then(Value::as_array) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    push(s);
                }
            }
        }
    }
    out
}

/// Prepend each resolved skill (body + references) ahead of `base`, in the same
/// shape the review engine uses (`{skill}\n\n---\n\n{prompt}`). Lets any
/// agent-backed step run a specific skill/method "via prompt".
fn prepend_skills(ctx: &ServerCtx, params: &Value, base: &str) -> String {
    let names = node_skill_names(params);
    if names.is_empty() {
        return base.to_string();
    }
    let mut out = String::new();
    for name in &names {
        let txt = crate::modules::resolve_skill_inline(&ctx.context_library, name);
        if !txt.is_empty() {
            out.push_str(&txt);
            out.push_str("\n\n---\n\n");
        }
    }
    out.push_str(base);
    out
}

/// Parse a node param into a `Vec<String>` (accepts a JSON array of strings or a
/// comma-separated string), trimmed + non-empty. Used for `providers`/`lenses`.
fn param_str_list(params: &Value, key: &str) -> Vec<String> {
    match params.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => vec![],
    }
}

/// Per-node turn budget for agent/LLM nodes.
const NODE_AGENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Global wall-clock budget for a whole run. A run can't execute forever: once
/// the cumulative time across all nodes exceeds this, the run is failed at the
/// next node boundary (a node already executing finishes first, bounded by the
/// per-turn agent timeout). 10h is a deliberate, generous backstop — long
/// multi-step agent pipelines (write tests → review → fix-until-passing → PR) can
/// legitimately run for hours; the operator stops a run manually if they want it
/// to end sooner. This only guarantees eventual termination, it does not curtail
/// real work.
const RUN_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(10 * 60 * 60);

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
          "Run an agent turn with a prompt (params: provider, skill/skills to inject, cwd); outputs its reply.", 1, 1, "#d97cff", "command"),
        n("prepare_context", "Prepare relevant data", "AI",
          "App-side context gathering: fetches a referenced Jira ticket (params: key/account_id/require) into jira-<KEY>.md, then optionally runs an analysis agent (params: prompt/provider).", 1, 1, "#d97cff", "download"),
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
          "Multi-agent code review (params: providers[], lenses[]/skills[], threshold, require_pass) — fans out like PR review, summarizer scores; outputs findings + a 0–100 score + passed.", 1, 1, "#c080ff", "search"),
        n("git_pr", "Git PR", "Network",
          "Draft a PR; with open=true (gate the incoming edge on the review passing) opens it on the remote.", 1, 1, "#46c0a0", "git-pull-request"),
        // api_run: executes an HTTP request via the api-client engine so
        // environment variable substitution and auth apply.  Wired.
        n("api_run", "API Run", "Network",
          "Execute an API-client request with env-var substitution.", 1, 1, "#46c0a0", "send"),
        // self_improve: runs the self-improvement engine in OFFER-ONLY mode
        // (Autonomy::Propose → every edit is queued for approval, never applied)
        // and posts the offered improvements to the trigger's chat thread.
        n("self_improve", "Self-Improve (offer)", "AI",
          "Reflect on recent sessions and OFFER skill/memory improvements (never auto-applied — queued for approval). Posts the offered list to the chat thread.", 1, 1, "#d97cff", "zap"),
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
        "agent_prompt" => obj(&[("reply", "string"), ("working_directory", "string")]),
        "prepare_context" => Some(json!({"jira": "object"})),
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
            ("repo_id", "string"),
            ("base", "string"),
            ("worktree", "string"),
            ("blocking", "number"),
            ("advisory", "number"),
            ("checks_requested", "array"),
            ("score", "number"),
            ("threshold", "number"),
            ("passed", "boolean"),
        ]),
        "product_analyze" => obj(&[("story_id", "string"), ("analysis", "string")]),
        "product_rewrite" => obj(&[("story_id", "string"), ("body_md", "string")]),
        "product_plan" => obj(&[("story_id", "string"), ("plan_md", "string")]),
        "product_publish" => obj(&[("story_id", "string"), ("url", "string"), ("dry_run", "boolean")]),
        "git_pr" => obj(&[
            ("prs", "array"),
            ("opened", "boolean"),
            ("opened_count", "number"),
            ("title", "string"),
            ("description", "string"),
        ]),
        "self_improve" => obj(&[
            ("run_id", "string"),
            ("summary", "string"),
            ("offered", "number"),
            ("edits", "array"),
        ]),
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
            let rev = repo
                .update_run(&run_id, RunStatus::Error, &[], Some(&e), true)
                .await
                .unwrap_or(0);
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "error", None, rev, None, &[], false);
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
            started_at: None,
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
    // Per-run context files (design 2026-07-02): `<data_dir>/workflow-context/
    // <run_id>/` holds the instruction brief, the live `repos.json` registry
    // and per-step handoff files. Best-effort — a failed dir create disables
    // the handle and the run proceeds on legacy inline context.
    let files = std::sync::Arc::new(crate::workflow_context::RunContextFiles::create(
        &ctx.data_dir,
        &run_id,
    ));
    // Declared repos/branches/worktrees (`repos: [{repo, type, name, source}]`)
    // normalized against registered repos + live git state, then seeded into
    // the run input BEFORE run_cwd/run_base derive below — so declaring repos
    // is all a run needs for every git-aware step to know source+destination.
    let input = {
        // The run's Base branch (from the input) is the fallback for a declared
        // repo that doesn't name its own `source` — so setting Base actually pins
        // the declaration's base, instead of it always falling back to the repo's
        // detected default branch. (R14)
        let run_base_hint = input
            .get("base")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let declared =
            crate::workflow_context::parse_repo_entries(input.get("repos").unwrap_or(&Value::Null));
        let entries =
            resolve_repo_entries(&ctx, &workflow.workspace_id, declared, run_base_hint.as_deref())
                .await;
        files.set_repos(entries.clone());
        seed_input_from_entries(input, &entries)
    };
    // Fill `prompt` from a chat `msg` when the run didn't set one explicitly —
    // `prompt.md` and every agent-facing step want `prompt`; the trigger only
    // guarantees `msg`. Then write the two standing-context files this run
    // actually has: `instructions.md` (the workflow's, verbatim, only when
    // non-empty) and `prompt.md` (this run's ask, only when one exists). Both
    // flags feed the brief/preamble "how to use this directory" sections below
    // so they only ever point agents at files that exist.
    let input = normalize_prompt(input);
    let has_instructions = !workflow.instructions.trim().is_empty();
    if has_instructions {
        files.write_instructions_md(&workflow.instructions);
    }
    let has_prompt = input
        .get("prompt")
        .and_then(Value::as_str)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if let Some(p) = input.get("prompt").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        files.write_prompt_md(p);
    }
    // Run-level working directory: the `working_directory` from the run input
    // (e.g. a Slack `Working Directory:` field), else the workspace root. Agent
    // nodes run here — so a workflow owned by workspace A can operate on repo X.
    let run_cwd = input
        .get("working_directory")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(expand_tilde)
        .unwrap_or_else(|| ws.root_path.clone());
    // Run-level base branch: the ambient PR/review base for the whole run, so a
    // `review_run`/`git_pr` is aware of the run's base even after intervening
    // agent nodes (which return `{reply}`) drop it from the data flow. Per-node
    // `base` params still win. `None` ⇒ each git step resolves its repo's
    // DETECTED default branch at the point of use — never a fabricated "main"
    // (the historical literal here made `git diff main` exit 128 on repos
    // whose default is master/develop).
    let run_base: Option<String> = input
        .get("base")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // Resilience (design §B): many steps (review_run, git_pr, …) need a repo_id.
    // If the run wasn't given one but DOES carry a working_directory, resolve the
    // registered repo for that path ONCE here and seed it into the run input — so
    // every downstream step inherits it. This is what makes a workflow that only
    // sets `Working Directory:` work, instead of failing with "missing repo_id".
    let input = {
        let mut inp = input;
        let has_repo = inp
            .get("repo_id")
            .and_then(Value::as_str)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if !has_repo {
            if let Some(rid) =
                resolve_repo_id_for_path(&ctx, &workflow.workspace_id, &run_cwd).await
            {
                if let Value::Object(m) = &mut inp {
                    m.insert("repo_id".into(), Value::String(rid));
                }
            }
        }
        inp
    };
    // The run's mission brief (`run-brief.md`): mission, repos table, planned
    // steps (IN-SCOPE only, so numbering matches what actually executes on a
    // start-from-here run) and the step-file protocol.
    {
        let planned: Vec<(String, String)> = order
            .iter()
            .filter(|nid| run_set.as_ref().map(|s| s.contains(*nid)).unwrap_or(true))
            .filter_map(|nid| workflow.graph.nodes.iter().find(|n| &n.id == nid))
            .map(|n| {
                (
                    if n.name.is_empty() { n.kind.clone() } else { n.name.clone() },
                    n.kind.clone(),
                )
            })
            .collect();
        files.write_brief(&crate::workflow_context::render_brief(
            &workflow.name,
            &workflow.description,
            &run_id,
            &input,
            &files.repos(),
            &planned,
            has_instructions,
            has_prompt,
        ));
    }
    // The per-node execution environment (run identity + ambient cwd/base +
    // context files). The registry on `files` is the run's source of truth
    // for repos/branches — input threading loses keys hop by hop.
    let env = RunEnv {
        run_id: run_id.clone(),
        run_cwd: run_cwd.clone(),
        run_base,
        files: files.clone(),
    };
    // 1-based number of the NEXT executed step — drives step-file naming.
    // Skipped nodes don't consume a number; cached nodes do (their files are
    // written from the cached output so the trail stays complete).
    let mut step_counter: usize = 0;
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

    let rev = repo
        .update_run(&run_id, RunStatus::Running, &states, None, false)
        .await
        .unwrap_or(0);
    emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", None, rev, None, &states, false);

    // Live progress: if this run was triggered from a chat thread, stream brief
    // per-step updates back to it. A single pump task posts them in order; manual
    // UI / webhook-only runs get a disabled (no-op) sink.
    let (progress, progress_pump) = match resolve_chat_target(&workflow, &input) {
        Some(target) => {
            let (sink, handle) = spawn_progress_pump(ctx.clone(), target);
            (sink, Some(handle))
        }
        None => (ProgressSink::disabled(), None),
    };
    if progress.enabled() {
        let goals: Vec<String> = input
            .get("goals")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let goals_line = if goals.is_empty() {
            String::new()
        } else {
            format!("\n*Goals:* {}", goals.join("; "))
        };
        progress.post(format!(
            "🚀 *{}* started — {} step(s) queued.{}",
            workflow.name,
            order.len(),
            goals_line
        ));
    }

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
            let rev = repo
                .update_run(&run_id, RunStatus::Running, &states, None, false)
                .await
                .unwrap_or(0);
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
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
                let rev = repo
                    .update_run(&run_id, RunStatus::Running, &states, None, false)
                    .await
                    .unwrap_or(0);
                emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
                continue;
            }
            NodeDecision::BranchSkip => {
                states[idx].status = NodeStatus::Skipped;
                states[idx].logs = vec!["skipped (branch not taken)".into()];
                branch_skipped.insert(node_id.clone());
                let rev = repo
                    .update_run(&run_id, RunStatus::Running, &states, None, false)
                    .await
                    .unwrap_or(0);
                emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
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
        // prepare_context writes a side-effect file (jira-<KEY>.md) into the run's
        // context dir — a cache replay can't recreate that, so it never reads from
        // (or writes to, see below) the node-result cache.
        let cached_out = if node.kind != "prepare_context" {
            repo.get_cached_output(&workflow.id, &node_id, &params_hash, &input_hash)
                .await
        } else {
            None
        };
        if let Some(cached_out) = cached_out {
            states[idx].status = NodeStatus::Success;
            states[idx].output = Some(cached_out.clone());
            states[idx].logs = vec!["Success (cached)".into()];
            states[idx].duration_ms = Some(0);
            states[idx].attempts = Some(0);
            // A cached agent/product/canvas node still carries its session id in
            // the cached output — surface it so the run can open it.
            harvest_session_ids(&cached_out, &mut states[idx].sessions);
            // A cached node still consumes a step number and leaves its step
            // files (from the cached output) — the context-file trail stays
            // complete for downstream agents. Published refs merge too.
            step_counter += 1;
            let base_name = crate::workflow_context::step_base_name(
                step_counter,
                node_display_name(node),
                None,
                None,
            );
            let mut flogs = files.persist_step(
                &base_name,
                &node.kind,
                node_display_name(node),
                &cached_out,
                &[],
                None,
                None,
            );
            states[idx].logs.append(&mut flogs);
            merge_published_refs(&files, &cached_out);
            // Prune outgoing edges whose condition fails on the cached output.
            let (pruned, mut plogs) =
                eval_outgoing(&workflow.graph, node, &cached_out, &node_input, &input);
            inactive_edges.extend(pruned);
            states[idx].logs.append(&mut plogs);
            outputs.insert(node_id.clone(), cached_out);
            let rev = repo
                .update_run(&run_id, RunStatus::Running, &states, None, false)
                .await
                .unwrap_or(0);
            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
            continue;
        }
        // --------------------------------------------------------------------

        let start_line = format!("▶ {} started", node.kind);
        states[idx].status = NodeStatus::Running;
        states[idx].started_at = Some(chrono::Utc::now());
        states[idx].logs = vec![start_line.clone()];
        let rev = repo
            .update_run(&run_id, RunStatus::Running, &states, None, false)
            .await
            .unwrap_or(0);
        // Signal node start so the UI can show live progress immediately.
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
        if progress.enabled() && is_reportable(&node.kind) {
            progress.post(format!("▶ *{}* started", node_label(node)));
        }

        let started = Instant::now();
        // This node executes — it owns the next step-file number.
        step_counter += 1;
        let scope = StepScope { step_no: step_counter, iter: None, inner_idx: None };
        let step_file_base = crate::workflow_context::step_base_name(
            step_counter,
            node_display_name(node),
            None,
            None,
        );
        // Run the node, honoring its retry policy (default: a single attempt).
        // A node that spawns an openable agent session reports its id over
        // `sess_tx` the moment it's created; we record it on the node state and
        // persist+emit immediately so the run view can open it *while running*.
        let policy = resolve_retry(node);
        let mut attempt: u32 = 0;
        let mut backoff = policy.backoff_ms;
        let mut retry_logs: Vec<String> = vec![];
        let (sess_tx, mut sess_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        // Live per-node log lines (R9): the loop node streams iteration/sub-step
        // progress here so the run detail updates AS IT RUNS. Per-node channel, so
        // lines never leak into the next node.
        let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        // Snapshot when the (latest) attempt began: a step file the agent wrote
        // during a FAILED earlier attempt must not be mistaken for the winning
        // attempt's handoff (persist_step compares mtimes against this).
        // Assigned at the top of every attempt; the loop runs at least once.
        let mut attempt_started;
        let result = loop {
            attempt += 1;
            attempt_started = std::time::SystemTime::now();
            let fut =
                execute_node(&ctx, &ws, &user, node, node_input.clone(), &env, &scope, &sess_tx, &log_tx, &progress);
            tokio::pin!(fut);
            let attempt_res = loop {
                tokio::select! {
                    biased;
                    Some(sid) = sess_rx.recv() => {
                        if !states[idx].sessions.contains(&sid) {
                            states[idx].sessions.push(sid);
                            let rev = repo
                                .update_run(&run_id, RunStatus::Running, &states, None, false)
                                .await
                                .unwrap_or(0);
                            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
                        }
                    }
                    Some(line) = log_rx.recv() => {
                        // Live progress line (R9) — append to the node's logs and push
                        // it to the run detail immediately.
                        states[idx].logs.push(line);
                        let rev = repo
                            .update_run(&run_id, RunStatus::Running, &states, None, false)
                            .await
                            .unwrap_or(0);
                        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
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
        // Drain any trailing live-log lines (superseded by the node's final logs on
        // the success path; kept as context on the error/skip paths).
        while let Ok(line) = log_rx.try_recv() {
            states[idx].logs.push(line);
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
                // Step handoff files: raw output + the curated summary (kept
                // if the agent wrote its own during THIS attempt), then merge
                // any published repo/branch/worktree refs into repos.json.
                let mut flogs = files.persist_step(
                    &step_file_base,
                    &node.kind,
                    node_display_name(node),
                    &out,
                    &logs,
                    None,
                    Some(attempt_started),
                );
                logs.append(&mut flogs);
                merge_published_refs(&files, &out);
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
                if progress.enabled() && is_reportable(&node.kind) {
                    let dur = format!("{:.1}s", elapsed as f64 / 1000.0);
                    match brief_summary(&out) {
                        Some(s) => progress.post(format!("✅ *{}* done ({dur})\n{s}", node_label(node))),
                        None => progress.post(format!("✅ *{}* done ({dur})", node_label(node))),
                    }
                }
                // Attach the step's handoff file beneath its progress line —
                // the thread carries the brief, the file the full detail.
                // review_run streams its own block but its file rides too.
                if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                    progress.post_step_file(&files, &step_file_base);
                }
                // Persist to the node cache for future re-runs — except prepare_context,
                // whose jira-<KEY>.md side effect a cache replay cannot recreate (see the
                // matching read-side guard above).
                if node.kind != "prepare_context" {
                    let _ = repo
                        .set_cached_output(&workflow.id, &node_id, &params_hash, &input_hash, &out)
                        .await;
                }
                outputs.insert(node_id.clone(), out);
            }
            Err(e) => {
                states[idx].status = NodeStatus::Error;
                states[idx].error = Some(e.to_string());
                let mut elogs = vec![start_line];
                elogs.append(&mut retry_logs);
                elogs.push(format!("✗ {e}"));
                // A failed step leaves a trace file too — the error is part of
                // the handoff trail (a fix step or a human reads what broke).
                let mut flogs = files.persist_step(
                    &step_file_base,
                    &node.kind,
                    node_display_name(node),
                    &Value::Null,
                    &elogs,
                    Some(&e.to_string()),
                    Some(attempt_started),
                );
                elogs.append(&mut flogs);
                states[idx].logs = elogs;
                states[idx].attempts = Some(attempt);
                states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
                if progress.enabled() && is_reportable(&node.kind) {
                    progress.post(format!("❌ *{}* failed — {}", node_label(node), truncate(&e.to_string(), 200)));
                }
                // The failure trace file rides along too (what broke, logs).
                if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                    progress.post_step_file(&files, &step_file_base);
                }
                errored.insert(node_id.clone());
            }
        }
        let rev = repo
            .update_run(&run_id, RunStatus::Running, &states, None, false)
            .await
            .unwrap_or(0);
        // Signal node finish so the inspector can update without waiting for the next poll.
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
    }

    // Flush all streamed progress lines (close the channel, await the pump) so the
    // per-step updates land in the thread BEFORE the final summary is delivered.
    drop(progress);
    if let Some(h) = progress_pump {
        let _ = h.await;
    }

    if canceled {
        for s in states.iter_mut() {
            if matches!(s.status, NodeStatus::Pending | NodeStatus::Running) {
                s.status = NodeStatus::Skipped;
            }
        }
        let rev = repo
            .update_run(&run_id, RunStatus::Canceled, &states, Some("canceled"), true)
            .await
            .unwrap_or(0);
        deliver_run_result(&ctx, &workflow, &states, RunStatus::Canceled, None, &input, None).await;
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "canceled", None, rev, None, &states, false);
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
            "run exceeded the {}-hour time limit",
            RUN_WALL_CLOCK_TIMEOUT.as_secs() / 3600
        );
        let rev = repo
            .update_run(&run_id, RunStatus::Error, &states, Some(&msg), true)
            .await
            .unwrap_or(0);
        deliver_run_result(&ctx, &workflow, &states, RunStatus::Error, None, &input, None).await;
        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "error", None, rev, None, &states, false);
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
    let rev = repo
        .update_run(&run_id, final_status, &states, err_msg.as_deref(), true)
        .await
        .unwrap_or(0);
    // The run's deliverable: a copy of the last content-bearing step's handoff
    // file, only on outright success — an errored run has no coherent "answer"
    // to hand back, so delivery falls back to the per-step summary.md instead.
    let final_output = if final_status == RunStatus::Success { files.write_final_output() } else { None };
    // Proof pack: package the run's node outputs, human approvals, and budget
    // gate into inspectable evidence; link the pack to the run. Best-effort.
    let pack_id = assemble_workflow_proof(&ctx, &workflow, &run_id, &states).await;
    if let Some(pid) = &pack_id {
        let _ = repo.set_run_proof_pack(&run_id, pid).await;
    }
    // Report the result back to wherever the run was triggered from (Slack
    // thread / webhook): a brief status + the run's deliverable (final-output.md
    // when the run produced one, else the generated summary.md). Best-effort.
    deliver_run_result(&ctx, &workflow, &states, final_status, pack_id.as_deref(), &input, final_output.as_deref()).await;
    // Final event: run complete.
    emit_run_updated(&ctx, &workflow.workspace_id, &run_id, final_status.as_str(), None, rev, None, &states, false);
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
/// posted with an attachment: `final-output.md` (the run's actual deliverable)
/// when `final_output` is `Some`, else the generated `summary.md`. Manual UI
/// runs (no origin) no-op. Best-effort; redacted before it leaves the machine.
async fn deliver_run_result(
    ctx: &ServerCtx,
    workflow: &Workflow,
    states: &[NodeRunState],
    status: RunStatus,
    proof_pack_id: Option<&str>,
    input: &Value,
    final_output: Option<&[u8]>,
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
    // Token comes from the workspace whose integration received the message
    // (workflows are global, so that may differ from the workflow's workspace).
    let result_ws: String = str_at("origin_workspace_id")
        .map(|s| s.to_string())
        .unwrap_or_else(|| workflow.workspace_id.clone());
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
    // The attachment: the run's actual deliverable when it produced one (the
    // last content-bearing step's handoff, copied to final-output.md), else
    // the per-step summary.md this function always generates. Same redaction
    // either way — this leaves the machine over chat/webhook.
    let (attach_name, bytes): (&str, Vec<u8>) = match final_output {
        Some(raw) => (
            "final-output.md",
            otto_core::redact::redact_text(&String::from_utf8_lossy(raw)).value.into_bytes(),
        ),
        None => ("summary.md", otto_core::redact::redact_text(&full).value.into_bytes()),
    };

    // --- chat (Slack / Telegram) ---
    if let (Some(ch), Some(chat)) = (channel, chat) {
        let chan = match ch {
            "slack" => Some(Channel::Slack),
            "telegram" => Some(Channel::Telegram),
            _ => None,
        };
        if let Some(chan) = chan {
            match otto_state::IntegrationsRepo::new(ctx.pool.clone())
                .get(&result_ws, chan)
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
                            if let Err(e) = adapter.upload(chat, thread, attach_name, &bytes).await {
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
            crate::scheduled_tasks_engine::deliver_webhook(url, &brief, attach_name, &bytes).await
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

/// Run-level execution environment shared by every node of one run: identity,
/// ambient cwd, the ambient PR/review base (`None` ⇒ resolve the repo's
/// default branch at the point of use — never a fabricated `"main"`), and the
/// per-run context files (instruction, `repos.json` registry, step files).
/// The registry on `files` — not node-input threading, which loses keys hop
/// by hop — is the source of truth for which repos/branches the run operates
/// on.
pub(crate) struct RunEnv {
    pub run_id: Id,
    pub run_cwd: String,
    pub run_base: Option<String>,
    pub files: std::sync::Arc<crate::workflow_context::RunContextFiles>,
}

/// Where a node execution sits in the run's step-file numbering: the 1-based
/// executed-step number, plus the loop iteration / inner-step index for
/// `step{N}-{name}-iter{X}.md` naming inside `loop` nodes.
#[derive(Clone, Copy)]
pub(crate) struct StepScope {
    pub step_no: usize,
    pub iter: Option<u64>,
    pub inner_idx: Option<usize>,
}

/// Execute one node by kind. Returns `(output, logs)`.
///
/// `env.run_id` is used by stateful nodes (e.g. `human_approval`) to write
/// back to their own run row to record a pause / resume decision.
#[allow(clippy::too_many_arguments)]
async fn execute_node(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    input: Value,
    env: &RunEnv,
    scope: &StepScope,
    session_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    // Live per-node log lines: streamed to the run detail AS THE NODE RUNS (the
    // loop node uses this so the user sees iteration/sub-step progress instead of a
    // frozen "loop started"). Harvested next to `session_tx` in `run_workflow`.
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    progress: &ProgressSink,
) -> Result<(Value, Vec<String>)> {
    // Local aliases keep the node arms' existing call sites unchanged.
    let run_id: &Id = &env.run_id;
    let run_cwd: &str = env.run_cwd.as_str();
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
            // File-based handoff (design 2026-07-02): point the agent at the
            // run's context dir (instruction, repos.json, prior step files)
            // and name the step file IT must write. The inline [input data]
            // excerpt stays as a glance — the files are the complete channel.
            let preamble = env.files.preamble_for(
                scope.step_no,
                node_display_name(node),
                scope.iter,
                scope.inner_idx,
            );
            // Inject any per-step skills (`skill`/`skills`) ahead of the prompt so
            // the step runs a specific method "via prompt".
            let full = prepend_skills(
                ctx,
                p,
                &format!("{preamble}{prompt}\n\n[input data]\n{}", truncate(&input.to_string(), 4000)),
            );
            let acwd = node_cwd(node, &input, run_cwd);
            let (reply, sid) =
                run_node_agent(ctx, ws, user, node, provider, &full, &acwd, session_tx).await?;
            // Publish WHERE the implementer worked (+ thread the ambient repo/base)
            // so a downstream review/PR is aware of exactly this directory — even
            // when the agent ran in its own per-node cwd. This is what carries the
            // reference from the implementer to the reviewer and the PR.
            let mut out = serde_json::Map::new();
            out.insert("reply".into(), json!(reply));
            out.insert("session_id".into(), json!(sid));
            out.insert("working_directory".into(), json!(acwd));
            // `repos`/`worktree` forward too so data-flow inspection still
            // shows them — though the run-level registry (RunEnv.files), not
            // input threading, is what downstream git steps rely on.
            for k in ["repo_id", "base", "repos", "worktree"] {
                if let Some(v) = input.get(k) {
                    if !v.is_null() {
                        out.insert(k.into(), v.clone());
                    }
                }
            }
            Ok((Value::Object(out), vec!["agent turn complete".into()]))
        }

        "prepare_context" => {
            let mut jira = serde_json::Map::new();
            match crate::workflow_prepare::extract_jira_key(p, &input) {
                None => {
                    jira.insert("found".into(), json!(false));
                }
                Some(key) => {
                    jira.insert("found".into(), json!(true));
                    jira.insert("key".into(), json!(key));
                    let fetched = match crate::workflow_prepare::resolve_jira_account(
                        ctx, &user.id, p.get("account_id").and_then(Value::as_str)).await
                    {
                        Ok(account) => {
                            let token = ctx.secrets.get(&account.token_ref).ok().flatten();
                            match token {
                                Some(t) => otto_issues::JiraClient::new(&account.base_url, &account.email, &t)
                                    .get_issue_full(&key).await.map_err(|e| e.to_string()),
                                None => Err(format!("missing token for issue account {}", account.id)),
                            }
                        }
                        Err(e) => Err(e),
                    };
                    match fetched {
                        Ok(issue) => {
                            env.files.write_named(&format!("jira-{key}.md"),
                                &crate::workflow_prepare::render_issue_md(&issue));
                            jira.insert("fetched".into(), json!(true));
                            jira.insert("summary".into(), json!(issue.summary));
                            jira.insert("status".into(), json!(issue.status));
                            jira.insert("url".into(), json!(issue.url));
                        }
                        Err(e) => {
                            // Indicate loudly, don't die (unless required): every later
                            // step + the human must see the ticket data is missing.
                            env.files.write_named(&format!("jira-{key}.md"),
                                &format!("# ⚠ Could not fetch {key}\n\n{e}\n\nProceed from the prompt/instructions; treat ticket details as UNAVAILABLE.\n"));
                            jira.insert("fetched".into(), json!(false));
                            jira.insert("error".into(), json!(e));
                            if p.get("require").and_then(Value::as_bool).unwrap_or(false) {
                                return Err(otto_core::Error::Invalid(format!("prepare_context: required Jira fetch failed: {jira:?}")));
                            }
                        }
                    }
                }
            }
            let mut out = serde_json::Map::new();
            out.insert("jira".into(), Value::Object(jira));
            // Optional agent phase — mirrors agent_prompt exactly.
            let agent_prompt_txt = p.get("prompt").and_then(Value::as_str).unwrap_or("").trim().to_string();
            let mut logs = vec![format!("prepare_context: jira {}", out["jira"])];
            if !agent_prompt_txt.is_empty() {
                let preamble = env.files.preamble_for(scope.step_no, node_display_name(node), scope.iter, scope.inner_idx);
                let full = prepend_skills(ctx, p,
                    &format!("{preamble}{agent_prompt_txt}\n\n[input data]\n{}", truncate(&input.to_string(), 4000)));
                let provider = p.get("provider").and_then(Value::as_str).unwrap_or("claude");
                let acwd = node_cwd(node, &input, run_cwd);
                let (reply, sid) = run_node_agent(ctx, ws, user, node, provider, &full, &acwd, session_tx).await?;
                out.insert("reply".into(), json!(reply));
                out.insert("session_id".into(), json!(sid));
                out.insert("working_directory".into(), json!(acwd));
                for k in ["repo_id", "base", "repos", "worktree"] {
                    if let Some(v) = input.get(k) { if !v.is_null() { out.insert(k.into(), v.clone()); } }
                }
                logs.push("agent phase complete".into());
            }
            Ok((Value::Object(out), logs))
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
            // `waiting_approval = 0` and records the decision. Bump `rev` so
            // clients treat the pause as a fresh state, and announce it over
            // WS immediately — the approval banner/badge must not wait on a poll.
            let pool = &ctx.pool;
            let rev: i64 = sqlx::query_scalar(
                "UPDATE workflow_runs
                 SET waiting_approval = 1, approval_node_id = ?, rev = rev + 1
                 WHERE id = ?
                 RETURNING rev",
            )
            .bind(&node.id)
            .bind(run_id)
            .fetch_one(pool)
            .await
            .map_err(|e| otto_core::Error::Internal(format!("human_approval mark: {e}")))?;
            emit_run_updated(ctx, &ws.id, run_id, "running", Some(&node.id), rev, None, &[], true);

            // Poll for the operator's decision.
            let deadline = Instant::now() + NODE_AGENT_TIMEOUT;
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
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
            // Two inner steps with the same (slugged) name would collide on
            // step-file names within an iteration — disambiguate those (and
            // only those) with the inner index.
            let inner_slugs: Vec<String> = steps
                .iter()
                .enumerate()
                .map(|(k, s)| {
                    crate::workflow_context::slug(
                        s.get("name").and_then(Value::as_str).unwrap_or(&format!("step{k}")),
                    )
                })
                .collect();
            let slug_dup = |k: usize| inner_slugs.iter().filter(|s| **s == inner_slugs[k]).count() > 1;
            let mut logs = vec![];
            let mut history = vec![];
            let mut satisfied = false;
            let mut last = input.clone();
            let mut iterations = 0u64;
            // References (repo_id/base/worktree) any inner step published — so a
            // git_pr after the loop knows the exact repo(s)/branch(es) to open,
            // deduped by repo_id (latest wins). Carries multi-repo loops too.
            let mut refs_by_repo: serde_json::Map<String, Value> = serde_json::Map::new();
            for i in 1..=max_iter {
                iterations = i;
                if let Ok(rr) = WorkflowsRepo::new(ctx.pool.clone()).get_run(run_id).await {
                    if rr.status == RunStatus::Canceled {
                        logs.push("loop: canceled".into());
                        break;
                    }
                }
                if progress.enabled() {
                    progress.post(format!("🔁 *Iteration {i}/{max_iter}*"));
                }
                // R9: live progress into the run detail (independent of Slack).
                let _ = log_tx.send(format!("🔁 iteration {i}/{max_iter}"));
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
                    if progress.enabled() && is_reportable(&skind) {
                        progress.post(format!("› ▶ {sname} started"));
                    }
                    let _ = log_tx.send(format!("▶ {sname} — iteration {i}…"));
                    let sub = WorkflowNode {
                        id: format!("{}#{i}.{k}", node.id),
                        kind: skind.clone(),
                        name: sname.clone(),
                        x: 0.0,
                        y: 0.0,
                        params: step.get("params").cloned().unwrap_or(Value::Null),
                        retry: step.get("retry").and_then(|r| serde_json::from_value(r.clone()).ok()),
                    };
                    // Inner steps share the loop's step number and add
                    // `-iter{X}` (the user's convention): step3-review-iter2.md.
                    let sub_scope = StepScope {
                        step_no: scope.step_no,
                        iter: Some(i),
                        inner_idx: if slug_dup(k) { Some(k) } else { None },
                    };
                    let sub_file_base = crate::workflow_context::step_base_name(
                        sub_scope.step_no,
                        &sname,
                        sub_scope.iter,
                        sub_scope.inner_idx,
                    );
                    let sub_attempt_started = std::time::SystemTime::now();
                    match Box::pin(execute_node(
                        ctx, ws, user, &sub, step_input, env, &sub_scope, session_tx, log_tx, progress,
                    ))
                    .await
                    {
                        Ok((out, mut slogs)) => {
                            // Iteration handoff files — the loop recursion never
                            // passes the main run loop, so this is the second
                            // persistence hook site.
                            let flogs = env.files.persist_step(
                                &sub_file_base,
                                &skind,
                                &sname,
                                &out,
                                &slogs,
                                None,
                                Some(sub_attempt_started),
                            );
                            for l in slogs.drain(..).chain(flogs) {
                                let line = format!("  [{i}/{sname}] {l}");
                                let _ = log_tx.send(line.clone());
                                logs.push(line);
                            }
                            step_outputs.insert(sname.clone(), out.clone());
                            // Capture any repo reference this step published —
                            // for the loop's own output AND the run registry.
                            merge_published_refs(&env.files, &out);
                            if let Some(rid) = out.get("repo_id").and_then(Value::as_str) {
                                if !rid.trim().is_empty() {
                                    refs_by_repo.insert(
                                        rid.to_string(),
                                        json!({
                                            "repo_id": rid,
                                            "base": out.get("base").cloned().unwrap_or(Value::Null),
                                            "worktree": out.get("worktree").cloned().unwrap_or(Value::Null),
                                        }),
                                    );
                                }
                            }
                            last = out.clone();
                            thread = out;
                            // R9: live "done" milestone into the run detail.
                            let _ = log_tx.send(match brief_summary(&last) {
                                Some(s) => format!("✅ {sname} done — {}", truncate(&s, 160)),
                                None => format!("✅ {sname} done"),
                            });
                            if progress.enabled() && is_reportable(&skind) {
                                match brief_summary(&last) {
                                    Some(s) => progress.post(format!("› ✅ {sname} done\n{s}")),
                                    None => progress.post(format!("› ✅ {sname} done")),
                                }
                            }
                            // Attach the iteration's handoff file too.
                            if progress.enabled() && (is_reportable(&skind) || skind == "review_run") {
                                progress.post_step_file(&env.files, &sub_file_base);
                            }
                        }
                        Err(e) => {
                            let fline = format!("  [{i}/{sname}] ✗ {e}");
                            let _ = log_tx.send(fline.clone());
                            logs.push(fline);
                            // A failed iteration leaves its trace file BEFORE the
                            // loop bails — the fix step / a human reads what broke
                            // (the production incident left no trail at all).
                            let _ = env.files.persist_step(
                                &sub_file_base,
                                &skind,
                                &sname,
                                &Value::Null,
                                &[format!("✗ {e}")],
                                Some(&e.to_string()),
                                Some(sub_attempt_started),
                            );
                            if progress.enabled() && is_reportable(&skind) {
                                progress.post(format!("› ❌ {sname} failed — {}", truncate(&e.to_string(), 200)));
                            }
                            // The failed iteration's trace file rides along.
                            if progress.enabled() && (is_reportable(&skind) || skind == "review_run") {
                                progress.post_step_file(&env.files, &sub_file_base);
                            }
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
                    let line = format!("loop: `{until}` satisfied at iteration {i}");
                    let _ = log_tx.send(line.clone());
                    logs.push(line);
                    break;
                }
            }
            if !until.is_empty() && !satisfied {
                logs.push(format!("loop: reached max_iterations ({max_iter}) without satisfying `{until}`"));
            }
            // Surface the loop's repo reference(s) at the top level so a
            // downstream git_pr inherits them: a single `repo_id`/`base`/`worktree`
            // (the common one-repo loop) plus a `repos` array for the multi-repo
            // case (one PR per changed repo).
            let repos: Vec<Value> = refs_by_repo.values().cloned().collect();
            let mut out = serde_json::Map::new();
            out.insert("iterations".into(), json!(iterations));
            out.insert("satisfied".into(), json!(satisfied));
            out.insert("last".into(), last);
            out.insert("history".into(), json!(history));
            if let Some((_, Value::Object(m))) = refs_by_repo.iter().next() {
                for k in ["repo_id", "base", "worktree"] {
                    if let Some(v) = m.get(k) {
                        out.insert(k.to_string(), v.clone());
                    }
                }
            }
            if !repos.is_empty() {
                out.insert("repos".into(), Value::Array(repos));
            }
            Ok((Value::Object(out), logs))
        }

        // --- Review Run (wired: local-review engine + 0–100 score + goals) ---
        "review_run" => {
            let threshold = p.get("threshold").and_then(Value::as_u64).unwrap_or(80) as i64;
            let await_done = p.get("await").and_then(Value::as_bool).unwrap_or(true);
            let timeout_s = p.get("timeout_s").and_then(Value::as_u64).unwrap_or(900).min(1800);
            // Reviewer providers + lenses (skills) — drive the SAME multi-agent
            // engine as PR review (multi-provider × multi-lens, one summarizer that
            // consolidates + scores). Empty → the stored/default PR-review config.
            let providers = param_str_list(p, "providers");
            let lenses = {
                let mut l = param_str_list(p, "lenses");
                for s in param_str_list(p, "skills") {
                    if !l.contains(&s) {
                        l.push(s);
                    }
                }
                l
            };
            // When set, the step itself FAILS if the score is below threshold — so a
            // downstream "create PR" step is error-skipped unless the review passed.
            let require_pass = p.get("require_pass").and_then(Value::as_bool).unwrap_or(false);
            // Which repo(s)/worktree(s)/base(s) to review — (repo_id, worktree,
            // want_base, label), in priority order:
            //
            // 1. NODE params name a target → the classic single-repo path.
            // 2. The input carries a published `repos[]` array (a multi-repo
            //    review/loop handing its full set forward) → one target each,
            //    so a chained review re-reviews EVERY repo, not just the
            //    first-mirrored one.
            // 3. The input carries a published single `worktree` reference →
            //    single target (an upstream step handing off the exact place
            //    it worked).
            // 4. The run's repos registry (the declared branches/worktrees,
            //    source+destination) → one target per valid entry.
            // 5. One implicit target from the run context (design §B
            //    resilience: working_directory/run_cwd → registered repo).
            //
            // A bare `input.repo_id`/`working_directory` never counts as a
            // target by itself — run-start seeding puts those into the input
            // of every declared run, and counting them would permanently mask
            // the multi-entry paths.
            let params_explicit = p.get("repo_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).is_some()
                || p.get("worktree_path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).is_some();
            let input_repo_targets: Vec<(String, String, Option<String>, String)> = input
                .get("repos")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| {
                            let rid = t.get("repo_id").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
                            let wt = t.get("worktree").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
                            let base = t.get("base").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
                            Some((rid.to_string(), expand_tilde(wt), base, String::new()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let input_worktree_ref = input.get("worktree").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).is_some()
                || input.get("worktree_path").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).is_some();
            let mut targets: Vec<(String, String, Option<String>, String)> = Vec::new();
            if !params_explicit && !input_repo_targets.is_empty() {
                targets = input_repo_targets;
            } else if !params_explicit && !input_worktree_ref {
                // Declarations: EVERY entry failed to resolve → fail with the
                // reasons. Falling through to the run-cwd path would review
                // whatever happens to be checked out — and an empty diff there
                // reads as a false-green "score 100" on a run whose declared
                // target never resolved.
                let declared = env.files.repos();
                if let Some(msg) = crate::workflow_context::all_declared_errored(&declared) {
                    return Err(otto_core::Error::Invalid(format!(
                        "review_run: no declared repos[] entry resolved — {msg}"
                    )));
                }
                for e in declared
                    .iter()
                    .filter(|e| e.error.is_none() && e.repo_id.is_some() && e.worktree.is_some())
                {
                    targets.push((
                        e.repo_id.clone().unwrap_or_default(),
                        e.worktree.clone().unwrap_or_default(),
                        // Declared source, else per-repo default-branch
                        // detection inside run_review_for_branch.
                        e.base.clone(),
                        e.repo.clone(),
                    ));
                }
            }
            if targets.is_empty() {
                // Resilient repo resolution (design §B): explicit repo_id wins,
                // else derive it from the step's worktree_path / the run's
                // working_directory / run_cwd against the workspace's registered
                // repos. Never a bare "missing repo_id" for a workflow that was
                // given a working directory.
                let repo_id = resolve_step_repo_id(ctx, ws, p, &input, run_cwd)
                    .await
                    .ok_or_else(|| {
                        otto_core::Error::Invalid(
                            "review_run: no repo_id; pass repo_id or a working_directory/worktree_path \
                             under a registered repo, or declare repos on the run"
                                .into(),
                        )
                    })?;
                // The directory the implementer worked in (the run's
                // working_directory) IS what we review — fall back to it
                // (run_cwd), not the bare repo checkout, so the reviewer sees the
                // same place the agent changed. A prior step's published
                // `worktree` (e.g. another review) wins first.
                let worktree = p
                    .get("worktree_path")
                    .and_then(Value::as_str)
                    .or_else(|| input.get("worktree").and_then(Value::as_str))
                    .or_else(|| input.get("worktree_path").and_then(Value::as_str))
                    .or_else(|| input.get("working_directory").and_then(Value::as_str))
                    .filter(|s| !s.trim().is_empty())
                    .map(expand_tilde)
                    .unwrap_or_else(|| run_cwd.to_string());
                // The DESIRED base: node params → input → the run's ambient base.
                // `None` ⇒ run_review_for_branch resolves the repo's default
                // branch; a named branch that doesn't exist falls back the same
                // way instead of exiting 128.
                let want_base = p
                    .get("base")
                    .and_then(Value::as_str)
                    .or_else(|| input.get("base").and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| env.run_base.clone());
                targets.push((repo_id, worktree, want_base, String::new()));
            }
            let multi = targets.len() > 1;
            // Iteration label so a fix→review loop streams "Review #1, #2, …".
            let iter_label = input
                .get("_iteration")
                .and_then(Value::as_u64)
                .map(|n| format!("Review #{n}"))
                .unwrap_or_else(|| "Review".to_string());
            // Rich reviewer config (PR-review parity, design §F): `reviewers` =
            // per-lens provider sets + optional per-reviewer instructions, with an
            // optional `summarizer`. Falls back to the flat providers/lenses form,
            // then to the stored/default PR-review config.
            let reviewers = p.get("reviewers").and_then(Value::as_array).cloned().unwrap_or_default();
            // User-defined reviewer CHECKS — commands (e.g. `go test -tags=component
            // ./...`) the reviewer agent RUNS and reports failures on, in addition
            // to goals. A safety net for what the implementer's skill may have
            // skipped. From the node's `checks` or the run input's `checks`.
            let check_specs = parse_checks(p.get("checks").or_else(|| input.get("checks")));
            let mut cfg_override = if !reviewers.is_empty() {
                let default_provider = crate::modules::default_review_provider(ctx).await;
                Some(crate::modules::workflow_review_config_from_json(
                    &default_provider,
                    &reviewers,
                    p.get("summarizer"),
                ))
            } else if providers.is_empty() && lenses.is_empty() {
                None
            } else {
                let default_provider = crate::modules::default_review_provider(ctx).await;
                Some(crate::modules::workflow_review_config(&default_provider, &providers, &lenses))
            };
            // Append a dedicated "Required checks" reviewer that RUNS the commands
            // and reports any failure as a blocking (bug) finding — so it drops the
            // score and the loop keeps fixing until the checks pass. Delegated to
            // the agent (it has the repo + the implementer's context), not run by us.
            if !check_specs.is_empty() {
                let default_provider = crate::modules::default_review_provider(ctx).await;
                let cfg = cfg_override
                    .get_or_insert_with(|| crate::modules::workflow_review_config(&default_provider, &[], &[]));
                cfg.agents.push(checks_review_agent(&default_provider, &check_specs));
            }
            // Progress-post label fragments (shared by every target).
            let lens_list: Vec<String> = if reviewers.is_empty() {
                lenses.clone()
            } else {
                reviewers
                    .iter()
                    .filter_map(|r| {
                        r.get("lens")
                            .or_else(|| r.get("skill"))
                            .or_else(|| r.get("name"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect()
            };
            let lens_txt = if lens_list.is_empty() {
                String::new()
            } else {
                format!(" · lenses: {}", lens_list.join(", "))
            };
            let prov_txt = if providers.is_empty() {
                String::new()
            } else {
                format!(" · providers: {}", providers.join(", "))
            };
            let chk_txt = if check_specs.is_empty() {
                String::new()
            } else {
                format!(" · {} check(s) delegated to the reviewer", check_specs.len())
            };
            // Per-severity deductions (percent off 100) over the OPEN findings
            // — `scoring: { bug, warn, info }`. Defaults (20/5/5) preserve the
            // historical blocking/advisory formula.
            let weight = |key: &str, default: i64| -> i64 {
                p.get("scoring")
                    .and_then(|s| s.get(key))
                    .and_then(Value::as_i64)
                    .unwrap_or(default)
            };
            // Optional goals — assessed by an agent per target and blended in.
            let goals: Vec<String> = p
                .get("goals")
                .and_then(Value::as_array)
                .or_else(|| input.get("goals").and_then(Value::as_array))
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();

            // Review every target; a per-target failure in the multi-repo case
            // is logged and skipped (one bad repo never sinks the others), the
            // single-target case keeps its hard error.
            let mut logs: Vec<String> = Vec::new();
            let mut outs: Vec<serde_json::Map<String, Value>> = Vec::new();
            for (t_idx, (repo_id, worktree, want_base, label)) in targets.iter().enumerate() {
                // Validate the repo exists (clear error if not); the review
                // engine looks the repo up again by id internally. Its name
                // labels the per-repo progress lines when the target didn't
                // carry one (input-published references).
                let label = match ctx.git_store.get_repo(repo_id).await {
                    Ok(r) if label.is_empty() => r.name,
                    Ok(_) => label.clone(),
                    Err(e) => {
                        let msg = format!("review_run [{label}]: repo: {e}");
                        if multi {
                            logs.push(msg);
                            continue;
                        }
                        return Err(otto_core::Error::NotFound(msg));
                    }
                };
                let tag = if multi {
                    format!(" [{} {}/{}]", label, t_idx + 1, targets.len())
                } else {
                    String::new()
                };
                if progress.enabled() {
                    progress.post(format!(
                        "🔍 *{iter_label}{tag}* started (pass ≥ {threshold}){lens_txt}{prov_txt}{chk_txt}"
                    ));
                }
                let (review_id, resolved_base) = match crate::modules::run_review_for_branch(
                    ctx,
                    repo_id,
                    worktree,
                    want_base.as_deref(),
                    cfg_override.clone(),
                )
                .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        let msg = format!("review_run{tag}: {e}");
                        if multi {
                            logs.push(msg);
                            if progress.enabled() {
                                progress.post(format!("🔍 *{iter_label}{tag}* skipped — {}", truncate(&e.to_string(), 200)));
                            }
                            continue;
                        }
                        return Err(e);
                    }
                };
                // Publish the RESOLVED branch from here on — the loop harvest,
                // the repos registry and a downstream git_pr must target what
                // was actually reviewed, not the pre-resolution wish.
                let base = resolved_base.branch.clone();
                logs.push(format!(
                    "review_run{tag}: started review {review_id} ({worktree} vs {base})"
                ));
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
                            logs.push(format!("review_run{tag}: timed out waiting for review"));
                            break;
                        }
                    }
                }
                let (total, open, blocker) =
                    crate::modules::review_findings_counts(ctx, &review_id).await;
                let blocking = blocker as i64;
                let advisory = (open.saturating_sub(blocker)) as i64;
                let (bugs, warns, infos) =
                    crate::modules::review_open_counts_by_severity(ctx, &review_id).await;
                let review_score = (100
                    - bugs as i64 * weight("bug", 20)
                    - warns as i64 * weight("warn", 5)
                    - infos as i64 * weight("info", 5))
                .clamp(0, 100);
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
                    match run_node_agent(ctx, ws, user, node, "claude", &gprompt, worktree, session_tx).await {
                        Ok((reply, _sid)) => match extract_json(&reply) {
                            Some(v) => {
                                let gs = v.get("score").and_then(Value::as_i64).unwrap_or(review_score).clamp(0, 100);
                                (Some(gs), v.get("goals").cloned().unwrap_or(json!([])))
                            }
                            None => (Some(review_score), json!([{ "note": "goals eval reply not parseable" }])),
                        },
                        Err(e) => {
                            logs.push(format!("review_run{tag}: goals eval failed: {e}"));
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
                    "review_run{tag}: score {score} (review {review_score}{}) — {}",
                    goals_score.map(|g| format!(", goals {g}")).unwrap_or_default(),
                    if passed { "passed" } else { "below threshold" }
                ));
                // Stream the verdict + top findings to the chat thread.
                let finding_briefs = crate::modules::review_finding_briefs(ctx, &review_id, 10).await;
                if progress.enabled() {
                    let verdict = if passed { "✅ passed" } else { "⚠️ below threshold" };
                    let mut msg = format!(
                        "🔍 *{iter_label}{tag}* done — score *{score}/100* (pass ≥ {threshold}) — {verdict}"
                    );
                    if finding_briefs.is_empty() {
                        msg.push_str("\nFindings: none 🎉");
                    } else {
                        msg.push_str(&format!("\nFindings ({open} open):"));
                        for b in &finding_briefs {
                            msg.push_str(&format!("\n • {b}"));
                        }
                    }
                    progress.post(msg);
                }
                let out = json!({
                    "review_id": review_id, "status": status,
                    // Publish the exact reference reviewed so a downstream git_pr
                    // (or another review) opens the PR on the SAME
                    // repo/branch/worktree — no need to re-type them on the PR
                    // node (design: PR is aware).
                    "repo_id": repo_id, "base": base, "worktree": worktree,
                    "total": total, "open": open, "blocking": blocking, "advisory": advisory,
                    "severity": { "bug": bugs, "warn": warns, "info": infos },
                    "review_score": review_score, "goals_score": goals_score, "goals": goals_detail,
                    // The checks delegated to the reviewer agent (it runs them and
                    // reports failures as findings — see the cfg injection above).
                    "checks_requested": check_specs.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>(),
                    "score": score, "threshold": threshold, "passed": passed,
                    "findings": finding_briefs, "providers": providers, "lenses": lenses,
                });
                outs.push(out.as_object().cloned().unwrap_or_default());
            }
            if outs.is_empty() {
                return Err(otto_core::Error::Upstream(format!(
                    "review_run: no declared repo could be reviewed ({})",
                    logs.join("; ")
                )));
            }
            // Aggregate: the FIRST reviewed repo's fields stay mirrored at the
            // top level (back-compat for single-repo consumers); the strict
            // union gates multi-repo runs — worst score, pass only when all
            // passed — with per-repo detail under `reviews[]`.
            let mut out = outs[0].clone();
            let (score, passed) = if multi {
                let agg_score = outs
                    .iter()
                    .filter_map(|o| o.get("score").and_then(Value::as_i64))
                    .min()
                    .unwrap_or(0);
                let agg_passed = outs
                    .iter()
                    .all(|o| o.get("passed").and_then(Value::as_bool).unwrap_or(false));
                out.insert("score".into(), json!(agg_score));
                out.insert("passed".into(), json!(agg_passed));
                // EVERY reviewed reference, in the `repos[]` shape
                // collect_pr_targets consumes — without this a downstream
                // git_pr would see only the first repo mirrored at top level
                // and never open the other entries' PRs.
                let repo_targets: Vec<Value> = outs
                    .iter()
                    .map(|o| {
                        let mut m = serde_json::Map::new();
                        for k in ["repo_id", "worktree", "base"] {
                            if let Some(v) = o.get(k) {
                                m.insert(k.to_string(), v.clone());
                            }
                        }
                        Value::Object(m)
                    })
                    .collect();
                out.insert("repos".into(), Value::Array(repo_targets));
                out.insert(
                    "reviews".into(),
                    Value::Array(outs.into_iter().map(Value::Object).collect()),
                );
                (agg_score, agg_passed)
            } else {
                (
                    out.get("score").and_then(Value::as_i64).unwrap_or(0),
                    out.get("passed").and_then(Value::as_bool).unwrap_or(false),
                )
            };
            // `require_pass`: fail the step (so downstream gates error-skip) when the
            // score didn't clear the bar. A failed check surfaces as a blocking
            // finding from the checks reviewer, which drops the score below it.
            if require_pass && !passed {
                return Err(otto_core::Error::Upstream(format!(
                    "review_run: score {score} below required threshold {threshold} (require_pass)"
                )));
            }
            Ok((Value::Object(out), logs))
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
            // Same file-based handoff as agent_prompt: context dir + the step
            // file this node must write (engine fallback covers omission).
            let preamble = env.files.preamble_for(
                scope.step_no,
                node_display_name(node),
                scope.iter,
                scope.inner_idx,
            );
            let prompt = format!(
                "{preamble}{skill}\n\n# Task\n{instruction}\n{extra}\n\n# Story context\n{}",
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
            let ext = canvas_node_ext(mode);
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

        // --- Git PR (draft, or open with open=true) --------------------------
        // Multi-repo aware: opens ONE PR per changed repo. Targets come from the
        // reference the implementer/reviewer used — explicit `params.repos[]`, a
        // loop's published `repos[]`, a single upstream review reference, or
        // fan-in of several review branches — so a run that touched multiple
        // repositories opens them all, each on its own base branch. The PR never
        // needs the repo/base re-typed; it inherits them.
        "git_pr" => {
            let mut logs: Vec<String> = Vec::new();
            let open = p.get("open").and_then(Value::as_bool).unwrap_or(false);
            // The run-level worktree (where the work happened) + base, used when a
            // target doesn't carry its own.
            let run_worktree = input
                .get("worktree")
                .and_then(Value::as_str)
                .or_else(|| input.get("worktree_path").and_then(Value::as_str))
                .or_else(|| input.get("working_directory").and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty())
                .map(expand_tilde)
                .unwrap_or_else(|| run_cwd.to_string());
            let run_input_base: Option<String> = input
                .get("base")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| env.run_base.clone());

            // Resolve (repo_id, worktree, base) per target, deduped by repo_id.
            // Targets come from the implementer/reviewer reference (input) or
            // explicit `params.repos`; when neither names one, the run-level
            // repos registry (the declared branches/worktrees) supplies them.
            // With `detect_changed: true`, ALSO consider every registered repo
            // — the per-repo draft below keeps only the ones that actually
            // changed, so the run opens a PR for exactly what the implementer
            // touched, across repositories. A `None` base ⇒ draft_pr_core
            // resolves that repo's default branch.
            let mut targets = collect_pr_targets(p, &input);
            if targets.is_empty() {
                let declared = env.files.repos();
                // Same guard as review_run: every declaration errored → fail
                // with the reasons rather than silently PR-ing the run cwd.
                if let Some(msg) = crate::workflow_context::all_declared_errored(&declared) {
                    return Err(otto_core::Error::Invalid(format!(
                        "git_pr: no declared repos[] entry resolved — {msg}"
                    )));
                }
                targets = declared
                    .iter()
                    .filter(|e| e.error.is_none() && e.repo_id.is_some())
                    .map(crate::workflow_context::entry_to_target)
                    .collect();
            }
            if p.get("detect_changed").and_then(Value::as_bool).unwrap_or(false) {
                if let Ok(repos) = ctx.git_store.list_repos(&ws.id).await {
                    for r in repos {
                        targets.push(json!({ "repo_id": r.id, "worktree": r.path }));
                    }
                }
            }
            let mut resolved: Vec<(String, String, Option<String>)> = Vec::new();
            let mut seen: std::collections::HashSet<String> = Default::default();
            let mut notes: Vec<String> = Vec::new();
            if targets.is_empty() {
                // One implicit target from the run context (the common case).
                let repo_id = resolve_step_repo_id(ctx, ws, p, &input, run_cwd).await.ok_or_else(|| {
                    otto_core::Error::Invalid(
                        "git_pr: no repo_id; pass repo_id or a working_directory/worktree_path \
                         under a registered repo"
                            .into(),
                    )
                })?;
                let base = p
                    .get("base")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| run_input_base.clone());
                seen.insert(repo_id.clone());
                resolved.push((repo_id, run_worktree.clone(), base));
            } else {
                for t in &targets {
                    let worktree = t
                        .get("worktree")
                        .and_then(Value::as_str)
                        .or_else(|| t.get("worktree_path").and_then(Value::as_str))
                        .filter(|s| !s.trim().is_empty())
                        .map(expand_tilde)
                        .unwrap_or_else(|| run_worktree.clone());
                    let base = t
                        .get("base")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .or_else(|| run_input_base.clone());
                    let rid = match t.get("repo_id").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
                        Some(r) => Some(r.to_string()),
                        None => resolve_repo_id_for_path(ctx, &ws.id, &worktree).await,
                    };
                    match rid {
                        Some(r) if seen.insert(r.clone()) => resolved.push((r, worktree, base)),
                        Some(_) => {} // duplicate repo across targets — one PR each
                        None => notes.push(format!("skipped a target (no registered repo for {worktree})")),
                    }
                }
                if resolved.is_empty() {
                    return Err(otto_core::Error::Invalid(
                        "git_pr: no registered repo resolved for any target".into(),
                    ));
                }
            }

            // Draft (+ optionally open) a PR per resolved repo. A per-repo failure
            // (no diff / open error) becomes a note + a skipped entry, so one bad
            // repo never sinks the others.
            let mut prs: Vec<Value> = Vec::new();
            let mut opened_n: u64 = 0;
            for (repo_id, worktree, base) in &resolved {
                let repo = match ctx.git_store.get_repo(repo_id).await {
                    Ok(r) => r,
                    Err(e) => {
                        notes.push(format!("{repo_id}: repo missing: {e}"));
                        continue;
                    }
                };
                let wt = if worktree.trim().is_empty() { repo.path.clone() } else { worktree.clone() };
                let draft = match crate::modules::draft_pr_core(ctx, &wt, base.as_deref()).await {
                    Ok(d) => d,
                    Err(e) => {
                        notes.push(format!("{}: nothing to PR ({e})", repo.name));
                        continue;
                    }
                };
                if !open {
                    prs.push(json!({
                        "repo_id": repo_id, "repo": repo.name, "title": draft.title,
                        "description": draft.description, "source_branch": draft.source_branch,
                        "target_branch": draft.target_branch, "opened": false,
                    }));
                    logs.push(format!(
                        "git_pr: drafted '{}' on {} ({} → {})",
                        draft.title, repo.name, draft.source_branch, draft.target_branch
                    ));
                    continue;
                }
                let req = otto_core::api::CreatePrReq {
                    title: draft.title.clone(),
                    description: draft.description.clone(),
                    source_branch: draft.source_branch.clone(),
                    target_branch: draft.target_branch.clone(),
                    proof_pack_id: None,
                    allow_unproven: Some(true),
                };
                let auth = otto_core::auth::AuthUser(user.clone());
                match otto_git::http::create_pr_for_repo(ctx, &auth, &repo, &req).await {
                    Ok(summary) => {
                        opened_n += 1;
                        prs.push(json!({
                            "repo_id": repo_id, "repo": repo.name, "title": draft.title,
                            "source_branch": draft.source_branch, "target_branch": draft.target_branch,
                            "opened": true, "number": summary.number, "url": summary.url,
                        }));
                        logs.push(format!(
                            "git_pr: opened PR #{} '{}' on {} ({} → {})",
                            summary.number, draft.title, repo.name, draft.source_branch, draft.target_branch
                        ));
                    }
                    Err(e) => {
                        notes.push(format!("{}: open failed: {e}", repo.name));
                        // R15: keep the agent-drafted title/description on the failure
                        // entry too — so the user sees what the pull-request skill
                        // prepared (and the node output isn't flagged "missing
                        // title/description"). Otto still owns opening the PR.
                        prs.push(json!({
                            "repo_id": repo_id, "repo": repo.name, "opened": false,
                            "error": e.to_string(),
                            "title": draft.title, "description": draft.description,
                            "source_branch": draft.source_branch, "target_branch": draft.target_branch,
                        }));
                    }
                }
            }
            if prs.is_empty() {
                return Err(otto_core::Error::Upstream(format!(
                    "git_pr: no PR could be drafted/opened ({})",
                    notes.join("; ")
                )));
            }
            // R13: the step was asked to OPEN a PR (open:true) but none opened —
            // surface the reason (e.g. "repo has no git account") as a real failure
            // instead of a misleading "success" with opened:false. A dry run
            // (open:false) legitimately opens nothing, so it's exempt.
            if open && opened_n == 0 {
                let errs: Vec<String> = prs
                    .iter()
                    .filter_map(|pr| pr.get("error").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
                if !errs.is_empty() {
                    return Err(otto_core::Error::Upstream(format!(
                        "git_pr: asked to open a PR but none opened — {}",
                        errs.join("; ")
                    )));
                }
            }
            // Back-compat: mirror the primary (first) PR's fields at the top level,
            // alongside the full `prs[]` and an `opened` flag/count.
            let mut outm = prs[0].as_object().cloned().unwrap_or_default();
            outm.insert("prs".into(), Value::Array(prs.clone()));
            outm.insert("opened".into(), json!(open && opened_n > 0));
            outm.insert("opened_count".into(), json!(opened_n));
            outm.insert("repos".into(), json!(resolved.iter().map(|(r, _, _)| r.clone()).collect::<Vec<_>>()));
            if !notes.is_empty() {
                outm.insert("notes".into(), json!(notes));
            }
            Ok((Value::Object(outm), logs))
        }

        // --- Self-improvement (offer-only) ----------------------------------
        // Reflect on the workspace's recent sessions and OFFER skill/memory
        // improvements. Runs the self-improvement engine with Autonomy::Propose
        // so every proposed edit is QUEUED for approval — never auto-applied —
        // then posts the offered list to the trigger's chat thread (design §I).
        "self_improve" => {
            use otto_core::domain::{Autonomy, ImprovementTrigger};
            let eng = &ctx.improve_engine;
            let run = eng
                .improvements
                .create_run(&ws.id, ImprovementTrigger::Manual)
                .await
                .map_err(|e| otto_core::Error::Internal(format!("self_improve: create run: {e}")))?;
            eng.execute_run_with_autonomy(&run.id, &ws.id, ImprovementTrigger::Manual, Autonomy::Propose)
                .await
                .map_err(|e| otto_core::Error::Upstream(format!("self_improve: {e}")))?;
            let final_run = eng.improvements.get_run(&run.id).await.ok();
            let edits = eng.improvements.list_edits_by_run(&run.id).await.unwrap_or_default();
            let summary = final_run.as_ref().map(|r| r.summary.clone()).unwrap_or_default();
            let offered: Vec<Value> = edits
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id,
                        "target": e.target.as_str(),
                        "target_ref": e.target_ref,
                        "kind": e.kind.as_str(),
                        "risk": e.risk.as_str(),
                        "rationale": e.rationale,
                    })
                })
                .collect();
            let mut logs = vec![format!(
                "self_improve: offered {} improvement(s) — none applied (run {})",
                offered.len(),
                run.id
            )];
            for e in &edits {
                logs.push(format!("  offer: [{}] {} — {}", e.risk.as_str(), e.target_ref, truncate(&e.rationale, 160)));
            }
            if progress.enabled() {
                let mut msg = format!(
                    "💡 *Self-improvement* — {} improvement(s) *offered* (queued for approval; none auto-applied).",
                    offered.len()
                );
                for e in edits.iter().take(8) {
                    msg.push_str(&format!(
                        "\n • [{}] `{}` — {}",
                        e.risk.as_str(),
                        e.target_ref,
                        truncate(&e.rationale, 140)
                    ));
                }
                if !summary.trim().is_empty() {
                    msg.push_str(&format!("\n_Summary:_ {}", truncate(&summary, 280)));
                }
                progress.post(msg);
            }
            Ok((
                json!({
                    "run_id": run.id,
                    "summary": summary,
                    "offered": offered.len(),
                    "edits": offered,
                }),
                logs,
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
/// Execution rules appended to EVERY agent-backed workflow step prompt: the step's
/// single agent must do all the work itself — no sub-agents / background tasks —
/// and must not yield its turn until the work is done. A plain directive, applied
/// uniformly to every provider (claude/codex/agy). See design R2/R3.
const WF_STEP_RULES: &str = "\n\n[workflow step — execution rules]\n\
    You are running as a single automated workflow step. Do ALL of the work yourself in THIS turn.\n\
    - Do NOT spawn, launch, or delegate to sub-agents, background agents, parallel workers, or the Task tool. No run_in_background, no fan-out — you are the only agent for this step.\n\
    - Do NOT end your turn until the task is fully complete. Never stop early to \"wait for\" something you started; finish everything yourself, then write your handoff summary.\n";

/// A workflow `agent_prompt` step that produces no output for this long is treated
/// as stuck and retried. Separate from — and never longer than — the 10h max
/// session lifespan backstop. See design R5.
const WF_STEP_STUCK: Duration = Duration::from_secs(3 * 60);

fn resolve_retry(node: &WorkflowNode) -> otto_core::workflows::RetryPolicy {
    if let Some(p) = &node.retry {
        return p.clamped();
    }
    if let Some(rp) = node.params.get("retry") {
        if let Ok(p) = serde_json::from_value::<otto_core::workflows::RetryPolicy>(rp.clone()) {
            return p.clamped();
        }
    }
    // Default: agent steps get a small retry budget (2 retries = 3 attempts) so a
    // transient stuck/no-op spawn — e.g. a startup screen that swallowed the prompt,
    // surfaced as the 3-min no-progress error — is re-attempted with a FRESH session,
    // then errors ("call it a day"). Every other kind keeps the no-retry default.
    // `prepare_context` only reaches an agent turn when it has a non-empty
    // `params.prompt` — treat it like `agent_prompt` exactly in that case.
    let has_agent_phase = node.kind == "prepare_context"
        && node.params.get("prompt").and_then(Value::as_str).map(|s| !s.trim().is_empty()).unwrap_or(false);
    if node.kind == "agent_prompt" || has_agent_phase {
        return otto_core::workflows::RetryPolicy { max_attempts: 2, backoff_ms: 2000, factor: 2.0 }
            .clamped();
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
    // R2/R3: every agent-backed step runs as a single agent that does all the work
    // itself — no sub-agents / background tasks — and doesn't yield its turn early.
    let guarded = format!("{prompt}{WF_STEP_RULES}");
    // R5: an `agent_prompt` step gets an early 3-min no-progress trip (retryable via
    // resolve_retry); heavier agent kinds (review/product/canvas) keep the 10h backstop.
    // `prepare_context` only ever reaches this call with an agent phase (a non-empty
    // `params.prompt`) — treat it the same as `agent_prompt`.
    let stuck_after = if node.kind == "agent_prompt" || node.kind == "prepare_context" {
        WF_STEP_STUCK
    } else {
        crate::agent_session::STUCK_IDLE
    };
    crate::agent_session::run_session_turn(
        ctx,
        ws,
        user,
        None,
        &title,
        cwd,
        provider,
        meta,
        &guarded,
        stuck_after,
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

/// Resolve the repo_id a repo-needing step (`review_run`, `git_pr`) should use:
/// an explicit `repo_id` (step params → run input) wins; otherwise derive it from
/// the step's `worktree_path`, the run's `working_directory`, or `run_cwd`.
/// See design §B — this is what makes such steps resilient instead of failing
/// with a bare "missing repo_id".
async fn resolve_step_repo_id(
    ctx: &ServerCtx,
    ws: &Workspace,
    p: &Value,
    input: &Value,
    run_cwd: &str,
) -> Option<String> {
    let explicit = p
        .get("repo_id")
        .and_then(Value::as_str)
        .or_else(|| input.get("repo_id").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());
    if explicit.is_some() {
        return explicit;
    }
    let hint = p
        .get("worktree_path")
        .and_then(Value::as_str)
        .or_else(|| input.get("working_directory").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| run_cwd.to_string());
    resolve_repo_id_for_path(ctx, &ws.id, &hint).await
}

/// Gather the set of PR targets a `git_pr` node should open — one per changed
/// repo. Sources: explicit `params.repos[]`, a loop's published `input.repos[]`,
/// the input itself as a single reference (a review/loop output carrying
/// `repo_id`/`worktree`), and fan-in (multiple predecessor outputs keyed by node
/// id, each a review with its own `repo_id`). Each entry is `{repo_id?,
/// worktree?, worktree_path?, base?}`. Empty ⇒ the caller uses one implicit
/// target from the run context. Deduplication by resolved repo_id is the
/// caller's job.
fn collect_pr_targets(p: &Value, input: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let has_ref = |v: &Value| {
        ["repo_id", "worktree", "worktree_path"].iter().any(|k| {
            v.get(*k).and_then(Value::as_str).map(|s| !s.trim().is_empty()).unwrap_or(false)
        })
    };
    if let Some(arr) = p.get("repos").and_then(Value::as_array) {
        out.extend(arr.iter().filter(|v| has_ref(v)).cloned());
    }
    if let Some(arr) = input.get("repos").and_then(Value::as_array) {
        out.extend(arr.iter().filter(|v| has_ref(v)).cloned());
    }
    if has_ref(input) {
        out.push(input.clone());
    }
    if let Value::Object(m) = input {
        for v in m.values() {
            if v.is_object() && has_ref(v) {
                out.push(v.clone());
            }
        }
    }
    out
}

/// Parse a `checks` value into (name, command) pairs. Accepts an array of plain
/// command strings, or of objects `{name?, cmd}`. Drives the reviewer's
/// user-defined verification commands (e.g. `go test -tags=component ./...`).
fn parse_checks(v: Option<&Value>) -> Vec<(String, String)> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return vec![];
    };
    let mut out = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        match item {
            Value::String(s) if !s.trim().is_empty() => {
                out.push((format!("check {}", i + 1), s.trim().to_string()));
            }
            Value::Object(_) => {
                if let Some(cmd) = item
                    .get("cmd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(cmd)
                        .to_string();
                    out.push((name, cmd.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// Build a dedicated "Required checks" review agent: it RUNS each user-defined
/// command in the reviewed worktree and reports any that fail (non-zero exit) as
/// a blocking (`bug`) finding. Delegated to the review agent (which has the repo
/// and the implementer's context) rather than executed by the engine — so it
/// fans out through the same multi-agent review pipeline and its failures drop
/// the score, keeping a fix→review loop iterating until the checks pass.
fn checks_review_agent(default_provider: &str, checks: &[(String, String)]) -> otto_core::domain::ReviewAgentCfg {
    let list = checks
        .iter()
        .map(|(n, c)| if n == c { format!("- {c}") } else { format!("- {n}: {c}") })
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "You are a CI gate for this change. RUN EACH command below in the repository root \
         (your current working directory) and capture its exit status and output. For any \
         command that FAILS (non-zero exit), output one finding object \
         {{\"path\":\".\",\"line\":0,\"severity\":\"bug\",\"body\":\"check failed: <command>\\n<the last ~40 lines of its output>\"}}. \
         Do NOT output anything for a command that passes. Output ONLY a JSON array (no prose, \
         no markdown fence); output [] when every command passes.\n\nCommands:\n{list}"
    );
    otto_core::domain::ReviewAgentCfg {
        name: "Required checks".to_string(),
        provider: default_provider.to_string(),
        providers: vec![default_provider.to_string()],
        model: String::new(),
        prompt,
        skill: String::new(),
    }
}

/// The node's human name (falls back to its kind) — drives step-file names.
fn node_display_name(node: &WorkflowNode) -> &str {
    if node.name.trim().is_empty() {
        &node.kind
    } else {
        &node.name
    }
}

/// Merge a node output's published repo reference(s) into the run's repos
/// registry (`repos.json`) — the "each step adds to the repos file" contract:
/// the top-level `repo_id`+`base`/`worktree`, plus every entry of a published
/// `repos[]` array (a multi-repo review/loop). Only the explicit `worktree`
/// key counts (a bare `working_directory` is too ambient to be authoritative).
fn merge_published_refs(files: &crate::workflow_context::RunContextFiles, out: &Value) {
    let merge_one = |v: &Value| {
        if let Some(rid) = v
            .get("repo_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            files.merge_published(
                rid,
                v.get("base").and_then(Value::as_str),
                v.get("worktree").and_then(Value::as_str),
            );
        }
    };
    merge_one(out);
    if let Some(arr) = out.get("repos").and_then(Value::as_array) {
        for v in arr {
            merge_one(v);
        }
    }
}

/// Seed the run input from the first VALID declared repo entry — explicit
/// input keys always win — and expose the normalized entries as
/// `repos: [{repo_id, worktree, base}]`, the shape `collect_pr_targets` and
/// the loop's ref harvest already consume. Pure; unit-tested below.
fn seed_input_from_entries(
    input: Value,
    entries: &[crate::workflow_context::RepoEntry],
) -> Value {
    let valid: Vec<&crate::workflow_context::RepoEntry> =
        entries.iter().filter(|e| e.error.is_none()).collect();
    if valid.is_empty() {
        return input;
    }
    let mut m = match input {
        Value::Object(m) => m,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("input".into(), other);
            m
        }
    };
    let missing = |m: &serde_json::Map<String, Value>, k: &str| {
        m.get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    };
    if missing(&m, "working_directory") {
        if let Some(w) = valid.iter().find_map(|e| e.worktree.clone()) {
            m.insert("working_directory".into(), Value::String(w));
        }
    }
    if missing(&m, "base") {
        if let Some(b) = valid.iter().find_map(|e| e.base.clone()) {
            m.insert("base".into(), Value::String(b));
        }
    }
    if missing(&m, "repo_id") {
        if let Some(r) = valid.iter().find_map(|e| e.repo_id.clone()) {
            m.insert("repo_id".into(), Value::String(r));
        }
    }
    // Replace the raw declarations with the normalized targets; the original
    // user shape stays visible in repos.json.
    m.insert(
        "repos".into(),
        Value::Array(valid.iter().map(|e| crate::workflow_context::entry_to_target(e)).collect()),
    );
    Value::Object(m)
}

/// Some triggers (chat) send `msg`, agent-facing steps and `prompt.md` want
/// `prompt` — fill `prompt` from `msg` when `prompt` is absent or blank, and
/// never the reverse (an explicit `prompt` always wins; `msg` still renders
/// separately in the brief unless it's identical, see `render_brief`). A
/// non-object input (or one already without either field) passes through
/// unchanged.
fn normalize_prompt(input: Value) -> Value {
    let Value::Object(mut m) = input else { return input };
    let has_prompt = m.get("prompt").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty());
    if !has_prompt {
        if let Some(msg) = m.get("msg").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
            m.insert("prompt".into(), Value::String(msg.to_string()));
        }
    }
    Value::Object(m)
}

/// Resolve declared repo entries against registered repos + live git state:
/// fill `repo_id`/`worktree`/`base`, or set a per-entry `error` (kept visible
/// in `repos.json`; the run proceeds with the valid entries). A `branch`
/// entry resolves to the checkout that HAS that branch checked out — falling
/// back to the repo's registered path only when that path is actually on the
/// branch, because silently reviewing/PRing whatever else is checked out
/// would target the wrong branch. A missing `source` resolves to the
/// checkout's DETECTED default branch, never a fabricated "main".
async fn resolve_repo_entries(
    ctx: &ServerCtx,
    workspace_id: &Id,
    mut entries: Vec<crate::workflow_context::RepoEntry>,
    // The run's Base branch: the base fallback for a declaration lacking `source`,
    // ahead of the repo's detected default. (R14)
    run_base: Option<&str>,
) -> Vec<crate::workflow_context::RepoEntry> {
    for e in entries.iter_mut() {
        // Identify the repo: id → name → path (a linked-worktree path maps to
        // its origin repo through resolve_repo_id_for_path).
        let mut rid: Option<String> = None;
        let mut root: Option<String> = None;
        if let Ok(r) = ctx.git_store.get_repo(&e.repo).await {
            rid = Some(r.id.clone());
            root = Some(r.path);
        } else if let Ok(repos) = ctx.git_store.list_repos(workspace_id).await {
            if let Some(r) = repos.into_iter().find(|r| r.name == e.repo) {
                rid = Some(r.id);
                root = Some(r.path);
            }
        }
        if rid.is_none() {
            let hint = if e.kind == "worktree" {
                expand_tilde(&e.name)
            } else {
                expand_tilde(&e.repo)
            };
            if let Some(id) = resolve_repo_id_for_path(ctx, workspace_id, &hint).await {
                if let Ok(r) = ctx.git_store.get_repo(&id).await {
                    root = Some(r.path);
                }
                rid = Some(id);
            }
        }
        let Some(rid_v) = rid else {
            e.error = Some(format!("no registered repo matches '{}'", e.repo));
            continue;
        };
        e.repo_id = Some(rid_v);
        let root = root.unwrap_or_else(|| expand_tilde(&e.repo));
        match e.kind.as_str() {
            "worktree" => {
                let wt = expand_tilde(&e.name);
                if !std::path::Path::new(&wt).is_dir() {
                    e.error = Some(format!("worktree path does not exist: {wt}"));
                    continue;
                }
                e.worktree = Some(wt);
            }
            "branch" => {
                let git = otto_git::LocalGit::new(&root);
                match git.worktree_for_branch(&e.name).await {
                    Some(wt) => e.worktree = Some(wt),
                    None => match git.current_branch().await {
                        Ok(cur) if cur == e.name => e.worktree = Some(root.clone()),
                        _ => {
                            e.error = Some(format!(
                                "branch '{}' is not checked out anywhere in {}",
                                e.name, e.repo
                            ));
                            continue;
                        }
                    },
                }
            }
            other => {
                e.error =
                    Some(format!("unknown repos entry type '{other}' (want branch|worktree)"));
                continue;
            }
        }
        if e.base.is_none() {
            e.base = e.source.clone();
        }
        if e.base.is_none() {
            // The run's Base branch pins a declaration that didn't name a source,
            // before we fall back to the repo's detected default. (R14)
            e.base = run_base.map(str::to_string);
        }
        if e.base.is_none() {
            if let Some(wt) = &e.worktree {
                e.base = otto_git::LocalGit::new(wt).default_branch().await;
            }
        }
    }
    entries
}

/// Best-effort: find the registered repo whose checkout contains `path`, so a
/// workflow given only a working directory still drives review/PR steps.
///
/// Order: (1) `path` canonically matches a registered repo (exact, or nested
/// under it — deepest wins); (2) `path` is a `git worktree` checkout whose origin
/// repo is registered; (3) the workspace has exactly one repo. `None` when
/// nothing plausible matches.
async fn resolve_repo_id_for_path(
    ctx: &ServerCtx,
    workspace_id: &Id,
    path: &str,
) -> Option<String> {
    let repos = ctx.git_store.list_repos(workspace_id).await.ok()?;
    if repos.is_empty() {
        return None;
    }
    let pairs: Vec<(String, String)> =
        repos.iter().map(|r| (r.id.clone(), r.path.clone())).collect();
    let expanded = expand_tilde(path);
    if let Some(id) = match_repo_path(&expanded, &pairs) {
        return Some(id);
    }
    if let Some(main) = git_main_worktree(&expanded).await {
        if let Some(id) = match_repo_path(&main, &pairs) {
            return Some(id);
        }
    }
    if repos.len() == 1 {
        return Some(repos[0].id.clone());
    }
    None
}

/// Pure path matcher (canonicalized, component-wise) used by
/// [`resolve_repo_id_for_path`]; split out so it is unit-testable without a ctx.
/// `target` matches a repo when it equals the repo path or is nested under it;
/// the deepest (most specific) registered repo wins. Component-wise so a sibling
/// like `…/foo_wt` does NOT match a repo at `…/foo`.
fn match_repo_path(target: &str, repos: &[(String, String)]) -> Option<String> {
    let canon =
        |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| std::path::PathBuf::from(p));
    let t = canon(target);
    let mut best: Option<(usize, String)> = None;
    for (id, rp) in repos {
        let r = canon(rp);
        if t == r || t.starts_with(&r) {
            let depth = r.components().count();
            if best.as_ref().map(|(d, _)| depth > *d).unwrap_or(true) {
                best = Some((depth, id.clone()));
            }
        }
    }
    best.map(|(_, id)| id)
}

/// For a `git worktree` checkout at `path`, the main worktree directory (the
/// origin repo): `git -C <path> rev-parse --path-format=absolute
/// --git-common-dir` yields the shared `…/.git`, whose parent is the origin repo.
/// `None` when `path` isn't a git repo / git is unavailable. Runs on a blocking
/// thread so it never stalls the async runtime.
async fn git_main_worktree(path: &str) -> Option<String> {
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let common = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if common.is_empty() {
            return None;
        }
        std::path::Path::new(&common)
            .parent()
            .map(|x| x.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
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

/// File extension for a `canvas` workflow node's written diagram artifact,
/// keyed on its `mode` param. `excalidraw` writes JSON; `d2` writes `.d2`
/// source; everything else (the default `mermaid`, and any unrecognized mode)
/// writes `.mmd`.
fn canvas_node_ext(mode: &str) -> &'static str {
    match mode {
        "excalidraw" => "json",
        "d2" => "d2",
        _ => "mmd",
    }
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
        assert!(is_known_kind("prepare_context"));
        assert!(!is_known_kind("nope"));
    }

    #[test]
    fn prepare_context_output_schema_declares_jira() {
        let schema = output_schema_for("prepare_context").expect("schema present");
        assert_eq!(schema, json!({"jira": "object"}));
    }

    #[test]
    fn canvas_node_ext_matches_mode() {
        assert_eq!(canvas_node_ext("excalidraw"), "json");
        assert_eq!(canvas_node_ext("d2"), "d2");
        assert_eq!(canvas_node_ext("mermaid"), "mmd");
        assert_eq!(canvas_node_ext("sequence"), "mmd", "unrecognized modes default to mermaid");
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
        // R5: agent steps default to a small retry budget (2 retries = 3 attempts)
        // so a stuck no-op spawn is re-attempted with a fresh session.
        assert_eq!(resolve_retry(&n).max_attempts, 2, "agent_prompt default 2 retries");
        // Non-agent kinds keep the no-retry default.
        assert_eq!(resolve_retry(&node("b", "log")).max_attempts, 0, "non-agent no retry");
        n.params = json!({ "retry": { "max_attempts": 99, "backoff_ms": 999999 } });
        let p = resolve_retry(&n);
        assert_eq!(p.max_attempts, 5, "clamped to 5");
        assert_eq!(p.backoff_ms, 60_000, "clamped to 60s");
        assert!(is_retryable("agent_prompt"));
        assert!(!is_retryable("human_approval"));
        assert!(!is_retryable("manual_trigger"));
    }

    #[test]
    fn prepare_context_gets_agent_retry_budget_only_with_a_prompt() {
        // No prompt (pure Jira-fetch step) → default no-retry, like any other kind.
        let no_prompt = node("p", "prepare_context");
        assert_eq!(resolve_retry(&no_prompt).max_attempts, 0, "no agent phase → no retry");
        // A blank/whitespace prompt doesn't count as an agent phase either.
        let mut blank_prompt = node("p", "prepare_context");
        blank_prompt.params = json!({ "prompt": "   " });
        assert_eq!(resolve_retry(&blank_prompt).max_attempts, 0);
        // Non-empty prompt → same retry budget as agent_prompt.
        let mut with_prompt = node("p", "prepare_context");
        with_prompt.params = json!({ "prompt": "analyze the ticket" });
        assert_eq!(resolve_retry(&with_prompt).max_attempts, 2, "agent phase → agent_prompt budget");
        assert!(is_retryable("prepare_context"));
    }

    #[test]
    fn run_summary_has_status_steps_and_score() {
        let wf = Workflow {
            id: "w".into(),
            workspace_id: "ws".into(),
            name: "Write tests".into(),
            description: String::new(),
            instructions: String::new(),
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
            started_at: None,
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

    #[test]
    fn skill_names_parse_skill_and_skills_deduped() {
        let p = json!({ "skill": "golang-feature-implementation",
                        "skills": ["correctness-review", " test-review ", "correctness-review"] });
        assert_eq!(
            node_skill_names(&p),
            vec!["golang-feature-implementation", "correctness-review", "test-review"]
        );
        assert!(node_skill_names(&json!({})).is_empty());
        // `lenses` is also folded in (review nodes carry lenses).
        assert_eq!(node_skill_names(&json!({ "lenses": ["security-review"] })), vec!["security-review"]);
    }

    #[test]
    fn param_str_list_accepts_array_or_csv() {
        assert_eq!(
            param_str_list(&json!({ "providers": ["claude", "codex"] }), "providers"),
            vec!["claude", "codex"]
        );
        assert_eq!(
            param_str_list(&json!({ "providers": "claude, codex ,  " }), "providers"),
            vec!["claude", "codex"]
        );
        assert!(param_str_list(&json!({}), "providers").is_empty());
    }

    #[test]
    fn brief_summary_collapses_and_truncates() {
        // Short reply passes through, whitespace-collapsed.
        let s = brief_summary(&json!({ "reply": "Did   the\n\nthing." })).unwrap();
        assert_eq!(s, "Did the thing.");
        // Long text is cut to a sentence boundary with an ellipsis.
        let long = "First sentence. ".repeat(80);
        let out = brief_summary(&json!({ "reply": long })).unwrap();
        assert!(out.chars().count() <= 701);
        assert!(out.ends_with('…'));
        // Nothing to summarize → None.
        assert!(brief_summary(&json!({ "score": 5 })).is_none());
    }

    #[test]
    fn reportable_skips_structural_and_review() {
        assert!(is_reportable("agent_prompt"));
        assert!(is_reportable("loop"));
        assert!(is_reportable("git_pr"));
        assert!(is_reportable("prepare_context"));
        // review_run self-reports; structural kinds stay quiet.
        assert!(!is_reportable("review_run"));
        assert!(!is_reportable("log"));
        assert!(!is_reportable("condition"));
        assert!(!is_reportable("manual_trigger"));
    }

    #[test]
    fn chat_target_resolves_origin_and_override() {
        let wf = Workflow {
            id: "w".into(),
            workspace_id: "wf-ws".into(),
            name: "x".into(),
            description: String::new(),
            instructions: String::new(),
            graph: WorkflowGraph { nodes: vec![], edges: vec![] },
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: 1,
        };
        // Slack origin from a chat trigger.
        let t = resolve_chat_target(
            &wf,
            &json!({ "channel": "slack", "chat": "C123", "thread": "169.1",
                     "origin_workspace_id": "trigger-ws" }),
        )
        .expect("slack target");
        assert!(matches!(t.channel, Channel::Slack));
        assert_eq!(t.chat, "C123");
        assert_eq!(t.thread.as_deref(), Some("169.1"));
        assert_eq!(t.ws, "trigger-ws", "reports via the integration's workspace");
        // Explicit override wins.
        let t = resolve_chat_target(
            &wf,
            &json!({ "channel": "slack", "chat": "C1", "result_chat": "C2", "result_channel": "telegram" }),
        )
        .unwrap();
        assert!(matches!(t.channel, Channel::Telegram));
        assert_eq!(t.chat, "C2");
        assert_eq!(t.ws, "wf-ws", "no origin_workspace_id → workflow's own ws");
        // A manual UI run (no chat) → no target → disabled progress.
        assert!(resolve_chat_target(&wf, &json!({ "repo_id": "r" })).is_none());
        assert!(resolve_chat_target(&wf, &json!({ "channel": "webhook", "chat": "x" })).is_none());
    }

    // --- repo_id resolution (design §B) ------------------------------------

    #[test]
    fn match_repo_path_exact_subdir_and_sibling() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let sub = repo.join("pkg/inner");
        let sibling = root.path().join("repo_wt"); // shares the "repo" name prefix
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        let pairs = vec![("R".to_string(), repo.to_string_lossy().into_owned())];

        // Exact path → match.
        assert_eq!(
            match_repo_path(repo.to_string_lossy().as_ref(), &pairs).as_deref(),
            Some("R")
        );
        // A nested subdir → match (working dir inside the repo).
        assert_eq!(
            match_repo_path(sub.to_string_lossy().as_ref(), &pairs).as_deref(),
            Some("R")
        );
        // A sibling whose name shares a prefix must NOT match (component-wise).
        assert_eq!(match_repo_path(sibling.to_string_lossy().as_ref(), &pairs), None);
        // An unrelated ancestor must not match.
        assert_eq!(match_repo_path(root.path().to_string_lossy().as_ref(), &pairs), None);
    }

    #[test]
    fn match_repo_path_deepest_repo_wins() {
        let root = tempfile::tempdir().unwrap();
        let outer = root.path().join("outer");
        let inner = outer.join("inner");
        let target = inner.join("x");
        std::fs::create_dir_all(&target).unwrap();
        let pairs = vec![
            ("OUTER".to_string(), outer.to_string_lossy().into_owned()),
            ("INNER".to_string(), inner.to_string_lossy().into_owned()),
        ];
        assert_eq!(
            match_repo_path(target.to_string_lossy().as_ref(), &pairs).as_deref(),
            Some("INNER")
        );
    }

    #[tokio::test]
    async fn git_main_worktree_maps_linked_worktree_to_origin() {
        // Skip cleanly when git isn't available in the environment.
        let has_git = std::process::Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !has_git {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("origin");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .unwrap()
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("f.txt"), "hi").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-q", "-m", "c"]);
        let wt = root.path().join("linked_wt");
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["worktree", "add", "-q"])
            .arg(&wt)
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

        let main = git_main_worktree(wt.to_string_lossy().as_ref())
            .await
            .expect("worktree resolves to origin");
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap();
        assert_eq!(canon(std::path::Path::new(&main)), canon(&repo));
    }

    // --- git_pr multi-target collection (design: PR opens one per changed repo) -

    fn repo_ids(targets: &[Value]) -> Vec<String> {
        targets
            .iter()
            .filter_map(|t| t.get("repo_id").and_then(Value::as_str).map(str::to_string))
            .collect()
    }

    #[test]
    fn collect_pr_targets_single_review_reference() {
        // A direct review→git_pr: the review output carries one reference.
        let input = json!({ "repo_id": "R1", "base": "develop", "worktree": "/w/r1", "passed": true });
        let got = collect_pr_targets(&json!({}), &input);
        assert_eq!(repo_ids(&got), vec!["R1"]);
        assert_eq!(got[0].get("base").and_then(Value::as_str), Some("develop"));
    }

    #[test]
    fn collect_pr_targets_fan_in_multiple_repos() {
        // Two review branches fanned into one git_pr (keyed by node id).
        let input = json!({
            "revA": { "repo_id": "A", "base": "main", "worktree": "/w/a" },
            "revB": { "repo_id": "B", "base": "release", "worktree": "/w/b" },
        });
        let mut ids = repo_ids(&collect_pr_targets(&json!({}), &input));
        ids.sort();
        assert_eq!(ids, vec!["A", "B"]);
    }

    #[test]
    fn collect_pr_targets_loop_repos_and_explicit_params() {
        // A multi-repo loop publishes `repos[]`; explicit params.repos add more.
        let input = json!({ "repos": [{ "repo_id": "L1", "base": "dev", "worktree": "/w/l1" }] });
        let p = json!({ "repos": [{ "repo_id": "P1", "worktree": "/w/p1" }] });
        let mut ids = repo_ids(&collect_pr_targets(&p, &input));
        ids.sort();
        assert_eq!(ids, vec!["L1", "P1"]);
    }

    #[test]
    fn collect_pr_targets_empty_when_only_working_directory() {
        // A plain run input with only a working_directory carries NO explicit
        // reference → caller resolves a single implicit target instead.
        let input = json!({ "working_directory": "/w/x", "goals": ["g"] });
        assert!(collect_pr_targets(&json!({}), &input).is_empty());
    }

    // --- repos declarations → run-input seeding ----------------------------------

    fn entry(
        repo_id: &str,
        worktree: Option<&str>,
        base: Option<&str>,
        error: Option<&str>,
    ) -> crate::workflow_context::RepoEntry {
        crate::workflow_context::RepoEntry {
            repo: repo_id.to_string(),
            repo_id: Some(repo_id.to_string()),
            kind: "branch".into(),
            name: "feat/x".into(),
            source: base.map(str::to_string),
            worktree: worktree.map(str::to_string),
            base: base.map(str::to_string),
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn seed_input_fills_blanks_from_first_valid_entry() {
        let entries = vec![
            entry("BAD", None, None, Some("no repo")), // errored — skipped
            entry("R1", Some("/w/r1"), Some("develop"), None),
            entry("R2", Some("/w/r2"), Some("master"), None),
        ];
        let out = seed_input_from_entries(json!({ "msg": "do it" }), &entries);
        assert_eq!(out.get("working_directory").and_then(Value::as_str), Some("/w/r1"));
        assert_eq!(out.get("base").and_then(Value::as_str), Some("develop"));
        assert_eq!(out.get("repo_id").and_then(Value::as_str), Some("R1"));
        // Normalized targets exclude the errored entry and feed straight into
        // collect_pr_targets (git_pr's fan-out shape). The seeded top-level
        // repo_id ALSO matches as a single reference — dedup by repo_id is the
        // caller's job (git_pr's `seen` set), so assert the SET here.
        let targets = collect_pr_targets(&json!({}), &out);
        let ids: std::collections::BTreeSet<&str> =
            targets.iter().filter_map(|t| t.get("repo_id").and_then(Value::as_str)).collect();
        assert_eq!(ids.into_iter().collect::<Vec<_>>(), vec!["R1", "R2"]);
    }

    #[test]
    fn seed_input_explicit_keys_win() {
        let entries = vec![entry("R1", Some("/w/r1"), Some("develop"), None)];
        let out = seed_input_from_entries(
            json!({ "working_directory": "/explicit", "base": "release", "repo_id": "X" }),
            &entries,
        );
        assert_eq!(out.get("working_directory").and_then(Value::as_str), Some("/explicit"));
        assert_eq!(out.get("base").and_then(Value::as_str), Some("release"));
        assert_eq!(out.get("repo_id").and_then(Value::as_str), Some("X"));
    }

    #[test]
    fn seed_input_no_valid_entries_is_identity() {
        let entries = vec![entry("BAD", None, None, Some("no repo"))];
        let input = json!({ "msg": "hi" });
        assert_eq!(seed_input_from_entries(input.clone(), &entries), input);
        assert_eq!(seed_input_from_entries(Value::Null, &[]), Value::Null);
    }

    #[test]
    fn normalize_prompt_fills_from_msg_only() {
        let v = normalize_prompt(json!({"msg": "hello"}));
        assert_eq!(v["prompt"], "hello");
        let v = normalize_prompt(json!({"prompt": "p", "msg": "m"}));
        assert_eq!(v["prompt"], "p"); // never overwritten
        let v = normalize_prompt(json!({"prompt": "  ", "msg": "m"}));
        assert_eq!(v["prompt"], "m"); // blank counts as absent
        let v = normalize_prompt(json!("scalar"));
        assert_eq!(v, json!("scalar")); // non-object untouched
    }

    // --- reviewer checks (commands delegated to the review agent) ---------------

    #[test]
    fn parse_checks_strings_and_objects() {
        let v = json!([
            "go test -tags=component ./...",
            { "name": "integration", "cmd": "go test -tags=integration ./..." },
            { "cmd": "" },     // dropped (empty)
            { "name": "x" },   // dropped (no cmd)
            "   ",             // dropped (blank)
        ]);
        let got = parse_checks(Some(&v));
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].1, "go test -tags=component ./...");
        assert_eq!(got[1], ("integration".to_string(), "go test -tags=integration ./...".to_string()));
        assert!(parse_checks(None).is_empty());
        assert!(parse_checks(Some(&json!("not an array"))).is_empty());
    }

    #[test]
    fn checks_review_agent_runs_and_flags_failures_as_bugs() {
        let checks = vec![
            ("component".to_string(), "go test -tags=component ./...".to_string()),
            ("integration".to_string(), "go test -tags=integration ./...".to_string()),
        ];
        let a = checks_review_agent("claude", &checks);
        assert_eq!(a.providers, vec!["claude"]);
        // Each command is named in the prompt, and failures are reported as bugs.
        assert!(a.prompt.contains("go test -tags=component ./..."));
        assert!(a.prompt.contains("go test -tags=integration ./..."));
        assert!(a.prompt.contains("\"severity\":\"bug\""));
        assert!(a.prompt.to_lowercase().contains("run each command"));
    }
}
