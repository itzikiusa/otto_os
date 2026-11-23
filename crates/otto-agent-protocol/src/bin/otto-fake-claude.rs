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
//! * `retrying <secs>` — an `api_retry` frame every 200 ms for `<secs>`,
//!   nothing else, until interrupted.
//! * `stream <n> <ms>` — `c0 `…`c<n-1> ` as text deltas, one per `<ms>`.
//! * `ask`         — a `can_use_tool` request for Bash `ls`; the answer
//!   decides: "allowed" or "denied", then `result`.
//! * `escape`      — starts `sleep 600` in a process group of its own (as
//!   Claude Code does with some jobs), writes its pid to
//!   `.fake-claude/escaped.pid`, then answers like `echo`.
//! * `mcp <server> <tool> [json]` — a real MCP call: launches `<server>` from
//!   the `--mcp-config` file (its `${NAME:-}` env references expanded from
//!   this process), runs `initialize` + `tools/call` off the main loop, and
//!   reports it as Claude Code does: a `mcp__<server>__<tool>` tool_use, its
//!   tool_result (the MCP `content`, `is_error`), "mcp done", `result`.
//! * (every start) `system init` lists `mcp_servers`: the `--mcp-config`
//!   servers ("connected" when the command exists, else "failed") and
//!   `own-docs` ("needs-auth", as if from Claude Code's own config).
//! * anything else — `echo: <text>` as stream events + `assistant`.
//!
//! An `interrupt` control request ends the running turn as interrupted; a
//! `stop_task` one kills that background task (`task_notification` killed).
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

fn wait_for_file(path: &std::path::Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn stream_delta(index: u64, chunk: &str) {
    out(
        json!({"type":"stream_event","event":{"type":"content_block_delta","index":index,"delta":{"type":"text_delta","text":chunk}},"parent_tool_use_id":null}),
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
    /// An `mcp` call's outcome: (tool_use id, MCP result or error text).
    McpDone(String, Result<Value, String>),
}

/// `${NAME:-}` / `${NAME}` from this process's env (Claude Code expands
/// `.mcp.json` env the same way).
fn expand_env(v: &str) -> String {
    let mut out = String::new();
    let mut rest = v;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let Some(end) = rest[i..].find('}') else {
            break;
        };
        let name = rest[i + 2..i + end].split(":-").next().unwrap_or_default();
        out.push_str(&std::env::var(name).unwrap_or_default());
        rest = &rest[i + end + 1..];
    }
    out.push_str(rest);
    out
}

/// `system init`'s `mcp_servers`: each `--mcp-config` server "connected"
/// when its command exists, else "failed", plus one server of the CLI's own
/// config waiting for a sign-in (`own-docs`, "needs-auth").
fn init_mcp_servers(config: &str) -> Value {
    let raw = std::fs::read_to_string(config).unwrap_or_default();
    let doc: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let mut list: Vec<Value> = doc["mcpServers"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, spec)| {
            let cmd = spec["command"].as_str().unwrap_or_default();
            let found = std::path::Path::new(cmd).is_file()
                || std::env::var_os("PATH")
                    .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()));
            json!({"name": name, "status": if found { "connected" } else { "failed" }})
        })
        .collect();
    list.push(json!({"name": "own-docs", "status": "needs-auth"}));
    Value::Array(list)
}

