//! js-debug child sessions against an in-process fake js-debug: a root that
//! announces two worker targets with `startDebugging`, each stopping at a
//! breakpoint, so the focus, the queue and the hand-off on Continue show.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::dap::session::{mark_resumed, BoxRead, BoxWrite, DapStartContext, SessionEntry};
use crate::dap::session_transport::handshake_and_run;
use crate::dap::wire::{DapMessage, DapReader, DapWriter};
use crate::dap::BridgeGuard;
use crate::types::{IdeBreakpoint, IdeDebugState, IdeStartDebugReq, IdeTargetSpec};

const BP_FILE: &str = "/ws/tests/app.spec.ts";

/// Serve one connection: the root (no `__pendingTargetId`) announces `workers`
/// targets after `configurationDone` and reports the answers on `answers`; a
/// child verifies breakpoint 1, stops in its own file, and resumes on `continue`.
async fn serve(stream: TcpStream, workers: usize, answers: mpsc::UnboundedSender<bool>) {
    serve_logged(stream, workers, answers, None).await;
}

/// One `setBreakpoints` a connection received: (its target, the source path, how many).
type BpCall = (Option<String>, String, usize);

async fn serve_logged(
    stream: TcpStream,
    workers: usize,
    answers: mpsc::UnboundedSender<bool>,
    bp_log: Option<mpsc::UnboundedSender<BpCall>>,
) {
    let (r, w) = stream.into_split();
    let mut reader = DapReader::new(r);
    let mut writer = DapWriter::new(w);
    let mut target: Option<String> = None;
    while let Ok(Some(msg)) = reader.next_message().await {
        let req = match msg {
            DapMessage::Request(req) => req,
            DapMessage::Response(resp) => {
                if resp.command == "startDebugging" {
                    let _ = answers.send(resp.success);
                }
                continue;
            }
            DapMessage::Event(_) => continue,
        };
        let args = req.arguments.clone().unwrap_or(Value::Null);
        let body = match req.command.as_str() {
            "initialize" => json!({ "supportsConfigurationDoneRequest": true }),
            "launch" => {
                target = args
                    .get("__pendingTargetId")
                    .and_then(Value::as_str)
                    .map(String::from);
                json!({})
            }
            "setBreakpoints" => {
                let n = args["breakpoints"].as_array().map_or(0, Vec::len);
                if let Some(log) = &bp_log {
                    let path = args["source"]["path"].as_str().unwrap_or("").to_string();
                    let _ = log.send((target.clone(), path, n));
                }
                let bps: Vec<Value> = (1..=n)
                    .map(|id| json!({ "id": id, "verified": false, "message": "Unbound breakpoint" }))
                    .collect();
                json!({ "breakpoints": bps })
            }
            "threads" => {
                json!({ "threads": [{ "id": 1, "name": target.clone().unwrap_or_default() }] })
            }
            "stackTrace" => {
                let file = format!("/ws/{}.ts", target.as_deref().unwrap_or("root"));
                json!({ "stackFrames": [{ "id": 7, "name": "test", "source": { "path": file }, "line": 4, "column": 1 }] })
            }
            "scopes" => json!({ "scopes": [{ "name": "Local", "variablesReference": 0 }] }),
            _ => json!({}),
        };
        let _ = writer
            .send_response(req.seq, &req.command, Ok(Some(body)))
            .await;
        match (req.command.as_str(), target.as_deref()) {
            ("initialize", None) => {
                let _ = writer.send_event("initialized", Some(json!({}))).await;
            }
            ("configurationDone", None) => {
                for i in 0..workers {
                    let msg = DapMessage::Request(crate::dap::wire::DapRequest::new(
                        100 + i as i64,
                        "startDebugging",
                        Some(json!({
                            "request": "launch",
                            "configuration": {
                                "type": "pwa-node",
                                "name": format!("workerProcessEntry.js [{i}]"),
                                "__pendingTargetId": format!("w{i}"),
                            },
                        })),
                    ));
                    let _ = writer.write_message(&msg).await;
                }
            }
            ("configurationDone", Some(_)) => {
                let _ = writer
                    .send_event(
                        "breakpoint",
                        Some(json!({ "reason": "changed", "breakpoint": {
                            "id": 1, "verified": true, "line": 3,
                            "source": { "path": BP_FILE },
                        } })),
                    )
                    .await;
                let _ = writer
                    .send_event(
                        "stopped",
                        Some(json!({ "reason": "breakpoint", "threadId": 1 })),
                    )
                    .await;
            }
            ("continue", Some(_)) => {
                let _ = writer
                    .send_event("continued", Some(json!({ "threadId": 1 })))
                    .await;
            }
            _ => {}
        }
    }
}

