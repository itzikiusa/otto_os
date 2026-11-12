//! Threads inside a thread: a person asks another agent from a thread's
//! composer ("Ask another agent", or "Ask several" for 2–4 at once), and
//! every child thread's finished turn comes back to the thread it was opened
//! from.
//!
//! After T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.): a delegated task
//! shows in its parent's timeline (`apps/web/src/components/chat/
//! V2LifecycleRow.tsx` `SubagentTimelineLink` / `SubagentNotificationLink`),
//! and its completion wakes the parent agent
//! (`apps/server/src/orchestration-v2/ProviderContinuationService.ts`,
//! `packages/contracts/src/orchestratorMcp.ts` "Completion wakes this
//! thread"). Otto's shape:
//!
//! * a child carries `meta.child_of {id, by}` — the thread that opened it
//!   (the group's root stays `parent_id`);
//! * a person's ask writes one notice in the parent's timeline ending in
//!   `[otto-thread:<id>[,<id>…]:started]`, which Threads draws as live cards;
//! * each child turn's end writes `… [otto-thread:<id>:finished|failed|stopped]`
//!   there ("Codex finished: <one line>");
//! * when the parent's AGENT opened the child and the turn answered something
//!   the parent asked (its framed opening prompt or `otto.thread_message`),
//!   the answer waits in the parent's queue and goes out at its next idle
//!   moment — at most one per child (a newer answer replaces an older one),
//!   never twice for one turn, visible in the queue bar. A dormant parent is
//!   not respawned for it; a closed one gets nothing. The parent's turn runs
//!   under its own access mode, so its approvals apply as for any prompt.

use otto_core::agent_runtime::{
    AskChildReq, ChildOf, ChildOpener, RuntimeAccessMode, RuntimeProvider, ASK_CHILD_MAX_TARGETS,
    CHILD_META_KEY,
};

use super::*;

/// Characters of a child's answer the wake prompt carries.
const WAKE_MAX_CHARS: usize = 16_000;
/// Characters of the one line a notice quotes.
const NOTICE_LINE_CHARS: usize = 140;
/// Characters of the prompt a child's default title takes.
const TITLE_MAX_CHARS: usize = 60;

/// A queued answer's origin: which child, which of its turns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct WakeKey {
    pub child: Id,
    pub turn: Id,
}

/// `meta.child_of` for a thread `opener` opens.
pub(super) fn child_meta(opener: &Id, by: ChildOpener) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert(
        CHILD_META_KEY.into(),
        serde_json::to_value(ChildOf {
            id: opener.clone(),
            by,
        })
        .unwrap_or_default(),
    );
    serde_json::Value::Object(m)
}

/// Narrow → wide.
fn access_rank(m: RuntimeAccessMode) -> u8 {
    match m {
        RuntimeAccessMode::ApprovalRequired => 0,
        RuntimeAccessMode::AutoAcceptEdits => 1,
        RuntimeAccessMode::FullAccess => 2,
    }
}

fn one_line(s: &str, max: usize) -> String {
    let line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() <= max {
        line.to_string()
    } else {
        format!("{}…", line.chars().take(max).collect::<String>())
    }
}

/// "Claude", "Codex", … for a notice's words (the UI draws its own label).
fn provider_label(p: RuntimeProvider) -> &'static str {
    match p {
        RuntimeProvider::Claude => "Claude",
        RuntimeProvider::Codex => "Codex",
        RuntimeProvider::Gemini => "Gemini",
        RuntimeProvider::Grok => "Grok",
        RuntimeProvider::Agy => "Antigravity",
        RuntimeProvider::Opencode => "opencode",
        RuntimeProvider::Pi => "pi",
    }
}

/// The parent's notice for children a person just asked.
pub(super) fn started_notice(children: &[RuntimeSession], prompt: &str) -> String {
    let who = children
        .iter()
        .map(|c| provider_label(c.provider))
        .collect::<Vec<_>>()
        .join(", ");
    let ids = children
        .iter()
        .map(|c| c.id.as_str())
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "You asked {who}: {} [otto-thread:{ids}:started]",
        one_line(prompt, NOTICE_LINE_CHARS)
    )
}

/// The parent's notice for a child turn that ended.
pub(super) fn ended_notice(
    child: &RuntimeSession,
    status: TurnTerminalStatus,
    answer: &str,
) -> String {
    let who = provider_label(child.provider);
    let (verb, event) = match status {
        TurnTerminalStatus::Completed => ("finished", "finished"),
        TurnTerminalStatus::Failed => ("failed", "failed"),
        TurnTerminalStatus::Interrupted | TurnTerminalStatus::Cancelled => ("stopped", "stopped"),
    };
    let line = one_line(answer, NOTICE_LINE_CHARS);
    let body = if line.is_empty() {
        format!("{who} {verb}")
    } else {
        format!("{who} {verb}: {line}")
    };
    format!("{body} [otto-thread:{}:{event}]", child.id)
}

