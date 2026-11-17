//! Direct Binary JDWP-to-DAP Bridge (`java -agentlib:jdwp=transport=dt_socket,server=y,suspend=y`).
//!
//! Connects directly to the JVM's binary JDWP loopback socket without `jdb`, supporting
//! single `.java` files, Maven, Gradle, and multi-repo workspaces (`service-app` + `toolkit`),
//! pre-launch & live mid-run breakpoints, `ClassPrepare` deferred binding, stack frames,
//! local variables + `this` + object/array child expansion, expression evaluation, stepping,
//! stdout/stderr streaming, and clean process teardown.

pub(crate) mod jdwp;

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use otto_ssh::SshTunnel;
use serde_json::{json, Value};
use tokio::io::{AsyncWrite, DuplexStream};
use tokio::sync::mpsc;

use self::jdwp::{
    JdwpBuf, JdwpClient, JdwpCursor, JdwpEvent, JdwpLocation, EVENT_BREAKPOINT,
    EVENT_CLASS_PREPARE, EVENT_SINGLE_STEP, STEP_DEPTH_INTO, STEP_DEPTH_OUT, STEP_DEPTH_OVER,
    SUSPEND_ALL,
};
use super::wire::{DapMessage, DapReader, DapRequest, DapWriter};
use super::{spawn_target_process, ssh_tunnel_config, BridgeGuard};
use crate::types::{IdeSshSpec, IdeTargetKind, IdeTargetSpec};

/// Launch/attach parameters for the built-in binary JDWP-to-DAP bridge.
#[derive(Clone)]
pub struct JavaJdwpBridgeSpec {
    pub java_bin: String,
    pub jvm_flags: Vec<String>,
    pub classpath: String,
    pub main_class: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    pub attach_port: Option<u16>,
    pub source_roots: Vec<String>,
    pub source_map: HashMap<String, String>,
}

/// Infer the fully qualified Java class name for `path`.
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

pub(crate) async fn spawn_java_jdwp_bridge(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: JavaJdwpBridgeSpec,
) -> Result<(DuplexStream, Arc<BridgeGuard>)> {
    let guard = BridgeGuard::new();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<(String, String)>();

    let remote_port = if let Some(port) = spec.attach_port {
        port
    } else {
        spawn_jdwp_jvm(target, ssh, &spec, &guard, &out_tx).await?
    };
    let connect_port = if target.kind == IdeTargetKind::Ssh {
        let ssh_spec = ssh.ok_or_else(|| Error::Invalid("missing SSH spec".into()))?;
        let tunnel =
            SshTunnel::open(&ssh_tunnel_config(ssh_spec)?, "127.0.0.1", remote_port).await?;
        let lp = tunnel.local_port();
        guard.push_tunnel(tunnel);
        lp
    } else {
        remote_port
    };

    let (client, mut jdwp_rx) = JdwpClient::connect(connect_port).await?;
    let (client_io, server_io) = tokio::io::duplex(65536);
    let target_clone = target.clone();
    let guard_task = guard.clone();
    tokio::spawn(async move {
        let _ = run_jdwp_bridge_loop(
            server_io,
            client,
            &mut jdwp_rx,
            &mut out_rx,
            target_clone,
            spec,
            guard_task,
        )
        .await;
    });
    Ok((client_io, guard))
}

