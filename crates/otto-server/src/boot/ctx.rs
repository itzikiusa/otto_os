//! The ONE `ServerCtx` struct literal.
//!
//! Both the daemon ([`super::build_ctx`]) and the test fixture
//! (`ServerCtx::for_tests`) assemble their context here. A caller constructs
//! only what genuinely differs between a real daemon and a test (the session
//! manager, the connection spawner, the usage engine, …) and hands it over as
//! [`CtxParts`]; every pool-backed repo, every service whose construction is
//! identical in both, and every empty in-memory registry is filled in by
//! [`ServerCtx::from_parts`]. Adding a context field therefore means editing
//! this file (plus a `CtxParts` field when the value is environment-specific).
//!
//! Next step (deliberately not done while the subsystem split is in flight):
//! group the ~75 fields into sub-structs along the section comments in
//! `state.rs` — review (`reviews_store`, `findings_store`, `review_cancels`,
//! …), eval lab (`skill_evals_store`, `golden_tasks_store`, …), product
//! (`product*`, `*_repo` of the Product arena), swarm, browser, run-with-otto.
//! That touches every `ctx.<field>` use across the crate, so it belongs after
//! the split agents have moved their subsystems out.

use std::path::PathBuf;
use std::sync::Arc;

use otto_connections::Spawner;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_improve::ImprovementEngine;
use otto_orchestrator::Orchestrator;
use otto_sessions::SessionManager;
use otto_state::DbPool;
use tokio::sync::broadcast;

use crate::state::ServerCtx;

/// The environment-specific inputs of a [`ServerCtx`] (see the module doc).
pub struct CtxParts {
    pub pool: DbPool,
    pub secrets: Arc<dyn SecretStore>,
    pub events: broadcast::Sender<Event>,
    /// Reported by `/meta` (the daemon's `CARGO_PKG_VERSION`).
    pub version: String,
    /// Loopback base URL of this daemon (`http://127.0.0.1:<port>`).
    pub base_url: String,
    pub data_dir: PathBuf,
    pub plugins: Arc<crate::plugins::PluginManager>,
    pub manager: Arc<SessionManager>,
    pub spawner: Arc<dyn Spawner>,
    /// The daemon persists an audit row per broker write; tests do not.
    pub brokers: Arc<otto_brokers::BrokersService>,
    pub channel_bridge: Option<Arc<otto_channels::Bridge>>,
    pub orchestrator: Arc<Orchestrator>,
    pub improve_engine: Arc<ImprovementEngine>,
    pub context_library: otto_context::Library,
    pub usage: Arc<otto_usage::UsageEngine>,
    pub telemetry: Option<Arc<otto_telemetry::TelemetryService>>,
    /// Local SQLite, a shared remote host, or vault write-through.
    pub memory: Arc<otto_memory::MemoryService>,
    /// Content-addressed proof media store (`None` keeps media inline).
    pub proof_media_dir: Option<PathBuf>,
    pub browser: Arc<crate::routes::browser::BrowserEngineHandle>,
    /// Wire `auth_cache` into the authenticator (the daemon). `false` keeps
    /// the authenticator uncached — every request hits the DB, as test
    /// fixtures always have (no 10 s stale-token window between assertions).
    pub cache_auth_lookups: bool,
}

