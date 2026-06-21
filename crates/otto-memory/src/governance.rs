//! Vault governance: lifecycle state transitions, soft-delete with undo tokens,
//! merge (N→1 with provenance), split (1→N with provenance), and governed import
//! of AGENTS.md / CLAUDE.md / .cursorrules text into tagged memories.

use otto_core::{new_id, Error, Result};
use otto_state::memory::Scope;
use otto_state::GovernedImport;

use crate::service::MemoryService;
use crate::types::{kind, Memory, NewMemory};

// ---------------------------------------------------------------------------
// Valid lifecycle states
// ---------------------------------------------------------------------------

/// The four valid lifecycle states for a memory.
pub const STATE_SUGGESTED: &str = "suggested";
pub const STATE_ACCEPTED: &str = "accepted";
pub const STATE_STALE: &str = "stale";
pub const STATE_CONTRADICTED: &str = "contradicted";

fn is_valid_state(s: &str) -> bool {
    matches!(s, "suggested" | "accepted" | "stale" | "contradicted")
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Request to set a memory's lifecycle state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SetStateReq {
    pub state: String,
}

/// Response from `forget`: carries the undo token the client must present to
/// restore the memory within the retention window.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ForgetResp {
    pub undo_token: String,
}

/// Request to undo a forget.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UndoForgetReq {
    pub undo_token: String,
}

/// Request to merge N memories into one. The `body` of the new memory is the
/// caller-supplied text; its kind, collection, and scope are inherited from the
/// first source memory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MergeReq {
    /// IDs of the source memories (2 or more).
    pub ids: Vec<String>,
    /// Title of the resulting merged memory.
    pub title: String,
    /// Body of the resulting merged memory.
    pub body: String,
}

/// Response from merge: the single resulting memory.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MergeResp {
    pub memory: Memory,
}

/// A single part in a split request.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SplitPart {
    pub title: String,
    pub body: String,
}

/// Request to split one memory into N parts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SplitReq {
    /// The parts to create (2 or more).
    pub parts: Vec<SplitPart>,
}

/// Response from split: the N child memories.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SplitResp {
    pub memories: Vec<Memory>,
}

/// Kind values for the import endpoint.
pub mod import_kind {
    pub const AGENTS_MD: &str = "agents-md";
    pub const CLAUDE_MD: &str = "claude-md";
    pub const CURSORRULES: &str = "cursorrules";
    pub const CUSTOM: &str = "custom";
}

/// Request to import a governance file into memories.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportReq {
    /// One of `agents-md`, `claude-md`, `cursorrules`, or `custom`.
    pub kind: String,
    /// Raw text content of the file to parse.
    pub content: String,
    /// Optional human-readable label (e.g. file path). Defaults to the kind.
    #[serde(default)]
    pub label: Option<String>,
}

/// Response from import: the number of memories created.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportResp {
    pub imported: usize,
    pub import_id: String,
}

// ---------------------------------------------------------------------------
// MemoryService governance extension
// ---------------------------------------------------------------------------

impl MemoryService {
    /// Transition a memory's lifecycle state. Only the four valid values are
    /// accepted; everything else is a 400 Bad Request.
    pub async fn set_state(&self, ws: &str, id: &str, state: &str) -> Result<Memory> {
        if !is_valid_state(state) {
            return Err(Error::Invalid(format!(
                "invalid state '{}'; must be one of suggested|accepted|stale|contradicted",
                state
            )));
        }
        self.repo().set_state(ws, id, state).await
    }

    /// Soft-delete a memory, returning an opaque `undo_token`. The memory is
    /// marked `active=0` and `state=stale`; it is NOT permanently removed.
    /// Presenting the token to `undo_forget` within the retention window restores it.
    pub async fn soft_forget(&self, ws: &str, id: &str) -> Result<ForgetResp> {
        let token = self.repo().soft_forget(ws, id).await?;
        Ok(ForgetResp { undo_token: token })
    }

    /// Undo a soft-delete using the token returned by `soft_forget`. Restores
    /// the memory to `active=1` and clears the token (single-use).
    pub async fn undo_forget(&self, ws: &str, undo_token: &str) -> Result<Memory> {
        self.repo().undo_forget(ws, undo_token).await
    }

    /// Merge N source memories into one. The resulting memory inherits the
    /// collection, scope, and story_id of the first source. Source memories are
    /// marked `contradicted`. Returns the new merged memory.
    pub async fn merge(&self, ws: &str, by: &str, req: MergeReq) -> Result<Memory> {
        if req.ids.len() < 2 {
            return Err(Error::Invalid(
                "merge requires at least 2 source memories".into(),
            ));
        }
        // Load the first source to inherit metadata.
        let first = self.repo().get(ws, &req.ids[0]).await?;

        let provenance = serde_json::json!({
            "op": "merge",
            "source_ids": req.ids,
        });
        let prov_str = provenance.to_string();

        // Create the merged memory as a new row.
        let new_mem = NewMemory {
            collection: first.collection.clone(),
            record_type: "item".into(),
            scope: first.scope,
            story_id: first.story_id.clone(),
            kind: first.kind.clone(),
            title: req.title,
            body: req.body,
            entities: vec![],
            tags: vec![],
            source_kind: "manual".into(),
            source_ref: None,
            refs: vec![],
            confidence: Some(first.confidence),
            salience: Some(first.salience),
            visibility: first.visibility.clone(),
        };
        let mut created = self.save(ws, by, vec![new_mem]).await?;
        let merged = created.remove(0);

        // Record provenance on the new row + mark sources contradicted.
        self.repo()
            .record_merge(ws, &merged.id, &req.ids, &prov_str)
            .await?;

        // Re-fetch to get the updated provenance_json column.
        self.repo().get(ws, &merged.id).await
    }

