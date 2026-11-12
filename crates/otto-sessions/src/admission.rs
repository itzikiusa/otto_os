//! Machine-aware admission for engine-owned background agents.
//!
//! Before 2026-11-12 the only limits were per-feature COUNT caps (8 reviewer
//! slots, 2 workflow runs) and an advisory live-session cap that only ever
//! suspended idle sessions. Two code reviews (6 reviewers each) and a nightly
//! docs job then started together on an 8-core / 16 GB Mac: swap went from 4
//! to 22 GB, every database query took seconds, and both reviews failed.
//!
//! This gate sits in front of every PTY spawn of an UNATTENDED background
//! agent ([`ADMISSION_GATED_SOURCES`]) and lets one start only when
//! - fewer than [`AdmissionPolicy::max_live`] of them are live (a hard
//!   ceiling — `agent_background_max_live`, default `max(2, cores × 3/4)`), and
//! - the 1-minute load is at most `cores × agent_admission_max_load_per_core`
//!   (default 1.5) and macOS memory pressure is below "warn".
//!
//! Waiters queue first-in first-out. Nothing waits forever: a load / memory
//! wait gives way after `agent_admission_max_wait_secs` (default 300 s) and
//! a ceiling wait after [`CEILING_WAIT_FACTOR`] times that, both with a
//! logged warning — a stuck engine holding its sessions live must not freeze
//! every review behind it. Sessions the user starts (Agents page, terminals,
//! connections) and the interactive assists a person is watching (canvas /
//! design / db assists, PR and commit drafts, channel replies, discovery
//! chats) never wait.
//!
//! The gate is armed only on a manager with a settings store (the daemon);
//! bare test harnesses spawn as before. `agent_admission_enabled: false`
//! turns it off at runtime, releasing anything queued.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::api::CreateSessionReq;
use otto_core::domain::{Session, SessionKind};
use otto_core::Id;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{watch, Notify};

use crate::machine_load::{MachineMonitor, MachineSample, MemoryPressure};

/// Setting (bool, default `true`): the admission gate on/off.
pub const ADMISSION_ENABLED_SETTING: &str = "agent_admission_enabled";
/// Setting (integer): hard ceiling on live gated background agents. Absent =
/// [`default_max_live`]; `0` = no ceiling (load / memory checks still apply).
pub const BACKGROUND_MAX_LIVE_SETTING: &str = "agent_background_max_live";
/// Setting (number, default [`DEFAULT_MAX_LOAD_PER_CORE`]): a background
/// agent waits while the 1-minute load average exceeds `cores × this`.
pub const MAX_LOAD_PER_CORE_SETTING: &str = "agent_admission_max_load_per_core";
/// Setting (integer seconds, default [`DEFAULT_MAX_WAIT`]): the longest a
/// background agent waits for load / memory before starting anyway.
pub const MAX_WAIT_SETTING: &str = "agent_admission_max_wait_secs";
/// Setting (bool, default `true`): run gated background agents at a lower
/// CPU priority ([`BACKGROUND_NICE`]).
pub const LOW_PRIORITY_SETTING: &str = "agent_background_low_priority";

/// 1.5 × cores: an 8-core Mac stays responsive up to a load of ~12; past it
/// every extra CLI (and the builds it starts) only adds queueing.
pub const DEFAULT_MAX_LOAD_PER_CORE: f64 = 1.5;
/// Five minutes: long enough for a build storm to pass, short enough that a
/// reviewer or workflow step still finishes inside its own budget.
pub const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(300);
/// The live ceiling is the real back-pressure (the load average lags a
/// minute behind), so it holds four times longer than a load wait before it
/// gives way — 20 minutes at the default, about one reviewer's run.
pub const CEILING_WAIT_FACTOR: u32 = 4;
/// Minimum gap between two admissions out of the QUEUE: the load average
/// cannot see a just-started agent yet, so draining a queue all at once the
/// moment the load dips would recreate the storm. Arrivals on a calm machine
/// with nobody queued start at once.
pub const ADMIT_SPACING: Duration = Duration::from_secs(5);
/// How often a waiter re-checks when nothing woke it (machine samples are
/// cached this long anyway).
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// At most one "machine busy" notice per this window.
pub const BUSY_NOTICE_EVERY: Duration = Duration::from_secs(10 * 60);
/// The live-session cap staying exceeded this long raises one notice.
pub const CAP_NOTICE_AFTER: Duration = Duration::from_secs(5 * 60);
/// The idle sweep runs every minute; a gap longer than this between two
/// "cap exceeded" observations starts a new episode.
const CAP_EPISODE_GAP: Duration = Duration::from_secs(150);

/// Niceness for gated background agents (inherited by everything they
/// spawn). A moderate `nice`, not macOS background QoS: `PRIO_DARWIN_BG`
/// also throttles disk I/O and confines the process to the efficiency cores,
/// which turns a reviewer's whole-repo read or a workflow's `cargo test`
/// into a crawl even on an idle machine. Nice 10 costs nothing when the
/// machine is idle and yields to the user's own session and the UI under
/// contention — which is the goal.
pub const BACKGROUND_NICE: i32 = 10;

/// `meta` key a pre-acquired permit rides in on ([`AdmissionPermit::attach`]).
/// Stripped by `create` before the row is written — never persisted.
pub const PERMIT_META_KEY: &str = "admission_permit";

