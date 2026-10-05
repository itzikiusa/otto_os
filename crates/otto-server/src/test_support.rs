//! Test fixtures for building a full [`ServerCtx`] without a daemon.
//!
//! Before this module every integration suite copied a ~70-field `ServerCtx`
//! struct literal, so adding a field to the context meant editing a dozen test
//! files. [`ServerCtx::for_tests`] is now the ONE place that wires a
//! self-contained context (in-memory secrets, no-op connection spawner, usage
//! engine disabled, no telemetry, no channel bridge). Suites that need a
//! different value override the public field afterwards:
//!
//! ```ignore
//! let mut ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
//! ctx.base_url = format!("http://{addr}");
//! ```
//!
//! Compiled for this crate's unit tests and, through the `test-util` feature
//! (enabled by the crate's self dev-dependency), for `tests/*.rs`. Never part
//! of a release build.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::DbPool;
use tokio::sync::broadcast;

use crate::state::ServerCtx;

/// Process-local secret store (the Keychain is never touched by tests).
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Connection spawner that refuses every spawn (tests never open SSH/DB PTYs).
pub struct NoopSpawner;

impl otto_connections::Spawner for NoopSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _conn: &'a otto_core::domain::Connection,
        _spec: otto_pty::CommandSpec,
        _first_command: Option<String>,
        _title: Option<String>,
    ) -> otto_core::auth::BoxFuture<'a, Result<otto_core::domain::Session>> {
        Box::pin(async { Err(Error::Internal("noop spawner".into())) })
    }
}

impl ServerCtx {
    /// A complete, isolated context over `pool` with every on-disk root under
    /// `data_dir`. Uses [`MemorySecrets`]; see [`Self::for_tests_with_secrets`].
    pub async fn for_tests(pool: &DbPool, data_dir: impl Into<PathBuf>) -> ServerCtx {
        Self::for_tests_with_secrets(pool, data_dir, Arc::new(MemorySecrets::default())).await
    }

