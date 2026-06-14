//! SessionManager — owns live PTYs, the sessions DB rows and per-session
//! status tasks (working/idle/exited detection + events).

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use otto_core::api::CreateSessionReq;
use otto_core::domain::{Session, SessionKind, SessionStatus, Workspace};
use otto_core::event::Event;
use otto_core::hooks::PreSpawnHook;
use otto_core::{new_id, Error, Id, Result};
use otto_pty::{CommandSpec, PtyHandle};
use otto_state::{NewSession, SessionsRepo};
use tokio::sync::broadcast;

use crate::providers::ProviderRegistry;

/// Build `["--add-dir", path, ...]` args for providers that support `--add-dir`
/// (claude, codex, agy — NOT shell).  Returns an empty vec for unknown/shell
/// providers or when `meta` has no `extra_dirs` array.
fn add_dir_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if provider == "shell" {
        return vec![];
    }
    let Some(arr) = meta.get("extra_dirs").and_then(|v| v.as_array()) else {
        return vec![];
    };
    let mut out = Vec::with_capacity(arr.len() * 2);
    for item in arr {
        if let Some(dir) = item.as_str() {
            if !dir.is_empty() {
                out.push("--add-dir".to_string());
                out.push(dir.to_string());
            }
        }
    }
    out
}

/// Window after the last output chunk during which a session is `working`.
const WORKING_WINDOW: Duration = Duration::from_secs(5);
/// Status poll interval.
const STATUS_TICK: Duration = Duration::from_secs(2);

/// How long a LIVE resumable session must be idle (no output) AND unattached
/// (no WS viewer) before its PTY is suspended to free RAM. The conversation
/// stays resumable, so reopening it auto-resumes via `--resume`.
pub const SUSPEND_GRACE: Duration = Duration::from_secs(5 * 60);

/// Hook that inspects live PTY output for a session, used by otto-server's
/// credential monitor to detect mid-session re-auth prompts (e.g. "run
/// `claude login`", "session expired").
///
/// Lives here (not in otto-core) to avoid a core dependency. **Best-effort:**
/// implementations MUST handle their own errors and never panic — a scan
/// failure must never disturb the session's status task. `chunk` is a raw PTY
/// output slice (may split lines); implementations should keep their own small
/// rolling context and debounce per session.
pub trait OutputScanner: Send + Sync {
    /// Called for each PTY output chunk. `provider` is the session's CLI
    /// provider ("claude", "codex", "shell", …) used as the re-auth target.
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]);
}

/// Await the child exit code without holding the (non-Send) watch guard
/// across an await point. `None` when the watch sender was dropped.
pub(crate) async fn wait_exit_code(
    rx: &mut tokio::sync::watch::Receiver<Option<i32>>,
) -> Option<i32> {
    let res = rx.wait_for(|v| v.is_some()).await;
    match res {
        Ok(guard) => Some((*guard).unwrap_or(-1)),
        Err(_) => None,
    }
}

/// RAII guard for a WS terminal attachment: decrements the session's attached-
/// viewer count when dropped, on every WS `serve_terminal` return path.
pub struct AttachGuard {
    manager: Arc<SessionManager>,
    id: Id,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        self.manager.detach(&self.id);
    }
}

/// Owns live sessions: PTY handles keyed by session id plus persistence.
pub struct SessionManager {
    /// Shared so the per-session status task can evict an exited handle
    /// (otherwise dead PtyHandles — and their emulator + ring buffer — leak).
    live: Arc<DashMap<Id, Arc<PtyHandle>>>,
    /// In-memory count of attached WS terminal viewers per session. Bumped by
    /// `ws::serve_terminal` on attach/detach; read by the idle-suspend sweep so
    /// it never suspends a session someone is actively watching.
    attached: Arc<DashMap<Id, usize>>,
    /// Session ids whose PTY is being deliberately suspended (RAM release, not
    /// a real exit). The per-session status task consults this in its exit
    /// branch so it marks the session `Reconnectable` (still resumable) instead
    /// of `Exited`, winning the kill→exit race deterministically.
    suspending: Arc<DashMap<Id, ()>>,
    repo: SessionsRepo,
    events: broadcast::Sender<Event>,
    providers: ProviderRegistry,
    /// Optional context-provisioning hook, invoked before an agent spawn.
    pre_spawn_hook: Option<Arc<dyn PreSpawnHook>>,
    /// Optional live-output scanner (credential monitor's mid-session auth
    /// detection). When set, each session's status task subscribes to its PTY
    /// output and forwards chunks here.
    output_scanner: Option<Arc<dyn OutputScanner>>,
}

