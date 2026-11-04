//! Test double: `agy --print= --input-format stream-json --output-format
//! stream-json` (agy 1.4.12's event shapes). One turn per stdin line;
//! behaviour by prompt text:
//!
//! * `deny`  — a `run_command` tool step, then a SUCCESS `result` listing it
//!   in `denied_actions` (headless agy cannot ask).
//! * `fail`  — an ERROR `result`.
//! * `hang`  — one delta, then waits; SIGINT → ERROR `result` with
//!   `finished_reason: "interrupted"` and exit 1 (as agy does).
//! * anything else — `echo: <text>` as two deltas.
//!
//! `--conversation <id>` resumes that id (the `init` reports it). stdin EOF
//! ends the process.

use std::io::{BufRead, Write};
use std::sync::OnceLock;

use serde_json::{json, Value};

static INTERRUPTED: OnceLock<String> = OnceLock::new();

fn out(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn step(conv: &str, index: u32, state: &str, ty: &str, extra: Value) {
    let mut s =
        json!({"conversation_id": conv, "step_index": index, "state": state, "step_type": ty});
    s.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    out(json!({"event": "step_update", "step_update": s}));
}

fn result(conv: &str, status: &str, extra: Value) {
    let mut r = json!({"conversation_id": conv, "status": status, "num_turns": 1});
    r.as_object_mut()
        .unwrap()
        .extend(extra.as_object().cloned().unwrap_or_default());
    out(json!({"event": "result", "result": r}));
}

extern "C" fn on_sigint(_: libc::c_int) {
    if let Some(line) = INTERRUPTED.get() {
        // SAFETY: write(2) and _exit(2) are async-signal-safe; `line` is
        // initialised before the handler is installed and never mutated.
        unsafe {
            libc::write(1, line.as_ptr().cast(), line.len());
            libc::_exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let conv = args
        .iter()
        .position(|a| a == "--conversation")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| format!("fake-conv-{}", std::process::id()));
    let interrupted = json!({"event": "result", "result": {"conversation_id": conv,
        "status": "ERROR", "error": "interrupted", "finished_reason": "interrupted"}});
    let _ = INTERRUPTED.set(format!("{interrupted}\n"));
    // SAFETY: installs a handler that only calls async-signal-safe functions.
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
    }
    let mut index = 0u32;
    let mut started = false;
    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let text = msg
            .pointer("/message/content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !std::mem::replace(&mut started, true) {
            out(json!({"event": "init", "conversation_id": conv,
                       "init": {"cwd": ".", "permission_mode": "request-review"}}));
        }
        step(&conv, index, "DONE", "user_input", json!({}));
        index += 2;
        let at = index - 1;
        match text.as_str() {
            "deny" => {
                let info = json!({"tool_info": {"name": "run_command",
                    "parameters": {"CommandLine": "echo hi > x.txt"}}});
                step(&conv, at, "ACTIVE", "tool", info.clone());
                step(&conv, at, "DONE", "tool", info);
                result(
                    &conv,
                    "SUCCESS",
                    json!({"response": "", "denied_actions":
                        [{"action": "command", "display_name": "RunCommand"}]}),
                );
            }
            "fail" => result(&conv, "ERROR", json!({"error": "quota exhausted"})),
            "hang" => {
                step(
                    &conv,
                    at,
                    "ACTIVE",
                    "agent_response",
                    json!({"text_delta": "1\n"}),
                );
                std::thread::park(); // until SIGINT
            }
            _ => {
                let usage = json!({"input_tokens": 3, "output_tokens": 2,
                                   "thinking_tokens": 1, "cache_read_tokens": 2});
                step(
                    &conv,
                    at,
                    "ACTIVE",
                    "agent_response",
                    json!({"text_delta": "echo: "}),
                );
                step(
                    &conv,
                    at,
                    "DONE",
                    "agent_response",
                    json!({"text_delta": text, "usage": usage}),
                );
                result(
                    &conv,
                    "SUCCESS",
                    json!({"response": format!("echo: {text}"), "usage": usage}),
                );
            }
        }
    }
}
