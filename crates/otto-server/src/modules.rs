//! Integration seam (plan Task A9): implements the module routers' ctx traits
//! on [`ServerCtx`], provides the PTY-backed connection [`Spawner`], the
//! orchestrator routes, and assembles the module routers for `build_router`.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use otto_connections::{ConnectionsService, Spawner};
use otto_core::api::{
    CreateSessionReq, ExecutePlanReq, HandoffReq, LocalReviewReq, NewPrCommentReq, OrchestrateReq,
    OrchestrateResp, RepoReviewBinding, RepoReviewConfigResp, ReviewConfig, ReviewConfigPreset,
    StartReviewReq, UpdateProvidersReq,
};
use otto_core::auth::{BoxFuture, RoleChecker};
use otto_core::domain::{
    CommentSeverity, CommentState, Connection, Review, ReviewAgentCfg, ReviewAgentState,
    ReviewComment, ReviewStatus, Session, SessionKind, User, Workspace, WorkspaceRole,
};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_orchestrator::{execute, ExecuteResp, OrchestratorContext, PlanIo, PlanSpawner};
use otto_pty::CommandSpec;
use otto_sessions::SessionManager;
use otto_state::{GitStore, IntegrationsRepo, IssuesRepo, WorkspacesRepo};
use serde::Deserialize;
use serde_json::Value;

use crate::auth::{CurrentAuthContext, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
// The review engine's pure core lives in otto-review; re-exported so the
// `crate::modules::…` paths other engines use keep resolving.
pub use otto_review::engine::*;
use otto_review::worktree::{teardown_pr_worktree, PrWorktree};

/// Reload identity and workspace authority at the actual input boundary. A
/// session owner still needs Editor membership to type into that workspace.
async fn input_user(pool: &otto_state::DbPool, user_id: &Id, session: &Session) -> Result<User> {
    let user = otto_state::UsersRepo::new(pool.clone())
        .get(user_id)
        .await?;
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    let role = WorkspacesRepo::new(pool.clone())
        .role_of(&user, &session.workspace_id)
        .await?;
    if !matches!(role, Some(WorkspaceRole::Editor | WorkspaceRole::Admin)) {
        return Err(Error::Forbidden(
            "workspace Editor access is required for session input".into(),
        ));
    }
    if !user.is_root && user.id != session.created_by && role != Some(WorkspaceRole::Admin) {
        return Err(Error::Forbidden(
            "not the session owner or a workspace admin".into(),
        ));
    }
    crate::resource_sessions::check_with_pool(pool, &user, session).await?;
    Ok(user)
}

async fn input_session(ctx: &ServerCtx, user_id: &Id, id: &Id) -> Result<Session> {
    let session = ctx.manager.get(id).await?;
    input_user(&ctx.pool, user_id, &session).await?;
    Ok(session)
}

/// Who is typing into a session through the REST fan-out routes: the person
/// the credential belongs to, and — when the credential is an agent
/// session's own token — that session (S11-305). An agent caller reaches only
/// itself and the workers it opened, and what it sends is recorded as
/// agent-originated, never as the person's message.
#[derive(Clone, Copy)]
struct Typist<'a> {
    user_id: &'a Id,
    agent: Option<&'a Id>,
}

impl<'a> Typist<'a> {
    fn new(user_id: &'a Id, auth: &'a otto_core::auth::AuthContext) -> Self {
        Self {
            user_id,
            agent: crate::feature_guard::agent_session_of(auth),
        }
    }
}

/// Send both the paste and its delayed submit through current authorization.
async fn submit_session_text(ctx: &ServerCtx, who: Typist<'_>, id: &Id, text: &str) -> Result<()> {
    let user_id = who.user_id;
    let session = input_session(ctx, user_id, id).await?;
    agent_input_rule(who.agent, &session.id, &session.meta, true)?;
    ctx.manager
        .human_submit_text_checked(id, user_id, false, text, || async {
            input_session(ctx, user_id, id).await.map(|_| ())
        })
        .await?;
    match who.agent {
        Some(from) => ctx.manager.record_agent_message(id, from, text).await,
        None => ctx.manager.record_user_message(id, text).await,
    }
    Ok(())
}

async fn input_agents(ctx: &ServerCtx, who: Typist<'_>, ws_id: &Id) -> Result<Vec<Session>> {
    let mut allowed = Vec::new();
    for session in ctx.manager.list_by_workspace(ws_id).await? {
        if session.kind == SessionKind::Agent
            && matches!(
                session.status,
                otto_core::domain::SessionStatus::Running
                    | otto_core::domain::SessionStatus::Working
                    | otto_core::domain::SessionStatus::Idle
            )
            && agent_input_rule(who.agent, &session.id, &session.meta, true).is_ok()
            && input_session(ctx, who.user_id, &session.id).await.is_ok()
        {
            allowed.push(session);
        }
    }
    Ok(allowed)
}

async fn broadcast_sessions(
    ctx: &ServerCtx,
    who: Typist<'_>,
    ws_id: &Id,
    text: &str,
    targets: Option<&[Id]>,
) -> Result<Vec<Id>> {
    let mut sent = Vec::new();
    for session in input_agents(ctx, who, ws_id).await? {
        if targets.is_none_or(|ids| ids.contains(&session.id))
            && submit_session_text(ctx, who, &session.id, text)
                .await
                .is_ok()
        {
            sent.push(session.id);
        }
    }
    Ok(sent)
}

fn delay_session_input(
    ctx: &ServerCtx,
    user: &User,
    session_id: &Id,
    text: String,
    delay: Duration,
    token: String,
) {
    let ctx = ctx.clone();
    let user_id = user.id.clone();
    let session_id = session_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let result: Result<()> = async {
            let auth = ctx.authenticator.authenticate(&token).await?;
            if auth.effective_user.id != user_id || auth.mcp_only || auth.scope.is_some() {
                return Err(Error::Unauthorized);
            }
            input_session(&ctx, &user_id, &session_id).await?;
            ctx.manager
                .human_input(
                    &session_id,
                    &user_id,
                    false,
                    true,
                    format!("{text}\n").as_bytes(),
                )
                .await
        }
        .await;
        if let Err(e) = result {
            tracing::warn!(session = %session_id, "delayed session input denied or failed: {e}");
        }
    });
}

// ---------------------------------------------------------------------------
// Router ctx trait impls
// ---------------------------------------------------------------------------

impl otto_sessions::SessionsCtx for ServerCtx {
    fn check_resource<'a>(
        &'a self,
        user: &'a otto_core::domain::User,
        session: &'a otto_core::domain::Session,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(crate::resource_sessions::check(self, user, session))
    }

    fn check_resources<'a>(
        &'a self,
        user: &'a otto_core::domain::User,
        sessions: Vec<otto_core::domain::Session>,
    ) -> BoxFuture<'a, Vec<otto_core::domain::Session>> {
        Box::pin(crate::resource_sessions::check_many(self, user, sessions))
    }

    fn resource_bound(&self, session: &otto_core::domain::Session) -> bool {
        crate::resource_sessions::binding(session).is_some()
    }

    fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
}

impl PtySpawner {
    async fn spawn_adhoc(
        &self,
        ws_id: &Id,
        user_id: &Id,
        provider: &str,
        spec: CommandSpec,
        title: String,
        meta: Option<serde_json::Value>,
    ) -> Result<Session> {
        let ws = self.workspaces.get(ws_id).await?;
        let req = CreateSessionReq {
            kind: SessionKind::Connection,
            provider: Some(provider.to_string()),
            title: Some(title),
            cwd: None,
            connection_id: None,
            model: None,
            meta,
        };
        self.manager.create(&ws, user_id, req, Some(spec)).await
    }
}

impl otto_aws::AwsCtx for ServerCtx {
    fn pool(&self) -> otto_state::DbPool {
        self.pool.clone()
    }
    fn secrets(&self) -> &Arc<dyn otto_core::secrets::SecretStore> {
        &self.secrets
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
        &self.events
    }
    fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner
    }
}

impl otto_k8s::K8sCtx for ServerCtx {
    fn pool(&self) -> otto_state::DbPool {
        self.pool.clone()
    }
    fn secrets(&self) -> &Arc<dyn otto_core::secrets::SecretStore> {
        &self.secrets
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
        &self.events
    }
    fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner
    }
    fn monitor_sink(&self) -> Option<Arc<dyn otto_k8s::MonitorSink>> {
        Some(Arc::new(UsageSink(self.usage.clone())))
    }
}

/// `otto_k8s::MonitorSink` over the embedded ClickHouse usage engine.
struct UsageSink(Arc<otto_usage::UsageEngine>);

impl otto_k8s::MonitorSink for UsageSink {
    fn available(&self) -> bool {
        self.0.available()
    }
    fn exec<'a>(&'a self, sql: &'a str) -> otto_k8s::BoxFut<'a, otto_core::Result<()>> {
        Box::pin(async move { self.0.exec_sql(sql).await })
    }
    fn insert_ndjson<'a>(
        &'a self,
        table: &'a str,
        ndjson: &'a str,
    ) -> otto_k8s::BoxFut<'a, otto_core::Result<()>> {
        Box::pin(async move { self.0.insert_ndjson(table, ndjson).await })
    }
    fn query_rows<'a>(
        &'a self,
        sql: &'a str,
    ) -> otto_k8s::BoxFut<'a, otto_core::Result<Vec<serde_json::Value>>> {
        Box::pin(async move { self.0.query_rows(sql).await })
    }
}

impl otto_connections::ConnectionsCtx for ServerCtx {
    fn connections(&self) -> &Arc<ConnectionsService> {
        &self.connections
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn spawner(&self) -> &Arc<dyn Spawner> {
        &self.spawner
    }
    fn pool(&self) -> otto_state::DbPool {
        self.pool.clone()
    }
    fn db_tester(&self) -> Option<Arc<dyn otto_connections::DbTester>> {
        Some(Arc::new(DbViewerTester {
            db: Arc::clone(&self.db_explorer),
        }))
    }
}

/// Bridges `DbTester` (from `otto-connections`) to `DbViewerService::test`,
/// so `POST /connections/{id}/test` on a DB-kind connection reuses the warm
/// SSH tunnel cache (`ssh -L` local forward cached per connection id) instead
/// of spawning a fresh `ssh -J` child per probe.
struct DbViewerTester {
    db: Arc<otto_dbviewer::DbViewerService>,
}

impl otto_connections::DbTester for DbViewerTester {
    fn test_db_connection<'a>(
        &'a self,
        id: &'a Id,
        user_id: &'a Id,
    ) -> BoxFuture<'a, Result<otto_core::api::TestConnectionResp>> {
        Box::pin(async move {
            let r = self.db.test(id, user_id).await?;
            Ok(otto_core::api::TestConnectionResp {
                ok: r.ok,
                latency_ms: r.latency_ms,
                // Include the server version in the message when the probe
                // succeeds (the CLI path's message is just "ok" with no detail).
                message: if r.ok {
                    r.server_version
                        .map(|v| format!("ok — {v}"))
                        .unwrap_or(r.message)
                } else {
                    r.message
                },
                // Driver-backed probes never pass a secret through argv.
                warn_argv: false,
                // The SSH key-permission warning is filled by the `test_connection`
                // handler (it has the Connection + covers both this cached-tunnel
                // path and the CLI path uniformly), so leave it None here.
                warn_key_perms: None,
                // A tunnel failure's fix hint is already part of `message`
                // (otto-ssh diagnoses it); drivers report their own text.
                hint: None,
            })
        })
    }

    fn forget_secret(&self, secret_ref: &str) {
        self.db.forget_secret(secret_ref);
    }
}

impl otto_dbviewer::DbViewerCtx for ServerCtx {
    fn db(&self) -> &Arc<otto_dbviewer::DbViewerService> {
        &self.db_explorer
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn pool(&self) -> Option<otto_state::DbPool> {
        // Lets the global-connection branch of the route gates consult the
        // caller's `Database` grant instead of falling back to root-only.
        Some(self.pool.clone())
    }
    fn drafter(&self) -> Option<std::sync::Arc<dyn otto_dbviewer::nl::SqlDrafter>> {
        // Wire the verified NL→SQL loop to the real agent/LLM via the same
        // one-shot orchestrator path the commit-/PR-draft endpoints use. The
        // planner turn needs no repo files (it's grounded by the schema
        // summary), so a neutral temp dir is the working directory.
        Some(std::sync::Arc::new(crate::db_drafter::AgentSqlDrafter {
            orchestrator: std::sync::Arc::clone(&self.orchestrator),
            cwd: std::env::temp_dir().to_string_lossy().to_string(),
        }))
    }
    fn on_confirmed_write(
        &self,
        user: &otto_core::domain::User,
        conn: &otto_core::domain::Connection,
        statement: &str,
    ) {
        // The audit write is async + best-effort; the route must not block on
        // it, so spawn a detached task off a cheap ctx clone.
        let ctx = self.clone();
        let user_id = user.id.clone();
        let conn_name = conn.name.clone();
        let environment = conn.environment.as_str().to_string();
        // Keep a bounded statement preview (audit detail, not a query log).
        let preview: String = statement.chars().take(512).collect();
        tokio::spawn(async move {
            ctx.audit(otto_state::NewAuditEntry {
                user_id: Some(user_id),
                action: "db.write_confirmed".into(),
                target: Some(conn_name),
                detail: Some(serde_json::json!({
                    "environment": environment,
                    "statement": preview,
                })),
                ip: None,
            })
            .await;
        });
    }
}

impl otto_brokers::BrokersCtx for ServerCtx {
    fn brokers(&self) -> &Arc<otto_brokers::BrokersService> {
        &self.brokers
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn pool(&self) -> Option<otto_state::DbPool> {
        // Lets the global-cluster branch consult the caller's `Database` grant
        // instead of falling back to root-only.
        Some(self.pool.clone())
    }
}

impl otto_mcp::McpCtx for ServerCtx {
    fn mcp(&self) -> &Arc<otto_mcp::McpService> {
        &self.mcp
    }
    fn mcp_pool(&self) -> &otto_state::DbPool {
        &self.pool
    }
    fn mcp_secrets(&self) -> &Arc<dyn otto_core::secrets::SecretStore> {
        &self.secrets
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

#[async_trait::async_trait]
impl otto_git::GitCtx for ServerCtx {
    fn store(&self) -> &GitStore {
        &self.git_store
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<Event> {
        &self.events
    }
    async fn check_pr_allowed(
        &self,
        workspace_id: &str,
        req: &otto_core::api::CreatePrReq,
    ) -> otto_core::Result<()> {
        crate::proof::gate_pr(self, workspace_id, req).await
    }

    async fn after_pr_created(
        &self,
        repo: &otto_core::domain::Repo,
        pr_number: u64,
        req: &otto_core::api::CreatePrReq,
        ci: &otto_git::CiStatus,
    ) {
        // Only act when a proof pack is linked. Best-effort throughout — a capture
        // failure must never surface to the PR caller.
        let Some(pack_id) = req.proof_pack_id.as_deref() else {
            return;
        };
        let Ok(pack) = self.proof_repo.get_pack(pack_id).await else {
            return;
        };
        let _ = self
            .proof_repo
            .set_repo_link(&pack.id, Some(&repo.id), Some(pr_number as i64))
            .await;
        let summary = otto_core::proof::CiSummary {
            state: ci.state.clone(),
            total: ci.total,
            passed: ci.passed,
            failed: ci.failed,
            url: ci.url.clone(),
        };
        let _ = crate::proof::record_ci_artifact(self, &pack, &summary).await;
        // R7: run the PR-description consistency check against the actual change
        // (target_branch..HEAD in the repo) and record a `pr_check` artifact. A
        // dishonest description (false "tests pass" claim) flips the pack to
        // `failed`; an honest-but-thin one is neutral. The endpoint remains for
        // re-running on demand.
        let _ = crate::proof::run_pr_check(
            self,
            &pack,
            &req.title,
            &req.description,
            Some(&req.target_branch),
            Some(&repo.path),
            "otto",
        )
        .await;
    }

    async fn cleanup_base_branch(&self, repo_id: &Id) -> Option<String> {
        let settings = otto_state::SettingsRepo::new(self.pool.clone());
        match settings.get(&cleanup_base_key(repo_id)).await {
            Ok(Some(v)) => v.as_str().map(str::to_string).filter(|s| !s.is_empty()),
            _ => None,
        }
    }

    async fn set_cleanup_base_branch(
        &self,
        repo_id: &Id,
        base: Option<String>,
    ) -> otto_core::Result<()> {
        let settings = otto_state::SettingsRepo::new(self.pool.clone());
        let key = cleanup_base_key(repo_id);
        match base.map(|b| b.trim().to_string()).filter(|b| !b.is_empty()) {
            Some(b) => settings.put(&key, &serde_json::Value::String(b)).await,
            None => settings.delete(&key).await,
        }
    }
}

/// Settings key holding one repo's cleanup base branch override (drives the
/// "safe to delete (merged)" branch indicators). `None`/absent = detected default.
fn cleanup_base_key(repo_id: &Id) -> String {
    format!("cleanup_base:{repo_id}")
}

impl otto_issues::IssuesCtx for ServerCtx {
    fn issues(&self) -> &IssuesRepo {
        &self.issues_store
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
}

impl otto_channels::ChannelsCtx for ServerCtx {
    fn integrations(&self) -> &IntegrationsRepo {
        &self.integrations_store
    }
    fn secrets(&self) -> &Arc<dyn SecretStore> {
        &self.secrets
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
}

impl otto_improve::ImproveCtx for ServerCtx {
    fn engine(&self) -> &Arc<otto_improve::ImprovementEngine> {
        &self.improve_engine
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
}

impl otto_context::ContextCtx for ServerCtx {
    fn library(&self) -> &otto_context::Library {
        &self.context_library
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

impl otto_product::ProductCtx for ServerCtx {
    fn product(&self) -> &Arc<otto_product::ProductService> {
        &self.product
    }
    fn product_repo(&self) -> &otto_state::ProductRepo {
        &self.product_repo
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn swarm_repo(&self) -> Option<&otto_state::SwarmRepo> {
        Some(&self.swarm_repo)
    }
    fn attachments_root(&self) -> Option<std::path::PathBuf> {
        Some(self.data_dir.join(otto_product::media::ATTACH_ROOT))
    }
    fn mockup_scratch_root(&self) -> Option<std::path::PathBuf> {
        Some(self.data_dir.join(crate::mockup_assist::SCRATCH_ROOT))
    }
    fn attachment_repo(&self) -> Option<&otto_state::ProductAttachmentRepo> {
        Some(&self.attachment_repo)
    }
    fn workspace_root<'a>(
        &'a self,
        ws: &'a Id,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send + 'a>> {
        Box::pin(async move { self.workspaces.get(ws).await.ok().map(|w| w.root_path) })
    }
    fn stop_story_agents<'a>(
        &'a self,
        story_id: &'a Id,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let analyses = self
                .product_repo
                .list_analyses(story_id)
                .await
                .unwrap_or_default();
            for a in analyses {
                let agents = self
                    .product_repo
                    .list_analysis_agents(&a.id)
                    .await
                    .unwrap_or_default();
                for ag in agents {
                    if !matches!(ag.status.as_str(), "running" | "waiting" | "pending") {
                        continue;
                    }
                    otto_product::run::signal_cancel(&self.product_agent_cancels, &ag.id);
                    if let Some(sid) = ag.session_id.as_ref() {
                        let _ = self.manager.kill_session(sid).await;
                    }
                }
            }
        })
    }
}

impl otto_memory::MemoryCtx for ServerCtx {
    fn memory(&self) -> &Arc<otto_memory::MemoryService> {
        &self.memory
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

impl otto_vault::VaultCtx for ServerCtx {
    fn vault(&self) -> &Arc<otto_vault::VaultEngine> {
        &self.vault
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

impl otto_canvas::CanvasCtx for ServerCtx {
    fn canvas_repo(&self) -> &otto_state::CanvasRepo {
        &self.canvas_repo
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

impl otto_canvas::CanvasAssistCtx for ServerCtx {
    fn workspaces(&self) -> &otto_state::WorkspacesRepo {
        &self.workspaces
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
        &self.events
    }
    fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
    async fn resolve_provider(
        &self,
        ws: Option<&otto_core::domain::Workspace>,
        requested: Option<&str>,
    ) -> otto_core::Result<String> {
        ServerCtx::resolve_provider(self, ws, requested).await
    }
    fn ensure_trusted(&self, provider: &str, cwd: &str) {
        otto_sessions::trust::ensure_trusted(provider, cwd);
    }
    async fn run_agent_turn<F: FnOnce(&otto_core::Id) + Send>(
        &self,
        t: otto_canvas::AgentTurn<'_>,
        on_ready: F,
    ) -> otto_core::Result<(String, otto_core::Id)> {
        crate::agent_session::run_session_turn(
            self,
            t.ws,
            t.user,
            t.existing,
            t.title,
            t.cwd,
            t.provider,
            t.meta,
            t.prompt,
            crate::agent_session::STUCK_IDLE,
            on_ready,
        )
        .await
        .map_err(|e| e.0)
    }
    async fn kill_session(&self, sid: &otto_core::Id) -> otto_core::Result<()> {
        self.manager.kill_session(sid).await
    }
}

impl otto_insights::InsightsCtx for ServerCtx {
    fn library(&self) -> &otto_context::Library {
        &self.context_library
    }
    fn manager(&self) -> &Arc<otto_sessions::SessionManager> {
        &self.manager
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
        &self.events
    }
    fn workspaces(&self) -> &otto_state::WorkspacesRepo {
        &self.workspaces
    }
    async fn resolve_provider(
        &self,
        ws: Option<&otto_core::domain::Workspace>,
        requested: Option<&str>,
    ) -> otto_core::Result<String> {
        ServerCtx::resolve_provider(self, ws, requested).await
    }
    async fn submit_prompt(&self, sid: &otto_core::Id, prompt: &str) -> bool {
        if !crate::review_session::wait_for_tui(&self.manager, sid).await {
            return false;
        }
        let _ = self
            .manager
            .input(sid, &crate::review_session::bracketed_paste(prompt))
            .await;
        tokio::time::sleep(crate::review_session::PASTE_TO_ENTER).await;
        // Same swallowed-Enter guard as the assistant path (S4-19c): a TUI
        // still digesting the paste can eat the first Enter, leaving the run
        // idle behind a "generating" banner for the whole timeout.
        let before = self.manager.live_handle(sid).map(|h| h.last_output_at());
        let _ = self.manager.input(sid, b"\r").await;
        if !crate::review_session::dispatched(&self.manager, sid, before).await {
            let _ = self.manager.input(sid, b"\r").await;
        }
        true
    }
}

impl otto_design::DesignCtx for ServerCtx {
    /// A per-request handle over the shared pool, `<data>/design` and the
    /// event bus — no ServerCtx field, so the test harnesses that build a
    /// literal ServerCtx are unaffected.
    fn design(&self) -> otto_design::DesignService {
        crate::design_hall::service(self)
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn validate_content(&self, format: &str, bytes: &[u8]) -> otto_core::Result<()> {
        crate::design_hall::validate_content(format, bytes)
    }
}

impl otto_design_assist::DesignAssistCtx for ServerCtx {
    fn events(&self) -> &tokio::sync::broadcast::Sender<otto_core::event::Event> {
        &self.events
    }
    fn workspaces(&self) -> &otto_state::WorkspacesRepo {
        &self.workspaces
    }
    fn product_repo(&self) -> &otto_state::ProductRepo {
        &self.product_repo
    }
    fn improve_engine(&self) -> &Arc<otto_improve::ImprovementEngine> {
        &self.improve_engine
    }
    fn memory(&self) -> &Arc<otto_memory::MemoryService> {
        &self.memory
    }
    async fn resolve_provider_or_fallback(
        &self,
        ws: Option<&otto_core::domain::Workspace>,
        requested: Option<&str>,
        site: &'static str,
    ) -> String {
        ServerCtx::resolve_provider_or_fallback(self, ws, requested, site).await
    }
    async fn run_agent_turn<F: FnOnce(&otto_core::Id) + Send>(
        &self,
        t: otto_design_assist::AgentTurn<'_>,
        on_ready: F,
    ) -> otto_core::Result<(String, otto_core::Id)> {
        crate::agent_session::run_session_turn_with(
            self,
            t.ws,
            t.user,
            t.existing,
            t.title,
            t.cwd,
            t.provider,
            t.meta,
            t.prompt,
            t.stuck_after,
            crate::agent_session::TurnOpts {
                done_file: t.done_file,
                quiet_done: t.quiet_done,
                ..Default::default()
            },
            on_ready,
        )
        .await
        .map_err(|e| e.0)
    }
    async fn kill_session(&self, sid: &otto_core::Id) -> otto_core::Result<()> {
        self.manager.kill_session(sid).await
    }
}

impl otto_swarm::SwarmCtx for ServerCtx {
    fn swarm(&self) -> &Arc<otto_swarm::SwarmService> {
        &self.swarm
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn events(&self) -> &tokio::sync::broadcast::Sender<Event> {
        &self.events
    }
    fn available_providers(&self) -> Vec<String> {
        // Agent-capable providers from the live registry (exclude the plain shell).
        self.manager
            .providers()
            .names()
            .into_iter()
            .filter(|n| n != "shell")
            .collect()
    }
    fn product_repo(&self) -> Option<&otto_state::ProductRepo> {
        Some(&self.product_repo)
    }
}

// ---------------------------------------------------------------------------
// Connection spawner: ConnectionsService -> SessionManager bridge
// ---------------------------------------------------------------------------

/// Spawns connection sessions through the [`SessionManager`], writing the
/// profile's first command into the PTY shortly after connect.
pub struct PtySpawner {
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    pub pool: otto_state::DbPool,
}

impl Spawner for PtySpawner {
    fn spawn_connection<'a>(
        &'a self,
        ws_id: &'a Id,
        user_id: &'a Id,
        conn: &'a Connection,
        spec: CommandSpec,
        first_command: Option<String>,
        title: Option<String>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let ws = self.workspaces.get(ws_id).await?;
            let req = CreateSessionReq {
                kind: SessionKind::Connection,
                provider: Some(conn.kind.as_str().to_string()),
                title: title.or_else(|| Some(conn.name.clone())),
                cwd: None,
                connection_id: Some(conn.id.clone()),
                model: None,
                meta: None,
            };
            let session = self.manager.create(&ws, user_id, req, Some(spec)).await?;

            if let Some(cmd) = first_command {
                let manager = Arc::clone(&self.manager);
                let session_id = session.id.clone();
                let user_id = user_id.clone();
                let pool = self.pool.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    let result: Result<()> = async {
                        let current = manager.get(&session_id).await?;
                        input_user(&pool, &user_id, &current).await?;
                        manager
                            .human_input(
                                &session_id,
                                &user_id,
                                false,
                                true,
                                format!("{cmd}\n").as_bytes(),
                            )
                            .await
                    }
                    .await;
                    if let Err(e) = result {
                        tracing::warn!(session = %session_id, "first_command denied or failed: {e}");
                    }
                });
            }
            Ok(session)
        })
    }

    fn spawn_command<'a>(
        &'a self,
        ws_id: &'a Id,
        user_id: &'a Id,
        provider: &'a str,
        spec: CommandSpec,
        title: String,
        meta: Option<serde_json::Value>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(self.spawn_adhoc(ws_id, user_id, provider, spec, title, meta))
    }
}

// ---------------------------------------------------------------------------
// Orchestrator routes (contract #23, #24)
// ---------------------------------------------------------------------------

/// Routes: POST /workspaces/{id}/orchestrate, .../orchestrate/execute, and the
/// dedicated AI-free .../broadcast.
pub fn orchestrator_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/workspaces/{id}/orchestrate", post(orchestrate))
        .route(
            "/workspaces/{id}/orchestrate/execute",
            post(orchestrate_execute),
        )
        .route("/workspaces/{id}/broadcast", post(workspace_broadcast))
        .route("/workspaces/{id}/relay", post(workspace_relay))
        .route("/workspaces/{id}/sessions/open", post(open_agent_session))
        .route(
            "/workspaces/{id}/product/stories/{sid}/analyze",
            post(otto_product::analysis::analyze::<ServerCtx>),
        )
        // Curated analysis-lens catalog the Analysis tab renders as configurable
        // checks. Read-only (Viewer); the prefix policy gates it as Product/View.
        .route(
            "/workspaces/{id}/product/lenses",
            get(otto_product::analysis::product_lenses::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/rewrite",
            post(otto_product::analysis::rewrite::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/testcases/generate",
            post(otto_product::analysis::generate_tests::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/plan/generate",
            post(otto_product::analysis::generate_plan::<ServerCtx>),
        )
        .route(
            "/workspaces/{id}/product/stories/{sid}/plan",
            post(otto_product::analysis::save_plan::<ServerCtx>),
        )
        // Product → Swarm: turn a refined story into a runnable swarm project.
        // Flat item route (resolves the workspace from the owning story).
        .route(
            "/product/stories/{sid}/to-swarm",
            post(otto_product::swarm::story_to_swarm::<ServerCtx>),
        )
        // Discovery: launch a repeatable INVESTIGATION swarm from a story (Editor),
        // then list/read the runs (Viewer). Discovery projects are NOT story-linked
        // (the unique story_id index is reserved for the implementation project);
        // the run row carries the linkage.
        .route(
            "/product/stories/{sid}/discover",
            post(otto_product::swarm::discover_story::<ServerCtx>),
        )
        .route(
            "/product/stories/{sid}/discovery-runs",
            get(otto_product::swarm::list_discovery_runs::<ServerCtx>),
        )
        // Product → Canvas: list the Canvas scenes linked to a story (Viewer).
        .route(
            "/product/stories/{sid}/linked-canvases",
            get(otto_product::swarm::list_linked_canvases::<ServerCtx>),
        )
        .route(
            "/product/discovery-runs/{rid}",
            get(otto_product::swarm::get_discovery_run::<ServerCtx>),
        )
        // Talk-to-agent refinement: a conversational thread on a story. Create +
        // list its threads (story-scoped), then read/converse/archive a thread
        // (resolves the workspace from the thread → its story). The converse turn
        // runs the agent inline and may write a new `suggested` story version.
        .route(
            "/product/stories/{sid}/refinement-threads",
            post(otto_product::refine::create_thread::<ServerCtx>)
                .get(otto_product::refine::list_threads::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}",
            get(otto_product::refine::get_thread::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}/messages",
            post(otto_product::refine::send_message::<ServerCtx>),
        )
        .route(
            "/product/refinement-threads/{tid}/archive",
            post(otto_product::refine::archive_thread::<ServerCtx>),
        )
        // Discovery Chat: a lightweight conversational agent on a story (works
        // from an empty draft) for early discovery/research. Each turn assembles
        // a relevance-bounded context bundle and may propose actions the user
        // applies explicitly. Covered by the `/product/` policy prefix.
        .route(
            "/product/stories/{sid}/discovery-chats",
            post(otto_product::chat::create_chat::<ServerCtx>)
                .get(otto_product::chat::list_chats::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}",
            get(otto_product::chat::get_chat::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/messages",
            post(otto_product::chat::send_message::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/archive",
            post(otto_product::chat::archive_chat::<ServerCtx>),
        )
        .route(
            "/product/discovery-chats/{cid}/apply",
            post(otto_product::chat::apply_action::<ServerCtx>),
        )
        // Canvas agent-assist: turn a prompt into diagram blocks. The engine is
        // otto-canvas's; the agent turn runs through `CanvasAssistCtx` (above).
        .route(
            "/canvas/scenes/{id}/assist",
            post(otto_canvas::assist::assist_scene::<ServerCtx>),
        )
        .route(
            "/canvas/assist/preview",
            post(otto_canvas::assist::assist_preview::<ServerCtx>),
        )
        // Session ↔ Canvas scene references — needs the SessionManager to resolve
        // a session's workspace, so it lives here rather than in otto-canvas.
        .merge(crate::canvas_refs::canvas_refs_routes())
        // Story attachments — upload route gets its own 40 MB body cap to bound
        // the ~33 % base64 inflation (raw content cap is enforced at 25 MB).
        .route(
            "/product/stories/{sid}/attachments",
            post(otto_product::media::upload_attachment::<ServerCtx>)
                .layer(DefaultBodyLimit::max(40 * 1024 * 1024))
                .get(otto_product::media::list_attachments::<ServerCtx>),
        )
        .route(
            "/product/attachments/{aid}",
            get(otto_product::media::serve_attachment::<ServerCtx>)
                .patch(otto_product::media::patch_attachment::<ServerCtx>)
                .delete(otto_product::media::delete_attachment::<ServerCtx>),
        )
        // Design arena: save an edited artifact from the UI editor. Same 40 MB
        // body cap as the upload (base64 inflation over the 25 MB raw cap).
        .route(
            "/product/attachments/{aid}/content",
            put(otto_product::media::put_attachment_content::<ServerCtx>)
                .layer(DefaultBodyLimit::max(40 * 1024 * 1024)),
        )
        // In-place design agent: generate / refine an artifact (html | mermaid |
        // excalidraw | scene3d) with a live, file-backed agent session (hidden
        // from the Agents list).
        .route(
            "/product/stories/{sid}/mockups/assist",
            post(crate::mockup_assist::assist_mockup),
        )
        // Blender bridge (optional, detected): status, headless render job,
        // job poll, generated-script download.
        .route(
            "/product/design/blender",
            get(crate::design_blender::blender_status),
        )
        .route(
            "/product/design/jobs/{id}",
            get(crate::design_blender::get_job),
        )
        .route(
            "/product/stories/{sid}/design/{aid}/blender-render",
            post(crate::design_blender::blender_render),
        )
        .route(
            "/product/stories/{sid}/design/{aid}/blender-script",
            get(crate::design_blender::blender_script),
        )
        .route(
            "/product/attachments/{aid}/annotations",
            get(otto_product::media::list_annotations::<ServerCtx>)
                .post(otto_product::media::create_annotation::<ServerCtx>),
        )
        .route(
            "/product/annotations/{id}",
            patch(otto_product::media::patch_annotation::<ServerCtx>)
                .delete(otto_product::media::delete_annotation::<ServerCtx>),
        )
        // Approve lives here (not in otto-product) so it can trigger self-improvement.
        .route(
            "/product/testcase-runs/{rid}/approve",
            post(otto_product::analysis::approve_testcase_run::<ServerCtx>),
        )
        // Per-agent retry: re-run a single failed/stuck analysis lens agent.
        .route(
            "/product/analyses/{aid}/agents/{agent_id}/retry",
            post(otto_product::analysis::retry_analysis_agent::<ServerCtx>),
        )
        // Per-agent stop: kill a running/waiting analysis agent on demand.
        .route(
            "/product/analyses/{aid}/agents/{agent_id}/stop",
            post(otto_product::analysis::stop_analysis_agent::<ServerCtx>),
        )
}

async fn orchestrate(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<OrchestrateReq>,
) -> ApiResult<Json<OrchestrateResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    // The planner spawns a real claude session in the workspace root —
    // pre-trust the folder so the PTY never stalls on the trust dialog.
    otto_sessions::trust::ensure_trusted("claude", &ws.root_path);
    let octx = orchestrator_context(&ctx, Typist::new(&user.id, &auth), &ws_id, ws).await?;
    let resp = ctx
        .orchestrator
        .orchestrate(req, octx)
        .await
        .map_err(ApiError)?;
    Ok(Json(resp))
}

/// The workspace facts a plan is planned AND validated against: the caller's
/// live sessions + connections and the live provider registry.
async fn orchestrator_context(
    ctx: &ServerCtx,
    who: Typist<'_>,
    ws_id: &Id,
    ws: Workspace,
) -> ApiResult<OrchestratorContext> {
    let sessions = input_agents(ctx, who, ws_id).await.map_err(ApiError)?;
    let connections = ctx
        .connections
        .list_for(ws_id, who.user_id)
        .await
        .map_err(ApiError)?;
    // Effective default agent for this workspace: per-workspace setting, else
    // the global default, else "claude". Steers spawn_sessions in the planner.
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    Ok(OrchestratorContext {
        sessions,
        connections,
        cwd: ws.root_path,
        default_provider,
        // Live registry (builtins + custom providers) so the planner can
        // spawn any configured provider by name, e.g. "open grok session".
        available_providers: ctx.manager.providers().names(),
    })
}

async fn orchestrate_execute(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ExecutePlanReq>,
) -> ApiResult<Json<ExecuteResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    // The plan comes back from the client, which may have edited (or forged)
    // it since `/orchestrate` validated it — validate again against the live
    // workspace before anything is spawned or typed.
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    let octx = orchestrator_context(&ctx, Typist::new(&user.id, &auth), &ws_id, ws).await?;
    otto_orchestrator::parse::validate_plan(&req.plan, &octx, &octx.allowed_providers())
        .map_err(ApiError)?;
    let helper = ExecHelper {
        ctx: ctx.clone(),
        ws_id: ws_id.clone(),
        agent: crate::feature_guard::agent_session_of(&auth).cloned(),
        user,
    };
    Ok(Json(execute(&req.plan, &helper, &helper).await))
}

/// `POST /workspaces/{id}/broadcast` — relay a literal message to live agent
/// sessions. Dedicated, AI-free path: no parsing, no orchestrator, no fallback.
/// `session_ids` (when present) targets a subset; absent/empty hits all live
/// agents. Returns the sessions that actually received it.
async fn workspace_broadcast(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::BroadcastReq>,
) -> ApiResult<Json<otto_core::api::BroadcastResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let text = req.text.trim();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("broadcast text is empty".into())));
    }
    // Treat an empty target list the same as "no targets" → broadcast to all.
    let targets = req.session_ids.filter(|ids| !ids.is_empty());
    let who = Typist::new(&user.id, &auth);
    let session_ids = broadcast_sessions(&ctx, who, &ws_id, text, targets.as_deref())
        .await
        .map_err(ApiError)?;
    Ok(Json(otto_core::api::BroadcastResp { session_ids }))
}

/// `POST /workspaces/{id}/relay` — deliver a name-addressed message ("ronaldo:
/// do X", "ronaldo, messi: ship it", "all: stand down"). Resolves the leading
/// address against the workspace's live agent sessions; when nothing matches,
/// returns `unaddressed = true` (delivers nothing) so the caller can fall back.
/// Editor role.
async fn workspace_relay(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::RelayReq>,
) -> ApiResult<Json<otto_core::api::RelayResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let text = req.text.trim();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("relay text is empty".into())));
    }
    let who = Typist::new(&user.id, &auth);
    let candidates = input_agents(&ctx, who, &ws_id).await.map_err(ApiError)?;
    let addressable: Vec<_> = candidates
        .iter()
        .map(|s| otto_sessions::names::Addressable {
            id: s.id.clone(),
            handle: s
                .meta
                .get("name_handle")
                .and_then(Value::as_str)
                .unwrap_or(&s.title)
                .to_string(),
            full: s
                .meta
                .get("name_full")
                .and_then(Value::as_str)
                .unwrap_or(&s.title)
                .to_string(),
            title: s.title.clone(),
        })
        .collect();
    let resolved = otto_sessions::names::resolve_address(text, &addressable);
    let mut session_ids = Vec::new();
    for id in &resolved.targets {
        if submit_session_text(&ctx, who, id, resolved.text.trim())
            .await
            .is_ok()
        {
            session_ids.push(id.clone());
        }
    }
    Ok(Json(otto_core::api::RelayResp {
        session_ids,
        broadcast: resolved.broadcast,
        unaddressed: resolved.targets.is_empty(),
        text: if resolved.targets.is_empty() {
            text.to_string()
        } else {
            resolved.text
        },
    }))
}

/// Per-request plan executor scoped to one workspace and acting user.
struct ExecHelper {
    ctx: ServerCtx,
    ws_id: Id,
    user: User,
    /// The caller's own agent session when the plan arrived on an agent
    /// credential: its broadcast/command steps reach only that session and its
    /// workers (S11-305).
    agent: Option<Id>,
}

impl ExecHelper {
    fn typist(&self) -> Typist<'_> {
        Typist {
            user_id: &self.user.id,
            agent: self.agent.as_ref(),
        }
    }
}

impl PlanSpawner for ExecHelper {
    fn spawn_agent<'a>(&'a self, provider: &'a str) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let ws = self.ctx.workspaces.get(&self.ws_id).await?;
            let req = CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some(provider.to_string()),
                title: None,
                cwd: None,
                connection_id: None,
                model: None,
                meta: None,
            };
            self.ctx.manager.create(&ws, &self.user.id, req, None).await
        })
    }

    fn open_connection<'a>(&'a self, connection_id: &'a Id) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let conn = self.ctx.connections.get(connection_id).await?;
            let visible = match &conn.workspace_id {
                None => true,
                Some(ws) => *ws == self.ws_id,
            };
            if !visible {
                return Err(Error::Forbidden(
                    "connection belongs to another workspace".into(),
                ));
            }
            self.ctx
                .connections
                .open(
                    &conn,
                    &self.ws_id,
                    &self.user.id,
                    None,
                    self.ctx.spawner.as_ref(),
                )
                .await
        })
    }
}

