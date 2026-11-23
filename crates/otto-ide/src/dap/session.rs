//! Stateful DAP session lifecycle manager (`DapManager`).
//! Drives Debug Adapter Protocol sessions over stdio, loopback TCP, SSH tunnels,
//! and in-memory bridges with bounded session & console-output retention.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::broadcast;

use super::adapters::{
    display_target_path, normalize_target_path, preflight_debug_request,
    resolve_dap_adapter_with_manager,
};
pub(super) use super::session_entry::{
    ActiveConnection, BoxRead, BoxWrite, DapWireClient, SessionEntry,
};
pub use super::session_inline::compute_inline_values;
pub(super) use super::session_inline::mark_resumed;
use super::session_inline::{build_session_title, shutdown_entry, stop_entry};
use super::session_inspect::{select_frame_and_refresh, sync_all_breakpoints};
use super::session_transport::{
    boot_session_connection, evict_terminated_sessions, handshake_and_run,
};
use super::BridgeGuard;
use crate::environments::EnvironmentManager;
use crate::types::{
    IdeDebugAction, IdeDebugCommandReq, IdeDebugOutputEntry, IdeDebugSessionSnapshot,
    IdeProjectEnvironment, IdeRunConfig, IdeSetBreakpointsReq, IdeSshSpec, IdeStartDebugReq,
    IdeTargetKind, IdeTargetSpec, PluginDapAdapterSpec,
};

pub(crate) const MAX_DEBUG_OUTPUT: usize = 400;
pub(crate) const MAX_DEBUG_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_TERMINATED_DEBUG_SESSIONS: usize = 4;

/// Optional host/environment context passed when starting a debug session.
#[derive(Clone, Default)]
pub struct DapStartContext {
    pub ssh: Option<IdeSshSpec>,
    pub effective_env: BTreeMap<String, String>,
    pub shell_env_snapshot: BTreeMap<String, String>,
    pub project_env: Option<IdeProjectEnvironment>,
    pub go_build_tags: Vec<String>,
    pub run_config: Option<IdeRunConfig>,
    pub plugin_adapters: Vec<PluginDapAdapterSpec>,
    pub netbridge: Option<Arc<crate::netbridge::NetBridgeLease>>,
    pub(crate) dormant_netbridge: Option<crate::netbridge::DormantNetBridge>,
    pub caller_is_root: bool,
    pub owner_user_id: String,
}

impl std::fmt::Debug for DapStartContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let eff_keys: Vec<&str> = self.effective_env.keys().map(String::as_str).collect();
        let shell_keys: Vec<&str> = self.shell_env_snapshot.keys().map(String::as_str).collect();
        f.debug_struct("DapStartContext")
            .field("ssh_present", &self.ssh.is_some())
            .field("effective_env_keys", &eff_keys)
            .field("shell_env_keys", &shell_keys)
            .field("go_build_tags", &self.go_build_tags)
            .field("netbridge_present", &self.netbridge.is_some())
            .field("caller_is_root", &self.caller_is_root)
            .field("owner_user_id", &self.owner_user_id)
            .finish()
    }
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

    pub fn with_shell_env_snapshot(mut self, env: BTreeMap<String, String>) -> Self {
        self.shell_env_snapshot = env;
        self
    }

    pub fn with_project_env(mut self, env: Option<IdeProjectEnvironment>) -> Self {
        self.project_env = env;
        self
    }

    pub fn with_go_build_tags(mut self, tags: Vec<String>) -> Self {
        self.go_build_tags = tags;
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

    pub fn with_netbridge(
        mut self,
        netbridge: Option<Arc<crate::netbridge::NetBridgeLease>>,
    ) -> Self {
        self.dormant_netbridge = netbridge.as_ref().map(|nb| nb.dormant_handle());
        self.netbridge = netbridge;
        self
    }

    pub fn with_caller_is_root(mut self, is_root: bool) -> Self {
        self.caller_is_root = is_root;
        self
    }

    pub fn with_owner_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.owner_user_id = user_id.into();
        self
    }
}

