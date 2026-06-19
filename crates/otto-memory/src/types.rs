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
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryHit {
    pub memory: Memory,
    pub score: f32,
    #[serde(default)]
    pub why: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RecallOpts {
    pub focus: Option<String>,
    pub token_budget: usize,
    pub kinds: Vec<String>,
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
