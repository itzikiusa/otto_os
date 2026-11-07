//! Test double: a tiny `codex app-server` over stdio (JSON-RPC without the
//! `"jsonrpc"` member, one message per line), written against the shapes of
//! codex-cli 0.166.0's `generate-json-schema`. Behaviour by prompt text:
//!
//! * `ask`   — a `commandExecution` item, then
//!   `item/commandExecution/requestApproval`; `accept*` runs it (output
//!   delta, exit 0), `decline` marks it declined, `cancel` interrupts the
//!   turn.
//! * `tool image` — a completed MCP result with text, metadata and two PNGs.
//! * `hang`  — one delta, then waits for `turn/interrupt` → `interrupted`.
//! * `crash` — exit 3 at once (mid-turn).
//! * `fail`  — a `commandExecution` that fails (exit 1, output `boom`).
//! * anything else — `echo: <text>` as `item/agentMessage/delta`s, plus
//!   ` [<model> <effort> <image count>]` when the turn sets any of them.
//!
//! `thread/resume` succeeds for ids this binary minted (`fake-th-*`) and
//! fails otherwise. stdin EOF ends the process.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn out(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn notify(method: &str, params: Value) {
    out(json!({"method": method, "params": params}));
}

fn turn(id: &str, status: &str) -> Value {
    json!({"id": id, "items": [], "status": status})
}

fn agent_message(thread: &str, turn_id: &str, item: &str, parts: &[&str]) {
    notify(
        "item/started",
        json!({"threadId": thread, "turnId": turn_id, "startedAtMs": 0,
               "item": {"type": "agentMessage", "id": item, "text": ""}}),
    );
    for p in parts {
        notify(
            "item/agentMessage/delta",
            json!({"threadId": thread, "turnId": turn_id, "itemId": item, "delta": p}),
        );
    }
    notify(
        "item/completed",
        json!({"threadId": thread, "turnId": turn_id, "completedAtMs": 0,
               "item": {"type": "agentMessage", "id": item, "text": parts.concat()}}),
    );
}

fn command(thread: &str, turn_id: &str, status: &str, output: Option<&str>) -> Value {
    json!({"threadId": thread, "turnId": turn_id, "startedAtMs": 0,
           "item": {"type": "commandExecution", "id": "cmd-1", "command": "make clean",
                    "cwd": "/", "commandActions": [], "status": status,
                    "aggregatedOutput": output, "exitCode": output.map(|_| 0)}})
}

