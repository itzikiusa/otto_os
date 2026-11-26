//! `ThreadStepRunner`: an unattended agent step on a structured runtime
//! thread (the default `StepRunner`; otto-server's `PtyStepRunner` is the
//! fallback). It waits for a machine place, opens a thread (or continues an
//! earlier step's), runs the turn to settle, nudges once for a missing
//! handoff, waits out a plan usage limit whose reset is known, and folds the
//! result into one of the runner's error classes.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_agent_run::step::{
    StepError, StepErrorClass, StepEvent, StepOutcome, StepRunner, StepTurn,
};
use otto_core::agent_runtime::{
    error_class, limit_kind, Attention, AttentionKind, CreateRuntimeSessionReq, LimitKind,
    ProviderTurnStatus, RuntimePromptReq, RuntimeSessionStatus, TurnTokenUsage, WaitDetail,
    WaitReason, ERROR_CLASS_AUTH, PROVIDER_SLUG_META_KEY,
};
use otto_core::cancel_signal::CancelSignal;
use otto_core::stack::boxed;
use otto_core::{Error, Id, Result};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::runtime::RESUME_REFUSED;
use crate::{AgentRuntime, Ended, SettledTurn, TurnOpts};

/// What the runner needs from the daemon around a thread.
#[async_trait]
pub trait StepHost: Send + Sync {
    /// Wait for a machine place for a background agent of `source`
    /// (dropping the future leaves the queue), reporting each wait. The
    /// spawn claims the place by its key; the guard gives it back if no
    /// spawn did.
    async fn admit(
        &self,
        source: &str,
        label: &str,
        on_wait: &(dyn Fn(WaitReason) + Send + Sync),
    ) -> Place;
    /// Why the step's provider / model / effort cannot run now (a model
    /// dropped from the catalog since the workflow was saved), checked
    /// before anything starts — `otto_sessions::provider_models::check_pick`.
    async fn refuse_pick(
        &self,
        _provider: &str,
        _model: Option<&str>,
        _effort: Option<&str>,
    ) -> Option<String> {
        None
    }
    /// Report the governed Otto MCP approval cards the calls of `thread` and
    /// of the sub-threads its agents opened wait on as `NeedsYou` (`via:
    /// mcp`) and their decision as `Resumed`, for as long as the turn runs
    /// (the runner drops this future at settle).
    async fn watch_mcp(&self, _rt: &AgentRuntime, _thread: &Id, _ev: UnboundedSender<StepEvent>) {
        std::future::pending::<()>().await
    }
    /// When `provider`'s plan usage limit resets, when Otto knows
    /// (`otto_usage::limits`); `meta` names the account.
    async fn usage_resets_at(&self, _provider: &str, _meta: &Value) -> Option<DateTime<Utc>> {
        None
    }
}

/// A machine place: `key` goes with the spawn that takes it over (the
/// create's `meta.admission_permit`, or `TurnOpts::permit`).
pub struct Place {
    pub key: Option<String>,
    pub guard: Box<dyn Send>,
}

/// No machine gate, no usage lookup (tests, and engines without a daemon).
pub struct NoStepHost;

#[async_trait]
impl StepHost for NoStepHost {
    async fn admit(&self, _: &str, _: &str, _: &(dyn Fn(WaitReason) + Send + Sync)) -> Place {
        Place {
            key: None,
            guard: Box::new(()),
        }
    }
}

/// The daemon's [`StepHost`]: the machine's admission gate and the provider
/// registry through the session manager; the usage lookup is the host's
/// (`otto_usage::limits::reset_for` over the owner's limits), when given.
pub struct DaemonStepHost {
    pub manager: Arc<otto_sessions::SessionManager>,
    /// The approval cards and the daemon bus that says when one changes.
    pub approvals: Option<(
        otto_state::McpApprovalRepo,
        tokio::sync::broadcast::Sender<otto_core::event::Event>,
    )>,
    #[allow(clippy::type_complexity)]
    pub resets: Option<
        Arc<
            dyn Fn(String, Value) -> otto_core::auth::BoxFuture<'static, Option<DateTime<Utc>>>
                + Send
                + Sync,
        >,
    >,
}

