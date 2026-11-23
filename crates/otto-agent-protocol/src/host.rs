//! What the runtime needs from the daemon: the workspace, the PTY path's
//! safety pipeline, and the event bus. [`SessionManagerHost`] is the
//! production implementation; tests supply their own.

use std::sync::Arc;

use otto_core::agent_runtime::{RuntimeAccessMode, RuntimeEvent, RuntimeProvider, RuntimeSession};
use otto_core::domain::{Session, SessionKind, SessionStatus, Workspace};
use otto_core::event::Event;
use otto_core::{Id, Result};
use otto_sessions::SessionManager;
use otto_state::WorkspacesRepo;

use crate::driver::{LaunchSpec, McpDelivery, McpServerDef};

#[async_trait::async_trait]
pub trait RuntimeHost: Send + Sync + 'static {
    async fn workspace(&self, id: &Id) -> Result<Workspace>;

    /// Whether the child will be Seatbelt-confined (drivers relax the
    /// provider's own nested sandbox accordingly).
    async fn sandboxed(&self, session: &RuntimeSession) -> bool;

    /// Apply the PTY sessions' safety pipeline to `spec` in place: cwd
    /// refusal, folder trust, MCP, context provisioning, Seatbelt wrap. An
    /// error refuses the spawn. Must move `spec.tail_args` behind the injected
    /// context flags (and leave `tail_args` empty). MCP follows `mcp`:
    /// `Launcher` applies it to `spec`; `Protocol` returns the servers for
    /// the driver's handshake.
    async fn prepare_spawn(
        &self,
        ws: &Workspace,
        session: &RuntimeSession,
        spec: &mut LaunchSpec,
        mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>>;

    /// The MCP servers the spawn just prepared for `session` gave its agent,
    /// and the enabled ones it could not take with why (its thread's server
    /// list). Empty by default (tests).
    fn mcp_status(
        &self,
        _session: &RuntimeSession,
    ) -> Vec<otto_core::agent_runtime::McpServerStatus> {
        Vec::new()
    }

    /// The child of `session` is gone (exited, stopped, closed): revoke what
    /// its spawn minted (the `otto` MCP credential). Best-effort.
    async fn retire(&self, _session: &RuntimeSession) {}

    /// Whether the spawn just prepared for `session` brought up a network
    /// profile. Its tunnels end with the daemon, so such a child is not
    /// held across a restart: it respawns (resumes) with fresh tunnels.
    fn network_profile_active(&self, _session: &RuntimeSession) -> bool {
        false
    }

    /// The access mode a new session starts in when the request names none —
    /// the default terminal sessions follow.
    fn default_access_mode(&self) -> RuntimeAccessMode {
        RuntimeAccessMode::ApprovalRequired
    }

    /// Resolve a custom provider slug (e.g. `"claudework"`) to its underlying
    /// structured [`RuntimeProvider`] when enabled in `ProviderRegistry`.
    fn resolve_custom_provider(&self, _slug: &str) -> Option<RuntimeProvider> {
        None
    }

    /// Whether `slug` (built-in or custom) is enabled in `ProviderRegistry`.
    /// Defaults to `true` for unit tests without a registry.
    fn is_provider_enabled(&self, _slug: &str) -> bool {
        true
    }

    /// Resolved custom `CLAUDE_CONFIG_DIR` for a Claude-family provider `slug`
    /// (including wrapper-script / `~/.zshrc` alias inference).
    fn claude_config_dir_for(&self, _slug: &str) -> Option<std::path::PathBuf> {
        None
    }

    /// Whether session persistence across daemon restarts is enabled.
    async fn session_persistence_enabled(&self) -> bool {
        true
    }

    /// Confine an agent-requested `cwd` for linked peer creation to
    /// `ws.root_path`, the calling thread's own `caller_cwd`, or (on
    /// `SessionManagerHost`) registered workspace repos / git worktrees.
    /// Returns `Error::Forbidden` for any path outside those roots.
    async fn check_agent_cwd(
        &self,
        ws: &Workspace,
        _caller_id: &Id,
        caller_cwd: &str,
        cwd: Option<&str>,
    ) -> Result<()> {
        check_agent_cwd_basic(ws, caller_cwd, cwd)
    }

    /// Deliver one event to WebSocket clients (owner / admin / root).
    fn publish(&self, session: &RuntimeSession, event: Arc<RuntimeEvent>);

    /// Thread `session` began waiting on its person (`Some(what)`: what its
    /// approval, question or secret request asks) or stopped waiting
    /// (`None`) — once per waiting episode (`runtime/attention.rs`; never
    /// for a background thread). Production raises / settles the "Agent
    /// needs attention" notice ([`crate::attention`]). Called under the
    /// session lock: must not block.
    fn attention(&self, _session: &RuntimeSession, _what: Option<&str>) {}
}