/// What the parent's agent receives when a child it asked answers.
pub(super) fn wake_text(
    child: &RuntimeSession,
    status: TurnTerminalStatus,
    answer: &str,
) -> String {
    let how = match status {
        TurnTerminalStatus::Completed => "finished",
        TurnTerminalStatus::Failed => "failed",
        TurnTerminalStatus::Interrupted | TurnTerminalStatus::Cancelled => "was stopped",
    };
    let answer = answer.trim();
    let body = if answer.is_empty() {
        "(no answer text)".to_string()
    } else if answer.chars().count() > WAKE_MAX_CHARS {
        format!(
            "{}…\n(cut here — read all of it with otto_thread_read)",
            answer.chars().take(WAKE_MAX_CHARS).collect::<String>()
        )
    } else {
        answer.to_string()
    };
    format!(
        "[Reply from thread \"{}\" ({}, id {}): its turn {how}. Answer it with the \
         otto_thread_message tool (session_id {}); read it again with otto_thread_read.]\n\n{body}",
        child.title,
        child.provider.as_str(),
        child.id,
        child.id
    )
}

/// The turn answered something `parent` asked: its prompt is the framed
/// opening prompt or a `thread_message` from that thread ([`AgentRuntime::peer_header`]).
pub(super) fn asked_by(prompt: &str, parent: &Id) -> bool {
    let first = prompt.lines().next().unwrap_or("");
    (first.starts_with("[Opened by thread ") || first.starts_with("[Message from thread "))
        && first.contains(&format!(", id {parent}). "))
}

