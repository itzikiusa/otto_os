//! The Product Story runners: analysis, rewrite, test-case and plan
//! generation, each driving real, openable agent sessions through the host's
//! [`ProductRunHost`].
//!
//! - [`run_analysis`] (`analysis`) fans one agent per (lens × provider) out on
//!   the shared runner (`otto_agent_run::fanout`, like code review, skill review
//!   and vault docs), then ONE summarizer agent consolidates / dedupes /
//!   resolves conflicts across the lens outputs; a Rust-side merge of the lens
//!   findings stands in when it fails. [`retry_analysis_agent`] (`retry`)
//!   re-runs one lens; [`reap_orphaned_agents_on_startup`] (`reap`) resumes
//!   lenses a restart stranded.
//! - [`run_generate_plan`] (`plan`) fans one planner per provider out on the
//!   same runner, consolidating only when there is more than one candidate.
//! - [`run_rewrite`] (`rewrite`) and [`run_generate_tests`] (`tests_gen`) are
//!   single-agent turns.
//! - `prompts` holds every prompt and output contract, `context` the shared
//!   context-file / skill / attribution helpers, `scratch` the per-session
//!   cwds, `narrative` the skill self-improvement narratives.
//!
//! Provider-honoring: every provider (claude / codex / agy / …) is spawned as a
//! real session via the SessionManager mechanism. One agent's failure never
//! aborts the others. Sessions are NOT killed when done — they stay
//! live/openable so the PO can inspect each agent's terminal afterward.
//!
//! [`ProductRunHost`]: crate::host::ProductRunHost

use otto_core::cancel::CancelRegistryExt;
use otto_core::Id;

mod analysis;
mod context;
mod narrative;
mod plan;
mod prompts;
mod reap;
mod retry;
mod rewrite;
mod scratch;
mod tests_gen;

pub use analysis::run_analysis;
pub use narrative::{
    build_improve_narrative_from_clarifications, build_improve_narrative_from_tests,
};
pub use plan::run_generate_plan;
pub use prompts::{
    augment_with_out_path, build_analysis_prompt, build_plan_prompt, build_rewrite_prompt,
    build_summarizer_prompt, build_tests_prompt, extract_json_as, extract_json_block,
    SummaryFindings,
};
pub use reap::{
    is_summarizer_agent, reap_orphaned_agents_on_startup, INTERRUPTED_SUMMARY,
    SUMMARIZER_NAME_PREFIX,
};
pub use retry::retry_analysis_agent;
pub(crate) use retry::retry_analysis_agent_owned;
pub use rewrite::run_rewrite;
pub use scratch::{
    session_cwd, spawn_scratch_sweep, sweep_stale_scratch, trust_session_cwd, SCRATCH_MAX_AGE,
    SCRATCH_ROOT,
};
pub use tests_gen::run_generate_tests;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One analysis agent specification (provided by the 3.3 handler).
pub struct AgentSpec {
    pub provider: String,
    pub model: Option<String>,
    pub skill: String,
    pub name: String,
}

/// The JSON schema each analysis agent must respond with.
#[derive(serde::Deserialize, Default)]
pub struct Findings {
    pub summary: String,
    pub related_repos: Vec<String>,
    pub functionalities: Vec<String>,
    pub integration_points: Vec<String>,
    pub risks: Vec<String>,
    pub open_questions: Vec<FoundQuestion>,
    pub suggested_learnings: Vec<FoundLearning>,
}

#[derive(serde::Deserialize, Clone)]
pub struct FoundQuestion {
    pub text: String,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub category: String,
}

#[derive(serde::Deserialize, Clone)]
pub struct FoundLearning {
    pub kind: String,
    pub title: String,
    pub body: String,
}

// ---------------------------------------------------------------------------
// Session-based, provider-honoring lens runner (mirrors review_session)
// ---------------------------------------------------------------------------

/// Outcome of one lens (or the summarizer) running as a real session. This is the
/// caller-facing shape; the host's run mechanics return their own outcome type,
/// which [`ProductRunHost::run_agent_with_recovery`] flattens into this (keeping
/// `reason` as a stable `&str` for the existing notification/error-note code).
/// [`LensRunResult::reason`] when a newer run (a Retry after Stop) took the
/// agent row over mid-flight: the caller must skip every status write (S4-16).
pub const SUPERSEDED: &str = "superseded";

pub struct LensRunResult {
    /// Raw text the agent wrote to its out file (or the claude transcript turn).
    pub raw: Option<String>,
    /// The live SessionManager session id (so the agent stays openable).
    pub session_id: Option<Id>,
    /// True if the agent never produced output (timeout / exit / start failure).
    pub errored: bool,
    /// Short reason when `errored` ("stuck", "timeout", "exited", "session-gone",
    /// "create-failed", "stopped") — surfaced in notifications and the agent error
    /// field. `None` on success.
    pub reason: Option<&'static str>,
}

/// How a product agent session presents itself: its terminal title and the
/// `meta.source` tag. The `source` matters for UI visibility — analysis sessions
/// use `"product-analysis"`, which the workspace store FILTERS OUT of the Agents
/// list/grid (they're opened on demand inline). Plan sessions instead use
/// `"product-plan"`, which is NOT filtered, so they tile side-by-side in the
/// Agents view by default — exactly what the Plan flow wants (watch them live).
#[derive(Clone)]
pub struct SessionAppearance {
    pub title: String,
    pub source: &'static str,
}

impl SessionAppearance {
    /// The analysis default (hidden from the Agents grid; opened inline).
    pub fn analysis(provider: &str) -> Self {
        Self {
            title: format!("Analysis: {provider}"),
            source: "product-analysis",
        }
    }
}

/// Total attempts for an agent (initial + retries) before it's marked errored.
pub const MAX_AGENT_ATTEMPTS: u32 = 3;

// ---------------------------------------------------------------------------
// Per-agent cancellation registry (the shared `otto_core::cancel` registry)
// ---------------------------------------------------------------------------

/// Maps analysis-agent id → a cancel flag, so a manual Stop (or shutdown) can
/// signal an in-flight `run_agent_with_recovery` to abort without it being
/// mistaken for a failure (which would auto-retry).
pub use otto_core::cancel::{new_cancel_registry, CancelRegistry};

/// Trip the cancel flag for `agent_id` if it is registered (in-flight).
pub fn signal_cancel(reg: &CancelRegistry, agent_id: &str) {
    reg.signal(agent_id);
}

/// Cancel-registry key prefix of a story run WITHOUT an agent row (rewrite,
/// test generation, plan generation) and of a whole analysis or plan run:
/// `story:<sid>:<unique>` (S4-23).
const STORY_RUN_CANCEL_PREFIX: &str = "story:";

/// A fresh cancel key for one agent-row-less run of `story_id`, so deleting
/// the story can stop it ([`signal_story_cancels`]) instead of the recovery
/// loop respawning a killed session.
pub fn story_run_cancel_key(story_id: &str) -> String {
    format!(
        "{STORY_RUN_CANCEL_PREFIX}{story_id}:{}",
        uuid::Uuid::new_v4().simple()
    )
}

/// Trip every in-flight rewrite / test / plan run of `story_id`, and stop its
/// analysis and plan fan-outs before their summarizers (S4-23).
pub fn signal_story_cancels(reg: &CancelRegistry, story_id: &str) {
    reg.signal_prefix(&format!("{STORY_RUN_CANCEL_PREFIX}{story_id}:"));
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine
mod tests;

#[cfg(test)]
mod engine_tests;
