//! Workflow engine domain types — an n8n-style node graph plus its run state.
//!
//! A [`Workflow`] is a DAG of [`WorkflowNode`]s joined by [`WorkflowEdge`]s. The
//! graph is stored as JSON; the executor (in `otto-server`) topologically sorts
//! it, runs each node by its `kind`, and threads each node's JSON output along
//! the edges to its successors. A [`WorkflowRun`] records per-node state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent_runtime::{Attention, TranscriptDigest, TurnTokenUsage, WaitReason};
use crate::Id;

/// Per-node retry policy. Defaults to *no retry* (`max_attempts = 0`), so an
/// existing workflow with no policy keeps its single-attempt behavior. The engine
/// retries a failed node up to `max_attempts` extra times, sleeping `backoff_ms`
/// before the first retry and multiplying by `factor` each subsequent retry. It
/// lives inside `graph_json` (no migration needed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Extra attempts after the first (0 = run once). Clamped to 5 by the engine.
    #[serde(default)]
    pub max_attempts: u32,
    /// Initial backoff before the first retry, in milliseconds. Clamped to 60000.
    #[serde(default)]
    pub backoff_ms: u64,
    /// Multiplier applied to the backoff after each retry (default 2.0).
    #[serde(default = "default_retry_factor")]
    pub factor: f64,
}

fn default_retry_factor() -> f64 {
    2.0
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 0,
            backoff_ms: 0,
            factor: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Clamp to sane bounds before the engine uses it.
    pub fn clamped(&self) -> RetryPolicy {
        RetryPolicy {
            max_attempts: self.max_attempts.min(5),
            backoff_ms: self.backoff_ms.min(60_000),
            factor: if self.factor.is_finite() && self.factor >= 1.0 {
                self.factor.min(10.0)
            } else {
                2.0
            },
        }
    }
}

/// A node in a workflow graph. `kind` selects the executor behavior (see the
/// node-type catalog in `otto-server`); `params` is the node's configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default)]
    pub params: Value,
    /// Optional retry policy for this node (migration-free; stored in graph_json).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
}

/// A directed connection from one node's output to another node's input.
///
/// `condition` is an optional [`crate::expr`] expression evaluated against
/// `{ output, input, node, run }` after the source node succeeds; the edge is
/// "active" (propagates output / makes its target runnable) only when the
/// expression is truthy. An absent condition is always active — the legacy
/// behavior, so existing graphs are unchanged. Stored in graph_json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// The full node graph.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowGraph {
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

/// A saved workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: Id,
    pub workspace_id: Id,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    pub graph: WorkflowGraph,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Monotonic version counter; bumped on every graph-changing update.
    #[serde(default = "default_workflow_version")]
    pub version: i64,
    /// What the startup reconciler does with a run of this workflow that a
    /// daemon restart interrupted: `"resume"` (default) re-enters it from the
    /// interrupted step, `"fail"` preserves the legacy hard-fail behavior.
    #[serde(default = "default_on_restart")]
    pub on_restart: String,
    /// Where the workflow came from: `None` (authored) | `goal`. Server-set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// `review:<id>` | `insights` | `goal_loop:<id>` | `mcp`. Server-set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_ref: Option<String>,
}

fn default_workflow_version() -> i64 {
    1
}

pub fn default_on_restart() -> String {
    "resume".into()
}

/// Execution scope of a (re-)entered run — which node execution starts from
/// and how. The default (`start_node: None`) runs the whole graph. Persisted
/// on the run row as `resume_scope_json` while the run is in flight so a
/// daemon restart can re-enter with the same scope, and cleared on terminal
/// status.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunScope {
    /// Restart continuation adopts settled nodes inside the ORIGINAL scope.
    /// Explicit operator retries leave this false and rerun their selected set.
    #[serde(default)]
    pub continue_unfinished: bool,
    /// Start execution from this node (it plus everything downstream unless
    /// `only_node`). Out-of-scope nodes adopt their prior states.
    #[serde(default)]
    pub start_node: Option<String>,
    /// Run only `start_node` itself, not its descendants.
    #[serde(default)]
    pub only_node: bool,
    /// Treat `start_node` itself as ALREADY DONE (adopt its prior state) and
    /// run only its descendants — the restart reconciler uses this when the
    /// interrupted step's handoff file proves the step actually finished.
    #[serde(default)]
    pub adopt_start: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Success,
    Error,
    Canceled,
}

