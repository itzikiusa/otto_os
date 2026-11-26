//! Machine-aware admission for engine-owned background agents.
//!
//! Before 2026-11-12 the only limits were per-feature COUNT caps (8 reviewer
//! slots, 2 workflow runs) and an advisory live-session cap that only ever
//! suspended idle sessions. Two code reviews (6 reviewers each) and a nightly
//! docs job then started together on an 8-core / 16 GB Mac: swap went from 4
//! to 22 GB, every database query took seconds, and both reviews failed.
//!
//! This gate sits in front of every spawn of an UNATTENDED background agent
//! ([`ADMISSION_GATED_SOURCES`]) — PTY sessions and structured-runtime
//! threads alike — and lets one start only when
//! - fewer than [`AdmissionPolicy::max_live`] of them are live (a hard
//!   ceiling — `agent_background_max_live`, default
//!   `max(2, min(cores × 3/4, ram_gb / 3))`: 5 on an 8-core / 16 GB Mac), and
//! - the machine is not under pressure — the 1-minute load at most
//!   `cores × agent_admission_max_load_per_core` (default 1.5) and macOS
//!   memory pressure below "warn" — OR fewer than
//!   [`AdmissionPolicy::pressure_floor`] of them are live
//!   (`agent_admission_pressure_floor`, default `max(1, ceiling / 2)`).
//!   Waiting only helps when Otto's own agents are the load: a machine that
//!   stays loaded for other reasons (the other Mac, from 2026-11-17 on) held
//!   every step of a lone workflow run 300 s and then started it anyway.
//!   Under CRITICAL memory pressure the floor is 1 — an agent starts only
//!   when no other one runs — and that wait never gives way: Otto's running
//!   agents finish, so waiting for them protects the machine.
//!
//! Waiters queue first-in first-out. Otherwise nothing waits forever: a load
//! / warn-memory wait gives way after `agent_admission_max_wait_secs`
//! (default 300 s) and a ceiling wait after [`CEILING_WAIT_FACTOR`] times
//! that, both with a logged warning and at most one per [`OVERDUE_SPACING`]
//! — a stuck engine holding its sessions live must not freeze every review
//! behind it, and a burst of give-ways must not recreate the storm. Every
//! wait is data ([`AdmissionWait`]) with its words, reported to the waiter,
//! `GET /machine/status` and the "Background agents queued" notice. The orchestrator's own headless claude turns take
//! the same gate when an unattended engine runs them
//! ([`ORCHESTRATOR_TURN_SOURCES`]). Sessions the user starts (Agents page, terminals,
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

use chrono::{DateTime, Utc};
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
/// Setting (integer): load / memory pressure holds a new background agent
/// only while at least this many are live. Absent = [`default_pressure_floor`];
/// `0` = always (the rule before 2026-11-26).
pub const PRESSURE_FLOOR_SETTING: &str = "agent_admission_pressure_floor";

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
/// Minimum gap between two admissions that GIVE WAY (waited the maximum,
/// still blocked): several waiters queued together reach the maximum
/// together, and starting them all within seconds is the storm again.
pub const OVERDUE_SPACING: Duration = Duration::from_secs(60);
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

/// Remove a pre-acquired permit's key from `meta` (call it before the row is
/// written); the key, if one was there.
pub fn take_permit_key(meta: &mut Value) -> Option<String> {
    match meta.as_object_mut()?.remove(PERMIT_META_KEY)? {
        Value::String(key) => Some(key),
        _ => None,
    }
}

/// The `meta.source` values the gate holds back: engine-owned agents that run
/// UNATTENDED, where nobody is watching the session start. A subset of
/// `BACKGROUND_SESSION_SOURCES` plus `personal_agent` (personal-agent runs
/// are deliberately outside that list — they show in the sidebar — but a
/// scheduled run is unattended work all the same).
///
/// Not `swarm`: its roles are started together and talk to each other, so
/// a ceiling that admits four of six would stall the four on the two it
/// holds back until the ceiling's long wait ran out.
///
/// Also the [`ORCHESTRATOR_TURN_SOURCES`]: headless one-shot turns engines run
/// through the orchestrator's own claude PTY, gated via
/// `SessionManager::background_admission_hook`.
pub const ADMISSION_GATED_SOURCES: [&str; 16] = [
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
    "scheduled_task",
    "finding",
    "personal_agent",
    "self_improvement",
    "product",
    "plugin",
];

/// Labels of UNATTENDED headless orchestrator turns (no session row, so never
/// a session's `meta.source` nor a `BACKGROUND_SESSION_SOURCES` entry): the
/// self-improvement analysis (`self_improvement`), the story watcher's
/// reconcile (`product`) and a runtime plugin's agent turn (`plugin`).
pub const ORCHESTRATOR_TURN_SOURCES: [&str; 3] = ["self_improvement", "product", "plugin"];

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

/// Default live ceiling: three quarters of the cores, and no more than one
/// agent per 3 GB of RAM (one CLI is about 0.8–0.9 GB plus its MCP process
/// and the builds it starts), at least 2 — 6 on an 8-core / 32 GB Mac, 5 on
/// an 8-core / 16 GB one. `ram_bytes` unknown: cores only.
pub fn default_max_live(cores: usize, ram_bytes: Option<u64>) -> usize {
    let by_ram = ram_bytes.map_or(usize::MAX, |b| (b >> 30) as usize / 3);
    (cores * 3 / 4).min(by_ram).max(2)
}

/// Default pressure floor: half the ceiling, at least 1 — a review's first
/// reviewers start at once on a machine that is loaded for other reasons.
pub fn default_pressure_floor(max_live: usize) -> usize {
    (max_live / 2).max(1)
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
    /// Load / memory pressure holds an agent only while at least this many
    /// gated agents are live (1 under critical memory pressure); `0` =
    /// always, with the give-way even under critical pressure (the old rule).
    pub pressure_floor: usize,
    pub max_load_per_core: f64,
    pub max_wait: Duration,
}

impl AdmissionPolicy {
    /// The defaults for a machine with `cores` logical CPUs and `ram_bytes`.
    pub fn defaults(cores: usize, ram_bytes: Option<u64>) -> Self {
        Self::from_settings(cores, ram_bytes, [None; 5])
    }

    /// Off: no settings store (bare harness) or switched off.
    pub fn disabled(cores: usize) -> Self {
        Self {
            enabled: false,
            ..Self::defaults(cores, None)
        }
    }

