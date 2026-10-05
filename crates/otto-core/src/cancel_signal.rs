//! A cancel flag a sleeping supervisor is WOKEN by (perf SG-12).
//!
//! The background schedulers used to sleep their 60 s scan in 500 ms slices
//! only to re-check an `AtomicBool` — ~120 timer wakeups per minute each, for
//! nothing (App Nap / battery). [`CancelSignal::sleep`] parks on one timer and
//! a [`Notify`]: an idle scheduler wakes once per scan, and `cancel()` still
//! stops it immediately.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
