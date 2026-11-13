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
//!
//! A `published` row with no description means Otto does NOT know what Jira
//! holds (the re-read after the write failed, or the row predates this
//! guard): the publish refuses until a Refresh records a `source` row again,
//! instead of silently falling back to last-writer-wins.

use otto_core::{Error, Id, Result};
use otto_state::ProductStoryVersion;

use crate::service::ProductService;

/// The prefix of the 409 a publish returns when the Jira issue changed since
/// Otto last synced it. The UI keys its "Refresh" offer on the 409 status.
pub(crate) const JIRA_CHANGED: &str = "the issue changed in Jira since Otto last synced it";

/// The refusal when the newest synced row is a publish whose result Otto
/// could not read back. Also a 409, so the UI's "Refresh" offer applies.
pub(crate) const JIRA_UNCONFIRMED: &str =
    "Otto could not confirm what is in Jira after the last publish. Refresh the story first, then publish again";

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

/// The base a Jira publish compares the live description with, from the
/// newest synced row (see [`newest_synced`]):
/// - no synced row at all (never imported/refreshed nor published) → `Ok(None)`:
///   the publish proceeds as designed;
/// - a row with a recorded description → that description;
/// - a `source` row without one (a story split off by Otto, a sync from
///   before descriptions were recorded) → `Ok(None)`, the old behaviour;
/// - a `published` row without one → refuse ([`JIRA_UNCONFIRMED`]): Otto
///   wrote to Jira and never saw the result, so it cannot tell a teammate's
///   later edit from its own.
pub(crate) fn jira_publish_base(newest: Option<&ProductStoryVersion>) -> Result<Option<String>> {
    let Some(v) = newest else {
        return Ok(None);
    };
    match raw_jira_description(v.raw_json.as_deref()) {
        Some(d) => Ok(Some(d)),
        None if v.kind == "published" => Err(Error::Conflict(JIRA_UNCONFIRMED.into())),
        None => Ok(None),
    }
}

/// Whether a Refresh of a Jira story must record a `source` row even though
/// the description did not change: the newest synced row is a `published`
/// row with no description, which [`jira_publish_base`] refuses on. Without
/// a new row the refusal would outlive every Refresh.
pub(crate) fn refresh_rearms_jira_guard(
    source: Option<&ProductStoryVersion>,
    published: Option<&ProductStoryVersion>,
) -> bool {
    published.is_some_and(|p| {
        source.is_none_or(|s| p.version_no > s.version_no)
            && raw_jira_description(p.raw_json.as_deref()).is_none()
    })
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
    /// The Jira description Otto last synced for a story — see
    /// [`jira_publish_base`] for when it is `None` and when it refuses.
    pub(crate) async fn synced_jira_description(&self, story_id: &Id) -> Result<Option<String>> {
        let source = self.repo.latest_version_of_kind(story_id, "source").await?;
        let published = self
            .repo
            .latest_version_of_kind(story_id, "published")
            .await?;
        jira_publish_base(newest_synced(source, published).as_ref())
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
    fn a_publish_otto_could_not_read_back_refuses_instead_of_overwriting() {
        // The re-read after the last publish failed: no description recorded.
        let p = row(4, "published", None);
        match jira_publish_base(Some(&p)) {
            Err(Error::Conflict(m)) => assert_eq!(m, JIRA_UNCONFIRMED),
            other => panic!("expected the unconfirmed refusal, got {other:?}"),
        }
        // Newer than an armed source row: still refused (it is the newest).
        let s = row(2, "source", Some(r#"{"description":"old"}"#));
        let newest = newest_synced(Some(s), Some(p));
        assert!(jira_publish_base(newest.as_ref()).is_err());
    }

    #[test]
    fn never_synced_and_armed_bases_are_distinct_from_unconfirmed() {
        // Never synced: no row at all → the first publish proceeds.
        assert_eq!(jira_publish_base(None).unwrap(), None);
        // A source row without a description (a split story) → old behaviour.
        assert_eq!(
            jira_publish_base(Some(&row(1, "source", None))).unwrap(),
            None
        );
        // A publish Otto read back → compared.
        let p = row(3, "published", Some(r#"{"description":"Body"}"#));
        assert_eq!(
            jira_publish_base(Some(&p)).unwrap().as_deref(),
            Some("Body")
        );
        let s = row(1, "source", Some(r#"{"description":"Synced"}"#));
        assert_eq!(
            jira_publish_base(Some(&s)).unwrap().as_deref(),
            Some("Synced")
        );
    }

    #[test]
    fn refresh_rearms_only_after_an_unconfirmed_publish() {
        let armed_src = row(2, "source", Some(r#"{"description":"x"}"#));
        let blind_pub = row(3, "published", None);
        assert!(refresh_rearms_jira_guard(
            Some(&armed_src),
            Some(&blind_pub)
        ));
        assert!(refresh_rearms_jira_guard(None, Some(&blind_pub)));
        // A later source row already re-armed it.
        let later_src = row(4, "source", Some(r#"{"description":"y"}"#));
        assert!(!refresh_rearms_jira_guard(
            Some(&later_src),
            Some(&blind_pub)
        ));
        // A publish that was read back needs no extra row.
        let seen_pub = row(3, "published", Some(r#"{"description":"x"}"#));
        assert!(!refresh_rearms_jira_guard(
            Some(&armed_src),
            Some(&seen_pub)
        ));
        assert!(!refresh_rearms_jira_guard(Some(&armed_src), None));
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
