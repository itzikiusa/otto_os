//! PTY holders end to end: a child spawned through a holder survives its
//! daemon-side handle going away (a "daemon restart"), is re-adopted with its
//! screen + scrollback, keeps taking input, and is cleaned up — child AND
//! holder — when deliberately closed.
//!
//! The test binary is its own holder executable: [`HolderLauncher`] re-runs
//! it with the `holder_entry` test selected, and that test turns into the
//! holder process when `OTTO_PTY_HOLDER` is set (a no-op otherwise).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use otto_pty::holder::{self, HolderConfig, HolderLauncher};
use otto_pty::{AdoptError, CommandSpec, PtyHandle};

/// Holder entry point: only does anything in a process the launcher started.
#[test]
fn holder_entry() {
    holder::run_if_holder_process();
}

fn config(dir: &Path) -> HolderConfig {
    let launcher = HolderLauncher::current_exe(vec![
        "holder_entry".into(),
        "--exact".into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
    ])
    .expect("current exe");
    let mut cfg = HolderConfig::new(dir.join("h"), launcher);
    cfg.exit_linger = Duration::from_secs(30);
    cfg
}

/// A short temp dir: unix socket paths are capped at ~104 bytes and macOS's
/// per-user `$TMPDIR` alone eats half of that.
fn short_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ph")
        .tempdir_in("/tmp")
        .expect("tempdir")
}

fn sh(script: &str) -> CommandSpec {
    CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: Some("/tmp".into()),
        env: vec![],
    }
}

fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only probes.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn wait_until(what: &str, within: Duration, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + within;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}");
}

fn screen_text(h: &PtyHandle) -> String {
    String::from_utf8_lossy(&h.snapshot_with_history(1000)).into_owned()
}

fn only_socket(cfg: &HolderConfig) -> PathBuf {
    let socks = cfg.sockets();
    assert_eq!(
        socks.len(),
        1,
        "expected exactly one holder socket: {socks:?}"
    );
    socks[0].clone()
}

#[test]
fn held_child_survives_handle_drop_and_is_readopted_with_scrollback() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    // `cat` echoes input back; the marker lines go into history first.
    let spec =
        sh("i=1; while [ $i -le 60 ]; do echo HIST_$i; i=$((i+1)); done; echo READY; exec cat");
    let meta = serde_json::json!({ "session_id": "S1", "ingest_token": "tok" });
    let first = PtyHandle::spawn_held(&cfg, &spec, 100, 30, meta.clone()).expect("spawn held");
    let info = first.holder().cloned().expect("held");
    let child = first.pid().expect("child pid");
    assert_eq!(info.child_pid, Some(child));
    assert_eq!(info.meta, meta);
    assert_ne!(
        info.holder_pid,
        std::process::id(),
        "the holder is its own process"
    );
    wait_until("READY", Duration::from_secs(10), || {
        screen_text(&first).contains("READY")
    });

    // Input through the holder reaches the child (and its echo comes back).
    first.write(b"before-restart\n").expect("write");
    wait_until("echo", Duration::from_secs(5), || {
        screen_text(&first).matches("before-restart").count() >= 2
    });

    // "Daemon restart": the handle goes away detached.
    first.detach();
    drop(first);
    std::thread::sleep(Duration::from_millis(300));
    assert!(pid_alive(child), "the child must survive its handle");
    assert!(
        pid_alive(info.holder_pid),
        "the holder must survive its client"
    );

    // A new "daemon" finds and re-adopts it: same process, same history.
    let socket = only_socket(&cfg);
    let second = PtyHandle::adopt(&socket).expect("adopt");
    assert_eq!(second.pid(), Some(child), "adopted the SAME process");
    assert_eq!(second.holder().map(|i| i.meta.clone()), Some(meta));
    assert_eq!(second.size(), (100, 30), "grid restored");
    let text = screen_text(&second);
    assert!(
        text.contains("HIST_1"),
        "history survives the restart: {text}"
    );
    assert!(
        text.contains("before-restart"),
        "screen survives the restart"
    );
    assert!(!second.has_exited());

    // It is fully live: input, output, resize.
    second.write(b"after-restart\n").expect("write after adopt");
    wait_until("echo after adopt", Duration::from_secs(5), || {
        screen_text(&second).matches("after-restart").count() >= 2
    });
    second.resize(90, 25).expect("resize");
    assert_eq!(second.size(), (90, 25));

    // Deliberate close (not detached): child AND holder go, socket removed.
    drop(second);
    wait_until("child gone", Duration::from_secs(10), || !pid_alive(child));
    wait_until("holder gone", Duration::from_secs(10), || {
        !pid_alive(info.holder_pid)
    });
    wait_until("socket removed", Duration::from_secs(5), || {
        cfg.sockets().is_empty()
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_exit_code_propagates_and_the_holder_exits() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let handle = tokio::task::spawn_blocking({
        let cfg = cfg.clone();
        move || {
            PtyHandle::spawn_held(
                &cfg,
                &sh("echo BYE; sleep 0.3; exit 7"),
                80,
                24,
                serde_json::Value::Null,
            )
        }
    })
    .await
    .unwrap()
    .expect("spawn held");
    let holder_pid = handle.holder().unwrap().holder_pid;
    let mut exit = handle.on_exit();
    let code = tokio::time::timeout(Duration::from_secs(10), exit.wait_for(|v| v.is_some()))
        .await
        .expect("exit in time")
        .expect("watch")
        .expect("code");
    assert_eq!(code, 7, "the child's exit status crosses the holder");
    assert!(handle.has_exited());
    assert!(
        screen_text(&handle).contains("BYE"),
        "final output arrives before the exit"
    );
    // The daemon side released it on EXITED: the holder goes on its own.
    let deadline = Instant::now() + Duration::from_secs(10);
    while pid_alive(holder_pid) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !pid_alive(holder_pid),
        "released holder exits after its child"
    );
    assert!(cfg.sockets().is_empty());
    // Writes after the exit fail fast instead of hanging.
    assert!(handle.write(b"x").is_err());
}

