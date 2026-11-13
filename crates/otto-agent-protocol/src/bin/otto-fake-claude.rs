//! Test double: a tiny `claude` speaking the Agent SDK's stream-json over
//! stdio (`--input-format stream-json --output-format stream-json`), with
//! the frame shapes of Claude Code 2.1.318. Behaviour by prompt text:
//!
//! * `bg <secs>`   — a `run_in_background` Bash (`task_started`, the launch
//!   tool result naming a real output file, "tick 1\ntick 2"), "Waiting
//!   for it." and `result`; `<secs>` later the task's
//!   `task_notification`, then a turn of its own (lasting ~0.8 s):
//!   "Background done: ok".
//! * `agent <secs>` — a background `Agent` call: its `subagent` frames (a
//!   text and a `Read`), the "Async agent launched" result, `result`;
//!   `<secs>` later `task_notification` + an own turn "Agent reported".
//! * `slow <secs>` — "Working…", then waits `<secs>`; a user message arriving
//!   meanwhile (a steer) is answered in the same turn: "steered: <text>".
//! * `silent`      — emits nothing until interrupted.
//! * `stream <n> <ms>` — `c0 `…`c<n-1> ` as text deltas, one per `<ms>`.
//! * `ask`         — a `can_use_tool` request for Bash `ls`; the answer
//!   decides: "allowed" or "denied", then `result`.
//! * `escape`      — starts `sleep 600` in a process group of its own (as
//!   Claude Code does with some jobs), writes its pid to
//!   `.fake-claude/escaped.pid`, then answers like `echo`.
//! * anything else — `echo: <text>` as stream events + `assistant`.
//!
//! An `interrupt` control request ends the running turn as interrupted.
//! `--resume <id>` works only for a session this binary started in the same
//! cwd (`.fake-claude/<id>` exists); otherwise it exits 1 like the CLI
//! ("No conversation found"). A `.fake-claude/<id>.flaky` marker fails the
//! next resume once with a passing error (the marker is removed). stdin EOF
//! ends the process.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// `<tmp>/claude-<uid>/otto-fake/<session>/tasks/<task>.output` with two
/// lines of output, as Claude Code lays out a background command's output.
fn task_output_file(session: &str, task: &str) -> std::path::PathBuf {
    let tmp = std::env::var_os("CLAUDE_CODE_TMPDIR")
        .map_or_else(|| "/tmp".into(), std::path::PathBuf::from);
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = tmp.join(format!("claude-{uid}/otto-fake/{session}/tasks"));
    let file = dir.join(format!("{task}.output"));
    let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&file, "tick 1\ntick 2\n"));
    file
}

