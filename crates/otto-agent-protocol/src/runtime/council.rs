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
    CouncilMeta, CouncilRole, CouncilSeatReq, CreateCouncilReq, OpenPeerReq, RuntimeAccessMode,
    SynthesizeCouncilReq, COUNCIL_MAX_SEATS, COUNCIL_META_KEY, COUNCIL_MIN_SEATS,
    PROVIDER_SLUG_META_KEY,
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
    custom_slug: Option<&str>,
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
    if let Some(slug) = custom_slug.map(str::trim).filter(|s| !s.is_empty()) {
        m.insert(
            PROVIDER_SLUG_META_KEY.into(),
            serde_json::Value::String(slug.to_string()),
        );
    }
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
    let p = seat
        .provider_slug
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| seat.provider.as_str());
    match seat
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        Some(m) => format!("{p} {m}"),
        None => p.to_string(),
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

/// One seat's latest exchange as the judge reads it: the last message a
/// person sent it and the answer to that message.
struct SeatAnswer {
    seat: u32,
    provider: String,
    model: Option<String>,
    question: String,
    text: String,
    /// How its last turn ended (`completed`, `interrupted`, …).
    turn_status: Option<String>,
}

impl SeatAnswer {
    /// "seat 2 (interrupted)" — how the judge's missing list names it.
    fn missing_label(&self) -> Option<String> {
        self.text.trim().is_empty().then(|| {
            let how = self.turn_status.as_deref().unwrap_or("no reply");
            format!("seat {} ({how})", self.seat)
        })
    }
}

/// The judge's prompt: the question (one, or each seat's own when they
/// differ), each seat's latest answer, the seats without one (named, never
/// dropped), the task.
fn synthesis_prompt(answers: &[SeatAnswer]) -> String {
    let mut p = format!(
        "You are the judge of a council: {} agents were asked the same question, \
         and their latest answers are below. Do not redo the work and do not change \
         any file. Compare the answers:\n\
         1. Where do they agree?\n\
         2. Where do they disagree, and which side is right (say why)?\n\
         3. Pick the best answer, or merge them into one better answer.\n\
         4. List the open questions none of them settled.\n\n",
        answers.len(),
    );
    let first = answers.first().map_or("", |a| a.question.trim());
    let same = answers.iter().all(|a| a.question.trim() == first);
    if same {
        p.push_str(&format!("## The question\n\n{first}\n"));
    } else {
        p.push_str(
            "Each seat was last asked something of its own; its question is shown \
             with its answer.\n",
        );
    }
    let missing: Vec<String> = answers
        .iter()
        .filter_map(SeatAnswer::missing_label)
        .collect();
    if !missing.is_empty() {
        p.push_str(&format!(
            "\nNo answer from: {}. Name them as missing; do not guess what they \
             would have said.\n",
            missing.join(", ")
        ));
    }
    for a in answers {
        let who = match &a.model {
            Some(m) => format!("{} ({m})", a.provider),
            None => a.provider.to_string(),
        };
        p.push_str(&format!("\n## Seat {} — {who}\n\n", a.seat));
        if !same {
            p.push_str(&format!("Asked: {}\n\n", a.question.trim()));
        }
        if a.text.trim().is_empty() {
            let how = a.turn_status.as_deref().unwrap_or("without a reply");
            p.push_str(&format!("(no answer: its turn ended {how})\n"));
        } else {
            p.push_str(&truncate(a.text.trim(), ANSWER_MAX_CHARS));
            p.push('\n');
            // A seat a Stop or a daemon restart cut off: say so, so the
            // judge does not read a half answer as the seat's last word.
            if let Some(how) = a.turn_status.as_deref().filter(|s| *s != "completed") {
                p.push_str(&format!(
                    "(its last turn ended {how}: this answer may be incomplete)\n"
                ));
            }
        }
    }
    p
}

/// The answer in a timeline stretch (a user message up to the next one):
/// its assistant messages, and how its last turn ended. THE rule for
/// "answered": any assistant text on record, whole or cut off (a Stop keeps
/// the partial text; the judge is told it may be incomplete). Text still
/// streaming when the daemon stopped was never persisted
/// (`Projection::apply`), so a seat a restart cut mid-answer has none.
fn reply_of(items: &[&TurnItem], turns: &HashMap<Id, ProviderTurn>) -> (String, Option<String>) {
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
    let status = items
        .iter()
        .filter_map(|i| i.turn_id.as_ref().and_then(|t| turns.get(t)))
        .max_by_key(|t| t.ordinal)
        .filter(|t| t.status.is_terminal())
        .and_then(|t| serde_json::to_value(t.status).ok())
        .and_then(|v| v.as_str().map(str::to_string));
    (text, status)
}