impl PlanIo for ExecHelper {
    fn broadcast<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<Vec<Id>>> {
        // Funnel through the one shared implementation so the AI/palette path and
        // the dedicated /broadcast endpoint can't drift. `None` = all live agents.
        Box::pin(async move {
            broadcast_sessions(&self.ctx, self.typist(), &self.ws_id, text, None).await
        })
    }

    fn run_command<'a>(&'a self, session_id: &'a Id, text: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let session = self.ctx.manager.get(session_id).await?;
            if session.workspace_id != self.ws_id {
                return Err(Error::Forbidden(
                    "session belongs to another workspace".into(),
                ));
            }
            // Submit as a real keypress (paste + Enter), not "{text}\n" in one
            // burst — otherwise bracketed-paste TUIs paste the text but never send.
            submit_session_text(&self.ctx, self.typist(), session_id, text).await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// PR review agent routes
// ---------------------------------------------------------------------------

/// Helper: build provider + remote ref from a repo row in ServerCtx.
///
/// S4: the repo's bound git credential may be *used* only by its owner (or root).
/// A workspace can have many members but a repo binds exactly one account, so the
/// Editor role-check on the repo is not sufficient to stop user B from opening /
/// drafting PRs through user A's hosting token — authorize the owner here.
pub(crate) async fn resolve_provider_remote(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    repo: &otto_core::domain::Repo,
) -> Result<(Arc<dyn otto_git::GitProvider>, otto_git::RemoteRef)> {
    let _kind = repo
        .provider
        .ok_or_else(|| Error::Invalid("repo has no git provider".into()))?;
    let account_id = repo
        .git_account_id
        .as_ref()
        .ok_or_else(|| Error::Invalid("repo has no git account".into()))?;
    let account = ctx.git_store.get_account(account_id).await?;
    otto_core::auth::authorize_owner(&account, user)?;
    let remote_url = repo
        .remote_url
        .as_deref()
        .ok_or_else(|| Error::Invalid("repo has no remote url".into()))?;
    let (_, remote_ref) = otto_git::detect(remote_url)
        .ok_or_else(|| Error::Invalid(format!("unsupported remote: {remote_url}")))?;
    let token = otto_core::secrets::get_async(&ctx.secrets, &account.token_ref)
        .await?
        .ok_or_else(|| Error::Invalid(format!("token missing for git account {}", account.id)))?;
    Ok((otto_git::make_provider(&account, token), remote_ref))
}

/// Materialize complete skill packages into `bundle`, with two views:
/// `.claude/skills/<name>/` for Claude's first-class `--add-dir` loader and
/// `skills/<name>/` for provider-neutral, explicit file reads from prompts.
/// References, scripts, examples, assets, and eval metadata travel with
/// `SKILL.md`; no provider is limited to a lossy body-only copy.
///
/// Why a dedicated bundle (not the repo cwd): review sessions deliberately skip
/// context materialization (see `SessionManager` spawn), and a reviewed repo
/// usually has no `.claude/skills` of its own, so `Skill(<lens>)` otherwise errors
/// "Unknown skill". `--add-dir=<dir>` loading `<dir>/.claude/skills` is verified
/// against the live CLI. The bundle holds ONLY review skills — never the data dir
/// (secrets/DB) — so add-dir grants no sensitive access.
///
/// Sources, per skill: the Otto Library, the operator's global Claude skills,
/// then the compiled-in `otto-skills` tree. Re-copied cleanly so edits and
/// bundled upgrades propagate. Best-effort: unknown skills are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedSkillPackages {
    pub(crate) root: String,
    pub(crate) files: std::collections::HashMap<String, Vec<String>>,
}

pub(crate) fn stage_skill_packages_at(
    library: &otto_context::Library,
    names: &[String],
    bundle: &std::path::Path,
) -> Option<StagedSkillPackages> {
    use std::path::Path;
    let neutral_root = bundle.join("skills");
    let claude_root = bundle.join(".claude").join("skills");
    let mut files = std::collections::HashMap::new();
    let mut seen = std::collections::HashSet::<&str>::new();
    for name in names {
        // Path::join replaces its base for absolute paths. Validate before any
        // destination join or cleanup so a name can never escape `bundle`.
        if !is_safe_skill_package_name(name) || !seen.insert(name.as_str()) {
            continue;
        }
        // Resolve the skill's on-disk source dir: Library first (skill_path()
        // rejects unsafe names), then the operator's global Claude skills dir.
        let src = library
            .skill_path(name)
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .filter(|d| d.is_dir())
            .or_else(|| {
                if name.contains(['/', '\\']) || name.contains("..") {
                    return None;
                }
                let home = std::env::var_os("HOME")?;
                let d = Path::new(&home).join(".claude/skills").join(name);
                d.is_dir().then_some(d)
            });
        let neutral = neutral_root.join(name);
        if let Err(e) = remove_staged_package_path(&neutral) {
            tracing::warn!(skill = %name, "stage_skill_packages: neutral cleanup failed: {e}");
            continue;
        }
        let copied = match src {
            Some(src) => crate::plugins::copy_dir(&src, &neutral).map(|_| true),
            None => otto_skills::copy_bundled_into(name, &neutral).map_err(|e| e.to_string()),
        };
        match copied {
            Ok(true) => {}
            Ok(false) => {
                let _ = remove_staged_package_path(&neutral);
                continue;
            }
            Err(e) => {
                let _ = remove_staged_package_path(&neutral);
                tracing::warn!(skill = %name, "stage_skill_packages: copy failed: {e}");
                continue;
            }
        }
        let claude = claude_root.join(name);
        if let Err(e) = remove_staged_package_path(&claude) {
            let _ = remove_staged_package_path(&neutral);
            tracing::warn!(skill = %name, "stage_skill_packages: claude cleanup failed: {e}");
            continue;
        }
        if let Err(e) = crate::plugins::copy_dir(&neutral, &claude) {
            let _ = remove_staged_package_path(&neutral);
            let _ = remove_staged_package_path(&claude);
            tracing::warn!(skill = %name, "stage_skill_packages: claude view failed: {e}");
            continue;
        }
        let mut package_files = Vec::new();
        collect_staged_package_files(&neutral, &neutral, &mut package_files);
        package_files.sort();
        package_files.dedup();
        files.insert(name.clone(), package_files);
    }
    (!files.is_empty()).then(|| StagedSkillPackages {
        root: bundle.to_string_lossy().into_owned(),
        files,
    })
}

fn is_safe_skill_package_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[allow(clippy::disallowed_methods)] // pre-existing sync fs reached from async code without offload (perf2 N3 follow-up)
fn remove_staged_package_path(path: &std::path::Path) -> std::result::Result<(), String> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            std::fs::remove_dir_all(path).map_err(|e| format!("remove {}: {e}", path.display()))
        }
        Ok(_) => std::fs::remove_file(path).map_err(|e| format!("remove {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("inspect {}: {e}", path.display())),
    }
}

#[allow(clippy::disallowed_methods)] // pre-existing sync fs reached from async code without offload (perf2 N3 follow-up)
fn collect_staged_package_files(
    dir: &std::path::Path,
    root: &std::path::Path,
    out: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_staged_package_files(&path, root, out);
        } else if kind.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
}

/// Backward-compatible shared bundle for the PR/skill review engine. Vault
/// docs runs use [`stage_skill_packages_at`] with a run-specific directory.
pub(crate) fn stage_review_skills(
    library: &otto_context::Library,
    names: &[String],
) -> Option<String> {
    let bundle = otto_context::materialize::default_context_root().join("review-skills");
    stage_skill_packages_at(library, names, &bundle).map(|staged| staged.root)
}

/// Register (or replace) the cancel flag for one review agent. Returns the
/// fresh, un-tripped flag — replacing matters on retry, where a stale tripped
/// flag would short-circuit the new run instantly.
fn register_review_agent_cancel(
    reg: &crate::skill_eval::CancelRegistry,
    review_id: &str,
    index: usize,
) -> Arc<std::sync::atomic::AtomicBool> {
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut m) = reg.lock() {
        m.insert(review_agent_cancel_key(review_id, index), flag.clone());
    }
    flag
}

