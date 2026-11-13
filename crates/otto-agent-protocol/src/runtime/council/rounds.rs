//! Council rounds: after the first answers the person asks the council again
//! — every seat, or one — and each seat gets the follow-up as a normal next
//! turn in its own thread (it keeps its own context). A seat still working
//! queues it, exactly like a message sent mid-turn in any thread.
//!
//! The rounds are one `council_rounds` entity on seat 1 (the group's root),
//! so they survive a reload and a daemon restart (`recover` leaves unknown
//! entity kinds alone; the session's delete cascades them). A council from
//! before rounds (no entity) has one round: its first question, every seat.
//! Which message answers which round is read off each seat's timeline by the
//! text the round sent, never stored per seat: a queued follow-up becomes a
//! message only when the seat's turn ends.

use std::sync::LazyLock;

use chrono::DateTime;

use super::*;

/// Rounds one council runs: its first question plus nine follow-ups. Every
/// round is started by the person (tokens are theirs); nothing re-asks.
pub const COUNCIL_MAX_ROUNDS: usize = 10;
/// The entity kind (on seat 1) the rounds are stored under.
const K_ROUNDS: &str = "council_rounds";
/// Characters one follow-up may carry ("Ask the others" quotes an answer).
const FOLLOW_UP_MAX_CHARS: usize = 64_000;

/// A follow-up and a synthesis both read, change and store the one rounds
/// row; one at a time keeps a round from being lost between them.
static ROUNDS_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// One round of a council: the text the person sent and the seats it went to.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CouncilRound {
    /// 1-based; round 1 is the council's first question.
    pub n: u32,
    pub text: String,
    /// The seats it was sent to, ascending.
    pub seats: Vec<u32>,
    /// Seats among `seats` it never reached, and why (a closed seat).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<CouncilSeatRefusal>,
    /// The judges opened on this round, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub syntheses: Vec<Id>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CouncilSeatRefusal {
    pub seat: u32,
    pub reason: String,
}

/// `GET|POST /runtime-sessions/{id}/council/rounds`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CouncilRoundsResp {
    pub rounds: Vec<CouncilRound>,
    pub max_rounds: u32,
}

/// `POST /runtime-sessions/{id}/council/rounds` — a follow-up round.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CouncilFollowUpReq {
    pub text: String,
    /// Seat numbers; empty = every seat.
    #[serde(default)]
    pub seats: Vec<u32>,
}

/// One seat's reply to one round, read off its timeline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RoundReply {
    pub(super) text: String,
    /// How its last turn in the round ended (`completed`, `interrupted`, …).
    pub(super) turn_status: Option<String>,
}

/// A council: its meta, seat 1, every seat by number, every member.
pub(super) struct Council {
    pub(super) meta: CouncilMeta,
    pub(super) root: RuntimeSession,
    pub(super) seats: Vec<(u32, RuntimeSession)>,
    pub(super) members: Vec<RuntimeSession>,
}

impl Council {
    pub(super) fn numbers(&self) -> Vec<u32> {
        self.seats.iter().map(|(n, _)| *n).collect()
    }
}

/// The texts of the rounds seat `n` was asked (and reached), in order.
pub(super) fn asked_of(rounds: &[CouncilRound], n: u32) -> Vec<&str> {
    rounds
        .iter()
        .filter(|r| r.seats.contains(&n) && !r.refused.iter().any(|x| x.seat == n))
        .map(|r| r.text.as_str())
        .collect()
}

/// Which of a seat's messages (`sent`: its top-level user messages, in
/// order) answers each of its rounds (`asked`, in order): each round takes
/// the first message with its text after the previous round's, and a re-ask
/// of the same text before the next round's message takes its place.
/// `None` = not on the timeline (still queued, or removed from the queue).
pub(super) fn round_messages(sent: &[&str], asked: &[&str]) -> Vec<Option<usize>> {
    let mut first = Vec::with_capacity(asked.len());
    let mut from = 0;
    for a in asked {
        let hit = (from..sent.len()).find(|&j| sent[j].trim() == a.trim());
        if let Some(j) = hit {
            from = j + 1;
        }
        first.push(hit);
    }
    (0..asked.len())
        .map(|k| {
            let start = first[k]?;
            let end = first[k + 1..].iter().find_map(|x| *x).unwrap_or(sent.len());
            (start..end).rfind(|&j| sent[j].trim() == asked[k].trim())
        })
        .collect()
}

