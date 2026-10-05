//! A cancel flag a sleeping supervisor is WOKEN by (perf SG-12).
//!
//! The background schedulers used to sleep their 60 s scan in 500 ms slices
//! only to re-check an `AtomicBool` — ~120 timer wakeups per minute each, for
//! nothing (App Nap / battery). [`CancelSignal::sleep`] parks on one timer and
//! a [`Notify`]: an idle scheduler wakes once per scan, and `cancel()` still
//! stops it immediately.
//!
//! Also the run bookkeeping the background engines share ([`InFlightSet`],
//! [`RunCancels`], [`until_cancelled`]). Lives in `otto-core` so leaf engine
//! crates (`otto-assistant`) can use it without depending on `otto-server`;
//! the daemon's own `crate::cancel_signal` / `scheduled_tasks_engine` copies are
//! to be folded onto this module once the parallel crate splits land.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Notify;

#[derive(Default)]
struct Inner {
    flag: AtomicBool,
    notify: Notify,
}

/// Cheap to clone; every clone observes the same cancellation.
#[derive(Clone, Default)]
pub struct CancelSignal {
    inner: Arc<Inner>,
}

impl CancelSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel: every current and future [`sleep`](Self::sleep) returns `true`.
    pub fn cancel(&self) {
        self.inner.flag.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.flag.load(Ordering::SeqCst)
    }

    /// Sleep `d`, or less if cancelled meanwhile. Returns `true` when the
    /// signal is cancelled (the caller should stop).
    pub async fn sleep(&self, d: Duration) -> bool {
        let notified = self.inner.notify.notified();
        tokio::pin!(notified);
        // Register interest BEFORE the flag check, so a cancel() racing in
        // between is never missed (notify_waiters only wakes registered ones).
        notified.as_mut().enable();
        if self.is_cancelled() {
            return true;
        }
        tokio::select! {
            _ = tokio::time::sleep(d) => self.is_cancelled(),
            _ = notified => true,
        }
    }

    /// Resolves once the signal is cancelled (immediately if it already is).
    pub async fn cancelled(&self) {
        while !self.sleep(Duration::from_secs(3600)).await {}
    }

    /// Two handles to the same signal (identity, like `Arc::ptr_eq`).
    pub fn same(&self, other: &CancelSignal) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

// ---- Run bookkeeping shared by the scheduled-task and personal-agent engines ----

/// Task ids with a run in flight. ONE set shared by the scheduler tick and the
/// manual "Run now" path: the scheduler used to keep its own private set while
/// Run-now only checked the newest DB row, so a scheduled occurrence fired on
/// top of a manual run still in progress (two agents, two worktrees, two
/// deliveries), and two quick Run-now clicks could both pass the row check.
#[derive(Clone, Default)]
pub struct InFlightSet(Arc<Mutex<HashSet<String>>>);

impl InFlightSet {
    /// Claim `task_id`; `None` when a run of it is already in flight. The
    /// claim is released when the returned guard drops — including on panic,
    /// so a crashed run can't wedge its task "in flight" until a restart.
    pub fn claim(&self, task_id: &str) -> Option<InFlightGuard> {
        let mut set = self.0.lock().unwrap_or_else(|e| e.into_inner());
        set.insert(task_id.to_string()).then(|| InFlightGuard {
            set: self.clone(),
            id: task_id.to_string(),
        })
    }
}

/// Releases a task's [`InFlightSet`] claim on drop. Poison-tolerant.
pub struct InFlightGuard {
    set: InFlightSet,
    id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.set
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Cancel handles of in-flight runs (run id → signal), behind
/// `POST /scheduled-tasks/runs/{run_id}/cancel` and its personal-agents twin.
/// A running run used to be unstoppable: killing its session only made the
/// retry loop open a fresh one.
#[derive(Default)]
pub struct RunCancels(Mutex<HashMap<String, CancelSignal>>);

impl RunCancels {
    /// Register `run_id` for the life of the returned guard.
    pub fn register(&'static self, run_id: &str) -> RunCancelGuard {
        let signal = CancelSignal::new();
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string(), signal.clone());
        RunCancelGuard {
            reg: self,
            id: run_id.to_string(),
            signal,
        }
    }

    /// Signal `run_id`'s run to stop; false when it isn't running here.
    pub fn cancel(&self, run_id: &str) -> bool {
        match self.0.lock().unwrap_or_else(|e| e.into_inner()).get(run_id) {
            Some(sig) => {
                sig.cancel();
                true
            }
            None => false,
        }
    }
}

/// A run's [`RunCancels`] registration; removed on drop (incl. panic).
pub struct RunCancelGuard {
    reg: &'static RunCancels,
    id: String,
    pub signal: CancelSignal,
}

impl Drop for RunCancelGuard {
    fn drop(&mut self) {
        self.reg
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
    }
}

/// Resolves once `sig` is cancelled.
pub async fn until_cancelled(sig: &CancelSignal) {
    while !sig.sleep(Duration::from_secs(3600)).await {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn an_uncancelled_sleep_runs_its_full_span() {
        let sig = CancelSignal::new();
        let t0 = Instant::now();
        assert!(!sig.sleep(Duration::from_millis(60)).await, "not cancelled");
        assert!(t0.elapsed() >= Duration::from_millis(60));
    }

    #[tokio::test]
    async fn cancelled_resolves_on_cancel_and_after_it() {
        let sig = CancelSignal::new();
        let s2 = sig.clone();
        let h = tokio::spawn(async move { s2.cancelled().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!h.is_finished(), "waits while not cancelled");
        sig.cancel();
        tokio::time::timeout(Duration::from_secs(5), h)
            .await
            .expect("woken by cancel")
            .unwrap();
        // Already cancelled → resolves immediately.
        tokio::time::timeout(Duration::from_millis(100), sig.cancelled())
            .await
            .expect("immediate");
    }

    #[tokio::test]
    async fn cancel_wakes_a_long_sleep_at_once() {
        let sig = CancelSignal::new();
        let s2 = sig.clone();
        let started = Instant::now();
        let h = tokio::spawn(async move { s2.sleep(Duration::from_secs(3600)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        sig.cancel();
        assert!(h.await.unwrap(), "reports cancelled");
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            sig.sleep(Duration::from_secs(3600)).await,
            "already cancelled → returns now"
        );
    }

    #[tokio::test]
    async fn clones_share_identity() {
        let a = CancelSignal::new();
        let b = a.clone();
        assert!(a.same(&b));
        assert!(!a.same(&CancelSignal::new()));
    }
}
