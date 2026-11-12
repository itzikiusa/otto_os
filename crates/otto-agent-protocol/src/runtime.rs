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

mod fresh;
mod request_ids;
use request_ids::RequestIds;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use chrono::Utc;
use dashmap::DashMap;
use otto_core::agent_runtime::{
    ApprovalDecision, Attachment, AttachmentKind, CancelReason, CreateRuntimeSessionReq,
    OpenPeerReq, PeerInfo, PeerReply, PlanArtifact, PromptDelivery, ProviderFailure,
    ProviderSession, ProviderSessionStatus, ProviderThread, ProviderTurn, ProviderTurnStatus,
    QueuedPrompt, ResponseCapability, RuntimeEvent, RuntimeEventBody, RuntimePromptReq,
    RuntimePromptResp, RuntimeReplay, RuntimeRequest, RuntimeRequestStatus, RuntimeSession,
    RuntimeSessionStatus, RuntimeSnapshot, TurnItem, TurnItemKind, TurnItemStatus, TurnOrigin,
    TurnTerminalStatus,
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
/// Open linked threads one group may hold besides its root (`open_peer`).
pub const MAX_LINKED_OPEN: usize = 8;

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
    /// Prompt attachments, one folder per session (`<dir>/<session>/<id>`).
    pub attachments_dir: PathBuf,
    /// How long a reopen waits for the resumed agent to get ready before
    /// trusting the resume (an exit before then = refused → fresh thread).
    pub start_wait: Duration,
    /// A turn that emits nothing for this long is stopped as stalled —
    /// unless it waits on a person (a pending approval) or on background
    /// work. Zero disables.
    pub stall_after: Duration,
    /// The same allowance while one of the turn's tools is still running
    /// (a long build, a subagent).
    pub stall_tool_after: Duration,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            ring: RingLimits::default(),
            seq_reserve: 65_536,
            interrupt_grace: Duration::from_secs(10),
            close_grace: Duration::from_secs(3),
            stream_coalesce: Duration::from_millis(50),
            attachments_dir: std::env::temp_dir().join("otto-runtime-attachments"),
            start_wait: Duration::from_secs(20),
            stall_after: Duration::from_secs(15 * 60),
            stall_tool_after: Duration::from_secs(45 * 60),
        }
    }
}

const K_SESSION: &str = "provider_session";
const K_THREAD: &str = "provider_thread";
const K_TURN: &str = "turn";
const K_ITEM: &str = "turn_item";
const K_REQUEST: &str = "runtime_request";
const K_PLAN: &str = "plan";
const K_QUEUE: &str = "queue";

/// A queued message as persisted: the whole prompt (its images / files
/// too), so it goes out unchanged after a restart.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct QueuedRecord {
    id: Id,
    created_at: chrono::DateTime<Utc>,
    req: RuntimePromptReq,
}

impl QueuedRecord {
    fn public(&self) -> QueuedPrompt {
        QueuedPrompt {
            id: self.id.clone(),
            text: self.req.text.clone(),
            attachments: (self.req.images.len() + self.req.files.len()).min(u32::MAX as usize)
                as u32,
            created_at: self.created_at,
        }
    }
}

enum PersistOp {
    Upsert(Vec<RuntimeEntityRow>),
    Reserve(u64),
    NativeThreadId(String),
    ClearNativeThreadId,
    Closed,
    Opened,
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
    /// Messages waiting for the running turn to end, oldest first.
    queue: Vec<QueuedRecord>,
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
                K_QUEUE => {
                    p.queue = serde_json::from_value(v).unwrap_or_default();
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
            queued_prompts: self.queue.iter().map(QueuedRecord::public).collect(),
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
            // The queue's row (the whole prompts) is written by `emit_queue`.
            RuntimeEventBody::CommandOutputDelta { .. }
            | RuntimeEventBody::TurnTerminal { .. }
            | RuntimeEventBody::QueueUpdated { .. } => None,
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
    request_ids: RequestIds,
    pid: u32,
    stdin: mpsc::UnboundedSender<String>,
    driver: Box<dyn StructuredDriver>,
}

struct ActiveTurn {
    id: Id,
    interrupting: bool,
    /// Stopped by the stall watchdog.
    stalled: bool,
}

struct State {
    session: RuntimeSession,
    next_seq: u64,
    reserved: u64,
    epoch: u64,
    /// Native identity is committed only after the first fresh prompt is accepted.
    fresh_starting: bool,
    ring: EventRing,
    proj: Projection,
    turn: Option<ActiveTurn>,
    proc: Option<Proc>,
    /// Streaming upserts waiting out their coalescing window (latest per id).
    held: Vec<TurnItem>,
    /// When each streaming item was last emitted.
    stream_emitted: HashMap<Id, Instant>,
    /// When the session last emitted anything (the stall watchdog's clock).
    last_activity: Instant,
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
    /// Serializes prompt admission, queue delivery and fresh provider replacement.
    prompt_lock: tokio::sync::Mutex<()>,
    spawn_lock: tokio::sync::Mutex<()>,
    /// One queue drain at a time (a queued message is sent exactly once).
    drain_lock: tokio::sync::Mutex<()>,
    state: Mutex<State>,
    /// External turn/request changes can arm a deadline while the child is silent.
    deadline_changed: tokio::sync::Notify,
    persist: mpsc::UnboundedSender<PersistOp>,
}

impl Live {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Inner {
    /// This runtime, for work a sync path hands off (draining the queue).
    me: Weak<Inner>,
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
            inner: Arc::new_cyclic(|me| Inner {
                me: me.clone(),
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
    /// session id. Also removes the attachments of sessions that no longer
    /// exist (a workspace delete cascades their rows). Returns sessions touched.
    pub async fn recover(&self) -> Result<usize> {
        self.inner.repo.expire_drafts().await?;
        let mut touched = 0;
        let mut queued = Vec::new();
        for sid in self.inner.repo.session_ids().await? {
            let rec = self.inner.repo.get(&sid).await?;
            let mut seq = rec.seq_reserved.max(rec.session.last_sequence);
            let now = Utc::now();
            let mut rows = Vec::new();
            let entities = self.inner.repo.entities(&sid).await?;
            // Messages queued before the restart go out now.
            if !rec.closed
                && entities.iter().any(|e| {
                    e.kind == K_QUEUE && e.payload.as_array().is_some_and(|a| !a.is_empty())
                })
            {
                queued.push(sid.clone());
            }
            for mut r in entities {
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
                        .filter(|s| {
                            s.status != ProviderSessionStatus::Stopped
                                || !s.background_tasks.is_empty()
                        })
                        .map(|mut s| {
                            // The process (and its background work) is gone.
                            s.background_tasks.clear();
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
        if let Ok(mut dirs) = tokio::fs::read_dir(&self.inner.config.attachments_dir).await {
            while let Ok(Some(d)) = dirs.next_entry().await {
                let sid = d.file_name().to_string_lossy().to_string();
                if matches!(self.inner.repo.get(&sid).await, Err(Error::NotFound(_))) {
                    let _ = tokio::fs::remove_dir_all(d.path()).await;
                }
            }
        }
        for sid in queued {
            self.kick_queue(&sid);
        }
        Ok(touched)
    }

    /// Daemon teardown: close every live child's stdin and SIGTERM its
    /// process group now (no grace timer — the tokio runtime is about to
    /// stop), then wait for the queued DB writes. The next boot's
    /// [`Self::recover`] settles the turns they leave open.
    pub async fn shutdown(&self) {
        let lives: Vec<Arc<Live>> = self
            .inner
            .live
            .iter()
            .map(|e| Arc::clone(e.value()))
            .collect();
        for live in &lives {
            if let Some(p) = live.lock().proc.take() {
                let pid = p.pid;
                drop(p);
                signal_group(pid, libc::SIGTERM);
            }
        }
        for live in lives {
            let (tx, rx) = oneshot::channel();
            let _ = live.persist.send(PersistOp::Flush(tx));
            let _ = rx.await;
        }
    }

    // ---- sessions -------------------------------------------------------

    pub async fn create(
        &self,
        user: &Id,
        ws_id: &Id,
        req: CreateRuntimeSessionReq,
    ) -> Result<RuntimeSession> {
        self.create_linked(user, ws_id, req, None).await
    }

    async fn create_linked(
        &self,
        user: &Id,
        ws_id: &Id,
        req: CreateRuntimeSessionReq,
        parent_id: Option<Id>,
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
                access_mode: req
                    .access_mode
                    .unwrap_or_else(|| self.inner.host.default_access_mode()),
                created_by: user.clone(),
                parent_id,
                meta: req
                    .meta
                    .filter(|m| m.as_object().is_some_and(|o| !o.is_empty())),
            })
            .await?;
        let id = rec.session.id.clone();
        // Spawn now so a refused folder / missing CLI fails the create.
        let live = self.live(&id).await?;
        if let Err(e) = self.ensure_process(&live).await {
            self.inner.live.remove(&id);
            let _ = self.inner.repo.delete(&id).await;
            let _ = tokio::fs::remove_dir_all(self.attachments(&id)).await;
            return Err(e);
        }
        let s = live.lock().session.clone();
        Ok(s)
    }

    // ---- linked threads (agent ↔ agent) ----------------------------------

    /// The linked group `caller` belongs to: its root and every thread opened
    /// under it, same owner, not closed — `caller` first-class among them.
    async fn group(&self, caller: &RuntimeSession) -> Result<Vec<RuntimeSession>> {
        let root = caller
            .parent_id
            .clone()
            .unwrap_or_else(|| caller.id.clone());
        Ok(self
            .list(&caller.workspace_id, Some(&caller.created_by))
            .await?
            .into_iter()
            .filter(|s| s.status != RuntimeSessionStatus::Closed)
            .filter(|s| s.id == root || s.parent_id.as_ref() == Some(&root))
            .collect())
    }

    fn peer_info(s: &RuntimeSession, caller: &Id) -> PeerInfo {
        PeerInfo {
            id: s.id.clone(),
            provider: s.provider,
            title: s.title.clone(),
            status: s.status,
            is_self: &s.id == caller,
        }
    }

    /// `peer` when it is in `caller`'s group (and not `caller` itself).
    async fn group_member(&self, caller: &RuntimeSession, peer: &Id) -> Result<RuntimeSession> {
        if peer == &caller.id {
            return Err(Error::Invalid("that is this thread itself".into()));
        }
        self.group(caller)
            .await?
            .into_iter()
            .find(|s| &s.id == peer)
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "no linked thread '{peer}' (list them with otto_thread_list)"
                ))
            })
    }

