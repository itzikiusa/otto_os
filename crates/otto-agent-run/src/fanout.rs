//! The fan-out + summarizer shape every multi-agent engine shares
//! (architecture-1-04): N reviewer/writer turns run concurrently with
//! per-agent error isolation, then ONE managed summarizer turn consolidates
//! their results, and a deterministic fallback stands in when the summarizer
//! fails. Engines (skill review, vault docs, code review) keep their prompts,
//! persistence and row shapes; this module owns only the control flow and the
//! state transitions each slot goes through.
//!
//! - [`run`] — the whole pass: fan out, fan in (completion order), stop if the
//!   run was cancelled, then [`summarize`].
//! - [`summarize`] — the summarizer step alone. A user's "retry the
//!   summarizer" re-runs just this over the reviewer results already in hand.
//! - [`SUMMARIZER_RETRY_IDX`] — the retry-queue index standing for the
//!   summarizer in engines that key retries by agent index.
//! - [`retry`] — the retrying flavour for engines whose user can retry ONE
//!   slot mid-run (vault docs): [`attempts`], [`run_slots`],
//!   [`summarize_with_retries`] over a shared [`RetryQueue`]. Its state
//!   callback is synchronous and runs on the slot's own task.

use std::future::Future;

pub mod retry;
pub use retry::{
    attempts, run_slots, summarize_with_retries, OnState, Retries, RetryClosed, RetryQueue,
    SlotControl, SlotFailure, WaitForRetry,
};

/// Retry-queue index standing for the summarizer (reviewers use their own
/// index). `usize::MAX` so it can never collide with a reviewer slot.
pub const SUMMARIZER_RETRY_IDX: usize = usize::MAX;

/// Which agent a state change is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The reviewer at this index (the order the engine passed them in).
    Reviewer(usize),
    Summarizer,
}

impl Slot {
    /// The engine's retry-queue index for this slot.
    pub fn retry_index(self) -> usize {
        match self {
            Slot::Reviewer(i) => i,
            Slot::Summarizer => SUMMARIZER_RETRY_IDX,
        }
    }
}

/// A slot's state as the runner drives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlotState {
    Running,
    Done,
    /// The turn failed (an `Err`, or its task panicked); the message is the
    /// reason. A failed reviewer contributes nothing; a failed summarizer is
    /// replaced by the deterministic fallback.
    Failed(String),
    /// A failed attempt is being re-run ([`retry`] only): `None` for a user
    /// retry, `Some(error)` for an automatic respawn after a transient error.
    Retrying(Option<String>),
    /// The run was cancelled under this slot ([`retry`] only).
    Cancelled,
}

/// What the summarizer step produced.
#[derive(Debug)]
pub struct Summary<S> {
    pub value: S,
    /// `None` when the managed summarizer produced `value`; the summarizer's
    /// error when `value` is the deterministic fallback.
    pub error: Option<String>,
}

/// The whole pass's result.
#[derive(Debug)]
pub struct Outcome<R, S> {
    /// Successful reviewer results with their slot index, in COMPLETION
    /// order (what the engines persisted before the runner existed).
    pub reviewers: Vec<(usize, R)>,
    pub summary: Summary<S>,
}

