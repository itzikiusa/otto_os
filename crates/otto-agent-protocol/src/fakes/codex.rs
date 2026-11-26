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
//! A prompt with `OTTO_FAKE:` lines acts out that script instead
//! (src/fake_script.rs; no `child`: this fake has no MCP client). Its
//! `background <ms>` is a background subagent (`collabAgentToolCall`
//! spawnAgent) whose thread finishes `<ms>` later, then Codex's own turn.
//!
//! `thread/resume` succeeds for ids this binary minted (`fake-th-*`) and
//! fails otherwise. stdin EOF ends the process.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

#[path = "../fake_script.rs"]
mod fake_script;

use fake_script::{FailClass, Turn, Wire};

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

/// A script's follow-up turn, owed when its background subagent finishes.
struct Owed {
    at: Instant,
    child: String,
    rest: Turn,
}

/// The first string anywhere in `v` (an answer's text, whatever its nesting).
fn first_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => a.iter().find_map(first_string),
        Value::Object(o) => o.values().find_map(first_string),
        _ => None,
    }
}

struct CodexWire<'a> {
    rx: &'a mpsc::Receiver<In>,
    thread: &'a str,
    tid: String,
    next_server_id: &'a mut u64,
    owed: &'a mut Vec<Owed>,
    items: u32,
}

impl CodexWire<'_> {
    fn complete(&self, status: &str) {
        notify(
            "turn/completed",
            json!({"threadId": self.thread, "turn": turn(&self.tid, status)}),
        );
    }

    /// The next message for this turn: a response to `rid` (`Ok`), or
    /// `Err(true)` when the turn was interrupted, `Err(false)` on timeout.
    fn wait_for(&mut self, rid: Option<u64>, until: Option<Instant>) -> Result<Value, bool> {
        loop {
            let msg = match until {
                Some(at) => match self
                    .rx
                    .recv_timeout(at.saturating_duration_since(Instant::now()))
                {
                    Ok(m) => m,
                    Err(mpsc::RecvTimeoutError::Timeout) => return Err(false),
                    Err(mpsc::RecvTimeoutError::Disconnected) => std::process::exit(0),
                },
                None => self.rx.recv().unwrap_or(In::Eof),
            };
            let In::Line(msg) = msg else {
                std::process::exit(0)
            };
            let id = msg.get("id").cloned().unwrap_or(Value::Null);
            match msg.get("method").and_then(Value::as_str) {
                Some("turn/interrupt") => {
                    out(json!({"id": id, "result": {}}));
                    if let Some(rid) = rid {
                        notify(
                            "serverRequest/resolved",
                            json!({"threadId": self.thread, "requestId": rid}),
                        );
                    }
                    self.complete("interrupted");
                    return Err(true);
                }
                Some(_) if !id.is_null() => out(
                    json!({"id": id, "error": {"code": -32600, "message": "busy with a scripted turn"}}),
                ),
                None if rid.is_some() && id.as_u64() == rid => return Ok(msg),
                _ => {}
            }
        }
    }

    fn request(&mut self, method: &str, params: Value) -> u64 {
        *self.next_server_id += 1;
        let rid = *self.next_server_id;
        out(json!({"id": rid, "method": method, "params": params}));
        rid
    }
}

