//! Built-in JDK `jdb` / JDWP-to-DAP Bridge (`java -agentlib:jdwp=transport=dt_socket,...`).
//!
//! Launches a target JVM suspended under JDWP on an ephemeral loopback socket, attaches
//! standard JDK `jdb`, and translates DAP breakpoints, stepping, stack frames, local
//! variables, and expression evaluation so Java debugging works cleanly with any JDK.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::mpsc;

use super::wire::{DapMessage, DapReader, DapRequest, DapWriter};
use super::{spawn_target_process, BridgeGuard};
use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

/// Launch/attach parameters for the built-in JDK `jdb`/JDWP-to-DAP bridge.
#[derive(Debug, Clone)]
pub struct JavaJdbBridgeSpec {
    pub java_bin: String,
    pub jdb_bin: String,
    pub classpath: String,
    pub main_class: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub attach_port: Option<u16>,
    pub source_map: HashMap<String, String>,
}

/// Infer the fully qualified Java class name for `path` using source `package`
/// declarations on local files or conventional `/src/main/java/` directory layout.
pub(crate) async fn infer_java_fqcn(target: &IdeTargetSpec, path: &str) -> String {
    let stem = Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Main");
    if target.kind == IdeTargetKind::Local {
        if let Ok(txt) = tokio::fs::read_to_string(path).await {
            let pkg = txt.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("package ")
                    .and_then(|r| r.strip_suffix(';'))
                    .map(|p| p.trim().to_string())
            });
            if let Some(p) = pkg.filter(|p| !p.is_empty()) {
                return format!("{p}.{stem}");
            }
            return stem.to_string();
        }
    }
    for marker in ["/src/main/java/", "/src/test/java/", "/src/"] {
        if let Some((_, rel)) = path.split_once(marker) {
            let without_ext = rel.strip_suffix(".java").unwrap_or(rel);
            let dotted = without_ext.replace('/', ".");
            if !dotted.is_empty() {
                return dotted;
            }
        }
    }
    stem.to_string()
}

/// Spawn the built-in JDK `jdb`/JDWP-to-DAP bridge over an in-memory [`DuplexStream`].
pub(crate) async fn spawn_java_jdb_bridge(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: JavaJdbBridgeSpec,
) -> Result<(DuplexStream, Arc<BridgeGuard>)> {
    let guard = BridgeGuard::new();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(String, String)>();

    let jdwp_port = if let Some(port) = spec.attach_port {
        port
    } else {
        spawn_jdwp_jvm(target, ssh, &spec, &guard, &out_tx).await?
    };

    let jdb_args = vec![
        "-connect".to_string(),
        format!("com.sun.jdi.SocketAttach:hostname=127.0.0.1,port={jdwp_port}"),
    ];
    let mut jdb =
        spawn_target_process(target, ssh, &spec.jdb_bin, &jdb_args, &spec.cwd, &spec.env)?;
    let jdb_in = jdb
        .stdin
        .take()
        .ok_or_else(|| Error::Internal("missing jdb stdin".into()))?;
    let jdb_out = jdb
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("missing jdb stdout".into()))?;
    guard.push_child(jdb);

    let (client_io, server_io) = tokio::io::duplex(65536);
    let target_clone = target.clone();
    let guard_task = guard.clone();
    tokio::spawn(async move {
        let _ = run_jdb_bridge_loop(
            server_io,
            jdb_in,
            jdb_out,
            target_clone,
            spec,
            &mut out_rx,
            guard_task,
        )
        .await;
    });
    Ok((client_io, guard))
}

async fn spawn_jdwp_jvm(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: &JavaJdbBridgeSpec,
    guard: &BridgeGuard,
    out_tx: &mpsc::UnboundedSender<(String, String)>,
) -> Result<u16> {
    let mut jvm_args = vec![
        "-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:0".to_string(),
        "-cp".to_string(),
        spec.classpath.clone(),
        spec.main_class.clone(),
    ];
    jvm_args.extend(spec.args.clone());

    let mut jvm =
        spawn_target_process(target, ssh, &spec.java_bin, &jvm_args, &spec.cwd, &spec.env)?;
    let jvm_out = jvm
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("missing java stdout".into()))?;
    if let Some(jvm_err) = jvm.stderr.take() {
        let tx = out_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(jvm_err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(("stderr".into(), format!("{l}\n")));
            }
        });
    }
    let mut out_lines = BufReader::new(jvm_out).lines();
    let mut bound_port: Option<u16> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(Some(line))) =
            tokio::time::timeout(Duration::from_secs(3), out_lines.next_line()).await
        {
            if let Some(rest) = line.split("address:").nth(1) {
                let p_str = rest.trim().rsplit(':').next().unwrap_or(rest.trim());
                if let Ok(p) = p_str.parse::<u16>() {
                    bound_port = Some(p);
                    break;
                }
            }
        } else {
            break;
        }
    }
    let Some(p) = bound_port else {
        let _ = jvm.start_kill();
        return Err(Error::Upstream(
            "JVM JDWP agent failed to report listening port".into(),
        ));
    };
    let tx = out_tx.clone();
    tokio::spawn(async move {
        while let Ok(Some(l)) = out_lines.next_line().await {
            let _ = tx.send(("stdout".into(), format!("{l}\n")));
        }
    });
    guard.push_child(jvm);
    Ok(p)
}