/// Run every reviewer concurrently, then the summarizer over their results.
///
/// - `reviewers`: one future per slot. Each runs on its own task, so a panic
///   or `Err` in one never affects the others.
/// - `cancelled`: checked once every reviewer has settled; `true` skips the
///   summarizer and returns `None` (the run was stopped).
/// - `summarizer` / `fallback`: see [`summarize`].
/// - `on_state`: awaited for every state change, in order, so an engine can
///   persist and broadcast each one.
pub async fn run<R, S, RF, SF, Sum, Fb, St, StF>(
    reviewers: Vec<RF>,
    cancelled: impl Fn() -> bool,
    summarizer: Sum,
    fallback: Fb,
    mut on_state: St,
) -> Option<Outcome<R, S>>
where
    R: Send + 'static,
    RF: Future<Output = Result<R, String>> + Send + 'static,
    Sum: FnOnce(&[(usize, R)]) -> SF,
    SF: Future<Output = Result<S, String>>,
    Fb: FnOnce(&[(usize, R)]) -> S,
    St: FnMut(Slot, SlotState) -> StF,
    StF: Future<Output = ()>,
{
    let mut set = tokio::task::JoinSet::new();
    let mut slots = std::collections::HashMap::new();
    for (index, fut) in reviewers.into_iter().enumerate() {
        on_state(Slot::Reviewer(index), SlotState::Running).await;
        let handle = set.spawn(async move { (index, fut.await) });
        slots.insert(handle.id(), index);
    }
    let mut done = Vec::new();
    while let Some(joined) = set.join_next_with_id().await {
        match joined {
            Ok((_, (index, Ok(result)))) => {
                on_state(Slot::Reviewer(index), SlotState::Done).await;
                done.push((index, result));
            }
            Ok((_, (index, Err(e)))) => {
                on_state(Slot::Reviewer(index), SlotState::Failed(e)).await;
            }
            Err(e) => {
                if let Some(&index) = slots.get(&e.id()) {
                    let reason = format!("agent task failed: {e}");
                    on_state(Slot::Reviewer(index), SlotState::Failed(reason)).await;
                }
            }
        }
    }
    if cancelled() {
        return None;
    }
    let summary = summarize(&done, summarizer, fallback, on_state).await;
    Some(Outcome {
        reviewers: done,
        summary,
    })
}

