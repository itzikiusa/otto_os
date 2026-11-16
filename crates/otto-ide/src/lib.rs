//! Full IDE engine for Otto (`crates/otto-ide`).
//!
//! Unifies Local and Remote SSH targets (`target`), OS/arch SDK & toolchain
//! installation (`toolchains`, `toolchains_catalog`), project environments &
//! run configurations (`environments`), multi-language gutter test/main runners
//! (`gutter`), a full Debug Adapter Protocol engine (`dap`), extended LSP
//! intelligence (`lsp_service`), "Share with Friends" live collaborative
//! sessions (`collab`), and zero-rebuild runtime IDE plugins (`plugins`).

pub mod build_tools;
pub mod collab;
pub mod dap;
pub mod db_watch;
pub mod deps_inspector;
pub mod environments;
pub mod go_build_cache;
pub mod go_build_tags;
pub mod gutter;
pub mod http;
pub mod java_workspace;
pub mod jsonrpc_framing;
pub mod local_history;
pub mod lsp_host;
pub mod lsp_service;
pub mod netbridge;
pub mod plugins;
pub mod projects;
pub mod scratches;
pub mod target;
pub mod text_pos;
pub mod toolchains;
pub mod toolchains_catalog;
pub mod types;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use otto_core::auth::{BoxFuture, RoleChecker};
use otto_core::domain::User;
use otto_core::secrets::SecretStore;
use otto_core::{Id, Result};
use otto_state::DbPool;

pub use lsp_service::extract_go_file_build_tags;
pub use types::IdeSshSpec;

/// Host context required by `otto-ide`'s routers and services.
pub trait IdeCtx: Clone + Send + Sync + 'static {
    fn ide(&self) -> &Arc<IdeEngine>;
    fn pool(&self) -> DbPool;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn data_dir(&self) -> &Path;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn resolve_ssh_connection<'a>(
        &'a self,
        user: &'a User,
        connection_id: &'a Id,
    ) -> BoxFuture<'a, Result<IdeSshSpec>>;
    fn authorize_local_target<'a>(
        &'a self,
        _user: &'a User,
        _spec: &'a types::IdeTargetSpec,
        _write: bool,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn list_saved_db_targets<'a>(
        &'a self,
        _user: &'a User,
    ) -> BoxFuture<'a, Result<Vec<db_watch::IdeSavedDbTarget>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn execute_db_watch_query<'a>(
        &'a self,
        _user: &'a User,
        _exec: db_watch::DbWatchQueryExec,
    ) -> BoxFuture<'a, Result<db_watch::DbWatchExecOutcome>> {
        Box::pin(async {
            Err(otto_core::Error::Invalid(
                "database watch query executor is not wired in this context".into(),
            ))
        })
    }
    fn resolve_k8s_cluster<'a>(
        &'a self,
        _user: &'a User,
        _cluster_id: &'a Id,
        _namespace: Option<&'a str>,
    ) -> BoxFuture<'a, Result<netbridge::IdeK8sSpec>> {
        Box::pin(async {
            Err(otto_core::Error::Invalid(
                "Kubernetes cluster resolver is not wired in this context".into(),
            ))
        })
    }
}

/// Shared stateful IDE engine mounted on `ServerCtx`.
pub struct IdeEngine {
    pub data_dir: PathBuf,
    pub targets: target::TargetManager,
    pub toolchains: toolchains::ToolchainManager,
    pub environments: environments::EnvironmentManager,
    pub dap: dap::DapManager,
    pub collab: collab::CollabHub,
    pub plugins: plugins::IdePluginRegistry,
    pub history: local_history::LocalHistoryStore,
    pub projects: projects::ProjectStore,
    pub lsp_pool: Arc<lsp_host::pool::LspPool>,
    pub db_watch: db_watch::DbWatchRegistry,
    pub netbridge: netbridge::NetBridgeManager,
}

impl IdeEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        let shell_env = Arc::new(environments::ShellEnvResolver::with_cache_dir(&data_dir));
        shell_env.ensure_warming_in_background();
        let history = local_history::LocalHistoryStore::new(data_dir.clone());
        let projects = projects::ProjectStore::new(data_dir.clone());
        let environments = environments::EnvironmentManager::with_shell_env(
            data_dir.clone(),
            Arc::clone(&shell_env),
        );
        let toolchains =
            toolchains::ToolchainManager::with_shell_env(data_dir.clone(), Arc::clone(&shell_env));
        let lsp_pool = lsp_host::pool::LspPool::new_managed(
            data_dir.clone(),
            lsp_host::pool::IDLE_TIMEOUT,
            environments.clone(),
        );
        let targets = target::TargetManager::with_services(
            history.clone(),
            Arc::clone(&lsp_pool),
            environments.clone(),
        );
        let dap = dap::DapManager::with_environments(data_dir.clone(), environments.clone());
        Self {
            targets,
            toolchains,
            environments,
            dap,
            collab: collab::CollabHub::new(),
            plugins: plugins::IdePluginRegistry::new(data_dir.clone()),
            history,
            projects,
            lsp_pool,
            db_watch: db_watch::DbWatchRegistry::new(),
            netbridge: netbridge::NetBridgeManager::new(data_dir.clone()),
            data_dir,
        }
    }

    /// Live root PIDs of active local IDE runs and DAP debug sessions, used by
    /// [`db_watch::DbWatchRegistry`] to discover in-process `miniredis` servers.
    pub fn active_local_pids(&self) -> Vec<u32> {
        let mut pids = self.environments.active_local_pids();
        pids.extend(self.dap.active_local_pids());
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    pub fn authorize_debug_session(&self, user: &User, debug_id: &str) -> Result<()> {
        self.dap.authorize_session(user, debug_id)
    }

    pub fn filter_debug_sessions(
        &self,
        user: &User,
        sessions: Vec<types::IdeDebugSessionSnapshot>,
    ) -> Vec<types::IdeDebugSessionSnapshot> {
        self.dap.filter_sessions_for_user(user, sessions)
    }

    /// Gracefully terminate all active IDE runs, DAP debuggers, pooled LSP servers,
    /// and network bridges on daemon shutdown.
    pub async fn shutdown(&self) {
        self.environments.shutdown().await;
        self.dap.shutdown().await;
        self.lsp_pool.shutdown();
        self.netbridge.shutdown().await;
    }
}

/// Protected `/ide/*` REST & SSE routes (nested under `/api/v1`).
pub fn api_router<S: IdeCtx>() -> Router<S> {
    http::api_router::<S>()
}

/// Public capability-token `/ide/collab/guest/*` routes (nested under `/api/v1`).
pub fn public_router<S: IdeCtx>() -> Router<S> {
    http::public_router::<S>()
}