    /// Split one memory into N parts. The parent memory is marked
    /// `contradicted`; each child inherits the parent's collection/scope/story_id.
    pub async fn split(&self, ws: &str, by: &str, mid: &str, req: SplitReq) -> Result<SplitResp> {
        if req.parts.len() < 2 {
            return Err(Error::Invalid(
                "split requires at least 2 parts".into(),
            ));
        }
        let parent = self.repo().get(ws, mid).await?;

        let provenance = serde_json::json!({
            "op": "split",
            "parent_id": mid,
        });
        let prov_str = provenance.to_string();

        let mut new_mems = Vec::with_capacity(req.parts.len());
        for part in &req.parts {
            new_mems.push(NewMemory {
                collection: parent.collection.clone(),
                record_type: "item".into(),
                scope: parent.scope,
                story_id: parent.story_id.clone(),
                kind: parent.kind.clone(),
                title: part.title.clone(),
                body: part.body.clone(),
                entities: vec![],
                tags: vec![],
                source_kind: "manual".into(),
                source_ref: None,
                refs: vec![],
                confidence: Some(parent.confidence),
                salience: Some(parent.salience),
                visibility: parent.visibility.clone(),
            });
        }
        let children = self.save(ws, by, new_mems).await?;
        let child_ids: Vec<String> = children.iter().map(|m| m.id.clone()).collect();

        // Record provenance on each child + mark parent contradicted.
        self.repo()
            .record_split(ws, mid, &child_ids, &prov_str)
            .await?;

        // Re-fetch to get updated provenance columns.
        let mut refreshed = Vec::with_capacity(children.len());
        for id in &child_ids {
            refreshed.push(self.repo().get(ws, id).await?);
        }
        Ok(SplitResp { memories: refreshed })
    }

    /// Parse a governance file (AGENTS.md, CLAUDE.md, .cursorrules) into a set
    /// of `suggested` memories tagged with the import kind, and persist them as
    /// a governed-import batch for auditability + reverting.
    pub async fn import_governed(
        &self,
        ws: &str,
        by: &str,
        req: ImportReq,
    ) -> Result<ImportResp> {
        let label = req
            .label
            .clone()
            .unwrap_or_else(|| req.kind.clone());

        let parsed = parse_governance_file(&req.kind, &req.content);
        let n = parsed.len();

        if n == 0 {
            return Ok(ImportResp {
                imported: 0,
                import_id: new_id(),
            });
        }

        // Save with state=suggested via a small shim: save() creates rows with
        // the default state ('accepted'); we flip them right after.
        let mems = self.save(ws, by, parsed).await?;
        let ids: Vec<String> = mems.iter().map(|m| m.id.clone()).collect();

        // Mark each as suggested + record the import kind in provenance_json.
        let prov = serde_json::json!({
            "op": "import",
            "kind": req.kind,
            "label": label,
        })
        .to_string();
        for id in &ids {
            let _ = self.repo().set_state(ws, id, STATE_SUGGESTED).await;
            let _ = sqlx::query(
                "UPDATE memories SET provenance_json=? WHERE id=? AND workspace_id=?",
            )
            .bind(&prov)
            .bind(id)
            .bind(ws)
            .execute(self.pool())
            .await;
        }

        let gi = self
            .repo()
            .create_governed_import(ws, &req.kind, &label, &ids, by)
            .await?;

        Ok(ImportResp {
            imported: n,
            import_id: gi.id,
        })
    }

    /// List all governed imports for a workspace.
    pub async fn list_governed_imports(&self, ws: &str) -> Result<Vec<GovernedImport>> {
        self.repo().list_governed_imports(ws).await
    }
}

// ---------------------------------------------------------------------------
// Governance file parser
// ---------------------------------------------------------------------------

