//! GitLab reviewer reads and writes: the "review requested from you" list
//! (`reviewer_id=` — GitLab's own filter, so pages stay full) and adding /
//! removing reviewers on an existing MR (a PUT of the WHOLE `reviewer_ids`
//! list; additions resolve by username, removals against the MR's current
//! reviewers by username or name).

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{summary_for, Gitlab};
use crate::providers::{varr, vstr, vu64, PrPage, RemoteRef};

/// The new reviewer ids: the current ones minus `remove`, plus `add` (each
/// once, in order). `current` is the MR's `reviewers` array.
pub(crate) fn next_reviewer_ids(current: &[Value], add: &[u64], remove: &[u64]) -> Vec<u64> {
    let mut out: Vec<u64> = current
        .iter()
        .map(|u| vu64(u, &["id"]))
        .filter(|id| *id != 0 && !remove.contains(id))
        .collect();
    for id in add {
        if !out.contains(id) {
            out.push(*id);
        }
    }
    out
}

/// One of the MR's current reviewers by username or name (case-insensitive).
fn reviewer_id(current: &[Value], name: &str) -> Option<u64> {
    let needle = name.trim().to_ascii_lowercase();
    current
        .iter()
        .find(|u| {
            vstr(u, &["username"]).to_ascii_lowercase() == needle
                || vstr(u, &["name"]).to_ascii_lowercase() == needle
        })
        .map(|u| vu64(u, &["id"]))
        .filter(|id| *id != 0)
}

impl Gitlab {
    pub(crate) async fn review_requested_page(
        &self,
        r: &RemoteRef,
        page: u32,
        per_page: u32,
    ) -> Result<PrPage> {
        let me = self.me().await.ok_or_else(|| {
            Error::Conflict(
                "could not tell which GitLab account this is (GET /user failed), so the reviews \
                 requested from you cannot be listed"
                    .into(),
            )
        })?;
        let id = me
            .value("id")
            .ok_or_else(|| Error::Conflict("this GitLab account has no user id".into()))?
            .to_string();
        let rb = self
            .req(reqwest::Method::GET, &Self::mr_path(r, ""))
            .query(&[
                ("state", "opened".to_string()),
                ("reviewer_id", id),
                ("per_page", per_page.to_string()),
                ("page", page.to_string()),
            ]);
        let (body, next) = self.http.get_cached_page(rb).await?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| Error::Upstream(format!("gitlab: bad json: {e}")))?;
        Ok(PrPage {
            items: varr(&v, &[])
                .iter()
                .map(|m| summary_for(m, Some(&me)))
                .collect(),
            has_more: next.is_some(),
        })
    }

    /// Add / remove reviewers; a name that resolves to nobody is refused
    /// (409) before anything is written.
    pub(crate) async fn edit_reviewers(
        &self,
        r: &RemoteRef,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let path = Self::mr_path(r, &format!("/{number}"));
        let mr = self
            .http
            .json(self.req(reqwest::Method::GET, &path))
            .await?;
        let current = varr(&mr, &["reviewers"]);
        let mut unknown = Vec::new();
        let mut add_ids = Vec::new();
        for name in add {
            match self.resolve_user_id(name.trim()).await? {
                Some(id) => add_ids.push(id),
                None => unknown.push(name.clone()),
            }
        }
        let mut remove_ids = Vec::new();
        for name in remove {
            match reviewer_id(current, name) {
                Some(id) => remove_ids.push(id),
                None => unknown.push(name.clone()),
            }
        }
        if !unknown.is_empty() {
            return Err(Error::Conflict(format!(
                "no such reviewer: {}",
                unknown.join(", ")
            )));
        }
        let body = json!({ "reviewer_ids": next_reviewer_ids(current, &add_ids, &remove_ids) });
        self.http
            .json(self.req(reqwest::Method::PUT, &path).json(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_ids_drop_removed_and_append_new_once() {
        let current = vec![json!({ "id": 1 }), json!({ "id": 2 })];
        assert_eq!(next_reviewer_ids(&current, &[3, 1], &[2]), vec![1, 3]);
        assert_eq!(next_reviewer_ids(&current, &[], &[1, 2]), Vec::<u64>::new());
    }

    #[test]
    fn removals_match_username_or_name() {
        let current = vec![json!({ "id": 5, "username": "ann", "name": "Ann Lee" })];
        assert_eq!(reviewer_id(&current, "ANN"), Some(5));
        assert_eq!(reviewer_id(&current, "ann lee"), Some(5));
        assert_eq!(reviewer_id(&current, "bob"), None);
    }
}
