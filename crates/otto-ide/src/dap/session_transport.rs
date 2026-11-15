//! Transport connection, handshake, and asynchronous DAP event driver for [`super::session::DapManager`].

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use otto_core::{Error, Result};
use otto_ssh::SshTunnel;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use super::adapters::{DapAdapterTransport, ResolvedDapAdapter};
use super::java_bridge::spawn_java_jdwp_bridge;
use super::node_bridge::spawn_node_inspector_bridge;
use super::session::{
    mark_resumed, ActiveConnection, BoxRead, BoxWrite, DapStartContext, DapWireClient,
    SessionEntry, MAX_TERMINATED_DEBUG_SESSIONS,
};
use super::session_inspect::{apply_breakpoint_event, handle_stopped_event, sync_all_breakpoints};
use super::shell_bridge::spawn_shell_dap_bridge;
use super::wire::DapEvent;
use super::{spawn_target_process, ssh_tunnel_config, BridgeGuard};
use crate::types::{IdeDebugState, IdeStartDebugReq, IdeTargetKind};

pub(super) fn spawn_line_reader<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: R,
    mut on_line: impl FnMut(String) + Send + 'static,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            on_line(line);
        }
    });
}

pub(super) async fn resolve_target_endpoint(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    guard: &BridgeGuard,
    host: String,
    port: u16,
) -> Result<(String, u16)> {
    if req.target.kind != IdeTargetKind::Ssh {
        return Ok((host, port));
    }
    let ssh = ctx
        .ssh
        .as_ref()
        .ok_or_else(|| Error::Invalid("missing SSH spec".into()))?;
    let tunnel = SshTunnel::open(&ssh_tunnel_config(ssh)?, &host, port).await?;
    let local_port = tunnel.local_port();
    guard.push_tunnel(tunnel);
    Ok(("127.0.0.1".to_string(), local_port))
}

pub(super) fn parse_listening_port(line: &str) -> Option<u16> {
    if let Some(idx) = line.rfind(':') {
        let tail = line[idx + 1..].trim().trim_end_matches('.');
        if let Some(port) = tail.parse::<u16>().ok().filter(|&p| p > 0) {
            return Some(port);
        }
    }
    let words: Vec<&str> = line.split_whitespace().collect();
    for win in words.windows(2) {
        if win[0].eq_ignore_ascii_case("port") {
            let trimmed = win[1].trim_matches(|c: char| !c.is_ascii_digit());
            if let Some(p) = trimmed.parse::<u16>().ok().filter(|&p| p > 0) {
                return Some(p);
            }
        }
    }
    None
}

pub(super) fn evict_terminated_sessions(sessions: &DashMap<String, Arc<SessionEntry>>) {
    let mut terminated: Vec<(String, u64)> = sessions
        .iter()
        .filter_map(|item| {
            item.value()
                .is_terminal()
                .then(|| (item.key().clone(), item.value().created_seq))
        })
        .collect();
    if terminated.len() <= MAX_TERMINATED_DEBUG_SESSIONS {
        return;
    }
    terminated.sort_by_key(|(_, seq)| *seq);
    let excess = terminated.len() - MAX_TERMINATED_DEBUG_SESSIONS;
    for (id, _) in terminated.into_iter().take(excess) {
        if let Some((_, entry)) = sessions.remove(&id) {
            entry.disconnect_and_kill();
        }
    }
}

pub(super) async fn boot_session_connection(
    sessions: &Arc<DashMap<String, Arc<SessionEntry>>>,
    entry: &Arc<SessionEntry>,
    resolved: ResolvedDapAdapter,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
) -> Result<()> {
    let io = connect_adapter_transport(entry, &resolved.adapter_name, resolved.transport, req, ctx)
        .await?;
    let cmds = (
        resolved.initialize_args,
        resolved.launch_command.as_str(),
        resolved.launch_args,
    );
    handshake_and_run(sessions, entry, io, cmds).await
}