impl RunStatus {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "success" => Some(Self::Success),
            "error" => Some(Self::Error),
            "canceled" => Some(Self::Canceled),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Success => "success",
            Self::Error => "error",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    #[default]
    Pending,
    Running,
    Success,
    Error,
    Skipped,
}

impl NodeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Success => "success",
            Self::Error => "error",
            Self::Skipped => "skipped",
        }
    }
}

/// One sub-agent / background task of a running agent step, for display.
/// Status comes ONLY from the parent transcript (launch seen, notification
/// seen); never from the child's own file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentActivity {
    pub id: String,
    /// Truncated to 80 chars.
    pub description: String,
    /// "running" | "done" | "failed"
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

/// Live activity of a RUNNING agent step (present only while it runs; the
/// engine clears it on finish). Bounded: ≤ 40 sub-agents, ids ≤ 64 chars.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeActivity {
    /// The phase log text without its prefix glyph (e.g. "waiting on 2 sub-agents").
    pub phase: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<DateTime<Utc>>,
    pub pending_tasks: u32,
    #[serde(default)]
    pub subagents: Vec<SubagentActivity>,
    /// Why the engine is holding a step that looks idle, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Run view: step phases, people, threads and the run overview
// (design/agreed-names.md §2). Every field is additive and optional, so the
// previous build reads rows that carry them (it ignores unknown keys) and
// `RunStatus` / `NodeStatus` never change. Enums end in an `Unknown`
// catch-all so a later build can add a value without breaking a rollback.
// ---------------------------------------------------------------------------

/// Where a RUNNING step is. `None` on `NodeRunState::phase` once it settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepPhase {
    /// Waiting for a run step slot (run order).
    Queued,
    /// Waiting for machine admission; `NodeRunState::wait` says why.
    Waiting,
    /// Thread created, CLI starting.
    Starting,
    Working,
    /// Waiting on a person; `NodeRunState::attention` says what for.
    NeedsYou,
    /// The turn ended; background work, handoff nudge, copy-out, close.
    Finishing,
    #[serde(other)]
    Unknown,
}

/// One entry of a step's append-only phase timeline (≤ 40 kept).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseMark {
    pub phase: StepPhase,
    pub at: DateTime<Utc>,
    /// ≤ 200 chars: "attempt 2 of 3 — …", a hold kind, "canceled".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A step's agent thread.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepThread {
    /// Runtime session id, or the PTY session id when `runner == "pty"`.
    pub thread_id: Id,
    /// `threads` | `pty`.
    pub runner: String,
    pub provider: String,
    /// What actually ran, not what was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Node id whose thread this step continued (`continue_from`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continued_from: Option<String>,
    /// Turn ordinal the step's prompt started at (restart: `await_settled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ordinal: Option<u64>,
    /// Prompted + agent-started turns of this step.
    #[serde(default)]
    pub turns: u32,
    #[serde(default)]
    pub usage: TurnTokenUsage,
    /// ≤ 8 KiB serialized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<TranscriptDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<DateTime<Utc>>,
    /// Computed on read (false once retention deleted the thread); never stored.
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub available: Option<bool>,
}

/// A sub-thread of a step: a reviewer, the scope agent, the summarizer, or a
/// thread the step's agent opened.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildThread {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Id>,
    /// "Correctness · Codex".
    pub label: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// `queued` | `pending` | `running` | `success` | `error` | `skipped` | `needs_you`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    /// "12 findings".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub usage: TurnTokenUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliverableState {
    Done,
    Partial,
    Failed,
    PendingApproval,
    #[serde(other)]
    Unknown,
}

/// A file that came with the chat message that started (`start`) or
/// steered (`steer`) a run, as copied into its context dir
/// (`<context>/attachments.json`; addendum-chat-optimize.md §1.2, DR-114).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttachmentRef {
    /// The sender's file name (display only).
    pub name: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    /// Context-relative copy, `attachments/<safe name>`; absent when the
    /// file could not be received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub source: AttachmentSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentSource {
    Start,
    Steer,
    #[serde(other)]
    Unknown,
}

