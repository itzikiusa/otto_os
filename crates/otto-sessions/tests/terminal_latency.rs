//! The keystroke path must not wait on SQLite.
//!
//! Regression guard for the "1–2 s typing lag with no CPU load" report: the
//! terminal WebSocket used to run `manager.get()` + a re-auth check ahead of
//! EVERY client frame, on the socket's own `select!` loop — so any slow
//! statement elsewhere in the daemon (the work-graph reconcile's 1.2–2.5 s
//! artifact read-back, say) froze the terminal in both directions. The re-auth
//! now lives in its own task behind a `watch` channel, and the input arm writes
//! straight to the PTY.
//!
//! This test pins the property that makes that safe: with EVERY `SessionsRepo::
//! get` artificially costing 2 s (`OTTO_TEST_DB_SLEEP_MS`, the test-only hook in
//! `otto-state`), a keystroke still reaches the child's tty promptly — while a
//! concurrent `get` (what the re-auth pass does) is parked on the DB.
//!
//! It spawns a REAL shell PTY, so — like `broadcast_e2e` — it is gated behind an
//! env var to keep a plain `cargo test` hermetic and fast:
//!
//!     OTTO_LATENCY_E2E=1 cargo test -p otto-sessions --test terminal_latency -- --nocapture
//!
//! The ungated half (`db_sleep_hook_only_touches_get`) always runs: it proves
//! the injection hook is wired to `get` and that `input`'s own code path never
//! touches the repo.

use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, Workspace};
use otto_core::{new_id, Id};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::SessionsRepo;
use tokio::sync::broadcast;

/// Every `SessionsRepo::get` in this process sleeps this long. Read ONCE (a
/// `OnceLock` in `otto-state`), so it must be set before the first `get` — i.e.
/// before the manager is built.
const DB_SLEEP_MS: u64 = 2000;
/// The budget a keystroke gets. Generously above a PTY write (microseconds) and
/// far below the injected DB cost — the assertion is "does NOT wait on SQLite",
/// not a microbenchmark.
const INPUT_BUDGET: Duration = Duration::from_millis(200);
/// Sentinel typed into the shell; the tty echoes it back into the scrollback.
const SENTINEL: &str = "OTTO_LAT_OK";

fn arm_db_sleep() {
    std::env::set_var("OTTO_TEST_DB_SLEEP_MS", DB_SLEEP_MS.to_string());
}

/// A `SessionManager` over a migrated temp SQLite with one workspace + user.
/// Mirrors the `broadcast_e2e` harness.
async fn manager() -> (Arc<SessionManager>, Workspace, Id) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let pool = otto_state::open(&db).await.unwrap();
    std::mem::forget(dir); // keep the db file alive for the test's lifetime

    let user = new_id();
    let ws_id = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
        .bind(&user).bind("u").bind("x").bind("U").bind(&now)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&ws_id)
        .bind("w")
        .bind("/tmp")
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

    let repo = SessionsRepo::new(pool);
    let (events, _rx) = broadcast::channel(64);
    let providers = ProviderRegistry::new(None);
    let mgr = Arc::new(SessionManager::new(repo, events, providers));
    let ws = Workspace {
        id: ws_id,
        name: "w".into(),
        root_path: "/tmp".into(),
        settings: serde_json::json!({}),
        archived: false,
        created_at: chrono::Utc::now(),
    };
    (mgr, ws, user)
}

/// The hook is on `get` and only on `get` — and `SessionManager::input` reaches
/// the PTY without consulting the repo at all (here there is no live PTY, so it
/// fails fast; the point is that it fails in microseconds, not in `DB_SLEEP_MS`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db_sleep_hook_only_touches_get() {
    arm_db_sleep();
    let (mgr, _ws, _user) = manager().await;

    let missing: Id = new_id();
    let started = Instant::now();
    assert!(mgr.input(&missing, b"x").await.is_err(), "no live session");
    let input_elapsed = started.elapsed();

    let started = Instant::now();
    assert!(mgr.get(&missing).await.is_err(), "no such session row");
    let get_elapsed = started.elapsed();

    assert!(
        get_elapsed >= Duration::from_millis(DB_SLEEP_MS),
        "OTTO_TEST_DB_SLEEP_MS is not wired to SessionsRepo::get (get took {get_elapsed:?})"
    );
    assert!(
        input_elapsed < INPUT_BUDGET,
        "input() consulted the state DB: {input_elapsed:?} >= {INPUT_BUDGET:?}"
    );
}

/// A keystroke lands on a real child tty while a re-auth-shaped `get` is parked
/// on the DB for 2 s.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keystroke_reaches_the_pty_while_a_db_read_is_stalled() {
    if std::env::var("OTTO_LATENCY_E2E").is_err() {
        eprintln!("skipping terminal_latency e2e (set OTTO_LATENCY_E2E=1; spawns a real shell)");
        return;
    }
    arm_db_sleep();
    let (mgr, ws, user) = manager().await;

    let cwd = tempfile::tempdir().unwrap();
    let session = mgr
        .create(
            &ws,
            &user,
            CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: None,
                cwd: Some(cwd.path().to_string_lossy().into_owned()),
                connection_id: None,
                model: None,
                meta: None,
            },
            None,
        )
        .await
        .expect("spawn shell");

    // Let the shell draw its prompt so the tty is in its steady (echoing) state.
    let ready = Instant::now() + Duration::from_secs(10);
    while Instant::now() < ready {
        match mgr.live_handle(&session.id) {
            Some(h) if !h.scrollback(4).is_empty() => break,
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    assert!(
        mgr.live_handle(&session.id).is_some(),
        "PTY did not come up"
    );

    // The re-auth pass, mid-flight: a `get` that will sit on SQLite for 2 s.
    let stalled = {
        let mgr = mgr.clone();
        let id = session.id.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let _ = mgr.get(&id).await;
            started.elapsed()
        })
    };
    tokio::time::sleep(Duration::from_millis(50)).await; // make sure it's parked

    // The keystroke. This is the assertion the bug report is about.
    let started = Instant::now();
    mgr.input(&session.id, format!("{SENTINEL}\r").as_bytes())
        .await
        .expect("write to pty");
    let input_elapsed = started.elapsed();
    assert!(
        input_elapsed < INPUT_BUDGET,
        "keystroke waited on the state DB: {input_elapsed:?} >= {INPUT_BUDGET:?}"
    );

    // …and it really reached the child: the tty echoes what was typed.
    let deadline = Instant::now() + Duration::from_secs(10);
    let echoed = loop {
        let screen = mgr
            .live_handle(&session.id)
            .map(|h| String::from_utf8_lossy(&h.scrollback(200)).into_owned())
            .unwrap_or_default();
        if screen.contains(SENTINEL) {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(echoed, "typed bytes never reached the child's tty");

    // Sanity: the concurrent read really was slow — i.e. the keystroke overtook
    // a 2 s statement rather than the injection being inert.
    let get_elapsed = stalled.await.expect("join");
    assert!(
        get_elapsed >= Duration::from_millis(DB_SLEEP_MS),
        "the injected DB stall did not happen ({get_elapsed:?})"
    );

    let _ = mgr.kill_session(&session.id).await;
}
