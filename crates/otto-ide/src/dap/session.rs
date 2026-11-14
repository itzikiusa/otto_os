//! Stateful DAP session lifecycle manager (`DapManager`).
//! Drives Debug Adapter Protocol sessions over stdio, loopback TCP, SSH tunnels,
//! and in-memory bridges with bounded session & console-output retention.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use otto_core::{new_id, Error, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};

use super::adapters::{normalize_target_path, resolve_dap_adapter};
use super::session_inspect::{fetch_variables, select_frame_and_refresh, sync_all_breakpoints};
use super::session_transport::{
    boot_session_connection, evict_terminated_sessions, handshake_and_run,
};
use super::wire::{DapEvent, DapMessage, DapReader, DapResponse, DapWriter};
use super::BridgeGuard;
use crate::types::{
    IdeDebugAction, IdeDebugCommandReq, IdeDebugEvaluateReq, IdeDebugEvaluateResp,
    IdeDebugOutputEntry, IdeDebugSessionSnapshot, IdeDebugState, IdeProjectEnvironment,
    IdeRunConfig, IdeSetBreakpointsReq, IdeSshSpec, IdeStartDebugReq, IdeVariable,
    PluginDapAdapterSpec,
};

pub use super::session_inspect::compute_inline_values;

pub(crate) const MAX_DEBUG_OUTPUT: usize = 400;
pub(crate) const MAX_DEBUG_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_TERMINATED_DEBUG_SESSIONS: usize = 4;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
pub(super) type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

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
    retired: AtomicBool,
}

impl DapWireClient {
    pub(super) fn spawn(
        reader_io: BoxRead,
        writer_io: BoxWrite,
        event_tx: mpsc::UnboundedSender<DapEvent>,
    ) -> Arc<Self> {
        let client = Arc::new(Self {
            writer: tokio::sync::Mutex::new(DapWriter::new(writer_io)),
            pending: Mutex::new(HashMap::new()),
            retired: AtomicBool::new(false),
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
                        if !client_ref.retired.load(Ordering::SeqCst) {
                            let _ = event_tx.send(ev);
                        }
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
            if !client_ref.retired.load(Ordering::SeqCst) {
                let _ = event_tx.send(DapEvent::new(0, "__eof__", None));
            }
        });
        client
    }

    pub(super) fn retire(&self) {
        self.retired.store(true, Ordering::SeqCst);
    }

    pub(super) fn abort_pending(&self) {
        if let Ok(mut map) = self.pending.lock() {
            map.clear();
        }
    }