#[async_trait]
impl StepHost for DaemonStepHost {
    async fn admit(
        &self,
        source: &str,
        label: &str,
        on_wait: &(dyn Fn(WaitReason) + Send + Sync),
    ) -> Place {
        let permit = self
            .manager
            .admit_background_with(source, label, |w| {
                if let Some(r) = wait_reason(w) {
                    on_wait(r);
                }
            })
            .await;
        let mut meta = serde_json::json!({});
        permit.stamp(&mut meta);
        Place {
            key: otto_sessions::admission::take_permit_key(&mut meta),
            guard: Box::new(permit),
        }
    }
    async fn refuse_pick(
        &self,
        provider: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Option<String> {
        let reg = self.manager.providers();
        match otto_sessions::provider_models::check_pick(reg, provider, model, effort).await {
            otto_sessions::provider_models::PickCheck::Invalid(m) => Some(m),
            _ => None,
        }
    }
    async fn watch_mcp(&self, rt: &AgentRuntime, thread: &Id, ev: UnboundedSender<StepEvent>) {
        let Some((repo, bus)) = &self.approvals else {
            return std::future::pending().await;
        };
        let mut reported: Vec<otto_state::McpApproval> = Vec::new();
        loop {
            let mut rx = bus.subscribe();
            let mut pending = Vec::new();
            for id in rt.family(thread).await {
                pending.extend(repo.pending_for_session(&id).await.unwrap_or_default());
            }
            for a in pending
                .iter()
                .filter(|a| !reported.iter().any(|r| r.id == a.id))
            {
                let mut att = Attention::new(AttentionKind::Approval, &a.title, Utc::now());
                att.via = Some(otto_core::agent_runtime::AttentionVia::Mcp);
                att.request_id = Some(a.id.clone());
                att.thread_id = a.requested_by_session_id.clone();
                let _ = ev.send(StepEvent::NeedsYou(att));
            }
            for gone in reported
                .iter()
                .filter(|r| !pending.iter().any(|a| a.id == r.id))
            {
                let decided = repo.get(&gone.id).await.ok();
                let _ = ev.send(StepEvent::Resumed {
                    request_id: Some(gone.id.clone()),
                    answer: decided.as_ref().map(|d| d.status.clone()),
                    by: decided.and_then(|d| d.decided_by),
                });
            }
            reported = pending;
            let changed = async {
                loop {
                    match rx.recv().await {
                        Ok(otto_core::event::Event::McpApprovalChanged { .. })
                        | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => return,
                        Ok(_) => continue,
                        Err(_) => return std::future::pending().await,
                    }
                }
            };
            let _ = tokio::time::timeout(std::time::Duration::from_secs(30), changed).await;
        }
    }
    async fn usage_resets_at(&self, provider: &str, meta: &Value) -> Option<DateTime<Utc>> {
        let f = self.resets.as_ref()?;
        f(provider.to_string(), meta.clone()).await
    }
}

/// The gate's wait in the run's words (`None` for the brief spacing).
fn wait_reason(w: &otto_sessions::admission::AdmissionWait) -> Option<WaitReason> {
    use otto_sessions::admission::AdmissionWait as A;
    let code = w.code()?;
    let n = |x: usize| u32::try_from(x).unwrap_or(u32::MAX);
    let detail = match w {
        A::Queue { ahead } => WaitDetail {
            ahead: Some(n(*ahead)),
            ..WaitDetail::default()
        },
        A::LiveCeiling { live, max, holders } => WaitDetail {
            live: n(*live),
            max: Some(n(*max)),
            holders: holders.iter().map(|h| h.label.clone()).collect(),
            ..WaitDetail::default()
        },
        A::Load { live, floor, .. } | A::MemoryPressure { live, floor, .. } => WaitDetail {
            live: n(*live),
            floor: Some(n(*floor)),
            ..WaitDetail::default()
        },
        A::Spacing => return None,
    };
    Some(WaitReason {
        code: code.to_string(),
        text: w.describe(),
        since: Utc::now(),
        detail: Some(detail),
    })
}

pub const NUDGE: &str = "You did not write {path}. Write it now; nothing else.";
pub const CONTINUE_AFTER_LIMIT: &str = "Continue the step where you stopped.";
pub const AFTER_RESTART: &str = "Otto restarted while you were working on this step. \
     Check what you already did and finish it.";

pub struct ThreadStepRunner {
    rt: AgentRuntime,
    host: Arc<dyn StepHost>,
}

/// One step's running tally across its turns (nudge, limit, restart).
struct Tally {
    start_ordinal: u32,
    started: Instant,
}

impl ThreadStepRunner {
    pub fn new(rt: AgentRuntime, host: Arc<dyn StepHost>) -> Self {
        Self { rt, host }
    }