/// Something a step put outside Otto (PR comments, a Jira comment, a page).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Deliverable {
    pub label: String,
    pub state: DeliverableState,
    /// "5 of 5 posted", "permission denied: …".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// What a person said to a step (an answer, a verification, a steering note).
/// Kept on the node (≤ 20), so it survives the thread's deletion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HumanEntry {
    /// `answer` | `verify` | `steer`.
    pub kind: String,
    /// The question, or the criterion id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<String>,
    /// ≤ 2 KiB.
    pub text: String,
    /// `verify` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub met: Option<bool>,
    /// `verify` only: SHA-256 of the criterion's canonical JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    /// Display name.
    pub by: String,
    pub at: DateTime<Utc>,
}

/// The run state shown to people (`RunOverview::state`), computed from the
/// stored status, the hold and the steps; never stored as a status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Running,
    Waiting,
    NeedsYou,
    Succeeded,
    Failed,
    Canceled,
    Paused,
    Blocked,
    Exhausted,
    #[default]
    #[serde(other)]
    Unknown,
}

/// A step waiting on a person, as the run overview lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttentionRef {
    pub node_id: String,
    pub name: String,
    #[serde(flatten)]
    pub attention: Attention,
}

/// A fan-out's progress (reviewers of a review step).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FanProgress {
    pub done: u32,
    pub total: u32,
    pub running: u32,
}

/// What the run is doing now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Now {
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub node_names: Vec<String>,
    pub phase: StepPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan: Option<FanProgress>,
}

/// A queued run's place in line (within its lane).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueInfo {
    /// 1-based.
    pub position: u32,
    pub ahead: u32,
    pub cap: u32,
    /// `normal` | `long`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
}

/// The failing step's own error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailedStep {
    pub node_id: String,
    pub name: String,
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
}

/// Where the run's wall clock went; the buckets sum to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeBuckets {
    #[serde(default)]
    pub queued_ms: u64,
    #[serde(default)]
    pub waiting_ms: u64,
    #[serde(default)]
    pub with_you_ms: u64,
    #[serde(default)]
    pub working_ms: u64,
}

/// Who or what started the run — names, not ids.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    /// `chat` | `manual` | `schedule` | `webhook` | `event` | `task` | `mcp` | `goal`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectPr {
    pub repo: String,
    pub number: u64,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubjectTicket {
    pub key: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// What the run is about.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Subject {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<SubjectPr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<SubjectTicket>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// A goal run's progress (the run's graph has a loop with a check step).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoalProgress {
    pub iteration: u32,
    /// Effective: `limits_json` over the loop's params.
    pub max_iterations: u32,
    pub criteria_met: u32,
    pub criteria_total: u32,
    #[serde(default)]
    pub active_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_ms: Option<u64>,
    /// Criterion ids parked for a person.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub awaiting: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldKind {
    Paused,
    Blocked,
    Exhausted,
    #[serde(other)]
    Unknown,
}

/// A held run (`workflow_runs.hold_json`). A held run is settled: paused =
/// status `canceled` + hold, blocked / exhausted = status `error` + hold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunHold {
    pub kind: HoldKind,
    /// ≤ 300 chars, words a person reads.
    pub reason: String,
    pub since: DateTime<Utc>,
    /// The person who paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id>,
    /// Nodes to re-enter on Resume.
    #[serde(default)]
    pub node_ids: Vec<String>,
    /// The loop's iteration when it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
}

impl RunHold {
    /// The one effective-hold rule (DR-105): a stored hold counts only while
    /// its kind matches the stored status — paused with `canceled`, blocked
    /// or exhausted with `error`. A hold the previous build left behind on a
    /// row it retried (then ran again) is ignored. `otto_state`'s `HOLD_SQL`
    /// is the same rule in SQL.
    pub fn is_effective(&self, status: RunStatus) -> bool {
        matches!(
            (self.kind, status),
            (HoldKind::Paused, RunStatus::Canceled)
                | (HoldKind::Blocked | HoldKind::Exhausted, RunStatus::Error)
        )
    }
}

