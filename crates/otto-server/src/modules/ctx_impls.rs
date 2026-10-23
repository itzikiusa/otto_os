//! The module routers' ctx-trait impls on [`ServerCtx`] and the PTY-backed
//! connection [`Spawner`].

use super::*;

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
    pub(crate) async fn spawn_adhoc(
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
pub(crate) struct UsageSink(Arc<otto_usage::UsageEngine>);

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
pub(crate) struct DbViewerTester {
    pub(crate) db: Arc<otto_dbviewer::DbViewerService>,
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
pub(crate) fn cleanup_base_key(repo_id: &Id) -> String {
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
            // Rewrite / test-generation / plan sessions have no agent row:
            // trip their story-keyed cancel flags (so recovery does not
            // respawn) and kill every live session attributed to the story.
            // (Killed last, after the analysis agents' flags are tripped too.)
            otto_product::run::signal_story_cancels(&self.product_agent_cancels, story_id);
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
            if let Ok(story) = self.product_repo.get_story(story_id).await {
                // Legacy studio metadata used top-level story_id; analysis and
                // plan sessions use work.story_id. Both belong to this story.
                for key in ["work.story_id", "story_id"] {
                    let live = self
                        .manager
                        .list_live_by_meta(&story.workspace_id, None, key, story_id)
                        .await
                        .unwrap_or_default();
                    for session in live {
                        let _ = self.manager.kill_session(&session.id).await;
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
    fn ensure_trusted_scratch(&self, provider: &str, root: &std::path::Path, cwd: &str) {
        otto_sessions::trust::ensure_trusted_scratch(provider, root, cwd);
    }
    fn sessions(&self) -> &Arc<otto_sessions::SessionManager> {
        &self.manager
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
        // The one locked submit (paste → Enter → re-Enter under the session's
        // submit lock, re-Enter covering a TUI that ate the first Enter).
        self.manager
            .submit_prompt(sid, prompt, otto_sessions::submit::SubmitOpts::default())
            .await
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
    fn sessions(&self) -> &Arc<otto_sessions::SessionManager> {
        &self.manager
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
