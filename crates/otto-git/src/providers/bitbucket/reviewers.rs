//! Bitbucket Cloud reviewer reads and writes: the "review requested from
//! you" list and adding / removing reviewers on an existing PR.
//!
//! The list is Bitbucket's own filter (`q=reviewers.uuid="{…}"`), so pages
//! stay full. Bitbucket keeps a reviewer on the PR after they approve, so this
//! is "you are a reviewer", not "still waiting on you" — the row's own
//! approval state says the rest.
//!
//! The reviewer write is a PUT of the PR with its WHOLE new `reviewers` list
//! (uuids): Bitbucket has no add/remove endpoint. Names resolve like the
//! create sheet's (nickname, display name or uuid): additions against the
//! workspace members, removals against the PR's current reviewers.

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{summary_for, Bitbucket, Member};
use crate::providers::identity::Me;
use crate::providers::{varr, vstr, PrPage, RemoteRef};

/// The `q=` filter for open PRs that name `me` as a reviewer, or `None` when
/// the identity has neither a uuid nor an account id.
pub(crate) fn review_query(me: &Me) -> Option<String> {
    let (field, id) = match (me.value("uuid"), me.value("account_id")) {
        (Some(u), _) => ("uuid", u),
        (None, Some(a)) => ("account_id", a),
        (None, None) => return None,
    };
    // The id comes from the forge's own `/user`; quotes would break the query.
    let id = id.replace('"', "");
    Some(format!(r#"reviewers.{field}="{id}" AND state="OPEN""#))
}

/// The new reviewer uuids: the current ones minus `remove`, plus `add`
/// (each once, in order). `current` is the PR's `reviewers` array.
pub(crate) fn next_reviewers(current: &[Value], add: &[String], remove: &[String]) -> Vec<String> {
    let mut out: Vec<String> = current
        .iter()
        .map(|u| vstr(u, &["uuid"]))
        .filter(|u| !u.is_empty() && !remove.contains(u))
        .collect();
    for u in add {
        if !out.contains(u) {
            out.push(u.clone());
        }
    }
    out
}

/// A typed name (nickname, display name or uuid, case-insensitive) as one
/// of `people`'s uuid.
fn resolve(name: &str, people: &[Member]) -> Option<String> {
    let needle = name.trim().to_ascii_lowercase();
    people
        .iter()
        .find(|m| {
            m.nickname.to_ascii_lowercase() == needle
                || m.display_name.to_ascii_lowercase() == needle
                || m.uuid.to_ascii_lowercase() == needle
        })
        .map(|m| m.uuid.clone())
}

/// The PR's `reviewers` user objects as members (for resolving removals).
fn as_members(reviewers: &[Value]) -> Vec<Member> {
    reviewers
        .iter()
        .map(|u| Member {
            uuid: vstr(u, &["uuid"]),
            nickname: vstr(u, &["nickname"]),
            display_name: vstr(u, &["display_name"]),
        })
        .collect()
}

impl Bitbucket {
    pub(crate) async fn review_requested_page(
        &self,
        r: &RemoteRef,
        page: u32,
        per_page: u32,
    ) -> Result<PrPage> {
        let me = self.me().await.ok_or_else(|| {
            Error::Conflict(
                "could not tell which Bitbucket account this is (GET /user failed — the app \
                 password needs Account: Read), so the reviews requested from you cannot be listed"
                    .into(),
            )
        })?;
        let q = review_query(&me)
            .ok_or_else(|| Error::Conflict("this Bitbucket account has no user id".into()))?;
        let path = format!(
            "{}?pagelen={per_page}&page={page}&q={}",
            Self::pr_path(r, ""),
            urlencoding::encode(&q)
        );
        let v = self.get_json_cached(&path).await?;
        Ok(PrPage {
            items: varr(&v, &["values"])
                .iter()
                .map(|p| summary_for(p, Some(&me)))
                .collect(),
            has_more: v.get("next").and_then(Value::as_str).is_some(),
        })
    }

    /// Add / remove reviewers by name; a name that resolves to nobody is
    /// refused (409) before anything is written.
    pub(crate) async fn edit_reviewers(
        &self,
        r: &RemoteRef,
        number: u64,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let pr_path = Self::pr_path(r, &format!("/{number}"));
        let pr = self.send_json(reqwest::Method::GET, &pr_path, None).await?;
        let current = varr(&pr, &["reviewers"]);
        let members = if add.is_empty() {
            Vec::new()
        } else {
            self.workspace_members(&r.owner).await?
        };
        let on_pr = as_members(current);
        let mut unknown = Vec::new();
        let mut pick = |names: &[String], people: &[Member]| -> Vec<String> {
            names
                .iter()
                .filter_map(|n| {
                    let hit = resolve(n, people);
                    if hit.is_none() {
                        unknown.push(n.clone());
                    }
                    hit
                })
                .collect()
        };
        let add_ids = pick(add, &members);
        let remove_ids = pick(remove, &on_pr);
        if !unknown.is_empty() {
            return Err(Error::Conflict(format!(
                "no such reviewer in {}: {}",
                r.owner,
                unknown.join(", ")
            )));
        }
        let next = next_reviewers(current, &add_ids, &remove_ids);
        let body = json!({
            "reviewers": next.iter().map(|u| json!({ "uuid": u })).collect::<Vec<_>>(),
        });
        self.send(reqwest::Method::PUT, &pr_path, Some(&body))
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::identity;

    #[test]
    fn the_query_filters_by_the_accounts_stable_id() {
        let me = Me::from_keys(vec![identity::key("uuid", "{U-1}")]).unwrap();
        assert_eq!(
            review_query(&me).as_deref(),
            Some(r#"reviewers.uuid="{u-1}" AND state="OPEN""#)
        );
        let by_account = Me::from_keys(vec![identity::key("account_id", "557:x")]).unwrap();
        assert_eq!(
            review_query(&by_account).as_deref(),
            Some(r#"reviewers.account_id="557:x" AND state="OPEN""#)
        );
        let neither = Me::from_keys(vec![identity::num_key(4)]).unwrap();
        assert_eq!(review_query(&neither), None);
    }

    #[test]
    fn next_reviewers_drops_removed_and_appends_new_once() {
        let current = vec![json!({ "uuid": "{a}" }), json!({ "uuid": "{b}" })];
        let next = next_reviewers(&current, &["{c}".into(), "{a}".into()], &["{b}".into()]);
        assert_eq!(next, vec!["{a}".to_string(), "{c}".to_string()]);
    }

    #[test]
    fn names_resolve_by_nickname_display_name_or_uuid() {
        let people =
            as_members(&[json!({ "uuid": "{a}", "nickname": "ann", "display_name": "Ann Lee" })]);
        assert_eq!(resolve("ANN", &people).as_deref(), Some("{a}"));
        assert_eq!(resolve("ann lee", &people).as_deref(), Some("{a}"));
        assert_eq!(resolve("{A}", &people).as_deref(), Some("{a}"));
        assert_eq!(resolve("bob", &people), None);
    }
}