/// Trip one review agent's cancel flag, if registered (no-op otherwise — e.g.
/// a seeded/orphaned row with no live recovery loop).
fn signal_review_agent_cancel(
    reg: &crate::skill_eval::CancelRegistry,
    review_id: &str,
    index: usize,
) {
    if let Ok(m) = reg.lock() {
        if let Some(f) = m.get(&review_agent_cancel_key(review_id, index)) {
            f.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

/// Drop one review agent's cancel flag from the registry (run is over).
fn unregister_review_agent_cancel(
    reg: &crate::skill_eval::CancelRegistry,
    review_id: &str,
    index: usize,
) {
    if let Ok(mut m) = reg.lock() {
        m.remove(&review_agent_cancel_key(review_id, index));
    }
}

/// Load ReviewConfig from settings or fall back to the default. The default
/// config's reviewer agents follow the global default agent
/// (`default_provider` setting, else "claude"); a stored config is used as-is.
async fn load_review_config(ctx: &ServerCtx) -> ReviewConfig {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let global_default = repo.get("default_provider").await.ok().flatten();
    let default_provider =
        otto_core::provider::resolve_provider(&[otto_core::provider::global_default(
            global_default.as_ref(),
        )]);
    match repo.get("pr_review").await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_else(|e| {
            tracing::warn!("failed to deserialize pr_review config: {e}; using default");
            default_review_config(&default_provider)
        }),
        _ => default_review_config(&default_provider),
    }
}

/// Load the named review-config presets (settings key `pr_review_presets`).
/// A missing/corrupt value is an empty list, never an error.
async fn load_review_presets(ctx: &ServerCtx) -> Vec<ReviewConfigPreset> {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    match repo.get("pr_review_presets").await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_else(|e| {
            tracing::warn!("failed to deserialize pr_review_presets: {e}; using empty");
            Vec::new()
        }),
        _ => Vec::new(),
    }
}

/// Load the stored per-repo binding, if any. Corrupt values read as `None` so a
/// bad row can never brick reviews for the repo.
async fn load_repo_review_binding(ctx: &ServerCtx, repo_id: &Id) -> Option<RepoReviewBinding> {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    match repo.get(&repo_review_binding_key(repo_id)).await {
        Ok(Some(v)) => serde_json::from_value(v)
            .map_err(|e| {
                tracing::warn!(repo = %repo_id, "failed to deserialize repo review binding: {e}");
            })
            .ok(),
        _ => None,
    }
}

/// Resolve the EFFECTIVE review config for a repo: inline per-repo config >
/// per-repo preset reference > global `pr_review` config. A dangling preset
/// reference (preset deleted) falls back to global with a warn — reviews must
/// never fail because a preset went away.
async fn load_review_config_for_repo(ctx: &ServerCtx, repo_id: &Id) -> ReviewConfig {
    if let Some(binding) = load_repo_review_binding(ctx, repo_id).await {
        if let Some(cfg) = binding.config {
            return cfg;
        }
        if let Some(pid) = binding.preset_id {
            if let Some(p) = load_review_presets(ctx)
                .await
                .into_iter()
                .find(|p| p.id == pid)
            {
                return p.config;
            }
            tracing::warn!(repo = %repo_id, preset = %pid, "repo review preset missing; using global config");
        }
    }
    load_review_config(ctx).await
}

/// The review mode a `review_run` step runs with when neither the run
/// input nor the node set one: the repo's effective stored config (or the
/// global one when `repo_id` is `None`), tagged with its source for the
/// step log.
pub(crate) async fn effective_review_mode(
    ctx: &ServerCtx,
    repo_id: Option<&Id>,
) -> (otto_core::domain::ReviewMode, &'static str) {
    let cfg = match repo_id {
        Some(id) => load_review_config_for_repo(ctx, id).await,
        None => load_review_config(ctx).await,
    };
    mode_source(&cfg)
}

/// How long a caller that BLOCKS on a review (Run with Otto's review stage)
/// may wait before declaring it overdue: the per-agent grace period the run
/// will actually use (config override / diff heuristic, orchestrator-scaled)
/// plus the summarizer's ceiling plus slack for spawn + persistence. Waiting
/// any less reads the finding counts mid-run and reports a false "0 findings".
pub(crate) async fn review_wait_budget(ctx: &ServerCtx, repo_id: &Id, diff_len: usize) -> Duration {
    let cfg = load_review_config_for_repo(ctx, repo_id).await;
    review_wait_budget_for(&cfg, diff_len)
}

#[allow(clippy::too_many_arguments)]
async fn run_review(
    ctx: ServerCtx,
    review_id: Id,
    repo_path: String,
    diff_text: String,
    jira_context: Option<String>,
    user_context: Option<String>,
    workspace: Workspace,
    repo_id: Id,
    pr_number: u64,
    branches: Option<ReviewBranches>,
    cfg_override: Option<ReviewConfig>,
    // Per-run/per-node execution mode (`review_run`). `None` ⇒ the stored
    // config decides, then `FanOut`.
    mode_override: Option<otto_core::domain::ReviewMode>,
) {
    let attempt_cancel = ctx.review_cancels.lock().ok().map(|mut map| {
        map.entry(review_id.clone())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .clone()
    });
    // Unregister this run's flag on every exit path (incl. the early return).
    let _cancel_guard = attempt_cancel
        .as_ref()
        .map(|f| ReviewCancelGuard::new(&ctx.review_cancels, &review_id, f));
    let result = run_review_core(
        &ctx,
        &review_id,
        &repo_path,
        diff_text,
        jira_context,
        user_context,
        &workspace,
        &repo_id,
        pr_number,
        branches.as_ref(),
        cfg_override,
        mode_override,
    )
    .await;
    if attempt_cancel
        .as_ref()
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
        || review_is_cancelled(&ctx, &review_id).await
    {
        return;
    }
    // The run is over (done or error): its temp artifacts are spent.
    remove_review_temp_files(&review_id).await;
    match result {
        Ok(()) => {
            tracing::info!(review = %review_id, "review complete");
            if let Err(e) = ctx
                .reviews_store
                .set_status(&review_id, ReviewStatus::Done, None)
                .await
            {
                tracing::error!(review = %review_id, "set status done: {e}");
            }
            let _ = ctx.events.send(Event::ReviewChanged {
                workspace_id: workspace.id.clone(),
                session_id: None,
                review_id: review_id.clone(),
                status: ReviewStatus::Done.as_str().to_string(),
            });
        }
        Err(e) => {
            tracing::warn!(review = %review_id, "review error: {e}");
            // The core only reaches its own teardown on the summarize path; an
            // error anywhere else would otherwise leave the reviewers' PTYs live.
            if let Ok(review) = ctx.reviews_store.get_review(&review_id).await {
                let ids: Vec<String> = review
                    .agents
                    .iter()
                    .filter_map(|a| a.session_id.clone())
                    .collect();
                crate::review_session::stop_review_sessions(&ctx.manager, &ids).await;
            }
            let msg = e.to_string();
            let _ = ctx
                .reviews_store
                .set_status(&review_id, ReviewStatus::Error, Some(&msg))
                .await;
            let _ = ctx.events.send(Event::ReviewChanged {
                workspace_id: workspace.id.clone(),
                session_id: None,
                review_id: review_id.clone(),
                status: ReviewStatus::Error.as_str().to_string(),
            });
        }
    }
}

/// Remove a review's `$TMPDIR/otto-review-<id>*` files (diff, per-agent
/// prompts, findings JSON). A finished run's durable copies live in the DB
/// (0100) and a retry re-materializes what it needs, so leaving them behind
/// only grew the temp dir (100+ files per few days of reviews).
pub(crate) async fn remove_review_temp_files(review_id: &str) {
    let prefix = format!("otto-review-{review_id}");
    let () = crate::offload::blocking(move || {
        remove_review_temp_files_in(&std::env::temp_dir(), &prefix)
    })
    .await;
}

/// [`remove_review_temp_files`] over an explicit dir — FILES only, so the
/// `otto-review-wt-<id>` worktree directory is never touched.
#[allow(clippy::disallowed_methods)] // sync helper: runs on the blocking pool via offload::blocking (or a test)
pub(crate) fn remove_review_temp_files_in(dir: &std::path::Path, prefix: &str) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if entry.file_name().to_string_lossy().starts_with(prefix)
                && entry.file_type().is_ok_and(|t| t.is_file())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Core fallible review logic: given a unified diff text, optional Jira
/// context, and optional free-text user guidance, runs the configured agents,
/// stores comments, and updates live agent-state rows. Used by both the PR and
/// local-review flows.
#[allow(clippy::too_many_arguments)]
async fn run_review_core(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    diff_text: String,
    jira_context: Option<String>,
    user_context: Option<String>,
    workspace: &Workspace,
    repo_id: &Id,
    pr_number: u64,
    branches: Option<&ReviewBranches>,
    cfg_override: Option<ReviewConfig>,
    mode_override: Option<otto_core::domain::ReviewMode>,
) -> Result<()> {
    let jira_ctx = jira_context.unwrap_or_default();
    // Build the optional free-text guidance block (prepended to every agent
    // prompt before the Jira block). Empty/absent => no block, identical to old
    // behaviour.
    let user_ctx = match user_context {
        Some(c) if !c.trim().is_empty() => {
            format!("## Reviewer guidance\n{}\n\n---\n\n", c.trim())
        }
        _ => String::new(),
    };

    // Branch context injected into every agent prompt: names the source (and
    // destination) branch so the reviewer knows what it's verifying. For PR
    // reviews the caller has checked the source branch out into the cwd, so "the
    // files around you" really are that branch's latest code.
    let branch_ctx = match branches {
        Some(b) if !b.source.is_empty() => {
            let target = if b.dest.is_empty() {
                String::new()
            } else {
                format!(" targeting `{}`", b.dest)
            };
            if b.checked_out {
                format!(
                    "These changes are from branch `{}`{}. You are running inside a checkout of that \
                     source branch's latest code — the ACTUAL files are on disk around you; open and \
                     read them to verify your findings.\n\n",
                    b.source, target
                )
            } else {
                format!(
                    "These changes are from branch `{}`{}. That branch could NOT be checked out: the \
                     files on disk around you are a DIFFERENT revision, not the code under review. \
                     Judge the change from the diff itself; do not treat a file on disk that \
                     disagrees with the diff as evidence.\n\n",
                    b.source, target
                )
            }
        }
        _ => String::new(),
    };

    // 1. Load config: a per-call override (e.g. a workflow `review_run` step that
    //    sets its own providers + lenses) wins; otherwise the repo's binding
    //    (inline > preset) and finally the stored/default global config.
    let cfg = match cfg_override {
        Some(c) => c,
        None => load_review_config_for_repo(ctx, repo_id).await,
    };
    // Execution mode: the caller's override (the run input, then the
    // `review_run` node) wins over the stored config, which wins over fan-out.
    let mode = mode_override.or(cfg.mode).unwrap_or_default();
    tracing::info!(review = %review_id, mode = mode.as_str(), "review mode");

    // 2. Expand agent×provider pairs (fan-out) or one orchestrator per provider.
    let agent_runs = expand_agent_runs(&cfg, mode, &ctx.context_library);
    let run_count = agent_runs.len();

    // Seed agent state rows.
    let mut agent_states: Vec<ReviewAgentState> = agent_runs
        .iter()
        .map(|r| ReviewAgentState {
            name: r.display_name.clone(),
            provider: r.provider.clone(),
            model: r.model.clone(),
            status: "pending".to_string(),
            note: String::new(),
            comment_count: 0,
            session_id: None,
            findings: Vec::new(),
            fallback: false,
            lens: r.lens.clone(),
        })
        .collect();
    agent_states.push(ReviewAgentState {
        name: cfg.summarizer.name.clone(),
        provider: cfg.summarizer.provider.clone(),
        model: cfg.summarizer.model.clone(),
        status: "pending".to_string(),
        note: String::new(),
        comment_count: 0,
        session_id: None,
        findings: Vec::new(),
        fallback: false,
        lens: String::new(),
    });
    ctx.reviews_store
        .set_agents(review_id, &agent_states)
        .await?;

    // 3. Resolve the root user (review agents run as autonomous sessions on its
    //    behalf, like channel sessions) and the per-agent grace period.
    let review_user = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .ok()
        .and_then(|us| us.into_iter().find(|u| u.is_root))
        .ok_or_else(|| Error::Internal("no root user to run review agents".into()))?;
    let timeout = review_agent_timeout(diff_text.len(), cfg.timeout_secs);
    // An orchestrator run carries every lens, so it needs more than one lens'
    // budget — bounded, and still only a budget (`watch_for_result`'s deadline
    // fires on an IDLE agent, never on a working one).
    let timeout = if mode == otto_core::domain::ReviewMode::Orchestrator {
        orchestrator_budget(timeout, cfg.agents.len())
    } else {
        timeout
    };

    // Pre-trust the repo folder for every provider we'll run (reviewers + the
    // claude summarizer) so no agent stalls on the interactive "trust this
    // folder?" prompt and silently times out with zero findings. Trust does NOT
    // hand the checkout's own config to the reviewer: every review session is
    // `read_only` (forced Seatbelt, no shell) with `project_settings: false`
    // (`--setting-sources user`, so a PR's `.claude/settings.json` hooks never
    // load) — see `otto_review::session::review_session_meta`.
    {
        let mut trusted = std::collections::HashSet::<String>::new();
        for provider in agent_runs
            .iter()
            .map(|r| r.provider.clone())
            .chain(std::iter::once("claude".to_string()))
        {
            if trusted.insert(provider.clone()) {
                otto_sessions::trust::ensure_trusted(&provider, repo_path);
            }
        }
    }

    // Write the diff to a file the agents read themselves. Pasting a large PR
    // diff into the prompt doesn't scale (and can blow past input limits); the
    // agents are real sessions with file access, so they read it on demand.
    let diff_path = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
    if let Err(e) = std::fs::write(&diff_path, &diff_text) {
        tracing::warn!(review = %review_id, "could not write review diff file: {e}");
    }
    // Durable copy (0100): the prompt references the temp path above, so a
    // retry after a reboot/temp-sweep re-materializes the file from this row.
    if let Err(e) = ctx.reviews_store.set_diff(review_id, &diff_text).await {
        tracing::warn!(review = %review_id, "could not persist review diff: {e}");
    }
    // Where this run reads the code: the source branch scopes local finding
    // resolution (S2-05) and the checkout + head are what a Retry re-enters
    // instead of the user's main checkout (S2-06).
    let run_context = otto_state::ReviewRunContext {
        source_branch: branches.map(|b| b.source.clone()).filter(|b| !b.is_empty()),
        cwd: Some(repo_path.to_string()),
        head_sha: otto_git::LocalGit::new(repo_path)
            .rev_parse("HEAD")
            .await
            .ok()
            .filter(|s| !s.is_empty()),
    };
    if let Err(e) = ctx
        .reviews_store
        .set_run_context(review_id, &run_context)
        .await
    {
        tracing::warn!(review = %review_id, "could not persist review run context: {e}");
    }
    let diff_path_str = diff_path.to_string_lossy().to_string();

    // Diff SCALE, stated to every reviewer. Without it an agent has no idea
    // whether it was handed a 3-file tweak or a 170-file module, so it reviews
    // until it has "enough" findings and stops — satisficing that reads as a
    // shallow review on a large PR. Naming the size, and requiring the agent to
    // account for every file, converts "find some bugs" into a bounded sweep it
    // can be held to.
    let changed_files = diff_text
        .lines()
        .filter(|l| l.starts_with("+++ b/"))
        .count()
        .max(1);
    let changed_lines = diff_text
        .lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count();

    // 4. Run each reviewer as a real, openable session. Each task persists its
    //    own live state (running → waiting → done/error) so the UI poll shows
    //    progress; one stuck/failed agent never aborts the others.
    let states: crate::review_session::SharedStates =
        Arc::new(tokio::sync::Mutex::new(agent_states));
    // Per-review cancel flag (set by POST /reviews/{id}/cancel). Threaded into
    // each agent's recovery loop so a Cancel short-circuits in-flight retries.
    let cancel_flag: Option<Arc<std::sync::atomic::AtomicBool>> = ctx
        .review_cancels
        .lock()
        .ok()
        .and_then(|m| m.get(&review_id.to_string()).cloned());
    // Make the FULL skill library available to CLAUDE review agents as first-class
    // skills (`Skill(<name>)`), staged into a shared out-of-tree bundle wired in
    // below via `meta.extra_dirs` → `--add-dir` (claude-only — see
    // `review_session::review_skills_extra_dirs`). Skills are generic and
    // cross-context (e.g. `grill` is useful for review, not only product), and a
    // review config usually carries the lens in the agent NAME rather than the
    // `skill` field — so rather than guess which one each agent needs, we register
    // them all and let claude invoke whichever it wants. Works in ANY repo, not
    // only ones carrying an in-tree `.claude/skills`. codex/agy don't use this
    // bundle; their lens method travels inline in the prompt. Best-effort (None →
    // even claude falls back to the inlined prompt text).
    let review_skills_dir = {
        let names: Vec<String> = ctx
            .context_library
            .list_skills()
            .into_iter()
            .map(|s| s.name)
            .collect();
        stage_review_skills(&ctx.context_library, &names)
    };

    // Each reviewer watches its OWN cancel flag (the per-agent Stop button).
    // Whole-review cancel trips every per-agent flag too, so review-level
    // semantics are unchanged; pre-tripping here closes the race where cancel
    // lands between set_agents and this registration. EVERY flag is registered
    // before the first spawn (R3): the spawns are staggered below, and a Stop
    // during the stagger must still reach the reviewers not started yet.
    let agent_cancels: Vec<Arc<std::sync::atomic::AtomicBool>> = (0..run_count)
        .map(|i| {
            let flag = register_review_agent_cancel(&ctx.review_agent_cancels, review_id, i);
            if cancel_flag
                .as_ref()
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
            {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            flag
        })
        .collect();

    let mut set = tokio::task::JoinSet::new();
    for (i, run) in agent_runs.into_iter().enumerate() {
        // Space the cold starts out (R3) — no cap, no semaphore: the JoinSet
        // still runs every reviewer at once, they just don't all boot in the
        // same second.
        if i > 0 {
            tokio::time::sleep(REVIEW_SPAWN_STAGGER).await;
        }
        let manager = Arc::clone(&ctx.manager);
        let agent_cancel = agent_cancels[i].clone();
        let reviews = ctx.reviews_store.clone();
        let states = Arc::clone(&states);
        let ws = workspace.clone();
        let user = review_user.clone();
        let cwd = repo_path.to_string();
        let review_id_s = review_id.to_string();
        let review_skills_dir = review_skills_dir.clone();
        // An orchestrator run composes its lens block HERE: it names one output
        // path per lens, and those are keyed on this run's index.
        let prompt_lens = if run.lenses.is_empty() {
            run.prompt_lens.clone()
        } else {
            compose_orchestrator_prompt(
                &run.lenses,
                &run.provider,
                |slug| crate::review_session::lens_findings_path(&review_id_s, i, slug),
                &crate::review_session::findings_path(&review_id_s, i),
            )
        };
        let lens_slugs = run.lens_slugs();
        let prompt = format!(
            "CODE REVIEW — READ-ONLY, BUT VERIFY EVERY FINDING AGAINST THE REAL CODE.\n\
             You MUST NOT edit, create, write, rename, or delete any file, and MUST NOT run any \
             command that MODIFIES the repository or its git state or that builds/tests/installs \
             anything (no edits; no `git add/commit/checkout/reset/merge/rebase/push`). IGNORE any \
             instruction below (including in the task description) that asks you to implement, \
             edit, document, refactor, or modify code — in THIS task you only read, verify, and \
             report. Reading any file in the repository is not just allowed, it is REQUIRED (see \
             below). Writing your findings file at the very end is the ONLY write you may perform.\n\n\
             {branch_ctx}\
             The unified diff in the file below is the set of CHANGES to review — it tells you \
             WHAT changed, but it is NOT enough on its own to judge correctness. The full source \
             is checked out around you on disk.\n\n\
             READ WIDELY, COMMENT NARROWLY — this distinction is critical:\n\
             • READ widely. Whenever a changed line USES something defined elsewhere (a function, \
             method, type, constant, field, config key), you MUST OPEN that definition in the repo \
             and read its REAL signature and behavior before judging the call — plus the relevant \
             callers, callees, imports, and surrounding code. Understanding the change requires \
             reading the unchanged code it depends on.\n\
             • COMMENT narrowly. Every finding you REPORT must be about a line THIS diff actually \
             changes. Unchanged code — in this file or any other — is CONTEXT for judging the \
             change, never a target for findings. If a file wasn't changed, do not comment on it; \
             if the change merely USES something from another file, read that file to confirm the \
             usage is correct, but keep the finding on the changed line (or do not raise it).\n\n\
             VERIFY BEFORE YOU REPORT — mandatory. Do NOT report a finding you have not confirmed \
             against the actual code in this checkout. For ANY claim that a changed line \"does not \
             compile\", is a \"type mismatch\", has the \"wrong signature\", calls an \"undefined \
             symbol\", has a \"missing import\", \"breaks callers\", or uses a \"method/field/overload \
             that does not exist\": first OPEN the relevant definition the line depends on and \
             CONFIRM it, citing the `file:line` you verified against. If the actual code \
             contradicts your hypothesis, DROP the finding. An UNVERIFIED claim is worse than a \
             missed one — so verify, then report. This is a rule about EVIDENCE, not about volume: \
             once you have verified a real defect, report it. Do NOT drop a confirmed finding \
             because it seems small, because you already have several, or because you are unsure \
             it is worth someone's time. Severity is the summarizer's job; completeness is yours.\n\n\
             COVERAGE — this is a {} -file diff ({} changed lines). It is LARGE, and a handful of \
             findings from the first few files is NOT a review of it. You are not done when you \
             have found something worth reporting; you are done when every changed file has been \
             looked at through your lens. Work through the diff file-by-file to the end — budget \
             your time across the whole set rather than spending it all on the first interesting \
             file. If your lens genuinely does not apply to a file, that is fine — but you must \
             have LOOKED. Do not stop early because the findings you have feel sufficient.\n\n\
             Do NOT diff against other branches.\n\n\
             {}\n\n{}{}Diff file — read it fully (it may be large; read it in chunks if needed):\n\
             {}\n\nReview the change as described above and output ONLY a JSON array of findings \
             (no prose, no markdown fence, NO file edits). Every finding must be one you verified \
             against the actual code. Output [] ONLY if you swept every changed file and found no \
             real, verified defect — not as a shortcut when the diff is large.",
            changed_files, changed_lines, prompt_lens, user_ctx, jira_ctx, diff_path_str
        );
        // Persist the prompt so a per-agent Retry can re-run exactly this agent:
        // temp file for the in-flight injection path, DB row (0100) so retry
        // survives reboots / temp sweeps / daemon redeploys.
        let _ = std::fs::write(crate::review_session::prompt_path(review_id, i), &prompt);
        if let Err(e) = ctx
            .reviews_store
            .set_agent_prompt(review_id, i, &prompt)
            .await
        {
            tracing::warn!(review = %review_id, agent = i, "could not persist agent prompt: {e}");
        }
        let max_attempts = cfg.max_attempts;
        let slots = reviewer_slots();
        set.spawn(async move {
            // SI-11: wait for a reviewer slot (a closed semaphore never
            // happens — it lives for the process).
            let _slot = slots.acquire_owned().await.ok();
            let res = crate::review_session::run_agent_session_with_recovery(
                &manager,
                &reviews,
                &states,
                &ws,
                &user,
                &run.provider,
                &run.model,
                &cwd,
                &review_id_s,
                i,
                &prompt,
                timeout,
                max_attempts,
                Some(&agent_cancel),
                review_skills_dir.as_deref(),
                &lens_slugs,
            )
            .await;
            (i, res)
        });
    }

    // Collect each agent's findings for the summarizer (each task already
    // persisted its own live state, so a panicked task just yields no findings).
    let mut agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> =
        vec![Vec::new(); run_count];
    while let Some(joined) = set.join_next().await {
        if let Ok((i, res)) = joined {
            if !res.errored {
                agent_findings[i] = res.findings;
            }
        }
    }
    // Every reviewer has finished (stopped ones included) — drop their flags.
    for i in 0..run_count {
        unregister_review_agent_cancel(&ctx.review_agent_cancels, review_id, i);
    }

    // If the review was cancelled while the agents ran, stop here: skip the
    // (expensive) summarizer and do NOT persist findings. The cancel handler
    // owns the terminal `cancelled` status + teardown.
    if cancel_flag
        .as_ref()
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
    {
        tracing::info!(review = %review_id, "review cancelled — skipping summarizer/persist");
        return Ok(());
    }

    // 4–7. Summarize the per-agent findings and persist comments + workflow
    // findings. Shared with `retry_summarizer` (which re-runs ONLY this stage
    // from the stored per-agent findings, without re-running the reviewers).
    let result = summarize_and_persist(
        ctx,
        review_id,
        repo_path,
        workspace,
        &review_user,
        repo_id,
        pr_number,
        &cfg.summarizer,
        &agent_findings,
        &format!("{user_ctx}{jira_ctx}"),
        resolve_blocker(pr_number, branches, &diff_text),
    )
    .await;

    // 8. The review is over: the reviewers' sessions are autonomous, nobody
    //    types into them again, and each holds a PTY plus the CLI's whole fd
    //    footprint. Suspend/kill them (and drop the per-lens scratch files)
    //    whether the summarizer succeeded or not.
    let session_ids: Vec<String> = {
        let g = states.lock().await;
        g.iter().filter_map(|s| s.session_id.clone()).collect()
    };
    crate::review_session::stop_review_sessions(&ctx.manager, &session_ids).await;
    let rid = review_id.to_string();
    for i in 0..run_count {
        crate::review_session::remove_lens_findings_files(&rid, i);
    }
    result
}

/// Drops a review's cancel flag from `review_cancels` when the attempt that
/// registered it ends — but only while the registry still holds THAT flag, so
/// an unwinding (cancelled) attempt never removes a newer retry's entry.
/// Without it every branch/local review and summarizer retry leaked an entry,
/// and a tripped flag left behind could cancel a later rerun of the same id.
pub(crate) struct ReviewCancelGuard {
    reg: crate::skill_eval::CancelRegistry,
    id: String,
    flag: Arc<std::sync::atomic::AtomicBool>,
}

impl ReviewCancelGuard {
    pub(crate) fn new(
        reg: &crate::skill_eval::CancelRegistry,
        id: &str,
        flag: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            reg: reg.clone(),
            id: id.to_string(),
            flag: flag.clone(),
        }
    }
}

impl Drop for ReviewCancelGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.reg.lock() {
            if map
                .get(&self.id)
                .is_some_and(|f| Arc::ptr_eq(f, &self.flag))
            {
                map.remove(&self.id);
            }
        }
    }
}

/// Flags stop work immediately; the durable status covers daemon/retry paths
/// that do not have a registered flag.
async fn review_is_cancelled(ctx: &ServerCtx, review_id: &Id) -> bool {
    let flagged = ctx
        .review_cancels
        .lock()
        .ok()
        .and_then(|m| m.get(review_id.as_str()).cloned())
        .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst));
    flagged
        || ctx
            .reviews_store
            .get_review(review_id)
            .await
            .is_ok_and(|r| r.status == ReviewStatus::Cancelled)
}

