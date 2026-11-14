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
pub mod deps_inspector;
pub mod environments;
pub mod gutter;
pub mod http;
pub mod java_workspace;
pub mod local_history;
pub mod lsp_host;
pub mod lsp_service;
pub mod plugins;
pub mod projects;
pub mod target;
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
}

impl IdeEngine {
    pub fn new(data_dir: PathBuf) -> Self {
        environments::ensure_shell_env_warming();
        let history = local_history::LocalHistoryStore::new(data_dir.clone());
        let projects = projects::ProjectStore::new(data_dir.clone());
        Self {
            targets: target::TargetManager::with_history(history.clone()),
            toolchains: toolchains::ToolchainManager::new(data_dir.clone()),
            environments: environments::EnvironmentManager::new(data_dir.clone()),
            dap: dap::DapManager::new(data_dir.clone()),
            collab: collab::CollabHub::new(),
            plugins: plugins::IdePluginRegistry::new(data_dir.clone()),
            history,
            projects,
            data_dir,
        }
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
