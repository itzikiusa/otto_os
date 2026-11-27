use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::cdp::InspectorWs;
use super::run_node_bridge_loop;
use crate::dap::wire::{DapMessage, DapReader, DapWriter};
use crate::dap::BridgeGuard;

/// One masked client frame's JSON payload, `None` once the bridge hangs up.
async fn read_client_json(sock: &mut TcpStream) -> Option<Value> {
    let mut hdr = [0u8; 2];
    sock.read_exact(&mut hdr).await.ok()?;
    let mut len = (hdr[1] & 0x7f) as usize;
    if len == 126 {
        let mut ext = [0u8; 2];
        sock.read_exact(&mut ext).await.ok()?;
        len = u16::from_be_bytes(ext) as usize;
    }
    let mut mask = [0u8; 4];
    sock.read_exact(&mut mask).await.ok()?;
    let mut payload = vec![0u8; len];
    sock.read_exact(&mut payload).await.ok()?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i & 3];
    }
    serde_json::from_slice(&payload).ok()
}

async fn write_server_json(sock: &mut TcpStream, val: Value) {
    let payload = serde_json::to_vec(&val).unwrap();
    let mut frame = vec![0x81];
    if payload.len() < 126 {
        frame.push(payload.len() as u8);
    } else {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&payload);
    sock.write_all(&frame).await.unwrap();
}

/// Node replies to a `Runtime.runIfWaitingForDebugger` that its main thread
/// dispatches before entering `waitForFrontend`, but drops it, then waits for the
/// frontend forever (seen live under load: node parked in `WaitForConnectByOptions`
/// after acking it). The fake inspector plays exactly that interleaving: the first
/// `runIfWaitingForDebugger` is a no-op, after which it starts waiting and emits
/// `NodeRuntime.waitingForDebugger` to sessions that sent `NodeRuntime.enable`.
#[tokio::test]
async fn run_if_waiting_dispatched_before_node_waits_still_releases_the_debuggee() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let inspector = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = BufReader::new(&mut sock);
        let mut line = String::new();
        while req.read_line(&mut line).await.unwrap() > 0 && line != "\r\n" {
            line.clear();
        }
        sock.write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n")
            .await
            .unwrap();
        let (mut node_runtime_enabled, mut waiting) = (false, false);
        while let Some(msg) = read_client_json(&mut sock).await {
            let id = msg["id"].clone();
            write_server_json(&mut sock, json!({ "id": id, "result": {} })).await;
            match msg["method"].as_str().unwrap_or("") {
                "NodeRuntime.enable" => node_runtime_enabled = true,
                "Runtime.runIfWaitingForDebugger" if waiting => {
                    let paused = json!({ "method": "Debugger.paused",
                        "params": { "reason": "other", "hitBreakpoints": ["bp"], "callFrames": [] } });
                    write_server_json(&mut sock, paused).await;
                }
                "Runtime.runIfWaitingForDebugger" => {
                    waiting = true;
                    if node_runtime_enabled {
                        let ev =
                            json!({ "method": "NodeRuntime.waitingForDebugger", "params": {} });
                        write_server_json(&mut sock, ev).await;
                    }
                }
                _ => {}
            }
        }
    });

    let ws = InspectorWs::connect(port, "/x").await.unwrap();
    let (client_io, server_io) = tokio::io::duplex(65536);
    let (_out_tx, mut out_rx) = mpsc::unbounded_channel();
    let bridge = tokio::spawn(async move {
        let _ = run_node_bridge_loop(
            server_io,
            ws,
            &mut out_rx,
            BridgeGuard::new(),
            String::new(),
            false,
        )
        .await;
    });
    let (r, w) = tokio::io::split(client_io);
    let mut dap_r = DapReader::new(r);
    let mut dap_w = DapWriter::new(w);
    for cmd in ["initialize", "launch", "configurationDone"] {
        dap_w.send_request(cmd, Some(json!({}))).await.unwrap();
    }
    let stopped = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match dap_r.next_message().await.unwrap() {
                Some(DapMessage::Event(ev)) if ev.event == "stopped" => return ev,
                Some(_) => {}
                None => panic!("bridge closed before the debuggee paused"),
            }
        }
    })
    .await
    .expect("debuggee stuck waiting for the debugger: never paused");
    assert_eq!(stopped.body.unwrap()["reason"], "breakpoint");
    bridge.abort();
    inspector.abort();
}

