//! [`AgentRuntime`]: the registry of runtime sessions, each owning at most one
//! child process, its driver, its replay ring and its entity projection.
//!
//! Ported from T3 Code's orchestration layer (MIT, Copyright (c) 2026 T3 Tools
//! Inc.): provider events become sequenced entity upserts
//! (`ProviderEventIngestor`), a failed terminal becomes an `error` turn item,
//! a reconnect gets the events since its sequence or — past 128 events /
//! 1 MiB — a snapshot (`ThreadStream.ts`), and on restart open turns are
//! cancelled and pending requests expire as not resumable
//! (`ProviderRuntimeRecoveryService.reconcileProjection`).
//!
//! Locking: one `std::sync::Mutex<State>` per session, never held across an
//! `.await`. Sequencing, the ring, the projection, WS publish and enqueueing
//! the persistence op all happen under it, so they agree on order; DB writes
//! run on a per-session persister task fed in that order.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::Utc;
use dashmap::DashMap;
use otto_core::agent_runtime::{
    ApprovalDecision, CreateRuntimeSessionReq, PlanArtifact, ProviderFailure, ProviderSession,
    ProviderSessionStatus, ProviderThread, ProviderTurn, ProviderTurnStatus, ResponseCapability,
    RuntimeEvent, RuntimeEventBody, RuntimePromptResp, RuntimeReplay, RuntimeRequest,
    RuntimeRequestStatus, RuntimeSession, RuntimeSessionStatus, RuntimeSnapshot, TurnItem,
    TurnItemKind, TurnItemStatus, TurnTerminalStatus,
};
use otto_core::{new_id, Error, Id, Result};
use otto_state::{AgentRuntimeRepo, NewRuntimeSession, RuntimeEntityRow};
use tokio::sync::{mpsc, oneshot};

use crate::driver::{
    item, ChildSignal, DriverEvent, DriverFactory, DriverStep, LaunchContext, StructuredDriver,
};
use crate::host::RuntimeHost;
use crate::process::{signal_group, spawn_child, ChildCommand, ChildMsg};
use crate::ring::{EventRing, RingLimits};

/// T3 Code `THREAD_RESUME_MAX_REPLAY_EVENTS` / `…_ENCODED_BYTES`.
pub const MAX_REPLAY_EVENTS: usize = 128;
pub const MAX_REPLAY_BYTES: usize = 1_048_576;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub ring: RingLimits,
    /// Sequences reserved ahead in the DB (the floor a restart resumes above).
    pub seq_reserve: u64,
    /// After an interrupt, how long the provider gets to end the turn itself
    /// before its process group is killed.
    pub interrupt_grace: Duration,
    /// After `close` (stdin EOF), how long before SIGTERM / SIGKILL.
    pub close_grace: Duration,
    /// Streaming upserts of one item are coalesced to at most one per window
    /// (T3 Code: `CODEX_ASSISTANT_DELTA_FLUSH_INTERVAL_MS` = 50 in
    /// `CodexAdapterV2.ts:212`, `ThreadLiveEventCoalescer.ts:18`). The first
    /// chunk goes out at once; the latest text of a held item is flushed
    /// when the window ends or before any other event. Zero disables.
    pub stream_coalesce: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            ring: RingLimits::default(),
            seq_reserve: 65_536,
            interrupt_grace: Duration::from_secs(10),
            close_grace: Duration::from_secs(3),
            stream_coalesce: Duration::from_millis(50),
        }
    }
}

const K_SESSION: &str = "provider_session";
const K_THREAD: &str = "provider_thread";
const K_TURN: &str = "turn";
const K_ITEM: &str = "turn_item";
const K_REQUEST: &str = "runtime_request";
const K_PLAN: &str = "plan";

enum PersistOp {
    Upsert(Vec<RuntimeEntityRow>),
    Reserve(u64),
    NativeThreadId(String),
    Closed,
    Flush(oneshot::Sender<()>),
}

/// Every entity's latest state (+ the sequence that set it).
#[derive(Default)]
struct Projection {
    session: Option<ProviderSession>,
    thread: Option<ProviderThread>,
    turns: HashMap<Id, ProviderTurn>,
    items: HashMap<Id, TurnItem>,
    requests: HashMap<Id, RuntimeRequest>,
    /// request id → its `approval_request` item id.
    request_items: HashMap<Id, Id>,
    plans: HashMap<String, PlanArtifact>,
    next_item_ordinal: u32,
    next_turn_ordinal: u32,
}

impl Projection {
    fn load(rows: Vec<RuntimeEntityRow>) -> Self {
        let mut p = Projection::default();
        for r in rows {
            let v = r.payload;
            match r.kind.as_str() {
                K_SESSION => p.session = serde_json::from_value(v).ok(),
                K_THREAD => p.thread = serde_json::from_value(v).ok(),
                K_TURN => {
                    if let Ok(t) = serde_json::from_value::<ProviderTurn>(v) {
                        p.next_turn_ordinal = p.next_turn_ordinal.max(t.ordinal);
                        p.turns.insert(t.id.clone(), t);
                    }
                }
                K_ITEM => {
                    if let Ok(i) = serde_json::from_value::<TurnItem>(v) {
                        p.next_item_ordinal = p.next_item_ordinal.max(i.ordinal);
                        if let TurnItemKind::ApprovalRequest { request_id, .. } = &i.kind {
                            p.request_items.insert(request_id.clone(), i.id.clone());
                        }
                        p.items.insert(i.id.clone(), i);
                    }
                }
                K_REQUEST => {
                    if let Ok(q) = serde_json::from_value::<RuntimeRequest>(v) {
                        p.requests.insert(q.id.clone(), q);
                    }
                }
                K_PLAN => {
                    if let Ok(pl) = serde_json::from_value::<PlanArtifact>(v) {
                        p.plans.insert(pl.id.clone(), pl);
                    }
                }
                _ => {}
            }
        }
        p
    }

