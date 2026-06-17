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
}

/// A directed connection from one node's output to another node's input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
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
