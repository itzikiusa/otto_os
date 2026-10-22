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
//! ## No node-result caching
//! Every node executes in its own run. Cross-run caching was removed (it
//! replayed outputs whose real input — repos, worktrees, external state — had
//! changed); read the "Cross-run node-result caching removed" note above
//! `RUN_WALL_CLOCK_TIMEOUT` before re-adding anything like it.
//! `workflow_node_cache` is a dormant table.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use otto_brokers::types::{ConsumeReq, ValueFormat};
use otto_channels::adapter::Adapter;
use otto_core::domain::{Channel, User, Workspace};
use otto_core::event::Event;
use otto_core::paths::expand_tilde;
use otto_core::text::{clip_bytes as truncate, extract_json};
use otto_core::workflows::{
    NodeActivity, NodeRunState, NodeStatus, RunScope, RunStatus, SubagentActivity, Workflow,
    WorkflowCheckpoint, WorkflowGraph, WorkflowNode, WorkflowRun,
};
use otto_core::{Id, Result};
use otto_dbviewer::QueryRequest;
use otto_state::{swarm::NewTask as NewSwarmTask, WorkflowsRepo};
use serde_json::{json, Value};

use crate::state::ServerCtx;
use crate::turn_oracle::{self, cap_node_logs, CompleteVia, Phase};
use otto_workflows::catalog::validate_node_output;
pub use otto_workflows::catalog::{is_known_kind, node_catalog};
pub(crate) use otto_workflows::location::{
    ensure_path_in_workspace, git_main_worktree, looks_like_path, match_repo_path,
};
use otto_workflows::retry::{outcome_error, retry_backoff};

/// Session and review associations share the ordered progress stream. Reviews
/// are recorded as soon as they exist, even when the step does not await them.
enum AgentAssociation {
    Session(String),
    Review(String),
}

fn record_association(state: &mut NodeRunState, association: AgentAssociation) -> bool {
    let (ids, id) = match association {
        AgentAssociation::Session(id) => (&mut state.sessions, id),
        AgentAssociation::Review(id) => (&mut state.review_ids, id),
    };
    if ids.contains(&id) {
        return false;
    }
    ids.push(id);
    true
}

/// Cap on a node's kept log lines. Phase lines are chatty (a 20-agent sweep
/// changes its count 40 times), so they are kept only up to here — and the cap
/// evicts ONLY phase lines, never a `▶`/`✓`/`⚠`/`↻`/`✗`, retry, edge or persist
/// line. Keeps `nodes_json` and `NODE_EVENT_MAX_BYTES` bounded. See design R5.5.
const NODE_LOG_CAP: usize = 200;

/// While a node RUNS its live log is capped here (same phase-only eviction as
/// [`NODE_LOG_CAP`], which still applies at node end): a chatty step used to
/// grow the vector without bound, and every persist re-serializes it.
const NODE_LIVE_LOG_CAP: usize = 5 * NODE_LOG_CAP;

/// Live log lines are persisted at most this often per node (plus a flush
/// every [`LOG_PERSIST_LINES`] lines): each persist rewrites the whole run's
/// `nodes_json` (O(run size)), so one write per line made a 500-step run cost
/// O(lines × run size). The run view refetches on the persisted rev, so a line
/// shows up at most this late.
const LOG_PERSIST_EVERY: Duration = Duration::from_millis(250);
/// A running node's backstop status read, in 2 s cancel-poll ticks (perf W1):
/// the first tick, then every 30 s. Cancels normally arrive as an
/// announcement (`otto_state::workflows::subscribe_cancels`) and wake the node
/// at once; this only covers a status flip that bypassed `request_cancel`.
const CANCEL_SAFETY_TICKS: u32 = 15;
/// A running node's cancel-poll tick (see [`CancelWatch`]).
const CANCEL_POLL: Duration = Duration::from_secs(2);

/// What a running node's [`CancelWatch`] woke for.
#[derive(Debug, PartialEq, Eq)]
enum CancelWake {
    /// The run's status is `canceled`: stop the node.
    Canceled,
    /// A chat `skip` marker for this step was consumed; whether the run is
    /// parked at an approval (a gate is never skippable).
    Skip { parked_at_approval: bool },
}

/// A running node's cancel watch (perf W1). It used to `get_run` — `SELECT *`
/// plus a parse of 50–200 KB of `nodes_json` — every 2 s for the life of every
/// running node. Now a cancel is ANNOUNCED by `request_cancel`
/// (`otto_state::workflows::subscribe_cancels`) and wakes the node at once;
/// the periodic tick only consumes the in-memory chat `skip` marker, and reads
/// the run's STATUS (never its body) on the first tick, every
/// [`CANCEL_SAFETY_TICKS`] after, and when a skip needs the approval flag.
struct CancelWatch {
    run_id: Id,
    rx: tokio::sync::broadcast::Receiver<Id>,
    poll: tokio::time::Interval,
    ticks: u32,
    /// An announcement for this run was received but its status check has
    /// not completed yet. A `select!` that drops [`Self::next`] mid-check
    /// (a log line won the race) used to lose the consumed announcement —
    /// the stop then waited for the 30 s safety read while the agent worked.
    pending: bool,
}

impl CancelWatch {
    fn new(run_id: &Id, period: Duration) -> Self {
        Self {
            run_id: run_id.clone(),
            rx: otto_state::workflows::subscribe_cancels(),
            poll: tokio::time::interval(period),
            ticks: 0,
            pending: false,
        }
    }

    /// Resolve on the next cancel or consumed skip marker. Cancel-safe for a
    /// `select!` arm: dropping it mid-way loses nothing but a status read.
    async fn next(&mut self, repo: &WorkflowsRepo, take_skip: impl Fn() -> bool) -> CancelWake {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            if self.pending {
                if repo.is_canceled(&self.run_id).await {
                    return CancelWake::Canceled;
                }
                self.pending = false;
            }
            tokio::select! {
                ev = self.rx.recv() => {
                    // Ours, or a lag (re-check). Closed can't happen: the bus
                    // is a static.
                    let ours = match ev {
                        Ok(id) => id == self.run_id,
                        Err(RecvError::Lagged(_)) => true,
                        Err(RecvError::Closed) => false,
                    };
                    // Checked at the top of the loop, so a drop mid-check
                    // keeps the announcement for the next call.
                    self.pending |= ours;
                }
                _ = self.poll.tick() => {
                    let safety = self.ticks.is_multiple_of(CANCEL_SAFETY_TICKS);
                    self.ticks = self.ticks.wrapping_add(1);
                    let skip = take_skip();
                    let mut parked_at_approval = false;
                    if safety || skip {
                        if let Ok(Some((status, waiting))) = repo.run_status(&self.run_id).await {
                            if status == RunStatus::Canceled {
                                return CancelWake::Canceled;
                            }
                            parked_at_approval = waiting;
                        }
                    }
                    if skip {
                        return CancelWake::Skip { parked_at_approval };
                    }
                }
            }
        }
    }
}
const LOG_PERSIST_LINES: usize = 20;

/// Block until an event `is_wake` accepts arrives, the bus lags (ours may have
/// been dropped — re-check), or `wake_at` passes. Other events are skipped; a
/// closed bus degrades to sleeping until `wake_at`. The building block of the
/// event-driven waits (a long-poll subscribes BEFORE its first read, then
/// re-reads only when this returns) that replaced fixed-interval row polls.
pub(crate) async fn wake_on_event(
    rx: &mut tokio::sync::broadcast::Receiver<Event>,
    is_wake: impl Fn(&Event) -> bool,
    wake_at: tokio::time::Instant,
) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match tokio::time::timeout_at(wake_at, rx.recv()).await {
            Err(_) => return,                        // safety / deadline tick
            Ok(Ok(ev)) if is_wake(&ev) => return,    // ours
            Ok(Ok(_)) => continue,                   // someone else's
            Ok(Err(RecvError::Lagged(_))) => return, // may have missed ours
            Ok(Err(RecvError::Closed)) => {
                tokio::time::sleep_until(wake_at).await;
                return;
            }
        }
    }
}

/// Slowest re-read of a parked `human_approval` gate. The decision is
/// announced (`approve_run` → `Event::WorkflowRunUpdated` for the gate node)
/// and wakes the gate at once; this only covers a decision written by a path
/// that does not announce.
const APPROVAL_SAFETY_RECHECK: Duration = Duration::from_secs(30);

/// Whether `ev` announces a decision on `run_id`'s approval gate `node_id`:
/// `approve_run` emits a NON-waiting update for the gate node (the engine's
/// own pause announcement for it is a waiting one).
fn is_gate_decision(ev: &Event, run_id: &str, node_id: &str) -> bool {
    matches!(ev, Event::WorkflowRunUpdated {
        run_id: rid,
        node_id: Some(nid),
        waiting_approval: false,
        ..
    } if rid == run_id && nid == node_id)
}

/// Slowest re-read of an awaited `review_run` review. Status transitions are
/// announced (`Event::ReviewChanged`); this read also surfaces reviewer
/// sessions as they spawn, which is not announced.
const REVIEW_WAIT_SAFETY_RECHECK: Duration = Duration::from_secs(5);

/// Live-log persist coalescing for one running node (see
/// [`LOG_PERSIST_EVERY`]): the first line after a quiet spell is written at
/// once, a burst is written every [`LOG_PERSIST_LINES`] lines or on a trailing
/// flush [`LOG_PERSIST_EVERY`] after the previous log write.
#[derive(Debug, Default)]
struct LogPersist {
    /// Lines appended but not yet written.
    unpersisted: usize,
    /// When the last write for a LOG line happened.
    last: Option<Instant>,
    /// Trailing flush deadline while lines are buffered.
    flush_at: Option<Instant>,
}

impl LogPersist {
    /// A line was appended at `now`: `true` = write the run now.
    fn on_line(&mut self, now: Instant) -> bool {
        self.unpersisted += 1;
        let due = self.unpersisted >= LOG_PERSIST_LINES
            || self
                .last
                .is_none_or(|t| now.saturating_duration_since(t) >= LOG_PERSIST_EVERY);
        if due {
            self.wrote_log(now);
        } else if self.flush_at.is_none() {
            self.flush_at = self.last.map(|t| t + LOG_PERSIST_EVERY);
        }
        due
    }

    /// The trailing flush (or an `on_line` write) went out at `now`.
    fn wrote_log(&mut self, now: Instant) {
        self.last = Some(now);
        self.unpersisted = 0;
        self.flush_at = None;
    }

    /// Some other write of the run (session association, activity) carried
    /// the buffered lines too.
    fn carried(&mut self) {
        self.unpersisted = 0;
        self.flush_at = None;
    }
}

/// True in the offline E2E daemon (same test as `agent_session`'s short-circuit).
fn otto_e2e() -> bool {
    matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true"))
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
    // The node's SUMMARY (perf W5): what `/progress` serves for it — logs and
    // output stripped, `detail_version` set — so the run view applies it in
    // place instead of refetching, and the WS frame carries a few hundred
    // bytes, not up to 32 KiB of logs. Bounded by construction (error clipped,
    // activity capped); the size rule stays as a backstop. Served from the
    // projection the write that preceded this emit memoized (perf N3): no
    // second `to_value` + SHA-256, and the size comes with it.
    let node = node.and_then(|n| {
        let index = states
            .iter()
            .position(|s| std::ptr::eq(s, n))
            .or_else(|| states.iter().position(|s| s.node_id == n.node_id))?;
        otto_state::workflow_progress::memoized_node_summary(run_id, index, n)
            .ok()
            .filter(|(_, len)| *len <= NODE_EVENT_MAX_BYTES)
            .map(|(v, _)| v)
    });
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

/// Cap on the prior-step context handed to reviewers. Their prompt already
/// carries the whole diff; the briefs are there to stop them re-deriving the
/// system, not to become the bulk of the prompt.
const REVIEW_CONTEXT_CAP: usize = 48_000;

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
        let Some(path) = files.step_md_path(base_name) else {
            return;
        };
        let Ok(mut text) = std::fs::read_to_string(&path) else {
            return;
        };
        if text.len() > PROGRESS_FILE_CAP {
            let mut end = PROGRESS_FILE_CAP;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str(
                "\n… [truncated for chat — the full file is in the run's context directory]",
            );
        }
        let _ = tx.send(ProgressItem::File {
            name: format!("{base_name}.md"),
            text,
        });
    }
    fn enabled(&self) -> bool {
        self.tx.is_some()
    }
}

/// Where a run reports back to: the chat integration + channel/thread the trigger
/// arrived on. Resolved once from the run input.
struct ChatTarget {
    /// Workspace whose integration reports the run — always the workflow's own
    /// (see `result_workspace`; chat triggers only start workflows of the
    /// receiving workspace, S3-03).
    ws: String,
    channel: Channel,
    chat: String,
    thread: Option<String>,
}

/// The workspace whose Slack/Telegram integration reports a run: always the
/// workflow's own. Chat triggers only start workflows of the receiving
/// workspace (S3-03), so a legitimate `origin_workspace_id` equals it; any
/// other value is a forged/legacy input and must not borrow another
/// workspace's bot token (S3-02).
fn result_workspace(workflow: &Workflow, origin: Option<&str>) -> String {
    if let Some(o) = origin.filter(|o| *o != workflow.workspace_id) {
        tracing::warn!(workflow_id = %workflow.id, origin = o,
              "run input names another workspace as its origin — reporting via the workflow's own");
    }
    workflow.workspace_id.clone()
}

