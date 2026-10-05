//! Glue: [`ServerCtx`] as the [`AutomationCtx`] the scheduled-task and goal-loop
//! engines (crate `otto-automation`) run against.
//!
//! Handles are plain field borrows; services forward to the otto-server modules
//! that still own them. `otto_automation::agent` re-exports the shared
//! `otto_agent_run` result types, so runner outcomes pass through unconverted.

use std::future::Future;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_automation::agent::{self as au, RoleTurn};
use otto_automation::goal_loop::GoalLoopRegistry;
use otto_automation::AutomationCtx;
use otto_core::domain::{NoticeSeverity, Workspace};
use otto_core::event::Event;
use otto_core::proof::{
    ProofArtifact, ProofArtifactKind, ProofArtifactStatus, ProofPack, WorkItemKind,
};
use otto_core::secrets::SecretStore;
use otto_core::workflows::{NodeRunState, RunScope, Workflow};
use otto_core::{Id, Result};
use otto_sessions::SessionManager;
use otto_state::{DbPool, GoalLoopsRepo, ProofRepo, ScheduledTasksRepo, WorkspacesRepo};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::agent_run as ar;
use crate::state::ServerCtx;

impl AutomationCtx for ServerCtx {
    fn pool(&self) -> &DbPool {
        &self.pool
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn events(&self) -> &broadcast::Sender<Event> {
        &self.events
    }
    fn data_dir(&self) -> &Path {
        &self.data_dir
    }
    fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn orchestrator(&self) -> &Arc<otto_orchestrator::Orchestrator> {
        &self.orchestrator
    }
    fn proof_repo(&self) -> &ProofRepo {
        &self.proof_repo
    }
    fn scheduled_tasks(&self) -> &ScheduledTasksRepo {
        &self.scheduled_tasks
    }
    fn goal_loops_repo(&self) -> &GoalLoopsRepo {
        &self.goal_loops_repo
    }
    fn goal_loops(&self) -> &GoalLoopRegistry {
        &self.goal_loops
    }

    async fn resolve_provider(
        &self,
        ws: Option<&Workspace>,
        requested: Option<&str>,
    ) -> Result<String> {
        ServerCtx::resolve_provider(self, ws, requested).await
    }

    async fn resolve_provider_for_ws(&self, ws_id: &Id, requested: Option<&str>) -> Result<String> {
        ServerCtx::resolve_provider_for_ws(self, ws_id, requested).await
    }

    async fn run_with_recovery<F, Fut>(
        &self,
        max_attempts: u32,
        backoff: &[Duration],
        cancel: Option<&Arc<AtomicBool>>,
        attempt: F,
    ) -> au::RunOutcome
    where
        F: FnMut(u32) -> Fut + Send,
        Fut: Future<Output = au::RunOutcome> + Send,
    {
        ar::run_with_recovery(&self.manager, max_attempts, backoff, cancel, attempt).await
    }

    async fn watch_for_result<F, Fut>(
        &self,
        sid: &Id,
        provider: &str,
        provider_session_id: Option<&str>,
        cwd: &str,
        out_path: &Path,
        timeout: Duration,
        waiting_idle: Duration,
        stuck_idle: Duration,
        transcript_ok: Option<fn(&str) -> bool>,
        on_status: F,
    ) -> au::RunOutcome
    where
        F: FnMut(au::WatchStatus) -> Fut + Send,
        Fut: Future<Output = ()> + Send,
    {
        ar::watch_for_result(
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

    async fn wait_for_tui(&self, sid: &Id) -> bool {
        crate::review_session::wait_for_tui(&self.manager, sid).await
    }

    async fn dispatched(&self, sid: &Id, before: Option<Instant>) -> bool {
        crate::review_session::dispatched(&self.manager, sid, before).await
    }

    fn bracketed_paste(&self, text: &str) -> Vec<u8> {
        crate::review_session::bracketed_paste(text)
    }

    fn paste_to_enter(&self) -> Duration {
        crate::review_session::PASTE_TO_ENTER
    }

    async fn run_role_turn<C, P, PF>(
        &self,
        turn: RoleTurn<'_>,
        cancelled: C,
        publish: P,
    ) -> std::result::Result<String, String>
    where
        C: Future<Output = ()> + Send,
        P: FnOnce(String) -> PF + Send,
        PF: Future<Output = std::result::Result<(), String>> + Send,
    {
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let opts = crate::agent_session::TurnOpts {
            done_file: Some(turn.done_file),
            done_file_validator: Some(turn.done_file_validator),
            ..Default::default()
        };
        let budget = turn.budget;
        let run = async {
            crate::agent_session::run_session_turn_with(
                self,
                turn.workspace,
                turn.user,
                None,
                turn.title,
                turn.cwd,
                turn.provider,
                turn.meta,
                turn.prompt,
                budget,
                opts,
                |id| {
                    let _ = ready_tx.send(id.clone());
                },
            )
            .await
            .map(|(text, _)| text)
            .map_err(|e| format!("{e:?}"))
        };
        crate::review_summarizer::drive(
            run,
            ready_rx,
            budget,
            cancelled,
            publish,
            |id| async move {
                crate::review_session::stop_review_sessions(&self.manager, &[id]).await;
            },
        )
        .await
    }

    fn compose_skill_prompt(&self, skill: &str, wrapped: &str) -> String {
        let skill_text = crate::modules::resolve_skill_inline(&self.context_library, skill);
        crate::modules::compose_draft_prompt(&skill_text, wrapped)
    }

    fn spawn_workflow_run(
        &self,
        ws: Workspace,
        workflow: Workflow,
        run_id: Id,
        input: Value,
        scope: RunScope,
        prior_nodes: Option<Vec<NodeRunState>>,
    ) {
        crate::workflow_engine::spawn_run(
            self.clone(),
            ws,
            workflow,
            run_id,
            input,
            scope,
            prior_nodes,
        );
    }

    async fn proof_gate(
        &self,
        kind: WorkItemKind,
        work_item_id: &str,
        workspace_id: &str,
        title: &str,
        created_by: &str,
    ) -> Result<ProofPack> {
        crate::proof::gate(self, kind, work_item_id, workspace_id, title, created_by).await
    }

    async fn proof_upsert_content_artifact(
        &self,
        pack: &ProofPack,
        kind: ProofArtifactKind,
        title: &str,
        content: &str,
        status: ProofArtifactStatus,
        extra_meta: Value,
        created_by: &str,
    ) -> Result<ProofArtifact> {
        crate::proof::upsert_content_artifact(
            self, pack, kind, title, content, status, extra_meta, created_by,
        )
        .await
    }

    async fn proof_recompute_and_emit(&self, pack_id: &str) -> Result<ProofPack> {
        crate::proof::recompute_and_emit(self, pack_id).await
    }

    async fn run_notice(
        &self,
        kind: &str,
        id: &str,
        failure: Option<(String, String)>,
        route: String,
        workspace_id: Option<String>,
        user_id: Option<String>,
    ) {
        let key = crate::run_notices::streak_key(kind, id);
        let Some((title, body)) = failure else {
            crate::run_notices::clear_streak(&key);
            return;
        };
        crate::run_notices::notify_failure(
            self,
            crate::run_notices::RunNotice {
                key,
                severity: NoticeSeverity::Error,
                title,
                body,
                route,
                workspace_id,
                user_id,
            },
        )
        .await;
    }
}
