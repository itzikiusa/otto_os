//! Supervised background loops: a long-lived task that panics is logged and
//! restarted with backoff instead of silently vanishing until the next
//! daemon restart.
//!
//! `tokio::spawn` catches a panic in the task and drops the task: each boot
//! sweep and retention loop (`otto-server` `boot::tasks`) ran in one such
//! task, so a pass ending in a panic stopped that sweep for the rest of the
//! daemon's life. [`spawn`] runs the loop in a child task and, when it
//! panics, starts it again from the top after an exponential backoff
//! ([`BACKOFF_START`] doubling up to [`BACKOFF_MAX`]; a run that lasted
//! [`HEALTHY_RUN`] resets it), counting every restart ([`restarts`]).
//!
//! A loop that RETURNS is finished and is not restarted. Aborting the
//! returned handle also aborts the run in progress.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::task::JoinHandle;

/// Delay before the first restart.
pub const BACKOFF_START: Duration = Duration::from_secs(1);
/// The backoff never grows past this.
pub const BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
/// A run that lasted this long before panicking restarts at
/// [`BACKOFF_START`] again: it was healthy, not crash-looping.
pub const HEALTHY_RUN: Duration = Duration::from_secs(10 * 60);

static RESTARTS: AtomicU64 = AtomicU64::new(0);

/// Restarts of supervised tasks since the process started (all tasks).
pub fn restarts() -> u64 {
    RESTARTS.load(Ordering::Relaxed)
}

/// Delay before restart number `consecutive` (1-based) of one task.
pub fn backoff(consecutive: u32) -> Duration {
    let shift = consecutive.saturating_sub(1).min(16);
    BACKOFF_START.saturating_mul(1 << shift).min(BACKOFF_MAX)
}

/// Spawn `run(state.clone())` as a supervised loop named `name`. `state` is
/// what the loop owns (handles, repos); each restart gets a fresh clone.
pub fn spawn<S, F, Fut>(name: &'static str, state: S, run: F) -> JoinHandle<()>
where
    S: Clone + Send + 'static,
    F: Fn(S) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(supervise(name, state, run, backoff))
}

/// Aborts the run in progress when the supervisor itself is dropped/aborted.
struct Child(JoinHandle<()>);

impl Drop for Child {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn supervise<S, F, Fut>(name: &'static str, state: S, run: F, delay: fn(u32) -> Duration)
where
    S: Clone + Send + 'static,
    F: Fn(S) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut consecutive = 0u32;
    loop {
        let started = tokio::time::Instant::now();
        let mut child = Child(tokio::spawn(run(state.clone())));
        let err = match (&mut child.0).await {
            Ok(()) => return,
            Err(e) if e.is_cancelled() => return,
            Err(e) => e,
        };
        if started.elapsed() >= HEALTHY_RUN {
            consecutive = 0;
        }
        consecutive = consecutive.saturating_add(1);
        RESTARTS.fetch_add(1, Ordering::Relaxed);
        let wait = delay(consecutive);
        let payload = err.into_panic();
        let msg = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic");
        tracing::error!(
            task = name,
            consecutive,
            total_restarts = restarts(),
            "background task {name} panicked ({msg}); restarting in {wait:?}"
        );
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::sync::Arc;

    fn fast(_: u32) -> Duration {
        Duration::from_millis(1)
    }

    #[test]
    fn backoff_doubles_from_the_start_and_caps() {
        assert_eq!(backoff(1), BACKOFF_START);
        assert_eq!(backoff(2), BACKOFF_START * 2);
        assert_eq!(backoff(3), BACKOFF_START * 4);
        assert_eq!(backoff(30), BACKOFF_MAX);
        assert_eq!(backoff(u32::MAX), BACKOFF_MAX);
        assert_eq!(backoff(0), BACKOFF_START, "never below the start");
    }

    #[tokio::test]
    async fn a_panicking_loop_is_restarted_and_counted() {
        let runs = Arc::new(AtomicU32::new(0));
        let before = restarts();
        let sup = tokio::spawn(supervise(
            "test-panics-twice",
            Arc::clone(&runs),
            |runs: Arc<AtomicU32>| async move {
                if runs.fetch_add(1, Ordering::SeqCst) < 2 {
                    panic!("boom");
                }
            },
            fast,
        ));
        tokio::time::timeout(Duration::from_secs(5), sup)
            .await
            .expect("supervisor ends once the loop returns")
            .expect("supervisor itself never panics");
        assert_eq!(runs.load(Ordering::SeqCst), 3, "two restarts, then done");
        assert!(restarts() >= before + 2);
    }

    #[tokio::test]
    async fn a_loop_that_returns_is_not_restarted() {
        let runs = Arc::new(AtomicU32::new(0));
        let sup = tokio::spawn(supervise(
            "test-returns",
            Arc::clone(&runs),
            |runs: Arc<AtomicU32>| async move {
                runs.fetch_add(1, Ordering::SeqCst);
            },
            fast,
        ));
        tokio::time::timeout(Duration::from_secs(5), sup)
            .await
            .expect("done")
            .expect("no panic");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn aborting_the_supervisor_stops_the_running_loop() {
        let (alive_tx, alive_rx) = tokio::sync::oneshot::channel::<()>();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        // Each run takes the (single) senders out of the shared slot.
        type Slot = Arc<
            std::sync::Mutex<
                Option<(
                    tokio::sync::oneshot::Sender<()>,
                    tokio::sync::oneshot::Sender<()>,
                )>,
            >,
        >;
        let slot: Slot = Arc::new(std::sync::Mutex::new(Some((alive_tx, started_tx))));
        let sup = spawn("test-abort", slot, |slot: Slot| async move {
            let taken = slot.lock().unwrap().take();
            if let Some((_alive, started)) = taken {
                let _ = started.send(());
                std::future::pending::<()>().await;
            }
        });
        started_rx.await.expect("loop started");
        sup.abort();
        let _ = sup.await;
        let closed = tokio::time::timeout(Duration::from_secs(5), alive_rx)
            .await
            .expect("the running loop was dropped");
        assert!(closed.is_err(), "the loop's sender went away with it");
    }
}
