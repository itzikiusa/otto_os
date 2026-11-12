//! GitHub reactions on PR comments.
//!
//! Review comments (`/pulls/comments/{id}`) and general issue comments
//! (`/issues/comments/{id}`) have separate reaction endpoints. Every comment
//! payload already carries a `reactions` rollup (counts per content), so
//! reading costs nothing extra; only WHETHER the caller reacted needs the
//! reaction list, read lazily for the comments that have any reaction,
//! bounded, and never failing the PR load.

use std::collections::{HashMap, HashSet};

use otto_core::api::{PrComment, PrReaction, ReactionKey};
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{user_keys, GhComment, Github};
use crate::providers::identity::Me;
use crate::providers::{varr, vu64, RemoteRef};

/// At most this many comments get their reaction list read per PR open (the
/// rest keep their counts, without `me`).
const REACTION_READS: usize = 30;
/// Concurrent reaction-list reads.
const REACTION_FANOUT: usize = 6;

/// GitHub's reaction `content` for a key.
pub(crate) fn content(key: ReactionKey) -> &'static str {
    match key {
        ReactionKey::ThumbsUp => "+1",
        ReactionKey::ThumbsDown => "-1",
        ReactionKey::Laugh => "laugh",
        ReactionKey::Hooray => "hooray",
        ReactionKey::Confused => "confused",
        ReactionKey::Heart => "heart",
        ReactionKey::Rocket => "rocket",
        ReactionKey::Eyes => "eyes",
    }
}

fn key_of(content_name: &str) -> Option<ReactionKey> {
    ReactionKey::ALL
        .into_iter()
        .find(|k| content(*k) == content_name)
}

/// A comment payload's `reactions` rollup → one row per present kind
/// (`me` unknown here: false).
pub(crate) fn from_rollup(v: &Value) -> Vec<PrReaction> {
    let Some(roll) = v.get("reactions") else {
        return Vec::new();
    };
    ReactionKey::ALL
        .into_iter()
        .filter_map(|key| {
            let count = vu64(roll, &[content(key)]) as u32;
            (count > 0).then_some(PrReaction {
                key,
                count,
                me: false,
            })
        })
        .collect()
}

/// A reaction list (`[{id, content, user}]`) → counts per kind, `me` by the
/// caller's stable ids.
pub(crate) fn from_rows(rows: &[Value], me: Option<&Me>) -> Vec<PrReaction> {
    let mut counts: HashMap<ReactionKey, (u32, bool)> = HashMap::new();
    for row in rows {
        let Some(key) = row.get("content").and_then(Value::as_str).and_then(key_of) else {
            continue;
        };
        let mine = row
            .get("user")
            .is_some_and(|u| crate::providers::identity::is_me(me, &user_keys(u)));
        let e = counts.entry(key).or_default();
        e.0 += 1;
        e.1 |= mine;
    }
    ReactionKey::ALL
        .into_iter()
        .filter_map(|key| {
            counts.get(&key).map(|&(count, mine)| PrReaction {
                key,
                count,
                me: mine,
            })
        })
        .collect()
}

/// The kinds `me` reacted with in a reaction list.
fn mine(rows: &[Value], me: &Me) -> HashSet<ReactionKey> {
    rows.iter()
        .filter(|row| row.get("user").is_some_and(|u| me.owns(&user_keys(u))))
        .filter_map(|row| row.get("content").and_then(Value::as_str).and_then(key_of))
        .collect()
}

/// Call `f` on every comment of `list` and on its replies.
fn walk(list: &mut [PrComment], f: &mut impl FnMut(&mut PrComment)) {
    for c in list {
        f(&mut *c);
        walk(&mut c.replies, f);
    }
}

impl Github {
    /// One comment's reaction list (`list` is `issues` or `pulls`).
    async fn reaction_rows(&self, repo: &str, list: &str, id: &str) -> Result<Value> {
        let url = format!("{repo}/{list}/comments/{id}/reactions");
        self.http
            .get_cached_json(
                self.req(reqwest::Method::GET, &url)
                    .query(&[("per_page", "100")]),
            )
            .await
    }