/// Live end-to-end verification against host `node` (skipped only if `node` is absent):
/// spawns `node --inspect-brk --enable-source-maps --experimental-strip-types`, hits a
/// breakpoint in a `.ts` file, verifies stackTrace + local variable inspection, and finishes.
#[tokio::test]
async fn live_node_typescript_debugger_hits_breakpoint_and_reads_locals() {
    use super::{spawn_node_inspector_bridge, NodeInspectorBridgeSpec};
    use crate::types::IdeTargetSpec;
    use std::collections::BTreeMap;

    let node_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| std::path::Path::new(d).join("node"))
        .find(|p| p.is_file());
    let Some(node_bin) = node_path else {
        return;
    };

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let script = root.join("calc.ts");
    std::fs::write(
        &script,
        "function total(a: number, b: number): number {\n  const sum = a + b;\n  return sum;\n}\nconsole.log('LIVE_NODE_RES=' + total(19, 23));\n",
    )
    .unwrap();

    let target = IdeTargetSpec::local(root.to_string_lossy().into_owned());
    let spec = NodeInspectorBridgeSpec {
        node_bin: node_bin.to_string_lossy().into_owned(),
        node_flags: vec![
            "--enable-source-maps".into(),
            "--experimental-strip-types".into(),
        ],
        program: script.to_string_lossy().into_owned(),
        args: Vec::new(),
        cwd: root.to_string_lossy().into_owned(),
        env: BTreeMap::new(),
        attach_port: None,
    };
    let (client_io, guard) = spawn_node_inspector_bridge(&target, None, spec)
        .await
        .expect("spawn node inspector bridge");
    let (r, w) = tokio::io::split(client_io);
    let mut dap_r = DapReader::new(r);
    let mut dap_w = DapWriter::new(w);

    dap_w
        .send_request("initialize", Some(json!({})))
        .await
        .unwrap();
    dap_w.send_request("launch", Some(json!({}))).await.unwrap();
    dap_w
        .send_request(
            "setBreakpoints",
            Some(json!({
                "source": { "path": script.to_string_lossy() },
                "breakpoints": [{ "line": 3 }]
            })),
        )
        .await
        .unwrap();
    dap_w
        .send_request("configurationDone", Some(json!({})))
        .await
        .unwrap();

    // Wait for breakpoint hit at line 3 (`return sum;`).
    let stopped = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match dap_r.next_message().await.unwrap() {
                Some(DapMessage::Event(ev)) if ev.event == "stopped" => return ev,
                Some(_) => {}
                None => panic!("node bridge closed before hitting breakpoint"),
            }
        }
    })
    .await
    .expect("timed out waiting for Node breakpoint");
    assert_eq!(stopped.body.unwrap()["reason"], "breakpoint");

    let st_seq = dap_w
        .send_request("stackTrace", Some(json!({ "threadId": 1 })))
        .await
        .unwrap();
    let frames = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(DapMessage::Response(resp)) = dap_r.next_message().await.unwrap() {
                if resp.request_seq == st_seq {
                    return resp.body.unwrap()["stackFrames"].clone();
                }
            }
        }
    })
    .await
    .expect("timed out waiting for stackTrace");
    let top = &frames[0];
    assert_eq!(top["name"], "total");
    assert_eq!(top["line"], 3);

    // Evaluate `sum` in the paused frame.
    let eval_seq = dap_w
        .send_request(
            "evaluate",
            Some(json!({ "expression": "sum", "frameId": 1, "context": "watch" })),
        )
        .await
        .unwrap();
    let eval_res = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(DapMessage::Response(resp)) = dap_r.next_message().await.unwrap() {
                if resp.request_seq == eval_seq {
                    return resp.body.unwrap()["result"].as_str().unwrap().to_string();
                }
            }
        }
    })
    .await
    .expect("timed out waiting for evaluate");
    assert_eq!(eval_res, "42");

    dap_w
        .send_request("continue", Some(json!({ "threadId": 1 })))
        .await
        .unwrap();
    let mut saw_output = false;
    let _ = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match dap_r.next_message().await.unwrap() {
                Some(DapMessage::Event(ev)) if ev.event == "output" => {
                    if ev
                        .body
                        .as_ref()
                        .and_then(|b| b.get("output"))
                        .and_then(Value::as_str)
                        .is_some_and(|s| s.contains("LIVE_NODE_RES=42"))
                    {
                        saw_output = true;
                    }
                }
                Some(DapMessage::Event(ev)) if ev.event == "terminated" => break,
                Some(_) => {}
                None => break,
            }
        }
    })
    .await;
    assert!(saw_output, "expected LIVE_NODE_RES=42 on stdout");
    guard.kill_all();
}