/// One stdio MCP call against `server` from the `--mcp-config` document.
fn mcp_call(config: &str, server: &str, tool: &str, args: Value) -> Result<Value, String> {
    use std::io::BufReader;
    use std::process::{Command, Stdio};
    let raw = std::fs::read_to_string(config).unwrap_or_else(|_| config.to_string());
    let doc: Value = serde_json::from_str(&raw).map_err(|e| format!("mcp config: {e}"))?;
    let spec = doc
        .pointer(&format!("/mcpServers/{server}"))
        .ok_or_else(|| format!("no MCP server {server}"))?;
    let mut cmd = Command::new(spec["command"].as_str().unwrap_or_default());
    for a in spec["args"].as_array().into_iter().flatten() {
        cmd.arg(a.as_str().unwrap_or_default());
    }
    for (k, v) in spec["env"].as_object().into_iter().flatten() {
        cmd.env(k, expand_env(v.as_str().unwrap_or_default()));
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn {server}: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let mut lines = BufReader::new(child.stdout.take().ok_or("no stdout")?).lines();
    let mut ask = |id: u64, method: &str, params: Value| -> Result<Value, String> {
        if method.starts_with("notifications/") {
            let note = json!({"jsonrpc":"2.0","method":method});
            return writeln!(stdin, "{note}")
                .map(|_| Value::Null)
                .map_err(|e| e.to_string());
        }
        let req = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        writeln!(stdin, "{req}").map_err(|e| e.to_string())?;
        loop {
            let line = lines
                .next()
                .ok_or("server closed")?
                .map_err(|e| e.to_string())?;
            let v: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
            if v["id"] == json!(id) {
                return match v.get("error") {
                    Some(e) => Err(e.to_string()),
                    None => Ok(v["result"].clone()),
                };
            }
        }
    };
    let hello = json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fake-claude","version":"0"}});
    ask(1, "initialize", hello)?;
    ask(0, "notifications/initialized", Value::Null)?;
    let res = ask(2, "tools/call", json!({"name": tool, "arguments": args}));
    let _ = child.kill();
    let _ = child.wait();
    res
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
    let mut mode = arg_after(&args, "--permission-mode").unwrap_or_default();
    let mut cost = 0.0f64;
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
    let mcp_config = arg_after(&args, "--mcp-config").unwrap_or_default();
    let (tx, rx) = mpsc::channel::<In>();
    let mcp_tx = tx.clone();
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
            In::McpDone(tu, res) => {
                waiting = None;
                let (content, is_error) = match res {
                    Ok(r) => (
                        r["content"].clone(),
                        r["isError"].as_bool().unwrap_or(false),
                    ),
                    Err(e) => (json!([{"type":"text","text":e}]), true),
                };
                out(
                    json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":tu,"content":content,"is_error":is_error}]},"parent_tool_use_id":null}),
                );
                n += 1;
                text_message(&format!("m{n}"), "mcp done");
                result_ok();
                continue;
            }
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
                if sub == Some("set_permission_mode") {
                    mode = msg
                        .pointer("/request/mode")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                }
                // `initialize` lists its slash commands, as Claude Code does.
                let body = if sub == Some("initialize") {
                    json!({"commands": [
                        {"name": "review", "description": "Review the current changes", "argumentHint": "[focus]"},
                        {"name": "compact", "description": "Clear history but keep a summary", "argumentHint": ""}
                    ]})
                } else {
                    json!({})
                };
                out(
                    json!({"type":"control_response","response":{"subtype":"success","request_id":rid,"response":body}}),
                );
                if sub == Some("initialize") {
                    out(
                        json!({"type":"system","subtype":"init","session_id":session,"model":"fake-claude","mcp_servers":init_mcp_servers(&mcp_config)}),
                    );
                }
                if sub == Some("interrupt") && waiting.take().is_some() {
                    result_interrupted();
                }
                // `stop_task`: the background task is killed at once.
                if sub == Some("stop_task") {
                    let task = msg.pointer("/request/task_id").and_then(Value::as_str);
                    if let Some(i) = dues.iter().position(|d| Some(d.task_id.as_str()) == task) {
                        let d = dues.remove(i);
                        out(
                            json!({"type":"system","subtype":"task_notification","task_id":d.task_id,
                            "tool_use_id":d.tool_use_id,"status":"killed","output_file":"","summary":"stopped"}),
                        );
                    }
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
                if let Some(gate_dir) = text.strip_prefix("gated ").map(str::trim) {
                    let gate = std::path::Path::new(gate_dir);
                    let _ = std::fs::write(gate.join("pid_turn1"), std::process::id().to_string());
                    out(
                        json!({"type":"stream_event","event":{"type":"message_start","message":{"id":mid}},"parent_tool_use_id":null}),
                    );
                    out(
                        json!({"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"parent_tool_use_id":null}),
                    );
                    stream_delta(0, "alpha beta ");
                    let _ = std::fs::write(gate.join("phase1_ready"), b"1");
                    if !wait_for_file(&gate.join("emit_gap")) {
                        std::process::exit(2);
                    }
                    stream_delta(0, "gamma ");
                    let _ = std::fs::write(gate.join("gap_emitted"), b"1");
                    if !wait_for_file(&gate.join("finish_turn")) {
                        std::process::exit(2);
                    }
                    stream_delta(0, "delta");
                    out(
                        json!({"type":"stream_event","event":{"type":"content_block_stop","index":0},"parent_tool_use_id":null}),
                    );
                    out(
                        json!({"type":"assistant","message":{"id":mid,"content":[{"type":"text","text":"alpha beta gamma delta"}]},"parent_tool_use_id":null}),
                    );
                    result_ok();
                    continue;
                }
                if let Some(pid_path) = text.strip_prefix("pid ").map(str::trim) {
                    let _ = std::fs::write(pid_path, std::process::id().to_string());
                    text_message(&mid, "second-turn-ok");
                    result_ok();
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
                    "retrying" => {
                        std::thread::spawn(move || {
                            let mut attempt = 0;
                            while Instant::now() < later {
                                attempt += 1;
                                out(
                                    json!({"type":"system","subtype":"api_retry","attempt":attempt,"max_retries":10,"retry_delay_ms":200,"error_status":529,"error":"overloaded"}),
                                );
                                std::thread::sleep(Duration::from_millis(200));
                            }
                        });
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
                    "mcp" => {
                        let mut parts = text.splitn(4, ' ').skip(1);
                        let server = parts.next().unwrap_or_default().to_string();
                        let tool = parts.next().unwrap_or_default().to_string();
                        let args: Value = parts
                            .next()
                            .and_then(|a| serde_json::from_str(a).ok())
                            .unwrap_or_else(|| json!({}));
                        let tu = format!("tu{n}");
                        out(
                            json!({"type":"assistant","message":{"id":mid,"content":[{"type":"tool_use","id":tu,"name":format!("mcp__{server}__{tool}"),"input":args}]},"parent_tool_use_id":null}),
                        );
                        let (config, done) = (mcp_config.clone(), mcp_tx.clone());
                        std::thread::spawn(move || {
                            let res = mcp_call(&config, &server, &tool, args);
                            let _ = done.send(In::McpDone(tu, res));
                        });
                        waiting = Some((None, false));
                    }
                    // `mode`: the permission mode it runs in (plan, default…).
                    "mode" => {
                        text_message(&mid, &format!("mode: {mode}"));
                        result_ok();
                    }
                    _ => {
                        text_message(&mid, &format!("echo: {text}"));
                        // Claude Code sums the process's cost on every result.
                        cost += 0.0125;
                        out(
                            json!({"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1},"total_cost_usd":cost}),
                        );
                    }
                }
            }
            _ => {}
        }
    }
}
