//! Outage recovery for [`UsageEngine`].
//!
//! The self-heal watcher only fires when an insert hits a DEAD server child.
//! When that restart (or the boot start) itself fails, the engine handle is
//! gone, nothing inserts against a child any more, and usage stayed down until
//! the daemon restarted (2026-11-12: six hours after an out-of-memory kill,
//! with the journal filling toward its 100k-row stop).
//!
//! So a failed (re)initialization arms a retry loop: [`RETRY_MINUTES`] apart,
//! then every 15 minutes, until a start succeeds. The loop yields to any
//! explicit `reinit` (it takes the same `reinit_lock`, and gives up its
//! schedule the moment another `reinit` ran), and stops when usage is disabled
//! or the engine shut down. The spacing is the protection against a machine
//! under memory pressure: one attempt per interval, never a spin.
//!
//! An idle-stopped (parked) server is not an outage: the handle stays present
//! and the next query wakes it, so nothing here keeps it awake.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use super::UsageEngine;

/// Minutes between start attempts while the engine is down; the last entry
/// repeats.
pub(super) const RETRY_MINUTES: [u64; 5] = [1, 2, 5, 10, 15];

/// How often a repeating warning may be logged again (with its count).
pub(super) const WARN_EVERY: Duration = Duration::from_secs(15 * 60);

/// The delay before retry number `attempt` (0-based), in units of `unit`
/// (a minute in production).
pub(super) fn retry_delay(attempt: usize, unit: Duration) -> Duration {
    let mins = RETRY_MINUTES[attempt.min(RETRY_MINUTES.len() - 1)];
    unit * mins as u32
}

/// Shared recovery state: the retry trigger, a generation bumped by every
/// (re)initialization, and when the current outage began.
pub(super) struct Recovery {
    /// Raised by an explicit `reinit` that left an enabled engine down. A
    /// permit is kept when the loop is busy, so a failure is never lost.
    pub(super) notify: tokio::sync::Notify,
    /// Bumped by every (re)initialization, explicit or retried.
    pub(super) gen: AtomicU64,
    /// When the engine became unavailable while enabled (cleared on success
    /// or when usage is disabled).
    outage_since: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    /// Set by `shutdown`: no retry may start a server after it.
    pub(super) shut_down: AtomicBool,
    /// Length of one RETRY_MINUTES unit, in ms (tests shorten it).
    unit_ms: AtomicU64,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            notify: tokio::sync::Notify::new(),
            gen: AtomicU64::new(0),
            outage_since: std::sync::Mutex::new(None),
            shut_down: AtomicBool::new(false),
            unit_ms: AtomicU64::new(60_000),
        }
    }
}

impl Recovery {
    /// Bookkeeping at the end of every (re)initialization.
    pub(super) fn after_reinit(&self, enabled: bool, available: bool) {
        self.gen.fetch_add(1, Ordering::SeqCst);
        let mut since = self.outage_since.lock().unwrap_or_else(|e| e.into_inner());
        if !enabled || available {
            *since = None;
        } else {
            since.get_or_insert_with(chrono::Utc::now);
        }
    }

    /// When the current outage began (RFC 3339), if the engine is down.
    pub(super) fn outage_since(&self) -> Option<String> {
        self.outage_since
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    }

    /// [`Self::outage_since`] as a time (the health notices measure it).
    pub(super) fn outage_started(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.outage_since.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn unit(&self) -> Duration {
        Duration::from_millis(self.unit_ms.load(Ordering::Relaxed))
    }

    #[cfg(test)]
    pub(super) fn set_unit(&self, unit: Duration) {
        self.unit_ms
            .store(unit.as_millis().max(1) as u64, Ordering::Relaxed);
    }
}

/// The retry loop. Weak, so it never keeps a dropped engine alive; the
/// engine's `Drop` raises `notify` so a waiting loop sees it gone and exits.
pub(super) fn spawn(weak: Weak<UsageEngine>, recovery: Arc<Recovery>) {
    tokio::spawn(async move {
        loop {
            recovery.notify.notified().await;
            let Some(mut seen) = weak
                .upgrade()
                .map(|e| e.recovery.gen.load(Ordering::SeqCst))
            else {
                return;
            };
            let mut attempt = 0;
            loop {
                tokio::time::sleep(retry_delay(attempt, recovery.unit())).await;
                let Some(engine) = weak.upgrade() else { return };
                match engine.retry_start(seen).await {
                    Some(gen) => {
                        seen = gen;
                        attempt += 1;
                    }
                    None => break,
                }
            }
        }
    });
}

impl UsageEngine {
    /// One retry. Returns the new generation while the engine is still down
    /// (keep backing off), or `None` when the loop should stop: the engine is
    /// up, usage is disabled, the engine shut down, or another `reinit` ran
    /// since `seen` (it re-arms the loop itself if it failed).
    pub(super) async fn retry_start(&self, seen: u64) -> Option<u64> {
        let _g = self.reinit_lock.lock().await;
        let rec = &self.recovery;
        if rec.shut_down.load(Ordering::SeqCst)
            || rec.gen.load(Ordering::SeqCst) != seen
            || self.available()
            || !self.config().enabled
        {
            return None;
        }
        tracing::info!("usage: retrying the clickhouse start after an outage");
        if self.reinit_locked(self.config()).await {
            return Some(rec.gen.load(Ordering::SeqCst));
        }
        if self.available() {
            tracing::info!("usage: clickhouse is back — usage tracking resumed");
        }
        None
    }
}

/// Logs a repeating warning on its first occurrence, then at most once per
/// [`WARN_EVERY`] with how many times it happened since. A success resets it,
/// so the next outage is reported at once.
#[derive(Default)]
pub(super) struct WarnGate {
    last: Option<Instant>,
    count: u64,
}

impl WarnGate {
    /// Record one occurrence; `Some(n)` when it should be logged now, `n`
    /// being the occurrences this log line covers.
    pub(super) fn hit(&mut self, now: Instant) -> Option<u64> {
        self.count += 1;
        match self.last {
            Some(t) if now.saturating_duration_since(t) < WARN_EVERY => None,
            _ => {
                self.last = Some(now);
                Some(std::mem::take(&mut self.count))
            }
        }
    }