    fn snapshot(&self) -> RuntimeSnapshot {
        let mut turns: Vec<ProviderTurn> = self.turns.values().cloned().collect();
        turns.sort_by_key(|t| t.ordinal);
        let mut turn_items: Vec<TurnItem> = self.items.values().cloned().collect();
        turn_items.sort_by_key(|i| i.ordinal);
        let mut runtime_requests: Vec<RuntimeRequest> = self.requests.values().cloned().collect();
        runtime_requests.sort_by_key(|r| r.created_at);
        let mut plans: Vec<PlanArtifact> = self.plans.values().cloned().collect();
        plans.sort_by(|a, b| a.id.cmp(&b.id));
        RuntimeSnapshot {
            provider_session: self.session.clone(),
            provider_thread: self.thread.clone(),
            turns,
            turn_items,
            runtime_requests,
            plans,
        }
    }

    /// Fold one event in; returns the entity row to persist, if any.
    fn apply(&mut self, body: &RuntimeEventBody, seq: u64) -> Option<RuntimeEntityRow> {
        let row = |kind: &str, id: &str, v: serde_json::Value| RuntimeEntityRow {
            kind: kind.into(),
            entity_id: id.into(),
            sequence: seq,
            payload: v,
        };
        let json = |v: &dyn erased::Ser| v.to_json();
        match body {
            RuntimeEventBody::ProviderSessionUpdated { provider_session } => {
                self.session = Some(provider_session.clone());
                Some(row(K_SESSION, "session", json(provider_session)))
            }
            RuntimeEventBody::ProviderThreadUpdated { provider_thread } => {
                self.thread = Some(provider_thread.clone());
                Some(row(K_THREAD, "thread", json(provider_thread)))
            }
            RuntimeEventBody::ProviderTurnUpdated { provider_turn } => {
                self.turns
                    .insert(provider_turn.id.clone(), provider_turn.clone());
                Some(row(K_TURN, &provider_turn.id, json(provider_turn)))
            }
            RuntimeEventBody::TurnItemUpdated { turn_item } => {
                if let TurnItemKind::ApprovalRequest { request_id, .. } = &turn_item.kind {
                    self.request_items
                        .insert(request_id.clone(), turn_item.id.clone());
                }
                self.items.insert(turn_item.id.clone(), turn_item.clone());
                // Streaming text is written once it settles (T3 Code persists
                // every update; the settled item carries the whole text).
                (!turn_item.is_streaming()).then(|| row(K_ITEM, &turn_item.id, json(turn_item)))
            }
            RuntimeEventBody::RuntimeRequestUpdated { runtime_request } => {
                self.requests
                    .insert(runtime_request.id.clone(), runtime_request.clone());
                Some(row(K_REQUEST, &runtime_request.id, json(runtime_request)))
            }
            RuntimeEventBody::PlanUpdated { plan } => {
                self.plans.insert(plan.id.clone(), plan.clone());
                Some(row(K_PLAN, &plan.id, json(plan)))
            }
            RuntimeEventBody::CommandOutputDelta { .. } | RuntimeEventBody::TurnTerminal { .. } => {
                None
            }
        }
    }
}

/// `&dyn Serialize` without the object-safety dance.
mod erased {
    pub trait Ser {
        fn to_json(&self) -> serde_json::Value;
    }
    impl<T: serde::Serialize> Ser for T {
        fn to_json(&self) -> serde_json::Value {
            serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
        }
    }
}

struct Proc {
    epoch: u64,
    pid: u32,
    stdin: mpsc::UnboundedSender<String>,
    driver: Box<dyn StructuredDriver>,
}

struct ActiveTurn {
    id: Id,
    interrupting: bool,
}

struct State {
    session: RuntimeSession,
    next_seq: u64,
    reserved: u64,
    epoch: u64,
    ring: EventRing,
    proj: Projection,
    turn: Option<ActiveTurn>,
    proc: Option<Proc>,
    /// Streaming upserts waiting out their coalescing window (latest per id).
    held: Vec<TurnItem>,
    /// When each streaming item was last emitted.
    stream_emitted: HashMap<Id, Instant>,
}

impl State {
    /// When the oldest held streaming upsert is due.
    fn held_deadline(&self, window: Duration) -> Option<Instant> {
        self.held
            .iter()
            .filter_map(|i| self.stream_emitted.get(&i.id))
            .min()
            .map(|t| *t + window)
    }
}

struct Live {
    spawn_lock: tokio::sync::Mutex<()>,
    state: Mutex<State>,
    persist: mpsc::UnboundedSender<PersistOp>,
}

