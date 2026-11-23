//! Instance-scoped [`ContainersService`] owning managed Kiri daemons,
//! per-`data_dir` runtime lifecycle locks, and short-TTL endpoint discovery caching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use tokio::sync::{Mutex as TokioMutex, RwLock};

use crate::client::DockerClient;
use crate::contexts::{
    active_endpoint_from_resp, discover_contexts, locate_docker_cli, normalize_socket_path,
    save_selection, ResolvedEndpoint, SavedContextSelection,
};
use crate::runtime::{
    install_runtime_locked, runtime_status_in, shutdown_all_managed, start_runtime_locked,
    stop_runtime_locked, ManagedRegistry,
};
use crate::types::{
    ContainersContextsResp, InstallRuntimeReq, RuntimeStatusResp, SelectContextReq,
};

/// Short TTL for cached context discovery + active endpoint resolution.
const DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(3);

/// Configuration injected into [`ContainersService`] at construction time.
///
/// Production builds use [`ContainersConfig::new`] (all overrides `None`);
/// isolated tests construct a config with explicit socket/home/kiri overrides
/// without polluting the production [`crate::ContainersCtx`] trait.
#[derive(Debug, Clone)]
pub struct ContainersConfig {
    pub data_dir: PathBuf,
    pub docker_socket_override: Option<PathBuf>,
    pub home_dir_override: Option<PathBuf>,
    pub kiri_bin_override: Option<PathBuf>,
    pub kiri_socket_override: Option<PathBuf>,
}

impl ContainersConfig {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            docker_socket_override: None,
            home_dir_override: None,
            kiri_bin_override: None,
            kiri_socket_override: None,
        }
    }
}

struct CachedDiscovery {
    fetched_at: Instant,
    contexts: ContainersContextsResp,
    active: ResolvedEndpoint,
}

/// Instance-scoped container runtime & discovery service owned by `ServerCtx`.
pub struct ContainersService {
    config: ContainersConfig,
    managed: ManagedRegistry,
    runtime_locks: StdMutex<HashMap<PathBuf, Arc<TokioMutex<()>>>>,
    discovery_cache: RwLock<Option<CachedDiscovery>>,
}

