//! Built-in Node.js / V8 Inspector-to-DAP Bridge (`node --inspect-brk=127.0.0.1:0`).
//!
//! Speaks RFC 6455 loopback WebSocket + Chrome DevTools Protocol (`Debugger.*` and
//! `Runtime.*`) to Node.js on one side and `Content-Length` DAP over an in-memory
//! [`tokio::io::DuplexStream`] on the other, so JS/TS debugging works out of the box
//! on any machine with Node.js installed even before downloading `js-debug`.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, ErrorKind};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use otto_ssh::SshTunnel;
use serde_json::{json, Value};
use tokio::io::{
    AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader, DuplexStream,
};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use super::wire::{self, DapMessage, DapReader, DapRequest, DapWriter};
use super::{spawn_target_process, ssh_tunnel_config, BridgeGuard};
use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

/// Launch/attach parameters for the built-in Node/V8 Inspector-to-DAP bridge.
#[derive(Debug, Clone)]
pub struct NodeInspectorBridgeSpec {
    pub node_bin: String,
    pub node_flags: Vec<String>,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub attach_port: Option<u16>,
}

struct InspectorWs {
    reader: BufReader<tokio::net::tcp::OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
}

impl InspectorWs {
    async fn connect(port: u16, ws_path: &str) -> Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|e| Error::Upstream(format!("connect V8 inspector 127.0.0.1:{port}: {e}")))?;
        let (read_half, mut writer) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let req = format!(
            "GET {ws_path} HTTP/1.1\r\n\
             Host: 127.0.0.1:{port}\r\n\
             Upgrade: websocket\r\n\
             Connection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
             Sec-WebSocket-Version: 13\r\n\r\n"
        );
        writer
            .write_all(req.as_bytes())
            .await
            .map_err(|e| Error::Upstream(format!("inspector handshake write: {e}")))?;
        writer
            .flush()
            .await
            .map_err(|e| Error::Upstream(format!("inspector handshake flush: {e}")))?;

        let mut status_ok = false;
        loop {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .await
                .map_err(|e| Error::Upstream(format!("inspector handshake read: {e}")))?;
            if n == 0 {
                return Err(Error::Upstream("V8 inspector closed before upgrade".into()));
            }
            if line.starts_with("HTTP/1.1 101") || line.starts_with("HTTP/1.0 101") {
                status_ok = true;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
        }
        if !status_ok {
            return Err(Error::Upstream(
                "V8 inspector rejected WebSocket upgrade".into(),
            ));
        }
        Ok(Self { reader, writer })
    }

    async fn send_json(&mut self, val: &Value) -> io::Result<()> {
        let payload = serde_json::to_vec(val)
            .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))?;
        Self::write_masked_frame(&mut self.writer, 0x81, &payload).await
    }

    async fn write_masked_frame(
        writer: &mut tokio::net::tcp::OwnedWriteHalf,
        first_byte: u8,
        payload: &[u8],
    ) -> io::Result<()> {
        let mask: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
        let mut header = Vec::with_capacity(14);
        header.push(first_byte);
        let len = payload.len();
        if len < 126 {
            header.push(0x80 | (len as u8));
        } else if len <= 65535 {
            header.push(0x80 | 126);
            header.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            header.push(0x80 | 127);
            header.extend_from_slice(&(len as u64).to_be_bytes());
        }
        header.extend_from_slice(&mask);
        let masked: Vec<u8> = payload
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ mask[i & 3])
            .collect();
        writer.write_all(&header).await?;
        writer.write_all(&masked).await?;
        writer.flush().await
    }

    async fn next_json(&mut self) -> io::Result<Option<Value>> {
        let mut message_buf = Vec::new();
        loop {
            let mut hdr = [0u8; 2];
            if self.reader.read_exact(&mut hdr).await.is_err() {
                return Ok(None);
            }
            let fin = (hdr[0] & 0x80) != 0;
            let opcode = hdr[0] & 0x0f;
            let masked = (hdr[1] & 0x80) != 0;
            let mut len = (hdr[1] & 0x7f) as usize;
            if len == 126 {
                let mut ext = [0u8; 2];
                self.reader.read_exact(&mut ext).await?;
                len = u16::from_be_bytes(ext) as usize;
            } else if len == 127 {
                let mut ext = [0u8; 8];
                self.reader.read_exact(&mut ext).await?;
                len = usize::try_from(u64::from_be_bytes(ext)).unwrap_or(usize::MAX);
            }
            if len > wire::MAX_MESSAGE_BYTES {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "V8 inspector frame exceeds 16 MiB",
                ));
            }
            let mut mask = [0u8; 4];
            if masked {
                self.reader.read_exact(&mut mask).await?;
            }
            let mut chunk = vec![0u8; len];
            self.reader.read_exact(&mut chunk).await?;
            if masked {
                for (i, b) in chunk.iter_mut().enumerate() {
                    *b ^= mask[i & 3];
                }
            }
            match opcode {
                0x8 => return Ok(None),
                0x9 => Self::write_masked_frame(&mut self.writer, 0x8a, &chunk).await?,
                0x0 | 0x1 => {
                    message_buf.extend_from_slice(&chunk);
                    if fin {
                        let val: Value = serde_json::from_slice(&message_buf).map_err(|e| {
                            io::Error::new(ErrorKind::InvalidData, format!("cdp json: {e}"))
                        })?;
                        return Ok(Some(val));
                    }
                }
                _ => {}
            }
        }
    }
}

