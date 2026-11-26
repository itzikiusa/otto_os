//! Unattended turns: send one prompt and wait until the thread has SETTLED —
//! no turn, no background work, nothing queued or about to be queued, no
//! pending request, and no busy agent-opened sub-thread anywhere below it.
//! The engines that run an agent with nobody watching (workflow steps, review
//! agents, personal agents) wait here instead of polling.
//!
//! Event-driven: the waiter re-evaluates whenever any watched thread emits
//! (`Live::deadline_changed`, which [`super::emit`] fires for every event).
//! The one timed wait is [`AUTO_TURN_GRACE`] after background work ends with
//! no turn running: Claude and Codex start a turn of their own then.

use std::collections::HashSet;
use std::pin::Pin;

use otto_agent_run::step::StepEvent;
use otto_core::agent_runtime::{
    ApprovalDecision, Attention, AttentionKind, AttentionVia, ChildOpener, QuestionAnswer,
    RequestKind, SubThreadUse, ThreadTokenUsage, TurnTokenUsage,
};
use otto_core::cancel_signal::CancelSignal;

use super::*;

/// How long a thread whose background work ended may take to start the
/// agent's own follow-up turn before it counts as settled.
pub const AUTO_TURN_GRACE: Duration = Duration::from_secs(10);
/// An engine thread's provider could not resume its conversation: the step
/// fails rather than going on with an agent that remembers nothing.
pub const RESUME_REFUSED: &str = "the agent could not resume its earlier conversation";
/// Background work a settled step may still wait on, at most.
pub const BACKGROUND_LIMIT: Duration = Duration::from_secs(45 * 60);

/// How a wait may end early.
#[derive(Clone)]
pub struct TurnOpts {
    pub cancel: CancelSignal,
    /// The step's hard budget (an unanswered wait included).
    pub deadline: Option<Duration>,
    /// No event for this long (paused while a request waits on a person or
    /// background work runs) ends it.
    pub no_progress: Option<Duration>,
    pub background_limit: Duration,
    /// A pre-admitted machine place for the spawn this turn may need (a
    /// dormant or closed thread respawning); claimed by the spawn.
    pub permit: Option<String>,
}

impl TurnOpts {
    pub fn new(cancel: CancelSignal) -> Self {
        Self {
            cancel,
            deadline: None,
            no_progress: None,
            background_limit: BACKGROUND_LIMIT,
            permit: None,
        }
    }
}

/// Why [`AgentRuntime::await_settled`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    Settled,
    Cancelled,
    Deadline,
    NoProgress,
    BackgroundLimit,
    /// The sub-thread reply loop guard held a reply back: a person must send it.
    LoopGuard,
    /// A daemon restart cut the thread's turn off (it was not held).
    ServerRestart,
}

/// What the thread did between `start_ordinal` and settle.
#[derive(Debug, Clone)]
pub struct SettledTurn {
    pub ended: Ended,
    /// The last turn's status (`None` when no turn ran).
    pub turn_status: Option<ProviderTurnStatus>,
    pub failure: Option<ProviderFailure>,
    /// The newest turn's top-level answer that is not empty.
    pub answer: String,
    pub start_ordinal: u32,
    pub turns: u32,
    pub usage: TurnTokenUsage,
    pub sub_threads: Vec<SubThreadUse>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context_usage: Option<ThreadTokenUsage>,
    /// What waited on a person when the wait ended (a deadline's message).
    pub waited_on: Option<String>,
}

/// `meta` key: the turn a daemon restart cut off on an engine thread that
/// was not held, or whose background work it killed (`recover_excluding`).
/// Persisted, so a second restart before the step resumes still reads it.
pub const RESTART_CUT_KEY: &str = "restart_cut_turn";

/// The latest turn above `since` is the one a restart cut off.
fn restart_cut(st: &State, since: u32) -> bool {
    let Some(cut) = st
        .session
        .meta
        .as_ref()
        .and_then(|m| m.get(RESTART_CUT_KEY))
        .and_then(|v| v.as_str())
    else {
        return false;
    };
    st.proj
        .turns
        .values()
        .max_by_key(|t| t.ordinal)
        .is_some_and(|t| t.ordinal > since && t.id == cut)
}

/// One request waiting on a person: `(thread, request)` and what to show.
struct Wait {
    key: (Id, Id),
    attention: Attention,
}

