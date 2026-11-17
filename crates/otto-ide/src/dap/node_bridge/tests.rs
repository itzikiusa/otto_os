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
        let _ = run_node_bridge_loop(server_io, ws, &mut out_rx, BridgeGuard::new()).await;
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
