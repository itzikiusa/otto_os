//! Council: one prompt sent to 2–4 agents at once (different providers, or
//! the same one twice with another model or effort), their answers shown side
//! by side, then one judge thread that compares them.
//!
//! The idea is T3 Code's multi-model send (MIT, Copyright (c) 2026 T3 Tools
//! Inc.: `apps/web/src/components/chat/useMultipleModelSend.ts`,
//! `chatViewSend.ts` `startFanoutThread`): one send starts one thread per
//! model with the same prompt. Otto keeps the seats together as ONE linked
//! group (seat 1 is the root, seats 2–4 are linked to it, every thread carries
//! `meta.council`), so the person reads them as one conversation, and adds
//! the synthesis T3 Code leaves to the person.
//!
//! Version 1 seats only read (`meta.read_only` forces the read-only Seatbelt
//! profile and tool list; access is approval-required): parallel agents in
//! one folder must not clash on files. A council is started and watched by a
//! person, so it is not a background source and takes no admission slot.

use otto_core::agent_runtime::{
    CouncilMeta, CouncilRole, CouncilSeatReq, CreateCouncilReq, RuntimeAccessMode,
    SynthesizeCouncilReq, COUNCIL_MAX_SEATS, COUNCIL_META_KEY, COUNCIL_MIN_SEATS,
};

use super::*;

/// Characters of one seat's answer the synthesis prompt carries.
const ANSWER_MAX_CHARS: usize = 24_000;
/// Characters of the prompt a council's default title takes.
const TITLE_MAX_CHARS: usize = 60;

/// The meta every council thread runs under.
fn council_meta(
    council: &Id,
    seat: u32,
    role: CouncilRole,
    effort: Option<String>,
) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    let council = CouncilMeta {
        id: council.clone(),
        seat,
        role,
        effort,
    };
    m.insert(
        COUNCIL_META_KEY.into(),
        serde_json::to_value(council).unwrap_or_default(),
    );
    // Version 1: every seat only reads, so parallel agents never clash on files.
    m.insert("read_only".into(), serde_json::Value::Bool(true));
    serde_json::Value::Object(m)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// The council's title: the person's, else the prompt's first line.
fn council_title(title: Option<&str>, prompt: &str) -> String {
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => {
            let line = prompt
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("Council");
            truncate(line, TITLE_MAX_CHARS)
        }
    }
}

/// "codex gpt-6.1-sol" / "claude" — how errors and titles name a seat.
fn seat_label(seat: &CouncilSeatReq) -> String {
    match seat
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(m) => format!("{} {m}", seat.provider.as_str()),
        None => seat.provider.as_str().to_string(),
    }
}

/// `e` with the seat it happened on, same status.
fn seat_error(n: usize, label: &str, e: Error) -> Error {
    let at = |m: String| format!("seat {n} ({label}): {m}");
    match e {
        Error::NotFound(m) => Error::NotFound(at(m)),
        Error::Unauthorized => Error::Unauthorized,
        Error::Forbidden(m) => Error::Forbidden(at(m)),
        Error::Conflict(m) => Error::Conflict(at(m)),
        Error::Invalid(m) => Error::Invalid(at(m)),
        Error::PayloadTooLarge(m) => Error::PayloadTooLarge(at(m)),
        Error::UnsupportedMedia(m) => Error::UnsupportedMedia(at(m)),
        Error::Upstream(m) => Error::Upstream(at(m)),
        Error::Internal(m) => Error::Internal(at(m)),
    }
}

/// One seat's answer as the judge reads it.
struct SeatAnswer {
    seat: u32,
    provider: &'static str,
    model: Option<String>,
    text: String,
    turn_status: Option<String>,
}

/// The judge's prompt: the question, each seat's last answer, the task.
fn synthesis_prompt(question: &str, answers: &[SeatAnswer]) -> String {
    let mut p = format!(
        "You are the judge of a council: the same question went to {} agents, \
         and their answers are below. Do not redo the work and do not change any \
         file. Compare the answers:\n\
         1. Where do they agree?\n\
         2. Where do they disagree, and which side is right (say why)?\n\
         3. Pick the best answer, or merge them into one better answer.\n\
         4. List the open questions none of them settled.\n\n\
         ## The question\n\n{}\n",
        answers.len(),
        question.trim()
    );
    for a in answers {
        let who = match &a.model {
            Some(m) => format!("{} ({m})", a.provider),
            None => a.provider.to_string(),
        };
        p.push_str(&format!("\n## Seat {} — {who}\n\n", a.seat));
        if a.text.trim().is_empty() {
            let how = a.turn_status.as_deref().unwrap_or("without a reply");
            p.push_str(&format!("(no answer: its turn ended {how})\n"));
        } else {
            p.push_str(&truncate(a.text.trim(), ANSWER_MAX_CHARS));
            p.push('\n');
        }
    }
    p
}

