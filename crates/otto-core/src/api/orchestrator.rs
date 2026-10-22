//! Orchestrator.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/orchestrator.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/orchestrate`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrateReq {
    pub text: String,
    #[serde(default)]
    pub optimize: bool,
    #[serde(default)]
    pub ai_fallback: bool,
    /// Session that currently has focus in the UI (fallback target).
    pub focused_session_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrateResp {
    pub plan: ActionPlan,
    /// The optimized prompt, when `optimize` was requested.
    pub optimized_text: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/orchestrate/execute`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutePlanReq {
    pub plan: ActionPlan,
}

pub type ActionPlan = Vec<Action>;

/// A single orchestrator action, produced by parsing plain English.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Spawn `count` agent sessions of `provider`.
    SpawnSessions { provider: String, count: u8 },
    /// Send `text` to every running agent session in the workspace.
    Broadcast { text: String },
    /// Open a saved connection as a new session.
    OpenConnection { connection_id: Id },
    /// Send `text` to one specific session.
    RunCommand { session_id: Id, text: String },
}