/// One loop's limits raised by Resume / Extend (`limits_json.loops[<node id>]`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LoopLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_active_s: Option<u64>,
    /// The iteration a person resumed a Blocked hold at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unblocked_at: Option<u32>,
}

/// `workflow_runs.limits_json`: run-level loop limits, written only by Resume.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunLimits {
    #[serde(default)]
    pub loops: std::collections::BTreeMap<String, LoopLimits>,
}

/// The run's one computed summary — the UI, the chat status message, MCP and
/// the phone read this object (field `overview`, never `summary`, which is
/// the progress projection's boolean marker). Stored in
/// `workflow_runs.overview_json` with the row's `rev`; a reader whose row
/// `rev` differs treats it as stale (see `otto_state`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunOverview {
    pub rev: i64,
    pub state: RunState,
    /// One sentence, ≤ 160 chars.
    pub headline: String,
    pub steps_done: u32,
    pub steps_total: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub now: Option<Now>,
    /// Oldest first.
    pub attention: Vec<AttentionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue: Option<QueueInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<FailedStep>,
    /// All steps', in step order.
    pub deliverables: Vec<Deliverable>,
    /// Sum over steps and children.
    pub usage: TurnTokenUsage,
    pub time: TimeBuckets,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Subject>,
    pub elapsed_ms: u64,
    /// Median of the workflow's last 20 successful runs; `None` under 3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typical_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<GoalProgress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold: Option<RunHold>,
}

/// How a `check` criterion is verified (wire values `command` | `agent` |
/// `human` | `expr`; the UI calls `expr` "Rule").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyKind {
    Command,
    Agent,
    Human,
    Expr,
    /// A newer build's kind: "not met: unknown check".
    #[serde(other)]
    Unknown,
}

/// One acceptance criterion of a `check` node or a goal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    /// `[a-z0-9-]{1,64}`, unique in the node.
    pub id: String,
    /// What must be true, ≤ 500 chars.
    pub text: String,
    pub verify: VerifyKind,
    /// Agent / human: what to inspect, ≤ 2,000 chars.
    #[serde(default)]
    pub how: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// Command only; default 600, 10..=3,600.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u64>,
}

/// One criterion's result in a `check` step's output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionResult {
    pub id: String,
    pub text: String,
    pub verify: VerifyKind,
    pub met: bool,
    pub evidence: String,
    /// `command` | `rule` | `agent` | the person's name.
    pub verified_by: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckVerdict {
    Achieved,
    Continue,
    Blocked,
    #[serde(other)]
    Unknown,
}

/// The completion review of a `check` with `final_review` (passes only when
/// approved with no findings).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompletionReview {
    pub approved: bool,
    #[serde(default)]
    pub findings: Vec<String>,
}

/// A `check` step's output (also rendered to its step file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckOutput {
    pub criteria: Vec<CriterionResult>,
    pub all_met: bool,
    pub met: u32,
    pub total: u32,
    pub progress_pct: u32,
    pub verdict: CheckVerdict,
    /// What the next iteration must do.
    #[serde(default)]
    pub feedback: String,
    /// ≤ 3 KiB: unmet criteria with evidence headlines + feedback.
    #[serde(default)]
    pub feedback_md: String,
    #[serde(default)]
    pub repeat_signature: String,
    #[serde(default)]
    pub repeated_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<CompletionReview>,
}