    /// Set `me` on the reactions of `issue` (general) and `review` (inline,
    /// with replies) comments: one reaction-list read per comment that has
    /// any reaction, at most [`REACTION_READS`]. A failed read leaves that
    /// comment's counts without `me`.
    pub(super) async fn stamp_my_reactions(
        &self,
        r: &RemoteRef,
        issue: &mut [PrComment],
        review: &mut [PrComment],
        me: Option<&Me>,
    ) {
        let Some(me) = me else { return };
        let mut wanted: Vec<(&'static str, String)> = Vec::new();
        for (list, comments) in [("issues", &mut *issue), ("pulls", &mut *review)] {
            walk(comments, &mut |c: &mut PrComment| {
                if !c.reactions.is_empty() && c.can_react {
                    wanted.push((list, c.id.clone()));
                }
            });
        }
        if wanted.is_empty() {
            return;
        }
        let repo = format!("/repos/{}/{}", r.owner, r.repo);
        // Plain batches, not a `Stream`: a stream combinator's closure that
        // borrows `self` / `me` is "not general enough" inside an
        // `async_trait` method that must be `Send`.
        let reads: Vec<(&'static str, String)> = wanted.into_iter().take(REACTION_READS).collect();
        let mut found: HashMap<(&'static str, String), HashSet<ReactionKey>> = HashMap::new();
        for batch in reads.chunks(REACTION_FANOUT) {
            let mut pending = Vec::with_capacity(batch.len());
            for (list, id) in batch {
                pending.push(self.reaction_rows(&repo, list, id));
            }
            let answers = futures_util::future::join_all(pending).await;
            for ((list, id), rows) in batch.iter().zip(answers) {
                match rows {
                    Ok(v) => {
                        found.insert((*list, id.clone()), mine(varr(&v, &[]), me));
                    }
                    Err(e) => tracing::debug!("github: reaction list unavailable: {e}"),
                }
            }
        }
        for (list, comments) in [("issues", issue), ("pulls", review)] {
            walk(comments, &mut |c: &mut PrComment| {
                if let Some(keys) = found.get(&(list, c.id.clone())) {
                    for rx in &mut c.reactions {
                        rx.me = keys.contains(&rx.key);
                    }
                }
            });
        }
    }

    /// `POST …/comments/{id}/reactions {content}` (GitHub answers 200 when
    /// the caller already had it), or — to remove — the caller's matching
    /// reaction found by stable identity and `DELETE`d by its id. Answers
    /// the comment's reactions afterwards.
    pub(super) async fn set_reaction_impl(
        &self,
        r: &RemoteRef,
        number: u64,
        comment_id: &str,
        key: ReactionKey,
        on: bool,
    ) -> Result<Vec<PrReaction>> {
        if comment_id.starts_with("review-") {
            return Err(Error::Invalid(
                "a GitHub review summary can't take reactions from Otto — react on GitHub".into(),
            ));
        }
        let (list, id) = match self.comment_kind(r, number, comment_id).await? {
            GhComment::Review(id) => ("pulls", id),
            GhComment::Issue(id, _) => ("issues", id),
        };
        let base = format!(
            "/repos/{}/{}/{list}/comments/{id}/reactions",
            r.owner, r.repo
        );
        let me = self.me().await;
        if on {
            self.http
                .ok(self
                    .req(reqwest::Method::POST, &base)
                    .json(&json!({ "content": content(key) })))
                .await?;
        } else {
            let me = me.as_ref().ok_or_else(|| {
                Error::Conflict(
                    "could not tell which GitHub account this is (GET /user failed), so your \
                     reaction cannot be found"
                        .into(),
                )
            })?;
            let rows = self
                .http
                .json(
                    self.req(reqwest::Method::GET, &base)
                        .query(&[("content", content(key)), ("per_page", "100")]),
                )
                .await?;
            let own = varr(&rows, &[])
                .iter()
                .filter(|row| row.get("user").is_some_and(|u| me.owns(&user_keys(u))))
                .map(|row| vu64(row, &["id"]))
                .filter(|rid| *rid > 0);
            for rid in own {
                self.http
                    .ok(self.req(reqwest::Method::DELETE, &format!("{base}/{rid}")))
                    .await?;
            }
        }
        let rows = self
            .http
            .paginate_json(
                self.req(reqwest::Method::GET, &base)
                    .query(&[("per_page", "100")]),
                self.http.client(),
                self.auth_header(),
            )
            .await?;
        let mut out = from_rows(&rows, me.as_ref());
        if me.is_none() {
            // The write itself says whose this one is.
            for rx in out.iter_mut().filter(|rx| rx.key == key) {
                rx.me = on;
            }
        }
        Ok(out)
    }
}