/// The answer in one round's stretch of a timeline (its message up to the
/// next one): the assistant messages, and how its last turn ended. THE rule
/// for "answered": any assistant text on record in the stretch, whole or
/// cut off (a Stop keeps the partial text; the judge is told it may be
/// incomplete). Text still streaming when the daemon stopped was never
/// persisted (`Projection::apply`), so a seat a restart cut mid-answer has
/// none. The view applies the same rule (`councilRounds.ts`
/// `roundSynthBlocker`).
fn reply_of(items: &[&TurnItem], turns: &HashMap<Id, ProviderTurn>) -> RoundReply {
    let text = items
        .iter()
        .filter_map(|i| match &i.kind {
            TurnItemKind::AssistantMessage { text, .. } if !text.trim().is_empty() => {
                Some(text.as_str())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let last = items
        .iter()
        .filter_map(|i| i.turn_id.as_ref().and_then(|t| turns.get(t)))
        .max_by_key(|t| t.ordinal);
    RoundReply {
        text,
        turn_status: last
            .filter(|t| t.status.is_terminal())
            .and_then(|t| serde_json::to_value(t.status).ok())
            .and_then(|v| v.as_str().map(str::to_string)),
    }
}

/// An error's own words, without the status prefix.
fn reason(e: &Error) -> String {
    match e {
        Error::NotFound(m)
        | Error::Forbidden(m)
        | Error::Conflict(m)
        | Error::Invalid(m)
        | Error::PayloadTooLarge(m)
        | Error::UnsupportedMedia(m)
        | Error::Upstream(m)
        | Error::Internal(m) => m.clone(),
        Error::Unauthorized => "not authorized".into(),
    }
}

impl AgentRuntime {
    /// The council `id` sits on.
    pub(super) async fn council_of(&self, id: &Id) -> Result<Council> {
        let me = self.get(id).await?;
        let (meta, members) = self.council_members(&me).await?;
        let mut seats: Vec<(u32, RuntimeSession)> = members
            .iter()
            .filter_map(|s| {
                s.council()
                    .filter(|c| c.role == CouncilRole::Seat)
                    .map(|c| (c.seat, s.clone()))
            })
            .collect();
        seats.sort_by_key(|(n, _)| *n);
        let root = seats
            .iter()
            .find(|(_, s)| s.parent_id.is_none())
            .map(|(_, s)| s.clone())
            .ok_or_else(|| Error::NotFound("the council's first seat is gone".into()))?;
        Ok(Council {
            meta,
            root,
            seats,
            members,
        })
    }

    /// The council's rounds and the stored row's sequence (0 = none stored:
    /// a council from before rounds, whose one round is its first question).
    pub(super) async fn load_rounds(&self, c: &Council) -> Result<(Vec<CouncilRound>, u64)> {
        let row = self
            .inner
            .repo
            .entities(&c.root.id)
            .await?
            .into_iter()
            .find(|r| r.kind == K_ROUNDS);
        if let Some(r) = row {
            let rounds = serde_json::from_value(r.payload).map_err(|e| {
                Error::Internal(format!("the council's rounds are unreadable: {e}"))
            })?;
            return Ok((rounds, r.sequence));
        }
        let text = self.first_user_text(&c.root.id).await?;
        Ok((vec![first_round(text, c.numbers(), c.root.created_at)], 0))
    }

    /// Store the rounds on seat 1, at or above the row's last sequence (an
    /// upsert below it is ignored) and never above the session's own.
    pub(super) async fn save_rounds(
        &self,
        root: &Id,
        rounds: &[CouncilRound],
        after: u64,
    ) -> Result<()> {
        let sequence = after.max(self.get(root).await?.last_sequence);
        let payload = serde_json::to_value(rounds).map_err(|e| Error::Internal(e.to_string()))?;
        let row = RuntimeEntityRow {
            kind: K_ROUNDS.into(),
            entity_id: "rounds".into(),
            sequence,
            payload,
        };
        self.inner
            .repo
            .upsert_entities(root, std::slice::from_ref(&row))
            .await?;
        Ok(())
    }

    /// `seat`'s reply to each of `asked` (its rounds' texts, in order);
    /// `None` = that round is not on its timeline.
    pub(super) async fn round_replies(
        &self,
        seat: &Id,
        asked: &[&str],
    ) -> Result<Vec<Option<RoundReply>>> {
        let live = self.live(seat).await?;
        let st = live.lock();
        let mut items: Vec<&TurnItem> = st
            .proj
            .items
            .values()
            .filter(|i| i.parent_item_id.is_none())
            .collect();
        items.sort_by_key(|i| i.ordinal);
        let users: Vec<(usize, &str)> = items
            .iter()
            .enumerate()
            .filter_map(|(ix, i)| match &i.kind {
                TurnItemKind::UserMessage { text, .. } => Some((ix, text.as_str())),
                _ => None,
            })
            .collect();
        let sent: Vec<&str> = users.iter().map(|(_, t)| *t).collect();
        let out = round_messages(&sent, asked)
            .into_iter()
            .map(|m| {
                m.map(|u| {
                    let start = users[u].0;
                    let end = users.get(u + 1).map_or(items.len(), |(ix, _)| *ix);
                    reply_of(&items[start..end], &st.proj.turns)
                })
            })
            .collect();
        Ok(out)
    }

    /// `GET /runtime-sessions/{id}/council/rounds`.
    pub async fn council_rounds(&self, id: &Id) -> Result<CouncilRoundsResp> {
        let c = self.council_of(id).await?;
        let (rounds, _) = self.load_rounds(&c).await?;
        Ok(CouncilRoundsResp {
            rounds,
            max_rounds: COUNCIL_MAX_ROUNDS as u32,
        })
    }

    /// A follow-up round: `req.text` to the chosen seats (all when none are
    /// named), each as its own thread's next turn — queued behind a turn
    /// still running. A closed seat (or one whose send is refused) is listed
    /// in the round's `refused`; when no seat takes it, nothing is recorded
    /// (409). 409 too once the council has run its `COUNCIL_MAX_ROUNDS`.
    pub async fn follow_up_council(
        &self,
        id: &Id,
        req: CouncilFollowUpReq,
    ) -> Result<CouncilRoundsResp> {
        let text = req.text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("a follow-up needs text".into()));
        }
        if text.chars().count() > FOLLOW_UP_MAX_CHARS {
            return Err(Error::Invalid(format!(
                "a follow-up holds at most {FOLLOW_UP_MAX_CHARS} characters"
            )));
        }
        let _rounds = rounds_lock().await;
        let c = self.council_of(id).await?;
        let (mut rounds, seq) = self.load_rounds(&c).await?;
        if rounds.len() >= COUNCIL_MAX_ROUNDS {
            return Err(Error::Conflict(format!(
                "this council has had its {COUNCIL_MAX_ROUNDS} rounds — start a new council to keep going"
            )));
        }
        let all = c.numbers();
        let mut want = if req.seats.is_empty() {
            all.clone()
        } else {
            req.seats.clone()
        };
        want.sort_unstable();
        want.dedup();
        if let Some(bad) = want.iter().find(|n| !all.contains(n)) {
            return Err(Error::Invalid(format!("this council has no seat {bad}")));
        }
        let mut refused = Vec::new();
        let mut sends = Vec::with_capacity(want.len());
        for (n, s) in c.seats.iter().filter(|(n, _)| want.contains(n)) {
            if s.status == RuntimeSessionStatus::Closed {
                refused.push(CouncilSeatRefusal {
                    seat: *n,
                    reason: "the seat is closed — reopen it to ask it again".into(),
                });
                continue;
            }
            let rt = self.clone();
            let sid = s.id.clone();
            let p = RuntimePromptReq {
                text: text.to_string(),
                effort: s.council().and_then(|m| m.effort),
                delivery: Some(PromptDelivery::Queue),
                ..RuntimePromptReq::default()
            };
            sends.push((
                *n,
                tokio::spawn(async move { rt.prompt_with(&sid, &p).await }),
            ));
        }
        for (n, send) in sends {
            let r = send
                .await
                .unwrap_or_else(|e| Err(Error::Internal(e.to_string())));
            if let Err(e) = r {
                refused.push(CouncilSeatRefusal {
                    seat: n,
                    reason: reason(&e),
                });
            }
        }
        refused.sort_by_key(|r| r.seat);
        if refused.len() == want.len() {
            let why: Vec<String> = refused
                .iter()
                .map(|r| format!("seat {}: {}", r.seat, r.reason))
                .collect();
            return Err(Error::Conflict(format!(
                "no seat took the follow-up ({})",
                why.join("; ")
            )));
        }
        rounds.push(CouncilRound {
            n: rounds.len() as u32 + 1,
            text: text.to_string(),
            seats: want,
            refused,
            syntheses: Vec::new(),
            created_at: Utc::now(),
        });
        self.save_rounds(&c.root.id, &rounds, seq).await?;
        Ok(CouncilRoundsResp {
            rounds,
            max_rounds: COUNCIL_MAX_ROUNDS as u32,
        })
    }

    /// Record a synthesis under the council's current round. Its judge is
    /// already open, so a failed write is logged, not returned.
    pub(super) async fn note_synthesis(
        &self,
        root: &Id,
        mut rounds: Vec<CouncilRound>,
        seq: u64,
        judge: &Id,
    ) {
        if let Some(r) = rounds.last_mut() {
            r.syntheses.push(judge.clone());
        }
        if let Err(e) = self.save_rounds(root, &rounds, seq).await {
            tracing::warn!(council_root = %root, "could not record the synthesis's round: {e}");
        }
    }
}

/// Round 1: the council's first question, to every seat.
pub(super) fn first_round(text: String, seats: Vec<u32>, at: DateTime<Utc>) -> CouncilRound {
    CouncilRound {
        n: 1,
        text,
        seats,
        refused: Vec::new(),
        syntheses: Vec::new(),
        created_at: at,
    }
}

/// The lock a synthesis holds while it reads and records the rounds.
pub(super) async fn rounds_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ROUNDS_LOCK.lock().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_round_takes_its_own_message_and_a_re_ask_replaces_it() {
        // Round 1, a side message the person typed in the seat, round 2.
        let sent = ["q1", "aside", "q2"];
        assert_eq!(round_messages(&sent, &["q1", "q2"]), vec![Some(0), Some(2)]);
        // Round 1 cut off and asked again before round 2: the re-ask counts.
        let sent = ["q1", "q1", "q2"];
        assert_eq!(round_messages(&sent, &["q1", "q2"]), vec![Some(1), Some(2)]);
        // Round 2 still queued: not on the timeline yet.
        assert_eq!(round_messages(&["q1"], &["q1", "q2"]), vec![Some(0), None]);
        // The same follow-up twice is two rounds, in order.
        assert_eq!(
            round_messages(&["q", "again", "again"], &["q", "again", "again"]),
            vec![Some(0), Some(1), Some(2)]
        );
    }

    #[test]
    fn a_refused_seat_is_not_asked_and_errors_lose_their_prefix() {
        let mut r2 = first_round("q2".into(), vec![1, 2], Utc::now());
        r2.n = 2;
        r2.refused.push(CouncilSeatRefusal {
            seat: 2,
            reason: "closed".into(),
        });
        let rounds = vec![first_round("q1".into(), vec![1, 2], Utc::now()), r2];
        assert_eq!(asked_of(&rounds, 1), vec!["q1", "q2"]);
        assert_eq!(asked_of(&rounds, 2), vec!["q1"]);
        assert_eq!(reason(&Error::Conflict("busy".into())), "busy");
        let body: CouncilFollowUpReq =
            serde_json::from_value(serde_json::json!({"text": "why?"})).unwrap();
        assert!(body.seats.is_empty(), "no seats named = every seat");
        let json = serde_json::to_value(&rounds[0]).unwrap();
        assert!(json.get("refused").is_none() && json.get("syntheses").is_none());
    }
}
