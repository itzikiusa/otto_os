//! Skills Evaluator; Eval lab — golden tasks, rating, regression, matrices.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/skills-eval.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Skills Evaluator
// ---------------------------------------------------------------------------

/// One validation the evaluator runs against the produced implementation. Each
/// validation fans out to one agent per entry in `providers` (so a single
/// validation can be cross-checked by several CLIs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEvalValidationCfg {
    /// Short identifier, e.g. "logs", "docs", "naming".
    pub name: String,
    /// What this validation checks and how to judge it. Passed to the agent.
    pub criteria: String,
    /// CLIs to run this validation on (one agent each). Empty falls back to the
    /// run's implementation CLI.
    #[serde(default)]
    pub providers: Vec<String>,
    /// Model hint ("haiku" | "sonnet" | "opus" | ""). Empty = provider default.
    #[serde(default)]
    pub model: String,
}

/// Config for the agent that edits/improves the skill between iterations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillEvalImproverCfg {
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

/// Persisted defaults for the Skills Evaluator (settings key `skill_eval`).
/// `GET /api/v1/settings/skill-eval` and `PUT /api/v1/settings/skill-eval`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillEvalConfig {
    /// Default validations offered when starting a new evaluation.
    pub validations: Vec<SkillEvalValidationCfg>,
    /// Default improver agent.
    pub improver: SkillEvalImproverCfg,
    /// Default number of iterations.
    #[serde(default = "default_iterations")]
    pub iterations: u32,
    /// Default validation passes (averaged) — see `StartSkillEvalReq`.
    #[serde(default = "default_validator_passes")]
    pub validator_passes: u32,
    /// Default composite-score weights.
    #[serde(default)]
    pub weights: ScoreWeights,
    /// Minimum composite score (0–100) required to promote a skill.
    #[serde(default = "default_promote_min_score")]
    pub promote_min_score: f64,
    /// Whether promotion also requires the iteration's proof pack to pass.
    #[serde(default = "default_true")]
    pub require_proof_pass: bool,
    /// Fallback test/lint commands when a golden task / run doesn't set them.
    #[serde(default)]
    pub default_test_cmd: String,
    #[serde(default)]
    pub default_lint_cmd: String,
}

pub(super) fn default_iterations() -> u32 {
    2
}

pub(super) fn default_promote_min_score() -> f64 {
    80.0
}

pub(super) fn default_true() -> bool {
    true
}

/// Where a `score_only` run (or matrix cell) finds the code to score.
/// - `kind="working"`: the workspace repo's live working tree (diff vs HEAD).
/// - `kind="branch"`: a disposable worktree at `git_ref` (diff `base_ref..git_ref`).
/// - `kind="path"`: an existing directory at `path` (diff working vs HEAD/base).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalTarget {
    pub kind: String,
    #[serde(default)]
    pub git_ref: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