/// Per-node execution state, captured during a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeRunState {
    pub node_id: String,
    pub status: NodeStatus,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    /// When this node started executing (set on the pending→running
    /// transition) — lets the UI show a live elapsed timer on a running step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// Wall-clock execution time of this node, once it has run.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Number of attempts the engine made (1 = no retry). Set when a retry
    /// policy is configured; `None`/`Some(1)` for the common single-attempt case.
    #[serde(default)]
    pub attempts: Option<u32>,
    /// Openable Otto session id(s) this node spawned (agent / product / canvas /
    /// loop-inner agents). Recorded live the moment a session is created so the
    /// run view can open it while running. Empty for non-agent nodes.
    #[serde(default)]
    pub sessions: Vec<String>,
    /// Reviews spawned by this node, published before waiting and retained for
    /// asynchronous reviews and later summarizer retries.
    #[serde(default)]
    pub review_ids: Vec<String>,
    /// Live sub-agent / phase snapshot while the node runs; `None` once it
    /// finished (and on rows persisted before the field existed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<NodeActivity>,
    /// The stable N of `stepN-…` (`None` on old rows and skipped nodes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_no: Option<u32>,
    /// `step3-checkout-pr-branch` (no extension).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_file: Option<String>,
    /// Only while `status == Running`; `None` once settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<StepPhase>,
    /// Append-only phase timeline, ≤ 40 (the first mark + the last 39).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phases: Vec<PhaseMark>,
    /// Set while `phase` is `waiting` (or `queued` on a run slot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<WaitReason>,
    /// Set while `phase` is `needs_you`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<Attention>,
    /// Runner: `usage_limit` | `rate_limit` | `auth` | `provider` |
    /// `stalled` | `timeout` | `spawn` | `server_restart` | `canceled`;
    /// engine: `script` | `outcome_unknown`. `error` stays the step's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_class: Option<String>,
    /// The step's agent thread (thread ids never go in `sessions`, which
    /// stays PTY-only for the previous build's run view).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<StepThread>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ChildThread>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliverables: Vec<Deliverable>,
    /// ≤ 20, oldest dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub human_log: Vec<HumanEntry>,
    /// One-line summary set at settle, ≤ 200 chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

/// One durable inner-loop attempt.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowCheckpoint {
    pub node_id: String,
    pub loop_id: String,
    pub iteration: u64,
    pub step_index: usize,
    pub kind: String,
    pub name: String,
    pub status: NodeStatus,
    pub attempts: u32,
    pub input: Value,
    pub output: Option<Value>,
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    pub updated_at: DateTime<Utc>,
    /// The attempt's wall time minus its time in `needs_you` (banked loop
    /// time: a loop's active time is the sum over its checkpoints).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub active_ms: u64,
    /// The inner step's thread, so a restart mid-iteration resumes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Id>,
    /// Turn ordinal the inner step's prompt started at (`await_settled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_ordinal: Option<u64>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// One execution of a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: Id,
    pub workflow_id: Id,
    pub workspace_id: Id,
    pub status: RunStatus,
    #[serde(default)]
    pub input: Value,
    #[serde(default)]
    pub nodes: Vec<NodeRunState>,
    /// Durable inner-loop attempts, populated on the run detail endpoint.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<WorkflowCheckpoint>,
    #[serde(default)]
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    /// Monotonic revision, bumped on every persisted progress write (node
    /// transitions, the human-approval pause, and the approve/reject
    /// decision). Clients use it to discard stale/out-of-order snapshots and
    /// to apply `workflow_run_updated` events in order. 0 for legacy rows.
    #[serde(default)]
    pub rev: i64,
    /// True while the run is paused at a `human_approval` node.
    #[serde(default)]
    pub waiting_approval: bool,
    /// The `human_approval` node the run is paused at (while waiting).
    #[serde(default)]
    pub approval_node_id: Option<String>,
    /// User id of the approver (None when rejected or still pending).
    #[serde(default)]
    pub approved_by: Option<String>,
    /// User who started the run (the user its agent sessions are created as);
    /// None for trigger / schedule / chat-initiated runs, which act as the
    /// workflow's creator.
    #[serde(default)]
    pub created_by: Option<Id>,
    /// Note attached to the approval/rejection decision.
    #[serde(default)]
    pub approval_note: Option<String>,
    /// When the decision was recorded.
    #[serde(default)]
    pub approved_at: Option<DateTime<Utc>>,
    /// The workflow `version` this run executed (snapshot at run start).
    #[serde(default)]
    pub workflow_version: Option<i64>,
    /// How many times the startup reconciler resumed this run after a daemon
    /// restart (0 = never interrupted). Drives the "resumed after restart
    /// (attempt N)" badge in the run view.
    #[serde(default)]
    pub resume_attempts: i64,
    /// Absolute path of the run's context directory
    /// (`<data_dir>/workflow-context/<run_id>/` — instruction brief,
    /// `repos.json`, per-step handoff files). Derived at the API layer when
    /// the directory exists on disk — NOT persisted; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_dir: Option<String>,
    /// The computed run summary; `None` when the stored one is stale (its
    /// `rev` is not the row's) — the server recomputes it then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<RunOverview>,
    /// Set while the run is held (paused / blocked / exhausted) — the
    /// effective hold only ([`RunHold::is_effective`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<RunHold>,
    /// Loop limits raised by Resume / Extend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<RunLimits>,
    /// Steering notes queued for the next agent step (the run's
    /// `steering.json`). Filled at the API layer — NOT persisted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steering_queued: Vec<String>,
    /// Files sent with the starting and steering chat messages. Filled at
    /// the API layer — NOT persisted on the row.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentRef>,
}