/// Resolve the chat target for live progress + result delivery from the run input.
/// Honors an explicit `result_chat`(+`result_channel`/`result_thread`) override,
/// else the incoming-hook origin (`channel`/`chat`/`thread`). Returns `None` for a
/// manual UI run or a webhook-only trigger (nothing to stream to).
fn resolve_chat_target(workflow: &Workflow, input: &Value) -> Option<ChatTarget> {
    let obj = input.as_object()?;
    let str_at = |k: &str| obj.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let ws = result_workspace(workflow, str_at("origin_workspace_id"));
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
fn spawn_progress_pump(
    ctx: ServerCtx,
    target: ChatTarget,
) -> (ProgressSink, tokio::task::JoinHandle<()>) {
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
        // One adapter for the whole run (texts + file uploads), built once: one
        // Keychain read and the shared HTTP pool, not one per progress line.
        let adapter = otto_channels::improve_notify::build_adapter(&ctx.secrets, &integ).await;
        while let Some(item) = rx.recv().await {
            match item {
                ProgressItem::Text(msg) => {
                    let msg = otto_core::redact::redact_text(&msg).value;
                    if let Some(a) = adapter.as_ref().filter(|_| !target.chat.trim().is_empty()) {
                        let _ = a
                            .send_formatted(&target.chat, target.thread.as_deref(), &msg)
                            .await;
                    }
                }
                ProgressItem::File { name, text } => {
                    // Same discipline as summary.md: redact before it leaves
                    // the machine; best-effort.
                    let text = otto_core::redact::redact_text(&text).value;
                    if let Some(adapter) = adapter.as_ref() {
                        if let Err(e) = adapter
                            .upload(
                                &target.chat,
                                target.thread.as_deref(),
                                &name,
                                text.as_bytes(),
                            )
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

// (Cross-run node-result caching removed: the cache keyed only on
// (node, params, upstream JSON input), but agent nodes' REAL input includes the
// run's repos/worktrees and external state — a multi-repo run replayed a prior
// single-repo run's checkout/vault outputs verbatim ("Success (cached)", 0ms)
// and reviewed the wrong scope. Every node now executes in its own run;
// `workflow_node_cache` stays only as a dormant table (migrations are
// append-only).)

/// Global wall-clock budget for a whole run. A run can't execute forever: once
/// the cumulative time across all nodes exceeds this, the run is failed at the
/// next node boundary (a node already executing finishes first, bounded by the
/// per-turn agent timeout). 10h is a deliberate, generous backstop — long
/// multi-step agent pipelines (write tests → review → fix-until-passing → PR) can
/// legitimately run for hours; the operator stops a run manually if they want it
/// to end sooner. This only guarantees eventual termination, it does not curtail
/// real work.
const RUN_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(10 * 60 * 60);

/// Cap on automatic restart-resumes per run: a run that keeps getting
/// interrupted (a crash-looping daemon, or a step that itself brings the
/// daemon down) must not re-execute forever — after this many resumes the
/// reconciler fails it for a human to look at.
pub const MAX_RESUME_ATTEMPTS: i64 = 2;

/// Node kinds the startup reconciler may re-execute after a restart caught
/// them MID-FLIGHT. The allowlist is deliberately explicit: an unlisted kind
/// interrupted mid-step has an UNKNOWN outcome (`git_pr` may have opened the
/// PR, `channel_notify` may have posted, `api_run`/`http_request` may have
/// fired) — replaying it silently would double its side effect, so such runs
/// fail with a pointer at the manual retry-a-step flow instead. Listed kinds
/// are safe to re-run: pure/deterministic nodes, read-only fetches, and
/// agent-backed steps that start a FRESH session in the run's still-alive
/// worktree (re-doing work costs time, not correctness).
fn is_restart_resumable_kind(kind: &str) -> bool {
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
    )
}

/// What [`reconcile_interrupted_runs`] decided for one interrupted run.
/// Produced by the pure [`classify_resume`] so the policy is unit-testable
/// without a daemon.
#[derive(Debug)]
pub enum ResumeDecision {
    /// Re-enter the run: persist `nodes` + `scope` and [`spawn_run`] it.
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
///   * paused at `human_approval` → re-enter AT the approval node (it re-parks;
///     side-effect free by construction);
///   * a node mid-flight → re-enter at it when its kind is restart-safe
///     ([`is_restart_resumable_kind`]), else fail with the step marked
///     "outcome unknown";
///   * no node mid-flight → resume the persisted scope (a re-queued retry that
///     never started), else the first still-pending node, else finish the run
///     with the status its settled nodes imply.
pub fn classify_resume(
    graph: &WorkflowGraph,
    run: &WorkflowRun,
    prior_scope: Option<RunScope>,
) -> ResumeDecision {
    let mut nodes = run.nodes.clone();
    let kind_of = |id: &str| {
        graph
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.kind.as_str())
    };
    // The node(s) a dead process left mid-flight. The engine executes
    // sequentially so there is at most one in practice; flip them ALL back to
    // Pending defensively (a stray Running state must never survive — nothing
    // would ever settle it).
    let running_ids: Vec<String> = nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Running)
        .map(|n| n.node_id.clone())
        .collect();
    // Paused at an approval? That node is the re-entry point regardless of its
    // recorded status — the pause is fully persisted and re-awaiting the
    // operator is free of side effects. But a DIFFERENT step caught
    // mid-flight wins: a stale pause flag (left by an older code path) must
    // never make a side-effect step (`git_pr`, `channel_notify`) look like a
    // resumable approval and get replayed.
    let entry = if run.waiting_approval {
        running_ids
            .iter()
            .find(|id| kind_of(id.as_str()).is_some_and(|k| k != "human_approval"))
            .cloned()
            .or_else(|| run.approval_node_id.clone())
            .or_else(|| running_ids.first().cloned())
    } else {
        running_ids.first().cloned()
    };

    if let Some(entry_id) = entry {
        let Some(kind) = kind_of(&entry_id) else {
            return ResumeDecision::Fail {
                nodes,
                error: format!(
                    "Interrupted by a daemon restart at unknown step '{entry_id}' — re-run the workflow."
                ),
            };
        };
        let checkpointed_loop = kind == "loop"
            && run.checkpoints.iter().any(|cp| cp.node_id == entry_id)
            && run
                .checkpoints
                .iter()
                .filter(|cp| {
                    cp.node_id == entry_id || cp.node_id.starts_with(&format!("{entry_id}#"))
                })
                .all(|cp| {
                    cp.status != NodeStatus::Running
                        || cp.kind == "loop"
                        || is_restart_resumable_kind(&cp.kind)
                });
        if !is_restart_resumable_kind(kind) && !checkpointed_loop {
            // Unknown outcome: mark the step itself, settle the rest.
            for n in nodes.iter_mut() {
                if n.node_id == entry_id {
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
            return ResumeDecision::Fail {
                nodes,
                error: format!(
                    "Interrupted by a daemon restart during '{entry_id}' ({kind}) — its outcome is unknown, so it was not replayed. Retry the step (or \"re-run from here\") once you've checked what landed."
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
        // that step; otherwise re-enter at the step and run its downstream.
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

    // Executing, but between node boundaries: resume at the first
    // still-pending node in topo order, or finish the run if none remain.
    if let Ok(order) = topo_order(graph) {
        if order.iter().any(|id| {
            nodes
                .iter()
                .any(|n| &n.node_id == id && n.status == NodeStatus::Pending)
        }) {
            let mut scope = prior_scope.unwrap_or_default();
            scope.continue_unfinished = true;
            return ResumeDecision::Resume { scope, nodes };
        }
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

/// Settle a run's node states for a terminal FAIL after a restart: a step
/// still `running` becomes `error` ("interrupted") — retryable, and no
/// ever-growing timer — and never-reached `pending` steps become `skipped`.
/// Settled states pass through untouched.
fn settle_interrupted_nodes(mut nodes: Vec<NodeRunState>) -> Vec<NodeRunState> {
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

/// Reconcile every run a previous daemon process left in flight: resume where
/// the workflow's `on_restart` policy and the interrupted step's kind allow,
/// fail otherwise. Replaces the old unconditional hard-fail (`Interrupted by a
/// daemon restart — re-run the workflow.`) — per-node progress was already
/// persisted after every step, so a restart no longer discards a run's work.
/// Returns `(resumed, failed_or_finished)`. Called once on daemon startup,
/// BEFORE [`resume_queued_runs`] (resumed rows go back to `pending` with node
/// progress, which the queue query ignores) and BEFORE
/// [`sweep_stale_run_worktrees`] (back-to-pending rows keep their worktrees).
pub async fn reconcile_interrupted_runs(ctx: &ServerCtx) -> (usize, usize) {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let rows = match repo.interrupted_runs().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("workflow recovery: listing interrupted runs: {e}");
            return (0, 0);
        }
    };
    let (mut resumed, mut settled) = (0, 0);
    for (mut run, scope_json) in rows {
        match repo.checkpoints(&run.id).await {
            Ok(checkpoints) => run.checkpoints = checkpoints,
            Err(error) => {
                tracing::warn!(run = %run.id, "cannot inspect loop checkpoints: {error}");
                // No evidence permits replay; the classifier rejects loops.
            }
        }
        let prior_scope: Option<RunScope> = scope_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok());
        // Workflow-level gates first: policy, retry cap, orphaned definition.
        let loaded = match repo.definition_for_run(&run).await {
            Ok(wf) => match ctx.workspaces.get(&wf.workspace_id).await {
                Ok(ws) => Ok((wf, ws)),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        let decision = match &loaded {
            Err(e) => ResumeDecision::Fail {
                nodes: run.nodes.clone(),
                error: format!("Interrupted by a daemon restart and cannot resume: {e}"),
            },
            Ok((wf, _)) if wf.on_restart == "fail" => ResumeDecision::Fail {
                nodes: run.nodes.clone(),
                error: "Interrupted by a daemon restart; resume is disabled for this workflow (on_restart = fail) — re-run it manually.".into(),
            },
            // A run parked at a human approval re-parks without side effects:
            // it never spends (or is failed for exhausting) the resume budget.
            Ok(_) if !run.waiting_approval && run.resume_attempts >= MAX_RESUME_ATTEMPTS => ResumeDecision::Fail {
                nodes: run.nodes.clone(),
                error: format!(
                    "Interrupted by a daemon restart; automatic resume exhausted after {MAX_RESUME_ATTEMPTS} attempts — use \"retry from this step\" or re-run."
                ),
            },
            Ok((wf, _)) => {
                let mut d = classify_resume(&wf.graph, &run, prior_scope);
                // Done-file oracle: the interrupted agent step may have
                // actually FINISHED between the last progress write and the
                // crash — its handoff `.md` (written by the agent, cleared
                // before every attempt) is the positive proof. Adopt it as
                // the step's output instead of re-running the whole turn.
                if let ResumeDecision::Resume { scope, nodes } = &mut d {
                    apply_done_file_oracle(ctx, &run, &wf.graph, scope, nodes).await;
                }
                d
            }
        };
        match decision {
            ResumeDecision::Resume { scope, nodes } => {
                let scope_json = serde_json::to_string(&scope).unwrap_or_else(|_| "{}".into());
                match repo
                    .prepare_resume(&run.id, &nodes, &scope_json, !run.waiting_approval)
                    .await
                {
                    Ok(rev) => {
                        let (wf, ws) = loaded.expect("Resume implies a loaded workflow");
                        tracing::info!(
                            run = %run.id, workflow = %wf.name,
                            attempt = run.resume_attempts + 1,
                            start = scope.start_node.as_deref().unwrap_or("(whole graph)"),
                            "workflow recovery: resuming interrupted run"
                        );
                        emit_run_updated(
                            ctx,
                            &run.workspace_id,
                            &run.id,
                            "pending",
                            None,
                            rev,
                            None,
                            &nodes,
                            false,
                        );
                        spawn_run(
                            ctx.clone(),
                            ws,
                            wf,
                            run.id.clone(),
                            run.input.clone(),
                            scope,
                            Some(nodes),
                        );
                        resumed += 1;
                    }
                    Err(e) => {
                        tracing::warn!(run = %run.id, "workflow recovery: prepare resume failed: {e}");
                        let _ = repo
                            .update_run(
                                &run.id,
                                RunStatus::Error,
                                &settle_interrupted_nodes(run.nodes.clone()),
                                Some("Interrupted by a daemon restart; resume bookkeeping failed — re-run the workflow."),
                                true,
                            )
                            .await;
                        settled += 1;
                    }
                }
            }
            ResumeDecision::Fail { nodes, error } => {
                // A failed run must not keep a step `running` forever (the UI
                // timer counts up and the step can't be retried): the policy
                // gates above hand over the raw snapshot.
                let nodes = settle_interrupted_nodes(nodes);
                let rev = repo
                    .update_run(&run.id, RunStatus::Error, &nodes, Some(&error), true)
                    .await
                    .unwrap_or(0);
                emit_run_updated(
                    ctx,
                    &run.workspace_id,
                    &run.id,
                    "error",
                    None,
                    rev,
                    None,
                    &nodes,
                    false,
                );
                // Tell the chat thread that started it — its last message was
                // otherwise "▶ … started" forever.
                if let Ok((wf, _)) = &loaded {
                    spawn_recovery_delivery(ctx, wf, &nodes, RunStatus::Error, &run.input);
                }
                settled += 1;
            }
            ResumeDecision::Finish {
                status,
                nodes,
                error,
            } => {
                let rev = repo
                    .update_run(&run.id, status, &nodes, error.as_deref(), true)
                    .await
                    .unwrap_or(0);
                emit_run_updated(
                    ctx,
                    &run.workspace_id,
                    &run.id,
                    status.as_str(),
                    None,
                    rev,
                    None,
                    &nodes,
                    false,
                );
                if let Ok((wf, _)) = &loaded {
                    spawn_recovery_delivery(ctx, wf, &nodes, status, &run.input);
                }
                settled += 1;
            }
        }
    }
    (resumed, settled)
}

/// Report a run settled by boot recovery back to its chat/webhook origin in
/// the BACKGROUND. Recovery runs before the listener serves; delivering inline
/// (a Keychain read + Slack/Telegram/webhook HTTP per run, serially) kept the
/// daemon and UI unreachable for as long as a post-reboot network flap or a
/// slow webhook lasted, multiplied by the interrupted runs (S3-06). Only the
/// DB writes stay on the boot path.
fn spawn_recovery_delivery(
    ctx: &ServerCtx,
    wf: &Workflow,
    nodes: &[NodeRunState],
    status: RunStatus,
    input: &Value,
) {
    let (ctx, wf, nodes, input) = (ctx.clone(), wf.clone(), nodes.to_vec(), input.clone());
    tokio::spawn(async move {
        deliver_run_result(&ctx, &wf, &nodes, status, None, &input, None).await;
    });
}

/// If the resume entry is an `agent_prompt` whose handoff `.md` exists in the
/// run's context dir and postdates the step's start, the step actually
/// finished — adopt it as Success (output = the handoff content) and set
/// `adopt_start` so the re-entry runs only its descendants. A single-step
/// (`only_node`) scope with a proven-done step leaves nothing to run — the
/// downstream Finish/adoption path handles that naturally since the node is
/// Success and the engine recomputes final status over adopted states.
async fn apply_done_file_oracle(
    ctx: &ServerCtx,
    run: &WorkflowRun,
    graph: &WorkflowGraph,
    scope: &mut RunScope,
    nodes: &mut [NodeRunState],
) {
    let Some(entry_id) = run
        .nodes
        .iter()
        .find(|n| n.status == NodeStatus::Running)
        .map(|n| n.node_id.clone())
        .or_else(|| scope.start_node.clone())
    else {
        return;
    };
    let Some(node) = graph.nodes.iter().find(|n| n.id == entry_id) else {
        return;
    };
    if node.kind != "agent_prompt" {
        return;
    }
    let Some(state) = nodes.iter_mut().find(|n| n.node_id == entry_id) else {
        return;
    };
    let (dir, slug, started_at) = (
        ctx.data_dir.join("workflow-context").join(&run.id),
        crate::workflow_context::slug(node_display_name(node)),
        state.started_at,
    );
    // Directory scan + file read: blocking pool, not a runtime worker.
    let found = crate::offload::blocking(move || find_step_handoff(&dir, &slug, started_at)).await;
    let Some(content) = found else {
        return;
    };
    // R1: the file alone is NOT proof. An agent may write its handoff while its
    // sub-agents are still running (or mid-turn) — adopting that marks the step
    // done with the work unfinished, which is the bug this batch fixes. Ask the
    // turn oracle about the interrupted session before believing the file.
    for sid in state.sessions.clone() {
        let Ok(s) = ctx.manager.get(&sid).await else {
            continue;
        };
        if s.provider != "claude" {
            continue;
        }
        let Some(psid) = s.provider_session_id.clone() else {
            continue;
        };
        let cwd = s.cwd.clone();
        // Whole-file read + parse, once per interrupted step — off the runtime.
        let scan = crate::offload::blocking(move || {
            let cwd = std::fs::canonicalize(&cwd)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(cwd);
            let path = otto_orchestrator::claude_pty::session_jsonl_path(&cwd, &psid);
            std::fs::read_to_string(&path)
                .ok()
                .map(|jsonl| turn_oracle::scan_claude(&jsonl))
        })
        .await;
        let Some(scan) = scan else {
            continue;
        };
        if !scan.pending.is_empty() || !scan.tail_is_assistant_end_turn {
            state.logs.push(format!(
                "⚠ handoff found after restart but the session had {} tasks pending / an open turn — re-running the step",
                scan.pending.len()
            ));
            return;
        }
    }
    state.status = NodeStatus::Success;
    state.error = None;
    state.output = Some(json!({ "reply": content }));
    state
        .logs
        .push("✓ adopted after daemon restart — the step's handoff file shows it finished".into());
    if !scope.continue_unfinished {
        scope.adopt_start = true;
    }
}

/// Locate the interrupted step's handoff file: `step{N}-{slug(name)}.md` in
/// `<data_dir>/workflow-context/<run_id>/`, where N (the execution-order step
/// number) is unknown after a restart — so match on the slug, require the
/// mtime to postdate the step's recorded start (a leftover from an earlier
/// attempt is cleared before each submit, but stay defensive), and require
/// non-trivial content. Returns the file's content.
#[allow(clippy::disallowed_methods)] // sync helper: apply_done_file_oracle runs it via offload::blocking
fn find_step_handoff(
    dir: &std::path::Path,
    slug: &str,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // step{N}-{slug}.md exactly (no loop-iteration/inner suffixes — those
        // belong to inner steps the loop node itself re-drives).
        let Some(rest) = name.strip_prefix("step") else {
            continue;
        };
        let Some(tail) = rest
            .split_once('-')
            .and_then(|(n, tail)| n.chars().all(|c| c.is_ascii_digit()).then_some(tail))
        else {
            continue;
        };
        if tail != format!("{slug}.md") {
            continue;
        }
        let meta = entry.metadata().ok()?;
        if let (Some(start), Ok(modified)) = (started_at, meta.modified()) {
            let modified: chrono::DateTime<chrono::Utc> = modified.into();
            if modified < start {
                continue;
            }
        }
        let content = std::fs::read_to_string(entry.path()).ok()?;
        if content.trim().len() > 40 {
            return Some(content);
        }
    }
    None
}

/// Default cap on workflow runs EXECUTING at once, daemon-wide. Every run
/// beyond it queues FIFO in [`spawn_run`] and stays `pending`. A single run
/// can fan out dozens of agent PTYs (a `review_run` node alone launches the
/// whole reviewer fleet), so uncapped parallel runs exhausted fds and starved
/// interactive sessions; 2 keeps one long run from blocking a quick one while
/// bounding the process/fd blast radius.
const DEFAULT_MAX_PARALLEL_RUNS: usize = 2;

/// The effective cap: `OTTO_WF_MAX_PARALLEL_RUNS` (≥ 1) when set, else
/// [`DEFAULT_MAX_PARALLEL_RUNS`]. Read once — the gate's permit count can't
/// change while runs hold permits.
fn max_parallel_runs() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("OTTO_WF_MAX_PARALLEL_RUNS")
            .ok()
            .and_then(|s| s.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(DEFAULT_MAX_PARALLEL_RUNS)
    })
}

/// Daemon-wide run gate. Tokio's semaphore queues waiters FIFO, so runs start
/// in the order they were requested.
fn run_gate() -> &'static tokio::sync::Semaphore {
    static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Semaphore::new(max_parallel_runs()))
}

tokio::task_local! {
    /// The run-gate permit of the run executing on this task — set by
    /// [`spawn_run`] around [`run_workflow`]. A `human_approval` gate hands it
    /// back while it waits for a human ([`release_run_permit`]) and re-takes
    /// it after the decision ([`reacquire_run_permit`]): a run parked for
    /// approval (up to 24h) used to hold one of the 2 daemon-wide slots the
    /// whole time, queueing every other workflow behind it.
    static RUN_PERMIT: std::cell::RefCell<Option<tokio::sync::SemaphorePermit<'static>>>;
}

/// Give this task's run-gate permit back while the run is parked. Returns
/// whether a permit was released (false off a `spawn_run` task, or when it
/// was already released) — pass it to [`reacquire_run_permit`].
fn release_run_permit() -> bool {
    RUN_PERMIT
        .try_with(|cell| cell.borrow_mut().take())
        .ok()
        .flatten()
        .is_some()
}

/// Re-take the run-gate permit released by [`release_run_permit`] — FIFO
/// behind runs that queued meanwhile. No-op when nothing was released.
async fn reacquire_run_permit(released: bool) {
    if !released {
        return;
    }
    if let Ok(permit) = run_gate().acquire().await {
        let _ = RUN_PERMIT.try_with(|cell| *cell.borrow_mut() = Some(permit));
    }
}

/// Key of a chat `skip` marker in `ServerCtx::wf_skip_current`: ONE step of
/// one run, so a marker can only ever skip the step it was aimed at.
pub(crate) fn skip_marker_key(run_id: &str, node_id: &str) -> String {
    format!("{run_id}/{node_id}")
}

/// Consume the skip marker for `(run_id, node_id)`; true when one was set.
fn take_skip_marker(ctx: &ServerCtx, run_id: &str, node_id: &str) -> bool {
    ctx.wf_skip_current
        .lock()
        .map(|mut s| s.remove(&skip_marker_key(run_id, node_id)))
        .unwrap_or(false)
}

/// Drop every skip marker of a run at run end — an unconsumed marker must not
/// fire on a later retry of the same run id.
fn clear_skip_markers(ctx: &ServerCtx, run_id: &str) {
    let prefix = format!("{run_id}/");
    if let Ok(mut s) = ctx.wf_skip_current.lock() {
        s.retain(|k| !k.starts_with(&prefix));
    }
}

/// The daemon side of the `otto-workflows` seam: the pieces of the workflow
/// subsystem that already live there (event triggers) reach the server
/// through these handles only.
impl otto_workflows::WorkflowCtx for ServerCtx {
    fn pool(&self) -> &otto_state::DbPool {
        &self.pool
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<Event> {
        &self.events
    }
    fn workspaces(&self) -> &otto_state::WorkspacesRepo {
        &self.workspaces
    }
    fn issues_store(&self) -> &otto_state::IssuesRepo {
        &self.issues_store
    }
    fn spawn_run(&self, ws: Workspace, wf: Workflow, run_id: Id, input: Value, scope: RunScope) {
        spawn_run(self.clone(), ws, wf, run_id, input, scope, None);
    }
    fn request_skip_current(&self, run_id: &str, node_id: &str) {
        if let Ok(mut s) = self.wf_skip_current.lock() {
            s.insert(skip_marker_key(run_id, node_id));
        }
    }
    async fn check_run_location(
        &self,
        ws: &Workspace,
        input: &Value,
    ) -> std::result::Result<(), String> {
        ensure_run_location(self, ws, input)
            .await
            .map_err(|e| match e {
                otto_core::Error::Invalid(m) => m,
                other => other.to_string(),
            })
    }
}

/// Wait for a run-gate permit — keeping the run's FIFO place — unless the
/// queued run is canceled first (`None`; also `None` when the gate closed).
async fn park_until_permit_or_cancel<'a>(
    gate: &'a tokio::sync::Semaphore,
    pool: &otto_state::DbPool,
    run_id: &Id,
) -> Option<tokio::sync::SemaphorePermit<'a>> {
    use tokio::sync::broadcast::error::RecvError;
    let mut cancels = otto_state::workflows::subscribe_cancels();
    let repo = WorkflowsRepo::new(pool.clone());
    // A cancel that landed before we subscribed.
    if repo.is_canceled(run_id).await {
        return None;
    }
    let acquire = gate.acquire();
    tokio::pin!(acquire);
    loop {
        tokio::select! {
            permit = &mut acquire => return permit.ok(),
            ev = cancels.recv() => {
                let ours = match ev {
                    Ok(id) => id == *run_id,
                    Err(RecvError::Lagged(_)) => true,
                    Err(RecvError::Closed) => false,
                };
                if ours && repo.is_canceled(run_id).await {
                    return None;
                }
            }
        }
    }
}

/// Spawn a workflow run through the daemon-wide concurrency gate. This is THE
/// way to launch [`run_workflow`] — every trigger path (manual run, retry,
/// webhook, schedule/event trigger, chat, scheduled task) goes through it so
/// at most [`max_parallel_runs`] runs execute at once and the rest wait FIFO.
///
/// The queue is PERSISTENT with no extra table: a queued run is exactly its
/// already-created `workflow_runs` row still in `pending` with no node
/// progress. If the daemon dies while runs wait, startup re-enqueues them via
/// [`resume_queued_runs`] in the same (creation) order. A run canceled while
/// queued is honored here — the permit is released without executing.
///
/// A SCOPED entry (`scope.start_node` set — a retry-a-step or a restart
/// resume) persists its scope on the row first, so a daemon death mid-retry
/// leaves enough on disk for [`reconcile_interrupted_runs`] to re-enter with
/// the same scope instead of failing the run.
pub fn spawn_run(
    ctx: ServerCtx,
    ws: Workspace,
    workflow: Workflow,
    run_id: Id,
    input: Value,
    scope: RunScope,
    prior_nodes: Option<Vec<NodeRunState>>,
) {
    // Registered BEFORE the task starts, so a retry request can never slip in
    // between "spawned" and "driver alive".
    let driver = DriverGuard::register(&run_id);
    tokio::spawn(async move {
        let _driver = driver;
        {
            let scope_json = serde_json::to_string(&scope).unwrap_or_else(|_| "{}".into());
            if let Err(e) = WorkflowsRepo::new(ctx.pool.clone())
                .set_run_resume_scope(&run_id, Some(&scope_json))
                .await
            {
                tracing::error!(%run_id, "persisting run scope failed: {e}");
                let nodes = prior_nodes.as_deref().unwrap_or(&[]);
                // Conditional: a user Cancel that landed first stays canceled
                // (no error overwrite, no misleading "error" event).
                let Ok(Some(rev)) = WorkflowsRepo::new(ctx.pool.clone())
                    .update_run_if(
                        &run_id,
                        &[RunStatus::Pending, RunStatus::Running],
                        RunStatus::Error,
                        nodes,
                        Some(&format!("cannot persist execution scope: {e}")),
                        true,
                    )
                    .await
                else {
                    return;
                };
                // Announce it: open run views otherwise kept showing "queued".
                emit_run_updated(
                    &ctx,
                    &workflow.workspace_id,
                    &run_id,
                    "error",
                    None,
                    rev,
                    None,
                    nodes,
                    false,
                );
                return;
            }
        }
        let gate = run_gate();
        let permit = match gate.try_acquire() {
            Ok(p) => p,
            Err(_) => {
                // Parked. Announce the (still all-pending) run so the sidebar
                // list picks it up; rev 0 makes clients refetch rather than
                // trust the event's empty node counts.
                tracing::info!(
                    %run_id, workflow = %workflow.name,
                    "workflow run queued ({} already executing)", max_parallel_runs()
                );
                emit_run_updated(
                    &ctx,
                    &workflow.workspace_id,
                    &run_id,
                    "pending",
                    None,
                    0,
                    None,
                    &[],
                    false,
                );
                // Watch for a cancel while parked: the driver guard is alive
                // all along, and a retry of the canceled run answered 409
                // "still stopping" for as long as the queue stayed full.
                match park_until_permit_or_cancel(gate, &ctx.pool, &run_id).await {
                    Some(p) => p,
                    None => return, // canceled while queued, or gate closed
                }
            }
        };
        // Canceled (or otherwise settled) while it sat in the queue → skip.
        let pinned_repo = WorkflowsRepo::new(ctx.pool.clone());
        let workflow = match pinned_repo.get_run(&run_id).await {
            Ok(r) if r.status == RunStatus::Pending => {
                match pinned_repo.definition_for_run(&r).await {
                    Ok(definition) => definition,
                    Err(error) => {
                        // Conditional: never overwrite a cancel that landed
                        // after the Pending read above.
                        let Ok(Some(rev)) = pinned_repo
                            .update_run_if(
                                &run_id,
                                &[RunStatus::Pending, RunStatus::Running],
                                RunStatus::Error,
                                &r.nodes,
                                Some(&error.to_string()),
                                true,
                            )
                            .await
                        else {
                            return;
                        };
                        emit_run_updated(
                            &ctx,
                            &workflow.workspace_id,
                            &run_id,
                            "error",
                            None,
                            rev,
                            None,
                            &r.nodes,
                            false,
                        );
                        return;
                    }
                }
            }
            Ok(r) => {
                tracing::info!(%run_id, status = ?r.status, "queued workflow run settled before start — skipping");
                return;
            }
            Err(e) => {
                // Leaving the row `pending` here (a transient DB error) parked
                // it until the next restart — and `has_active_run` blocked
                // every schedule/event trigger of the workflow meanwhile.
                tracing::warn!(%run_id, "queued workflow run unavailable — failing it: {e}");
                let rev = pinned_repo
                    .update_run_if(
                        &run_id,
                        &[RunStatus::Pending],
                        RunStatus::Error,
                        prior_nodes.as_deref().unwrap_or(&[]),
                        Some(&format!("could not start the queued run: {e}")),
                        true,
                    )
                    .await
                    .ok()
                    .flatten();
                if let Some(rev) = rev {
                    emit_run_updated(
                        &ctx,
                        &workflow.workspace_id,
                        &run_id,
                        "error",
                        None,
                        rev,
                        None,
                        prior_nodes.as_deref().unwrap_or(&[]),
                        false,
                    );
                }
                return;
            }
        };
        // The permit rides in a task-local so a parked approval gate can hand
        // it back (see RUN_PERMIT); it is released when the run ends.
        RUN_PERMIT
            .scope(
                std::cell::RefCell::new(Some(permit)),
                run_workflow(ctx, ws, workflow, run_id, input, scope, prior_nodes),
            )
            .await;
    });
}

/// Run ids with a live driver task (count per id: a stale driver and its
/// successor may briefly overlap). A cancel flips the row at once but the
/// driver notices only at its next poll — mid retry-backoff that used to be a
/// minute — so "Retry" on a just-canceled run started a second driver next to
/// the first: two agents in one worktree, both writing the run's nodes.
fn live_drivers() -> &'static std::sync::Mutex<HashMap<Id, usize>> {
    static LIVE: std::sync::OnceLock<std::sync::Mutex<HashMap<Id, usize>>> =
        std::sync::OnceLock::new();
    LIVE.get_or_init(Default::default)
}

/// Whether a driver task for `run_id` is still running (see [`live_drivers`]).
pub fn driver_alive(run_id: &Id) -> bool {
    live_drivers()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(run_id)
        .is_some_and(|n| *n > 0)
}

/// A run's TERMINAL CAS write, retried briefly: a transient failure (a busy
/// or locked DB) used to be logged and ignored — the row stayed
/// `pending`/`running` with no driver, and one-at-a-time admission refused
/// every later trigger of the workflow until a restart (S3-07). A write that
/// still fails is left to [`sweep_orphaned_runs`] once this driver exits.
async fn write_terminal(
    repo: &WorkflowsRepo,
    run_id: &Id,
    expected: &[RunStatus],
    status: RunStatus,
    nodes: &[NodeRunState],
    error: Option<&str>,
) -> otto_core::Result<Option<i64>> {
    write_status_retrying(repo, run_id, expected, status, nodes, error, true).await
}

/// [`write_terminal`]'s retry loop, shared with the run's START write
/// (Pending→Running): one transient failure there left the row `pending`
/// while the run executed — the UI showed it queued throughout, and every
/// `human_approval` gate answered 409 (approvals need `running`) until its
/// 24 h timeout failed the run.
async fn write_status_retrying(
    repo: &WorkflowsRepo,
    run_id: &Id,
    expected: &[RunStatus],
    status: RunStatus,
    nodes: &[NodeRunState],
    error: Option<&str>,
    terminal: bool,
) -> otto_core::Result<Option<i64>> {
    let mut attempt = 0u64;
    loop {
        match repo
            .update_run_if(run_id, expected, status, nodes, error, terminal)
            .await
        {
            Ok(r) => return Ok(r),
            Err(e) if attempt < 3 => {
                attempt += 1;
                tracing::warn!(%run_id, attempt, "workflow status write failed, retrying: {e}");
                tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// How often [`start_orphan_run_sweep`] looks for driverless live runs.
const ORPHAN_SWEEP: Duration = Duration::from_secs(60);

/// Runs errored as orphaned since boot (metric; also logged per sweep).
pub static ORPHANED_RUNS_SWEPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Start the runtime orphan-run sweep (post-listen): a `pending`/`running`
/// row with no live driver — even a queued run holds one, see [`spawn_run`]
/// — can never finish, and blocks its workflow's triggers (S3-07).
pub fn start_orphan_run_sweep(ctx: &ServerCtx) {
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let mut suspects = HashSet::new();
        loop {
            tokio::time::sleep(ORPHAN_SWEEP).await;
            suspects = sweep_orphaned_runs(&ctx, &suspects).await;
        }
    });
}

/// One sweep pass. A live row must be driverless on TWO consecutive passes
/// before it is errored, so a row created an instant before its
/// [`spawn_run`] is never mistaken for an orphan. Returns this pass's
/// suspects (driverless, first sighting).
pub(crate) async fn sweep_orphaned_runs(ctx: &ServerCtx, prev: &HashSet<Id>) -> HashSet<Id> {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let Ok(ids) = repo.list_active_run_ids_global().await else {
        return prev.clone();
    };
    let mut suspects = HashSet::new();
    for id in ids {
        if driver_alive(&id) {
            continue;
        }
        if !prev.contains(&id) {
            suspects.insert(id);
            continue;
        }
        let Ok(run) = repo.get_run(&id).await else {
            continue;
        };
        let nodes = settle_interrupted_nodes(run.nodes.clone());
        if let Ok(Some(rev)) = repo
            .update_run_if(
                &id,
                &[RunStatus::Pending, RunStatus::Running],
                RunStatus::Error,
                &nodes,
                Some("The run lost its engine driver and can't finish — re-run the workflow."),
                true,
            )
            .await
        {
            let total = ORPHANED_RUNS_SWEPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            tracing::warn!(run = %id, orphaned_total = total, "workflow orphan sweep: errored a driverless run");
            emit_run_updated(
                ctx,
                &run.workspace_id,
                &id,
                "error",
                None,
                rev,
                None,
                &nodes,
                false,
            );
            // The same cleanup a cancel gets (S3-305): a panicked driver left
            // its agents running, its chat/webhook origin on "▶ started" and
            // its worktrees provisioned.
            kill_run_sessions(ctx, &nodes).await;
            if let Ok(wf) = repo.get(&run.workflow_id).await {
                spawn_recovery_delivery(ctx, &wf, &nodes, RunStatus::Error, &run.input);
            }
            clear_skip_markers(ctx, &id);
            reap_run_worktrees(ctx, &id).await;
        }
    }
    suspects
}

/// Holds a run's [`live_drivers`] entry for the life of its driver task —
/// released on drop, including when the task panics or is aborted.
struct DriverGuard(Id);

impl DriverGuard {
    fn register(run_id: &Id) -> Self {
        *live_drivers()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(run_id.clone())
            .or_insert(0) += 1;
        Self(run_id.clone())
    }
}

impl Drop for DriverGuard {
    fn drop(&mut self) {
        let mut live = live_drivers().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = live.get_mut(&self.0) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                live.remove(&self.0);
            }
        }
    }
}

/// Re-enqueue runs that were QUEUED (fresh `pending`) when the previous daemon
/// process died, oldest first — the persistent half of the run queue. Called
/// once on daemon startup, after [`reconcile_interrupted_runs`] has resumed or
/// settled every run with node progress. Returns how many runs were re-enqueued.
pub async fn resume_queued_runs(ctx: &ServerCtx) -> usize {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let ids = match repo.queued_run_ids().await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!("workflow queue resume: listing queued runs: {e}");
            return 0;
        }
    };
    let mut resumed = 0;
    for id in ids {
        let Ok(run) = repo.get_run(&id).await else {
            continue;
        };
        // Workflow/workspace gone (deleted while queued) → the run can never
        // execute; settle it instead of leaving a forever-pending row.
        let loaded = match repo.definition_for_run(&run).await {
            Ok(wf) => match ctx.workspaces.get(&wf.workspace_id).await {
                Ok(ws) => Ok((wf, ws)),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        match loaded {
            Ok((wf, ws)) => {
                let scope = match repo.run_scope(&run.id).await {
                    Ok(scope) => scope,
                    Err(error) => {
                        let _ = repo
                            .update_run(
                                &run.id,
                                RunStatus::Error,
                                &[],
                                Some(&error.to_string()),
                                true,
                            )
                            .await;
                        continue;
                    }
                };
                spawn_run(
                    ctx.clone(),
                    ws,
                    wf,
                    run.id.clone(),
                    run.input.clone(),
                    scope,
                    None,
                );
                resumed += 1;
            }
            Err(e) => {
                let _ = repo
                    .update_run(
                        &id,
                        RunStatus::Error,
                        &[],
                        Some(&format!("queued run can no longer start: {e}")),
                        true,
                    )
                    .await;
            }
        }
    }
    resumed
}

// The node-kind catalog (`node_catalog`, `is_known_kind`, output schemas) lives in
// `otto_workflows::catalog`; re-exported above so `workflow_engine::node_catalog` keeps working.

/// Run a workflow to completion in the current task, persisting progress to the
/// `workflow_runs` row after every node. Spawn this on a background task.
///
/// Emits `Event::WorkflowRunUpdated` on the shared event bus at every node
/// transition and at run completion; the UI subscribes to these events and
/// replaces its 700ms poll loop with a WS-driven refresh (a capped poll is kept
/// as a fallback). Every node executes — there is no cross-run node cache
/// (see the "Cross-run node-result caching removed" note in this file).
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow(
    ctx: ServerCtx,
    ws: Workspace,
    workflow: Workflow,
    run_id: Id,
    input: Value,
    scope: RunScope,
    // Retry-a-step re-entry (`POST /workflow-runs/{id}/retry-node`) and the
    // restart reconciler: the run's persisted node states. Out-of-scope nodes
    // ADOPT them — status, output, sessions — so the run keeps its history
    // (siblings stay Success/Skipped instead of flipping to "outside run
    // scope") and the final status still accounts for other nodes' failures.
    // `None` for every normal run.
    prior_nodes: Option<Vec<NodeRunState>>,
) {
    run_workflow_with_budget(
        ctx,
        ws,
        workflow,
        run_id,
        input,
        scope,
        prior_nodes,
        RUN_WALL_CLOCK_TIMEOUT,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn run_workflow_with_budget(
    ctx: ServerCtx,
    ws: Workspace,
    workflow: Workflow,
    run_id: Id,
    input: Value,
    scope: RunScope,
    prior_nodes: Option<Vec<NodeRunState>>,
    active_budget: Duration,
) {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let order = match crate::workflow_validation::ensure_valid(&workflow.graph)
        .map_err(|e| e.to_string())
        .and_then(|_| topo_order(&workflow.graph))
    {
        Ok(o) => o,
        Err(e) => {
            // Conditional: an invalid graph must not turn a user's cancel
            // into an error (nor announce one).
            let Ok(Some(rev)) = repo
                .update_run_if(
                    &run_id,
                    &[RunStatus::Pending, RunStatus::Running],
                    RunStatus::Error,
                    &[],
                    Some(&e),
                    true,
                )
                .await
            else {
                return;
            };
            emit_run_updated(
                &ctx,
                &workflow.workspace_id,
                &run_id,
                "error",
                None,
                rev,
                None,
                &[],
                false,
            );
            return;
        }
    };

    // The set of nodes to actually execute (start-from-here / run-only); `None`
    // means the whole graph. Nodes outside the set are marked skipped. An
    // `adopt_start` scope (restart resume whose entry step provably finished)
    // treats the entry itself as done — it drops OUT of the set so the
    // adoption pass below keeps its prior Success state/output while its
    // descendants run.
    let RunScope {
        start_node,
        only_node,
        adopt_start,
        continue_unfinished,
    } = scope;
    let run_set: Option<std::collections::HashSet<String>> = match &start_node {
        None => None,
        Some(s) if only_node => {
            let mut set: std::collections::HashSet<String> = std::iter::once(s.clone()).collect();
            if adopt_start {
                // Single-step scope whose step is already proven done —
                // nothing executes; the run finalizes over adopted states.
                set.remove(s);
            }
            Some(set)
        }
        Some(s) => {
            let mut set = descendants_inclusive(&workflow.graph, s);
            if adopt_start {
                set.remove(s);
            }
            Some(set)
        }
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
            review_ids: Vec::new(),
            activity: None,
        })
        .collect();
    // Resolve the user this run acts as (for spawning visible agent sessions):
    // the user who STARTED the run when known, else the latest changer of what
    // it runs (trigger / schedule / webhook / chat runs, PC-3). Acting as the
    // starter is what lets a non-root editor attach to the sessions their own
    // run spawns — those are gated owner-or-admin, and the workflow's creator
    // is usually someone else.
    let acting = run_identity(&ctx, &run_id, &workflow.created_by).await;
    let user = resolve_run_user(&ctx, &acting).await;
    // Seed the run input from the entry manual_trigger node's configured fields
    // (its inspector — prompt/working_directory/repo_id/goals/…), letting the
    // actual /run body or chat-trigger input override per key. This is what makes
    // the Start node the place to set the input for a manual run.
    let input = {
        let mut seeded = serde_json::Map::new();
        if let Some(mt) = workflow
            .graph
            .nodes
            .iter()
            .find(|n| n.kind == "manual_trigger")
        {
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
        let mut declared =
            crate::workflow_context::parse_repo_entries(input.get("repos").unwrap_or(&Value::Null));
        // No repos declared but a `Working Directory:` was given → derive a single
        // worktree entry from it, so repos.json is built AT START by looking up the
        // registered repo AT THAT PATH (a worktree path resolves back to its origin
        // repo). This honors the originating repository's identity — name, git
        // account, default branch — instead of a later step publishing a bare,
        // unnamed entry. Explicit `repos:` declarations are never overridden.
        if declared.is_empty() {
            if let Some(wd) = input
                .get("working_directory")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                // A comma-separated `Working Directory:` declares SEVERAL repos
                // (e.g. a cross-service change reviewed as one run) — one entry
                // per path, all sharing the run's base hint.
                for path in wd.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    declared.push(crate::workflow_context::RepoEntry {
                        repo: path.to_string(),
                        repo_id: None,
                        repo_name: None,
                        kind: "worktree".into(),
                        name: path.to_string(),
                        source: None,
                        worktree: None,
                        base: None,
                        error: None,
                    });
                }
            }
        }
        let mut entries = resolve_repo_entries(
            &ctx,
            &workflow.workspace_id,
            declared,
            run_base_hint.as_deref(),
        )
        .await;
        // Put the work on a clean, ISOLATED checkout of the BASE branch: for a repo
        // we'd otherwise operate on in the user's own working copy, cut a dedicated
        // linked worktree from `base` under the data dir and repoint the entry at
        // it — so agent steps never run on whatever branch the user's checkout
        // happens to be on. Best-effort + SAFE (only creates a new worktree under
        // data_dir; never touches the user's checkout). Returns the primary
        // provisioned worktree to adopt as the run's working directory.
        let (provisioned_cwd, isolation_failures) =
            provision_wf_worktrees(&ctx, &run_id, &mut entries).await;
        files.set_repos(entries.clone());
        if !isolation_failures.is_empty() {
            // Fail LOUDLY before any step runs: never let agents work in the
            // user's own checkout because isolation silently failed.
            let msg = format!(
                "cannot isolate this run from your own checkout — {}",
                isolation_failures.join("; ")
            );
            tracing::warn!(%run_id, "{msg}");
            // Nothing ran: a retry keeps its prior step history, a fresh run
            // shows every step skipped.
            let nodes: Vec<NodeRunState> = match &prior_nodes {
                Some(prior) => prior.clone(),
                None => states
                    .iter()
                    .cloned()
                    .map(|mut s| {
                        s.status = NodeStatus::Skipped;
                        s
                    })
                    .collect(),
            };
            let rev = repo
                .update_run_if(
                    &run_id,
                    &[RunStatus::Pending, RunStatus::Running],
                    RunStatus::Error,
                    &nodes,
                    Some(&msg),
                    true,
                )
                .await
                .ok()
                .flatten();
            if let Some(rev) = rev {
                deliver_run_result(
                    &ctx,
                    &workflow,
                    &nodes,
                    RunStatus::Error,
                    None,
                    &input,
                    None,
                )
                .await;
                emit_run_updated(
                    &ctx,
                    &workflow.workspace_id,
                    &run_id,
                    "error",
                    None,
                    rev,
                    None,
                    &nodes,
                    false,
                );
            }
            reap_run_worktrees(&ctx, &run_id).await;
            return;
        }
        let input = seed_input_from_entries(input, &entries);
        match provisioned_cwd {
            Some(wt) => {
                let mut m = match input {
                    Value::Object(m) => m,
                    other => {
                        let mut m = serde_json::Map::new();
                        m.insert("input".into(), other);
                        m
                    }
                };
                // run_cwd + agent nodes follow the provisioned base worktree.
                m.insert("working_directory".into(), Value::String(wt));
                Value::Object(m)
            }
            None => input,
        }
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
    if let Some(p) = input
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
    {
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
                    if n.name.is_empty() {
                        n.kind.clone()
                    } else {
                        n.name.clone()
                    },
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
    // Resolve the workspace's effective default agent once (workspace default →
    // global default → "claude"), so nodes left on "default" honor the user's
    // configured provider instead of a bare hardcoded "claude".
    let global_default_setting = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    // `resolve_default` (not bare `resolve_provider`) so a DISABLED provider is
    // never auto-selected — it skips excluded providers, matching the UI picker.
    let default_provider = ctx.manager.providers().resolve_default(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default_setting.as_ref()),
    ]);
    let env = RunEnv {
        budget: budget::ActiveBudget::new(active_budget),
        run_id: run_id.clone(),
        wf_name: workflow.name.clone(),
        run_cwd: run_cwd.clone(),
        run_base,
        files: files.clone(),
        default_provider,
        run_input: input.clone(),
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

    // Retry re-entry: adopt the prior run's states/outputs for everything the
    // run scope excludes. In-scope nodes start fresh (Pending). Adopted Error
    // states seed `errored` so downstream decisions still see the poison.
    if let Some(prior) = &prior_nodes {
        for ps in prior {
            let out_of_scope = run_set.as_ref().is_some_and(|s| !s.contains(&ps.node_id));
            let settled = matches!(
                ps.status,
                NodeStatus::Success | NodeStatus::Skipped | NodeStatus::Error
            );
            if !(out_of_scope || continue_unfinished && settled) {
                continue;
            }
            if let Some(idx) = states.iter().position(|s| s.node_id == ps.node_id) {
                if let Some(o) = &ps.output {
                    outputs.insert(ps.node_id.clone(), o.clone());
                }
                if ps.status == NodeStatus::Error {
                    errored.insert(ps.node_id.clone());
                }
                states[idx] = ps.clone();
            }
        }
    }

    // Reconstruct adopted branch decisions without rerunning their nodes.
    for id in &order {
        let Some(node) = workflow.graph.nodes.iter().find(|n| &n.id == id) else {
            continue;
        };
        if let Some(output) = outputs.get(id) {
            let sources: Vec<String> = incoming_edges(&workflow.graph, id)
                .iter()
                .filter(|e| outputs.contains_key(&e.source) && !inactive_edges.contains(&e.id))
                .map(|e| e.source.clone())
                .collect();
            let adopted_input = assemble_input(&sources, &outputs, &input);
            let (pruned, _) = eval_outgoing(&workflow.graph, node, output, &adopted_input, &input);
            inactive_edges.extend(pruned);
        } else if states
            .iter()
            .any(|s| &s.node_id == id && s.status == NodeStatus::Skipped)
        {
            if incoming_edges(&workflow.graph, id)
                .iter()
                .any(|e| errored.contains(&e.source))
            {
                errored.insert(id.clone());
            } else {
                branch_skipped.insert(id.clone());
            }
        }
    }

    // Record which workflow version this run executed (best-effort).
    let _ = repo.set_run_version(&run_id, workflow.version).await;

    // Lifecycle transition Pending→Running — a compare-and-set. Startup
    // (repo resolution, `git worktree add` of a full tree) takes seconds; a
    // Cancel that landed in that window already wrote `canceled`, and the old
    // unconditional write flipped it back to `running` and executed the whole
    // workflow the user saw canceled. Every LATER in-loop write uses
    // `update_run_progress` (nodes only, no status) for the same reason.
    match write_status_retrying(
        &repo,
        &run_id,
        &[RunStatus::Pending],
        RunStatus::Running,
        &states,
        None,
        false,
    )
    .await
    {
        Ok(Some(rev)) => emit_run_updated(
            &ctx,
            &workflow.workspace_id,
            &run_id,
            "running",
            None,
            rev,
            None,
            &states,
            false,
        ),
        Ok(None) => {
            // Settled (canceled) before it started: execute nothing — the
            // node loop's first iteration breaks straight to the canceled
            // finalize, which reaps the worktrees just provisioned.
            tracing::info!(%run_id, "workflow run canceled during startup — not executing");
            canceled = true;
        }
        Err(e) => tracing::warn!(%run_id, "workflow run start write failed: {e}"),
    }

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
    if progress.enabled() && !canceled {
        let goals: Vec<String> = input
            .get("goals")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
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

    // Active execution budget is enforced during nodes and retry backoff,
    // including loop children, rather than only at top-level boundaries.
    // The budget is per EXECUTION (a fresh run, a retry-a-step, a restart
    // resume) and excludes time parked at a `human_approval` gate. It used to
    // be anchored to the row's `started_at`, which failed every approval given
    // after 10h (the gate itself waits up to 24h) and every retry of a run
    // older than 10h before it executed a single node. A crash-looping daemon
    // still can't extend a run indefinitely: restart resumes are capped at
    // MAX_RESUME_ATTEMPTS.

    for node_id in order {
        // Canceled before it started (the Pending→Running CAS above lost).
        if canceled {
            break;
        }
        // Honor a cancel request (the API flips the run status to Canceled).
        if repo.is_canceled(&run_id).await {
            canceled = true;
            break;
        }

        // Stop once the run has exceeded its global time budget.
        if env.budget.expired() {
            timed_out = true;
            if let Some(state) = states.iter_mut().find(|state| state.node_id == node_id) {
                state.status = NodeStatus::Error;
                state.error = Some("workflow active time budget exhausted".into());
            }
            break;
        }

        let Some(node) = workflow.graph.nodes.iter().find(|n| n.id == node_id) else {
            continue;
        };
        let idx = states.iter().position(|s| s.node_id == node_id).unwrap();

        let adopted = states[idx].status != NodeStatus::Pending
            && (continue_unfinished || run_set.as_ref().is_some_and(|set| !set.contains(&node_id)));
        if adopted {
            if matches!(states[idx].status, NodeStatus::Success | NodeStatus::Error) {
                step_counter += 1;
                if states[idx].status == NodeStatus::Success {
                    let base = crate::workflow_context::step_base_name(
                        step_counter,
                        node_display_name(node),
                        None,
                        None,
                    );
                    files.adopt_step(&base, &node.kind);
                }
            }
            continue;
        }

        // Outside the run scope (start-from-here) → skip without running. A
        // retry re-entry already adopted the node's prior state — keep it.
        if run_set.as_ref().is_some_and(|set| !set.contains(&node_id)) {
            if states[idx].status != NodeStatus::Pending {
                continue;
            }
            states[idx].status = NodeStatus::Skipped;
            states[idx].logs = vec!["outside run scope".into()];
            let rev = repo
                .update_run_progress(&run_id, &states)
                .await
                .unwrap_or(0);
            emit_run_updated(
                &ctx,
                &workflow.workspace_id,
                &run_id,
                "running",
                Some(&node_id),
                rev,
                Some(&states[idx]),
                &states,
                false,
            );
            continue;
        }

        // Decide whether to run this node based on its incoming edges. Only
        // edges whose source is within the run scope constrain control flow (a
        // start-from-here run leaves ancestors out of scope; their edges don't
        // poison or branch-skip the entry node — it falls back to the run input).
        let views: Vec<EdgeView> = incoming_edges(&workflow.graph, &node_id)
            .iter()
            .filter(|e| dependency_is_relevant(&e.source, run_set.as_ref(), prior_nodes.as_deref()))
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
                states[idx].logs = vec![SKIPPED_UPSTREAM.into()];
                errored.insert(node_id.clone());
                let rev = repo
                    .update_run_progress(&run_id, &states)
                    .await
                    .unwrap_or(0);
                emit_run_updated(
                    &ctx,
                    &workflow.workspace_id,
                    &run_id,
                    "running",
                    Some(&node_id),
                    rev,
                    Some(&states[idx]),
                    &states,
                    false,
                );
                continue;
            }
            NodeDecision::BranchSkip => {
                states[idx].status = NodeStatus::Skipped;
                states[idx].logs = vec!["skipped (branch not taken)".into()];
                branch_skipped.insert(node_id.clone());
                let rev = repo
                    .update_run_progress(&run_id, &states)
                    .await
                    .unwrap_or(0);
                emit_run_updated(
                    &ctx,
                    &workflow.workspace_id,
                    &run_id,
                    "running",
                    Some(&node_id),
                    rev,
                    Some(&states[idx]),
                    &states,
                    false,
                );
                continue;
            }
            NodeDecision::Run(satisfied) => assemble_input(&satisfied, &outputs, &input),
        };

        // (No cross-run node-result cache: every node executes in its own run —
        // see the removal note above the old `cache_exempt_kind` site.)

        let start_line = format!("▶ {} started", node.kind);
        states[idx].status = NodeStatus::Running;
        states[idx].started_at = Some(chrono::Utc::now());
        states[idx].logs = vec![start_line];
        let rev = repo
            .update_run_progress(&run_id, &states)
            .await
            .unwrap_or(0);
        // Signal node start so the UI can show live progress immediately.
        emit_run_updated(
            &ctx,
            &workflow.workspace_id,
            &run_id,
            "running",
            Some(&node_id),
            rev,
            Some(&states[idx]),
            &states,
            false,
        );
        if progress.enabled() && is_reportable(&node.kind) {
            progress.post(format!("▶ *{}* started", node_label(node)));
        }

        let started = Instant::now();
        // This node executes — it owns the next step-file number.
        step_counter += 1;
        let scope = StepScope {
            step_no: step_counter,
            iter: None,
            inner_idx: None,
        };
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
        let mut attempt_limit = policy.max_attempts + 1;
        let mut backoff = policy.backoff_ms;
        let mut retry_logs: Vec<String> = vec![];
        let (sess_tx, mut sess_rx) = tokio::sync::mpsc::unbounded_channel::<AgentAssociation>();
        // Live per-node log lines (R9): the loop node streams iteration/sub-step
        // progress here so the run detail updates AS IT RUNS. Per-node channel, so
        // lines never leak into the next node.
        let (log_tx, mut log_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        // Live sub-agent / phase snapshot of a RUNNING agent step (R1.4). Rate-
        // limited to one persist per 5s — it changes on every count change and
        // `update_run_progress` rewrites `nodes_json`.
        let (activity_tx, mut activity_rx) = tokio::sync::mpsc::unbounded_channel::<NodeActivity>();
        let mut last_activity_persist: Option<Instant> = None;
        // Live-log persist coalescing (see LOG_PERSIST_EVERY).
        let mut log_persist = LogPersist::default();
        // Snapshot when the (latest) attempt began: a step file the agent wrote
        // during a FAILED earlier attempt must not be mistaken for the winning
        // attempt's handoff (persist_step compares mtimes against this).
        // Assigned at the top of every attempt; the loop runs at least once.
        let mut attempt_started;
        // Catch a cancel WHILE this node runs — the node-boundary check only
        // fires between nodes, so without this a long agent turn would ignore a
        // cancel for up to TURN_TIMEOUT. Perf W1: a cancel is ANNOUNCED
        // (`request_cancel` → `subscribe_cancels`) and wakes the select at
        // once; the 2 s tick only consumes the in-memory chat `skip` marker,
        // and reads the run's status (status-only, never the 50–200 KB body)
        // on its first tick and then every CANCEL_SAFETY_TICKS as a backstop.
        let mut cancel_watch = CancelWatch::new(&run_id, CANCEL_POLL);
        // A chat `skip` command marks this run in `wf_skip_current`; unlike a
        // cancel it aborts only the CURRENT node, which is then marked Skipped and
        // the run continues to the next node.
        let mut skip_current = false;
        let result = loop {
            attempt += 1;
            states[idx].attempts = Some(attempt);
            states[idx].logs.push(format!(
                "{} — attempt {}/{}",
                node_display_name(node),
                attempt,
                attempt_limit
            ));
            let rev = repo
                .update_run_progress(&run_id, &states)
                .await
                .unwrap_or(0);
            emit_run_updated(
                &ctx,
                &workflow.workspace_id,
                &run_id,
                "running",
                Some(&node_id),
                rev,
                Some(&states[idx]),
                &states,
                false,
            );
            attempt_started = std::time::SystemTime::now();
            // Heap-pin the large node future before the driver's nested select frames.
            let fut = Box::pin(async {
                if node.kind == "loop" {
                    let checkpoint = loop_checkpoint(node, &node.id, 0, 0, node_input.clone());
                    crate::workflow_checkpoint::execute(
                        &repo,
                        &run_id,
                        checkpoint,
                        &policy,
                        true,
                        || {
                            execute_node(
                                &ctx,
                                &ws,
                                &user,
                                node,
                                node_input.clone(),
                                &env,
                                &scope,
                                &sess_tx,
                                &log_tx,
                                &activity_tx,
                                &progress,
                            )
                        },
                    )
                    .await
                } else {
                    execute_node(
                        &ctx,
                        &ws,
                        &user,
                        node,
                        node_input.clone(),
                        &env,
                        &scope,
                        &sess_tx,
                        &log_tx,
                        &activity_tx,
                        &progress,
                    )
                    .await
                }
            });
            let attempt_res = crate::workflow_node_driver::drive(fut, async |mut done| {
                loop {
                tokio::select! {
                    biased;
                    () = env.budget.exhausted() => {
                        timed_out = true;
                        break Err(otto_core::Error::Upstream("workflow active time budget exhausted".into()));
                    }
                    Some(sid) = sess_rx.recv() => {
                        if record_association(&mut states[idx], sid) {
                            let rev = repo
                                .update_run_progress(&run_id, &states)
                                .await
                                .unwrap_or(0);
                            // The write carried any buffered log lines too.
                            log_persist.carried();
                            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
                        }
                    }
                    Some(line) = log_rx.recv() => {
                        // Live progress line (R9) — append to the node's logs. The
                        // first line after a quiet spell is written (and pushed to
                        // the run detail) at once; a burst is coalesced into one
                        // write per LOG_PERSIST_EVERY / LOG_PERSIST_LINES.
                        states[idx].logs.push(line);
                        if states[idx].logs.len() > NODE_LIVE_LOG_CAP {
                            cap_node_logs(&mut states[idx].logs, NODE_LIVE_LOG_CAP);
                        }
                        if log_persist.on_line(Instant::now()) {
                            let rev = repo
                                .update_run_progress(&run_id, &states)
                                .await
                                .unwrap_or(0);
                            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
                        }
                    }
                    () = tokio::time::sleep_until(tokio::time::Instant::from_std(
                        log_persist.flush_at.unwrap_or_else(Instant::now),
                    )), if log_persist.flush_at.is_some() => {
                        // Trailing flush of a coalesced log burst.
                        log_persist.wrote_log(Instant::now());
                        let rev = repo
                            .update_run_progress(&run_id, &states)
                            .await
                            .unwrap_or(0);
                        emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
                    }
                    Some(a) = activity_rx.recv() => {
                        // R1.4: the live sub-agent/phase snapshot. Always kept on
                        // the state (the next emit carries it), persisted at most
                        // every 5s so a chatty sweep can't hammer the run row.
                        states[idx].activity = Some(bound_activity(a));
                        if last_activity_persist.is_none_or(|t| t.elapsed() >= Duration::from_secs(5)) {
                            last_activity_persist = Some(Instant::now());
                            let rev = repo
                                .update_run_progress(&run_id, &states)
                                .await
                                .unwrap_or(0);
                            log_persist.carried();
                            emit_run_updated(&ctx, &workflow.workspace_id, &run_id, "running", Some(&node_id), rev, Some(&states[idx]), &states, false);
                        }
                    }
                    wake = cancel_watch.next(&repo, || take_skip_marker(&ctx, &run_id, &node_id)) => {
                        // Cancel drops the scoped node driver; finalization kills
                        // this run's sessions. No node task remains detached.
                        let parked_at_approval = match wake {
                            CancelWake::Canceled => {
                                canceled = true;
                                break Err(otto_core::Error::Internal("run canceled".into()));
                            }
                            CancelWake::Skip { parked_at_approval } => parked_at_approval,
                        };
                        // A chat `skip` command (consume-once) for THIS step → abort
                        // it and skip it; the run continues to the next node. The
                        // marker is keyed by (run, step), so one set for an earlier
                        // step never fires on this one before it did any work. An
                        // approval gate is never skippable — that would pass it
                        // unapproved (the chat user needn't hold approve rights).
                        // That includes a LOOP parked at an approval inside it:
                        // skipping the loop dropped the gate and let the steps
                        // after it (a PR, say) run with nobody's sign-off.
                        if node.kind == "human_approval" || parked_at_approval {
                            states[idx].logs.push(
                                "⚠ skip ignored — an approval step must be approved or rejected".into(),
                            );
                        } else {
                            skip_current = true;
                            break Err(otto_core::Error::Internal("step skipped".into()));
                        }
                    }
                    r = &mut done => break r.unwrap_or_else(|_| Err(otto_core::Error::Internal("workflow node driver closed without a result".into()))),
                }
                }
            }).await;
            match attempt_res {
                Ok(ok) => break Ok(ok),
                Err(e) => {
                    // Canceled or skipped mid-node → stop immediately, never retry.
                    if canceled || skip_current || timed_out {
                        break Err(e);
                    }
                    // R5.2: a `529 Overloaded` / fd-exhaustion failure needs a
                    // real pause (and a bigger budget) — 2s→4s just burns the
                    // attempts against a provider that is still overloaded.
                    let jitter = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos() as u64 % 5_000)
                        .unwrap_or(0);
                    let plan = if is_retryable(&node.kind) {
                        retry_backoff(&e.to_string(), &policy, attempt, backoff, jitter)
                    } else {
                        None
                    };
                    let Some((sleep_ms, max_eff, reason)) = plan else {
                        break Err(e);
                    };
                    attempt_limit = max_eff + 1;
                    states[idx].logs.push(format!(
                        "↻ retry {}/{} in {}s ({reason})",
                        attempt + 1,
                        max_eff + 1,
                        sleep_ms / 1000
                    ));
                    let rev = repo
                        .update_run_progress(&run_id, &states)
                        .await
                        .unwrap_or(0);
                    emit_run_updated(
                        &ctx,
                        &workflow.workspace_id,
                        &run_id,
                        "running",
                        Some(&node_id),
                        rev,
                        Some(&states[idx]),
                        &states,
                        false,
                    );
                    // The failed attempt's agent must not stay alive next to the
                    // retry's fresh one (5 live PTYs per step in a 529 storm; and
                    // fd exhaustion is itself a retry class). Suspend/kill it the
                    // way a successful step's session is stopped.
                    if stop_step_sessions_wanted(&node.kind, &node.params) {
                        while let Ok(sid) = sess_rx.try_recv() {
                            record_association(&mut states[idx], sid);
                        }
                        for sid in states[idx].sessions.clone() {
                            stop_step_session(&ctx, &sid, &run_id, &mut retry_logs).await;
                        }
                    }
                    // Bail out of the backoff promptly if the run is canceled
                    // — before AND during the sleep (up to ~65s for an
                    // overloaded provider). It used to check once and then
                    // sleep blind, and to settle the step as a FAILED step: a
                    // "❌ failed" chat post and trace for a run the user
                    // stopped, and a retry clicked meanwhile raced a second
                    // attempt of this driver.
                    tokio::select! {
                        biased;
                        () = env.budget.exhausted() => {
                            timed_out = true;
                            break Err(otto_core::Error::Upstream("workflow active time budget exhausted".into()));
                        }
                        stopped = backoff_canceled(&repo, &run_id, Duration::from_millis(sleep_ms)) => {
                            if stopped {
                                canceled = true;
                                break Err(otto_core::Error::Internal("run canceled".into()));
                            }
                        }
                    }
                    backoff = ((sleep_ms as f64) * policy.factor) as u64;
                    backoff = backoff.clamp(1, 60_000);
                }
            }
        };
        // Drain any session ids reported right as the node finished.
        while let Ok(sid) = sess_rx.try_recv() {
            record_association(&mut states[idx], sid);
        }
        // Drain any trailing live-log lines — kept on EVERY path now (R5.5): the
        // phase lines ARE the record of how the step completed.
        while let Ok(line) = log_rx.try_recv() {
            states[idx].logs.push(line);
        }
        // Drain trailing activity snapshots (last one wins), then clear it:
        // `activity` is only meaningful while the node RUNS.
        while activity_rx.try_recv().is_ok() {}
        states[idx].activity = None;
        // Skipped mid-node via a chat `skip` command: stop THIS node's agents,
        // mark it Skipped, and continue to the NEXT node (unlike cancel, the run
        // proceeds). A marker output keeps dependents satisfied — an empty output
        // would make them BranchSkip and cascade the whole tail away.
        // A marker nobody consumed (the step finished first) dies with the step.
        take_skip_marker(&ctx, &run_id, &node_id);
        if skip_current {
            // A skipped review step's fleet is cancelled through the review
            // engine, like a run cancel does: killing a reviewer's PTY alone
            // makes its recovery loop respawn it, ownerless.
            for review_id in states[idx].review_ids.clone() {
                if let Ok(review) = ctx.reviews_store.get_review(&review_id).await {
                    crate::modules::cancel_running_review(&ctx, &review, &workflow.workspace_id)
                        .await;
                }
            }
            for sid in &states[idx].sessions {
                if let Err(e) = ctx.manager.kill_session(sid).await {
                    tracing::warn!("skip: failed to kill workflow session {sid}: {e}");
                }
            }
            states[idx].status = NodeStatus::Skipped;
            states[idx].logs.push("⏭ skipped via chat command".into());
            // A skipped step decided nothing, so none of its CONDITIONAL
            // out-edges is taken (those branches BranchSkip). Nothing used to be
            // pruned here — both sides of a branch after a skipped condition /
            // review ran. Unconditional edges stay live; the marker output below
            // satisfies those dependents.
            for e in outgoing_edges(&workflow.graph, &node_id) {
                if e.condition.is_some() {
                    inactive_edges.insert(e.id.clone());
                    states[idx]
                        .logs
                        .push(format!("edge → {} not taken (step skipped)", e.target));
                }
            }
            cap_node_logs(&mut states[idx].logs, NODE_LOG_CAP);
            states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
            outputs.insert(
                node_id.clone(),
                json!({ "skipped": true, "note": "skipped via chat command" }),
            );
            let rev = repo
                .update_run_progress(&run_id, &states)
                .await
                .unwrap_or(0);
            emit_run_updated(
                &ctx,
                &workflow.workspace_id,
                &run_id,
                "running",
                Some(&node_id),
                rev,
                Some(&states[idx]),
                &states,
                false,
            );
            continue;
        }
        // Canceled mid-node: bail straight to the finalize block, which marks the
        // remaining nodes Skipped, flips the run to Canceled, and kills every
        // session this run spawned (incl. this node's, harvested just above).
        if canceled || timed_out {
            break;
        }
        match result {
            Ok((out, mut ret_logs)) => {
                let failure = outcome_error(&out).map(str::to_owned);
                states[idx].status = if failure.is_some() {
                    NodeStatus::Error
                } else {
                    NodeStatus::Success
                };
                states[idx].error = failure.clone();
                if failure.is_some() {
                    errored.insert(node_id.clone());
                }
                states[idx].output = Some(out.clone());
                // R5.5: the LIVE vector (which already starts with `start_line`
                // and carries every streamed phase line) is the base — the
                // returned lines are appended, never substituted for it.
                let mut logs = std::mem::take(&mut states[idx].logs);
                logs.append(&mut ret_logs);
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
                    failure.as_deref(),
                    Some(attempt_started),
                );
                logs.append(&mut flogs);
                merge_published_refs(&files, &out);
                // R5.1: the step succeeded — stop the agent session it spawned.
                // With sub-agents allowed this leaks whole TREES of agents
                // otherwise. `review_run`'s sessions belong to the review, and
                // `keep_session: true` opts a step out.
                if stop_step_sessions_wanted(&node.kind, &node.params) {
                    for sid in states[idx].sessions.clone() {
                        stop_step_session(&ctx, &sid, &run_id, &mut logs).await;
                    }
                }
                // Prune outgoing edges whose condition fails on this output.
                let (pruned, mut plogs) =
                    eval_outgoing(&workflow.graph, node, &out, &node_input, &input);
                inactive_edges.extend(pruned);
                logs.append(&mut plogs);
                cap_node_logs(&mut logs, NODE_LOG_CAP);
                states[idx].logs = logs;
                states[idx].attempts = Some(attempt);
                // Also harvest a session id carried in the output (dedups with the
                // live channel report).
                harvest_session_ids(&out, &mut states[idx].sessions);
                let elapsed = started.elapsed().as_millis() as u64;
                states[idx].duration_ms = Some(elapsed);
                if progress.enabled() && is_reportable(&node.kind) && failure.is_none() {
                    let dur = format!("{:.1}s", elapsed as f64 / 1000.0);
                    match brief_summary(&out) {
                        Some(s) => {
                            progress.post(format!("✅ *{}* done ({dur})\n{s}", node_label(node)))
                        }
                        None => progress.post(format!("✅ *{}* done ({dur})", node_label(node))),
                    }
                }
                // Attach the step's handoff file beneath its progress line —
                // the thread carries the brief, the file the full detail.
                // review_run streams its own block but its file rides too.
                if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                    progress.post_step_file(&files, &step_file_base);
                }
                outputs.insert(node_id.clone(), out);
            }
            Err(e) => {
                states[idx].status = NodeStatus::Error;
                states[idx].error = Some(e.to_string());
                let mut elogs = std::mem::take(&mut states[idx].logs);
                elogs.append(&mut retry_logs);
                elogs.push(format!("✗ {e}"));
                // A failed step's agent is stopped like a successful one's —
                // it used to idle at its prompt until the idle sweep found it.
                if stop_step_sessions_wanted(&node.kind, &node.params) {
                    for sid in states[idx].sessions.clone() {
                        stop_step_session(&ctx, &sid, &run_id, &mut elogs).await;
                    }
                }
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
                cap_node_logs(&mut elogs, NODE_LOG_CAP);
                states[idx].logs = elogs;
                states[idx].attempts = Some(attempt);
                states[idx].duration_ms = Some(started.elapsed().as_millis() as u64);
                if progress.enabled() && is_reportable(&node.kind) {
                    progress.post(format!(
                        "❌ *{}* failed — {}",
                        node_label(node),
                        truncate(&e.to_string(), 200)
                    ));
                }
                // The failure trace file rides along too (what broke, logs).
                if progress.enabled() && (is_reportable(&node.kind) || node.kind == "review_run") {
                    progress.post_step_file(&files, &step_file_base);
                }
                errored.insert(node_id.clone());
            }
        }
        let rev = repo
            .update_run_progress(&run_id, &states)
            .await
            .unwrap_or(0);
        // Signal node finish so the inspector can update without waiting for the next poll.
        emit_run_updated(
            &ctx,
            &workflow.workspace_id,
            &run_id,
            "running",
            Some(&node_id),
            rev,
            Some(&states[idx]),
            &states,
            false,
        );
    }

    run::finalize::finish(
        ctx,
        repo,
        workflow,
        run_id,
        states,
        input,
        files,
        progress,
        progress_pump,
        canceled,
        timed_out,
    )
    .await;
}

/// Sleep a retry backoff of `total`, polling the run row every couple of
/// seconds; `true` as soon as the run is canceled (also when it already is).
async fn backoff_canceled(repo: &WorkflowsRepo, run_id: &Id, total: Duration) -> bool {
    let deadline = Instant::now() + total;
    loop {
        if repo.is_canceled(run_id).await {
            return true;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        tokio::time::sleep(left.min(Duration::from_secs(2))).await;
    }
}

/// The canceled finalize of [`run_workflow`]: cancel the run's reviews, kill
/// every session it spawned, mark unfinished steps skipped, write the
/// `canceled` terminal state (CAS — it never overwrites a run that settled
/// otherwise), report back and reap the worktrees. Shared by the in-loop
/// cancel, a cancel that landed during startup, and one that landed after the
/// last node boundary (the success/error CAS lost).
async fn stop_run_reviews(
    ctx: &ServerCtx,
    repo: &WorkflowsRepo,
    workspace_id: &Id,
    run_id: &Id,
    states: &[NodeRunState],
) {
    let mut ids: HashSet<Id> = states
        .iter()
        .flat_map(|s| s.review_ids.iter().cloned())
        .collect();
    if let Ok(checkpoints) = repo.checkpoints(run_id).await {
        for checkpoint in checkpoints {
            if checkpoint.kind == "review_run" {
                if let Some(id) = checkpoint
                    .output
                    .as_ref()
                    .and_then(|out| out.get("review_id"))
                    .and_then(Value::as_str)
                {
                    ids.insert(id.to_owned());
                }
            }
        }
    }
    for id in ids {
        if let Ok(review) = ctx.reviews_store.get_review(&id).await {
            crate::modules::cancel_running_review(ctx, &review, workspace_id).await;
        }
    }
}

async fn finalize_canceled_run(
    ctx: &ServerCtx,
    repo: &WorkflowsRepo,
    workflow: &Workflow,
    run_id: &Id,
    states: &mut [NodeRunState],
    input: &Value,
) {
    stop_run_reviews(ctx, repo, &workflow.workspace_id, run_id, states).await;
    kill_run_sessions(ctx, states).await;
    for s in states.iter_mut() {
        if matches!(s.status, NodeStatus::Pending | NodeStatus::Running) {
            s.status = NodeStatus::Skipped;
        }
    }
    // CAS on `canceled` ONLY — every caller got here because the row says
    // canceled. Accepting pending/running too let a stale driver overwrite a
    // RETRY the user started meanwhile (its fresh `pending` row got this
    // driver's old node states and a canceled status: the retry died silently).
    let rev = match write_terminal(
        repo,
        run_id,
        &[RunStatus::Canceled],
        RunStatus::Canceled,
        states,
        Some("canceled"),
    )
    .await
    {
        Ok(Some(rev)) => rev,
        Ok(None) => {
            tracing::info!(%run_id, "canceled run was re-opened by a retry — leaving it to the new driver");
            return;
        }
        Err(e) => {
            tracing::warn!(%run_id, "workflow cancel finalize write failed: {e}");
            0
        }
    };
    deliver_run_result(
        ctx,
        workflow,
        states,
        RunStatus::Canceled,
        None,
        input,
        None,
    )
    .await;
    emit_run_updated(
        ctx,
        &workflow.workspace_id,
        run_id,
        "canceled",
        None,
        rev,
        None,
        states,
        false,
    );
    clear_skip_markers(ctx, run_id);
    reap_run_worktrees(ctx, run_id).await;
}

/// Stop every agent session a run spawned — a cancel (or an orphaned run)
/// must halt the live agents (each is a real claude/codex PTY that would
/// otherwise keep working and burning tokens), not just flip the run row.
/// Includes agent steps AND review reviewers/summarizer (their ids are
/// harvested into `sessions`). Best-effort: a failure on one session is
/// logged and never blocks the rest.
async fn kill_run_sessions(ctx: &ServerCtx, states: &[NodeRunState]) {
    let session_ids: Vec<Id> = states
        .iter()
        .flat_map(|s| s.sessions.iter().cloned())
        .collect();
    for sid in session_ids {
        if let Err(e) = ctx.manager.kill_session(&sid).await {
            tracing::warn!("workflow: failed to kill run session {sid}: {e}");
        }
    }
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
    let arow = sqlx::query(
        "SELECT approved_by, approval_note, approved_at FROM workflow_runs WHERE id = ?",
    )
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
            let _ = crate::proof::upsert_content_artifact(
                ctx,
                &pack,
                K::Approval,
                &title,
                &body,
                astatus,
                meta,
                "otto",
            )
            .await;
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
            let _ = crate::proof::upsert_content_artifact(
                ctx,
                &pack,
                K::Log,
                &title,
                &content,
                art_status,
                meta,
                "otto",
            )
            .await;
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
    let ok = states
        .iter()
        .filter(|s| s.status == NodeStatus::Success)
        .count();
    let failed = states
        .iter()
        .filter(|s| s.status == NodeStatus::Error)
        .count();
    let skipped = states
        .iter()
        .filter(|s| s.status == NodeStatus::Skipped)
        .count();

    // Pull a review score out of any node output that carries one.
    let score = states.iter().find_map(|s| {
        s.output
            .as_ref()
            .and_then(|o| o.get("score"))
            .and_then(Value::as_i64)
    });

    let counts = format!("{ok}/{total} steps ok · {failed} failed · {skipped} skipped");
    let score_line = score
        .map(|sc| format!("\n*Review score:* {sc}/100"))
        .unwrap_or_default();
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
        let dur = s
            .duration_ms
            .map(|d| format!(" · {d}ms"))
            .unwrap_or_default();
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
    // Token comes from the workflow's OWN workspace integration (see
    // `result_workspace`).
    let result_ws: String = result_workspace(workflow, str_at("origin_workspace_id"));
    let (channel, chat, thread) = match str_at("result_chat") {
        Some(c) => (
            str_at("result_channel").or_else(|| str_at("channel")),
            Some(c),
            str_at("result_thread"),
        ),
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
            otto_core::redact::redact_text(&String::from_utf8_lossy(raw))
                .value
                .into_bytes(),
        ),
        None => (
            "summary.md",
            otto_core::redact::redact_text(&full).value.into_bytes(),
        ),
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
                        &ctx.secrets,
                        &integ,
                        chat,
                        thread,
                        &brief,
                    )
                    .await;
                    if sent {
                        if let Some(adapter) =
                            otto_channels::improve_notify::build_adapter(&ctx.secrets, &integ).await
                        {
                            if let Err(e) = adapter.upload(chat, thread, attach_name, &bytes).await
                            {
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
pub(crate) fn descendants_inclusive(
    graph: &WorkflowGraph,
    start: &str,
) -> std::collections::HashSet<String> {
    let mut adj: HashMap<String, Vec<String>> = HashMap::new();
    for e in &graph.edges {
        adj.entry(e.source.clone())
            .or_default()
            .push(e.target.clone());
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
fn dependency_is_relevant(
    source: &str,
    run_set: Option<&std::collections::HashSet<String>>,
    prior: Option<&[NodeRunState]>,
) -> bool {
    run_set.is_none_or(|set| set.contains(source))
        || prior.is_some_and(|nodes| {
            nodes
                .iter()
                .any(|node| node.node_id == source && node.status != NodeStatus::Pending)
        })
}

fn assemble_input(
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
    budget: budget::ActiveBudget,
    pub run_id: Id,
    /// The workflow's name — stamped into each agent session's title + meta so a
    /// session is attributable to its specific run (two concurrent runs of the
    /// same workflow otherwise spawn indistinguishable "Workflow: X" sessions).
    pub wf_name: String,
    pub run_cwd: String,
    pub run_base: Option<String>,
    pub files: std::sync::Arc<crate::workflow_context::RunContextFiles>,
    /// The workspace's effective default agent provider (per-workspace default →
    /// global default → "claude"), resolved once at run start. Agent-running
    /// nodes fall back to THIS when their `provider` param is empty — so a node
    /// left on "default" honors the user's configured default, not a bare
    /// hardcoded "claude".
    pub default_provider: String,
    /// The run's own input (what the trigger emitted / `POST /run` seeded),
    /// verbatim. Node inputs lose keys hop by hop (`assemble_input`), so a
    /// run-level override such as `review_mode` is read from HERE by the
    /// `review_run` arm, wherever that node sits in the graph.
    pub run_input: Value,
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
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    // Live per-node log lines: streamed to the run detail AS THE NODE RUNS (the
    // loop node uses this so the user sees iteration/sub-step progress instead of a
    // frozen "loop started"). Harvested next to `session_tx` in `run_workflow`.
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    // Live sub-agent / phase snapshot of a running agent step (R1.4). Harvested
    // next to `log_tx` in `run_workflow` and cleared when the node finishes.
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    progress: &ProgressSink,
) -> Result<(Value, Vec<String>)> {
    // The per-kind dispatch (and the few-line kinds) live in
    // `workflow_engine/nodes/mod.rs`; every kind takes the whole call.
    let call = nodes::NodeCall {
        ctx,
        ws,
        user,
        node,
        input,
        env,
        scope,
        session_tx,
        log_tx,
        activity_tx,
        progress,
    };
    nodes::dispatch(call).await
}

// ---------------------------------------------------------------------------
// Graph helpers
// ---------------------------------------------------------------------------

/// Edges entering `node_id` (in graph order).
fn incoming_edges<'a>(
    graph: &'a WorkflowGraph,
    node_id: &str,
) -> Vec<&'a otto_core::workflows::WorkflowEdge> {
    graph.edges.iter().filter(|e| e.target == node_id).collect()
}

/// Edges leaving `node_id` (in graph order).
fn outgoing_edges<'a>(
    graph: &'a WorkflowGraph,
    node_id: &str,
) -> Vec<&'a otto_core::workflows::WorkflowEdge> {
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
/// Completion protocol appended to EVERY agent-backed workflow step prompt.
/// R1.3: sub-agents are now ALLOWED (they are the mechanism R2 relies on) — the
/// turn oracle waits for every one of them to report back, so the rules tell the
/// agent how to finish rather than forbidding delegation. See design §1.1.
const WF_STEP_RULES: &str = "\n\n[workflow step — completion protocol]\n\
    You are one automated workflow step. You MAY delegate to sub-agents or background tasks.\n\
    - The step is finished only when EVERY sub-agent / background task you launched has reported back and you have folded its result into your work.\n\
    - Write your handoff file LAST — after all results are in. Never write it while anything you launched is still running.\n\
    - Do not end with \"waiting for …\": if you are waiting, keep waiting (the harness will wake you when a task finishes).\n\
    - Stop every background process you started (dev servers, watchers) before you finish.\n";

/// Default no-REAL-progress trip for agent-backed workflow steps (overridable
/// via the `wf_step_stall_secs` setting; 0 disables). Progress means the
/// provider's transcript/rollout growing or child processes burning CPU — not
/// PTY repaints, which a stuck TUI spinner keeps emitting forever. A tripped
/// step is retried per `resolve_retry`. Separate from — and never longer
/// than — the 10h max session lifespan backstop. See design R5.
const WF_STEP_STALL_DEFAULT: Duration = Duration::from_secs(5 * 60);

/// Stop each inner attempt before the checkpoint driver can retry it. Forward
/// associations immediately so run cancellation can still own every child.
#[allow(clippy::too_many_arguments)]
async fn execute_inner_attempt(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    input: Value,
    env: &RunEnv,
    scope: &StepScope,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    progress: &ProgressSink,
) -> Result<(Value, Vec<String>)> {
    if env.budget.expired() {
        return Err(otto_core::Error::Upstream(
            "workflow active time budget exhausted".into(),
        ));
    }
    run_owned_inner_attempt(
        ctx,
        ws,
        node,
        env,
        session_tx,
        log_tx,
        |local_tx| async move {
            execute_node(
                ctx,
                ws,
                user,
                node,
                input,
                env,
                scope,
                &local_tx,
                log_tx,
                activity_tx,
                progress,
            )
            .await
        },
    )
    .await
}

/// Own every child association until this attempt has settled and cleanup is
/// complete. The action receives the forwarding channel before it can spawn.
async fn run_owned_inner_attempt<F, Fut>(
    ctx: &ServerCtx,
    ws: &Workspace,
    node: &WorkflowNode,
    env: &RunEnv,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    action: F,
) -> Result<(Value, Vec<String>)>
where
    F: FnOnce(tokio::sync::mpsc::UnboundedSender<AgentAssociation>) -> Fut,
    Fut: std::future::Future<Output = Result<(Value, Vec<String>)>>,
{
    let (local_tx, mut local_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut sessions = Vec::new();
    let mut reviews = Vec::new();
    let result = {
        let mut forward = |association: AgentAssociation| {
            match &association {
                AgentAssociation::Session(id) => sessions.push(id.clone()),
                AgentAssociation::Review(id) => reviews.push(id.clone()),
            }
            let _ = session_tx.send(association);
        };
        let future = Box::pin(action(local_tx));
        let result = crate::workflow_node_driver::drive(future, async |mut done| {
        loop {
            tokio::select! {
                biased;
                Some(association) = local_rx.recv() => forward(association),
                result = &mut done => break result.unwrap_or_else(|_| Err(otto_core::Error::Internal("inner workflow driver closed".into()))),
            }
        }
    }).await;
        while let Ok(association) = local_rx.try_recv() {
            forward(association);
        }
        result
    };
    if result.is_err() {
        for review_id in reviews {
            if let Ok(review) = ctx.reviews_store.get_review(&review_id).await {
                crate::modules::cancel_running_review(ctx, &review, &ws.id).await;
            }
        }
    }
    let mut cleanup = Vec::new();
    if stop_step_sessions_wanted(&node.kind, &node.params) {
        for id in sessions {
            stop_step_session(ctx, &id, &env.run_id, &mut cleanup).await;
        }
    }
    for line in &cleanup {
        let _ = log_tx.send(line.clone());
    }
    result.map(|(output, mut logs)| {
        logs.extend(cleanup);
        (output, logs)
    })
}

fn review_coverage_complete(
    review: &otto_core::domain::Review,
    allow_summary_fallback: bool,
) -> bool {
    otto_review::engine::review_run_complete(
        &review.agents,
        review.summary_fallback.unwrap_or(false) && !allow_summary_fallback,
    )
}

fn loop_checkpoint(
    node: &WorkflowNode,
    loop_id: &str,
    iteration: u64,
    step_index: usize,
    input: Value,
) -> WorkflowCheckpoint {
    WorkflowCheckpoint {
        node_id: node.id.clone(),
        loop_id: loop_id.into(),
        iteration,
        step_index,
        kind: node.kind.clone(),
        name: node_display_name(node).into(),
        status: NodeStatus::Pending,
        attempts: 0,
        input,
        output: None,
        error: None,
        logs: vec![],
        updated_at: chrono::Utc::now(),
    }
}

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
        && node
            .params
            .get("prompt")
            .and_then(Value::as_str)
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
    if node.kind == "agent_prompt" || has_agent_phase {
        return otto_core::workflows::RetryPolicy {
            max_attempts: 2,
            backoff_ms: 2000,
            factor: 2.0,
        }
        .clamped();
    }
    otto_core::workflows::RetryPolicy::default()
}

/// Whether a node kind should be retried on failure. Interactive / entry kinds
/// are never retried.
fn is_retryable(kind: &str) -> bool {
    !matches!(kind, "human_approval" | "manual_trigger")
}

// Retry classification + backoff live in `otto_workflows::retry` (imported above).

/// R5.1: whether the engine stops the sessions a SUCCESSFUL step spawned.
/// `review_run`'s sessions belong to the review engine (it tears them down
/// itself), and `params.keep_session: true` opts a step out so an operator can
/// keep poking at the agent afterwards.
fn stop_step_sessions_wanted(kind: &str, params: &Value) -> bool {
    kind != "review_run"
        && !params
            .get("keep_session")
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

/// Suspend a resumable provider's session, kill the rest. Suspending leaves the
/// row `Reconnectable`, so "Open session" on a finished step still resumes the
/// transcript; killing is the only option for a provider that cannot resume.
fn stop_action(supports_resume: bool) -> &'static str {
    if supports_resume {
        "suspend"
    } else {
        "kill"
    }
}

/// Stop ONE session a successful step spawned (R5.1). Best-effort and never an
/// error: a session the manager has no row for — the E2E stub's synthetic id, or
/// one the idle sweep already archived — is skipped silently, and a
/// suspend/kill failure is a `⚠` log line on the step.
async fn stop_step_session(ctx: &ServerCtx, sid: &Id, run_id: &Id, logs: &mut Vec<String>) {
    let s = match ctx.manager.get(sid).await {
        Ok(s) => s,
        Err(otto_core::Error::NotFound(_)) => return,
        Err(e) => {
            logs.push(format!("⚠ could not stop session {sid}: {e}"));
            return;
        }
    };
    // Only OUR run's workflow sessions — never an interactive one that happens
    // to be listed on the node.
    if s.meta.get("source").and_then(Value::as_str) != Some("workflow")
        || s.meta.get("run_id").and_then(Value::as_str) != Some(run_id.as_str())
    {
        return;
    }
    // Already parked (by the idle sweep, or a step that outlived it): suspending
    // again succeeds and writes a SECOND "Suspended" row into the user's session
    // history. Same skip as `review_session::stop_review_sessions`.
    if matches!(
        s.status,
        otto_core::domain::SessionStatus::Exited | otto_core::domain::SessionStatus::Reconnectable
    ) {
        return;
    }
    let action = stop_action(ctx.manager.providers().supports_resume(&s.provider));
    let res = if action == "suspend" {
        ctx.manager.suspend(sid).await
    } else {
        ctx.manager.kill_session(sid).await
    };
    if let Err(e) = res {
        logs.push(format!("⚠ could not stop session {sid}: {e}"));
    }
}

/// R2: the review mode a `review_run` step was TOLD to use, with the source for
/// its log line — the run's own input first (the run dialog's override, which
/// must beat a node that sets its own), then the node's `params.mode`. `None`
/// when neither names a valid mode; the caller then falls back to the repo's
/// stored config. Unparseable values are treated as absent (never an error).
///
/// Note the signature: there is no node-INPUT parameter. Node inputs lose
/// run-level keys hop by hop, so a `review_mode` riding one is ignored by
/// construction.
pub(crate) fn resolve_review_mode_source(
    run_input: &Value,
    params: &Value,
) -> Option<(otto_core::domain::ReviewMode, &'static str)> {
    if let Some(m) = run_input
        .get("review_mode")
        .and_then(Value::as_str)
        .and_then(otto_core::domain::ReviewMode::parse)
    {
        return Some((m, "run override"));
    }
    params
        .get("mode")
        .and_then(Value::as_str)
        .and_then(otto_core::domain::ReviewMode::parse)
        .map(|m| (m, "node"))
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
/// Last 6 chars of a run id — a compact, human-scannable tag that disambiguates
/// concurrent runs of the same workflow in session titles / lists.
fn short_id(id: &Id) -> String {
    let n = id.chars().count();
    id.chars().skip(n.saturating_sub(6)).collect()
}

#[allow(clippy::too_many_arguments)]
async fn run_node_agent(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    wf_name: &str,
    run_id: &Id,
    provider: &str,
    // Optional model alias (e.g. "opus"/"haiku"). Empty/None = provider default.
    // Forwarded via `meta.model` so `SessionManager::model_args` adds `--model`.
    model: Option<&str>,
    prompt: &str,
    cwd: &str,
    // The step's handoff `.md` (named by the preamble) as a completion MARKER:
    // for providers without a pollable transcript (codex/agy) it is the ONLY
    // positive "the step finished" signal — without it they sit at their input
    // box until the stall trip errors the step. `None` for steps whose reply
    // must come from the transcript (canvas/goals/PR-draft agents).
    done_file: Option<std::path::PathBuf>,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    // The node's LIVE log channel: the oracle's phase lines stream here as the
    // turn runs and are kept in the step's logs afterwards (R1.4/R5.5).
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    // Live sub-agent snapshot for the run view's chip + nested rows.
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
) -> Result<(String, Id)> {
    // Title carries the workflow name, the step, and a short run id so two
    // concurrent runs of the same workflow are distinguishable in the Agents
    // list (they'd otherwise both read "Workflow: Write tests").
    let label = if node.name.is_empty() {
        node.kind.as_str()
    } else {
        node.name.as_str()
    };
    let title = format!("WF {wf_name} · {label} · run {}", short_id(run_id));
    // Stamp the FULL run id (+ workflow name) into meta so every workflow-spawned
    // session is provably attributable to its specific run — the UI/global Agents
    // list can filter + group sessions by exact run.
    let mut meta = json!({
        "source": "workflow",
        "run_id": run_id,
        "wf_name": wf_name,
        "node_id": node.id,
        "node_kind": node.kind,
        "cwd": cwd,
    });
    // Forward the model alias so the spawn adds `--model` (providers without the
    // flag ignore it). Empty string = provider default → omitted.
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        meta["model"] = json!(m);
    }
    // R5.5: the step's phases are visible from the first line. The engine logs
    // the boot line itself (the oracle's `Phase::Booting` is therefore never
    // re-logged by the forwarder below).
    let _ = log_tx.send(format!("⏳ starting {provider} session"));
    // E2E: the offline daemon never spawns a CLI, so a spec that needs a step
    // with sub-agents drives a synthetic sequence off a prompt sentinel.
    if otto_e2e() {
        if let Some((n, hold)) = e2e_subagents(prompt) {
            e2e_subagent_sequence(n, hold, log_tx, activity_tx).await;
        }
    }
    let tx = session_tx.clone();
    // The session id the turn creates — the phase forwarder needs it to ask the
    // oracle for this step's sub-agent rows (it only exists once `on_ready`
    // fires, a few seconds in).
    let live_sid: std::sync::Arc<std::sync::Mutex<Option<Id>>> = Default::default();
    let sid_slot = live_sid.clone();
    // R1.3: sub-agents ARE allowed; the appended block is a completion protocol
    // (finish every task you launched, handoff LAST) and the turn oracle holds
    // the step open until the parent's pending set is empty.
    let guarded = format!("{prompt}{WF_STEP_RULES}");
    // R5: EVERY agent-backed step gets the no-real-progress trip (retryable via
    // resolve_retry). Heavy kinds are safe under a short threshold because the
    // clock only counts genuine stalls — transcript/rollout growth and child
    // CPU reset it (see agent_session), so a long review or quiet test run
    // never false-trips while a codex spinner can no longer mask a hang.
    let stall = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("wf_step_stall_secs")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .map_or(WF_STEP_STALL_DEFAULT, Duration::from_secs);
    let stuck_after = if stall.is_zero() {
        crate::agent_session::STUCK_IDLE
    } else {
        stall
    };
    // A leftover handoff from a PRIOR attempt/run of this step would complete
    // the new turn instantly with stale content — clear it before submitting.
    if let Some(df) = &done_file {
        let _ = std::fs::remove_file(df);
    }
    // Steps whose reply must come from the transcript (goals eval, PR draft,
    // canvas) never name a handoff file — the oracle completes them on the turn
    // ending with nothing pending, and the "handoff missing" ⚠ line would be a
    // lie for them.
    let expects_handoff = done_file.is_some();
    // R1: the turn ORACLE decides completion for workflow steps — a claude
    // `end_turn` while sub-agents are still working is not "done". Phases stream
    // back on `phase_tx`; the accepted rule rides `outcome_tx`.
    let (phase_tx, mut phase_rx) = tokio::sync::mpsc::unbounded_channel::<Phase>();
    let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<CompleteVia>();
    let turn = crate::agent_session::run_session_turn_with(
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
        crate::agent_session::TurnOpts {
            done_file,
            done_file_validator: None,
            transcript_fallback: false,
            cancel: None,
            // Silence is not evidence that delegated work has completed.
            quiet_done: None,
            // The retry spawns a fresh session; don't leave the stuck one
            // alive (its spinner defeats the idle-suspend sweep).
            kill_on_stall: true,
            oracle: true,
            phase_tx: Some(phase_tx),
            outcome_tx: Some(outcome_tx),
        },
        move |id| {
            let _ = tx.send(AgentAssociation::Session(id.to_string()));
            if let Ok(mut slot) = sid_slot.lock() {
                *slot = Some(id.clone());
            }
        },
    );
    tokio::pin!(turn);
    // Phase → log/activity forwarder. Runs BESIDE the turn (not spawned) so it
    // can read the live session without cloning the whole context.
    let mut feed = PhaseFeed::new(provider);
    let result = loop {
        tokio::select! {
            biased;
            Some(p) = phase_rx.recv() => {
                let sid = live_sid.lock().ok().and_then(|g| g.clone());
                feed.on_phase(ctx, sid.as_ref(), p, log_tx, activity_tx).await;
            }
            r = &mut turn => break r,
        }
    };
    while let Ok(p) = phase_rx.try_recv() {
        let sid = live_sid.lock().ok().and_then(|g| g.clone());
        feed.on_phase(ctx, sid.as_ref(), p, log_tx, activity_tx)
            .await;
    }
    let out = result.map_err(|e| e.0)?;
    // How the oracle accepted the turn — `Err` means the sender was dropped
    // without a send (E2E short-circuit / legacy path): nothing to report.
    if let Ok(via) = outcome_rx.await {
        if expects_handoff || via != CompleteVia::IdleTurnNoHandoff {
            let _ = log_tx.send(feed.completion_line(via));
        }
    }
    Ok(out)
}

/// Turns the oracle's phase stream into the node's kept log lines and the live
/// `activity` snapshot.
///
/// Emission rule (R5.5): a line is logged only when the phase IDENTITY changes
/// (`phase_key` ignores a countdown's remaining time), and an identity re-logs
/// at most once per 5 s — so `⚙ working` appears once, the countdown phases
/// once on entry, and `🧩 sub-agents: …` on every count change.
struct PhaseFeed {
    provider: String,
    /// Seeded with `Booting`: `run_node_agent` already logged its `⏳` line.
    last_key: Option<(u8, usize, usize)>,
    logged_at: HashMap<(u8, usize, usize), Instant>,
    /// `📄 handoff file written` is logged once, the first time a phase implies
    /// the file exists.
    handoff_logged: bool,
    last_probe: Option<Instant>,
    subagents: Vec<SubagentActivity>,
    last_progress_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Remembered for the completion line's `{n}` / `{id}`.
    pending: usize,
    running_id: Option<String>,
    /// Incremental reader behind the sub-agent snapshot (see [`StepProbe`]).
    probe: Option<StepProbe>,
}

impl PhaseFeed {
    fn new(provider: &str) -> Self {
        Self {
            provider: provider.to_string(),
            last_key: Some(turn_oracle::phase_key(&Phase::Booting)),
            logged_at: HashMap::new(),
            handoff_logged: false,
            last_probe: None,
            subagents: vec![],
            last_progress_at: None,
            pending: 0,
            running_id: None,
            probe: None,
        }
    }

    async fn on_phase(
        &mut self,
        ctx: &ServerCtx,
        sid: Option<&Id>,
        p: Phase,
        log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
        activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
    ) {
        let key = turn_oracle::phase_key(&p);
        if self.last_key == Some(key) {
            return;
        }
        self.last_key = Some(key);
        // Refresh the sub-agent snapshot (one transcript read, ≤ once per 5s).
        // A `Subagents` phase gets the read unconditionally: its identity only
        // changes when a count changes (≤ 2 × pending ids over a step), and the
        // snapshot it is annotated with — and the rows the run view shows —
        // would otherwise describe the PREVIOUS count.
        if matches!(p, Phase::Subagents { .. })
            || self
                .last_probe
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(5))
        {
            self.last_probe = Some(Instant::now());
            if let Some((subs, stamp)) = step_activity_probe(ctx, sid, &mut self.probe).await {
                self.running_id = subs
                    .iter()
                    .find(|s| s.status == turn_oracle::SubStatus::Running)
                    .map(|s| s.id.clone());
                self.subagents = subs.iter().map(to_subagent_activity).collect();
                self.last_progress_at = stamp.map(chrono::DateTime::<chrono::Utc>::from);
            }
        }
        let mut line = turn_oracle::phase_line(&p, &self.provider);
        // Name the description that just completed, when we know it.
        if let Phase::Subagents { done, .. } = &p {
            if *done > 0 {
                if let Some(d) = self
                    .subagents
                    .iter()
                    .rfind(|s| s.status == "done")
                    .map(|s| s.description.clone())
                {
                    line.push_str(&format!(" ({d} ✓)"));
                }
            }
        }
        // `hold_reason` is for a step the engine is deliberately HOLDING while
        // it looks idle. A handoff written with nothing pending is just the turn
        // finishing, so it is not one of them.
        let holds = matches!(
            p,
            Phase::IdleConfirming { .. }
                | Phase::HandoffMissingGrace { .. }
                | Phase::HandoffWrittenWaiting { pending: 1.. }
                | Phase::BashLinger { .. }
        );
        self.pending = match &p {
            Phase::HandoffWrittenWaiting { pending } | Phase::BashLinger { pending, .. } => {
                *pending
            }
            Phase::Subagents { running, .. } => *running,
            _ => 0,
        };
        // The handoff file is on disk the moment either of these phases holds.
        if !self.handoff_logged
            && matches!(
                p,
                Phase::IdleConfirming { .. } | Phase::HandoffWrittenWaiting { .. }
            )
        {
            self.handoff_logged = true;
            let _ = log_tx.send("📄 handoff file written".to_string());
        }
        // 5s floor per identity: A→B→A inside the window logs A once.
        let recent = self
            .logged_at
            .get(&key)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(5));
        if !recent {
            self.logged_at.insert(key, Instant::now());
            let _ = log_tx.send(line.clone());
        }
        let _ = activity_tx.send(bound_activity(NodeActivity {
            phase: phase_text(&line),
            updated_at: chrono::Utc::now(),
            last_progress_at: self.last_progress_at,
            pending_tasks: self.pending as u32,
            subagents: self.subagents.clone(),
            hold_reason: holds.then(|| line.clone()),
        }));
    }

    /// The `✓`/`⚠` line that closes an accepted step (design §4, verbatim).
    fn completion_line(&self, via: CompleteVia) -> String {
        match via {
            CompleteVia::HandoffAndIdleTurn => "✓ step complete (handoff + idle turn)".into(),
            CompleteVia::CodexTaskComplete => {
                "✓ step complete (codex task_complete + handoff)".into()
            }
            CompleteVia::IdleTurnNoHandoff => {
                "⚠ handoff file missing — accepted the agent's final reply after 90s idle".into()
            }
            CompleteVia::HandoffLingerCap => format!(
                "⚠ handoff written; {} sub-agent{} never reported after 15m — moving on",
                self.pending,
                if self.pending == 1 { "" } else { "s" }
            ),
            CompleteVia::BashLingerCap => format!(
                "⚠ background task {} never finished after 15m — moving on",
                self.running_id.as_deref().unwrap_or("(unknown)")
            ),
            CompleteVia::QuietFallback => {
                "⚠ legacy completion record: silence fallback (no longer accepted)".into()
            }
        }
    }
}

/// A phase log line without its glyph — what `NodeActivity.phase` carries.
fn phase_text(line: &str) -> String {
    line.trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim()
        .to_string()
}

fn to_subagent_activity(s: &turn_oracle::SubagentInfo) -> SubagentActivity {
    SubagentActivity {
        id: s.id.chars().take(64).collect(),
        description: s.description.chars().take(80).collect(),
        status: match s.status {
            turn_oracle::SubStatus::Running => "running",
            turn_oracle::SubStatus::Done => "done",
            turn_oracle::SubStatus::Failed => "failed",
        }
        .to_string(),
        started_at: s.started_at.map(chrono::DateTime::<chrono::Utc>::from),
        finished_at: s.finished_at.map(chrono::DateTime::<chrono::Utc>::from),
    }
}

/// Clamp an activity snapshot to the documented bounds (§2): ≤ 40 sub-agents,
/// ids ≤ 64 chars, descriptions ≤ 80 — so even a 40-agent sweep adds under
/// 12 KiB to `nodes_json` and the WS frame, well inside
/// `NODE_EVENT_MAX_BYTES`.
fn bound_activity(mut a: NodeActivity) -> NodeActivity {
    a.subagents.truncate(40);
    for s in &mut a.subagents {
        if s.id.chars().count() > 64 {
            s.id = s.id.chars().take(64).collect();
        }
        if s.description.chars().count() > 80 {
            s.description = s.description.chars().take(80).collect();
        }
    }
    a.phase = a.phase.chars().take(200).collect();
    a.hold_reason = a.hold_reason.map(|r| r.chars().take(200).collect());
    a
}

/// The running step's sub-agents + newest child-file mtime, straight from the
/// oracle. `None` until the session exists (or for a provider with no
/// transcript) — the snapshot then simply carries no rows.
///
/// Each probe used to re-read and re-parse the WHOLE step transcript on the
/// runtime (once per sub-agent start/finish — ~40 × 40 ms on a 30 MB step). The
/// [`StepProbe`] kept in `slot` reads only what was appended since the last
/// probe and re-reads only new/changed sidecars, off the runtime.
async fn step_activity_probe(
    ctx: &ServerCtx,
    sid: Option<&Id>,
    slot: &mut Option<StepProbe>,
) -> Option<(
    Vec<turn_oracle::SubagentInfo>,
    Option<std::time::SystemTime>,
)> {
    let sid = sid?;
    let s = ctx.manager.get(sid).await.ok()?;
    if s.provider != "claude" {
        return None;
    }
    let psid = s.provider_session_id.clone()?;
    let mut probe = match slot.take() {
        Some(p) if &p.sid == sid && p.psid == psid => p,
        _ => {
            let (sid, cwd) = (sid.clone(), s.cwd.clone());
            crate::offload::blocking(move || StepProbe::new(sid, &cwd, psid)).await
        }
    };
    let (probe, out) = crate::offload::blocking(move || {
        let out = probe.run();
        (probe, out)
    })
    .await;
    *slot = Some(probe);
    out
}

/// Per-step incremental state for [`step_activity_probe`].
struct StepProbe {
    sid: Id,
    psid: String,
    proj: std::path::PathBuf,
    main: std::path::PathBuf,
    tail: turn_oracle::ClaudeTail,
    metas: turn_oracle::SubagentMetaCache,
}

impl StepProbe {
    /// Resolve the transcript paths. Blocking (canonicalize).
    fn new(sid: Id, cwd: &str, psid: String) -> Self {
        // claude symlink-resolves the spawn cwd for its transcript dir.
        let cwd = std::fs::canonicalize(cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| cwd.to_string());
        let proj = otto_orchestrator::claude_pty::project_dir(&cwd);
        let main = proj.join(format!("{psid}.jsonl"));
        Self {
            sid,
            tail: turn_oracle::ClaudeTail::new(&main),
            psid,
            proj,
            main,
            metas: turn_oracle::SubagentMetaCache::default(),
        }
    }

    /// One probe. Blocking IO (only the appended bytes + changed sidecars).
    fn run(
        &mut self,
    ) -> Option<(
        Vec<turn_oracle::SubagentInfo>,
        Option<std::time::SystemTime>,
    )> {
        let scan = self.tail.poll()?;
        let subdir = self.proj.join(&self.psid).join("subagents");
        let stamp = turn_oracle::progress_stamp(&self.main, Some(&subdir), None);
        let subs = turn_oracle::subagents_cached(&self.proj, &self.psid, &scan, &mut self.metas);
        Some((subs, stamp))
    }
}

/// Parse the E2E sub-agent sentinel from a step prompt:
/// `OTTO_E2E_SUBAGENTS: <n>` (1..=40) with an optional ` hold_ms=<ms>`
/// (1000..=60000) on the same line; without it the hold is
/// `OTTO_E2E_STEP_HOLD_MS` (default 4000 ms).
fn e2e_subagents(prompt: &str) -> Option<(usize, Duration)> {
    let line = prompt
        .lines()
        .find(|l| l.trim_start().starts_with("OTTO_E2E_SUBAGENTS:"))?;
    let rest = line.trim_start().strip_prefix("OTTO_E2E_SUBAGENTS:")?;
    let n: usize = rest.split_whitespace().next()?.parse().ok()?;
    if !(1..=40).contains(&n) {
        return None;
    }
    let hold = rest
        .split_whitespace()
        .find_map(|t| t.strip_prefix("hold_ms="))
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| (1000..=60_000).contains(ms))
        .or_else(|| {
            std::env::var("OTTO_E2E_STEP_HOLD_MS")
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
        })
        .unwrap_or(4000);
    Some((n, Duration::from_millis(hold)))
}

/// The synthetic phase/activity sequence the E2E daemon emits for a step whose
/// prompt carries the sub-agent sentinel — the real oracle never runs there (no
/// CLI is spawned). Mirrors a real run's line order; the canned reply's own
/// `agent turn complete` follows once `run_session_turn_with` returns.
async fn e2e_subagent_sequence(
    n: usize,
    hold: Duration,
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
) {
    let half = hold / 2;
    let _ = log_tx.send("✉ prompt accepted".to_string());
    let started = chrono::Utc::now();
    let mut subs: Vec<SubagentActivity> = (1..=n)
        .map(|i| SubagentActivity {
            id: format!("e2e-agent-{i}"),
            description: format!("E2E sub-agent {i}"),
            status: "running".into(),
            started_at: Some(started),
            finished_at: None,
        })
        .collect();
    let _ = log_tx.send(format!("🧩 sub-agents: {n} running · 0 done"));
    let _ = activity_tx.send(bound_activity(NodeActivity {
        phase: format!("sub-agents: {n} running · 0 done"),
        updated_at: chrono::Utc::now(),
        last_progress_at: Some(started),
        pending_tasks: n as u32,
        subagents: subs.clone(),
        hold_reason: None,
    }));
    tokio::time::sleep(half).await;
    let finished = chrono::Utc::now();
    for s in &mut subs {
        s.status = "done".into();
        s.finished_at = Some(finished);
    }
    let _ = log_tx.send(format!("🧩 sub-agents: 0 running · {n} done"));
    let _ = activity_tx.send(bound_activity(NodeActivity {
        phase: format!("sub-agents: 0 running · {n} done"),
        updated_at: finished,
        last_progress_at: Some(finished),
        pending_tasks: 0,
        subagents: subs,
        hold_reason: None,
    }));
    tokio::time::sleep(hold.saturating_sub(half)).await;
    let _ = log_tx.send("📄 handoff file written".to_string());
    let _ = log_tx.send("✓ step complete (handoff + idle turn)".to_string());
}

/// Resolve the repo_id a repo-needing step (`review_run`, `git_pr`) should use:
/// an explicit `repo_id` (step params → run input) wins; otherwise derive it from
/// the step's `worktree_path`, the run's `working_directory`, or `run_cwd`.
/// See design §B — this is what makes such steps resilient instead of failing
/// with a bare "missing repo_id".
///
/// An explicit `repo_id` must be a repo registered in Otto (S3-302) — in any
/// workspace, so one workflow can review a repo another workspace registered.
async fn resolve_step_repo_id(
    ctx: &ServerCtx,
    ws: &Workspace,
    p: &Value,
    input: &Value,
    run_cwd: &str,
) -> otto_core::Result<Option<String>> {
    let explicit = p
        .get("repo_id")
        .and_then(Value::as_str)
        .or_else(|| input.get("repo_id").and_then(Value::as_str))
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty());
    if let Some(id) = explicit {
        ensure_repo_registered(ctx, &id).await?;
        return Ok(Some(id));
    }
    let hint = p
        .get("worktree_path")
        .and_then(Value::as_str)
        .or_else(|| input.get("working_directory").and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| run_cwd.to_string());
    Ok(resolve_repo_id_for_path(ctx, &ws.id, &hint).await)
}

/// `repo_id` names a repo registered in Otto (any workspace) — else
/// `Invalid`. S3-302.
pub(crate) async fn ensure_repo_registered(
    ctx: &ServerCtx,
    repo_id: &str,
) -> otto_core::Result<()> {
    match ctx.git_store.get_repo(&repo_id.to_string()).await {
        Ok(_) => Ok(()),
        Err(_) => Err(otto_core::Error::Invalid(format!(
            "repo '{repo_id}' is not a registered repo"
        ))),
    }
}

/// S3-302: where a run may work. Its `working_directory` (comma-separated
/// paths), `repos` declarations (repo, worktree path, `repo_id`) and an
/// explicit `repo_id` must all name a repo registered in Otto — in ANY
/// workspace, so a shared workflow (e.g. a PR reviewer) can work on a repo
/// another workspace registered — or a linked worktree of one, or the
/// workflow's workspace root, and paths inside them. Arbitrary unregistered
/// folders stay refused. Checked where a run starts from caller input (the
/// manual-run route, chat); the error names the offending value.
///
/// Chat, webhook, and event starts use this strict form. A root user's manual
/// run passes [`ensure_run_location_as`] with the host-folder exception.
pub(crate) async fn ensure_run_location(
    ctx: &ServerCtx,
    ws: &Workspace,
    input: &Value,
) -> otto_core::Result<()> {
    ensure_run_location_as(ctx, ws, input, false).await
}

/// [`ensure_run_location`], plus an existing absolute host folder when
/// `allow_existing_host_folder` is set. Only the manual route sets that, and
/// only for a person's credential whose user row has `is_root`. The workflow
/// owner does not count: an external trigger stays on [`ensure_run_location`].
pub(crate) async fn ensure_run_location_as(
    ctx: &ServerCtx,
    ws: &Workspace,
    input: &Value,
    allow_existing_host_folder: bool,
) -> otto_core::Result<()> {
    let repos = ctx.git_store.list_all_repos().await?;
    let pairs: Vec<(String, String)> = repos
        .iter()
        .map(|r| (r.id.clone(), r.path.clone()))
        .collect();
    let text = |k: &str, v: &Value| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Some(rid) = text("repo_id", input) {
        ensure_repo_registered(ctx, &rid).await?;
    }
    if let Some(wd) = text("working_directory", input) {
        for path in wd.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            ensure_path_in_workspace(&ws.root_path, &pairs, path, allow_existing_host_folder)
                .await?;
        }
    }
    for e in crate::workflow_context::parse_repo_entries(input.get("repos").unwrap_or(&Value::Null))
    {
        let registered = repos.iter().any(|r| r.id == e.repo || r.name == e.repo);
        if !registered {
            // A path names a folder; anything else (`org/name` included) is a
            // repo name or id, and an unknown one says so — not "must be an
            // absolute path".
            if looks_like_path(&e.repo) {
                ensure_path_in_workspace(
                    &ws.root_path,
                    &pairs,
                    &e.repo,
                    allow_existing_host_folder,
                )
                .await?;
            } else {
                return Err(otto_core::Error::Invalid(format!(
                    "repo '{}' is not a registered repo — name a registered repo by its \
                     name or id, or give its absolute path",
                    e.repo
                )));
            }
        }
        if let Some(rid) = &e.repo_id {
            ensure_repo_registered(ctx, rid).await?;
        }
        if e.kind == "worktree" {
            ensure_path_in_workspace(&ws.root_path, &pairs, &e.name, allow_existing_host_folder)
                .await?;
        }
        if let Some(wt) = &e.worktree {
            ensure_path_in_workspace(&ws.root_path, &pairs, wt, allow_existing_host_folder).await?;
        }
    }
    Ok(())
}

/// The run note for a repo whose in-worktree hooks Otto's own git skipped.
fn skipped_hooks_note(repo: &str) -> String {
    format!(
        "⚠ git_pr: {repo}'s hooks live inside its work tree (core.hooksPath, e.g. husky) \
         and did not run for Otto's own commit / push — an agent there can edit them, so \
         unattended runs skip them; CI or your next commit still runs them"
    )
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
            v.get(*k)
                .and_then(Value::as_str)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
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
fn checks_review_agent(
    default_provider: &str,
    checks: &[(String, String)],
) -> otto_core::domain::ReviewAgentCfg {
    let list = checks
        .iter()
        .map(|(n, c)| {
            if n == c {
                format!("- {c}")
            } else {
                format!("- {n}: {c}")
            }
        })
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
        name: crate::modules::CHECKS_REVIEWER_NAME.to_string(),
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
fn seed_input_from_entries(input: Value, entries: &[crate::workflow_context::RepoEntry]) -> Value {
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
        Value::Array(
            valid
                .iter()
                .map(|e| crate::workflow_context::entry_to_target(e))
                .collect(),
        ),
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
    let Value::Object(mut m) = input else {
        return input;
    };
    let has_prompt = m
        .get("prompt")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if !has_prompt {
        if let Some(msg) = m
            .get("msg")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
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
/// Cut a dedicated linked worktree from each entry's `base` branch when the entry
/// would otherwise run in the repo's MAIN checkout (the user's live working copy),
/// and repoint the entry at it. Returns the FIRST provisioned worktree path (the
/// run's new working directory), or `None` if nothing was provisioned.
///
/// SAFETY (load-bearing): this only ever CREATES a new linked worktree + a fresh
/// `otto-wf/<run_id>` branch under the data dir. It never checks out, resets, or
/// switches branches in the user's own repo, and never deletes. `rev-parse`
/// resolves the base — the local branch, else its remote-tracking
/// `origin/<base>` (a repo that only has `origin/develop` used to fall through
/// here), else a single-branch fetch into `refs/remotes/origin/<base>` (a base
/// created on the remote after the clone last fetched). No-op when no `base`
/// is set.
///
/// Returns `(primary, isolation_failures)`. A failure is an entry that WOULD
/// run in the user's own checkout because its base could not be resolved or
/// its worktree could not be created: the caller fails the run with those
/// messages rather than silently letting agents edit/commit in the user's
/// working copy (seen for `koala-zenith-go`: base `develop` only on origin).
async fn provision_wf_worktrees(
    ctx: &ServerCtx,
    run_id: &Id,
    entries: &mut [crate::workflow_context::RepoEntry],
) -> (Option<String>, Vec<String>) {
    let mut primary: Option<String> = None;
    let mut failures: Vec<String> = Vec::new();
    let canon = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| std::path::PathBuf::from(p));
    for e in entries.iter_mut() {
        if e.error.is_some() {
            continue;
        }
        let (Some(repo_id), Some(base), Some(worktree)) =
            (e.repo_id.clone(), e.base.clone(), e.worktree.clone())
        else {
            continue;
        };
        let base = base.trim().to_string();
        if base.is_empty() {
            continue;
        }
        let Ok(repo) = ctx.git_store.get_repo(&repo_id).await else {
            continue;
        };
        // Only provision when we'd otherwise operate on the repo's MAIN checkout;
        // an entry already pointing at a dedicated worktree is left as-is.
        if canon(&worktree) != canon(&repo.path) {
            continue;
        }
        let git = otto_git::LocalGit::new(&repo.path);
        // READ-ONLY: resolve the base branch to a commit in the user's repo.
        let Some(base_commit) = resolve_wf_base(&git, &base).await else {
            tracing::warn!(
                "wf worktree: base '{base}' not found in {} (locally or as origin/{base}) — refusing to run in the user's checkout",
                repo.path
            );
            failures.push(format!(
                "base branch '{base}' was not found in {} (neither locally nor as origin/{base}) — fetch it or fix the run's base",
                repo.name
            ));
            continue;
        };
        let wt_path = ctx
            .data_dir
            .join("workflow-runs")
            .join(run_id)
            .join(crate::workflow_context::slug(&repo.name));
        let wt_str = wt_path.to_string_lossy().to_string();
        let branch = format!("otto-wf/{run_id}");
        match git
            .worktree_add_if_absent(&wt_str, &branch, &base_commit)
            .await
        {
            Ok(_) => {
                tracing::info!("wf worktree: {} @ {base} → {wt_str}", repo.name);
                e.worktree = Some(wt_str.clone());
                if primary.is_none() {
                    primary = Some(wt_str);
                }
            }
            Err(err) => {
                tracing::warn!(
                    "wf worktree provision failed for {} @ {base}: {err} — refusing to run in the user's checkout",
                    repo.name
                );
                failures.push(format!(
                    "could not create an isolated worktree of {} @ {base}: {err}",
                    repo.name
                ));
            }
        }
    }
    (primary, failures)
}

/// Resolve a run's declared base to a commit: the ref as given (a local
/// branch, tag or sha), else the remote-tracking `origin/<base>`. Read-only.
async fn resolve_wf_base(git: &otto_git::LocalGit, base: &str) -> Option<String> {
    if let Ok(c) = git.rev_parse(base).await {
        return Some(c);
    }
    let remote = format!("origin/{base}");
    if let Ok(c) = git.rev_parse(&remote).await {
        return Some(c);
    }
    // Neither ref exists yet — typically a branch created on the remote after
    // the clone last fetched (a PR targeting a fresh `hotfix/*`). Fetch THAT
    // ONE branch into its remote-tracking ref and retry. Safe: it writes only
    // `refs/remotes/origin/<base>` — never the user's branches, HEAD or tree —
    // and runs non-interactively (BatchMode SSH), so it can't prompt.
    if !fetchable_branch_name(base) {
        return None;
    }
    let refspec = format!("+refs/heads/{base}:refs/remotes/origin/{base}");
    match git
        .run_automation(&["fetch", "--no-tags", "origin", &refspec], true)
        .await
    {
        Ok(_) => tracing::info!("wf worktree: fetched missing base '{base}' from origin"),
        Err(err) => {
            tracing::warn!("wf worktree: fetching base '{base}' from origin failed: {err}");
            return None;
        }
    }
    git.rev_parse(&remote).await.ok()
}

/// A base name that is safe to splice into a fetch refspec: no option-like
/// prefix, no refspec/glob metacharacters, no whitespace or control chars.
/// `git` itself still validates the full ref name.
fn fetchable_branch_name(base: &str) -> bool {
    !base.is_empty()
        && !base.starts_with('-')
        && !base.starts_with('/')
        && !base.contains("..")
        && !base
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || ":^~?*[\\+".contains(c))
}

/// Reap the worktrees a run provisioned under
/// `<data_dir>/workflow-runs/<run_id>/` and, when safe, the matching
/// `otto-wf/<run_id>` branch in each owning repo. Without this, every
/// scheduled/chat run left a permanent linked worktree + branch in the user's
/// real repository (24/day for an hourly workflow).
///
/// Branch policy — never discard commits: the branch is deleted only when its
/// commits are reachable elsewhere (merged into HEAD) or pushed to origin;
/// otherwise the branch stays (cheap) and only the worktree directory goes
/// (`worktree_add_if_absent` re-attaches a surviving branch on a later run).
///
/// Work policy — never discard UNCOMMITTED work either: leftovers in a dirty
/// worktree are swept into a commit on the run branch BEFORE removal (2026-07-14:
/// a failed run's reap destroyed an implement step's uncommitted test suite —
/// the branch policy alone protects only what was committed). If that sweep
/// fails, the worktree directory is kept rather than destroyed.
///
/// Reopen policy: while a step session whose cwd is one of these worktrees can
/// still be reopened, the worktrees stay ([`run_worktrees_in_use`]) — "Open
/// session" on a finished step resumes the agent IN its worktree, and reaping
/// it under the session re-created an empty non-git dir that the next
/// startup sweep then deleted with whatever the user did there. Deferred
/// runs are reaped by the next sweep once their sessions are gone (or the
/// reopen TTL has passed).
async fn reap_run_worktrees(ctx: &ServerCtx, run_id: &str) {
    let dir = ctx.data_dir.join("workflow-runs").join(run_id);
    // Listing (and the per-entry `is_dir` stats) is sync fs work: blocking pool.
    #[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
    let listed = {
        let dir = dir.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::read_dir(&dir).ok().map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect::<Vec<_>>()
            })
        })
        .await
        .ok()
        .flatten()
    };
    let Some(worktrees) = listed else {
        return; // nothing provisioned
    };
    if run_worktrees_in_use(ctx, run_id, &dir).await {
        tracing::debug!(
            "wf reap: keeping {} — a step session there can still be reopened",
            dir.display()
        );
        return;
    }
    let branch = format!("otto-wf/{run_id}");
    let mut kept_any = false;
    for wt in worktrees {
        let wt_str = wt.to_string_lossy().to_string();
        // Owning repo root: the worktree's git-common-dir is `<root>/.git`.
        let wt_git = otto_git::LocalGit::new(&wt_str);
        // Not (or no longer) a readable git worktree: its content can't be
        // swept into a commit, so the directory cleanup below must not
        // bulldoze it either.
        let Ok(common) = wt_git
            .run(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .await
        else {
            tracing::warn!("wf reap: {wt_str} is not a readable git worktree — keeping it");
            kept_any = true;
            continue;
        };
        let common = common.trim();
        let Some(root) = std::path::Path::new(common).parent() else {
            kept_any = true;
            continue;
        };
        match wt_git
            .commit_all_if_dirty(&format!(
                "chore: preserve workflow leftovers (run {run_id})"
            ))
            .await
        {
            Ok(Some(sha)) => tracing::info!(
                "wf reap: committed leftover changes in {wt_str} onto {branch} ({})",
                &sha[..sha.len().min(9)]
            ),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    "wf reap: could not commit leftovers in {wt_str}: {e} — keeping the worktree"
                );
                kept_any = true;
                continue;
            }
        }
        let repo_git = otto_git::LocalGit::new(root);
        let _ = repo_git.worktree_remove(&wt_str).await;
        // Safe-delete the run branch: merged into HEAD OR pushed to origin.
        let merged = repo_git
            .run(&["branch", "--merged", "HEAD", "--list", &branch])
            .await
            .map(|out| !out.trim().is_empty())
            .unwrap_or(false);
        // Quiet probe: a deleted remote ref is the common case, not an error.
        let pushed = repo_git.origin_branch_exists(&branch).await;
        if merged || pushed {
            let _ = repo_git.run(&["branch", "-D", &branch]).await;
        } else {
            tracing::debug!(
                "wf reap: keeping branch {branch} in {} (unmerged, unpushed commits)",
                root.display()
            );
        }
    }
    // Only when every worktree was safely handled — a kept (unsweepable)
    // worktree must not be bulldozed by the directory cleanup.
    if !kept_any {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

/// Startup sweep: reap worktree leftovers of every run directory whose run is
/// no longer pending/running (crashed daemon, pre-reap versions). Called once
/// from ottod startup after [`reconcile_interrupted_runs`] — runs it chose to
/// resume are `pending` again by then, so their worktrees survive.
pub async fn sweep_stale_run_worktrees(ctx: &ServerCtx) {
    // One sweep at a time: it also runs at every run end (deferred reaps), and
    // two sweeps committing/removing the same worktree would race.
    static SWEEP: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let Ok(_sweeping) = SWEEP.get_or_init(|| tokio::sync::Mutex::new(())).try_lock() else {
        return;
    };
    let base = ctx.data_dir.join("workflow-runs");
    #[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
    let run_ids: Vec<String> = tokio::task::spawn_blocking(move || {
        std::fs::read_dir(&base)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    for run_id in run_ids {
        if run_id.is_empty() {
            continue;
        }
        // Active runs keep their worktrees.
        let active: Option<String> = sqlx::query_scalar(
            "SELECT id FROM workflow_runs WHERE id = ? AND status IN ('pending','running')",
        )
        .bind(&run_id)
        .fetch_optional(&ctx.pool)
        .await
        .ok()
        .flatten();
        if active.is_some() {
            continue;
        }
        reap_run_worktrees(ctx, &run_id).await;
    }
}

/// How long a FINISHED run's worktrees outlive it while one of its step
/// sessions is still reopenable (suspended). A LIVE session — a
/// `keep_session: true` step, or one still at its prompt — holds them for as
/// long as it lives.
const RUN_WORKTREE_REOPEN_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether any step session of `run_id` still works in (or can be reopened
/// in) the run's worktree dir `dir`: a live session always holds it, a
/// suspended (`reconnectable`) one within [`RUN_WORKTREE_REOPEN_TTL`] of the
/// run's end. Sessions elsewhere (review fleets, interactive ones) never hold.
async fn run_worktrees_in_use(ctx: &ServerCtx, run_id: &str, dir: &std::path::Path) -> bool {
    use otto_core::domain::SessionStatus;
    let Ok(run) = WorkflowsRepo::new(ctx.pool.clone())
        .get_run(&run_id.to_string())
        .await
    else {
        return false;
    };
    let ttl = chrono::Duration::from_std(RUN_WORKTREE_REOPEN_TTL)
        .unwrap_or_else(|_| chrono::Duration::hours(24));
    let reopen_window = run.finished_at.is_none_or(|f| chrono::Utc::now() - f < ttl);
    for sid in run.nodes.iter().flat_map(|n| n.sessions.iter()) {
        let Ok(s) = ctx.manager.get(sid).await else {
            continue;
        };
        if !std::path::Path::new(&s.cwd).starts_with(dir) {
            continue;
        }
        match s.status {
            SessionStatus::Exited => {}
            SessionStatus::Reconnectable => {
                if reopen_window {
                    return true;
                }
            }
            SessionStatus::Running | SessionStatus::Working | SessionStatus::Idle => return true,
        }
    }
    false
}

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
        // The origin repo's display name — filled from whichever registered repo
        // we resolve to, so `repos.json` names the repo even for a worktree-only run.
        let mut repo_name: Option<String> = None;
        // Any registered repo (S3-302). A declared NAME prefers the
        // workflow's own workspace, then the first registered repo of that name.
        let by_id = ctx.git_store.get_repo(&e.repo).await.ok();
        if let Some(r) = by_id {
            rid = Some(r.id.clone());
            repo_name = Some(r.name.clone());
            root = Some(r.path);
        } else if let Ok(repos) = ctx.git_store.list_all_repos().await {
            let named = || repos.iter().filter(|r| r.name == e.repo);
            if let Some(r) = named()
                .find(|r| &r.workspace_id == workspace_id)
                .or_else(|| named().next())
                .cloned()
            {
                rid = Some(r.id);
                repo_name = Some(r.name.clone());
                root = Some(r.path);
            }
        }
        if rid.is_none() {
            let hint = if e.kind == "worktree" {
                expand_tilde(&e.name)
            } else {
                expand_tilde(&e.repo)
            };
            // A worktree path resolves back to its ORIGIN repo (never treated as a
            // standalone repo) — origin identity, incl. its git account, is what
            // git-aware steps must use.
            if let Some(id) = resolve_repo_id_for_path(ctx, workspace_id, &hint).await {
                if let Ok(r) = ctx.git_store.get_repo(&id).await {
                    repo_name = Some(r.name.clone());
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
        e.repo_name = repo_name;
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
                e.error = Some(format!(
                    "unknown repos entry type '{other}' (want branch|worktree)"
                ));
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
    // Match by PATH against ALL registered repos, in any workspace (S3-302:
    // a run may work on any registered repo). Matching globally matters:
    // otherwise a path nested under a repo registered in the run's workspace
    // (e.g. the home dir) would win over the exact repo registered elsewhere
    // and silently target the wrong repo. `match_repo_path` picks the
    // deepest/exact match, so ~/proj/app → the app repo, not a parent-dir repo.
    let expanded = expand_tilde(path);
    let all = ctx.git_store.list_all_repos().await.ok()?;
    let pairs: Vec<(String, String)> = all.iter().map(|r| (r.id.clone(), r.path.clone())).collect();
    if let Some(id) = match_repo_path(&expanded, &pairs) {
        return Some(id);
    }
    if let Some(main) = git_main_worktree(&expanded).await {
        if let Some(id) = match_repo_path(&main, &pairs) {
            return Some(id);
        }
    }
    // Last resort (unchanged): a run workspace with exactly one repo → use it.
    if let Ok(ws_repos) = ctx.git_store.list_repos(workspace_id).await {
        if ws_repos.len() == 1 {
            return Some(ws_repos[0].id.clone());
        }
    }
    None
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
        adj.entry(e.source.clone())
            .or_default()
            .push(e.target.clone());
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

/// The version id a `product_*` step with `persist: true` saved. A failed
/// save used to be dropped silently: the step succeeded without its
/// `version_id`, and nothing said the result was never stored.
fn persisted_version(
    saved: otto_core::Result<otto_state::ProductStoryVersion>,
) -> otto_core::Result<String> {
    saved.map(|v| v.id).map_err(|e| {
        otto_core::Error::Internal(format!(
            "the step finished but saving its result as a product version failed: {e}"
        ))
    })
}

/// The log line of a step skipped because an upstream step failed — the
/// marker a single-step retry looks for to re-run what its target blocked.
pub(crate) const SKIPPED_UPSTREAM: &str = "skipped (upstream did not succeed)";

/// Data-dir-relative file of a `canvas` step's diagram. Step ids are only
/// checked for being non-empty and unique, so an id like `../../../tmp/x`
/// wrote (and created directories) outside the data dir; such an id falls
/// back to its slug, while an ordinary id keeps its exact file name.
fn canvas_rel(run_id: &str, node_id: &str, ext: &str) -> String {
    let name = otto_core::paths::safe_component(node_id)
        .filter(|n| !n.starts_with('.'))
        .map(str::to_string)
        .unwrap_or_else(|| crate::workflow_context::slug(node_id));
    format!("workflow-canvas/{run_id}/{name}.{ext}")
}

#[cfg(test)]
mod event_wait_tests {
    use super::{is_gate_decision, wake_on_event, Event};
    use std::time::Duration;

    fn update(run: &str, node: Option<&str>, waiting: bool) -> Event {
        Event::WorkflowRunUpdated {
            workspace_id: "ws".into(),
            run_id: run.into(),
            status: "running".into(),
            node_id: node.map(Into::into),
            rev: 1,
            node: None,
            nodes_done: 0,
            nodes_total: 0,
            waiting_approval: waiting,
        }
    }

    /// Only `approve_run`'s non-waiting update for THIS gate wakes it — not
    /// the engine's own pause announcement, another node, or another run.
    #[test]
    fn a_gate_wakes_only_on_its_own_decision() {
        assert!(is_gate_decision(
            &update("r", Some("gate"), false),
            "r",
            "gate"
        ));
        assert!(!is_gate_decision(
            &update("r", Some("gate"), true),
            "r",
            "gate"
        ));
        assert!(!is_gate_decision(
            &update("r", Some("other"), false),
            "r",
            "gate"
        ));
        assert!(!is_gate_decision(
            &update("r2", Some("gate"), false),
            "r",
            "gate"
        ));
        assert!(!is_gate_decision(&update("r", None, false), "r", "gate"));
    }

    /// A buffered decision (sent right after subscribing, before the wait
    /// starts) wakes at once; otherwise the wait lasts until the safety tick.
    #[tokio::test]
    async fn wake_on_event_returns_on_a_buffered_match_or_the_deadline() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        tx.send(update("r", Some("other"), false)).unwrap();
        tx.send(update("r", Some("gate"), false)).unwrap();
        let far = tokio::time::Instant::now() + Duration::from_secs(30);
        tokio::time::timeout(
            Duration::from_secs(2),
            wake_on_event(&mut rx, |ev| is_gate_decision(ev, "r", "gate"), far),
        )
        .await
        .expect("woke on the buffered decision");
        let soon = tokio::time::Instant::now() + Duration::from_millis(50);
        let started = tokio::time::Instant::now();
        wake_on_event(&mut rx, |ev| is_gate_decision(ev, "r", "gate"), soon).await;
        assert!(started.elapsed() >= Duration::from_millis(45));
    }
}

#[cfg(test)]
#[path = "../tests/unit/workflow_queue_cancel.rs"]
mod queue_cancel_tests;

#[cfg(test)]
#[path = "../tests/unit/workflow_canvas_persist.rs"]
mod canvas_rel_tests;

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // isolated fixtures use sync fs / process / secret store
#[path = "../tests/unit/workflow_engine.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/workflow_product_publish_tests.rs"]
mod product_publish_tests;

#[path = "workflow_product_publish.rs"]
mod product_publication;

#[cfg(test)]
#[path = "../tests/unit/workflow_recovery_tests.rs"]
mod recovery_tests;

#[path = "workflow_budget.rs"]
mod budget;

#[path = "workflow_api_request.rs"]
mod api_request;

mod nodes;

mod run;
#[cfg(test)]
use run::identity::acting_user_id;
use run::identity::run_identity;