async fn wait_for(
    entry: &SessionEntry,
    what: &str,
    ok: impl Fn(&crate::types::IdeDebugSessionSnapshot) -> bool,
) {
    for _ in 0..200 {
        if ok(&entry.read_snapshot()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}: {:?}", entry.read_snapshot());
}

#[tokio::test]
async fn child_targets_attach_verify_breakpoints_and_hand_the_focus_on_continue() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (answers_tx, mut answers) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve(stream, 2, answers_tx.clone()));
        }
    });

    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/ws"),
        language: "typescript".into(),
        breakpoints: vec![IdeBreakpoint {
            path: BP_FILE.into(),
            line: 3,
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let meta = (
        "s1".into(),
        "Playwright".into(),
        "typescript".into(),
        "js-debug".into(),
    );
    let entry = SessionEntry::new(1, meta, Vec::new(), req, DapStartContext::default());
    entry.targets.enable("127.0.0.1".into(), port);
    let sessions = Arc::new(DashMap::new());
    sessions.insert("s1".to_string(), entry.clone());
    let (r, w) = TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap()
        .into_split();
    let io = (
        Box::new(r) as BoxRead,
        Box::new(w) as BoxWrite,
        BridgeGuard::new(),
    );
    handshake_and_run(&sessions, &entry, io, (json!({}), "launch", json!({})))
        .await
        .unwrap();

    // Both workers stop: the first stop has the focus, the second waits for it.
    wait_for(&entry, "both targets paused", |s| {
        s.state == IdeDebugState::Paused
            && s.targets.len() == 2
            && s.targets.iter().all(|t| t.paused)
    })
    .await;
    for _ in 0..2 {
        assert_eq!(
            answers.recv().await,
            Some(true),
            "startDebugging is answered success"
        );
    }
    let snap = entry.read_snapshot();
    assert!(
        snap.targets
            .iter()
            .all(|t| t.name.starts_with("Playwright worker [")),
        "{:?}",
        snap.targets
    );
    assert_eq!(
        snap.breakpoints[0].verified,
        Some(true),
        "a child verified the breakpoint"
    );
    let first = snap.paused_file.clone().expect("the focused worker's stop");
    assert!(first == "/ws/w0.ts" || first == "/ws/w1.ts", "{first}");

    // Continue the focus (as `command` does): the queued worker's stop takes it.
    mark_resumed(&entry);
    let conn = entry.active_conn().unwrap();
    conn.wire
        .request("continue", Some(json!({ "threadId": 1 })))
        .await
        .unwrap();
    let second = if first == "/ws/w0.ts" {
        "/ws/w1.ts"
    } else {
        "/ws/w0.ts"
    };
    wait_for(&entry, "the queued stop", |s| {
        s.state == IdeDebugState::Paused && s.paused_file.as_deref() == Some(second)
    })
    .await;
    let snap = entry.read_snapshot();
    assert_eq!(
        snap.targets.iter().filter(|t| t.paused).count(),
        1,
        "{:?}",
        snap.targets
    );
    // A worker's own "not bound yet" never unverifies another's binding.
    assert_eq!(snap.breakpoints[0].verified, Some(true));

    entry.disconnect_and_kill();
    assert!(entry.targets.connections().is_empty());
}

#[tokio::test]
async fn removing_a_files_last_breakpoint_clears_it_in_every_child() {
    // SEC6-3: each js-debug connection must be told when a file loses its last breakpoint.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (answers_tx, _answers) = mpsc::unbounded_channel();
    let (log_tx, mut log) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(serve_logged(
                stream,
                2,
                answers_tx.clone(),
                Some(log_tx.clone()),
            ));
        }
    });
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/ws"),
        language: "typescript".into(),
        breakpoints: vec![IdeBreakpoint {
            path: BP_FILE.into(),
            line: 3,
            enabled: true,
            ..Default::default()
        }],
        ..Default::default()
    };
    let meta = (
        "s2".into(),
        "Playwright".into(),
        "typescript".into(),
        "js-debug".into(),
    );
    let entry = SessionEntry::new(1, meta, Vec::new(), req, DapStartContext::default());
    entry.targets.enable("127.0.0.1".into(), port);
    let sessions = Arc::new(DashMap::new());
    sessions.insert("s2".to_string(), entry.clone());
    let (r, w) = TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap()
        .into_split();
    let io = (
        Box::new(r) as BoxRead,
        Box::new(w) as BoxWrite,
        BridgeGuard::new(),
    );
    handshake_and_run(&sessions, &entry, io, (json!({}), "launch", json!({})))
        .await
        .unwrap();
    wait_for(&entry, "both targets paused", |s| {
        s.targets.len() == 2 && s.targets.iter().all(|t| t.paused)
    })
    .await;
    while log.try_recv().is_ok() {}

    entry.mutate_and_broadcast(|snap| snap.breakpoints.clear());
    crate::dap::session_inspect::sync_breakpoints_everywhere(&entry)
        .await
        .unwrap();
    let mut cleared: Vec<Option<String>> = Vec::new();
    while let Ok((target, path, n)) = log.try_recv() {
        if path == BP_FILE && n == 0 {
            cleared.push(target);
        }
    }
    cleared.sort();
    assert_eq!(
        cleared,
        vec![None, Some("w0".to_string()), Some("w1".to_string())],
        "the root and both workers get the empty setBreakpoints"
    );
    entry.disconnect_and_kill();
}

#[tokio::test]
async fn other_reverse_requests_are_refused() {
    let (client, server) = tokio::io::duplex(4096);
    let (cr, cw) = tokio::io::split(client);
    let (ev_tx, _ev_rx) = mpsc::unbounded_channel();
    let _wire = crate::dap::session::DapWireClient::spawn(Box::new(cr), Box::new(cw), ev_tx);
    let (sr, sw) = tokio::io::split(server);
    let mut reader = DapReader::new(sr);
    let mut writer = DapWriter::new(sw);
    let msg = DapMessage::Request(crate::dap::wire::DapRequest::new(
        9,
        "runInTerminal",
        Some(json!({})),
    ));
    writer.write_message(&msg).await.unwrap();
    let Some(DapMessage::Response(resp)) = reader.next_message().await.unwrap() else {
        panic!("expected a response");
    };
    assert_eq!(resp.request_seq, 9);
    assert!(!resp.success);
    assert_eq!(resp.message.as_deref(), Some("not supported"));
}