/// One evaluation of a thread and its open agent-opened descendants.
#[derive(Default)]
struct Look {
    turn: bool,
    background: bool,
    /// A queued message that will go out (not one held by the loop guard).
    ready_queue: bool,
    held_wake: bool,
    pending_wakes: bool,
    pending: bool,
    closed: bool,
    last_activity: Option<Instant>,
    background_ended: Option<Instant>,
    descendants_busy: bool,
    waits: Vec<Wait>,
    descendants: Vec<(Id, Arc<Live>)>,
}

impl Look {
    fn settled(&self, now: Instant) -> bool {
        !self.turn
            && !self.background
            && !self.ready_queue
            && !self.pending_wakes
            && !self.pending
            && !self.descendants_busy
            && !self.held_wake
            && self
                .background_ended
                .is_none_or(|t| now >= t + AUTO_TURN_GRACE)
    }
}

/// A request's question or prompt, as the run shows it.
fn request_text(st: &State, r: &RuntimeRequest) -> (AttentionKind, String) {
    if matches!(r.kind, RequestKind::Question | RequestKind::McpElicitation) {
        let q = r
            .question_title
            .clone()
            .or_else(|| r.questions.first().map(|q| q.prompt.clone()))
            .unwrap_or_default();
        return (AttentionKind::Question, format!("asks: {}", q.trim()));
    }
    let prompt = st
        .proj
        .request_items
        .get(&r.id)
        .and_then(|i| st.proj.items.get(i))
        .and_then(|i| match &i.kind {
            TurnItemKind::ApprovalRequest { prompt, reason, .. } => {
                prompt.clone().or_else(|| reason.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| format!("{:?}", r.kind).to_lowercase());
    (
        AttentionKind::Approval,
        format!("wants approval: {}", prompt.trim()),
    )
}

/// What a person answered, in words.
fn answer_text(r: &RuntimeRequest) -> Option<String> {
    if let Some(answers) = &r.answers {
        let line = |a: &QuestionAnswer| {
            let (qid, v) = match a {
                QuestionAnswer::Single { question_id, value } => (question_id, value.clone()),
                QuestionAnswer::Multi {
                    question_id,
                    values,
                } => (question_id, values.join(", ")),
                QuestionAnswer::Text { question_id, text } => (question_id, text.clone()),
            };
            let q = r.questions.iter().find(|q| &q.id == qid);
            format!("{}: {v}", q.map_or(qid.as_str(), |q| q.prompt.trim()))
        };
        return Some(answers.iter().map(line).collect::<Vec<_>>().join("\n"));
    }
    r.decision.map(|d| {
        match d {
            ApprovalDecision::Accept => "approved",
            ApprovalDecision::AcceptForSession => "approved for the session",
            ApprovalDecision::AcceptAlways => "approved always",
            ApprovalDecision::Decline | ApprovalDecision::DeclineAlways => "declined",
            ApprovalDecision::Cancel => "canceled",
        }
        .to_string()
    })
}

/// Whether `st` has something queued that will go out on its own
/// ([`super::drain`]'s rule), and whether the loop guard holds a reply.
fn queue_state(st: &State) -> (bool, bool) {
    let held = st
        .proj
        .queue
        .iter()
        .any(|q| q.error.as_deref() == Some(loop_guard::PAUSED_QUEUED));
    let mut ready = st.proj.queue.iter().filter(|q| q.error.is_none());
    let goes_out = if st.proc.is_none() {
        ready.any(|q| q.wake.is_none())
    } else {
        ready.next().is_some()
    };
    (goes_out, held)
}

fn wait_of(st: &State, r: &RuntimeRequest, thread: &Id) -> Wait {
    let (kind, text) = request_text(st, r);
    let mut a = Attention::new(kind, &text, r.created_at);
    if kind == AttentionKind::Approval {
        a.via = Some(AttentionVia::Provider);
    }
    a.thread_id = Some(thread.clone());
    a.request_id = Some(r.id.clone());
    Wait {
        key: (thread.clone(), r.id.clone()),
        attention: a,
    }
}

impl AgentRuntime {
    /// Send `req` as a new turn and wait until the thread settles. Refused
    /// (409) while a turn runs: an unattended caller never queues.
    pub async fn run_turn(
        &self,
        id: &Id,
        req: &RuntimePromptReq,
        opts: TurnOpts,
        mut on: impl FnMut(StepEvent) + Send,
    ) -> Result<SettledTurn> {
        let since = {
            let live = self.live(id).await?;
            let st = live.lock();
            if st.turn.is_some() {
                return Err(Error::Conflict(TURN_RUNNING.into()));
            }
            if let Some(key) = &opts.permit {
                *live.permit.lock().unwrap_or_else(|e| e.into_inner()) = Some(key.clone());
            }
            st.proj.next_turn_ordinal
        };
        self.prompt_with(id, req).await?;
        on(StepEvent::Working);
        self.settle(id, since, opts, on, Some(true)).await
    }

    /// Wait until the thread settles; usage and turns count every turn with
    /// an ordinal above `since`. Emits `Working` / `Finishing` as the turn
    /// runs and ends, `NeedsYou` for each request waiting on a person (a
    /// request already pending on entry too) and `Resumed` when it clears.
    pub async fn await_settled(
        &self,
        id: &Id,
        since: u32,
        opts: TurnOpts,
        on: impl FnMut(StepEvent) + Send,
    ) -> Result<SettledTurn> {
        self.settle(id, since, opts, on, None).await
    }

    /// `working`: the phase already reported (`run_turn` reports its turn).
    async fn settle(
        &self,
        id: &Id,
        since: u32,
        opts: TurnOpts,
        mut on: impl FnMut(StepEvent) + Send,
        mut working: Option<bool>,
    ) -> Result<SettledTurn> {
        let started = Instant::now();
        let root = self.live(id).await?;
        let mut descendants: Vec<(Id, Arc<Live>)> = Vec::new();
        let mut reported: Vec<Wait> = Vec::new();
        let mut sideband_after: Option<u64> = None;
        let mut background_since: Option<Instant> = None;
        let ended = loop {
            if restart_cut(&root.lock(), since) {
                break Ended::ServerRestart;
            }
            // Arm every wake BEFORE looking, so an event in between is not lost.
            let watched: Vec<Arc<Live>> = std::iter::once(Arc::clone(&root))
                .chain(descendants.iter().map(|(_, l)| Arc::clone(l)))
                .collect();
            let mut wakes: Vec<Pin<Box<tokio::sync::futures::Notified<'_>>>> = watched
                .iter()
                .map(|l| {
                    let mut n = Box::pin(l.deadline_changed.notified());
                    n.as_mut().enable();
                    n
                })
                .collect();
            let mut look = self.look(id, &root);
            let ids = |d: &[(Id, Arc<Live>)]| d.iter().map(|(i, _)| i.clone()).collect::<Vec<_>>();
            if ids(&look.descendants) != ids(&descendants) {
                descendants = look.descendants;
                continue;
            }
            let now = Instant::now();
            // Requests waiting on a person: new ones first, then cleared ones.
            for w in &look.waits {
                if !reported.iter().any(|r| r.key == w.key) {
                    on(StepEvent::NeedsYou(w.attention.clone()));
                }
            }
            for gone in reported
                .iter()
                .filter(|r| !look.waits.iter().any(|w| w.key == r.key))
            {
                let (answer, by) = self.resolution(&gone.key).await;
                on(StepEvent::Resumed {
                    request_id: Some(gone.key.1.clone()),
                    answer,
                    by,
                });
            }
            let waiting = !look.waits.is_empty();
            reported = std::mem::take(&mut look.waits);
            if look.closed {
                break Ended::Settled;
            }
            let busy_turn = look.turn;
            if working != Some(busy_turn) {
                if busy_turn {
                    on(StepEvent::Working);
                } else if working.is_some() && !look.settled(now) {
                    on(StepEvent::Finishing);
                }
                working = Some(busy_turn);
            }
            if look.settled(now) {
                // An ACP sideband may still report background work late: one
                // synchronous poll after the last event, then look again.
                let seq = root.lock().session.last_sequence;
                if sideband_after != Some(seq) {
                    sideband_after = Some(seq);
                    if poll_driver_sideband(&self.inner, &root, None).await
                        || root.lock().session.last_sequence != seq
                    {
                        continue;
                    }
                }
                break Ended::Settled;
            }
            if look.held_wake
                && !look.turn
                && !look.background
                && !look.ready_queue
                && !look.descendants_busy
                && !look.pending
            {
                break Ended::LoopGuard;
            }
            if opts.cancel.is_cancelled() {
                break Ended::Cancelled;
            }
            let background_now = look.background && !look.turn;
            background_since = background_now.then(|| background_since.unwrap_or(now));
            let mut next: Vec<(Instant, Ended)> = Vec::new();
            if let Some(d) = opts.deadline {
                next.push((started + d, Ended::Deadline));
            }
            if let (Some(d), Some(at), false) = (
                opts.no_progress,
                look.last_activity,
                look.pending || look.background || waiting,
            ) {
                next.push((at + d, Ended::NoProgress));
            }
            if let Some(t) = background_since {
                next.push((t + opts.background_limit, Ended::BackgroundLimit));
            }
            if let Some(t) = look.background_ended {
                next.push((t + AUTO_TURN_GRACE, Ended::Settled));
            }
            next.sort_by_key(|(t, _)| *t);
            if let Some((t, e)) = next.first().copied().filter(|(t, _)| *t <= now) {
                let _ = t;
                if e != Ended::Settled {
                    break e;
                }
            }
            let timer = next.iter().map(|(t, _)| *t).find(|t| *t > now);
            tokio::select! {
                _ = futures_util::future::select_all(wakes.iter_mut().map(|w| w.as_mut())) => {}
                _ = opts.cancel.cancelled() => {}
                _ = async {
                    match timer {
                        Some(t) => tokio::time::sleep_until(t.into()).await,
                        None => std::future::pending().await,
                    }
                } => {}
            }
        };
        match ended {
            Ended::Settled | Ended::LoopGuard | Ended::ServerRestart => {}
            Ended::BackgroundLimit => {
                let _ = self.stop_all_tasks(id).await;
                let _ = self.interrupt(id).await;
            }
            _ => {
                let _ = self.interrupt(id).await;
            }
        }
        let waited_on = reported.first().map(|w| w.attention.text.clone());
        let mut out = self.settled_turn(&root, since, ended);
        out.waited_on = waited_on;
        out.sub_threads = self.sub_thread_use(id).await;
        Ok(out)
    }

    /// The thread and its open agent-opened descendants, as they are now.
    fn look(&self, id: &Id, root: &Arc<Live>) -> Look {
        let mut look = Look::default();
        {
            let st = root.lock();
            look.closed = st.session.status == RuntimeSessionStatus::Closed;
            look.turn = st.turn.is_some() || st.starting > 0;
            look.background = !st.session.background_tasks.is_empty();
            (look.ready_queue, look.held_wake) = queue_state(&st);
            look.pending = has_pending(&st);
            look.last_activity = Some(st.last_activity);
            look.background_ended = st.background_ended_idle_at;
            for r in st.proj.requests.values() {
                if r.status == RuntimeRequestStatus::Pending {
                    look.waits.push(wait_of(&st, r, id));
                }
            }
        }
        look.pending_wakes = !root.pending_wakes().is_empty();
        // Open threads by opener (agent-opened only), then the tree under `id`.
        let mut by_opener: HashMap<Id, Vec<(Id, Arc<Live>)>> = HashMap::new();
        let all: Vec<(Id, Arc<Live>)> = self
            .inner
            .live
            .iter()
            .map(|e| (e.key().clone(), Arc::clone(e.value())))
            .collect();
        for (cid, l) in all {
            let (of, group) = {
                let st = l.lock();
                (st.session.child_of(), st.session.parent_id.clone())
            };
            if let Some(of) = of.filter(|o| o.by == ChildOpener::Agent) {
                // Its opener was closed and left memory: a member of the
                // group `id` leads (a step's thread) still counts below it.
                let gone = !self.inner.live.contains_key(&of.id);
                let opener = if gone && group.as_ref() == Some(id) {
                    id.clone()
                } else {
                    of.id
                };
                by_opener.entry(opener).or_default().push((cid, l));
            }
        }
        let mut frontier = vec![(id.clone(), Arc::clone(root))];
        let mut seen: HashSet<Id> = HashSet::from([id.clone()]);
        while let Some((opener, opener_live)) = frontier.pop() {
            let opener_idle = {
                let st = opener_live.lock();
                st.turn.is_none() && !queue_state(&st).0
            } && opener_live.pending_wakes().is_empty();
            let mut kids = by_opener.remove(&opener).unwrap_or_default();
            kids.sort_by(|a, b| a.0.cmp(&b.0));
            for (cid, l) in kids {
                if !seen.insert(cid.clone()) {
                    continue;
                }
                {
                    let st = l.lock();
                    // A closed sub-thread may still have open ones below it.
                    if st.session.status == RuntimeSessionStatus::Closed {
                        drop(st);
                        frontier.push((cid, l));
                        continue;
                    }
                    let (ready, held) = queue_state(&st);
                    if st.turn.is_some()
                        || st.starting > 0
                        || !st.session.background_tasks.is_empty()
                        || ready
                        || has_pending(&st)
                    {
                        look.descendants_busy = true;
                    }
                    look.held_wake |= held;
                    look.last_activity = look.last_activity.max(Some(st.last_activity));
                    look.background_ended = look.background_ended.max(st.background_ended_idle_at);
                    // A sub-thread's question goes to its opener's agent first:
                    // it needs a person only when that agent left it pending.
                    for r in st.proj.requests.values() {
                        if r.status != RuntimeRequestStatus::Pending {
                            continue;
                        }
                        let question = r.kind == RequestKind::Question && !r.for_person;
                        if !question || opener_idle {
                            look.waits.push(wait_of(&st, r, &cid));
                        }
                    }
                }
                if !l.pending_wakes().is_empty() {
                    look.descendants_busy = true;
                }
                look.descendants.push((cid.clone(), Arc::clone(&l)));
                frontier.push((cid, l));
            }
        }
        look
    }

    /// The answer and who gave it for a request that is no longer pending.
    async fn resolution(&self, key: &(Id, Id)) -> (Option<String>, Option<Id>) {
        let Ok(live) = self.live(&key.0).await else {
            return (None, None);
        };
        let st = live.lock();
        st.proj
            .requests
            .get(&key.1)
            .map(|r| (answer_text(r), r.resolved_by.clone()))
            .unwrap_or_default()
    }

    /// One snapshot of what the turns above `since` did.
    fn settled_turn(&self, live: &Arc<Live>, since: u32, ended: Ended) -> SettledTurn {
        let st = live.lock();
        let mut turns: Vec<&ProviderTurn> = st
            .proj
            .turns
            .values()
            .filter(|t| t.ordinal > since)
            .collect();
        turns.sort_by_key(|t| t.ordinal);
        let mut usage = TurnTokenUsage::default();
        for t in &turns {
            if let Some(u) = &t.turn_token_usage {
                usage.add(u);
            }
        }
        let last = turns.last().copied();
        let failure = last.and_then(|t| {
            st.proj
                .items
                .get(&format!("{}:error", t.id))
                .and_then(|i| match &i.kind {
                    TurnItemKind::Error { failure } => Some(failure.clone()),
                    _ => None,
                })
        });
        let answer_of = |t: &ProviderTurn| {
            let mut items: Vec<&TurnItem> = st
                .proj
                .items
                .values()
                .filter(|i| i.turn_id.as_ref() == Some(&t.id) && i.parent_item_id.is_none())
                .collect();
            items.sort_by_key(|i| i.ordinal);
            items
                .iter()
                .filter_map(|i| match &i.kind {
                    TurnItemKind::AssistantMessage { text, .. } if !text.trim().is_empty() => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let answer = turns
            .iter()
            .rev()
            .map(|t| answer_of(t))
            .find(|a| !a.is_empty())
            .unwrap_or_default();
        SettledTurn {
            ended,
            turn_status: last.map(|t| t.status),
            failure,
            answer,
            start_ordinal: since,
            turns: turns.len() as u32,
            usage,
            sub_threads: Vec::new(),
            model: last
                .and_then(|t| t.model.clone())
                .or(st.session.model.clone()),
            effort: last
                .and_then(|t| t.effort.clone())
                .or(st.session.effort.clone()),
            context_usage: st
                .proj
                .thread
                .as_ref()
                .and_then(|t| t.context_usage.clone()),
            waited_on: None,
        }
    }

    /// Every thread an agent of `id`'s group opened below it (closed ones
    /// too), with the usage of all its turns.
    async fn sub_thread_use(&self, id: &Id) -> Vec<SubThreadUse> {
        let Ok(rows) = self.linked(id).await else {
            return Vec::new();
        };
        // Sub-threads an agent removed before the step settled: their record
        // stays on the thread that opened them (`otto.thread_close remove`).
        let mut out: Vec<SubThreadUse> = rows
            .iter()
            .filter_map(|s| {
                s.meta
                    .as_ref()?
                    .get(peers::REMOVED_CHILDREN_META_KEY)?
                    .as_array()
            })
            .flatten()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();
        for s in rows.into_iter().filter(|s| &s.id != id) {
            if s.child_of().is_none_or(|o| o.by != ChildOpener::Agent) {
                continue;
            }
            let mut usage = TurnTokenUsage::default();
            if let Ok(entities) = self.inner.repo.entities(&s.id).await {
                for e in entities.into_iter().filter(|e| e.kind == K_TURN) {
                    if let Ok(t) = serde_json::from_value::<ProviderTurn>(e.payload) {
                        if let Some(u) = &t.turn_token_usage {
                            usage.add(u);
                        }
                    }
                }
            }
            out.push(SubThreadUse {
                id: s.id.clone(),
                provider: s.provider.as_str().to_string(),
                model: s.model.clone(),
                title: Some(s.title.clone()),
                status: s
                    .last_turn_status
                    .map_or("idle", |o| match o {
                        otto_core::agent_runtime::TurnOutcome::Completed => "completed",
                        otto_core::agent_runtime::TurnOutcome::Failed => "failed",
                        otto_core::agent_runtime::TurnOutcome::Interrupted => "interrupted",
                    })
                    .to_string(),
                usage,
            });
        }
        out
    }

    /// Record who is about to answer `request_id` (a person's user id, or
    /// `thread:<id>` for a parent agent): the resolved request carries it as
    /// `resolved_by`, so a step's run can say who answered.
    pub async fn note_resolver(&self, id: &Id, request_id: &str, by: &str) {
        if let Ok(live) = self.live(id).await {
            live.resolvers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(request_id.to_string(), by.to_string());
        }
    }

    /// Whether the thread's agent process runs (a turn needs no spawn).
    pub async fn has_process(&self, id: &Id) -> Result<bool> {
        Ok(self.live(id).await?.lock().proc.is_some())
    }

    /// The thread's latest turn ordinal (a new turn gets the next one).
    pub async fn turn_ordinal(&self, id: &Id) -> Result<u32> {
        Ok(self.live(id).await?.lock().proj.next_turn_ordinal)
    }

    /// What the turns above `since` did, as they are now (no wait).
    pub async fn settled_since(&self, id: &Id, since: u32) -> Result<SettledTurn> {
        let live = self.live(id).await?;
        let mut out = self.settled_turn(&live, since, Ended::Settled);
        out.sub_threads = self.sub_thread_use(id).await;
        Ok(out)
    }

    /// Stop a thread's agent process now (as the idle release does), leaving
    /// the thread open: its next turn respawns it, resuming the conversation.
    pub async fn release_process(&self, id: &Id) -> Result<()> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        let mut st = live.lock();
        if st.turn.is_some() {
            return Err(Error::Conflict(TURN_RUNNING.into()));
        }
        auto_close::release_locked(&self.inner, &live, &mut st);
        Ok(())
    }
}

impl Live {
    /// Wakes announced by this thread's sub-threads but not queued yet.
    pub(super) fn pending_wakes(&self) -> MutexGuard<'_, Vec<String>> {
        self.pending_wakes.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Count a spawned (or adopted) agent process with the host for as long as
/// its pump runs — the pump ends with the process — then run the pump.
pub(super) async fn register_and_pump(
    inner: &Inner,
    session: &RuntimeSession,
    pid: u32,
    pump: impl std::future::Future<Output = ()> + Send + 'static,
) {
    let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);
    inner
        .host
        .register_runtime(session, Some(pid), exit_rx)
        .await;
    tokio::spawn(async move {
        pump.await;
        drop(exit_tx);
    });
}

/// Who answered `request_id` ([`AgentRuntime::note_resolver`]), once.
pub(super) fn take_resolver(live: &Live, request_id: &str) -> Option<Id> {
    live.resolvers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(request_id)
}

/// Note a wake a sub-thread is about to queue on `parent` (`key` = the
/// child's id, or `question:<request id>`); [`wake_done`] clears it.
pub(super) fn wake_announced(inner: &Inner, parent: &Id, key: &str) {
    if let Some(l) = inner.live.get(parent) {
        l.pending_wakes().push(key.to_string());
    }
}

pub(super) fn wake_done(inner: &Inner, parent: &Id, key: &str) {
    if let Some(l) = inner.live.get(parent) {
        let mut w = l.pending_wakes();
        if let Some(i) = w.iter().position(|k| k == key) {
            w.remove(i);
        }
        drop(w);
        l.deadline_changed.notify_waiters();
    }
}

#[cfg(test)]
#[path = "unattended_tests.rs"]
mod tests;