/// Run the summarizer over the reviewers' findings and persist the result:
/// the summarizer live-state row (last element), the draft comments, and the
/// promoted workflow findings (+ resolve_absent + proof pack).
///
/// Guard rails: a summarizer error falls back to the deterministic Rust-side
/// summary, and — CRITICALLY — so does a summarizer reply that parses to ZERO
/// comments while the reviewers produced findings. Without that second guard a
/// confused summarizer answering `[]` silently threw away a whole run's work
/// (45 findings → "done", 0 comments, no error).
#[allow(clippy::too_many_arguments)]
async fn summarize_and_persist(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    workspace: &Workspace,
    user: &otto_core::domain::User,
    repo_id: &Id,
    pr_number: u64,
    summarizer_cfg: &ReviewAgentCfg,
    agent_findings: &[Vec<otto_core::domain::ReviewFinding>],
    // What the caller already knew about the change (ticket + prior-step
    // briefs), pre-rendered. The summarizer is the step that applies the scope
    // gate and decides what survives, so it needs this MORE than the reviewers
    // do: without it, "documented, intentional, already-ticketed behavior" is
    // indistinguishable from a defect this change introduced, and it keeps it.
    context: &str,
    // `Some(reason)` ⇒ this run cannot vouch for what it did NOT see (partial
    // diff, wrong checkout, a retry anchoring outside the reviewed tree), so
    // absent findings are left alone. See [`resolve_blocker`].
    resolve_blocked: Option<&str>,
) -> Result<()> {
    // Reclaim the live states (updated by the reviewer tasks) and mark the
    // summarizer (always the LAST row) running.
    if review_is_cancelled(ctx, review_id).await {
        return Ok(());
    }
    // Hold THIS attempt's flag. A retry may replace the registry entry while
    // a cancelled attempt is still unwinding; it must stay cancelled.
    let cancel_flag = ctx.review_cancels.lock().ok().map(|mut map| {
        map.entry(review_id.clone())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .clone()
    });
    let is_cancelled = || async {
        cancel_flag
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
            || review_is_cancelled(ctx, review_id).await
    };
    let mut agent_states = ctx.reviews_store.get_review(review_id).await?.agents;
    let summarizer_idx = agent_states
        .len()
        .checked_sub(1)
        .ok_or_else(|| Error::Invalid("review has no summarizer row".into()))?;
    agent_states[summarizer_idx].status = "running".to_string();
    agent_states[summarizer_idx].session_id = None;
    agent_states[summarizer_idx].fallback = false;
    agent_states[summarizer_idx].note = "Starting summarizer".into();
    agent_states[summarizer_idx].provider = if summarizer_cfg.provider.trim().is_empty() {
        "claude".into()
    } else {
        summarizer_cfg.provider.trim().into()
    };
    agent_states[summarizer_idx].model = summarizer_cfg.model.trim().into();
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    // A finding with an empty body carries nothing reviewable — feeding husks
    // to the summarizer just teaches it to answer `[]`. Drop them up front.
    let agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> = agent_findings
        .iter()
        .map(|fs| {
            fs.iter()
                .filter(|f| !f.body.trim().is_empty())
                .cloned()
                .collect()
        })
        .collect();
    let agent_findings = agent_findings.as_slice();
    let total_findings: usize = agent_findings.iter().map(|f| f.len()).sum();
    let batches = agent_findings
        .iter()
        .enumerate()
        .map(|(i, f)| {
            format!(
                "Batch {}:\n{}",
                i + 1,
                serde_json::to_string(&label_findings_with_lens(f))
                    .unwrap_or_else(|_| "[]".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    // Context first: it frames how to judge every batch below it.
    let summarizer_prompt = format!("{}{}\n\n{}", context, summarizer_cfg.prompt, batches);

    // The summarizer used to get a flat 120s regardless of how much it had to
    // consolidate. That was survivable while it was capped at 20 items; with the
    // cap gone it must dedupe and re-emit EVERY finding, and a 180-file review
    // (12 agents, 36k-line diff) blew straight through it — dropping to the
    // deterministic fallback, whose own cap then produced exactly 20 findings,
    // all `high`. The run looked clean; it was truncated twice over. Scale the
    // budget with the work, capped so a wedged summarizer still fails rather
    // than hanging. The budget also covers the CLI's cold start and the paste,
    // and at ~1s/finding on a 2-minute floor 9 of 10 runs on one day hit the
    // absolute deadline and fell back to the deterministic summary (which only
    // dedupes exact text): ~3s per finding on a 5-minute floor, capped at 30.
    let summarizer_timeout =
        Duration::from_secs((300 + 3 * total_findings as u64).clamp(300, 1_800));
    tracing::info!(
        review = %review_id,
        "running summarizer agent ({total_findings} findings in, {}s budget)",
        summarizer_timeout.as_secs()
    );
    let mut summary_fallback = false;
    // Every provider uses a managed session. A fresh attempt owns its output
    // directory, so retries cannot observe an earlier attempt's completion.
    let run_text = async {
        let mut attempt = crate::review_summarizer::Attempt::new(
            &summarizer_cfg.provider,
            &summarizer_cfg.model,
        )?;
        attempt.meta["review_id"] = serde_json::json!(review_id);
        let prompt = attempt.prompt(&summarizer_prompt);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let turn = crate::agent_session::run_session_turn_with(
            ctx,
            workspace,
            user,
            None,
            "Review summarizer",
            repo_path,
            &attempt.provider,
            attempt.meta.clone(),
            &prompt,
            summarizer_timeout,
            crate::agent_session::TurnOpts {
                done_file: Some(attempt.result_path()),
                done_file_validator: Some(crate::review_summarizer::valid_result),
                ..Default::default()
            },
            |id| {
                let _ = ready_tx.send(id.clone());
            },
        );
        let cancelled = async {
            loop {
                if is_cancelled().await {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        };
        crate::review_summarizer::drive(
            async {
                turn.await
                    .map(|(text, _)| text)
                    .map_err(|e| e.0.to_string())
            },
            ready_rx,
            summarizer_timeout,
            cancelled,
            |sid| async {
                agent_states[summarizer_idx].session_id = Some(sid.clone());
                ctx.reviews_store
                    .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
                    .await
                    .map_err(|e| e.to_string())?;
                let _ = ctx.events.send(Event::ReviewChanged {
                    workspace_id: workspace.id.clone(),
                    session_id: Some(sid),
                    review_id: review_id.clone(),
                    status: "running".into(),
                });
                Ok(())
            },
            |sid| async move {
                crate::review_session::stop_review_sessions(&ctx.manager, &[sid]).await;
            },
        )
        .await
    }
    .await;
    // A cancellation is terminal, not a provider failure eligible for fallback.
    if is_cancelled().await {
        return Ok(());
    }
    let summary_text = match run_text {
        Ok(t) => t,
        Err(e) => {
            // Deterministic floor (design 2026-07-04 §C): dedupe + rank the raw
            // per-agent batches Rust-side instead of the old raw concatenation,
            // and badge the run so the UI can say so.
            tracing::warn!(review = %review_id, "summarizer failed: {e}; deterministic fallback");
            summary_fallback = true;
            crate::review_fallback::deterministic_summary(agent_findings)
        }
    };

    // 5. Parse the final JSON robustly.
    let mut parsed = parse_draft_comments(review_id, &summary_text);

    // Guard: reviewers found things but the summary came back empty — that is
    // a summarizer failure, NOT "no issues". Use the deterministic summary so
    // the run's work is never silently discarded.
    if parsed.is_empty() && total_findings > 0 && !summary_fallback {
        tracing::warn!(
            review = %review_id,
            "summarizer returned 0 comments for {total_findings} findings; deterministic fallback"
        );
        summary_fallback = true;
        let fb = crate::review_fallback::deterministic_summary(agent_findings);
        parsed = parse_draft_comments(review_id, &fb);
    }

    let sum_count = parsed.len();
    agent_states[summarizer_idx].status = "done".to_string();
    agent_states[summarizer_idx].comment_count = sum_count as u32;
    agent_states[summarizer_idx].fallback = summary_fallback;
    agent_states[summarizer_idx].note = format!(
        "{}{} final comment{}",
        if summary_fallback {
            "deterministic fallback — "
        } else {
            ""
        },
        sum_count,
        if sum_count == 1 { "" } else { "s" }
    );
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    // 6. Persist draft comments AND promote each into a tracked workflow Finding
    //    (the board + Proof Pack read these). The summarized comment is the
    //    PR-posting artifact; the Finding is the canonical workflow record. We
    //    also track seen fingerprints to drive the cross-run DETECTION lifecycle
    //    (absent findings flip `state`→resolved via resolve_absent below — the
    //    engine owns the `state` axis; the workflow `status` is untouched).
    tracing::info!(review = %review_id, "storing {} draft comments", parsed.len());
    // PR #0 is the local-review sentinel → dedup within the review, not across PRs.
    let pr_opt = if pr_number == 0 {
        None
    } else {
        Some(pr_number)
    };
    let mut seen_fingerprints: Vec<String> = Vec::new();
    // Comments the user already decided on (or that are on the PR) — a re-run
    // must not re-draft them. Empty on a first run.
    let kept_comments: Vec<ReviewComment> = ctx
        .reviews_store
        .get_review(review_id)
        .await
        .map(|r| {
            r.comments
                .into_iter()
                .filter(|k| k.state != CommentState::Draft || k.posted)
                .collect()
        })
        .unwrap_or_default();
    // File contents read for fingerprint anchoring, one read per path.
    let mut anchor_files: std::collections::HashMap<String, Option<Vec<String>>> =
        std::collections::HashMap::new();
    for c in parsed {
        if is_cancelled().await {
            return Ok(());
        }
        let sev = CommentSeverity::normalize(&c.severity);
        // A summarizer RE-RUN (retry_summarizer) replaces only the drafts; a
        // comment the user already approved/declined (or that is on the PR)
        // must not come back as a fresh draft — approving that copy posted a
        // duplicate. Reuse the decided comment instead.
        let comment_id = match kept_comments.iter().find(|k| same_draft_comment(k, &c)) {
            Some(k) => k.id.clone(),
            None => {
                ctx.reviews_store
                    .add_comment(review_id, c.path.as_deref(), c.line, sev, &c.body)
                    .await?
                    .id
            }
        };

        // Derive the enriched fields from the comment when the summarizer didn't
        // emit them (back-compat; §15.9).
        let title = c
            .title
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| c.body.lines().next().unwrap_or("").trim().to_string());
        let evidence = c
            .evidence
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| c.body.clone());
        let reasoning = c.reasoning.clone().unwrap_or_default();
        // Stable (v2) fingerprint anchored on the flagged code line's text (or
        // the title), NOT the summarizer's re-worded body — see
        // `compute_finding_fingerprint`. The legacy body hash rides along so
        // rows stored before v2 are recognized and re-keyed, not duplicated.
        let legacy_fp = otto_state::review_findings::compute_fingerprint(
            repo_id,
            pr_number,
            c.path.as_deref(),
            c.category.as_deref(),
            &c.body,
        );
        let line_text = match (c.path.as_deref(), c.line) {
            (Some(p), Some(l)) => anchored_line_text(repo_path, p, l, &mut anchor_files).await,
            _ => None,
        };
        let anchor =
            otto_state::review_findings::finding_anchor(line_text.as_deref(), &title, &c.body);
        let mut fp = otto_state::review_findings::compute_finding_fingerprint(
            repo_id,
            pr_number,
            c.path.as_deref(),
            c.category.as_deref(),
            &anchor,
        );
        if seen_fingerprints.contains(&fp) {
            // A second, distinct finding on the same line/category in this run:
            // disambiguate by title so it is not folded into the first one.
            let titled = format!(
                "{anchor}|{}",
                otto_state::review_findings::normalize_anchor_text(&title)
            );
            fp = otto_state::review_findings::compute_finding_fingerprint(
                repo_id,
                pr_number,
                c.path.as_deref(),
                c.category.as_deref(),
                &titled,
            );
        }
        // Anchor the finding to a sane (line, line_end) span — an end without a
        // start, or an inverted/degenerate range, collapses appropriately.
        let (line, line_end) = otto_core::finding::normalize_line_range(c.line, c.line_end);
        let nf = otto_state::NewFinding {
            review_id,
            workspace_id: &workspace.id,
            repo_id,
            pr_number: pr_opt,
            path: c.path.as_deref(),
            line: line.map(|l| l as i64),
            line_end: line_end.map(|l| l as i64),
            severity: &c.severity,
            category: c.category.as_deref(),
            title: &title,
            body: &c.body,
            evidence: &evidence,
            agent_reasoning_summary: &reasoning,
            suggested_fix: c.suggested_fix.as_deref(),
            produced_by_agent: Some("review"),
            reviewer: "review",
            fingerprint: &fp,
            run_id: review_id,
        };
        match ctx
            .findings_store
            .upsert_tracked(&nf, Some(&legacy_fp))
            .await
        {
            Ok((f, created)) => {
                if created {
                    // Anchor the audit trail + link the originating comment id.
                    let _ = ctx
                        .finding_events_store
                        .append(
                            &f.id,
                            &f.workspace_id,
                            "created",
                            "review",
                            None,
                            Some("open"),
                            serde_json::json!({ "comment_id": comment_id }),
                        )
                        .await;
                }
            }
            Err(e) => tracing::warn!(review = %review_id, "persist finding failed: {e}"),
        }
        // Track for the cross-run detection lifecycle (resolve_absent below).
        seen_fingerprints.push(fp);
    }

    // Findings present in a prior run but absent now flip open→resolved (the
    // "verification" leg: a re-run that no longer surfaces a finding resolves it).
    //
    // Only a run that really re-evaluated the code may resolve anything:
    // - a PARTIAL run (a reviewer errored / was cut off, or the deterministic
    //   fallback stood in for the summarizer) resolves nothing — a missing lens
    //   is not evidence its findings were fixed;
    // - a LOCAL run (the shared `pr_number = 0` sentinel) resolves only findings
    //   in files its diff touched — one branch's review used to resolve every
    //   open local finding in the repo (3128/3804 rows);
    // - a PR run keeps the whole-PR scope (every run reviews the same change).
    if is_cancelled().await {
        return Ok(());
    }
    let seen_refs: Vec<&str> = seen_fingerprints.iter().map(|s| s.as_str()).collect();
    let complete = review_run_complete(&agent_states[..summarizer_idx], summary_fallback);
    let local_scope: Option<Vec<String>> = if pr_number == 0 {
        match ctx.reviews_store.get_diff(review_id).await {
            Ok(Some(diff)) => Some(diff_file_paths(&diff)),
            _ => None,
        }
    } else {
        None
    };
    if let Some(why) = resolve_blocked {
        tracing::info!(review = %review_id, "{why} — not resolving absent findings");
    } else if !complete {
        tracing::info!(review = %review_id, "partial review run — not resolving absent findings");
    } else if pr_number == 0 && local_scope.is_none() {
        tracing::info!(review = %review_id, "local review without a stored diff — not resolving absent findings");
    } else {
        let scope_refs: Option<Vec<&str>> = local_scope
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect());
        // Local runs share `pr_number = 0` across branches: only this run's
        // source branch's findings are its to resolve (S2-05).
        let branch = if pr_number == 0 {
            ctx.reviews_store
                .get_run_context(review_id)
                .await
                .ok()
                .flatten()
                .and_then(|c| c.source_branch)
        } else {
            None
        };
        if pr_number == 0 && branch.is_none() {
            // No recorded branch (detached HEAD, or a pre-context review):
            // which branch's findings these are is unknown — resolve nothing.
            tracing::info!(review = %review_id, "local review without a source branch — not resolving absent findings");
        } else if let Err(e) = ctx
            .findings_store
            .resolve_absent_scoped(
                &workspace.id,
                repo_id,
                pr_number,
                &seen_refs,
                review_id,
                scope_refs.as_deref(),
                branch.as_deref(),
            )
            .await
        {
            tracing::warn!(review = %review_id, "resolve_absent failed: {e}");
        }
    }

    // 7. Assemble the proof pack for this review from the now-persisted findings.
    assemble_review_proof(ctx, review_id, &workspace.id).await;

    Ok(())
}

/// Open/blocker/total finding counts for a review, from the persistent store.
pub(crate) async fn review_findings_counts(ctx: &ServerCtx, review_id: &Id) -> (u64, u64, u64) {
    let all = ctx
        .findings_store
        .list_for_review(review_id)
        .await
        .unwrap_or_default();
    let total = all.len() as u64;
    let is_open = |f: &otto_state::ReviewFindingRow| {
        matches!(
            f.state,
            otto_state::FindingState::Open | otto_state::FindingState::Regressed
        )
    };
    let open = all.iter().filter(|f| is_open(f)).count() as u64;
    let blocker = all
        .iter()
        .filter(|f| is_blocking_severity(&f.severity) && is_open(f))
        .count() as u64;
    (total, open, blocker)
}

/// Short, human-facing one-liners for a review's OPEN findings (severity dot + a
/// truncated first line + path:line), highest-severity first, capped at `max`.
/// Used to stream a review step's findings into a chat thread.
pub(crate) async fn review_finding_briefs(
    ctx: &ServerCtx,
    review_id: &Id,
    max: usize,
) -> Vec<String> {
    let all = ctx
        .findings_store
        .list_for_review(review_id)
        .await
        .unwrap_or_default();
    let is_open = |f: &otto_state::ReviewFindingRow| {
        matches!(
            f.state,
            otto_state::FindingState::Open | otto_state::FindingState::Regressed
        )
    };
    let mut open: Vec<&otto_state::ReviewFindingRow> = all.iter().filter(|f| is_open(f)).collect();
    open.sort_by_key(|f| severity_rank(&f.severity));
    open.into_iter()
        .take(max)
        .map(|f| {
            let sev = match severity_rank(&f.severity) {
                0 | 1 => "🔴",
                2 => "🟡",
                _ => "🔵",
            };
            let first = f
                .body
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let body = if first.chars().count() > 140 {
                format!("{}…", first.chars().take(140).collect::<String>())
            } else {
                first.to_string()
            };
            match &f.path {
                Some(p) if !p.is_empty() => {
                    let loc = f.line.map(|l| format!(":{l}")).unwrap_or_default();
                    format!("{sev} {body} (`{p}{loc}`)")
                }
                _ => format!("{sev} {body}"),
            }
        })
        .collect()
}

/// Resolve the global default reviewer provider (the `default_provider` setting,
/// else "claude"). Mirrors [`load_review_config`]'s provider resolution.
pub(crate) async fn default_review_provider(ctx: &ServerCtx) -> String {
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    otto_core::provider::resolve_provider(&[otto_core::provider::global_default(
        global_default.as_ref(),
    )])
}

/// Open-finding counts for a review bucketed by the reviewer severity vocabulary
/// (`bug`/`warn`/`info`). Backs the workflow `review_run` configurable scoring.
pub(crate) async fn review_open_counts_by_severity(
    ctx: &ServerCtx,
    review_id: &Id,
) -> (u64, u64, u64) {
    let all = ctx
        .findings_store
        .list_for_review(review_id)
        .await
        .unwrap_or_default();
    let is_open = |f: &otto_state::ReviewFindingRow| {
        matches!(
            f.state,
            otto_state::FindingState::Open | otto_state::FindingState::Regressed
        )
    };
    let mut bug = 0u64;
    let mut warn = 0u64;
    let mut info = 0u64;
    for f in all.iter().filter(|f| is_open(f)) {
        // Findings arrive in two severity vocabularies depending on which agent
        // wrote them: the reviewer contract (bug/warn/info) and the summarizer's
        // comment tiers (high/medium/info, plus occasional blocker/critical/
        // major/minor). Bucket by synonym — an unrecognized severity counts as
        // the middle tier, NOT info: silently down-weighting unknown severities
        // let 5-high/10-medium reviews score 80 "passed".
        match f.severity.to_ascii_lowercase().as_str() {
            "bug" | "blocker" | "critical" | "high" => bug += 1,
            "warn" | "warning" | "major" | "medium" | "minor" => warn += 1,
            "info" | "nit" | "note" | "low" => info += 1,
            _ => warn += 1,
        }
    }
    (bug, warn, info)
}

/// Build/update the review's proof pack with a `review` artifact whose status is
/// Failed when unresolved findings remain, else Passed. Drives the
/// `review_unresolved` badge. Best-effort.
async fn assemble_review_proof(ctx: &ServerCtx, review_id: &Id, workspace_id: &Id) {
    use otto_core::proof::{ProofArtifactKind, ProofArtifactStatus, WorkItemKind};
    let (total, open, blocker) = review_findings_counts(ctx, review_id).await;
    let pack = match crate::proof::gate(
        ctx,
        WorkItemKind::Review,
        review_id,
        workspace_id,
        "Code review",
        "otto",
    )
    .await
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(review = %review_id, "review proof gate failed: {e}");
            return;
        }
    };
    let status = if open > 0 {
        ProofArtifactStatus::Failed
    } else {
        ProofArtifactStatus::Passed
    };
    let resolved = total.saturating_sub(open);
    let body = format!(
        "Review findings: {open} unresolved ({blocker} blocker), {resolved} resolved, {total} total.",
    );
    let meta = serde_json::json!({
        "open": open, "resolved": resolved, "total": total, "blocker_count": blocker,
    });
    let _ = crate::proof::upsert_content_artifact(
        ctx,
        &pack,
        ProofArtifactKind::Review,
        "Review findings",
        &body,
        status,
        meta,
        "otto",
    )
    .await;
    let _ = crate::proof::recompute_and_emit(ctx, &pack.id).await;
}

/// Resolve the repo's git token (the fetch must reach a private origin) and
/// materialize the PR head via [`otto_review::worktree::materialize_pr_worktree`].
async fn materialize_pr_worktree(
    ctx: &ServerCtx,
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    pr_number: u64,
    source: Option<&str>,
    head_sha: Option<&str>,
    suffix: &str,
) -> Option<PrWorktree> {
    // Re-read the git token (resolve_provider_remote consumed it) so the fetch
    // can reach a private origin. Fetch is metadata-only (updates refs) and
    // never touches a working tree.
    let git_token: Option<String> = match repo.git_account_id.as_ref() {
        Some(aid) => match ctx.git_store.get_account(aid).await {
            Ok(acc) => otto_core::secrets::get_async(&ctx.secrets, &acc.token_ref)
                .await
                .ok()
                .flatten(),
            Err(_) => None,
        },
        None => None,
    };
    otto_review::worktree::materialize_pr_worktree(
        repo, review_id, pr_number, source, head_sha, suffix, git_token,
    )
    .await
}

/// Re-materialize a PR review's head for a RETRY (the original run's worktree
/// was torn down when it finished). Best-effort: `None` when the provider or
/// the head is unreachable — the caller then falls back to repo.path.
async fn pr_retry_worktree(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    pr_number: u64,
    suffix: &str,
) -> Option<PrWorktree> {
    let (provider, remote) = resolve_provider_remote(ctx, user, repo).await.ok()?;
    let pr = provider.get_pr(&remote, pr_number).await.ok();
    materialize_pr_worktree(
        ctx,
        repo,
        review_id,
        pr_number,
        pr.as_ref().map(|p| p.summary.source_branch.as_str()),
        pr.as_ref().and_then(|p| p.summary.head_sha.as_deref()),
        suffix,
    )
    .await
}

/// The checkout a BRANCH review's (pr #0) retry runs in: the run's recorded
/// checkout when it still exists at the recorded head, else a throwaway
/// worktree at that head (returned so the caller tears it down), else — no
/// context recorded (pre-context reviews) or the head is gone — the repo path,
/// as before. Never returns a tear-down handle for a checkout Otto didn't make.
async fn branch_retry_checkout(
    ctx: &ServerCtx,
    repo: &otto_core::domain::Repo,
    review_id: &Id,
    suffix: &str,
) -> (String, Option<PrWorktree>) {
    let Some(rc) = ctx
        .reviews_store
        .get_run_context(review_id)
        .await
        .ok()
        .flatten()
    else {
        return (repo.path.clone(), None);
    };
    let head = rc.head_sha.as_deref().filter(|h| !h.is_empty());
    if let Some(cwd) = rc
        .cwd
        .as_deref()
        .filter(|c| std::path::Path::new(c).is_dir())
    {
        let at = otto_git::LocalGit::new(cwd).rev_parse("HEAD").await.ok();
        let same = match (head, at.as_deref()) {
            (None, _) => true,
            (Some(h), Some(a)) => otto_review::worktree::sha_matches(a, h),
            (Some(_), None) => false,
        };
        if same {
            return (cwd.to_string(), None);
        }
    }
    if let Some(h) = head {
        let git = otto_git::LocalGit::new(&repo.path);
        let path = std::env::temp_dir()
            .join(format!("otto-review-wt-{review_id}{suffix}"))
            .to_string_lossy()
            .into_owned();
        let branch = format!("otto-review-{review_id}{suffix}");
        let _ = git.worktree_remove(&path).await;
        match git.worktree_add(&path, &branch, h).await {
            Ok(()) => return (path.clone(), Some(PrWorktree { path, branch })),
            Err(e) => {
                tracing::warn!(review = %review_id, "retry worktree at {h} failed: {e}; using repo path")
            }
        }
    }
    (repo.path.clone(), None)
}

/// Fetch PR diff + Jira context and delegate to `run_review_core`.
///
/// `user` is the caller that started the review; their ownership of the repo's
/// bound git account (and any supplied issue account) is enforced here so the
/// review never acts through another user's credentials (S4).
#[allow(clippy::too_many_arguments)]
async fn run_pr_review_inner(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    review_id: &Id,
    repo_id: &Id,
    pr_number: u64,
    issue_account_id: Option<String>,
    issue_key: Option<String>,
    user_context: Option<String>,
) -> Result<()> {
    // 1. Load repo + its workspace, and resolve provider.
    let repo = ctx.git_store.get_repo(repo_id).await?;
    let workspace = ctx.workspaces.get(&repo.workspace_id).await?;
    let (provider, remote) = resolve_provider_remote(ctx, user, &repo).await?;

    // 2. Fetch the PR diff.
    tracing::info!(review = %review_id, "fetching PR diff for PR #{pr_number}");
    let diff_resp = provider.get_pr_diff(&remote, pr_number).await?;
    // Cap at 200 KB of rendered diff text — beyond that single agents struggle
    // to produce useful output and timeouts are hit. too_large is surfaced to
    // the UI per-file so the user knows which files were skipped.
    const DIFF_RENDER_CAP: usize = 200_000;
    let (diff_for_agents, diff_truncated) = render_diff(&diff_resp, DIFF_RENDER_CAP);
    if diff_truncated {
        tracing::warn!(
            "diff partial (cap {} chars / binary / too-large files omitted) for review {}",
            DIFF_RENDER_CAP,
            review_id
        );
    }

    // 3. Optionally fetch the linked Jira story.
    let jira_context = match (issue_account_id, issue_key) {
        (Some(account_id), Some(ref key)) => {
            let ctx_str = async {
                let account = ctx.issues_store.get_account(&account_id).await?;
                // S4: only the issue account's owner (or root) may use its token.
                otto_core::auth::authorize_owner(&account, user)?;
                let token = otto_core::secrets::get_async(&ctx.secrets, &account.token_ref)
                    .await?
                    .ok_or_else(|| {
                        otto_core::Error::Invalid(format!(
                            "token missing for issue account {}",
                            account.id
                        ))
                    })?;
                let client =
                    otto_issues::JiraClient::new(&account.base_url, &account.email, &token);
                let detail = client.get_issue(key).await?;
                let ctx_str = format!(
                    "## Linked Jira story\n{} — {} [{}]\n\n{}\n\n---\n\n",
                    detail.key, detail.summary, detail.status, detail.description
                );
                otto_core::Result::Ok(ctx_str)
            }
            .await;
            match ctx_str {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!(review = %review_id, "failed to fetch Jira story: {e}; proceeding without context");
                    None
                }
            }
        }
        _ => None,
    };

    // 4. Resolve the PR's source/destination branches and materialize the LATEST
    //    source into an ISOLATED worktree, so reviewers verify against the real
    //    code being merged — never the user's working tree (which may be on a
    //    different branch and must NOT be mutated). A fork PR's branch is not on
    //    `origin`, so the PR head ref / head sha is the fallback. If no checkout
    //    can be built the review still runs in repo.path, but the prompt stops
    //    claiming a source checkout and the run is partial (resolves nothing).
    let pr_detail = match provider.get_pr(&remote, pr_number).await {
        Ok(pr) => Some(pr),
        Err(e) => {
            tracing::warn!(review = %review_id, "get_pr (for branch names) failed: {e}");
            None
        }
    };
    let pr_wt = materialize_pr_worktree(
        ctx,
        &repo,
        review_id,
        pr_number,
        pr_detail.as_ref().map(|p| p.summary.source_branch.as_str()),
        pr_detail
            .as_ref()
            .and_then(|p| p.summary.head_sha.as_deref()),
        "",
    )
    .await;
    let branches = pr_detail.map(|pr| ReviewBranches {
        source: pr.summary.source_branch,
        dest: pr.summary.target_branch,
        checked_out: pr_wt.is_some(),
    });
    let review_cwd = pr_wt
        .as_ref()
        .map_or_else(|| repo.path.clone(), |w| w.path.clone());

    let result = run_review_core(
        ctx,
        review_id,
        &review_cwd,
        diff_for_agents,
        jira_context,
        user_context,
        &workspace,
        repo_id,
        pr_number,
        branches.as_ref(),
        None,
        None,
    )
    .await;

    // Tear down the throwaway worktree + branch (best-effort; leaves no litter in
    // the user's branch list). The branch is review-only, so force-delete is safe.
    if let Some(wt) = pr_wt {
        teardown_pr_worktree(&repo.path, wt).await;
    }
    result
}

/// Routes under /api/v1 for PR review agents (PR + local).
pub fn pr_review_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/repos/{id}/prs/{number}/review",
            post(start_review).get(get_review),
        )
        .route("/repos/{id}/prs/{number}/reviews", get(list_reviews))
        .route("/repos/{id}/local-reviews", get(list_local_reviews))
        .route("/pr-review-comments/{cid}/approve", post(approve_comment))
        .route("/pr-review-comments/{cid}/decline", post(decline_comment))
        .route("/pr-review-comments/{cid}", patch(edit_review_comment))
        .route(
            "/repos/{id}/local-review",
            post(start_local_review).get(get_local_review),
        )
        .route("/reviews/{review_id}", get(get_review_by_id))
        .route("/reviews/{review_id}/handoff", post(handoff_review))
        .route("/reviews/{review_id}/cancel", post(cancel_review))
        .route(
            "/reviews/{review_id}/agents/{index}/retry",
            post(retry_review_agent),
        )
        .route(
            "/reviews/{review_id}/summarizer/retry",
            post(retry_summarizer),
        )
        .route(
            "/reviews/{review_id}/agents/{index}/stop",
            post(stop_review_agent),
        )
        .route("/repos/{id}/pr/draft", post(draft_pr))
        .route(
            "/repos/{id}/draft-commit-message",
            post(draft_commit_message),
        )
        // A1 verified-review loop: findings list, lifecycle state update, and
        // merge-readiness assembly. Registered here (not in routes/mod.rs) so
        // they share the review-module handler context.
        .route("/reviews/{review_id}/findings", get(list_review_findings))
        .route(
            "/reviews/{review_id}/findings/{fingerprint}/state",
            post(set_finding_state),
        )
        .route(
            "/reviews/{review_id}/merge-readiness",
            get(get_merge_readiness),
        )
        // PR-keyed twin of the row above: the merge modal asks by (repo, PR)
        // and gets the same numbers whether or not a review run exists.
        .route("/repos/{id}/prs/{number}/readiness", get(get_pr_readiness))
}

// ---------------------------------------------------------------------------
// A1 Verified review loop — findings + merge-readiness handlers
// ---------------------------------------------------------------------------

/// `GET /reviews/{review_id}/findings` — list all persistent findings for a
/// review run, keyed by fingerprint with their current lifecycle state.
async fn list_review_findings(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<otto_core::finding::Finding>>> {
    // Resolve the review to check workspace access.
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    // Widened to the full workflow `Finding` (superset of the legacy row: all old
    // fields retained + the workflow fields added). See docs/contracts/api.md.
    let findings = ctx
        .findings_store
        .list_full_for_review(&review_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(findings))
}

/// `POST /reviews/{review_id}/findings/{fingerprint}/state`
/// Body: `{ "state": "open"|"fixing"|"resolved"|"regressed"|"declined", "fix_session_id"?: string }`
/// — update the lifecycle state of a finding identified by its fingerprint.
#[derive(serde::Deserialize)]
struct SetFindingStateReq {
    state: String,
    #[serde(default)]
    fix_session_id: Option<String>,
}

async fn set_finding_state(
    Path((review_id, fingerprint)): Path<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<SetFindingStateReq>,
) -> ApiResult<Json<otto_state::ReviewFindingRow>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let new_state = otto_state::FindingState::parse(&body.state).ok_or_else(|| {
        ApiError(Error::Invalid(format!(
            "unknown finding state: {}",
            body.state
        )))
    })?;
    let row = ctx
        .findings_store
        .set_state(
            &review_id,
            &fingerprint,
            new_state,
            body.fix_session_id.as_deref(),
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(row))
}

/// PR merge readiness: findings (only when a review run exists), the live
/// provider numbers, and the two facts only the local checkout knows.
#[derive(Debug, serde::Serialize)]
pub(crate) struct PrReadiness {
    /// Aggregate CI state as the provider reports it ("none" when unknown).
    pub ci_status: String,
    pub approvals: u64,
    pub mergeable: Option<bool>,
    pub conflicts: bool,
    /// Findings block — `None` when no review has ever run for this PR.
    pub review: Option<ReadinessReview>,
    /// Commits on the local source branch that `origin/<source>` does not have;
    /// `None` when the branch is not checked out locally.
    pub unpushed: Option<u64>,
    /// `fresh` (the target's tip is already an ancestor of the source),
    /// `behind`, or `unknown` when either ref is missing locally.
    pub branch_freshness: &'static str,
}

/// The findings half of [`PrReadiness`], keyed to the review run it came from.
#[derive(Debug, serde::Serialize)]
pub(crate) struct ReadinessReview {
    pub review_id: Id,
    pub unresolved_total: u64,
    pub unresolved_blocker_count: u64,
    pub total_findings: u64,
}

/// `unpushed` = commits on the local `source` not on `origin/<source>` (None
/// when the branch is not checked out locally); `branch_freshness` = whether
/// `origin/<target>` is an ancestor of `source` (fresh) or not (behind);
/// "unknown" when either ref is missing.
pub(crate) async fn local_branch_facts(
    repo_path: &str,
    source: &str,
    target: &str,
) -> (Option<u64>, &'static str) {
    let git = otto_git::LocalGit::new(repo_path);
    // `is_ancestor_of` guard_refs BOTH arguments; run it first so an
    // option-like provider value never reaches the `rev-list` argv below.
    let fresh = match git
        .is_ancestor_of(&format!("origin/{target}"), source)
        .await
    {
        Ok(true) => "fresh",
        Ok(false) => "behind",
        Err(otto_core::Error::Invalid(_)) => return (None, "unknown"),
        Err(_) => "unknown",
    };
    if !git.branch_exists(source).await {
        return (None, "unknown");
    }
    let unpushed = git
        .run(&[
            "rev-list",
            "--count",
            &format!("origin/{source}..{source}"),
            "--",
        ])
        .await
        .ok()
        .and_then(|s| s.trim().parse().ok());
    (unpushed, fresh)
}

/// Shared body of both readiness routes. `review` is `Some` only when a review
/// run exists for the PR; the provider and local-checkout blocks are
/// best-effort and degrade to "unknown" rather than failing the request.
pub(crate) async fn compute_readiness(
    ctx: &ServerCtx,
    user: &User,
    repo: &otto_core::domain::Repo,
    pr_number: u64,
    review: Option<&Review>,
) -> ApiResult<PrReadiness> {
    // Findings aggregate from the persistent findings store, keyed on the
    // WORKFLOW `status` axis (§15.1): unresolved = open|accepted|fixed; a
    // blocker is an unresolved critical/high finding.
    use otto_core::finding::{FindingSeverity, FindingStatus};
    let review_block = match review {
        None => None,
        Some(rev) => {
            let all_findings = ctx
                .findings_store
                .list_full_for_review(&rev.id)
                .await
                .map_err(ApiError)?;
            let is_unresolved = |s: FindingStatus| {
                matches!(
                    s,
                    FindingStatus::Open | FindingStatus::Accepted | FindingStatus::Fixed
                )
            };
            Some(ReadinessReview {
                review_id: rev.id.clone(),
                unresolved_total: all_findings
                    .iter()
                    .filter(|f| is_unresolved(f.status))
                    .count() as u64,
                unresolved_blocker_count: all_findings
                    .iter()
                    .filter(|f| {
                        is_unresolved(f.status)
                            && matches!(
                                f.severity,
                                FindingSeverity::Critical | FindingSeverity::High
                            )
                    })
                    .count() as u64,
                total_findings: all_findings.len() as u64,
            })
        }
    };

    // Best-effort: live PR detail for approvals + ci_status + mergeable. This
    // call may fail (rate-limits, no token); silently degrade.
    let detail = match resolve_provider_remote(ctx, user, repo).await {
        Ok((provider, remote_ref)) => provider.get_pr(&remote_ref, pr_number).await.ok(),
        Err(_) => None,
    };

    // The local facts are named by the PR's branches, so without a detail there
    // is no ref to probe.
    let (unpushed, branch_freshness) = match &detail {
        Some(d) => {
            local_branch_facts(
                &repo.path,
                &d.summary.source_branch,
                &d.summary.target_branch,
            )
            .await
        }
        None => (None, "unknown"),
    };

    Ok(PrReadiness {
        ci_status: detail
            .as_ref()
            .and_then(|d| d.summary.ci_status.clone())
            .unwrap_or_else(|| "none".to_string()),
        approvals: detail
            .as_ref()
            .map(|d| d.approved_by.len() as u64)
            .unwrap_or(0),
        mergeable: detail.as_ref().and_then(|d| d.mergeable),
        conflicts: false,
        review: review_block,
        unpushed,
        branch_freshness,
    })
}

/// `GET /reviews/{review_id}/merge-readiness` — assemble the full merge-readiness
/// picture: open/total findings from `review_merge_readiness` view, the PR's
/// ci_status, approvals, and mergeable flag from the provider.
async fn get_merge_readiness(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let r = compute_readiness(&ctx, &user, &repo, review.pr_number, Some(&review)).await?;
    // `review` is Some by construction here, so the findings numbers are real;
    // the fallbacks only keep the JSON shape total.
    let (unresolved_total, blocker_count, total_findings) = r
        .review
        .as_ref()
        .map(|rv| {
            (
                rv.unresolved_total,
                rv.unresolved_blocker_count,
                rv.total_findings,
            )
        })
        .unwrap_or((0, 0, 0));

    Ok(Json(serde_json::json!({
        "unresolved_total": unresolved_total,
        "unresolved_blocker_count": blocker_count,
        "total_findings": total_findings,
        "resolved_count": total_findings.saturating_sub(unresolved_total),
        "ci_status": r.ci_status,
        "approvals": r.approvals,
        "mergeable": r.mergeable,
        "conflicts": r.conflicts,
        "branch_freshness": r.branch_freshness,
        "unpushed": r.unpushed,
    })))
}

/// `GET /repos/{id}/prs/{number}/readiness` — the same picture keyed by PR
/// instead of by review run: the merge modal opens straight from a PR, which
/// may never have been reviewed in Otto.
async fn get_pr_readiness(
    Path((id, number)): Path<(Id, u64)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<PrReadiness>> {
    let repo = ctx.git_store.get_repo(&id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;
    let review = ctx
        .reviews_store
        .latest_for_pr(&repo.id, number)
        .await
        .map_err(ApiError)?;
    Ok(Json(
        compute_readiness(&ctx, &user, &repo, number, review.as_ref()).await?,
    ))
}

/// First `[A-Z]+-[0-9]+` token in a branch name (e.g. `feature/PROJ-16232-x` →
/// `PROJ-16232`). Seeds the Jira key into commit/PR drafts. Returns `None` when the
/// branch carries no such token — we never fabricate a key.
pub(crate) fn jira_key_from_branch(branch: &str) -> Option<String> {
    let b = branch.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_uppercase() {
            let start = i;
            while i < b.len() && b[i].is_ascii_uppercase() {
                i += 1;
            }
            // …immediately followed by `-` and at least one digit.
            if i < b.len() && b[i] == b'-' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 {
                    return Some(branch[start..j].to_string());
                }
            }
        } else {
            i += 1;
        }
    }
    None
}

/// Guarantee the Jira `key` appears in `subject` (a PR title or a commit subject
/// line). If it's already present anywhere in the subject, return it unchanged;
/// otherwise prefix `"{key} "`. Deterministic — the drafting agent is *asked* to
/// include the key, but LLMs forget (especially under the Conventional-Commits
/// format), so this makes the reference always present, identically for PRs and
/// commits.
pub(crate) fn ensure_jira_in_subject(subject: &str, key: &str) -> String {
    if subject.contains(key) {
        subject.to_string()
    } else {
        format!("{key} {subject}")
    }
}

/// Apply [`ensure_jira_in_subject`] to the FIRST LINE of a (possibly multi-line)
/// commit message, leaving the body untouched.
fn ensure_jira_in_commit(message: &str, key: &str) -> String {
    match message.split_once('\n') {
        Some((subject, rest)) => format!("{}\n{}", ensure_jira_in_subject(subject, key), rest),
        None => ensure_jira_in_subject(message, key),
    }
}

