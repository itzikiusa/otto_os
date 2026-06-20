//! Product → memory façade: ingest a story's structured artifacts (answered
//! questions, learnings, latest analysis, latest version) into the memory layer,
//! and recall a compact background brief. Best-effort; dedup/embedding happen
//! inside `MemoryService::save`.

use std::sync::Arc;

use otto_core::{Id, Result};
use otto_memory::{MemoryService, RecallBrief, RecallOpts};
use otto_state::ProductRepo;

use crate::extract;

pub struct ProductMemory {
    mem: Arc<MemoryService>,
}

impl ProductMemory {
    pub fn new(mem: Arc<MemoryService>) -> Self {
        Self { mem }
    }

    /// Extract a story's structured artifacts into memory. Returns the number of
    /// candidate memories submitted (exact-dups are deduped inside `save`).
    pub async fn ingest_story(
        &self,
        repo: &ProductRepo,
        ws: &Id,
        story_id: &Id,
        by: &Id,
    ) -> Result<usize> {
        let mut items = Vec::new();
        for q in repo.list_questions(story_id).await? {
            if let Some(m) = extract::from_answered_question(story_id, &q) {
                items.push(m);
            }
        }
        for l in repo.list_learnings(ws, true).await? {
            items.push(extract::from_learning(&l));
        }
        // Latest analysis with a non-empty summary.
        if let Some(a) = repo
            .list_analyses(story_id)
            .await?
            .into_iter()
            .find(|a| !a.summary.trim().is_empty())
        {
            items.extend(extract::from_analysis_summary(story_id, &a));
        }
        // Newest version (highest version_no).
        if let Some(v) = repo
            .list_versions(story_id)
            .await?
            .into_iter()
            .max_by_key(|v| v.version_no)
        {
            if let Some(m) = extract::from_version(story_id, &v) {
                items.push(m);
            }
        }
        let n = items.len();
        self.mem.save(ws, by, items).await?;
        Ok(n)
    }

    /// Recall a compact background brief for a story (replaces re-reading raw
    /// artifacts on every agent run).
    pub async fn recall_brief(
        &self,
        ws: &Id,
        story_id: &Id,
        focus: Option<&str>,
    ) -> Result<RecallBrief> {
        self.mem
            .recall_brief(
                ws,
                story_id,
                RecallOpts {
                    focus: focus.map(|s| s.to_string()),
                    token_budget: 4000,
                    kinds: vec![],
                    viewer: None,
                },
            )
            .await
    }
}