/// The `meta.source` values the gate holds back: engine-owned agents that run
/// UNATTENDED, where nobody is watching the session start. A subset of
/// `BACKGROUND_SESSION_SOURCES` plus `personal_agent` (personal-agent runs
/// are deliberately outside that list — they show in the sidebar — but a
/// scheduled run is unattended work all the same).
///
/// Not `swarm`: its roles are started together and talk to each other, so
/// a ceiling that admits four of six would stall the four on the two it
/// holds back until the ceiling's long wait ran out.
pub const ADMISSION_GATED_SOURCES: [&str; 14] = [
    "review",
    "review_summarizer",
    "skilleval",
    "skillreview",
    "product-analysis",
    "workflow",
    "vault-docs",
    "vault-docs-review",
    "insights",
    "run_with_otto",
    "goal_loop",
    "scheduled_task",
    "finding",
    "personal_agent",
];

/// Whether `source` is one the gate holds back.
pub fn is_gated_source(source: &str) -> bool {
    ADMISSION_GATED_SOURCES.contains(&source)
}

/// The gated source of an agent about to be created, if any.
pub fn gated_source(kind: SessionKind, meta: Option<&Value>) -> Option<&str> {
    if kind != SessionKind::Agent {
        return None;
    }
    meta?
        .get("source")
        .and_then(Value::as_str)
        .filter(|s| is_gated_source(s))
}

/// [`gated_source`] for an existing session.
pub fn is_gated_session(session: &Session) -> bool {
    gated_source(session.kind, Some(&session.meta)).is_some()
}

/// Default live ceiling: three quarters of the cores, at least 2 — 6 on an
/// 8-core Mac, so the six lenses of ONE review run together and a second
/// review waits its turn. The load and memory checks still hold any of them
/// back when the machine is busy.
pub fn default_max_live(cores: usize) -> usize {
    (cores * 3 / 4).max(2)
}

/// `agent_background_low_priority` (default on).
pub fn low_priority_enabled(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool).unwrap_or(true)
}

/// The gate's thresholds, read from settings on every check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdmissionPolicy {
    pub enabled: bool,
    /// `None` = no ceiling.
    pub max_live: Option<usize>,
    pub max_load_per_core: f64,
    pub max_wait: Duration,
}

impl AdmissionPolicy {
    /// The defaults for a machine with `cores` logical CPUs.
    pub fn defaults(cores: usize) -> Self {
        Self::from_settings(cores, None, None, None, None)
    }

    /// Off: no settings store (bare harness) or switched off.
    pub fn disabled(cores: usize) -> Self {
        Self {
            enabled: false,
            ..Self::defaults(cores)
        }
    }

    /// Parse the four settings; anything missing or malformed takes its
    /// default (a typo never disables the gate or removes the ceiling).
    pub fn from_settings(
        cores: usize,
        enabled: Option<&Value>,
        max_live: Option<&Value>,
        max_load_per_core: Option<&Value>,
        max_wait_secs: Option<&Value>,
    ) -> Self {
        let max_live = match max_live.and_then(Value::as_u64) {
            Some(0) => None,
            Some(n) => Some(n as usize),
            None => Some(default_max_live(cores)),
        };
        Self {
            enabled: enabled.and_then(Value::as_bool).unwrap_or(true),
            max_live,
            max_load_per_core: max_load_per_core
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(DEFAULT_MAX_LOAD_PER_CORE),
            max_wait: max_wait_secs
                .and_then(Value::as_u64)
                .map(Duration::from_secs)
                .unwrap_or(DEFAULT_MAX_WAIT),
        }
    }
}

/// Why a background agent is not starting yet.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitReason {
    /// Behind earlier queued agents (first in, first out).
    Queue,
    /// Just after another queued agent started ([`ADMIT_SPACING`]).
    Spacing,
    /// The live ceiling is reached.
    LiveCeiling { live: usize, max: usize },
    /// The 1-minute load is above the threshold.
    Load { load1: f64, limit: f64 },
    /// macOS memory pressure is warn or critical.
    MemoryPressure { level: MemoryPressure },
}

impl WaitReason {
    /// One line for logs and notices.
    pub fn describe(&self) -> String {
        match self {
            Self::Queue => "waiting behind other background agents".into(),
            Self::Spacing => "starting queued agents a few seconds apart".into(),
            Self::LiveCeiling { live, max } => {
                format!("{live} background agents already running (limit {max})")
            }
            Self::Load { load1, limit } => {
                format!("load average {load1:.1} is above {limit:.1}")
            }
            Self::MemoryPressure { level } => match level {
                MemoryPressure::Critical => "memory pressure is critical".into(),
                _ => "memory pressure is high".into(),
            },
        }
    }
}

/// [`decide`]'s answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Admit,
    /// Still blocked, but waited the maximum: start anyway (logged).
    AdmitOverdue(WaitReason),
    Wait(WaitReason),
}

/// The pure decision for the agent at the head of the queue: `live` gated
/// background agents running (or admitted and starting), `waited` since it
/// queued. The ceiling is checked first — it is instant, while the load
/// average trails reality by about a minute.
pub fn decide(
    sample: &MachineSample,
    policy: &AdmissionPolicy,
    live: usize,
    waited: Duration,
) -> Verdict {
    if let Some(max) = policy.max_live {
        if live >= max {
            let reason = WaitReason::LiveCeiling { live, max };
            return if waited >= policy.max_wait * CEILING_WAIT_FACTOR {
                Verdict::AdmitOverdue(reason)
            } else {
                Verdict::Wait(reason)
            };
        }
    }
    let limit = sample.cores.max(1) as f64 * policy.max_load_per_core;
    let reason = if sample.pressure.is_elevated() {
        Some(WaitReason::MemoryPressure {
            level: sample.pressure,
        })
    } else {
        sample
            .load1
            .filter(|l| *l > limit)
            .map(|load1| WaitReason::Load { load1, limit })
    };
    match reason {
        None => Verdict::Admit,
        Some(r) if waited >= policy.max_wait => Verdict::AdmitOverdue(r),
        Some(r) => Verdict::Wait(r),
    }
}