    /// The header a peer's message arrives under, so the receiving agent
    /// knows who wrote it and how to answer.
    fn peer_header(from: &RuntimeSession, verb: &str) -> String {
        format!(
            "[{verb} thread \"{}\" ({}, id {}). Answer it with the otto_thread_message tool \
             (session_id {}); see every linked thread with otto_thread_list.]",
            from.title,
            from.provider.as_str(),
            from.id,
            from.id
        )
    }

    /// Open a linked thread from `caller`: same owner, workspace, cwd and
    /// access mode (never wider than the opener's), grouped under the
    /// caller's root; the opening prompt (if any) is sent framed. The
    /// daemon-only `req.cwd` / `req.meta` give the thread its own folder and
    /// meta (a delegated personal agent); a read-only opener's thread is
    /// read-only too, whatever `req.meta` says.
    pub async fn open_peer(&self, caller: &Id, req: OpenPeerReq) -> Result<RuntimeSession> {
        let c = self.get(caller).await?;
        if c.status == RuntimeSessionStatus::Closed {
            return Err(Error::Conflict("the session is closed".into()));
        }
        // One confused agent must not fan out without bound: `group` lists
        // the OPEN threads of this linked group, the root included.
        if self.group(&c).await?.len() > MAX_LINKED_OPEN {
            return Err(Error::Conflict(format!(
                "this thread group already has {MAX_LINKED_OPEN} open linked threads — close \
                 one (otto_thread_close) before opening another"
            )));
        }
        let root = c.parent_id.clone().unwrap_or_else(|| c.id.clone());
        let mut meta = req.meta;
        if c.meta_flag("read_only") {
            let m = meta.get_or_insert_with(|| serde_json::json!({}));
            if let Some(o) = m.as_object_mut() {
                o.insert("read_only".into(), serde_json::Value::Bool(true));
            }
        }
        let created = self
            .create_linked(
                &c.created_by,
                &c.workspace_id,
                CreateRuntimeSessionReq {
                    provider: Some(req.provider),
                    cwd: Some(req.cwd.unwrap_or_else(|| c.cwd.clone())),
                    title: req.title,
                    model: req.model,
                    access_mode: Some(c.access_mode),
                    meta,
                },
                Some(root),
            )
            .await?;
        if let Some(p) = req.prompt.filter(|p| !p.trim().is_empty()) {
            let text = format!("{}\n\n{p}", Self::peer_header(&c, "Opened by"));
            let mut prompt = RuntimePromptReq::of(&text);
            prompt.effort = req.effort.filter(|e| !e.trim().is_empty());
            self.prompt_with(&created.id, &prompt).await?;
        }
        self.get(&created.id).await
    }

    /// Every thread in `caller`'s linked group.
    pub async fn peers(&self, caller: &Id) -> Result<Vec<PeerInfo>> {
        let c = self.get(caller).await?;
        Ok(self
            .group(&c)
            .await?
            .iter()
            .map(|s| Self::peer_info(s, caller))
            .collect())
    }