impl Live {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Inner {
    repo: AgentRuntimeRepo,
    host: Arc<dyn RuntimeHost>,
    drivers: Arc<dyn DriverFactory>,
    config: RuntimeConfig,
    live: DashMap<Id, Arc<Live>>,
}

/// Cheap to clone.
#[derive(Clone)]
pub struct AgentRuntime {
    inner: Arc<Inner>,
}

impl AgentRuntime {
    pub fn new(
        repo: AgentRuntimeRepo,
        host: Arc<dyn RuntimeHost>,
        drivers: Arc<dyn DriverFactory>,
        config: RuntimeConfig,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                repo,
                host,
                drivers,
                config,
                live: DashMap::new(),
            }),
        }
    }

    pub fn repo(&self) -> &AgentRuntimeRepo {
        &self.inner.repo
    }

    /// Boot recovery (T3 Code `reconcileProjection`,
    /// `ProviderRuntimeRecoveryService.ts:369-396`): every turn a previous
    /// life left open ends `interrupted` (T3 Code says `cancelled`; Otto
    /// reports what happened to the person — the daemon stopped it), its open
    /// items too, every pending request `expired` / not resumable, the
    /// provider session `stopped`. Written above the persisted sequence
    /// floor. The next prompt respawns the CLI and resumes the provider's own
    /// session id. Returns sessions touched.
    pub async fn recover(&self) -> Result<usize> {
        let mut touched = 0;
        for sid in self.inner.repo.session_ids().await? {
            let rec = self.inner.repo.get(&sid).await?;
            let mut seq = rec.seq_reserved.max(rec.session.last_sequence);
            let now = Utc::now();
            let mut rows = Vec::new();
            for mut r in self.inner.repo.entities(&sid).await? {
                let changed = match r.kind.as_str() {
                    K_TURN => serde_json::from_value::<ProviderTurn>(r.payload.clone())
                        .ok()
                        .filter(|t| !t.status.is_terminal())
                        .map(|mut t| {
                            t.status = ProviderTurnStatus::Interrupted;
                            t.completed_at = Some(now);
                            serde_json::to_value(t)
                        }),
                    K_ITEM => serde_json::from_value::<TurnItem>(r.payload.clone())
                        .ok()
                        .filter(|i| i.status.is_open() || i.is_streaming())
                        .map(|mut i| {
                            settle_item(&mut i, TurnItemStatus::Interrupted, now);
                            serde_json::to_value(i)
                        }),
                    K_REQUEST => serde_json::from_value::<RuntimeRequest>(r.payload.clone())
                        .ok()
                        .filter(|q| q.status == RuntimeRequestStatus::Pending)
                        .map(|mut q| {
                            q.status = RuntimeRequestStatus::Expired;
                            q.response_capability = ResponseCapability::NotResumable {
                                reason: "The server restarted while this request was pending."
                                    .into(),
                            };
                            serde_json::to_value(q)
                        }),
                    K_SESSION => serde_json::from_value::<ProviderSession>(r.payload.clone())
                        .ok()
                        .filter(|s| s.status != ProviderSessionStatus::Stopped)
                        .map(|mut s| {
                            s.status = ProviderSessionStatus::Stopped;
                            s.updated_at = now;
                            serde_json::to_value(s)
                        }),
                    _ => None,
                };
                if let Some(Ok(v)) = changed {
                    seq += 1;
                    r.payload = v;
                    r.sequence = seq;
                    rows.push(r);
                }
            }
            if !rows.is_empty() {
                touched += 1;
                self.inner.repo.upsert_entities(&sid, &rows).await?;
            }
        }
        Ok(touched)
    }

    /// Ask every live child to exit (stdin EOF, then SIGTERM) — daemon shutdown.
    pub fn shutdown(&self) {
        for entry in self.inner.live.iter() {
            let mut st = entry.value().lock();
            if let Some(p) = st.proc.take() {
                stop_child(p, self.inner.config.close_grace);
            }
        }
    }

    // ---- sessions -------------------------------------------------------

    pub async fn create(
        &self,
        user: &Id,
        ws_id: &Id,
        req: CreateRuntimeSessionReq,
    ) -> Result<RuntimeSession> {
        let provider = req
            .provider
            .ok_or_else(|| Error::Invalid("provider is required".into()))?;
        if self.inner.drivers.create(provider).is_none() {
            return Err(Error::Invalid(format!(
                "no structured driver for provider '{}' yet",
                provider.as_str()
            )));
        }
        let ws = self.inner.host.workspace(ws_id).await?;
        let cwd = req
            .cwd
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| ws.root_path.clone());
        if !std::path::Path::new(&cwd).is_absolute() {
            return Err(Error::Invalid("cwd must be an absolute path".into()));
        }
        let title = req
            .title
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| format!("{} thread", provider.as_str()));
        let model = req
            .model
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty());
        let rec = self
            .inner
            .repo
            .create(NewRuntimeSession {
                id: new_id(),
                workspace_id: ws.id.clone(),
                provider,
                title,
                cwd,
                model,
                created_by: user.clone(),
            })
            .await?;
        let id = rec.session.id.clone();
        // Spawn now so a refused folder / missing CLI fails the create.
        let live = self.live(&id).await?;
        if let Err(e) = self.ensure_process(&live).await {
            self.inner.live.remove(&id);
            let _ = self.inner.repo.delete(&id).await;
            return Err(e);
        }
        let s = live.lock().session.clone();
        Ok(s)
    }

    pub async fn get(&self, id: &Id) -> Result<RuntimeSession> {
        if let Some(live) = self.inner.live.get(id).map(|l| Arc::clone(l.value())) {
            return Ok(live.lock().session.clone());
        }
        Ok(self.inner.repo.get(id).await?.session)
    }

    pub async fn list(&self, ws: &Id, owner: Option<&Id>) -> Result<Vec<RuntimeSession>> {
        let recs = self.inner.repo.list(ws, owner).await?;
        Ok(recs
            .into_iter()
            .map(|r| match self.inner.live.get(&r.session.id) {
                Some(l) => l.lock().session.clone(),
                None => r.session,
            })
            .collect())
    }

    /// The session's live entry, loaded from the DB on first use in this
    /// daemon life (its sequence resumes above the persisted floor).
    async fn live(&self, id: &Id) -> Result<Arc<Live>> {
        if let Some(l) = self.inner.live.get(id) {
            return Ok(Arc::clone(l.value()));
        }
        let rec = self.inner.repo.get(id).await?;
        let proj = Projection::load(self.inner.repo.entities(id).await?);
        let (tx, rx) = mpsc::unbounded_channel();
        let next_seq = rec.seq_reserved.max(rec.session.last_sequence) + 1;
        let live = Arc::new(Live {
            spawn_lock: tokio::sync::Mutex::new(()),
            state: Mutex::new(State {
                session: rec.session,
                next_seq,
                reserved: rec.seq_reserved,
                epoch: 0,
                ring: EventRing::new(self.inner.config.ring),
                proj,
                turn: None,
                proc: None,
                held: Vec::new(),
                stream_emitted: HashMap::new(),
            }),
            persist: tx,
        });
        let entry = self.inner.live.entry(id.clone()).or_insert_with(|| {
            tokio::spawn(persister(self.inner.repo.clone(), id.clone(), rx));
            Arc::clone(&live)
        });
        Ok(Arc::clone(entry.value()))
    }

    async fn ensure_process(&self, live: &Arc<Live>) -> Result<()> {
        let _spawning = live.spawn_lock.lock().await;
        let session = {
            let st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed {
                return Err(Error::Conflict("the session is closed".into()));
            }
            if st.proc.is_some() {
                return Ok(());
            }
            st.session.clone()
        };
        let mut driver = self.inner.drivers.create(session.provider).ok_or_else(|| {
            Error::Invalid(format!(
                "no structured driver for provider '{}'",
                session.provider.as_str()
            ))
        })?;
        let ws = self.inner.host.workspace(&session.workspace_id).await?;
        let ctx = LaunchContext {
            session_id: session.id.clone(),
            cwd: session.cwd.clone(),
            model: session.model.clone(),
            resume: session.native_thread_id.clone(),
            sandboxed: self.inner.host.sandboxed(&session).await,
        };
        let mut spec = driver.launch(&ctx);
        self.inner
            .host
            .prepare_spawn(&ws, &session, &mut spec)
            .await?;
        // A host that left the tail in place (tests) still gets it last.
        let mut args = spec.args;
        args.append(&mut spec.tail_args);
        let child = spawn_child(&ChildCommand {
            program: spec.program.clone(),
            args,
            cwd: session.cwd.clone(),
            env: spec.env,
        })
        .map_err(|e| {
            Error::Invalid(format!(
                "could not start {} ({}): {e}",
                session.provider.as_str(),
                spec.program
            ))
        })?;
        let (epoch, flushed) = {
            let mut st = live.lock();
            st.epoch += 1;
            let epoch = st.epoch;
            let step = driver.start(&ctx);
            st.proc = Some(Proc {
                epoch,
                pid: child.pid,
                stdin: child.stdin,
                driver,
            });
            st.reserved = st.next_seq + self.inner.config.seq_reserve;
            let _ = live.persist.send(PersistOp::Reserve(st.reserved));
            let (ftx, frx) = oneshot::channel();
            let _ = live.persist.send(PersistOp::Flush(ftx));
            set_session_status(
                &self.inner,
                live,
                &mut st,
                ProviderSessionStatus::Starting,
                None,
            );
            apply_step(&self.inner, live, &mut st, step);
            (epoch, frx)
        };
        tokio::spawn(pump(
            Arc::clone(&self.inner),
            Arc::clone(live),
            epoch,
            child.messages,
        ));
        // This life's sequence floor is durable before we return.
        let _ = flushed.await;
        Ok(())
    }

    // ---- turn control ---------------------------------------------------

    pub async fn prompt(&self, id: &Id, text: &str) -> Result<RuntimePromptResp> {
        if text.trim().is_empty() {
            return Err(Error::Invalid("prompt text is empty".into()));
        }
        let live = self.live(id).await?;
        check_can_prompt(&live.lock())?;
        self.ensure_process(&live).await?;
        let mut st = live.lock();
        check_can_prompt(&st)?;
        let turn_id = new_id();
        let Some(proc) = st.proc.as_mut() else {
            return Err(Error::Conflict("the agent process exited; retry".into()));
        };
        let step = proc
            .driver
            .prompt(&turn_id, text)
            .map_err(|e| Error::Conflict(e.0))?;
        let now = Utc::now();
        st.proj.next_turn_ordinal += 1;
        let turn = ProviderTurn {
            id: turn_id.clone(),
            ordinal: st.proj.next_turn_ordinal,
            status: ProviderTurnStatus::Running,
            started_at: Some(now),
            completed_at: None,
            turn_token_usage: None,
        };
        st.turn = Some(ActiveTurn {
            id: turn_id.clone(),
            interrupting: false,
        });
        let sequence = emit(
            &self.inner,
            &live,
            &mut st,
            RuntimeEventBody::ProviderTurnUpdated {
                provider_turn: turn,
            },
        );
        let mut user = item(
            format!("{turn_id}:user"),
            TurnItemStatus::Completed,
            None,
            TurnItemKind::UserMessage {
                text: text.to_string(),
            },
        );
        user.turn_id = Some(turn_id.clone());
        upsert_item(&self.inner, &live, &mut st, user);
        set_session_status(
            &self.inner,
            &live,
            &mut st,
            ProviderSessionStatus::Running,
            None,
        );
        st.session.last_active_at = now;
        apply_step(&self.inner, &live, &mut st, step);
        Ok(RuntimePromptResp { turn_id, sequence })
    }

    /// Interrupt the running turn. `false` when no turn was running.
    pub async fn interrupt(&self, id: &Id) -> Result<bool> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        let Some(turn) = st.turn.as_mut() else {
            return Ok(false);
        };
        if turn.interrupting {
            return Ok(true);
        }
        turn.interrupting = true;
        let turn_id = turn.id.clone();
        cancel_pending(&self.inner, &live, &mut st, RuntimeRequestStatus::Cancelled);
        let Some(proc) = st.proc.as_mut() else {
            finish_turn(
                &self.inner,
                &live,
                &mut st,
                TurnTerminalStatus::Interrupted,
                None,
                None,
            );
            return Ok(true);
        };
        let step = proc.driver.interrupt();
        let (epoch, pid) = (proc.epoch, proc.pid);
        apply_step(&self.inner, &live, &mut st, step);
        drop(st);
        let grace = self.inner.config.interrupt_grace;
        let live = Arc::clone(&live);
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let st = live.lock();
            let stuck = st.turn.as_ref().is_some_and(|t| t.id == turn_id)
                && st.proc.as_ref().is_some_and(|p| p.epoch == epoch);
            if stuck {
                tracing::warn!("runtime turn ignored interrupt; killing its process group");
                signal_group(pid, libc::SIGKILL);
            }
        });
        Ok(true)
    }

    /// Answer a pending request (T3 Code `runtime-request.respond`).
    pub async fn respond(
        &self,
        id: &Id,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<()> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        let pending = st
            .proj
            .requests
            .get(request_id)
            .is_some_and(|r| r.status == RuntimeRequestStatus::Pending);
        if !pending {
            return Err(Error::NotFound("no such pending request".into()));
        }
        let offered = st
            .proj
            .request_items
            .get(request_id)
            .and_then(|iid| st.proj.items.get(iid))
            .is_some_and(|item| match &item.kind {
                TurnItemKind::ApprovalRequest { options, .. } => {
                    decision == ApprovalDecision::Cancel
                        || options.iter().any(|o| o.decision == decision)
                }
                _ => false,
            });
        if !offered {
            return Err(Error::Invalid("that decision was not offered".into()));
        }
        let Some(proc) = st.proc.as_mut() else {
            return Err(Error::Conflict("the agent process exited".into()));
        };
        let step = proc
            .driver
            .respond(request_id, decision)
            .map_err(|e| Error::Conflict(e.0))?;
        resolve_request(
            &self.inner,
            &live,
            &mut st,
            request_id,
            RuntimeRequestStatus::Resolved,
            Some(decision),
        );
        if st.turn.is_some() && !has_pending(&st) {
            set_session_status(
                &self.inner,
                &live,
                &mut st,
                ProviderSessionStatus::Running,
                None,
            );
        }
        apply_step(&self.inner, &live, &mut st, step);
        Ok(())
    }

    /// Close the session: end any turn as interrupted, stop the child, refuse
    /// further prompts. Idempotent.
    pub async fn close(&self, id: &Id) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        if st.session.status == RuntimeSessionStatus::Closed {
            return Ok(st.session.clone());
        }
        finish_turn(
            &self.inner,
            &live,
            &mut st,
            TurnTerminalStatus::Interrupted,
            None,
            Some("the session was closed".into()),
        );
        if let Some(p) = st.proc.take() {
            stop_child(p, self.inner.config.close_grace);
        }
        set_session_status(
            &self.inner,
            &live,
            &mut st,
            ProviderSessionStatus::Stopped,
            None,
        );
        st.session.status = RuntimeSessionStatus::Closed;
        let _ = live.persist.send(PersistOp::Closed);
        Ok(st.session.clone())
    }

    // ---- replay -----------------------------------------------------------

    /// The events after `after_sequence`, or a snapshot when the ring no
    /// longer holds them or they exceed T3 Code's replay limits.
    pub async fn replay(&self, id: &Id, after_sequence: u64) -> Result<RuntimeReplay> {
        let live = self.live(id).await?;
        let st = live.lock();
        let last_sequence = st.next_seq - 1;
        if after_sequence >= last_sequence {
            return Ok(RuntimeReplay::Events {
                events: Vec::new(),
                last_sequence,
            });
        }
        if let Some(events) =
            st.ring
                .replayable(after_sequence, MAX_REPLAY_EVENTS, MAX_REPLAY_BYTES)
        {
            return Ok(RuntimeReplay::Events {
                events,
                last_sequence,
            });
        }
        Ok(RuntimeReplay::Snapshot {
            snapshot_sequence: last_sequence,
            snapshot: Box::new(st.proj.snapshot()),
        })
    }

    /// Wait until every queued DB write of `id` landed (tests, shutdown).
    pub async fn flush(&self, id: &Id) {
        if let Some(live) = self.inner.live.get(id).map(|l| Arc::clone(l.value())) {
            let (tx, rx) = oneshot::channel();
            let _ = live.persist.send(PersistOp::Flush(tx));
            let _ = rx.await;
        }
    }
}