#[test]
fn exit_while_detached_is_reported_on_adoption() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let h = PtyHandle::spawn_held(
        &cfg,
        &sh("echo READY; read x; echo DONE_$x; exit 3"),
        80,
        24,
        serde_json::Value::Null,
    )
    .expect("spawn held");
    wait_until("READY", Duration::from_secs(10), || {
        screen_text(&h).contains("READY")
    });
    let holder_pid = h.holder().unwrap().holder_pid;
    // Detach BEFORE the input that makes the child exit: its EXITED report can
    // then race the hang-up without the detached handle releasing the holder.
    h.detach();
    h.write(b"z\n").expect("write");
    drop(h);
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        pid_alive(holder_pid),
        "an exited, unreleased holder lingers for the daemon"
    );
    let socket = only_socket(&cfg);
    let adopted = PtyHandle::adopt(&socket).expect("adopt the lingering holder");
    assert!(adopted.has_exited(), "dead on arrival");
    assert_eq!(*adopted.on_exit().borrow(), Some(3));
    assert!(
        screen_text(&adopted).contains("DONE_z"),
        "its final screen is still there"
    );
    drop(adopted);
    wait_until("holder gone", Duration::from_secs(10), || {
        !pid_alive(holder_pid)
    });
}

#[test]
fn kill_through_the_holder_escalates_past_an_ignored_hup() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let h = PtyHandle::spawn_held(
        &cfg,
        &sh("trap '' HUP; echo READY; while :; do sleep 0.2; done"),
        80,
        24,
        serde_json::Value::Null,
    )
    .expect("spawn held");
    wait_until("READY", Duration::from_secs(10), || {
        screen_text(&h).contains("READY")
    });
    let child = h.pid().unwrap();
    h.kill().expect("kill");
    wait_until(
        "child killed",
        otto_pty::KILL_GRACE + Duration::from_secs(5),
        || h.has_exited(),
    );
    assert!(!pid_alive(child));
}

#[test]
fn stale_socket_is_reported_and_removed() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    cfg.ensure_dir().unwrap();
    let path = cfg.new_socket_path();
    // A socket file nobody listens on (its holder was SIGKILLed).
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    // The other tests in this binary spawn PTY children concurrently, and a
    // fork that lands while the listener above is open hands the child a copy
    // of its fd until that child execs (CLOEXEC only closes it AT exec). In
    // that window the socket still listens: adopt connects, then reads EOF
    // instead of a HELLO_ACK and reports `Failed`, not `Stale`. Once our own
    // fd is closed no new fork can inherit it, so the first refused connect
    // means the socket is dead for good — the state a SIGKILLed holder leaves.
    wait_until(
        "no forked child still holds the listener",
        Duration::from_secs(10),
        || {
            matches!(
                std::os::unix::net::UnixStream::connect(&path),
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused
            )
        },
    );
    assert!(matches!(PtyHandle::adopt(&path), Err(AdoptError::Stale)));
    assert!(!path.exists(), "stale socket file removed");
}

#[test]
fn terminate_ends_a_holder_without_adopting_it() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let h = PtyHandle::spawn_held(
        &cfg,
        &sh("echo READY; exec sleep 60"),
        80,
        24,
        serde_json::Value::Null,
    )
    .expect("spawn held");
    let child = h.pid().unwrap();
    let holder_pid = h.holder().unwrap().holder_pid;
    h.detach();
    drop(h);
    holder::terminate(&only_socket(&cfg)).expect("terminate");
    wait_until("child gone", Duration::from_secs(10), || !pid_alive(child));
    wait_until("holder gone", Duration::from_secs(10), || {
        !pid_alive(holder_pid)
    });
}

