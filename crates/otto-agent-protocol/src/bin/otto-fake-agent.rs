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
//! * `agent-mode <id>` — the agent switches its own mode
//!   (`current_mode_update`), then `end_turn`.
//! * `fs-read <path>` / `fs-write <path>` — `fs/read_text_file` /
//!   `fs/write_text_file` (content `written\n`) on the client, then
//!   `read: <content>` / `wrote` (or `refused <code>`).
//! * `term-keep <command>` — `terminal/create` only (left running), then
//!   `started <terminalId>`.
//! * `term <command>` — an `execute` tool call run on a client terminal:
//!   `terminal/create`, the terminal embedded in the call's content,
//!   `wait_for_exit`, `output`, `release`, then `term: <output>`.
//! * `tool`       — two `read` tool calls that complete on their own (no
//!   permission request; the work fold collapses), then `done`.
//! * `notice`     — a `notice` and a finished `compaction_update`, then `noted`.
//! * `image`      — an image (1×1 PNG) as the answer.
//! * `ask-never`  — a permission request offering `reject_always`; answers
//!   `answer: <optionId>`.
//! * `elicit-url <url>` — a url-mode `elicitation/create`; answers
//!   `url: <action>`.
//! * `edit <path>` — an `edit` tool call on `<path>` (relative to the cwd)
//!   that completes with a `diff` content block (`old` → `new` text), as
//!   real ACP agents report a file change, then `edited`.
//! * `big:<n>`    — `n` one-byte message chunks.
//! * `usage`      — a `usage_update` (context 1200 of 1048576, $0.25 so far)
//!   and a `session_info_update` naming the session, then `end_turn`.
//! * anything else — a thought chunk + `echo: <text>` in two chunks.
//!
//! With `--refuse-session` every `session/new` fails with `-32000` (signed
//! out, no auth method to offer). `--approval-mode <m>` sets the starting
//! mode; `session/new|load` advertise the modes `default`, `autoEdit`,
//! `accept-edits` and `plan` (not `yolo`), which `session/set_mode` switches
//! between, a `thought_level` config option (low … xhigh), and list two
//! slash commands (`available_commands_update`).
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

/// The agent's slash commands (`available_commands_update`).
fn commands(sid: &str) {
    update(
        sid,
        json!({"sessionUpdate":"available_commands_update","availableCommands":[
            {"name":"compact","description":"Summarize the conversation"},
            {"name":"review","description":"Review changes","input":{"hint":"what to focus on"}}]}),
    );
}