fn check_can_prompt(st: &State) -> Result<()> {
    if st.session.status == RuntimeSessionStatus::Closed {
        return Err(Error::Conflict("the session is closed".into()));
    }
    if st.turn.is_some() {
        return Err(Error::Conflict("a turn is already running".into()));
    }
    Ok(())
}

fn has_pending(st: &State) -> bool {
    st.proj
        .requests
        .values()
        .any(|r| r.status == RuntimeRequestStatus::Pending)
}

/// Close stdin (EOF), then SIGTERM and SIGKILL the group after `grace`.
fn stop_child(p: Proc, grace: Duration) {
    let pid = p.pid;
    drop(p);
    tokio::spawn(async move {
        tokio::time::sleep(grace).await;
        signal_group(pid, libc::SIGTERM);
        tokio::time::sleep(grace).await;
        signal_group(pid, libc::SIGKILL);
    });
}

/// Sequence, project, record and publish one event. Returns its sequence.
fn emit(inner: &Inner, live: &Live, st: &mut State, body: RuntimeEventBody) -> u64 {
    // Held streaming text goes out before anything that follows it.
    flush_held(inner, live, st);
    let seq = st.next_seq;
    st.next_seq += 1;
    if seq >= st.reserved {
        st.reserved = seq + inner.config.seq_reserve;
        let _ = live.persist.send(PersistOp::Reserve(st.reserved));
    }
    if let Some(row) = st.proj.apply(&body, seq) {
        let _ = live.persist.send(PersistOp::Upsert(vec![row]));
    }
    let ev = Arc::new(RuntimeEvent {
        session_id: st.session.id.clone(),
        sequence: seq,
        ts: Utc::now(),
        body,
    });
    st.ring.push(Arc::clone(&ev));
    st.session.last_sequence = seq;
    inner.host.publish(&st.session, ev);
    seq
}