/// Outcome of one [`AdmissionGate::poll`].
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    Admitted(Admitted),
    Wait(WaitReason),
    /// The ticket is no longer queued (cancelled).
    Gone,
}

/// A successful admission.
#[derive(Debug, Clone, PartialEq)]
pub struct Admitted {
    /// Reservation key ([`AdmissionPermit::attach`]).
    pub key: String,
    pub waited: Duration,
    /// It had to wait at least once.
    pub queued: bool,
    /// Started past the maximum wait, still blocked by this.
    pub overdue: Option<WaitReason>,
}

struct Waiter {
    ticket: u64,
    source: String,
    since: Instant,
    reason: Option<WaitReason>,
    waited_once: bool,
}

struct Reserved {
    key: String,
}

#[derive(Default)]
struct CapWatch {
    since: Option<Instant>,
    last_seen: Option<Instant>,
    noticed: bool,
}

#[derive(Default)]
struct GateState {
    next_ticket: u64,
    queue: VecDeque<Waiter>,
    /// Admitted, process not yet registered (or a pre-acquired permit not
    /// yet used). Counted toward the ceiling.
    reserved: HashMap<u64, Reserved>,
    last_admit: Option<Instant>,
    busy_notice: Option<Instant>,
    cap: CapWatch,
    /// A low-disk notice is out (cleared by the recovery notice).
    low_disk_noticed: bool,
}

/// A spawned agent process the gate counts while it runs.
struct Running {
    gated: bool,
    exit: watch::Receiver<Option<i32>>,
}

impl Running {
    /// Still running: no exit code yet and the PTY waiter still there.
    fn alive(&self) -> bool {
        self.exit.has_changed().is_ok() && self.exit.borrow().is_none()
    }
}

/// The daemon-wide admission state: machine monitor, FIFO queue, the
/// reservations of admitted-but-starting agents and the running agent
/// processes (by their exit watch — a suspend, kill or exit drops them out
/// without any teardown path having to remember the gate).
pub struct AdmissionGate {
    monitor: MachineMonitor,
    state: Mutex<GateState>,
    changed: Notify,
    running: DashMap<Id, Running>,
}

impl AdmissionGate {
    pub fn new(monitor: MachineMonitor) -> Self {
        Self {
            monitor,
            state: Mutex::new(GateState::default()),
            changed: Notify::new(),
            running: DashMap::new(),
        }
    }

    /// The OS-backed gate the daemon uses.
    pub fn os() -> Self {
        Self::new(MachineMonitor::os())
    }

    pub fn monitor(&self) -> &MachineMonitor {
        &self.monitor
    }

    /// Resolves when a reservation is released, a waiter leaves the queue or
    /// one is admitted. Create it BEFORE polling so a change in between is
    /// not missed (`notify_waiters` reaches every future already created).
    pub fn changed(&self) -> tokio::sync::futures::Notified<'_> {
        self.changed.notified()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Join the queue; returns the ticket to [`Self::poll`] with.
    pub fn enqueue(&self, source: &str, now: Instant) -> u64 {
        let mut st = self.state();
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        st.queue.push_back(Waiter {
            ticket,
            source: source.to_string(),
            since: now,
            reason: None,
            waited_once: false,
        });
        ticket
    }

    /// Leave the queue without being admitted (the waiter was dropped).
    pub fn cancel(&self, ticket: u64) {
        let removed = {
            let mut st = self.state();
            let before = st.queue.len();
            st.queue.retain(|w| w.ticket != ticket);
            st.queue.len() != before
        };
        if removed {
            self.changed.notify_waiters();
        }
    }

    /// Give back an admitted reservation (the permit was dropped).
    pub fn release(&self, ticket: u64) {
        if self.state().reserved.remove(&ticket).is_some() {
            self.changed.notify_waiters();
        }
    }

    /// Whether `key` names a live reservation (a pre-acquired permit).
    pub fn holds(&self, key: &str) -> bool {
        self.state().reserved.values().any(|r| r.key == key)
    }

    /// Take over the reservation `key` names (a pre-acquired permit handed to
    /// `create`): it moves to a fresh ticket the caller now owns, so the
    /// original permit's drop no longer releases it — its holder may keep it
    /// for as long as it likes without the place being counted twice.
    pub fn claim(&self, key: &str) -> Option<(u64, String)> {
        let mut st = self.state();
        let old = st
            .reserved
            .iter()
            .find(|(_, r)| r.key == key)
            .map(|(t, _)| *t)?;
        st.reserved.remove(&old);
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        let key = format!("{}:{ticket}", uuid::Uuid::new_v4());
        st.reserved.insert(ticket, Reserved { key: key.clone() });
        Some((ticket, key))
    }

    /// Count `id`'s freshly spawned process until it exits. `gated` marks it
    /// as one of the ceiling's background agents.
    pub fn register(&self, id: &Id, gated: bool, exit: watch::Receiver<Option<i32>>) {
        self.running.insert(id.clone(), Running { gated, exit });
    }

