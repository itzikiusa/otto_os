//! Zero-install Shell (`bash` / `.sh`) Debug Adapter Protocol bridge (`shell_bridge`).
//!
//! Spawns `bash` with `set -T` (`functrace`) and a `trap ... DEBUG` hook communicating
//! over dedicated out-of-band FIFOs so the debuggee script's `stdin`/`stdout`/`stderr`
//! and `$()` subshells remain completely unpolluted. Compatible with macOS stock
//! `/bin/bash` 3.2 as well as Bash 4/5.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use otto_core::{new_id, Error, Result};
use serde_json::{json, Value};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::process::Command;
use tokio::sync::mpsc;

use super::wire::{DapMessage, DapReader, DapWriter};
use super::BridgeGuard;
use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

#[derive(Debug, Clone)]
pub struct ShellBridgeSpec {
    pub bash_bin: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
struct ShellBp {
    line: u32,
    condition: Option<String>,
    hit_condition: Option<u32>,
    log_message: Option<String>,
    hits: u32,
}

#[derive(Debug, Clone)]
struct PausedState {
    frames: Vec<(String, String, u32)>, // (func, path, line)
    vars: Vec<(String, String)>,
}

const BASH_PRELUDE: &str = r#"
_otto_fifo_evt="$OTTO_SH_DAP_EVT"
_otto_fifo_cmd="$OTTO_SH_DAP_CMD"
_otto_target="$OTTO_SH_DAP_TARGET"
unset OTTO_SH_DAP_EVT OTTO_SH_DAP_CMD OTTO_SH_DAP_TARGET

exec 8<>"$_otto_fifo_cmd"
exec 9<>"$_otto_fifo_evt"

_otto_mode="step_in"
_otto_target_depth=0
_otto_bps="|"
_otto_started=0

# Wait for initial breakpoint configuration & configurationDone before running script
while IFS= read -r _otto_init_cmd <&8; do
  case "$_otto_init_cmd" in
    BPS\|*) _otto_bps="${_otto_init_cmd#BPS}" ;;
    RUN\|*) _otto_mode="${_otto_init_cmd#RUN|}"; break ;;
    QUIT) exit 0 ;;
  esac
done

