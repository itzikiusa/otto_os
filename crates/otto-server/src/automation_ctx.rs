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

#[cfg(test)]
mod handoff_tests {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    use otto_automation::AutomationCtx;
    use otto_core::workflows::{RunStatus, WorkflowGraph};
    use otto_state::{NewScheduledTask, WorkflowsRepo};
    use serde_json::json;

    /// S3-303: a daemon restart while a workflow-kind scheduled run waits on
    /// its workflow must not record a false "interrupted" failure — boot
    /// re-attaches the waiter (without blocking), and the run settles with the
    /// workflow's REAL outcome once boot recovery lets the workflow finish.
    #[tokio::test]
    async fn restart_reattaches_a_workflow_handoff_and_records_its_real_outcome() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = mem_pool().await;
        seed_workspace(&pool, "ho-ws").await;
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('u','u','x','U',0,?)")
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        let wfs = WorkflowsRepo::new(pool.clone());
        let wf = wfs
            .create(
                &"ho-ws".into(),
                "WF",
                "",
                "",
                &WorkflowGraph::default(),
                &"u".into(),
            )
            .await
            .unwrap();
        let wf_run = wfs
            .create_run(&wf.id, &"ho-ws".into(), &json!({}), None)
            .await
            .unwrap();
        let repo = ctx.scheduled_tasks();
        let task = repo
            .create(NewScheduledTask {
                kind: "workflow".into(),
                workflow_id: Some(wf.id.clone()),
                schedule: json!({"cadence": "interval", "every_min": 60}),
                destination: json!({"type": "none"}),
                ..NewScheduledTask::defaults("ho-ws".into(), "T".into())
            })
            .await
            .unwrap();
        // Production admission persists the definition needed for safe recovery.
        // Legacy rows without that snapshot are covered by the recovery tests.
        let run = repo.admit_run(&task, "manual").await.unwrap();
        repo.set_run_workflow_run(&run.id, &wf_run.id)
            .await
            .unwrap();

        // Boot: the reap leaves the hand-off alone and returns at once.
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::scheduled_tasks_scheduler::reap_interrupted(&ctx),
        )
        .await
        .expect("boot reap never blocks on the workflow");
        assert_eq!(repo.get_run(&run.id).await.unwrap().status, "running");

        // The resumed workflow finishes; the re-attached waiter records it.
        wfs.update_run_if(
            &wf_run.id,
            &[RunStatus::Pending, RunStatus::Running],
            RunStatus::Success,
            &[],
            None,
            true,
        )
        .await
        .unwrap()
        .expect("workflow run settled");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let r = repo.get_run(&run.id).await.unwrap();
            if r.status != "running" {
                assert_eq!(r.status, "ok", "{:?}", r.error);
                assert_eq!(r.workflow_run_id.as_deref(), Some(wf_run.id.as_str()));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the re-attached waiter never settled the run"
            );
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }
}