    /// Send `text` to linked thread `peer` as its next turn. A peer still
    /// working its turn refuses (409) — read it with [`Self::peer_reply`]
    /// and send again.
    pub async fn message_peer(
        &self,
        caller: &Id,
        peer: &Id,
        text: &str,
    ) -> Result<RuntimePromptResp> {
        if text.trim().is_empty() {
            return Err(Error::Invalid("message text is empty".into()));
        }
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        let framed = format!("{}\n\n{text}", Self::peer_header(&c, "Message from"));
        self.prompt(&p.id, &framed).await.map_err(|e| match e {
            Error::Conflict(m) if m.contains("turn is already running") => Error::Conflict(format!(
                "thread \"{}\" is busy with a turn — wait for it with otto_thread_read, then send again",
                p.title
            )),
            other => other,
        })
    }

    /// Close linked thread `peer` (stop its agent; it stays in History). A
    /// thread closes the ones in its group, never itself or the group's
    /// root (the lead a person opened) — those stay the person's to close.
    pub async fn close_peer(&self, caller: &Id, peer: &Id) -> Result<RuntimeSession> {
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        if p.parent_id.is_none() {
            return Err(Error::Forbidden(
                "that is the group's lead thread — only a person closes it".into(),
            ));
        }
        self.close(&p.id).await
    }

    /// Linked thread `peer`'s latest reply: waits up to `wait` for its
    /// running turn to settle, then returns that turn's assistant text.
    pub async fn peer_reply(&self, caller: &Id, peer: &Id, wait: Duration) -> Result<PeerReply> {
        let c = self.get(caller).await?;
        let p = self.group_member(&c, peer).await?;
        let mut reply = self.latest_reply(&p.id, wait).await?;
        reply.peer.is_self = false;
        Ok(reply)
    }

