//! Test double: a tiny `pi --mode rpc` (pi 1.x's JSONL RPC: `get_state`,
//! `get_commands`, `prompt`, `steer`, `set_thinking_level`, `abort`; session
//! events up to `agent_settled`). It acts out the `OTTO_FAKE:` scripts
//! (src/fake_script.rs) — pi has no approvals, no questions and no background
//! work, so `ask`, `approve`, `background` and `child` answer "unsupported".
//! Any other prompt answers `echo: <text>`.
//!
//! `--session <id>` resumes only an id this binary minted (`fake-pi-*`);
//! otherwise it exits 1 as pi does for an unknown session. stdin EOF ends it.

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

fn respond(id: &Value, command: &str, data: Value) {
    out(json!({"type": "response", "id": id, "command": command, "success": true, "data": data}));
}

fn assistant(text: &str) {
    out(json!({"type": "message_start", "message": {"role": "assistant"}}));
    out(json!({"type": "message_update", "assistantMessageEvent": {"type": "text_start"}}));
    out(
        json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "delta": text}}),
    );
    out(
        json!({"type": "message_end", "message": {"role": "assistant",
        "content": [{"type": "text", "text": text}]}}),
    );
}

/// The run ends: `agent_end` with the last message's stop reason, then
/// `agent_settled` (pi 1.x will not continue on its own).
fn settle(stop: &str, error: Option<&str>) {
    let mut last = json!({"role": "assistant", "stopReason": stop});
    if let Some(e) = error {
        last["errorMessage"] = json!(e);
    }
    out(json!({"type": "agent_end", "messages": [last], "willRetry": false}));
    out(json!({"type": "agent_settled", "aborted": stop == "aborted"}));
}

struct PiWire<'a> {
    rx: &'a mpsc::Receiver<Option<Value>>,
}

impl Wire for PiWire<'_> {
    fn say(&mut self, text: &str) {
        assistant(text);
    }

    fn wait(&mut self, d: Duration) -> bool {
        let until = Instant::now() + d;
        loop {
            match self
                .rx
                .recv_timeout(until.saturating_duration_since(Instant::now()))
            {
                Err(mpsc::RecvTimeoutError::Timeout) => return true,
                Err(mpsc::RecvTimeoutError::Disconnected) | Ok(None) => std::process::exit(0),
                Ok(Some(cmd)) => {
                    let kind = cmd["type"].as_str().unwrap_or_default().to_string();
                    respond(&cmd["id"], &kind, json!({}));
                    if kind == "abort" {
                        settle("aborted", None);
                        return false;
                    }
                }
            }
        }
    }

    fn ask(&mut self, _question: &str) -> Option<String> {
        unreachable!("pi has no questions")
    }

    fn approve(&mut self, _tool: &str) -> Option<bool> {
        unreachable!("pi has no approvals")
    }

    fn usage(&mut self, input: u64, output: u64) {
        out(json!({"type": "message_start", "message": {"role": "assistant"}}));
        out(
            json!({"type": "message_end", "message": {"role": "assistant", "content": [],
            "usage": {"input": input, "output": output, "cacheRead": 0, "cacheWrite": 0}}}),
        );
    }

    fn fail(&mut self, class: FailClass, message: &str) {
        let _ = class;
        settle("error", Some(message));
    }

    fn background(&mut self, _after: Duration, _rest: Turn) {
        unreachable!("pi has no background work")
    }

    fn mcp(&mut self, _tool: &str, _args: Value) -> Result<Value, String> {
        Err("no MCP client".into())
    }

    fn done(&mut self) {
        settle("stop", None);
    }
}

fn main() {
    fake_script::log_start();
    let args: Vec<String> = std::env::args().collect();
    let session = match args
        .iter()
        .position(|a| a == "--session")
        .and_then(|i| args.get(i + 1))
    {
        Some(id) if id.starts_with("fake-pi-") => id.clone(),
        Some(id) => {
            eprintln!("No session found: {id}");
            std::process::exit(1);
        }
        None => format!(
            "fake-pi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ),
    };
    let cfg = fake_script::Config::from_env(
        "pi",
        &["ask", "approve", "background", "child", "finish-child"],
    );
    let cwd = std::env::current_dir().unwrap_or_default();
    let (tx, rx) = mpsc::channel::<Option<Value>>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(Some(v));
            }
        }
        let _ = tx.send(None);
    });
    let mut turns = 0u32;
    while let Ok(Some(cmd)) = rx.recv() {
        let id = cmd["id"].clone();
        match cmd["type"].as_str().unwrap_or_default() {
            "get_state" => respond(
                &id,
                "get_state",
                json!({"sessionId": session,
                    "model": {"provider": "fake", "id": "pi-fake", "contextWindow": 1000}}),
            ),
            "get_commands" => respond(&id, "get_commands", json!({"commands": []})),
            "set_thinking_level" => {
                fake_script::argv_log(json!({"effort": cmd["level"]}));
                respond(&id, "set_thinking_level", json!({}));
            }
            "prompt" => {
                turns += 1;
                respond(&id, "prompt", json!({}));
                out(json!({"type": "agent_start"}));
                let text = cmd["message"].as_str().unwrap_or_default();
                let mut wire = PiWire { rx: &rx };
                match Turn::start(text, &cwd, &session, turns, &cfg) {
                    Some(turn) => fake_script::drive(turn, &mut wire),
                    None => {
                        wire.say(&format!("echo: {text}"));
                        wire.done();
                    }
                }
            }
            other => respond(&id, other, json!({})),
        }
    }
}
