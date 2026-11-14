//! Built-in Node.js / V8 Inspector-to-DAP Bridge (`node --inspect-brk=127.0.0.1:0`).
//!
//! Speaks RFC 6455 loopback WebSocket + Chrome DevTools Protocol (`Debugger.*` and
//! `Runtime.*`) to Node.js on one side and `Content-Length` DAP over an in-memory
//! [`tokio::io::DuplexStream`] on the other, supporting `.js`, `.mjs`, `.cjs`, and `.ts`
//! (`--experimental-strip-types` / `tsx`), symlinked macOS paths (`/var` <-> `/private/var`),
//! conditional/hit-count/logpoint breakpoints, `callFrames`, `Runtime.getProperties`,
//! `Debugger.evaluateOnCallFrame`, stepping, stdout/stderr streaming, and clean teardown.

mod cdp;
mod events;

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use otto_core::Result;
use serde_json::{json, Value};
use tokio::io::{AsyncWrite, DuplexStream};
use tokio::sync::mpsc;

use self::cdp::{discover_attach_ws_path, maybe_tunnel_port, spawn_inspected_node, InspectorWs};
use self::events::handle_node_cdp_msg;
use super::wire::{DapMessage, DapReader, DapRequest, DapWriter};
use super::BridgeGuard;
use crate::types::{IdeSshSpec, IdeTargetSpec};

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

pub(super) struct NodeBridgeState {
    pub(super) cdp_id: i64,
    pub(super) pending_cdp: HashMap<i64, (i64, String, String)>,
    pub(super) script_urls: HashMap<String, String>,
    pub(super) bp_ids_by_file: HashMap<String, Vec<String>>,
    pub(super) user_bp_lines: HashMap<String, Vec<u32>>,
    pub(super) initial_brk_skipped: bool,
    pub(super) frames: Vec<Value>,
    pub(super) obj_refs: HashMap<i64, String>,
    pub(super) next_var_ref: i64,
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

    pub(super) fn alloc_cdp(&mut self) -> i64 {
        let id = self.cdp_id;
        self.cdp_id += 1;
        id
    }

    pub(super) fn alloc_obj_ref(&mut self, object_id: String) -> i64 {
        let r = self.next_var_ref;
        self.next_var_ref += 1;
        self.obj_refs.insert(r, object_id);
        r
    }
}

pub(super) fn file_url_to_path(url: &str) -> Option<String> {
    let raw = url.strip_prefix("file://")?;
    let decoded = percent_decode(raw);
    if let Ok(canon) = std::fs::canonicalize(&decoded) {
        return Some(canon.to_string_lossy().into_owned());
    }
    Some(decoded)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn path_to_url_regex(path: &str) -> String {
    let canon = std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let mut alts = Vec::new();
    if let Some(tail) = canon
        .strip_prefix("/private/var/")
        .or_else(|| canon.strip_prefix("/var/"))
    {
        alts.push(format!("file://(?:/private)?/var/{}", regex::escape(tail)));
    } else if let Some(tail) = canon
        .strip_prefix("/private/tmp/")
        .or_else(|| canon.strip_prefix("/tmp/"))
    {
        alts.push(format!("file://(?:/private)?/tmp/{}", regex::escape(tail)));
    } else {
        alts.push(format!("file://{}", regex::escape(&canon)));
    }
    if canon != path {
        alts.push(format!("file://{}", regex::escape(path)));
    }
    if let Some(name) = Path::new(&canon).file_name().and_then(|n| n.to_str()) {
        alts.push(format!("/{}$", regex::escape(name)));
    }
    format!("(?:{})", alts.join("|"))
}

fn build_cdp_bp_condition(bp: &Value) -> String {
    let cond = bp
        .get("condition")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let log_msg = bp.get("logMessage").and_then(Value::as_str).unwrap_or("");
    if !log_msg.is_empty() {
        let mut js_tpl = String::new();
        let mut rest = log_msg;
        while let Some(open) = rest.find('{') {
            js_tpl.push_str(&escape_js_template(&rest[..open]));
            if let Some(close) = rest[open + 1..].find('}') {
                let expr = rest[open + 1..open + 1 + close].trim();
                js_tpl.push_str("${");
                js_tpl.push_str(expr);
                js_tpl.push('}');
                rest = &rest[open + 1 + close + 1..];
            } else {
                js_tpl.push_str(&escape_js_template(&rest[open..]));
                rest = "";
                break;
            }
        }
        js_tpl.push_str(&escape_js_template(rest));
        if cond.is_empty() {
            return format!("(console.log(`{js_tpl}`), false)");
        }
        return format!("(({cond}) ? (console.log(`{js_tpl}`), false) : false)");
    }
    cond.to_string()
}

fn escape_js_template(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace("${", "\\${")
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
                while let Ok((cat, text)) = out_rx.try_recv() {
                    dap_w.send_event("output", Some(json!({ "category": cat, "output": text }))).await?;
                }
                let should_terminate = match cdp_msg? {
                    Some(msg) => handle_node_cdp_msg(msg, &mut st, &mut ws, &mut dap_w).await?,
                    None => true,
                };
                if should_terminate {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    while let Ok((cat, text)) = out_rx.try_recv() {
                        let _ = dap_w
                            .send_event("output", Some(json!({ "category": cat, "output": text })))
                            .await;
                    }
                    let _ = dap_w.send_event("terminated", Some(json!({}))).await;
                    break;
                }
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
                        "supportsLogPoints": true,
                        "supportsTerminateRequest": true,
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
    let raw_path = args
        .and_then(|a| a.get("source"))
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = std::fs::canonicalize(raw_path)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| raw_path.to_string());
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
        let cond = build_cdp_bp_condition(&bp);
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
        verified.push(json!({ "verified": true, "line": line, "source": { "path": path } }));
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
