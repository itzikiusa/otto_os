//! Test double: a tiny `codex app-server` over stdio (JSON-RPC without the
//! `"jsonrpc"` member, one message per line), written against the shapes of
//! codex-cli 0.166.0's `generate-json-schema`. Behaviour by prompt text:
//!
//! * `ask`   — a `commandExecution` item, then
//!   `item/commandExecution/requestApproval`; `accept*` runs it (output
//!   delta, exit 0), `decline` marks it declined, `cancel` interrupts the
//!   turn.
//! * `tool image` — a completed MCP result with text, metadata and two PNGs.
//! * `slow <secs>` — "Working…", then waits `<secs>` before completing; a
//!   `turn/steer` meanwhile emits `steered: <text>` in the same turn.
//! * `hang`  — one delta, then waits for `turn/interrupt` → `interrupted`.
//! * `crash` — exit 3 at once (mid-turn).
//! * `fail`  — a `commandExecution` that fails (exit 1, output `boom`).
//! * `limit` — the turn fails with Codex's usage-limit error
//!   (`codexErrorInfo: "usageLimitExceeded"`).
//! * anything else — `echo: <text>` as `item/agentMessage/delta`s, plus
//!   ` [<model> <effort> <image count>]` when the turn sets any of them.
//!
//! `thread/resume` succeeds for ids this binary minted (`fake-th-*`) and
//! fails otherwise. stdin EOF ends the process.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

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

/// What a plain turn echoes besides its text: `model` / `effort` / the image
/// count, a collaboration mode and a skill input.
fn echo_extras(params: &Value, input: &[Value]) -> String {
    let images = input.iter().filter(|i| i["type"] == "image").count();
    let mut extras = match (params["model"].as_str(), params["effort"].as_str()) {
        (None, None) if images == 0 => String::new(),
        (m, e) => format!(" [{} {} {images}]", m.unwrap_or("-"), e.unwrap_or("-")),
    };
    if let Some(m) = params
        .pointer("/collaborationMode/mode")
        .and_then(Value::as_str)
    {
        extras.push_str(&format!(" (mode {m})"));
    }
    if let Some(skill) = input.iter().find(|i| i["type"] == "skill") {
        extras.push_str(&format!(
            " (skill {})",
            skill["name"].as_str().unwrap_or("?")
        ));
    }
    extras
}

enum In {
    Line(Value),
    Eof,
}