    fn source(t: &StepTurn) -> String {
        t.meta
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("workflow")
            .to_string()
    }

    /// The machine place, or `None` when the step was canceled meanwhile.
    async fn admit(
        &self,
        t: &StepTurn,
        ev: &UnboundedSender<StepEvent>,
        cancel: &CancelSignal,
    ) -> Option<Place> {
        let ev = ev.clone();
        let on_wait = move |w: WaitReason| {
            let _ = ev.send(StepEvent::Waiting(w));
        };
        let source = Self::source(t);
        tokio::select! {
            p = self.host.admit(&source, &t.title, &on_wait) => Some(p),
            _ = cancel.cancelled() => None,
        }
    }

    /// A fresh thread for `t`, or the earlier step's it continues.
    async fn thread(
        &self,
        t: &StepTurn,
        key: Option<String>,
    ) -> std::result::Result<Id, StepError> {
        let spawn = |m: String| StepError::new(StepErrorClass::Spawn, m);
        if let Some(id) = &t.continue_thread {
            let s = match self.rt.get(id).await {
                Ok(s) => s,
                Err(Error::NotFound(_)) => {
                    return Err(spawn(
                        "the conversation of the earlier step was deleted by retention".into(),
                    ))
                }
                Err(e) => return Err(spawn(e.to_string())),
            };
            if s.provider_slug() != t.provider {
                return Err(spawn(format!(
                    "this step continues a conversation that runs on {}; it names {}",
                    s.provider_slug(),
                    t.provider
                )));
            }
            if s.status == RuntimeSessionStatus::Closed {
                // No spawn here: the turn's own spawn takes the place and
                // resumes strictly.
                self.rt
                    .reopen_dormant(id)
                    .await
                    .map_err(|e| spawn(e.to_string()))?;
            }
            return Ok(id.clone());
        }
        let provider = self
            .rt
            .resolve_runtime_provider(&t.provider)
            .ok_or_else(|| spawn(format!("{} is not an enabled provider", t.provider)))?;
        let mut meta = t.meta.clone();
        if let Some(m) = meta.as_object_mut() {
            if t.read_only {
                m.insert("read_only".into(), Value::Bool(true));
            }
            if let Some(key) = key {
                m.insert(
                    otto_sessions::admission::PERMIT_META_KEY.into(),
                    Value::String(key),
                );
            }
            if provider.as_str() != t.provider {
                m.insert(
                    PROVIDER_SLUG_META_KEY.into(),
                    Value::String(t.provider.clone()),
                );
            }
        }
        let req = CreateRuntimeSessionReq {
            provider: Some(provider),
            cwd: Some(t.cwd.display().to_string()),
            title: Some(t.title.clone()),
            model: t.model.clone().filter(|m| !m.trim().is_empty()),
            access_mode: None,
            title_auto: false,
            meta: Some(meta),
        };
        self.rt
            .create(&t.user_id, &t.workspace_id, req)
            .await
            .map(|s| s.id)
            .map_err(|e| spawn(e.to_string()))
    }

    /// One turn to settle; an error that kept the prompt from going out is
    /// folded into a [`StepError`].
    #[allow(clippy::too_many_arguments)]
    async fn turn(
        &self,
        id: &Id,
        text: &str,
        t: &StepTurn,
        first: bool,
        tally: &Tally,
        ev: &UnboundedSender<StepEvent>,
        cancel: &CancelSignal,
    ) -> std::result::Result<SettledTurn, StepError> {
        let mut req = RuntimePromptReq::of(text);
        // A fresh thread got its model at create; a continued one keeps what
        // it runs unless the step names its own. Effort rides the first turn.
        if first && t.continue_thread.is_some() {
            req.model = t.model.clone();
        }
        if first {
            req.effort = t.effort.clone().filter(|e| !e.trim().is_empty());
        }
        let mut opts = TurnOpts::new(cancel.clone());
        opts.no_progress = t.no_progress;
        opts.deadline = t.timeout.map(|d| d.saturating_sub(tally.started.elapsed()));
        // A thread with no process respawns for this turn: wait for its
        // machine place first, outside every runtime lock.
        let mut _place = None;
        if !self.rt.has_process(id).await.unwrap_or(true) {
            let Some(p) = self.admit(t, ev, cancel).await else {
                return Err(StepError::new(StepErrorClass::Canceled, "canceled"));
            };
            opts.permit = p.key.clone();
            _place = Some(p);
        }
        let ev2 = ev.clone();
        let run = boxed(|| {
            self.rt.run_turn(id, &req, opts, move |e| {
                let _ = ev2.send(e);
            })
        });
        self.watched(id, ev, run).await.map_err(|e| {
            let m = e.to_string();
            if m.contains(RESUME_REFUSED) {
                StepError::new(
                    StepErrorClass::Provider,
                    "could not resume the conversation of the earlier step",
                )
            } else {
                StepError::new(StepErrorClass::Spawn, m)
            }
        })
    }