async fn discover_attach_ws_path(port: u16) -> Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| Error::Upstream(format!("connect Node inspector 127.0.0.1:{port}: {e}")))?;
    let req =
        format!("GET /json/list HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .await
        .map_err(|e| Error::Upstream(format!("query /json/list: {e}")))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .await
        .map_err(|e| Error::Upstream(format!("read /json/list: {e}")))?;
    let text = String::from_utf8_lossy(&raw);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or(text.as_ref());
    let arr: Value = serde_json::from_str(body.trim())
        .map_err(|e| Error::Upstream(format!("parse /json/list: {e}")))?;
    let ws_url = arr
        .as_array()
        .and_then(|a| a.first())
        .and_then(|item| item.get("webSocketDebuggerUrl"))
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Upstream("Node inspector returned no debuggable target".into()))?;
    extract_ws_port_and_path(ws_url)
        .map(|(_, path)| path)
        .ok_or_else(|| Error::Upstream(format!("invalid webSocketDebuggerUrl '{ws_url}'")))
}

fn extract_ws_port_and_path(line: &str) -> Option<(u16, String)> {
    let idx = line.find("ws://")?;
    let rest = &line[idx + 5..];
    let token = rest.split_whitespace().next()?;
    let (host_port, path_part) = match token.split_once('/') {
        Some((hp, p)) => (hp, format!("/{p}")),
        None => (token, "/".to_string()),
    };
    let port: u16 = host_port.rsplit_once(':')?.1.parse().ok()?;
    Some((port, path_part))
}

/// Spawn the built-in Node/V8 Inspector-to-DAP bridge, returning an in-memory
/// [`DuplexStream`] speaking standard `Content-Length` DAP.
pub(crate) async fn spawn_node_inspector_bridge(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: NodeInspectorBridgeSpec,
) -> Result<(DuplexStream, Arc<BridgeGuard>)> {
    let guard = BridgeGuard::new();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(String, String)>();

    let (local_port, ws_path) = if let Some(attach_port) = spec.attach_port {
        let port = maybe_tunnel_port(target, ssh, &guard, attach_port).await?;
        let path = discover_attach_ws_path(port).await?;
        (port, path)
    } else {
        let (remote_port, path) = spawn_inspected_node(target, ssh, &spec, &guard, &out_tx).await?;
        let port = maybe_tunnel_port(target, ssh, &guard, remote_port).await?;
        (port, path)
    };

    let ws = InspectorWs::connect(local_port, &ws_path).await?;
    let (client_io, server_io) = tokio::io::duplex(65536);
    let guard_for_task = guard.clone();
    tokio::spawn(async move {
        let _ = run_node_bridge_loop(server_io, ws, &mut out_rx, guard_for_task).await;
    });
    Ok((client_io, guard))
}

