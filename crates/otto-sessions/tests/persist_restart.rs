//! Sessions survive a daemon restart (setting `session_persistence`).
//!
//! One `SessionManager` creates a shell session the user started — it runs in
//! a PTY holder — then "the daemon restarts": the manager shuts down for a
//! restart (detaching the session) and is dropped, and a NEW manager over the
//! same database + holder directory runs the boot restore. The new manager
//! must re-adopt the SAME live process (same pid), with its screen and
//! scrollback, accept input into it, and clean the process AND its holder up
//! when the session is archived. Engine-owned sessions and the setting's OFF
//! position keep the old in-process PTY.
//!
//! The test binary is its own holder executable: the launcher re-runs it with
//! only `holder_entry` selected, which turns into the holder process when the
//! `OTTO_PTY_HOLDER` marker is set (and is a no-op test otherwise).
//!
//! Everything runs in ONE scenario test on purpose: a daemon shutdown flips a
//! process-wide "detach on drop" switch, which must not leak into unrelated
//! tests running concurrently in this binary.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, SessionStatus, Workspace};
use otto_core::new_id;
use otto_sessions::pty_holder::{HolderConfig, HolderLauncher};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{SessionsRepo, SettingsRepo};
use tokio::sync::broadcast;

#[test]
fn holder_entry() {
    otto_sessions::pty_holder::run_if_holder_process();
}

fn holder_config(dir: &Path) -> HolderConfig {
    let launcher = HolderLauncher::current_exe(vec![
        "holder_entry".into(),
        "--exact".into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ])
    .expect("current exe");
    HolderConfig::new(dir.join("holders"), launcher)
}

fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn wait_until(what: &str, within: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

fn screen(mgr: &SessionManager, id: &str) -> String {
    mgr.live_handle(&id.to_string())
        .map(|h| String::from_utf8_lossy(&h.snapshot_with_history(1000)).into_owned())
        .unwrap_or_default()
}

struct World {
    pool: otto_state::DbPool,
    ws: Workspace,
    user: String,
    cwd: String,
}

async fn world(root: &Path) -> World {
    let pool = otto_state::open(&root.join("otto.db")).await.expect("db");
    let user = new_id();
    let ws_id = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
        .bind(&user).bind("u").bind("x").bind("U").bind(&now)
        .execute(&pool).await.unwrap();
    let cwd = root.join("ws");
    std::fs::create_dir_all(&cwd).unwrap();
    let cwd = cwd.to_string_lossy().into_owned();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&ws_id)
        .bind("w")
        .bind(&cwd)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
    let ws = Workspace {
        id: ws_id,
        name: "w".into(),
        root_path: cwd.clone(),
        settings: serde_json::json!({}),
        archived: false,
        created_at: chrono::Utc::now(),
    };
    World {
        pool,
        ws,
        user,
        cwd,
    }
}

/// One "daemon run": a manager over the shared DB and holder directory.
fn daemon(w: &World, holders: &HolderConfig) -> Arc<SessionManager> {
    let (events, _) = broadcast::channel(256);
    // A plain POSIX shell keeps the test independent of the host's $SHELL rc files.
    let providers =
        ProviderRegistry::new(Some(&serde_json::json!({ "shell": { "cmd": "/bin/sh" } })));
    Arc::new(
        SessionManager::new(SessionsRepo::new(w.pool.clone()), events, providers)
            .with_settings_repo(SettingsRepo::new(w.pool.clone()))
            .with_auth_repo(otto_rbac::AuthRepo::new(w.pool.clone()))
            .with_pty_holders(holders.clone()),
    )
}