    /// `wait`, with the MCP approval cards it waits on reported meanwhile.
    async fn watched<T>(
        &self,
        id: &Id,
        ev: &UnboundedSender<StepEvent>,
        wait: impl std::future::Future<Output = T>,
    ) -> T {
        tokio::pin!(wait);
        let watch = self.host.watch_mcp(&self.rt, id, ev.clone());
        tokio::pin!(watch);
        let mut watching = true;
        loop {
            tokio::select! {
                r = &mut wait => return r,
                _ = &mut watch, if watching => watching = false,
            }
        }
    }

    /// The class a settled wait ends with, if it failed.
    fn fold(s: &SettledTurn) -> Option<StepError> {
        let class = match s.ended {
            Ended::Settled => None,
            Ended::Cancelled => Some((StepErrorClass::Canceled, "canceled".to_string())),
            Ended::Deadline => Some((
                StepErrorClass::Timeout,
                match &s.waited_on {
                    Some(w) => format!("the step's time ran out while it waited: {w}"),
                    None => "the step's time ran out".into(),
                },
            )),
            Ended::NoProgress => Some((StepErrorClass::Stalled, "no progress".into())),
            Ended::BackgroundLimit => Some((
                StepErrorClass::Stalled,
                "background work was still running at the limit".into(),
            )),
            Ended::LoopGuard => Some((
                StepErrorClass::Stalled,
                "the sub-thread reply loop guard held a reply — a person must send it".into(),
            )),
            Ended::ServerRestart => Some((
                StepErrorClass::ServerRestart,
                "Otto restarted while the step ran".into(),
            )),
        };
        if let Some((class, message)) = class {
            return Some(StepError::new(class, message));
        }
        match s.turn_status {
            Some(ProviderTurnStatus::Failed) => {}
            Some(ProviderTurnStatus::Interrupted | ProviderTurnStatus::Cancelled) => {
                return Some(StepError::new(
                    StepErrorClass::Stalled,
                    "the agent's turn was stopped",
                ))
            }
            _ => return None,
        }
        let f = s
            .failure
            .clone()
            .unwrap_or_else(|| otto_core::agent_runtime::ProviderFailure {
                class: "provider_error".into(),
                message: "the agent's turn failed".into(),
                code: None,
                retryable: None,
            });
        let class = match limit_kind(&f) {
            Some(LimitKind::Rate) => StepErrorClass::RateLimit,
            Some(_) => StepErrorClass::UsageLimit,
            None if error_class(&f) == ERROR_CLASS_AUTH => StepErrorClass::Auth,
            None => StepErrorClass::Provider,
        };
        Some(StepError {
            class,
            message: f.message.clone(),
            code: f.code.clone(),
        })
    }

    /// When a plan limit that stopped `s` resets: Claude's own text first.
    async fn resets_at(&self, s: &SettledTurn, t: &StepTurn) -> Option<DateTime<Utc>> {
        match s.failure.as_ref().and_then(limit_kind) {
            Some(LimitKind::Plan {
                resets_at: Some(at),
            }) => Some(at),
            Some(LimitKind::Plan { resets_at: None }) => {
                self.host.usage_resets_at(&t.provider, &t.meta).await
            }
            _ => None,
        }
    }