    /// Session `id`'s latest reply: waits up to `wait` for it to settle —
    /// no turn running (the person's or one the agent started itself) and
    /// no background work pending — then returns the assistant messages of
    /// its last turn that has any, joined (a subagent's own messages are
    /// not the agent's answer). `busy` = the wait ran out. What a
    /// daemon-driven run (a personal agent's) waits on for its result.
    pub async fn latest_reply(&self, id: &Id, wait: Duration) -> Result<PeerReply> {
        let deadline = Instant::now() + wait;
        let live = self.live(id).await?;
        loop {
            if !is_busy(&live.lock()) || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let st = live.lock();
        let busy = is_busy(&st);
        let snap = st.proj.snapshot();
        let session = st.session.clone();
        drop(st);
        let mut turns: Vec<&ProviderTurn> = snap.turns.iter().collect();
        turns.sort_by_key(|t| std::cmp::Reverse(t.ordinal));
        let answer = |t: &ProviderTurn| {
            snap.turn_items
                .iter()
                .filter(|i| i.turn_id.as_ref() == Some(&t.id) && i.parent_item_id.is_none())
                .filter_map(|i| match &i.kind {
                    TurnItemKind::AssistantMessage { text, .. } if !text.trim().is_empty() => {
                        Some(text.as_str())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        let last = turns.first().copied();
        let text = turns
            .iter()
            .map(|t| answer(t))
            .find(|t| !t.is_empty())
            .unwrap_or_default();
        Ok(PeerReply {
            peer: Self::peer_info(&session, id),
            busy,
            background_tasks: session.background_tasks.len().min(u32::MAX as usize) as u32,
            turn_status: last
                .filter(|t| t.status.is_terminal())
                .and_then(|t| serde_json::to_value(t.status).ok())
                .and_then(|v| v.as_str().map(str::to_string)),
            text,
        })
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

    /// Record the agent-UI-control grant (closed sessions too — a revoke must
    /// always land). Keeps a live session's cached row in step.
    pub async fn set_ui_control(
        &self,
        id: &Id,
        record: serde_json::Value,
    ) -> Result<RuntimeSession> {
        self.inner.repo.set_ui_control(id, &record).await?;
        if let Some(live) = self.inner.live.get(id).map(|l| Arc::clone(l.value())) {
            let mut st = live.lock();
            st.session.ui_control = Some(record);
            return Ok(st.session.clone());
        }
        Ok(self.inner.repo.get(id).await?.session)
    }

    /// Rename (closed sessions too — they stay listed as history).
    pub async fn rename(&self, id: &Id, title: &str) -> Result<RuntimeSession> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::Invalid("title is empty".into()));
        }
        let live = self.live(id).await?;
        self.inner.repo.set_title(id, title).await?;
        let mut st = live.lock();
        st.session.title = title.to_string();
        Ok(st.session.clone())
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
            prompt_lock: tokio::sync::Mutex::new(()),
            spawn_lock: tokio::sync::Mutex::new(()),
            drain_lock: tokio::sync::Mutex::new(()),
            deadline_changed: tokio::sync::Notify::new(),
            state: Mutex::new(State {
                session: rec.session,
                next_seq,
                reserved: rec.seq_reserved,
                epoch: 0,
                fresh_starting: false,
                ring: EventRing::new(self.inner.config.ring),
                proj,
                turn: None,
                proc: None,
                held: Vec::new(),
                stream_emitted: HashMap::new(),
                last_activity: Instant::now(),
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
        let mut ctx = LaunchContext {
            session_id: session.id.clone(),
            cwd: session.cwd.clone(),
            model: session.model.clone(),
            resume: session.native_thread_id.clone(),
            sandboxed: self.inner.host.sandboxed(&session).await,
            access_mode: session.access_mode,
            mcp_servers: Vec::new(),
        };
        let mut spec = driver.launch(&ctx);
        ctx.mcp_servers = self
            .inner
            .host
            .prepare_spawn(&ws, &session, &mut spec, driver.mcp_delivery())
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
                request_ids: RequestIds::new(),
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
        self.prompt_with(id, &RuntimePromptReq::of(text)).await
    }

    /// A prompt with an optional model / effort / access mode and inline
    /// images and files.
    pub async fn prompt_with(&self, id: &Id, req: &RuntimePromptReq) -> Result<RuntimePromptResp> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        self.prompt_locked(id, &live, req).await
    }

    async fn prompt_locked(
        &self,
        id: &Id,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
    ) -> Result<RuntimePromptResp> {
        if req.text.trim().is_empty() && req.images.is_empty() && req.files.is_empty() {
            return Err(Error::Invalid("prompt text is empty".into()));
        }
        if let Some(bad) = req.images.iter().find(|i| !i.mime.starts_with("image/")) {
            return Err(Error::Invalid(format!(
                "'{}' is not an image type",
                bad.mime
            )));
        }
        if let Some(bad) = req.files.iter().find(|f| f.mime != "application/pdf") {
            return Err(Error::Invalid(format!(
                "'{}' ({}) is not supported; only PDF files can be attached",
                bad.name, bad.mime
            )));
        }
        if req.fresh_context {
            return self.prompt_fresh(id, live, req).await;
        }
        let first = check_can_prompt(&live.lock());
        match (first, req.delivery) {
            (Err(Error::Conflict(m)), Some(d)) if m == TURN_RUNNING => {
                return self.deliver_busy(id, live, req, d).await;
            }
            (r, _) => r?,
        }
        match self.prompt_now(id, live, req).await {
            Err(Error::Conflict(m)) if m == TURN_RUNNING && req.delivery.is_some() => {
                let d = req.delivery.unwrap_or(PromptDelivery::Queue);
                self.deliver_busy(id, live, req, d).await
            }
            other => other,
        }
    }

    /// Start a turn with `req` (no turn may be running).
    async fn prompt_now(
        &self,
        id: &Id,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
    ) -> Result<RuntimePromptResp> {
        let live = Arc::clone(live);
        let model = req.model.clone().filter(|m| !m.trim().is_empty());
        if model.is_some() || req.access_mode.is_some() {
            let _spawning = live.spawn_lock.lock().await;
            let (stale, model, mode) = {
                let mut st = live.lock();
                check_can_prompt(&st)?;
                let model = model.filter(|m| st.session.model.as_ref() != Some(m));
                let mode = req.access_mode.filter(|m| *m != st.session.access_mode);
                // A driver that cannot switch live restarts in the new
                // selection and resumes its thread.
                let live_switch = st.proc.as_ref().is_some_and(|p| {
                    (model.is_none() || p.driver.switches_model_live())
                        && mode.is_none_or(|m| p.driver.switches_access_live(m))
                });
                if let Some(m) = &model {
                    st.session.model = Some(m.clone());
                }
                if let Some(m) = mode {
                    st.session.access_mode = m;
                }
                let changed = model.is_some() || mode.is_some();
                (
                    (changed && !live_switch).then(|| st.proc.take()).flatten(),
                    model,
                    mode,
                )
            };
            if let Some(p) = stale {
                stop_child(p, self.inner.config.interrupt_grace);
            }
            if let Some(m) = &model {
                self.inner.repo.set_model(id, m).await?;
            }
            if let Some(m) = mode {
                self.inner.repo.set_access_mode(id, m).await?;
            }
        }
        self.spawn_resuming(id, &live).await?;
        if req.fresh_context && !self.fresh_process_ready(&live).await {
            return Err(Error::Invalid(
                "the fresh agent could not start — retry the message".into(),
            ));
        }
        let attachments = self.store_attachments(id, req).await?;
        let res = self.start_turn(&live, req, attachments.clone());
        if res.is_err() {
            for a in &attachments {
                let _ = tokio::fs::remove_file(self.attachments(id).join(&a.id)).await;
            }
        }
        res
    }

    fn start_turn(
        &self,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
        attachments: Vec<Attachment>,
    ) -> Result<RuntimePromptResp> {
        let mut st = live.lock();
        check_can_prompt(&st)?;
        let turn_id = new_id();
        let Some(proc) = st.proc.as_mut() else {
            // It died in its handshake (signed out, bad argv) before this
            // prompt reached it; the next prompt respawns.
            let why = st.proj.session.as_ref().and_then(|s| s.last_error.clone());
            return Err(Error::Invalid(format!(
                "the agent could not start: {}",
                why.unwrap_or_else(|| "it exited".into())
            )));
        };
        let step = proc
            .driver
            .prompt(&turn_id, req)
            .map_err(|e| Error::Invalid(e.0))?;
        if req.fresh_context {
            self.commit_fresh(live, &mut st, &turn_id);
        }
        let now = Utc::now();
        st.proj.next_turn_ordinal += 1;
        let turn = ProviderTurn {
            id: turn_id.clone(),
            ordinal: st.proj.next_turn_ordinal,
            status: ProviderTurnStatus::Running,
            started_at: Some(now),
            completed_at: None,
            turn_token_usage: None,
            origin: TurnOrigin::User,
        };
        st.turn = Some(ActiveTurn {
            id: turn_id.clone(),
            interrupting: false,
            stalled: false,
        });
        let sequence = emit(
            &self.inner,
            live,
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
                text: req.text.clone(),
                attachments,
            },
        );
        user.turn_id = Some(turn_id.clone());
        upsert_item(&self.inner, live, &mut st, user);
        set_session_status(
            &self.inner,
            live,
            &mut st,
            ProviderSessionStatus::Running,
            None,
        );
        st.session.last_active_at = now;
        apply_step(&self.inner, live, &mut st, step);
        Ok(RuntimePromptResp {
            turn_id,
            sequence,
            queued_id: None,
            steered: false,
        })
    }

    /// A message for a session whose turn is running: steer it into the
    /// turn (when the driver can), or queue it — first and stop the turn
    /// for `interrupt`.
    async fn deliver_busy(
        &self,
        id: &Id,
        live: &Arc<Live>,
        req: &RuntimePromptReq,
        delivery: PromptDelivery,
    ) -> Result<RuntimePromptResp> {
        if delivery == PromptDelivery::Steer {
            let attachments = self.store_attachments(id, req).await?;
            if let Some(resp) = steer_turn(&self.inner, live, req, attachments.clone()) {
                return Ok(resp);
            }
            for a in &attachments {
                let _ = tokio::fs::remove_file(self.attachments(id).join(&a.id)).await;
            }
        }
        let front = delivery == PromptDelivery::Interrupt;
        let (queued_id, sequence) = {
            let mut st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed {
                return Err(Error::Conflict("the session is closed".into()));
            }
            let rec = QueuedRecord {
                id: new_id(),
                created_at: Utc::now(),
                req: RuntimePromptReq {
                    delivery: None,
                    ..req.clone()
                },
            };
            let qid = rec.id.clone();
            if front {
                st.proj.queue.insert(0, rec);
            } else {
                st.proj.queue.push(rec);
            }
            (qid, emit_queue(&self.inner, live, &mut st))
        };
        if front {
            self.interrupt(id).await?;
        }
        // The turn may have ended meanwhile: nothing would drain it then.
        self.kick_queue(id);
        Ok(RuntimePromptResp {
            turn_id: String::new(),
            sequence,
            queued_id: Some(queued_id),
            steered: false,
        })
    }

    /// Drop a queued message (`false` when it is not queued any more).
    pub async fn unqueue(&self, id: &Id, queued_id: &str) -> Result<bool> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        let before = st.proj.queue.len();
        st.proj.queue.retain(|q| q.id != queued_id);
        if st.proj.queue.len() == before {
            return Ok(false);
        }
        emit_queue(&self.inner, &live, &mut st);
        Ok(true)
    }

    /// Deliver a queued message now: steer it into the running turn, or
    /// (`interrupt`) stop the turn and send it next. With no turn running
    /// it simply goes out.
    pub async fn deliver_queued(
        &self,
        id: &Id,
        queued_id: &str,
        delivery: PromptDelivery,
    ) -> Result<RuntimePromptResp> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        let rec = {
            let mut st = live.lock();
            let Some(n) = st.proj.queue.iter().position(|q| q.id == queued_id) else {
                return Err(Error::NotFound("that message is no longer queued".into()));
            };
            if delivery == PromptDelivery::Queue {
                return Ok(RuntimePromptResp {
                    turn_id: String::new(),
                    sequence: st.session.last_sequence,
                    queued_id: Some(queued_id.to_string()),
                    steered: false,
                });
            }
            let rec = st.proj.queue.remove(n);
            emit_queue(&self.inner, &live, &mut st);
            rec
        };
        let req = RuntimePromptReq {
            delivery: Some(delivery),
            ..rec.req.clone()
        };
        let res = self.prompt_locked(id, &live, &req).await;
        if res.is_err() {
            // Back where it was, so nothing is lost.
            let mut st = live.lock();
            if !st.proj.queue.iter().any(|q| q.id == rec.id) {
                st.proj.queue.insert(0, rec);
                emit_queue(&self.inner, &live, &mut st);
            }
        }
        res
    }

    /// Send the next queued message once no turn runs (spawned off the
    /// sync paths that end a turn).
    fn kick_queue(&self, id: &Id) {
        kick_queue(&self.inner, id);
    }

    async fn drain_queue(&self, id: &Id) {
        let Ok(live) = self.live(id).await else {
            return;
        };
        let _one = live.drain_lock.lock().await;
        let _prompt = live.prompt_lock.lock().await;
        let next = {
            let st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed || st.turn.is_some() {
                return;
            }
            st.proj.queue.first().cloned()
        };
        let Some(rec) = next else {
            return;
        };
        let res = self.prompt_now(id, &live, &rec.req).await;
        let mut st = live.lock();
        match res {
            // A turn started first: this one waits for its end.
            Err(Error::Conflict(m)) if m == TURN_RUNNING => return,
            Ok(_) => {}
            Err(e) => {
                let mut n = item(
                    new_id(),
                    TurnItemStatus::Failed,
                    None,
                    TurnItemKind::SystemNotice {
                        message: format!("A queued message could not be sent: {e}"),
                    },
                );
                n.turn_id = None;
                upsert_item(&self.inner, &live, &mut st, n);
            }
        }
        st.proj.queue.retain(|q| q.id != rec.id);
        emit_queue(&self.inner, &live, &mut st);
    }

    /// The session's attachment folder.
    fn attachments(&self, id: &Id) -> PathBuf {
        self.inner.config.attachments_dir.join(id)
    }

    /// Write the prompt's images and files under the session's folder.
    async fn store_attachments(&self, id: &Id, req: &RuntimePromptReq) -> Result<Vec<Attachment>> {
        use base64::Engine;
        let images = req
            .images
            .iter()
            .map(|i| (AttachmentKind::Image, &i.mime, None, &i.data_base64));
        let files = req
            .files
            .iter()
            .map(|f| (AttachmentKind::File, &f.mime, Some(&f.name), &f.data_base64));
        let mut out = Vec::new();
        for (kind, mime, name, data) in images.chain(files) {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|e| Error::Invalid(format!("an attachment is not valid base64: {e}")))?;
            let a = Attachment {
                id: new_id(),
                kind,
                mime: mime.clone(),
                name: name.cloned(),
            };
            let dir = self.attachments(id);
            tokio::fs::create_dir_all(&dir)
                .await
                .and(tokio::fs::write(dir.join(&a.id), bytes).await)
                .map_err(|e| Error::Internal(format!("could not store an attachment: {e}")))?;
            out.push(a);
        }
        Ok(out)
    }

