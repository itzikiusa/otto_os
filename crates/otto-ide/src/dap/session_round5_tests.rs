//! Round 5 unit tests for DAP session lifecycle invariants:
//! - `configurationDone` failure marks session `IdeDebugState::Error` (never `Running`).
//! - Terminal states (`Terminated` / `Error`) never revert to `Running` on late events or `mark_resumed`.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::json;

use super::*;
use crate::dap::{DapMessage, DapReader, DapWriter};
use crate::types::{
    IdeDebugAction, IdeDebugCommandReq, IdeDebugLaunchSpec, IdeDebugState, IdeStartDebugReq,
    IdeTargetSpec,
};

#[tokio::test]
async fn configuration_done_failure_marks_session_error_never_running() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move {
        let (sr, sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let mut w = DapWriter::new(sw);
        while let Ok(Some(DapMessage::Request(req))) = r.next_message().await {
            match req.command.as_str() {
                "initialize" => {
                    w.send_response(
                        req.seq,
                        "initialize",
                        Ok(Some(json!({
                            "supportsConfigurationDoneRequest": true,
                            "supportsTerminateRequest": true
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "launch" => {
                    w.send_event("initialized", Some(json!({}))).await.unwrap();
                    w.send_response(req.seq, "launch", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
                "setBreakpoints" => {
                    w.send_response(
                        req.seq,
                        "setBreakpoints",
                        Ok(Some(json!({ "breakpoints": [] }))),
                    )
                    .await
                    .unwrap();
                }
                "configurationDone" => {
                    w.send_response(
                        req.seq,
                        "configurationDone",
                        Err("delve rejected configurationDone".into()),
                    )
                    .await
                    .unwrap();
                    break;
                }
                _ => {}
            }
        }
    });

    let sessions: Arc<DashMap<String, Arc<SessionEntry>>> = Arc::new(DashMap::new());
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/tmp/demo"),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: "/tmp/demo/main.go".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = DapStartContext::new().with_owner_user_id("u_test");
    let entry = SessionEntry::new(
        1,
        (
            "dbg_cfg_err".into(),
            "main.go".into(),
            "go".into(),
            "dlv".into(),
        ),
        Vec::new(),
        req,
        ctx,
    );
    sessions.insert("dbg_cfg_err".into(), entry.clone());

    let (cr, cw) = tokio::io::split(client_io);
    let res = handshake_and_run(
        &sessions,
        &entry,
        (Box::new(cr), Box::new(cw), BridgeGuard::new()),
        (json!({}), "launch", json!({})),
    )
    .await;

    let err = res.expect_err("configurationDone rejection must fail handshake");
    assert!(
        err.to_string().contains("configurationDone failed"),
        "unexpected error: {err}"
    );

    // Mirror `DapManager::start_session` error handling and assert state is Error, never Running.
    entry.fail_and_evict(&sessions, err.to_string());
    let snap = entry.read_snapshot();
    assert_eq!(snap.state, IdeDebugState::Error);
    assert!(snap
        .error
        .as_deref()
        .is_some_and(|e| e.contains("configurationDone failed")));
    server_task.await.unwrap();
}

#[tokio::test]
async fn terminal_states_never_revert_to_running_on_late_continued_events() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(async move {
        let (sr, sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let mut w = DapWriter::new(sw);
        while let Ok(Some(DapMessage::Request(req))) = r.next_message().await {
            match req.command.as_str() {
                "initialize" => {
                    w.send_response(
                        req.seq,
                        "initialize",
                        Ok(Some(json!({ "supportsConfigurationDoneRequest": true }))),
                    )
                    .await
                    .unwrap();
                }
                "launch" => {
                    w.send_event("initialized", Some(json!({}))).await.unwrap();
                    w.send_response(req.seq, "launch", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
                "configurationDone" => {
                    w.send_response(req.seq, "configurationDone", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                    w.send_event("terminated", Some(json!({}))).await.unwrap();
                    // Late out-of-order `continued` event emitted right after `terminated`.
                    w.send_event("continued", Some(json!({ "threadId": 1 })))
                        .await
                        .unwrap();
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    break;
                }
                _ => {}
            }
        }
    });

    let sessions: Arc<DashMap<String, Arc<SessionEntry>>> = Arc::new(DashMap::new());
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/tmp/demo"),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: "/tmp/demo/main.go".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let entry = SessionEntry::new(
        2,
        (
            "dbg_term".into(),
            "main.go".into(),
            "go".into(),
            "dlv".into(),
        ),
        Vec::new(),
        req,
        DapStartContext::new(),
    );
    sessions.insert("dbg_term".into(), entry.clone());

    let (cr, cw) = tokio::io::split(client_io);
    handshake_and_run(
        &sessions,
        &entry,
        (Box::new(cr), Box::new(cw), BridgeGuard::new()),
        (json!({}), "launch", json!({})),
    )
    .await
    .unwrap();

    server_task.await.unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;

    assert_eq!(entry.read_snapshot().state, IdeDebugState::Terminated);
    mark_resumed(&entry);
    assert_eq!(
        entry.read_snapshot().state,
        IdeDebugState::Terminated,
        "mark_resumed must never resurrect a Terminated session back to Running"
    );

    entry.fail_and_evict(&sessions, "fatal adapter crash".into());
    assert_eq!(entry.read_snapshot().state, IdeDebugState::Error);
    mark_resumed(&entry);
    assert_eq!(
        entry.read_snapshot().state,
        IdeDebugState::Error,
        "mark_resumed must never overwrite an Error session back to Running"
    );
}

#[tokio::test]
async fn restart_entry_preserves_launch_spec_and_config_id_across_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws-restart");
    std::fs::create_dir_all(&ws).unwrap();
    let counter_file = ws.join("runs.count");
    let script_path = ws.join("prog.sh");
    std::fs::write(&script_path, "#!/bin/bash\necho run >> \"$1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let ws_str = std::fs::canonicalize(&ws)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let script_str = std::fs::canonicalize(&script_path)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let counter_str = counter_file.to_string_lossy().into_owned();

    let dap = DapManager::new(tmp.path().join("data"));
    let snap = dap
        .start_session(
            IdeStartDebugReq {
                target: IdeTargetSpec::local(ws_str.clone()),
                language: "shell".into(),
                config_id: Some("rc_demo".into()),
                launch: IdeDebugLaunchSpec {
                    program: script_str.clone(),
                    args: vec![counter_str.clone()],
                    cwd: Some(ws_str.clone()),
                    ..Default::default()
                },
                ..Default::default()
            },
            DapStartContext::new()
                .with_caller_is_root(true)
                .with_owner_user_id("u_root"),
        )
        .await
        .expect("start shell debug session");

    assert_eq!(snap.config_id.as_deref(), Some("rc_demo"));
    let init_launch = snap.launch.as_ref().expect("initial snapshot launch spec");
    assert_eq!(init_launch.program, script_str);
    assert_eq!(init_launch.args, vec![counter_str.clone()]);

    let wait_done = |sid: String| {
        let dap_ref = &dap;
        async move {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            loop {
                let s = dap_ref.get_session(&sid).unwrap();
                match s.state {
                    IdeDebugState::Terminated => return s,
                    IdeDebugState::Error => panic!("debug session {sid} failed: {:?}", s.error),
                    _ => {}
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for debug session {sid} to terminate"
                );
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        }
    };

    let after_first = wait_done(snap.id.clone()).await;
    assert_eq!(after_first.config_id.as_deref(), Some("rc_demo"));
    assert!(after_first.launch.is_some());
    assert_eq!(
        std::fs::read_to_string(&counter_file)
            .unwrap()
            .lines()
            .count(),
        1
    );

    let restarted = dap
        .command(
            &snap.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::Restart,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .expect("restart debug session");
    assert_eq!(restarted.config_id.as_deref(), Some("rc_demo"));
    let restarted_launch = restarted
        .launch
        .as_ref()
        .expect("restarted snapshot must preserve launch spec");
    assert_eq!(restarted_launch.program, script_str);
    assert_eq!(restarted_launch.args, vec![counter_str.clone()]);

    let after_second = wait_done(snap.id.clone()).await;
    assert_eq!(after_second.config_id.as_deref(), Some("rc_demo"));
    assert!(after_second.launch.is_some());
    assert_eq!(
        std::fs::read_to_string(&counter_file)
            .unwrap()
            .lines()
            .count(),
        2
    );
}

fn bare_entry(id: &str) -> Arc<SessionEntry> {
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/tmp"),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: "main.go".into(),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };
    let meta = (id.to_string(), "T".into(), "go".into(), "dlv".into());
    SessionEntry::new(1, meta, Vec::new(), req, DapStartContext::new())
}

/// A debuggee logging thousands of lines a second must not cost one full
/// snapshot clone + broadcast per line.
#[tokio::test]
async fn debug_output_flood_is_coalesced_into_a_few_broadcasts() {
    use tokio::sync::broadcast::error::TryRecvError;
    let entry = bare_entry("dbg_flood");
    let mut rx = entry.events_tx.subscribe();
    for i in 0..10_000 {
        entry.append_output("stdout", format!("line-{i}\n"));
    }
    tokio::time::sleep(crate::dap::session_entry::OUTPUT_COALESCE * 3).await;
    let mut messages = 0u64;
    let mut last = None;
    loop {
        match rx.try_recv() {
            Ok(snap) => {
                messages += 1;
                last = Some(snap);
            }
            Err(TryRecvError::Lagged(n)) => messages += n,
            Err(_) => break,
        }
    }
    assert!(messages <= 3, "{messages} broadcasts for one burst");
    let last = last.expect("the burst is flushed");
    assert_eq!(
        last.output.last().map(|o| o.text.as_str()),
        Some("line-9999\n")
    );
    assert!(
        last.revision > 10_000,
        "every line still moves the revision"
    );
}

/// Stop while a boot step (a build) is in flight cancels it at once: the step
/// is dropped (killing its process group) and the lifecycle lock is released,
/// so Stop does not wait for the build or a 15 s adapter timeout.
#[tokio::test]
async fn stop_cancels_a_boot_in_flight_and_does_not_wait_on_its_lock() {
    use std::sync::atomic::{AtomicBool, Ordering as AtOrd};
    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, AtOrd::SeqCst);
        }
    }
    let entry = bare_entry("dbg_boot_stop");
    let my_gen = entry.boot_generation.load(AtOrd::SeqCst);
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = DropFlag(Arc::clone(&dropped));
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let boot_entry = Arc::clone(&entry);
    let boot = tokio::spawn(async move {
        let _lock = boot_entry.lifecycle_lock.lock().await;
        let _ = locked_tx.send(());
        let step = async move {
            let _flag = flag;
            tokio::time::sleep(Duration::from_secs(60)).await;
        };
        boot_entry.unless_superseded(my_gen, step).await
    });
    locked_rx.await.unwrap();

    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(3), stop_entry(&entry))
        .await
        .expect("Stop waited on the booting session");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        dropped.load(AtOrd::SeqCst),
        "the build step was not dropped"
    );
    assert_eq!(boot.await.unwrap(), None);
    assert_eq!(entry.read_snapshot().state, IdeDebugState::Terminated);
}

/// Daemon shutdown stops sessions concurrently and never blocks on a session
/// whose boot holds the lifecycle lock and ignores cancellation.
#[tokio::test]
async fn shutdown_is_bounded_even_when_boots_hold_the_lifecycle_lock() {
    let mgr = DapManager::new(std::path::PathBuf::from("/tmp/otto-dap-shutdown-test"));
    let mut holders = Vec::new();
    for i in 0..4 {
        let entry = bare_entry(&format!("dbg_stuck_{i}"));
        mgr.sessions
            .insert(format!("dbg_stuck_{i}"), Arc::clone(&entry));
        let (tx, rx) = tokio::sync::oneshot::channel();
        holders.push(tokio::spawn(async move {
            let _lock = entry.lifecycle_lock.lock().await;
            let _ = tx.send(());
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
        rx.await.unwrap();
    }
    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(3), mgr.shutdown())
        .await
        .expect("shutdown blocked on a booting session");
    assert!(
        started.elapsed() < Duration::from_millis(2500),
        "sessions were stopped one after another: {:?}",
        started.elapsed()
    );
    assert!(mgr.sessions.is_empty());
    for h in holders {
        h.abort();
    }
}

/// A build that times out takes its whole process group with it, not just the
/// leader (cargo's rustc children, `sh -lc`'s javac).
#[cfg(unix)]
#[tokio::test]
async fn run_target_command_timeout_kills_the_whole_process_group() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("child.pid");
    let script = format!("sleep 60 & echo $! > '{}'; wait", pid_file.display());
    let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
    let err = crate::dap::run_target_command(
        &target,
        None,
        "/bin/sh",
        &["-c".into(), script],
        &tmp.path().to_string_lossy(),
        &std::collections::BTreeMap::new(),
        Duration::from_millis(400),
    )
    .await
    .expect_err("must time out");
    assert!(err.to_string().contains("timed out"), "{err}");
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut gone = false;
    for _ in 0..50 {
        let alive = tokio::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());
        if !alive {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "grandchild {pid} outlived the timed-out build");
}

/// A reply over the 16 MiB cap (a huge `variables` body) fails that request;
/// the session's wire keeps working for the next one.
#[tokio::test]
async fn oversized_adapter_reply_fails_one_request_and_keeps_the_session() {
    use tokio::io::AsyncWriteExt;
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (cr, cw) = tokio::io::split(client_io);
    let (ev_tx, _ev_rx) = tokio::sync::mpsc::unbounded_channel();
    let wire = DapWireClient::spawn(Box::new(cr), Box::new(cw), ev_tx);
    let server = tokio::spawn(async move {
        let (sr, mut sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let Ok(Some(DapMessage::Request(req1))) = r.next_message().await else {
            panic!("expected the variables request");
        };
        let len = crate::dap::wire::MAX_MESSAGE_BYTES + 1024;
        let mut body = format!(
            r#"{{"seq":1,"type":"response","request_seq":{},"success":true,"command":"variables","body":{{"variables":["#,
            req1.seq
        )
        .into_bytes();
        body.resize(len, b' ');
        sw.write_all(format!("Content-Length: {len}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        sw.write_all(&body).await.unwrap();
        let Ok(Some(DapMessage::Request(req2))) = r.next_message().await else {
            panic!("expected the threads request");
        };
        let mut w = DapWriter::new(sw);
        w.send_response(req2.seq, "threads", Ok(Some(json!({ "threads": [] }))))
            .await
            .unwrap();
    });
    let err = wire
        .request("variables", Some(json!({ "variablesReference": 1 })))
        .await
        .expect_err("the oversized reply must fail its request");
    assert!(err.to_string().contains("could not be used"), "{err}");
    let ok = wire
        .request("threads", None)
        .await
        .expect("wire still alive");
    assert!(ok.success);
    server.await.unwrap();
}
