//! DTOs for the memory engine. The persistence structs come from
//! `otto_state::memory` (otto-state can't depend on us); the query/recall types
//! are defined here.

pub use otto_state::memory::{Memory, MemoryLink, MemoryPatch, MemoryRef, NewMemory, Scope};

use serde::{Deserialize, Serialize};

/// Memory `kind` taxonomy (semantic/episodic split, grounded in the product domain).
pub mod kind {
    pub const FACT: &str = "fact";
    pub const DECISION: &str = "decision";
    pub const REQUIREMENT: &str = "requirement";
    pub const CONSTRAINT: &str = "constraint";
    pub const QA: &str = "qa";
    pub const LEARNING: &str = "learning";
    pub const SUMMARY: &str = "summary";
    pub const ENTITY: &str = "entity";
    pub const SNAPSHOT: &str = "snapshot";
    pub const GLOSSARY: &str = "glossary";
    pub const CHUNK: &str = "chunk";
}

/// Provenance `source_kind` values.
pub mod source {
    pub const JIRA: &str = "jira";
    pub const CONFLUENCE: &str = "confluence";
    pub const ANALYSIS: &str = "analysis";
    pub const TRANSCRIPT: &str = "transcript";
    pub const QUESTION: &str = "question";
    pub const LEARNING: &str = "learning";
    pub const VERSION: &str = "version";
    pub const EVENT: &str = "event";
    pub const SESSION: &str = "session";
    pub const CODE: &str = "code";
    pub const MANUAL: &str = "manual";
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    #[default]
    Hybrid,
    Semantic,
    Keyword,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryQuery {
    pub text: Option<String>,
    pub collection: Option<String>,
    pub scope: Option<Scope>,
    pub story_id: Option<String>,
    pub kinds: Vec<String>,
    pub tags: Vec<String>,
    pub entities: Vec<String>,
    pub k: usize,
    pub mode: SearchMode,
    pub include_inactive: bool,
    pub recency_half_life_days: Option<f32>,
    /// Set server-side from the authenticated user (never from the client). When
    /// present, results exclude other users' `private` memories.
    #[serde(skip)]
    pub viewer: Option<String>,
}

/// A structured "why this was selected" reason — the explainability surface.
/// `kind` is one of: `vector` (semantic similarity), `keyword` (FTS/term match),
/// `symbol` (defines/uses a matched symbol), `graph` (reachable in the dependency
/// graph from the focus), `recent` (recently changed), `test` (a test for a
/// matched file), `doc` (a linked doc).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextReason {
    pub kind: String,
    pub detail: String,
    pub score: f32,
}

impl ContextReason {
    pub fn new(kind: &str, detail: impl Into<String>, score: f32) -> Self {
        Self {
            kind: kind.to_string(),
            detail: detail.into(),
            score,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryHit {
    pub memory: Memory,
    pub score: f32,
    /// Human-readable reasons (kept for back-compat — mirrors `reasons[].detail`).
    #[serde(default)]
    pub why: Vec<String>,
    /// Structured selection reasons (Vault v2 explainability).
    #[serde(default)]
    pub reasons: Vec<ContextReason>,
}

/// Outcome of indexing a repo into the Vault.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexResult {
    pub repo_id: String,
    pub files: usize,
    pub symbols: usize,
    pub edges: usize,
    pub chunks: usize,
}

/// A node in the unified Vault graph (knowledge + code), for the Obsidian-style
/// full graph view. `group` is `knowledge` or `code`; `kind` is the finer type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FullGraphNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub group: String,
    pub file: Option<String>,
    pub line: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FullGraphEdge {
    pub src: String,
    pub dst: String,
    pub rel: String,
    #[serde(default)]
    pub detail: String,
}

/// The whole Vault as one graph: memory links + code dependency edges.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FullGraph {
    pub nodes: Vec<FullGraphNode>,
    pub edges: Vec<FullGraphEdge>,
}

/// The assembled "Repo Brain" injected into an agent's context — sections of
/// recalled knowledge/code, each carrying its selection reasons.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RepoBrain {
    pub focus: String,
    pub sections: Vec<BriefSection>,
    pub reasons: Vec<ContextReason>,
    pub token_estimate: usize,
    /// Rendered markdown block (what gets injected into CONTEXT.md).
    pub markdown: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecallOpts {
    pub focus: Option<String>,
    pub token_budget: usize,
    pub kinds: Vec<String>,
    /// Server-set viewer id (see `MemoryQuery::viewer`).
    #[serde(skip)]
    pub viewer: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BriefSection {
    pub heading: String,
    pub body_md: String,
    pub refs: Vec<MemoryRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecallBrief {
    pub story_id: String,
    pub sections: Vec<BriefSection>,
    pub token_estimate: usize,
    pub used: Vec<String>,
}
