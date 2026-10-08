//! Active execution budget shared by top-level nodes and loop children.
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) struct ActiveBudget(Mutex<State>);
struct State {
    deadline: Instant,
    paused: Option<Instant>,
}

impl ActiveBudget {
    pub(super) fn new(duration: Duration) -> Self {
        Self(Mutex::new(State {
            deadline: Instant::now() + duration,
            paused: None,
        }))
    }

    pub(super) fn expired(&self) -> bool {
        let state = self.0.lock().unwrap();
        state.paused.is_none() && Instant::now() >= state.deadline
    }

    pub(super) async fn exhausted(&self) {
        loop {
            let delay = {
                let state = self.0.lock().unwrap();
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

    pub(super) fn pause(&self) -> ApprovalPause<'_> {
        self.0.lock().unwrap().paused = Some(Instant::now());
        ApprovalPause(self)
    }
}

/// Dropping a canceled approval future also resumes accounting.
pub(super) struct ApprovalPause<'a>(&'a ActiveBudget);
impl Drop for ApprovalPause<'_> {
    fn drop(&mut self) {
        let mut state = self.0 .0.lock().unwrap();
        if let Some(start) = state.paused.take() {
            state.deadline += start.elapsed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn approval_pause_excludes_wait_and_drop_restores_deadline() {
        let budget = ActiveBudget::new(Duration::ZERO);
        assert!(budget.expired());
        let paused = budget.pause();
        assert!(!budget.expired());
        assert!(
            tokio::time::timeout(Duration::from_millis(5), budget.exhausted())
                .await
                .is_err()
        );
        drop(paused);
        assert!(budget.expired());
        tokio::time::timeout(Duration::from_millis(50), budget.exhausted())
            .await
            .unwrap();
    }
}