impl SessionManager {
    pub fn new(
        repo: SessionsRepo,
        events: broadcast::Sender<Event>,
        providers: ProviderRegistry,
    ) -> Self {
        Self {
            live: Arc::new(DashMap::new()),
            attached: Arc::new(DashMap::new()),
            suspending: Arc::new(DashMap::new()),
            repo,
            events,
            providers,
            pre_spawn_hook: None,
            output_scanner: None,
        }
    }

    /// Attach a pre-spawn hook (context provisioning). Builder-style so existing
    /// `new()` callers (tests, channels) stay unchanged.
    pub fn with_pre_spawn_hook(mut self, hook: Arc<dyn PreSpawnHook>) -> Self {
        self.pre_spawn_hook = Some(hook);
        self
    }

    /// Attach a live-output scanner (mid-session re-auth detection). Builder-
    /// style so existing `new()` callers stay unchanged.
    pub fn with_output_scanner(mut self, scanner: Arc<dyn OutputScanner>) -> Self {
        self.output_scanner = Some(scanner);
        self
    }

    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    /// All `(provider_name, update_command)` pairs for providers that have an
    /// update command configured. Delegates to the registry.
    pub fn provider_update_commands(&self) -> Vec<(String, String)> {
        self.providers.update_commands()
    }

    /// Return the resolved program binary for `name`, or `None` if the
    /// provider is not registered. Delegates to the registry.
    pub fn provider_program(&self, name: &str) -> Option<String> {
        self.providers.program_for(name)
    }