async fn connect_adapter_transport(
    entry: &Arc<SessionEntry>,
    adapter_name: &str,
    transport: DapAdapterTransport,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
) -> Result<(BoxRead, BoxWrite, Arc<BridgeGuard>)> {
    let guard = BridgeGuard::new();
    match transport {
        DapAdapterTransport::Stdio {
            command,
            args,
            cwd,
            env,
        } => {
            let mut child =
                spawn_target_process(&req.target, ctx.ssh.as_ref(), &command, &args, &cwd, &env)?;
            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| Error::Internal("missing DAP adapter stdin".into()))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| Error::Internal("missing DAP adapter stdout".into()))?;
            if let Some(stderr) = child.stderr.take() {
                let entry_err = entry.clone();
                spawn_line_reader(stderr, move |l| {
                    entry_err.append_output("stderr", format!("{l}\n"));
                });
            }
            guard.push_child(child);
            Ok((Box::new(stdout), Box::new(stdin), guard))
        }
        DapAdapterTransport::TcpListen {
            command,
            args,
            cwd,
            env,
        } => {
            let cmd_spec = (command.as_str(), args.as_slice(), cwd.as_str(), &env);
            spawn_tcp_listen_adapter(entry, adapter_name, cmd_spec, req, ctx, guard).await
        }
        DapAdapterTransport::DirectTcpAttach { host, port } => {
            let (connect_host, connect_port) =
                resolve_target_endpoint(req, ctx, &guard, host, port).await?;
            let tcp = TcpStream::connect((connect_host.as_str(), connect_port))
                .await
                .map_err(|e| {
                    Error::Upstream(format!("attach DAP {connect_host}:{connect_port}: {e}"))
                })?;
            let _ = tcp.set_nodelay(true);
            let (r, w) = tcp.into_split();
            Ok((Box::new(r), Box::new(w), guard))
        }
        DapAdapterTransport::NodeInspectorBridge(spec) => {
            let (duplex, bridge_guard) =
                spawn_node_inspector_bridge(&req.target, ctx.ssh.as_ref(), spec).await?;
            let (r, w) = tokio::io::split(duplex);
            Ok((Box::new(r), Box::new(w), bridge_guard))
        }
        DapAdapterTransport::JavaJdwpBridge(spec) => {
            let (duplex, bridge_guard) =
                spawn_java_jdwp_bridge(&req.target, ctx.ssh.as_ref(), spec).await?;
            let (r, w) = tokio::io::split(duplex);
            Ok((Box::new(r), Box::new(w), bridge_guard))
        }
        DapAdapterTransport::ShellBridge(spec) => {
            let (duplex, bridge_guard) =
                spawn_shell_dap_bridge(&req.target, ctx.ssh.as_ref(), spec).await?;
            let (r, w) = tokio::io::split(duplex);
            Ok((Box::new(r), Box::new(w), bridge_guard))
        }
    }
}