    /// Live (agents, gated background agents); prunes the exited ones.
    pub fn live_counts(&self) -> (usize, usize) {
        self.running.retain(|_, r| r.alive());
        let gated = self.running.iter().filter(|r| r.gated).count();
        (self.running.len(), gated)
    }

    /// Live agent processes other than `id` (a respawn replaces its own).
    pub fn live_agents_except(&self, id: &Id) -> usize {
        self.live_counts();
        self.running.iter().filter(|r| r.key() != id).count()
    }

    /// One admission attempt for `ticket` at `now`. Only the head of the
    /// queue can be admitted; on success it leaves the queue and holds a
    /// reservation until its permit is dropped.
    pub fn poll(
        &self,
        ticket: u64,
        sample: &MachineSample,
        policy: &AdmissionPolicy,
        now: Instant,
    ) -> Poll {
        let (_, gated_live) = self.live_counts();
        let mut st = self.state();
        let Some(pos) = st.queue.iter().position(|w| w.ticket == ticket) else {
            return Poll::Gone;
        };
        if pos > 0 {
            let w = &mut st.queue[pos];
            w.reason = Some(WaitReason::Queue);
            w.waited_once = true;
            return Poll::Wait(WaitReason::Queue);
        }
        let live = gated_live + st.reserved.len();
        let head = &st.queue[0];
        let waited = now.saturating_duration_since(head.since);
        let spaced = head.waited_once
            && st
                .last_admit
                .is_some_and(|t| now.saturating_duration_since(t) < ADMIT_SPACING);
        let verdict = match decide(sample, policy, live, waited) {
            Verdict::Admit if spaced => Verdict::Wait(WaitReason::Spacing),
            v => v,
        };
        let overdue = match verdict {
            Verdict::Wait(reason) => {
                let head = &mut st.queue[0];
                head.reason = Some(reason);
                head.waited_once = true;
                return Poll::Wait(reason);
            }
            Verdict::Admit => None,
            Verdict::AdmitOverdue(reason) => Some(reason),
        };
        let Some(head) = st.queue.pop_front() else {
            return Poll::Gone;
        };
        let key = format!("{}:{ticket}", uuid::Uuid::new_v4());
        st.reserved.insert(ticket, Reserved { key: key.clone() });
        st.last_admit = Some(now);
        drop(st);
        // The next head may be admissible right away.
        self.changed.notify_waiters();
        Poll::Admitted(Admitted {
            key,
            waited,
            queued: head.waited_once,
            overdue,
        })
    }

    /// True at most once per [`BUSY_NOTICE_EVERY`]: raise the notice now.
    pub fn take_busy_notice(&self, now: Instant) -> bool {
        let mut st = self.state();
        let due = st
            .busy_notice
            .is_none_or(|t| now.saturating_duration_since(t) >= BUSY_NOTICE_EVERY);
        if due {
            st.busy_notice = Some(now);
        }
        due
    }

    /// The live-session cap was seen exceeded at `now`. True exactly once
    /// per episode, when it has stayed exceeded for [`CAP_NOTICE_AFTER`].
    pub fn observe_cap_exceeded(&self, now: Instant) -> bool {
        let mut st = self.state();
        let cap = &mut st.cap;
        let new_episode = cap
            .last_seen
            .is_none_or(|t| now.saturating_duration_since(t) > CAP_EPISODE_GAP);
        if new_episode {
            cap.since = Some(now);
            cap.noticed = false;
        }
        cap.last_seen = Some(now);
        let since = cap.since.unwrap_or(now);
        if !cap.noticed && now.saturating_duration_since(since) >= CAP_NOTICE_AFTER {
            cap.noticed = true;
            return true;
        }
        false
    }

    /// The data volume was seen `low` (below `machine_low_disk_gb`) or not.
    /// `Some(true)` when an episode starts (raise the warning), `Some(false)`
    /// when one that was announced ends (post the recovery), else `None`.
    pub fn observe_low_disk(&self, low: bool) -> Option<bool> {
        let mut st = self.state();
        if st.low_disk_noticed == low {
            return None;
        }
        st.low_disk_noticed = low;
        Some(low)
    }

    /// The snapshot the status chip / settings page show.
    pub fn status(
        &self,
        sample: &MachineSample,
        policy: &AdmissionPolicy,
        now: Instant,
    ) -> MachineStatus {
        let (agents_live, background_live) = self.live_counts();
        let st = self.state();
        let queued: Vec<QueuedAgent> = st
            .queue
            .iter()
            .map(|w| QueuedAgent {
                source: w.source.clone(),
                waiting_secs: now.saturating_duration_since(w.since).as_secs(),
                reason: w.reason.map(|r| r.describe()),
            })
            .collect();
        let live = background_live + st.reserved.len();
        let busy_reason = match decide(sample, policy, live, Duration::ZERO) {
            Verdict::Wait(r) | Verdict::AdmitOverdue(r) => Some(r.describe()),
            Verdict::Admit => None,
        };
        MachineStatus {
            load1: sample.load1,
            cores: sample.cores,
            load_per_core: sample.load_per_core(),
            memory_pressure: sample.pressure,
            swap_used_bytes: sample.swap_used_bytes,
            admission_enabled: policy.enabled,
            max_load_per_core: policy.max_load_per_core,
            max_wait_secs: policy.max_wait.as_secs(),
            background_live: live,
            background_max_live: policy.max_live,
            agents_live,
            busy: policy.enabled && busy_reason.is_some(),
            busy_reason: busy_reason.filter(|_| policy.enabled),
            queued,
            disk_free_bytes: None,
            low_disk_gb: crate::machine_load::DEFAULT_LOW_DISK_GB,
            low_disk: false,
        }
    }
}

