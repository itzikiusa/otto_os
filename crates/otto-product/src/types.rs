//! Request DTOs and response types for the Product Story Analysis feature.
//! These live here, not in otto-core, as they are feature-specific.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use otto_state::{
    ProductAnalysis, ProductAnalysisAgent, ProductStory, ProductStoryVersion, ProductTestcase,
    ProductTestcaseRun,
};

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ImportStoryReq {
    pub source_kind: String,
    pub account_id: String,
    pub source_key: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub watch_enabled: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateStoryReq {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub watch_enabled: Option<bool>,
    #[serde(default)]
    pub watch_cadence_min: Option<i64>,
    #[serde(default)]
    pub tags: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewQuestionReq {
    pub text: String,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateQuestionReq {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub answer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PostQuestionsReq {
    pub ids: Vec<String>,
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NewNoteReq {
    pub body: String,
    #[serde(default)]
    pub section: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNoteReq {
    pub body: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct PublishVersionReq {}

#[derive(Debug, Deserialize)]
pub struct NewLearningReq {
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub refs: Option<Value>,
    #[serde(default)]
    pub source_story_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateLearningReq {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub refs: Option<Value>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateTestcaseReq {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub steps: Option<Value>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub review_note: Option<String>,
    #[serde(default)]
    pub order_idx: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PublishTestsReq {
    #[serde(default)]
    pub space_key: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AnalyzeReq {
    #[serde(default)]
    pub agents: Vec<AnalyzeAgentReq>,
    /// Provider for the final consolidating summarizer agent (defaults to the
    /// workspace/global default provider when omitted).
    #[serde(default)]
    pub summarizer_provider: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional instruction all analysis agents should pay special attention to.
    /// Written near the top of the context file so agents prioritize it.
    #[serde(default)]
    pub focus: Option<String>,
}

/// One analysis lens. It runs once per provider in `providers` (each as its own
/// real, openable session), so a single lens can be analyzed by claude AND
/// codex AND agy. Empty `providers` falls back to the default provider.
#[derive(Debug, Deserialize)]
pub struct AnalyzeAgentReq {
    pub skill: String,
    pub name: Option<String>,
    #[serde(default)]
    pub providers: Vec<String>,
    pub model: Option<String>,
}

/// Request body for the `POST …/rewrite` endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct RewriteReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional instruction the writer agent should pay special attention to.
    #[serde(default)]
    pub focus: Option<String>,
}

/// Request body for the `POST …/testcases/generate` endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct GenerateTestsReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional instruction the test-generation agent should pay special attention to.
    #[serde(default)]
    pub focus: Option<String>,
}

/// Request body for the `POST …/plan/generate` endpoint.
#[derive(Debug, Default, Deserialize)]
pub struct GeneratePlanReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Optional instruction the planning agent should pay special attention to.
    #[serde(default)]
    pub focus: Option<String>,
}

/// Request body for the `POST …/plan` endpoint. Persists PO checkbox toggles by
/// overwriting the latest `kind="plan"` version's body in place (no new version).
#[derive(Debug, Deserialize)]
pub struct SavePlanReq {
    pub body_md: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct NewDraftReq {
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDraftReq {
    pub title: String,
    pub body_md: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct NewTranscriptReq {
    #[serde(default)]
    pub title: Option<String>,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishAsRfcReq {
    pub account_id: String,
    pub space_key: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PublishAsStoryReq {
    pub account_id: String,
    pub project_key: String,
    pub issue_type: String,
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Detailed story view: the story itself + its latest source version + counts.
#[derive(Debug, Clone, Serialize)]
pub struct ProductStoryDetail {
    pub story: ProductStory,
    pub source: Option<ProductStoryVersion>,
    pub counts: StoryCounts,
}

/// Aggregate counts for a story's child collections.
#[derive(Debug, Clone, Serialize)]
pub struct StoryCounts {
    pub versions: i64,
    pub analyses: i64,
    pub open_questions: i64,
    pub notes: i64,
    pub testcases: i64,
}

/// Detailed analysis view: the analysis itself + all its agents.
#[derive(Debug, Clone, Serialize)]
pub struct ProductAnalysisDetail {
    pub analysis: ProductAnalysis,
    pub agents: Vec<ProductAnalysisAgent>,
}

/// Detailed testcase-run view: one run bundled with all its test cases.
#[derive(Debug, Clone, Serialize)]
pub struct ProductTestcaseRunDetail {
    pub run: ProductTestcaseRun,
    pub cases: Vec<ProductTestcase>,
}

/// One named section of an inject bundle.
#[derive(Debug, Clone, Serialize)]
pub struct InjectSection {
    pub heading: String,
    pub body: String,
}

/// Consolidated context bundle for agent injection.
#[derive(Debug, Clone, Serialize)]
pub struct InjectBundle {
    pub markdown: String,
    pub sections: Vec<InjectSection>,
}

/// Request body for `POST …/inject-session`.
#[derive(Debug, Default, Deserialize)]
pub struct InjectSessionReq {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}