async fn spawn_tcp_listen_adapter(
    entry: &Arc<SessionEntry>,
    adapter_name: &str,
    (command, args, cwd, env): (&str, &[String], &str, &BTreeMap<String, String>),
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    guard: Arc<BridgeGuard>,
) -> Result<(BoxRead, BoxWrite, Arc<BridgeGuard>)> {
    let mut child = spawn_target_process(&req.target, ctx.ssh.as_ref(), command, args, cwd, env)?;
    let (port_tx, mut port_rx) = mpsc::unbounded_channel::<(String, String)>();
    if let Some(out) = child.stdout.take() {
        let tx = port_tx.clone();
        spawn_line_reader(out, move |l| {
            let _ = tx.send(("stdout".into(), l));
        });
    }
    if let Some(err) = child.stderr.take() {
        let tx = port_tx.clone();
        spawn_line_reader(err, move |l| {
            let _ = tx.send(("stderr".into(), l));
        });
    }
    drop(port_tx);

    let mut found_port: Option<u16> = None;
    let mut diag = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), port_rx.recv()).await {
            Ok(Some((_, line))) => {
                if let Some(p) = parse_listening_port(&line) {
                    found_port = Some(p);
                    break;
                }
                if diag.len() < 4096 {
                    diag.push_str(&line);
                    diag.push('\n');
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let Some(remote_port) = found_port else {
        let _ = child.start_kill();
        return Err(Error::Upstream(format!(
            "DAP adapter '{adapter_name}' failed to report listening port: {}",
            diag.trim()
        )));
    };
    let entry_out = entry.clone();
    tokio::spawn(async move {
        while let Some((cat, l)) = port_rx.recv().await {
            entry_out.append_output(&cat, format!("{l}\n"));
        }
    });
    guard.push_child(child);

    let (connect_host, connect_port) =
        resolve_target_endpoint(req, ctx, &guard, "127.0.0.1".into(), remote_port).await?;
    let tcp = TcpStream::connect((connect_host.as_str(), connect_port))
        .await
        .map_err(|e| Error::Upstream(format!("connect DAP {connect_host}:{connect_port}: {e}")))?;
    let _ = tcp.set_nodelay(true);
    let (r, w) = tcp.into_split();
    Ok((Box::new(r), Box::new(w), guard))
}

pub(super) async fn handshake_and_run(
    sessions: &Arc<DashMap<String, Arc<SessionEntry>>>,
    entry: &Arc<SessionEntry>,
    (read_io, write_io, guard): (BoxRead, BoxWrite, Arc<BridgeGuard>),
    (initialize_args, launch_command, launch_args): (Value, &str, Value),
) -> Result<()> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let wire = DapWireClient::spawn(read_io, write_io, ev_tx);

    spawn_event_driver(Arc::clone(sessions), entry.clone(), ev_rx);

    let init_resp = wire.request("initialize", Some(initialize_args)).await?;
    let caps = init_resp.body.as_ref();
    let supports_terminate = caps
        .and_then(|c| c.get("supportsTerminateRequest"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let exception_filters: Vec<String> = caps
        .and_then(|c| c.get("exceptionBreakpointFilters"))
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|f| f.get("filter").and_then(Value::as_str).map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let active_conn = Arc::new(ActiveConnection {
        wire: wire.clone(),
        guard,
        supports_terminate,
        exception_filters,
    });
    *entry.conn.write().unwrap_or_else(|e| e.into_inner()) = Some(active_conn.clone());

    let mut launch_rx = wire
        .send_request_nowait(launch_command, Some(launch_args))
        .await?;

    let mut launch_done = false;
    tokio::select! {
        _ = tokio::time::timeout(Duration::from_secs(8), entry.initialized_notify.notified()) => {}
        launch_res = &mut launch_rx => {
            launch_done = true;
            match launch_res {
                Ok(resp) if !resp.success => {
                    let msg = resp.message.unwrap_or_else(|| "Debug target launch failed".into());
                    return Err(Error::Invalid(msg));
                }
                Err(_) => {
                    return Err(Error::Upstream(
                        "DAP adapter closed before initialization completed".into(),
                    ));
                }
                Ok(_) => {
                    let _ = tokio::time::timeout(
                        Duration::from_millis(200),
                        entry.initialized_notify.notified(),
                    )
                    .await;
                }
            }
        }
    }

    sync_all_breakpoints(entry, &active_conn).await?;
    let _ = wire.request("configurationDone", Some(json!({}))).await;

    if !launch_done {
        if let Ok(launch_res) =
            tokio::time::timeout(Duration::from_millis(60), &mut launch_rx).await
        {
            launch_done = true;
            if let Ok(resp) = launch_res {
                if !resp.success {
                    let msg = resp
                        .message
                        .unwrap_or_else(|| "Debug target launch failed".into());
                    return Err(Error::Invalid(msg));
                }
            }
        }
    }

    entry.mutate_and_broadcast(|snap| {
        if snap.state == IdeDebugState::Starting {
            snap.state = IdeDebugState::Running;
        }
    });

    if !launch_done {
        let sessions_launch = Arc::clone(sessions);
        let entry_launch = entry.clone();
        tokio::spawn(async move {
            if let Ok(resp) = launch_rx.await {
                if !resp.success {
                    let msg = resp
                        .message
                        .unwrap_or_else(|| "Debug target launch failed".into());
                    entry_launch.fail_and_evict(&sessions_launch, msg);
                }
            }
        });
    }

    Ok(())
}

fn spawn_event_driver(
    sessions: Arc<DashMap<String, Arc<SessionEntry>>>,
    entry: Arc<SessionEntry>,
    mut ev_rx: mpsc::UnboundedReceiver<DapEvent>,
) {
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            match ev.event.as_str() {
                "initialized" => {
                    entry.initialized_notify.notify_one();
                }
                "breakpoint" => {
                    if let Some(body) = &ev.body {
                        apply_breakpoint_event(&entry, body);
                    }
                }
                "output" => {
                    if let Some(body) = &ev.body {
                        let cat = body
                            .get("category")
                            .and_then(Value::as_str)
                            .unwrap_or("stdout");
                        if let Some(txt) = body
                            .get("output")
                            .and_then(Value::as_str)
                            .filter(|_| cat != "telemetry")
                        {
                            entry.append_output(cat, txt.to_string());
                        }
                    }
                }
                "continued" => {
                    if !entry.suppress_transient_pause.load(Ordering::SeqCst) {
                        mark_resumed(&entry);
                    }
                }
                "stopped" => {
                    let reason = ev
                        .body
                        .as_ref()
                        .and_then(|b| b.get("reason"))
                        .and_then(Value::as_str)
                        .unwrap_or("breakpoint")
                        .to_string();
                    if entry.suppress_transient_pause.load(Ordering::SeqCst)
                        && matches!(reason.as_str(), "pause" | "halt" | "interrupt")
                    {
                        entry.halted_notify.notify_waiters();
                        continue;
                    }
                    let stopped_tid = ev
                        .body
                        .as_ref()
                        .and_then(|b| b.get("threadId"))
                        .and_then(Value::as_i64);
                    if let Ok(conn) = entry.active_conn() {
                        handle_stopped_event(&entry, &conn, reason, stopped_tid).await;
                    }
                }
                "terminated" | "exited" | "__eof__" => {
                    entry.disconnect_and_kill();
                    if !entry.is_terminal() {
                        entry.mark_terminated();
                    }
                    evict_terminated_sessions(&sessions);
                    if ev.event == "__eof__" {
                        break;
                    }
                }
                _ => {}
            }
        }
    });
}