impl AgentRuntime {
    /// Start a council: 2–4 seats, created all or none (a seat that fails to
    /// start removes the ones already made), then the prompt goes to every
    /// seat at once. Returns the seats, seat 1 (the group's root) first.
    pub async fn create_council(
        &self,
        user: &Id,
        ws_id: &Id,
        req: CreateCouncilReq,
    ) -> Result<Vec<RuntimeSession>> {
        let prompt = req.prompt.trim();
        if prompt.is_empty() {
            return Err(Error::Invalid("a council needs a prompt".into()));
        }
        let n = req.seats.len();
        if !(COUNCIL_MIN_SEATS..=COUNCIL_MAX_SEATS).contains(&n) {
            return Err(Error::Invalid(format!(
                "a council has {COUNCIL_MIN_SEATS} to {COUNCIL_MAX_SEATS} seats, not {n}"
            )));
        }
        // An unknown provider is refused before anything is spawned.
        for (i, seat) in req.seats.iter().enumerate() {
            if self.inner.drivers.create(seat.provider).is_none() {
                return Err(seat_error(
                    i + 1,
                    &seat_label(seat),
                    Error::Invalid("no structured driver for this provider yet".into()),
                ));
            }
        }
        let council = new_id();
        let base = council_title(req.title.as_deref(), prompt);
        let mut seats: Vec<RuntimeSession> = Vec::with_capacity(n);
        for (i, seat) in req.seats.iter().enumerate() {
            let label = seat_label(seat);
            let effort = seat.effort.clone().filter(|e| !e.trim().is_empty());
            let create = CreateRuntimeSessionReq {
                provider: Some(seat.provider),
                cwd: req.cwd.clone(),
                title: Some(format!("{base} · {label}")),
                model: seat.model.clone(),
                access_mode: Some(RuntimeAccessMode::ApprovalRequired),
                meta: Some(council_meta(
                    &council,
                    (i + 1) as u32,
                    CouncilRole::Seat,
                    effort,
                )),
            };
            let root = seats.first().map(|s| s.id.clone());
            match self.create_linked(user, ws_id, create, root).await {
                Ok(s) => seats.push(s),
                Err(e) => {
                    self.discard_seats(&seats).await;
                    return Err(seat_error(i + 1, &label, e));
                }
            }
        }
        let mut sends = Vec::with_capacity(n);
        for (s, seat) in seats.iter().zip(&req.seats) {
            let rt = self.clone();
            let id = s.id.clone();
            let p = RuntimePromptReq {
                text: prompt.to_string(),
                effort: seat.effort.clone().filter(|e| !e.trim().is_empty()),
                images: req.images.clone(),
                ..RuntimePromptReq::default()
            };
            sends.push(tokio::spawn(async move { rt.prompt_with(&id, &p).await }));
        }
        let mut failed = None;
        for (i, send) in sends.into_iter().enumerate() {
            let r = send
                .await
                .unwrap_or_else(|e| Err(Error::Internal(e.to_string())));
            if let (Err(e), true) = (r, failed.is_none()) {
                failed = Some(seat_error(i + 1, &seat_label(&req.seats[i]), e));
            }
        }
        if let Some(e) = failed {
            self.discard_seats(&seats).await;
            return Err(e);
        }
        let mut out = Vec::with_capacity(n);
        for s in &seats {
            out.push(self.get(&s.id).await?);
        }
        Ok(out)
    }

    /// Remove seats a failed council start already made (linked seats
    /// first, the root last). Nothing of the person's is lost: no seat has
    /// answered yet.
    async fn discard_seats(&self, seats: &[RuntimeSession]) {
        for s in seats.iter().rev() {
            if let Err(e) = self.delete(&s.id).await {
                tracing::warn!(session = %s.id, "could not remove a council seat: {e}");
            }
        }
    }

    /// Every thread of `id`'s council (seats and syntheses, closed ones
    /// too), and that council's meta.
    async fn council_members(
        &self,
        me: &RuntimeSession,
    ) -> Result<(CouncilMeta, Vec<RuntimeSession>)> {
        let council = me
            .council()
            .ok_or_else(|| Error::Invalid("this thread is not part of a council".into()))?;
        let root = me.parent_id.clone().unwrap_or_else(|| me.id.clone());
        let members = self
            .list(&me.workspace_id, Some(&me.created_by))
            .await?
            .into_iter()
            .filter(|s| s.id == root || s.parent_id.as_ref() == Some(&root))
            .filter(|s| s.council().is_some_and(|c| c.id == council.id))
            .collect();
        Ok((council, members))
    }

    /// The first message a person sent to `id` (a council's question).
    async fn first_user_text(&self, id: &Id) -> Result<String> {
        let live = self.live(id).await?;
        let st = live.lock();
        let text = st
            .proj
            .items
            .values()
            .filter(|i| i.parent_item_id.is_none())
            .filter_map(|i| match &i.kind {
                TurnItemKind::UserMessage { text, .. } => Some((i.ordinal, text)),
                _ => None,
            })
            .min_by_key(|(ordinal, _)| *ordinal)
            .map(|(_, t)| t.clone())
            .unwrap_or_default();
        Ok(text)
    }