    /// A stored attachment of `id`: its mime type and bytes.
    pub async fn attachment(&self, id: &Id, attachment_id: &str) -> Result<(String, Vec<u8>)> {
        let live = self.live(id).await?;
        let mime = live.lock().proj.items.values().find_map(|i| match &i.kind {
            TurnItemKind::UserMessage { attachments, .. } => attachments
                .iter()
                .find(|a| a.id == attachment_id)
                .map(|a| a.mime.clone()),
            _ => None,
        });
        let mime = mime.ok_or_else(|| Error::NotFound("no such attachment".into()))?;
        let bytes = tokio::fs::read(self.attachments(id).join(attachment_id))
            .await
            .map_err(|_| Error::NotFound("the attachment is gone".into()))?;
        Ok((mime, bytes))
    }

    /// Interrupt the running turn. `false` when no turn was running.
    pub async fn interrupt(&self, id: &Id) -> Result<bool> {
        let live = self.live(id).await?;
        Ok(interrupt_turn(&self.inner, &live))
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
        let native_id = proc.request_ids.native(request_id)?.to_string();
        let step = proc
            .driver
            .respond(&native_id, decision)
            .map_err(|e| Error::Conflict(e.0))?;
        resolve_request(
            &self.inner,
            &live,
            &mut st,
            request_id,
            RuntimeRequestStatus::Resolved,
            Some(decision),
            None,
        );
        if !has_pending(&st) {
            let next = settled_status(&st);
            set_session_status(&self.inner, &live, &mut st, next, None);
        }
        apply_step(&self.inner, &live, &mut st, step);
        Ok(())
    }

    /// Answer only a live structured question. Validation precedes consuming the native request.
    pub async fn respond_question(
        &self,
        id: &Id,
        request_id: &str,
        answers: Vec<otto_core::agent_runtime::QuestionAnswer>,
    ) -> Result<()> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        let request = st
            .proj
            .requests
            .get(request_id)
            .filter(|r| r.status == RuntimeRequestStatus::Pending)
            .ok_or_else(|| Error::NotFound("no such pending request".into()))?;
        if request.kind != otto_core::agent_runtime::RequestKind::Question {
            return Err(Error::Invalid(
                "this request needs an approval decision".into(),
            ));
        }
        otto_core::agent_runtime::validate_question_answers(&request.questions, &answers)
            .map_err(Error::Invalid)?;
        let Some(proc) = st.proc.as_mut() else {
            return Err(Error::Conflict("the agent process exited".into()));
        };
        let native_id = proc.request_ids.native(request_id)?.to_string();
        let step = proc
            .driver
            .respond_question(&native_id, &answers)
            .map_err(|e| Error::Conflict(e.0))?;
        if let Some(request) = st.proj.requests.get_mut(request_id) {
            request.answers = Some(answers);
        }
        resolve_request(
            &self.inner,
            &live,
            &mut st,
            request_id,
            RuntimeRequestStatus::Resolved,
            None,
            None,
        );
        if !has_pending(&st) {
            let next = settled_status(&st);
            set_session_status(&self.inner, &live, &mut st, next, None);
        }
        apply_step(&self.inner, &live, &mut st, step);
        Ok(())
    }