    /// Parse the five settings — enabled, max live, pressure floor, max
    /// load per core, max wait secs, in that order; anything missing or
    /// malformed takes its default (a typo never disables the gate or
    /// removes the ceiling).
    pub fn from_settings(
        cores: usize,
        ram_bytes: Option<u64>,
        values: [Option<&Value>; 5],
    ) -> Self {
        let [enabled, max_live, floor, max_load_per_core, max_wait_secs] = values;
        let default_live = default_max_live(cores, ram_bytes);
        let max_live = match max_live.and_then(Value::as_u64) {
            Some(0) => None,
            Some(n) => Some(n as usize),
            None => Some(default_live),
        };
        Self {
            enabled: enabled.and_then(Value::as_bool).unwrap_or(true),
            max_live,
            pressure_floor: floor.and_then(Value::as_u64).map_or_else(
                || default_pressure_floor(max_live.unwrap_or(default_live)),
                |n| n as usize,
            ),
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

/// A gated background agent that is running (or holds a place): what
/// fills the ceiling, named in [`AdmissionWait::LiveCeiling`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentHolder {
    /// `None` for a place reserved before its session exists.
    pub session_id: Option<Id>,
    pub source: String,
    pub label: String,
    /// The workflow run it belongs to (`meta.run_id`).
    pub run_id: Option<Id>,
    pub since: DateTime<Utc>,
}

/// Why a background agent is not starting yet. `live` counts the gated
/// background agents running or holding a place; `floor` is the pressure
/// floor in force (1 under critical memory pressure).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdmissionWait {
    /// Behind earlier queued agents (first in, first out).
    Queue { ahead: usize },
    /// Just after another queued agent started ([`ADMIT_SPACING`]).
    Spacing,
    /// The live ceiling is reached; `holders` are the agents filling it.
    LiveCeiling {
        live: usize,
        max: usize,
        holders: Vec<AgentHolder>,
    },
    /// The 1-minute load is above the threshold.
    Load {
        load1: f64,
        limit: f64,
        live: usize,
        floor: usize,
    },
    /// macOS memory pressure is warn or critical.
    MemoryPressure {
        level: MemoryPressure,
        live: usize,
        floor: usize,
    },
}

/// "1 Otto agent is running" / "3 Otto agents are running".
fn agents_running(live: usize) -> String {
    match live {
        1 => "1 Otto agent is running".into(),
        n => format!("{n} Otto agents are running"),
    }
}

impl AdmissionWait {
    /// The words a person reads (step card, notice, status).
    pub fn describe(&self) -> String {
        match self {
            Self::Queue { ahead } => {
                format!("Waiting for a free agent slot — {ahead} ahead in line")
            }
            Self::Spacing => "Starting queued agents a few seconds apart".into(),
            Self::LiveCeiling { live, max, .. } => {
                format!("Waiting for a free agent slot — {live} of {max} running")
            }
            Self::Load {
                load1, limit, live, ..
            } => format!(
                "Waiting for the machine — load {load1:.1} (limit {limit:.1}), {}",
                agents_running(*live)
            ),
            Self::MemoryPressure { live, .. } => {
                format!("Waiting for memory — {}", agents_running(*live))
            }
        }
    }

    /// The step-event code: `slots` | `memory` | `load`; `None` for the
    /// few seconds of [`Self::Spacing`] (not reported).
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Queue { .. } | Self::LiveCeiling { .. } => Some("slots"),
            Self::Load { .. } => Some("load"),
            Self::MemoryPressure { .. } => Some("memory"),
            Self::Spacing => None,
        }
    }

    /// What a waiter is told again when it changes: the kind and its count
    /// (a load that moves by a tenth is not news).
    pub(crate) fn news_key(&self) -> (Option<&'static str>, usize) {
        let n = match self {
            Self::Queue { ahead } => *ahead,
            Self::LiveCeiling { live, .. }
            | Self::Load { live, .. }
            | Self::MemoryPressure { live, .. } => *live,
            Self::Spacing => 0,
        };
        (self.code(), n)
    }

    /// The machine's side only, for the busy chip: "memory pressure is high".
    fn machine_words(&self) -> String {
        match self {
            Self::Load { load1, limit, .. } => {
                format!("load average {load1:.1} is above {limit:.1}")
            }
            Self::MemoryPressure {
                level: MemoryPressure::Critical,
                ..
            } => "memory pressure is critical".into(),
            _ => "memory pressure is high".into(),
        }
    }
}

/// [`decide`]'s answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Admit,
    /// Still blocked, but waited the maximum: start anyway (logged).
    AdmitOverdue(AdmissionWait),
    Wait(AdmissionWait),
}

/// The machine's load or memory pressure, ignoring Otto's own agents (no
/// ceiling, no floor): what `machine_busy` and the busy chip report. The
/// reason's `live` / `floor` are 0 here; [`decide`] fills them in.
pub fn pressure_reason(sample: &MachineSample, policy: &AdmissionPolicy) -> Option<AdmissionWait> {
    if sample.pressure.is_elevated() {
        return Some(AdmissionWait::MemoryPressure {
            level: sample.pressure,
            live: 0,
            floor: 0,
        });
    }
    let limit = sample.cores.max(1) as f64 * policy.max_load_per_core;
    sample
        .load1
        .filter(|l| *l > limit)
        .map(|load1| AdmissionWait::Load {
            load1,
            limit,
            live: 0,
            floor: 0,
        })
}

/// The pure decision for the agent at the head of the queue: `live` gated
/// background agents running (or admitted and starting), `waited` since it
/// queued. The ceiling is checked first — it is instant, while the load
/// average trails reality by about a minute. Pressure then holds it only
/// while at least the floor of Otto's own agents run (waiting cannot help
/// otherwise); critical memory pressure lowers the floor to 1 and never
/// gives way (a floor of 0 keeps the old rule: always hold, always give way).
pub fn decide(
    sample: &MachineSample,
    policy: &AdmissionPolicy,
    live: usize,
    waited: Duration,
) -> Verdict {
    if let Some(max) = policy.max_live {
        if live >= max {
            let reason = AdmissionWait::LiveCeiling {
                live,
                max,
                holders: Vec::new(),
            };
            return if waited >= policy.max_wait * CEILING_WAIT_FACTOR {
                Verdict::AdmitOverdue(reason)
            } else {
                Verdict::Wait(reason)
            };
        }
    }
    let Some(mut reason) = pressure_reason(sample, policy) else {
        return Verdict::Admit;
    };
    let hold = policy.pressure_floor > 0 && sample.pressure == MemoryPressure::Critical;
    let floor = if hold { 1 } else { policy.pressure_floor };
    if live < floor {
        return Verdict::Admit;
    }
    if let AdmissionWait::Load {
        live: l, floor: f, ..
    }
    | AdmissionWait::MemoryPressure {
        live: l, floor: f, ..
    } = &mut reason
    {
        (*l, *f) = (live, floor);
    }
    if !hold && waited >= policy.max_wait {
        Verdict::AdmitOverdue(reason)
    } else {
        Verdict::Wait(reason)
    }
}

/// How soon a deferred maintenance pass looks again ([`defer_maintenance`]).
pub const MAINTENANCE_RETRY: Duration = Duration::from_secs(5 * 60);
/// The longest a maintenance pass waits for a quiet machine before it runs
/// anyway: retention must not starve on a machine that never calms down.
pub const MAINTENANCE_MAX_DEFER: Duration = Duration::from_secs(6 * 3600);

/// The machine is over the gate's load / memory threshold ([`pressure_reason`]:
/// the ceiling and the floor are about agents, not the machine). Never busy
/// while the gate is off.
pub fn machine_busy(sample: &MachineSample, policy: &AdmissionPolicy) -> bool {
    policy.enabled && pressure_reason(sample, policy).is_some()
}