    /// The condition cleared. Returns whether the gate had fired (so the
    /// caller can log the recovery once).
    pub(super) fn clear(&mut self) -> bool {
        let was = self.last.is_some();
        *self = Self::default();
        was
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UsageConfig;

    #[test]
    fn retry_schedule_backs_off_then_holds_at_fifteen_minutes() {
        let min = Duration::from_secs(60);
        let got: Vec<u64> = (0..8).map(|a| retry_delay(a, min).as_secs() / 60).collect();
        assert_eq!(got, [1, 2, 5, 10, 15, 15, 15, 15]);
    }

    #[test]
    fn warn_gate_logs_first_then_every_window_with_a_count() {
        let mut g = WarnGate::default();
        let t0 = Instant::now();
        assert_eq!(g.hit(t0), Some(1), "the first occurrence is logged");
        for i in 1..10 {
            assert_eq!(g.hit(t0 + Duration::from_secs(90 * i)), None);
        }
        // 10 more: the 9 held plus this one.
        assert_eq!(g.hit(t0 + WARN_EVERY), Some(10));
        assert!(g.clear());
        assert!(!g.clear());
        assert_eq!(
            g.hit(t0 + WARN_EVERY + Duration::from_secs(1)),
            Some(1),
            "a new outage is reported at once"
        );
    }

    #[test]
    fn outage_start_is_kept_across_failed_retries_and_cleared_on_success() {
        let r = Recovery::default();
        r.after_reinit(true, true);
        assert_eq!(r.outage_since(), None);
        r.after_reinit(true, false);
        let first = r.outage_since().expect("outage recorded");
        r.after_reinit(true, false);
        assert_eq!(r.outage_since(), Some(first), "a retry keeps the start");
        r.after_reinit(true, true);
        assert_eq!(r.outage_since(), None);
        r.after_reinit(true, false);
        r.after_reinit(false, false);
        assert_eq!(r.outage_since(), None, "disabled is not an outage");
        assert_eq!(r.gen.load(Ordering::SeqCst), 6);
    }

    fn failing() -> UsageConfig {
        UsageConfig {
            enabled: true,
            clickhouse_path: Some("/usr/bin/false".into()),
            ..Default::default()
        }
    }

    async fn wait_gen(engine: &UsageEngine, at_least: u64) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        while engine.recovery.gen.load(Ordering::SeqCst) < at_least {
            if Instant::now() > deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    /// A failed start is retried in the background with no caller and no
    /// insert — the 2026-11-12 outage stayed down until a daemon restart.
    #[tokio::test]
    async fn a_failed_start_is_retried_until_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let engine = UsageEngine::start(failing(), dir.path().to_path_buf()).await;
        // Before the boot start runs (current-thread runtime: it has not).
        engine.recovery.set_unit(Duration::from_millis(5));
        assert!(wait_gen(&engine, 3).await, "the loop never retried");
        assert!(!engine.available());
        assert!(engine.recovery.outage_since().is_some());
        let why = engine.start_error().expect("start error recorded");
        assert!(why.contains("server start failed"), "{why}");

        // Disabling stops the loop: no further (re)initialization happens.
        engine
            .reinit(UsageConfig {
                enabled: false,
                ..Default::default()
            })
            .await;
        let gen = engine.recovery.gen.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(engine.recovery.gen.load(Ordering::SeqCst), gen);
        assert_eq!(engine.recovery.outage_since(), None);
        engine.shutdown().await;
    }

    /// A retry yields to an explicit reinit that ran meanwhile, and never
    /// starts a server after shutdown.
    #[tokio::test]
    async fn retry_yields_to_an_explicit_reinit_and_to_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let engine = UsageEngine::start(failing(), dir.path().to_path_buf()).await;
        // Keep the background loop out of the way: an hour per unit.
        engine.recovery.set_unit(Duration::from_secs(3600));
        assert!(wait_gen(&engine, 1).await);
        let seen = engine.recovery.gen.load(Ordering::SeqCst);
        engine.reinit(failing()).await;
        assert_eq!(engine.retry_start(seen).await, None, "stale schedule");
        let now = engine.recovery.gen.load(Ordering::SeqCst);
        assert_eq!(engine.retry_start(now).await, Some(now + 1));
        engine.shutdown().await;
        assert_eq!(engine.retry_start(now + 1).await, None);
    }
}
