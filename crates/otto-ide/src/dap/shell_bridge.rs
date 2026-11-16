//! Zero-install Shell (`bash` + `zsh`) Debug Adapter Protocol bridge (`shell_bridge`).
//!
//! Spawns `bash` (`set -T` + `trap ... DEBUG`) or `zsh` (`setopt DEBUG_BEFORE_CMD` +
//! `TRAPDEBUG` + `funcfiletrace`) communicating over dedicated out-of-band FIFOs +
//! atomic breakpoint/pause control files so debuggee `stdin`/`stdout`/`stderr` and `$()`
//! subshells remain completely unpolluted, and mid-run breakpoints & `pause` take effect
//! immediately while loops are running.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Stdio;
use std::sync::Arc;
#[cfg(unix)]
use std::sync::Mutex;

#[cfg(unix)]
use otto_core::new_id;
use otto_core::{Error, Result};
#[cfg(unix)]
use serde_json::{json, Value};
#[cfg(unix)]
use tokio::io::AsyncWriteExt;
use tokio::io::DuplexStream;
#[cfg(unix)]
use tokio::net::unix::pipe::{self, Receiver as PipeReceiver, Sender as PipeSender};
#[cfg(unix)]
use tokio::process::Command;
#[cfg(unix)]
use tokio::sync::mpsc;

#[cfg(unix)]
use super::shell_bridge_eval::{
    check_shell_condition, encode_bp_list, interpolate_shell_log, spawn_trap_readers,
};
use super::shell_prelude::{BASH_PRELUDE, ZSH_PRELUDE};
#[cfg(unix)]
use super::wire::{DapMessage, DapReader, DapWriter};
use super::BridgeGuard;
#[cfg(unix)]
use crate::types::IdeTargetKind;
use crate::types::{IdeSshSpec, IdeTargetSpec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellFlavor {
    Bash,
    Zsh,
}

#[derive(Clone)]
pub struct ShellBridgeSpec {
    pub shell_bin: String,
    pub flavor: ShellFlavor,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub(super) struct ShellBp {
    pub(super) line: u32,
    pub(super) condition: Option<String>,
    pub(super) hit_condition: Option<u32>,
    pub(super) log_message: Option<String>,
    pub(super) hits: u32,
}

#[cfg(unix)]
#[derive(Debug, Clone)]
pub(super) struct PausedState {
    pub(super) frames: Vec<(String, String, u32)>,
    pub(super) vars: Vec<(String, String)>,
}

#[cfg(not(unix))]
pub(crate) async fn spawn_shell_dap_bridge(
    _target: &IdeTargetSpec,
    _ssh: Option<&IdeSshSpec>,
    _spec: ShellBridgeSpec,
) -> Result<(DuplexStream, Arc<BridgeGuard>)> {
    Err(Error::Invalid(
        "Shell interactive debugger requires a Unix host".into(),
    ))
}

#[cfg(unix)]
pub(crate) async fn spawn_shell_dap_bridge(
    target: &IdeTargetSpec,
    _ssh: Option<&IdeSshSpec>,
    spec: ShellBridgeSpec,
) -> Result<(DuplexStream, Arc<BridgeGuard>)> {
    if target.kind != IdeTargetKind::Local {
        return Err(Error::Invalid(
            "Shell interactive debugger currently requires a Local workspace target".into(),
        ));
    }

    let ipc_dir = std::env::temp_dir().join(format!("otto-sh-dap-{}", new_id()));
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(&ipc_dir)
            .map_err(|e| Error::Internal(format!("create shell debug ipc dir: {e}")))?;
    }
    let cmd_fifo = ipc_dir.join("cmd.fifo");
    let evt_fifo = ipc_dir.join("evt.fifo");
    let bps_file = ipc_dir.join("bps.txt");
    let pause_file = ipc_dir.join("pause.flag");
    let (prelude_name, prelude_body) = match spec.flavor {
        ShellFlavor::Bash => ("prelude.bash", BASH_PRELUDE),
        ShellFlavor::Zsh => ("prelude.zsh", ZSH_PRELUDE),
    };
    let prelude_file = ipc_dir.join(prelude_name);

    make_fifo(&cmd_fifo).await?;
    make_fifo(&evt_fifo).await?;
    write_ipc_file_0600(&ipc_dir, &bps_file, b"|\n")
        .map_err(|e| Error::Internal(format!("write shell bps file: {e}")))?;
    write_ipc_file_0600(&ipc_dir, &prelude_file, prelude_body.as_bytes())
        .map_err(|e| Error::Internal(format!("write shell debug prelude: {e}")))?;

    let evt_pipe = pipe::OpenOptions::new()
        .open_receiver(&evt_fifo)
        .map_err(|e| Error::Internal(format!("open evt fifo receiver: {e}")))?;
    let evt_keepalive = Arc::new(Mutex::new(Some(
        pipe::OpenOptions::new()
            .open_sender(&evt_fifo)
            .map_err(|e| Error::Internal(format!("open evt fifo keepalive: {e}")))?,
    )));
    let cmd_keepalive = pipe::OpenOptions::new()
        .open_receiver(&cmd_fifo)
        .map_err(|e| Error::Internal(format!("open cmd fifo keepalive: {e}")))?;
    let cmd_pipe = pipe::OpenOptions::new()
        .open_sender(&cmd_fifo)
        .map_err(|e| Error::Internal(format!("open cmd fifo sender: {e}")))?;

    let mut env = spec.env.clone();
    env.insert(
        "OTTO_SH_DAP_CMD".into(),
        cmd_fifo.to_string_lossy().into_owned(),
    );
    env.insert(
        "OTTO_SH_DAP_EVT".into(),
        evt_fifo.to_string_lossy().into_owned(),
    );
    env.insert(
        "OTTO_SH_DAP_BPS".into(),
        bps_file.to_string_lossy().into_owned(),
    );
    env.insert(
        "OTTO_SH_DAP_PAUSE".into(),
        pause_file.to_string_lossy().into_owned(),
    );
    env.insert("OTTO_SH_DAP_TARGET".into(), spec.program.clone());

    let mut cmd = Command::new(&spec.shell_bin);
    if spec.flavor == ShellFlavor::Zsh {
        cmd.arg("--no-rcs");
    }
    cmd.arg(&prelude_file)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
        .envs(&env)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("failed to spawn shell debugger: {e}")))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let guard = BridgeGuard::new();
    guard.push_child(child);
    guard.push_temp_dir(ipc_dir.clone());

    let (client_end, bridge_end) = tokio::io::duplex(65536);
    tokio::spawn(run_shell_bridge_loop(
        bridge_end,
        cmd_pipe,
        cmd_keepalive,
        evt_pipe,
        evt_keepalive,
        stdout,
        stderr,
        ipc_dir,
    ));

    Ok((client_end, guard))
}

