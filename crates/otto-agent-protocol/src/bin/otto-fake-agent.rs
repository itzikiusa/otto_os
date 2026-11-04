//! Test double: a tiny ACP v1 agent over stdio (JSON-RPC 2.0, one message per
//! line), written against the ACP spec's wire shapes. Behaviour is chosen by
//! the prompt text:
//!
//! * `crash`      — exit 3 at once (mid-turn), after a line on stderr.
//! * `garbage`    — print a non-JSON line, then answer like echo.
//! * `ask`        — a `tool_call`, then `session/request_permission`; waits.
//! * `hang`       — one chunk, then waits for `session/cancel` → `cancelled`.
//! * `hang-hard`  — like `hang` but ignores `session/cancel`.
//! * `plan`       — a `plan` update, then `end_turn`.
//! * `fs`         — calls `fs/read_text_file` on the client and reports the error.
//! * `big:<n>`    — `n` one-byte message chunks.
//! * anything else — a thought chunk + `echo: <text>` in two chunks.
//!
//! With `--refuse-session` every `session/new` fails with `-32000` (signed
//! out, no auth method to offer).
//!
//! `session/load` replays one history chunk before answering (clients must
//! not show it). stdin EOF ends the process (exit 0), like the real CLIs.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn out(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn next(lines: &mut impl Iterator<Item = std::io::Result<String>>) -> Option<Value> {
    loop {
        let line = lines.next()?.ok()?;
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            return Some(v);
        }
    }
}

fn method(v: &Value) -> &str {
    v.get("method").and_then(Value::as_str).unwrap_or_default()
}

fn update(sid: &str, update: Value) {
    out(
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":update}}),
    );
}

fn chunk(sid: &str, kind: &str, text: &str) {
    update(
        sid,
        json!({"sessionUpdate":kind,"content":{"type":"text","text":text}}),
    );
}

fn reply(id: &Value, result: Value) {
    out(json!({"jsonrpc":"2.0","id":id,"result":result}));
}

fn main() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut asks = 0u32;
    let refuse = std::env::args().any(|a| a == "--refuse-session");
    while let Some(msg) = next(&mut lines) {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match method(&msg) {
            "initialize" => reply(
                &id,
                json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
            ),
            "session/new" if refuse => out(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"Authentication required"}}),
            ),
            "session/new" => reply(
                &id,
                json!({"sessionId": format!("fake-{}", std::process::id())}),
            ),
            "session/load" => {
                let sid = params["sessionId"].as_str().unwrap_or_default().to_string();
                chunk(&sid, "agent_message_chunk", "(history)");
                reply(&id, json!({}));
            }
            "session/prompt" => {
                let sid = params["sessionId"].as_str().unwrap_or_default().to_string();
                let text = params["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let done = |stop: &str| reply(&id, json!({"stopReason":stop}));
                match text.as_str() {
                    "crash" => {
                        eprintln!("fake agent: crashing on purpose");
                        std::process::exit(3);
                    }
                    "ask" => {
                        asks += 1;
                        let rid = format!("perm-{asks}");
                        update(
                            &sid,
                            json!({"sessionUpdate":"tool_call","toolCallId":"c1","title":"ls","kind":"execute","status":"pending","rawInput":{"command":"ls"}}),
                        );
                        out(
                            json!({"jsonrpc":"2.0","id":rid,"method":"session/request_permission","params":{
                            "sessionId":sid,
                            "toolCall":{"toolCallId":"c1","kind":"execute","title":"Run ls"},
                            "options":[
                                {"optionId":"a1","name":"Allow","kind":"allow_once"},
                                {"optionId":"a2","name":"Always allow","kind":"allow_always"},
                                {"optionId":"r1","name":"Deny","kind":"reject_once"}]}}),
                        );
                        loop {
                            let Some(m) = next(&mut lines) else { return };
                            if m.get("id") != Some(&json!(rid)) || m.get("result").is_none() {
                                continue; // session/cancel arrives too; the answer decides
                            }
                            let outcome = &m["result"]["outcome"];
                            match (outcome["outcome"].as_str(), outcome["optionId"].as_str()) {
                                (Some("selected"), Some("a1" | "a2")) => {
                                    update(
                                        &sid,
                                        json!({"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"a.txt\n"}}]}),
                                    );
                                    chunk(&sid, "agent_message_chunk", "allowed");
                                    done("end_turn");
                                }
                                (Some("selected"), _) => {
                                    update(
                                        &sid,
                                        json!({"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"failed"}),
                                    );
                                    done("end_turn");
                                }
                                _ => done("cancelled"),
                            }
                            break;
                        }
                    }
                    "hang" | "hang-hard" => {
                        chunk(&sid, "agent_message_chunk", "working");
                        loop {
                            let Some(m) = next(&mut lines) else { return };
                            if method(&m) == "session/cancel" && text == "hang" {
                                done("cancelled");
                                break;
                            }
                        }
                    }
                    "plan" => {
                        update(
                            &sid,
                            json!({"sessionUpdate":"plan","entries":[
                            {"content":"read","priority":"high","status":"completed"},
                            {"content":"write","priority":"medium","status":"in_progress"}]}),
                        );
                        done("end_turn");
                    }
                    "fs" => {
                        out(
                            json!({"jsonrpc":"2.0","id":"fs1","method":"fs/read_text_file","params":{"sessionId":sid,"path":"/etc/hosts"}}),
                        );
                        let Some(m) = next(&mut lines) else { return };
                        let code = m["error"]["code"].as_i64().unwrap_or(0);
                        chunk(&sid, "agent_message_chunk", &format!("fs refused {code}"));
                        done("end_turn");
                    }
                    t if t.starts_with("big:") => {
                        let n: usize = t[4..].parse().unwrap_or(0);
                        for _ in 0..n {
                            chunk(&sid, "agent_message_chunk", "x");
                        }
                        done("end_turn");
                    }
                    t => {
                        if t == "garbage" {
                            println!("this is not json");
                        }
                        chunk(&sid, "agent_thought_chunk", "thinking");
                        chunk(&sid, "agent_message_chunk", "echo: ");
                        chunk(&sid, "agent_message_chunk", t);
                        done("end_turn");
                    }
                }
            }
            _ => {}
        }
    }
}
