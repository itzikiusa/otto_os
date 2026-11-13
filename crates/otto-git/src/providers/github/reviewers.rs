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

use otto_core::api::{PrReviewSummary, PrState, PrSummary};
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{summary_for, user_keys, Github};
use crate::providers::client::{memo_read, memo_store, write_generation};
use crate::providers::identity::Me;
use crate::providers::{varr, vstr, PrPage, RemoteRef};

/// The PR row asks `me` for a review (stable ids, never a display name).
pub(crate) fn requested_from(pr: &Value, me: &Me) -> bool {
    varr(pr, &["requested_reviewers"])
        .iter()
        .any(|u| me.owns(&user_keys(u)))
}

/// One PR's verdict from its `reviewDecision` and latest opinionated reviews
/// (one per reviewer). `requested`: the REST row still asks someone.
fn review_of(pr: &Value, requested: bool) -> Option<PrReviewSummary> {
    let decision = vstr(pr, &["reviewDecision"]);
    let states: Vec<String> = varr(pr, &["latestOpinionatedReviews", "nodes"])
        .iter()
        .map(|n| vstr(n, &["state"]))
        .collect();
    PrReviewSummary::from_counts(
        decision == "CHANGES_REQUESTED" || states.iter().any(|s| s == "CHANGES_REQUESTED"),
        states.iter().filter(|s| *s == "APPROVED").count() as u32,
        requested || decision == "REVIEW_REQUIRED",
    )
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
        let mut items: Vec<PrSummary> = varr(&v, &[])
            .iter()
            .filter(|p| requested_from(p, &me))
            .map(|p| summary_for(p, Some(&me)))
            .collect();
        self.overlay_reviews(r, &mut items).await;
        Ok(PrPage {
            items,
            has_more: next.is_some(),
        })
    }

    /// The verdicts of a page's open PRs in ONE GraphQL read (the REST list
    /// carries none), memoised like the review threads: a write to the repo
    /// drops it. Best-effort — on failure a row keeps its REST "waiting" hint.
    pub(crate) async fn overlay_reviews(&self, r: &RemoteRef, items: &mut [PrSummary]) {
        let open: Vec<u64> = items
            .iter()
            .filter(|p| p.state == PrState::Open)
            .map(|p| p.number)
            .collect();
        if open.is_empty() {
            return;
        }
        let fields: String = open
            .iter()
            .map(|n| {
                format!(
                    "p{n}:pullRequest(number:{n}){{reviewDecision \
                     latestOpinionatedReviews(first:100){{nodes{{state}}}}}}"
                )
            })
            .collect();
        let q = format!(
            "query($owner:String!,$name:String!){{repository(owner:$owner,name:$name){{{fields}}}}}"
        );
        let numbers: Vec<String> = open.iter().map(u64::to_string).collect();
        let memo_url = format!(
            "{}/repos/{}/{}/pulls#reviews={}",
            self.base,
            r.owner,
            r.repo,
            numbers.join(",")
        );
        let auth = format!("Bearer {}", self.token);
        let repo = match memo_read(&memo_url, &auth).and_then(|b| serde_json::from_str(&b).ok()) {
            Some(repo) => repo,
            None => {
                let generation = write_generation(&memo_url);
                let vars = json!({ "owner": r.owner, "name": r.repo });
                match self.graphql(&q, vars).await {
                    Ok(v) => {
                        let repo = v["data"]["repository"].clone();
                        memo_store(&memo_url, &auth, repo.to_string(), generation);
                        repo
                    }
                    Err(e) => {
                        tracing::debug!("github: PR review verdicts unavailable: {e}");
                        return;
                    }
                }
            }
        };
        for p in items.iter_mut() {
            if let Some(pr) = repo.get(format!("p{}", p.number)) {
                p.review = review_of(pr, p.review.is_some());
            }
        }
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