    pub async fn dismiss_question(
        &self,
        id: &Id,
        request_id: &str,
        action: otto_core::agent_runtime::QuestionDismissAction,
    ) -> Result<()> {
        let live = self.live(id).await?;
        let mut st = live.lock();
        let request = st
            .proj
            .requests
            .get(request_id)
            .filter(|r| r.status == RuntimeRequestStatus::Pending)
            .ok_or_else(|| Error::NotFound("no such pending request".into()))?;
        if request.kind != otto_core::agent_runtime::RequestKind::Question {
            return Err(Error::Invalid(
                "this request needs an approval decision".into(),
            ));
        }
        let Some(proc) = st.proc.as_mut() else {
            return Err(Error::Conflict("the agent process exited".into()));
        };
        let native_id = proc.request_ids.native(request_id)?.to_string();
        let step = proc
            .driver
            .dismiss_question(&native_id, action)
            .map_err(|e| Error::Conflict(e.0))?;
        resolve_request(
            &self.inner,
            &live,
            &mut st,
            request_id,
            RuntimeRequestStatus::Cancelled,
            None,
            None,
        );
        if !has_pending(&st) {
            let next = settled_status(&st);
            set_session_status(&self.inner, &live, &mut st, next, None);
        }
        apply_step(&self.inner, &live, &mut st, step);
        Ok(())
    }

    /// Close the session: end any turn as interrupted, stop the child, refuse
    /// further prompts. Idempotent.
    pub async fn close(&self, id: &Id) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        let mut st = live.lock();
        if st.session.status == RuntimeSessionStatus::Closed {
            return Ok(st.session.clone());
        }
        cancel_pending(&self.inner, &live, &mut st, CancelReason::Closed);
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
        settle_background(&self.inner, &live, &mut st, TurnItemStatus::Interrupted);
        retire(&self.inner, &st.session);
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

    /// Revive a closed or dormant session: it takes prompts again and its agent is
    /// respawned now, resuming the provider's own thread (`native_thread_id`).
    /// A provider that cannot resume it (its process dies before it is
    /// ready) gets a fresh thread instead and the timeline says so (a
    /// `system_notice`). Its meta is kept. A live process is left alone.
    pub async fn reopen(&self, id: &Id) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        {
            let mut st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed {
                st.session.status = RuntimeSessionStatus::Dormant;
                // After the close's own write, in order.
                let _ = live.persist.send(PersistOp::Opened);
            } else if st.proc.is_some() {
                return Ok(st.session.clone());
            }
        }
        self.flush(id).await;
        self.spawn_resuming(id, &live).await?;
        let s = live.lock().session.clone();
        Ok(s)
    }

    /// Spawn the agent if it is not running, resuming the provider's own
    /// thread. A provider that refuses the resume (its process dies before
    /// it is ready — e.g. Claude's "No conversation found" for a thread that
    /// never got a message before a restart) gets a fresh thread instead and
    /// the timeline says so; without this every later prompt failed the same
    /// way and the chat was dead for good.
    async fn spawn_resuming(&self, id: &Id, live: &Arc<Live>) -> Result<()> {
        if live.lock().proc.is_some() {
            return Ok(());
        }
        self.ensure_process(live).await?;
        let resume = live.lock().session.native_thread_id.clone();
        if resume.is_none() || self.wait_started(live).await {
            return Ok(());
        }
        // Keep the old key until the fresh thread is up.
        live.lock().session.native_thread_id = None;
        let fresh = self.ensure_process(live).await;
        if fresh.is_err() || !self.wait_started(live).await {
            live.lock().session.native_thread_id = resume;
            fresh?;
            return Err(Error::Invalid(
                "the agent could not start again — send a message to retry".into(),
            ));
        }
        if live.lock().session.native_thread_id.is_none() {
            self.inner.repo.clear_native_thread_id(id).await?;
        }
        self.notice(
            id,
            "The earlier conversation could not be resumed, so the agent starts fresh: \
             the history above is kept here, but the agent does not remember it.",
        )
        .await
    }