async fn maybe_tunnel_port(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    guard: &BridgeGuard,
    remote_port: u16,
) -> Result<u16> {
    if target.kind == IdeTargetKind::Ssh {
        let ssh_spec = ssh.ok_or_else(|| Error::Invalid("missing SSH spec".into()))?;
        let tunnel =
            SshTunnel::open(&ssh_tunnel_config(ssh_spec)?, "127.0.0.1", remote_port).await?;
        let lp = tunnel.local_port();
        guard.push_tunnel(tunnel);
        Ok(lp)
    } else {
        Ok(remote_port)
    }
}

async fn spawn_inspected_node(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: &NodeInspectorBridgeSpec,
    guard: &BridgeGuard,
    out_tx: &mpsc::UnboundedSender<(String, String)>,
) -> Result<(u16, String)> {
    let mut argv = vec!["--inspect-brk=127.0.0.1:0".to_string()];
    argv.extend(spec.node_flags.clone());
    argv.push(spec.program.clone());
    argv.extend(spec.args.clone());

    let mut child = spawn_target_process(target, ssh, &spec.node_bin, &argv, &spec.cwd, &spec.env)?;
    let stdout = child.stdout.take();
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("missing node stderr pipe".into()))?;

    if let Some(out) = stdout {
        let tx = out_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = tx.send(("stdout".into(), format!("{line}\n")));
            }
        });
    }

    let mut err_lines = BufReader::new(stderr).lines();
    let mut found: Option<(u16, String)> = None;
    let mut boot_err = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), err_lines.next_line()).await {
            Ok(Ok(Some(line))) => {
                if let Some(parsed) = extract_ws_port_and_path(&line) {
                    found = Some(parsed);
                    break;
                }
                if boot_err.len() < 4096 {
                    boot_err.push_str(&line);
                    boot_err.push('\n');
                }
            }
            Ok(Ok(None)) => break,
            _ => {}
        }
    }
    let Some(res) = found else {
        let _ = child.start_kill();
        return Err(Error::Upstream(format!(
            "node --inspect-brk did not expose inspector endpoint: {}",
            boot_err.trim()
        )));
    };

    let tx_err = out_tx.clone();
    tokio::spawn(async move {
        while let Ok(Some(line)) = err_lines.next_line().await {
            if !line.starts_with("Debugger listening on ")
                && !line.starts_with("For help, see: https://nodejs.org")
                && !line.starts_with("Debugger attached.")
                && !line.starts_with("Waiting for the debugger to disconnect")
            {
                let _ = tx_err.send(("stderr".into(), format!("{line}\n")));
            }
        }
    });

    guard.push_child(child);
    Ok(res)
}

struct NodeBridgeState {
    cdp_id: i64,
    pending_cdp: HashMap<i64, (i64, String, String)>,
    script_urls: HashMap<String, String>,
    bp_ids_by_file: HashMap<String, Vec<String>>,
    user_bp_lines: HashMap<String, Vec<u32>>,
    initial_brk_skipped: bool,
    frames: Vec<Value>,
    obj_refs: HashMap<i64, String>,
    next_var_ref: i64,
}

impl NodeBridgeState {
    fn new() -> Self {
        Self {
            cdp_id: 1,
            pending_cdp: HashMap::new(),
            script_urls: HashMap::new(),
            bp_ids_by_file: HashMap::new(),
            user_bp_lines: HashMap::new(),
            initial_brk_skipped: false,
            frames: Vec::new(),
            obj_refs: HashMap::new(),
            next_var_ref: 1000,
        }
    }

    fn alloc_cdp(&mut self) -> i64 {
        let id = self.cdp_id;
        self.cdp_id += 1;
        id
    }

    fn alloc_obj_ref(&mut self, object_id: String) -> i64 {
        let r = self.next_var_ref;
        self.next_var_ref += 1;
        self.obj_refs.insert(r, object_id);
        r
    }
}