/// Baseline agent `cwd` confinement used by [`RuntimeHost::check_agent_cwd`]:
/// `cwd` must be absolute, exist, and canonicalize under `ws.root_path` or
/// `caller_cwd`.
pub(crate) fn check_agent_cwd_basic(
    ws: &Workspace,
    caller_cwd: &str,
    cwd: Option<&str>,
) -> Result<()> {
    let Some(raw) = cwd.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(());
    };
    let want = std::path::Path::new(raw);
    if !want.is_absolute() {
        return Err(otto_core::Error::Forbidden(
            "an agent may only open sessions at an absolute folder".into(),
        ));
    }
    let want = std::fs::canonicalize(want).map_err(|_| {
        otto_core::Error::Forbidden(format!("an agent may not open a session in {raw}"))
    })?;
    let mut bases = vec![std::path::PathBuf::from(&ws.root_path)];
    if !caller_cwd.trim().is_empty() {
        bases.push(std::path::PathBuf::from(caller_cwd));
    }
    let allowed = bases
        .iter()
        .filter_map(|b| std::fs::canonicalize(b).ok())
        .any(|b| want.starts_with(b));
    if allowed {
        Ok(())
    } else {
        Err(otto_core::Error::Forbidden(format!(
            "an agent may only open linked threads inside the workspace or its own folder (not {raw})"
        )))
    }
}

/// The synthetic PTY-shaped [`Session`] the safety helpers read (never
/// persisted to `sessions`). Its meta is `{"runtime": "structured"}` plus the
/// thread's server-owned [`RuntimeSession::meta`] — so a personal agent's
/// thread spawns confined exactly as its terminal session would (`read_only`
/// forces the Seatbelt profile and the read-only tool list, `browser` adds
/// the browser MCP, `personal_agent` binds its `otto` tools to the agent).
pub fn as_agent_session(s: &RuntimeSession) -> Session {
    let mut meta = serde_json::json!({ "runtime": "structured" });
    if let Some(serde_json::Value::Object(extra)) = &s.meta {
        for (k, v) in extra {
            if k != "runtime" {
                meta[k.as_str()] = v.clone();
            }
        }
    }
    Session {
        id: s.id.clone(),
        workspace_id: s.workspace_id.clone(),
        kind: SessionKind::Agent,
        provider: s.provider_slug().to_string(),
        title: s.title.clone(),
        status: SessionStatus::Running,
        cwd: s.cwd.clone(),
        provider_session_id: s.native_thread_id.clone(),
        connection_id: None,
        created_by: s.created_by.clone(),
        created_at: s.created_at,
        last_active_at: s.last_active_at,
        archived: false,
        meta,
    }
}

/// Production host: `SessionManager` for safety, the daemon bus for events.
pub struct SessionManagerHost {
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    /// Publishes on the WS streaming path (otto-server's `publish_stream`).
    pub publish: Box<dyn Fn(Event) + Send + Sync>,
    /// The "Agent needs attention" notices ([`RuntimeHost::attention`]).
    pub notices: crate::attention::AttentionNotices,
}