    /// Drive `id` from its first prompt to the step's end; `settled`: a
    /// turn that already settled (a resume), taken as the first one's.
    #[allow(clippy::too_many_arguments)]
    async fn drive(
        &self,
        id: &Id,
        mut prompt: String,
        mut first: bool,
        mut settled: Option<SettledTurn>,
        t: &StepTurn,
        tally: Tally,
        ev: &UnboundedSender<StepEvent>,
        cancel: &CancelSignal,
    ) -> StepOutcome {
        let mut nudged = false;
        let mut last_reset: Option<DateTime<Utc>> = None;
        let error = loop {
            let settled = match settled.take() {
                Some(s) => s,
                None => {
                    match boxed(|| self.turn(id, &prompt, t, first, &tally, ev, cancel)).await {
                        Ok(s) => s,
                        Err(e) => break Some(e),
                    }
                }
            };
            first = false;
            let error = Self::fold(&settled);
            match &error {
                None => {
                    let missing = t.handoff.as_ref().filter(|h| !h.exists());
                    if let (Some(h), false) = (missing, nudged) {
                        nudged = true;
                        let _ = ev.send(StepEvent::Finishing);
                        prompt = NUDGE.replace("{path}", &h.display().to_string());
                        continue;
                    }
                    break None;
                }
                Some(e) if e.class == StepErrorClass::UsageLimit => {
                    let at = self.resets_at(&settled, t).await;
                    // A limit that comes back after its reset passed: give up.
                    let Some(at) = at.filter(|a| last_reset.is_none_or(|l| *a > l)) else {
                        break error;
                    };
                    last_reset = Some(at);
                    let wait = (at - Utc::now()).to_std().unwrap_or_default();
                    let budget = t.timeout.map(|d| d.saturating_sub(tally.started.elapsed()));
                    if budget.is_some_and(|b| b < wait) {
                        break Some(StepError::new(
                            StepErrorClass::Timeout,
                            "the step's time ran out while it waited for the usage limit to reset",
                        ));
                    }
                    let mut a = Attention::new(
                        AttentionKind::UsageLimit,
                        &format!(
                            "{}'s usage limit stopped the step — resets at {}",
                            t.provider,
                            at.with_timezone(&chrono::Local).format("%H:%M")
                        ),
                        Utc::now(),
                    );
                    a.thread_id = Some(id.clone());
                    a.resets_at = Some(at);
                    let _ = ev.send(StepEvent::NeedsYou(a));
                    // Parked: no process, no machine place until the reset;
                    // the next turn re-admits and resumes strictly.
                    self.rt.release_process(id).await.ok();
                    if cancel.sleep(wait).await {
                        break Some(StepError::new(StepErrorClass::Canceled, "canceled"));
                    }
                    let _ = ev.send(StepEvent::Resumed {
                        request_id: None,
                        answer: None,
                        by: None,
                    });
                    prompt = CONTINUE_AFTER_LIMIT.to_string();
                }
                Some(_) => break error,
            }
        };
        self.outcome(id, &tally, t, error).await
    }

    async fn outcome(
        &self,
        id: &Id,
        tally: &Tally,
        t: &StepTurn,
        error: Option<StepError>,
    ) -> StepOutcome {
        let s = self.rt.settled_since(id, tally.start_ordinal).await.ok();
        let turn_status = match (&error, s.as_ref().and_then(|s| s.turn_status)) {
            (Some(e), _) if e.class == StepErrorClass::Canceled => "canceled",
            (_, Some(ProviderTurnStatus::Completed)) if error.is_none() => "completed",
            (_, Some(ProviderTurnStatus::Interrupted | ProviderTurnStatus::Cancelled)) => {
                "interrupted"
            }
            _ if error.is_some() => "failed",
            _ => "completed",
        };
        let digest = self.rt.digest(id).await.ok();
        StepOutcome {
            thread_id: id.clone(),
            reply: s.as_ref().map(|s| s.answer.clone()).unwrap_or_default(),
            handoff_written: t.handoff.as_ref().is_some_and(|h| h.exists()),
            turn_status: turn_status.to_string(),
            error,
            context_usage: s.as_ref().and_then(|s| s.context_usage.clone()),
            turns: s.as_ref().map_or(0, |s| s.turns),
            usage: s
                .as_ref()
                .map(|s| {
                    let mut u = s.usage.clone();
                    for c in &s.sub_threads {
                        u.add(&c.usage);
                    }
                    u
                })
                .unwrap_or_default(),
            model: s.as_ref().and_then(|s| s.model.clone()),
            effort: s.as_ref().and_then(|s| s.effort.clone()),
            sub_threads: s.map(|s| s.sub_threads).unwrap_or_default(),
            digest,
        }
    }