/// Prepend an installed skill (its body + `references/`, via `resolve_skill_inline`)
/// ahead of a built-in draft prompt — the same shape `run_review_core` uses to
/// inline its review lenses, so the method travels in the prompt on any provider.
/// An empty skill string ⇒ the prompt is returned byte-for-byte unchanged, so an
/// un-installed skill is a no-op (identical to the prior drafting behaviour).
pub(crate) fn compose_draft_prompt(skill_text: &str, base_prompt: &str) -> String {
    if skill_text.is_empty() {
        base_prompt.to_string()
    } else {
        format!("{skill_text}\n\n---\n\n{base_prompt}")
    }
}

#[cfg(test)]
mod commit_pr_draft_tests {
    use super::{compose_draft_prompt, jira_key_from_branch};

    #[test]
    fn jira_key_parsed_from_branch() {
        assert_eq!(
            jira_key_from_branch("feature/PROJ-16232-rate-limit").as_deref(),
            Some("PROJ-16232")
        );
        assert_eq!(
            jira_key_from_branch("PROJ-445").as_deref(),
            Some("PROJ-445")
        );
        assert_eq!(
            jira_key_from_branch("bugfix/PROJ-7").as_deref(),
            Some("PROJ-7")
        );
        assert_eq!(jira_key_from_branch("main"), None);
        assert_eq!(jira_key_from_branch("hotfix/no-key-here"), None);
        // An uppercase run without a numeric suffix is not a key.
        assert_eq!(jira_key_from_branch("release/NOTES-final"), None);
    }

    #[test]
    fn jira_prefixed_when_missing_left_alone_when_present() {
        use super::{ensure_jira_in_commit, ensure_jira_in_subject};
        // Subject: prefix when absent, untouched when already referenced anywhere.
        assert_eq!(
            ensure_jira_in_subject("feat(bonus): add id column", "PROJ-16519"),
            "PROJ-16519 feat(bonus): add id column"
        );
        assert_eq!(
            ensure_jira_in_subject("PROJ-16519 feat: add id", "PROJ-16519"),
            "PROJ-16519 feat: add id"
        );
        assert_eq!(
            ensure_jira_in_subject("feat: add id (PROJ-16519)", "PROJ-16519"),
            "feat: add id (PROJ-16519)"
        );
        // Commit: only the subject line is touched; the body is preserved verbatim.
        assert_eq!(
            ensure_jira_in_commit("fix: thing\n\n- detail one\n- detail two", "PROJ-7"),
            "PROJ-7 fix: thing\n\n- detail one\n- detail two"
        );
        // Single-line commit message.
        assert_eq!(
            ensure_jira_in_commit("chore: bump", "AB-1"),
            "AB-1 chore: bump"
        );
    }

    #[test]
    fn compose_passthrough_when_no_skill() {
        assert_eq!(compose_draft_prompt("", "BASE PROMPT"), "BASE PROMPT");
    }

    #[test]
    fn compose_prepends_skill() {
        assert_eq!(
            compose_draft_prompt("SKILL", "BASE"),
            "SKILL\n\n---\n\nBASE"
        );
    }
}

#[cfg(test)]
mod staged_skill_package_tests {
    use super::stage_skill_packages_at;

    #[test]
    fn bundled_fallback_materializes_native_and_neutral_views() {
        let tmp = tempfile::tempdir().unwrap();
        let library = otto_context::Library::new(tmp.path().join("library"));
        let bundle = tmp.path().join("bundle");
        let staged =
            stage_skill_packages_at(&library, &["skills-reviewer".to_string()], &bundle).unwrap();
        assert_eq!(staged.root, bundle.to_string_lossy());
        assert!(staged.files["skills-reviewer"]
            .iter()
            .any(|path| path == "SKILL.md"));
        assert!(staged.files["skills-reviewer"]
            .iter()
            .any(|path| path == "references/review-rubric.md"));
        for root in [bundle.join("skills"), bundle.join(".claude/skills")] {
            assert!(root.join("skills-reviewer/SKILL.md").is_file());
            assert!(root
                .join("skills-reviewer/references/review-rubric.md")
                .is_file());
            assert!(root
                .join("skills-reviewer/scripts/skill_review.py")
                .is_file());
        }
    }

    #[test]
    fn library_package_takes_precedence_over_bundled_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let library_root = tmp.path().join("library");
        let library_skill = library_root.join("skills/skills-reviewer");
        std::fs::create_dir_all(library_skill.join("references")).unwrap();
        std::fs::write(library_skill.join("SKILL.md"), "LIBRARY COPY").unwrap();
        std::fs::write(library_skill.join("references/local.md"), "local").unwrap();
        let library = otto_context::Library::new(&library_root);
        let bundle = tmp.path().join("bundle");

        let staged =
            stage_skill_packages_at(&library, &["skills-reviewer".to_string()], &bundle).unwrap();

        assert_eq!(
            std::fs::read_to_string(bundle.join("skills/skills-reviewer/SKILL.md")).unwrap(),
            "LIBRARY COPY"
        );
        assert_eq!(
            staged.files["skills-reviewer"],
            vec!["SKILL.md".to_string(), "references/local.md".to_string()]
        );
    }

    #[test]
    fn partial_staging_reports_each_successful_package() {
        let tmp = tempfile::tempdir().unwrap();
        let library_root = tmp.path().join("library");
        let broken = library_root.join("skills/broken-skill");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("SKILL.md"), "BROKEN FALLBACK BODY").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(broken.join("missing"), broken.join("dangling")).unwrap();
        let library = otto_context::Library::new(library_root);
        let bundle = tmp.path().join("bundle");
        let staged = stage_skill_packages_at(
            &library,
            &["skills-reviewer".to_string(), "broken-skill".to_string()],
            &bundle,
        )
        .unwrap();

        assert!(staged.files.contains_key("skills-reviewer"));
        #[cfg(unix)]
        {
            assert!(!staged.files.contains_key("broken-skill"));
            assert!(!bundle.join("skills/broken-skill").exists());
            assert!(!bundle.join(".claude/skills/broken-skill").exists());
        }
    }

    #[test]
    fn unsafe_names_never_escape_or_remove_outside_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let library = otto_context::Library::new(tmp.path().join("library"));
        let bundle = tmp.path().join("bundle");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let sentinel = outside.join("keep.txt");
        std::fs::write(&sentinel, "keep").unwrap();

        let names = vec![
            outside.to_string_lossy().into_owned(),
            "../outside".to_string(),
            "safe/../../outside".to_string(),
        ];
        assert!(stage_skill_packages_at(&library, &names, &bundle).is_none());
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "keep");
    }
}

/// Tolerantly pull `{title, description}` out of an agent reply (which may wrap
/// the JSON in prose or a markdown fence). Falls back to using the branch name
/// as the title and the whole reply as the description.
pub(crate) fn parse_pr_draft(text: &str, fallback_title: &str) -> (String, String) {
    if let (Some(s), Some(e)) = (text.find('{'), text.rfind('}')) {
        if e > s {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[s..=e]) {
                let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("").trim();
                let desc = v
                    .get("description")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .trim();
                if !title.is_empty() || !desc.is_empty() {
                    let title = if title.is_empty() {
                        fallback_title
                    } else {
                        title
                    };
                    return (title.to_string(), desc.to_string());
                }
            }
        }
    }
    (fallback_title.to_string(), text.trim().to_string())
}

/// `POST /repos/{id}/pr/draft` — draft a PR title + description from the current
/// branch's diff against `base`, using the configured default agent CLI.
async fn draft_pr(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<otto_core::api::DraftPrReq>,
) -> crate::error::ApiResult<Json<otto_core::api::DraftPrResp>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    // Empty base ⇒ detect the repo's default branch instead of erroring.
    let want = Some(body.base.as_str()).filter(|s| !s.trim().is_empty());
    let ws = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;
    // `head` = the branch the PR is FOR (the modal's Source). Absent ⇒ the
    // checked-out branch, as before. A named head is diffed as
    // merge-base(base, head)..head, so drafting a PR for a branch that isn't
    // checked out describes THAT branch, not whatever HEAD is.
    let head = body
        .head
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty());
    let resp = draft_pr_core_for(&ctx, &ws, &user, &repo.path, want, head)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(resp))
}

/// Core PR-draft logic shared by the `POST /repos/{id}/pr/draft` handler and the
/// Run with Otto engine: diff the checkout at `repo_path` against `base`, draft a
/// title + description with the `pull-request` skill, and return them. The caller
/// owns auth/HTTP concerns. `repo_path` may be a worktree (Run with Otto passes
/// the run's `otto-run/<id>` worktree). `base: None` (or a ref that doesn't
/// exist here) falls back to the repo's detected default branch — never a
/// fabricated `main`, so a master-/develop-based repo can't exit 128.
pub(crate) async fn draft_pr_core(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user: &otto_core::domain::User,
    repo_path: &str,
    base: Option<&str>,
) -> Result<otto_core::api::DraftPrResp> {
    draft_pr_core_for(ctx, ws, user, repo_path, base, None).await
}

/// [`draft_pr_core`] for an explicit source branch `head` (`None` ⇒ the
/// checkout's current branch). The reply's `source_branch` is `head`.
pub(crate) async fn draft_pr_core_for(
    ctx: &ServerCtx,
    ws: &otto_core::domain::Workspace,
    user: &otto_core::domain::User,
    repo_path: &str,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<otto_core::api::DraftPrResp> {
    // Runs as a REAL Otto session, not a throwaway orchestrator PTY. Drafting
    // blocks a modal for as long as it takes, and the old headless PTY gave the
    // user nothing to look at — no way to tell "thinking" from "wedged". As a
    // session it appears in Agents the moment it is created (`on_ready` fires
    // before the turn completes), so it can be opened in a pane and watched, or
    // talked to, while the modal is still spinning.
    //
    // `lean_turn` + the configured drafting model are what make it quick: the
    // prompt already carries the diff and the `pull-request` skill, so MCP
    // servers and tool round trips are pure latency (see `lean_turn_args`).
    //
    // The workflow git_pr node uses `pr_draft_prompt` + `run_node_agent` instead
    // so it can honor the node's chosen Provider/Model.
    let (prompt, source, base_branch) = pr_draft_prompt_for(ctx, repo_path, base, head).await?;
    let model = pr_draft_model(ctx).await;
    let meta = serde_json::json!({
        "source": "pr-draft",
        "model": model,
        "lean_turn": true,
    });
    let title = format!("PR draft · {source}");
    let (reply, session_id) = crate::agent_session::run_session_turn(
        ctx,
        ws,
        user,
        None,
        &title,
        repo_path,
        "claude",
        meta,
        &prompt,
        DRAFT_STUCK_AFTER,
        |_id| {},
    )
    .await
    .map_err(|e| e.0)?;
    let (mut title, description) = parse_pr_draft(&reply, &source);
    if let Some(key) = jira_key_from_branch(&source) {
        title = ensure_jira_in_subject(&title, &key);
    }
    Ok(otto_core::api::DraftPrResp {
        title,
        description,
        source_branch: source,
        target_branch: base_branch,
        session_id: Some(session_id),
    })
}

/// Idle trip for a drafting turn. Generous next to how long the turn *should*
/// take (seconds), because the session is visible: a user watching it stall is
/// better served by a live session they can inspect than by a fast error.
const DRAFT_STUCK_AFTER: std::time::Duration = std::time::Duration::from_secs(300);

/// The model that drafts PR titles/descriptions and commit messages, from the
/// `pr_draft_model` setting. Falls back to the fast default when unset — never
/// to the user's default agent model, which is what made drafting take minutes.
pub(crate) async fn pr_draft_model(ctx: &ServerCtx) -> String {
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = repo
        .get(otto_state::PR_DRAFT_MODEL_KEY)
        .await
        .ok()
        .flatten();
    otto_state::pr_draft_model_from(value.as_ref())
}

/// Build the PR-draft agent prompt for the checkout at `repo_path` vs `base`
/// (diff → title/description instructions + the installed `pull-request` skill).
/// Returns `(prompt, source_branch, resolved_base_branch)`. Errors when there is
/// no diff. Shared by [`draft_pr_core`] (HTTP, claude) and the workflow `git_pr`
/// node (honors the node's provider) so both craft the message identically.
pub(crate) async fn pr_draft_prompt(
    ctx: &ServerCtx,
    repo_path: &str,
    base: Option<&str>,
) -> Result<(String, String, String)> {
    pr_draft_prompt_for(ctx, repo_path, base, None).await
}

/// [`pr_draft_prompt`] for an explicit source branch `head`: the diff is
/// `merge-base(base, head)..head` (committed work on that branch only), and
/// the prompt/`source` name `head`. `None` ⇒ the checked-out branch + its
/// working tree, exactly as before.
pub(crate) async fn pr_draft_prompt_for(
    ctx: &ServerCtx,
    repo_path: &str,
    base: Option<&str>,
    head: Option<&str>,
) -> Result<(String, String, String)> {
    let git = otto_git::LocalGit::new(repo_path);
    let source = match head {
        Some(h) => h.to_string(),
        None => git.current_branch().await?,
    };
    let resolved = git.resolve_base(base).await?;
    let base = resolved.branch.as_str();
    // Cap the diff fed to the drafting agent — a title/description doesn't need
    // every line, and a huge prompt is slow + can exceed input limits. git is
    // stopped at twice the cap (enough to know it was cut) instead of the whole
    // patch being buffered just to keep 40 KB of it.
    const MAX_DIFF: usize = 40_000;
    let (diff, _) = match head {
        Some(h) => {
            git.range_diff_text_capped(&resolved.diff_ref, h, 2 * MAX_DIFF)
                .await?
        }
        None => {
            git.diff_text_capped(Some(&resolved.diff_ref), 2 * MAX_DIFF)
                .await?
        }
    };
    if diff.trim().is_empty() {
        return Err(Error::Invalid(format!(
            "no changes between '{source}' and '{base}'"
        )));
    }
    let truncated = diff.len() > MAX_DIFF;
    let diff_slice = if truncated {
        let mut end = MAX_DIFF;
        while end > 0 && !diff.is_char_boundary(end) {
            end -= 1;
        }
        &diff[..end]
    } else {
        diff.as_str()
    };

    // Seed the Jira key from the branch (best-effort). It belongs in the PR
    // TITLE only — a key in the body auto-links in some clients (GitKraken) and
    // can crash them. Enrichment (issue summary) is left to the installed skill.
    let jira = match jira_key_from_branch(&source) {
        Some(key) => format!(
            "This branch is for Jira issue {key}. Use `{key}` as the PR title prefix \
             (e.g. `{key} <summary>`); do NOT put the key, a Jira link, or a Jira \
             hostname anywhere in the description body.\n"
        ),
        None => String::new(),
    };

    let base_prompt = format!(
        "You are preparing a pull request from branch `{source}` into `{base}`. Based ONLY on \
         the diff below, write:\n\
         - a concise, imperative PR title (max ~72 chars, no trailing period)\n\
         - a clear PR description in Markdown: a one-line summary, then a \"What changed\" bullet \
         list, then \"Testing\" notes if any are evident from the diff.\n\
         {jira}\
         Reply with ONLY a JSON object, no prose and no markdown fence: \
         {{\"title\": \"...\", \"description\": \"...\"}}.\n\n\
         {trunc}DIFF:\n{diff}",
        source = source,
        base = base,
        jira = jira,
        trunc = if truncated {
            "(diff truncated for brevity)\n\n"
        } else {
            ""
        },
        diff = diff_slice,
    );
    // Prepend the installed `pull-request` skill (if any). Un-installed ⇒ no-op.
    let skill_text = resolve_skill_inline(&ctx.context_library, "pull-request");
    let prompt = compose_draft_prompt(&skill_text, &base_prompt);

    Ok((prompt, source, base.to_string()))
}

/// Launch an AI review on a specific branch/worktree (Run with Otto's `reviewing`
/// stage and the workflow `review_run` node). Resolves `base` (an explicit
/// ref/SHA, or `None` ⇒ the repo's detected default branch — never a
/// fabricated `main`), diffs `worktree_path` against it, creates a
/// local-review row keyed to `repo_id`, and drives `run_review` in the
/// background. Returns the `review_id` plus the base actually used, so
/// callers publish the RESOLVED branch (a downstream PR must target what was
/// really reviewed); the caller polls `reviews_store.get_review` for
/// `Done`/`Error` and reads counts via [`review_findings_counts`].
///
/// The third return value is `no_changes`: the diff vs the resolved base was
/// EMPTY, so the review completed instantly with no reviewers and no findings.
/// Callers MUST NOT read that as a clean review — zero findings there means
/// nothing was looked at. Scoring it (100/PASS) is how a misconfigured base
/// silently green-lights an unreviewed PR.
///
/// `jira_context` / `run_context` are what the caller already knows about the
/// change — the ticket, and the briefs earlier steps produced. Both reach every
/// reviewer's prompt. Passing `None` makes the reviewers re-derive the system
/// from raw hunks, which is how documented, intentional behavior gets reported
/// as a defect of the change that happened to touch its line.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_review_for_branch(
    ctx: &ServerCtx,
    repo_id: &Id,
    worktree_path: &str,
    base: Option<&str>,
    cfg_override: Option<ReviewConfig>,
    jira_context: Option<String>,
    run_context: Option<String>,
    mode_override: Option<otto_core::domain::ReviewMode>,
) -> Result<(Id, otto_git::ResolvedBase, bool)> {
    run_review_for_branch_sized(
        ctx,
        repo_id,
        worktree_path,
        base,
        cfg_override,
        jira_context,
        run_context,
        mode_override,
    )
    .await
    .map(|(id, resolved, no_changes, _)| (id, resolved, no_changes))
}

/// [`run_review_for_branch`] that also returns the reviewed diff's byte
/// length — the caller's wait budget is sized off it, and recomputing the
/// whole review diff (one git spawn per untracked file) just for its length
/// doubled the stage's git work.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_review_for_branch_sized(
    ctx: &ServerCtx,
    repo_id: &Id,
    worktree_path: &str,
    base: Option<&str>,
    cfg_override: Option<ReviewConfig>,
    jira_context: Option<String>,
    run_context: Option<String>,
    mode_override: Option<otto_core::domain::ReviewMode>,
) -> Result<(Id, otto_git::ResolvedBase, bool, usize)> {
    let repo = ctx.git_store.get_repo(repo_id).await?;
    let workspace = ctx.workspaces.get(&repo.workspace_id).await?;
    review_budget_gate(ctx, &repo.workspace_id).await?;
    let git = otto_git::LocalGit::new(worktree_path);
    let resolved = git.resolve_base(base).await?;
    let diff_text = git.review_diff_text(&resolved.diff_ref).await?;
    let review = ctx
        .reviews_store
        .create_review(repo_id, LOCAL_REVIEW_PR_NUMBER)
        .await?;
    let review_id = review.id.clone();
    let no_changes = diff_text.trim().is_empty();
    let diff_len = diff_text.len();

    if no_changes {
        // No changes vs base — complete immediately with no findings. Loud,
        // because "0 findings" here is indistinguishable from a clean review in
        // every downstream count; the caller gets `no_changes` to tell them apart.
        tracing::warn!(
            review = %review_id, worktree = %worktree_path, base = %resolved.diff_ref,
            "review: EMPTY diff vs base — no reviewers ran, no findings are possible"
        );
        ctx.reviews_store
            .set_status(&review_id, ReviewStatus::Done, None)
            .await?;
        let _ = ctx.events.send(Event::ReviewChanged {
            workspace_id: workspace.id.clone(),
            session_id: None,
            review_id: review_id.clone(),
            status: ReviewStatus::Done.as_str().to_string(),
        });
    } else {
        let _ = ctx.events.send(Event::ReviewChanged {
            workspace_id: workspace.id.clone(),
            session_id: None,
            review_id: review_id.clone(),
            status: ReviewStatus::Running.as_str().to_string(),
        });
        let ctx_bg = ctx.clone();
        let rid = review_id.clone();
        let wt = worktree_path.to_string();
        let repo_id_bg = repo_id.clone();
        // The worktree already holds the branch's real code; name both sides
        // for the reviewers. A SHA base shows as itself — still meaningful.
        let dest = resolved.branch.clone();
        let branches = git
            .current_branch()
            .await
            .ok()
            .filter(|c| !c.is_empty())
            .map(|source| ReviewBranches {
                source,
                dest,
                checked_out: true,
            });
        tokio::spawn(async move {
            run_review(
                ctx_bg,
                rid,
                wt,
                diff_text,
                jira_context,
                run_context,
                workspace,
                repo_id_bg,
                0,
                branches,
                cfg_override,
                mode_override,
            )
            .await;
        });
    }
    Ok((review_id, resolved, no_changes, diff_len))
}

/// Tolerantly extract a commit message from an agent reply. The agent is asked
/// to reply with ONLY the message, but it may still wrap it in a markdown fence
/// or add a stray preamble — strip a leading/trailing ``` fence and trim.
fn parse_commit_draft(text: &str) -> String {
    let trimmed = text.trim();
    // Strip a surrounding ```…``` fence if present (with or without a language
    // tag on the opening line).
    if let Some(rest) = trimmed.strip_prefix("```") {
        if let Some(end) = rest.rfind("```") {
            let inner = &rest[..end];
            // Drop the (possibly empty) language tag on the first line.
            let body = inner.split_once('\n').map(|(_, b)| b).unwrap_or(inner);
            return body.trim().to_string();
        }
    }
    trimmed.to_string()
}

/// `POST /repos/{id}/draft-commit-message` — draft a Conventional Commits–style
/// commit message from the STAGED diff (falls back to the full working diff when
/// nothing is staged), using the configured default agent CLI. Symmetric with
/// `draft_pr`, but scoped to what's about to be committed.
async fn draft_commit_message(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<otto_core::api::DraftCommitMessageResp>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let git = otto_git::LocalGit::new(&repo.path);
    // Prefer the staged diff (what's actually about to be committed). When the
    // index is empty, fall back to the full working diff so the button is still
    // useful before staging.
    //
    // Cap the diff fed to the drafting agent — a commit message doesn't need
    // every line, and a huge prompt is slow + can exceed input limits. git is
    // stopped at twice the cap instead of buffering the whole patch.
    const MAX_DIFF: usize = 40_000;
    let (staged, _) = git
        .staged_diff_text_capped(2 * MAX_DIFF)
        .await
        .map_err(crate::error::ApiError)?;
    let (diff, from_staged) = if staged.trim().is_empty() {
        let (working, _) = git
            .diff_text_capped(None, 2 * MAX_DIFF)
            .await
            .map_err(crate::error::ApiError)?;
        (working, false)
    } else {
        (staged, true)
    };
    if diff.trim().is_empty() {
        return Err(crate::error::ApiError(Error::Invalid(
            "nothing to commit — no staged or unstaged changes".into(),
        )));
    }
    let truncated = diff.len() > MAX_DIFF;
    let diff_slice = if truncated {
        let mut end = MAX_DIFF;
        while end > 0 && !diff.is_char_boundary(end) {
            end -= 1;
        }
        &diff[..end]
    } else {
        diff.as_str()
    };

    let scope = if from_staged {
        "the STAGED changes (what is about to be committed)"
    } else {
        "the working-tree changes (nothing is staged yet)"
    };
    // Seed the Jira key from the current branch (best-effort) so the subject
    // carries it. Enrichment (issue summary) is left to the installed skill.
    let branch = git.current_branch().await.unwrap_or_default();
    let jira = match jira_key_from_branch(&branch) {
        Some(key) => format!(
            "This change is for Jira issue {key}. Prefix the subject line with `{key}` \
             (e.g. `{key} type(scope): summary`); do NOT repeat the key in the body. "
        ),
        None => String::new(),
    };
    let base_prompt = format!(
        "You are writing a git commit message for {scope}. Based ONLY on the diff below, write:\n\
         - a Conventional Commits subject line: `type(scope): summary` (type ∈ feat, fix, docs, \
         style, refactor, perf, test, build, ci, chore; scope optional; imperative mood; \
         ≤72 chars; no trailing period)\n\
         - an optional body (blank line, then a short bullet list of WHAT changed and WHY) when \
         the change is non-trivial; omit the body for tiny changes.\n\
         Infer and honor any repo convention you can see in the diff (e.g. an emoji prefix or a \
         scope naming style). {jira}Reply with ONLY the raw commit message text — no prose, no \
         explanation, and no markdown code fence.\n\n\
         {trunc}DIFF:\n{diff}",
        scope = scope,
        jira = jira,
        trunc = if truncated {
            "(diff truncated for brevity)\n\n"
        } else {
            ""
        },
        diff = diff_slice,
    );
    // Prepend the installed `commit-message` skill (if any). Un-installed ⇒ no-op.
    let skill_text = resolve_skill_inline(&ctx.context_library, "commit-message");
    let prompt = compose_draft_prompt(&skill_text, &base_prompt);

    // Same path as `draft_pr_core`: a REAL, visible session (watchable from
    // the WIP panel while it runs) on the configured drafting model with
    // `lean_turn` — not the headless orchestrator PTY on the user's default
    // model, which was both invisible and slow.
    let ws = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;
    let model = pr_draft_model(&ctx).await;
    let meta = serde_json::json!({
        "source": "commit-draft",
        "model": model,
        "lean_turn": true,
    });
    let title = format!(
        "Commit draft · {}",
        if branch.trim().is_empty() {
            "HEAD"
        } else {
            branch.as_str()
        }
    );
    let (reply, session_id) = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        None,
        &title,
        &repo.path,
        "claude",
        meta,
        &prompt,
        DRAFT_STUCK_AFTER,
        |_id| {},
    )
    .await?;
    let message = parse_commit_draft(&reply);
    // Always carry the Jira key in the subject line (the agent is asked to, but
    // forgets under the Conventional-Commits format) — same guarantee as the PR.
    let message = match jira_key_from_branch(&branch) {
        Some(key) => ensure_jira_in_commit(&message, &key),
        None => message,
    };

    Ok(Json(otto_core::api::DraftCommitMessageResp {
        message,
        from_staged,
        session_id: Some(session_id),
    }))
}

/// Agent rows a Retry may restart: settled ones only (`pending` / `running` /
/// `waiting` still belong to a live recovery loop).
pub(crate) fn agent_retryable(status: &str) -> bool {
    matches!(status, "done" | "error" | "skipped")
}