/// Whether a periodic maintenance pass (retention DELETEs, WAL checkpoint,
/// vacuum) should wait for a quieter tick. On 2026-11-12 the hourly pass ran
/// during an overnight overload and a single retention DELETE took 29 s.
/// `deferred_since` is the caller's episode clock (start it `None`): after
/// [`MAINTENANCE_MAX_DEFER`] of continuous deferral the pass runs anyway.
pub fn defer_maintenance(busy: bool, deferred_since: &mut Option<Instant>, now: Instant) -> bool {
    if !busy {
        *deferred_since = None;
        return false;
    }
    let since = *deferred_since.get_or_insert(now);
    if now.saturating_duration_since(since) >= MAINTENANCE_MAX_DEFER {
        *deferred_since = None;
        return false;
    }
    true
}

/// Outcome of one [`AdmissionGate::poll`].
#[derive(Debug, Clone, PartialEq)]
pub enum Poll {
    Admitted(Admitted),
    Wait(AdmissionWait),
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
    pub overdue: Option<AdmissionWait>,
}

struct Waiter {
    ticket: u64,
    source: String,
    label: String,
    since: Instant,
    reason: Option<AdmissionWait>,
    waited_once: bool,
}

struct Reserved {
    key: String,
    holder: AgentHolder,
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
    /// The last admission that gave way ([`OVERDUE_SPACING`]).
    last_overdue: Option<Instant>,
    busy_notice: Option<Instant>,
    cap: CapWatch,
    /// A low-disk notice is out (cleared by the recovery notice).
    low_disk_noticed: bool,
}

/// A spawned agent process the gate counts while it runs.
struct Running {
    /// `Some` for one of the ceiling's gated background agents.
    gated: Option<AgentHolder>,
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

