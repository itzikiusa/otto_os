//! Per-slot attempts with user retries (architecture-1-04): the turn loop
//! every retryable fan-out slot runs. One attempt normally; a user retry
//! (the engine's retry endpoint kills the slot's session and calls
//! [`RetryQueue::request`]) makes the failed attempt re-run in a fresh
//! session; a transient spawn death gets a bounded automatic respawn; a
//! failed slot may keep listening for a late user retry (see
//! [`WaitForRetry`]). Engines keep the session turn itself (the `attempt`
//! closure) and map each [`SlotState`] onto their own rows.
//!
//! - [`attempts`] — one slot's loop.
//! - [`run_slots`] — every slot concurrently through [`attempts`], fanned in
//!   in completion order (the retrying counterpart of [`super::run`]'s
//!   reviewer stage).
//! - [`summarize_with_retries`] — the summarizer step with the same loop,
//!   falling back to the deterministic result when it finally fails.

use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{Slot, SlotState, Summary};

/// The user-retry bookkeeping a run shares between its retry endpoint and the
/// slot loops, keyed by retry index ([`Slot::retry_index`], or an engine's own
/// key space for later stages).
#[derive(Debug, Default)]
pub struct RetryQueue {
    requested: HashSet<usize>,
    /// Slots whose final permitted attempt has been admitted (or that have
    /// settled): nothing will consume another retry for them.
    closed: HashSet<usize>,
}

/// Why [`RetryQueue::request`] refused a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryClosed;

impl RetryQueue {
    /// Queue a user retry for `key`; refused once the slot is closed.
    pub fn request(&mut self, key: usize) -> Result<(), RetryClosed> {
        if self.closed.contains(&key) {
            return Err(RetryClosed);
        }
        self.requested.insert(key);
        Ok(())
    }

    /// Consume a queued retry for `key` (true when there was one).
    pub fn take(&mut self, key: usize) -> bool {
        self.requested.remove(&key)
    }

    /// No further retry for `key` will be served; drops any queued one.
    pub fn close(&mut self, key: usize) {
        self.closed.insert(key);
        self.requested.remove(&key);
    }

    pub fn is_closed(&self, key: usize) -> bool {
        self.closed.contains(&key)
    }

    pub fn is_requested(&self, key: usize) -> bool {
        self.requested.contains(&key)
    }

    /// True when no retry is queued for any slot.
    pub fn nothing_requested(&self) -> bool {
        self.requested.is_empty()
    }
}

/// A run's retry queue, shared by its endpoint and its slot loops.
pub type Retries = Arc<Mutex<RetryQueue>>;

/// A synchronous state callback, called inline from the slot's own task so
/// it is ordered with the engine's other writes for that slot (a session's
/// `on_ready`, for one).
pub type OnState = Arc<dyn Fn(Slot, SlotState) + Send + Sync>;

/// What a failed slot does after reporting [`SlotState::Failed`].
#[derive(Clone)]
pub enum WaitForRetry {
    /// Give up at once.
    Never,
    /// Keep listening for a user retry until the run is cancelled; a cancel
    /// ends the slot [`SlotState::Cancelled`].
    UntilCancelled,
    /// Keep listening while the predicate holds (e.g. while any peer slot is
    /// still moving, so a stage barrier never hangs on a slot nobody can
    /// un-stick) or until the run is cancelled.
    WhileOpen(Arc<dyn Fn(Slot) -> bool + Send + Sync>),
}

/// How a slot's attempts are driven. Cheap to clone (every field is shared).
#[derive(Clone)]
pub struct SlotControl {
    pub retries: Retries,
    pub cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
    /// User retries one slot may consume.
    pub max_user_retries: u32,
    /// Automatic respawns after an error `transient` accepts.
    pub max_auto_retries: u32,
    pub transient: fn(&str) -> bool,
    pub wait: WaitForRetry,
    /// How often a waiting slot polls the queue and the cancel flag.
    pub poll: Duration,
    /// The slot's key in `retries` ([`Slot::retry_index`] unless an engine
    /// keys a later stage in its own space).
    pub key: Arc<dyn Fn(Slot) -> usize + Send + Sync>,
    pub on_state: OnState,
}

impl SlotControl {
    /// One user retry budget, no automatic respawn, no waiting: override the
    /// fields an engine needs.
    pub fn new(
        retries: Retries,
        cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
        max_user_retries: u32,
        on_state: OnState,
    ) -> Self {
        Self {
            retries,
            cancelled,
            max_user_retries,
            max_auto_retries: 0,
            transient: |_| false,
            wait: WaitForRetry::Never,
            poll: Duration::from_millis(250),
            key: Arc::new(Slot::retry_index),
            on_state,
        }
    }

    fn take(&self, key: usize) -> bool {
        self.retries.lock().unwrap().take(key)
    }

    fn close(&self, key: usize) {
        self.retries.lock().unwrap().close(key);
    }
}

/// How a slot ended without a result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotFailure {
    /// The last attempt's error, or `"cancelled"` when the run was cancelled
    /// before an attempt or while the slot waited for a retry.
    pub error: String,
    /// The run was cancelled (the error is an expected consequence of it).
    pub cancelled: bool,
}

impl SlotFailure {
    fn cancelled(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            cancelled: true,
        }
    }
}