/// Lightweight summary of an in-flight workflow run for the "Running" sidebar
/// list. Joins the run with its workflow name and pre-computes step progress so
/// the UI needs no extra fetch per run. Returned by
/// `GET /workspaces/{wid}/workflow-runs/active` (status pending|running only).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveWorkflowRun {
    pub run_id: Id,
    pub workflow_id: Id,
    pub workspace_id: Id,
    pub workflow_name: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    /// Total nodes in the run, and how many have reached a terminal state
    /// (success or skipped) — drives the "3/5 steps" progress chip.
    pub nodes_total: u32,
    pub nodes_done: u32,
    /// The run is paused on a human-approval node (surfaced as a ⏸ in the list).
    #[serde(default)]
    pub waiting_approval: bool,
    /// The step the run is on: the first `running` node (else the first
    /// `pending` one once something ran) — its id and display name (the node's
    /// name, else its kind). `None` before the first step starts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    /// The run's stored overview when fresh (see [`WorkflowRun::overview`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<RunOverview>,
}

/// One row of the run history (`GET /workspaces/{wid}/workflow-runs`): what a
/// run was, where it worked and how it went — no node/input bodies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunListItem {
    pub id: Id,
    pub workflow_id: Id,
    pub workflow_name: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// `manual` (no trigger recorded), `chat` (Slack/Telegram), `schedule`,
    /// `event`, `webhook`, … — the run input's `trigger`.
    pub trigger: String,
    /// The ask: the input's `msg`, else `prompt` (clipped to 160 chars).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Repo (last path segment of the working directory / first declared repo).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The PR's source branch when named, else the run's base.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The input's `pr` as given (`15`, `repo-a#15, repo-b#340`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    /// The step a running run is on (see [`ActiveWorkflowRun::current_step`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<String>,
    /// The run's error, clipped to 240 chars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The run's stored overview when fresh (see [`WorkflowRun::overview`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overview: Option<RunOverview>,
    /// What the run is about: its overview's subject, else derived from the
    /// input (a run from before overviews existed stores none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Subject>,
}

/// A page of [`RunListItem`]s, newest first; `next_cursor` is opaque and
/// absent on the last page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunListPage {
    pub items: Vec<RunListItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Catalog entry describing a node kind for the editor palette. Returned by
/// `GET /workflows/node-types` so the UI and executor stay in sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeSpec {
    pub kind: String,
    pub label: String,
    pub category: String,
    pub description: String,
    /// Number of input ports (0 = trigger/source node).
    pub inputs: u8,
    /// Number of output ports.
    pub outputs: u8,
    /// Accent color (hex) for the node card.
    pub color: String,
    /// Icon name (matches the UI Icon set).
    pub icon: String,
    /// JSON-Schema-ish description of this kind's output shape (the keys it
    /// produces). Drives the UI's expression hints and the engine's warn-only
    /// output validation. `None` when the output is free-form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// JSON-Schema-ish description of this kind's params (best-effort; for the UI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params_schema: Option<Value>,
}

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CreateWorkflowReq {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub graph: Option<WorkflowGraph>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWorkflowReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub graph: Option<WorkflowGraph>,
    /// Restart policy: `"resume"` | `"fail"` (see [`Workflow::on_restart`]).
    #[serde(default)]
    pub on_restart: Option<String>,
    /// The `version` the editor started from; a newer stored version is a
    /// `409` instead of silently overwriting edits made elsewhere.
    #[serde(default)]
    pub expected_version: Option<i64>,
}