    /// [`Self::for_tests`] with a caller-supplied secret store.
    pub async fn for_tests_with_secrets(
        pool: &DbPool,
        data_dir: impl Into<PathBuf>,
        secrets: Arc<dyn SecretStore>,
    ) -> ServerCtx {
        let data_dir: PathBuf = data_dir.into();
        let (events, _rx) = broadcast::channel(256);
        let manager = Arc::new(otto_sessions::SessionManager::new(
            otto_state::SessionsRepo::new(pool.clone()),
            events.clone(),
            otto_sessions::ProviderRegistry::new(None),
        ));
        let orchestrator = Arc::new(otto_orchestrator::Orchestrator::new("claude"));
        let improve_engine = Arc::new(otto_improve::ImprovementEngine {
            improvements: otto_state::ImprovementsRepo::new(pool.clone()),
            sessions: otto_state::SessionsRepo::new(pool.clone()),
            workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
            producer: Arc::new(otto_improve::RealProposalProducer::new(
                orchestrator.clone(),
            )),
            events: events.clone(),
            library_root: data_dir.join("lib"),
        });
        let connections = Arc::new(otto_connections::ConnectionsService::new(
            otto_state::ConnectionsRepo::new(pool.clone()),
            otto_state::ConnectionSectionsRepo::new(pool.clone()),
            secrets.clone(),
        ));
        let db_explorer = Arc::new(otto_dbviewer::DbViewerService::new(
            otto_state::ConnectionsRepo::new(pool.clone()),
            secrets.clone(),
            otto_state::DbExplorerRepo::new(pool.clone()),
        ));
        let brokers = Arc::new(otto_brokers::BrokersService::new(
            otto_state::BrokerClustersRepo::new(pool.clone()),
            secrets.clone(),
            None,
        ));
        let mcp = Arc::new(otto_mcp::McpService::new(pool.clone(), secrets.clone()));
        let swarm_repo = otto_state::SwarmRepo::new(pool.clone());
        let swarm = Arc::new(otto_swarm::SwarmService::new(swarm_repo.clone()));
        let product_repo = otto_state::ProductRepo::new(pool.clone());
        let product = Arc::new(otto_product::ProductService::new(
            product_repo.clone(),
            otto_state::IssuesRepo::new(pool.clone()),
            secrets.clone(),
        ));
        let usage = otto_usage::UsageEngine::start(
            otto_usage::UsageConfig {
                enabled: false, // Fixtures never start ClickHouse.
                ..Default::default()
            },
            data_dir.join("usage"),
        )
        .await;

        ServerCtx {
            pool: pool.clone(),
            secrets,
            events: events.clone(),
            authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
            roles: Arc::new(otto_rbac::RbacRoleChecker::new(pool.clone())),
            auth_cache: otto_rbac::AuthCache::new(),
            version: "test".into(),
            base_url: "http://127.0.0.1:0".into(),
            data_dir: data_dir.clone(),
            plugins: Arc::new(crate::plugins::PluginManager::new(
                otto_state::PluginsRepo::new(pool.clone()),
                data_dir.join("plugins"),
                data_dir.clone(),
                "http://127.0.0.1:7700/api/v1/plugin-host".into(),
            )),
            manager,
            rooms: Default::default(),
            workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
            connections,
            db_explorer,
            db_assist: crate::db_assist::new_registry(),
            transcript_cache: Default::default(),
            brokers,
            mcp,
            spawner: Arc::new(NoopSpawner),
            git_store: otto_state::GitStore::new(pool.clone()),
            issues_store: otto_state::IssuesRepo::new(pool.clone()),
            integrations_store: otto_state::IntegrationsRepo::new(pool.clone()),
            channel_bridge: None,
            wf_skip_current: Default::default(),
            reviews_store: otto_state::ReviewsRepo::new(pool.clone()),
            review_cancels: Default::default(),
            review_agent_cancels: Default::default(),
            findings_store: otto_state::ReviewFindingsRepo::new(pool.clone()),
            finding_events_store: otto_state::FindingEventsRepo::new(pool.clone()),
            repo_rules_store: otto_state::RepoRulesRepo::new(pool.clone()),
            proof_packs_store: otto_state::ReviewProofPacksRepo::new(pool.clone()),
            skill_evals_store: otto_state::SkillEvalsRepo::new(pool.clone()),
            golden_tasks_store: otto_state::GoldenTasksRepo::new(pool.clone()),
            eval_matrices_store: otto_state::EvalMatricesRepo::new(pool.clone()),
            skill_eval_cancels: Default::default(),
            skill_reviews_store: otto_state::SkillReviewsRepo::new(pool.clone()),
            skill_review_cancels: Default::default(),
            orchestrator,
            improve_engine,
            context_library: otto_context::Library::new(data_dir.join("ctx")),
            usage,
            telemetry: None,
            product,
            product_repo,
            attachment_repo: otto_state::ProductAttachmentRepo::new(pool.clone()),
            discovery_repo: otto_state::ProductDiscoveryRepo::new(pool.clone()),
            refinement_repo: otto_state::ProductRefinementRepo::new(pool.clone()),
            mockup_repo: otto_state::ProductMockupRepo::new(pool.clone()),
            discovery_chat_repo: otto_state::DiscoveryChatRepo::new(pool.clone()),
            canvas_repo: otto_state::CanvasRepo::new(pool.clone()),
            product_agent_cancels: crate::product_run::new_cancel_registry(),
            design_jobs: crate::design_blender::new_job_registry(),
            memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
            vault: Arc::new(otto_vault::VaultEngine::new(pool.clone())),
            vault_docs_runs: crate::vault_docs_agent::new_run_registry(),
            vault_docs_refine: crate::vault_docs_agent::new_refine_registry(),
            swarm,
            swarm_repo,
            swarm_coords: crate::swarm_runtime::new_registry(),
            swarm_run_cancels: crate::swarm_run::new_cancel_registry(),
            goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
            goal_loops: crate::goal_loop::new_registry(),
            workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
                otto_state::WorkGraphRepo::new(pool.clone()),
                events.clone(),
            )),
            scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
            proof_repo: otto_state::ProofRepo::new(pool.clone()),
            proof_locks: crate::proof::new_locks(),
            runs: otto_state::RunsRepo::new(pool.clone()),
            runs_engine: crate::run_engine::RunEngine::new(),
            browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
            browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
            browser_credentials: otto_state::BrowserCredentialsRepo::new(pool.clone()),
            browser: Arc::new(crate::routes::browser::BrowserEngineHandle::new(
                None,
                data_dir.join("browser"),
            )),
            ui_bridge: Default::default(),
        }
    }
}

/// A fresh in-memory SQLite pool with every migration applied (one connection,
/// so the in-memory DB is shared by every query).
pub async fn mem_pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}