/// Run one slot: `attempt` once, again after each user retry the queue holds
/// for the slot's key (up to `max_user_retries`) and after each transient error (up to
/// `max_auto_retries`). States: [`SlotState::Done`] on success,
/// [`SlotState::Retrying`] before every re-run (`Some(error)` for an
/// automatic one), [`SlotState::Cancelled`] when the run was cancelled under
/// a failed attempt (or under a waiting slot), [`SlotState::Failed`] when it
/// gave up. The slot's key is closed when this returns.
pub async fn attempts<R, A, Fut>(
    slot: Slot,
    ctl: &SlotControl,
    mut attempt: A,
) -> Result<R, SlotFailure>
where
    A: FnMut() -> Fut,
    Fut: Future<Output = Result<R, String>>,
{
    let key = (ctl.key)(slot);
    let (mut user, mut auto) = (0u32, 0u32);
    let out = loop {
        if user >= ctl.max_user_retries {
            ctl.close(key);
        }
        if (ctl.cancelled)() {
            break Err(SlotFailure::cancelled("cancelled"));
        }
        let res = attempt().await;
        let want_retry = ctl.take(key);
        let error = match res {
            Ok(value) => {
                (ctl.on_state)(slot, SlotState::Done);
                break Ok(value);
            }
            Err(e) => e,
        };
        if (ctl.cancelled)() {
            (ctl.on_state)(slot, SlotState::Cancelled);
            break Err(SlotFailure::cancelled(error));
        }
        if want_retry && user < ctl.max_user_retries {
            user += 1;
            (ctl.on_state)(slot, SlotState::Retrying(None));
            continue;
        }
        if auto < ctl.max_auto_retries && (ctl.transient)(&error) {
            auto += 1;
            (ctl.on_state)(slot, SlotState::Retrying(Some(error)));
            continue;
        }
        (ctl.on_state)(slot, SlotState::Failed(error.clone()));
        match wait_for_retry(slot, key, ctl, user).await {
            Waited::Retry => {
                user += 1;
                (ctl.on_state)(slot, SlotState::Retrying(None));
            }
            Waited::Cancelled => {
                (ctl.on_state)(slot, SlotState::Cancelled);
                break Err(SlotFailure::cancelled("cancelled"));
            }
            Waited::GaveUp => {
                break Err(SlotFailure {
                    error,
                    cancelled: false,
                })
            }
        }
    };
    ctl.close(key);
    out
}

enum Waited {
    Retry,
    Cancelled,
    GaveUp,
}

async fn wait_for_retry(slot: Slot, key: usize, ctl: &SlotControl, used: u32) -> Waited {
    if used >= ctl.max_user_retries {
        return Waited::GaveUp;
    }
    loop {
        let open = match &ctl.wait {
            WaitForRetry::Never => return Waited::GaveUp,
            WaitForRetry::UntilCancelled => true,
            WaitForRetry::WhileOpen(open) => open(slot),
        };
        if (ctl.cancelled)() {
            return Waited::Cancelled;
        }
        if ctl.take(key) {
            return Waited::Retry;
        }
        if !open {
            return Waited::GaveUp;
        }
        tokio::time::sleep(ctl.poll).await;
    }
}

/// Every slot concurrently through [`attempts`] (slot `i` is
/// [`Slot::Reviewer`]`(i)`), each on its own task. Returns the successful results in COMPLETION order,
/// or `None` when the run was cancelled. A panicking slot is reported
/// [`SlotState::Failed`] and contributes nothing.
pub async fn run_slots<R, A, Fut>(slots: Vec<A>, ctl: &SlotControl) -> Option<Vec<(usize, R)>>
where
    R: Send + 'static,
    A: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<R, String>> + Send + 'static,
{
    let mut set = tokio::task::JoinSet::new();
    let mut ids = std::collections::HashMap::new();
    for (index, attempt) in slots.into_iter().enumerate() {
        let ctl = ctl.clone();
        let slot = Slot::Reviewer(index);
        let handle = set.spawn(async move {
            let out = attempts(slot, &ctl, attempt).await;
            (index, out)
        });
        ids.insert(handle.id(), index);
    }
    let mut done = Vec::new();
    while let Some(joined) = set.join_next_with_id().await {
        match joined {
            Ok((_, (index, Ok(result)))) => done.push((index, result)),
            Ok((_, (_, Err(_)))) => {}
            Err(e) => {
                if let Some(&index) = ids.get(&e.id()) {
                    ctl.close((ctl.key)(Slot::Reviewer(index)));
                    (ctl.on_state)(
                        Slot::Reviewer(index),
                        SlotState::Failed(format!("agent task failed: {e}")),
                    );
                }
            }
        }
    }
    if (ctl.cancelled)() {
        return None;
    }
    Some(done)
}

/// The summarizer step through [`attempts`]:
/// when the managed summarizer finally fails, `fallback` builds the
/// deterministic result from the same inputs. The flag is true when the run
/// was cancelled under the summarizer (the value is then the fallback too).
pub async fn summarize_with_retries<R, S, A, Fut>(
    results: &[(usize, R)],
    ctl: &SlotControl,
    mut summarizer: A,
    fallback: impl FnOnce(&[(usize, R)]) -> S,
) -> (Summary<S>, bool)
where
    A: FnMut(&[(usize, R)]) -> Fut,
    Fut: Future<Output = Result<S, String>>,
{
    (ctl.on_state)(Slot::Summarizer, SlotState::Running);
    match attempts(Slot::Summarizer, ctl, || summarizer(results)).await {
        Ok(value) => (Summary { value, error: None }, false),
        Err(failure) => (
            Summary {
                value: fallback(results),
                error: Some(failure.error),
            },
            failure.cancelled,
        ),
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
