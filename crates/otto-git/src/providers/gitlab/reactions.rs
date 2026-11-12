//! GitLab award emoji on MR notes, as Otto's reactions.
//!
//! Note payloads carry no award summary, so reading costs one
//! `GET …/notes/{note_id}/award_emoji` per note — bounded per MR open,
//! cached like every other PR read, and never failing the MR load.

use std::collections::HashMap;

use otto_core::api::{PrComment, PrReaction, ReactionKey};
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::{user_keys, Gitlab};
use crate::providers::identity::{self, Me};
use crate::providers::{varr, vstr, vu64, RemoteRef};

/// At most this many notes get their awards read per MR open.
const AWARD_READS: usize = 30;
/// Concurrent award reads.
const AWARD_FANOUT: usize = 6;

/// GitLab's award-emoji name for a key.
pub(crate) fn award_name(key: ReactionKey) -> &'static str {
    match key {
        ReactionKey::ThumbsUp => "thumbsup",
        ReactionKey::ThumbsDown => "thumbsdown",
        ReactionKey::Laugh => "laughing",
        ReactionKey::Hooray => "tada",
        ReactionKey::Confused => "confused",
        ReactionKey::Heart => "heart",
        ReactionKey::Rocket => "rocket",
        ReactionKey::Eyes => "eyes",
    }
}

/// An award list (`[{id, name, user}]`) → counts per kind; awards outside
/// Otto's eight kinds are left out. `me` by the caller's user id.
pub(crate) fn from_awards(rows: &[Value], me: Option<&Me>) -> Vec<PrReaction> {
    let mut counts: HashMap<ReactionKey, (u32, bool)> = HashMap::new();
    for row in rows {
        let name = vstr(row, &["name"]);
        let Some(key) = ReactionKey::ALL
            .into_iter()
            .find(|k| award_name(*k) == name)
        else {
            continue;
        };
        let mine = row
            .get("user")
            .is_some_and(|u| identity::is_me(me, &user_keys(u)));
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

impl Gitlab {
    fn awards_path(r: &RemoteRef, number: u64, note: u64) -> String {
        Self::mr_path(r, &format!("/{number}/notes/{note}/award_emoji"))
    }

    /// One note's award-emoji list.
    async fn award_rows(&self, r: &RemoteRef, number: u64, note: u64) -> Result<Value> {
        self.http
            .get_cached_json(self.req(reqwest::Method::GET, &Self::awards_path(r, number, note)))
            .await
    }

    /// Fill `reactions` on `comments` (heads and replies). `note_ids[i]` holds
    /// the note ids of `comments[i]`: the head's first, then each reply's.
    pub(super) async fn fill_reactions(
        &self,
        r: &RemoteRef,
        number: u64,
        comments: &mut [PrComment],
        note_ids: &[Vec<u64>],
        me: Option<&Me>,
    ) {
        let wanted: Vec<u64> = note_ids
            .iter()
            .flatten()
            .copied()
            .filter(|n| *n > 0)
            .take(AWARD_READS)
            .collect();
        if wanted.is_empty() {
            return;
        }
        // Plain batches, not a `Stream` (see the GitHub twin): a stream
        // closure borrowing `self` / `me` is "not general enough" here.
        let mut found: HashMap<u64, Vec<PrReaction>> = HashMap::new();
        for batch in wanted.chunks(AWARD_FANOUT) {
            let mut pending = Vec::with_capacity(batch.len());
            for note in batch {
                pending.push(self.award_rows(r, number, *note));
            }
            let answers = futures_util::future::join_all(pending).await;
            for (note, rows) in batch.iter().zip(answers) {
                match rows {
                    Ok(v) => {
                        found.insert(*note, from_awards(varr(&v, &[]), me));
                    }
                    Err(e) => tracing::debug!("gitlab: award emoji unavailable: {e}"),
                }
            }
        }
        for (c, ids) in comments.iter_mut().zip(note_ids) {
            let mut ids = ids.iter();
            if let Some(rx) = ids.next().and_then(|n| found.get(n)) {
                c.reactions = rx.clone();
            }
            for (reply, n) in c.replies.iter_mut().zip(ids) {
                if let Some(rx) = found.get(n) {
                    reply.reactions = rx.clone();
                }
            }
        }
    }

    /// `POST …/notes/{note}/award_emoji {name}` (an award the caller already
    /// gave is the wanted end state), or — to remove — the caller's award of
    /// that name found by user id and `DELETE`d. A thread head's id is its
    /// discussion id, resolved to its first note. Answers the note's
    /// reactions afterwards.
    pub(super) async fn set_reaction_impl(
        &self,
        r: &RemoteRef,
        number: u64,
        comment_id: &str,
        key: ReactionKey,
        on: bool,
    ) -> Result<Vec<PrReaction>> {
        let note = self.note_id(r, number, comment_id).await?;
        let path = Self::awards_path(r, number, note);
        let me = self.me().await;
        if on {
            match self
                .http
                .ok(self
                    .req(reqwest::Method::POST, &path)
                    .json(&json!({ "name": award_name(key) })))
                .await
            {
                Ok(()) => {}
                Err(e) if e.to_string().contains("already been taken") => {}
                Err(e) => return Err(e),
            }
        } else {
            let me = me.as_ref().ok_or_else(|| {
                Error::Conflict(
                    "could not tell which GitLab account this is (GET /user failed), so your \
                     reaction cannot be found"
                        .into(),
                )
            })?;
            let rows = self.awards(&path).await?;
            let own = rows
                .iter()
                .filter(|a| vstr(a, &["name"]) == award_name(key))
                .filter(|a| a.get("user").is_some_and(|u| me.owns(&user_keys(u))))
                .map(|a| vu64(a, &["id"]))
                .filter(|id| *id > 0);
            for id in own {
                self.http
                    .ok(self.req(reqwest::Method::DELETE, &format!("{path}/{id}")))
                    .await?;
            }
        }
        let rows = self.awards(&path).await?;
        let mut out = from_awards(&rows, me.as_ref());
        if me.is_none() {
            // The write itself says whose this one is.
            for rx in out.iter_mut().filter(|rx| rx.key == key) {
                rx.me = on;
            }
        }
        Ok(out)
    }

    /// Every award on a note, read fresh (after a write).
    async fn awards(&self, path: &str) -> Result<Vec<Value>> {
        self.http
            .paginate_json(
                self.req(reqwest::Method::GET, path)
                    .query(&[("per_page", "100")]),
                self.http.client(),
                self.auth_header(),
            )
            .await
    }
}