async fn run_jdb_bridge_loop(
    server_io: DuplexStream,
    mut jdb_in: tokio::process::ChildStdin,
    jdb_out: tokio::process::ChildStdout,
    target: IdeTargetSpec,
    spec: JavaJdbBridgeSpec,
    out_rx: &mut mpsc::UnboundedReceiver<(String, String)>,
    guard: Arc<BridgeGuard>,
) -> io::Result<()> {
    let (r, w) = tokio::io::split(server_io);
    let mut dap_r = DapReader::new(r);
    let mut dap_w = DapWriter::new(w);
    let mut jdb_lines = BufReader::new(jdb_out).lines();
    let mut paused_frames: Vec<Value> = Vec::new();
    let mut paused_vars: Vec<Value> = Vec::new();
    let mut source_map = spec.source_map;

    loop {
        tokio::select! {
            Some((cat, text)) = out_rx.recv() => {
                dap_w.send_event("output", Some(json!({ "category": cat, "output": text }))).await?;
            }
            jdb_line = jdb_lines.next_line() => {
                let Ok(Some(line)) = jdb_line else {
                    let _ = dap_w.send_event("terminated", Some(json!({}))).await;
                    break;
                };
                if line.contains("Breakpoint hit:") || line.contains("Step completed:") {
                    let reason = if line.contains("Breakpoint hit:") { "breakpoint" } else { "step" };
                    query_jdb_state(&mut jdb_in, &mut jdb_lines, &source_map, &mut paused_frames, &mut paused_vars).await?;
                    dap_w.send_event("stopped", Some(json!({ "reason": reason, "threadId": 1, "allThreadsStopped": true }))).await?;
                } else if line.contains("The application exited") {
                    let _ = dap_w.send_event("terminated", Some(json!({}))).await;
                    break;
                }
            }
            dap_msg = dap_r.next_message() => {
                let Some(DapMessage::Request(req)) = dap_msg? else { break; };
                let mut ctx = JdbReqCtx {
                    target: &target,
                    jdb_in: &mut jdb_in,
                    jdb_lines: &mut jdb_lines,
                    source_map: &mut source_map,
                    paused_frames: &paused_frames,
                    paused_vars: &paused_vars,
                    guard: &guard,
                };
                if handle_jdb_dap_req(&req, &mut ctx, &mut dap_w).await? {
                    break;
                }
            }
        }
    }
    guard.kill_all();
    Ok(())
}

struct JdbReqCtx<'a> {
    target: &'a IdeTargetSpec,
    jdb_in: &'a mut tokio::process::ChildStdin,
    jdb_lines: &'a mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    source_map: &'a mut HashMap<String, String>,
    paused_frames: &'a [Value],
    paused_vars: &'a [Value],
    guard: &'a BridgeGuard,
}

async fn query_jdb_state(
    jdb_in: &mut tokio::process::ChildStdin,
    jdb_lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    source_map: &HashMap<String, String>,
    frames: &mut Vec<Value>,
    vars: &mut Vec<Value>,
) -> io::Result<()> {
    frames.clear();
    vars.clear();
    jdb_in
        .write_all(b"where\nlocals\necho __OTTO_JDB_SYNC__\n")
        .await?;
    jdb_in.flush().await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let Ok(Ok(Some(line))) =
            tokio::time::timeout(Duration::from_millis(800), jdb_lines.next_line()).await
        else {
            break;
        };
        if line.contains("__OTTO_JDB_SYNC__") {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if let (Some(rb), Some(lp), Some(rp)) =
                (trimmed.find(']'), trimmed.rfind('('), trimmed.rfind(')'))
            {
                let idx_num: i64 = trimmed[1..rb].trim().parse().unwrap_or(1);
                let fn_name = trimmed[rb + 1..lp].trim();
                let loc = &trimmed[lp + 1..rp];
                if let Some((fname, lno_s)) = loc.rsplit_once(':') {
                    let lno: u32 = lno_s.trim().parse().unwrap_or(1);
                    let src = source_map
                        .get(fname.trim())
                        .cloned()
                        .map(|p| json!({ "path": p }));
                    frames.push(json!({
                        "id": idx_num,
                        "name": fn_name,
                        "source": src,
                        "line": lno,
                        "column": 1
                    }));
                }
            }
        } else if let Some((vname, vval)) = trimmed.split_once(" = ") {
            let clean_name = vname.trim();
            if !clean_name.contains(' ') && !clean_name.is_empty() {
                vars.push(json!({
                    "name": clean_name,
                    "value": vval.trim(),
                    "variablesReference": 0
                }));
            }
        }
    }
    Ok(())
}