fn out(v: Value) {
    let mut o = std::io::stdout().lock();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn text_message(id: &str, text: &str) {
    out(
        json!({"type":"stream_event","event":{"type":"message_start","message":{"id":id}},"parent_tool_use_id":null}),
    );
    out(
        json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}),
    );
    out(
        json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}},"parent_tool_use_id":null}),
    );
    out(
        json!({"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null}),
    );
    out(
        json!({"type":"assistant","message":{"id":id,"content":[{"type":"text","text":text}]},"parent_tool_use_id":null}),
    );
}

fn result_ok() {
    out(
        json!({"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}),
    );
}

fn result_interrupted() {
    out(
        json!({"type":"result","subtype":"error_during_execution","is_error":true,"errors":["Request was interrupted"]}),
    );
}

fn user_text(msg: &Value) -> String {
    match msg.pointer("/message/content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

enum In {
    Line(Value),
    Eof,
}

/// What the fake owes later: a background task's completion.
struct Due {
    at: Instant,
    task_id: String,
    tool_use_id: String,
    agent: bool,
}

fn arg_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let store = std::path::Path::new(".fake-claude");
    let session = match arg_after(&args, "--resume") {
        Some(id) => {
            if !store.join(&id).exists() {
                eprintln!("No conversation found with session ID: {id}");
                std::process::exit(1);
            }
            if std::fs::remove_file(store.join(format!("{id}.flaky"))).is_ok() {
                eprintln!("API Error: Connection error.");
                std::process::exit(1);
            }
            id
        }
        None => {
            let id = arg_after(&args, "--session-id").unwrap_or_else(|| "fake-session".into());
            let _ = std::fs::create_dir_all(store);
            let _ = std::fs::write(store.join(&id), b"");
            id
        }
    };
    let (tx, rx) = mpsc::channel::<In>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(In::Line(v));
            }
        }
        let _ = tx.send(In::Eof);
    });
    let mut dues: Vec<Due> = Vec::new();
    let mut n = 0u32;
    // A turn that waits: (until, steerable) — `None` until = until interrupted.
    let mut waiting: Option<(Option<Instant>, bool)> = None;
    // The `can_use_tool` request an `ask` turn waits on.
    let mut asking: Option<String> = None;
    loop {
        let next_due = dues.iter().map(|d| d.at).min();
        let wake = [next_due, waiting.and_then(|w| w.0)]
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
        let now = Instant::now();
        let Some(msg) = msg else {
            if let Some((Some(until), _)) = waiting {
                if until <= now {
                    waiting = None;
                    result_ok();
                    continue;
                }
            }
            if waiting.is_some() {
                continue; // a background completion waits for the turn
            }
            if let Some(i) = dues.iter().position(|d| d.at <= now) {
                let d = dues.remove(i);
                out(
                    json!({"type":"system","subtype":"task_notification","task_id":d.task_id,
                    "tool_use_id":d.tool_use_id,"status":"completed","output_file":"",
                    "summary": if d.agent { "Found 2 open PRs" } else { "exit 0" }}),
                );
                // The notification reaches the model: a turn of its own.
                n += 1;
                let reply = if d.agent {
                    "Agent reported"
                } else {
                    "Background done: ok"
                };
                text_message(&format!("m{n}"), reply);
                // It lasts a moment (a message sent meanwhile must wait).
                waiting = Some((Some(Instant::now() + Duration::from_millis(800)), false));
            }
            continue;
        };
        let msg = match msg {
            In::Line(v) => v,
            In::Eof => return,
        };
        match msg.get("type").and_then(Value::as_str) {
            Some("control_response")
                if asking.is_some()
                    && msg.pointer("/response/request_id").and_then(Value::as_str)
                        == asking.as_deref() =>
            {
                asking = None;
                let allow = msg
                    .pointer("/response/response/behavior")
                    .and_then(Value::as_str)
                    == Some("allow");
                n += 1;
                text_message(&format!("m{n}"), if allow { "allowed" } else { "denied" });
                result_ok();
            }
            Some("control_request") => {
                let rid = msg.get("request_id").cloned().unwrap_or(Value::Null);
                let sub = msg.pointer("/request/subtype").and_then(Value::as_str);
                out(
                    json!({"type":"control_response","response":{"subtype":"success","request_id":rid,"response":{}}}),
                );
                if sub == Some("initialize") {
                    out(
                        json!({"type":"system","subtype":"init","session_id":session,"model":"fake-claude"}),
                    );
                }
                if sub == Some("interrupt") && waiting.take().is_some() {
                    result_interrupted();
                }
            }
            Some("user") => {
                let text = user_text(&msg);
                n += 1;
                let mid = format!("m{n}");
                if let Some((_, steerable)) = waiting {
                    if steerable {
                        text_message(&mid, &format!("steered: {text}"));
                    }
                    continue;
                }
                let mut words = text.split_whitespace();
                let cmd = words.next().unwrap_or_default();
                let secs: f64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(1.0);
                let later = now + Duration::from_secs_f64(secs);
                match cmd {
                    "bg" => {
                        let (tu, task) = (format!("tu{n}"), format!("b{n}"));
                        out(
                            json!({"type":"assistant","message":{"id":mid,"content":[{"type":"tool_use","id":tu,"name":"Bash","input":{"command":"sleep 300","run_in_background":true}}]},"parent_tool_use_id":null}),
                        );
                        out(
                            json!({"type":"system","subtype":"task_started","task_id":task,"tool_use_id":tu,"description":"sleep 300","task_type":"local_bash"}),
                        );
                        // Where Claude Code writes it (the daemon reads it back).
                        let file = task_output_file(&session, &task);
                        out(
                            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":tu,"content":format!("Command running in background with ID: {task}. Output is being written to: {}", file.display())}]},"parent_tool_use_id":null}),
                        );
                        text_message(&format!("{mid}b"), "Waiting for it.");
                        result_ok();
                        dues.push(Due {
                            at: later,
                            task_id: task,
                            tool_use_id: tu,
                            agent: false,
                        });
                    }
                    "agent" => {
                        let (tu, task) = (format!("tu{n}"), format!("a{n}"));
                        out(
                            json!({"type":"assistant","message":{"id":mid,"content":[{"type":"tool_use","id":tu,"name":"Agent","input":{"description":"PR sweep","prompt":"Sweep the open PRs","subagent_type":"general-purpose","run_in_background":true}}]},"parent_tool_use_id":null}),
                        );
                        out(
                            json!({"type":"system","subtype":"task_started","task_id":task,"tool_use_id":tu,"description":"PR sweep","task_type":"local_agent","subagent_type":"general-purpose","is_backgrounded":true}),
                        );
                        out(
                            json!({"type":"assistant","message":{"id":format!("{mid}s"),"content":[{"type":"text","text":"Looking at PRs"},{"type":"tool_use","id":format!("{tu}r"),"name":"Read","input":{"file_path":"/w/README.md"}}]},"parent_tool_use_id":tu}),
                        );
                        out(
                            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":format!("{tu}r"),"content":"# readme"}]},"parent_tool_use_id":tu}),
                        );
                        out(
                            json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":tu,"content":"Async agent launched successfully. agentId: x1 (internal ID)"}]},"parent_tool_use_id":null}),
                        );
                        text_message(
                            &format!("{mid}b"),
                            "I'm waiting for the PR sweep to come back.",
                        );
                        result_ok();
                        dues.push(Due {
                            at: later,
                            task_id: task,
                            tool_use_id: tu,
                            agent: true,
                        });
                    }
                    "stream" => {
                        let count: usize = secs as usize;
                        let ms: u64 = words.next().and_then(|w| w.parse().ok()).unwrap_or(0);
                        out(
                            json!({"type":"stream_event","event":{"type":"message_start","message":{"id":mid}},"parent_tool_use_id":null}),
                        );
                        out(
                            json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}),
                        );
                        let mut all = String::new();
                        for i in 0..count {
                            let d = format!("c{i} ");
                            all.push_str(&d);
                            out(
                                json!({"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":d}},"parent_tool_use_id":null}),
                            );
                            std::thread::sleep(Duration::from_millis(ms));
                        }
                        out(
                            json!({"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null}),
                        );
                        out(
                            json!({"type":"assistant","message":{"id":mid,"content":[{"type":"text","text":all}]},"parent_tool_use_id":null}),
                        );
                        result_ok();
                    }
                    "ask" => {
                        let rid = format!("perm-{n}");
                        out(
                            json!({"type":"control_request","request_id":rid,"request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":format!("tu{n}")}}),
                        );
                        asking = Some(rid);
                    }
                    "slow" => {
                        text_message(&mid, "Working…");
                        waiting = Some((Some(later), true));
                    }
                    "silent" => {
                        waiting = Some((None, false));
                    }
                    "escape" => {
                        use std::os::unix::process::CommandExt;
                        use std::process::{Command, Stdio};
                        if let Ok(job) = Command::new("sleep")
                            .arg("600")
                            .process_group(0)
                            .stdin(Stdio::null())
                            .stdout(Stdio::null())
                            .stderr(Stdio::null())
                            .spawn()
                        {
                            let _ = std::fs::write(store.join("escaped.pid"), job.id().to_string());
                        }
                        text_message(&mid, &format!("echo: {text}"));
                        result_ok();
                    }
                    _ => {
                        text_message(&mid, &format!("echo: {text}"));
                        result_ok();
                    }
                }
            }
            _ => {}
        }
    }
}
