//! Round 5 unit tests for DAP session lifecycle invariants:
//! - `configurationDone` failure marks session `IdeDebugState::Error` (never `Running`).
//! - Terminal states (`Terminated` / `Error`) never revert to `Running` on late events or `mark_resumed`.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::json;

use super::*;
use crate::dap::{DapMessage, DapReader, DapWriter};
use crate::types::{IdeDebugLaunchSpec, IdeDebugState, IdeStartDebugReq, IdeTargetSpec};

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