fn main() {
    let (tx, rx) = mpsc::channel::<In>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(msg) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(In::Line(msg));
            }
        }
        let _ = tx.send(In::Eof);
    });
    let mut thread = String::new();
    let mut turns = 0u32;
    let mut steers = 0u32;
    let mut current: Option<String> = None;
    let mut waiting_until: Option<Instant> = None;
    // The approval this server is waiting on: (rpc id, turn id).
    let mut asking: Option<(u64, String)> = None;
    let mut question: Option<(u64, String)> = None;
    let mut next_server_id = 100u64;
    loop {
        let msg = match waiting_until {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        let Some(msg) = msg else {
            if let Some(until) = waiting_until {
                if until <= Instant::now() {
                    waiting_until = None;
                    if let Some(tid) = current.take() {
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                    }
                }
            }
            continue;
        };
        let msg = match msg {
            In::Line(v) => v,
            In::Eof => return,
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
                // Otto's delegation rules ride a thread's first prompt (runtime::delegation).
                let had_rules = text.contains("</otto_instructions>\n\n");
                let text = match text.split_once("</otto_instructions>\n\n") {
                    Some((_, t)) => t.to_string(),
                    None => text,
                };
                // `rules?`: whether this prompt carried them (after a handoff too).
                let text = match text.rsplit_once("\n\n").map_or(text.as_str(), |(_, t)| t) {
                    "rules?" => format!("rules: {}", if had_rules { "yes" } else { "no" }),
                    _ if text == "rules?" => {
                        format!("rules: {}", if had_rules { "yes" } else { "no" })
                    }
                    _ => text,
                };
                // A subagent's task header (runtime::delegation): act on the task.
                let text = match text.split_once("]\n\n") {
                    Some((h, t)) if h.starts_with("[Subagent task from thread ") => t.to_string(),
                    _ => text,
                };
                let extras = echo_extras(&params, &input);
                match text.as_str() {
                    // `stream:<n>:<ms>`: `c0 `…`c<n-1> `, one delta per <ms>.
                    t if t.starts_with("stream:") => {
                        let mut f = t[7..].split(':');
                        let n: usize = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        let ms: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        notify(
                            "item/started",
                            json!({"threadId": thread, "turnId": tid, "startedAtMs": 0,
                                   "item": {"type": "agentMessage", "id": "msg-s", "text": ""}}),
                        );
                        let mut all = String::new();
                        for i in 0..n {
                            let d = format!("c{i} ");
                            all.push_str(&d);
                            notify(
                                "item/agentMessage/delta",
                                json!({"threadId": thread, "turnId": tid, "itemId": "msg-s", "delta": d}),
                            );
                            std::thread::sleep(std::time::Duration::from_millis(ms));
                        }
                        notify(
                            "item/completed",
                            json!({"threadId": thread, "turnId": tid, "completedAtMs": 0,
                                   "item": {"type": "agentMessage", "id": "msg-s", "text": all}}),
                        );
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
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
                    // `name me`: Codex names the thread.
                    "name me" => {
                        notify(
                            "thread/name/updated",
                            json!({"threadId": thread, "threadName": "Named by Codex"}),
                        );
                        agent_message(&thread, &tid, "msg-n", &["named"]);
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
                    // `make image`: an image it generated, as a file on disk.
                    "make image" => {
                        use base64::Engine;
                        let png = base64::engine::general_purpose::STANDARD
                            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=")
                            .unwrap_or_default();
                        let path = std::env::temp_dir()
                            .join(format!("fake-codex-{}-{tid}.png", std::process::id()));
                        let _ = std::fs::write(&path, png);
                        notify(
                            "item/completed",
                            json!({"threadId": thread, "turnId": tid, "item": {"type": "imageGeneration",
                                "id": "ig-1", "status": "completed", "revisedPrompt": "a red dot",
                                "savedPath": path, "inputImages": []}}),
                        );
                        agent_message(&thread, &tid, "msg-i", &["made it"]);
                        notify(
                            "turn/completed",
                            json!({"threadId": thread, "turn": turn(&tid, "completed")}),
                        );
                        current = None;
                    }
                    "crash" => {
                        eprintln!("fake codex: crashing on purpose");
                        std::process::exit(3);
                    }
                    "question form" => {
                        next_server_id += 1;
                        question = Some((next_server_id, tid.clone()));
                        out(
                            json!({"id":next_server_id,"method":"item/tool/requestUserInput","params":{
                            "threadId":thread,"turnId":tid,"itemId":"question-1","questions":[
                                {"id":"strategy_native","header":"Strategy","question":"Which approach should I use?","isOther":true,"options":[{"label":"Careful","description":"Small changes"},{"label":"Broad","description":"Wider changes"}]},
                                {"id":"note_native","header":"Context","question":"What should I keep in mind?","options":null}
                            ]}}),
                        );
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
                    "limit" => {
                        let mut t = turn(&tid, "failed");
                        t["error"] = json!({"message": "You've hit your usage limit. Try again later.",
                                            "codexErrorInfo": "usageLimitExceeded"});
                        notify("turn/completed", json!({"threadId": thread, "turn": t}));
                        current = None;
                    }
                    "hang" => {
                        notify(
                            "item/agentMessage/delta",
                            json!({"threadId": thread, "turnId": tid, "itemId": "msg-h", "delta": "working"}),
                        );
                    }
                    s if s == "slow" || s.starts_with("slow ") => {
                        let secs: f64 = s
                            .split_whitespace()
                            .nth(1)
                            .and_then(|w| w.parse().ok())
                            .unwrap_or(1.0);
                        agent_message(&thread, &tid, "msg-slow", &["Working…"]);
                        waiting_until = Some(Instant::now() + Duration::from_secs_f64(secs));
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
            "skills/list" => out(
                json!({"id": id, "result": {"data": [{"cwd": "/", "errors": [], "skills": [
                    {"name": "review", "description": "Review a diff thoroughly", "enabled": true,
                     "path": "/fake/skills/review/SKILL.md", "scope": "user"}
                ]}]}}),
            ),
            "turn/steer" => {
                let want_thread = params["threadId"].as_str().unwrap_or_default();
                let want_turn = params["expectedTurnId"].as_str().unwrap_or_default();
                if want_thread != thread || current.as_deref() != Some(want_turn) {
                    out(json!({
                        "id": id,
                        "error": {"code": -32600, "message": "no matching active turn to steer"},
                    }));
                    continue;
                }
                let tid = current.clone().unwrap();
                out(json!({"id": id, "result": {"turnId": tid}}));
                let input = params["input"].as_array().cloned().unwrap_or_default();
                let text = input
                    .iter()
                    .find_map(|i| i["text"].as_str())
                    .unwrap_or_default();
                let images = input.iter().filter(|i| i["type"] == "image").count();
                let reply = if images > 0 {
                    format!("steered: {text} [{images}]")
                } else {
                    format!("steered: {text}")
                };
                steers += 1;
                agent_message(&thread, &tid, &format!("msg-steer-{steers}"), &[&reply]);
            }
            "turn/interrupt" => {
                waiting_until = None;
                out(json!({"id": id, "result": {}}));
                if let Some((rid, _)) = question.take() {
                    notify(
                        "serverRequest/resolved",
                        json!({"threadId":thread,"requestId":rid}),
                    );
                }
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
                if question
                    .as_ref()
                    .is_some_and(|(rid, _)| id.as_u64() == Some(*rid))
                {
                    let (_, tid) = question.take().unwrap();
                    let response = msg
                        .pointer("/result/answers")
                        .cloned()
                        .unwrap_or(Value::Null);
                    let text = format!("question answers: {response}");
                    agent_message(&thread, &tid, "question-answer", &[&text]);
                    notify(
                        "turn/completed",
                        json!({"threadId":thread,"turn":turn(&tid,"completed")}),
                    );
                    current = None;
                    continue;
                }
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
