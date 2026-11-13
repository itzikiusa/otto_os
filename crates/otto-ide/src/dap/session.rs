//! Stateful DAP session lifecycle manager (`DapManager`).
//!
//! Drives Debug Adapter Protocol sessions over stdio, loopback TCP, SSH tunnels,
//! and in-memory bridges (`tokio::io::duplex`), handling:
//! - `initialize` -> `launch`/`attach` -> `initialized` -> `setBreakpoints` -> `configurationDone`
//! - Live breakpoint & exception-breakpoint updates
//! - Execution controls (`continue`, `pause`, `step_over`, `step_into`, `step_out`,
//!   `run_to_cursor`, `select_frame`, `restart`, `stop`)
//! - Automatic `stopped` inspection (`threads` -> `stackTrace` -> `scopes` -> `variables`)
//!   + IntelliJ-style `inline_values` extraction from paused source files
//! - Lazy child variable expansion (`variables`) and REPL/watch `evaluate`
//! - Real-time `tokio::sync::broadcast` snapshot streaming for SSE endpoints.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use otto_core::{new_id, Error, Result};
use otto_ssh::SshTunnel;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, mpsc, oneshot, Notify};

use super::adapters::{
    normalize_target_path, resolve_dap_adapter, DapAdapterTransport, ResolvedDapAdapter,
};
use super::java_bridge::spawn_java_jdb_bridge;
use super::node_bridge::spawn_node_inspector_bridge;
use super::session_inspect::{
    fetch_variables, handle_stopped_event, select_frame_and_refresh, sync_all_breakpoints,
};
use super::wire::{DapEvent, DapMessage, DapReader, DapResponse, DapWriter};
use super::{spawn_target_process, ssh_tunnel_config, BridgeGuard};
use crate::types::{
    IdeDebugAction, IdeDebugCommandReq, IdeDebugEvaluateReq, IdeDebugEvaluateResp,
    IdeDebugOutputEntry, IdeDebugSessionSnapshot, IdeDebugState, IdeProjectEnvironment,
    IdeRunConfig, IdeSetBreakpointsReq, IdeSshSpec, IdeStartDebugReq, IdeTargetKind, IdeVariable,
    PluginDapAdapterSpec,
};

pub use super::session_inspect::compute_inline_values;

const MAX_OUTPUT_ENTRIES: usize = 500;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// Optional host/environment context passed when starting a debug session.
#[derive(Debug, Clone, Default)]
pub struct DapStartContext {
    pub ssh: Option<IdeSshSpec>,
    pub effective_env: BTreeMap<String, String>,
    pub project_env: Option<IdeProjectEnvironment>,
    pub run_config: Option<IdeRunConfig>,
    pub plugin_adapters: Vec<PluginDapAdapterSpec>,
    pub caller_is_root: bool,
}

impl DapStartContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_ssh(mut self, ssh: Option<IdeSshSpec>) -> Self {
        self.ssh = ssh;
        self
    }

    pub fn with_effective_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.effective_env = env;
        self
    }

    pub fn with_project_env(mut self, env: Option<IdeProjectEnvironment>) -> Self {
        self.project_env = env;
        self
    }

    pub fn with_run_config(mut self, cfg: Option<IdeRunConfig>) -> Self {
        self.run_config = cfg;
        self
    }

    pub fn with_plugin_adapters(mut self, adapters: Vec<PluginDapAdapterSpec>) -> Self {
        self.plugin_adapters = adapters;
        self
    }

    pub fn with_caller_is_root(mut self, is_root: bool) -> Self {
        self.caller_is_root = is_root;
        self
    }
}

/// Multiplexed async DAP request/response client decoupled from event streaming.
pub(super) struct DapWireClient {
    writer: tokio::sync::Mutex<DapWriter<BoxWrite>>,
    pending: Mutex<HashMap<i64, oneshot::Sender<DapResponse>>>,
}