/// Machine load + the gate's queue, for the UI (status chip, Settings).
#[derive(Debug, Clone, Serialize)]
pub struct MachineStatus {
    /// 1-minute load average (`null` when unreadable).
    pub load1: Option<f64>,
    pub cores: usize,
    pub load_per_core: f64,
    /// `unknown` | `normal` | `warn` | `critical`.
    pub memory_pressure: MemoryPressure,
    pub swap_used_bytes: Option<u64>,
    pub admission_enabled: bool,
    pub max_load_per_core: f64,
    pub max_wait_secs: u64,
    /// Gated background agents running or starting.
    pub background_live: usize,
    /// The ceiling (`null` = none).
    pub background_max_live: Option<usize>,
    /// Every agent process Otto started that is still running.
    pub agents_live: usize,
    /// A background agent arriving now would queue.
    pub busy: bool,
    pub busy_reason: Option<String>,
    /// Waiting agents, oldest first.
    pub queued: Vec<QueuedAgent>,
    /// Free bytes on the volume holding Otto's data dir (`null` = unknown).
    pub disk_free_bytes: Option<u64>,
    /// The low-disk floor in GiB (`machine_low_disk_gb`; 0 = off).
    pub low_disk_gb: f64,
    /// Free space is below the floor. Shown, never acted on.
    pub low_disk: bool,
}

impl MachineStatus {
    /// Add the data volume's free space and the low-disk verdict.
    pub fn with_disk(mut self, free_bytes: Option<u64>, low_disk_gb: f64) -> Self {
        self.disk_free_bytes = free_bytes;
        self.low_disk_gb = low_disk_gb;
        self.low_disk = crate::machine_load::is_low_disk(free_bytes, low_disk_gb);
        self
    }
}

/// One waiting background agent.
#[derive(Debug, Clone, Serialize)]
pub struct QueuedAgent {
    /// Its `meta.source` (`review`, `workflow`, …).
    pub source: String,
    pub waiting_secs: u64,
    pub reason: Option<String>,
}

/// Removes a waiter from the queue if its future is dropped before it was
/// admitted (a cancelled review, a dropped create).
pub(crate) struct QueuedTicket {
    gate: Arc<AdmissionGate>,
    ticket: u64,
    armed: bool,
}