    pub(super) async fn send_request_nowait(
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
    pub(super) created_seq: u64,
    pub(super) orig_req: RwLock<IdeStartDebugReq>,
    pub(super) orig_ctx: RwLock<DapStartContext>,
    pub(super) snapshot: RwLock<IdeDebugSessionSnapshot>,
    pub(super) output_buf: Mutex<VecDeque<IdeDebugOutputEntry>>,
    pub(super) configured_bp_files: Mutex<BTreeSet<String>>,
    pub(super) temp_run_to_cursor: Mutex<Option<(String, u32)>>,
    pub(super) break_on_exceptions: Mutex<bool>,
    pub(super) conn: RwLock<Option<Arc<ActiveConnection>>>,
    pub(super) events_tx: broadcast::Sender<IdeDebugSessionSnapshot>,
    pub(super) initialized_notify: Arc<Notify>,
    pub(super) halted_notify: Arc<Notify>,
    pub(super) suppress_transient_pause: AtomicBool,
    pub(super) restarting: AtomicBool,
}

impl SessionEntry {
    fn new(
        created_seq: u64,
        (id, title, language, adapter): (String, String, String, String),
        initial_output: Vec<IdeDebugOutputEntry>,
        req: IdeStartDebugReq,
        ctx: DapStartContext,
    ) -> Arc<Self> {
        let (events_tx, _) = broadcast::channel(128);
        let mut output_buf = VecDeque::with_capacity(initial_output.len().min(MAX_DEBUG_OUTPUT));
        for item in initial_output {
            if output_buf.len() >= MAX_DEBUG_OUTPUT {
                output_buf.pop_front();
            }
            output_buf.push_back(item);
        }
        let initial_snap = IdeDebugSessionSnapshot {
            id,
            title,
            language,
            adapter,
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
        Arc::new(Self {
            created_seq,
            orig_req: RwLock::new(req),
            orig_ctx: RwLock::new(ctx),
            snapshot: RwLock::new(initial_snap),
            output_buf: Mutex::new(output_buf),
            configured_bp_files: Mutex::new(BTreeSet::new()),
            temp_run_to_cursor: Mutex::new(None),
            break_on_exceptions: Mutex::new(false),
            conn: RwLock::new(None),
            events_tx,
            initialized_notify: Arc::new(Notify::new()),
            halted_notify: Arc::new(Notify::new()),
            suppress_transient_pause: AtomicBool::new(false),
            restarting: AtomicBool::new(false),
        })
    }

    fn snapshot_output(&self) -> Vec<IdeDebugOutputEntry> {
        self.output_buf
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    pub(super) fn is_terminal(&self) -> bool {
        let state = self
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .state;
        matches!(state, IdeDebugState::Terminated | IdeDebugState::Error)
    }

    pub(super) fn disconnect_and_kill(&self) {
        if self.restarting.load(Ordering::SeqCst) {
            return;
        }
        if let Some(conn) = self.conn.write().unwrap_or_else(|e| e.into_inner()).take() {
            conn.wire.retire();
            conn.wire.abort_pending();
            conn.guard.kill_all();
        }
    }

    pub(super) fn mark_terminated(&self) {
        if self.restarting.load(Ordering::SeqCst) {
            return;
        }
        self.mutate_and_broadcast(|snap| {
            snap.state = IdeDebugState::Terminated;
            snap.stopped_reason = None;
            snap.paused_line = None;
            snap.threads.clear();
            snap.stack_frames.clear();
            snap.scopes.clear();
            snap.inline_values.clear();
        });
    }

    pub(super) fn fail_and_evict(
        &self,
        sessions: &DashMap<String, Arc<SessionEntry>>,
        msg: String,
    ) {
        self.disconnect_and_kill();
        self.mutate_and_broadcast(|snap| {
            snap.state = IdeDebugState::Error;
            snap.threads.clear();
            snap.stack_frames.clear();
            snap.scopes.clear();
            snap.inline_values.clear();
            snap.error = Some(msg);
        });
        evict_terminated_sessions(sessions);
    }

    pub(super) fn read_snapshot(&self) -> IdeDebugSessionSnapshot {
        let mut snap = self
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        snap.output = self.snapshot_output();
        snap
    }

    pub(super) fn mutate_and_broadcast(
        &self,
        f: impl FnOnce(&mut IdeDebugSessionSnapshot),
    ) -> IdeDebugSessionSnapshot {
        let mut updated = {
            let mut snap = self.snapshot.write().unwrap_or_else(|e| e.into_inner());
            f(&mut snap);
            snap.clone()
        };
        updated.output = self.snapshot_output();
        let _ = self.events_tx.send(updated.clone());
        updated
    }

    pub(super) fn append_output(&self, category: &str, text: String) {
        if text.is_empty() {
            return;
        }
        let clipped = if text.len() > 16_384 {
            let mut end = 16_384;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…\n", &text[..end])
        } else {
            text
        };
        {
            let mut buf = self.output_buf.lock().unwrap_or_else(|e| e.into_inner());
            if buf.len() >= MAX_DEBUG_OUTPUT {
                buf.pop_front();
            }
            buf.push_back(IdeDebugOutputEntry {
                category: category.to_string(),
                text: clipped,
            });
            let mut total_bytes: usize = buf.iter().map(|e| e.text.len()).sum();
            while total_bytes > MAX_DEBUG_OUTPUT_BYTES && buf.len() > 1 {
                if let Some(removed) = buf.pop_front() {
                    total_bytes = total_bytes.saturating_sub(removed.text.len());
                }
            }
        }
        self.mutate_and_broadcast(|_| {});
    }

    pub(super) fn active_conn(&self) -> Result<Arc<ActiveConnection>> {
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
    seq_counter: Arc<AtomicU64>,
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
            seq_counter: Arc::new(AtomicU64::new(0)),
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

    fn get_entry(&self, id: &str) -> Result<Arc<SessionEntry>> {
        self.sessions
            .get(id)
            .map(|e| e.value().clone())
            .ok_or_else(|| Error::NotFound(format!("debug session '{id}' not found")))
    }

    /// Return the current snapshot for debug session `id`.
    pub fn get_session(&self, id: &str) -> Result<IdeDebugSessionSnapshot> {
        Ok(self.get_entry(id)?.read_snapshot())
    }

    /// Subscribe to real-time [`IdeDebugSessionSnapshot`] updates for SSE streaming.
    pub fn subscribe_events(
        &self,
        id: &str,
    ) -> Result<broadcast::Receiver<IdeDebugSessionSnapshot>> {
        Ok(self.get_entry(id)?.events_tx.subscribe())
    }

    fn register_entry(
        &self,
        req: &mut IdeStartDebugReq,
        ctx: DapStartContext,
        language: String,
        adapter: String,
        initial_output: Vec<IdeDebugOutputEntry>,
    ) -> Arc<SessionEntry> {
        for bp in &mut req.breakpoints {
            bp.path = normalize_target_path(&req.target, &bp.path);
        }
        let id = format!("dbg_{}", new_id());
        let title = build_session_title(req, &adapter);
        let seq = self.seq_counter.fetch_add(1, Ordering::Relaxed);
        let meta = (id.clone(), title, language, adapter);
        let entry = SessionEntry::new(seq, meta, initial_output, req.clone(), ctx);
        self.sessions.insert(id, entry.clone());
        evict_terminated_sessions(&self.sessions);
        entry
    }

    /// Resolve the language adapter, launch/attach the target, configure breakpoints,
    /// and return the live session snapshot.
    pub async fn start_session(
        &self,
        mut req: IdeStartDebugReq,
        mut ctx: DapStartContext,
    ) -> Result<IdeDebugSessionSnapshot> {
        for folder in &req.project_folders {
            let trimmed = folder.trim();
            if !trimmed.is_empty() && !req.target.project_folders.iter().any(|f| f == trimmed) {
                req.target.project_folders.push(trimmed.to_string());
            }
            if !trimmed.is_empty() && !req.target.additional_roots.iter().any(|f| f == trimmed) {
                req.target.additional_roots.push(trimmed.to_string());
            }
        }
        let spring_profiles = if !req.spring_profiles.is_empty() {
            &req.spring_profiles
        } else {
            &req.launch.spring_profiles
        };
        if !spring_profiles.is_empty() {
            let joined = spring_profiles.join(",");
            ctx.effective_env
                .entry("SPRING_PROFILES_ACTIVE".into())
                .or_insert(joined);
        }
        let resolved = resolve_dap_adapter(&self.data_dir, &req, &ctx).await?;
        let entry = self.register_entry(
            &mut req,
            ctx.clone(),
            resolved.language.clone(),
            resolved.adapter_name.clone(),
            resolved.initial_output.clone(),
        );
        if let Err(err) =
            boot_session_connection(&self.sessions, &entry, resolved, &req, &ctx).await
        {
            entry.fail_and_evict(&self.sessions, err.to_string());
            return Err(err);
        }
        Ok(entry.read_snapshot())
    }

    /// Start a debug session over a pre-connected async stream.
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
        let adapter_str = adapter_name.into();
        let ctx = DapStartContext::new().with_caller_is_root(caller_is_root);
        let lang = req.language.clone();
        let entry = self.register_entry(&mut req, ctx, lang, adapter_str.clone(), Vec::new());

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
        let io = (
            Box::new(r) as BoxRead,
            Box::new(w) as BoxWrite,
            BridgeGuard::new(),
        );
        let cmds = (init_args, launch_cmd, launch_args);
        if let Err(err) = handshake_and_run(&self.sessions, &entry, io, cmds).await {
            entry.fail_and_evict(&self.sessions, err.to_string());
            return Err(err);
        }
        Ok(entry.read_snapshot())
    }

    /// Update source and exception breakpoints on an active session.
    pub async fn set_breakpoints(
        &self,
        id: &str,
        mut req: IdeSetBreakpointsReq,
    ) -> Result<IdeDebugSessionSnapshot> {
        let entry = self.get_entry(id)?;

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
        let entry = self.get_entry(id)?;

        match req.action {
            IdeDebugAction::Stop => {
                stop_entry(&entry).await;
                evict_terminated_sessions(&self.sessions);
                return Ok(entry.read_snapshot());
            }
            IdeDebugAction::Restart => {
                return self.restart_entry(&entry).await;
            }
            IdeDebugAction::SelectFrame => {
                let conn = entry.active_conn()?;
                select_frame_and_refresh(&entry, &conn, req.thread_id, req.frame_id).await?;
                return Ok(entry.read_snapshot());
            }
            _ => {}
        }

        let conn = entry.active_conn()?;
        let snap = entry.read_snapshot();
        let thread_id = req.thread_id.or(snap.active_thread_id).unwrap_or(1);

        let tid_arg = Some(json!({ "threadId": thread_id }));
        match req.action {
            IdeDebugAction::Continue
            | IdeDebugAction::StepOver
            | IdeDebugAction::StepInto
            | IdeDebugAction::StepOut => {
                let dap_cmd = match req.action {
                    IdeDebugAction::Continue => "continue",
                    IdeDebugAction::StepOver => "next",
                    IdeDebugAction::StepInto => "stepIn",
                    _ => "stepOut",
                };
                mark_resumed(&entry);
                conn.wire.request(dap_cmd, tid_arg).await?;
            }
            IdeDebugAction::Pause => {
                conn.wire.request("pause", tid_arg).await?;
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
                if let Ok(mut slot) = entry.temp_run_to_cursor.lock() {
                    *slot = Some((normalize_target_path(&target, raw_path), line));
                }
                sync_all_breakpoints(&entry, &conn).await?;
                mark_resumed(&entry);
                conn.wire.request("continue", tid_arg).await?;
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
        let conn = self.get_entry(id)?.active_conn()?;
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
        let entry = self.get_entry(id)?;
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

    async fn restart_entry(&self, entry: &Arc<SessionEntry>) -> Result<IdeDebugSessionSnapshot> {
        use futures_util::FutureExt;

        entry.restarting.store(true, Ordering::SeqCst);
        teardown_connection(entry).await;
        // Let any event queued before `wire.retire()` drain harmlessly while
        // `restarting` suppresses `disconnect_and_kill` / `mark_terminated`.
        tokio::task::yield_now().await;

        if let Ok(mut files) = entry.configured_bp_files.lock() {
            files.clear();
        }
        if let Ok(mut rtc) = entry.temp_run_to_cursor.lock() {
            *rtc = None;
        }
        entry
            .suppress_transient_pause
            .store(false, Ordering::SeqCst);
        let _ = entry.initialized_notify.notified().now_or_never();
        let _ = entry.halted_notify.notified().now_or_never();

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
            snap.threads.clear();
            snap.stack_frames.clear();
            snap.scopes.clear();
            snap.inline_values.clear();
            snap.error = None;
        });
        entry.restarting.store(false, Ordering::SeqCst);

        let resolved = match resolve_dap_adapter(&self.data_dir, &orig_req, &orig_ctx).await {
            Ok(res) => res,
            Err(err) => {
                entry.fail_and_evict(&self.sessions, err.to_string());
                return Err(err);
            }
        };
        if let Err(err) =
            boot_session_connection(&self.sessions, entry, resolved, &orig_req, &orig_ctx).await
        {
            entry.fail_and_evict(&self.sessions, err.to_string());
            return Err(err);
        }
        Ok(entry.read_snapshot())
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

pub(super) fn mark_resumed(entry: &SessionEntry) {
    entry.mutate_and_broadcast(|snap| {
        snap.state = IdeDebugState::Running;
        snap.stopped_reason = None;
        snap.paused_file = None;
        snap.paused_line = None;
        snap.inline_values.clear();
    });
}

async fn teardown_connection(entry: &SessionEntry) {
    let old = entry.conn.write().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(conn) = old {
        conn.wire.retire();
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
        conn.wire.abort_pending();
        conn.guard.kill_all();
    }
}

async fn stop_entry(entry: &SessionEntry) {
    teardown_connection(entry).await;
    entry.mark_terminated();
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
