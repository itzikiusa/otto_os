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

use super::adapters::{normalize_target_path, resolve_dap_adapter_with_manager};
pub(super) use super::session_entry::{
    ActiveConnection, BoxRead, BoxWrite, DapWireClient, SessionEntry,
};
pub use super::session_inline::compute_inline_values;
pub(super) use super::session_inline::mark_resumed;
use super::session_inline::{build_session_title, stop_entry};
use super::session_inspect::{select_frame_and_refresh, sync_all_breakpoints};
use super::session_transport::{
    boot_session_connection, evict_terminated_sessions, handshake_and_run,
};
use super::BridgeGuard;
use crate::environments::EnvironmentManager;
use crate::types::{
    IdeDebugAction, IdeDebugCommandReq, IdeDebugOutputEntry, IdeDebugSessionSnapshot,
    IdeProjectEnvironment, IdeRunConfig, IdeSetBreakpointsReq, IdeSshSpec, IdeStartDebugReq,
    IdeTargetSpec, PluginDapAdapterSpec,
};

pub(crate) const MAX_DEBUG_OUTPUT: usize = 400;
pub(crate) const MAX_DEBUG_OUTPUT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_TERMINATED_DEBUG_SESSIONS: usize = 4;

/// Optional host/environment context passed when starting a debug session.
#[derive(Debug, Clone, Default)]
pub struct DapStartContext {
    pub ssh: Option<IdeSshSpec>,
    pub effective_env: BTreeMap<String, String>,
    pub shell_env_snapshot: BTreeMap<String, String>,
    pub project_env: Option<IdeProjectEnvironment>,
    pub run_config: Option<IdeRunConfig>,
    pub plugin_adapters: Vec<PluginDapAdapterSpec>,
    pub caller_is_root: bool,
    pub owner_user_id: String,
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

    pub fn with_owner_user_id(mut self, user_id: impl Into<String>) -> Self {
        self.owner_user_id = user_id.into();
        self
    }
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

    /// List snapshots visible to `user` (all for root, owner-only otherwise).
    pub fn list_sessions_for_user(&self, user: &User) -> Vec<IdeDebugSessionSnapshot> {
        let mut out: Vec<IdeDebugSessionSnapshot> = self
            .sessions
            .iter()
            .filter(|item| user.is_root || item.value().owner_user_id == user.id.as_str())
            .map(|item| item.value().read_snapshot())
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        out
    }

    /// Filter an arbitrary snapshot slice down to sessions owned by `user`.
    pub fn filter_sessions_for_user(
        &self,
        user: &User,
        sessions: Vec<IdeDebugSessionSnapshot>,
    ) -> Vec<IdeDebugSessionSnapshot> {
        if user.is_root {
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
        if user.is_root || entry.owner_user_id == user.id.as_str() {
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
        let resolved =
            resolve_dap_adapter_with_manager(&self.data_dir, &req, &ctx, Some(&self.environments))
                .await?;
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