impl QueuedTicket {
    pub(crate) fn new(gate: &Arc<AdmissionGate>, ticket: u64) -> Self {
        Self {
            gate: Arc::clone(gate),
            ticket,
            armed: true,
        }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for QueuedTicket {
    fn drop(&mut self) {
        if self.armed {
            self.gate.cancel(self.ticket);
        }
    }
}

/// A place under the background-agent ceiling. `create` holds one until the
/// session's process has spawned — from then on the running process itself
/// is counted. A pre-acquired one (`SessionManager::admit_background`) is
/// taken over by the `create` it is attached to; dropped before that, it
/// gives the place back.
#[must_use = "dropping the permit gives the place back at once"]
pub struct AdmissionPermit {
    slot: Option<(Arc<AdmissionGate>, u64, String)>,
}

impl AdmissionPermit {
    /// No reservation: the gate is off, or this agent is not gated.
    pub fn none() -> Self {
        Self { slot: None }
    }

    pub(crate) fn reserved(gate: Arc<AdmissionGate>, ticket: u64, key: String) -> Self {
        Self {
            slot: Some((gate, ticket, key)),
        }
    }

    /// Whether this permit holds a place.
    pub fn is_reserved(&self) -> bool {
        self.slot.is_some()
    }

    /// Mark `req` as already admitted, so `SessionManager::create` does not
    /// queue it a second time (the key is stripped before the row is saved).
    pub fn attach(&self, req: &mut CreateSessionReq) {
        if self.slot.is_some() {
            self.stamp(req.meta.get_or_insert_with(|| serde_json::json!({})));
        }
    }

    /// [`Self::attach`] for a caller that only holds the session `meta`
    /// object (it must end up as the create request's `meta`).
    pub fn stamp(&self, meta: &mut Value) {
        let (Some((_, _, key)), Some(obj)) = (&self.slot, meta.as_object_mut()) else {
            return;
        };
        obj.insert(PERMIT_META_KEY.into(), Value::String(key.clone()));
    }
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        if let Some((gate, ticket, _)) = self.slot.take() {
            gate.release(ticket);
        }
    }
}

/// Lower `pid`'s CPU priority to [`BACKGROUND_NICE`]; everything it starts
/// afterwards inherits it. Raising niceness of our own child needs no
/// privilege. Off macOS a no-op (the app ships for macOS only).
#[cfg(target_os = "macos")]
pub fn lower_priority(pid: u32) -> std::io::Result<()> {
    let who = libc::id_t::from(pid);
    // SAFETY: plain syscall on a pid we spawned; no memory is passed.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, who, BACKGROUND_NICE) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Off macOS a no-op (the app ships for macOS only).
#[cfg(not(target_os = "macos"))]
pub fn lower_priority(_pid: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample(load1: f64, cores: usize, pressure: MemoryPressure) -> MachineSample {
        MachineSample {
            load1: Some(load1),
            cores,
            pressure,
            swap_used_bytes: None,
            taken: Instant::now(),
        }
    }

    fn calm() -> MachineSample {
        sample(1.0, 8, MemoryPressure::Normal)
    }

    fn policy() -> AdmissionPolicy {
        AdmissionPolicy::defaults(8)
    }

    fn gate() -> Arc<AdmissionGate> {
        struct Never;
        impl crate::machine_load::Sampler for Never {
            fn sample(&self) -> MachineSample {
                sample(0.0, 8, MemoryPressure::Normal)
            }
        }
        Arc::new(AdmissionGate::new(MachineMonitor::new(Arc::new(Never))))
    }

    /// A process that is still running, and its exit sender.
    fn running() -> (watch::Sender<Option<i32>>, watch::Receiver<Option<i32>>) {
        watch::channel(None)
    }

    fn admitted(p: Poll) -> Admitted {
        match p {
            Poll::Admitted(a) => a,
            other => panic!("expected admission, got {other:?}"),
        }
    }

    #[test]
    fn defaults_fit_an_8_core_mac() {
        let p = policy();
        assert!(p.enabled);
        assert_eq!(p.max_live, Some(6));
        assert_eq!(p.max_load_per_core, 1.5);
        assert_eq!(p.max_wait, Duration::from_secs(300));
        assert_eq!(default_max_live(2), 2);
        assert_eq!(default_max_live(16), 12);
    }

    #[test]
    fn settings_parse_and_bad_values_fall_back() {
        let p = AdmissionPolicy::from_settings(
            8,
            Some(&json!(false)),
            Some(&json!(0)),
            Some(&json!(2.5)),
            Some(&json!(60)),
        );
        assert!(!p.enabled);
        assert_eq!(p.max_live, None, "0 = no ceiling");
        assert_eq!(p.max_load_per_core, 2.5);
        assert_eq!(p.max_wait, Duration::from_secs(60));
        let p = AdmissionPolicy::from_settings(
            8,
            Some(&json!("yes")),
            Some(&json!("many")),
            Some(&json!(-1)),
            Some(&json!("soon")),
        );
        assert_eq!(p, AdmissionPolicy::defaults(8));
        assert!(!AdmissionPolicy::disabled(8).enabled);
        assert!(low_priority_enabled(None));
        assert!(!low_priority_enabled(Some(&json!(false))));
    }

    #[test]
    fn only_unattended_background_agents_are_gated() {
        let meta = |s: &str| json!({ "source": s });
        for s in [
            "review",
            "review_summarizer",
            "workflow",
            "scheduled_task",
            "goal_loop",
        ] {
            assert_eq!(
                gated_source(SessionKind::Agent, Some(&meta(s))),
                Some(s),
                "{s}"
            );
        }
        for s in [
            "channel",
            "swarm",
            "canvas_assist",
            "db_assist",
            "pr-draft",
            "discovery_chat",
        ] {
            assert_eq!(
                gated_source(SessionKind::Agent, Some(&meta(s))),
                None,
                "{s}"
            );
        }
        // The user's own sessions and connections never wait.
        assert_eq!(gated_source(SessionKind::Agent, Some(&json!({}))), None);
        assert_eq!(gated_source(SessionKind::Agent, None), None);
        assert_eq!(
            gated_source(SessionKind::Connection, Some(&meta("review"))),
            None
        );
        // Every gated source except personal_agent is a background source.
        for s in ADMISSION_GATED_SOURCES {
            assert!(
                s == "personal_agent" || otto_core::domain::BACKGROUND_SESSION_SOURCES.contains(&s),
                "{s} is not a background source"
            );
        }
    }

    #[test]
    fn a_calm_machine_admits_at_once() {
        assert_eq!(
            decide(&calm(), &policy(), 0, Duration::ZERO),
            Verdict::Admit
        );
        assert_eq!(
            decide(&calm(), &policy(), 3, Duration::ZERO),
            Verdict::Admit
        );
        // Exactly at the threshold is still fine (strictly above waits).
        let at = sample(12.0, 8, MemoryPressure::Normal);
        assert_eq!(decide(&at, &policy(), 0, Duration::ZERO), Verdict::Admit);
        // Unknown pressure / load never blocks.
        let unknown = MachineSample {
            load1: None,
            ..sample(0.0, 8, MemoryPressure::Unknown)
        };
        assert_eq!(
            decide(&unknown, &policy(), 0, Duration::ZERO),
            Verdict::Admit
        );
    }

    #[test]
    fn high_load_or_memory_pressure_waits_then_gives_way() {
        let loaded = sample(13.0, 8, MemoryPressure::Normal);
        assert_eq!(
            decide(&loaded, &policy(), 0, Duration::from_secs(10)),
            Verdict::Wait(WaitReason::Load {
                load1: 13.0,
                limit: 12.0
            })
        );
        assert!(matches!(
            decide(&loaded, &policy(), 0, Duration::from_secs(300)),
            Verdict::AdmitOverdue(WaitReason::Load { .. })
        ));
        for level in [MemoryPressure::Warn, MemoryPressure::Critical] {
            let tight = sample(0.5, 8, level);
            assert_eq!(
                decide(&tight, &policy(), 0, Duration::ZERO),
                Verdict::Wait(WaitReason::MemoryPressure { level })
            );
        }
    }

    #[test]
    fn the_live_ceiling_holds_longer_than_a_load_wait() {
        let max_wait = policy().max_wait;
        assert_eq!(
            decide(&calm(), &policy(), 6, max_wait),
            Verdict::Wait(WaitReason::LiveCeiling { live: 6, max: 6 })
        );
        assert_eq!(
            decide(&calm(), &policy(), 6, max_wait * CEILING_WAIT_FACTOR),
            Verdict::AdmitOverdue(WaitReason::LiveCeiling { live: 6, max: 6 })
        );
        // No ceiling configured: only load / memory matter.
        let open = AdmissionPolicy {
            max_live: None,
            ..policy()
        };
        assert_eq!(decide(&calm(), &open, 40, Duration::ZERO), Verdict::Admit);
    }

    #[test]
    fn the_queue_is_first_in_first_out() {
        let g = gate();
        let t0 = Instant::now();
        let busy = sample(30.0, 8, MemoryPressure::Normal);
        let a = g.enqueue("review", t0);
        let b = g.enqueue("workflow", t0);
        assert_eq!(
            g.poll(b, &calm(), &policy(), t0),
            Poll::Wait(WaitReason::Queue)
        );
        assert!(matches!(
            g.poll(a, &busy, &policy(), t0),
            Poll::Wait(WaitReason::Load { .. })
        ));
        // Calm again: the head goes first, and only the head.
        let first = admitted(g.poll(a, &calm(), &policy(), t0 + Duration::from_secs(30)));
        assert!(first.queued);
        assert_eq!(first.overdue, None);
        // b is now the head, but the queue drains a few seconds apart.
        let soon = t0 + Duration::from_secs(31);
        assert_eq!(
            g.poll(b, &calm(), &policy(), soon),
            Poll::Wait(WaitReason::Spacing)
        );
        admitted(g.poll(b, &calm(), &policy(), soon + ADMIT_SPACING));
        assert_eq!(g.poll(a, &calm(), &policy(), soon), Poll::Gone);
    }

    #[test]
    fn reservations_and_running_agents_count_toward_the_ceiling() {
        let g = gate();
        let p = AdmissionPolicy {
            max_live: Some(2),
            ..policy()
        };
        let t0 = Instant::now();
        // One gated agent running, one admitted and still starting.
        let (exit_tx, exit_rx) = running();
        g.register(&"s1".to_string(), true, exit_rx);
        // A foreground agent runs too, but the ceiling ignores it.
        let (_fg_tx, fg_rx) = running();
        g.register(&"fg".to_string(), false, fg_rx);
        let a = g.enqueue("review", t0);
        let first = admitted(g.poll(a, &calm(), &p, t0));
        let permit = AdmissionPermit::reserved(Arc::clone(&g), a, first.key.clone());
        assert!(g.holds(&first.key));
        let b = g.enqueue("review", t0);
        assert_eq!(
            g.poll(b, &calm(), &p, t0),
            Poll::Wait(WaitReason::LiveCeiling { live: 2, max: 2 })
        );
        assert_eq!(g.live_counts(), (2, 1));
        // The running one exits: still at the ceiling with the reservation.
        exit_tx.send(Some(0)).unwrap();
        // The starting one's permit is released (its process registers).
        drop(permit);
        assert!(!g.holds(&first.key));
        assert_eq!(g.live_counts(), (1, 0));
        admitted(g.poll(b, &calm(), &p, t0 + ADMIT_SPACING));
    }

    #[test]
    fn a_dropped_exit_sender_counts_as_exited() {
        let g = gate();
        let (tx, rx) = running();
        g.register(&"s".to_string(), true, rx);
        assert_eq!(g.live_counts(), (1, 1));
        drop(tx);
        assert_eq!(g.live_counts(), (0, 0));
        // A respawn replaces the old entry rather than double-counting it.
        let (_tx1, rx1) = running();
        let (_tx2, rx2) = running();
        g.register(&"s".to_string(), true, rx1);
        g.register(&"s".to_string(), true, rx2);
        assert_eq!(g.live_counts(), (1, 1));
        assert_eq!(g.live_agents_except(&"s".to_string()), 0);
        assert_eq!(g.live_agents_except(&"other".to_string()), 1);
    }

    #[test]
    fn a_cancelled_waiter_leaves_the_queue() {
        let g = gate();
        let t0 = Instant::now();
        let a = g.enqueue("review", t0);
        let b = g.enqueue("review", t0);
        {
            let _guard = QueuedTicket::new(&g, a);
        }
        assert_eq!(g.poll(a, &calm(), &policy(), t0), Poll::Gone);
        admitted(g.poll(b, &calm(), &policy(), t0));
        // A disarmed guard (admitted) does not touch the queue.
        let c = g.enqueue("review", t0);
        let mut guard = QueuedTicket::new(&g, c);
        guard.disarm();
        drop(guard);
        assert!(matches!(
            g.poll(c, &calm(), &policy(), t0 + ADMIT_SPACING),
            Poll::Admitted(_)
        ));
    }

    #[test]
    fn a_waiter_overdue_on_load_starts_anyway() {
        let g = gate();
        let t0 = Instant::now();
        let busy = sample(30.0, 8, MemoryPressure::Normal);
        let a = g.enqueue("review", t0);
        assert!(matches!(g.poll(a, &busy, &policy(), t0), Poll::Wait(_)));
        let late = admitted(g.poll(a, &busy, &policy(), t0 + DEFAULT_MAX_WAIT));
        assert!(matches!(late.overdue, Some(WaitReason::Load { .. })));
        assert_eq!(late.waited, DEFAULT_MAX_WAIT);
    }

    #[test]
    fn attach_marks_the_request_and_a_none_permit_does_nothing() {
        let g = gate();
        let t0 = Instant::now();
        let a = g.enqueue("review_summarizer", t0);
        let ad = admitted(g.poll(a, &calm(), &policy(), t0));
        let permit = AdmissionPermit::reserved(Arc::clone(&g), a, ad.key.clone());
        let mut req = CreateSessionReq {
            kind: SessionKind::Agent,
            provider: Some("claude".into()),
            title: None,
            cwd: None,
            connection_id: None,
            meta: None,
            model: None,
        };
        permit.attach(&mut req);
        assert_eq!(
            req.meta.as_ref().and_then(|m| m.get(PERMIT_META_KEY)),
            Some(&json!(ad.key))
        );
        let mut plain = CreateSessionReq { meta: None, ..req };
        AdmissionPermit::none().attach(&mut plain);
        assert_eq!(plain.meta, None);
        assert!(permit.is_reserved());
        assert!(!AdmissionPermit::none().is_reserved());
    }

    #[test]
    fn claiming_a_reservation_moves_it_to_the_claimer() {
        let g = gate();
        let t0 = Instant::now();
        let a = g.enqueue("review_summarizer", t0);
        let ad = admitted(g.poll(a, &calm(), &policy(), t0));
        let caller = AdmissionPermit::reserved(Arc::clone(&g), a, ad.key.clone());
        let mut meta = json!({ "source": "review_summarizer" });
        caller.stamp(&mut meta);
        assert_eq!(meta[PERMIT_META_KEY], json!(ad.key));
        let (ticket, key) = g.claim(&ad.key).expect("claimable");
        assert!(g.claim(&ad.key).is_none(), "claimed once");
        assert!(g.holds(&key));
        // The caller's permit no longer releases anything…
        drop(caller);
        assert!(g.holds(&key));
        // …the claimer's does.
        drop(AdmissionPermit::reserved(
            Arc::clone(&g),
            ticket,
            key.clone(),
        ));
        assert!(!g.holds(&key));
        assert!(g.claim("bogus").is_none());
    }

    #[test]
    fn the_busy_notice_is_rate_limited() {
        let g = gate();
        let t0 = Instant::now();
        assert!(g.take_busy_notice(t0));
        assert!(!g.take_busy_notice(t0 + Duration::from_secs(60)));
        assert!(g.take_busy_notice(t0 + BUSY_NOTICE_EVERY));
    }

    #[test]
    fn the_cap_notice_fires_once_per_long_episode() {
        let g = gate();
        let t0 = Instant::now();
        let minute = Duration::from_secs(60);
        // Exceeded at every sweep for 4 minutes: not yet.
        for m in 0..5u32 {
            assert!(!g.observe_cap_exceeded(t0 + minute * m), "minute {m}");
        }
        assert!(g.observe_cap_exceeded(t0 + minute * 5));
        assert!(!g.observe_cap_exceeded(t0 + minute * 6), "only once");
        // Back under the cap for a while: a new episode starts the clock again.
        let later = t0 + minute * 20;
        for m in 0..5u32 {
            assert!(!g.observe_cap_exceeded(later + minute * m), "later {m}");
        }
        assert!(g.observe_cap_exceeded(later + CAP_NOTICE_AFTER));
    }

    #[test]
    fn status_reports_load_queue_and_busy() {
        let g = gate();
        let t0 = Instant::now();
        let busy = sample(20.0, 8, MemoryPressure::Warn);
        let a = g.enqueue("workflow", t0);
        g.poll(a, &busy, &policy(), t0);
        let s = g.status(&busy, &policy(), t0 + Duration::from_secs(42));
        assert!(s.busy);
        assert_eq!(s.busy_reason.as_deref(), Some("memory pressure is high"));
        assert_eq!(s.queued.len(), 1);
        assert_eq!(s.queued[0].source, "workflow");
        assert_eq!(s.queued[0].waiting_secs, 42);
        assert_eq!(s.background_max_live, Some(6));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["memory_pressure"], "warn");
        // Disabled: never reported busy.
        let off = g.status(&busy, &AdmissionPolicy::disabled(8), t0);
        assert!(!off.busy);
        assert_eq!(off.busy_reason, None);
        // Disk is unknown until the manager adds it.
        assert_eq!(off.disk_free_bytes, None);
        assert!(!off.low_disk);
        let gib = 1024 * 1024 * 1024;
        let low = off.with_disk(Some(12 * gib), 30.0);
        assert!(low.low_disk);
        let v = serde_json::to_value(&low).unwrap();
        assert_eq!(v["disk_free_bytes"], 12 * gib);
        assert_eq!(v["low_disk_gb"], 30.0);
        assert_eq!(v["low_disk"], json!(true));
    }

    #[test]
    fn the_low_disk_notice_fires_once_per_episode_and_clears() {
        let g = gate();
        assert_eq!(g.observe_low_disk(false), None, "fine from the start");
        assert_eq!(g.observe_low_disk(true), Some(true), "episode starts");
        assert_eq!(g.observe_low_disk(true), None, "only once");
        assert_eq!(g.observe_low_disk(false), Some(false), "recovered");
        assert_eq!(g.observe_low_disk(false), None);
        assert_eq!(g.observe_low_disk(true), Some(true), "a new episode");
    }
}
