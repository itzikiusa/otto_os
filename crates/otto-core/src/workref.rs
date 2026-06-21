//! Work-graph attribution — a small, all-optional reference tying a unit of
//! agent work back to the things it touched: repo/branch/PR, product story,
//! swarm task, workflow, channel, review. Stamped into `sessions.meta_json`
//! under `work` at session creation and flattened into usage ingest so cost and
//! tokens can be attributed across modules ("why did this run cost so much?").
//!
//! Every field is optional: a plain shell session carries an empty `WorkRef`,
//! which serializes to `{}` (all `None` skipped). Each crate sets only the
//! dimensions it knows; [`WorkRef::merge`] overlays late knowledge onto early.

use serde::{Deserialize, Serialize};

use crate::Id;

/// Cross-module attribution for a unit of work. See module docs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRef {
    /// Git repository id (Otto connection/repo id), when the work is tied to a repo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<Id>,
    /// Git branch the work targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Pull-request number the work is associated with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_number: Option<i64>,
    /// Product story id (Jira/Confluence story being worked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_id: Option<Id>,
    /// Swarm task id that scheduled this work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_task_id: Option<Id>,
    /// Workflow (run/definition) id that launched this work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_id: Option<Id>,
    /// Channel (Slack/Telegram) integration id that originated the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    /// Code-review id this work belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<Id>,
    /// Free-form origin tag describing what launched the work
    /// ("review" | "product" | "swarm" | "workflow" | "channel" | "manual" | …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl WorkRef {
    /// `true` when no dimension is set (serializes to `{}`).
    pub fn is_empty(&self) -> bool {
        *self == WorkRef::default()
    }

    /// Overlay `other`'s set fields onto `self`; `other` wins where present, but
    /// a `None` in `other` never clears an existing value in `self`.
    pub fn merge(&mut self, other: &WorkRef) {
        if other.repo_id.is_some() {
            self.repo_id = other.repo_id.clone();
        }
        if other.branch.is_some() {
            self.branch = other.branch.clone();
        }
        if other.pr_number.is_some() {
            self.pr_number = other.pr_number;
        }
        if other.story_id.is_some() {
            self.story_id = other.story_id.clone();
        }
        if other.swarm_task_id.is_some() {
            self.swarm_task_id = other.swarm_task_id.clone();
        }
        if other.workflow_id.is_some() {
            self.workflow_id = other.workflow_id.clone();
        }
        if other.channel.is_some() {
            self.channel = other.channel.clone();
        }
        if other.review_id.is_some() {
            self.review_id = other.review_id.clone();
        }
        if other.origin.is_some() {
            self.origin = other.origin.clone();
        }
    }

    /// String-keyed view for flattening into usage-ingest dimensions. Only the
    /// dimensions that are set are returned; keys match the usage schema columns
    /// (`repo_id`, `branch`, `pr_number`, `story_id`, `swarm_task_id`,
    /// `workflow_id`, `channel`, `review_id`, `origin`).
    pub fn dimensions(&self) -> Vec<(&'static str, String)> {
        let mut d: Vec<(&'static str, String)> = Vec::new();
        if let Some(v) = &self.repo_id {
            d.push(("repo_id", v.clone()));
        }
        if let Some(v) = &self.branch {
            d.push(("branch", v.clone()));
        }
        if let Some(v) = self.pr_number {
            d.push(("pr_number", v.to_string()));
        }
        if let Some(v) = &self.story_id {
            d.push(("story_id", v.clone()));
        }
        if let Some(v) = &self.swarm_task_id {
            d.push(("swarm_task_id", v.clone()));
        }
        if let Some(v) = &self.workflow_id {
            d.push(("workflow_id", v.clone()));
        }
        if let Some(v) = &self.channel {
            d.push(("channel", v.clone()));
        }
        if let Some(v) = &self.review_id {
            d.push(("review_id", v.clone()));
        }
        if let Some(v) = &self.origin {
            d.push(("origin", v.clone()));
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_serializes_to_empty_object() {
        let wr = WorkRef::default();
        assert!(wr.is_empty());
        assert_eq!(serde_json::to_string(&wr).unwrap(), "{}");
    }

    #[test]
    fn set_fields_round_trip_and_skip_none() {
        let wr = WorkRef {
            repo_id: Some("repo1".into()),
            pr_number: Some(42),
            origin: Some("review".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&wr).unwrap();
        assert!(json.contains("\"repo_id\":\"repo1\""));
        assert!(json.contains("\"pr_number\":42"));
        assert!(!json.contains("branch"));
        let back: WorkRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, wr);
        assert!(!wr.is_empty());
    }

    #[test]
    fn merge_overlays_present_only() {
        let mut base = WorkRef {
            repo_id: Some("repo1".into()),
            branch: Some("main".into()),
            ..Default::default()
        };
        let late = WorkRef {
            branch: Some("feature".into()),
            pr_number: Some(7),
            ..Default::default()
        };
        base.merge(&late);
        assert_eq!(base.repo_id.as_deref(), Some("repo1")); // untouched
        assert_eq!(base.branch.as_deref(), Some("feature")); // overlaid
        assert_eq!(base.pr_number, Some(7)); // added
    }

    #[test]
    fn dimensions_lists_only_present() {
        let wr = WorkRef {
            repo_id: Some("r".into()),
            pr_number: Some(3),
            ..Default::default()
        };
        let dims = wr.dimensions();
        assert_eq!(dims.len(), 2);
        assert!(dims.contains(&("repo_id", "r".to_string())));
        assert!(dims.contains(&("pr_number", "3".to_string())));
    }
}
