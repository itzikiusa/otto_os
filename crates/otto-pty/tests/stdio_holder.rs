//! Stdio holders end to end: a piped child survives its client going away,
//! a re-attach replays exactly the lines after its cursor, stdin keeps
//! flowing, the exit is reported, and a deliberate close ends child AND
//! holder. The test binary is its own holder executable (see holder.rs).

use std::path::Path;
use std::time::{Duration, Instant};

use otto_pty::holder::{self, HolderConfig, HolderLauncher};
use otto_pty::stdio_holder::{self, HeldStdio, StdioControl, StdioEvent};
use otto_pty::{AdoptError, CommandSpec};

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
    HolderConfig::new(dir.join("h"), launcher)
}

fn short_tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("sh")
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

async fn next(h: &mut HeldStdio) -> StdioEvent {
    tokio::time::timeout(Duration::from_secs(10), h.events.recv())
        .await
        .expect("an event in time")
        .expect("the stream is open")
}

async fn line(h: &mut HeldStdio) -> (u64, String) {
    match next(h).await {
        StdioEvent::Line { seq, text } => (seq, text),
        other => panic!("expected a line, got {other:?}"),
    }
}

/// Prints `n1`..`n6` 100 ms apart, then echoes stdin until EOF, then exits 3.
const SCRIPT: &str = "for i in 1 2 3 4 5 6; do echo n$i; sleep 0.1; done; \
                      while read l; do echo got:$l; done; echo bye >&2; exit 3";

#[tokio::test]
async fn a_child_outlives_its_client_and_a_reattach_replays_after_the_cursor() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let mut h = stdio_holder::spawn(&cfg, &sh(SCRIPT), serde_json::json!({"k": 1}))
        .await
        .unwrap();
    assert_eq!(h.info.meta["k"], 1);
    let child = h.info.child_pid.unwrap();
    assert_eq!(line(&mut h).await, (1, "n1".into()));
    assert_eq!(line(&mut h).await, (2, "n2".into()));
    // Durable up to 1 only; seq 2 was seen but not checkpointed.
    h.control.send(StdioControl::Ack(1)).unwrap();
    h.control.send(StdioControl::Detach).unwrap();
    let socket = h.socket.clone();
    drop(h);
    // The "daemon" is gone a while; the child keeps printing into the log.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(pid_alive(child), "the child survived its client");

    // Asking for less than was acknowledged would lose lines: refused.
    assert!(matches!(
        stdio_holder::attach(&socket, 0).await,
        Err(AdoptError::Failed(_))
    ));
    let mut h = stdio_holder::attach(&socket, 1).await.unwrap();
    assert_eq!(h.info.child_pid, Some(child));
    let mut got = Vec::new();
    for _ in 0..5 {
        got.push(line(&mut h).await);
    }
    assert_eq!(
        got,
        (2..=6).map(|i| (i, format!("n{i}"))).collect::<Vec<_>>(),
        "every line after the cursor, exactly once, in order"
    );
    h.control.send(StdioControl::Line("hello".into())).unwrap();
    assert_eq!(line(&mut h).await, (7, "got:hello".into()));
    h.control.send(StdioControl::CloseStdin).unwrap();
    match next(&mut h).await {
        StdioEvent::Exited {
            seq,
            code,
            stderr_tail,
        } => {
            assert_eq!(seq, 8);
            assert_eq!(code, Some(3));
            assert!(stderr_tail.contains("bye"), "{stderr_tail:?}");
        }
        other => panic!("expected the exit, got {other:?}"),
    }
    // Dropping the client without Detach releases the exited holder.
    drop(h);
    let deadline = Instant::now() + Duration::from_secs(5);
    while socket.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!socket.exists(), "a released holder exits");
}

#[tokio::test]
async fn an_exit_while_detached_is_kept_for_the_next_client() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let mut h = stdio_holder::spawn(
        &cfg,
        &sh("echo a; sleep 0.3; echo b; exit 5"),
        serde_json::Value::Null,
    )
    .await
    .unwrap();
    assert_eq!(line(&mut h).await, (1, "a".into()));
    h.control.send(StdioControl::Ack(1)).unwrap();
    h.control.send(StdioControl::Detach).unwrap();
    let socket = h.socket.clone();
    drop(h);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut h = stdio_holder::attach(&socket, 1).await.unwrap();
    assert_eq!(h.info.exited, Some(5));
    assert_eq!(line(&mut h).await, (2, "b".into()));
    assert!(matches!(
        next(&mut h).await,
        StdioEvent::Exited { code: Some(5), .. }
    ));
}

#[tokio::test]
async fn terminate_ends_a_running_child_and_its_holder() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let mut h = stdio_holder::spawn(
        &cfg,
        &sh("trap '' TERM; echo up; sleep 600"),
        serde_json::Value::Null,
    )
    .await
    .unwrap();
    let child = h.info.child_pid.unwrap();
    assert_eq!(line(&mut h).await, (1, "up".into()));
    h.control.send(StdioControl::Detach).unwrap();
    let socket = h.socket.clone();
    drop(h);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let s = socket.clone();
    tokio::task::spawn_blocking(move || holder::terminate(&s))
        .await
        .unwrap()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while (pid_alive(child) || socket.exists()) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!pid_alive(child), "KILL escalates past an ignored TERM");
    assert!(!socket.exists(), "the holder exits with its child");
}

#[tokio::test]
async fn a_second_client_supersedes_the_first() {
    let dir = short_tempdir();
    let cfg = config(dir.path());
    let mut a = stdio_holder::spawn(&cfg, &sh("echo x; sleep 600"), serde_json::Value::Null)
        .await
        .unwrap();
    assert_eq!(line(&mut a).await, (1, "x".into()));
    let mut b = stdio_holder::attach(&a.socket, 0).await.unwrap();
    assert_eq!(line(&mut b).await, (1, "x".into()));
    // The old client's stream just ends — no exit is reported to it.
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), a.events.recv())
            .await
            .unwrap(),
        None
    );
    a.control.send(StdioControl::Detach).unwrap();
    // Dropping b (no Detach) closes stdin + releases; `sleep` ignores EOF,
    // so end it for real.
    let s = b.socket.clone();
    b.control.send(StdioControl::Detach).unwrap();
    drop(b);
    tokio::task::spawn_blocking(move || holder::terminate(&s))
        .await
        .unwrap()
        .unwrap();
}
