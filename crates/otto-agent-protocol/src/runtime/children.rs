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
//!   not respawned for it — not after a restart either; the person's next
//!   message or Revive sends it. A closed one gets nothing: children outlive
//!   it (Threads shows them detached) and keep their answers. The parent's
//!   turn runs under its own access mode, so its approvals apply as for any
//!   prompt;
//! * a child turn a daemon restart cut off reaches the parent the same way,
//!   as `stopped` ("… was interrupted when Otto restarted").

use otto_core::agent_runtime::{
    AskChildReq, ChildOf, ChildOpener, OpenPeerReq, RuntimeAccessMode, RuntimeProvider,
    ASK_CHILD_MAX_TARGETS, CHILD_META_KEY, PROVIDER_SLUG_META_KEY,
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
    child_meta_with_slug(opener, by, None)
}

/// `meta.child_of` (plus optional `provider_slug`) for a thread `opener` opens.
pub(super) fn child_meta_with_slug(
    opener: &Id,
    by: ChildOpener,
    custom_slug: Option<&str>,
) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert(
        CHILD_META_KEY.into(),
        serde_json::to_value(ChildOf {
            id: opener.clone(),
            by,
        })
        .unwrap_or_default(),
    );
    if let Some(slug) = custom_slug.map(str::trim).filter(|s| !s.is_empty()) {
        m.insert(
            PROVIDER_SLUG_META_KEY.into(),
            serde_json::Value::String(slug.to_string()),
        );
    }
    serde_json::Value::Object(m)
}

/// The opener's tool set a subagent keeps (`open_linked`): its MCP profiles,
/// browser opt-in and network profile — and its named account when the
/// subagent runs on the same provider (an account belongs to one CLI). What
/// the opener's request already set wins.
pub(super) fn inherit_tools(
    opener: &RuntimeSession,
    provider: RuntimeProvider,
    meta: &mut Option<serde_json::Value>,
) {
    let Some(from) = opener.meta.as_ref().and_then(|m| m.as_object()) else {
        return;
    };
    let same = opener.provider == provider;
    let m = meta.get_or_insert_with(|| serde_json::json!({}));
    let Some(dst) = m.as_object_mut() else {
        return;
    };
    for k in [
        "mcp_profiles",
        "browser",
        "network_profile_id",
        "account_id",
    ] {
        if k == "account_id" && !same {
            continue;
        }
        if let Some(v) = from.get(k).filter(|v| !v.is_null()) {
            dst.entry(k).or_insert_with(|| v.clone());
        }
    }
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
pub(super) fn provider_label(p: RuntimeProvider) -> &'static str {
    match p {
        RuntimeProvider::Claude => "Claude",
        RuntimeProvider::Codex => "Codex",
        RuntimeProvider::Gemini => "Gemini",
        RuntimeProvider::Grok => "Grok",
        RuntimeProvider::Agy => "Antigravity",
        RuntimeProvider::Opencode => "opencode",
        RuntimeProvider::Pi => "pi",
        RuntimeProvider::Muse => "Muse",
    }
}

fn session_provider_label(s: &RuntimeSession) -> &str {
    let slug = s.provider_slug();
    if slug != s.provider.as_str() {
        slug
    } else {
        provider_label(s.provider)
    }
}

/// The parent's notice for children a person just asked.
pub(super) fn started_notice(children: &[RuntimeSession], prompt: &str) -> String {
    let who = children
        .iter()
        .map(session_provider_label)
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
    let who = session_provider_label(child);
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

/// The parent's notice for a child turn a daemon restart cut off (the card
/// reads it as stopped; the words say why).
pub(super) fn restart_notice(child: &RuntimeSession, answer: &str) -> String {
    let who = session_provider_label(child);
    let line = one_line(answer, NOTICE_LINE_CHARS);
    let body = if line.is_empty() {
        format!("{who} was interrupted when Otto restarted")
    } else {
        format!("{who} was interrupted when Otto restarted: {line}")
    };
    format!("{body} [otto-thread:{}:stopped]", child.id)
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
    wake_text_how(child, how, answer)
}

/// [`wake_text`] with the words for how the turn ended.
fn wake_text_how(child: &RuntimeSession, how: &str, answer: &str) -> String {
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
        "[Reply from subagent \"{}\" ({}, id {}): its turn {how}. Use it and carry on; send it \
         follow-up work with otto_thread_message only if more is needed.]\n\n{body}",
        child.title,
        child.provider_slug(),
        child.id
    )
}