/// Map the provider session status onto Otto's session status and emit
/// `provider_session.updated` when it changed.
fn set_session_status(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    status: ProviderSessionStatus,
    last_error: Option<String>,
) {
    if st.session.status != RuntimeSessionStatus::Closed {
        st.session.status = match status {
            ProviderSessionStatus::Starting => RuntimeSessionStatus::Starting,
            ProviderSessionStatus::Ready => RuntimeSessionStatus::Idle,
            ProviderSessionStatus::Running => RuntimeSessionStatus::Working,
            ProviderSessionStatus::Waiting => RuntimeSessionStatus::AwaitingPermission,
            ProviderSessionStatus::Stopped | ProviderSessionStatus::Error => {
                RuntimeSessionStatus::Dormant
            }
        };
    }
    let model = st.session.model.clone();
    let cwd = st.session.cwd.clone();
    let prev = st.proj.session.as_ref();
    if prev.is_some_and(|p| p.status == status && p.last_error == last_error) {
        return;
    }
    let provider_session = ProviderSession {
        status,
        cwd,
        model: prev.and_then(|p| p.model.clone()).or(model),
        updated_at: Utc::now(),
        last_error,
    };
    emit(
        inner,
        live,
        st,
        RuntimeEventBody::ProviderSessionUpdated { provider_session },
    );
}