/// Live OS resource telemetry for one active debug session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DapSessionResourceMetrics {
    pub session_id: String,
    pub pids: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fd_count: Option<u32>,
}

/// Multi-session Debug Adapter Protocol engine mounted on `IdeEngine`.
pub struct DapManager {
    pub(super) data_dir: PathBuf,
    pub(super) environments: EnvironmentManager,
    pub(super) sessions: Arc<DashMap<String, Arc<SessionEntry>>>,
    seq_counter: Arc<AtomicU64>,
}

impl Default for DapManager {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("otto-ide-dap"))
    }
}

impl DapManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let environments = EnvironmentManager::new(data_dir.clone());
        Self::with_environments(data_dir, environments)
    }

    pub fn with_environments(data_dir: PathBuf, environments: EnvironmentManager) -> Self {
        Self {
            data_dir,
            environments,
            sessions: Arc::new(DashMap::new()),
            seq_counter: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Sample live RSS and open file-descriptor counts for all active debug
    /// sessions off the async runtime (`spawn_blocking`).
    pub async fn resource_metrics(&self) -> Vec<DapSessionResourceMetrics> {
        let mut per_session: Vec<(String, Vec<u32>)> = Vec::new();
        let mut all_pids: Vec<u32> = Vec::new();
        for entry in self.sessions.iter() {
            let sid = entry.key().clone();
            let pids = entry
                .value()
                .conn
                .read()
                .ok()
                .and_then(|c| c.as_ref().map(|ac| ac.guard.pids()))
                .unwrap_or_default();
            all_pids.extend(pids.iter().copied());
            per_session.push((sid, pids));
        }
        let samples = crate::lsp_host::pool_spawn::sample_pids_metrics_async(all_pids).await;
        per_session
            .into_iter()
            .map(|(session_id, pids)| {
                let mut rss_sum = 0u64;
                let mut any_rss = false;
                let mut fd_sum = 0u32;
                let mut any_fd = false;
                for pid in &pids {
                    if let Some(s) = samples.get(pid) {
                        if let Some(r) = s.rss_bytes {
                            rss_sum = rss_sum.saturating_add(r);
                            any_rss = true;
                        }
                        if let Some(f) = s.fd_count {
                            fd_sum = fd_sum.saturating_add(f);
                            any_fd = true;
                        }
                    }
                }
                DapSessionResourceMetrics {
                    session_id,
                    pids,
                    rss_bytes: any_rss.then_some(rss_sum),
                    fd_count: any_fd.then_some(fd_sum),
                }
            })
            .collect()
    }

    /// List snapshots of all debug sessions, newest first.
    pub fn list_sessions(&self) -> Vec<IdeDebugSessionSnapshot> {
        self.newest_first(|_| true)
    }

    /// Snapshots of the sessions `keep` accepts, newest first (by start order:
    /// ids are random UUIDs and say nothing about age).
    fn newest_first(&self, keep: impl Fn(&str) -> bool) -> Vec<IdeDebugSessionSnapshot> {
        let mut out: Vec<(u64, IdeDebugSessionSnapshot)> = self
            .sessions
            .iter()
            .filter(|item| keep(item.value().owner_user_id.as_str()))
            .map(|item| (item.value().created_seq, item.value().read_snapshot()))
            .collect();
        out.sort_by_key(|(seq, _)| std::cmp::Reverse(*seq));
        out.into_iter().map(|(_, snap)| snap).collect()
    }

    /// List snapshots visible to `user` (all for root, owner-only otherwise).
    pub fn list_sessions_for_user(&self, user: &User) -> Vec<IdeDebugSessionSnapshot> {
        let has_root_auth = otto_core::auth::root_authority(user);
        self.newest_first(|owner| has_root_auth || owner == user.id.as_str())
    }

    /// Filter an arbitrary snapshot slice down to sessions owned by `user`.
    pub fn filter_sessions_for_user(
        &self,
        user: &User,
        sessions: Vec<IdeDebugSessionSnapshot>,
    ) -> Vec<IdeDebugSessionSnapshot> {
        if otto_core::auth::root_authority(user) {
            return sessions;
        }
        sessions
            .into_iter()
            .filter(|s| {
                self.sessions
                    .get(&s.id)
                    .is_some_and(|e| e.value().owner_user_id == user.id.as_str())
            })
            .collect()
    }

    /// Verify that `user` is authorized to inspect or control debug session `id`.
    pub fn authorize_session(&self, user: &User, id: &str) -> Result<()> {
        let entry = self.get_entry(id)?;
        if otto_core::auth::root_authority(user) || entry.owner_user_id == user.id.as_str() {
            Ok(())
        } else {
            Err(Error::Forbidden(
                "debug session belongs to another user".into(),
            ))
        }
    }

    /// Return `(owner_user_id, target)` for an existing debug session.
    pub fn session_owner_and_target(&self, id: &str) -> Option<(String, IdeTargetSpec)> {
        let entry = self.sessions.get(id)?;
        let target = entry
            .value()
            .orig_req
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .target
            .clone();
        Some((entry.value().owner_user_id.clone(), target))
    }

    /// If debug session `id` had an enabled [`crate::types::IdeNetworkBridgeConfig`]
    /// whose active lease was released on target exit, return `(target, bridge_cfg, confirm_prod_targets)`
    /// so the HTTP handler can re-authorize and re-acquire a fresh lease before `Restart`.
    pub fn session_restart_netbridge_spec(
        &self,
        id: &str,
    ) -> Option<(IdeTargetSpec, crate::types::IdeNetworkBridgeConfig, bool)> {
        let entry = self.sessions.get(id)?;
        let ctx_guard = entry
            .value()
            .orig_ctx
            .read()
            .unwrap_or_else(|e| e.into_inner());
        if ctx_guard.netbridge.is_some() {
            return None;
        }
        let nb = ctx_guard
            .project_env
            .as_ref()
            .and_then(|p| p.network_bridge.as_ref())
            .filter(|nb| nb.enabled)?
            .clone();
        drop(ctx_guard);
        let req_guard = entry
            .value()
            .orig_req
            .read()
            .unwrap_or_else(|e| e.into_inner());
        Some((req_guard.target.clone(), nb, req_guard.confirm_prod_targets))
    }

    /// Attach a freshly re-acquired [`crate::netbridge::NetBridgeLease`] onto session `id`
    /// prior to `Restart`.
    pub fn set_session_netbridge(
        &self,
        id: &str,
        lease: Option<Arc<crate::netbridge::NetBridgeLease>>,
    ) {
        if let Some(entry) = self.sessions.get(id) {
            let mut ctx_guard = entry
                .value()
                .orig_ctx
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(ref l) = lease {
                ctx_guard.dormant_netbridge = Some(l.dormant_handle());
            }
            ctx_guard.netbridge = lease;
        }
    }

    /// Refresh active runtime plugin DAP adapter contributions on session `id` prior to `Restart`.
    pub fn set_session_plugin_adapters(
        &self,
        id: &str,
        adapters: Vec<crate::types::PluginDapAdapterSpec>,
    ) {
        if let Some(entry) = self.sessions.get(id) {
            let mut ctx_guard = entry
                .value()
                .orig_ctx
                .write()
                .unwrap_or_else(|e| e.into_inner());
            ctx_guard.plugin_adapters = adapters;
        }
    }

    /// Stop and tear down all active debug sessions on daemon shutdown —
    /// concurrently, each bounded, so one slow adapter or a session still
    /// booting cannot use up the host's shutdown slot.
    pub async fn shutdown(&self) {
        let entries: Vec<Arc<SessionEntry>> =
            self.sessions.iter().map(|e| e.value().clone()).collect();
        futures_util::future::join_all(entries.iter().map(|e| shutdown_entry(e))).await;
        self.sessions.clear();
    }

    /// Return the newest active debug session for `target` owned by `owner_user_id`.
    pub fn active_session_for_target_and_owner(
        &self,
        target: &IdeTargetSpec,
        owner_user_id: &str,
    ) -> Option<IdeDebugSessionSnapshot> {
        let want_key = target.project_key();
        let mut matches: Vec<(u64, IdeDebugSessionSnapshot)> = self
            .sessions
            .iter()
            .filter_map(|item| {
                let entry = item.value();
                if entry.owner_user_id != owner_user_id || entry.is_terminal() {
                    return None;
                }
                let tk = entry
                    .orig_req
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .target
                    .project_key();
                (tk == want_key).then(|| (entry.created_seq, entry.read_snapshot()))
            })
            .collect();
        matches.sort_by_key(|(seq, _)| std::cmp::Reverse(*seq));
        matches.into_iter().next().map(|(_, snap)| snap)
    }

    pub(super) fn get_entry(&self, id: &str) -> Result<Arc<SessionEntry>> {
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
            bp.path = display_target_path(&req.target, &bp.path);
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

    fn prepare_start_inputs(req: &mut IdeStartDebugReq, ctx: &mut DapStartContext) {
        if ctx.caller_is_root {
            for folder in &req.project_folders {
                let trimmed = folder.trim();
                if !trimmed.is_empty() && !req.target.project_folders.iter().any(|f| f == trimmed) {
                    req.target.project_folders.push(trimmed.to_string());
                }
                if !trimmed.is_empty() && !req.target.additional_roots.iter().any(|f| f == trimmed)
                {
                    req.target.additional_roots.push(trimmed.to_string());
                }
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
    }

    /// Resolve the language adapter, launch/attach the target, configure breakpoints,
    /// and return the live session snapshot.
    pub async fn start_session(
        &self,
        mut req: IdeStartDebugReq,
        mut ctx: DapStartContext,
    ) -> Result<IdeDebugSessionSnapshot> {
        Self::prepare_start_inputs(&mut req, &mut ctx);
        let resolved = otto_telemetry::context::measure_result(
            "ide.dap.resolve_adapter",
            resolve_dap_adapter_with_manager(&self.data_dir, &req, &ctx, Some(&self.environments)),
        )
        .await?;
        let entry = self.register_entry(
            &mut req,
            ctx.clone(),
            resolved.language.clone(),
            resolved.adapter_name.clone(),
            resolved.initial_output.clone(),
        );
        if let Err(err) = otto_telemetry::context::measure_result(
            "ide.dap.handshake",
            boot_session_connection(&self.sessions, &entry, resolved, &req, &ctx),
        )
        .await
        {
            entry.fail_start(&self.sessions, &err);
            return Err(err);
        }
        Ok(entry.read_snapshot())
    }

    /// Non-blocking HTTP start path: runs synchronous RBAC/target/language
    /// validation immediately (`400`/`403`), registers the session in `Starting`
    /// state, and offloads target compilation (`go build`/`cargo build`/`javac`)
    /// plus adapter boot onto a background task streaming state via `events_tx`.
    pub fn start_session_async(
        &self,
        mut req: IdeStartDebugReq,
        mut ctx: DapStartContext,
    ) -> Result<(
        IdeDebugSessionSnapshot,
        broadcast::Receiver<IdeDebugSessionSnapshot>,
    )> {
        Self::prepare_start_inputs(&mut req, &mut ctx);
        let (language, provisional_adapter) =
            preflight_debug_request(&self.data_dir, &req, &ctx, Some(&self.environments))?;
        let entry = self.register_entry(
            &mut req,
            ctx.clone(),
            language,
            provisional_adapter,
            Vec::new(),
        );
        let my_gen = entry.boot_generation.load(Ordering::SeqCst);
        let rx = entry.events_tx.subscribe();
        let initial_snap = entry.read_snapshot();

        let data_dir = self.data_dir.clone();
        let environments = self.environments.clone();
        let sessions = Arc::clone(&self.sessions);
        tokio::spawn(async move {
            let inherit_shell = ctx
                .project_env
                .as_ref()
                .map(|p| p.inherit_shell_env)
                .unwrap_or(true)
                && ctx
                    .run_config
                    .as_ref()
                    .map(|c| c.inherit_shell_env)
                    .unwrap_or(true)
                && req.launch.inherit_shell_env.unwrap_or(true);
            if ctx.caller_is_root
                && req.target.kind == IdeTargetKind::Local
                && inherit_shell
                && environments.shell_env_resolver().is_cold_or_warming()
            {
                let shell_vars = environments.shell_env_resolver().vars_or_load().await;
                EnvironmentManager::merge_late_shell_vars(&mut ctx.effective_env, shell_vars);
            }
            if entry.boot_generation.load(Ordering::SeqCst) != my_gen || entry.is_terminal() {
                return;
            }
            // Stop while the target is still compiling drops the build (and
            // kills its process group) instead of letting it run to the end.
            let resolve = otto_telemetry::context::measure_result(
                "ide.dap.resolve_adapter",
                resolve_dap_adapter_with_manager(&data_dir, &req, &ctx, Some(&environments)),
            );
            let Some(resolved) = entry.unless_superseded(my_gen, resolve).await else {
                return;
            };
            let resolved = match resolved {
                Ok(r) => r,
                Err(err) => {
                    let _guard = entry.lifecycle_lock.lock().await;
                    if entry.boot_generation.load(Ordering::SeqCst) == my_gen
                        && !entry.is_terminal()
                    {
                        entry.fail_start(&sessions, &err);
                    }
                    return;
                }
            };
            let _guard = entry.lifecycle_lock.lock().await;
            if entry.boot_generation.load(Ordering::SeqCst) != my_gen || entry.is_terminal() {
                return;
            }
            for item in &resolved.initial_output {
                entry.append_output(&item.category, item.text.clone());
            }
            entry.mutate_and_broadcast(|snap| {
                snap.language = resolved.language.clone();
                snap.adapter = resolved.adapter_name.clone();
            });
            // The lifecycle lock is held from here: Stop supersedes the boot so
            // it does not wait out a 15 s port or `initialize` timeout for it.
            let boot = otto_telemetry::context::measure_result(
                "ide.dap.handshake",
                boot_session_connection(&sessions, &entry, resolved, &req, &ctx),
            );
            if let Some(Err(err)) = entry.unless_superseded(my_gen, boot).await {
                if entry.boot_generation.load(Ordering::SeqCst) == my_gen && !entry.is_terminal() {
                    entry.fail_start(&sessions, &err);
                }
            }
        });

        Ok((initial_snap, rx))
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
            bp.path = display_target_path(&target, &bp.path);
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
            IdeDebugAction::Restart => return self.restart_entry(&entry).await,
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

    pub fn active_local_pids(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for item in self.sessions.iter() {
            let is_local = item
                .value()
                .orig_req
                .read()
                .map(|r| r.target.kind == crate::types::IdeTargetKind::Local)
                .unwrap_or(false);
            if !is_local {
                continue;
            }
            if let Ok(conn_guard) = item.value().conn.read() {
                if let Some(conn) = conn_guard.as_ref() {
                    out.extend(conn.guard.pids());
                }
            }
        }
        out
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_security_tests.rs"]
mod security_tests;

#[cfg(test)]
#[path = "session_round5_tests.rs"]
mod round5_tests;

#[cfg(test)]
#[path = "session_round6_tests.rs"]
mod round6_tests;