impl UpdateWorkflowReq {
    /// Refuse an unknown `on_restart`, or a save based on an older `version`
    /// than the stored `current` one.
    pub fn check(&self, current: i64) -> crate::Result<()> {
        if let Some(v) = self.on_restart.as_deref() {
            if !matches!(v, "resume" | "fail") {
                return Err(crate::Error::Invalid(format!(
                    "on_restart must be 'resume' or 'fail', got '{v}'"
                )));
            }
        }
        if self.expected_version.is_some_and(|v| v != current) {
            return Err(crate::Error::Conflict(format!(
                "workflow changed elsewhere (now version {current})"
            )));
        }
        Ok(())
    }
}

/// `POST /workflow-runs/{id}/retry-node` — re-enter a finished run in place
/// (see the route for semantics). Default: retry ONE errored step. With
/// `include_downstream`, re-run the step AND everything after it — the
/// "run from here, keeping this run's files/worktree" flow.
#[derive(Debug, Clone, Deserialize)]
pub struct RetryRunNodeReq {
    pub node_id: String,
    #[serde(default)]
    pub include_downstream: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunWorkflowReq {
    #[serde(default)]
    pub input: Option<Value>,
    /// Start execution from this node (it plus everything downstream). When
    /// absent, the whole graph runs. Upstream nodes are marked skipped.
    #[serde(default)]
    pub start_node: Option<String>,
    /// Run only `start_node` itself, not its descendants.
    #[serde(default)]
    pub only_node: bool,
    /// Per-run review-mode override (`"fan_out"` | `"orchestrator"`): seeded into
    /// `input.review_mode`, wins over every `review_run` node's `params.mode`.
    /// Validated by the route (400 on any other value).
    #[serde(default)]
    pub review_mode: Option<String>,
}

/// A ready-made example workflow the user can instantiate (e.g. game pipelines
/// combining an agent design step + the engine scaffold).
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub instructions: String,
    /// Icon name for the UI.
    pub icon: String,
    pub graph: WorkflowGraph,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FromTemplateReq {
    pub template_id: String,
    #[serde(default)]
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// Versioning
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of a workflow's graph. A new version row is written
/// on create (v1) and on every graph-changing update; a run records which version
/// it executed. History is append-only — restoring a version writes a *new*
/// version equal to the chosen one rather than rewinding the counter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVersion {
    pub id: Id,
    pub workflow_id: Id,
    pub version: i64,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub instructions: String,
    pub graph: WorkflowGraph,
    /// A short note (e.g. "edited graph", "restored from v3").
    #[serde(default)]
    pub note: String,
    /// Restart policy snapshot (see [`Workflow::on_restart`]).
    #[serde(default = "default_on_restart")]
    pub on_restart: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

/// Metadata-only keyset history row; full definitions are fetched by version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowVersionSummary {
    pub id: Id,
    pub workflow_id: Id,
    pub version: i64,
    pub note: String,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
}

/// Request body for `POST /workflows/{id}/versions/{v}/restore`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RestoreVersionReq {
    /// Optional note recorded on the new version created by the restore.
    #[serde(default)]
    pub note: Option<String>,
}

/// Request body for `POST /scheduled-tasks/{id}/convert-to-workflow`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ConvertTaskReq {
    /// Disable the source scheduled task after converting (default false).
    #[serde(default)]
    pub disable_task: bool,
}