/// Upsert a turn item: new items get the next ordinal, known ones keep theirs.
/// Emit every held streaming upsert now (in the order first held).
fn flush_held(inner: &Inner, live: &Live, st: &mut State) {
    if st.held.is_empty() {
        return;
    }
    let now = Instant::now();
    for item in std::mem::take(&mut st.held) {
        st.stream_emitted.insert(item.id.clone(), now);
        emit(
            inner,
            live,
            st,
            RuntimeEventBody::TurnItemUpdated { turn_item: item },
        );
    }
}

fn upsert_item(inner: &Inner, live: &Live, st: &mut State, mut item: TurnItem) {
    // Superseded: the newer state of a held item replaces it.
    st.held.retain(|h| h.id != item.id);
    let window = inner.config.stream_coalesce;
    if item.is_streaming() && !window.is_zero() {
        let now = Instant::now();
        let recent = st
            .stream_emitted
            .get(&item.id)
            .is_some_and(|t| now.duration_since(*t) < window);
        if recent && st.proj.items.contains_key(&item.id) {
            item.ordinal = st.proj.items[&item.id].ordinal;
            item.updated_at = Utc::now();
            st.held.push(item);
            return;
        }
        st.stream_emitted.insert(item.id.clone(), now);
    } else {
        st.stream_emitted.remove(&item.id);
    }
    item.ordinal = match st.proj.items.get(&item.id) {
        Some(existing) => existing.ordinal,
        None => {
            st.proj.next_item_ordinal += 1;
            st.proj.next_item_ordinal
        }
    };
    item.updated_at = Utc::now();
    emit(
        inner,
        live,
        st,
        RuntimeEventBody::TurnItemUpdated { turn_item: item },
    );
}

fn settle_item(item: &mut TurnItem, status: TurnItemStatus, now: chrono::DateTime<Utc>) {
    if item.status.is_open() {
        item.status = status;
        item.completed_at = Some(now);
    }
    match &mut item.kind {
        TurnItemKind::AssistantMessage { streaming, .. }
        | TurnItemKind::Reasoning { streaming, .. } => {
            *streaming = false;
        }
        _ => {}
    }
}

fn resolve_request(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    request_id: &str,
    status: RuntimeRequestStatus,
    decision: Option<ApprovalDecision>,
) {
    let Some(mut req) = st.proj.requests.get(request_id).cloned() else {
        return;
    };
    let now = Utc::now();
    req.status = status;
    req.resolved_at = Some(now);
    req.decision = decision;
    emit(
        inner,
        live,
        st,
        RuntimeEventBody::RuntimeRequestUpdated {
            runtime_request: req,
        },
    );
    let item = st
        .proj
        .request_items
        .get(request_id)
        .and_then(|iid| st.proj.items.get(iid))
        .cloned();
    if let Some(mut item) = item {
        let s = if status == RuntimeRequestStatus::Resolved {
            TurnItemStatus::Completed
        } else {
            TurnItemStatus::Cancelled
        };
        settle_item(&mut item, s, now);
        upsert_item(inner, live, st, item);
    }
}

fn cancel_pending(inner: &Inner, live: &Live, st: &mut State, status: RuntimeRequestStatus) {
    let mut ids: Vec<Id> = st
        .proj
        .requests
        .values()
        .filter(|r| r.status == RuntimeRequestStatus::Pending)
        .map(|r| r.id.clone())
        .collect();
    ids.sort();
    for id in ids {
        resolve_request(inner, live, st, &id, status, None);
    }
}

