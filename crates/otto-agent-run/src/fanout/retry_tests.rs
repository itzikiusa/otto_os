//! The retrying fan-out: a user retry of ONE slot re-runs only that slot, a
//! failed writer never stops the summarizer, a failed summarizer yields the
//! deterministic fallback, and the queue refuses retries for closed slots.

use super::*;
use crate::fanout::SUMMARIZER_RETRY_IDX;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

type Log = Arc<Mutex<Vec<(Slot, SlotState)>>>;
type BoxFut = std::pin::Pin<Box<dyn Future<Output = Result<u32, String>> + Send>>;

fn control(retries: &Retries, cancel: &Arc<AtomicBool>, log: &Log) -> SlotControl {
    let cancel = Arc::clone(cancel);
    let log = Arc::clone(log);
    let mut ctl = SlotControl::new(
        Arc::clone(retries),
        Arc::new(move || cancel.load(Ordering::SeqCst)),
        2,
        Arc::new(move |slot: Slot, state: SlotState| log.lock().unwrap().push((slot, state))),
    );
    ctl.poll = Duration::from_millis(5);
    ctl
}

fn states(log: &Log, slot: Slot) -> Vec<SlotState> {
    log.lock()
        .unwrap()
        .iter()
        .filter(|(s, _)| *s == slot)
        .map(|(_, st)| st.clone())
        .collect()
}

#[tokio::test]
async fn a_user_retry_of_one_writer_reruns_only_that_slot() {
    let retries = Retries::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let log = Log::default();
    let mut ctl = control(&retries, &cancel, &log);
    // Slot 1 fails once and keeps listening while slot 0 still runs; the
    // user's retry (queued while it waits) re-runs it.
    let open = Arc::new(AtomicBool::new(true));
    let open_in = Arc::clone(&open);
    ctl.wait = WaitForRetry::WhileOpen(Arc::new(move |_: Slot| open_in.load(Ordering::SeqCst)));
    let calls: Vec<Arc<AtomicU32>> = (0..2).map(|_| Arc::new(AtomicU32::new(0))).collect();
    let (c0, c1) = (Arc::clone(&calls[0]), Arc::clone(&calls[1]));
    let (r_in, open_in) = (Arc::clone(&retries), Arc::clone(&open));
    let slot0 = move || {
        let (c0, r_in, open_in) = (Arc::clone(&c0), Arc::clone(&r_in), Arc::clone(&open_in));
        async move {
            c0.fetch_add(1, Ordering::SeqCst);
            // Give slot 1 time to fail, then the user retries it.
            tokio::time::sleep(Duration::from_millis(30)).await;
            r_in.lock().unwrap().request(1).expect("slot 1 still open");
            tokio::time::sleep(Duration::from_millis(60)).await;
            open_in.store(false, Ordering::SeqCst);
            Ok::<u32, String>(10)
        }
    };
    let slot1 = move || {
        let c1 = Arc::clone(&c1);
        async move {
            match c1.fetch_add(1, Ordering::SeqCst) {
                0 => Err("session vanished".to_string()),
                _ => Ok(20),
            }
        }
    };
    type Boxed = Box<dyn FnMut() -> BoxFut + Send>;
    let s0 = slot0;
    let s1 = slot1;
    let slots: Vec<Boxed> = vec![
        Box::new(move || -> BoxFut { Box::pin(s0()) }),
        Box::new(move || -> BoxFut { Box::pin(s1()) }),
    ];
    let mut done = run_slots(slots, &ctl).await.expect("not cancelled");
    done.sort_unstable();
    assert_eq!(done, vec![(0, 10), (1, 20)]);
    assert_eq!(
        calls[0].load(Ordering::SeqCst),
        1,
        "the healthy slot never re-ran"
    );
    assert_eq!(
        calls[1].load(Ordering::SeqCst),
        2,
        "the retried slot re-ran once"
    );
    assert_eq!(
        states(&log, Slot::Reviewer(1)),
        vec![
            SlotState::Failed("session vanished".into()),
            SlotState::Retrying(None),
            SlotState::Done
        ]
    );
    assert_eq!(states(&log, Slot::Reviewer(0)), vec![SlotState::Done]);
    let q = retries.lock().unwrap();
    assert!(q.is_closed(0) && q.is_closed(1), "settled slots are closed");
}

