//! Request DTOs and composite responses for the swarm API. Row structs live in
//! `otto_state::swarm` and serve directly as response payloads.

use otto_core::Id;
use otto_state::{Swarm, SwarmAgent, SwarmProject};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// --- Swarms ----------------------------------------------------------------

/// Optional present-or-absent budget fields. `Some(None)` clears a limit
/// (unlimited); `None` (absent) leaves it untouched. Used by `UpdateSwarmReq`.
fn de_double_option<'de, D, T>(de: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Some(Option::<T>::deserialize(de)?))
}

#[derive(Debug, Deserialize)]
pub struct CreateSwarmReq {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub preset_slug: Option<String>,
    #[serde(default)]
    pub config: Option<Value>,
    /// Budget guardrails (all nullable = unlimited).
    #[serde(default)]
    pub max_total_runs: Option<i64>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub max_runtime_secs: Option<i64>,
    /// Per-task attempt ceiling (default 3 when omitted).
    #[serde(default)]
    pub max_attempts: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateSwarmReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub config: Option<Value>,
    // Budget guardrails. `null` in the body clears the limit; an absent key
    // leaves it untouched (double-Option).
    #[serde(default, deserialize_with = "de_double_option")]
    pub max_total_runs: Option<Option<i64>>,
    #[serde(default, deserialize_with = "de_double_option")]
    pub max_cost_usd: Option<Option<f64>>,
    #[serde(default, deserialize_with = "de_double_option")]
    pub max_runtime_secs: Option<Option<i64>>,
    #[serde(default)]
    pub max_attempts: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SwarmCounts {
    pub agents: usize,
    pub projects: usize,
    pub tasks: usize,
    pub running_runs: i64,
    /// Total runs ever enqueued (basis for the `max_total_runs` budget).
    pub total_runs: i64,
    /// Accumulated backfilled spend in USD (basis for the `max_cost_usd` budget).
    pub cost_usd: f64,
}

#[derive(Debug, Serialize)]
pub struct SwarmDetail {
    #[serde(flatten)]
    pub swarm: Swarm,
    pub agents: Vec<SwarmAgent>,
    pub projects: Vec<SwarmProject>,
    pub counts: SwarmCounts,
}

// --- Agents ----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateAgentReq {
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub reports_to: Option<Id>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub soul_name: Option<String>,
    #[serde(default)]
    pub soul_md: Option<String>,
    #[serde(default)]
    pub specialization: Option<String>,
    #[serde(default)]
    pub scope_md: Option<String>,
    #[serde(default)]
    pub skills: Option<Value>,
    #[serde(default)]
    pub schedule: Option<Value>,
    #[serde(default)]
    pub cwd_mode: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub order_idx: Option<i64>,
}

/// PATCH agent. Present fields are applied; nullable fields are set (not cleared)
/// — to clear a schedule, send `{"schedule": {...,"enabled": false}}`.
#[derive(Debug, Deserialize)]
pub struct UpdateAgentReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub reports_to: Option<Id>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub soul_name: Option<String>,
    #[serde(default)]
    pub soul_md: Option<String>,
    #[serde(default)]
    pub specialization: Option<String>,
    #[serde(default)]
    pub scope_md: Option<String>,
    #[serde(default)]
    pub skills: Option<Value>,
    #[serde(default)]
    pub schedule: Option<Value>,
    #[serde(default)]
    pub cwd_mode: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub order_idx: Option<i64>,
}

// --- Projects --------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateProjectReq {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repo_path: Option<String>,
    #[serde(default)]
    pub goal_md: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub repo_path: Option<String>,
    #[serde(default)]
    pub goal_md: Option<String>,
    /// Project-level skill set (`[{name, must_use?}]` or `["name", …]`), layered
    /// on team + per-agent skills.
    #[serde(default)]
    pub skills: Option<serde_json::Value>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub order_idx: Option<i64>,
}

// --- Tasks -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateTaskReq {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub assignee_agent_id: Option<Id>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub depends_on: Option<Value>,
    #[serde(default)]
    pub labels: Option<Value>,
    #[serde(default)]
    pub order_idx: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskReq {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub assignee_agent_id: Option<Id>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub depends_on: Option<Value>,
    #[serde(default)]
    pub labels: Option<Value>,
    #[serde(default)]
    pub order_idx: Option<i64>,
}

// --- Board -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PostMessageReq {
    #[serde(default)]
    pub project_id: Option<Id>,
    #[serde(default)]
    pub task_id: Option<Id>,
    #[serde(default)]
    pub to_agent_id: Option<Id>,
    #[serde(default)]
    pub kind: Option<String>,
    pub body: String,
}

// --- Recruiter / planner (runtime, handled in otto-server; DTOs here) -------

#[derive(Debug, Deserialize)]
pub struct RecruitReq {
    #[serde(default)]
    pub swarm_id: Option<Id>,
    pub role: String,
    #[serde(default)]
    pub context: Option<String>,
    /// Optional naming theme (e.g. "famous footballers") — the recruiter derives
    /// the agent's name from it for a cohesive roster.
    #[serde(default)]
    pub naming_theme: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitedSkill {
    pub name: String,
    #[serde(default)]
    pub must_use: bool,
    #[serde(default)]
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitedAgent {
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub reports_to_title: Option<String>,
    pub specialization: String,
    pub soul_md: String,
    pub scope_md: String,
    pub skills: Vec<RecruitedSkill>,
    pub suggested_provider: String,
    #[serde(default)]
    pub suggested_model: Option<String>,
    #[serde(default)]
    pub suggested_schedule: Option<Value>,
    #[serde(default)]
    pub avatar: String,
}

#[derive(Debug, Deserialize)]
pub struct PlanReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RunTaskReq {}

// --- Run graph -------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct GraphNode {
    pub id: String,
    pub kind: String,   // task | run
    pub label: String,
    pub status: String,
    pub agent_id: Option<Id>,
    pub session_id: Option<Id>,
    pub project_id: Option<Id>,
}

#[derive(Debug, Serialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String,   // depends | handoff | review
}

#[derive(Debug, Serialize)]
pub struct SwarmGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

// --- Presets ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct PresetAgent {
    pub key: String,
    pub name: String,
    pub title: String,
    #[serde(default)]
    pub reports_to: Option<String>,
    pub provider: String,
    #[serde(default)]
    pub specialization: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SwarmPreset {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub max_parallel_sessions: i64,
    /// Budget guardrails for swarms created from this preset (None = unlimited).
    pub max_total_runs: Option<i64>,
    pub max_runtime_secs: Option<i64>,
    pub max_cost_usd: Option<f64>,
    pub max_attempts: Option<i64>,
    pub agents: Vec<PresetAgent>,
}

// ---------------------------------------------------------------------------
// Product↔Swarm closure response  (`GET /swarm/tasks/{tid}/story`)
// ---------------------------------------------------------------------------

/// Back-link from a swarm task to the Product story that originated it (if any).
/// Returned by `GET /swarm/tasks/{tid}/story`.  Read-only; no migration needed.
#[derive(Debug, Clone, Serialize)]
pub struct TaskStoryLink {
    /// The source Product story, when the task's project was created from a
    /// story's implementation plan (Plan → Swarm hand-off).  `None` when the
    /// task was created directly in the swarm UI.
    pub story: Option<otto_state::ProductStory>,
    /// Acceptance criteria extracted from the task `description` field.  For
    /// tasks generated from a story plan this often contains structured ACs.
    /// Surfaced here as a convenience so the task view can show them directly.
    pub acceptance: Option<String>,
}