impl ContainersService {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_config(ContainersConfig::new(data_dir))
    }

    pub fn with_config(config: ContainersConfig) -> Self {
        Self {
            config,
            managed: StdMutex::new(HashMap::new()),
            runtime_locks: StdMutex::new(HashMap::new()),
            discovery_cache: RwLock::new(None),
        }
    }

    pub fn config(&self) -> &ContainersConfig {
        &self.config
    }

    pub fn data_dir(&self) -> &Path {
        &self.config.data_dir
    }

    fn runtime_lock_for(&self, data_dir: &Path) -> Arc<TokioMutex<()>> {
        let mut locks = self.runtime_locks.lock().unwrap_or_else(|p| p.into_inner());
        locks
            .entry(data_dir.to_path_buf())
            .or_insert_with(|| Arc::new(TokioMutex::new(())))
            .clone()
    }

    /// Clear the cached context discovery / active endpoint snapshot.
    pub async fn invalidate_cache(&self) {
        *self.discovery_cache.write().await = None;
    }

    fn env_docker_host(&self) -> Option<String> {
        if self.config.docker_socket_override.is_some() {
            None
        } else {
            std::env::var("DOCKER_HOST").ok()
        }
    }

    /// Discover available Docker contexts (serving from the short-TTL cache
    /// unless `force_refresh` is true).
    pub async fn discover_contexts(&self, force_refresh: bool) -> ContainersContextsResp {
        if !force_refresh {
            let guard = self.discovery_cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < DISCOVERY_CACHE_TTL {
                    return cached.contexts.clone();
                }
            }
        }
        let env_host = self.env_docker_host();
        let resp = discover_contexts(
            self.config.home_dir_override.as_deref(),
            &self.config.data_dir,
            self.config.docker_socket_override.as_deref(),
            env_host.as_deref(),
        )
        .await;
        let active = active_endpoint_from_resp(&resp);
        *self.discovery_cache.write().await = Some(CachedDiscovery {
            fetched_at: Instant::now(),
            contexts: resp.clone(),
            active,
        });
        resp
    }

    /// Resolve the active Docker endpoint (cached for [`DISCOVERY_CACHE_TTL`]).
    pub async fn active_endpoint(&self) -> ResolvedEndpoint {
        {
            let guard = self.discovery_cache.read().await;
            if let Some(cached) = guard.as_ref() {
                if cached.fetched_at.elapsed() < DISCOVERY_CACHE_TTL {
                    return cached.active.clone();
                }
            }
        }
        let resp = self.discover_contexts(true).await;
        active_endpoint_from_resp(&resp)
    }

    /// Construct a [`DockerClient`] bound to the cached active endpoint.
    pub async fn active_client(&self) -> DockerClient {
        let ep = self.active_endpoint().await;
        DockerClient::new(ep.socket_path)
    }

    /// Locate the `docker` CLI binary using this service's configured `$HOME`.
    pub async fn locate_docker_cli(&self) -> Option<PathBuf> {
        locate_docker_cli(self.config.home_dir_override.as_deref()).await
    }

    /// Persist a newly selected context or custom Unix socket path, invalidate
    /// the discovery cache, and return the refreshed context list.
    pub async fn select_context(&self, req: &SelectContextReq) -> Result<ContainersContextsResp> {
        let current = self.discover_contexts(true).await;

        let selection = if let Some(raw_sock) = req
            .socket_path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let normalized = normalize_socket_path(raw_sock)?;
            let name = req
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("custom")
                .to_string();
            SavedContextSelection {
                name,
                socket_path: normalized.to_string_lossy().into_owned(),
            }
        } else if let Some(name) = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            let found = current
                .contexts
                .iter()
                .find(|c| c.name == name)
                .ok_or_else(|| Error::NotFound(format!("unknown Docker context `{name}`")))?;
            SavedContextSelection {
                name: found.name.clone(),
                socket_path: found.socket_path.clone(),
            }
        } else {
            return Err(Error::Invalid(
                "provide either `name` or `socket_path`".into(),
            ));
        };

        save_selection(&self.config.data_dir, &selection).await?;
        self.invalidate_cache().await;
        Ok(self.discover_contexts(true).await)
    }

    /// Inspect embedded Kiri runtime status.
    pub async fn runtime_status(&self, last_error: Option<String>) -> RuntimeStatusResp {
        runtime_status_in(
            &self.managed,
            &self.config.data_dir,
            self.config.home_dir_override.as_deref(),
            self.config.kiri_bin_override.as_deref(),
            self.config.kiri_socket_override.as_deref(),
            last_error,
        )
        .await
    }

    /// Provision (and optionally start) the embedded Kiri runtime (serialized per `data_dir`).
    pub async fn install_runtime(&self, req: &InstallRuntimeReq) -> Result<RuntimeStatusResp> {
        let lock = self.runtime_lock_for(&self.config.data_dir);
        let _guard = lock.lock().await;
        let res = install_runtime_locked(
            &self.managed,
            &self.config.data_dir,
            self.config.home_dir_override.as_deref(),
            self.config.kiri_bin_override.as_deref(),
            self.config.kiri_socket_override.as_deref(),
            req,
        )
        .await;
        self.invalidate_cache().await;
        res
    }

    /// Start the embedded Kiri runtime (serialized per `data_dir`).
    pub async fn start_runtime(&self) -> Result<RuntimeStatusResp> {
        let lock = self.runtime_lock_for(&self.config.data_dir);
        let _guard = lock.lock().await;
        let res = start_runtime_locked(
            &self.managed,
            &self.config.data_dir,
            self.config.home_dir_override.as_deref(),
            self.config.kiri_bin_override.as_deref(),
            self.config.kiri_socket_override.as_deref(),
        )
        .await;
        self.invalidate_cache().await;
        res
    }

    /// Stop the embedded Kiri runtime (serialized per `data_dir`).
    pub async fn stop_runtime(&self) -> Result<RuntimeStatusResp> {
        let lock = self.runtime_lock_for(&self.config.data_dir);
        let _guard = lock.lock().await;
        let res = stop_runtime_locked(
            &self.managed,
            &self.config.data_dir,
            self.config.home_dir_override.as_deref(),
            self.config.kiri_bin_override.as_deref(),
            self.config.kiri_socket_override.as_deref(),
        )
        .await;
        self.invalidate_cache().await;
        res
    }

    /// Restart the embedded Kiri runtime atomically under one per-`data_dir` lock.
    pub async fn restart_runtime(&self) -> Result<RuntimeStatusResp> {
        let lock = self.runtime_lock_for(&self.config.data_dir);
        let _guard = lock.lock().await;
        let _ = stop_runtime_locked(
            &self.managed,
            &self.config.data_dir,
            self.config.home_dir_override.as_deref(),
            self.config.kiri_bin_override.as_deref(),
            self.config.kiri_socket_override.as_deref(),
        )
        .await?;
        let res = start_runtime_locked(
            &self.managed,
            &self.config.data_dir,
            self.config.home_dir_override.as_deref(),
            self.config.kiri_bin_override.as_deref(),
            self.config.kiri_socket_override.as_deref(),
        )
        .await;
        self.invalidate_cache().await;
        res
    }

    /// Gracefully stop every Otto-managed Kiri daemon owned by this service.
    /// Invoked during `ottod` teardown so child daemons are never orphaned.
    pub async fn shutdown(&self) {
        let lock = self.runtime_lock_for(&self.config.data_dir);
        let _guard = lock.lock().await;
        shutdown_all_managed(&self.managed).await;
        self.invalidate_cache().await;
    }
}

impl Drop for ContainersService {
    fn drop(&mut self) {
        if let Ok(mut map) = self.managed.lock() {
            for (_, mut proc) in map.drain() {
                let _ = proc.child.start_kill();
            }
        }
    }
}