impl ServerCtx {
    /// Assemble the full context from its environment-specific `parts`.
    pub fn from_parts(parts: CtxParts) -> ServerCtx {
        let CtxParts {
            pool,
            secrets,
            events,
            version,
            base_url,
            data_dir,
            plugins,
            manager,
            spawner,
            brokers,
            channel_bridge,
            orchestrator,
            improve_engine,
            context_library,
            usage,
            telemetry,
            memory,
            proof_media_dir,
            browser,
            cache_auth_lookups,
        } = parts;

        // One shared auth-lookup cache: the authenticator fills it, the grants
        // route (via ServerCtx.auth_cache) evicts from it on set_grants. Clones
        // share the same backing map (Arc-backed).
        let auth_cache = otto_rbac::AuthCache::new();
        let product_repo = otto_state::ProductRepo::new(pool.clone());
        let swarm_repo = otto_state::SwarmRepo::new(pool.clone());
        let proof_repo = match proof_media_dir {
            // Proof media lives in a content-addressed file store, not the state DB.
            Some(dir) => otto_state::ProofRepo::new(pool.clone()).with_media_dir(dir),
            None => otto_state::ProofRepo::new(pool.clone()),
        };

        let authenticator = if cache_auth_lookups {
            otto_rbac::RbacAuthenticator::new_with_cache(pool.clone(), auth_cache.clone())
        } else {
            otto_rbac::RbacAuthenticator::new(pool.clone())
        };

        ServerCtx {
            authenticator: Arc::new(authenticator),
            roles: Arc::new(otto_rbac::RbacRoleChecker::new(pool.clone())),
            auth_cache,
            version,
            base_url,
            data_dir,
            plugins,
            manager,
            rooms: Default::default(),
            workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
            connections: Arc::new(otto_connections::ConnectionsService::new(
                otto_state::ConnectionsRepo::new(pool.clone()),
                otto_state::ConnectionSectionsRepo::new(pool.clone()),
                secrets.clone(),
            )),
            // Native data-access layer for the DB Explorer: reuses connection
            // profiles + keychain secrets, persists saved queries / history /
            // dashboards.
            db_explorer: Arc::new(otto_dbviewer::DbViewerService::new(
                otto_state::ConnectionsRepo::new(pool.clone()),
                secrets.clone(),
                otto_state::DbExplorerRepo::new(pool.clone()),
            )),
            db_assist: crate::db_assist::new_registry(),
            transcript_cache: Default::default(),
            brokers,
            // MCP Control Plane: outbound MCP client + governance pipeline.
            // Server config is the augmented `mcp_servers`; secrets resolve
            // from the same store.
            mcp: Arc::new(otto_mcp::McpService::new(pool.clone(), secrets.clone())),
            spawner,
            git_store: otto_state::GitStore::new(pool.clone()),
            issues_store: otto_state::IssuesRepo::new(pool.clone()),
            integrations_store: otto_state::IntegrationsRepo::new(pool.clone()),
            channel_bridge,
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
            context_library,
            usage,
            telemetry,
            product: Arc::new(otto_product::ProductService::new(
                product_repo.clone(),
                otto_state::IssuesRepo::new(pool.clone()),
                secrets.clone(),
            )),
            product_repo,
            attachment_repo: otto_state::ProductAttachmentRepo::new(pool.clone()),
            discovery_repo: otto_state::ProductDiscoveryRepo::new(pool.clone()),
            refinement_repo: otto_state::ProductRefinementRepo::new(pool.clone()),
            mockup_repo: otto_state::ProductMockupRepo::new(pool.clone()),
            discovery_chat_repo: otto_state::DiscoveryChatRepo::new(pool.clone()),
            canvas_repo: otto_state::CanvasRepo::new(pool.clone()),
            product_agent_cancels: otto_product::run::new_cancel_registry(),
            design_jobs: crate::design_blender::new_job_registry(),
            memory,
            // Vault v3 — the file-backed docs home (markdown vaults on disk,
            // derived SQLite index, OKF validation). No embeddings anywhere.
            vault: Arc::new(otto_vault::VaultEngine::new(pool.clone())),
            vault_docs_runs: crate::vault_docs_agent::new_run_registry(),
            vault_docs_refine: crate::vault_docs_agent::new_refine_registry(),
            swarm: Arc::new(otto_swarm::SwarmService::new(swarm_repo.clone())),
            swarm_repo,
            swarm_coords: otto_swarm::runtime::engine::new_registry(),
            swarm_run_cancels: otto_swarm::runtime::run::new_cancel_registry(),
            goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
            goal_loops: crate::goal_loop::new_registry(),
            workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
                otto_state::WorkGraphRepo::new(pool.clone()),
                events.clone(),
            )),
            scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
            proof_repo,
            proof_locks: crate::proof::new_locks(),
            runs: otto_state::RunsRepo::new(pool.clone()),
            runs_engine: crate::run_engine::RunEngine::new(),
            browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
            browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
            browser_credentials: otto_state::BrowserCredentialsRepo::new(pool.clone()),
            browser,
            ui_bridge: Default::default(),
            secrets,
            events,
            pool,
        }
    }
}