impl DapWireClient {
    fn spawn(
        reader_io: BoxRead,
        writer_io: BoxWrite,
        event_tx: mpsc::UnboundedSender<DapEvent>,
    ) -> Arc<Self> {
        let client = Arc::new(Self {
            writer: tokio::sync::Mutex::new(DapWriter::new(writer_io)),
            pending: Mutex::new(HashMap::new()),
        });
        let client_ref = client.clone();
        tokio::spawn(async move {
            let mut reader = DapReader::new(reader_io);
            while let Ok(Some(msg)) = reader.next_message().await {
                match msg {
                    DapMessage::Response(resp) => {
                        let tx = client_ref
                            .pending
                            .lock()
                            .ok()
                            .and_then(|mut map| map.remove(&resp.request_seq));
                        if let Some(tx) = tx {
                            let _ = tx.send(resp);
                        }
                    }
                    DapMessage::Event(ev) => {
                        let _ = event_tx.send(ev);
                    }
                    DapMessage::Request(rev_req) => {
                        let mut w = client_ref.writer.lock().await;
                        let _ = w
                            .send_response(rev_req.seq, &rev_req.command, Ok(Some(json!({}))))
                            .await;
                    }
                }
            }
            if let Ok(mut map) = client_ref.pending.lock() {
                map.clear();
            }
            let _ = event_tx.send(DapEvent::new(0, "__eof__", None));
        });
        client
    }

    async fn send_request_nowait(
        &self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<oneshot::Receiver<DapResponse>> {
        let (tx, rx) = oneshot::channel();
        let mut w = self.writer.lock().await;
        let seq = w.alloc_seq();
        if let Ok(mut map) = self.pending.lock() {
            map.insert(seq, tx);
        }
        let msg = DapMessage::Request(super::wire::DapRequest::new(seq, command, arguments));
        if let Err(e) = w.write_message(&msg).await {
            if let Ok(mut map) = self.pending.lock() {
                map.remove(&seq);
            }
            return Err(Error::Upstream(format!(
                "DAP write '{command}' failed: {e}"
            )));
        }
        Ok(rx)
    }

    pub(super) async fn request(
        &self,
        command: &str,
        arguments: Option<Value>,
    ) -> Result<DapResponse> {
        let rx = self.send_request_nowait(command, arguments).await?;
        let resp = tokio::time::timeout(REQUEST_TIMEOUT, rx)
            .await
            .map_err(|_| Error::Upstream(format!("DAP '{command}' timed out")))?
            .map_err(|_| Error::Upstream(format!("DAP adapter closed during '{command}'")))?;
        if !resp.success {
            let reason = resp
                .message
                .unwrap_or_else(|| format!("DAP '{command}' failed"));
            return Err(Error::Invalid(reason));
        }
        Ok(resp)
    }
}

pub(super) struct ActiveConnection {
    pub(super) wire: Arc<DapWireClient>,
    pub(super) guard: Arc<BridgeGuard>,
    pub(super) supports_terminate: bool,
    pub(super) exception_filters: Vec<String>,
}

pub(super) struct SessionEntry {
    pub(super) orig_req: RwLock<IdeStartDebugReq>,
    pub(super) orig_ctx: RwLock<DapStartContext>,
    pub(super) snapshot: RwLock<IdeDebugSessionSnapshot>,
    pub(super) configured_bp_files: Mutex<BTreeSet<String>>,
    pub(super) temp_run_to_cursor: Mutex<Option<(String, u32)>>,
    pub(super) break_on_exceptions: Mutex<bool>,
    pub(super) conn: RwLock<Option<Arc<ActiveConnection>>>,
    pub(super) events_tx: broadcast::Sender<IdeDebugSessionSnapshot>,
    pub(super) initialized_notify: Arc<Notify>,
}

impl SessionEntry {
    pub(super) fn read_snapshot(&self) -> IdeDebugSessionSnapshot {
        self.snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(super) fn mutate_and_broadcast(
        &self,
        f: impl FnOnce(&mut IdeDebugSessionSnapshot),
    ) -> IdeDebugSessionSnapshot {
        let updated = {
            let mut snap = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
            f(&mut snap);
            snap.clone()
        };
        let _ = self.events_tx.send(updated.clone());
        updated
    }

    fn append_output(&self, category: &str, text: String) {
        if text.is_empty() {
            return;
        }
        self.mutate_and_broadcast(|snap| {
            if snap.output.len() >= MAX_OUTPUT_ENTRIES {
                snap.output.remove(0);
            }
            snap.output.push(IdeDebugOutputEntry {
                category: category.to_string(),
                text,
            });
        });
    }

    fn active_conn(&self) -> Result<Arc<ActiveConnection>> {
        self.conn
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| Error::Conflict("debug session is no longer connected".into()))
    }
}

/// Multi-session Debug Adapter Protocol engine mounted on `IdeEngine`.
pub struct DapManager {
    data_dir: PathBuf,
    sessions: Arc<DashMap<String, Arc<SessionEntry>>>,
}

impl Default for DapManager {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("otto-ide-dap"))
    }
}