fn shell_req(w: &World, meta: serde_json::Value) -> CreateSessionReq {
    CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("shell".into()),
        title: None,
        cwd: Some(w.cwd.clone()),
        connection_id: None,
        meta: Some(meta),
        model: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn user_sessions_survive_a_daemon_restart_and_engine_ones_do_not() {
    // Short root: unix socket paths are capped at ~104 bytes.
    let root = tempfile::Builder::new()
        .prefix("pr")
        .tempdir_in("/tmp")
        .unwrap();
    let w = world(root.path()).await;
    let holders = holder_config(root.path());

    // ── daemon run #1 ──────────────────────────────────────────────────────
    let first = daemon(&w, &holders);
    let mine = first
        .create(
            &w.ws,
            &w.user,
            shell_req(&w, serde_json::json!({ "work": { "origin": "manual" } })),
            None,
        )
        .await
        .expect("create user shell");
    let id = mine.id.clone();
    let handle = first.live_handle(&id).expect("live");
    assert!(
        handle.holder().is_some(),
        "a session the user started runs in a pty holder"
    );
    let pid = handle.pid().expect("pid");
    let holder_pid = handle.holder().unwrap().holder_pid;
    drop(handle);

    // An engine-owned session stays in-process (its engine dies with the daemon).
    let engine = first
        .create(
            &w.ws,
            &w.user,
            shell_req(&w, serde_json::json!({ "work": { "origin": "workflow" } })),
            None,
        )
        .await
        .expect("create engine shell");
    let engine_handle = first.live_handle(&engine.id).expect("live");
    assert!(
        engine_handle.holder().is_none(),
        "engine sessions are never held"
    );
    let engine_pid = engine_handle.pid().expect("pid");
    drop(engine_handle);

    // Give the shell some state worth keeping: a variable and some history.
    first
        .input(
            &id,
            b"KEEP=still-here; for i in 1 2 3 4 5; do echo LINE_$i; done; echo MARK_BEFORE\n",
        )
        .await
        .unwrap();
    wait_until("output before restart", Duration::from_secs(10), || {
        screen(&first, &id).contains("MARK_BEFORE")
    })
    .await;

    // ── the restart ────────────────────────────────────────────────────────
    let (killed, kept) = first.shutdown_for_restart().await;
    assert_eq!(
        (killed, kept),
        (1, 1),
        "the engine session is killed, the user's is kept"
    );
    drop(first);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pid_alive(pid), "the user's shell must outlive its daemon");
    assert!(pid_alive(holder_pid), "…and so must its holder");
    wait_until("engine shell gone", Duration::from_secs(10), || {
        !pid_alive(engine_pid)
    })
    .await;

    // ── daemon run #2 ──────────────────────────────────────────────────────
    let before = SessionsRepo::new(w.pool.clone())
        .get(&id)
        .await
        .unwrap()
        .last_active_at;
    let second = daemon(&w, &holders);
    let summary = second
        .restore_all(&|_: &String| None::<String>)
        .await
        .expect("restore");
    assert_eq!(
        summary,
        otto_sessions::RestoreSummary {
            kept_running: 1,
            suspended: 0
        }
    );
    assert!(second.is_live(&id), "re-adopted on boot");
    // A restart is not activity (A14): the row's idle clock is untouched and
    // the adopted handle's last-output clock is back-dated to the holder's.
    assert_eq!(
        second.get(&id).await.unwrap().last_active_at,
        before,
        "re-adoption must not stamp last_active_at"
    );
    let quiet = second
        .live_handle(&id)
        .expect("live")
        .last_output_at()
        .elapsed();
    assert!(
        quiet >= Duration::from_millis(400),
        "idle clock reset by the restart: quiet for only {quiet:?}"
    );
    assert_eq!(
        second.live_pid(&id),
        Some(pid),
        "the SAME process, not a respawn"
    );
    let status = second.get(&id).await.unwrap().status;
    assert!(
        matches!(
            status,
            SessionStatus::Running | SessionStatus::Idle | SessionStatus::Working
        ),
        "re-adopted session is live, got {status:?}"
    );
    assert_eq!(
        second.get(&engine.id).await.unwrap().status,
        SessionStatus::Exited,
        "the engine session restores the old way"
    );
    let text = screen(&second, &id);
    assert!(
        text.contains("LINE_1") && text.contains("MARK_BEFORE"),
        "scrollback survives: {text}"
    );

    // Shell state survives too: the variable set before the restart.
    second.input(&id, b"echo VALUE=$KEEP\n").await.unwrap();
    wait_until("shell state after restart", Duration::from_secs(10), || {
        screen(&second, &id).contains("VALUE=still-here")
    })
    .await;

    // ── deliberate close: process AND holder go, nothing leaks ─────────────
    second.archive(&id).await.expect("archive");
    wait_until("shell gone", Duration::from_secs(10), || !pid_alive(pid)).await;
    wait_until("holder gone", Duration::from_secs(10), || {
        !pid_alive(holder_pid)
    })
    .await;
    wait_until("socket removed", Duration::from_secs(5), || {
        holders.sockets().is_empty()
    })
    .await;

    // ── setting OFF: new sessions are in-process again ─────────────────────
    SettingsRepo::new(w.pool.clone())
        .put(
            otto_sessions::SESSION_PERSISTENCE_SETTING,
            &serde_json::json!(false),
        )
        .await
        .unwrap();
    assert!(!second.persistence_enabled().await);
    let plain = second
        .create(&w.ws, &w.user, shell_req(&w, serde_json::json!({})), None)
        .await
        .expect("create with persistence off");
    let plain_handle = second.live_handle(&plain.id).expect("live");
    assert!(
        plain_handle.holder().is_none(),
        "persistence off → local pty"
    );
    drop(plain_handle);
    second.kill_session(&plain.id).await.unwrap();
    assert!(holders.sockets().is_empty());
}
