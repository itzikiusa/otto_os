//! What the runtime needs from the daemon: the workspace, the PTY path's
//! safety pipeline, and the event bus. [`SessionManagerHost`] is the
//! production implementation; tests supply their own.

use std::sync::Arc;

use otto_core::agent_runtime::{RuntimeEvent, RuntimeSession};
use otto_core::domain::{Session, SessionKind, SessionStatus, Workspace};
use otto_core::event::Event;
use otto_core::{Id, Result};
use otto_sessions::SessionManager;
use otto_state::WorkspacesRepo;

use crate::driver::LaunchSpec;

#[async_trait::async_trait]
pub trait RuntimeHost: Send + Sync + 'static {
    async fn workspace(&self, id: &Id) -> Result<Workspace>;

    /// Whether the child will be Seatbelt-confined (drivers relax the
    /// provider's own nested sandbox accordingly).
    async fn sandboxed(&self, session: &RuntimeSession) -> bool;

    /// Apply the PTY sessions' safety pipeline to `spec` in place: cwd
    /// refusal, folder trust, context provisioning, Seatbelt wrap. An error
    /// refuses the spawn. Must move `spec.tail_args` behind the injected
    /// context flags (and leave `tail_args` empty).
    async fn prepare_spawn(
        &self,
        ws: &Workspace,
        session: &RuntimeSession,
        spec: &mut LaunchSpec,
    ) -> Result<()>;

    /// Deliver one event to WebSocket clients (owner / admin / root).
    fn publish(&self, session: &RuntimeSession, event: Arc<RuntimeEvent>);
}

/// The synthetic PTY-shaped [`Session`] the safety helpers read (never
/// persisted to `sessions`).
pub fn as_agent_session(s: &RuntimeSession) -> Session {
    Session {
        id: s.id.clone(),
        workspace_id: s.workspace_id.clone(),
        kind: SessionKind::Agent,
        provider: s.provider.as_str().to_string(),
        title: s.title.clone(),
        status: SessionStatus::Running,
        cwd: s.cwd.clone(),
        provider_session_id: s.native_thread_id.clone(),
        connection_id: None,
        created_by: s.created_by.clone(),
        created_at: s.created_at,
        last_active_at: s.last_active_at,
        archived: false,
        meta: serde_json::json!({ "runtime": "structured" }),
    }
}

/// Production host: `SessionManager` for safety, the daemon bus for events.
pub struct SessionManagerHost {
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    /// Publishes on the WS streaming path (otto-server's `publish_stream`).
    pub publish: Box<dyn Fn(Event) + Send + Sync>,
}

#[async_trait::async_trait]
impl RuntimeHost for SessionManagerHost {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        self.workspaces.get(id).await
    }

    async fn sandboxed(&self, session: &RuntimeSession) -> bool {
        self.manager
            .runtime_has_process_sandbox(session.provider.as_str())
            .await
    }

    async fn prepare_spawn(
        &self,
        ws: &Workspace,
        session: &RuntimeSession,
        spec: &mut LaunchSpec,
    ) -> Result<()> {
        let mut cmd = otto_pty::CommandSpec {
            program: std::mem::take(&mut spec.program),
            args: std::mem::take(&mut spec.args),
            cwd: Some(session.cwd.clone()),
            env: std::mem::take(&mut spec.env),
        };
        let tail = std::mem::take(&mut spec.tail_args);
        let res = self
            .manager
            .prepare_runtime_spawn(ws, &as_agent_session(session), &mut cmd, tail)
            .await;
        spec.program = cmd.program;
        spec.args = cmd.args;
        spec.env = cmd.env;
        res
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