/// End the running turn: cancel its requests, settle its open items, set the
/// turn's terminal status, turn a failure into an `error` item (T3 Code's
/// ingestor does the same), emit `turn.terminal`.
fn finish_turn(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    status: TurnTerminalStatus,
    failure: Option<ProviderFailure>,
    reason: Option<String>,
) {
    let Some(active) = st.turn.take() else {
        return;
    };
    // Settle what the client saw last: the held text first.
    flush_held(inner, live, st);
    cancel_pending(inner, live, st, RuntimeRequestStatus::Cancelled);
    let now = Utc::now();
    let mut open: Vec<TurnItem> = st
        .proj
        .items
        .values()
        .filter(|i| {
            i.turn_id.as_deref() == Some(active.id.as_str())
                && (i.status.is_open() || i.is_streaming())
        })
        .cloned()
        .collect();
    open.sort_by_key(|i| i.ordinal);
    for mut item in open {
        settle_item(&mut item, status.item_status(), now);
        upsert_item(inner, live, st, item);
    }
    if let Some(mut turn) = st.proj.turns.get(&active.id).cloned() {
        turn.status = status.turn_status();
        turn.completed_at = Some(now);
        emit(
            inner,
            live,
            st,
            RuntimeEventBody::ProviderTurnUpdated {
                provider_turn: turn,
            },
        );
    }
    if let Some(f) = &failure {
        upsert_item(
            inner,
            live,
            st,
            TurnItem {
                id: format!("{}:error", active.id),
                turn_id: Some(active.id.clone()),
                ordinal: 0,
                status: TurnItemStatus::Failed,
                title: None,
                started_at: Some(now),
                completed_at: Some(now),
                updated_at: now,
                kind: TurnItemKind::Error { failure: f.clone() },
            },
        );
    }
    emit(
        inner,
        live,
        st,
        RuntimeEventBody::TurnTerminal {
            turn_id: active.id,
            status,
            failure,
            reason,
        },
    );
    let next = if st.proc.is_some() {
        ProviderSessionStatus::Ready
    } else {
        ProviderSessionStatus::Stopped
    };
    set_session_status(inner, live, st, next, None);
}

/// Apply a driver step: events, then stdin writes, then the signal.
fn apply_step(inner: &Inner, live: &Live, st: &mut State, step: DriverStep) {
    for ev in step.events {
        match ev {
            DriverEvent::Ready {
                native_thread_id,
                model,
            } => {
                if let Some(native) = native_thread_id {
                    if st.session.native_thread_id.as_deref() != Some(native.as_str()) {
                        st.session.native_thread_id = Some(native.clone());
                        let _ = live.persist.send(PersistOp::NativeThreadId(native));
                    }
                }
                if model.is_some() && st.session.model.is_none() {
                    st.session.model = model;
                }
                let provider_thread = ProviderThread {
                    native_thread_id: st.session.native_thread_id.clone(),
                    context_usage: st
                        .proj
                        .thread
                        .as_ref()
                        .and_then(|t| t.context_usage.clone()),
                    updated_at: Utc::now(),
                };
                emit(
                    inner,
                    live,
                    st,
                    RuntimeEventBody::ProviderThreadUpdated { provider_thread },
                );
                let status = if st.turn.is_some() {
                    ProviderSessionStatus::Running
                } else {
                    ProviderSessionStatus::Ready
                };
                set_session_status(inner, live, st, status, None);
            }
            DriverEvent::TurnItem(mut item) => {
                if item.turn_id.is_none() {
                    item.turn_id = st.turn.as_ref().map(|t| t.id.clone());
                }
                upsert_item(inner, live, st, item);
            }
            DriverEvent::Plan { steps, explanation } => {
                let turn_id = st.turn.as_ref().map(|t| t.id.clone());
                let id = format!("{}:plan", turn_id.as_deref().unwrap_or_default());
                let plan = PlanArtifact {
                    id: id.clone(),
                    turn_id: turn_id.clone(),
                    steps: steps.clone(),
                    explanation: explanation.clone(),
                };
                emit(inner, live, st, RuntimeEventBody::PlanUpdated { plan });
                let mut todo = item(
                    id.clone(),
                    TurnItemStatus::Running,
                    None,
                    TurnItemKind::TodoList {
                        plan_id: id,
                        steps,
                        explanation,
                    },
                );
                todo.turn_id = turn_id;
                upsert_item(inner, live, st, todo);
            }
            DriverEvent::CommandOutput { item_id, chunk } => {
                emit(
                    inner,
                    live,
                    st,
                    RuntimeEventBody::CommandOutputDelta { item_id, chunk },
                );
            }
            DriverEvent::Request {
                id,
                kind,
                title,
                prompt,
                reason,
                options,
            } => {
                let turn_id = st.turn.as_ref().map(|t| t.id.clone());
                let request = RuntimeRequest {
                    id: id.clone(),
                    turn_id: turn_id.clone(),
                    kind,
                    status: RuntimeRequestStatus::Pending,
                    response_capability: ResponseCapability::Live,
                    created_at: Utc::now(),
                    resolved_at: None,
                    decision: None,
                };
                let mut item = item(
                    format!("{}:approval:{id}", turn_id.as_deref().unwrap_or_default()),
                    TurnItemStatus::Waiting,
                    title.clone(),
                    TurnItemKind::ApprovalRequest {
                        request_id: id,
                        request_kind: kind,
                        prompt: prompt.or(title),
                        reason,
                        options,
                    },
                );
                item.turn_id = turn_id;
                emit(
                    inner,
                    live,
                    st,
                    RuntimeEventBody::RuntimeRequestUpdated {
                        runtime_request: request,
                    },
                );
                upsert_item(inner, live, st, item);
                set_session_status(inner, live, st, ProviderSessionStatus::Waiting, None);
            }
            DriverEvent::RequestClosed { request_id, status } => {
                let open = st
                    .proj
                    .requests
                    .get(&request_id)
                    .is_some_and(|r| r.status == RuntimeRequestStatus::Pending);
                if open {
                    resolve_request(inner, live, st, &request_id, status, None);
                    if !has_pending(st) && st.turn.is_some() {
                        set_session_status(inner, live, st, ProviderSessionStatus::Running, None);
                    }
                }
            }
            DriverEvent::Usage(usage) => {
                let turn = st
                    .turn
                    .as_ref()
                    .and_then(|t| st.proj.turns.get(&t.id))
                    .cloned();
                if let Some(mut turn) = turn {
                    turn.turn_token_usage = Some(usage);
                    emit(
                        inner,
                        live,
                        st,
                        RuntimeEventBody::ProviderTurnUpdated {
                            provider_turn: turn,
                        },
                    );
                }
            }
            DriverEvent::ContextUsage(usage) => {
                let provider_thread = ProviderThread {
                    native_thread_id: st.session.native_thread_id.clone(),
                    context_usage: Some(usage),
                    updated_at: Utc::now(),
                };
                emit(
                    inner,
                    live,
                    st,
                    RuntimeEventBody::ProviderThreadUpdated { provider_thread },
                );
            }
            DriverEvent::Terminal {
                status,
                failure,
                reason,
            } => {
                let Some(active) = st.turn.as_ref() else {
                    continue;
                };
                // An interrupted turn the provider ends as failed is still
                // "interrupted" to the client (T3 Code's Codex adapter overrides
                // the terminal the same way).
                let status = if active.interrupting && status == TurnTerminalStatus::Failed {
                    TurnTerminalStatus::Interrupted
                } else {
                    status
                };
                finish_turn(inner, live, st, status, failure, reason);
            }
            DriverEvent::Error(failure) => {
                let mut e = item(
                    new_id(),
                    TurnItemStatus::Failed,
                    None,
                    TurnItemKind::Error { failure },
                );
                e.turn_id = st.turn.as_ref().map(|t| t.id.clone());
                upsert_item(inner, live, st, e);
            }
        }
    }
    if let Some(p) = st.proc.as_ref() {
        for line in step.writes {
            let _ = p.stdin.send(line);
        }
        if step.signal == Some(ChildSignal::Interrupt) {
            signal_group(p.pid, libc::SIGINT);
        }
    }
}