fn file_url_to_path(url: &str) -> Option<String> {
    url.strip_prefix("file://").map(String::from)
}

fn path_to_url_regex(path: &str) -> String {
    let escaped = regex::escape(path);
    if let Some(name) = Path::new(path).file_name().and_then(|n| n.to_str()) {
        format!("(file://{escaped}|/{}$)", regex::escape(name))
    } else {
        format!("file://{escaped}")
    }
}

async fn run_node_bridge_loop(
    server_io: DuplexStream,
    mut ws: InspectorWs,
    out_rx: &mut mpsc::UnboundedReceiver<(String, String)>,
    guard: Arc<BridgeGuard>,
) -> io::Result<()> {
    let (r, w) = tokio::io::split(server_io);
    let mut dap_r = DapReader::new(r);
    let mut dap_w = DapWriter::new(w);
    let mut st = NodeBridgeState::new();

    loop {
        tokio::select! {
            Some((cat, text)) = out_rx.recv() => {
                dap_w.send_event("output", Some(json!({ "category": cat, "output": text }))).await?;
            }
            dap_msg = dap_r.next_message() => {
                let Some(DapMessage::Request(req)) = dap_msg? else { break; };
                if handle_node_dap_req(&req, &mut st, &mut ws, &mut dap_w, &guard).await? {
                    break;
                }
            }
            cdp_msg = ws.next_json() => {
                let Some(msg) = cdp_msg? else {
                    let _ = dap_w.send_event("terminated", Some(json!({}))).await;
                    break;
                };
                handle_node_cdp_msg(msg, &mut st, &mut ws, &mut dap_w).await?;
            }
        }
    }
    guard.kill_all();
    Ok(())
}