    /// Open the council's judge: a thread linked to seat 1 whose prompt is
    /// the question plus each seat's last answer. Refused (409), in this
    /// order: a seat or an earlier synthesis is still working, seat 1 is
    /// closed, the linked group is full, no seat has answered yet.
    pub async fn synthesize_council(
        &self,
        id: &Id,
        req: SynthesizeCouncilReq,
    ) -> Result<RuntimeSession> {
        let me = self.get(id).await?;
        let (council, members) = self.council_members(&me).await?;
        for s in members
            .iter()
            .filter(|s| s.status != RuntimeSessionStatus::Closed)
        {
            let busy = self
                .inner
                .live
                .get(&s.id)
                .is_some_and(|l| is_busy(&l.lock()));
            if busy {
                let what = match s.council() {
                    Some(c) if c.role == CouncilRole::Seat => format!("seat {}", c.seat),
                    _ => "the last synthesis".to_string(),
                };
                return Err(Error::Conflict(format!(
                    "{what} is still working — wait for it or stop it, then synthesize"
                )));
            }
        }
        let mut seats: Vec<(u32, &RuntimeSession)> = members
            .iter()
            .filter_map(|s| {
                s.council()
                    .filter(|c| c.role == CouncilRole::Seat)
                    .map(|c| (c.seat, s))
            })
            .collect();
        seats.sort_by_key(|(n, _)| *n);
        let root = seats
            .iter()
            .find(|(_, s)| s.parent_id.is_none())
            .map(|(_, s)| (*s).clone())
            .ok_or_else(|| Error::NotFound("the council's first seat is gone".into()))?;
        if root.status == RuntimeSessionStatus::Closed {
            return Err(Error::Conflict(
                "seat 1 is closed — reopen it, then synthesize".into(),
            ));
        }
        // A full group refuses before the answers are read: closing a thread
        // is what the person must do either way.
        self.ensure_group_room(&root).await?;
        let mut answers = Vec::with_capacity(seats.len());
        for (n, s) in &seats {
            let reply = self.latest_reply(&s.id, Duration::ZERO).await?;
            answers.push(SeatAnswer {
                seat: *n,
                provider: s.provider.as_str(),
                model: s.model.clone(),
                text: reply.text,
                turn_status: reply.turn_status,
            });
        }
        if answers.iter().all(|a| a.text.trim().is_empty()) {
            return Err(Error::Conflict(
                "no seat has answered yet — nothing to synthesize".into(),
            ));
        }
        let question = self.first_user_text(&root.id).await?;
        let base = root
            .title
            .rsplit_once(" · ")
            .map_or(root.title.as_str(), |(b, _)| b)
            .to_string();
        let effort = req.effort.filter(|e| !e.trim().is_empty());
        self.open_linked(
            &root.id,
            OpenPeerReq {
                provider: req.provider,
                prompt: Some(synthesis_prompt(&question, &answers)),
                title: Some(format!("{base} · synthesis")),
                model: req.model,
                effort: effort.clone(),
                cwd: None,
                meta: Some(council_meta(&council.id, 0, CouncilRole::Synthesis, effort)),
            },
            false,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_labels_and_errors_name_the_seat() {
        assert_eq!(council_title(Some("  Cache  "), "x"), "Cache");
        assert_eq!(
            council_title(None, "\n  which cache?\nmore"),
            "which cache?"
        );
        assert_eq!(council_title(None, &"a".repeat(80)).chars().count(), 61);
        let seat = CouncilSeatReq {
            provider: otto_core::agent_runtime::RuntimeProvider::Codex,
            model: Some("gpt-6.1-sol".into()),
            effort: None,
        };
        assert_eq!(seat_label(&seat), "codex gpt-6.1-sol");
        let e = seat_error(2, "codex", Error::Conflict("busy".into()));
        assert!(matches!(e, Error::Conflict(m) if m == "seat 2 (codex): busy"));
    }

    #[test]
    fn the_judge_prompt_carries_the_question_every_answer_and_the_task() {
        let p = synthesis_prompt(
            "Which cache?",
            &[
                SeatAnswer {
                    seat: 1,
                    provider: "claude",
                    model: Some("opus".into()),
                    text: "Redis".into(),
                    turn_status: Some("completed".into()),
                },
                SeatAnswer {
                    seat: 2,
                    provider: "codex",
                    model: None,
                    text: String::new(),
                    turn_status: Some("failed".into()),
                },
            ],
        );
        assert!(p.contains("same question went to 2 agents"));
        assert!(p.contains("## The question\n\nWhich cache?"));
        assert!(p.contains("## Seat 1 — claude (opus)\n\nRedis"));
        assert!(p.contains("## Seat 2 — codex\n\n(no answer: its turn ended failed)"));
        assert!(p.contains("agree") && p.contains("disagree") && p.contains("open questions"));
        let meta = council_meta(&"c1".into(), 0, CouncilRole::Synthesis, Some("high".into()));
        assert_eq!(meta["council"]["effort"], "high");
        assert_eq!(meta["council"]["role"], "synthesis");
        assert_eq!(meta["read_only"], true);
    }
}