/// Feed one child's messages to its driver until it exits.
async fn pump(inner: Arc<Inner>, live: Arc<Live>, epoch: u64, mut rx: mpsc::Receiver<ChildMsg>) {
    let window = inner.config.stream_coalesce;
    loop {
        let due = live.lock().held_deadline(window);
        let msg = match due {
            Some(at) => match tokio::time::timeout_at(at.into(), rx.recv()).await {
                Ok(m) => m,
                Err(_) => {
                    let mut st = live.lock();
                    if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                        return; // closed or superseded
                    }
                    flush_held(&inner, &live, &mut st);
                    continue;
                }
            },
            None => rx.recv().await,
        };
        let Some(msg) = msg else { break };
        let mut st = live.lock();
        if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
            return; // closed or superseded
        }
        match msg {
            ChildMsg::Line(line) => {
                let step = st
                    .proc
                    .as_mut()
                    .expect("checked above")
                    .driver
                    .on_line(&line);
                apply_step(&inner, &live, &mut st, step);
            }
            ChildMsg::Overlong(n) => {
                let step = DriverStep {
                    events: vec![DriverEvent::Error(ProviderFailure {
                        class: "protocol_error".into(),
                        message: format!("dropped an agent output line of {n} bytes"),
                        code: None,
                        retryable: None,
                    })],
                    ..DriverStep::default()
                };
                apply_step(&inner, &live, &mut st, step);
            }
            ChildMsg::Exited { code, stderr_tail } => {
                let mut proc = st.proc.take().expect("checked above");
                let step = proc.driver.on_exit(code);
                apply_step(&inner, &live, &mut st, step);
                let detail = stderr_tail
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .map(|l| format!(": {}", l.trim()))
                    .unwrap_or_default();
                let code_s = code.map_or("a signal".to_string(), |c| format!("code {c}"));
                let message = format!("the agent process exited ({code_s}){detail}");
                if let Some(turn) = st.turn.as_ref() {
                    // T3 Code finalizes a turn whose process died as a
                    // `transport_error` failure.
                    let (status, failure) = if turn.interrupting {
                        (TurnTerminalStatus::Interrupted, None)
                    } else {
                        (
                            TurnTerminalStatus::Failed,
                            Some(ProviderFailure {
                                class: "transport_error".into(),
                                message: message.clone(),
                                code: code.map(|c| c.to_string()),
                                retryable: Some(true),
                            }),
                        )
                    };
                    finish_turn(&inner, &live, &mut st, status, failure, None);
                } else {
                    cancel_pending(&inner, &live, &mut st, RuntimeRequestStatus::Cancelled);
                }
                let last_error = (code != Some(0)).then_some(message);
                set_session_status(
                    &inner,
                    &live,
                    &mut st,
                    ProviderSessionStatus::Stopped,
                    last_error,
                );
                return;
            }
        }
    }
}

async fn persister(repo: AgentRuntimeRepo, id: Id, mut rx: mpsc::UnboundedReceiver<PersistOp>) {
    while let Some(op) = rx.recv().await {
        let res = match op {
            PersistOp::Upsert(rows) => repo.upsert_entities(&id, &rows).await,
            PersistOp::Reserve(upto) => repo.reserve_seq(&id, upto).await,
            PersistOp::NativeThreadId(n) => repo.set_native_thread_id(&id, &n).await,
            PersistOp::Closed => repo.set_closed(&id).await,
            PersistOp::Flush(done) => {
                let _ = done.send(());
                Ok(())
            }
        };
        if let Err(e) = res {
            tracing::warn!(session = %id, "agent runtime persistence failed: {e}");
        }
    }
}