/// Response for the convert endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct ConvertTaskResp {
    pub workflow_id: Id,
    pub trigger_id: Option<Id>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A node that uses none of the run-view fields writes exactly the keys
    /// the previous build wrote, so its rows stay byte-for-byte as before.
    #[test]
    fn a_plain_node_writes_only_the_old_keys() {
        let node = NodeRunState {
            node_id: "n1".into(),
            ..Default::default()
        };
        let v = serde_json::to_value(&node).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "attempts",
                "duration_ms",
                "error",
                "logs",
                "node_id",
                "output",
                "review_ids",
                "sessions",
                "status"
            ]
        );
        assert_eq!(v["status"], "pending");
    }

    /// A value a later build adds decodes as the catch-all instead of failing
    /// the whole row (rollback safety for the next pull request).
    #[test]
    fn unknown_values_decode_as_the_catch_all() {
        let phase: StepPhase = serde_json::from_value(json!("parked_somewhere")).unwrap();
        assert_eq!(phase, StepPhase::Unknown);
        let state: RunState = serde_json::from_value(json!("snoozed")).unwrap();
        assert_eq!(state, RunState::Unknown);
        let hold: HoldKind = serde_json::from_value(json!("frozen")).unwrap();
        assert_eq!(hold, HoldKind::Unknown);
        let verify: VerifyKind = serde_json::from_value(json!("oracle")).unwrap();
        assert_eq!(verify, VerifyKind::Unknown);
        let state: DeliverableState = serde_json::from_value(json!("queued")).unwrap();
        assert_eq!(state, DeliverableState::Unknown);
        let ov: RunOverview = serde_json::from_value(json!({"state": "needs_you"})).unwrap();
        assert_eq!((ov.state, ov.rev), (RunState::NeedsYou, 0));
    }

    /// The overview's attention rows carry the attention's fields at the top
    /// level beside the node id and name (the UI and chat read them flat).
    #[test]
    fn attention_ref_is_flat_on_the_wire() {
        let at = chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 1, 1, 0, 0, 0).unwrap();
        let r = AttentionRef {
            node_id: "n4".into(),
            name: "Post comments".into(),
            attention: Attention {
                request_id: Some("q1".into()),
                ..Attention::new(crate::agent_runtime::AttentionKind::Approval, "Approve", at)
            },
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(
            v,
            json!({"node_id": "n4", "name": "Post comments", "kind": "approval",
                   "text": "Approve", "since": "2026-01-01T00:00:00Z", "request_id": "q1"})
        );
        assert_eq!(serde_json::from_value::<AttentionRef>(v).unwrap(), r);
    }

    /// `thread.available` is computed on read: written to the API, never
    /// taken back from a row.
    #[test]
    fn thread_availability_is_never_read_back() {
        let t: StepThread = serde_json::from_value(json!({
            "thread_id": "t1", "runner": "threads", "provider": "claude", "available": false
        }))
        .unwrap();
        assert_eq!(t.available, None);
        assert_eq!(t.turns, 0);
        let shown = StepThread {
            available: Some(false),
            ..t
        };
        assert_eq!(serde_json::to_value(&shown).unwrap()["available"], false);
    }

    #[test]
    fn a_hold_counts_only_with_its_matching_status() {
        let hold = |kind| RunHold {
            kind,
            reason: "r".into(),
            since: Utc::now(),
            by: None,
            node_ids: vec![],
            iteration: None,
        };
        use HoldKind::*;
        use RunStatus::*;
        for (kind, status, want) in [
            (Paused, Canceled, true),
            (Blocked, Error, true),
            (Exhausted, Error, true),
            (Paused, Error, false),
            (Blocked, Canceled, false),
            (Paused, Success, false),
            (Blocked, Running, false),
            (Exhausted, Pending, false),
            (Unknown, Error, false),
        ] {
            assert_eq!(
                hold(kind).is_effective(status),
                want,
                "{kind:?} × {status:?}"
            );
        }
    }

    #[test]
    fn checkpoint_active_time_defaults_and_is_omitted_at_zero() {
        let cp: WorkflowCheckpoint = serde_json::from_value(json!({
            "node_id": "l#0.0", "loop_id": "l", "iteration": 0, "step_index": 0,
            "kind": "agent_prompt", "name": "Work", "status": "success", "attempts": 1,
            "input": {}, "output": null, "error": null, "updated_at": "2026-01-01T00:00:00Z"
        }))
        .unwrap();
        assert_eq!(cp.active_ms, 0);
        assert!(serde_json::to_value(&cp)
            .unwrap()
            .get("active_ms")
            .is_none());
        let banked = WorkflowCheckpoint {
            active_ms: 42,
            ..cp
        };
        assert_eq!(serde_json::to_value(&banked).unwrap()["active_ms"], 42);
    }
}