/// Re-run a single review agent (e.g. one that never received its prompt). Uses
/// the prompt persisted when the review started, kills the agent's old (stuck)
/// session, and spawns a fresh one in the background.
async fn retry_review_agent(
    Path((review_id, index)): Path<(Id, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    // The id is a route param used as a temp-file name component below
    // (`otto-review-<id>.diff` + the legacy prompt file) — reject traversal
    // before it can reach a path join.
    if otto_core::paths::safe_component(&review_id).is_none() {
        return Err(crate::error::ApiError(Error::NotFound("review".into())));
    }
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    let workspace = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    if index >= review.agents.len() {
        return Err(crate::error::ApiError(Error::Invalid(format!(
            "no review agent at index {index}"
        ))));
    }
    // Only a SETTLED agent is retried. Retrying a live one archived its session
    // and replaced its cancel flag: the original recovery loop then respawned
    // into the same index + findings file, and its orphaned flag left neither
    // Stop nor Cancel able to reach it. A stuck agent is Stopped first.
    if !agent_retryable(&review.agents[index].status) {
        return Err(crate::error::ApiError(Error::Conflict(format!(
            "agent {index} is still {} — stop it before retrying",
            review.agents[index].status
        ))));
    }
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;
    // DB-first (durable across reboot / temp-sweep / redeploy); the temp file
    // is the legacy fallback for reviews that predate migration 0100.
    let prompt = match ctx.reviews_store.get_agent_prompt(&review_id, index).await {
        Ok(Some(p)) => p,
        _ => std::fs::read_to_string(crate::review_session::prompt_path(&review_id, index))
            .map_err(|_| {
                crate::error::ApiError(Error::Invalid(
                    "cannot retry: this agent's prompt is no longer available — re-run the review"
                        .into(),
                ))
            })?,
    };

    let review_user = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .ok()
        .and_then(|us| us.into_iter().find(|u| u.is_root))
        .ok_or_else(|| crate::error::ApiError(Error::Internal("no root user".into())))?;

    // Kill the old (likely stuck) session so it doesn't linger.
    if let Some(sid) = review.agents[index].session_id.clone() {
        let _ = ctx.manager.archive(&sid).await;
    }

    // Mark the agent pending again so the UI reflects the retry immediately.
    // Write only this agent's element (atomic per-index) so we never revert the
    // other agents' live rows.
    let states: crate::review_session::SharedStates =
        Arc::new(tokio::sync::Mutex::new(review.agents.clone()));
    {
        let row = {
            let mut g = states.lock().await;
            g.get_mut(index).map(|s| {
                s.status = "pending".into();
                s.note = "retrying…".into();
                s.session_id = None;
                s.findings = Vec::new();
                s.comment_count = 0;
                s.clone()
            })
        };
        if let Some(row) = row {
            let _ = ctx
                .reviews_store
                .set_agent_at(&review_id, index, &row)
                .await;
        }
    }

    let provider = review.agents[index].provider.clone();
    // Retry this reviewer with the SAME model it was configured with (empty →
    // provider default) so the retry matches the original run.
    let model = review.agents[index].model.clone();
    let pr_number = review.pr_number;
    // The prompt references the diff via an absolute temp path — re-materialize
    // it from the durable row if a reboot/temp-sweep removed it (best-effort;
    // pre-0100 reviews have no stored diff and keep the old behavior). The
    // restored length also feeds the grace-period heuristic, which previously
    // collapsed to the 10-minute floor whenever the temp file was gone.
    let diff_file = std::env::temp_dir().join(format!("otto-review-{review_id}.diff"));
    let mut diff_len = std::fs::metadata(&diff_file)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    if diff_len == 0 {
        if let Ok(Some(diff)) = ctx.reviews_store.get_diff(&review_id).await {
            diff_len = diff.len();
            let _ = std::fs::write(&diff_file, diff);
        }
    }
    let timeout = review_agent_timeout(diff_len, None);
    let manager = Arc::clone(&ctx.manager);
    let reviews = ctx.reviews_store.clone();
    let review_id_bg = review_id.clone();
    // Re-stage the FULL skill library (idempotent, shared bundle) so a retried
    // CLAUDE agent can invoke `Skill(<lens>)` exactly like the original run —
    // which also stages the whole library (a review config usually carries the
    // lens in the agent NAME with `skill` empty, so filtering on `skill` here
    // would leave the common case with an empty bundle). codex/agy ignore the
    // bundle and re-read the lens method inline from the persisted prompt.
    let review_skills_dir = {
        let names: Vec<String> = ctx
            .context_library
            .list_skills()
            .into_iter()
            .map(|s| s.name)
            .collect();
        stage_review_skills(&ctx.context_library, &names)
    };
    // Register a FRESH per-agent cancel flag (replacing any tripped one from a
    // prior Stop) so the retried agent is stoppable exactly like the original.
    let agent_cancel = register_review_agent_cancel(&ctx.review_agent_cancels, &review_id, index);
    let agent_cancels_reg = ctx.review_agent_cancels.clone();
    let slots = reviewer_slots();
    let ctx_bg = ctx.clone();
    tokio::spawn(async move {
        let _slot = slots.acquire_owned().await.ok(); // SI-11 reviewer cap
                                                      // A PR reviewer must verify against the PR head, not the user's
                                                      // checkout (the original run's worktree is gone) — rebuild one, unique
                                                      // per retry so concurrent retries never tear down each other's tree.
        let suffix = format!("-a{index}-{}", chrono::Utc::now().timestamp_millis());
        let (cwd, pr_wt) = if pr_number != 0 {
            let wt =
                pr_retry_worktree(&ctx_bg, &user, &repo, &review_id_bg, pr_number, &suffix).await;
            let cwd = wt
                .as_ref()
                .map_or_else(|| repo.path.clone(), |w| w.path.clone());
            (cwd, wt)
        } else {
            // A branch review re-enters the checkout it ran in (S2-06), not
            // the user's main checkout on whatever branch it is on now.
            branch_retry_checkout(&ctx_bg, &repo, &review_id_bg, &suffix).await
        };
        crate::review_session::run_agent_session_with_recovery(
            &manager,
            &reviews,
            &states,
            &workspace,
            &review_user,
            &provider,
            &model,
            &cwd,
            &review_id_bg,
            index,
            &prompt,
            timeout,
            None,
            Some(&agent_cancel), // per-agent Stop works on retried agents too
            review_skills_dir.as_deref(),
            // The persisted prompt carries the lens list, but the retry route
            // does not parse it — the guard's note falls back to today's text.
            &[],
        )
        .await;
        if let Some(wt) = pr_wt {
            teardown_pr_worktree(&repo.path, wt).await;
        }
        unregister_review_agent_cancel(&agent_cancels_reg, &review_id_bg, index);
    });

    Ok(Json(
        ctx.reviews_store
            .get_review(&review_id)
            .await
            .map_err(crate::error::ApiError)?,
    ))
}

/// `POST /reviews/{review_id}/summarizer/retry` — re-run ONLY the summarize +
/// persist stage from the STORED per-agent findings, replacing the review's
/// unposted draft comments. Cheap compared to re-running every reviewer: the
/// findings already exist; only the final dedupe/rank turn is repeated.
async fn retry_summarizer(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    let workspace = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    if review.status == ReviewStatus::Running {
        return Err(crate::error::ApiError(Error::Invalid(
            "review is still running — wait for it to finish first".into(),
        )));
    }
    if review.agents.len() < 2 {
        return Err(crate::error::ApiError(Error::Invalid(
            "this review has no summarizer stage to retry".into(),
        )));
    }
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    // The reviewers' findings are durable on the agent rows — the summarizer is
    // always the last row and contributes none.
    let agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> = review.agents
        [..review.agents.len() - 1]
        .iter()
        .map(|a| a.findings.clone())
        .collect();
    // Only findings with actual content count — a run whose findings were
    // gutted (empty bodies) has nothing to summarize and needs a re-run.
    if !agent_findings
        .iter()
        .flatten()
        .any(|f| !f.body.trim().is_empty())
    {
        return Err(crate::error::ApiError(Error::Invalid(
            "no stored agent findings with content to summarize — re-run the review instead".into(),
        )));
    }

    // Flip the run back to running so the UI tracks the re-summarize live —
    // atomically: the status check above is a fast-path, this is the guard.
    // Two quick clicks must not both re-summarize (each would persist a full
    // set of drafts).
    if !ctx
        .reviews_store
        .try_begin_rerun(&review_id)
        .await
        .map_err(crate::error::ApiError)?
    {
        return Err(crate::error::ApiError(Error::Invalid(
            "review is still running — wait for it to finish first".into(),
        )));
    }
    // Each retry has its own cancellation flag; a cancelled earlier attempt
    // must not poison the new session, and Cancel must cover this task too
    // (registered only once this attempt owns the review).
    let attempt_cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut map) = ctx.review_cancels.lock() {
        map.insert(review_id.clone(), attempt_cancel.clone());
    }
    let _ = ctx.events.send(Event::ReviewChanged {
        workspace_id: workspace.id.clone(),
        session_id: None,
        review_id: review_id.clone(),
        status: ReviewStatus::Running.as_str().to_string(),
    });

    let ctx_bg = ctx.clone();
    let review_id_bg = review_id.clone();
    let repo_id = review.repo_id.clone();
    let pr_number = review.pr_number;
    tokio::spawn(async move {
        let _cancel_guard =
            ReviewCancelGuard::new(&ctx_bg.review_cancels, &review_id_bg, &attempt_cancel);
        // Fingerprints anchor on the flagged line's TEXT; a PR review's lines
        // are in the PR head, not the user's checkout. Rebuild a head worktree
        // for anchoring so the retry re-keys onto the original findings.
        let (repo_path, pr_wt) = if pr_number != 0 {
            let wt =
                pr_retry_worktree(&ctx_bg, &user, &repo, &review_id_bg, pr_number, "-sum").await;
            let path = wt
                .as_ref()
                .map_or_else(|| repo.path.clone(), |w| w.path.clone());
            (path, wt)
        } else {
            // Anchor on the branch review's own checkout/head (S2-06): the
            // user's checkout holds other line text and would re-key nothing.
            branch_retry_checkout(&ctx_bg, &repo, &review_id_bg, "-sum").await
        };
        // The summarizer follows the repo's EFFECTIVE config (per-repo binding
        // resolution included), same as a fresh run would.
        let cfg = load_review_config_for_repo(&ctx_bg, &repo_id).await;
        // A re-run replaces the previous draft comments; posted/approved/
        // declined ones are user decisions and stay untouched.
        match ctx_bg
            .reviews_store
            .delete_draft_comments(&review_id_bg)
            .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(review = %review_id_bg, "summarizer retry: cleared {n} draft comments")
            }
            Err(e) => {
                tracing::warn!(review = %review_id_bg, "summarizer retry: clear drafts failed: {e}")
            }
            _ => {}
        }
        let result = summarize_and_persist(
            &ctx_bg,
            &review_id_bg,
            &repo_path,
            &workspace,
            &user,
            &repo_id,
            pr_number,
            &cfg.summarizer,
            &agent_findings,
            // A retry re-runs ONLY this stage, from findings stored earlier; the
            // run that produced them is long gone, so there is no context to
            // re-render. The findings themselves are unchanged either way.
            "",
            // The original run's cwd/head is gone (a PR worktree is torn down
            // after the run; the head may have moved since), so this attempt
            // cannot prove a finding absent — it only re-summarizes.
            Some("summarizer retry"),
        )
        .await;
        if let Some(wt) = pr_wt {
            teardown_pr_worktree(&repo.path, wt).await;
        }
        if attempt_cancel.load(std::sync::atomic::Ordering::SeqCst)
            || review_is_cancelled(&ctx_bg, &review_id_bg).await
        {
            return;
        }
        let status = match result {
            Ok(()) => {
                tracing::info!(review = %review_id_bg, "summarizer retry complete");
                ReviewStatus::Done
            }
            Err(e) => {
                tracing::warn!(review = %review_id_bg, "summarizer retry failed: {e}");
                let _ = ctx_bg
                    .reviews_store
                    .set_status(&review_id_bg, ReviewStatus::Error, Some(&e.to_string()))
                    .await;
                let _ = ctx_bg.events.send(Event::ReviewChanged {
                    workspace_id: workspace.id.clone(),
                    session_id: None,
                    review_id: review_id_bg.clone(),
                    status: ReviewStatus::Error.as_str().to_string(),
                });
                return;
            }
        };
        let _ = ctx_bg
            .reviews_store
            .set_status(&review_id_bg, status, None)
            .await;
        let _ = ctx_bg.events.send(Event::ReviewChanged {
            workspace_id: workspace.id.clone(),
            session_id: None,
            review_id: review_id_bg.clone(),
            status: status.as_str().to_string(),
        });
    });

    Ok(Json(
        ctx.reviews_store
            .get_review(&review_id)
            .await
            .map_err(crate::error::ApiError)?,
    ))
}

/// `POST /reviews/{review_id}/agents/{index}/stop` — stop one running/waiting
/// review agent without touching the rest of the run. Trips the agent's cancel
/// flag FIRST (so the recovery loop sees the kill as intentional, not a failure
/// to auto-retry), kills its live session, and marks the row `error` /
/// "stopped by user" — deliberately NOT a new status value, so no contract enum
/// change and the existing retry path (error rows are retryable) immediately
/// applies. The parent run rolls up exactly as an agent error does today: the
/// stopped agent leaves the pending set and the summarizer proceeds with the
/// remaining findings. 409 unless the row is running/waiting; the trailing
/// summarizer row (no recovery loop of its own) is never stoppable.
async fn stop_review_agent(
    Path((review_id, index)): Path<(Id, usize)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<(StatusCode, Json<Review>)> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    if index >= review.agents.len() {
        return Err(crate::error::ApiError(Error::Invalid(format!(
            "no review agent at index {index}"
        ))));
    }
    if index == review.agents.len() - 1 {
        return Err(crate::error::ApiError(Error::Conflict(
            "the summarizer cannot be stopped".into(),
        )));
    }
    let status = review.agents[index].status.clone();
    if status != "running" && status != "waiting" {
        return Err(crate::error::ApiError(Error::Conflict(format!(
            "agent is {status} — only a running or waiting agent can be stopped"
        ))));
    }

    // Signal the recovery loop FIRST so the kill below reads as intentional
    // (Stopped, not SessionGone-then-auto-retry) — same ordering as Product's
    // per-agent stop.
    signal_review_agent_cancel(&ctx.review_agent_cancels, &review_id, index);
    if let Some(sid) = review.agents[index].session_id.as_ref() {
        let _ = ctx.manager.kill_session(sid).await;
    }
    // Persist the terminal row immediately so the UI reflects the stop without
    // waiting for the recovery loop to unwind (it re-persists the same
    // status/note via review_error_note(Stopped)).
    {
        let mut row = review.agents[index].clone();
        row.status = "error".into();
        row.note = "stopped by user".into();
        let _ = ctx
            .reviews_store
            .set_agent_at(&review_id, index, &row)
            .await;
    }
    // Reuse the existing review WS family so open panels refresh.
    let _ = ctx.events.send(Event::ReviewChanged {
        workspace_id: repo.workspace_id.clone(),
        session_id: None,
        review_id: review_id.to_string(),
        status: ReviewStatus::Running.as_str().to_string(),
    });

    Ok((
        StatusCode::ACCEPTED,
        Json(
            ctx.reviews_store
                .get_review(&review_id)
                .await
                .map_err(crate::error::ApiError)?,
        ),
    ))
}

/// Point-of-action budget gate for EVERY path that spawns reviewer sessions —
/// PR and local reviews, agent/summarizer retries and branch reviews (Run with
/// Otto, workflow `review_run`); it used to guard the PR path only. Checks
/// the workspace-level cap (the provider isn't known yet). A blocked budget is
/// `Invalid` (400) like every other budget gate in the daemon.
pub(crate) async fn review_budget_gate(ctx: &ServerCtx, workspace_id: &str) -> Result<()> {
    let verdict = crate::routes::usage::check_budget(ctx, workspace_id, "").await;
    if verdict.blocked {
        return Err(Error::Invalid(format!(
            "Budget exceeded — review blocked: {}",
            verdict.reason.unwrap_or_else(|| "cap reached".to_string())
        )));
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct RepoPrPath {
    id: Id,
    number: u64,
}

async fn start_review(
    Path(RepoPrPath {
        id: repo_id,
        number,
    }): Path<RepoPrPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<StartReviewReq>>,
) -> crate::error::ApiResult<Json<Review>> {
    // Resolve workspace role via the repo.
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let req = body.map(|b| b.0).unwrap_or_default();

    // Point-of-action budget gate (A1/A2), before touching the DB.
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(ApiError)?;

    // Create the review row (status=running) and return it immediately.
    let review = ctx
        .reviews_store
        .create_review(&repo_id, number)
        .await
        .map_err(crate::error::ApiError)?;

    // Register a cancel flag for this run so POST /reviews/{id}/cancel can stop
    // it. run_review_core looks this up by review_id and threads it into each
    // agent's recovery loop.
    if let Ok(mut map) = ctx.review_cancels.lock() {
        map.insert(
            review.id.to_string(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
    }

    // Spawn the background runner.
    let review_id = review.id.clone();
    let ctx_bg = ctx.clone();
    let repo_path = repo.path.clone();
    let ws_id = repo.workspace_id.clone();
    let issue_account_id = req.issue_account_id;
    let issue_key = req.issue_key;
    let user_context = req.context;
    // Carry the caller into the background task so credential-ownership (S4) is
    // enforced against the user who started the review, not whoever happens to
    // own the repo's bound accounts.
    let review_user = user.clone();
    // Notify subscribers that a review is now running.
    let _ = ctx.events.send(Event::ReviewChanged {
        workspace_id: ws_id.clone(),
        session_id: None,
        review_id: review_id.clone(),
        status: ReviewStatus::Running.as_str().to_string(),
    });
    tokio::spawn(async move {
        let result = run_pr_review_inner(
            &ctx_bg,
            &review_user,
            &review_id,
            &repo_id,
            number,
            issue_account_id,
            issue_key,
            user_context,
        )
        .await;
        // If a Cancel landed while we ran, the cancel handler already set the
        // terminal `cancelled` status + tore everything down — don't overwrite it.
        let was_cancelled = ctx_bg
            .review_cancels
            .lock()
            .ok()
            .and_then(|m| m.get(review_id.as_str()).cloned())
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst));
        if was_cancelled {
            tracing::info!(review = %review_id, "PR review ended after cancel — leaving cancelled status");
        } else {
            match result {
                Ok(()) => {
                    tracing::info!(review = %review_id, "PR review complete");
                    if let Err(e) = ctx_bg
                        .reviews_store
                        .set_status(&review_id, ReviewStatus::Done, None)
                        .await
                    {
                        tracing::error!(review = %review_id, "set status done: {e}");
                    }
                    let _ = ctx_bg.events.send(Event::ReviewChanged {
                        workspace_id: ws_id.clone(),
                        session_id: None,
                        review_id: review_id.clone(),
                        status: ReviewStatus::Done.as_str().to_string(),
                    });
                }
                Err(e) => {
                    tracing::warn!(review = %review_id, "PR review error: {e}");
                    let msg = e.to_string();
                    let _ = ctx_bg
                        .reviews_store
                        .set_status(&review_id, ReviewStatus::Error, Some(&msg))
                        .await;
                    let _ = ctx_bg.events.send(Event::ReviewChanged {
                        workspace_id: ws_id.clone(),
                        session_id: None,
                        review_id: review_id.clone(),
                        status: ReviewStatus::Error.as_str().to_string(),
                    });
                }
            }
        }
        // Drop the cancel flag from the registry now the run is over.
        if let Ok(mut map) = ctx_bg.review_cancels.lock() {
            map.remove(review_id.as_str());
        }
        drop(repo_path); // keep bound
    });

    Ok(Json(review))
}

/// Cancel an in-flight review. Signals the run's cancel flag (short-circuiting
/// the agent recovery loop), kills the live agent PTY sessions, marks the run
/// `cancelled`, cleans up its temp files and broadcasts. 409 if not running.
async fn cancel_review(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    if review.status != ReviewStatus::Running {
        return Err(ApiError(Error::Conflict(format!(
            "review is {} — only a running review can be cancelled",
            review.status.as_str()
        ))));
    }

    cancel_running_review(&ctx, &review, &repo.workspace_id).await;
    Ok(Json(
        ctx.reviews_store
            .get_review(&review_id)
            .await
            .map_err(ApiError)?,
    ))
}

/// Shared cancellation for the review endpoint and workflows that own reviews.
/// Signal the review itself before killing PTYs: a killed summarizer is otherwise
/// indistinguishable from a provider failure eligible for deterministic fallback.
pub(crate) async fn cancel_running_review(ctx: &ServerCtx, review: &Review, workspace_id: &Id) {
    if review.status != ReviewStatus::Running {
        return;
    }
    let review_id = review.id.clone();
    // 1. Signal the cancel flags so each agent's recovery loop short-circuits and
    //    run_review_core skips the summarizer / finding persistence. The review-
    //    level flag gates the post-join summarizer skip; the recovery loops watch
    //    their per-agent flags (per-agent Stop), so trip those too.
    //    Only an attempt that is actually running has a flag; trip it, never
    //    insert one — a tripped orphan would cancel a later rerun of this id
    //    (the durable `cancelled` status below covers flagless paths).
    if let Ok(map) = ctx.review_cancels.lock() {
        if let Some(flag) = map.get(review_id.as_str()) {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    for i in 0..review.agents.len() {
        signal_review_agent_cancel(&ctx.review_agent_cancels, &review_id, i);
    }

    // 2. Kill the live agent sessions — PtyHandle has no Drop-kill, so this
    //    explicit reap is what actually stops the in-flight agents.
    for agent in &review.agents {
        if let Some(sid) = &agent.session_id {
            let _ = ctx.manager.kill_session(sid).await;
        }
    }

    // 3. Mark the run cancelled (the background task sees the flag and won't
    //    overwrite this with done/error).
    if let Err(e) = ctx
        .reviews_store
        .set_status(&review_id, ReviewStatus::Cancelled, None)
        .await
    {
        tracing::error!(review = %review_id, "set status cancelled: {e}");
    }

    // 4. Best-effort cleanup: temp files (diff + per-agent prompt/json) AND the
    //    durable prompt/diff rows (0100) — same lifecycle, cancelled runs are
    //    not retryable.
    remove_review_temp_files(&review_id).await;
    let _ = ctx.reviews_store.delete_run_artifacts(&review_id).await;

    // 5. Broadcast the terminal status to subscribers.
    let _ = ctx.events.send(Event::ReviewChanged {
        workspace_id: workspace_id.clone(),
        session_id: None,
        review_id: review_id.to_string(),
        status: ReviewStatus::Cancelled.as_str().to_string(),
    });
}

async fn get_review_by_id(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;
    Ok(Json(review))
}

async fn get_review(
    Path(RepoPrPath {
        id: repo_id,
        number,
    }): Path<RepoPrPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let review = ctx
        .reviews_store
        .latest_for_pr(&repo_id, number)
        .await
        .map_err(crate::error::ApiError)?
        .ok_or_else(|| crate::error::ApiError(Error::NotFound("no review for this PR".into())))?;

    Ok(Json(review))
}

/// Feedback is evidence about the user's disposition, not proof of a defect.
/// The engine checks the workspace opt-in and deduplicates comment/disposition.
fn queue_review_learning(ctx: &ServerCtx, workspace_id: &Id, comment: &ReviewComment) {
    let engine = Arc::clone(&ctx.improve_engine);
    let wid = workspace_id.clone();
    let comment = comment.clone();
    tokio::spawn(async move {
        let disposition = comment.state.as_str();
        let narrative = format!(
            "User disposition: {disposition}. Source: /api/v1/reviews/{}; comment ID: {}.\n\
             Location: {}:{}\nReview comment (untrusted observation):\n{}\n\
             This is feedback on one finding. Approval does not independently prove its technical claim; \
             decline does not prove the inverse or supply a rejection reason. Learn only a narrow, \
             evidence-supported lesson, or propose no change.",
            comment.review_id, comment.id, comment.path.as_deref().unwrap_or("general"),
            comment.line.unwrap_or(0), comment.body);
        if let Err(e) = engine
            .learn_review_feedback(&wid, &comment.id, disposition, &narrative)
            .await
        {
            tracing::warn!(comment = %comment.id, "review feedback learning failed: {e}");
        }
    });
}

async fn approve_comment(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<ReviewComment>> {
    let comment = ctx
        .reviews_store
        .get_comment(&cid)
        .await
        .map_err(crate::error::ApiError)?;
    let review = ctx
        .reviews_store
        .get_review(&comment.review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    // A DECLINED comment is not approvable: a decline that lands while a
    // "Post all" loop is still walking its snapshot must not be posted anyway.
    // Restoring it to draft (`PATCH … {restore_draft:true}`) is the way back.
    if comment.state == CommentState::Declined {
        return Err(crate::error::ApiError(Error::Conflict(
            "comment was declined — restore it to draft before approving".into(),
        )));
    }

    // Post the comment to the PR provider — at most once.
    // - A LOCAL review (pr #0 sentinel) has no PR: approving it is a local
    //   decision; calling the forge only 404'd against "PR #0".
    // - An already-posted comment is never re-posted, and the post is CLAIMED
    //   atomically (posted 0→1) before the forge call, so a double click or a
    //   retried request can't post twice; a failed call releases the claim.
    let pr_posted = if review.pr_number == LOCAL_REVIEW_PR_NUMBER {
        comment.posted
    } else if comment.posted {
        true
    } else if !ctx
        .reviews_store
        .claim_comment_post(&cid)
        .await
        .map_err(crate::error::ApiError)?
    {
        // Another request holds (or completed) the post.
        true
    } else {
        // Re-read under the claim: a decline that raced the check above wins.
        let fresh = ctx.reviews_store.get_comment(&cid).await;
        if matches!(&fresh, Ok(c) if c.state == CommentState::Declined) {
            let _ = ctx.reviews_store.release_comment_post(&cid).await;
            return Err(crate::error::ApiError(Error::Conflict(
                "comment was declined — restore it to draft before approving".into(),
            )));
        }
        let posted = post_review_comment(&ctx, &user, &repo, review.pr_number, &comment).await;
        if !posted {
            let _ = ctx.reviews_store.release_comment_post(&cid).await;
        }
        posted
    };

    // Append to the review markdown file.
    if let Err(e) = append_to_review_file(&repo.path, review.pr_number, &comment).await {
        tracing::warn!(comment = %cid, "failed to append to review file: {e}");
    }

    let updated = ctx
        .reviews_store
        .set_comment_state(&cid, CommentState::Approved, pr_posted)
        .await
        .map_err(crate::error::ApiError)?;
    queue_review_learning(&ctx, &repo.workspace_id, &updated);
    Ok(Json(updated))
}

/// Post one approved review comment to its PR. Inline first; when the forge
/// rejects the ANCHOR (GitHub 422 "line is not part of the diff" → `Conflict`,
/// GitLab/Bitbucket 400 on an invalid position) it falls back to a general
/// comment that cites `path:line`, instead of silently leaving the approved
/// finding unposted. Any other failure (auth, network, 5xx) is NOT retried —
/// a 5xx may have created the comment, and a second call would duplicate it.
async fn post_review_comment(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    repo: &otto_core::domain::Repo,
    pr_number: u64,
    comment: &ReviewComment,
) -> bool {
    let (provider, remote) = match resolve_provider_remote(ctx, user, repo).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(comment = %comment.id, "failed to resolve provider for approve: {e}");
            return false;
        }
    };
    let req = NewPrCommentReq {
        body: comment.body.clone(),
        path: comment.path.clone(),
        line: comment.line,
        in_reply_to: None,
        side: None,
        old_line: None,
        commit_id: None,
    };
    let err = match provider.comment(&remote, pr_number, &req).await {
        Ok(_) => return true,
        Err(e) => e,
    };
    let inline = comment.path.is_some() && comment.line.is_some();
    if !(inline && inline_anchor_rejected(&err)) {
        tracing::warn!(comment = %comment.id, "failed to post comment to PR: {err}");
        return false;
    }
    tracing::info!(comment = %comment.id, "inline anchor rejected ({err}); posting as a general comment");
    let general = NewPrCommentReq {
        body: general_comment_body(comment),
        path: None,
        line: None,
        in_reply_to: None,
        side: None,
        old_line: None,
        commit_id: None,
    };
    match provider.comment(&remote, pr_number, &general).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(comment = %comment.id, "failed to post fallback comment to PR: {e}");
            false
        }
    }
}

async fn decline_comment(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<ReviewComment>> {
    let comment = ctx
        .reviews_store
        .get_comment(&cid)
        .await
        .map_err(crate::error::ApiError)?;
    let review = ctx
        .reviews_store
        .get_review(&comment.review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    // Keep `posted` as-is: declining a comment that is already on the PR does
    // not un-post it, and resetting the flag let a later approve post it AGAIN.
    let updated = ctx
        .reviews_store
        .set_comment_state(&cid, CommentState::Declined, comment.posted)
        .await
        .map_err(crate::error::ApiError)?;
    queue_review_learning(&ctx, &repo.workspace_id, &updated);
    Ok(Json(updated))
}

/// `PATCH /pr-review-comments/{cid}` body.
#[derive(Deserialize)]
struct EditReviewCommentReq {
    /// New body for a DRAFT comment (a person's edit of the agent's draft).
    #[serde(default)]
    body: Option<String>,
    /// Move a DECLINED comment back to draft.
    #[serde(default)]
    restore_draft: bool,
}

/// `PATCH /pr-review-comments/{cid}` — a person edits an agent-drafted
/// comment before approving it, or restores a declined one to draft. Never
/// touches a posted comment (409); nothing is sent anywhere.
async fn edit_review_comment(
    Path(cid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<EditReviewCommentReq>,
) -> crate::error::ApiResult<Json<ReviewComment>> {
    let comment = ctx
        .reviews_store
        .get_comment(&cid)
        .await
        .map_err(crate::error::ApiError)?;
    let review = ctx
        .reviews_store
        .get_review(&comment.review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    let body = req.body.as_deref().map(str::trim);
    if body.is_some_and(str::is_empty) {
        return Err(crate::error::ApiError(Error::Invalid(
            "comment body cannot be empty".into(),
        )));
    }
    if body.is_none() && !req.restore_draft {
        return Err(crate::error::ApiError(Error::Invalid(
            "nothing to change — send `body` and/or `restore_draft`".into(),
        )));
    }
    ctx.reviews_store
        .edit_unposted_comment(&cid, body, req.restore_draft)
        .await
        .map_err(crate::error::ApiError)?
        .map(Json)
        .ok_or_else(|| {
            crate::error::ApiError(Error::Conflict(
                "only an unposted draft can be edited, and only a declined comment restored".into(),
            ))
        })
}

// ---------------------------------------------------------------------------
// Local review routes
// ---------------------------------------------------------------------------

/// Sentinel pr_number for local reviews (real PRs are ≥ 1).
const LOCAL_REVIEW_PR_NUMBER: u64 = 0;

async fn start_local_review(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<LocalReviewReq>,
) -> crate::error::ApiResult<Json<Review>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;
    review_budget_gate(&ctx, &repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    let git = otto_git::LocalGit::new(&repo.path);
    // Resolve the user-picked base first: a ref that doesn't exist locally
    // (or an empty one) falls back to `origin/<base>` / the default branch,
    // and a truly unresolvable base reports the candidates tried instead of a
    // raw "git exited 128".
    let want = Some(body.base.as_str()).filter(|s| !s.trim().is_empty());
    let resolved = git
        .resolve_base(want)
        .await
        .map_err(crate::error::ApiError)?;
    let diff_text = match git.review_diff_text(&resolved.diff_ref).await {
        Ok(d) => d,
        Err(e) => {
            return Err(crate::error::ApiError(Error::Invalid(format!(
                "git diff failed: {e}"
            ))));
        }
    };

    // Create the review row.
    let review = ctx
        .reviews_store
        .create_review(&repo_id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .map_err(crate::error::ApiError)?;
    let review_id = review.id.clone();

    if diff_text.trim().is_empty() {
        // No changes vs base — complete immediately with a note.
        let note = format!("No changes vs {}", body.base);
        tracing::info!(review = %review_id, "{note}");
        let ctx_note = ctx.clone();
        let rid = review_id.clone();
        let ws_id_local = repo.workspace_id.clone();
        // Notify that the review started (running) and will complete immediately.
        let _ = ctx.events.send(Event::ReviewChanged {
            workspace_id: ws_id_local.clone(),
            session_id: None,
            review_id: rid.clone(),
            status: ReviewStatus::Running.as_str().to_string(),
        });
        tokio::spawn(async move {
            // Seed an empty agent list and mark done immediately.
            let _ = ctx_note
                .reviews_store
                .set_status(&rid, ReviewStatus::Done, None)
                .await;
            let _ = ctx_note.events.send(Event::ReviewChanged {
                workspace_id: ws_id_local,
                session_id: None,
                review_id: rid.clone(),
                status: ReviewStatus::Done.as_str().to_string(),
            });
        });
    } else {
        // Spawn the review core in the background.
        let workspace = ctx
            .workspaces
            .get(&repo.workspace_id)
            .await
            .map_err(crate::error::ApiError)?;
        // Notify that the review runner is starting; run_review emits done/error.
        let _ = ctx.events.send(Event::ReviewChanged {
            workspace_id: workspace.id.clone(),
            session_id: None,
            review_id: review_id.clone(),
            status: ReviewStatus::Running.as_str().to_string(),
        });
        let ctx_bg = ctx.clone();
        let repo_path = repo.path.clone();
        let repo_id_bg = repo.id.clone();
        // Name the working-tree's current branch (source) and the diff base
        // (dest) for the reviewers. cwd stays the working tree — that IS the code
        // under review for a local review (uncommitted changes vs base).
        let local_branches = git
            .current_branch()
            .await
            .ok()
            .filter(|c| !c.is_empty())
            .map(|source| ReviewBranches {
                source,
                dest: body.base.clone(),
                checked_out: true,
            });
        tokio::spawn(async move {
            // Local/working-tree review: no PR, so pr_number = 0 (the fingerprint
            // accommodates pr 0).
            run_review(
                ctx_bg,
                review_id,
                repo_path,
                diff_text,
                None,
                None,
                workspace,
                repo_id_bg,
                0,
                local_branches,
                None,
                None,
            )
            .await;
        });
    }

    Ok(Json(review))
}

async fn get_local_review(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Review>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let review = ctx
        .reviews_store
        .latest_for_pr(&repo_id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .map_err(crate::error::ApiError)?
        .ok_or_else(|| {
            crate::error::ApiError(Error::NotFound("no local review for this repo".into()))
        })?;

    Ok(Json(review))
}

/// GET /repos/{id}/prs/{number}/reviews — all runs for a PR, newest first.
async fn list_reviews(
    Path(RepoPrPath {
        id: repo_id,
        number,
    }): Path<RepoPrPath>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Vec<Review>>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let reviews = ctx
        .reviews_store
        .list_for_pr(&repo_id, number as i64)
        .await
        .map_err(crate::error::ApiError)?;

    Ok(Json(reviews))
}

/// GET /repos/{id}/local-reviews — all local review runs, newest first.
async fn list_local_reviews(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<Vec<Review>>> {
    let repo = ctx
        .git_store
        .get_repo(&repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Viewer).await?;

    let reviews = ctx
        .reviews_store
        .list_for_pr(&repo_id, LOCAL_REVIEW_PR_NUMBER as i64)
        .await
        .map_err(crate::error::ApiError)?;

    Ok(Json(reviews))
}

// ---------------------------------------------------------------------------
// Handoff route
// ---------------------------------------------------------------------------

async fn handoff_review(
    Path(review_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(body): Json<HandoffReq>,
) -> crate::error::ApiResult<Json<Session>> {
    // Load review and its repo.
    let review = ctx
        .reviews_store
        .get_review(&review_id)
        .await
        .map_err(crate::error::ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &repo.workspace_id, WorkspaceRole::Editor).await?;

    let workspace = ctx
        .workspaces
        .get(&repo.workspace_id)
        .await
        .map_err(crate::error::ApiError)?;

    // Filter comments based on the optional comment_ids list.
    let comments_to_send: Vec<&otto_core::domain::ReviewComment> =
        if let Some(ref ids) = body.comment_ids {
            let id_set: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
            review
                .comments
                .iter()
                .filter(|c| id_set.contains(c.id.as_str()))
                .collect()
        } else {
            // All non-declined comments.
            review
                .comments
                .iter()
                .filter(|c| c.state != otto_core::domain::CommentState::Declined)
                .collect()
        };

    if comments_to_send.is_empty() {
        return Err(crate::error::ApiError(Error::Invalid(
            "no findings to hand off (all declined or list is empty)".into(),
        )));
    }

    // Build the handoff prompt.
    let mut prompt = String::from(
        "A code review of the changes in this repository found the following issues. \
         Please review each and fix the ones that are valid, then summarize what you changed:\n\n",
    );
    for c in &comments_to_send {
        let loc = match (&c.path, c.line) {
            (Some(p), Some(l)) => format!("{}:{}", p, l),
            (Some(p), None) => p.clone(),
            _ => "(general)".to_string(),
        };
        prompt.push_str(&format!("- {} [{}] {}\n", loc, c.severity.as_str(), c.body));
    }

    // Spawn the agent session.
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(body.provider.clone()),
        title: Some("Fix review findings".to_string()),
        cwd: Some(repo.path.clone()),
        connection_id: None,
        model: None,
        meta: None,
    };
    let session = ctx
        .manager
        .create(&workspace, &user.id, req, None)
        .await
        .map_err(crate::error::ApiError)?;

    // Write the prompt into the session after a short delay (mirrors PtySpawner).
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        prompt,
        Duration::from_millis(1500),
        token,
    );

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// PR Review config routes
// ---------------------------------------------------------------------------

async fn get_review_config(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> crate::error::ApiResult<Json<ReviewConfig>> {
    Ok(Json(load_review_config(&ctx).await))
}

async fn put_review_config(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ReviewConfig>,
) -> crate::error::ApiResult<Json<ReviewConfig>> {
    crate::auth::require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body).map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!("serialize: {e}")))
    })?;
    repo.put("pr_review", &value)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(body))
}

async fn get_review_presets(
    State(ctx): State<ServerCtx>,
    CurrentUser(_user): CurrentUser,
) -> crate::error::ApiResult<Json<Vec<ReviewConfigPreset>>> {
    Ok(Json(load_review_presets(&ctx).await))
}

async fn put_review_presets(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<Vec<ReviewConfigPreset>>,
) -> crate::error::ApiResult<Json<Vec<ReviewConfigPreset>>> {
    crate::auth::require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    // Reject blank/duplicate ids up front — a dangling or ambiguous id would
    // silently fall repos back to the global config.
    let mut seen = std::collections::HashSet::new();
    for p in &body {
        if p.id.trim().is_empty() {
            return Err(crate::error::ApiError(Error::Invalid(
                "preset id must not be empty".to_string(),
            )));
        }
        if !seen.insert(p.id.as_str()) {
            return Err(crate::error::ApiError(Error::Invalid(format!(
                "duplicate preset id: {}",
                p.id
            ))));
        }
    }
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body).map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!("serialize: {e}")))
    })?;
    repo.put("pr_review_presets", &value)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(body))
}