#[tokio::test]
async fn a_failed_writer_gives_up_when_its_stage_closes_and_the_summarizer_still_runs() {
    let retries = Retries::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let log = Log::default();
    let mut ctl = control(&retries, &cancel, &log);
    ctl.wait = WaitForRetry::WhileOpen(Arc::new(|_: Slot| false));
    type Ready = std::future::Ready<Result<u32, String>>;
    let slots: Vec<Box<dyn FnMut() -> Ready + Send>> = vec![
        Box::new(|| -> Ready { std::future::ready(Ok(3)) }),
        Box::new(|| -> Ready { std::future::ready(Err("auth".to_string())) }),
    ];
    let done = run_slots(slots, &ctl).await.expect("not cancelled");
    assert_eq!(done, vec![(0, 3)], "the failed writer contributes nothing");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_in = Arc::clone(&seen);
    let (summary, cancelled) = summarize_with_retries(
        &done,
        &ctl,
        move |r: &[(usize, u32)]| {
            seen_in.lock().unwrap().extend(r.iter().map(|(i, _)| *i));
            std::future::ready(Ok::<u32, String>(r.iter().map(|(_, v)| v).sum()))
        },
        |_: &[(usize, u32)]| 0,
    )
    .await;
    assert!(!cancelled);
    assert_eq!(summary.value, 3);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![0],
        "only the surviving writer's result"
    );
    assert_eq!(
        states(&log, Slot::Reviewer(1)),
        vec![SlotState::Failed("auth".into())]
    );
}

#[tokio::test]
async fn a_failed_summarizer_yields_the_deterministic_fallback() {
    let retries = Retries::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let log = Log::default();
    let ctl = control(&retries, &cancel, &log);
    let results = vec![(0usize, 2u32), (1, 5)];
    let (summary, cancelled) = summarize_with_retries(
        &results,
        &ctl,
        |_: &[(usize, u32)]| std::future::ready(Err::<u32, _>("rate limited".to_string())),
        |r: &[(usize, u32)]| r.iter().map(|(_, v)| v).sum(),
    )
    .await;
    assert!(!cancelled);
    assert_eq!(summary.value, 7, "the fallback merged the writer results");
    assert_eq!(summary.error.as_deref(), Some("rate limited"));
    assert_eq!(
        states(&log, Slot::Summarizer),
        vec![SlotState::Running, SlotState::Failed("rate limited".into())]
    );
}

#[tokio::test]
async fn a_summarizer_retry_reruns_it_and_transient_errors_respawn_within_budget() {
    let retries = Retries::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let log = Log::default();
    let mut ctl = control(&retries, &cancel, &log);
    ctl.max_auto_retries = 1;
    ctl.transient = |e: &str| e.contains("vanished");
    let calls = Arc::new(AtomicU32::new(0));
    let (calls_in, r_in) = (Arc::clone(&calls), Arc::clone(&retries));
    let (summary, _) = summarize_with_retries(
        &[(0usize, 1u32)],
        &ctl,
        move |_: &[(usize, u32)]| {
            let n = calls_in.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // The user's retry lands while the first attempt runs.
                r_in.lock().unwrap().request(SUMMARIZER_RETRY_IDX).unwrap();
            }
            std::future::ready(match n {
                0 => Err("killed for retry".to_string()),
                1 => Err("session vanished".to_string()),
                _ => Ok(42u32),
            })
        },
        |_: &[(usize, u32)]| 0,
    )
    .await;
    assert_eq!(summary.value, 42);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        states(&log, Slot::Summarizer),
        vec![
            SlotState::Running,
            SlotState::Retrying(None),
            SlotState::Retrying(Some("session vanished".into())),
            SlotState::Done
        ]
    );
    assert_eq!(
        retries.lock().unwrap().request(SUMMARIZER_RETRY_IDX),
        Err(RetryClosed),
        "a settled slot refuses further retries"
    );
}

#[tokio::test]
async fn user_retries_are_capped_and_a_cancel_ends_a_waiting_slot() {
    let retries = Retries::default();
    let cancel = Arc::new(AtomicBool::new(false));
    let log = Log::default();
    let mut ctl = control(&retries, &cancel, &log);
    ctl.max_user_retries = 1;
    // Budget: the one retry is used, then the slot gives up without waiting.
    retries.lock().unwrap().request(4).unwrap();
    let out = attempts(Slot::Reviewer(4), &ctl, || {
        std::future::ready(Err::<u32, _>("boom".to_string()))
    })
    .await;
    assert_eq!(
        out,
        Err(SlotFailure {
            error: "boom".into(),
            cancelled: false
        })
    );
    // A slot waiting until cancel ends Cancelled.
    ctl.max_user_retries = 3;
    ctl.wait = WaitForRetry::UntilCancelled;
    let cancel_in = Arc::clone(&cancel);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel_in.store(true, Ordering::SeqCst);
    });
    let out = attempts(Slot::Reviewer(7), &ctl, || {
        std::future::ready(Err::<u32, _>("boom".to_string()))
    })
    .await;
    assert!(out.unwrap_err().cancelled);
    assert_eq!(
        states(&log, Slot::Reviewer(7)),
        vec![SlotState::Failed("boom".into()), SlotState::Cancelled]
    );
}
