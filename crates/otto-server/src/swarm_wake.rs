//! Event-driven swarm coordinator wakeups (perf W7).
//!
//! The coordinator ticked every 5 s per active swarm — 4–6 queries each — even
//! with nothing ready, all day. Everything that can make work ready already
//! announces itself on the event bus (`SwarmTaskUpdated` for a created /
//! changed / unblocked task, `SwarmRunUpdated` when a run frees an agent,
//! `SwarmStatus` on resume, `SwarmGoalUpdated`, `SwarmProjectCleared`). One
//! listener turns those into a per-swarm bell, and the coordinator parks on it
//! with a long safety tick ([`SAFETY_TICK`]) instead of polling.
//!
//! A burst of events (a tick's own dispatches, a run's status changes) wakes
//! the loop once; [`MIN_GAP`] keeps an event storm from ticking faster than
//! once per gap. Kept out of `swarm_runtime.rs` to touch the coordinator loop
//! as little as possible.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::event::Event;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Notify;

use crate::cancel_signal::CancelSignal;
use crate::state::ServerCtx;

/// Longest a coordinator sleeps without an event: a backstop for a missed
/// emit point and for time-based budgets (`max_runtime_secs`).
pub const SAFETY_TICK: Duration = Duration::from_secs(60);
/// Shortest spacing between two event-driven ticks of one swarm.
pub const MIN_GAP: Duration = Duration::from_secs(2);

/// A registered bell: the `Notify` plus how many coordinator loops hold it
/// (a restart overlaps the old loop's exit with the new loop's start).
struct Bell {
    notify: Arc<Notify>,
    users: usize,
}

fn bells() -> &'static Mutex<HashMap<String, Bell>> {
    static B: OnceLock<Mutex<HashMap<String, Bell>>> = OnceLock::new();
    B.get_or_init(Default::default)
}

/// A coordinator loop's hold on its swarm's bell. Registered when the loop
/// starts — before its first tick, so an event during that tick is kept as a
/// permit — and released when the loop returns (drop): the last holder
/// removes the entry (perf N7: bells used to live for the daemon's life).
pub struct BellGuard {
    swarm_id: String,
    notify: Arc<Notify>,
}

/// Register (or join) `swarm_id`'s bell for a coordinator loop.
pub fn register(swarm_id: &str) -> BellGuard {
    let mut map = bells().lock().unwrap_or_else(|e| e.into_inner());
    let b = map.entry(swarm_id.to_string()).or_insert_with(|| Bell {
        notify: Arc::default(),
        users: 0,
    });
    b.users += 1;
    BellGuard {
        swarm_id: swarm_id.to_string(),
        notify: b.notify.clone(),
    }
}

impl Drop for BellGuard {
    fn drop(&mut self) {
        let mut map = bells().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(b) = map.get_mut(&self.swarm_id) {
            b.users = b.users.saturating_sub(1);
            if b.users == 0 {
                map.remove(&self.swarm_id);
            }
        }
    }
}

/// Wake `swarm_id`'s coordinator (a permit is kept if it is mid-tick, so the
/// wake is never lost). Only a swarm with a running coordinator has a bell:
/// an event for any other swarm is a no-op and never allocates (perf N7 —
/// `poke` used to insert a bell for every swarm that ever emitted an event).
pub fn poke(swarm_id: &str) {
    let b = bells()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(swarm_id)
        .map(|b| b.notify.clone());
    if let Some(b) = b {
        b.notify_one();
    }
}

/// Whether `swarm_id` has a registered bell (tests / diagnostics).
pub fn has_bell(swarm_id: &str) -> bool {
    bells()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(swarm_id)
}

/// The swarm a coordinator-relevant event concerns.
fn swarm_of(ev: &Event) -> Option<&str> {
    match ev {
        Event::SwarmTaskUpdated { swarm_id, .. }
        | Event::SwarmRunUpdated { swarm_id, .. }
        | Event::SwarmStatus { swarm_id, .. }
        | Event::SwarmGoalUpdated { swarm_id, .. }
        | Event::SwarmProjectCleared { swarm_id, .. } => Some(swarm_id),
        _ => None,
    }
}

/// Start the bus listener once per process (idempotent; called from
/// `start_coordinator`). A lagged bus pokes every known swarm.
pub fn ensure_listener(ctx: &ServerCtx) {
    static STARTED: OnceLock<()> = OnceLock::new();
    if STARTED.set(()).is_err() {
        return;
    }
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let Some(sid) = swarm_of(&ev) {
                        poke(sid);
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    let all: Vec<Arc<Notify>> = bells()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .values()
                        .map(|b| b.notify.clone())
                        .collect();
                    for b in all {
                        b.notify_one();
                    }
                }
                Err(RecvError::Closed) => return,
            }
        }
    });
}

