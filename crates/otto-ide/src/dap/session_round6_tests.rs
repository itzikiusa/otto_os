//! Debug session life (IDE design round): monotonic output `seq`, the launch
//! `target_root`, classified start failures, frame selection that never moves
//! the execution point, thread switching with only a `thread_id`, and the
//! built-in Python bridge refusing to "verify" lines that cannot break.

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::{json, Value};

use super::*;
use crate::dap::{DapEvent, DapMessage, DapReader, DapWriter};
use crate::types::{IdeDebugLaunchSpec, IdeDebugState, IdeStackFrame, IdeStartDebugReq};

fn entry_for(id: &str, root: &str, language: &str) -> Arc<SessionEntry> {
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(root),
        language: language.into(),
        launch: IdeDebugLaunchSpec {
            program: format!("{root}/main.py"),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };
    let meta = (id.to_string(), "T".into(), language.into(), "dlv".into());
    SessionEntry::new(1, meta, Vec::new(), req, DapStartContext::new())
}

#[test]
fn output_seq_is_monotonic_and_survives_the_ring_dropping_old_entries() {
    let entry = entry_for("dbg_seq", "/tmp/proj", "go");
    let total = MAX_DEBUG_OUTPUT + 50;
    for i in 0..total {
        entry.append_output("stdout", format!("line-{i}\n"));
    }
    let out = entry.read_snapshot().output;
    assert_eq!(out.len(), MAX_DEBUG_OUTPUT);
    assert_eq!(out.last().map(|o| o.seq), Some(total as u64));
    assert_eq!(out.first().map(|o| o.seq), Some(51));
    assert!(out.windows(2).all(|w| w[1].seq == w[0].seq + 1));
}

#[test]
fn initial_adapter_output_is_numbered_from_one() {
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/tmp/proj"),
        language: "java".into(),
        ..IdeStartDebugReq::default()
    };
    let initial = vec![
        IdeDebugOutputEntry {
            category: "console".into(),
            text: "Compiled Main\n".into(),
            seq: 0,
        },
        IdeDebugOutputEntry {
            category: "console".into(),
            text: "second\n".into(),
            seq: 0,
        },
    ];
    let meta = ("dbg_init".into(), "T".into(), "java".into(), "jdwp".into());
    let entry = SessionEntry::new(1, meta, initial, req, DapStartContext::new());
    entry.append_output("stdout", "third\n".into());
    let seqs: Vec<u64> = entry.read_snapshot().output.iter().map(|o| o.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
}

#[test]
fn snapshot_carries_the_launch_target_root() {
    let entry = entry_for("dbg_root", "/tmp/proj-a", "go");
    let snap = entry.read_snapshot();
    assert_eq!(snap.target_root.as_deref(), Some("/tmp/proj-a"));
    let wire = serde_json::to_value(&snap).unwrap();
    assert_eq!(wire["target_root"], "/tmp/proj-a");
}

#[test]
fn a_failed_start_reports_kind_and_ecosystem_and_a_human_message() {
    let sessions: DashMap<String, Arc<SessionEntry>> = DashMap::new();
    let entry = entry_for("dbg_fail", "/tmp/proj", "go");
    sessions.insert("dbg_fail".into(), entry.clone());
    let err = Error::NotFound(
        "Go debugger 'dlv' (Delve) not found; install it from IDE → Toolchains & SDKs".into(),
    );
    entry.fail_start(&sessions, &err);
    let snap = entry.read_snapshot();
    assert_eq!(snap.state, IdeDebugState::Error);
    assert_eq!(snap.error_kind.as_deref(), Some("adapter_missing"));
    assert_eq!(snap.error_ecosystem.as_deref(), Some("go"));
    let msg = snap.error.unwrap_or_default();
    assert!(msg.starts_with("Go debugger"), "{msg}");

    // A plain runtime failure keeps the old shape: message only.
    let other = entry_for("dbg_fail2", "/tmp/proj", "go");
    other.fail_and_evict(&sessions, "fatal adapter crash".into());
    let snap = other.read_snapshot();
    assert_eq!(snap.error_kind, None);
    let wire = serde_json::to_value(&snap).unwrap();
    assert!(wire.get("error_kind").is_none());
}