/// A request to the client; waits for its answer (other lines are dropped).
fn call(
    lines: &mut impl Iterator<Item = std::io::Result<String>>,
    id: &str,
    method: &str,
    params: Value,
) -> Option<Value> {
    out(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
    loop {
        let m = next(lines)?;
        if m.get("id") == Some(&json!(id)) && m.get("method").is_none() {
            return Some(m);
        }
    }
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
        {"id": "accept-edits", "name": "Accept edits"}, {"id": "plan", "name": "Plan"}]});
    // The `thought_level` option real agy / Muse catalogs advertise.
    let efforts = json!([{"id":"thought_level","category":"thought_level","type":"select",
        "currentValue":"medium","options":[{"value":"low"},{"value":"medium"},{"value":"high"},{"value":"xhigh"}]}]);
    let mut model = "none".to_string();
    let mut effort = "none".to_string();
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
                let sid = format!("fake-{}", std::process::id());
                reply(
                    &id,
                    json!({"sessionId": sid, "modes": modes, "configOptions": efforts}),
                );
                commands(&sid);
            }
            "session/set_model" => {
                model = params["modelId"].as_str().unwrap_or_default().to_string();
                reply(&id, json!({}));
            }
            "session/set_config_option" => {
                let value = params["value"].as_str().unwrap_or_default().to_string();
                match params["configId"].as_str() {
                    Some("model") => model = value,
                    Some("thought_level") => effort = value,
                    _ => {}
                }
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
                commands(&sid);
            }
            "session/prompt" => {
                let sid = params["sessionId"].as_str().unwrap_or_default().to_string();
                let text = params["prompt"]
                    .as_array()
                    .and_then(|b| b.iter().find_map(|b| b["text"].as_str()))
                    .unwrap_or_default()
                    .to_string();
                // Otto's delegation rules ride a thread's first prompt (runtime::delegation).
                let text = match text.split_once("</otto_instructions>\n\n") {
                    Some((_, t)) => t.to_string(),
                    None => text,
                };
                // A subagent's task header (runtime::delegation): act on the task.
                let text = match text.split_once("]\n\n") {
                    Some((h, t)) if h.starts_with("[Subagent task from thread ") => t.to_string(),
                    _ => text,
                };
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
                    "effort" => {
                        chunk(&sid, "agent_message_chunk", &format!("effort: {effort}"));
                        done("end_turn");
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
                    "usage" => {
                        update(
                            &sid,
                            json!({"sessionUpdate":"usage_update","used":1200,"size":1048576,
                                "cost":{"amount":0.25,"currency":"USD"}}),
                        );
                        update(
                            &sid,
                            json!({"sessionUpdate":"session_info_update","title":"Named by the agent"}),
                        );
                        chunk(&sid, "agent_message_chunk", "counted");
                        done("end_turn");
                    }
                    "hold-chunk" => {
                        chunk(&sid, "agent_message_chunk", "part1");
                        chunk(&sid, "agent_message_chunk", "part2");
                        loop {
                            let Some(m) = next(&mut lines) else { return };
                            if method(&m) == "session/cancel" {
                                done("cancelled");
                                break;
                            }
                        }
                    }
                    "timed-think" => {
                        chunk(&sid, "agent_thought_chunk", "step 1 ");
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        chunk(&sid, "agent_thought_chunk", "step 2");
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        chunk(&sid, "agent_message_chunk", "answer 1 ");
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        chunk(&sid, "agent_message_chunk", "answer 2");
                        std::thread::sleep(std::time::Duration::from_millis(25));
                        done("end_turn");
                    }
                    "schedule" => {
                        update(
                            &sid,
                            json!({"sessionUpdate":"tool_call","toolCallId":"sc1","title":"schedule","kind":"other","status":"pending","rawInput":{"DurationSeconds":10,"Prompt":"check status"}}),
                        );
                        update(
                            &sid,
                            json!({"sessionUpdate":"tool_call_update","toolCallId":"sc1","status":"completed"}),
                        );
                        chunk(&sid, "agent_message_chunk", "scheduled");
                        done("end_turn");
                        let bg_sid = sid.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(40));
                            chunk(&bg_sid, "agent_thought_chunk", "timer fired");
                            chunk(&bg_sid, "agent_message_chunk", "autonomous report 1");
                            std::thread::sleep(std::time::Duration::from_millis(180));
                            chunk(&bg_sid, "agent_message_chunk", "autonomous report 2");
                        });
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
                    t if t.starts_with("agent-mode ") => {
                        mode = t[11..].trim().to_string();
                        update(
                            &sid,
                            json!({"sessionUpdate":"current_mode_update","currentModeId":mode}),
                        );
                        done("end_turn");
                    }
                    t if t.starts_with("fs-")
                        || t.starts_with("term ")
                        || t.starts_with("term-keep ") =>
                    {
                        if client_turn(&mut lines, &id, &sid, t).is_none() {
                            return;
                        }
                    }
                    t @ ("tool" | "notice" | "image" | "ask-never") => {
                        if more_turn(&mut lines, &id, &sid, t).is_none() {
                            return;
                        }
                    }
                    t if t.starts_with("elicit-url ") => {
                        if more_turn(&mut lines, &id, &sid, t).is_none() {
                            return;
                        }
                    }
                    t if t.starts_with("edit ") => {
                        // ACP paths are absolute; the fake runs in the session cwd.
                        let path = std::env::current_dir()
                            .map(|d| d.join(t[5..].trim()).display().to_string())
                            .unwrap_or_default();
                        let path = path.as_str();
                        update(
                            &sid,
                            json!({"sessionUpdate":"tool_call","toolCallId":"e1","title":format!("Edit {path}"),"kind":"edit","status":"in_progress","locations":[{"path":path}]}),
                        );
                        update(
                            &sid,
                            json!({"sessionUpdate":"tool_call_update","toolCallId":"e1","status":"completed","locations":[{"path":path}],
                                "content":[{"type":"diff","path":path,"oldText":"old line\n","newText":"new line\n"}]}),
                        );
                        chunk(&sid, "agent_message_chunk", "edited");
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

/// The `fs-read` / `fs-write` / `term` prompts: client methods Otto serves.
fn client_turn(
    lines: &mut impl Iterator<Item = std::io::Result<String>>,
    id: &Value,
    sid: &str,
    text: &str,
) -> Option<()> {
    let sid = sid.to_string();
    let done = |stop: &str| reply(id, json!({"stopReason":stop}));
    let mut lines = lines;
    match text {
        // A terminal left running: created, never waited on or released.
        t if t.starts_with("term-keep ") => {
            let m = call(
                lines,
                "tk",
                "terminal/create",
                json!({"sessionId":sid,"command":t[10..].to_string()}),
            )?;
            let tid = m["result"]["terminalId"]
                .as_str()
                .unwrap_or("none")
                .to_string();
            chunk(&sid, "agent_message_chunk", &format!("started {tid}"));
            done("end_turn");
        }
        t if t.starts_with("fs-read ") || t.starts_with("fs-write ") => {
            let path = std::env::current_dir()
                .map(|d| {
                    d.join(t[t.find(' ').unwrap() + 1..].trim())
                        .display()
                        .to_string()
                })
                .unwrap_or_default();
            let read = t.starts_with("fs-read ");
            let (method, params) = if read {
                ("fs/read_text_file", json!({"sessionId":sid,"path":path}))
            } else {
                (
                    "fs/write_text_file",
                    json!({"sessionId":sid,"path":path,"content":"written\n"}),
                )
            };
            let m = call(&mut lines, "fs1", method, params)?;
            let text = match (&m["error"]["code"], read) {
                (Value::Number(c), _) => format!("refused {c}"),
                (_, true) => format!(
                    "read: {}",
                    m["result"]["content"].as_str().unwrap_or_default()
                ),
                (_, false) => "wrote".into(),
            };
            chunk(&sid, "agent_message_chunk", &text);
            done("end_turn");
        }
        t if t.starts_with("term ") => {
            let command = t[5..].to_string();
            update(
                &sid,
                json!({"sessionUpdate":"tool_call","toolCallId":"x1","title":command,"kind":"execute","status":"in_progress","rawInput":{"command":command}}),
            );
            let m = call(
                &mut lines,
                "t1",
                "terminal/create",
                json!({"sessionId":sid,"command":command}),
            )?;
            let Some(tid) = m["result"]["terminalId"].as_str().map(str::to_string) else {
                chunk(
                    &sid,
                    "agent_message_chunk",
                    &format!("refused {}", m["error"]["code"]),
                );
                update(
                    &sid,
                    json!({"sessionUpdate":"tool_call_update","toolCallId":"x1","status":"failed"}),
                );
                done("end_turn");
                return Some(());
            };
            update(
                &sid,
                json!({"sessionUpdate":"tool_call_update","toolCallId":"x1","content":[{"type":"terminal","terminalId":tid}]}),
            );
            let ids = json!({"sessionId":sid,"terminalId":tid});
            call(&mut lines, "t2", "terminal/wait_for_exit", ids.clone());
            let o = call(&mut lines, "t3", "terminal/output", ids.clone())?;
            call(&mut lines, "t4", "terminal/release", ids);
            let output = o["result"]["output"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            update(
                &sid,
                json!({"sessionUpdate":"tool_call_update","toolCallId":"x1","status":"completed"}),
            );
            chunk(
                &sid,
                "agent_message_chunk",
                &format!("term: {}", output.trim()),
            );
            done("end_turn");
        }
        _ => {}
    }
    Some(())
}

/// The `tool` / `notice` / `image` / `ask-never` / `elicit-url` prompts.
fn more_turn(
    lines: &mut impl Iterator<Item = std::io::Result<String>>,
    id: &Value,
    sid: &str,
    text: &str,
) -> Option<()> {
    let done = |stop: &str| reply(id, json!({"stopReason":stop}));
    match text {
        "tool" => {
            for (c, f) in [("t1", "README.md"), ("t2", "Cargo.toml")] {
                update(
                    sid,
                    json!({"sessionUpdate":"tool_call","toolCallId":c,"title":format!("Read {f}"),"kind":"read","status":"in_progress","rawInput":{"path":f}}),
                );
                update(
                    sid,
                    json!({"sessionUpdate":"tool_call_update","toolCallId":c,"status":"completed","content":[{"type":"content","content":{"type":"text","text":"hello\n"}}]}),
                );
            }
            chunk(sid, "agent_message_chunk", "done");
        }
        "notice" => {
            update(
                sid,
                json!({"sessionUpdate":"notice","severity":"warning","title":"Rate limited","description":"retrying in 5s"}),
            );
            update(
                sid,
                json!({"sessionUpdate":"compaction_update","compactionId":"k1","status":"completed"}),
            );
            chunk(sid, "agent_message_chunk", "noted");
        }
        "image" => {
            update(
                sid,
                json!({"sessionUpdate":"agent_message_chunk","content":{"type":"image","mimeType":"image/png",
                    "data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="}}),
            );
        }
        "ask-never" => {
            update(
                sid,
                json!({"sessionUpdate":"tool_call","toolCallId":"n1","title":"rm -rf build","kind":"execute","status":"pending","rawInput":{"command":"rm -rf build"}}),
            );
            let m = call(
                lines,
                "pn",
                "session/request_permission",
                json!({"sessionId":sid,"toolCall":{"toolCallId":"n1","kind":"execute","title":"rm -rf build"},
                    "options":[{"optionId":"a1","name":"Allow","kind":"allow_once"},
                        {"optionId":"r1","name":"Deny","kind":"reject_once"},
                        {"optionId":"never","name":"Always reject","kind":"reject_always"}]}),
            )?;
            let picked = m["result"]["outcome"]["optionId"]
                .as_str()
                .unwrap_or("cancelled")
                .to_string();
            update(
                sid,
                json!({"sessionUpdate":"tool_call_update","toolCallId":"n1","status":"failed"}),
            );
            chunk(sid, "agent_message_chunk", &format!("answer: {picked}"));
        }
        t => {
            let url = t.trim_start_matches("elicit-url ").to_string();
            let m = call(
                lines,
                "eu",
                "elicitation/create",
                json!({"sessionId":sid,"mode":"url","elicitationId":"e1","url":url,"message":"Sign in to continue"}),
            )?;
            let action = m["result"]["action"]
                .as_str()
                .unwrap_or("error")
                .to_string();
            chunk(sid, "agent_message_chunk", &format!("url: {action}"));
        }
    }
    done("end_turn");
    Some(())
}