#[async_trait::async_trait]
impl RuntimeHost for SessionManagerHost {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        self.workspaces.get(id).await
    }

    async fn sandboxed(&self, session: &RuntimeSession) -> bool {
        self.manager
            .runtime_session_sandboxed(&as_agent_session(session))
            .await
    }

    async fn prepare_spawn(
        &self,
        ws: &Workspace,
        session: &RuntimeSession,
        spec: &mut LaunchSpec,
        mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        let mut cmd = otto_pty::CommandSpec {
            program: std::mem::take(&mut spec.program),
            args: std::mem::take(&mut spec.args),
            cwd: Some(session.cwd.clone()),
            env: std::mem::take(&mut spec.env),
        };
        let tail = std::mem::take(&mut spec.tail_args);
        let delivery = match mcp {
            McpDelivery::Launcher => otto_sessions::manager::RuntimeMcpDelivery::Launcher,
            McpDelivery::Protocol => otto_sessions::manager::RuntimeMcpDelivery::Protocol,
        };
        let res = self
            .manager
            .prepare_runtime_spawn(ws, &as_agent_session(session), &mut cmd, tail, delivery)
            .await;
        spec.program = cmd.program;
        spec.args = cmd.args;
        spec.env = cmd.env;
        res.map(|servers| {
            servers
                .into_iter()
                .map(|s| McpServerDef {
                    name: s.name,
                    command: s.command,
                    args: s.args,
                    env: s.env,
                })
                .collect()
        })
    }

    async fn retire(&self, session: &RuntimeSession) {
        self.manager
            .retire_runtime_credentials(&as_agent_session(session))
            .await;
    }

    fn mcp_status(
        &self,
        session: &RuntimeSession,
    ) -> Vec<otto_core::agent_runtime::McpServerStatus> {
        self.manager.runtime_mcp_status(&session.id)
    }

    fn network_profile_active(&self, session: &RuntimeSession) -> bool {
        self.manager
            .network_status(session.id.as_str())
            .profile_id
            .is_some()
    }

    /// Full access while Settings › Providers skips permission prompts (the
    /// default) — what a terminal session launches with.
    fn default_access_mode(&self) -> RuntimeAccessMode {
        if self.manager.providers().skip_permissions() {
            RuntimeAccessMode::FullAccess
        } else {
            RuntimeAccessMode::ApprovalRequired
        }
    }

    fn resolve_custom_provider(&self, slug: &str) -> Option<RuntimeProvider> {
        self.manager.providers().runtime_provider_for(slug)
    }

    fn is_provider_enabled(&self, slug: &str) -> bool {
        self.manager.providers().is_enabled(slug)
    }

    fn claude_config_dir_for(&self, slug: &str) -> Option<std::path::PathBuf> {
        self.manager.claude_config_dir_for(slug)
    }

    async fn session_persistence_enabled(&self) -> bool {
        self.manager.session_persistence_enabled().await
    }

    async fn check_agent_cwd(
        &self,
        ws: &Workspace,
        caller_id: &Id,
        caller_cwd: &str,
        cwd: Option<&str>,
    ) -> Result<()> {
        if check_agent_cwd_basic(ws, caller_cwd, cwd).is_ok() {
            return Ok(());
        }
        self.manager.check_agent_cwd(ws, caller_id, cwd).await
    }

    fn attention(&self, session: &RuntimeSession, what: Option<&str>) {
        self.notices.signal(session, what);
    }

    fn publish(&self, session: &RuntimeSession, event: Arc<RuntimeEvent>) {
        (self.publish)(Event::AgentRuntimeEvent {
            workspace_id: session.workspace_id.clone(),
            session_id: session.id.clone(),
            owner_id: session.created_by.clone(),
            event,
        });
    }
}