fn main() {
    let stdin = std::io::stdin();
    let mut thread = String::new();
    let mut turns = 0u32;
    let mut current: Option<String> = None;
    // The approval this server is waiting on: (rpc id, turn id).
    let mut asking: Option<(u64, String)> = None;
    let mut next_server_id = 100u64;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match method {
            "initialize" => out(
                json!({"id": id, "result": {"userAgent": "otto/0.166.0 (fake)",
                "codexHome": "/tmp", "platformFamily": "unix", "platformOs": "macos"}}),
            ),
            "initialized" => {}
            "thread/start" => {
                thread = format!("fake-th-{}", std::process::id());
                out(
                    json!({"id": id, "result": {"thread": {"id": thread}, "model": "fake-model",
                    "approvalPolicy": params["approvalPolicy"], "sandbox": params["sandbox"]}}),
                );
            }
            "thread/resume" => {
                let tid = params["threadId"].as_str().unwrap_or_default();
                if tid.starts_with("fake-th-") {
                    thread = tid.to_string();
                    out(
                        json!({"id": id, "result": {"thread": {"id": thread}, "model": "fake-model"}}),
                    );
                } else {
                    out(
                        json!({"id": id, "error": {"code": -32600, "message": format!("no rollout found for thread id {tid}")}}),
                    );
                }
            }
            "turn/start" => {
                turns += 1;
                let tid = format!("turn-{turns}");
                current = Some(tid.clone());
                out(json!({"id": id, "result": {"turn": turn(&tid, "inProgress")}}));
                notify(
                    "turn/started",
                    json!({"threadId": thread, "turn": turn(&tid, "inProgress")}),
                );
                let input = params["input"].as_array().cloned().unwrap_or_default();
                let text = input
                    .iter()
                    .find_map(|i| i["text"].as_str())
                    .unwrap_or_default()
                    .to_string();
                // `model` / `effort` / image inputs are echoed back.
                let images = input.iter().filter(|i| i["type"] == "image").count();
                let extras = match (params["model"].as_str(), params["effort"].as_str()) {
                    (None, None) if images == 0 => String::new(),
                    (m, e) => format!(" [{} {} {images}]", m.unwrap_or("-"), e.unwrap_or("-")),
                };
                match text.as_str() {
                    "tool image" => {
                        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=";
                        notify(
                            "item/completed",
                            json!({"threadId":thread,"turnId":tid,"item":{
                            "type":"mcpToolCall","id":"tool-images","server":"preview","tool":"screenshot",
                            "arguments":{"page":"example"},"status":"completed","result":{
                                "content":[{"type":"text","text":"Screenshot captured"},
                                    {"type":"image","mimeType":"image/png","data":png},
                                    {"type":"image","mimeType":"image/png","data":png}],
                                "structuredContent":{"width":1,"height":1}
                            }}}),
                        );
                        notify(
                            "turn/completed",
                            json!({"threadId":thread,"turn":turn(&tid,"completed")}),
                        );
                        current = None;
                    }
                    "crash" => {
                        eprintln!("fake codex: crashing on purpose");
                        std::process::exit(3);
                    }
                    "ask" => {
                        notify("item/started", command(&thread, &tid, "inProgress", None));
                        next_server_id += 1;
                        asking = Some((next_server_id, tid.clone()));
                        out(
                            json!({"id": next_server_id, "method": "item/commandExecution/requestApproval",
                            "params": {"threadId": thread, "turnId": tid, "itemId": "cmd-1",
                                       "startedAtMs": 0, "command": "make clean", "reason": "clean the build"}}),
                        );
                    }
                    "fail" => {
                        notify("item/started", command(&thread, &tid, "inProgress", None));
                        let mut done = command(&thread, &tid, "failed", Some("boom\n"));
                        done["item"]["exitCode"] = json!(1);
                        notify("item/completed", done);
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
                    "hang" => {
                        notify(
                            "item/agentMessage/delta",
                            json!({"threadId": thread, "turnId": tid, "itemId": "msg-h", "delta": "working"}),
                        );
                    }
                    other => {
                        let parts: Vec<&str> = ["echo: ", other, &extras]
                            .into_iter()
                            .filter(|p| !p.is_empty())
                            .collect();
                        agent_message(&thread, &tid, "msg-1", &parts);
                        notify(
                            "thread/tokenUsage/updated",
                            json!({"threadId": thread, "turnId": tid,
                            "tokenUsage": {"last": {"inputTokens": 5, "outputTokens": 2, "cachedInputTokens": 0,
                                "reasoningOutputTokens": 0, "totalTokens": 7},
                                "total": {"inputTokens": 5, "outputTokens": 2, "cachedInputTokens": 0,
                                "reasoningOutputTokens": 0, "totalTokens": 7}, "modelContextWindow": 1000}}),
                        );
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
                }
            }
            "turn/interrupt" => {
                out(json!({"id": id, "result": {}}));
                if let Some((rid, _)) = asking.take() {
                    notify(
                        "serverRequest/resolved",
                        json!({"threadId": thread, "requestId": rid}),
                    );
                }
                if let Some(tid) = current.take() {
                    notify(
                        "turn/completed",
                        json!({"threadId": thread, "turn": turn(&tid, "interrupted")}),
                    );
                }
            }
            "" if !id.is_null() => {
                // A response to our approval request.
                let Some((rid, tid)) = asking.take() else {
                    continue;
                };
                if id.as_u64() != Some(rid) {
                    continue;
                }
                let decision = msg
                    .pointer("/result/decision")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match decision {
                    "accept" | "acceptForSession" => {
                        notify(
                            "item/commandExecution/outputDelta",
                            json!({"threadId": thread, "turnId": tid, "itemId": "cmd-1", "delta": "cleaned\n"}),
                        );
                        notify(
                            "item/completed",
                            command(&thread, &tid, "completed", Some("cleaned\n")),
                        );
                        agent_message(&thread, &tid, "msg-2", &["done"]);
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
                    "decline" => {
                        notify("item/completed", command(&thread, &tid, "declined", None));
                        agent_message(&thread, &tid, "msg-2", &["skipped"]);
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
                    _ => {
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "interrupted")}),
                        );
                        current = None;
                    }
                }
            }
            _ if !id.is_null() => {
                out(json!({"id": id, "error": {"code": -32601, "message": "unsupported"}}))
            }
            _ => {}
        }
    }
}
