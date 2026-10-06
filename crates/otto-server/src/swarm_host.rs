//! Glue for the swarm runtime (`otto_swarm::runtime`): [`ServerCtx`] is its
//! [`SwarmHost`] — narrow handles plus the server-owned behaviours the runtime
//! needs (budget gate, Product seeding, the PTY drive helpers it shares with the
//! review engine). The runtime's types mirror `agent_run`'s one-to-one; the
//! conversions below are the only place they meet.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::extract::FromRef;
use otto_core::auth::RoleChecker;
use otto_core::domain::Workspace;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::Id;
use otto_orchestrator::Orchestrator;
use otto_sessions::SessionManager;
use otto_state::{IntegrationsRepo, SwarmProject, SwarmRepo, WorkspacesRepo};
use otto_swarm::runtime::engine::CoordinatorRegistry;
use otto_swarm::runtime::host::{AttemptFn, PtyTimings, RunOutcome, StatusFn, TurnTail};
use otto_swarm::runtime::run::CancelRegistry;
use otto_swarm::runtime::{SwarmHost, SwarmRt};
use tokio::sync::broadcast;

use crate::agent_run;
use crate::review_session;
use crate::state::ServerCtx;

impl ServerCtx {
    /// This context as the swarm runtime's host handle.
    pub fn swarm_rt(&self) -> SwarmRt {
        SwarmRt::new(Arc::new(self.clone()))
    }
}

impl FromRef<ServerCtx> for SwarmRt {
    fn from_ref(ctx: &ServerCtx) -> Self {
        ctx.swarm_rt()
    }
}

/// The turn oracle's claude tail, as the runtime's [`TurnTail`].
struct ClaudeTurnTail(crate::turn_oracle::ClaudeTail);

impl TurnTail for ClaudeTurnTail {
    fn poll_last_turn_text(&mut self) -> Option<String> {
        self.0.poll().and_then(|s| s.last_turn_text)
    }
}

#[async_trait]
impl SwarmHost for ServerCtx {
    fn swarm_repo(&self) -> &SwarmRepo {
        &self.swarm_repo
    }
    fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }
    fn events(&self) -> &broadcast::Sender<Event> {
        &self.events
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn integrations_store(&self) -> &IntegrationsRepo {
        &self.integrations_store
    }
    fn orchestrator(&self) -> &Arc<Orchestrator> {
        &self.orchestrator
    }
    fn context_library(&self) -> &otto_context::Library {
        &self.context_library
    }
    fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn swarm_coords(&self) -> &CoordinatorRegistry {
        &self.swarm_coords
    }
    fn swarm_run_cancels(&self) -> &CancelRegistry {
        &self.swarm_run_cancels
    }

    fn available_providers(&self) -> Vec<String> {
        <Self as otto_swarm::SwarmCtx>::available_providers(self)
    }
    async fn resolve_provider_or_fallback(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
        site: &'static str,
    ) -> String {
        ServerCtx::resolve_provider_or_fallback(self, ws, requested, site).await
    }
    async fn budget_blocked(&self, workspace_id: &str) -> Option<Option<String>> {
        let verdict = crate::routes::usage::check_budget(self, workspace_id, "").await;
        verdict.blocked.then_some(verdict.reason)
    }
    async fn session_usage_totals(
        &self,
        session_id: &str,
        since: chrono::DateTime<chrono::Utc>,
    ) -> Option<(u64, u64, f64)> {
        // Background (S9-303): flush-and-wait so this turn's rows count, and
        // never reset ClickHouse's idle clock from autonomous work.
        self.usage
            .session_totals_for_background(session_id, Some(since))
            .await
            .map(|t| (t.input_tokens, t.output_tokens, t.cost_usd))
    }
    async fn seed_tasks(&self, project: &SwarmProject, creator: &Id, goal: &str) {
        let _ = otto_product::swarm::seed_tasks(self, project, creator, goal).await;
    }

    fn pty_timings(&self) -> PtyTimings {
        PtyTimings {
            paste_to_enter: review_session::PASTE_TO_ENTER,
            prompt_land_wait: review_session::PROMPT_LAND_WAIT,
            waiting_idle: review_session::WAITING_IDLE,
        }
    }
    fn bracketed_paste(&self, text: &str) -> Vec<u8> {
        review_session::bracketed_paste(text)
    }
    async fn wait_for_tui(&self, sid: &Id) -> bool {
        review_session::wait_for_tui(&self.manager, sid).await
    }
    async fn dispatched(&self, sid: &Id, before: Option<Instant>) -> bool {
        review_session::dispatched(&self.manager, sid, before).await
    }
    fn transcript_len(&self, cwd: &str, psid: &str) -> u64 {
        review_session::transcript_len(cwd, psid)
    }
    async fn claude_prompt_landed(&self, sid: &Id, cwd: &str, offset: u64, wait: Duration) -> bool {
        review_session::claude_prompt_landed(&self.manager, sid, cwd, offset, wait).await
    }
    fn claude_turn_tail(&self, path: PathBuf, offset: u64) -> Box<dyn TurnTail> {
        Box::new(ClaudeTurnTail(crate::turn_oracle::ClaudeTail::from_offset(
            path, offset,
        )))
    }
    async fn watch_for_result<'a>(
        &'a self,
        sid: &'a Id,
        provider: &'a str,
        provider_session_id: Option<&'a str>,
        cwd: &'a str,
        out_path: &'a Path,
        timeout: Duration,
        waiting_idle: Duration,
        stuck_idle: Duration,
        transcript_ok: Option<for<'s> fn(&'s str) -> bool>,
        on_status: &'a mut StatusFn<'a>,
    ) -> RunOutcome {
        agent_run::watch_for_result(
            &self.manager,
            sid,
            provider,
            provider_session_id,
            cwd,
            out_path,
            timeout,
            waiting_idle,
            stuck_idle,
            transcript_ok,
            on_status,
        )
        .await
    }
    async fn run_with_recovery<'a>(
        &'a self,
        max_attempts: u32,
        backoff: &'a [Duration],
        cancel: Option<&'a Arc<AtomicBool>>,
        attempt: &'a mut AttemptFn<'a>,
    ) -> RunOutcome {
        agent_run::run_with_recovery(&self.manager, max_attempts, backoff, cancel, attempt).await
    }
}
