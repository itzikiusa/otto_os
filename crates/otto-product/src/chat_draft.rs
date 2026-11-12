//! Where a story body the agent proposes in the story Chat lands.
//!
//! The agent's text must never replace a person's. A proposed body ALWAYS
//! becomes a new `suggested` version — on a local draft and on an imported
//! Jira / Confluence story alike — which the Rewrite tab shows beside the
//! current text and publishes on request. The story's title and its `draft`
//! / `source` rows are left alone. (Discovery Chat's "Replace draft" used to
//! write a new `draft` revision on a local draft; the merged Chat follows
//! Refine's rule instead.)

use otto_core::{Error, Id, Result};
use otto_state::{NewVersion, ProductStoryVersion};

use crate::service::ProductService;

impl ProductService {
    /// Save `body_md` as a `suggested` version of `story_id`. `title`
    /// defaults to the story's title when absent or blank. Refuses an empty
    /// body (an agent that omitted it used to blank the draft).
    pub async fn propose_story_body(
        &self,
        story_id: &Id,
        title: Option<&str>,
        body_md: &str,
        change_notes: Option<String>,
        by: &Id,
    ) -> Result<ProductStoryVersion> {
        if body_md.trim().is_empty() {
            return Err(Error::Invalid(
                "the proposed story body is empty; nothing was saved".into(),
            ));
        }
        let story = self.repo.get_story(story_id).await?;
        let title = title
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(story.title.as_str());
        self.repo
            .add_version(NewVersion {
                story_id: story_id.clone(),
                kind: "suggested".into(),
                title: title.into(),
                body_md: body_md.into(),
                raw_json: None,
                change_notes,
                created_by: by.clone(),
            })
            .await
    }
}
