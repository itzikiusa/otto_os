//! PR Review config; PR Review start request; Local review.
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/review.ts`.
use super::*;

// ---------------------------------------------------------------------------
// PR Review config
// ---------------------------------------------------------------------------

/// Persisted configuration for the PR review pipeline.
/// Stored in the `settings` table under the key `pr_review`.
/// `GET /api/v1/settings/pr-review` and `PUT /api/v1/settings/pr-review`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfig {
    pub agents: Vec<ReviewAgentCfg>,
    pub summarizer: ReviewAgentCfg,
    /// User-defined reusable reviewer presets the UI offers alongside the
    /// built-in ones. Persisted with the config; not used by the runner.
    #[serde(default)]
    pub custom_presets: Vec<ReviewAgentCfg>,
    /// Maximum total attempts per agent (initial + retries). `None` → default 3.
    #[serde(default)]
    pub max_attempts: Option<u32>,
    /// Per-agent timeout in seconds. When set, overrides the diff-size heuristic.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Execution mode (`fan_out` | `orchestrator`). `None` ⇒ fan-out; kept
    /// as an Option so the engine can tell "stored" from "defaulted".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<crate::domain::ReviewMode>,
}

/// A named, reusable full review configuration ("conf A" / "conf Y").
/// Stored as a list in the `settings` table under the key `pr_review_presets`.
/// `GET /api/v1/settings/pr-review/presets` and `PUT /api/v1/settings/pr-review/presets`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConfigPreset {
    pub id: String,
    pub name: String,
    pub config: ReviewConfig,
}

/// Per-repo review-config binding, stored in the `settings` table under the
/// key `pr_review_repo:<repo_id>`. Exactly one of `preset_id` / `config` is
/// set: a preset reference tracks later edits to the preset; an inline config
/// is fully custom to the repo. No stored binding = the repo follows the
/// global `pr_review` config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepoReviewBinding {
    /// Reference into `pr_review_presets` (by preset `id`).
    #[serde(default)]
    pub preset_id: Option<String>,
    /// Inline repo-specific config (wins over `preset_id` if both are set).
    #[serde(default)]
    pub config: Option<ReviewConfig>,
}

/// `GET /api/v1/repos/{id}/review-config` — the repo's binding plus the
/// EFFECTIVE config reviews of this repo will run with (after preset / global
/// resolution), so the UI can render both the choice and its result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoReviewConfigResp {
    /// "global" | "preset" | "custom".
    pub scope: String,
    /// Set when `scope == "preset"`.
    #[serde(default)]
    pub preset_id: Option<String>,
    /// Preset display name (resolved; `None` if the preset was deleted).
    #[serde(default)]
    pub preset_name: Option<String>,
    /// The effective config after resolution.
    pub config: ReviewConfig,
}

// ---------------------------------------------------------------------------
// PR Review start request
// ---------------------------------------------------------------------------

/// `POST /api/v1/repos/{id}/prs/{number}/review` — optional request body.
/// All fields default to `None` so an empty (or absent) body still parses.
#[derive(Debug, Default, Deserialize)]
pub struct StartReviewReq {
    #[serde(default)]
    pub issue_account_id: Option<String>,
    #[serde(default)]
    pub issue_key: Option<String>,
    /// Free-text guidance from the user (e.g. "what to focus on"). Passed to the
    /// review agents alongside the diff. Empty/absent behaves as before.
    #[serde(default)]
    pub context: Option<String>,
}

// ---------------------------------------------------------------------------
// Local review
// ---------------------------------------------------------------------------

/// `POST /api/v1/repos/{id}/local-review` — start a review of the local
/// working tree against a base branch/ref.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalReviewReq {
    /// The base git ref to diff against (e.g. `origin/develop`, `main`).
    pub base: String,
}

/// `POST /api/v1/reviews/{id}/handoff` — hand review findings to a new agent
/// session so the agent can fix them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffReq {
    /// The provider to spawn (e.g. "claude", "codex", "agy").
    pub provider: String,
    /// Optional list of comment ids to include. When `None`, all non-declined
    /// comments are included. When `Some`, only the listed comments are sent.
    #[serde(default)]
    pub comment_ids: Option<Vec<String>>,
}

/// Where a handover brief is delivered: a freshly spawned agent, or an existing
/// running agent in the same workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoverTarget {
    /// Spawn a new agent of `provider` ("claude" | "codex" | "agy" | …).
    NewAgent { provider: String },
    /// Inject into an existing agent session already in this workspace.
    ExistingSession { session_id: Id },
}

/// `POST /api/v1/sessions/{id}/handover` — pass the source agent's working
/// context (summarized best-effort, optionally with git state) into the target
/// agent, so it can continue the work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverReq {
    /// Where to deliver the brief.
    pub target: HandoverTarget,
    /// Free-text note describing what the receiving agent should focus on. The
    /// generated brief is weighted toward this.
    #[serde(default)]
    pub focus: Option<String>,
    /// Title for the new session (NewAgent only). Defaults to "Handover from <source>".
    #[serde(default)]
    pub title: Option<String>,
    /// A pre-generated/edited brief. When present, the server skips
    /// summarization and injects this verbatim (the "review before sending" flow).
    #[serde(default)]
    pub brief: Option<String>,
    /// Include the repo's git state (branch, changed files, recent commits) in
    /// the generated brief. Ignored when `brief` is supplied. Defaults to true.
    #[serde(default)]
    pub include_git: Option<bool>,
    /// Summarize with a fast model (haiku) instead of the default. Ignored when
    /// `brief` is supplied. Defaults to false.
    #[serde(default)]
    pub fast: Option<bool>,
    /// Archive the source only after explicit receipt acknowledgement on the target. Defaults to false.
    #[serde(default)]
    pub archive_source: Option<bool>,
}

/// `POST /api/v1/sessions/{id}/handover/brief` — generate the handover brief
/// (synchronously) so the user can review/edit it before sending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverBriefReq {
    #[serde(default)]
    pub focus: Option<String>,
    #[serde(default)]
    pub include_git: Option<bool>,
    #[serde(default)]
    pub fast: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoverBriefResp {
    /// The generated brief (markdown). Empty when there was no context at all.
    pub brief: String,
    /// True when summarization was unavailable and `brief` is raw context.
    pub fallback: bool,
    /// True when some source context (transcript/scrollback/git) was found.
    pub had_context: bool,
}