    fn failed(thread_id: Id, error: StepError) -> StepOutcome {
        StepOutcome {
            thread_id,
            reply: String::new(),
            handoff_written: false,
            turn_status: if error.class == StepErrorClass::Canceled {
                "canceled"
            } else {
                "failed"
            }
            .into(),
            error: Some(error),
            context_usage: None,
            turns: 0,
            usage: TurnTokenUsage::default(),
            model: None,
            effort: None,
            sub_threads: Vec::new(),
            digest: None,
        }
    }
}

#[async_trait]
impl StepRunner for ThreadStepRunner {
    async fn run(
        &self,
        t: StepTurn,
        ev: UnboundedSender<StepEvent>,
        cancel: CancelSignal,
    ) -> Result<StepOutcome> {
        let started = Instant::now();
        let refused = self
            .host
            .refuse_pick(&t.provider, t.model.as_deref(), t.effort.as_deref())
            .await;
        if let Some(m) = refused {
            let id = t.continue_thread.clone().unwrap_or_default();
            return Ok(Self::failed(id, StepError::new(StepErrorClass::Spawn, m)));
        }
        // A fresh thread's place rides its create (a continued thread's, its
        // first turn: `turn` admits when it has no process).
        let place = match t.continue_thread {
            Some(_) => None,
            None => match self.admit(&t, &ev, &cancel).await {
                Some(p) => Some(p),
                None => {
                    return Ok(Self::failed(
                        String::new(),
                        StepError::new(StepErrorClass::Canceled, "canceled"),
                    ))
                }
            },
        };
        let thread = self
            .thread(&t, place.as_ref().and_then(|p| p.key.clone()))
            .await;
        drop(place);
        let id = match thread {
            Ok(id) => id,
            Err(e) => {
                return Ok(Self::failed(
                    t.continue_thread.clone().unwrap_or_default(),
                    e,
                ))
            }
        };
        let start_ordinal = self.rt.turn_ordinal(&id).await?;
        let _ = ev.send(StepEvent::Started {
            thread_id: id.clone(),
            start_ordinal,
        });
        let tally = Tally {
            start_ordinal,
            started,
        };
        let prompt = t.prompt.clone();
        Ok(boxed(|| self.drive(&id, prompt, true, None, &t, tally, &ev, &cancel)).await)
    }

    async fn resume(
        &self,
        thread_id: &Id,
        start_ordinal: u32,
        t: &StepTurn,
        ev: UnboundedSender<StepEvent>,
        cancel: CancelSignal,
    ) -> Result<StepOutcome> {
        let started = Instant::now();
        let tally = Tally {
            start_ordinal,
            started,
        };
        let mut opts = TurnOpts::new(cancel.clone());
        opts.no_progress = t.no_progress;
        opts.deadline = t.timeout;
        let ev2 = ev.clone();
        let wait = boxed(|| {
            self.rt
                .await_settled(thread_id, start_ordinal, opts, move |e| {
                    let _ = ev2.send(e);
                })
        });
        let settled = match self.watched(thread_id, &ev, wait).await {
            Ok(s) => s,
            Err(Error::NotFound(_)) => {
                return Ok(Self::failed(
                    thread_id.clone(),
                    StepError::new(StepErrorClass::Spawn, "the step's thread is gone"),
                ))
            }
            Err(e) => return Err(e),
        };
        let (prompt, first, settled) = match settled.ended {
            // An unheld thread: tell the agent and finish on the same conversation.
            Ended::ServerRestart => (AFTER_RESTART.to_string(), false, None),
            // The restart came before the step's prompt went out: send it now.
            Ended::Settled if settled.turns == 0 => (t.prompt.clone(), true, None),
            // Held through the restart: settle it as a step from here (handoff
            // nudge and usage limits included).
            _ => (String::new(), false, Some(settled)),
        };
        Ok(boxed(|| self.drive(thread_id, prompt, first, settled, t, tally, &ev, &cancel)).await)
    }

    async fn close(&self, thread_id: &Id, retain_until: DateTime<Utc>) {
        if let Err(e) = self.rt.close_retained(thread_id, retain_until).await {
            tracing::warn!(session = %thread_id, "step thread close: {e}");
        }
    }

    async fn set_retain_until(&self, thread_ids: &[Id], retain_until: DateTime<Utc>) {
        if let Err(e) = self.rt.set_retain_until(thread_ids, retain_until).await {
            tracing::warn!("step threads retention: {e}");
        }
    }

    async fn release_process(&self, thread_id: &Id) {
        if let Err(e) = self.rt.release_process(thread_id).await {
            tracing::debug!(session = %thread_id, "step thread release: {e}");
        }
    }

    async fn delete(&self, thread_id: &Id) {
        if let Err(e) = self.rt.delete(thread_id).await {
            tracing::warn!(session = %thread_id, "step thread delete: {e}");
        }
    }
}
