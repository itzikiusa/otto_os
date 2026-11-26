//! Goals: a goal is a workflow the server builds from a short [`GoalSpec`]
//! (Start → loop { Work → Check criteria }), started through
//! `POST /workspaces/{wid}/goals`. The run-side shapes (criteria, holds,
//! progress) live in `otto_core::workflows`.

use serde::{Deserialize, Serialize};

use crate::workflows::{Criterion, Workflow, WorkflowRun};
use crate::Id;

/// `POST /workspaces/{wid}/goals` (and `POST /goal-loops/{id}/continue`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalSpec {
    /// 1..=120 chars.
    pub title: String,
    /// The outcome in words, 1..=8,000 chars.
    pub goal: String,
    #[serde(default)]
    pub objectives: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    /// 1..=40; human criteria allowed.
    pub criteria: Vec<Criterion>,
    pub work: GoalWork,
    #[serde(default)]
    pub budget: GoalBudget,
    #[serde(default)]
    pub worker: AgentChoice,
    /// MCP executors after the first; not in the form.
    #[serde(default)]
    pub extra_workers: Vec<NamedAgentChoice>,
    #[serde(default)]
    pub checker: AgentChoice,
    #[serde(default)]
    pub final_review: bool,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    /// Continue an older goal loop as a goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continue_loop_id: Option<Id>,
    /// `insights` when the Insights handoff opened the form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// Where a goal's work happens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GoalWork {
    /// A run worktree of a registered repository, from `base` (default: the
    /// checkout's current commit).
    Build {
        repo_id: Id,
        #[serde(default)]
        base: Option<String>,
    },
    /// A research folder in the run's context directory.
    Research,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoalBudget {
    /// 1..=50.
    pub iterations: u32,
    /// Active seconds, 900..=28,800.
    pub active_s: u64,
    /// One step's limit, 300..=7,200.
    pub step_timeout_s: u64,
}

impl Default for GoalBudget {
    fn default() -> Self {
        Self {
            iterations: 5,
            active_s: 3_600,
            step_timeout_s: 1_200,
        }
    }
}

/// Provider / model / effort for a goal's worker or checker; empty = the run default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentChoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// An extra worker (MCP `executors[1..]`), named.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NamedAgentChoice {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// What starting a goal returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalStartResp {
    pub workflow: Workflow,
    pub run: WorkflowRun,
}

/// `POST /workflow-runs/{id}/resume`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResumeReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extend: Option<ExtendReq>,
}

/// New totals for the loop that ran out (not increments).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtendReq {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_s: Option<u64>,
}

/// `POST /workflow-runs/{id}/steer`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SteerReq {
    /// Trimmed, ≤ 4,000 chars.
    pub note: String,
}

/// `POST /workflow-runs/{id}/nodes/{node_id}/criteria/{criterion_id}/verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyCriterionReq {
    pub met: bool,
    /// Non-empty, ≤ 2,000 chars.
    pub evidence: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_minimal_spec_takes_the_default_budget() {
        let spec: GoalSpec = serde_json::from_value(json!({
            "title": "t", "goal": "g",
            "criteria": [{"id": "c1", "text": "tests pass", "verify": "command", "command": "make test"}],
            "work": {"kind": "build", "repo_id": "r1"}
        }))
        .unwrap();
        assert_eq!(spec.budget, GoalBudget::default());
        assert_eq!(
            spec.work,
            GoalWork::Build {
                repo_id: "r1".into(),
                base: None
            }
        );
        let research: GoalWork = serde_json::from_value(json!({"kind": "research"})).unwrap();
        assert_eq!(research, GoalWork::Research);
        let partial: GoalBudget = serde_json::from_value(json!({"iterations": 9})).unwrap();
        assert_eq!((partial.iterations, partial.active_s), (9, 3_600));
    }
}
