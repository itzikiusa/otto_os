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

fn bells() -> &'static Mutex<HashMap<String, Arc<Notify>>> {
    static B: OnceLock<Mutex<HashMap<String, Arc<Notify>>>> = OnceLock::new();
    B.get_or_init(Default::default)
}

fn bell(swarm_id: &str) -> Arc<Notify> {
    bells()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(swarm_id.to_string())
        .or_default()
        .clone()
}

/// Wake `swarm_id`'s coordinator (a permit is kept if it is mid-tick, so the
/// wake is never lost).
pub fn poke(swarm_id: &str) {
    bell(swarm_id).notify_one();
}

/// Create `swarm_id`'s bell when its coordinator starts, so the bus listener
/// (which only rings EXISTING bells) never misses an event that lands before
/// the loop's first park.
pub fn arm(swarm_id: &str) {
    let _ = bell(swarm_id);
}

/// Ring `swarm_id`'s bell only if a coordinator armed one — the bus carries
/// events of every swarm, and an inactive swarm must not gain a bell (perf
/// §15 N8). Returns whether a bell rang.
fn poke_known(swarm_id: &str) -> bool {
    let b = bells()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(swarm_id)
        .cloned();
    match b {
        Some(b) => {
            b.notify_one();
            true
        }
        None => false,
    }
}

#[cfg(test)]
fn known(swarm_id: &str) -> bool {
    bells()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(swarm_id)
}

/// Forget a swarm's bell (coordinator stopped for good).
pub fn forget(swarm_id: &str) {
    bells()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(swarm_id);
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
                        poke_known(sid);
                    }
                }
                Err(RecvError::Lagged(_)) => {
                    let all: Vec<Arc<Notify>> = bells()
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .values()
                        .cloned()
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

/// Park until the swarm's bell rings (no sooner than [`MIN_GAP`] after
/// `last_tick`), [`SAFETY_TICK`] passes, or `cancel` fires. Returns `true`
/// when cancelled (the caller stops).
pub async fn wait(cancel: &CancelSignal, swarm_id: &str, last_tick: Instant) -> bool {
    // A loop stopped mid-tick must not re-create the bell `forget` dropped.
    if cancel.is_cancelled() {
        return true;
    }
    let b = bell(swarm_id);
    tokio::select! {
        stop = cancel.sleep(SAFETY_TICK) => return stop,
        _ = b.notified() => {}
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
        let long_ago = Instant::now() - MIN_GAP * 2;
        let t = Instant::now();
        let (stopped, ()) = tokio::join!(wait(&cancel, sid, long_ago), async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            poke(sid);
        });
        assert!(!stopped);
        assert!(t.elapsed() < Duration::from_secs(1), "woken by the poke");
        // A poke while the loop was busy is kept (permit), not lost.
        poke(sid);
        let t = Instant::now();
        assert!(!wait(&cancel, sid, long_ago).await);
        assert!(t.elapsed() < Duration::from_millis(500));
        // Right after a tick, an event waits out the rest of the gap.
        poke(sid);
        let t = Instant::now();
        assert!(!wait(&cancel, sid, Instant::now()).await);
        assert!(t.elapsed() >= MIN_GAP - Duration::from_millis(50));
        forget(sid);
    }

    #[tokio::test]
    async fn cancel_stops_the_wait_at_once() {
        let cancel = CancelSignal::new();
        let c = cancel.clone();
        let t = Instant::now();
        let (stopped, ()) = tokio::join!(
            wait(&cancel, "swarm-wake-test-cancel", Instant::now()),
            async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                c.cancel();
            }
        );
        assert!(stopped);
        assert!(t.elapsed() < Duration::from_secs(1));
        forget("swarm-wake-test-cancel");
    }

    /// perf §15 N8: the listener path never creates a bell for a swarm with
    /// no coordinator; an armed bell rings until it is forgotten.
    #[test]
    fn listener_pokes_only_armed_swarms_and_forget_drops_the_bell() {
        let sid = "swarm-wake-test-armed";
        assert!(!poke_known(sid));
        assert!(!known(sid), "an inactive swarm gains no bell");
        arm(sid);
        assert!(poke_known(sid));
        forget(sid);
        assert!(!known(sid));
        assert!(!poke_known(sid));
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