/// The summarizer step on its own (also the summarizer-only retry): run the
/// managed `summarizer` turn over `results`; when it fails, `fallback` builds
/// the deterministic result from the same inputs instead.
pub async fn summarize<R, S, SF, St, StF>(
    results: &[(usize, R)],
    summarizer: impl FnOnce(&[(usize, R)]) -> SF,
    fallback: impl FnOnce(&[(usize, R)]) -> S,
    mut on_state: St,
) -> Summary<S>
where
    SF: Future<Output = Result<S, String>>,
    St: FnMut(Slot, SlotState) -> StF,
    StF: Future<Output = ()>,
{
    on_state(Slot::Summarizer, SlotState::Running).await;
    match summarizer(results).await {
        Ok(value) => {
            on_state(Slot::Summarizer, SlotState::Done).await;
            Summary { value, error: None }
        }
        Err(e) => {
            on_state(Slot::Summarizer, SlotState::Failed(e.clone())).await;
            Summary {
                value: fallback(results),
                error: Some(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<(Slot, SlotState)>>>;

    fn recorder(log: &Log) -> impl FnMut(Slot, SlotState) -> std::future::Ready<()> {
        let log = Arc::clone(log);
        move |slot, state| {
            log.lock().unwrap().push((slot, state));
            std::future::ready(())
        }
    }

    type BoxedReviewer = std::pin::Pin<Box<dyn Future<Output = Result<u32, String>> + Send>>;

    async fn reviewer(value: Result<u32, &'static str>, delay_ms: u64) -> Result<u32, String> {
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        value.map_err(str::to_owned)
    }

    fn sum(results: &[(usize, u32)]) -> u32 {
        results.iter().map(|(_, v)| v).sum()
    }

    fn states(log: &Log, slot: Slot) -> Vec<SlotState> {
        let log = log.lock().unwrap();
        log.iter()
            .filter(|(s, _)| *s == slot)
            .map(|(_, st)| st.clone())
            .collect()
    }

    #[tokio::test]
    async fn all_reviewers_ok_feed_the_summarizer_in_completion_order() {
        let log = Log::default();
        let out = run(
            vec![reviewer(Ok(1), 40), reviewer(Ok(2), 0), reviewer(Ok(4), 20)],
            || false,
            |r: &[(usize, u32)]| std::future::ready(Ok::<_, String>(sum(r) * 10)),
            |_: &[(usize, u32)]| 0,
            recorder(&log),
        )
        .await
        .expect("not cancelled");
        let order: Vec<usize> = out.reviewers.iter().map(|(i, _)| *i).collect();
        assert_eq!(order, vec![1, 2, 0], "completion order, not slot order");
        assert_eq!(out.summary.value, 70);
        assert!(out.summary.error.is_none());
        for i in 0..3 {
            assert_eq!(
                states(&log, Slot::Reviewer(i)),
                vec![SlotState::Running, SlotState::Done]
            );
        }
        assert_eq!(
            states(&log, Slot::Summarizer),
            vec![SlotState::Running, SlotState::Done]
        );
    }

    #[tokio::test]
    async fn one_failed_reviewer_is_isolated_and_reported() {
        let log = Log::default();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_in = Arc::clone(&seen);
        let out = run(
            vec![
                reviewer(Ok(1), 0),
                reviewer(Err("auth"), 0),
                reviewer(Ok(4), 0),
            ],
            || false,
            move |r: &[(usize, u32)]| {
                seen_in.lock().unwrap().extend(r.iter().map(|(i, _)| *i));
                std::future::ready(Ok::<_, String>(sum(r)))
            },
            |_: &[(usize, u32)]| 0,
            recorder(&log),
        )
        .await
        .unwrap();
        assert_eq!(
            out.summary.value, 5,
            "the failed reviewer contributes nothing"
        );
        let mut seen = seen.lock().unwrap().clone();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 2]);
        assert_eq!(
            states(&log, Slot::Reviewer(1)),
            vec![SlotState::Running, SlotState::Failed("auth".into())]
        );
    }

    #[tokio::test]
    async fn a_panicking_reviewer_fails_only_its_own_slot() {
        let log = Log::default();
        let boom = async {
            if true {
                panic!("reviewer crashed");
            }
            Ok::<u32, String>(0)
        };
        let reviewers: Vec<BoxedReviewer> = vec![Box::pin(reviewer(Ok(3), 0)), Box::pin(boom)];
        let out = run(
            reviewers,
            || false,
            |r: &[(usize, u32)]| std::future::ready(Ok::<_, String>(sum(r))),
            |_: &[(usize, u32)]| 0,
            recorder(&log),
        )
        .await
        .unwrap();
        assert_eq!(out.summary.value, 3);
        assert!(matches!(
            states(&log, Slot::Reviewer(1)).as_slice(),
            [SlotState::Running, SlotState::Failed(_)]
        ));
    }

    #[tokio::test]
    async fn a_failed_summarizer_falls_back_to_the_deterministic_result() {
        let log = Log::default();
        let out = run(
            vec![reviewer(Ok(2), 0), reviewer(Ok(3), 0)],
            || false,
            |_: &[(usize, u32)]| std::future::ready(Err::<u32, _>("rate limited".to_string())),
            sum,
            recorder(&log),
        )
        .await
        .unwrap();
        assert_eq!(
            out.summary.value, 5,
            "the fallback merged the reviewer results"
        );
        assert_eq!(out.summary.error.as_deref(), Some("rate limited"));
        assert_eq!(
            states(&log, Slot::Summarizer),
            vec![SlotState::Running, SlotState::Failed("rate limited".into())]
        );
    }

    #[tokio::test]
    async fn a_cancelled_run_never_starts_the_summarizer() {
        let log = Log::default();
        let out = run(
            vec![reviewer(Ok(1), 0)],
            || true,
            |_: &[(usize, u32)]| -> std::future::Ready<Result<u32, String>> {
                panic!("summarizer must not run")
            },
            |_: &[(usize, u32)]| 0u32,
            recorder(&log),
        )
        .await;
        assert!(out.is_none());
        assert!(states(&log, Slot::Summarizer).is_empty());
    }

    #[tokio::test]
    async fn summarizer_only_retry_reuses_the_reviewer_results() {
        // First pass: the summarizer fails, the fallback stands in.
        let log = Log::default();
        let first = run(
            vec![reviewer(Ok(2), 0), reviewer(Ok(5), 0)],
            || false,
            |_: &[(usize, u32)]| std::future::ready(Err::<u32, _>("boom".to_string())),
            sum,
            recorder(&log),
        )
        .await
        .unwrap();
        assert!(first.summary.error.is_some());
        // The retry re-runs ONLY the summarizer, over the same results.
        let retry_log = Log::default();
        let retried = summarize(
            &first.reviewers,
            |r: &[(usize, u32)]| std::future::ready(Ok::<_, String>(sum(r) * 100)),
            |_: &[(usize, u32)]| 0,
            recorder(&retry_log),
        )
        .await;
        assert_eq!(retried.value, 700);
        assert!(retried.error.is_none());
        assert!(
            retry_log
                .lock()
                .unwrap()
                .iter()
                .all(|(slot, _)| *slot == Slot::Summarizer),
            "no reviewer runs again"
        );
        assert_eq!(Slot::Summarizer.retry_index(), SUMMARIZER_RETRY_IDX);
        assert_eq!(Slot::Reviewer(3).retry_index(), 3);
    }
}