    /// Join the queue; returns the ticket to [`Self::poll`] with. `label`
    /// names the agent while it holds a place.
    pub fn enqueue(&self, source: &str, label: &str, now: Instant) -> u64 {
        let mut st = self.state();
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        st.queue.push_back(Waiter {
            ticket,
            source: source.to_string(),
            label: label.to_string(),
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
        let holder = st.reserved.remove(&old)?.holder;
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        let key = format!("{}:{ticket}", uuid::Uuid::new_v4());
        st.reserved.insert(
            ticket,
            Reserved {
                key: key.clone(),
                holder,
            },
        );
        Some((ticket, key))
    }

    /// A place taken at once, without queueing — for a spawn that must not
    /// wait (a person's prompt into a dormant engine thread, a sub-thread, a
    /// wake): it is counted from now on, and its process registers next.
    pub fn reserve_now(self: &Arc<Self>, source: &str, label: &str) -> AdmissionPermit {
        let mut st = self.state();
        st.next_ticket += 1;
        let ticket = st.next_ticket;
        let key = format!("{}:{ticket}", uuid::Uuid::new_v4());
        let holder = AgentHolder {
            session_id: None,
            source: source.to_string(),
            label: label.to_string(),
            run_id: None,
            since: Utc::now(),
        };
        st.reserved.insert(
            ticket,
            Reserved {
                key: key.clone(),
                holder,
            },
        );
        AdmissionPermit::reserved(Arc::clone(self), ticket, key)
    }

    /// Count `id`'s freshly spawned process — PTY or structured runtime —
    /// until it exits. `gated` names one of the ceiling's background agents.
    pub fn register(
        &self,
        id: &Id,
        gated: Option<AgentHolder>,
        exit: watch::Receiver<Option<i32>>,
    ) {
        self.running.insert(id.clone(), Running { gated, exit });
    }

    /// Live (agents, gated background agents); prunes the exited ones.
    pub fn live_counts(&self) -> (usize, usize) {
        self.running.retain(|_, r| r.alive());
        let gated = self.running.iter().filter(|r| r.gated.is_some()).count();
        (self.running.len(), gated)
    }

    /// Live agent processes other than `id` (a respawn replaces its own).
    pub fn live_agents_except(&self, id: &Id) -> usize {
        self.live_counts();
        self.running.iter().filter(|r| r.key() != id).count()
    }

    /// The gated agents filling the ceiling — running, then holding a place
    /// — oldest first.
    pub fn holders(&self) -> Vec<AgentHolder> {
        self.live_counts();
        let mut all: Vec<AgentHolder> = self
            .running
            .iter()
            .filter_map(|r| r.gated.clone())
            .collect();
        all.extend(self.state().reserved.values().map(|r| r.holder.clone()));
        all.sort_by_key(|h| h.since);
        all
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
            let reason = AdmissionWait::Queue { ahead: pos };
            let w = &mut st.queue[pos];
            w.reason = Some(reason.clone());
            w.waited_once = true;
            return Poll::Wait(reason);
        }
        let live = gated_live + st.reserved.len();
        let head = &st.queue[0];
        let waited = now.saturating_duration_since(head.since);
        let spaced = head.waited_once
            && st
                .last_admit
                .is_some_and(|t| now.saturating_duration_since(t) < ADMIT_SPACING);
        let overdue_spaced = st
            .last_overdue
            .is_some_and(|t| now.saturating_duration_since(t) < OVERDUE_SPACING);
        let verdict = match decide(sample, policy, live, waited) {
            Verdict::Admit if spaced => Verdict::Wait(AdmissionWait::Spacing),
            Verdict::AdmitOverdue(reason) if spaced || overdue_spaced => Verdict::Wait(reason),
            v => v,
        };
        let overdue = match verdict {
            Verdict::Wait(mut reason) => {
                if let AdmissionWait::LiveCeiling { holders, .. } = &mut reason {
                    drop(st);
                    *holders = self.holders();
                    st = self.state();
                }
                if let Some(head) = st.queue.iter_mut().find(|w| w.ticket == ticket) {
                    head.reason = Some(reason.clone());
                    head.waited_once = true;
                }
                return Poll::Wait(reason);
            }
            Verdict::Admit => None,
            Verdict::AdmitOverdue(reason) => {
                st.last_overdue = Some(now);
                Some(reason)
            }
        };
        let Some(head) = st.queue.pop_front() else {
            return Poll::Gone;
        };
        let key = format!("{}:{ticket}", uuid::Uuid::new_v4());
        let holder = AgentHolder {
            session_id: None,
            source: head.source,
            label: head.label,
            run_id: None,
            since: Utc::now(),
        };
        st.reserved.insert(
            ticket,
            Reserved {
                key: key.clone(),
                holder,
            },
        );
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
        let holders = self.holders();
        let st = self.state();
        let queued: Vec<QueuedAgent> = st
            .queue
            .iter()
            .map(|w| QueuedAgent {
                source: w.source.clone(),
                waiting_secs: now.saturating_duration_since(w.since).as_secs(),
                reason: w.reason.as_ref().map(AdmissionWait::describe),
                wait: w.reason.clone(),
            })
            .collect();
        let live = background_live + st.reserved.len();
        // What a background agent arriving now would see.
        let admission = if !policy.enabled {
            None
        } else if !st.queue.is_empty() {
            Some(AdmissionWait::Queue {
                ahead: st.queue.len(),
            })
        } else {
            match decide(sample, policy, live, Duration::ZERO) {
                Verdict::Wait(AdmissionWait::LiveCeiling { live, max, .. }) => {
                    Some(AdmissionWait::LiveCeiling { live, max, holders })
                }
                Verdict::Wait(r) | Verdict::AdmitOverdue(r) => Some(r),
                Verdict::Admit => None,
            }
        };
        let busy_reason = pressure_reason(sample, policy)
            .filter(|_| policy.enabled)
            .map(|r| r.machine_words());
        MachineStatus {
            load1: sample.load1,
            cores: sample.cores,
            load_per_core: sample.load_per_core(),
            memory_pressure: sample.pressure,
            swap_used_bytes: sample.swap_used_bytes,
            ram_bytes: sample.ram_bytes,
            admission_enabled: policy.enabled,
            max_load_per_core: policy.max_load_per_core,
            max_wait_secs: policy.max_wait.as_secs(),
            background_live: live,
            background_max_live: policy.max_live,
            pressure_floor: policy.pressure_floor,
            agents_live,
            busy: busy_reason.is_some(),
            busy_reason,
            admission,
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
    /// Physical memory (`null` when unreadable).
    pub ram_bytes: Option<u64>,
    pub admission_enabled: bool,
    pub max_load_per_core: f64,
    pub max_wait_secs: u64,
    /// Gated background agents running or starting (PTY and runtime).
    pub background_live: usize,
    /// The ceiling (`null` = none).
    pub background_max_live: Option<usize>,
    /// Pressure holds a new background agent only from this many live (0 =
    /// always).
    pub pressure_floor: usize,
    /// Every agent process Otto started that is still running (PTY and
    /// runtime).
    pub agents_live: usize,
    /// The machine is under load or memory pressure (never while the gate is
    /// off). Whether a new agent would wait is [`Self::admission`].
    pub busy: bool,
    pub busy_reason: Option<String>,
    /// What a background agent arriving now would wait for (`null` = it
    /// would start at once).
    pub admission: Option<AdmissionWait>,
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
    /// [`AdmissionWait::describe`] of [`Self::wait`].
    pub reason: Option<String>,
    pub wait: Option<AdmissionWait>,
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
            ram_bytes: None,
            taken: Instant::now(),
        }
    }

    fn calm() -> MachineSample {
        sample(1.0, 8, MemoryPressure::Normal)
    }

    fn policy() -> AdmissionPolicy {
        AdmissionPolicy::defaults(8, None)
    }

    /// Pressure holds every agent, as before 2026-11-26 (floor 0).
    fn strict() -> AdmissionPolicy {
        AdmissionPolicy {
            pressure_floor: 0,
            ..policy()
        }
    }

    const GIB: u64 = 1 << 30;

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

    fn holder(id: &str) -> AgentHolder {
        AgentHolder {
            session_id: Some(id.to_string()),
            source: "review".into(),
            label: id.into(),
            run_id: Some("run-1".into()),
            since: Utc::now() - chrono::Duration::seconds(60),
        }
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
        assert_eq!(p.pressure_floor, 3);
        assert_eq!(p.max_load_per_core, 1.5);
        assert_eq!(p.max_wait, Duration::from_secs(300));
        assert_eq!(default_max_live(2, None), 2);
        assert_eq!(default_max_live(16, None), 12);
    }

    /// DR-023.1: one CLI per 3 GB of RAM at most.
    #[test]
    fn the_default_ceiling_is_capped_by_ram() {
        // The other Mac: 8 cores, 16 GB — ceiling 5, floor 2.
        let other = AdmissionPolicy::defaults(8, Some(16 * GIB));
        assert_eq!((other.max_live, other.pressure_floor), (Some(5), 2));
        assert_eq!(default_max_live(14, Some(18 * GIB)), 6, "14 cores / 18 GB");
        assert_eq!(default_max_live(8, Some(64 * GIB)), 6, "cores bind");
        assert_eq!(default_max_live(4, Some(8 * GIB)), 2, "never below 2");
        assert_eq!(default_pressure_floor(2), 1);
        // An explicit ceiling sets the floor's default too; 0 = no ceiling.
        let set = AdmissionPolicy::from_settings(
            8,
            Some(16 * GIB),
            [None, Some(&json!(8)), None, None, None],
        );
        assert_eq!((set.max_live, set.pressure_floor), (Some(8), 4));
    }

    #[test]
    fn settings_parse_and_bad_values_fall_back() {
        let p = AdmissionPolicy::from_settings(
            8,
            None,
            [
                Some(&json!(false)),
                Some(&json!(0)),
                Some(&json!(0)),
                Some(&json!(2.5)),
                Some(&json!(60)),
            ],
        );
        assert!(!p.enabled);
        assert_eq!(p.max_live, None, "0 = no ceiling");
        assert_eq!(p.pressure_floor, 0, "0 = the old rule");
        assert_eq!(p.max_load_per_core, 2.5);
        assert_eq!(p.max_wait, Duration::from_secs(60));
        let p = AdmissionPolicy::from_settings(
            8,
            None,
            [
                Some(&json!("yes")),
                Some(&json!("many")),
                Some(&json!("few")),
                Some(&json!(-1)),
                Some(&json!("soon")),
            ],
        );
        assert_eq!(p, AdmissionPolicy::defaults(8, None));
        assert!(!AdmissionPolicy::disabled(8).enabled);
        assert!(low_priority_enabled(None));
        assert!(!low_priority_enabled(Some(&json!(false))));
    }

    #[test]
    fn only_unattended_background_agents_are_gated() {
        let meta = |s: &str| json!({ "source": s });
        for s in ["review", "review_summarizer", "workflow", "scheduled_task"] {
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
        // Every gated source except personal_agent and the headless turn
        // labels is a background session source.
        for s in ADMISSION_GATED_SOURCES {
            assert!(
                s == "personal_agent"
                    || ORCHESTRATOR_TURN_SOURCES.contains(&s)
                    || otto_core::domain::BACKGROUND_SESSION_SOURCES.contains(&s),
                "{s} is not a background source"
            );
        }
        // Every headless turn label is gated, and none doubles as a session
        // source (it would then also gate that engine's sessions).
        for s in ORCHESTRATOR_TURN_SOURCES {
            assert!(is_gated_source(s), "{s}");
            assert!(
                !otto_core::domain::BACKGROUND_SESSION_SOURCES.contains(&s),
                "{s}"
            );
        }
    }

    #[test]
    fn maintenance_waits_for_a_quiet_machine_but_not_forever() {
        let loaded = sample(13.0, 8, MemoryPressure::Normal);
        let tight = sample(0.5, 8, MemoryPressure::Warn);
        assert!(machine_busy(&loaded, &policy()));
        assert!(machine_busy(&tight, &policy()));
        assert!(!machine_busy(&calm(), &policy()));
        assert!(
            !machine_busy(&loaded, &AdmissionPolicy::disabled(8)),
            "gate off"
        );
        // A configured zero wait still counts as busy.
        let no_wait = AdmissionPolicy {
            max_wait: Duration::ZERO,
            ..policy()
        };
        assert!(machine_busy(&loaded, &no_wait));

        let t0 = Instant::now();
        let mut since = None;
        assert!(!defer_maintenance(false, &mut since, t0), "quiet: run now");
        assert!(defer_maintenance(true, &mut since, t0));
        assert!(defer_maintenance(true, &mut since, t0 + MAINTENANCE_RETRY));
        // Quiet again: run, and the episode clock resets.
        assert!(!defer_maintenance(
            false,
            &mut since,
            t0 + MAINTENANCE_RETRY * 2
        ));
        assert_eq!(since, None);
        // Busy for the whole maximum: run anyway, then a new episode starts.
        let t1 = t0 + Duration::from_secs(3600);
        assert!(defer_maintenance(true, &mut since, t1));
        assert!(!defer_maintenance(
            true,
            &mut since,
            t1 + MAINTENANCE_MAX_DEFER
        ));
        assert!(defer_maintenance(
            true,
            &mut since,
            t1 + MAINTENANCE_MAX_DEFER
        ));
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
    fn high_load_or_memory_pressure_holds_from_the_floor_then_gives_way() {
        let loaded = sample(13.0, 8, MemoryPressure::Normal);
        // Below the floor (3 on 8 cores) waiting cannot help: start.
        for live in 0..3 {
            assert_eq!(
                decide(&loaded, &policy(), live, Duration::ZERO),
                Verdict::Admit,
                "{live}"
            );
        }
        assert_eq!(
            decide(&loaded, &policy(), 3, Duration::from_secs(10)),
            Verdict::Wait(AdmissionWait::Load {
                load1: 13.0,
                limit: 12.0,
                live: 3,
                floor: 3
            })
        );
        assert!(matches!(
            decide(&loaded, &policy(), 3, Duration::from_secs(300)),
            Verdict::AdmitOverdue(AdmissionWait::Load { .. })
        ));
        let tight = sample(0.5, 8, MemoryPressure::Warn);
        assert_eq!(
            decide(&tight, &policy(), 4, Duration::ZERO),
            Verdict::Wait(AdmissionWait::MemoryPressure {
                level: MemoryPressure::Warn,
                live: 4,
                floor: 3
            })
        );
        // Floor 0 is the old rule: hold even a lone agent, give way at 300 s,
        // critical included.
        let critical = sample(0.5, 8, MemoryPressure::Critical);
        assert!(matches!(
            decide(&loaded, &strict(), 0, Duration::ZERO),
            Verdict::Wait(AdmissionWait::Load { live: 0, .. })
        ));
        assert!(matches!(
            decide(&critical, &strict(), 0, Duration::from_secs(300)),
            Verdict::AdmitOverdue(AdmissionWait::MemoryPressure { .. })
        ));
    }

    /// DR-009.2: the floor must not hide the machine's state from maintenance
    /// deferral and the busy chip.
    #[test]
    fn pressure_reason_ignores_the_floor_and_the_ceiling() {
        let loaded = sample(13.0, 8, MemoryPressure::Normal);
        assert!(matches!(
            pressure_reason(&loaded, &policy()),
            Some(AdmissionWait::Load { .. })
        ));
        assert_eq!(
            decide(&loaded, &policy(), 0, Duration::ZERO),
            Verdict::Admit
        );
        assert!(machine_busy(&loaded, &policy()), "busy although it admits");
        assert_eq!(pressure_reason(&calm(), &policy()), None);
    }

    #[test]
    fn wait_reasons_carry_their_words_and_codes() {
        let words = |w: AdmissionWait| (w.code(), w.describe());
        assert_eq!(
            words(AdmissionWait::Queue { ahead: 2 }),
            (
                Some("slots"),
                "Waiting for a free agent slot — 2 ahead in line".into()
            )
        );
        assert_eq!(
            words(AdmissionWait::LiveCeiling {
                live: 5,
                max: 5,
                holders: vec![]
            }),
            (
                Some("slots"),
                "Waiting for a free agent slot — 5 of 5 running".into()
            )
        );
        assert_eq!(
            words(AdmissionWait::MemoryPressure {
                level: MemoryPressure::Warn,
                live: 1,
                floor: 1
            }),
            (
                Some("memory"),
                "Waiting for memory — 1 Otto agent is running".into()
            )
        );
        assert_eq!(
            words(AdmissionWait::Load {
                load1: 30.04,
                limit: 12.0,
                live: 2,
                floor: 2
            }),
            (
                Some("load"),
                "Waiting for the machine — load 30.0 (limit 12.0), 2 Otto agents are running"
                    .into()
            )
        );
        assert_eq!(AdmissionWait::Spacing.code(), None);
        let v = serde_json::to_value(AdmissionWait::Load {
            load1: 13.0,
            limit: 12.0,
            live: 3,
            floor: 3,
        })
        .unwrap();
        assert_eq!(
            v,
            json!({"kind": "load", "load1": 13.0, "limit": 12.0, "live": 3, "floor": 3})
        );
    }

    #[test]
    fn the_live_ceiling_holds_longer_than_a_load_wait() {
        let max_wait = policy().max_wait;
        assert_eq!(
            decide(&calm(), &policy(), 6, max_wait),
            Verdict::Wait(AdmissionWait::LiveCeiling {
                live: 6,
                max: 6,
                holders: vec![]
            })
        );
        assert_eq!(
            decide(&calm(), &policy(), 6, max_wait * CEILING_WAIT_FACTOR),
            Verdict::AdmitOverdue(AdmissionWait::LiveCeiling {
                live: 6,
                max: 6,
                holders: vec![]
            })
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
        let a = g.enqueue("review", "review", t0);
        let b = g.enqueue("workflow", "workflow", t0);
        assert_eq!(
            g.poll(b, &calm(), &policy(), t0),
            Poll::Wait(AdmissionWait::Queue { ahead: 1 })
        );
        assert!(matches!(
            g.poll(a, &busy, &strict(), t0),
            Poll::Wait(AdmissionWait::Load { .. })
        ));
        // Calm again: the head goes first, and only the head.
        let first = admitted(g.poll(a, &calm(), &policy(), t0 + Duration::from_secs(30)));
        assert!(first.queued);
        assert_eq!(first.overdue, None);
        // b is now the head, but the queue drains a few seconds apart.
        let soon = t0 + Duration::from_secs(31);
        assert_eq!(
            g.poll(b, &calm(), &policy(), soon),
            Poll::Wait(AdmissionWait::Spacing)
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
        g.register(&"s1".to_string(), Some(holder("s1")), exit_rx);
        // A foreground agent runs too, but the ceiling ignores it.
        let (_fg_tx, fg_rx) = running();
        g.register(&"fg".to_string(), None, fg_rx);
        let a = g.enqueue("review", "review", t0);
        let first = admitted(g.poll(a, &calm(), &p, t0));
        let permit = AdmissionPermit::reserved(Arc::clone(&g), a, first.key.clone());
        assert!(g.holds(&first.key));
        let b = g.enqueue("review", "review", t0);
        // The wait names what fills the ceiling, oldest first.
        let Poll::Wait(AdmissionWait::LiveCeiling { live, max, holders }) =
            g.poll(b, &calm(), &p, t0)
        else {
            panic!("expected the ceiling");
        };
        assert_eq!((live, max), (2, 2));
        let labels: Vec<_> = holders.iter().map(|h| h.label.as_str()).collect();
        assert_eq!(labels, ["s1", "review"]);
        assert_eq!(holders[0].session_id.as_deref(), Some("s1"));
        assert_eq!(holders[0].run_id.as_deref(), Some("run-1"));
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
        g.register(&"s".to_string(), Some(holder("s")), rx);
        assert_eq!(g.live_counts(), (1, 1));
        drop(tx);
        assert_eq!(g.live_counts(), (0, 0));
        // A respawn replaces the old entry rather than double-counting it.
        let (_tx1, rx1) = running();
        let (_tx2, rx2) = running();
        g.register(&"s".to_string(), Some(holder("s")), rx1);
        g.register(&"s".to_string(), Some(holder("s")), rx2);
        assert_eq!(g.live_counts(), (1, 1));
        assert_eq!(g.live_agents_except(&"s".to_string()), 0);
        assert_eq!(g.live_agents_except(&"other".to_string()), 1);
    }

    #[test]
    fn a_cancelled_waiter_leaves_the_queue() {
        let g = gate();
        let t0 = Instant::now();
        let a = g.enqueue("review", "review", t0);
        let b = g.enqueue("review", "review", t0);
        {
            let _guard = QueuedTicket::new(&g, a);
        }
        assert_eq!(g.poll(a, &calm(), &policy(), t0), Poll::Gone);
        admitted(g.poll(b, &calm(), &policy(), t0));
        // A disarmed guard (admitted) does not touch the queue.
        let c = g.enqueue("review", "review", t0);
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
        let a = g.enqueue("review", "review", t0);
        assert!(matches!(g.poll(a, &busy, &strict(), t0), Poll::Wait(_)));
        let late = admitted(g.poll(a, &busy, &strict(), t0 + DEFAULT_MAX_WAIT));
        assert!(matches!(late.overdue, Some(AdmissionWait::Load { .. })));
        assert_eq!(late.waited, DEFAULT_MAX_WAIT);
    }

    #[test]
    fn attach_marks_the_request_and_a_none_permit_does_nothing() {
        let g = gate();
        let t0 = Instant::now();
        let a = g.enqueue("review_summarizer", "review_summarizer", t0);
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
        let a = g.enqueue("review_summarizer", "review_summarizer", t0);
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
        let a = g.enqueue("workflow", "workflow", t0);
        g.poll(a, &busy, &strict(), t0);
        let s = g.status(&busy, &strict(), t0 + Duration::from_secs(42));
        assert!(s.busy);
        assert_eq!(s.busy_reason.as_deref(), Some("memory pressure is high"));
        assert_eq!(s.queued.len(), 1);
        assert_eq!(s.queued[0].source, "workflow");
        assert_eq!(s.queued[0].waiting_secs, 42);
        assert_eq!(
            s.queued[0].reason.as_deref(),
            Some("Waiting for memory — 0 Otto agents are running")
        );
        assert!(matches!(
            s.queued[0].wait,
            Some(AdmissionWait::MemoryPressure { .. })
        ));
        assert_eq!(s.admission, Some(AdmissionWait::Queue { ahead: 1 }));
        assert_eq!(s.background_max_live, Some(6));
        assert_eq!(s.pressure_floor, 0);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["memory_pressure"], "warn");
        assert_eq!(v["admission"]["kind"], "queue");
        assert_eq!(v["queued"][0]["wait"]["kind"], "memory_pressure");
        // Busy but below the floor, nobody queued: busy, yet an arrival starts.
        g.cancel(a);
        let s = g.status(&busy, &policy(), t0);
        assert!(s.busy);
        assert_eq!(s.admission, None);
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

    // ---- runtime.md §3.8: what the rule does for the export's cases ----
    //
    // A one-second virtual clock drives the real gate: jobs arrive (at a
    // time or when earlier jobs finish), queue, poll in queue order each
    // tick, run for their duration once admitted (registered with an exit
    // watch, as a spawned agent), then exit. "Permanent pressure" is the
    // export's machine: load above the limit and memory warn at every spawn.

    enum Arrive {
        At(u64),
        /// When every listed job has finished, plus a delay.
        After(Vec<usize>, u64),
    }

    struct Job {
        arrive: Arrive,
        runs: u64,
    }

    fn at(t: u64, runs: u64) -> Job {
        Job {
            arrive: Arrive::At(t),
            runs,
        }
    }

    fn after(deps: &[usize], delay: u64, runs: u64) -> Job {
        Job {
            arrive: Arrive::After(deps.to_vec(), delay),
            runs,
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct Started {
        arrived: u64,
        at: u64,
        overdue: bool,
        /// Other gated agents live when it was admitted.
        live: usize,
        critical: bool,
    }

    impl Started {
        fn waited(&self) -> u64 {
            self.at - self.arrived
        }
    }

    struct Sim {
        starts: Vec<Started>,
        max_live: usize,
    }

    fn other_mac() -> AdmissionPolicy {
        AdmissionPolicy::defaults(8, Some(16 * GIB))
    }

    fn pressured(_t: u64, _live: usize) -> MachineSample {
        sample(30.0, 8, MemoryPressure::Warn)
    }

    #[allow(clippy::needless_range_loop)]
    fn simulate(
        jobs: &[Job],
        policy: &AdmissionPolicy,
        machine: impl Fn(u64, usize) -> MachineSample,
    ) -> Sim {
        let g = gate();
        let t0 = Instant::now();
        let n = jobs.len();
        let mut arrived: Vec<Option<u64>> = vec![None; n];
        let mut ticket: Vec<Option<u64>> = vec![None; n];
        let mut started: Vec<Option<Started>> = vec![None; n];
        let mut finish: Vec<Option<u64>> = vec![None; n];
        let mut exits: Vec<Option<watch::Sender<Option<i32>>>> = (0..n).map(|_| None).collect();
        let mut max_live = 0;
        for t in 0..20_000u64 {
            for i in 0..n {
                if finish[i] == Some(t) {
                    if let Some(tx) = exits[i].take() {
                        tx.send(Some(0)).unwrap();
                    }
                }
            }
            for (i, job) in jobs.iter().enumerate() {
                if arrived[i].is_some() {
                    continue;
                }
                let due = match &job.arrive {
                    Arrive::At(a) => Some(*a),
                    Arrive::After(deps, d) => deps
                        .iter()
                        .map(|&j| finish[j].filter(|f| *f <= t))
                        .collect::<Option<Vec<_>>>()
                        .map(|f| f.into_iter().max().unwrap_or(0) + d),
                };
                if due.is_some_and(|d| d <= t) {
                    arrived[i] = Some(t);
                    ticket[i] = Some(g.enqueue("review", "review", t0 + Duration::from_secs(t)));
                }
            }
            let now = t0 + Duration::from_secs(t);
            for i in 0..n {
                let Some(tk) = ticket[i].filter(|_| started[i].is_none()) else {
                    continue;
                };
                let live = g.live_counts().1;
                let s = machine(t, live);
                if let Poll::Admitted(a) = g.poll(tk, &s, policy, now) {
                    let (tx, rx) = running();
                    g.register(&format!("job{i}"), Some(holder(&format!("job{i}"))), rx);
                    g.release(tk); // its process registered: the place moves to it
                    exits[i] = Some(tx);
                    finish[i] = Some(t + jobs[i].runs);
                    started[i] = Some(Started {
                        arrived: arrived[i].unwrap(),
                        at: t,
                        overdue: a.overdue.is_some(),
                        live,
                        critical: s.pressure == MemoryPressure::Critical,
                    });
                    max_live = max_live.max(g.live_counts().1);
                }
            }
            if started.iter().all(Option::is_some)
                && finish.iter().all(|f| f.is_some_and(|f| f <= t))
            {
                break;
            }
        }
        Sim {
            starts: started
                .into_iter()
                .map(|s| s.expect("every job started"))
                .collect(),
            max_live,
        }
    }

    fn total_wait(sim: &Sim) -> u64 {
        sim.starts.iter().map(Started::waited).sum()
    }

    /// Five agent steps of a lone run, one after another (each step's
    /// thread is closed before the next starts).
    fn lone_steps(n: usize, runs: u64) -> Vec<Job> {
        (0..n)
            .map(|i| {
                if i == 0 {
                    at(0, runs)
                } else {
                    after(&[i - 1], 0, runs)
                }
            })
            .collect()
    }

    /// THE DEFECT: under permanent pressure a lone run's every agent waited
    /// 300 s and then started anyway. Now each starts at once.
    #[test]
    fn row_lone_pr_reviewer_run_starts_every_step_at_once_under_permanent_pressure() {
        let jobs = lone_steps(5, 120);
        let now = simulate(&jobs, &other_mac(), pressured);
        for s in &now.starts {
            assert_eq!(s.waited(), 0, "{s:?}");
        }
        // Today's rule on the same run: 300 s per step, 25 minutes in all.
        let before = AdmissionPolicy {
            pressure_floor: 0,
            ..other_mac()
        };
        let today = simulate(&jobs, &before, pressured);
        assert_eq!(total_wait(&today), 5 * 300);
    }

    #[test]
    fn row_scope_assessment_and_summarizer_start_at_once() {
        // scope → (nothing else live) → summarizer.
        let jobs = vec![at(0, 40), after(&[0], 600, 60)];
        let sim = simulate(&jobs, &other_mac(), pressured);
        assert_eq!(total_wait(&sim), 0);
    }

    /// Four reviewers 1–2 s apart, the two Codex ones first: they take the
    /// floor's places at once; the Claude ones wait for a reviewer to finish
    /// or the maximum, give-ways 60 s apart.
    #[test]
    fn row_reviewer_wave_starts_the_floor_at_once_and_the_rest_waits() {
        let wave = |codex: u64| vec![at(0, codex), at(1, codex), at(3, 179), at(4, 179)];
        let sim = simulate(&wave(419), &other_mac(), pressured);
        let s = &sim.starts;
        assert_eq!(
            (s[0].waited(), s[1].waited()),
            (0, 0),
            "Codex first, at once"
        );
        assert!(
            s[2].overdue && (300..=301).contains(&s[2].waited()),
            "{:?}",
            s[2]
        );
        assert!(s[3].overdue && s[3].at >= s[2].at + OVERDUE_SPACING.as_secs());
        let wave_ends = s
            .iter()
            .zip([419, 419, 179, 179])
            .map(|(s, r)| s.at + r)
            .max();
        assert!(
            wave_ends.is_some_and(|e| (480..=630).contains(&e)),
            "{wave_ends:?}"
        );
        // A Codex reviewer finishing early frees a place below the floor.
        let quick = simulate(&wave(100), &other_mac(), pressured);
        assert_eq!(quick.starts[2].at, 100);
        assert!(!quick.starts[2].overdue);
    }

    /// The whole PR Reviewer run: five steps, scope, a reviewer wave of four
    /// and the summarizer — about 35 minutes of gate wait gone.
    #[test]
    fn row_whole_pr_reviewer_run_loses_most_of_its_gate_wait() {
        let mut jobs = lone_steps(5, 120); // 0..=4
        jobs.push(after(&[4], 0, 40)); // 5: scope
        for (k, runs) in [419, 419, 179, 179].into_iter().enumerate() {
            jobs.push(after(&[5], k as u64, runs)); // 6..=9: reviewers
        }
        jobs.push(after(&[6, 7, 8, 9], 0, 60)); // 10: summarizer
        let before = AdmissionPolicy {
            pressure_floor: 0,
            ..other_mac()
        };
        let now = total_wait(&simulate(&jobs, &other_mac(), pressured));
        let today = total_wait(&simulate(&jobs, &before, pressured));
        assert!(now <= 12 * 60, "now {now} s");
        assert!(today - now >= 35 * 60, "saved {} s", today - now);
    }

    #[test]
    fn row_lone_quick_review_waits_only_for_its_third_reviewer() {
        let mut jobs = lone_steps(3, 90); // 0..=2
        jobs.push(after(&[2], 0, 40)); // 3: scope
        for k in 0..3 {
            jobs.push(after(&[3], k, 179)); // 4..=6
        }
        jobs.push(after(&[4, 5, 6], 0, 60)); // 7: summarizer
        let sim = simulate(&jobs, &other_mac(), pressured);
        for (i, s) in sim.starts.iter().enumerate() {
            let limit = if i == 6 {
                DEFAULT_MAX_WAIT.as_secs() + 1
            } else {
                0
            };
            assert!(s.waited() <= limit, "agent {i}: {s:?}");
        }
        assert!(sim.starts[6].waited() > 0, "the third reviewer is held");
    }

    #[test]
    fn row_twelve_reviewers_never_pass_the_ceiling_and_give_way_60_s_apart() {
        let jobs: Vec<Job> = (0..12).map(|k| at(k, 419)).collect();
        let sim = simulate(&jobs, &other_mac(), pressured);
        assert!(sim.max_live <= 5, "ceiling 5, saw {}", sim.max_live);
        assert_eq!((sim.starts[0].waited(), sim.starts[1].waited()), (0, 0));
        let mut overdue: Vec<u64> = sim
            .starts
            .iter()
            .filter(|s| s.overdue)
            .map(|s| s.at)
            .collect();
        overdue.sort_unstable();
        assert!(!overdue.is_empty());
        for w in overdue.windows(2) {
            assert!(w[1] - w[0] >= OVERDUE_SPACING.as_secs(), "{overdue:?}");
        }
        // Every start past the first two was a give-way or a freed place
        // below the floor — never more than the floor through pressure alone.
        for s in &sim.starts[2..] {
            assert!(s.overdue || s.live < 2, "{s:?}");
        }
    }

    /// Run B's checkout while run A has four reviewers live: this is where
    /// waiting helps — until A drops below the floor, or 300 s.
    #[test]
    fn row_a_second_run_waits_while_the_first_fills_the_floor() {
        let g = gate();
        let p = other_mac();
        let t0 = Instant::now();
        let busy = sample(30.0, 8, MemoryPressure::Warn);
        let mut a = Vec::new();
        for i in 0..4 {
            let (tx, rx) = running();
            g.register(&format!("a{i}"), Some(holder(&format!("a{i}"))), rx);
            a.push(tx);
        }
        let b = g.enqueue("workflow", "checkout", t0);
        let at = |s: u64| t0 + Duration::from_secs(s);
        assert!(matches!(
            g.poll(b, &busy, &p, t0),
            Poll::Wait(AdmissionWait::MemoryPressure {
                live: 4,
                floor: 2,
                ..
            })
        ));
        a.pop().unwrap().send(Some(0)).unwrap();
        a.pop().unwrap().send(Some(0)).unwrap();
        assert!(
            matches!(g.poll(b, &busy, &p, at(100)), Poll::Wait(_)),
            "2 live: still"
        );
        a.pop().unwrap().send(Some(0)).unwrap();
        let got = admitted(g.poll(b, &busy, &p, at(120)));
        assert_eq!(got.overdue, None, "A dropped below the floor");
        // Nobody finishes: it gives way at the maximum.
        let c = g.enqueue("workflow", "checkout", at(200));
        let (_t1, r1) = running();
        g.register(&"x".to_string(), Some(holder("x")), r1);
        g.release(b);
        assert!(matches!(g.poll(c, &busy, &p, at(499)), Poll::Wait(_)));
        assert!(admitted(g.poll(c, &busy, &p, at(500))).overdue.is_some());
    }

    /// Critical memory: the floor is 1 and the hold never gives way.
    #[test]
    fn row_critical_memory_waits_until_no_other_agent_runs() {
        let g = gate();
        let p = other_mac();
        let t0 = Instant::now();
        let critical = sample(1.0, 8, MemoryPressure::Critical);
        let (tx, rx) = running();
        g.register(&"r".to_string(), Some(holder("r")), rx);
        let a = g.enqueue("review", "review", t0);
        for late in [DEFAULT_MAX_WAIT, DEFAULT_MAX_WAIT * CEILING_WAIT_FACTOR * 2] {
            assert!(
                matches!(
                    g.poll(a, &critical, &p, t0 + late),
                    Poll::Wait(AdmissionWait::MemoryPressure {
                        live: 1,
                        floor: 1,
                        ..
                    })
                ),
                "no give-way after {late:?}"
            );
        }
        tx.send(Some(0)).unwrap();
        assert_eq!(
            admitted(g.poll(a, &critical, &p, t0 + DEFAULT_MAX_WAIT)).overdue,
            None
        );
        // Alone under critical pressure: waiting cannot help.
        assert_eq!(decide(&critical, &p, 0, Duration::ZERO), Verdict::Admit);
    }

    #[test]
    fn row_a_calm_machine_starts_anything_below_the_ceiling_at_once() {
        for live in 0..5 {
            assert_eq!(
                decide(&calm(), &other_mac(), live, Duration::ZERO),
                Verdict::Admit
            );
        }
        let jobs: Vec<Job> = (0..4).map(|k| at(k, 300)).collect();
        let sim = simulate(&jobs, &other_mac(), |_, _| calm());
        assert_eq!(total_wait(&sim), 0);
    }

    /// The swap storm (2026-11-12): two 6-reviewer reviews and a docs job on
    /// an 8-core / 16 GB Mac; memory goes critical once 4 agents run.
    #[test]
    fn row_the_swap_storm_is_held_by_ceiling_floor_and_critical_memory() {
        let mut jobs: Vec<Job> = (0..12).map(|k| at(k / 2, 419)).collect();
        jobs.push(at(0, 900)); // the docs job
        let storm = |_t: u64, live: usize| {
            let level = if live >= 4 {
                MemoryPressure::Critical
            } else {
                MemoryPressure::Warn
            };
            sample(40.0, 8, level)
        };
        let sim = simulate(&jobs, &other_mac(), storm);
        assert!(sim.max_live <= 5);
        for s in &sim.starts {
            assert!(
                !s.critical || s.live == 0,
                "admitted into critical memory: {s:?}"
            );
        }
        let mut overdue: Vec<u64> = sim
            .starts
            .iter()
            .filter(|s| s.overdue)
            .map(|s| s.at)
            .collect();
        overdue.sort_unstable();
        for w in overdue.windows(2) {
            assert!(w[1] - w[0] >= OVERDUE_SPACING.as_secs(), "{overdue:?}");
        }
        // Today's rule let 6 run and gave way into critical memory.
        let before = AdmissionPolicy {
            max_live: Some(6),
            pressure_floor: 0,
            ..other_mac()
        };
        let today = simulate(&jobs, &before, storm);
        assert!(today.starts.iter().any(|s| s.critical && s.live > 0));
    }

    #[test]
    fn a_reservation_taken_at_once_counts_and_names_its_holder() {
        let g = gate();
        let p = AdmissionPolicy {
            max_live: Some(1),
            ..policy()
        };
        let now_place = g.reserve_now("workflow", "WF step");
        assert!(now_place.is_reserved());
        let a = g.enqueue("review", "review", Instant::now());
        let Poll::Wait(AdmissionWait::LiveCeiling { holders, .. }) =
            g.poll(a, &calm(), &p, Instant::now())
        else {
            panic!("the reserved place fills the ceiling");
        };
        assert_eq!(holders[0].label, "WF step");
        drop(now_place);
        admitted(g.poll(a, &calm(), &p, Instant::now()));
    }

    /// DR-115.4: parked steps waking at the same reset go through the gate
    /// one by one (queue, then spacing), not all at once.
    #[test]
    fn three_parked_steps_waking_together_pass_one_by_one() {
        let g = gate();
        let p = other_mac();
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Two agents run (the floor) on a loaded Mac when the reset comes.
        let mut live = Vec::new();
        for i in 0..2 {
            let (tx, rx) = running();
            g.register(&format!("r{i}"), Some(holder(&format!("r{i}"))), rx);
            live.push(tx);
        }
        let busy = sample(30.0, 8, MemoryPressure::Warn);
        let woken: Vec<u64> = (0..3)
            .map(|_| g.enqueue("workflow", "WF step", t0))
            .collect();
        for &t in &woken {
            assert!(matches!(g.poll(t, &busy, &p, t0), Poll::Wait(_)));
        }
        assert!(matches!(
            g.poll(woken[1], &busy, &p, t0),
            Poll::Wait(AdmissionWait::Queue { ahead: 1 })
        ));
        // The pressure clears: the head starts, the rest follow 5 s apart.
        let mut starts = Vec::new();
        for s in 10..30 {
            for &t in &woken {
                if let Poll::Admitted(_) = g.poll(t, &calm(), &p, at(s)) {
                    starts.push(s);
                }
            }
        }
        assert_eq!(starts, [10, 15, 20]);
    }

    #[test]
    fn the_permit_key_is_taken_out_of_meta() {
        let mut meta = json!({ "source": "workflow", "admission_permit": "k:1" });
        assert_eq!(take_permit_key(&mut meta).as_deref(), Some("k:1"));
        assert_eq!(meta, json!({ "source": "workflow" }));
        assert_eq!(take_permit_key(&mut meta), None);
        assert_eq!(take_permit_key(&mut json!(null)), None);
    }
}
