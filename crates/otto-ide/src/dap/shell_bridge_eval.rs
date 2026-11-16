//! Helper functions for Shell DAP FIFO event reading, breakpoint encoding,
//! logpoint interpolation, and conditional breakpoint evaluation with a scrubbed env.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use crate::line_reader::{LossyLines, MAX_OUTPUT_LINE_BYTES, MAX_PROTOCOL_LINE_BYTES};
use tokio::net::unix::pipe::{Receiver as PipeReceiver, Sender as PipeSender};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::shell_bridge::{PausedState, ShellBp, TrapMsg};

pub(super) fn spawn_trap_readers(
    evt_pipe: PipeReceiver,
    evt_keepalive: Arc<Mutex<Option<PipeSender>>>,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    trap_tx: mpsc::UnboundedSender<TrapMsg>,
) -> (
    Option<JoinHandle<()>>,
    Option<JoinHandle<()>>,
    JoinHandle<()>,
) {
    let stdout_handle = stdout.map(|out| {
        let tx = trap_tx.clone();
        let keepalive = Arc::clone(&evt_keepalive);
        tokio::spawn(async move {
            let mut lines = LossyLines::new(out, MAX_OUTPUT_LINE_BYTES);
            while let Some(l) = lines.next_line().await {
                let _ = tx.send(TrapMsg::Output {
                    category: "stdout",
                    text: format!("{l}\n"),
                });
            }
            if let Ok(mut g) = keepalive.lock() {
                *g = None;
            }
        })
    });
    let stderr_handle = stderr.map(|err| {
        let tx = trap_tx.clone();
        tokio::spawn(async move {
            let mut lines = LossyLines::new(err, MAX_OUTPUT_LINE_BYTES);
            while let Some(l) = lines.next_line().await {
                let _ = tx.send(TrapMsg::Output {
                    category: "stderr",
                    text: format!("{l}\n"),
                });
            }
        })
    });
    let evt_handle = tokio::spawn(async move {
        let mut lines = LossyLines::new(evt_pipe, MAX_PROTOCOL_LINE_BYTES);
        let mut cur_reason = String::new();
        let mut cur_frames: Vec<(String, String, u32)> = Vec::new();
        let mut cur_vars: Vec<(String, String)> = Vec::new();
        while let Some(line) = lines.next_line().await {
            if line == "READY" {
                if let Ok(mut g) = evt_keepalive.lock() {
                    *g = None;
                }
                continue;
            }
            if line == "EXIT" {
                break;
            }
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
    (stdout_handle, stderr_handle, evt_handle)
}

pub(super) fn encode_bp_list(bps_by_file: &HashMap<String, Vec<ShellBp>>) -> String {
    let mut out = String::from("|");
    for (path, list) in bps_by_file {
        for bp in list {
            out.push_str(&format!("{path}:{}|", bp.line));
        }
    }
    out
}

pub(super) fn interpolate_shell_log(tpl: &str, vars: &[(String, String)]) -> String {
    let mut out = tpl.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), v);
        out = out.replace(&format!("${{{k}}}"), v);
    }
    out
}

pub(super) async fn check_shell_condition(cond: &str, vars: &[(String, String)]) -> bool {
    let mut cmd = Command::new("/bin/bash");
    cmd.env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env());
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