async fn spawn_jdwp_jvm(
    target: &IdeTargetSpec,
    ssh: Option<&IdeSshSpec>,
    spec: &JavaJdwpBridgeSpec,
    guard: &BridgeGuard,
    out_tx: &mpsc::UnboundedSender<(String, String)>,
) -> Result<u16> {
    let mut jvm_args = vec![
        "-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:0".to_string(),
    ];
    jvm_args.extend(spec.jvm_flags.clone());
    jvm_args.push("-cp".to_string());
    jvm_args.push(spec.classpath.clone());
    jvm_args.push(spec.main_class.clone());
    jvm_args.extend(spec.args.clone());

    let mut jvm =
        spawn_target_process(target, ssh, &spec.java_bin, &jvm_args, &spec.cwd, &spec.env).await?;
    let jvm_out = jvm
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("missing java stdout".into()))?;
    let mut err_diag = Arc::new(std::sync::Mutex::new(String::new()));
    if let Some(jvm_err) = jvm.stderr.take() {
        let tx = out_tx.clone();
        let diag_ref = Arc::clone(&err_diag);
        crate::line_reader::spawn_line_drain(
            jvm_err,
            crate::line_reader::MAX_OUTPUT_LINE_BYTES,
            move |l| {
                if let Ok(mut d) = diag_ref.lock() {
                    if d.len() < 2048 {
                        d.push_str(&l);
                        d.push('\n');
                    }
                }
                let _ = tx.send(("stderr".into(), format!("{l}\n")));
            },
        );
    }
    let mut out_lines =
        crate::line_reader::LossyLines::new(jvm_out, crate::line_reader::MAX_OUTPUT_LINE_BYTES);
    let mut bound_port: Option<u16> = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(3), out_lines.next_line()).await {
            Ok(Some(line)) => {
                if let Some(rest) = line.split("address:").nth(1) {
                    let p_str = rest.trim().rsplit(':').next().unwrap_or(rest.trim());
                    if let Ok(p) = p_str.parse::<u16>() {
                        bound_port = Some(p);
                        break;
                    }
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let Some(p) = bound_port else {
        let _ = jvm.start_kill();
        let stderr_msg = std::mem::take(&mut err_diag)
            .lock()
            .map(|g| g.trim().to_string())
            .unwrap_or_default();
        return Err(Error::Upstream(format!(
            "JVM JDWP agent failed to report listening port{}",
            if stderr_msg.is_empty() {
                String::new()
            } else {
                format!(": {stderr_msg}")
            }
        )));
    };
    let tx = out_tx.clone();
    tokio::spawn(async move {
        while let Some(l) = out_lines.next_line().await {
            let _ = tx.send(("stdout".into(), format!("{l}\n")));
        }
    });
    guard.push_child(jvm);
    Ok(p)
}

#[derive(Clone)]
struct JavaBpSpec {
    path: String,
    fqcn: String,
    line: u32,
    condition: Option<String>,
    hit_condition: Option<u32>,
    log_message: Option<String>,
    hits: u32,
}

#[derive(Clone)]
struct CachedFrame {
    thread_id: u64,
    jdwp_frame_id: u64,
    loc: JdwpLocation,
    class_sig: String,
}

#[derive(Clone)]
enum VarExpansion {
    FrameLocals(i64),
    ObjectFields(u64),
    ArrayItems(u64),
}

struct JdwpBridgeState {
    target: IdeTargetSpec,
    source_roots: Vec<String>,
    source_map: HashMap<String, String>,
    bps_by_file: HashMap<String, Vec<JavaBpSpec>>,
    file_req_ids: HashMap<String, Vec<(u8, i32)>>,
    req_to_bp: HashMap<i32, (String, u32)>,
    step_req_id: Option<i32>,
    vm_started: bool,
    config_done: bool,
    initial_resumed: bool,
    is_attach: bool,
    frames: HashMap<i64, CachedFrame>,
    ordered_frames: Vec<Value>,
    var_refs: HashMap<i64, VarExpansion>,
    next_ref: i64,
}

impl JdwpBridgeState {
    fn new(target: IdeTargetSpec, spec: &JavaJdwpBridgeSpec) -> Self {
        let is_attach = spec.attach_port.is_some();
        Self {
            target,
            source_roots: spec.source_roots.clone(),
            source_map: spec.source_map.clone(),
            bps_by_file: HashMap::new(),
            file_req_ids: HashMap::new(),
            req_to_bp: HashMap::new(),
            step_req_id: None,
            vm_started: is_attach,
            config_done: false,
            initial_resumed: false,
            is_attach,
            frames: HashMap::new(),
            ordered_frames: Vec::new(),
            var_refs: HashMap::new(),
            next_ref: 2000,
        }
    }

    fn alloc_ref(&mut self, exp: VarExpansion) -> i64 {
        self.next_ref += 1;
        self.var_refs.insert(self.next_ref, exp);
        self.next_ref
    }

    fn resolve_source_path(&self, class_sig: &str, source_file: Option<&str>) -> Option<String> {
        let internal = class_sig
            .strip_prefix('L')
            .and_then(|s| s.strip_suffix(';'))
            .unwrap_or(class_sig);
        let top_internal = internal.split('$').next().unwrap_or(internal);
        let rel_java = if let Some(sf) = source_file.filter(|s| !s.is_empty()) {
            match top_internal.rsplit_once('/') {
                Some((pkg_dir, _)) => format!("{pkg_dir}/{sf}"),
                None => sf.to_string(),
            }
        } else {
            format!("{top_internal}.java")
        };
        for root in &self.source_roots {
            let cand = Path::new(root).join(&rel_java);
            if cand.is_file() {
                return Some(
                    std::fs::canonicalize(&cand)
                        .unwrap_or(cand)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        if let Some(sf) = source_file {
            if let Some(mapped) = self.source_map.get(sf) {
                return Some(mapped.clone());
            }
        }
        let base = Path::new(&rel_java).file_name()?.to_str()?;
        self.source_map.get(base).cloned()
    }
}

async fn run_jdwp_bridge_loop(
    server_io: DuplexStream,
    client: Arc<JdwpClient>,
    jdwp_rx: &mut mpsc::UnboundedReceiver<Option<Vec<u8>>>,
    out_rx: &mut mpsc::UnboundedReceiver<(String, String)>,
    target: IdeTargetSpec,
    spec: JavaJdwpBridgeSpec,
    guard: Arc<BridgeGuard>,
) -> io::Result<()> {
    let (r, w) = tokio::io::split(server_io);
    let mut dap_rx = DapReader::new(r).into_channel();
    let mut dap_w = DapWriter::new(w);
    let mut st = JdwpBridgeState::new(target, &spec);

    loop {
        tokio::select! {
            Some((cat, text)) = out_rx.recv() => {
                dap_w.send_event("output", Some(json!({ "category": cat, "output": text }))).await?;
            }
            maybe_pkt = jdwp_rx.recv() => {
                while let Ok((cat, text)) = out_rx.try_recv() {
                    dap_w.send_event("output", Some(json!({ "category": cat, "output": text }))).await?;
                }
                let Some(Some(pkt)) = maybe_pkt else {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                    while let Ok((cat, text)) = out_rx.try_recv() {
                        let _ = dap_w.send_event("output", Some(json!({ "category": cat, "output": text }))).await;
                    }
                    let _ = dap_w.send_event("terminated", Some(json!({}))).await;
                    break;
                };
                if handle_jdwp_composite(&pkt, &client, &mut st, &mut dap_w).await? {
                    break;
                }
            }
            dap_msg = dap_rx.recv() => {
                let Some(DapMessage::Request(req)) = dap_msg.unwrap_or(Ok(None))? else { break; };
                if handle_dap_request(&req, &client, &mut st, &mut dap_w, &guard).await? {
                    break;
                }
            }
        }
    }
    guard.kill_all();
    Ok(())
}

async fn handle_jdwp_composite<W: AsyncWrite + Unpin>(
    pkt: &[u8],
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    dap_w: &mut DapWriter<W>,
) -> io::Result<bool> {
    let Ok((_policy, events)) = client.parse_composite(pkt) else {
        return Ok(false);
    };
    let mut stop_info: Option<(&'static str, u64, Option<i32>)> = None;
    let mut saw_class_prepare = false;

    for ev in events {
        match ev {
            JdwpEvent::VmStart => {
                st.vm_started = true;
                if st.config_done && !st.initial_resumed {
                    st.initial_resumed = true;
                    let _ = client.vm_resume().await;
                }
            }
            JdwpEvent::VmDeath => {
                dap_w.send_event("terminated", Some(json!({}))).await?;
                return Ok(true);
            }
            JdwpEvent::ClassPrepare {
                ref_type_tag,
                type_id,
                signature,
            } => {
                saw_class_prepare = true;
                bind_prepared_class_bps(client, st, ref_type_tag, type_id, &signature, dap_w)
                    .await?;
            }
            JdwpEvent::Breakpoint {
                request_id, thread, ..
            } => {
                stop_info = Some(("breakpoint", thread, Some(request_id)));
            }
            JdwpEvent::SingleStep {
                request_id, thread, ..
            } => {
                client.clear_event(EVENT_SINGLE_STEP, request_id).await;
                if st.step_req_id == Some(request_id) {
                    st.step_req_id = None;
                }
                if stop_info.is_none() {
                    stop_info = Some(("step", thread, None));
                }
            }
            JdwpEvent::Exception { thread, .. } => {
                if stop_info.is_none() {
                    stop_info = Some(("exception", thread, None));
                }
            }
        }
    }

    if let Some((reason, tid, bp_req_id)) = stop_info {
        if let Some(prev_step) = st.step_req_id.take() {
            client.clear_event(EVENT_SINGLE_STEP, prev_step).await;
        }
        populate_stopped_frames(client, st, tid).await;
        if let Some(rid) = bp_req_id {
            if should_auto_resume_bp(client, st, rid, dap_w).await? {
                let _ = client.vm_resume().await;
                return Ok(false);
            }
        }
        dap_w
            .send_event(
                "stopped",
                Some(json!({
                    "reason": reason,
                    "threadId": tid as i64,
                    "allThreadsStopped": true
                })),
            )
            .await?;
    } else if saw_class_prepare {
        let _ = client.vm_resume().await;
    }
    Ok(false)
}

async fn bind_prepared_class_bps<W: AsyncWrite + Unpin>(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    ref_type_tag: u8,
    class_id: u64,
    signature: &str,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    let fqcn = signature
        .strip_prefix('L')
        .and_then(|s| s.strip_suffix(';'))
        .unwrap_or(signature)
        .replace('/', ".");
    let outer_fqcn = fqcn.split('$').next().unwrap_or(&fqcn);

    let matching_files: Vec<(String, Vec<u32>)> = st
        .bps_by_file
        .iter()
        .filter_map(|(file, bps)| {
            let lines: Vec<u32> = bps
                .iter()
                .filter(|b| b.fqcn == fqcn || b.fqcn == outer_fqcn)
                .map(|b| b.line)
                .collect();
            (!lines.is_empty()).then(|| (file.clone(), lines))
        })
        .collect();

    if matching_files.is_empty() {
        return Ok(());
    }
    let methods = client.methods(class_id).await.unwrap_or_default();
    for (file, lines) in matching_files {
        for want_ln in lines {
            if let Some((mid, code_idx, resolved_ln)) =
                find_line_in_methods(client, class_id, &methods, want_ln).await
            {
                let loc = JdwpLocation {
                    type_tag: ref_type_tag,
                    class_id,
                    method_id: mid,
                    index: code_idx,
                };
                if let Some(req_id) = set_location_breakpoint(client, &loc).await {
                    st.file_req_ids
                        .entry(file.clone())
                        .or_default()
                        .push((EVENT_BREAKPOINT, req_id));
                    st.req_to_bp.insert(req_id, (file.clone(), want_ln));
                    dap_w
                        .send_event(
                            "breakpoint",
                            Some(json!({
                                "reason": "changed",
                                "breakpoint": {
                                    "verified": true,
                                    "line": resolved_ln,
                                    "source": { "path": file }
                                }
                            })),
                        )
                        .await?;
                }
            }
        }
    }
    Ok(())
}

async fn find_line_in_methods(
    client: &JdwpClient,
    class_id: u64,
    methods: &[jdwp::JdwpMethodInfo],
    want_line: u32,
) -> Option<(u64, u64, u32)> {
    for m in methods {
        let table = client.line_table(class_id, m.method_id).await.ok()?;
        if let Some((idx, ln)) = table
            .iter()
            .filter(|(_, ln)| *ln == want_line)
            .min_by_key(|(idx, _)| *idx)
            .copied()
        {
            return Some((m.method_id, idx, ln));
        }
    }
    None
}

async fn set_location_breakpoint(client: &JdwpClient, loc: &JdwpLocation) -> Option<i32> {
    let mut b = JdwpBuf::new();
    b.put_u8(EVENT_BREAKPOINT);
    b.put_u8(SUSPEND_ALL);
    b.put_i32(1);
    b.put_u8(7); // ModKind::LocationOnly
    b.put_location(loc, &client.sizes);
    let (ec, data) = client.cmd(15, 1, &b.into_vec()).await.ok()?;
    (ec == 0)
        .then(|| JdwpCursor::new(&data).i32().ok())
        .flatten()
}

async fn set_class_prepare_filter(client: &JdwpClient, pattern: &str) -> Option<i32> {
    let mut b = JdwpBuf::new();
    b.put_u8(EVENT_CLASS_PREPARE);
    b.put_u8(SUSPEND_ALL);
    b.put_i32(1);
    b.put_u8(5); // ModKind::ClassMatch
    b.put_str(pattern);
    let (ec, data) = client.cmd(15, 1, &b.into_vec()).await.ok()?;
    (ec == 0)
        .then(|| JdwpCursor::new(&data).i32().ok())
        .flatten()
}

async fn handle_dap_request<W: AsyncWrite + Unpin>(
    req: &DapRequest,
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
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
                        "supportsHitConditionalBreakpoints": true,
                        "supportsLogPoints": true,
                        "supportsTerminateRequest": true,
                        "supportsEvaluateForHovers": true,
                    }))),
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
            apply_java_breakpoints(req.seq, args, client, st, dap_w).await?;
        }
        "setExceptionBreakpoints" => {
            dap_w
                .send_response(req.seq, "setExceptionBreakpoints", Ok(Some(json!({}))))
                .await?;
        }
        "configurationDone" => {
            st.config_done = true;
            if st.vm_started && !st.initial_resumed && !st.is_attach {
                st.initial_resumed = true;
                let _ = client.vm_resume().await;
            }
            dap_w
                .send_response(req.seq, "configurationDone", Ok(Some(json!({}))))
                .await?;
        }
        "threads" => {
            let thrs = list_jdwp_threads(client).await;
            dap_w
                .send_response(req.seq, "threads", Ok(Some(json!({ "threads": thrs }))))
                .await?;
        }
        "stackTrace" => {
            let tid = args
                .and_then(|a| a.get("threadId"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as u64;
            if st.ordered_frames.is_empty() {
                populate_stopped_frames(client, st, tid).await;
            }
            dap_w
                .send_response(
                    req.seq,
                    "stackTrace",
                    Ok(Some(json!({
                        "stackFrames": st.ordered_frames,
                        "totalFrames": st.ordered_frames.len()
                    }))),
                )
                .await?;
        }
        "scopes" => {
            let fid = args
                .and_then(|a| a.get("frameId"))
                .and_then(Value::as_i64)
                .unwrap_or(1);
            let vref = st.alloc_ref(VarExpansion::FrameLocals(fid));
            dap_w
                .send_response(
                    req.seq,
                    "scopes",
                    Ok(Some(json!({
                        "scopes": [{ "name": "Locals", "variablesReference": vref, "expensive": false }]
                    }))),
                )
                .await?;
        }
        "variables" => {
            let vref = args
                .and_then(|a| a.get("variablesReference"))
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let vars = resolve_variables(client, st, vref).await;
            dap_w
                .send_response(req.seq, "variables", Ok(Some(json!({ "variables": vars }))))
                .await?;
        }
        "evaluate" => {
            let expr = args
                .and_then(|a| a.get("expression"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let fid = args
                .and_then(|a| a.get("frameId"))
                .and_then(Value::as_i64)
                .unwrap_or(1);
            match evaluate_java_expr(client, st, fid, expr).await {
                Some((res, vt, cref)) => {
                    dap_w
                        .send_response(
                            req.seq,
                            "evaluate",
                            Ok(Some(json!({
                                "result": res,
                                "type": vt,
                                "variablesReference": cref
                            }))),
                        )
                        .await?;
                }
                None => {
                    dap_w
                        .send_response(
                            req.seq,
                            "evaluate",
                            Err(format!("cannot evaluate '{expr}' in current Java frame")),
                        )
                        .await?;
                }
            }
        }
        "continue" => {
            if let Some(prev) = st.step_req_id.take() {
                client.clear_event(EVENT_SINGLE_STEP, prev).await;
            }
            let _ = client.vm_resume().await;
            dap_w
                .send_response(
                    req.seq,
                    "continue",
                    Ok(Some(json!({ "allThreadsContinued": true }))),
                )
                .await?;
        }
        "next" | "stepIn" | "stepOut" => {
            let tid = args
                .and_then(|a| a.get("threadId"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as u64;
            let depth = match req.command.as_str() {
                "next" => STEP_DEPTH_OVER,
                "stepIn" => STEP_DEPTH_INTO,
                _ => STEP_DEPTH_OUT,
            };
            if let Some(prev) = st.step_req_id.take() {
                client.clear_event(EVENT_SINGLE_STEP, prev).await;
            }
            st.step_req_id = register_step_request(client, tid, depth).await;
            let _ = client.vm_resume().await;
            dap_w
                .send_response(req.seq, &req.command, Ok(Some(json!({}))))
                .await?;
        }
        "pause" => {
            let _ = client.vm_suspend().await;
            let thrs = list_jdwp_threads(client).await;
            let tid = thrs
                .first()
                .and_then(|t| t.get("id"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as u64;
            populate_stopped_frames(client, st, tid).await;
            dap_w
                .send_response(req.seq, "pause", Ok(Some(json!({}))))
                .await?;
            dap_w
                .send_event(
                    "stopped",
                    Some(json!({
                        "reason": "pause",
                        "threadId": tid as i64,
                        "allThreadsStopped": true
                    })),
                )
                .await?;
        }
        "disconnect" | "terminate" => {
            client.vm_exit(0).await;
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

#[path = "eval.rs"]
mod eval;
#[path = "inspect.rs"]
mod inspect;
use eval::{evaluate_java_expr, register_step_request, should_auto_resume_bp};
use inspect::{
    apply_java_breakpoints, list_jdwp_threads, populate_stopped_frames, resolve_variables,
};
