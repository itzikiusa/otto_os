//! FROZEN copy of the workflow serde types persisted by Otto 0.6 (main at
//! 27599cf2d): `workflows.graph_json` / `workflow_versions.graph_json`
//! (WorkflowGraph), `workflow_runs.nodes_json` (Vec<NodeRunState>),
//! `workflow_runs.resume_scope_json` (RunScope) and
//! `workflow_checkpoints.checkpoint_json` (WorkflowCheckpoint).
//!
//! Never edit this file to follow `otto_core::workflows`: it is what an old
//! build wrote and reads. `workflow_rows_compat.rs` proves rows written by
//! these types decode into today's types (an upgrade with old rows on disk) and
//! today's rows decode into these (a rollback to the old binary).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    #[serde(default)]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff_ms: u64,
    #[serde(default = "default_retry_factor")]
    pub factor: f64,
}

fn default_retry_factor() -> f64 {
    2.0
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowGraph {
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunScope {
    #[serde(default)]
    pub continue_unfinished: bool,
    #[serde(default)]
    pub start_node: Option<String>,
    #[serde(default)]
    pub only_node: bool,
    #[serde(default)]
    pub adopt_start: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentActivity {
    pub id: String,
    pub description: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeActivity {
    pub phase: String,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_progress_at: Option<DateTime<Utc>>,
    pub pending_tasks: u32,
    #[serde(default)]
    pub subagents: Vec<SubagentActivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_reason: Option<String>,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub attempts: Option<u32>,
    #[serde(default)]
    pub sessions: Vec<String>,
    #[serde(default)]
    pub review_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<NodeActivity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