impl DapManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            sessions: Arc::new(DashMap::new()),
        }
    }

    /// List snapshots of all debug sessions (newest first by ID).
    pub fn list_sessions(&self) -> Vec<IdeDebugSessionSnapshot> {
        let mut out: Vec<IdeDebugSessionSnapshot> = self
            .sessions
            .iter()
            .map(|item| item.value().read_snapshot())
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out
    }

    /// Return the current snapshot for debug session `id`.
    pub fn get_session(&self, id: &str) -> Result<IdeDebugSessionSnapshot> {
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))?;
        Ok(entry.value().read_snapshot())
    }

    /// Subscribe to real-time [`IdeDebugSessionSnapshot`] updates for SSE streaming.
    pub fn subscribe_events(
        &self,
        id: &str,
    ) -> Result<broadcast::Receiver<IdeDebugSessionSnapshot>> {
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))?;
        Ok(entry.value().events_tx.subscribe())
    }

    /// Resolve the language adapter, launch/attach the target, configure breakpoints,
    /// and return the live session snapshot.
    pub async fn start_session(
        &self,
        mut req: IdeStartDebugReq,
        ctx: DapStartContext,
    ) -> Result<IdeDebugSessionSnapshot> {
        for bp in &mut req.breakpoints {
            bp.path = normalize_target_path(&req.target, &bp.path);
        }
        let resolved = resolve_dap_adapter(&self.data_dir, &req, &ctx).await?;
        let id = format!("dbg_{}", new_id());
        let title = build_session_title(&req, &resolved.adapter_name);
        let (events_tx, _) = broadcast::channel(128);

        let initial_snap = IdeDebugSessionSnapshot {
            id: id.clone(),
            title,
            language: resolved.language.clone(),
            adapter: resolved.adapter_name.clone(),
            state: IdeDebugState::Starting,
            stopped_reason: None,
            active_thread_id: None,
            active_frame_id: None,
            paused_file: None,
            paused_line: None,
            threads: Vec::new(),
            stack_frames: Vec::new(),
            scopes: Vec::new(),
            inline_values: Vec::new(),
            breakpoints: req.breakpoints.clone(),
            output: resolved.initial_output.clone(),
            error: None,
        };

        let entry = Arc::new(SessionEntry {
            orig_req: RwLock::new(req.clone()),
            orig_ctx: RwLock::new(ctx.clone()),
            snapshot: RwLock::new(initial_snap),
            configured_bp_files: Mutex::new(BTreeSet::new()),
            temp_run_to_cursor: Mutex::new(None),
            break_on_exceptions: Mutex::new(false),
            conn: RwLock::new(None),
            events_tx,
            initialized_notify: Arc::new(Notify::new()),
        });
        self.sessions.insert(id, entry.clone());

        if let Err(err) = boot_session_connection(&entry, resolved, &req, &ctx).await {
            entry.mutate_and_broadcast(|snap| {
                snap.state = IdeDebugState::Error;
                snap.error = Some(err.to_string());
            });
            return Err(err);
        }
        Ok(entry.read_snapshot())
    }

    /// Start a debug session over a pre-connected async stream (e.g. `tokio::io::duplex`
    /// in unit tests or custom sidecars).
    pub async fn start_session_with_stream<S>(
        &self,
        mut req: IdeStartDebugReq,
        adapter_name: impl Into<String>,
        stream: S,
        caller_is_root: bool,
    ) -> Result<IdeDebugSessionSnapshot>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        for bp in &mut req.breakpoints {
            bp.path = normalize_target_path(&req.target, &bp.path);
        }
        let adapter_str = adapter_name.into();
        let id = format!("dbg_{}", new_id());
        let title = build_session_title(&req, &adapter_str);
        let (events_tx, _) = broadcast::channel(128);
        let ctx = DapStartContext::new().with_caller_is_root(caller_is_root);

        let initial_snap = IdeDebugSessionSnapshot {
            id: id.clone(),
            title,
            language: req.language.clone(),
            adapter: adapter_str.clone(),
            state: IdeDebugState::Starting,
            stopped_reason: None,
            active_thread_id: None,
            active_frame_id: None,
            paused_file: None,
            paused_line: None,
            threads: Vec::new(),
            stack_frames: Vec::new(),
            scopes: Vec::new(),
            inline_values: Vec::new(),
            breakpoints: req.breakpoints.clone(),
            output: Vec::new(),
            error: None,
        };

        let entry = Arc::new(SessionEntry {
            orig_req: RwLock::new(req.clone()),
            orig_ctx: RwLock::new(ctx),
            snapshot: RwLock::new(initial_snap),
            configured_bp_files: Mutex::new(BTreeSet::new()),
            temp_run_to_cursor: Mutex::new(None),
            break_on_exceptions: Mutex::new(false),
            conn: RwLock::new(None),
            events_tx,
            initialized_notify: Arc::new(Notify::new()),
        });
        self.sessions.insert(id, entry.clone());

        let (r, w) = tokio::io::split(stream);
        let launch_cmd = if req.launch.attach_port.is_some() {
            "attach"
        } else {
            "launch"
        };
        let init_args = json!({
            "adapterID": adapter_str,
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        });
        let launch_args = json!({
            "program": req.launch.program,
            "args": req.launch.args,
            "cwd": req.launch.cwd.clone().unwrap_or_else(|| req.target.root.clone()),
        });
        handshake_and_run(
            &entry,
            Box::new(r),
            Box::new(w),
            BridgeGuard::new(),
            init_args,
            launch_cmd,
            launch_args,
        )
        .await?;
        Ok(entry.read_snapshot())
    }

    /// Update source and exception breakpoints on an active session.
    pub async fn set_breakpoints(
        &self,
        id: &str,
        mut req: IdeSetBreakpointsReq,
    ) -> Result<IdeDebugSessionSnapshot> {
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))?
            .value()
            .clone();

        let target = entry
            .orig_req
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .target
            .clone();
        for bp in &mut req.breakpoints {
            bp.path = normalize_target_path(&target, &bp.path);
        }
        if let Ok(mut b) = entry.break_on_exceptions.lock() {
            *b = req.break_on_exceptions;
        }
        entry.mutate_and_broadcast(|snap| {
            snap.breakpoints = req.breakpoints.clone();
        });

        if let Ok(conn) = entry.active_conn() {
            sync_all_breakpoints(&entry, &conn).await?;
        }
        Ok(entry.read_snapshot())
    }

    /// Execute a stepping, frame selection, run-to-cursor, restart, or stop command.
    pub async fn command(
        &self,
        id: &str,
        req: IdeDebugCommandReq,
    ) -> Result<IdeDebugSessionSnapshot> {
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))?
            .value()
            .clone();

        match req.action {
            IdeDebugAction::Stop => {
                stop_entry(&entry).await;
                return Ok(entry.read_snapshot());
            }
            IdeDebugAction::Restart => {
                stop_entry(&entry).await;
                let (mut orig_req, orig_ctx) = {
                    let mut r = entry
                        .orig_req
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    r.breakpoints = entry.read_snapshot().breakpoints;
                    let c = entry
                        .orig_ctx
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .clone();
                    (r, c)
                };
                for bp in &mut orig_req.breakpoints {
                    bp.path = normalize_target_path(&orig_req.target, &bp.path);
                }
                entry.mutate_and_broadcast(|snap| {
                    snap.state = IdeDebugState::Starting;
                    snap.stopped_reason = None;
                    snap.paused_file = None;
                    snap.paused_line = None;
                    snap.stack_frames.clear();
                    snap.scopes.clear();
                    snap.inline_values.clear();
                    snap.error = None;
                });
                let resolved = resolve_dap_adapter(&self.data_dir, &orig_req, &orig_ctx).await?;
                boot_session_connection(&entry, resolved, &orig_req, &orig_ctx).await?;
                return Ok(entry.read_snapshot());
            }
            IdeDebugAction::SelectFrame => {
                let frame_id = req.frame_id.ok_or_else(|| {
                    Error::Invalid("frame_id is required for 'select_frame'".into())
                })?;
                let conn = entry.active_conn()?;
                select_frame_and_refresh(&entry, &conn, frame_id).await?;
                return Ok(entry.read_snapshot());
            }
            _ => {}
        }

        let conn = entry.active_conn()?;
        let snap = entry.read_snapshot();
        let thread_id = req.thread_id.or(snap.active_thread_id).unwrap_or(1);

        match req.action {
            IdeDebugAction::Continue => {
                mark_resumed(&entry);
                conn.wire
                    .request("continue", Some(json!({ "threadId": thread_id })))
                    .await?;
            }
            IdeDebugAction::Pause => {
                conn.wire
                    .request("pause", Some(json!({ "threadId": thread_id })))
                    .await?;
            }
            IdeDebugAction::StepOver => {
                mark_resumed(&entry);
                conn.wire
                    .request("next", Some(json!({ "threadId": thread_id })))
                    .await?;
            }
            IdeDebugAction::StepInto => {
                mark_resumed(&entry);
                conn.wire
                    .request("stepIn", Some(json!({ "threadId": thread_id })))
                    .await?;
            }
            IdeDebugAction::StepOut => {
                mark_resumed(&entry);
                conn.wire
                    .request("stepOut", Some(json!({ "threadId": thread_id })))
                    .await?;
            }
            IdeDebugAction::RunToCursor => {
                let raw_path = req
                    .path
                    .as_deref()
                    .or(snap.paused_file.as_deref())
                    .ok_or_else(|| Error::Invalid("path is required for 'run_to_cursor'".into()))?;
                let line = req
                    .line
                    .ok_or_else(|| Error::Invalid("line is required for 'run_to_cursor'".into()))?;
                let target = entry
                    .orig_req
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    .clone();
                let norm_path = normalize_target_path(&target, raw_path);
                if let Ok(mut slot) = entry.temp_run_to_cursor.lock() {
                    *slot = Some((norm_path, line));
                }
                sync_all_breakpoints(&entry, &conn).await?;
                mark_resumed(&entry);
                conn.wire
                    .request("continue", Some(json!({ "threadId": thread_id })))
                    .await?;
            }
            IdeDebugAction::SelectFrame | IdeDebugAction::Restart | IdeDebugAction::Stop => {}
        }
        Ok(entry.read_snapshot())
    }

    /// Expand child properties/elements of a compound variable (`variablesReference > 0`).
    pub async fn expand_variables(
        &self,
        id: &str,
        variables_reference: i64,
    ) -> Result<Vec<IdeVariable>> {
        if variables_reference <= 0 {
            return Ok(Vec::new());
        }
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))?
            .value()
            .clone();
        let conn = entry.active_conn()?;
        fetch_variables(&conn.wire, variables_reference).await
    }

    /// Evaluate an expression in the active (or specified) stack frame.
    pub async fn evaluate(
        &self,
        id: &str,
        req: IdeDebugEvaluateReq,
    ) -> Result<IdeDebugEvaluateResp> {
        if req.expression.trim().is_empty() {
            return Err(Error::Invalid("expression must not be empty".into()));
        }
        let entry = self
            .sessions
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))?
            .value()
            .clone();
        let conn = entry.active_conn()?;
        let frame_id = req
            .frame_id
            .or_else(|| entry.read_snapshot().active_frame_id);
        let context = req.context.unwrap_or_else(|| "repl".to_string());
        let mut args = json!({
            "expression": req.expression,
            "context": context,
        });
        if let Some(fid) = frame_id {
            args["frameId"] = json!(fid);
        }
        let resp = conn.wire.request("evaluate", Some(args)).await?;
        let body = resp.body.unwrap_or_else(|| json!({}));
        let result = body
            .get("result")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let var_type = body.get("type").and_then(Value::as_str).map(String::from);
        let variables_reference = body
            .get("variablesReference")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        Ok(IdeDebugEvaluateResp {
            result,
            var_type,
            variables_reference,
        })
    }
}

