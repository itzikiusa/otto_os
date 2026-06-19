//! Collection ingestion: chunk source/docs into `chunk` records, and import a
//! graphify `graph.json` (nodes → entity memories, edges → links). graphify runs
//! as a skill-invoked CLI that emits `graph.json`; this is the import side.

use serde::{Deserialize, Serialize};

use crate::types::{kind, source, MemoryRef, NewMemory, Scope};

/// Split text into overlapping line-window chunks. Symbol-aware (tree-sitter)
/// chunking is a later upgrade; line windows are robust and language-agnostic.
pub fn chunk_text(
    collection: &str,
    path: &str,
    content: &str,
    window: usize,
    overlap: usize,
) -> Vec<NewMemory> {
    let lines: Vec<&str> = content.lines().collect();
    let window = window.max(1);
    let step = window.saturating_sub(overlap).max(1);
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + window).min(lines.len());
        let body = lines[start..end].join("\n");
        if !body.trim().is_empty() {
            out.push(NewMemory {
                collection: collection.into(),
                record_type: "chunk".into(),
                visibility: "shared".into(),
                scope: Scope::Workspace,
                story_id: None,
                kind: kind::CHUNK.into(),
                title: format!("{path}:{}-{}", start + 1, end),
                body,
                entities: vec![],
                tags: vec![],
                source_kind: source::CODE.into(),
                source_ref: Some(format!("{path}#{}", start + 1)),
                refs: vec![MemoryRef {
                    kind: "file".into(),
                    reference: path.into(),
                    url: None,
                    label: None,
                }],
                confidence: Some(0.5),
                salience: Some(0.4),
            });
        }
        if end >= lines.len() {
            break;
        }
        start += step;
    }
    out
}

// --- graphify graph.json import ---

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphifyNode {
    pub id: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphifyEdge {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub rel: Option<String>,
    /// graphify tags each edge EXTRACTED | INFERRED | AMBIGUOUS.
    #[serde(default)]
    pub certainty: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphifyGraph {
    #[serde(default)]
    pub nodes: Vec<GraphifyNode>,
    #[serde(default)]
    pub edges: Vec<GraphifyEdge>,
}

/// A graphify node → an `entity` memory (source_ref = node id, so edges can be
/// resolved to the created rows afterward).
pub fn node_to_memory(collection: &str, n: &GraphifyNode) -> NewMemory {
    let title = n.label.clone().unwrap_or_else(|| n.id.clone());
    let body = n
        .summary
        .clone()
        .unwrap_or_else(|| title.clone());
    NewMemory {
        collection: collection.into(),
        record_type: "item".into(),
        visibility: "shared".into(),
        scope: Scope::Entity,
        story_id: None,
        kind: kind::ENTITY.into(),
        title,
        body,
        entities: vec![],
        tags: n.kind.clone().into_iter().collect(),
        source_kind: "graphify".into(),
        source_ref: Some(n.id.clone()),
        refs: n
            .file
            .clone()
            .map(|f| {
                vec![MemoryRef {
                    kind: "file".into(),
                    reference: f,
                    url: None,
                    label: None,
                }]
            })
            .unwrap_or_default(),
        confidence: Some(0.6),
        salience: Some(0.5),
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportStats {
    pub nodes: usize,
    pub edges: usize,
}