/// A wiped holder directory (a test harness deleting its throwaway data dir,
/// an uninstall) leaves the holder unreachable for good: it must end its child
/// and exit on its own instead of running out the orphan TTL.
#[test]
fn holder_whose_socket_vanished_ends_its_child() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let h = PtyHandle::spawn_held(
        &cfg,
        &sh("echo READY; exec sleep 60"),
        80,
        24,
        serde_json::Value::Null,
    )
    .expect("spawn held");
    let child = h.pid().unwrap();
    let holder_pid = h.holder().unwrap().holder_pid;
    h.detach();
    drop(h);
    std::fs::remove_dir_all(&cfg.dir).expect("wipe the holder dir");
    wait_until("child gone", Duration::from_secs(10), || !pid_alive(child));
    wait_until("holder gone", Duration::from_secs(10), || {
        !pid_alive(holder_pid)
    });
}

/// Two daemons overlapping during a restart: the newer adoption wins, the
/// older client is told it was superseded and lets go — it neither reports
/// an exit nor reconnects (which would kick the new owner back), and dropping
/// it does not end the session.
#[test]
fn a_newer_adoption_supersedes_the_old_client_without_ending_the_session() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let old = PtyHandle::spawn_held(
        &cfg,
        &sh("echo READY; exec cat"),
        80,
        24,
        serde_json::Value::Null,
    )
    .expect("spawn held");
    wait_until("READY", Duration::from_secs(10), || {
        screen_text(&old).contains("READY")
    });
    let child = old.pid().unwrap();
    let new = PtyHandle::adopt(&only_socket(&cfg)).expect("second adoption");
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !old.has_exited(),
        "a superseded client must not report an exit"
    );
    drop(old); // superseded = detached: must not kill the child
    std::thread::sleep(Duration::from_millis(300));
    assert!(pid_alive(child));
    new.write(b"still-mine\n")
        .expect("the new owner keeps input");
    wait_until("echo", Duration::from_secs(5), || {
        screen_text(&new).matches("still-mine").count() >= 2
    });
    assert!(!new.has_exited());
    drop(new);
    wait_until("child gone", Duration::from_secs(10), || !pid_alive(child));
}

/// Perf 01 N1: the daemon's unviewed-history cap reaches the HOLDER's
/// emulator — the copy a future adoption is rebuilt from — and restoring the
/// cap lets the holder's history grow back.
#[test]
fn history_cap_reaches_the_holder_emulator_and_regrows() {
    use otto_pty::{EMULATOR_SCROLLBACK_LINES, UNVIEWED_SCROLLBACK_LINES};
    let dir = short_tempdir();
    let cfg = config(dir.path());
    // 1500 history rows, then (on the first input line) 1500 more.
    let spec = sh(
        "i=1; while [ $i -le 1500 ]; do printf 'H%04d\\n' $i; i=$((i+1)); done; echo READY; \
         read x; i=1; while [ $i -le 1500 ]; do printf 'J%04d\\n' $i; i=$((i+1)); done; echo DONE; exec cat",
    );
    let full = |h: &PtyHandle| {
        String::from_utf8_lossy(&h.snapshot_with_history(EMULATOR_SCROLLBACK_LINES)).into_owned()
    };
    let first =
        PtyHandle::spawn_held(&cfg, &spec, 80, 24, serde_json::json!({})).expect("spawn held");
    wait_until("READY", Duration::from_secs(20), || {
        full(&first).contains("READY")
    });
    assert!(
        full(&first).contains("H0001"),
        "the full cap keeps all 1500 rows"
    );

    // Unviewed: shrink. The daemon mirror drops the oldest rows now…
    first.set_history_cap(UNVIEWED_SCROLLBACK_LINES);
    assert!(!full(&first).contains("H0001"));
    // …and so does the holder: a re-adoption is rebuilt without them.
    std::thread::sleep(Duration::from_millis(200));
    first.detach();
    drop(first);
    let second = PtyHandle::adopt(&only_socket(&cfg)).expect("adopt");
    let text = full(&second);
    assert!(text.contains("H1500"), "recent history survives");
    assert!(
        !text.contains("H0400"),
        "the holder dropped rows past the {UNVIEWED_SCROLLBACK_LINES}-row cap"
    );

    // Viewed again: the cap is restored in the holder too (the new handle
    // has never told it anything, so the first call is always sent).
    second.set_history_cap(EMULATOR_SCROLLBACK_LINES);
    std::thread::sleep(Duration::from_millis(200));
    second.write(b"go\n").expect("write");
    wait_until("DONE", Duration::from_secs(20), || {
        full(&second).contains("DONE")
    });
    std::thread::sleep(Duration::from_millis(200));
    second.detach();
    drop(second);
    let third = PtyHandle::adopt(&only_socket(&cfg)).expect("re-adopt");
    assert!(
        full(&third).contains("J0001"),
        "history regrew past {UNVIEWED_SCROLLBACK_LINES} rows in the holder"
    );
    drop(third);
}