#[cfg(unix)]
#[allow(clippy::disallowed_methods)]
fn write_ipc_file_0600(ipc_dir: &Path, dst: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let mut tmp = tempfile::NamedTempFile::new_in(ipc_dir)?;
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o600))?;
    tmp.write_all(bytes)?;
    tmp.persist(dst).map(|_| ()).map_err(|e| e.error)
}

#[cfg(unix)]
async fn make_fifo(path: &Path) -> Result<()> {
    let status = Command::new("mkfifo")
        .arg("-m")
        .arg("600")
        .arg(path)
        .status()
        .await
        .map_err(|e| Error::Internal(format!("mkfifo failed: {e}")))?;
    if !status.success() {
        return Err(Error::Internal(format!(
            "mkfifo exited with {status} for {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(super) enum TrapMsg {
    Stopped {
        reason: String,
        state: PausedState,
    },
    EvalResult {
        rc: i32,
        output: String,
    },
    Output {
        category: &'static str,
        text: String,
    },
    Eof,
}

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
async fn run_shell_bridge_loop(
    bridge_io: DuplexStream,
    mut cmd_pipe: PipeSender,
    _cmd_keepalive: PipeReceiver,
    evt_pipe: PipeReceiver,
    evt_keepalive: Arc<Mutex<Option<PipeSender>>>,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    ipc_dir: PathBuf,
) {
    let (r_half, w_half) = tokio::io::split(bridge_io);
    let mut dap_in = DapReader::new(r_half);
    let mut dap_out = DapWriter::new(w_half);
    let bps_file = ipc_dir.join("bps.txt");
    let pause_file = ipc_dir.join("pause.flag");

    let (trap_tx, mut trap_rx) = mpsc::unbounded_channel::<TrapMsg>();
    let (stdout_handle, stderr_handle, evt_handle) = spawn_trap_readers(
        evt_pipe,
        Arc::clone(&evt_keepalive),
        stdout,
        stderr,
        trap_tx,
    );

    let mut bps_by_file: HashMap<String, Vec<ShellBp>> = HashMap::new();
    let mut paused: Option<PausedState> = None;
    let mut pending_eval_seq: Option<i64> = None;
    let mut started = false;

    loop {
        tokio::select! {
            msg_res = dap_in.next_message() => {
                let Ok(Some(DapMessage::Request(req))) = msg_res else { break; };
                match req.command.as_str() {
                    "initialize" => {
                        let _ = dap_out.send_response(req.seq, "initialize", Ok(Some(json!({
                            "supportsConfigurationDoneRequest": true,
                            "supportsConditionalBreakpoints": true,
                            "supportsHitConditionalBreakpoints": true,
                            "supportsLogPoints": true,
                            "supportsTerminateRequest": true,
                        })))).await;
                    }
                    "launch" | "attach" => {
                        let _ = dap_out.send_event("initialized", Some(json!({}))).await;
                        let _ = dap_out.send_response(req.seq, &req.command, Ok(Some(json!({})))).await;
                    }
                    "setBreakpoints" => {
                        let args = req.arguments.unwrap_or_else(|| json!({}));
                        let raw_path = args.get("source").and_then(|s| s.get("path")).and_then(Value::as_str).unwrap_or("");
                        let canon = std::fs::canonicalize(raw_path)
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|_| raw_path.to_string());
                        let mut file_bps = Vec::new();
                        let mut ack = Vec::new();
                        if let Some(arr) = args.get("breakpoints").and_then(Value::as_array) {
                            for b in arr {
                                let ln = b.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
                                let cond = b.get("condition").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                                let hit = b.get("hitCondition").and_then(Value::as_str).and_then(|s| s.trim().trim_start_matches(['=', '>', ' ']).parse::<u32>().ok());
                                let log = b.get("logMessage").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from);
                                file_bps.push(ShellBp { line: ln, condition: cond, hit_condition: hit, log_message: log, hits: 0 });
                                ack.push(json!({ "verified": true, "line": ln }));
                            }
                        }
                        bps_by_file.insert(canon, file_bps);
                        let encoded = encode_bp_list(&bps_by_file);
                        let _ = write_ipc_file_0600(&ipc_dir, &bps_file, format!("{encoded}\n").as_bytes());
                        if !started || paused.is_some() {
                            let _ = cmd_pipe.write_all(format!("BPS{encoded}\n").as_bytes()).await;
                        }
                        let _ = dap_out.send_response(req.seq, "setBreakpoints", Ok(Some(json!({ "breakpoints": ack })))).await;
                    }
                    "configurationDone" => {
                        started = true;
                        let _ = cmd_pipe.write_all(b"RUN|run\n").await;
                        let _ = dap_out.send_response(req.seq, "configurationDone", Ok(Some(json!({})))).await;
                    }
                    "pause" => {
                        let _ = write_ipc_file_0600(&ipc_dir, &pause_file, b"1");
                        let _ = dap_out.send_response(req.seq, "pause", Ok(Some(json!({})))).await;
                    }
                    "threads" => {
                        let _ = dap_out.send_response(req.seq, "threads", Ok(Some(json!({
                            "threads": [{ "id": 1, "name": "shell main" }]
                        })))).await;
                    }
                    "stackTrace" => {
                        let sfs: Vec<Value> = paused.as_ref().map(|st| {
                            st.frames.iter().enumerate().map(|(i, (fn_name, path, ln))| {
                                json!({
                                    "id": (i + 1) as i64,
                                    "name": fn_name,
                                    "source": { "path": path },
                                    "line": ln,
                                    "column": 1
                                })
                            }).collect()
                        }).unwrap_or_default();
                        let total = sfs.len();
                        let _ = dap_out.send_response(req.seq, "stackTrace", Ok(Some(json!({
                            "stackFrames": sfs,
                            "totalFrames": total
                        })))).await;
                    }
                    "scopes" => {
                        let _ = dap_out.send_response(req.seq, "scopes", Ok(Some(json!({
                            "scopes": [{ "name": "Shell Variables", "variablesReference": 1001, "expensive": false }]
                        })))).await;
                    }
                    "variables" => {
                        let vars: Vec<Value> = paused.as_ref().map(|st| {
                            st.vars.iter().take(150).map(|(k, v)| {
                                json!({
                                    "name": k,
                                    "value": v,
                                    "type": "string",
                                    "variablesReference": 0
                                })
                            }).collect()
                        }).unwrap_or_default();
                        let _ = dap_out.send_response(req.seq, "variables", Ok(Some(json!({ "variables": vars })))).await;
                    }
                    "evaluate" => {
                        let expr = req.arguments.as_ref().and_then(|a| a.get("expression")).and_then(Value::as_str).unwrap_or("").trim();
                        if paused.is_some() {
                            let eval_cmd = if expr.starts_with('$') || !expr.contains(' ') {
                                let var_name = expr.trim_start_matches('$').trim_matches(['{', '}']);
                                format!("printf '%s' \"${{{var_name}-}}\"")
                            } else {
                                expr.replace('\n', "; ")
                            };
                            pending_eval_seq = Some(req.seq);
                            let _ = cmd_pipe.write_all(format!("EVAL|{eval_cmd}\n").as_bytes()).await;
                        } else {
                            let _ = dap_out.send_response(req.seq, "evaluate", Err("Shell is not paused".into())).await;
                        }
                    }
                    "continue" => {
                        paused = None;
                        let _ = cmd_pipe.write_all(b"CONT\n").await;
                        let _ = dap_out.send_response(req.seq, "continue", Ok(Some(json!({ "allThreadsContinued": true })))).await;
                    }
                    "next" => {
                        paused = None;
                        let _ = cmd_pipe.write_all(b"NEXT\n").await;
                        let _ = dap_out.send_response(req.seq, "next", Ok(Some(json!({})))).await;
                    }
                    "stepIn" => {
                        paused = None;
                        let _ = cmd_pipe.write_all(b"STEP\n").await;
                        let _ = dap_out.send_response(req.seq, "stepIn", Ok(Some(json!({})))).await;
                    }
                    "stepOut" => {
                        paused = None;
                        let _ = cmd_pipe.write_all(b"OUT\n").await;
                        let _ = dap_out.send_response(req.seq, "stepOut", Ok(Some(json!({})))).await;
                    }
                    "terminate" | "disconnect" => {
                        let _ = cmd_pipe.write_all(b"QUIT\n").await;
                        let _ = dap_out.send_response(req.seq, &req.command, Ok(Some(json!({})))).await;
                        let _ = dap_out.send_event("terminated", Some(json!({}))).await;
                        break;
                    }
                    other => {
                        let _ = dap_out.send_response(req.seq, other, Ok(Some(json!({})))).await;
                    }
                }
            }
            Some(tmsg) = trap_rx.recv() => {
                match tmsg {
                    TrapMsg::Output { category, text } => {
                        let _ = dap_out.send_event("output", Some(json!({ "category": category, "output": text }))).await;
                    }
                    TrapMsg::EvalResult { rc, output } => {
                        if let Some(seq) = pending_eval_seq.take() {
                            let _ = dap_out.send_response(seq, "evaluate", Ok(Some(json!({
                                "result": output,
                                "type": if rc == 0 { "shell" } else { "exit!=0" },
                                "variablesReference": 0
                            })))).await;
                        }
                    }
                    TrapMsg::Stopped { reason, state } => {
                        if let Some((_, top_path, top_ln)) = state.frames.first() {
                            if let Some(bp) = bps_by_file.get_mut(top_path).and_then(|v| v.iter_mut().find(|b| b.line == *top_ln)) {
                                if let Some(cond) = &bp.condition {
                                    if !check_shell_condition(cond, &state.vars).await {
                                        let _ = cmd_pipe.write_all(b"CONT\n").await;
                                        continue;
                                    }
                                }
                                bp.hits += 1;
                                if let Some(need_hits) = bp.hit_condition {
                                    if bp.hits < need_hits {
                                        let _ = cmd_pipe.write_all(b"CONT\n").await;
                                        continue;
                                    }
                                }
                                if let Some(log_tpl) = &bp.log_message {
                                    let expanded = interpolate_shell_log(log_tpl, &state.vars);
                                    let _ = dap_out.send_event("output", Some(json!({
                                        "category": "console",
                                        "output": format!("{expanded}\n")
                                    }))).await;
                                    let _ = cmd_pipe.write_all(b"CONT\n").await;
                                    continue;
                                }
                            }
                        }
                        paused = Some(state);
                        let _ = dap_out.send_event("stopped", Some(json!({
                            "reason": reason,
                            "threadId": 1,
                            "allThreadsStopped": true
                        }))).await;
                    }
                    TrapMsg::Eof => {
                        let _ = dap_out.send_event("terminated", Some(json!({}))).await;
                        break;
                    }
                }
            }
        }
    }
    evt_handle.abort();
    if let Some(h) = stdout_handle {
        h.abort();
    }
    if let Some(h) = stderr_handle {
        h.abort();
    }
    if let Ok(mut g) = evt_keepalive.lock() {
        *g = None;
    }
    let _ = tokio::fs::remove_dir_all(&ipc_dir).await;
}
