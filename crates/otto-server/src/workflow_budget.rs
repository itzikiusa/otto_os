//! Active execution budget shared by top-level nodes and loop children, and
//! the run's parking rule (engine.md §2.4, DR-007): while every executing node
//! waits on a person the run hands its run-gate slot back and the budget
//! stops counting; the first node to work again takes both back first.
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) struct ActiveBudget {
    state: Mutex<State>,
    /// Serializes re-taking the slot, so two unparks never take two permits.
    retake: tokio::sync::Mutex<()>,
}
struct State {
    deadline: Instant,
    paused: Option<Instant>,
    executing: usize,
    parked: usize,
    /// `Some(permit_released)` while every executing node is parked.
    released: Option<bool>,
}

impl ActiveBudget {
    pub(super) fn new(duration: Duration) -> Self {
        Self {
            state: Mutex::new(State {
                deadline: Instant::now() + duration,
                paused: None,
                executing: 0,
                parked: 0,
                released: None,
            }),
            retake: tokio::sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn expired(&self) -> bool {
        let state = self.lock();
        state.paused.is_none() && Instant::now() >= state.deadline
    }

    pub(super) async fn exhausted(&self) {
        loop {
            let delay = {
                let state = self.lock();
                if state.paused.is_some() {
                    Duration::from_millis(100)
                } else {
                    state
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(1))
                }
            };
            if delay.is_zero() {
                return;
            }
            tokio::time::sleep(delay).await;
        }
    }

    /// A node starts executing: leaving "all parked" re-takes the slot first.
    pub(super) async fn on_start(&self) {
        let _g = self.retake.lock().await;
        self.lock().executing += 1;
        self.retake_now().await;
    }

    /// A node settled: when every node still executing is parked, let go.
    pub(super) fn on_settle(&self) {
        let mut state = self.lock();
        state.executing = state.executing.saturating_sub(1);
        Self::reconcile(&mut state);
    }

    /// The node waits on a person until [`Park::unpark`] (or the drop of a
    /// canceled node, which only counts it out: the run is ending then).
    pub(super) fn park(&self) -> Park<'_> {
        let mut state = self.lock();
        state.parked += 1;
        Self::reconcile(&mut state);
        Park(self)
    }

    fn reconcile(state: &mut State) {
        if state.released.is_none() && state.executing > 0 && state.parked >= state.executing {
            state.released = Some(super::release_run_permit());
            state.paused = Some(Instant::now());
        }
    }

    /// `released` stays set while the slot is re-taken, so a park or a
    /// settle meanwhile does not release (and pause) a second time.
    async fn retake_now(&self) {
        let Some(permit) = self.lock().released else {
            return;
        };
        super::reacquire_run_permit(permit).await;
        let mut state = self.lock();
        state.released = None;
        if let Some(start) = state.paused.take() {
            state.deadline += start.elapsed();
        }
    }
}

/// One node parked on a person.
pub(super) struct Park<'a>(&'a ActiveBudget);
impl Park<'_> {
    pub(super) async fn unpark(self) {
        let budget = self.0;
        let _g = budget.retake.lock().await;
        budget.retake_now().await;
        // Drop counts the node out of the parked ones.
    }
}
impl Drop for Park<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        state.parked = state.parked.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CR1-022: a settle while the slot is being re-taken does not release
    /// (and pause) a second time; the re-take ends the parked state.
    #[tokio::test]
    async fn a_settle_during_a_retake_does_not_release_again() {
        let gate: &'static tokio::sync::Semaphore =
            Box::leak(Box::new(tokio::sync::Semaphore::new(1)));
        let permit = gate.try_acquire().unwrap();
        let cell = std::cell::RefCell::new((gate, Some(permit)));
        super::super::RUN_PERMIT
            .scope(cell, async {
                let budget = ActiveBudget::new(Duration::from_secs(60));
                budget.on_start().await;
                budget.on_start().await;
                let (a, b) = (budget.park(), budget.park());
                assert_eq!(
                    budget.lock().released,
                    Some(true),
                    "both parked: the slot is given back"
                );
                let other = gate.try_acquire().unwrap();
                let mut retake = Box::pin(a.unpark());
                assert!(
                    futures_util::poll!(&mut retake).is_pending(),
                    "the gate is full"
                );
                drop(b);
                budget.on_settle();
                drop(other);
                retake.await;
                let state = budget.lock();
                assert_eq!((state.released, state.paused), (None, None));
            })
            .await;
    }
    #[tokio::test]
    async fn a_park_pauses_the_budget_only_while_every_executing_node_is_parked() {
        let budget = ActiveBudget::new(Duration::ZERO);
        assert!(budget.expired());
        budget.on_start().await;
        budget.on_start().await;
        let first = budget.park();
        assert!(budget.expired(), "a sibling still works: the budget counts");
        let second = budget.park();
        assert!(!budget.expired(), "both parked: paused");
        assert!(
            tokio::time::timeout(Duration::from_millis(5), budget.exhausted())
                .await
                .is_err()
        );
        second.unpark().await;
        assert!(budget.expired(), "one works again: counting again");
        budget.on_settle();
        assert!(
            !budget.expired(),
            "the worker settled beside a parked node: paused"
        );
        drop(first);
        budget.on_start().await;
        assert!(budget.expired());
        tokio::time::timeout(Duration::from_millis(50), budget.exhausted())
            .await
            .unwrap();
    }
}
