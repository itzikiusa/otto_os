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