/// Called with the lock held when a turn ends: a child's end goes to the
/// thread it was opened from (spawned — the notice and the queue take
/// other sessions' locks).
pub(super) fn on_turn_end(inner: &Inner, st: &State, turn_id: &Id, status: TurnTerminalStatus) {
    let Some(of) = st.session.child_of() else {
        return;
    };
    let Some(me) = inner.me.upgrade() else {
        return;
    };
    let mut answer: Vec<&TurnItem> = st
        .proj
        .items
        .values()
        .filter(|i| i.turn_id.as_ref() == Some(turn_id) && i.parent_item_id.is_none())
        .filter(|i| matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if !text.trim().is_empty()))
        .collect();
    answer.sort_by_key(|i| i.ordinal);
    let answer = answer
        .iter()
        .filter_map(|i| match &i.kind {
            TurnItemKind::AssistantMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let asked = match st
        .proj
        .items
        .get(&format!("{turn_id}:user"))
        .map(|i| &i.kind)
    {
        Some(TurnItemKind::UserMessage { text, .. }) => asked_by(text, &of.id),
        _ => false,
    };
    let child = st.session.clone();
    let turn = turn_id.clone();
    let rt = AgentRuntime { inner: me };
    tokio::spawn(async move {
        rt.child_turn_ended(child, of, turn, status, answer, asked)
            .await;
    });
}

impl AgentRuntime {
    /// `POST /runtime-sessions/{id}/children`: a person sends one message to
    /// 1–4 new threads linked to `caller` — its folder, its access mode or a
    /// narrower one — created all or none; the parent's timeline gets one
    /// notice naming them. A full group is refused (409) with what to do.
    pub async fn ask_children(&self, caller: &Id, req: AskChildReq) -> Result<Vec<RuntimeSession>> {
        let prompt = req.prompt.trim().to_string();
        if prompt.is_empty() && req.images.is_empty() {
            return Err(Error::Invalid("write what to ask first".into()));
        }
        let n = req.targets.len();
        if !(1..=ASK_CHILD_MAX_TARGETS).contains(&n) {
            return Err(Error::Invalid(format!(
                "ask 1 to {ASK_CHILD_MAX_TARGETS} agents at once, not {n}"
            )));
        }
        let c = self.get(caller).await?;
        if c.status == RuntimeSessionStatus::Closed {
            return Err(Error::Conflict(
                "this thread is closed — reopen it first".into(),
            ));
        }
        let mode = req.access_mode.unwrap_or(c.access_mode);
        if access_rank(mode) > access_rank(c.access_mode) {
            return Err(Error::Invalid(format!(
                "a thread opened from this one can't have more access than it ({})",
                c.access_mode.as_str()
            )));
        }
        let open = self.group(&c).await?.len();
        if open + n > MAX_LINKED_OPEN + 1 {
            let room = (MAX_LINKED_OPEN + 1).saturating_sub(open);
            return Err(Error::Conflict(format!(
                "this thread already has {} open threads beside it (at most {MAX_LINKED_OPEN}); \
                 there is room for {room} more — close one before asking another agent",
                open.saturating_sub(1)
            )));
        }
        for t in &req.targets {
            if self.inner.drivers.create(t.provider).is_none() {
                return Err(Error::Invalid(format!(
                    "no structured driver for provider '{}' yet",
                    t.provider.as_str()
                )));
            }
        }
        let base = req
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                let l = one_line(&prompt, TITLE_MAX_CHARS);
                if l.is_empty() {
                    "Question".into()
                } else {
                    l
                }
            });
        let mut made: Vec<RuntimeSession> = Vec::with_capacity(n);
        for t in &req.targets {
            let title = if n > 1 {
                format!("{base} · {}", provider_label(t.provider))
            } else {
                base.clone()
            };
            let opened = self
                .open_linked(
                    caller,
                    OpenPeerReq {
                        provider: t.provider,
                        prompt: None,
                        title: Some(title),
                        model: t.model.clone(),
                        effort: None,
                        cwd: None,
                        meta: Some(child_meta(caller, ChildOpener::Person)),
                    },
                    false,
                )
                .await;
            match opened {
                Ok(s) => made.push(s),
                Err(e) => {
                    self.discard_children(&made).await;
                    return Err(e);
                }
            }
        }
        for (s, t) in made.iter().zip(&req.targets) {
            let p = RuntimePromptReq {
                text: prompt.clone(),
                effort: t.effort.clone().filter(|e| !e.trim().is_empty()),
                images: req.images.clone(),
                // Narrower than the parent's: the child's first turn sets it.
                access_mode: (mode != s.access_mode).then_some(mode),
                ..RuntimePromptReq::default()
            };
            if let Err(e) = self.prompt_with(&s.id, &p).await {
                self.discard_children(&made).await;
                return Err(e);
            }
        }
        let mut out = Vec::with_capacity(n);
        for s in &made {
            out.push(self.get(&s.id).await?);
        }
        if let Err(e) = self.notice(caller, &started_notice(&out, &prompt)).await {
            tracing::warn!(session = %caller, "could not note the asked threads: {e}");
        }
        Ok(out)
    }

    /// Remove children a failed ask already made (none has answered yet).
    async fn discard_children(&self, made: &[RuntimeSession]) {
        for s in made.iter().rev() {
            if let Err(e) = self.delete(&s.id).await {
                tracing::warn!(session = %s.id, "could not remove an asked thread: {e}");
            }
        }
    }

    /// A child's turn ended: note it in the thread it came from, and hand an
    /// answer the parent's agent asked for back to that agent.
    async fn child_turn_ended(
        &self,
        child: RuntimeSession,
        of: ChildOf,
        turn: Id,
        status: TurnTerminalStatus,
        answer: String,
        asked: bool,
    ) {
        let Ok(parent) = self.get(&of.id).await else {
            return;
        };
        if parent.status == RuntimeSessionStatus::Closed {
            return;
        }
        if let Err(e) = self
            .notice(&of.id, &ended_notice(&child, status, &answer))
            .await
        {
            tracing::debug!(session = %of.id, "child notice skipped: {e}");
        }
        if of.by == ChildOpener::Agent && asked {
            let text = wake_text(&child, status, &answer);
            if let Err(e) = self.queue_wake(&of.id, &child.id, &turn, text).await {
                tracing::warn!(session = %of.id, child = %child.id, "could not queue a reply: {e}");
            }
        }
    }

    /// Queue a child's answer for `parent`'s next idle moment: one entry per
    /// child (a newer turn's answer replaces the older one in place), never
    /// the same turn twice. Kicks the queue when the parent's agent runs and
    /// is idle; a dormant parent keeps it queued until it is revived.
    pub(super) async fn queue_wake(
        &self,
        parent: &Id,
        child: &Id,
        turn: &Id,
        text: String,
    ) -> Result<()> {
        let live = self.live(parent).await?;
        let kick = {
            let mut st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed {
                return Ok(());
            }
            let key = WakeKey {
                child: child.clone(),
                turn: turn.clone(),
            };
            let at = st
                .proj
                .queue
                .iter()
                .position(|q| q.wake.as_ref().is_some_and(|w| &w.child == child));
            match at {
                Some(i) if st.proj.queue[i].wake.as_ref() == Some(&key) => return Ok(()),
                Some(i) => {
                    let q = &mut st.proj.queue[i];
                    q.req = RuntimePromptReq::of(&text);
                    q.wake = Some(key);
                }
                None => st.proj.queue.push(QueuedRecord {
                    id: new_id(),
                    created_at: Utc::now(),
                    req: RuntimePromptReq::of(&text),
                    wake: Some(key),
                }),
            }
            emit_queue(&self.inner, &live, &mut st);
            st.proc.is_some() && st.turn.is_none()
        };
        if kick {
            self.kick_queue(parent);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(provider: RuntimeProvider) -> RuntimeSession {
        RuntimeSession {
            id: "kid".into(),
            workspace_id: "w".into(),
            provider,
            title: "Check the tests".into(),
            cwd: "/tmp".into(),
            model: None,
            native_thread_id: None,
            status: RuntimeSessionStatus::Idle,
            access_mode: RuntimeAccessMode::ApprovalRequired,
            created_by: "u".into(),
            created_at: Utc::now(),
            last_active_at: Utc::now(),
            last_sequence: 0,
            parent_id: Some("root".into()),
            ui_control: None,
            background_tasks: Vec::new(),
            meta: Some(child_meta(&"lead".into(), ChildOpener::Agent)),
        }
    }

    #[test]
    fn child_meta_round_trips_through_the_session() {
        let s = session(RuntimeProvider::Codex);
        assert_eq!(
            s.child_of(),
            Some(ChildOf {
                id: "lead".into(),
                by: ChildOpener::Agent
            })
        );
        assert_eq!(s.meta.as_ref().unwrap()["child_of"]["by"], "agent");
    }

    #[test]
    fn notices_carry_a_marker_threads_can_read() {
        let a = session(RuntimeProvider::Codex);
        let mut b = session(RuntimeProvider::Gemini);
        b.id = "kid2".into();
        assert_eq!(
            started_notice(&[a.clone(), b], "  \nWhy does CI fail?\nmore"),
            "You asked Codex, Gemini: Why does CI fail? [otto-thread:kid,kid2:started]"
        );
        assert_eq!(
            ended_notice(
                &a,
                TurnTerminalStatus::Completed,
                "The flaky test.\nDetails"
            ),
            "Codex finished: The flaky test. [otto-thread:kid:finished]"
        );
        assert_eq!(
            ended_notice(&a, TurnTerminalStatus::Interrupted, ""),
            "Codex stopped [otto-thread:kid:stopped]"
        );
        let long = "x".repeat(400);
        assert!(ended_notice(&a, TurnTerminalStatus::Failed, &long).contains("…"));
    }

    #[test]
    fn only_a_prompt_the_parent_sent_counts_as_asked() {
        let header = AgentRuntime::peer_header(&session(RuntimeProvider::Claude), "Opened by");
        // `peer_header` names the session it is given; here "kid".
        assert!(asked_by(&format!("{header}\n\nlook at x"), &"kid".into()));
        assert!(!asked_by(
            &format!("{header}\n\nlook at x"),
            &"other".into()
        ));
        assert!(!asked_by("look at x", &"kid".into()));
        // A delivered reply never counts: no chain of wakes up the group.
        let reply = wake_text(
            &session(RuntimeProvider::Codex),
            TurnTerminalStatus::Completed,
            "ok",
        );
        assert!(!asked_by(&reply, &"kid".into()));
    }

    #[test]
    fn the_wake_names_the_child_and_cuts_a_long_answer() {
        let s = session(RuntimeProvider::Codex);
        let t = wake_text(&s, TurnTerminalStatus::Completed, "All green.");
        assert!(t.starts_with(
            "[Reply from thread \"Check the tests\" (codex, id kid): its turn finished."
        ));
        assert!(t.ends_with("All green."));
        let long = wake_text(
            &s,
            TurnTerminalStatus::Failed,
            &"y".repeat(WAKE_MAX_CHARS + 10),
        );
        assert!(long.contains("its turn failed"));
        assert!(long.contains("cut here"));
        assert!(wake_text(&s, TurnTerminalStatus::Completed, " ").ends_with("(no answer text)"));
    }

    #[test]
    fn access_never_widens() {
        assert!(
            access_rank(RuntimeAccessMode::FullAccess)
                > access_rank(RuntimeAccessMode::AutoAcceptEdits)
        );
        assert!(
            access_rank(RuntimeAccessMode::AutoAcceptEdits)
                > access_rank(RuntimeAccessMode::ApprovalRequired)
        );
    }
}