_otto_trap_handler() {
  [ "${BASH_SUBSHELL:-0}" -eq 0 ] || return 0
  local _fn="${FUNCNAME[1]:-main}"
  case "$_fn" in
    _otto_*) return 0 ;;
  esac
  local _src="${BASH_SOURCE[1]:-$_otto_target}"
  local _ln="${1:-1}"
  [ -z "$_src" ] && return 0
  case "$_src" in
    /*) ;;
    *) _src="$PWD/$_src" ;;
  esac

  local _depth="${#FUNCNAME[@]}"
  local _hit=0
  case "$_otto_bps" in
    *"|$_src:$_ln|"*) _hit=1 ;;
  esac

  local _stop=0
  if [ "$_hit" -eq 1 ]; then
    _stop=1
  elif [ "$_otto_mode" = "step_in" ]; then
    _stop=1
  elif [ "$_otto_mode" = "step_over" ] && [ "$_depth" -le "$_otto_target_depth" ]; then
    _stop=1
  elif [ "$_otto_mode" = "step_out" ] && [ "$_depth" -lt "$_otto_target_depth" ]; then
    _stop=1
  fi
  [ "$_stop" -eq 1 ] || return 0

  local _reason="step"
  [ "$_hit" -eq 1 ] && _reason="breakpoint"

  printf 'STOP|%s|%s|%s|%s\n' "$_reason" "$_fn" "$_src" "$_ln" >&9
  local _i=1
  local _max="${#BASH_SOURCE[@]}"
  while [ "$_i" -lt "$_max" ]; do
    local _f_name="${FUNCNAME[$_i]:-main}"
    local _f_src="${BASH_SOURCE[$_i]:-$_src}"
    local _f_ln="$_ln"
    if [ "$_i" -gt 1 ]; then
      local _prev=$((_i - 1))
      _f_ln="${BASH_LINENO[$_prev]:-1}"
    fi
    printf 'FRAME|%s|%s|%s\n' "$_f_name" "$_f_src" "$_f_ln" >&9
    _i=$((_i + 1))
  done

  while IFS='=' read -r _vk _vv; do
    case "$_vk" in
      _otto_*|BASH*|FUNCNAME|LINENO|OPTIND|OPTERR|PPID|SHELLOPTS|UID|EUID|GROUPS|IFS|_*|*[!A-Za-z0-9_]*) ;;
      *) printf 'VAR|%s|%s\n' "$_vk" "${_vv}" >&9 ;;
    esac
  done <<EOF_VARS
$(set -o posix; set)
EOF_VARS
  printf 'END_STOP\n' >&9

  while IFS= read -r _cmd <&8; do
    case "$_cmd" in
      BPS\|*) _otto_bps="${_cmd#BPS}" ;;
      CONT) _otto_mode="run"; return 0 ;;
      NEXT) _otto_mode="step_over"; _otto_target_depth="$_depth"; return 0 ;;
      STEP) _otto_mode="step_in"; return 0 ;;
      OUT)  _otto_mode="step_out"; _otto_target_depth="$_depth"; return 0 ;;
      EVAL\|*)
        local _expr="${_cmd#EVAL|}"
        local _out=""
        local _rc=0
        _out="$(eval "$_expr" 2>&1)" || _rc=$?
        _out="${_out//$'\n'/ }"
        printf 'EVAL_RES|%s|%s\n' "$_rc" "$_out" >&9
        ;;
      QUIT) exit 0 ;;
    esac
  done
}

set -T
trap '_otto_trap_handler "$LINENO"' DEBUG
. "$_otto_target" "$@"
"#;

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
    tokio::fs::create_dir_all(&ipc_dir)
        .await
        .map_err(|e| Error::Internal(format!("create shell debug ipc dir: {e}")))?;
    let cmd_fifo = ipc_dir.join("cmd.fifo");
    let evt_fifo = ipc_dir.join("evt.fifo");
    let prelude_file = ipc_dir.join("prelude.bash");

    make_fifo(&cmd_fifo).await?;
    make_fifo(&evt_fifo).await?;
    tokio::fs::write(&prelude_file, BASH_PRELUDE)
        .await
        .map_err(|e| Error::Internal(format!("write shell debug prelude: {e}")))?;

    let mut env = spec.env.clone();
    env.insert(
        "OTTO_SH_DAP_CMD".into(),
        cmd_fifo.to_string_lossy().into_owned(),
    );
    env.insert(
        "OTTO_SH_DAP_EVT".into(),
        evt_fifo.to_string_lossy().into_owned(),
    );
    env.insert("OTTO_SH_DAP_TARGET".into(), spec.program.clone());

    let mut cmd = Command::new(&spec.bash_bin);
    cmd.arg(&prelude_file)
        .args(&spec.args)
        .current_dir(&spec.cwd)
        .envs(&env)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("failed to spawn bash debugger: {e}")))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let guard = BridgeGuard::new();
    guard.push_child(child);
    guard.push_temp_dir(ipc_dir.clone());

    let cmd_pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&cmd_fifo)
        .await
        .map_err(|e| Error::Internal(format!("open cmd fifo: {e}")))?;
    let evt_pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&evt_fifo)
        .await
        .map_err(|e| Error::Internal(format!("open evt fifo: {e}")))?;

    let (client_end, bridge_end) = tokio::io::duplex(65536);
    tokio::spawn(run_shell_bridge_loop(
        bridge_end, cmd_pipe, evt_pipe, stdout, stderr, ipc_dir,
    ));

    Ok((client_end, guard))
}

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

enum TrapMsg {
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

async fn run_shell_bridge_loop(
    bridge_io: DuplexStream,
    mut cmd_pipe: tokio::fs::File,
    evt_pipe: tokio::fs::File,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    ipc_dir: PathBuf,
) {
    let (r_half, w_half) = tokio::io::split(bridge_io);
    let mut dap_in = DapReader::new(r_half);
    let mut dap_out = DapWriter::new(w_half);

    let (trap_tx, mut trap_rx) = mpsc::unbounded_channel::<TrapMsg>();

    if let Some(out) = stdout {
        let tx = trap_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(TrapMsg::Output {
                    category: "stdout",
                    text: format!("{l}\n"),
                });
            }
        });
    }
    if let Some(err) = stderr {
        let tx = trap_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(TrapMsg::Output {
                    category: "stderr",
                    text: format!("{l}\n"),
                });
            }
        });
    }

    tokio::spawn(async move {
        let mut lines = BufReader::new(evt_pipe).lines();
        let mut cur_reason = String::new();
        let mut cur_frames: Vec<(String, String, u32)> = Vec::new();
        let mut cur_vars: Vec<(String, String)> = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(rest) = line.strip_prefix("STOP|") {
                let parts: Vec<&str> = rest.splitn(4, '|').collect();
                if parts.len() == 4 {
                    cur_reason = parts[0].to_string();
                    cur_frames.clear();
                    cur_vars.clear();
                }
            } else if let Some(rest) = line.strip_prefix("FRAME|") {
                let parts: Vec<&str> = rest.splitn(3, '|').collect();
                if parts.len() == 3 {
                    let ln = parts[2].parse::<u32>().unwrap_or(1);
                    let canon = std::fs::canonicalize(parts[1])
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|_| parts[1].to_string());
                    cur_frames.push((parts[0].to_string(), canon, ln));
                }
            } else if let Some(rest) = line.strip_prefix("VAR|") {
                if let Some((k, v)) = rest.split_once('|') {
                    cur_vars.push((k.to_string(), v.to_string()));
                }
            } else if line == "END_STOP" {
                let _ = trap_tx.send(TrapMsg::Stopped {
                    reason: std::mem::take(&mut cur_reason),
                    state: PausedState {
                        frames: std::mem::take(&mut cur_frames),
                        vars: std::mem::take(&mut cur_vars),
                    },
                });
            } else if let Some(rest) = line.strip_prefix("EVAL_RES|") {
                if let Some((rc_s, out_s)) = rest.split_once('|') {
                    let _ = trap_tx.send(TrapMsg::EvalResult {
                        rc: rc_s.parse().unwrap_or(0),
                        output: out_s.to_string(),
                    });
                }
            }
        }
        let _ = trap_tx.send(TrapMsg::Eof);
    });

    let mut bps_by_file: HashMap<String, Vec<ShellBp>> = HashMap::new();
    let mut paused: Option<PausedState> = None;
    let mut pending_eval_seq: Option<i64> = None;

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
                        let _ = cmd_pipe.write_all(format!("BPS{encoded}\n").as_bytes()).await;
                        let _ = dap_out.send_response(req.seq, "setBreakpoints", Ok(Some(json!({ "breakpoints": ack })))).await;
                    }
                    "configurationDone" => {
                        let initial_mode = if bps_by_file.values().any(|v| !v.is_empty()) { "run" } else { "step_in" };
                        let _ = cmd_pipe.write_all(format!("RUN|{initial_mode}\n").as_bytes()).await;
                        let _ = dap_out.send_response(req.seq, "configurationDone", Ok(Some(json!({})))).await;
                    }
                    "threads" => {
                        let _ = dap_out.send_response(req.seq, "threads", Ok(Some(json!({
                            "threads": [{ "id": 1, "name": "bash main" }]
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
    let _ = tokio::fs::remove_dir_all(&ipc_dir).await;
}

fn encode_bp_list(bps_by_file: &HashMap<String, Vec<ShellBp>>) -> String {
    let mut out = String::from("|");
    for (path, list) in bps_by_file {
        for bp in list {
            out.push_str(&format!("{path}:{}|", bp.line));
        }
    }
    out
}

fn interpolate_shell_log(tpl: &str, vars: &[(String, String)]) -> String {
    let mut out = tpl.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

async fn check_shell_condition(cond: &str, vars: &[(String, String)]) -> bool {
    let mut cmd = Command::new("/bin/bash");
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let expr = if cond.trim_start().starts_with('[') || cond.trim_start().starts_with("test ") {
        cond.to_string()
    } else {
        format!("(( {cond} )) 2>/dev/null || [ {cond} ] 2>/dev/null")
    };
    cmd.arg("-c")
        .arg(expr)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}