/// A council: its meta, seat 1, every seat by number, every member.
struct Council {
    meta: CouncilMeta,
    root: RuntimeSession,
    seats: Vec<(u32, RuntimeSession)>,
    members: Vec<RuntimeSession>,
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
        let mut resolved = Vec::with_capacity(n);
        for (i, seat) in req.seats.iter().enumerate() {
            let label = seat_label(seat);
            let (provider, custom_slug) = self
                .resolve_target_provider(seat.provider, seat.provider_slug.as_deref())
                .map_err(|e| seat_error(i + 1, &label, e))?;
            if self.inner.drivers.create(provider).is_none() {
                return Err(seat_error(
                    i + 1,
                    &label,
                    Error::Invalid("no structured driver for this provider yet".into()),
                ));
            }
            resolved.push((provider, custom_slug));
        }
        let council = new_id();
        let base = council_title(req.title.as_deref(), prompt);
        let mut seats: Vec<RuntimeSession> = Vec::with_capacity(n);
        for (i, (seat, (provider, custom_slug))) in req.seats.iter().zip(&resolved).enumerate() {
            let label = seat_label(seat);
            let effort = seat.effort.clone().filter(|e| !e.trim().is_empty());
            let create = CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(*provider),
                cwd: req.cwd.clone(),
                title: Some(format!("{base} · {label}")),
                model: seat.model.clone(),
                access_mode: Some(RuntimeAccessMode::ApprovalRequired),
                meta: Some(council_meta(
                    &council,
                    (i + 1) as u32,
                    CouncilRole::Seat,
                    effort,
                    custom_slug.as_deref(),
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

    /// The council `id` sits on.
    async fn council_of(&self, id: &Id) -> Result<Council> {
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

    /// `seat`'s latest exchange: the last top-level message a person sent
    /// it, and the answer after it (see `reply_of`). A follow-up typed in the
    /// seat's own composer counts like any other message.
    async fn last_exchange(&self, seat: &Id) -> Result<(String, String, Option<String>)> {
        let live = self.live(seat).await?;
        let st = live.lock();
        let mut items: Vec<&TurnItem> = st
            .proj
            .items
            .values()
            .filter(|i| i.parent_item_id.is_none())
            .collect();
        items.sort_by_key(|i| i.ordinal);
        let last = items
            .iter()
            .rposition(|i| matches!(i.kind, TurnItemKind::UserMessage { .. }));
        let Some(at) = last else {
            return Ok((String::new(), String::new(), None));
        };
        let question = match &items[at].kind {
            TurnItemKind::UserMessage { text, .. } => text.clone(),
            _ => String::new(),
        };
        let (text, status) = reply_of(&items[at..], &st.proj.turns);
        Ok((question, text, status))
    }

    /// Open the council's judge: a thread linked to seat 1 whose prompt is
    /// each seat's LATEST exchange (its last question and the answer to it;
    /// a seat without one is named as missing). Refused (409), in this
    /// order: a seat or an earlier synthesis is still working, seat 1 is
    /// closed, the linked group is full, no seat has an answer (see
    /// `reply_of` for what counts).
    pub async fn synthesize_council(
        &self,
        id: &Id,
        req: SynthesizeCouncilReq,
    ) -> Result<RuntimeSession> {
        let c = self.council_of(id).await?;
        for s in c
            .members
            .iter()
            .filter(|s| s.status != RuntimeSessionStatus::Closed)
        {
            let what = match s.council() {
                Some(m) if m.role == CouncilRole::Seat => format!("seat {}", m.seat),
                _ => "the last synthesis".to_string(),
            };
            let busy = self
                .inner
                .live
                .get(&s.id)
                .is_some_and(|l| is_busy(&l.lock()));
            if busy {
                return Err(Error::Conflict(format!(
                    "{what} is still working — wait for it or stop it, then synthesize"
                )));
            }
        }
        if c.root.status == RuntimeSessionStatus::Closed {
            return Err(Error::Conflict(
                "seat 1 is closed — reopen it, then synthesize".into(),
            ));
        }
        // A full group refuses before the answers are read: closing a thread
        // is what the person must do either way.
        self.ensure_group_room(&c.root).await?;
        let (judge_provider, judge_slug) =
            self.resolve_target_provider(req.provider, req.provider_slug.as_deref())?;
        if self.inner.drivers.create(judge_provider).is_none() {
            return Err(Error::Invalid(format!(
                "no structured driver for provider '{}' yet",
                judge_slug
                    .as_deref()
                    .unwrap_or_else(|| judge_provider.as_str())
            )));
        }
        let mut answers = Vec::with_capacity(c.seats.len());
        for (n, s) in &c.seats {
            let (question, text, turn_status) = self.last_exchange(&s.id).await?;
            answers.push(SeatAnswer {
                seat: *n,
                provider: s.provider_slug().to_string(),
                model: s.model.clone(),
                question,
                text,
                turn_status,
            });
        }
        if answers.iter().all(|a| a.text.trim().is_empty()) {
            return Err(Error::Conflict(
                "no seat has answered yet — nothing to synthesize".into(),
            ));
        }
        let base = c
            .root
            .title
            .rsplit_once(" · ")
            .map_or(c.root.title.as_str(), |(b, _)| b);
        let title = format!("{base} · synthesis");
        let effort = req.effort.filter(|e| !e.trim().is_empty());
        self.open_linked(
            &c.root.id,
            OpenPeerReq {
                provider: judge_provider,
                prompt: Some(synthesis_prompt(&answers)),
                title: Some(title),
                model: req.model,
                effort: effort.clone(),
                cwd: None,
                meta: Some(council_meta(
                    &c.meta.id,
                    0,
                    CouncilRole::Synthesis,
                    effort,
                    judge_slug.as_deref(),
                )),
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
            provider_slug: None,
            model: Some("gpt-6.1-sol".into()),
            effort: None,
        };
        assert_eq!(seat_label(&seat), "codex gpt-6.1-sol");
        let custom_seat = CouncilSeatReq {
            provider: otto_core::agent_runtime::RuntimeProvider::Claude,
            provider_slug: Some("claudework".into()),
            model: Some("opus".into()),
            effort: None,
        };
        assert_eq!(seat_label(&custom_seat), "claudework opus");
        let e = seat_error(2, "codex", Error::Conflict("busy".into()));
        assert!(matches!(e, Error::Conflict(m) if m == "seat 2 (codex): busy"));
    }

    fn answer(seat: u32, question: &str, text: &str, status: &str) -> SeatAnswer {
        SeatAnswer {
            seat,
            provider: if seat == 1 { "claude" } else { "codex" }.into(),
            model: (seat == 1).then(|| "opus".into()),
            question: question.into(),
            text: text.into(),
            turn_status: Some(status.into()),
        }
    }

    #[test]
    fn the_judge_prompt_carries_the_question_every_answer_and_the_task() {
        let p = synthesis_prompt(&[
            answer(1, "Which cache?", "Redis", "completed"),
            answer(2, "Which cache?", "", "failed"),
        ]);
        assert!(p.contains("2 agents were asked the same question"));
        assert!(p.contains("## The question\n\nWhich cache?"));
        assert!(p.contains("## Seat 1 — claude (opus)\n\nRedis"));
        assert!(p.contains("## Seat 2 — codex\n\n(no answer: its turn ended failed)"));
        assert!(!p.contains("Asked:"), "one shared question is not repeated");
        assert!(
            !p.contains("may be incomplete"),
            "a completed answer is whole"
        );
        assert!(
            p.contains("No answer from: seat 2 (failed)."),
            "a seat without an answer is named: {p}"
        );
        let cut = synthesis_prompt(&[answer(1, "q", "Redis, beca", "interrupted")]);
        assert!(cut.contains(
            "Redis, beca\n(its last turn ended interrupted: this answer may be incomplete)"
        ));
        assert!(p.contains("agree") && p.contains("disagree") && p.contains("open questions"));
        let meta = council_meta(
            &"c1".into(),
            0,
            CouncilRole::Synthesis,
            Some("high".into()),
            Some("claudework"),
        );
        assert_eq!(meta["council"]["effort"], "high");
        assert_eq!(meta["council"]["role"], "synthesis");
        assert_eq!(meta["read_only"], true);
        assert_eq!(meta["provider_slug"], "claudework");
    }

    /// A seat followed up in its own composer was last asked something
    /// else: the judge reads each seat's own question with its answer.
    #[test]
    fn seats_asked_different_follow_ups_show_each_question() {
        let p = synthesis_prompt(&[
            answer(1, "Which cache?", "Redis", "completed"),
            answer(2, "Why not Memcached?", "Fewer types", "completed"),
        ]);
        assert!(!p.contains("## The question\n"), "{p}");
        assert!(p.contains("## Seat 1 — claude (opus)\n\nAsked: Which cache?\n\nRedis"));
        assert!(p.contains("## Seat 2 — codex\n\nAsked: Why not Memcached?\n\nFewer types"));
    }
}