fn frame(id: i64, name: &str, path: Option<&str>, line: u32) -> Value {
    let mut f = json!({ "id": id, "name": name, "line": line, "column": 1 });
    if let Some(p) = path {
        f["source"] = json!({ "path": p });
    }
    f
}

/// Thread 1 is stopped in `work` (11) called from `main` (18); thread 2's top
/// frame has no source (a runtime frame) above `worker` (4).
fn spawn_frames_server(server_io: tokio::io::DuplexStream) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (sr, sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let mut w = DapWriter::new(sw);
        while let Ok(Some(DapMessage::Request(req))) = r.next_message().await {
            let args = req.arguments.clone().unwrap_or(Value::Null);
            let body = match req.command.as_str() {
                "stackTrace" => {
                    let frames = if args["threadId"] == 2 {
                        vec![
                            frame(20, "runtime.park", None, 1),
                            frame(21, "worker", Some("/tmp/proj/worker.py"), 4),
                        ]
                    } else {
                        vec![
                            frame(10, "work", Some("/tmp/proj/main.py"), 11),
                            frame(11, "main", Some("/tmp/proj/caller.py"), 18),
                        ]
                    };
                    json!({ "stackFrames": frames })
                }
                "scopes" => json!({ "scopes": [] }),
                _ => json!({}),
            };
            if w.send_response(req.seq, req.command.clone(), Ok(Some(body)))
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

fn paused_on_thread_one(entry: &SessionEntry) {
    entry.mutate_and_broadcast(|s| {
        s.state = IdeDebugState::Paused;
        s.active_thread_id = Some(1);
        s.active_frame_id = Some(10);
        s.paused_file = Some("/tmp/proj/main.py".into());
        s.paused_line = Some(11);
        s.selected_frame_file = Some("/tmp/proj/main.py".into());
        s.selected_frame_line = Some(11);
        s.stack_frames = vec![
            IdeStackFrame {
                id: 10,
                name: "work".into(),
                path: Some("/tmp/proj/main.py".into()),
                line: 11,
                column: 1,
            },
            IdeStackFrame {
                id: 11,
                name: "main".into(),
                path: Some("/tmp/proj/caller.py".into()),
                line: 18,
                column: 1,
            },
        ];
    });
}

fn mock_conn(client_io: tokio::io::DuplexStream) -> Arc<ActiveConnection> {
    let (cr, cw) = tokio::io::split(client_io);
    let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel::<DapEvent>();
    Arc::new(ActiveConnection {
        wire: DapWireClient::spawn(Box::new(cr), Box::new(cw), event_tx),
        guard: BridgeGuard::new(),
        supports_terminate: false,
        exception_filters: Vec::new(),
    })
}

#[tokio::test]
async fn selecting_a_caller_frame_keeps_the_paused_location() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let server = spawn_frames_server(server_io);
    let conn = mock_conn(client_io);
    let entry = entry_for("dbg_frames", "/tmp/proj", "python");
    paused_on_thread_one(&entry);

    select_frame_and_refresh(&entry, &conn, Some(1), Some(11))
        .await
        .unwrap();
    let snap = entry.read_snapshot();
    assert_eq!(snap.active_frame_id, Some(11));
    assert_eq!(snap.paused_line, Some(11), "execution point must not move");
    assert!(snap
        .paused_file
        .as_deref()
        .is_some_and(|p| p.ends_with("main.py")));
    assert_eq!(snap.selected_frame_line, Some(18));
    assert!(snap
        .selected_frame_file
        .as_deref()
        .is_some_and(|p| p.ends_with("caller.py")));

    conn.shutdown_immediately();
    server.abort();
}

#[tokio::test]
async fn switching_thread_with_only_a_thread_id_selects_its_first_frame_with_source() {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let server = spawn_frames_server(server_io);
    let conn = mock_conn(client_io);
    let entry = entry_for("dbg_threads", "/tmp/proj", "go");
    paused_on_thread_one(&entry);

    select_frame_and_refresh(&entry, &conn, Some(2), None)
        .await
        .unwrap();
    let snap = entry.read_snapshot();
    assert_eq!(snap.active_thread_id, Some(2));
    assert_eq!(
        snap.active_frame_id,
        Some(21),
        "skips the source-less runtime frame"
    );
    assert_eq!(snap.selected_frame_line, Some(4));
    // Another thread is stopped somewhere else: its own top frame with source.
    assert_eq!(snap.paused_line, Some(4));
    assert!(snap
        .paused_file
        .as_deref()
        .is_some_and(|p| p.ends_with("worker.py")));

    conn.shutdown_immediately();
    server.abort();
}

#[test]
fn resuming_clears_the_selected_frame() {
    let entry = entry_for("dbg_resume", "/tmp/proj", "go");
    paused_on_thread_one(&entry);
    super::mark_resumed(&entry);
    let snap = entry.read_snapshot();
    assert_eq!(snap.selected_frame_line, None);
    assert_eq!(snap.selected_frame_file, None);
}

/// Spawn the embedded bdb bridge, send `setBreakpoints` for `lines` of
/// `source`, and return the adapter's per-breakpoint answers.
async fn bdb_set_breakpoints(source: &str, lines: &[u32]) -> Option<Vec<Value>> {
    let mut python = None;
    for candidate in ["python3", "/usr/bin/python3"] {
        let probe = tokio::process::Command::new(candidate)
            .arg("--version")
            .output()
            .await;
        if probe.is_ok() {
            python = Some(candidate);
            break;
        }
    }
    let python = python?;
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("review.py");
    std::fs::write(&file, source).unwrap();
    let mut child = tokio::process::Command::new(python)
        .args(["-u", "-c", super::super::py_bridge::PY_STDLIB_DAP_BRIDGE])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut w = DapWriter::new(child.stdin.take().unwrap());
    let mut r = DapReader::new(child.stdout.take().unwrap());
    let bps: Vec<Value> = lines.iter().map(|l| json!({ "line": l })).collect();
    let seq = w
        .send_request(
            "setBreakpoints",
            Some(json!({
                "source": { "path": file.to_string_lossy() },
                "breakpoints": bps,
            })),
        )
        .await
        .unwrap();
    let answer = tokio::time::timeout(Duration::from_secs(10), async {
        while let Ok(Some(msg)) = r.next_message().await {
            if let DapMessage::Response(resp) = msg {
                if resp.request_seq == seq {
                    return resp.body;
                }
            }
        }
        None
    })
    .await
    .expect("bridge answered setBreakpoints in time")?;
    let _ = w.send_request("disconnect", Some(json!({}))).await;
    let _ = child.kill().await;
    answer.get("breakpoints").and_then(Value::as_array).cloned()
}

#[tokio::test]
async fn python_bridge_moves_or_refuses_breakpoints_on_lines_without_code() {
    let source = "\
import sys

def build():
    order = {'items': [1, 2]}

    return order

def main():
    total = 0
    for i in range(3):
        total += i

if __name__ == '__main__':
    main()

";
    // 5: blank line inside build() -> its next line (6); 11: a real line;
    // 15: trailing blank line after the last statement -> nothing to stop on.
    let Some(acks) = bdb_set_breakpoints(source, &[5, 11, 15]).await else {
        eprintln!("python3 not available; skipping the bdb bridge check");
        return;
    };
    assert_eq!(acks.len(), 3, "{acks:?}");
    assert_eq!(acks[0]["verified"], true);
    assert_eq!(acks[0]["line"], 6);
    assert_eq!(acks[1]["verified"], true);
    assert_eq!(acks[1]["line"], 11);
    assert_eq!(acks[2]["verified"], false);
    assert_eq!(acks[2]["message"], "No code on this line");
}