impl Wire for CodexWire<'_> {
    fn say(&mut self, text: &str) {
        self.items += 1;
        agent_message(
            self.thread,
            &self.tid,
            &format!("msg-f{}", self.items),
            &[text],
        );
    }

    fn wait(&mut self, d: Duration) -> bool {
        self.wait_for(None, Some(Instant::now() + d)) == Err(false)
    }

    fn ask(&mut self, question: &str) -> Option<String> {
        let rid = self.request(
            "item/tool/requestUserInput",
            json!({"threadId": self.thread, "turnId": self.tid, "itemId": "question-f",
                "questions": [{"id": "q1", "header": "Question", "question": question,
                    "isOther": true, "options": null}]}),
        );
        let m = self.wait_for(Some(rid), None).ok()?;
        Some(first_string(&m["result"]["answers"]).unwrap_or_default())
    }

    fn approve(&mut self, tool: &str) -> Option<bool> {
        let rid = self.request(
            "item/commandExecution/requestApproval",
            json!({"threadId": self.thread, "turnId": self.tid, "itemId": "cmd-f",
                "startedAtMs": 0, "command": tool, "reason": "the script asks"}),
        );
        let m = self.wait_for(Some(rid), None).ok()?;
        match m.pointer("/result/decision").and_then(Value::as_str) {
            Some("accept" | "acceptForSession") => Some(true),
            Some("decline") => Some(false),
            _ => {
                self.complete("interrupted");
                None
            }
        }
    }

    fn usage(&mut self, input: u64, output: u64) {
        let u = json!({"inputTokens": input, "outputTokens": output, "cachedInputTokens": 0,
            "reasoningOutputTokens": 0, "totalTokens": input + output});
        notify(
            "thread/tokenUsage/updated",
            json!({"threadId": self.thread, "turnId": self.tid,
                "tokenUsage": {"last": u, "total": u, "modelContextWindow": 1000000}}),
        );
    }

    fn fail(&mut self, class: FailClass, message: &str) {
        let info = match class {
            FailClass::UsageLimit => "usageLimitExceeded",
            FailClass::Auth => "unauthorized",
            _ => "internalServerError",
        };
        let mut t = turn(&self.tid, "failed");
        t["error"] = json!({"message": message, "codexErrorInfo": info});
        notify(
            "turn/completed",
            json!({"threadId": self.thread, "turn": t}),
        );
    }

    fn background(&mut self, after: Duration, rest: Turn) {
        self.items += 1;
        let child = format!(
            "fake-bg-th-{}-{}",
            std::process::id(),
            self.owed.len() + self.items as usize
        );
        for status in ["inProgress", "completed"] {
            notify(
                if status == "completed" {
                    "item/completed"
                } else {
                    "item/started"
                },
                json!({"threadId": self.thread, "turnId": self.tid, "item": {
                    "type": "collabAgentToolCall", "id": format!("col-f{}", self.items),
                    "tool": "spawnAgent", "role": "background", "prompt": "background work",
                    "receiverThreadIds": [child], "status": status}}),
            );
        }
        self.say("Working in the background.");
        self.complete("completed");
        self.owed.push(Owed {
            at: Instant::now() + after,
            child,
            rest,
        });
    }

    fn mcp(&mut self, _tool: &str, _args: Value) -> Result<Value, String> {
        Err("no MCP client".into())
    }

    fn done(&mut self) {
        self.complete("completed");
    }
}

/// The turn's text as the fake acts on it: without Otto's delegation rules
/// (runtime::delegation; `rules?` answers whether they came, after a handoff
/// too) and without a subagent's task header.
fn prompt_text(input: &[Value]) -> String {
    let text = input
        .iter()
        .find_map(|i| i["text"].as_str())
        .unwrap_or_default()
        .to_string();
    let had_rules = text.contains("</otto_instructions>\n\n");
    let text = match text.split_once("</otto_instructions>\n\n") {
        Some((_, t)) => t.to_string(),
        None => text,
    };
    let last = text.rsplit_once("\n\n").map_or(text.as_str(), |(_, t)| t);
    if last == "rules?" {
        return format!("rules: {}", if had_rules { "yes" } else { "no" });
    }
    match text.split_once("]\n\n") {
        Some((h, t)) if h.starts_with("[Subagent task from thread ") => t.to_string(),
        _ => text,
    }
}