fn build_session_title(req: &IdeStartDebugReq, adapter: &str) -> String {
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        return format!("Debug {tname} ({adapter})");
    }
    let prog_label = std::path::Path::new(&req.launch.program)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&req.language);
    format!("Debug {prog_label} ({adapter})")
}

fn mark_resumed(entry: &SessionEntry) {
    entry.mutate_and_broadcast(|snap| {
        snap.state = IdeDebugState::Running;
        snap.stopped_reason = None;
        snap.paused_file = None;
        snap.paused_line = None;
        snap.inline_values.clear();
    });
}

async fn stop_entry(entry: &SessionEntry) {
    let old = entry.conn.write().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(conn) = old {
        if conn.supports_terminate {
            let _ = tokio::time::timeout(
                Duration::from_millis(800),
                conn.wire.request("terminate", Some(json!({}))),
            )
            .await;
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(900),
            conn.wire
                .request("disconnect", Some(json!({ "terminateDebuggee": true }))),
        )
        .await;
        conn.guard.kill_all();
    }
    entry.mutate_and_broadcast(|snap| {
        snap.state = IdeDebugState::Terminated;
        snap.stopped_reason = None;
        snap.paused_line = None;
        snap.inline_values.clear();
    });
}

fn parse_listening_port(line: &str) -> Option<u16> {
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

async fn boot_session_connection(
    entry: &Arc<SessionEntry>,
    resolved: ResolvedDapAdapter,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
) -> Result<()> {
    let (read_io, write_io, guard) =
        connect_adapter_transport(entry, &resolved.adapter_name, resolved.transport, req, ctx)
            .await?;
    handshake_and_run(
        entry,
        read_io,
        write_io,
        guard,
        resolved.initialize_args,
        &resolved.launch_command,
        resolved.launch_args,
    )
    .await
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
                tokio::spawn(async move {
                    let mut lines = BufReader::new(stderr).lines();
                    while let Ok(Some(l)) = lines.next_line().await {
                        entry_err.append_output("stderr", format!("{l}\n"));
                    }
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
            spawn_tcp_listen_adapter(
                entry,
                adapter_name,
                &command,
                &args,
                &cwd,
                &env,
                req,
                ctx,
                guard,
            )
            .await
        }
        DapAdapterTransport::DirectTcpAttach { host, port } => {
            let (connect_host, connect_port) = if req.target.kind == IdeTargetKind::Ssh {
                let ssh = ctx
                    .ssh
                    .as_ref()
                    .ok_or_else(|| Error::Invalid("missing SSH spec".into()))?;
                let tunnel = SshTunnel::open(&ssh_tunnel_config(ssh)?, &host, port).await?;
                let lp = tunnel.local_port();
                guard.push_tunnel(tunnel);
                ("127.0.0.1".to_string(), lp)
            } else {
                (host, port)
            };
            let tcp = TcpStream::connect((connect_host.as_str(), connect_port))
                .await
                .map_err(|e| {
                    Error::Upstream(format!("attach DAP {connect_host}:{connect_port}: {e}"))
                })?;
            let (r, w) = tcp.into_split();
            Ok((Box::new(r), Box::new(w), guard))
        }
        DapAdapterTransport::NodeInspectorBridge(spec) => {
            let (duplex, bridge_guard) =
                spawn_node_inspector_bridge(&req.target, ctx.ssh.as_ref(), spec).await?;
            let (r, w) = tokio::io::split(duplex);
            Ok((Box::new(r), Box::new(w), bridge_guard))
        }
        DapAdapterTransport::JavaJdbBridge(spec) => {
            let (duplex, bridge_guard) =
                spawn_java_jdb_bridge(&req.target, ctx.ssh.as_ref(), spec).await?;
            let (r, w) = tokio::io::split(duplex);
            Ok((Box::new(r), Box::new(w), bridge_guard))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn spawn_tcp_listen_adapter(
    entry: &Arc<SessionEntry>,
    adapter_name: &str,
    command: &str,
    args: &[String],
    cwd: &str,
    env: &BTreeMap<String, String>,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    guard: Arc<BridgeGuard>,
) -> Result<(BoxRead, BoxWrite, Arc<BridgeGuard>)> {
    let mut child = spawn_target_process(&req.target, ctx.ssh.as_ref(), command, args, cwd, env)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (port_tx, mut port_rx) = mpsc::unbounded_channel::<(String, String)>();
    if let Some(out) = stdout {
        let tx = port_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(("stdout".into(), l));
            }
        });
    }
    if let Some(err) = stderr {
        let tx = port_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                let _ = tx.send(("stderr".into(), l));
            }
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

    let connect_port = if req.target.kind == IdeTargetKind::Ssh {
        let ssh = ctx
            .ssh
            .as_ref()
            .ok_or_else(|| Error::Invalid("missing SSH spec".into()))?;
        let tunnel = SshTunnel::open(&ssh_tunnel_config(ssh)?, "127.0.0.1", remote_port).await?;
        let lp = tunnel.local_port();
        guard.push_tunnel(tunnel);
        lp
    } else {
        remote_port
    };
    let tcp = TcpStream::connect(("127.0.0.1", connect_port))
        .await
        .map_err(|e| Error::Upstream(format!("connect DAP 127.0.0.1:{connect_port}: {e}")))?;
    let (r, w) = tcp.into_split();
    Ok((Box::new(r), Box::new(w), guard))
}

async fn handshake_and_run(
    entry: &Arc<SessionEntry>,
    read_io: BoxRead,
    write_io: BoxWrite,
    guard: Arc<BridgeGuard>,
    initialize_args: Value,
    launch_command: &str,
    launch_args: Value,
) -> Result<()> {
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let wire = DapWireClient::spawn(read_io, write_io, ev_tx);

    spawn_event_driver(entry.clone(), ev_rx);

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

    let launch_rx = wire
        .send_request_nowait(launch_command, Some(launch_args))
        .await?;

    let _ = tokio::time::timeout(Duration::from_secs(8), entry.initialized_notify.notified()).await;

    sync_all_breakpoints(entry, &active_conn).await?;
    let _ = wire.request("configurationDone", Some(json!({}))).await;

    entry.mutate_and_broadcast(|snap| {
        if snap.state == IdeDebugState::Starting {
            snap.state = IdeDebugState::Running;
        }
    });

    let entry_launch = entry.clone();
    tokio::spawn(async move {
        if let Ok(resp) = launch_rx.await {
            if !resp.success {
                let msg = resp
                    .message
                    .unwrap_or_else(|| "Debug target launch failed".into());
                entry_launch.mutate_and_broadcast(|snap| {
                    snap.state = IdeDebugState::Error;
                    snap.error = Some(msg);
                });
            }
        }
    });

    Ok(())
}

fn spawn_event_driver(entry: Arc<SessionEntry>, mut ev_rx: mpsc::UnboundedReceiver<DapEvent>) {
    tokio::spawn(async move {
        while let Some(ev) = ev_rx.recv().await {
            match ev.event.as_str() {
                "initialized" => {
                    entry.initialized_notify.notify_one();
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
                    mark_resumed(&entry);
                }
                "stopped" => {
                    let reason = ev
                        .body
                        .as_ref()
                        .and_then(|b| b.get("reason"))
                        .and_then(Value::as_str)
                        .unwrap_or("breakpoint")
                        .to_string();
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
                    let should_mark = {
                        let cur = entry.read_snapshot().state;
                        !matches!(cur, IdeDebugState::Terminated | IdeDebugState::Error)
                    };
                    if should_mark {
                        entry.mutate_and_broadcast(|snap| {
                            snap.state = IdeDebugState::Terminated;
                            snap.stopped_reason = None;
                            snap.paused_line = None;
                            snap.inline_values.clear();
                        });
                    }
                    if ev.event == "__eof__" {
                        break;
                    }
                }
                _ => {}
            }
        }
    });
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
