//! End-to-end test for the broadcast feature.
//!
//! Spins up REAL shell PTYs and drives `SessionManager::broadcast_message`
//! exactly as the daemon does, then proves the message was both *delivered* and
//! *submitted* (a shell only writes the marker file if Enter actually landed),
//! and that *targeting* works (untargeted sessions don't run the command).
//!
//! It spawns real interactive shells, so it's gated behind `OTTO_BCAST_E2E=1`
//! to keep normal `cargo test` hermetic. Run it with:
//!
//!     OTTO_BCAST_E2E=1 cargo test -p otto-sessions --test broadcast_e2e -- --nocapture
//!
//! This is the regression guard for "broadcast pasted the text but never sent":
//! writing `"{text}\n"` in one burst makes bracketed-paste TUIs treat the
//! newline as pasted content. `broadcast_message` uses paste + delay + `\r`,
//! which a shell here turns into an executed command.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, Workspace};
use otto_core::{new_id, Id};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::SessionsRepo;
use tokio::sync::broadcast;

fn gated() -> bool {
    if std::env::var("OTTO_BCAST_E2E").is_err() {
        eprintln!("skipping broadcast_e2e (set OTTO_BCAST_E2E=1 to run; spawns real shells)");
        return false;
    }
    true
}

/// Build a SessionManager backed by a migrated temp sqlite, with one workspace
/// and user seeded. Mirrors the in-crate manager test harness.
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
        .bind(&ws_id).bind("w").bind("/tmp").bind(&now)
        .execute(&pool).await.unwrap();

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

/// Spawn a real interactive shell session whose cwd is `cwd`.
async fn spawn_shell(mgr: &SessionManager, ws: &Workspace, user: &Id, cwd: &Path) -> Id {
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("shell".into()),
        title: None,
        cwd: Some(cwd.to_string_lossy().into_owned()),
        connection_id: None,
        meta: None,
    };
    let s = mgr.create(ws, user, req, None).await.expect("spawn shell");
    s.id
}

/// Wait until the shell has drawn its prompt and gone briefly quiet, so its line
/// editor (and bracketed-paste handling) is active before we type.
async fn wait_ready(mgr: &SessionManager, id: &Id) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(h) = mgr.live_handle(id) {
            if !h.scrollback(4).is_empty() && h.last_output_at().elapsed() >= Duration::from_millis(400)
            {
                return;
            }
        }
        if Instant::now() >= deadline {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Poll until `path` exists (the shell executed the broadcast command) or time out.
async fn wait_for_file(path: &Path, within: Duration) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if path.exists() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn broadcast_delivers_submits_and_targets() {
    if !gated() {
        return;
    }
    // Clean shell startup (no heavy user rc); bracketed paste is a ZLE default.
    let zdot = tempfile::tempdir().unwrap();
    std::env::set_var("ZDOTDIR", zdot.path());

    let (mgr, ws, user) = manager().await;

    // Three shells, each in its own cwd so we can tell who ran the command.
    let d0 = tempfile::tempdir().unwrap();
    let d1 = tempfile::tempdir().unwrap();
    let d2 = tempfile::tempdir().unwrap();
    let s0 = spawn_shell(&mgr, &ws, &user, d0.path()).await;
    let s1 = spawn_shell(&mgr, &ws, &user, d1.path()).await;
    let s2 = spawn_shell(&mgr, &ws, &user, d2.path()).await;

    wait_ready(&mgr, &s0).await;
    wait_ready(&mgr, &s1).await;
    wait_ready(&mgr, &s2).await;

    // 1. Targeted broadcast to s0 + s1 only.
    let targets = [s0.clone(), s1.clone()];
    let hit = mgr
        .broadcast_message(&ws.id, "echo OTTO_OK > sent.txt", Some(&targets))
        .await
        .expect("broadcast");
    assert_eq!(hit.len(), 2, "targeted broadcast should hit exactly 2 sessions");
    assert!(hit.contains(&s0) && hit.contains(&s1));

    let f0 = d0.path().join("sent.txt");
    let f1 = d1.path().join("sent.txt");
    let f2 = d2.path().join("sent.txt");
    assert!(
        wait_for_file(&f0, Duration::from_secs(8)).await,
        "s0 should have executed the broadcast (delivered + submitted)"
    );
    assert!(
        wait_for_file(&f1, Duration::from_secs(8)).await,
        "s1 should have executed the broadcast (delivered + submitted)"
    );
    // Give the untargeted shell the same window; it must NOT have run anything.
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!f2.exists(), "s2 was not targeted and must not run the command");

    // 2. Broadcast to ALL (targets = None) reaches the previously-untargeted s2.
    let hit_all = mgr
        .broadcast_message(&ws.id, "echo OTTO_ALL > all.txt", None)
        .await
        .expect("broadcast all");
    assert_eq!(hit_all.len(), 3, "broadcast-to-all should hit every live agent");

    let a2 = d2.path().join("all.txt");
    assert!(
        wait_for_file(&a2, Duration::from_secs(8)).await,
        "s2 should receive + execute the broadcast-to-all"
    );

    // Cleanup live PTYs.
    for id in [&s0, &s1, &s2] {
        if let Some(h) = mgr.live_handle(id) {
            let _ = h.kill();
        }
    }
}