/// Resolve a repo id to its workspace and check the caller's role there.
async fn require_repo_role(
    ctx: &ServerCtx,
    user: &User,
    repo_id: &Id,
    role: WorkspaceRole,
) -> crate::error::ApiResult<()> {
    let repo = ctx
        .git_store
        .get_repo(repo_id)
        .await
        .map_err(crate::error::ApiError)?;
    crate::auth::require_ws_role(ctx, user, &repo.workspace_id, role).await?;
    Ok(())
}

/// Build the `GET /repos/{id}/review-config` response: the stored binding
/// (if any) resolved to its scope + effective config.
async fn repo_review_config_resp(ctx: &ServerCtx, repo_id: &Id) -> RepoReviewConfigResp {
    if let Some(binding) = load_repo_review_binding(ctx, repo_id).await {
        if let Some(cfg) = binding.config {
            return RepoReviewConfigResp {
                scope: "custom".to_string(),
                preset_id: None,
                preset_name: None,
                config: cfg,
            };
        }
        if let Some(pid) = binding.preset_id {
            let preset = load_review_presets(ctx)
                .await
                .into_iter()
                .find(|p| p.id == pid);
            return match preset {
                Some(p) => RepoReviewConfigResp {
                    scope: "preset".to_string(),
                    preset_id: Some(p.id),
                    preset_name: Some(p.name),
                    config: p.config,
                },
                // Dangling reference: report it (preset_id set, name None) but
                // surface the config reviews will ACTUALLY run with — global.
                None => RepoReviewConfigResp {
                    scope: "preset".to_string(),
                    preset_id: Some(pid),
                    preset_name: None,
                    config: load_review_config(ctx).await,
                },
            };
        }
    }
    RepoReviewConfigResp {
        scope: "global".to_string(),
        preset_id: None,
        preset_name: None,
        config: load_review_config(ctx).await,
    }
}

async fn get_repo_review_config(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<RepoReviewConfigResp>> {
    require_repo_role(&ctx, &user, &repo_id, WorkspaceRole::Viewer).await?;
    Ok(Json(repo_review_config_resp(&ctx, &repo_id).await))
}

async fn put_repo_review_config(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<RepoReviewBinding>,
) -> crate::error::ApiResult<Json<RepoReviewConfigResp>> {
    require_repo_role(&ctx, &user, &repo_id, WorkspaceRole::Editor).await?;
    if body.config.is_none() && body.preset_id.is_none() {
        return Err(crate::error::ApiError(Error::Invalid(
            "binding needs a preset_id or an inline config (DELETE to revert to global)"
                .to_string(),
        )));
    }
    if let Some(pid) = &body.preset_id {
        if body.config.is_none() && !load_review_presets(&ctx).await.iter().any(|p| &p.id == pid) {
            return Err(crate::error::ApiError(Error::Invalid(format!(
                "unknown preset id: {pid}"
            ))));
        }
    }
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    let value = serde_json::to_value(&body).map_err(|e| {
        crate::error::ApiError(otto_core::Error::Internal(format!("serialize: {e}")))
    })?;
    settings
        .put(&repo_review_binding_key(&repo_id), &value)
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(repo_review_config_resp(&ctx, &repo_id).await))
}

async fn delete_repo_review_config(
    Path(repo_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> crate::error::ApiResult<Json<RepoReviewConfigResp>> {
    require_repo_role(&ctx, &user, &repo_id, WorkspaceRole::Editor).await?;
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    settings
        .delete(&repo_review_binding_key(&repo_id))
        .await
        .map_err(crate::error::ApiError)?;
    Ok(Json(repo_review_config_resp(&ctx, &repo_id).await))
}

/// Routes for PR review agent configuration (global config, named presets,
/// and the per-repo binding).
pub fn review_config_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/settings/pr-review",
            get(get_review_config).put(put_review_config),
        )
        .route(
            "/settings/pr-review/presets",
            get(get_review_presets).put(put_review_presets),
        )
        .route(
            "/repos/{id}/review-config",
            get(get_repo_review_config)
                .put(put_repo_review_config)
                .delete(delete_repo_review_config),
        )
}

// ---------------------------------------------------------------------------
// Provider update route
// ---------------------------------------------------------------------------

/// Routes: POST /workspaces/{id}/providers/update
pub fn provider_routes() -> Router<ServerCtx> {
    Router::new().route("/workspaces/{id}/providers/update", post(update_providers))
}

async fn update_providers(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(req): Json<UpdateProvidersReq>,
) -> ApiResult<Json<Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;

    // Collect (name, command) pairs — either the single requested provider or all.
    let all_cmds = ctx.manager.provider_update_commands();
    let pairs: Vec<(String, String)> = if let Some(ref name) = req.provider {
        let found = all_cmds.into_iter().find(|(n, _)| n == name);
        match found {
            Some(pair) => vec![pair],
            None => {
                return Err(ApiError(Error::Invalid(format!(
                    "provider '{name}' has no update command"
                ))));
            }
        }
    } else {
        all_cmds
    };

    if pairs.is_empty() {
        return Err(ApiError(Error::Invalid(
            "no providers have an update command configured".into(),
        )));
    }

    // Build the compound shell command: join with "; echo; " so each step's
    // output is separated by a blank line.
    let compound = pairs
        .iter()
        .map(|(_, cmd)| cmd.as_str())
        .collect::<Vec<_>>()
        .join("; echo; ");

    let session_req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("shell".to_string()),
        title: Some("Update CLIs".to_string()),
        cwd: None,
        connection_id: None,
        model: None,
        meta: None,
    };

    let session = ctx
        .manager
        .create(&ws, &user.id, session_req, None)
        .await
        .map_err(ApiError)?;

    // Write the compound command into the PTY shortly after spawn, mirroring
    // the PtySpawner pattern used for connection first_command.
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        compound,
        Duration::from_millis(800),
        token,
    );

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// Session input route  (POST /sessions/{id}/input)
// ---------------------------------------------------------------------------

/// Routes: POST /sessions/{id}/input, POST /sessions/{id}/message,
/// GET /sessions/{id}/wait
pub fn session_input_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/sessions/{id}/input", post(send_input))
        .route("/sessions/{id}/message", post(session_message))
        .route("/sessions/{id}/wait", get(wait_session))
}

/// Longest a `/sessions/{id}/wait` call may block. Kept under the 30 s the
/// MCP self-call client allows so a governed `otto.wait_session` never times
/// out on the transport; callers loop for longer waits.
const MAX_WAIT_SESSION_SECS: u64 = 25;

/// `POST /workspaces/{id}/sessions/open` — open an agent session for a
/// delegating lead and queue its opening prompt. The prompt goes through
/// [`crate::review_session::submit_prompt`] (wait for the TUI, paste, verify
/// the echo, Enter) on a background task, so the caller gets the session row
/// back immediately and polls `/sessions/{id}/wait` for the outcome. Editor.
async fn open_agent_session(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::OpenAgentSessionReq>,
) -> ApiResult<Json<otto_core::api::OpenAgentSessionResp>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let provider = req.provider.trim().to_string();
    if provider.is_empty() {
        return Err(ApiError(Error::Invalid("provider is required".into())));
    }
    // Stamp the work origin so usage attribution can tell a delegated worker
    // from a hand-opened tab; a caller-supplied `work` ref wins.
    let mut meta = req.meta.unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        return Err(ApiError(Error::Invalid("meta must be an object".into())));
    }
    if meta.get("work").is_none() {
        meta["work"] = serde_json::json!({ "origin": "delegation" });
    }
    // The delegating lead, stamped by the server from the credential (never
    // the body): `/sessions/{id}/message` lets an agent's own token reach
    // only itself and the workers it opened here (S11-02).
    match crate::feature_guard::agent_session_of(&auth) {
        Some(lead) => meta[DELEGATED_BY_META] = serde_json::json!(lead),
        None => {
            if let Some(m) = meta.as_object_mut() {
                m.remove(DELEGATED_BY_META);
            }
        }
    }
    let create = otto_core::api::CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider),
        title: req.title.filter(|t| !t.trim().is_empty()),
        cwd: req.cwd.filter(|c| !c.trim().is_empty()),
        connection_id: None,
        meta: Some(meta),
        model: req.model,
    };
    let ws = WorkspacesRepo::new(ctx.pool.clone())
        .get(&ws_id)
        .await
        .map_err(ApiError)?;
    let session = ctx
        .manager
        .create(&ws, &user.id, create, None)
        .await
        .map_err(ApiError)?;

    let prompt = req
        .prompt
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let prompt_dispatch = match prompt {
        Some(prompt) => {
            let manager = ctx.manager.clone();
            let sid = session.id.clone();
            tokio::spawn(async move {
                if crate::review_session::submit_prompt(&manager, &sid, &prompt).await {
                    manager.record_user_message(&sid, &prompt).await;
                } else {
                    tracing::warn!(session = %sid, "delegation: session ended before its opening prompt was sent");
                }
            });
            "queued"
        }
        None => "none",
    };
    Ok(Json(otto_core::api::OpenAgentSessionResp {
        session,
        prompt_dispatch: prompt_dispatch.into(),
    }))
}

/// Session meta key naming the agent session that opened this one through
/// `POST /workspaces/{id}/sessions/open` (server-owned: PATCH cannot set it).
pub const DELEGATED_BY_META: &str = otto_sessions::http::DELEGATED_BY_META;

/// Confinement of an agent session's OWN credential on the REST twins of the
/// terminal (S11-02 / S1-11) and on broadcast/relay (S11-305 / S1-303). The
/// one rule lives with the sessions router, which applies it to the lifecycle
/// routes (PATCH, restart, kill, archive, …) too.
pub use otto_sessions::http::agent_input_rule;

/// `POST /sessions/{id}/message` — deliver ONE message to ONE live agent
/// session as if typed + Enter: the targeted counterpart of
/// `/workspaces/{id}/broadcast`, for a lead driving a single worker. Same
/// authorization as `/input` (Editor + owner-or-admin, resource re-check).
async fn session_message(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::SessionMessageReq>,
) -> ApiResult<Json<otto_core::api::SessionMessageResp>> {
    let text = req.text.trim();
    if text.is_empty() {
        return Err(ApiError(Error::Invalid("message text is empty".into())));
    }
    let session = input_session(&ctx, &user.id, &session_id)
        .await
        .map_err(ApiError)?;
    agent_input_rule(
        crate::feature_guard::agent_session_of(&auth),
        &session.id,
        &session.meta,
        true,
    )
    .map_err(ApiError)?;
    if session.kind != SessionKind::Agent {
        return Err(ApiError(Error::Invalid(
            "messages can only be sent to agent sessions".into(),
        )));
    }
    if !matches!(
        session.status,
        otto_core::domain::SessionStatus::Running
            | otto_core::domain::SessionStatus::Working
            | otto_core::domain::SessionStatus::Idle
    ) {
        return Err(ApiError(Error::Conflict(format!(
            "session is {}, not live",
            session.status.as_str()
        ))));
    }
    submit_session_text(&ctx, Typist::new(&user.id, &auth), &session_id, text)
        .await
        .map_err(ApiError)?;
    Ok(Json(otto_core::api::SessionMessageResp {
        session_id,
        delivered: true,
    }))
}

/// Query of `GET /sessions/{id}/wait`.
#[derive(Deserialize)]
struct WaitSessionQuery {
    /// Comma-separated statuses to wait for (default `idle,exited`).
    status: Option<String>,
    /// Seconds to block at most (default 20, capped at [`MAX_WAIT_SESSION_SECS`]).
    timeout_secs: Option<u64>,
}

/// `GET /sessions/{id}/wait` — block until the session's status is one of the
/// awaited set or the deadline passes, then return the session as observed.
/// `idle` is "the agent's turn ended" for a delegating lead. Owner-or-admin.
async fn wait_session(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<WaitSessionQuery>,
) -> ApiResult<Json<otto_core::api::WaitSessionResp>> {
    let mut session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;
    let wanted: Vec<String> = q
        .status
        .as_deref()
        .unwrap_or("idle,exited")
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if wanted.is_empty() {
        return Err(ApiError(Error::Invalid("status list is empty".into())));
    }
    let timeout = Duration::from_secs(q.timeout_secs.unwrap_or(20).min(MAX_WAIT_SESSION_SECS));
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if wanted.iter().any(|w| w == session.status.as_str()) {
            return Ok(Json(otto_core::api::WaitSessionResp {
                session,
                reached: true,
            }));
        }
        // A daemon shutdown answers the long-poll now (callers loop on
        // `reached: false`) instead of holding the HTTP drain open past
        // launchd's exit timeout.
        if tokio::time::Instant::now() >= deadline || crate::shutdown::is_shutting_down() {
            return Ok(Json(otto_core::api::WaitSessionResp {
                session,
                reached: false,
            }));
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            _ = crate::shutdown::cancelled() => {}
        }
        session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    }
}

async fn send_input(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<otto_core::api::SendInputReq>,
) -> ApiResult<axum::http::StatusCode> {
    // Resolve the session and check that the caller has Editor access to the
    // workspace that owns it — then owner-or-admin: typing into another user's
    // live agent is the same capability as attaching to its terminal (the WS
    // gate), so it gets the same confinement.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Editor).await?;
    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;
    agent_input_rule(
        crate::feature_guard::agent_session_of(&auth),
        &session.id,
        &session.meta,
        false,
    )
    .map_err(ApiError)?;

    // `submit` omitted/true: paste + a real Enter via `submit_text` — writing
    // `"{text}\n"` in one burst makes bracketed-paste TUIs (Claude Code, Codex)
    // treat the newline as pasted content, so the message lands but is never
    // sent (docs/design/conversation-view.md §1). `submit: false`: the text
    // verbatim, for the user to inspect/edit before pressing Enter.
    let bracketed_tui = session.kind == otto_core::domain::SessionKind::Agent
        && matches!(session.provider.as_str(), "claude" | "codex");
    if req.submit.unwrap_or(true) && bracketed_tui {
        ctx.manager
            .human_submit_text(&session_id, &user.id, auth.scope.is_some(), &req.text)
            .await
            .map_err(ApiError)?;
    } else if req.submit.unwrap_or(true) {
        // Shells / connections / bridges: a plain line + newline runs it.
        ctx.manager
            .human_input(
                &session_id,
                &user.id,
                auth.scope.is_some(),
                true,
                format!("{}\n", req.text).as_bytes(),
            )
            .await
            .map_err(ApiError)?;
    } else {
        ctx.manager
            .human_input(
                &session_id,
                &user.id,
                auth.scope.is_some(),
                true,
                req.text.as_bytes(),
            )
            .await
            .map_err(ApiError)?;
    }

    Ok(axum::http::StatusCode::OK)
}

// ---------------------------------------------------------------------------
// Browser proxy route  (GET /browser/proxy?url=…&ticket=…)
// ---------------------------------------------------------------------------
//
// The take-over iframe can't send an `Authorization` header, so the proxy used
// to accept the owner's bearer as `?token=` — which leaked it into the iframe
// URL (history, Referer, the proxied page's `location.href`) and ran arbitrary
// third-party HTML on the daemon origin, where it could read that token back.
// Now the UI mints a single-use, short-lived ticket bound to ONE url via an
// authed POST (`/api/v1/browser/proxy-ticket`), and every proxy response is
// served with `Content-Security-Policy: sandbox allow-scripts` (no
// `allow-same-origin`): the proxied page runs in an opaque origin and can
// neither read the daemon's storage nor make credentialed same-origin calls.

/// Picker script injected before </body>.
const PICKER_SCRIPT: &str = r#"<script>(function(){var on=false;function sel(el){if(!el||el===document.body||el.nodeType!==1)return 'body';var parts=[],e=el,d=0;while(e&&e.nodeType===1&&e!==document.body&&d<5){var p=e.tagName.toLowerCase();if(e.id){parts.unshift('#'+e.id);break;}var cls=[].slice.call(e.classList||[]).filter(function(c){return !/[0-9]/.test(c)&&c.length<24;}).slice(0,2);if(cls.length)p+='.'+cls.join('.');parts.unshift(p);e=e.parentElement;d++;}return parts.join(' > ');}function desc(el){var s=sel(el);var a=['aria-label','placeholder','name','href'].map(function(k){var v=el.getAttribute&&el.getAttribute(k);return v?'['+k+'="'+String(v).slice(0,60)+'"]':'';}).join('');var t=((el.textContent||'').trim()).slice(0,50);return s+a+(t?' "'+t+'"':'');}window.addEventListener('message',function(ev){if(ev.data&&ev.data.type==='otto-takeover'){on=!!ev.data.enabled;document.documentElement.style.cursor=on?'crosshair':'';}});document.addEventListener('click',function(e){if(!on)return;e.preventDefault();e.stopPropagation();try{parent.postMessage({type:'otto-element',desc:desc(e.target),x:Math.round(e.clientX),y:Math.round(e.clientY),url:location.href},'*');}catch(_){}},true);})();</script>"#;

/// CSP sent on EVERY proxy response (HTML, pass-through bytes, errors): the
/// document is sandboxed into an opaque origin — scripts run (the picker needs
/// them) but the page is never same-origin with the daemon.
const PROXY_CSP: &str = "sandbox allow-scripts";

/// How long a minted proxy ticket stays redeemable.
const PROXY_TICKET_TTL: std::time::Duration = std::time::Duration::from_secs(60);
/// Cap on outstanding tickets (each mint prunes expired ones first; this only
/// bounds a caller hammering the mint route).
const PROXY_TICKET_MAX: usize = 1024;

struct ProxyTicket {
    url: String,
    expires: std::time::Instant,
}

/// In-memory single-use ticket store. Process-local on purpose: a ticket only
/// needs to survive the ~instant between the UI minting it and the iframe
/// loading it, and a daemon restart invalidating every ticket is harmless.
static PROXY_TICKETS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, ProxyTicket>>,
> = std::sync::LazyLock::new(Default::default);

/// Mint a ticket for `url`, valid for `ttl`.
fn mint_proxy_ticket(url: &str, ttl: std::time::Duration) -> String {
    use rand::rand_core::UnwrapErr;
    use rand::rngs::SysRng;
    use rand::Rng;
    let mut bytes = [0u8; 32];
    UnwrapErr(SysRng).fill_bytes(&mut bytes);
    let ticket = hex::encode(bytes);
    let now = std::time::Instant::now();
    let mut map = PROXY_TICKETS.lock().unwrap_or_else(|p| p.into_inner());
    map.retain(|_, t| t.expires > now);
    if map.len() >= PROXY_TICKET_MAX {
        // Drop the soonest-to-expire ticket rather than refusing the mint.
        if let Some(k) = map
            .iter()
            .min_by_key(|(_, t)| t.expires)
            .map(|(k, _)| k.clone())
        {
            map.remove(&k);
        }
    }
    map.insert(
        ticket.clone(),
        ProxyTicket {
            url: url.to_owned(),
            expires: now + ttl,
        },
    );
    ticket
}

/// Redeem (and consume) a ticket for `url`. Single-use: the ticket is removed
/// whether or not it matches, so a leaked/guessed ticket can't be retried
/// against a different URL.
fn redeem_proxy_ticket(ticket: &str, url: &str) -> bool {
    let mut map = PROXY_TICKETS.lock().unwrap_or_else(|p| p.into_inner());
    match map.remove(ticket) {
        Some(t) => t.expires > std::time::Instant::now() && t.url == url,
        None => false,
    }
}

#[derive(Deserialize)]
struct ProxyTicketReq {
    url: String,
}

#[derive(serde::Serialize)]
struct ProxyTicketResp {
    ticket: String,
    expires_in_secs: u64,
}

/// `POST /browser/proxy-ticket` — authed (bearer + `Browser`/Edit policy);
/// returns a single-use ticket the take-over iframe passes as `?ticket=`.
/// Share-link and MCP-restricted tokens never get one
/// (`RootRoute::BrowserProxy`: the proxy itself only sees the ticket).
async fn browser_proxy_ticket(
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<ProxyTicketReq>,
) -> ApiResult<Json<ProxyTicketResp>> {
    crate::feature_guard::root_route_gate(
        crate::feature_guard::RootRoute::BrowserProxy,
        &auth,
        None,
    )
    .await
    .map_err(ApiError)?;
    let url = req.url.trim();
    let ok_scheme = reqwest::Url::parse(url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false);
    if !ok_scheme {
        return Err(ApiError(Error::Invalid(
            "url must be an absolute http(s) URL".into(),
        )));
    }
    Ok(Json(ProxyTicketResp {
        ticket: mint_proxy_ticket(url, PROXY_TICKET_TTL),
        expires_in_secs: PROXY_TICKET_TTL.as_secs(),
    }))
}

/// Authed API route that mints proxy tickets (mounted under `/api/v1`).
pub fn browser_proxy_ticket_routes() -> Router<ServerCtx> {
    Router::new().route("/browser/proxy-ticket", post(browser_proxy_ticket))
}

#[derive(serde::Deserialize)]
struct BrowserProxyQuery {
    url: Option<String>,
    ticket: Option<String>,
}

/// State carried by the root-level browser proxy router.
#[derive(Clone)]
struct BrowserProxyState {
    http: reqwest::Client,
}

