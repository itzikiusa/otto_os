//! GitHub reviewer reads and writes: the "review requested from you" list
//! and adding / removing requested reviewers on an existing PR.
//!
//! GitHub has no list filter for "review requested from user X" on the pulls
//! endpoint (only the search API, whose results are issues without branches),
//! but every pulls-list row carries `requested_reviewers`. So one page is read
//! as usual and narrowed to the rows naming the bound account — a page may
//! come back shorter than `per_page`, while `has_more` stays GitHub's own
//! `Link: rel="next"`. GitHub drops a reviewer from `requested_reviewers` once
//! they review, so this is exactly "still waiting on you".

use otto_core::api::PrSummary;
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{summary_for, user_keys, Github};
use crate::providers::identity::Me;
use crate::providers::{varr, PrPage, RemoteRef};

/// The PR row asks `me` for a review (stable ids, never a display name).
pub(crate) fn requested_from(pr: &Value, me: &Me) -> bool {
    varr(pr, &["requested_reviewers"])
        .iter()
        .any(|u| me.owns(&user_keys(u)))
}

/// The bodies of the two reviewer writes: `POST` (add) and `DELETE` (remove)
/// `/pulls/{n}/requested_reviewers`. `None` = nothing to send that way.
pub(crate) fn reviewer_bodies(add: &[String], remove: &[String]) -> (Option<Value>, Option<Value>) {
    let body = |names: &[String]| {
        let names: Vec<&str> = names
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        (!names.is_empty()).then(|| json!({ "reviewers": names }))
    };
    (body(add), body(remove))
}

impl Github {
    pub(crate) async fn review_requested_page(
        &self,
        r: &RemoteRef,
        page: u32,
        per_page: u32,
    ) -> Result<PrPage> {
        let (listed, me) = tokio::join!(
            self.http
                .get_cached_page(self.req(reqwest::Method::GET, &Self::prs_path(r)).query(&[
                    ("state", "open".to_string()),
                    ("per_page", per_page.to_string()),
                    ("page", page.to_string()),
                ])),
            self.me(),
        );
        let me = me.ok_or_else(|| {
            Error::Conflict(
                "could not tell which GitHub account this is (GET /user failed), so the reviews \
                 requested from you cannot be listed"
                    .into(),
            )
        })?;
        let (body, next) = listed?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| Error::Upstream(format!("github: bad json: {e}")))?;
        let items: Vec<PrSummary> = varr(&v, &[])
            .iter()
            .filter(|p| requested_from(p, &me))
            .map(|p| summary_for(p, Some(&me)))
            .collect();
        Ok(PrPage {
            items,
            has_more: next.is_some(),
        })
    }

    /// Request (`add`) and un-request (`remove`) reviewers by login. A login
    /// GitHub refuses (not a collaborator) fails the call with its message.
    pub(crate) async fn edit_reviewers(
        &self,
        r: &RemoteRef,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let path = format!("{}/{number}/requested_reviewers", Self::prs_path(r));
        let (add, remove) = reviewer_bodies(add, remove);
        if let Some(body) = add {
            self.http
                .json(self.req(reqwest::Method::POST, &path).json(&body))
                .await?;
        }
        if let Some(body) = remove {
            self.http
                .ok(self.req(reqwest::Method::DELETE, &path).json(&body))
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::identity;

    #[test]
    fn a_row_is_kept_only_when_it_names_the_account() {
        let me = Me::from_keys(vec![identity::key("login", "Ann"), identity::num_key(7)]).unwrap();
        let asks = json!({ "requested_reviewers": [{ "login": "bob", "id": 2 }, { "login": "ann", "id": 7 }] });
        let other = json!({ "requested_reviewers": [{ "login": "bob", "id": 2 }] });
        let none = json!({ "number": 1 });
        assert!(requested_from(&asks, &me));
        assert!(!requested_from(&other, &me));
        assert!(!requested_from(&none, &me));
    }

    #[test]
    fn reviewer_bodies_skip_empty_sides_and_blank_names() {
        let (add, remove) = reviewer_bodies(&["ann".into(), " ".into()], &[]);
        assert_eq!(add, Some(json!({ "reviewers": ["ann"] })));
        assert_eq!(remove, None);
        let (add, remove) = reviewer_bodies(&[], &["bob".into()]);
        assert_eq!(add, None);
        assert_eq!(remove, Some(json!({ "reviewers": ["bob"] })));
    }
}