/// Where the skill under test comes from.
/// - `kind = "library"`: `reference` is the Otto library skill name.
/// - `kind = "path"`: `reference` is an absolute path to a skill folder, a
///   `SKILL.md`/`.md` file, or a `.zip`/`.gz`/`.tgz` archive containing one.
/// - `kind = "provider"`: `reference` is a skill name under
///   `~/.<provider>/skills/<name>/SKILL.md` (provider in `provider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceReq {
    pub kind: String,
    pub reference: String,
    #[serde(default)]
    pub provider: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/skill-evaluations` — start an evaluation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartSkillEvalReq {
    pub source: SkillSourceReq,
    /// The task to implement using the skill (e.g. "add endpoint X").
    pub task: String,
    /// The single CLI that implements the task.
    pub impl_cli: String,
    /// Validations to run after each implementation.
    pub validations: Vec<SkillEvalValidationCfg>,
    /// Total iterations (rounds). >= 1. Round 1 is the baseline; each later
    /// round improves the skill and re-runs.
    pub iterations: u32,
    /// Agent that edits the skill between iterations (defaults to the impl CLI).
    #[serde(default)]
    pub improver: Option<SkillEvalImproverCfg>,
    /// Git ref to create each iteration's worktree from (defaults to HEAD).
    #[serde(default)]
    pub base_ref: Option<String>,
    /// How many times to run each validation and average — higher reduces the
    /// noise from nondeterministic graders. 1–3, defaults to 1.
    #[serde(default = "default_validator_passes")]
    pub validator_passes: u32,
    /// "generate" (default — agent implements the task) | "score_only" (no agent —
    /// score the `target` as-is). `impl_cli` is only required for "generate".
    #[serde(default)]
    pub mode: String,
    /// Golden task this run evaluates (supplies prompt + test/lint/rubric).
    #[serde(default)]
    pub golden_task_id: Option<String>,
    /// For `score_only`: what to score.
    #[serde(default)]
    pub target: Option<EvalTarget>,
    /// Repo-specific test command to run as a scoring signal (overrides golden /
    /// config default).
    #[serde(default)]
    pub test_cmd: Option<String>,
    /// Repo-specific lint command (overrides golden / config default).
    #[serde(default)]
    pub lint_cmd: Option<String>,
    /// Composite-score weights (overrides config default).
    #[serde(default)]
    pub weights: Option<ScoreWeights>,
}

pub(super) fn default_validator_passes() -> u32 {
    1
}

/// `POST /api/v1/skill-evaluations/{id}/promote` — save an iteration's skill
/// back into the Otto library under `name`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteSkillReq {
    pub iteration_id: Id,
    /// "tested" = the skill that iteration ran with; "improved" = the edited
    /// version it produced for the next round.
    pub source: String,
    /// Target library skill name (safe segment).
    pub name: String,
    /// Bypass the score+proof promote gate (root only; records an audit + proof
    /// waive). Default false.
    #[serde(default)]
    pub force: bool,
}

/// `GET /api/v1/skill-evaluations/{id}/iterations/{iter_id}/diff` — the code the
/// implementation agent produced in that iteration's worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplDiffResp {
    pub diff: String,
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// Eval lab — golden tasks, rating, regression, matrices
// ---------------------------------------------------------------------------

/// Create/update a golden task. `POST /workspaces/{id}/golden-tasks`,
/// `PUT /golden-tasks/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenTaskReq {
    pub name: String,
    pub prompt: String,
    #[serde(default)]
    pub skill: String,
    #[serde(default)]
    pub test_cmd: String,
    #[serde(default)]
    pub lint_cmd: String,
    #[serde(default)]
    pub build_cmd: String,
    #[serde(default)]
    pub rubric: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Repo this task belongs to (registered repo id, else workspace id). Defaults
    /// to the workspace id when absent.
    #[serde(default)]
    pub repo_key: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// `POST /golden-tasks/{id}/run` — start an eval from a golden task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunGoldenReq {
    /// "generate" | "score_only" (default).
    #[serde(default)]
    pub mode: String,
    /// Implementation CLI for generate mode.
    #[serde(default)]
    pub provider: Option<String>,
    /// What to score (score_only).
    #[serde(default)]
    pub target: Option<EvalTarget>,
}

/// `POST /skill-evaluations/{id}/iterations/{iter_id}/rate` — record a human rating.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateIterationReq {
    /// 0–5.
    pub rating: u8,
    #[serde(default)]
    pub note: String,
}

/// `POST /skill-evaluations/{id}/iterations/{iter_id}/regression` — capture a
/// (failed) iteration as a regression golden task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegressionReq {
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /workspaces/{id}/eval-matrices` — start a provider × skill × prompt run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMatrixReq {
    pub name: String,
    /// "generate" | "score_only".
    #[serde(default)]
    pub mode: String,
    pub providers: Vec<String>,
    pub skills: Vec<SkillSourceReq>,
    pub prompts: Vec<MatrixPrompt>,
    #[serde(default)]
    pub target: Option<EvalTarget>,
    #[serde(default)]
    pub test_cmd: Option<String>,
    #[serde(default)]
    pub lint_cmd: Option<String>,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub weights: Option<ScoreWeights>,
    #[serde(default)]
    pub validations: Vec<SkillEvalValidationCfg>,
    #[serde(default = "default_iterations")]
    pub iterations: u32,
}

/// A discoverable skill source the UI can offer in the start form.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourceInfo {
    /// "library" | "provider".
    pub kind: String,
    /// Skill name.
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Set for `kind = "provider"` (claude/codex/agy).
    #[serde(default)]
    pub provider: Option<String>,
}

/// `GET /api/v1/workspaces/{id}/skill-sources` — skills the user can pick from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSourcesResp {
    pub sources: Vec<SkillSourceInfo>,
}
