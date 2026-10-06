//! `otto_assistant::AssistantCtx` for [`ServerCtx`] — the Assistant +
//! Personal Agents engines live in the `otto-assistant` crate; this module is
//! the daemon glue: shared handles plus thin delegations to the daemon-owned
//! helpers they use (session driving via `agent_run` / `review_session`,
//! `report_delivery`, `run_notices`, transcript resolution, `cadence`).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use otto_assistant::personal_agents_engine::{
    self as pa_engine, RunPlan, MAX_ATTEMPTS, RETRY_BACKOFF, RUN_NO_PROGRESS, STUCK_IDLE,
    WAITING_IDLE,
};
use otto_assistant::{
    AgentSessionOutcome, AgentSessionRun, AssistantCtx, BoxFut, RunFailureNotice,
};
use otto_core::api::CreateSessionReq;
use otto_core::domain::{Session, SessionKind};
use otto_core::event::Event;
use otto_core::{Error, Id, Result};
use otto_state::{DbPool, PersonalAgent, WorkspacesRepo};
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tracing::warn;

use crate::agent_run::run_with_recovery;
use crate::agent_run::watch_for_result;
use crate::cadence;
use crate::report_delivery::augment_report_prompt;
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
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
    fn create_notice(&self, notice: otto_state::NewNotice) -> BoxFut<'_, Result<()>> {
        Box::pin(async move { self.notifications().create(notice).await.map(|_| ()) })
    }

    fn submit_prompt<'a>(&'a self, sid: &'a Id, prompt: &'a str) -> BoxFut<'a, bool> {
        Box::pin(crate::review_session::submit_prompt(
            &self.manager,
            sid,
            prompt,
        ))
    }

    fn run_agent_session<'a>(
        &'a self,
        run: AgentSessionRun<'a>,
    ) -> BoxFut<'a, Result<AgentSessionOutcome>> {
        Box::pin(run_agent_session(self, run))
    }

    fn session_transcript<'a>(
        &'a self,
        session: &'a Session,
    ) -> BoxFut<'a, Option<(otto_transcript::Provider, PathBuf)>> {
        Box::pin(async move {
            let resolved = crate::routes::transcript::resolve_transcript(self, session)
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

/// A personal agent's visible-session attempt loop (moved verbatim from
/// `personal_agents_engine::execute_agent`): the agent writes its report to a
/// scratch file the watcher returns, retried per [`run_with_recovery`].
async fn run_agent_session(
    ctx: &ServerCtx,
    run: AgentSessionRun<'_>,
) -> Result<AgentSessionOutcome> {
    let AgentSessionRun {
        agent,
        run_id,
        owner,
        cwd,
        prompt,
        plan,
        cancel,
    } = run;
    let ws = ctx.workspaces.get(&agent.workspace_id).await?;

    // The agent writes its report here; the watcher returns its contents.
    let run_name = otto_core::paths::safe_component(run_id).unwrap_or("invalid");
    let out_path =
        pa_engine::default_agent_dir(ctx, &agent.id).join(format!("{run_name}.report.md"));
    if let Some(p) = out_path.parent() {
        let _ = tokio::fs::create_dir_all(p).await;
    }
    let _ = std::fs::remove_file(&out_path);
    let augmented = augment_report_prompt(prompt, &out_path.to_string_lossy());

    // Pre-trust so the session doesn't stall on the "trust this folder?" prompt.
    otto_sessions::trust::ensure_trusted(&agent.provider, cwd);

    let captured_sid: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let attempts = Arc::new(std::sync::atomic::AtomicI64::new(0));

    let outcome = run_with_recovery(
        &ctx.manager,
        MAX_ATTEMPTS,
        &RETRY_BACKOFF,
        None,
        |_attempt| {
            let captured = captured_sid.clone();
            let attempts = attempts.clone();
            let ws = ws.clone();
            let owner = owner.to_string();
            let cwd = cwd.to_string();
            let augmented = augmented.clone();
            let out_path = out_path.clone();
            async move {
                attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                run_one_session(
                    ctx, &ws, &owner, agent, run_id, &cwd, &augmented, &out_path, &captured, plan,
                    cancel,
                )
                .await
            }
        },
    )
    .await;
    // The watcher already read the report into `outcome`; the scratch file
    // would otherwise pile up one per run (for a personal agent, inside the
    // folder its next runs work in — where they could read stale reports).
    let _ = std::fs::remove_file(&out_path);

    let session_id = captured_sid
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let failure = outcome.errored().then(|| {
        outcome
            .reason
            .map(|r| r.as_str())
            .unwrap_or("unknown")
            .to_string()
    });
    Ok(AgentSessionOutcome {
        report: outcome.raw,
        session_id,
        attempts: attempts.load(std::sync::atomic::Ordering::Relaxed),
        failure,
    })
}

/// One attempt: create a fresh visible session of the agent's provider, inject
/// the prompt, and watch for the report file. Mirrors
/// `scheduled_tasks_engine::run_one_agent_session`. (Moved out of
/// `otto_assistant::personal_agents_engine`: it drives the shared `agent_run`
/// / `review_session` session plumbing, which stays in the daemon.)
#[allow(clippy::too_many_arguments)]
async fn run_one_session(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    owner: &str,
    agent: &PersonalAgent,
    run_id: &str,
    cwd: &str,
    prompt: &str,
    out_path: &std::path::Path,
    captured_sid: &Arc<Mutex<Option<String>>>,
    plan: &RunPlan,
    cancel: &otto_core::cancel_signal::CancelSignal,
) -> crate::agent_run::RunOutcome {
    use crate::agent_run::{FailReason, RunOutcome};

    let _ = std::fs::remove_file(out_path);
    // `personal_agent` in meta is the session→agent identity the room MCP tools
    // resolve; `browser` makes the manager reconcile the otto-browser MCP into
    // this cwd; `model` is the per-session model pin (same plumbing as
    // scheduled tasks). `work.origin` marks the run as ENGINE-owned: `source:
    // "personal_agent"` is deliberately outside `BACKGROUND_SESSION_SOURCES`
    // (these sessions stay listed in the Agents tab), so without the explicit
    // origin the idle sweep would read them as the user's own and never
    // reclaim them (`otto_sessions::manager::is_user_started`).
    // `read_only` confines the session (daemon tool policy + CLI tool list +
    // forced sandbox); a read-only run gets no browser automation either —
    // Playwright can click and submit, the CLI's own web reading stays.
    let mut meta = json!({
        "source": "personal_agent",
        "personal_agent": agent.id,
        "run_id": run_id,
        "browser": agent.browser && !plan.read_only,
        "agent_mode": plan.mode,
        "read_only": plan.read_only,
        "work": { "origin": "personal_agent" },
    });
    if !agent.model.trim().is_empty() {
        meta["model"] = json!(agent.model.trim());
    }
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(agent.provider.clone()),
        title: Some(format!("Agent: {}", agent.name)),
        cwd: Some(cwd.to_string()),
        connection_id: None,
        model: None,
        meta: Some(meta),
    };
    // Session creation must finish even if the run future is canceled. Its
    // completion publishes the exact session ID, then kills it when Stop won
    // before publication. No prompt can be submitted by this detached setup.
    let (ctx2, ws2, owner2, run2, stopped) = (
        ctx.clone(),
        ws.clone(),
        owner.to_string(),
        run_id.to_string(),
        cancel.clone(),
    );
    let creation = tokio::spawn(async move {
        let session = ctx2.manager.create(&ws2, &owner2, req, None).await?;
        if let Err(e) = pa_engine::repo(&ctx2)
            .set_run_session(&run2, &session.id)
            .await
        {
            let _ = ctx2.manager.kill_session(&session.id).await;
            return Err(e);
        }
        if stopped.is_cancelled() {
            ctx2.manager.kill_session(&session.id).await?;
            return Err(Error::Conflict(
                "run stopped during session creation".into(),
            ));
        }
        Ok(session)
    });
    let session = match creation
        .await
        .unwrap_or_else(|e| Err(Error::Internal(e.to_string())))
    {
        Ok(s) => s,
        Err(e) => {
            warn!(agent = %agent.id, "personal agent: create session ({}): {e}", agent.provider);
            return RunOutcome::failed(None, FailReason::CreateFailed);
        }
    };
    let sid = session.id.clone();
    *captured_sid.lock().unwrap_or_else(|e| e.into_inner()) = Some(sid.clone());

    if wait_for_tui(&ctx.manager, &sid).await {
        let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            let _ = ctx.manager.input(&sid, b"\r").await;
        }
    }

    watch_for_result(
        &ctx.manager,
        &sid,
        &agent.provider,
        session.provider_session_id.as_deref(),
        cwd,
        out_path,
        RUN_NO_PROGRESS,
        WAITING_IDLE,
        STUCK_IDLE,
        Some(|t| !t.trim().is_empty()),
        |_st| async {},
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_assistant::assistant::tasks::resolve_run_at;

    // Moved from `assistant::tasks` with the engine: it exercises the real
    // `cadence` through this impl's hooks.
    #[test]
    fn run_at_resolves_through_the_once_cadence() {
        let now = Utc::now();
        let future = (now + chrono::Duration::hours(2)).to_rfc3339();
        let (spec, at) = resolve_run_at::<ServerCtx>(&future, "UTC", now).unwrap();
        assert_eq!(spec["cadence"], "once");
        assert!(
            (at - (now + chrono::Duration::hours(2)))
                .num_seconds()
                .abs()
                <= 1
        );
        // Local wall-clock in a named zone.
        let (_, at) =
            resolve_run_at::<ServerCtx>("2099-01-01T09:00", "Asia/Jerusalem", now).unwrap();
        assert_eq!(at.to_rfc3339(), "2099-01-01T07:00:00+00:00");
        // Past and garbage are refused.
        assert!(resolve_run_at::<ServerCtx>("2001-01-01T09:00", "UTC", now).is_err());
        assert!(resolve_run_at::<ServerCtx>("at five", "UTC", now).is_err());
    }
}