/// Parse a governance file into a flat list of `NewMemory` items.
/// Strategy: split on level-2 headings (##). If no headings are found, treat
/// the whole file as a single `summary` memory. Each section becomes a `fact`
/// or `learning` memory tagged with the import kind.
fn parse_governance_file(kind: &str, content: &str) -> Vec<NewMemory> {
    let tag = kind.to_string();
    let source_kind = "manual";
    let collection = "platform-map";
    let memory_kind = match kind {
        import_kind::AGENTS_MD => kind::FACT,
        import_kind::CLAUDE_MD => kind::FACT,
        import_kind::CURSORRULES => kind::CONSTRAINT,
        _ => kind::LEARNING,
    };

    // Split on ## headings — standard markdown section delimiter.
    let sections = split_markdown_sections(content);

    if sections.is_empty() {
        return vec![];
    }

    sections
        .into_iter()
        .filter(|(_, body)| !body.trim().is_empty())
        .map(|(title, body)| NewMemory {
            collection: collection.into(),
            record_type: "item".into(),
            scope: Scope::Workspace,
            story_id: None,
            kind: memory_kind.into(),
            title,
            body: body.trim().to_string(),
            entities: vec![],
            tags: vec![tag.clone()],
            source_kind: source_kind.into(),
            source_ref: None,
            refs: vec![],
            confidence: Some(0.9),
            salience: Some(0.7),
            visibility: "shared".into(),
        })
        .collect()
}

/// Split markdown text on level-2 headings (`## …`). Returns a list of
/// (section_title, section_body) pairs. Content before the first `##` becomes a
/// section titled after the first level-1 heading found, or "Overview".
fn split_markdown_sections(text: &str) -> Vec<(String, String)> {
    let mut sections: Vec<(String, String)> = Vec::new();
    let mut current_title = extract_h1_title(text).unwrap_or_else(|| "Overview".into());
    let mut current_body = String::new();

    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if !current_body.trim().is_empty() {
                sections.push((current_title.clone(), current_body.clone()));
            }
            current_title = heading.trim().to_string();
            current_body = String::new();
        } else {
            current_body.push_str(line);
            current_body.push('\n');
        }
    }
    if !current_body.trim().is_empty() {
        sections.push((current_title, current_body));
    }

    // Fallback: if no sections were found, treat the whole text as one block.
    if sections.is_empty() && !text.trim().is_empty() {
        sections.push(("Governance rules".into(), text.to_string()));
    }
    sections
}

/// Extract the first `# Heading` from markdown, if present.
fn extract_h1_title(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with("# "))
        .map(|l| l.trim_start_matches('#').trim().to_string())
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── state transition ──────────────────────────────────────────────────

    #[test]
    fn valid_states_accepted() {
        assert!(is_valid_state("suggested"));
        assert!(is_valid_state("accepted"));
        assert!(is_valid_state("stale"));
        assert!(is_valid_state("contradicted"));
    }

    #[test]
    fn invalid_states_rejected() {
        assert!(!is_valid_state("unknown"));
        assert!(!is_valid_state(""));
        assert!(!is_valid_state("ACCEPTED")); // case-sensitive
        assert!(!is_valid_state("active"));
    }

    // ── markdown parser ───────────────────────────────────────────────────

    #[test]
    fn parses_h2_sections() {
        let md = "# My File\n\nPreamble.\n\n## Section A\n\nBody A.\n\n## Section B\n\nBody B.\n";
        let sections = split_markdown_sections(md);
        // Preamble (before first ##) + 2 sections.
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].0, "My File");
        assert!(sections[0].1.contains("Preamble"));
        assert_eq!(sections[1].0, "Section A");
        assert!(sections[1].1.contains("Body A"));
        assert_eq!(sections[2].0, "Section B");
        assert!(sections[2].1.contains("Body B"));
    }

    #[test]
    fn parses_no_headings_as_single_section() {
        let md = "Just some flat rules.\nLine 2.";
        let sections = split_markdown_sections(md);
        // No ##-headings and no # heading: the whole text lands as a single
        // preamble section titled "Overview" (the fallback for missing H1).
        assert_eq!(sections.len(), 1);
        assert!(sections[0].1.contains("flat rules"));
    }

    #[test]
    fn empty_sections_filtered() {
        let md = "## Empty\n\n## Has content\n\nActual body.";
        let mems = parse_governance_file(import_kind::AGENTS_MD, md);
        // Only "Has content" (and possibly the implicit empty preamble) survive.
        assert!(!mems.is_empty());
        for m in &mems {
            assert!(!m.body.trim().is_empty());
        }
    }

    #[test]
    fn cursorrules_tagged_constraint() {
        let md = "## Rule 1\n\nAlways use TypeScript.";
        let mems = parse_governance_file(import_kind::CURSORRULES, md);
        assert_eq!(mems[0].kind, kind::CONSTRAINT);
    }

    #[test]
    fn agents_md_tagged_fact() {
        let md = "## Build commands\n\ncargo build --workspace";
        let mems = parse_governance_file(import_kind::AGENTS_MD, md);
        assert_eq!(mems[0].kind, kind::FACT);
        assert!(mems[0].tags.contains(&"agents-md".to_string()));
    }

    #[test]
    fn empty_content_yields_no_memories() {
        let mems = parse_governance_file(import_kind::CLAUDE_MD, "");
        assert!(mems.is_empty());
    }

    // ── forget / undo lifecycle ───────────────────────────────────────────
    // Full round-trip tests (set_state, soft_forget, undo_forget, merge, split)
    // require a real SQLite pool and run via cargo test -p otto-memory under
    // the integration-test harness (test_support.rs). The logic above is
    // covered by the unit tests; the repo layer is tested in otto-state.
}
