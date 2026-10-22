//! Goal Loops.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/goal-loops.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Goal Loops
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/goal-loops/define` — run the AI goal-definer to
/// turn a rough seed into a structured draft. Creates a managed definer session,
/// but does not create a loop until the user launches it.
/// Supplying `feedback` (with the prior draft echoed in `context`) refines it.
#[derive(Debug, Clone, Deserialize)]
pub struct DefineGoalReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    /// The rough goal text the user typed.
    pub seed: String,
    /// The repo the loop will work in (gives the definer codebase context).
    pub repo_path: String,
    /// Optional extra guidance, or the prior draft when refining.
    #[serde(default)]
    pub context: Option<String>,
    /// When refining: what to change about the prior draft.
    #[serde(default)]
    pub feedback: Option<String>,
}

/// Explicit human evidence; verifier identity is taken from authentication.
#[derive(Debug, Clone, Deserialize)]
pub struct VerifyGoalCriterionReq {
    pub evidence: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnswerGoalQuestionReq {
    pub answer: String,
}

/// The definer's structured suggestion. The user edits this before launching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalLoopDraft {
    pub definition: crate::domain::GoalLoopDefinition,
    pub suggested_limits: crate::domain::GoalLoopLimits,
    pub suggested_config: crate::domain::GoalLoopConfig,
}

/// `POST /api/v1/workspaces/{id}/goal-loops` — create a loop (optionally start it).
#[derive(Debug, Clone, Deserialize)]
pub struct CreateGoalLoopReq {
    pub name: String,
    pub repo_path: String,
    pub definition: crate::domain::GoalLoopDefinition,
    pub limits: crate::domain::GoalLoopLimits,
    pub config: crate::domain::GoalLoopConfig,
    /// When true, the controller starts immediately after creation.
    #[serde(default)]
    pub autostart: bool,
}

/// `PATCH /api/v1/goal-loops/{id}` — edit a loop. `config` is editable in Draft
/// only (reshaping executors mid-run breaks live agent indices); `limits` may be
/// raised while Paused/Blocked/Exhausted; `name` any non-terminal state.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateGoalLoopReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub limits: Option<crate::domain::GoalLoopLimits>,
    #[serde(default)]
    pub config: Option<crate::domain::GoalLoopConfig>,
}