    /// Whether the spawned process got ready (`false` = it exited first).
    async fn wait_started(&self, live: &Arc<Live>) -> bool {
        let deadline = Instant::now() + self.inner.config.start_wait;
        loop {
            {
                let st = live.lock();
                if st.proc.is_none() {
                    return false;
                }
                let ready = st.proj.session.as_ref().is_some_and(|p| {
                    !matches!(
                        p.status,
                        ProviderSessionStatus::Starting | ProviderSessionStatus::Stopped
                    )
                });
                if ready {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return true; // slow, not refused: the next prompt finds out
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Add a `system_notice` to the timeline (in the running turn, if any) —
    /// the daemon telling the person something about the thread. Refused on
    /// a closed session (reopen it first).
    pub async fn notice(&self, id: &Id, message: &str) -> Result<()> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Error::Invalid("notice text is empty".into()));
        }
        let live = self.live(id).await?;
        let mut st = live.lock();
        if st.session.status == RuntimeSessionStatus::Closed {
            return Err(Error::Conflict("the session is closed".into()));
        }
        let mut n = item(
            new_id(),
            TurnItemStatus::Completed,
            None,
            TurnItemKind::SystemNotice {
                message: message.to_string(),
            },
        );
        n.turn_id = st.turn.as_ref().map(|t| t.id.clone());
        upsert_item(&self.inner, &live, &mut st, n);
        Ok(())
    }

    /// Delete the session: close it (stopping its agent), then remove its
    /// row, its projection and its attachments. Not undoable.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        self.close(id).await?;
        self.flush(id).await;
        self.inner.live.remove(id);
        self.inner.repo.delete(id).await?;
        let _ = tokio::fs::remove_dir_all(self.attachments(id)).await;
        Ok(())
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

const TURN_RUNNING: &str = "a turn is already running";

fn check_can_prompt(st: &State) -> Result<()> {
    if st.session.status == RuntimeSessionStatus::Closed {
        return Err(Error::Conflict("the session is closed".into()));
    }
    if st.turn.is_some() {
        return Err(Error::Conflict(TURN_RUNNING.into()));
    }
    Ok(())
}

/// Interrupt the running turn (a person's Stop, the stall watchdog): the
/// driver asks the provider to end it; past `interrupt_grace` its process
/// group is killed. `false` when no turn was running.
fn interrupt_turn(inner: &Arc<Inner>, live: &Arc<Live>) -> bool {
    let mut st = live.lock();
    let Some(turn) = st.turn.as_mut() else {
        return false;
    };
    if turn.interrupting {
        return true;
    }
    turn.interrupting = true;
    let turn_id = turn.id.clone();
    cancel_pending(inner, live, &mut st, CancelReason::Interrupt);
    let Some(proc) = st.proc.as_mut() else {
        finish_turn(
            inner,
            live,
            &mut st,
            TurnTerminalStatus::Interrupted,
            None,
            None,
        );
        return true;
    };
    let step = proc.driver.interrupt();
    let (epoch, pid) = (proc.epoch, proc.pid);
    apply_step(inner, live, &mut st, step);
    drop(st);
    let grace = inner.config.interrupt_grace;
    let live = Arc::clone(live);
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
    true
}

/// Steer `req` into the running turn when its driver takes it.
fn steer_turn(
    inner: &Inner,
    live: &Live,
    req: &RuntimePromptReq,
    attachments: Vec<Attachment>,
) -> Option<RuntimePromptResp> {
    let mut st = live.lock();
    let turn_id = st.turn.as_ref()?.id.clone();
    let step = st.proc.as_mut()?.driver.steer(req).ok()?;
    let mut user = item(
        format!("{turn_id}:steer:{}", new_id()),
        TurnItemStatus::Completed,
        None,
        TurnItemKind::UserMessage {
            text: req.text.clone(),
            attachments,
        },
    );
    user.turn_id = Some(turn_id.clone());
    upsert_item(inner, live, &mut st, user);
    let sequence = st.session.last_sequence;
    st.session.last_active_at = Utc::now();
    apply_step(inner, live, &mut st, step);
    Some(RuntimePromptResp {
        turn_id,
        sequence,
        queued_id: None,
        steered: true,
    })
}

/// Spawn a drain of `id`'s queue (no-op when the runtime is gone).
fn kick_queue(inner: &Inner, id: &Id) {
    if let Some(me) = inner.me.upgrade() {
        let rt = AgentRuntime { inner: me };
        let id = id.clone();
        tokio::spawn(async move { rt.drain_queue(&id).await });
    }
}

/// Persist and publish the queue (`queue.updated`). Returns its sequence.
fn emit_queue(inner: &Inner, live: &Live, st: &mut State) -> u64 {
    let queued_prompts = st.proj.queue.iter().map(QueuedRecord::public).collect();
    let seq = emit(
        inner,
        live,
        st,
        RuntimeEventBody::QueueUpdated { queued_prompts },
    );
    let _ = live.persist.send(PersistOp::Upsert(vec![RuntimeEntityRow {
        kind: K_QUEUE.into(),
        entity_id: "queue".into(),
        sequence: seq,
        payload: serde_json::to_value(&st.proj.queue).unwrap_or(serde_json::Value::Null),
    }]));
    seq
}

/// When the running turn counts as stalled, if it can: not while it waits
/// on a person or on background work; longer while one of its tools runs.
fn stall_deadline(st: &State, cfg: &RuntimeConfig) -> Option<Instant> {
    let turn = st.turn.as_ref().filter(|t| !t.interrupting)?;
    if cfg.stall_after.is_zero() || has_pending(st) || !st.session.background_tasks.is_empty() {
        return None;
    }
    let tool_running = st.proj.items.values().any(|i| {
        i.turn_id.as_deref() == Some(turn.id.as_str())
            && i.status.is_open()
            && matches!(
                i.kind,
                TurnItemKind::CommandExecution { .. }
                    | TurnItemKind::DynamicTool { .. }
                    | TurnItemKind::WebSearch { .. }
                    | TurnItemKind::FileChange { .. }
                    | TurnItemKind::Subagent { .. }
            )
    });
    let allowance = if tool_running {
        cfg.stall_tool_after.max(cfg.stall_after)
    } else {
        cfg.stall_after
    };
    Some(st.last_activity + allowance)
}

/// Working: a turn (the person's or the agent's own) or background work.
fn is_busy(st: &State) -> bool {
    st.turn.is_some() || !st.session.background_tasks.is_empty()
}

/// The provider status once no request is pending: running a turn, ready
/// between turns, stopped without a process.
fn settled_status(st: &State) -> ProviderSessionStatus {
    if st.turn.is_some() {
        ProviderSessionStatus::Running
    } else if st.proc.is_some() {
        ProviderSessionStatus::Ready
    } else {
        ProviderSessionStatus::Stopped
    }
}

/// The provider session's current status + error (to re-emit it with a
/// changed background list).
fn current_status(st: &State) -> (ProviderSessionStatus, Option<String>) {
    match st.proj.session.as_ref() {
        Some(p) => (p.status, p.last_error.clone()),
        None => (settled_status(st), None),
    }
}

/// The agent started a turn on its own (`origin: agent`): it runs like a
/// prompted one — Working, Stop interrupts it, prompts wait for its end.
fn open_agent_turn(inner: &Inner, live: &Live, st: &mut State, turn_id: String) {
    if st.turn.is_some() || st.session.status == RuntimeSessionStatus::Closed {
        return;
    }
    let now = Utc::now();
    st.proj.next_turn_ordinal += 1;
    let turn = ProviderTurn {
        id: turn_id.clone(),
        ordinal: st.proj.next_turn_ordinal,
        status: ProviderTurnStatus::Running,
        started_at: Some(now),
        completed_at: None,
        turn_token_usage: None,
        origin: TurnOrigin::Agent,
    };
    st.turn = Some(ActiveTurn {
        id: turn_id,
        interrupting: false,
        stalled: false,
    });
    emit(
        inner,
        live,
        st,
        RuntimeEventBody::ProviderTurnUpdated {
            provider_turn: turn,
        },
    );
    let status = if has_pending(st) {
        ProviderSessionStatus::Waiting
    } else {
        ProviderSessionStatus::Running
    };
    set_session_status(inner, live, st, status, None);
    st.session.last_active_at = now;
}

/// The process (and so its background work) is gone: settle every
/// background subagent still open and clear the background list.
fn settle_background(inner: &Inner, live: &Live, st: &mut State, status: TurnItemStatus) {
    let now = Utc::now();
    let mut open: Vec<TurnItem> = st
        .proj
        .items
        .values()
        .filter(|i| i.is_background_open())
        .cloned()
        .collect();
    open.sort_by_key(|i| i.ordinal);
    for mut item in open {
        settle_item(&mut item, status, now);
        upsert_item(inner, live, st, item);
    }
    if !st.session.background_tasks.is_empty() {
        st.session.background_tasks.clear();
        let (status, last_error) = current_status(st);
        set_session_status(inner, live, st, status, last_error);
    }
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
    st.last_activity = Instant::now();
    // Prompt admission and request responses need not produce child stdout.
    // Broadcast: a superseded process pump must not consume the new one's wake.
    live.deadline_changed.notify_waiters();
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
    let background_tasks = st.session.background_tasks.clone();
    let prev = st.proj.session.as_ref();
    if prev.is_some_and(|p| {
        p.status == status && p.last_error == last_error && p.background_tasks == background_tasks
    }) {
        return;
    }
    let provider_session = ProviderSession {
        status,
        cwd,
        model: prev.and_then(|p| p.model.clone()).or(model),
        updated_at: Utc::now(),
        last_error,
        background_tasks,
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
    cancel_reason: Option<CancelReason>,
) {
    let Some(mut req) = st.proj.requests.get(request_id).cloned() else {
        return;
    };
    let now = Utc::now();
    req.status = status;
    req.resolved_at = Some(now);
    req.decision = decision;
    req.cancel_reason = cancel_reason;
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
        if let TurnItemKind::ApprovalRequest {
            cancel_reason: r, ..
        } = &mut item.kind
        {
            *r = cancel_reason;
        }
        upsert_item(inner, live, st, item);
    }
}

fn cancel_pending(inner: &Inner, live: &Live, st: &mut State, reason: CancelReason) {
    let mut ids: Vec<Id> = st
        .proj
        .requests
        .values()
        .filter(|r| r.status == RuntimeRequestStatus::Pending)
        .map(|r| r.id.clone())
        .collect();
    ids.sort();
    for id in ids {
        resolve_request(
            inner,
            live,
            st,
            &id,
            RuntimeRequestStatus::Cancelled,
            None,
            Some(reason),
        );
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
    let reason = if active.stalled {
        Some("stalled".to_string())
    } else {
        reason
    };
    // Settle what the client saw last: the held text first.
    flush_held(inner, live, st);
    cancel_pending(inner, live, st, CancelReason::Agent);
    let now = Utc::now();
    let mut open: Vec<TurnItem> = st
        .proj
        .items
        .values()
        .filter(|i| {
            i.turn_id.as_deref() == Some(active.id.as_str())
                && (i.status.is_open() || i.is_streaming())
                && !i.is_background_open()
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
                error: None,
                parent_item_id: None,
                kind: TurnItemKind::Error { failure: f.clone() },
            },
        );
    }
    emit(
        inner,
        live,
        st,
        RuntimeEventBody::TurnTerminal {
            turn_id: active.id.clone(),
            status,
            failure,
            reason,
        },
    );
    if active.stalled {
        let mins = inner.config.stall_after.as_secs().div_ceil(60);
        let mut n = item(
            format!("{}:stalled", active.id),
            TurnItemStatus::Interrupted,
            None,
            TurnItemKind::SystemNotice {
                message: format!(
                    "Stopped: the agent showed no progress for about {mins} min. \
                     Send a message to continue."
                ),
            },
        );
        n.turn_id = Some(active.id.clone());
        upsert_item(inner, live, st, n);
    }
    let next = if st.proc.is_some() {
        ProviderSessionStatus::Ready
    } else {
        ProviderSessionStatus::Stopped
    };
    set_session_status(inner, live, st, next, None);
    if !st.proj.queue.is_empty() {
        kick_queue(inner, &st.session.id);
    }
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
                        if !st.fresh_starting {
                            let _ = live.persist.send(PersistOp::NativeThreadId(native));
                        }
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
                tool_item_id,
            } => {
                let id = scope_request_id(st, &id);
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
                    cancel_reason: None,
                    questions: vec![],
                    question_title: None,
                    answers: None,
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
                        tool_item_id,
                        cancel_reason: None,
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
            DriverEvent::Question {
                id,
                title,
                questions,
                tool_item_id: _,
            } => {
                let id = scope_request_id(st, &id);
                let turn_id = st.turn.as_ref().map(|t| t.id.clone());
                let request = RuntimeRequest {
                    id,
                    turn_id,
                    kind: otto_core::agent_runtime::RequestKind::Question,
                    status: RuntimeRequestStatus::Pending,
                    response_capability: ResponseCapability::Live,
                    created_at: Utc::now(),
                    resolved_at: None,
                    decision: None,
                    cancel_reason: None,
                    questions,
                    question_title: title,
                    answers: None,
                };
                emit(
                    inner,
                    live,
                    st,
                    RuntimeEventBody::RuntimeRequestUpdated {
                        runtime_request: request,
                    },
                );
                set_session_status(inner, live, st, ProviderSessionStatus::Waiting, None);
            }
            DriverEvent::RequestClosed { request_id, status } => {
                let request_id = scope_request_id(st, &request_id);
                let open = st
                    .proj
                    .requests
                    .get(&request_id)
                    .is_some_and(|r| r.status == RuntimeRequestStatus::Pending);
                if open {
                    let reason =
                        (status == RuntimeRequestStatus::Cancelled).then_some(CancelReason::Agent);
                    resolve_request(inner, live, st, &request_id, status, None, reason);
                    if !has_pending(st) {
                        let next = settled_status(st);
                        set_session_status(inner, live, st, next, None);
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
            DriverEvent::TurnStarted { turn_id } => open_agent_turn(inner, live, st, turn_id),
            DriverEvent::BackgroundTasks(tasks) => {
                if st.session.background_tasks != tasks {
                    st.session.background_tasks = tasks;
                    let (status, last_error) = current_status(st);
                    set_session_status(inner, live, st, status, last_error);
                }
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
            DriverEvent::Fatal(failure) => {
                if st.turn.is_some() {
                    finish_turn(
                        inner,
                        live,
                        st,
                        TurnTerminalStatus::Failed,
                        Some(failure.clone()),
                        None,
                    );
                }
                if let Some(p) = st.proc.take() {
                    stop_child(p, inner.config.close_grace);
                }
                set_session_status(
                    inner,
                    live,
                    st,
                    ProviderSessionStatus::Stopped,
                    Some(failure.message),
                );
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
        // Register before reading state, so a change between the snapshot and
        // select cannot leave an idle pump waiting indefinitely on child output.
        let changed = live.deadline_changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let due = {
            let st = live.lock();
            if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                return; // closed or superseded
            }
            let held = st.held_deadline(window);
            let stall = stall_deadline(&st, &inner.config);
            held.into_iter().chain(stall).min()
        };
        let msg = tokio::select! {
            _ = &mut changed => continue,
            msg = rx.recv() => msg,
            _ = async {
                match due {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                let mut st = live.lock();
                if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                    return; // closed or superseded
                }
                flush_held(&inner, &live, &mut st);
                let stalled =
                    stall_deadline(&st, &inner.config).is_some_and(|at| at <= Instant::now());
                if stalled {
                    tracing::warn!(session = %st.session.id, "runtime turn stalled; stopping it");
                    if let Some(t) = st.turn.as_mut() {
                        t.stalled = true;
                    }
                    drop(st);
                    interrupt_turn(&inner, &live);
                }
                continue;
            }
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
                retire(&inner, &st.session);
                let step = proc.driver.on_exit(code);
                apply_step(&inner, &live, &mut st, step);
                settle_background(&inner, &live, &mut st, TurnItemStatus::Interrupted);
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
                    cancel_pending(&inner, &live, &mut st, CancelReason::Agent);
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
            PersistOp::ClearNativeThreadId => repo.clear_native_thread_id(&id).await,
            PersistOp::Closed => repo.set_closed(&id).await,
            PersistOp::Opened => repo.set_open(&id).await,
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

/// Revoke what the session's spawn minted, off the session lock.
fn retire(inner: &Arc<Inner>, session: &RuntimeSession) {
    let host = Arc::clone(&inner.host);
    let session = session.clone();
    tokio::spawn(async move { host.retire(&session).await });
}

fn scope_request_id(st: &State, native: &str) -> String {
    st.proc
        .as_ref()
        .expect("driver event has a live process")
        .request_ids
        .scoped(native)
}
