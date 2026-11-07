//! Recoverable composer content, scoped to a person and runtime session.
use crate::agent_runtime::{PromptFile, RuntimeAccessMode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComposerContextKind {
    ReviewComment,
    Diff,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposerContext {
    pub id: String,
    pub kind: ComposerContextKind,
    pub label: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RuntimeDraftContent {
    pub text: String,
    pub images: Vec<PromptFile>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub access_mode: Option<RuntimeAccessMode>,
    pub fresh_context: bool,
    pub context: Vec<ComposerContext>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeDraftRecord {
    pub revision: i64,
    pub draft: Option<RuntimeDraftContent>,
}
#[derive(Debug, Deserialize)]
pub struct SaveRuntimeDraftReq {
    pub revision: i64,
    pub draft: RuntimeDraftContent,
}