async fn handle_node_dap_req<W: AsyncWrite + Unpin>(
    req: &DapRequest,
    st: &mut NodeBridgeState,
    ws: &mut InspectorWs,
    dap_w: &mut DapWriter<W>,
    guard: &BridgeGuard,
) -> io::Result<bool> {
    let args = req.arguments.as_ref();
    match req.command.as_str() {
        "initialize" => {
            dap_w
                .send_response(
                    req.seq,
                    "initialize",
                    Ok(Some(json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsConditionalBreakpoints": true,
                        "supportsEvaluateForHovers": true,
                        "exceptionBreakpointFilters": [
                            { "filter": "uncaught", "label": "Uncaught Exceptions" },
                            { "filter": "all", "label": "All Exceptions" }
                        ]
                    }))),
                )
                .await?;
        }
        "launch" | "attach" => {
            let id1 = st.alloc_cdp();
            ws.send_json(&json!({ "id": id1, "method": "Runtime.enable" }))
                .await?;
            let id2 = st.alloc_cdp();
            ws.send_json(&json!({ "id": id2, "method": "Debugger.enable" }))
                .await?;
            dap_w
                .send_response(req.seq, &req.command, Ok(Some(json!({}))))
                .await?;
            dap_w.send_event("initialized", Some(json!({}))).await?;
        }
        "setBreakpoints" => handle_node_set_bps(req.seq, args, st, ws, dap_w).await?,
        "setExceptionBreakpoints" => {
            let filters = args
                .and_then(|a| a.get("filters"))
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_default();
            let mode = if filters.contains(&"all") {
                "all"
            } else if !filters.is_empty() {
                "uncaught"
            } else {
                "none"
            };
            let cid = st.alloc_cdp();
            ws.send_json(&json!({
                "id": cid,
                "method": "Debugger.setPauseOnExceptions",
                "params": { "state": mode }
            }))
            .await?;
            dap_w
                .send_response(req.seq, "setExceptionBreakpoints", Ok(Some(json!({}))))
                .await?;
        }
        "configurationDone" => {
            let cid = st.alloc_cdp();
            ws.send_json(&json!({ "id": cid, "method": "Runtime.runIfWaitingForDebugger" }))
                .await?;
            dap_w
                .send_response(req.seq, "configurationDone", Ok(Some(json!({}))))
                .await?;
        }
        "threads" => {
            dap_w
                .send_response(
                    req.seq,
                    "threads",
                    Ok(Some(
                        json!({ "threads": [{ "id": 1, "name": "Node.js Main" }] }),
                    )),
                )
                .await?;
        }
        "stackTrace" => {
            let frames_out = build_node_stack_frames(st);
            dap_w
                .send_response(
                    req.seq,
                    "stackTrace",
                    Ok(Some(
                        json!({ "stackFrames": frames_out, "totalFrames": frames_out.len() }),
                    )),
                )
                .await?;
        }
        "scopes" => {
            let fid = args
                .and_then(|a| a.get("frameId"))
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let scopes_out = build_node_scopes(st, (fid.saturating_sub(1)) as usize);
            dap_w
                .send_response(req.seq, "scopes", Ok(Some(json!({ "scopes": scopes_out }))))
                .await?;
        }
        "variables" | "evaluate" => {
            handle_node_inspect_req(req, args, st, ws, dap_w).await?;
        }
        "continue" | "next" | "stepIn" | "stepOut" | "pause" => {
            let cdp_method = match req.command.as_str() {
                "continue" => "Debugger.resume",
                "next" => "Debugger.stepOver",
                "stepIn" => "Debugger.stepInto",
                "stepOut" => "Debugger.stepOut",
                _ => "Debugger.pause",
            };
            let cid = st.alloc_cdp();
            ws.send_json(&json!({ "id": cid, "method": cdp_method }))
                .await?;
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
            guard.kill_all();
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

async fn handle_node_set_bps<W: AsyncWrite + Unpin>(
    req_seq: i64,
    args: Option<&Value>,
    st: &mut NodeBridgeState,
    ws: &mut InspectorWs,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    let path = args
        .and_then(|a| a.get("source"))
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if let Some(old_ids) = st.bp_ids_by_file.remove(&path) {
        for bid in old_ids {
            let cid = st.alloc_cdp();
            ws.send_json(&json!({
                "id": cid,
                "method": "Debugger.removeBreakpoint",
                "params": { "breakpointId": bid }
            }))
            .await?;
        }
    }
    let bps = args
        .and_then(|a| a.get("breakpoints"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut lines = Vec::new();
    let mut verified = Vec::new();
    let url_re = path_to_url_regex(&path);
    for bp in bps {
        let line = bp.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
        let cond = bp.get("condition").and_then(Value::as_str).unwrap_or("");
        lines.push(line);
        let cid = st.alloc_cdp();
        st.pending_cdp
            .insert(cid, (0, "setBp".into(), path.clone()));
        ws.send_json(&json!({
            "id": cid,
            "method": "Debugger.setBreakpointByUrl",
            "params": {
                "lineNumber": line.saturating_sub(1),
                "urlRegex": url_re,
                "condition": cond
            }
        }))
        .await?;
        verified.push(json!({ "verified": true, "line": line }));
    }
    st.user_bp_lines.insert(path, lines);
    dap_w
        .send_response(
            req_seq,
            "setBreakpoints",
            Ok(Some(json!({ "breakpoints": verified }))),
        )
        .await?;
    Ok(())
}

fn build_node_stack_frames(st: &NodeBridgeState) -> Vec<Value> {
    st.frames
        .iter()
        .enumerate()
        .map(|(idx, cf)| {
            let name = cf
                .get("functionName")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .unwrap_or("(anonymous)");
            let url = cf.get("url").and_then(Value::as_str).unwrap_or("");
            let script_id = cf
                .get("location")
                .and_then(|l| l.get("scriptId"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let resolved_url = if !url.is_empty() {
                url.to_string()
            } else {
                st.script_urls.get(script_id).cloned().unwrap_or_default()
            };
            let source = file_url_to_path(&resolved_url).map(|p| json!({ "path": p }));
            let line = cf
                .get("location")
                .and_then(|l| l.get("lineNumber"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            let col = cf
                .get("location")
                .and_then(|l| l.get("columnNumber"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                + 1;
            json!({
                "id": (idx + 1) as i64,
                "name": name,
                "source": source,
                "line": line,
                "column": col
            })
        })
        .collect()
}

fn build_node_scopes(st: &mut NodeBridgeState, frame_idx: usize) -> Vec<Value> {
    let mut scopes_out = Vec::new();
    let Some(chain) = st
        .frames
        .get(frame_idx)
        .and_then(|cf| cf.get("scopeChain"))
        .and_then(Value::as_array)
        .cloned()
    else {
        return scopes_out;
    };
    for sc in chain {
        let stype = sc.get("type").and_then(Value::as_str).unwrap_or("local");
        if stype == "global" {
            continue;
        }
        if let Some(oid) = sc
            .get("object")
            .and_then(|o| o.get("objectId"))
            .and_then(Value::as_str)
        {
            let vref = st.alloc_obj_ref(oid.to_string());
            let title = match stype {
                "local" => "Locals",
                "block" => "Block",
                "closure" => "Closure",
                "module" | "script" => "Module",
                other => other,
            };
            scopes_out.push(json!({
                "name": title,
                "variablesReference": vref,
                "expensive": false
            }));
        }
    }
    scopes_out
}

async fn handle_node_inspect_req<W: AsyncWrite + Unpin>(
    req: &DapRequest,
    args: Option<&Value>,
    st: &mut NodeBridgeState,
    ws: &mut InspectorWs,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    if req.command == "variables" {
        let vref = args
            .and_then(|a| a.get("variablesReference"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if let Some(oid) = st.obj_refs.get(&vref).cloned() {
            let cid = st.alloc_cdp();
            st.pending_cdp
                .insert(cid, (req.seq, "variables".into(), String::new()));
            ws.send_json(&json!({
                "id": cid,
                "method": "Runtime.getProperties",
                "params": { "objectId": oid, "ownProperties": true, "generatePreview": true }
            }))
            .await?;
        } else {
            dap_w
                .send_response(req.seq, "variables", Ok(Some(json!({ "variables": [] }))))
                .await?;
        }
        return Ok(());
    }

    let expr = args
        .and_then(|a| a.get("expression"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let fid = args
        .and_then(|a| a.get("frameId"))
        .and_then(Value::as_i64)
        .unwrap_or(1);
    let idx = (fid.saturating_sub(1)) as usize;
    let cid = st.alloc_cdp();
    st.pending_cdp
        .insert(cid, (req.seq, "evaluate".into(), String::new()));
    if let Some(cf_id) = st
        .frames
        .get(idx)
        .and_then(|f| f.get("callFrameId"))
        .and_then(Value::as_str)
    {
        ws.send_json(&json!({
            "id": cid,
            "method": "Debugger.evaluateOnCallFrame",
            "params": { "callFrameId": cf_id, "expression": expr, "generatePreview": true }
        }))
        .await
    } else {
        ws.send_json(&json!({
            "id": cid,
            "method": "Runtime.evaluate",
            "params": { "expression": expr, "generatePreview": true }
        }))
        .await
    }
}

fn format_remote_object(st: &mut NodeBridgeState, obj: &Value) -> (String, Option<String>, i64) {
    let vtype = obj.get("type").and_then(Value::as_str).map(String::from);
    if obj.get("subtype").and_then(Value::as_str) == Some("null") {
        return ("null".into(), Some("null".into()), 0);
    }
    let display = if let Some(val) = obj.get("value") {
        match val {
            Value::String(s) => format!("{s:?}"),
            other => other.to_string(),
        }
    } else if let Some(desc) = obj.get("description").and_then(Value::as_str) {
        desc.to_string()
    } else {
        "undefined".into()
    };
    let child_ref = if vtype.as_deref() == Some("object") {
        obj.get("objectId")
            .and_then(Value::as_str)
            .map(|oid| st.alloc_obj_ref(oid.to_string()))
            .unwrap_or(0)
    } else {
        0
    };
    (display, vtype, child_ref)
}

async fn handle_node_cdp_msg<W: AsyncWrite + Unpin>(
    msg: Value,
    st: &mut NodeBridgeState,
    ws: &mut InspectorWs,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    if let Some(id) = msg.get("id").and_then(Value::as_i64) {
        if let Some((req_seq, kind, meta)) = st.pending_cdp.remove(&id) {
            handle_node_cdp_reply(req_seq, &kind, meta, msg.get("result"), st, dap_w).await?;
        }
        return Ok(());
    }

    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params");
    match method {
        "Debugger.scriptParsed" => {
            if let (Some(sid), Some(url)) = (
                params
                    .and_then(|p| p.get("scriptId"))
                    .and_then(Value::as_str),
                params
                    .and_then(|p| p.get("url"))
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty()),
            ) {
                st.script_urls.insert(sid.to_string(), url.to_string());
            }
        }
        "Debugger.paused" => {
            let reason_raw = params
                .and_then(|p| p.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("other");
            let hit_bps = params
                .and_then(|p| p.get("hitBreakpoints"))
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let call_frames = params
                .and_then(|p| p.get("callFrames"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            if !st.initial_brk_skipped && hit_bps == 0 {
                st.initial_brk_skipped = true;
                let top_line = call_frames
                    .first()
                    .and_then(|cf| cf.get("location"))
                    .and_then(|l| l.get("lineNumber"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32
                    + 1;
                let has_bp_at_entry = st
                    .user_bp_lines
                    .values()
                    .any(|lines| lines.contains(&top_line));
                if !has_bp_at_entry {
                    let cid = st.alloc_cdp();
                    ws.send_json(&json!({ "id": cid, "method": "Debugger.resume" }))
                        .await?;
                    return Ok(());
                }
            }
            st.initial_brk_skipped = true;
            st.frames = call_frames;
            let dap_reason = if hit_bps > 0 {
                "breakpoint"
            } else if reason_raw.contains("exception") || reason_raw == "promiseRejection" {
                "exception"
            } else {
                "step"
            };
            dap_w
                .send_event(
                    "stopped",
                    Some(json!({
                        "reason": dap_reason,
                        "threadId": 1,
                        "allThreadsStopped": true
                    })),
                )
                .await?;
        }
        "Inspector.detached" => {
            dap_w.send_event("terminated", Some(json!({}))).await?;
        }
        _ => {}
    }
    Ok(())
}

async fn handle_node_cdp_reply<W: AsyncWrite + Unpin>(
    req_seq: i64,
    kind: &str,
    meta: String,
    result: Option<&Value>,
    st: &mut NodeBridgeState,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    match kind {
        "setBp" => {
            if let Some(bid) = result
                .and_then(|r| r.get("breakpointId"))
                .and_then(Value::as_str)
            {
                st.bp_ids_by_file
                    .entry(meta)
                    .or_default()
                    .push(bid.to_string());
            }
        }
        "variables" => {
            let mut vars = Vec::new();
            if let Some(props) = result
                .and_then(|r| r.get("result"))
                .and_then(Value::as_array)
            {
                for p in props {
                    let Some(name) = p.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    if name == "__proto__" {
                        continue;
                    }
                    if let Some(vobj) = p.get("value") {
                        let (val, vt, cref) = format_remote_object(st, vobj);
                        vars.push(json!({
                            "name": name,
                            "value": val,
                            "type": vt,
                            "variablesReference": cref
                        }));
                    }
                }
            }
            dap_w
                .send_response(req_seq, "variables", Ok(Some(json!({ "variables": vars }))))
                .await?;
        }
        "evaluate" => {
            if let Some(r_obj) = result.and_then(|r| r.get("result")) {
                let (val, vt, cref) = format_remote_object(st, r_obj);
                dap_w
                    .send_response(
                        req_seq,
                        "evaluate",
                        Ok(Some(json!({
                            "result": val,
                            "type": vt,
                            "variablesReference": cref
                        }))),
                    )
                    .await?;
            } else {
                dap_w
                    .send_response(req_seq, "evaluate", Err("evaluation failed".into()))
                    .await?;
            }
        }
        _ => {}
    }
    Ok(())
}
