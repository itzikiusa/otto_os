//! Optimistic-concurrency guard for publishing a rewrite back to Jira.
//!
//! Confluence pages carry a `version` Otto records at sync time and checks
//! before it overwrites the page. Jira descriptions have no such number, and
//! the issue's `fields.updated` moves on every comment (Otto's own posted
//! questions included), which would make every publish after "Post questions"
//! a false conflict. So the base is the description itself: the Markdown Otto
//! last saw (recorded in the newest `source` / `published` row's `raw_json`)
//! is compared with the live description, converted by the same ADF→Markdown
//! path, right before the overwrite.

use otto_core::{Error, Id, Result};
use otto_state::ProductStoryVersion;

use crate::service::ProductService;

/// The prefix of the 409 a publish returns when the Jira issue changed since
/// Otto last synced it. The UI keys its "Refresh" offer on the 409 status.
pub(crate) const JIRA_CHANGED: &str = "the issue changed in Jira since Otto last synced it";

/// The Jira description recorded in a version row's `raw_json` (import and
/// refresh write the issue summary including `description`; a publish writes
/// `{"description": …}` with what Jira held right after the write). `None`
/// for rows that carry none (published before this guard existed).
pub(crate) fn raw_jira_description(raw_json: Option<&str>) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(raw_json?)
        .ok()?
        .get("description")?
        .as_str()
        .map(str::to_string)
}

/// The newer of the newest `source` and newest `published` row (by
/// `version_no`): whichever Otto saw last is the publish's base.
pub(crate) fn newest_synced(
    source: Option<ProductStoryVersion>,
    published: Option<ProductStoryVersion>,
) -> Option<ProductStoryVersion> {
    match (source, published) {
        (Some(s), Some(p)) => Some(if p.version_no > s.version_no { p } else { s }),
        (s, p) => s.or(p),
    }
}

/// `Some(conflict)` when the live description differs from the base Otto
/// last synced; `None` when they match.
pub(crate) fn jira_publish_conflict(base: &str, live: &str) -> Option<Error> {
    (base.trim_end() != live.trim_end()).then(|| {
        Error::Conflict(format!(
            "{JIRA_CHANGED}; refresh the story, review the changes, then publish again"
        ))
    })
}

impl ProductService {
    /// The Jira description Otto last synced for a story, or `None` when the
    /// newest synced row recorded none (the publish then keeps its old
    /// last-writer-wins behaviour rather than raising a false conflict).
    pub(crate) async fn synced_jira_description(&self, story_id: &Id) -> Result<Option<String>> {
        let source = self.repo.latest_version_of_kind(story_id, "source").await?;
        let published = self
            .repo
            .latest_version_of_kind(story_id, "published")
            .await?;
        Ok(newest_synced(source, published)
            .and_then(|v| raw_jira_description(v.raw_json.as_deref())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn row(version_no: i64, kind: &str, raw: Option<&str>) -> ProductStoryVersion {
        ProductStoryVersion {
            id: format!("v{version_no}"),
            story_id: "s".into(),
            version_no,
            kind: kind.into(),
            title: "t".into(),
            body_md: String::new(),
            raw_json: raw.map(str::to_string),
            change_notes: None,
            created_by: "u".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn raw_jira_description_reads_import_and_publish_rows() {
        assert_eq!(
            raw_jira_description(Some(r#"{"key":"P-1","description":"Body"}"#)).as_deref(),
            Some("Body")
        );
        assert_eq!(raw_jira_description(Some(r#"{"version":3}"#)), None);
        assert_eq!(raw_jira_description(None), None);
        assert_eq!(raw_jira_description(Some("not json")), None);
    }

    #[test]
    fn newest_synced_prefers_the_higher_version_no() {
        let s = row(2, "source", Some(r#"{"description":"old"}"#));
        let p = row(5, "published", Some(r#"{"description":"new"}"#));
        assert_eq!(
            newest_synced(Some(s.clone()), Some(p.clone())).unwrap().id,
            "v5"
        );
        let s_newer = row(7, "source", None);
        assert_eq!(newest_synced(Some(s_newer), Some(p)).unwrap().id, "v7");
        assert_eq!(newest_synced(Some(s), None).unwrap().id, "v2");
        assert!(newest_synced(None, None).is_none());
    }

    #[test]
    fn jira_conflict_only_when_the_live_description_moved() {
        assert!(jira_publish_conflict("Body\n", "Body").is_none());
        match jira_publish_conflict("Body", "Body edited in Jira") {
            Some(Error::Conflict(m)) => assert!(m.starts_with(JIRA_CHANGED)),
            other => panic!("expected a conflict, got {other:?}"),
        }
    }
}