/// `stream:<n>:<ms>[:<tools>]`: `c0 `…`c<n-1> `, one delta per <ms>, with
/// <tools> completed commands spread through it (perf specs).
fn stream_turn(thread: &str, tid: &str, t: &str) {
    let mut f = t[7..].split(':');
    let n: usize = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let ms: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let tools: usize = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    for k in 0..tools {
        let mut c = command(thread, tid, "completed", Some("ok\n"));
        c["item"]["id"] = json!(format!("cmd-s{k}"));
        c["item"]["command"] = json!(format!("rg step_{k} src"));
        notify("item/started", c.clone());
        notify("item/completed", c);
    }
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
        json!({"threadId": thread, "turn": turn(tid, "completed")}),
    );
}

/// `tool image`: a completed MCP result with text, metadata and two PNGs.
fn tool_image_turn(thread: &str, tid: &str) {
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
        json!({"threadId":thread,"turn":turn(tid,"completed")}),
    );
}

/// `make image`: an image it generated, as a file on disk.
fn make_image_turn(thread: &str, tid: &str) {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=")
            .unwrap_or_default();
    let path = std::env::temp_dir().join(format!("fake-codex-{}-{tid}.png", std::process::id()));
    let _ = std::fs::write(&path, png);
    notify(
        "item/completed",
        json!({"threadId": thread, "turnId": tid, "item": {"type": "imageGeneration",
                "id": "ig-1", "status": "completed", "revisedPrompt": "a red dot",
                "savedPath": path, "inputImages": []}}),
    );
    agent_message(thread, tid, "msg-i", &["made it"]);
    notify(
        "turn/completed",
        json!({"threadId": thread, "turn": turn(tid, "completed")}),
    );
}

fn main() {
    fake_script::log_start();
    let cfg = fake_script::Config::from_env("codex", &["child", "finish-child"]);
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut owed: Vec<Owed> = Vec::new();
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
        let wake = [waiting_until, owed.iter().map(|o| o.at).min()]
            .into_iter()
            .flatten()
            .min();
        let msg = match wake {
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
            // A script's background subagent finished: Codex's own turn goes on.
            if let Some(i) = owed.iter().position(|o| o.at <= Instant::now()) {
                if current.is_none() {
                    let o = owed.remove(i);
                    let ct = format!("{}-turn", o.child);
                    notify(
                        "turn/started",
                        json!({"threadId": o.child, "turn": turn(&ct, "inProgress")}),
                    );
                    notify(
                        "turn/completed",
                        json!({"threadId": o.child, "turn": turn(&ct, "completed")}),
                    );
                    turns += 1;
                    let tid = format!("turn-{turns}");
                    notify(
                        "turn/started",
                        json!({"threadId": thread, "turn": turn(&tid, "inProgress")}),
                    );
                    let mut rest = o.rest;
                    rest.resumed(turns);
                    let mut wire = CodexWire {
                        rx: &rx,
                        thread: &thread,
                        tid,
                        next_server_id: &mut next_server_id,
                        owed: &mut owed,
                        items: turns * 100,
                    };
                    wire.say("Background agent reported.");
                    fake_script::drive(rest, &mut wire);
                }
                continue;
            }
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
                let text = prompt_text(&input);
                let extras = echo_extras(&params, &input);
                if params["model"].is_string() || params["effort"].is_string() {
                    fake_script::argv_log(
                        json!({"model": params["model"], "effort": params["effort"]}),
                    );
                }
                if let Some(script) = Turn::start(&text, &cwd, &thread, turns, &cfg) {
                    let mut wire = CodexWire {
                        rx: &rx,
                        thread: &thread,
                        tid,
                        next_server_id: &mut next_server_id,
                        owed: &mut owed,
                        items: turns * 100,
                    };
                    fake_script::drive(script, &mut wire);
                    current = None;
                    continue;
                }
                match text.as_str() {
                    // `stream:<n>:<ms>[:<tools>]`: `c0 `…`c<n-1> `, one delta per <ms>,
                    // with <tools> completed commands spread through it (perf specs).
                    t if t.starts_with("stream:") => {
                        stream_turn(&thread, &tid, t);
                        current = None;
                    }
                    "tool image" => {
                        tool_image_turn(&thread, &tid);
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
                        make_image_turn(&thread, &tid);
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
