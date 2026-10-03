//! Runtime-lag gate (perf GAPS §10 D-7): a background path must never block a
//! tokio worker. Each case runs on a ONE-worker multi-thread runtime next to a
//! 1 ms ticker task; any synchronous stretch on the worker shows up as ticker
//! lag. One harness guards the whole "blocking I/O / CPU on a runtime worker"
//! class (PATTERNS P12) — add a case for every new heavy background path.
//!
//! Budget: `OTTO_RUNTIME_LAG_BUDGET_MS` (default 20 ms — the plan's 10 ms plus
//! headroom for a loaded CI box; the negative control below blocks 120 ms, so
//! the gate is never vacuous).

use std::future::Future;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::event::Event;
use otto_server::transcript_cache::{CacheKey, Snapshot, TranscriptCache};
use otto_transcript::{FoldOpts, Provider};
use tokio::sync::broadcast;

fn budget() -> Duration {
    let ms = std::env::var("OTTO_RUNTIME_LAG_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);
    Duration::from_millis(ms)
}

/// One worker (so a blocked worker starves the ticker) + a blocking pool.
fn one_worker_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(4)
        .enable_all()
        .build()
        .expect("runtime")
}

/// Run `fut` as a task on the single worker while a 1 ms ticker measures how
/// late it gets scheduled. Returns the output and the worst lag seen.
fn max_lag_while<F>(fut: F) -> (F::Output, Duration)
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let rt = one_worker_runtime();
    rt.block_on(async move {
        let stop = Arc::new(AtomicBool::new(false));
        let worst_us = Arc::new(AtomicU64::new(0));
        let ticker = {
            let (stop, worst_us) = (stop.clone(), worst_us.clone());
            tokio::spawn(async move {
                let mut iv = tokio::time::interval(Duration::from_millis(1));
                iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                iv.tick().await;
                let mut last = Instant::now();
                while !stop.load(Ordering::Relaxed) {
                    iv.tick().await;
                    let now = Instant::now();
                    let lag = now
                        .duration_since(last)
                        .saturating_sub(Duration::from_millis(1));
                    worst_us.fetch_max(lag.as_micros() as u64, Ordering::Relaxed);
                    last = now;
                }
            })
        };
        // Let the ticker settle before the work starts.
        tokio::time::sleep(Duration::from_millis(20)).await;
        worst_us.store(0, Ordering::Relaxed);
        let out = tokio::spawn(fut).await.expect("case task");
        tokio::time::sleep(Duration::from_millis(5)).await;
        stop.store(true, Ordering::Relaxed);
        let _ = ticker.await;
        (out, Duration::from_micros(worst_us.load(Ordering::Relaxed)))
    })
}

#[test]
fn negative_control_a_blocking_step_is_caught() {
    let ((), lag) = max_lag_while(async {
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(120)); // the P12 bug, on purpose
        tokio::task::yield_now().await;
    });
    assert!(
        lag >= Duration::from_millis(60),
        "the harness must see a blocked worker (saw {lag:?})"
    );
}

/// A synthetic Claude transcript of roughly `mb` megabytes.
fn synthetic_jsonl(dir: &std::path::Path, mb: usize) -> std::path::PathBuf {
    let path = dir.join("session.jsonl");
    let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
    let body = "x".repeat(4000);
    let mut written = 0usize;
    let mut i = 0u64;
    while written < mb * 1024 * 1024 {
        let (kind, role) = if i.is_multiple_of(2) {
            ("user", "user")
        } else {
            ("assistant", "assistant")
        };
        let line = serde_json::json!({
            "type": kind,
            "uuid": format!("u{i}"),
            "parentUuid": if i == 0 { serde_json::Value::Null } else { format!("u{}", i - 1).into() },
            "timestamp": "2026-09-27T10:00:00.000Z",
            "message": { "role": role, "content": [{ "type": "text", "text": format!("{i} {body}") }] },
        })
        .to_string();
        writeln!(f, "{line}").unwrap();
        written += line.len() + 1;
        i += 1;
    }
    f.flush().unwrap();
    path
}