/// The turn answered something `parent` asked: its prompt is the framed
/// opening prompt or a `thread_message` from that thread ([`AgentRuntime::peer_header`]).
pub(super) fn asked_by(prompt: &str, parent: &Id) -> bool {
    let first = prompt.lines().next().unwrap_or("");
    (first.starts_with("[Subagent task from thread ")
        || first.starts_with("[Opened by thread ")
        || first.starts_with("[Message from thread "))
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
    let (answer, asked) = turn_reply(st.proj.items.values(), turn_id, &of.id);
    let child = st.session.clone();
    let turn = turn_id.clone();
    let rt = AgentRuntime { inner: me };
    tokio::spawn(async move {
        rt.child_turn_ended(child, of, turn, status, answer, asked, false)
            .await;
    });
}

/// Turn `turn_id`'s answer (its top-level assistant messages, in order) and
/// whether its prompt was something `parent` asked.
fn turn_reply<'a>(
    items: impl Iterator<Item = &'a TurnItem>,
    turn_id: &Id,
    parent: &Id,
) -> (String, bool) {
    let mut said: Vec<(u32, &str)> = Vec::new();
    let mut asked = false;
    let mut error = None;
    for i in items.filter(|i| i.turn_id.as_ref() == Some(turn_id) && i.parent_item_id.is_none()) {
        match &i.kind {
            TurnItemKind::AssistantMessage { text, .. } if !text.trim().is_empty() => {
                said.push((i.ordinal, text.as_str()));
            }
            TurnItemKind::UserMessage { text, .. } if i.id == format!("{turn_id}:user") => {
                asked = asked_by(text, parent);
            }
            // Why it failed, so the asker can say so instead of retrying blind.
            TurnItemKind::Error { failure } => error = Some(failure.message.as_str()),
            _ => {}
        }
    }
    said.sort_by_key(|(o, _)| *o);
    let mut answer = said
        .iter()
        .map(|(_, t)| *t)
        .collect::<Vec<_>>()
        .join("\n\n");
    if let Some(e) = error {
        if !answer.is_empty() {
            answer.push_str("\n\n");
        }
        answer.push_str(&format!("Error: {e}"));
    }
    (answer, asked)
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
        let mut resolved = Vec::with_capacity(n);
        for t in &req.targets {
            let (provider, custom_slug) =
                self.resolve_target_provider(t.provider, t.provider_slug.as_deref())?;
            if self.inner.drivers.create(provider).is_none() {
                return Err(Error::Invalid(format!(
                    "no structured driver for provider '{}' yet",
                    custom_slug.as_deref().unwrap_or_else(|| provider.as_str())
                )));
            }
            resolved.push((provider, custom_slug));
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
        for (t, (provider, custom_slug)) in req.targets.iter().zip(&resolved) {
            let title = if n > 1 {
                let who = custom_slug
                    .as_deref()
                    .unwrap_or_else(|| provider_label(*provider));
                format!("{base} · {who}")
            } else {
                base.clone()
            };
            let opened = self
                .open_linked(
                    caller,
                    OpenPeerReq {
                        provider: *provider,
                        prompt: None,
                        title: Some(title),
                        model: t.model.clone(),
                        effort: None,
                        cwd: None,
                        meta: Some(child_meta_with_slug(
                            caller,
                            ChildOpener::Person,
                            custom_slug.as_deref(),
                        )),
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
    /// answer the parent's agent asked for back to that agent. `restarted` =
    /// a daemon restart cut the turn off (boot recovery reports it).
    #[allow(clippy::too_many_arguments)]
    async fn child_turn_ended(
        &self,
        child: RuntimeSession,
        of: ChildOf,
        turn: Id,
        status: TurnTerminalStatus,
        answer: String,
        asked: bool,
        restarted: bool,
    ) {
        let Ok(parent) = self.get(&of.id).await else {
            return;
        };
        // A closed parent gets nothing: its children outlive it (shown as
        // detached), their answers stay in their own threads.
        if parent.status == RuntimeSessionStatus::Closed {
            return;
        }
        let notice = if restarted {
            restart_notice(&child, &answer)
        } else {
            ended_notice(&child, status, &answer)
        };
        if let Err(e) = self.notice(&of.id, &notice).await {
            tracing::debug!(session = %of.id, "child notice skipped: {e}");
        }
        if of.by == ChildOpener::Agent && asked {
            let text = if restarted {
                wake_text_how(&child, "was interrupted when Otto restarted", &answer)
            } else {
                wake_text(&child, status, &answer)
            };
            if let Err(e) = self.queue_wake(&of.id, &child.id, &turn, text).await {
                tracing::warn!(session = %of.id, child = %child.id, "could not queue a reply: {e}");
            }
        }
    }

    /// Boot recovery: each child whose turn the restart cut off (already
    /// settled `interrupted`) tells its parent once — the notice its card
    /// reads, and for an agent's own ask the reply queued like any other. A
    /// dormant parent is not respawned for it (`drain_queue`).
    pub(super) async fn report_restart_cut(&self, cut: Vec<(RuntimeSession, Id)>) {
        for (child, turn) in cut {
            let Some(of) = child.child_of() else {
                continue;
            };
            let items: Vec<TurnItem> = match self.inner.repo.entities(&child.id).await {
                Ok(rows) => rows
                    .into_iter()
                    .filter(|r| r.kind == K_ITEM)
                    .filter_map(|r| serde_json::from_value(r.payload).ok())
                    .collect(),
                Err(e) => {
                    tracing::warn!(session = %child.id, "could not read an interrupted child: {e}");
                    continue;
                }
            };
            let (answer, asked) = turn_reply(items.iter(), &turn, &of.id);
            self.child_turn_ended(
                child,
                of,
                turn,
                TurnTerminalStatus::Interrupted,
                answer,
                asked,
                true,
            )
            .await;
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
        let key = WakeKey {
            child: child.clone(),
            turn: turn.clone(),
        };
        let parent_row = {
            let st = live.lock();
            let queued = st.proj.queue.iter().any(|q| q.wake.as_ref() == Some(&key));
            if st.session.status == RuntimeSessionStatus::Closed || queued {
                return Ok(());
            }
            st.session.clone()
        };
        // A reply to an agent is an agent-to-agent message: past the group's
        // cap it is held in the queue (kept, not sent) until the person sends it.
        let error = self
            .admit_agent_prompt(&parent_row)
            .await
            .err()
            .map(|_| loop_guard::PAUSED_QUEUED.to_string());
        let kick = {
            let mut st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed {
                return Ok(());
            }
            let at = st
                .proj
                .queue
                .iter()
                .position(|q| q.wake.as_ref().is_some_and(|w| &w.child == child));
            match at {
                Some(i) if st.proj.queue[i].wake.as_ref() == Some(&key) => return Ok(()),
                Some(i) => {
                    let q = &mut st.proj.queue[i];
                    q.req = Arc::new(RuntimePromptReq::of(&text));
                    q.wake = Some(key);
                    q.error = error.clone();
                }
                None => st.proj.queue.push(QueuedRecord {
                    id: new_id(),
                    created_at: Utc::now(),
                    req: Arc::new(RuntimePromptReq::of(&text)),
                    wake: Some(key),
                    error: error.clone(),
                }),
            }
            emit_queue(&self.inner, &live, &mut st);
            error.is_none() && st.proc.is_some() && st.turn.is_none()
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
            title_auto: false,
            id: "kid".into(),
            workspace_id: "w".into(),
            provider,
            title: "Check the tests".into(),
            cwd: "/tmp".into(),
            model: None,
            effort: None,
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
            capabilities: Default::default(),
            meta: Some(child_meta(&"lead".into(), ChildOpener::Agent)),
            last_turn_status: None,
            last_error_class: None,
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
        assert_eq!(
            restart_notice(&a, "Half way.\nmore"),
            "Codex was interrupted when Otto restarted: Half way. [otto-thread:kid:stopped]"
        );
        assert_eq!(
            restart_notice(&a, ""),
            "Codex was interrupted when Otto restarted [otto-thread:kid:stopped]"
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
    fn a_turns_reply_is_its_own_top_level_text_in_order() {
        let header = AgentRuntime::peer_header(&session(RuntimeProvider::Claude), "Opened by");
        let at = |id: &str, turn: &str, ordinal: u32, kind: TurnItemKind| {
            let mut i = item(id.into(), TurnItemStatus::Completed, None, kind);
            i.turn_id = Some(turn.into());
            i.ordinal = ordinal;
            i
        };
        let said = |text: &str| TurnItemKind::AssistantMessage {
            message_id: "m".into(),
            text: text.into(),
            streaming: false,
        };
        let mut sub = at("t1:s", "t1", 4, said("a subagent's own words"));
        sub.parent_item_id = Some("card".into());
        let items = [
            at(
                "t1:user",
                "t1",
                1,
                TurnItemKind::UserMessage {
                    text: format!("{header}\n\nlook"),
                    attachments: Vec::new(),
                },
            ),
            at("t1:b", "t1", 3, said("second")),
            at("t1:a", "t1", 2, said("first")),
            at("t0:a", "t0", 0, said("an older turn")),
            sub,
        ];
        let (answer, asked) = turn_reply(items.iter(), &"t1".into(), &"kid".into());
        assert_eq!(answer, "first\n\nsecond");
        assert!(asked, "the framed prompt came from `kid`");
        let (_, asked) = turn_reply(items.iter(), &"t1".into(), &"other".into());
        assert!(!asked);
        assert_eq!(
            turn_reply(items.iter(), &"t9".into(), &"kid".into()),
            (String::new(), false)
        );
    }

    #[test]
    fn a_failed_turns_reply_carries_its_error() {
        let mut e = item(
            "t1:error".into(),
            TurnItemStatus::Failed,
            None,
            TurnItemKind::Error {
                failure: otto_core::agent_runtime::ProviderFailure {
                    class: "auth_required".into(),
                    message: "log in to Gemini first".into(),
                    code: None,
                    retryable: None,
                },
            },
        );
        e.turn_id = Some("t1".into());
        let (answer, _) = turn_reply([e].iter(), &"t1".into(), &"kid".into());
        assert_eq!(answer, "Error: log in to Gemini first");
    }

    #[test]
    fn the_wake_names_the_child_and_cuts_a_long_answer() {
        let s = session(RuntimeProvider::Codex);
        let t = wake_text(&s, TurnTerminalStatus::Completed, "All green.");
        assert!(t.starts_with(
            "[Reply from subagent \"Check the tests\" (codex, id kid): its turn finished."
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
    fn a_subagent_keeps_the_openers_tools_and_its_account_only_on_the_same_cli() {
        let mut lead = session(RuntimeProvider::Claude);
        lead.meta = Some(serde_json::json!({
            "mcp_profiles": ["db"], "browser": true, "network_profile_id": "vpn",
            "account_id": "work", "personal_agent": "ops",
        }));
        let mut meta = Some(child_meta(&lead.id, ChildOpener::Agent));
        inherit_tools(&lead, RuntimeProvider::Codex, &mut meta);
        let m = meta.unwrap();
        assert_eq!(m["mcp_profiles"], serde_json::json!(["db"]));
        assert_eq!(m["browser"], true);
        assert_eq!(m["network_profile_id"], "vpn");
        assert!(m.get("account_id").is_none(), "another CLI's account");
        assert!(m.get("personal_agent").is_none());
        assert_eq!(m["child_of"]["id"], "kid");
        let mut same = Some(serde_json::json!({"account_id": "mine"}));
        inherit_tools(&lead, RuntimeProvider::Claude, &mut same);
        assert_eq!(
            same.unwrap()["account_id"],
            "mine",
            "the request's own pick wins"
        );
        let mut same = None;
        inherit_tools(&lead, RuntimeProvider::Claude, &mut same);
        assert_eq!(same.unwrap()["account_id"], "work");
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
