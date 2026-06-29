//! Workflow engine domain types — an n8n-style node graph plus its run state.
//!
//! A [`Workflow`] is a DAG of [`WorkflowNode`]s joined by [`WorkflowEdge`]s. The
//! graph is stored as JSON; the executor (in `otto-server`) topologically sorts
//! it, runs each node by its `kind`, and threads each node's JSON output along
//! the edges to its successors. A [`WorkflowRun`] records per-node state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        Self { max_attempts: 0, backoff_ms: 0, factor: 2.0 }
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
    pub graph: WorkflowGraph,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Monotonic version counter; bumped on every graph-changing update.
    #[serde(default = "default_workflow_version")]
    pub version: i64,
}

fn default_workflow_version() -> i64 {
    1
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
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

/// Per-node execution state, captured during a run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRunState {
    pub node_id: String,
    pub status: NodeStatus,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub logs: Vec<String>,
    /// Wall-clock execution time of this node, once it has run.
    #[serde(default)]
    pub duration_ms: Option<u64>,
    /// Number of attempts the engine made (1 = no retry). Set when a retry
    /// policy is configured; `None`/`Some(1)` for the common single-attempt case.
    #[serde(default)]
    pub attempts: Option<u32>,
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
    #[serde(default)]
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(default)]
    pub finished_at: Option<DateTime<Utc>>,
    /// The workflow `version` this run executed (snapshot at run start).
    #[serde(default)]
    pub workflow_version: Option<i64>,
    /// The Proof Pack assembled for this run on completion, if any.
    #[serde(default)]
    pub proof_pack_id: Option<String>,
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
    pub graph: Option<WorkflowGraph>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateWorkflowReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub graph: Option<WorkflowGraph>,
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
}

/// A ready-made example workflow the user can instantiate (e.g. game pipelines
/// combining an agent design step + the engine scaffold).
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
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
    pub graph: WorkflowGraph,
    /// A short note (e.g. "edited graph", "restored from v3").
    #[serde(default)]
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