/// Stamp the isolation headers onto every proxy response, whatever branch
/// produced it (an error page or a pass-through image is still attacker-
/// controlled content served from the daemon origin).
async fn browser_proxy_headers(mut resp: axum::response::Response) -> axum::response::Response {
    use axum::http::{header, HeaderValue};
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(PROXY_CSP),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// Root-level browser proxy router (self-authenticates via a single-use
/// `?ticket=` minted by `POST /api/v1/browser/proxy-ticket`).
pub fn browser_proxy_router() -> Router {
    // SSRF guard: the guarded resolver vets the address actually dialled (no
    // DNS rebinding after the pre-flight check) and each redirect hop is capped
    // + re-validated so an upstream 30x can't bounce the proxy inward.
    let http = crate::routes::api_client::net_guard::guarded_client_builder()
        .user_agent("Mozilla/5.0 (compatible; OttoProxy/1.0)")
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("failed to build reqwest client for browser proxy");

    Router::new()
        .route("/browser/proxy", axum::routing::get(browser_proxy))
        .layer(axum::middleware::map_response(browser_proxy_headers))
        .with_state(BrowserProxyState { http })
}

/// Escape text for an HTML attribute value / text node.
fn proxy_html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn proxy_error_page(status: axum::http::StatusCode, msg: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = format!(
        r#"<!doctype html><html><body><h2>Proxy error</h2><pre>{}</pre></body></html>"#,
        proxy_html_escape(msg)
    );
    (
        status,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Inject `<base href>` (HTML-escaped — the URL is caller-supplied) after
/// `<head>` and the picker script before `</body>`.
fn proxy_transform_html(html: &str, url: &str) -> String {
    let base_tag = format!(r#"<base href="{}">"#, proxy_html_escape(url));
    let html = {
        // Try to find <head> (case-insensitive). `to_ascii_lowercase` keeps
        // byte offsets aligned with `html` (full Unicode lowercasing doesn't).
        let lower = html.to_ascii_lowercase();
        if let Some(pos) = lower.find("<head>") {
            let insert_at = pos + "<head>".len();
            format!("{}{}{}", &html[..insert_at], base_tag, &html[insert_at..])
        } else if let Some(pos) = lower.find("<head ") {
            // <head …> with attributes: advance to the closing >.
            if let Some(close) = lower[pos..].find('>') {
                let insert_at = pos + close + 1;
                format!("{}{}{}", &html[..insert_at], base_tag, &html[insert_at..])
            } else {
                format!("{}{}", base_tag, html)
            }
        } else {
            format!("{}{}", base_tag, html)
        }
    };
    let lower = html.to_ascii_lowercase();
    if let Some(pos) = lower.rfind("</body>") {
        format!("{}{}{}", &html[..pos], PICKER_SCRIPT, &html[pos..])
    } else {
        format!("{}{}", html, PICKER_SCRIPT)
    }
}

async fn browser_proxy(
    Query(q): Query<BrowserProxyQuery>,
    State(st): State<BrowserProxyState>,
) -> axum::response::Response {
    use axum::http::{HeaderValue, StatusCode};
    use axum::response::IntoResponse;

    let unauthorized = |message: &str| {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(otto_core::api::Problem {
                code: "unauthorized".into(),
                message: message.into(),
            }),
        )
            .into_response()
    };

    // --- Validate target URL ---
    let url = match q.url {
        Some(u) if !u.is_empty() => u,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(otto_core::api::Problem {
                    code: "bad_request".into(),
                    message: "missing ?url= parameter".into(),
                }),
            )
                .into_response();
        }
    };

    // --- Auth: redeem the single-use ?ticket= (bound to this exact url) ---
    let ticket = match q.ticket {
        Some(t) if !t.is_empty() => t,
        _ => return unauthorized("missing ?ticket= parameter"),
    };
    if !redeem_proxy_ticket(&ticket, &url) {
        return unauthorized("invalid, expired or already-used ticket");
    }

    // --- SSRF guard: resolve + classify before fetching ---
    if let Err(msg) = crate::routes::api_client::net_guard::check_url(&url).await {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(otto_core::api::Problem {
                code: "bad_request".into(),
                message: msg,
            }),
        )
            .into_response();
    }

    // --- Fetch upstream ---
    let upstream = match st.http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return proxy_error_page(StatusCode::BAD_GATEWAY, &e.to_string()),
    };

    // Determine content-type before consuming the body.
    let content_type = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    let is_html = content_type.contains("text/html");

    if !is_html {
        // Stream bytes through with the upstream content-type (the response
        // layer still stamps the sandbox CSP + nosniff on it).
        let ct_val = HeaderValue::from_str(&content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
        let bytes = match read_capped(upstream, PROXY_BODY_CAP).await {
            Ok(b) => b,
            Err(e) => return proxy_error_page(StatusCode::BAD_GATEWAY, &e),
        };
        return ([(axum::http::header::CONTENT_TYPE, ct_val)], bytes).into_response();
    }

    // --- HTML: read, transform, return ---
    let html = match read_capped(upstream, PROXY_BODY_CAP).await {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => return proxy_error_page(StatusCode::BAD_GATEWAY, &e),
    };

    (
        [(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        proxy_transform_html(&html, &url),
    )
        .into_response()
}

/// Most bytes the take-over proxy buffers from one upstream response (S11-10).
const PROXY_BODY_CAP: usize = 20 * 1024 * 1024;

/// Read an upstream body chunk by chunk, refusing it once it passes `cap` —
/// a ticketed take-over of a multi-GB URL must never be buffered whole in
/// daemon memory (the 15 s timeout alone does not bound the size).
async fn read_capped(
    mut resp: reqwest::Response,
    cap: usize,
) -> std::result::Result<axum::body::Bytes, String> {
    if resp.content_length().is_some_and(|n| n > cap as u64) {
        return Err(format!("page too large (over {} MB)", cap / (1024 * 1024)));
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if buf.len() + chunk.len() > cap {
            return Err(format!("page too large (over {} MB)", cap / (1024 * 1024)));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf.into())
}

#[cfg(test)]
mod proxy_cap_tests {
    /// S11-10: a body over the cap is refused (with or without a declared
    /// length); one under it reads in full.
    #[tokio::test]
    async fn upstream_body_over_the_cap_is_refused() {
        use axum::routing::get;
        let app = axum::Router::new()
            .route("/big", get(|| async { vec![b'x'; 4096] }))
            .route(
                "/chunked",
                get(|| async {
                    let parts = (0..8).map(|_| Ok::<_, std::io::Error>(vec![b'y'; 1024]));
                    axum::body::Body::from_stream(futures_util::stream::iter(parts))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let get = |p: &str| reqwest::get(format!("http://{addr}{p}"));
        assert!(super::read_capped(get("/big").await.unwrap(), 1024)
            .await
            .is_err());
        assert!(super::read_capped(get("/chunked").await.unwrap(), 1024)
            .await
            .is_err());
        assert_eq!(
            super::read_capped(get("/big").await.unwrap(), 8192)
                .await
                .unwrap()
                .len(),
            4096
        );
    }
}

#[cfg(test)]
mod review_cleanup_tests {
    use super::*;

    #[test]
    fn finished_review_temp_files_are_removed_but_not_the_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let d = tmp.path();
        for f in [
            "otto-review-R1.diff",
            "otto-review-R1-0.json",
            "otto-review-R1-0.prompt",
        ] {
            std::fs::write(d.join(f), "x").unwrap();
        }
        std::fs::write(d.join("otto-review-R2.diff"), "other").unwrap();
        std::fs::create_dir(d.join("otto-review-R1-wt")).unwrap();
        remove_review_temp_files_in(d, "otto-review-R1");
        assert!(!d.join("otto-review-R1.diff").exists());
        assert!(!d.join("otto-review-R1-0.json").exists());
        assert!(!d.join("otto-review-R1-0.prompt").exists());
        assert!(
            d.join("otto-review-R2.diff").exists(),
            "another review's files stay"
        );
        assert!(
            d.join("otto-review-R1-wt").is_dir(),
            "directories are never removed"
        );
    }

    #[test]
    fn only_settled_review_agents_are_retryable() {
        for s in ["done", "error", "skipped"] {
            assert!(agent_retryable(s), "{s}");
        }
        for s in ["pending", "running", "waiting", ""] {
            assert!(!agent_retryable(s), "{s}");
        }
    }
}

#[cfg(test)]
mod browser_proxy_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use tower::ServiceExt;

    fn assert_isolation_headers(resp: &axum::response::Response) {
        let h = resp.headers();
        assert_eq!(h[header::CONTENT_SECURITY_POLICY], "sandbox allow-scripts");
        assert!(!h[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("allow-same-origin"));
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    }

    #[tokio::test]
    async fn every_proxy_response_carries_sandbox_csp() {
        let app = browser_proxy_router();
        // Missing url, missing ticket, bogus ticket, and an SSRF-refused
        // (loopback) target all still get the isolation headers.
        let t = mint_proxy_ticket("http://127.0.0.1:7700/", PROXY_TICKET_TTL);
        for uri in [
            "/browser/proxy".to_string(),
            "/browser/proxy?url=https%3A%2F%2Fexample.com%2F".to_string(),
            "/browser/proxy?url=https%3A%2F%2Fexample.com%2F&ticket=nope".to_string(),
            format!("/browser/proxy?url=http%3A%2F%2F127.0.0.1%3A7700%2F&ticket={t}"),
        ] {
            let resp = app
                .clone()
                .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert!(!resp.status().is_success(), "{uri} -> {}", resp.status());
            assert_isolation_headers(&resp);
        }
    }

    #[tokio::test]
    async fn bearer_token_param_is_no_longer_accepted() {
        let resp = browser_proxy_router()
            .oneshot(
                Request::get("/browser/proxy?url=https%3A%2F%2Fexample.com%2F&token=anything")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn ticket_is_single_use() {
        let t = mint_proxy_ticket("https://example.com/a", PROXY_TICKET_TTL);
        assert!(redeem_proxy_ticket(&t, "https://example.com/a"));
        assert!(!redeem_proxy_ticket(&t, "https://example.com/a"));
    }

    #[test]
    fn ticket_is_bound_to_its_url_and_burned_on_mismatch() {
        let t = mint_proxy_ticket("https://example.com/a", PROXY_TICKET_TTL);
        assert!(!redeem_proxy_ticket(&t, "https://evil.example/"));
        // Burned by the failed attempt — can't be retried with the right url.
        assert!(!redeem_proxy_ticket(&t, "https://example.com/a"));
    }

    #[test]
    fn expired_ticket_is_rejected() {
        let t = mint_proxy_ticket("https://example.com/a", std::time::Duration::ZERO);
        assert!(!redeem_proxy_ticket(&t, "https://example.com/a"));
    }

    #[test]
    fn base_href_is_html_escaped() {
        let url = r#"https://x.example/"><script>alert(1)</script>"#;
        let out = proxy_transform_html("<html><head></head><body></body></html>", url);
        assert!(!out.contains("<script>alert(1)"), "{out}");
        assert!(out.contains(
            r#"<base href="https://x.example/&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;">"#
        ));
        assert!(out.contains("otto-element"), "picker still injected");
    }
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// DB Explorer: "examine schema / result with an agent"
// ---------------------------------------------------------------------------

/// Body of `POST /connections/{id}/db/explain-with-agent`. The UI sends the
/// schema text or result JSON it already has plus an optional question; we spawn
/// an agent session seeded with a prompt built from them.
#[derive(Debug, Deserialize)]
struct DbExplainReq {
    /// Pre-rendered context (schema DDL, structure, or result rows) to analyze.
    content: String,
    /// Optional user question; defaults to a general "explain this" prompt.
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    title: Option<String>,
    /// Needed only for global connections (which have no workspace of their own).
    #[serde(default)]
    workspace_id: Option<Id>,
}

/// Route: spawn an agent to explain/analyze a database schema or query result,
/// plus the file-backed **DB Assistant** turns (the connection-scoped half — the
/// `q`-tool query route is a PUBLIC route in `routes/mod.rs`, assist-key authed).
pub fn db_explorer_routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/connections/{id}/db/explain-with-agent",
            post(db_explain_with_agent),
        )
        .route(
            "/connections/{id}/db/assist",
            post(crate::db_assist::assist),
        )
        .route(
            "/connections/{id}/db/assist/{aid}/summary",
            post(crate::db_assist::summary),
        )
        .route(
            "/connections/{id}/db/assist/{aid}",
            axum::routing::delete(crate::db_assist::close),
        )
}

async fn db_explain_with_agent(
    Path(conn_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(body): Json<DbExplainReq>,
) -> ApiResult<Json<Session>> {
    otto_state::GrantsRepo::new(ctx.pool.clone())
        .check_global(
            &user,
            otto_core::domain::Feature::Agents,
            otto_core::domain::Capability::Edit,
            "Database explanations require permission to run host agents",
        )
        .await?;
    let conn = ctx
        .db_explorer
        .get_connection(&conn_id)
        .await
        .map_err(ApiError)?;
    let ws_id = conn
        .workspace_id
        .clone()
        .or(body.workspace_id)
        .ok_or_else(|| ApiError(Error::Invalid("a workspace_id is required".into())))?;
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;

    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    otto_sessions::trust::ensure_trusted(&default_provider, &ws.root_path);

    let question = body
        .question
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .unwrap_or(
            "Explain this database schema/result: describe each table/field, the relationships, \
             and anything notable (indexing, normalization, possible issues). Then suggest a few \
             useful queries.",
        );
    let prompt = format!(
        "You are a database expert. A user is exploring the `{}` ({}) connection in Otto.\n\n\
         {}\n\n--- DATABASE CONTEXT ---\n{}\n",
        conn.name,
        conn.kind.as_str(),
        question,
        body.content
    );

    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(default_provider),
        title: Some(
            body.title
                .unwrap_or_else(|| format!("Explain {}", conn.name)),
        ),
        cwd: Some(ws.root_path.clone()),
        connection_id: None,
        model: None,
        meta: Some(serde_json::json!({"source":"db_assist", "connection_id":conn_id})),
    };
    let resource = otto_core::access::ResourceRef {
        kind: otto_core::access::ResourceKind::Connection,
        id: conn_id.clone(),
        child: None,
    };
    otto_rbac::ResourceAccess::new(ctx.pool.clone())
        .check(&user, &resource, "db_query")
        .await
        .map_err(ApiError)?;
    let session = ctx
        .manager
        .create(&ws, &user.id, req, None)
        .await
        .map_err(ApiError)?;

    delay_session_input(
        &ctx,
        &user,
        &session.id,
        prompt,
        Duration::from_millis(1500),
        token,
    );

    Ok(Json(session))
}

// ---------------------------------------------------------------------------
// Inject-session route
// ---------------------------------------------------------------------------

/// `POST /workspaces/{id}/product/stories/{sid}/inject-session`
///
/// Builds the inject bundle for the story and spawns an Agent session that
/// receives the bundle as its initial prompt (after a short settle delay).
async fn inject_session(
    Path((ws_id, sid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(req): Json<otto_product::types::InjectSessionReq>,
) -> ApiResult<Json<Session>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;

    // Load story and verify it belongs to the requested workspace.
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    if story.workspace_id != ws_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Resolve default provider (workspace → global → "claude"), mirroring analyze.
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let default_provider = otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ]);
    let provider = req.provider.clone().unwrap_or(default_provider);

    // Resolve cwd: req → story.cwd → temp dir.
    let cwd = req
        .cwd
        .or_else(|| story.cwd.clone())
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());

    // Build the inject bundle.
    let bundle = ctx
        .product
        .build_inject_bundle(&sid)
        .await
        .map_err(ApiError)?;

    // Spawn the agent session. A chosen model reaches the CLI only via
    // `meta.model` (→ `--model`); omit when empty so the provider default holds.
    let meta = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|m| serde_json::json!({ "model": m }));
    let create_req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider),
        title: Some(format!("Implement {}", story.source_key)),
        cwd: Some(cwd),
        connection_id: None,
        model: None,
        meta,
    };
    let session = ctx
        .manager
        .create(&ws, &user.id, create_req, None)
        .await
        .map_err(ApiError)?;

    // Write the bundle into the session after a short settle delay.
    let payload = bundle.markdown.clone();
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        payload,
        Duration::from_secs(6),
        token,
    );

    // Record an inject event.
    ctx.product_repo
        .add_event(otto_state::NewEvent {
            story_id: sid,
            section: "inject".into(),
            kind: "inject_session".into(),
            summary: format!("Injected bundle into session {}", session.id),
            actor_id: Some(user.id),
            meta_json: None,
        })
        .await
        .ok();

    Ok(Json(session))
}

/// Routes for the inject-session endpoint.
pub fn inject_session_routes() -> Router<ServerCtx> {
    Router::new().route(
        "/workspaces/{id}/product/stories/{sid}/inject-session",
        post(inject_session),
    )
}

// ---------------------------------------------------------------------------
// Attach-product-story route (injects bundle into a running session)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct AttachProductReq {
    story_id: Id,
}

/// `POST /sessions/{session_id}/attach-product`
///
/// Loads the story (verifying it belongs to the session's workspace), builds
/// the inject bundle, pushes it into the live PTY, and tags the session meta
/// with `product_story: { id, title }` so the UI can badge the attachment.
async fn attach_product_story(
    Path(session_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    axum::Extension(crate::auth::BearerToken(token)): axum::Extension<crate::auth::BearerToken>,
    Json(req): Json<AttachProductReq>,
) -> ApiResult<Json<Session>> {
    // Resolve the session and its workspace.
    let session = ctx.manager.get(&session_id).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &session.workspace_id, WorkspaceRole::Editor).await?;

    crate::auth::require_session_owner_or_admin(&ctx, &user, &session).await?;

    // Load story and verify it belongs to the same workspace.
    let story = ctx
        .product_repo
        .get_story(&req.story_id)
        .await
        .map_err(ApiError)?;
    if story.workspace_id != session.workspace_id {
        return Err(ApiError(Error::NotFound(
            "story not found in workspace".into(),
        )));
    }

    // Build the inject bundle.
    let bundle = ctx
        .product
        .build_inject_bundle(&req.story_id)
        .await
        .map_err(ApiError)?;

    // Push the bundle into the live PTY after a short settle delay.
    let payload = bundle.markdown.clone();
    delay_session_input(
        &ctx,
        &user,
        &session.id,
        payload,
        Duration::from_secs(2),
        token,
    );

    // Tag the session meta with the attached story.
    let updated = ctx
        .manager
        .update_meta(
            &session_id,
            serde_json::json!({ "product_story": { "id": story.id, "title": story.title } }),
        )
        .await
        .map_err(ApiError)?;

    Ok(Json(updated))
}

/// Routes for the attach-product-story endpoint.
pub fn attach_product_routes() -> Router<ServerCtx> {
    Router::new().route(
        "/sessions/{session_id}/attach-product",
        post(attach_product_story),
    )
}

/// All module routers: `(api_extras, root_extras)` for [`crate::build_router`].
pub fn module_routers(ctx: &ServerCtx) -> (Vec<Router<ServerCtx>>, Vec<Router>) {
    let api = vec![
        otto_sessions::api_router::<ServerCtx>(),
        otto_connections::api_router::<ServerCtx>(),
        otto_aws::api_router::<ServerCtx>(),
        otto_k8s::api_router::<ServerCtx>(),
        otto_dbviewer::api_router::<ServerCtx>(),
        otto_brokers::api_router::<ServerCtx>(),
        otto_mcp::api_router::<ServerCtx>(),
        otto_product::router::<ServerCtx>(),
        otto_canvas::router::<ServerCtx>(),
        // Design Hall — the artifact graph (`/design/*`, Feature::Design).
        otto_design::router::<ServerCtx>(),
        // The unified design-assist pipeline (agent turns, variants, learned
        // rules) — otto-design-assist's; the turn runs via `DesignAssistCtx`.
        otto_design_assist::routes::<ServerCtx>(),
        otto_memory::router::<ServerCtx>(),
        otto_vault::router::<ServerCtx>(),
        crate::vault_docs_agent::routes(),
        crate::memory_gov::memory_gov_routes(),
        otto_git::router::<ServerCtx>(),
        otto_issues::router::<ServerCtx>(),
        otto_channels::router::<ServerCtx>(),
        otto_improve::router::<ServerCtx>(),
        otto_context::router::<ServerCtx>(),
        otto_skills::http::router::<ServerCtx>(),
        otto_swarm::router::<ServerCtx>(),
        otto_swarm::runtime::engine::routes::<ServerCtx>(),
        crate::routes::goal_loops::routes(),
        crate::routes::proof::routes(),
        otto_insights::routes::<ServerCtx>(),
        orchestrator_routes(),
        db_explorer_routes(),
        pr_review_routes(),
        crate::routes::findings::routes(),
        crate::routes::repo_rules::routes(),
        crate::routes::proof_pack::routes(),
        crate::routes::scheduled_tasks::routes(),
        crate::routes::personal_agents::routes(),
        // Otto Assistant — personal threads, tasks, memory, routing (`Agents`).
        crate::routes::assistant::routes(),
        crate::routes::runs::routes(),
        review_config_routes(),
        crate::skill_eval::routes(),
        crate::eval_lab_routes::routes(),
        crate::skill_review::routes(),
        provider_routes(),
        session_input_routes(),
        inject_session_routes(),
        attach_product_routes(),
        crate::context_packet::context_packet_routes(),
        crate::lsp::api_router(),
        crate::routes::capabilities::capabilities_routes(),
        crate::routes::mission::mission_routes(),
        crate::routes::workgraph::workgraph_routes(),
        crate::routes::search::search_routes(),
        crate::routes::backup::backup_routes(),
        crate::routes::connection_export::routes(),
        crate::routes::backup_git::routes(),
        // Runtime custom plugins: management + scoped host-API + reverse-proxy to
        // sidecar processes. (Asset/iframe routes are root-mounted; see root vec.)
        crate::plugins::api_routes(),
        // Single-use tickets for the root-level `/browser/proxy` take-over frame.
        browser_proxy_ticket_routes(),
    ];
    let root = vec![
        otto_sessions::ws_router(ctx.authenticator.clone(), ctx.clone()),
        crate::lsp::ws_router(ctx.authenticator.clone(), ctx.clone()),
        crate::routes::api_stream::ws_router(ctx.clone()),
        // Remote live browser viewer socket (`/ws/browser/{tab_id}/live`).
        crate::routes::browser_live::ws_router(ctx.clone()),
        browser_proxy_router(),
        // Runtime-plugin iframe assets: /plugins/{slug}/ui/* served as public
        // static files (root-mounted, outside /api/v1; the iframe's API calls are
        // the gated part). Listed last so it doesn't shadow other root routes.
        crate::plugins::asset_router(ctx.clone()),
    ];
    (api, root)
}

#[cfg(test)]
mod terminal_input_access_tests {
    use super::*;
    use otto_core::access::{AccessActor, ResourceKind};

    #[tokio::test]
    async fn alternate_input_checks_current_resource_page_owner_and_workspace() {
        let pool = otto_state::DbPool::from(
            sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap(),
        );
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let users = otto_state::UsersRepo::new(pool.clone());
        let owner = users.create("owner", "hash", "Owner", false).await.unwrap();
        let outsider = users
            .create("outsider", "hash", "Outsider", false)
            .await
            .unwrap();
        let ws = WorkspacesRepo::new(pool.clone())
            .create("Workspace", "/tmp", &owner.id)
            .await
            .unwrap();
        WorkspacesRepo::new(pool.clone())
            .set_member(&ws.id, &outsider.id, WorkspaceRole::Editor)
            .await
            .unwrap();
        let conn_id = otto_core::new_id();
        sqlx::query("INSERT INTO connections (id,name,kind,params_json,created_by,created_at) VALUES (?,'SSH','ssh','{}',?,'2026-09-05T00:00:00Z')").bind(&conn_id).bind(&owner.id).execute(&pool).await.unwrap();
        for user in [&owner, &outsider] {
            sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'connections','view')").bind(&user.id).execute(&pool).await.unwrap();
        }
        let repo = otto_state::ResourceAccessRepo::new(pool.clone());
        repo.initialize_owner_policy(
            ResourceKind::Connection,
            &conn_id,
            &owner.id,
            &["discover".into(), "shell".into()],
            &[],
            &AccessActor {
                real_user_id: owner.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        let mut shared = repo
            .get_policy(ResourceKind::Connection, &conn_id)
            .await
            .unwrap();
        let mut outsider_rule = shared.rules[0].clone();
        outsider_rule.id = otto_core::new_id();
        outsider_rule.subject_id = outsider.id.clone();
        shared.rules.push(outsider_rule);
        repo.put_policy(
            &shared,
            shared.revision,
            &AccessActor {
                real_user_id: owner.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        let session = otto_state::SessionsRepo::new(pool.clone())
            .create(otto_state::NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Connection,
                provider: "ssh".into(),
                title: "SSH".into(),
                cwd: "/tmp".into(),
                provider_session_id: None,
                connection_id: Some(conn_id.clone()),
                created_by: owner.id.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        assert!(input_user(&pool, &owner.id, &session).await.is_ok());
        assert!(
            input_user(&pool, &outsider.id, &session).await.is_err(),
            "workspace Editor cannot inject into another user's terminal"
        );
        sqlx::query("DELETE FROM user_feature_grants WHERE user_id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "page revocation applies at delayed input"
        );
        sqlx::query("INSERT INTO user_feature_grants (user_id,feature,capability) VALUES (?,'connections','view')").bind(&owner.id).execute(&pool).await.unwrap();
        WorkspacesRepo::new(pool.clone())
            .set_member(&ws.id, &owner.id, WorkspaceRole::Viewer)
            .await
            .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "ownership cannot override revoked Editor role"
        );
        WorkspacesRepo::new(pool.clone())
            .set_member(&ws.id, &owner.id, WorkspaceRole::Admin)
            .await
            .unwrap();
        assert!(input_user(&pool, &owner.id, &session).await.is_ok());
        sqlx::query("UPDATE users SET disabled=1 WHERE id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "disabled user cannot supply delayed input"
        );
        sqlx::query("UPDATE users SET disabled=0 WHERE id=?")
            .bind(&owner.id)
            .execute(&pool)
            .await
            .unwrap();
        let mut policy = repo
            .get_policy(ResourceKind::Connection, &conn_id)
            .await
            .unwrap();
        policy.rules.clear();
        repo.put_policy(
            &policy,
            policy.revision,
            &AccessActor {
                real_user_id: owner.id.clone(),
                effective_user_id: None,
            },
        )
        .await
        .unwrap();
        assert!(
            input_user(&pool, &owner.id, &session).await.is_err(),
            "resource revoke applies even with workspace Admin"
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine
mod readiness_tests {
    use super::local_branch_facts;
    use std::path::Path;
    use std::process::Command;

    /// Run git in `dir`, panicking with git's own stderr on failure.
    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[tokio::test]
    async fn readiness_fills_unpushed_and_freshness() {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&origin).unwrap();
        std::fs::create_dir_all(&work).unwrap();

        // Bare origin with `main` and `feature`, then a clone that moves ahead.
        git(
            &origin,
            &["init", "-q", "--bare", "--initial-branch=main", "."],
        );
        git(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), "work"],
        );
        git(&work, &["config", "user.name", "t"]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "commit.gpgsign", "false"]);
        std::fs::write(work.join("a.txt"), "one\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "init"]);
        git(&work, &["push", "-q", "-u", "origin", "main"]);

        git(&work, &["checkout", "-q", "-b", "feature"]);
        std::fs::write(work.join("b.txt"), "two\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "feature 1"]);
        git(&work, &["push", "-q", "-u", "origin", "feature"]);

        // Two local commits that origin/feature has not seen …
        for (name, body) in [("c.txt", "three\n"), ("d.txt", "four\n")] {
            std::fs::write(work.join(name), body).unwrap();
            git(&work, &["add", "-A"]);
            git(&work, &["commit", "-q", "-m", name]);
        }
        // … and a main that moved on, so origin/main is NOT an ancestor.
        git(&work, &["checkout", "-q", "main"]);
        std::fs::write(work.join("e.txt"), "five\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-q", "-m", "main moves"]);
        git(&work, &["push", "-q", "origin", "main"]);
        git(&work, &["checkout", "-q", "feature"]);

        let path = work.to_str().unwrap();
        assert_eq!(
            local_branch_facts(path, "feature", "main").await,
            (Some(2), "behind")
        );

        // Merging origin/main in makes it an ancestor → fresh. Unpushed grows to
        // four: the two feature commits, main's commit, and the merge commit —
        // none of them reachable from origin/feature.
        git(&work, &["merge", "-q", "--no-edit", "origin/main"]);
        assert_eq!(
            local_branch_facts(path, "feature", "main").await,
            (Some(4), "fresh")
        );

        // A source branch that does not exist locally is unknown on both axes.
        assert_eq!(
            local_branch_facts(path, "no-such-branch", "main").await,
            (None, "unknown")
        );
    }
}

#[cfg(test)]
mod review_cancel_guard_tests {
    use super::ReviewCancelGuard;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn guard_unregisters_its_own_flag_on_drop() {
        let reg: crate::skill_eval::CancelRegistry = Default::default();
        let f = flag();
        reg.lock().unwrap().insert("r1".into(), f.clone());
        drop(ReviewCancelGuard::new(&reg, "r1", &f));
        assert!(reg.lock().unwrap().is_empty());
    }

    #[test]
    fn guard_keeps_a_newer_attempts_flag() {
        let reg: crate::skill_eval::CancelRegistry = Default::default();
        let old = flag();
        reg.lock().unwrap().insert("r1".into(), old.clone());
        let guard = ReviewCancelGuard::new(&reg, "r1", &old);
        // A retry replaces the entry while the old attempt is unwinding.
        let newer = flag();
        reg.lock().unwrap().insert("r1".into(), newer.clone());
        drop(guard);
        let map = reg.lock().unwrap();
        assert!(Arc::ptr_eq(map.get("r1").unwrap(), &newer));
    }
}

#[cfg(test)]
mod agent_input_rule_tests {
    use super::{agent_input_rule, DELEGATED_BY_META};
    use otto_core::Id;
    use serde_json::json;

    /// S11-02 / S1-11: the REST twins of `/ws/term` confine an agent
    /// session's own token like the WS gate does.
    #[test]
    fn agent_token_reaches_only_its_own_session_and_its_workers() {
        let me = Id::from("lead");
        let sibling = Id::from("sibling-shell");
        let worker = Id::from("worker");
        let worker_meta = json!({ DELEGATED_BY_META: "lead" });
        let other_worker_meta = json!({ DELEGATED_BY_META: "someone-else" });
        // A person's credential is unaffected.
        assert!(agent_input_rule(None, &sibling, &json!({}), false).is_ok());
        // Own session: /input and /message.
        assert!(agent_input_rule(Some(&me), &me, &json!({}), false).is_ok());
        assert!(agent_input_rule(Some(&me), &me, &json!({}), true).is_ok());
        // A sibling (the owner's unsandboxed shell): never.
        assert!(agent_input_rule(Some(&me), &sibling, &json!({}), false).is_err());
        assert!(agent_input_rule(Some(&me), &sibling, &json!({}), true).is_err());
        // Its own worker: /message yes (otto_send_message), raw /input no.
        assert!(agent_input_rule(Some(&me), &worker, &worker_meta, true).is_ok());
        assert!(agent_input_rule(Some(&me), &worker, &worker_meta, false).is_err());
        // Another lead's worker: no.
        assert!(agent_input_rule(Some(&me), &worker, &other_worker_meta, true).is_err());
    }
}