/// Park until the swarm's bell (`bell`, from [`register`]) rings (no sooner than [`MIN_GAP`] after
/// `last_tick`), [`SAFETY_TICK`] passes, or `cancel` fires. Returns `true`
/// when cancelled (the caller stops).
pub async fn wait(cancel: &CancelSignal, bell: &BellGuard, last_tick: Instant) -> bool {
    tokio::select! {
        stop = cancel.sleep(SAFETY_TICK) => return stop,
        _ = bell.notify.notified() => {}
    }
    let gap = MIN_GAP.saturating_sub(last_tick.elapsed());
    if !gap.is_zero() && cancel.sleep(gap).await {
        return true;
    }
    cancel.is_cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_poke_wakes_the_coordinator_after_the_min_gap() {
        let cancel = CancelSignal::new();
        let sid = "swarm-wake-test-poke";
        let bell = register(sid);
        let long_ago = Instant::now() - MIN_GAP * 2;
        let t = Instant::now();
        let (stopped, ()) = tokio::join!(wait(&cancel, &bell, long_ago), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            poke(sid);
        });
        assert!(!stopped);
        assert!(t.elapsed() < Duration::from_secs(1), "woken by the poke");
        // A poke while the loop was busy is kept (permit), not lost.
        poke(sid);
        let t = Instant::now();
        assert!(!wait(&cancel, &bell, long_ago).await);
        assert!(t.elapsed() < Duration::from_millis(500));
        // Right after a tick, an event waits out the rest of the gap.
        poke(sid);
        let t = Instant::now();
        assert!(!wait(&cancel, &bell, Instant::now()).await);
        assert!(t.elapsed() >= MIN_GAP - Duration::from_millis(50));
    }

    /// Perf N7: an event for a swarm without a coordinator allocates no
    /// bell; a loop's guard registers it and the last guard dropped (the loop
    /// returned) removes it — an overlapping restart keeps it alive.
    #[test]
    fn poke_never_inserts_and_the_last_guard_drops_the_bell() {
        let sid = "swarm-wake-test-noinsert";
        poke(sid);
        assert!(!has_bell(sid), "poke for an unregistered swarm is a no-op");
        let old_loop = register(sid);
        let new_loop = register(sid);
        assert!(has_bell(sid));
        drop(old_loop);
        assert!(has_bell(sid), "the restarted loop still holds it");
        drop(new_loop);
        assert!(!has_bell(sid), "gone once no loop runs");
    }

    /// Perf N2: an idle coordinator wakes once per burst. Fifty pokes inside
    /// MIN_GAP of the last tick resolve ONE wait (after the gap), and the
    /// next wait parks again (no stored burst of wakeups) until the safety
    /// tick.
    #[tokio::test]
    async fn a_burst_of_pokes_is_one_tick() {
        let sid = "swarm-wake-test-burst";
        let cancel = CancelSignal::new();
        let bell = register(sid);
        let ticked = Instant::now();
        for _ in 0..50 {
            poke(sid);
        }
        let t = Instant::now();
        assert!(!wait(&cancel, &bell, ticked).await);
        assert!(
            t.elapsed() >= MIN_GAP - Duration::from_millis(100),
            "gap kept"
        );
        let long_ago = Instant::now() - MIN_GAP * 2;
        let next =
            tokio::time::timeout(Duration::from_millis(200), wait(&cancel, &bell, long_ago)).await;
        assert!(next.is_err(), "no stored wakeups after a burst");
        assert!(SAFETY_TICK >= Duration::from_secs(60));
    }

    #[tokio::test]
    async fn cancel_stops_the_wait_at_once() {
        let cancel = CancelSignal::new();
        let c = cancel.clone();
        let bell = register("swarm-wake-test-cancel");
        let t = Instant::now();
        let (stopped, ()) = tokio::join!(wait(&cancel, &bell, Instant::now()), async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            c.cancel();
        });
        assert!(stopped);
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn only_swarm_events_ring() {
        let ev = Event::SwarmStatus {
            workspace_id: "w".into(),
            swarm_id: "s1".into(),
            status: "active".into(),
        };
        assert_eq!(swarm_of(&ev), Some("s1"));
        let other = Event::UsageMetricsTick { ts: "t".into() };
        assert_eq!(swarm_of(&other), None);
    }
}