#[test]
fn transcript_fold_of_a_20mb_file_never_blocks_a_worker() {
    let dir = tempfile::tempdir().unwrap();
    let path = synthetic_jsonl(dir.path(), 20);
    let key = CacheKey {
        root: dir.path().to_path_buf(),
        path: path.clone(),
        provider: Provider::Claude,
        sub: None,
    };
    let (turns, lag) = max_lag_while(async move {
        let cache = TranscriptCache::default();
        let snap = cache
            .get(key, move || {
                let folded = otto_transcript::fold_file(
                    Provider::Claude,
                    &path,
                    FoldOpts {
                        images: None,
                        price: None,
                        subagents: Vec::new(),
                    },
                )
                .map_err(|e| otto_core::Error::Internal(e.to_string()))?;
                Ok(Snapshot {
                    folded,
                    subagents: Vec::new(),
                })
            })
            .await
            .expect("fold");
        snap.folded.turns.len()
    });
    eprintln!("transcript fold (20 MB): worst worker lag {lag:?}");
    assert!(turns > 0, "the fold produced turns");
    assert!(
        lag < budget(),
        "transcript fold stalled a worker for {lag:?} (budget {:?})",
        budget()
    );
}

#[test]
fn events_socket_serialization_of_big_frames_stays_under_budget() {
    // Built before the ticker starts: the gate measures the fan-out
    // serialization path, not this test's own 12.8 MB of payload copying.
    let events: Vec<Event> = (0..200)
        .map(|i| Event::Notice {
            level: "info".into(),
            title: format!("t{i}"),
            body: "y".repeat(64 * 1024),
        })
        .collect();
    let (lag_items, lag) = max_lag_while(async move {
        let (bus, _keep) = broadcast::channel::<Event>(1024);
        let mut rx = otto_server::ws_fanout::subscribe(&bus);
        for ev in events {
            bus.send(ev).unwrap();
            // Producers emit one event at a time and yield in between.
            tokio::task::yield_now().await;
        }
        let mut bytes = 0usize;
        for _ in 0..200 {
            match rx.recv().await.expect("frame") {
                otto_server::ws_fanout::FanItem::Event(f) => bytes += f.text().expect("text").len(),
                otto_server::ws_fanout::FanItem::Lagged(n) => panic!("lagged {n}"),
            }
            // A socket yields between frames (its sink send is async).
            tokio::task::yield_now().await;
        }
        bytes
    });
    eprintln!("events serialization (200 × 64 KB): worst worker lag {lag:?}");
    assert!(lag_items >= 200 * 64 * 1024);
    assert!(
        lag < budget(),
        "events serialization stalled a worker for {lag:?}"
    );
}

/// The burst variant (r3-10-02): the paced case above yields after every send
/// AND every frame, so its consumer never sees a backlog and the budget can
/// barely fail. Here all 200 frames are queued at once — a flood of trail /
/// status events from one producer — and the socket loop is modelled like
/// `ws_events`: `recv` → `text()` → a sink write that completes without
/// yielding (a fast loopback client), paced by the socket loop's own
/// `ws_fanout::Pacer`. Without the pacer this measured ~208 ms of blocked
/// worker (debug build); if a change drops or loosens it, this fails.
/// Payloads are still built before the ticker starts (test-side copies are
/// not product cost).
///
/// The lag is the single worst tick gap, so one OS preemption on a shared CI
/// runner can exceed the budget by itself. A real regression (no pacer)
/// blocks EVERY burst for ~200 ms; noise doesn't hit three in a row — so the
/// gate is the best of three bursts.
#[test]
fn events_fanout_of_a_queued_burst_stays_under_budget() {
    let lag = (0..3)
        .map(|_| queued_burst_lag())
        .min()
        .expect("three bursts");
    eprintln!("events burst (200 × 64 KB queued): best-of-3 worst worker lag {lag:?}");
    assert!(
        lag < budget(),
        "an events burst stalled a worker for {lag:?} (best of 3)"
    );
}

/// One queued burst; returns its worst worker lag.
fn queued_burst_lag() -> Duration {
    let events: Vec<Event> = (0..200)
        .map(|i| Event::Notice {
            level: "info".into(),
            title: format!("t{i}"),
            body: "y".repeat(64 * 1024),
        })
        .collect();
    let (lag_items, lag) = max_lag_while(async move {
        let (bus, _keep) = broadcast::channel::<Event>(1024);
        let mut rx = otto_server::ws_fanout::subscribe(&bus);
        for ev in events {
            bus.send(ev).unwrap();
        }
        let mut bytes = 0usize;
        let mut pacer = otto_server::ws_fanout::Pacer::new();
        for _ in 0..200 {
            match rx.recv().await.expect("frame") {
                otto_server::ws_fanout::FanItem::Event(f) => {
                    let text = f.text().expect("text");
                    // A sink write that is immediately ready (no yield).
                    std::future::ready(()).await;
                    bytes += text.len();
                    pacer.sent(text.len()).await;
                }
                otto_server::ws_fanout::FanItem::Lagged(n) => panic!("lagged {n}"),
            }
        }
        bytes
    });
    assert!(lag_items >= 200 * 64 * 1024);
    lag
}
