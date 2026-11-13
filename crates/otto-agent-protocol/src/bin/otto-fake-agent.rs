//! Test double: a tiny ACP v1 agent over stdio (JSON-RPC 2.0, one message per
//! line), written against the ACP spec's wire shapes. Behaviour is chosen by
//! the prompt text:
//!
//! * `crash`      — exit 3 at once (mid-turn), after a line on stderr.
//! * `garbage`    — print a non-JSON line, then answer like echo.
//! * `ask`        — a `tool_call`, then `session/request_permission`; waits.
//! * `ask2`       — two `tool_call`s and both permission requests at once;
//!   each answer settles its tool (allowed → `completed`, else `failed` with
//!   content), the turn ends when both are answered.
//! * `mode`       — `mode: <current mode id>`.
//! * `model`      — `model: <the session/set_model id, or none>`; the
//!   response carries a draft `usage` (5 in + 2 cached, 3 out).
//! * `system`     — `system: <the _meta.antigravity.systemPrompt.append of
//!   session/new|load, or none>`.
//! * `hang`       — one chunk, then waits for `session/cancel` → `cancelled`.
//! * `hang-hard`  — like `hang` but ignores `session/cancel`.
//! * `plan`       — a `plan` update, then `end_turn`.
//! * `fs`         — calls `fs/read_text_file` on the client and reports the error.
//! * `big:<n>`    — `n` one-byte message chunks.
//! * anything else — a thought chunk + `echo: <text>` in two chunks.
//!
//! With `--refuse-session` every `session/new` fails with `-32000` (signed
//! out, no auth method to offer). `--approval-mode <m>` sets the starting
//! mode; `session/new|load` advertise the modes `default`, `autoEdit` and
//! `accept-edits` (not `yolo`), which `session/set_mode` switches between.
//! `initialize` advertises `promptCapabilities.embeddedContext` (no images).
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
    let args: Vec<String> = std::env::args().collect();
    let refuse = args.iter().any(|a| a == "--refuse-session");
    let mut mode = args
        .windows(2)
        .find(|w| w[0] == "--approval-mode")
        .map_or("default".to_string(), |w| w[1].clone());
    let modes = json!({"currentModeId": "default", "availableModes": [
        {"id": "default", "name": "Default"}, {"id": "autoEdit", "name": "Auto edit"},
        {"id": "accept-edits", "name": "Accept edits"}]});
    let mut model = "none".to_string();
    let mut system = "none".to_string();
    let system_of = |p: &Value| {
        p.pointer("/_meta/antigravity/systemPrompt/append")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_string()
    };
    while let Some(msg) = next(&mut lines) {
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match method(&msg) {
            "initialize" => reply(
                &id,
                json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true,
                    "promptCapabilities":{"embeddedContext":true}}}),
            ),
            "session/new" if refuse => out(
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":"Authentication required"}}),
            ),
            "session/new" => {
                system = system_of(&params);
                reply(
                    &id,
                    json!({"sessionId": format!("fake-{}", std::process::id()), "modes": modes}),
                )
            }
            "session/set_model" => {
                model = params["modelId"].as_str().unwrap_or_default().to_string();
                reply(&id, json!({}));
            }
            "session/set_mode" => {
                mode = params["modeId"].as_str().unwrap_or_default().to_string();
                reply(&id, json!({}));
            }
            "session/load" => {
                system = system_of(&params);
                let sid = params["sessionId"].as_str().unwrap_or_default().to_string();
                chunk(&sid, "agent_message_chunk", "(history)");
                reply(&id, json!({"modes": modes}));
            }
            "session/prompt" => {
                let sid = params["sessionId"].as_str().unwrap_or_default().to_string();
                let text = params["prompt"]
                    .as_array()
                    .and_then(|b| b.iter().find_map(|b| b["text"].as_str()))
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
                    "mode" => {
                        chunk(&sid, "agent_message_chunk", &format!("mode: {mode}"));
                        done("end_turn");
                    }
                    "model" => {
                        chunk(&sid, "agent_message_chunk", &format!("model: {model}"));
                        reply(
                            &id,
                            json!({"stopReason":"end_turn","usage":{"inputTokens":5,
                                "cachedReadTokens":2,"outputTokens":3,"thoughtTokens":1}}),
                        );
                    }
                    "system" => {
                        chunk(&sid, "agent_message_chunk", &format!("system: {system}"));
                        done("end_turn");
                    }
                    "ask2" => {
                        for c in ["c1", "c2"] {
                            update(
                                &sid,
                                json!({"sessionUpdate":"tool_call","toolCallId":c,"title":c,"kind":"fetch","status":"pending"}),
                            );
                        }
                        for c in ["c1", "c2"] {
                            out(
                                json!({"jsonrpc":"2.0","id":format!("p-{c}"),"method":"session/request_permission","params":{
                                "sessionId":sid,
                                "toolCall":{"toolCallId":c,"kind":"fetch","title":c},
                                "options":[
                                    {"optionId":"a1","name":"Allow","kind":"allow_once"},
                                    {"optionId":"r1","name":"Deny","kind":"reject_once"}]}}),
                            );
                        }
                        let mut open = 2;
                        while open > 0 {
                            let Some(m) = next(&mut lines) else { return };
                            let Some(c) = m["id"].as_str().and_then(|i| i.strip_prefix("p-"))
                            else {
                                continue;
                            };
                            open -= 1;
                            let allowed = m["result"]["outcome"]["optionId"] == "a1";
                            let (status, text) = if allowed {
                                ("completed", "ok")
                            } else {
                                ("failed", "denied by the user")
                            };
                            update(
                                &sid,
                                json!({"sessionUpdate":"tool_call_update","toolCallId":c,"status":status,
                                "content":[{"type":"content","content":{"type":"text","text":text}}]}),
                            );
                        }
                        done("end_turn");
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
                    // `stream:<n>:<ms>`: `c0 `…`c<n-1> `, one chunk per <ms>.
                    t if t.starts_with("stream:") => {
                        let mut f = t[7..].split(':');
                        let n: usize = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        let ms: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
                        for i in 0..n {
                            chunk(&sid, "agent_message_chunk", &format!("c{i} "));
                            std::thread::sleep(std::time::Duration::from_millis(ms));
                        }
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