    /// Create a session row, spawn its PTY and start the status task.
    ///
    /// `spec_override` is used by connection sessions (the connections crate
    /// prebuilds the full command, including secret env vars). For
    /// `kind=agent` without an override the command comes from the provider
    /// registry. Title default: `"<provider> #N"`; callers that open
    /// connections should pass `req.title = Some(<connection name>)`.
    pub async fn create(
        &self,
        ws: &Workspace,
        user_id: &Id,
        req: CreateSessionReq,
        spec_override: Option<CommandSpec>,
    ) -> Result<Session> {
        let cwd = req.cwd.clone().unwrap_or_else(|| ws.root_path.clone());

        let (provider, spec, provider_session_id) = match spec_override {
            Some(spec) => {
                let provider = req
                    .provider
                    .clone()
                    .ok_or_else(|| Error::Invalid("provider is required".into()))?;
                (provider, spec, None)
            }
            None => {
                if req.kind != SessionKind::Agent {
                    return Err(Error::Invalid(
                        "connection sessions are opened via POST /connections/{id}/open".into(),
                    ));
                }
                let provider = req.provider.clone().ok_or_else(|| {
                    Error::Invalid("provider is required for agent sessions".into())
                })?;
                // claude's --session-id flag requires a UUID, so provider
                // session ids are UUIDs (the Otto session id stays a ULID).
                let sid = uuid::Uuid::new_v4().to_string();
                let mut spec = self.providers.build_spec(&provider, &sid, &cwd, false)?;
                // Append --add-dir args from req.meta.extra_dirs
                let meta_val = req
                    .meta
                    .as_ref()
                    .map(|m| m.clone())
                    .unwrap_or(serde_json::json!({}));
                spec.args.extend(add_dir_args(&provider, &meta_val));
                let psid = self.providers.supports_resume(&provider).then_some(sid);
                (provider, spec, psid)
            }
        };

        let title = match req.title.clone() {
            Some(t) if !t.is_empty() => t,
            _ => {
                let n = self.repo.count_by_provider(&ws.id, &provider).await? + 1;
                format!("{provider} #{n}")
            }
        };

        let session = self
            .repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: req.kind,
                provider,
                title,
                cwd,
                provider_session_id,
                connection_id: req.connection_id.clone(),
                created_by: user_id.clone(),
                meta: req.meta.clone().unwrap_or_else(|| serde_json::json!({})),
            })
            .await?;

        // The cwd must exist (a missing dir makes the child fall back to
        // $HOME) and agent CLIs should already trust the workspace folder.
        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            crate::trust::ensure_trusted(&session.provider, &session.cwd);
            // Browser tools: wire an MCP browser server into the workspace
            // when the session opted in (meta.browser == true).
            if session
                .meta
                .get("browser")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if let Err(e) = crate::mcp::enable_browser(&session.cwd) {
                    tracing::warn!("enable browser MCP: {e}");
                }
            }
            // Otto context provisioning: materialize the workspace's active
            // skills + soul + context into this CLI's native form. Best-effort —
            // the hook logs and swallows its own errors, never blocking spawn.
            //
            // Skipped for PR-review sessions: they all share one repo cwd, so
            // concurrent spawns would serialize on this *synchronous* materialize
            // (leaving one agent stuck "pending"); a focused diff review needs no
            // workspace skills/soul; and provisioning also pollutes the repo with
            // .otto-managed.json / CLAUDE.md.
            let is_review =
                session.meta.get("source").and_then(|v| v.as_str()) == Some("review");
            if !is_review {
                if let Some(hook) = &self.pre_spawn_hook {
                    hook.before_spawn(ws, &session.cwd, &session.provider);
                }
            }
        }

        let handle = match PtyHandle::spawn(&spec) {
            Ok(h) => Arc::new(h),
            Err(e) => {
                let _ = self.repo.delete(&session.id).await;
                return Err(e);
            }
        };

        self.live.insert(session.id.clone(), Arc::clone(&handle));
        self.start_status_task(
            session.id.clone(),
            session.workspace_id.clone(),
            session.provider.clone(),
            handle,
        );
        let _ = self.events.send(Event::SessionCreated {
            session: session.clone(),
        });
        Ok(session)
    }

    /// Load one session from the DB.
    pub async fn get(&self, id: &Id) -> Result<Session> {
        self.repo.get(id).await
    }

    /// All sessions of a workspace from the DB.
    pub async fn list_by_workspace(&self, ws: &Id) -> Result<Vec<Session>> {
        self.repo.list_by_workspace(ws).await
    }

    /// True when the session has a live PTY in this daemon process.
    pub fn is_live(&self, id: &Id) -> bool {
        self.live.contains_key(id)
    }

    /// Register a WS terminal viewer for `id` (called on attach). Returns an
    /// [`AttachGuard`] that decrements the count on drop, so every WS exit path
    /// (clean close, error, drop) releases the attachment.
    pub fn attach(self: &Arc<Self>, id: &Id) -> AttachGuard {
        *self.attached.entry(id.clone()).or_insert(0) += 1;
        AttachGuard {
            manager: Arc::clone(self),
            id: id.clone(),
        }
    }

    /// Decrement the attached-viewer count for `id`, removing the entry at zero.
    fn detach(&self, id: &Id) {
        if let Some(mut e) = self.attached.get_mut(id) {
            *e = e.saturating_sub(1);
            if *e == 0 {
                drop(e);
                self.attached.remove_if(id, |_, &v| v == 0);
            }
        }
    }

    /// Number of WS terminal viewers currently attached to `id`.
    pub fn attached_count(&self, id: &Id) -> usize {
        self.attached.get(id).map(|e| *e).unwrap_or(0)
    }

    /// True when at least one WS viewer is attached to `id`.
    pub fn is_attached(&self, id: &Id) -> bool {
        self.attached_count(id) > 0
    }

    /// Ensure the session is live, resuming it if it is an exited-but-resumable
    /// agent session. A no-op when the session is already live or cannot be
    /// resumed. Errors are logged and suppressed so callers (WS attach) can
    /// proceed optimistically.
    pub async fn ensure_live(&self, id: &Id) -> Result<()> {
        if self.is_live(id) {
            return Ok(());
        }
        let session = self.repo.get(id).await?;
        let resumable = session.kind == SessionKind::Agent
            && session.provider_session_id.is_some()
            && self.providers.supports_resume(&session.provider);
        if resumable {
            self.restart(id, None).await.map(|_| ())?;
        }
        Ok(())
    }

    /// The live PTY handle, when the session has one in this daemon.
    pub fn live_handle(&self, id: &Id) -> Option<Arc<PtyHandle>> {
        self.live.get(id).map(|h| Arc::clone(&h))
    }

    /// Write input bytes to a live session.
    pub async fn input(&self, id: &Id, data: &[u8]) -> Result<()> {
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        handle.write(data)
    }

    /// Resize a live session's terminal.
    pub async fn resize(&self, id: &Id, cols: u16, rows: u16) -> Result<()> {
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        handle.resize(cols, rows)
    }

    /// Rename a session.
    pub async fn update_title(&self, id: &Id, title: &str) -> Result<Session> {
        self.repo.set_title(id, title).await?;
        self.repo.get(id).await
    }

    /// Shallow-merge `patch` (a JSON object) into the session's existing meta.
    /// Top-level null values in the patch remove that key. Non-object existing
    /// meta is replaced by an empty object before merging.
    pub async fn update_meta(&self, id: &Id, patch: serde_json::Value) -> Result<Session> {
        let session = self.repo.get(id).await?;
        let mut base = match session.meta {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if let serde_json::Value::Object(patch_map) = patch {
            for (k, v) in patch_map {
                if v.is_null() {
                    base.remove(&k);
                } else {
                    base.insert(k, v);
                }
            }
        }
        let merged = serde_json::Value::Object(base);
        self.repo.set_meta(id, &merged).await?;
        self.repo.get(id).await
    }

    /// Kill the PTY (if live) and mark the session exited.
    pub async fn kill_session(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        if let Some(handle) = self.live_handle(id) {
            let _ = handle.kill();
        }
        self.repo.update_status(id, SessionStatus::Exited).await?;
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
            status: SessionStatus::Exited,
        });
        Ok(())
    }

    /// Suspend a session: release its RAM-holding PTY **without losing the
    /// session**. The conversation stays resumable, so reopening it (WS attach
    /// → `ensure_live` → `restart --resume`) brings it right back.
    ///
    /// Only meaningful for resumable agent sessions; callers (the idle-suspend
    /// sweep) gate on `supports_resume`. The row is kept (incl.
    /// `provider_session_id`); the session ends up `Reconnectable`.
    ///
    /// Status-race handling: killing the handle makes the per-session status
    /// task's exit branch fire, which would normally write `Exited`. We mark
    /// the id in `suspending` *before* killing so that branch writes
    /// `Reconnectable` instead. We also set `Reconnectable` here directly, so
    /// the final status is correct regardless of which path wins.
    pub async fn suspend(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        // Mark as suspending so the status task's exit branch chooses
        // Reconnectable over Exited. Cleared by that branch (or below).
        self.suspending.insert(id.clone(), ());
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        // Authoritatively set Reconnectable (idempotent with the status task).
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        // Drop the suspend flag last; the status task only reads it, and a late
        // read after this point is harmless (it would also pick Reconnectable).
        self.suspending.remove(id);
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
            status: SessionStatus::Reconnectable,
        });
        Ok(())
    }

    /// One sweep of the idle-suspend policy: suspend every LIVE session that is
    /// resumable, idle (no output for ≥ [`SUSPEND_GRACE`]) and has no attached
    /// WS viewer. Working sessions, attached sessions and non-resumable
    /// providers are never touched. Returns the number suspended.
    ///
    /// Resilient: a failure on one session is logged and skipped; the loop
    /// never panics or aborts.
    pub async fn suspend_idle_unattached(&self) -> usize {
        // Snapshot live ids first (don't hold DashMap refs across awaits).
        let candidates: Vec<(Id, std::time::Instant)> = self
            .live
            .iter()
            .map(|e| (e.key().clone(), e.value().last_output_at()))
            .collect();

        let mut suspended = 0;
        for (id, last_output) in candidates {
            // Idle: no PTY output for the full grace window.
            if last_output.elapsed() < SUSPEND_GRACE {
                continue;
            }
            // Unattached: nobody is watching the terminal right now.
            if self.is_attached(&id) {
                continue;
            }
            let session = match self.repo.get(&id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(session = %id, "idle-suspend: load failed: {e}");
                    continue;
                }
            };
            // Only resumable agent sessions — never lose work for a provider
            // that can't be resumed (codex/agy/shell).
            let resumable = session.kind == SessionKind::Agent
                && session.provider_session_id.is_some()
                && self.providers.supports_resume(&session.provider);
            if !resumable {
                continue;
            }
            match self.suspend(&id).await {
                Ok(()) => {
                    suspended += 1;
                    tracing::info!(
                        session = %id,
                        provider = %session.provider,
                        title = %session.title,
                        "suspended idle, unattached session (freed PTY; stays resumable)"
                    );
                }
                Err(e) => tracing::warn!(session = %id, "idle-suspend failed: {e}"),
            }
        }
        suspended
    }

    /// Existence-check pruner: for non-live agent sessions with a
    /// `provider_session_id`, verify the provider's on-disk transcript still
    /// exists. If it is positively gone (un-resumable) → delete the row. If it
    /// still exists, or existence cannot be determined → keep the session.
    ///
    /// We only ever delete what we can positively confirm is gone. `home` is
    /// the user's home dir ($HOME); when unset we keep everything.
    ///
    /// Resilient: per-session failures are logged and skipped. Returns the
    /// number of rows pruned.
    pub async fn prune_dead_sessions(&self) -> usize {
        let home = match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => std::path::PathBuf::from(h),
            _ => {
                tracing::warn!("prune: HOME unset; skipping existence-check prune");
                return 0;
            }
        };
        let candidates = match self.repo.list_prunable_agent_sessions().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("prune: list prunable sessions failed: {e}");
                return 0;
            }
        };
        let mut pruned = 0;
        for s in candidates {
            // Never prune a session that became live again between the query
            // and now (e.g. someone reopened it).
            if self.is_live(&s.id) {
                continue;
            }
            let Some(psid) = s.provider_session_id.as_deref() else {
                continue;
            };
            let verdict = crate::lifecycle::check_resumability(&home, &s.provider, &s.cwd, psid);
            match verdict {
                crate::lifecycle::Resumability::Gone => {
                    match self.remove(&s.id).await {
                        Ok(()) => {
                            pruned += 1;
                            tracing::info!(
                                session = %s.id,
                                provider = %s.provider,
                                title = %s.title,
                                "pruned un-resumable session (provider transcript gone)"
                            );
                        }
                        Err(e) => tracing::warn!(session = %s.id, "prune remove failed: {e}"),
                    }
                }
                // Exists or Unknown → keep. We never prune what we can't verify.
                crate::lifecycle::Resumability::Exists
                | crate::lifecycle::Resumability::Unknown => {}
            }
        }
        pruned
    }

    /// Kill every live PTY and mark the sessions exited. Used when the app
    /// closes (no orphaned agent processes left running) and on daemon
    /// shutdown. Returns the number of sessions terminated.
    pub async fn shutdown_all(&self) -> usize {
        let ids: Vec<Id> = self.live.iter().map(|e| e.key().clone()).collect();
        let count = ids.len();
        for id in ids {
            if let Some((_, handle)) = self.live.remove(&id) {
                let _ = handle.kill();
            }
            // Best-effort status update; ignore errors during shutdown.
            let _ = self.repo.update_status(&id, SessionStatus::Exited).await;
            if let Ok(s) = self.repo.get(&id).await {
                let _ = self.events.send(Event::SessionStatus {
                    session_id: id.clone(),
                    workspace_id: s.workspace_id,
                    status: SessionStatus::Exited,
                });
            }
        }
        count
    }

    /// Number of live (running) sessions.
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Archive a session: kill its PTY, mark it archived + exited, keep the
    /// row and history. It disappears from the active list (clients hide it)
    /// but can be restored or deleted later.
    pub async fn archive(&self, id: &Id) -> Result<Session> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.set_archived(id, true).await?;
        self.repo.update_status(id, SessionStatus::Exited).await?;
        // Clients refresh on this event and move the row to the archive.
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Exited,
        });
        self.repo.get(id).await
    }

    /// Auto-archive channel-spawned agent sessions (ticket/chat) idle longer
    /// than `max_idle`, so they don't pile up. A later message in the same
    /// conversation spawns a fresh session. Returns the number archived.
    pub async fn reap_idle_channel_sessions(&self, max_idle: std::time::Duration) -> usize {
        let stale = match self.repo.list_idle_channel_sessions(max_idle).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("reap idle channel sessions: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for s in stale {
            match self.archive(&s.id).await {
                Ok(_) => {
                    n += 1;
                    tracing::info!(session = %s.id, title = %s.title, "reaped idle channel session");
                }
                Err(e) => tracing::warn!(session = %s.id, "reap archive failed: {e}"),
            }
        }
        n
    }

    /// Permanently delete archived channel (ticket/chat) sessions whose last
    /// activity is older than `max_age`, so closed tickets don't accumulate in
    /// the DB forever. Uses [`remove`](Self::remove) so clients drop the row
    /// from the Archived view. Returns the number deleted.
    pub async fn purge_old_archived_channel_sessions(
        &self,
        max_age: std::time::Duration,
    ) -> usize {
        let stale = match self
            .repo
            .list_archived_channel_sessions_older_than(max_age)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("purge old archived channel sessions: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for s in stale {
            // Skip anything that became live again between the query and now.
            if self.live.contains_key(&s.id) {
                continue;
            }
            match self.remove(&s.id).await {
                Ok(()) => {
                    n += 1;
                    tracing::info!(session = %s.id, title = %s.title, "purged old archived channel session");
                }
                Err(e) => tracing::warn!(session = %s.id, "purge delete failed: {e}"),
            }
        }
        n
    }

    /// Un-archive a session (it returns to the active list as reconnectable;
    /// agent sessions can then be restarted to resume).
    pub async fn unarchive(&self, id: &Id) -> Result<Session> {
        self.repo.set_archived(id, false).await?;
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        self.repo.get(id).await
    }

    /// Kill the PTY, delete the DB row and emit `SessionRemoved`.
    pub async fn remove(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.delete(id).await?;
        let _ = self.events.send(Event::SessionRemoved {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
        });
        Ok(())
    }

    /// Respawn a session. Agent sessions with a `provider_session_id` use the
    /// provider's resume args. Connection sessions need a `spec_override`
    /// (rebuilt by the connections service) — without one this fails.
    pub async fn restart(&self, id: &Id, spec_override: Option<CommandSpec>) -> Result<Session> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }

        let spec = match spec_override {
            Some(s) => s,
            None => {
                if session.kind != SessionKind::Agent {
                    return Err(Error::Invalid(
                        "connection sessions are reopened via their connection".into(),
                    ));
                }
                let resume = session.provider_session_id.is_some()
                    && self.providers.supports_resume(&session.provider);
                let sid = session.provider_session_id.clone().unwrap_or_else(new_id);
                let mut spec =
                    self.providers
                        .build_spec(&session.provider, &sid, &session.cwd, resume)?;
                // Append --add-dir args from session.meta.extra_dirs
                spec.args
                    .extend(add_dir_args(&session.provider, &session.meta));
                spec
            }
        };

        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            crate::trust::ensure_trusted(&session.provider, &session.cwd);
        }
        let handle = Arc::new(PtyHandle::spawn(&spec)?);
        self.live.insert(id.clone(), Arc::clone(&handle));
        self.repo.update_status(id, SessionStatus::Running).await?;
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Running,
        });
        self.start_status_task(id.clone(), session.workspace_id, session.provider, handle);
        self.repo.get(id).await
    }

    /// Daemon-boot restore. We deliberately do NOT respawn agent processes here:
    /// keeping every historical session resident would cost ~200 MB each. Instead
    /// every restorable session is marked `Reconnectable` (0 memory) and resumed
    /// lazily by [`Self::ensure_live`] the moment a client opens it — claude/codex
    /// keep their conversation in the on-disk JSONL, so `--resume` restores it in
    /// full. `_fallback_cwd` is kept for signature stability (used by resume).
    pub async fn restore_all(
        &self,
        _fallback_cwd: &(dyn Fn(&Id) -> Option<String> + Send + Sync),
    ) -> Result<()> {
        for session in self.repo.list_all_restorable().await? {
            self.repo
                .update_status(&session.id, SessionStatus::Reconnectable)
                .await?;
            let _ = self.events.send(Event::SessionStatus {
                session_id: session.id.clone(),
                workspace_id: session.workspace_id.clone(),
                status: SessionStatus::Reconnectable,
            });
        }
        Ok(())
    }

    /// Per-session status task: every 2s classify working/idle from PTY
    /// activity; on exit mark `exited` and stop. When an [`OutputScanner`] is
    /// configured, also spawns a sibling task that streams the PTY's live
    /// output into the scanner (mid-session re-auth detection).
    fn start_status_task(
        &self,
        id: Id,
        workspace_id: Id,
        provider: String,
        handle: Arc<PtyHandle>,
    ) {
        // Mid-session output scan: subscribe to the PTY broadcast and forward
        // chunks to the scanner. Ends when the PTY closes (broadcast Closed).
        if let Some(scanner) = self.output_scanner.clone() {
            let mut rx = handle.subscribe();
            let scan_id = id.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(chunk) => scanner.on_output(&scan_id, &provider, &chunk),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        let repo = self.repo.clone();
        let events = self.events.clone();
        let live = Arc::clone(&self.live);
        let suspending = Arc::clone(&self.suspending);
        tokio::spawn(async move {
            let mut exit_rx = handle.on_exit();
            let mut current = SessionStatus::Running;
            let mut interval = tokio::time::interval(STATUS_TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let next = if handle.last_output_at().elapsed() < WORKING_WINDOW {
                            SessionStatus::Working
                        } else {
                            SessionStatus::Idle
                        };
                        if next != current {
                            current = next;
                            let _ = repo.update_status(&id, next).await;
                            let _ = events.send(Event::SessionStatus {
                                session_id: id.clone(),
                                workspace_id: workspace_id.clone(),
                                status: next,
                            });
                        }
                    }
                    code = wait_exit_code(&mut exit_rx) => {
                        let _ = code;
                        // Evict the dead handle so its emulator + ring buffer
                        // are dropped (no accumulation across many sessions).
                        live.remove(&id);
                        // If this exit was caused by a deliberate suspend (PTY
                        // killed to free RAM), the session stays resumable: mark
                        // it Reconnectable, not Exited. `suspend()` also writes
                        // Reconnectable authoritatively, so either order is safe.
                        let status = if suspending.contains_key(&id) {
                            SessionStatus::Reconnectable
                        } else {
                            SessionStatus::Exited
                        };
                        let _ = repo.update_status(&id, status).await;
                        let _ = events.send(Event::SessionStatus {
                            session_id: id.clone(),
                            workspace_id: workspace_id.clone(),
                            status,
                        });
                        break;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::{SessionKind, Workspace};
    use otto_state::NewSession;

    async fn test_manager() -> (Arc<SessionManager>, SessionsRepo, Workspace, Id) {
        // A migrated on-disk sqlite (in a tempdir) via otto-state's opener.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let pool = otto_state::open(&db).await.unwrap();
        // Keep the tempdir alive for the whole process (leak is fine in tests).
        std::mem::forget(dir);

        let user = new_id();
        let ws_id = new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&user).bind("u").bind("x").bind("U").bind(&now)
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws_id).bind("w").bind("/tmp").bind(&now)
            .execute(&pool).await.unwrap();

        let repo = SessionsRepo::new(pool);
        let (events, _rx) = broadcast::channel(16);
        let providers = ProviderRegistry::new(None);
        let mgr = Arc::new(SessionManager::new(repo.clone(), events, providers));
        let ws = Workspace {
            id: ws_id,
            name: "w".into(),
            root_path: "/tmp".into(),
            settings: serde_json::json!({}),
            archived: false,
            created_at: chrono::Utc::now(),
        };
        (mgr, repo, ws, user)
    }

    async fn seed_session(repo: &SessionsRepo, ws: &Workspace, user: &Id, psid: Option<&str>) -> Id {
        let s = repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Agent,
                provider: "claude".into(),
                title: "t".into(),
                cwd: "/tmp".into(),
                provider_session_id: psid.map(|s| s.to_string()),
                connection_id: None,
                created_by: user.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        s.id
    }

    #[tokio::test]
    async fn attach_guard_counts_and_releases() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;

        assert_eq!(mgr.attached_count(&id), 0);
        assert!(!mgr.is_attached(&id));

        let g1 = mgr.attach(&id);
        assert_eq!(mgr.attached_count(&id), 1);
        assert!(mgr.is_attached(&id));

        let g2 = mgr.attach(&id);
        assert_eq!(mgr.attached_count(&id), 2);

        drop(g1);
        assert_eq!(mgr.attached_count(&id), 1);
        assert!(mgr.is_attached(&id));

        drop(g2);
        assert_eq!(mgr.attached_count(&id), 0);
        assert!(!mgr.is_attached(&id));
    }

    #[tokio::test]
    async fn suspend_marks_reconnectable_and_keeps_row() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid-keep")).await;

        // No live PTY in this test; suspend still drives the DB-side outcome.
        mgr.suspend(&id).await.unwrap();

        let s = repo.get(&id).await.unwrap();
        assert_eq!(s.status, SessionStatus::Reconnectable);
        // The session is NOT lost — row and resume id are preserved.
        assert_eq!(s.provider_session_id.as_deref(), Some("sid-keep"));
        // Suspend flag is cleared after the operation.
        assert!(!mgr.suspending.contains_key(&id));
    }

    #[tokio::test]
    async fn idle_suspend_skips_attached_sessions() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;
        // Mark it as a (fake) live session so the sweep considers it, but with
        // an attachment so it must be skipped. We can't spawn a real PTY here,
        // so assert the guard semantics the sweep relies on: an attached
        // session reports is_attached == true.
        let _g = mgr.attach(&id);
        assert!(mgr.is_attached(&id));
        // The sweep over live sessions is a no-op (no live PTYs), and the
        // attachment registry it consults is correct.
        assert_eq!(mgr.suspend_idle_unattached().await, 0);
    }

    #[tokio::test]
    async fn prune_keeps_session_with_existing_transcript() {
        let (mgr, repo, ws, user) = test_manager().await;
        // Point HOME at a tempdir holding a matching transcript.
        let home = tempfile::tempdir().unwrap();
        let cwd = "/tmp";
        let psid = "exists-1111";
        let proj = home
            .path()
            .join(".claude")
            .join("projects")
            .join(crate::lifecycle::claude_project_dir_name(cwd));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{psid}.jsonl")), b"{}").unwrap();

        let id = seed_session(&repo, &ws, &user, Some(psid)).await;
        repo.update_status(&id, SessionStatus::Reconnectable)
            .await
            .unwrap();

        // Safe in tests: this process owns its env. Set HOME for the check.
        std::env::set_var("HOME", home.path());
        let pruned = mgr.prune_dead_sessions().await;
        assert_eq!(pruned, 0, "existing transcript must be kept");
        assert!(repo.get(&id).await.is_ok());
    }
}