async fn handle_jdb_dap_req<W: AsyncWrite + Unpin>(
    req: &DapRequest,
    ctx: &mut JdbReqCtx<'_>,
    dap_w: &mut DapWriter<W>,
) -> io::Result<bool> {
    let args = req.arguments.as_ref();
    match req.command.as_str() {
        "initialize" => {
            dap_w
                .send_response(
                    req.seq,
                    "initialize",
                    Ok(Some(json!({ "supportsConfigurationDoneRequest": true }))),
                )
                .await?;
        }
        "launch" | "attach" => {
            dap_w
                .send_response(req.seq, &req.command, Ok(Some(json!({}))))
                .await?;
            dap_w.send_event("initialized", Some(json!({}))).await?;
        }
        "setBreakpoints" => {
            let path = args
                .and_then(|a| a.get("source"))
                .and_then(|s| s.get("path"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(fname) = Path::new(path).file_name().and_then(|n| n.to_str()) {
                ctx.source_map.insert(fname.to_string(), path.to_string());
            }
            let fqcn = infer_java_fqcn(ctx.target, path).await;
            let bps = args
                .and_then(|a| a.get("breakpoints"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut verified = Vec::new();
            for bp in bps {
                let line = bp.get("line").and_then(Value::as_u64).unwrap_or(1);
                ctx.jdb_in
                    .write_all(format!("stop at {fqcn}:{line}\n").as_bytes())
                    .await?;
                verified.push(json!({ "verified": true, "line": line }));
            }
            ctx.jdb_in.flush().await?;
            dap_w
                .send_response(
                    req.seq,
                    "setBreakpoints",
                    Ok(Some(json!({ "breakpoints": verified }))),
                )
                .await?;
        }
        "setExceptionBreakpoints" => {
            dap_w
                .send_response(req.seq, "setExceptionBreakpoints", Ok(Some(json!({}))))
                .await?;
        }
        "configurationDone" => {
            ctx.jdb_in.write_all(b"cont\n").await?;
            ctx.jdb_in.flush().await?;
            dap_w
                .send_response(req.seq, "configurationDone", Ok(Some(json!({}))))
                .await?;
        }
        "threads" => {
            dap_w
                .send_response(
                    req.seq,
                    "threads",
                    Ok(Some(json!({ "threads": [{ "id": 1, "name": "main" }] }))),
                )
                .await?;
        }
        "stackTrace" => {
            dap_w
                .send_response(
                    req.seq,
                    "stackTrace",
                    Ok(Some(json!({
                        "stackFrames": ctx.paused_frames,
                        "totalFrames": ctx.paused_frames.len()
                    }))),
                )
                .await?;
        }
        "scopes" => {
            dap_w
                .send_response(
                    req.seq,
                    "scopes",
                    Ok(Some(json!({
                        "scopes": [{ "name": "Locals", "variablesReference": 1001, "expensive": false }]
                    }))),
                )
                .await?;
        }
        "variables" => {
            dap_w
                .send_response(
                    req.seq,
                    "variables",
                    Ok(Some(json!({ "variables": ctx.paused_vars }))),
                )
                .await?;
        }
        "evaluate" => {
            let expr = args
                .and_then(|a| a.get("expression"))
                .and_then(Value::as_str)
                .unwrap_or("");
            ctx.jdb_in
                .write_all(format!("eval {expr}\necho __OTTO_EVAL_END__\n").as_bytes())
                .await?;
            ctx.jdb_in.flush().await?;
            let mut result_text = String::new();
            while let Ok(Ok(Some(l))) =
                tokio::time::timeout(Duration::from_secs(2), ctx.jdb_lines.next_line()).await
            {
                if l.contains("__OTTO_EVAL_END__") {
                    break;
                }
                if let Some((_, rhs)) = l.split_once(" = ") {
                    result_text = rhs.trim().to_string();
                }
            }
            dap_w
                .send_response(
                    req.seq,
                    "evaluate",
                    Ok(Some(
                        json!({ "result": result_text, "variablesReference": 0 }),
                    )),
                )
                .await?;
        }
        "continue" | "next" | "stepIn" | "stepOut" => {
            let cmd = match req.command.as_str() {
                "continue" => "cont\n",
                "next" => "next\n",
                "stepIn" => "step\n",
                _ => "step up\n",
            };
            ctx.jdb_in.write_all(cmd.as_bytes()).await?;
            ctx.jdb_in.flush().await?;
            let body = if req.command == "continue" {
                json!({ "allThreadsContinued": true })
            } else {
                json!({})
            };
            dap_w
                .send_response(req.seq, &req.command, Ok(Some(body)))
                .await?;
        }
        "disconnect" | "terminate" => {
            let _ = ctx.jdb_in.write_all(b"quit\n").await;
            ctx.guard.kill_all();
            dap_w
                .send_response(req.seq, &req.command, Ok(Some(json!({}))))
                .await?;
            let _ = dap_w.send_event("terminated", Some(json!({}))).await;
            return Ok(true);
        }
        other => {
            dap_w
                .send_response(req.seq, other, Ok(Some(json!({}))))
                .await?;
        }
    }
    Ok(false)
}
