//! `otto_assistant::AssistantCtx` for [`ServerCtx`] — the Assistant +
//! Personal Agents engines live in the `otto-assistant` crate; this module is
//! the daemon glue: shared handles plus thin delegations to the daemon-owned
//! helpers they use (`report_delivery`, `run_notices`, transcript
//! resolution, `cadence`). A run's session (a Threads thread, or a terminal
//! session for the other providers) is driven in `otto-assistant`.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use otto_assistant::{AssistantCtx, BoxFut, RunFailureNotice};
use otto_core::domain::Session;
use otto_core::event::Event;
use otto_core::{Id, Result};
use otto_state::{DbPool, WorkspacesRepo};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::cadence;
use crate::state::ServerCtx;

impl AssistantCtx for ServerCtx {
    fn pool(&self) -> &DbPool {
        &self.pool
    }
    fn events(&self) -> &broadcast::Sender<Event> {
        &self.events
    }
    fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn manager(&self) -> &Arc<otto_sessions::SessionManager> {
        &self.manager
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn mcp(&self) -> &Arc<otto_mcp::McpService> {
        &self.mcp
    }
    fn orchestrator(&self) -> &Arc<otto_orchestrator::Orchestrator> {
        &self.orchestrator
    }
    fn context_library(&self) -> &otto_context::Library {
        &self.context_library
    }
    fn memory(&self) -> &Arc<otto_memory::MemoryService> {
        &self.memory
    }
    fn roles(&self) -> &Arc<dyn otto_core::auth::RoleChecker> {
        &self.roles
    }
    fn agent_runtime(&self) -> &otto_agent_protocol::AgentRuntime {
        &self.agent_runtime
    }
    fn create_notice(&self, notice: otto_state::NewNotice) -> BoxFut<'_, Result<()>> {
        Box::pin(async move { self.notifications().create(notice).await.map(|_| ()) })
    }

    fn submit_prompt<'a>(&'a self, sid: &'a Id, prompt: &'a str) -> BoxFut<'a, bool> {
        Box::pin(otto_sessions::submit::submit_prompt(
            &self.manager,
            sid,
            prompt,
        ))
    }

    fn session_transcript<'a>(
        &'a self,
        session: &'a Session,
    ) -> BoxFut<'a, Option<(otto_transcript::Provider, PathBuf)>> {
        Box::pin(async move {
            let resolved = crate::transcript::resolve::resolve_transcript(self, session)
                .await
                .ok()?;
            Some((resolved.provider, resolved.path))
        })
    }

    fn blocking<T, F>(f: F) -> impl Future<Output = T> + Send
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        crate::offload::blocking(f)
    }

    fn deliver_destination<'a>(
        &'a self,
        workspace_id: &'a str,
        owner: Option<&'a str>,
        name: &'a str,
        destination: &'a Value,
        summary: &'a str,
        report: &'a str,
    ) -> BoxFut<'a, (bool, Option<String>)> {
        Box::pin(crate::report_delivery::deliver_destination(
            self,
            workspace_id,
            owner,
            name,
            destination,
            summary,
            report,
        ))
    }

    fn notify_run_failure(&self, n: RunFailureNotice) -> BoxFut<'_, ()> {
        Box::pin(crate::run_notices::notify_failure(
            self,
            crate::run_notices::RunNotice {
                key: n.key,
                severity: otto_core::domain::NoticeSeverity::Error,
                title: n.title,
                body: n.body,
                route: n.route,
                workspace_id: n.workspace_id,
                user_id: n.user_id,
            },
        ))
    }
    fn run_streak_key(kind: &str, id: &str) -> String {
        crate::run_notices::streak_key(kind, id)
    }
    fn clear_run_streak(key: &str) {
        crate::run_notices::clear_streak(key)
    }
    fn extract_summary(report: &str) -> String {
        crate::report_delivery::extract_summary(report)
    }
    fn report_hash(report: &str) -> String {
        crate::report_delivery::report_hash(report)
    }
    fn report_hash_matches(stored: &str, report: &str) -> bool {
        crate::report_delivery::report_hash_matches(stored, report)
    }
    fn augment_report_prompt(base: &str, out_path: &str) -> String {
        crate::report_delivery::augment_report_prompt(base, out_path)
    }
    fn write_report<'a>(abs: &'a Path, report: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(crate::report_delivery::write_report(abs, report))
    }

    fn cadence_validate(spec: &Value) -> Result<()> {
        cadence::validate(spec)
    }
    fn cadence_once_at(spec: &Value, tz: &str) -> Option<DateTime<Utc>> {
        cadence::once_at(spec, cadence::task_tz(tz))
    }
    fn cadence_is_due(
        spec: &Value,
        last_run: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        tz: &str,
    ) -> bool {
        cadence::is_due(spec, last_run, now, cadence::task_tz(tz))
    }
    fn cadence_is_due_since(
        spec: &Value,
        last_run: Option<DateTime<Utc>>,
        created: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        tz: &str,
    ) -> bool {
        cadence::is_due_since(spec, last_run, created, now, cadence::task_tz(tz))
    }
    fn cadence_effective_cursor(
        spec: &Value,
        last_run: Option<DateTime<Utc>>,
        armed: Option<DateTime<Utc>>,
    ) -> Option<DateTime<Utc>> {
        cadence::effective_cursor(spec, last_run, armed)
    }
    fn cadence_next_run(spec: &Value, from: DateTime<Utc>, tz: &str) -> Option<DateTime<Utc>> {
        cadence::next_run(spec, from, cadence::task_tz(tz))
    }
}
