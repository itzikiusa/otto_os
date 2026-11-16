//! OS/arch-aware SDK, Language Server (LSP), and Debug Adapter (DAP) manager
//! and background installer (`ToolchainManager`).
//!
//! Works transparently on both `IdeTargetKind::Local` (storing managed SDKs
//! under `<data_dir>/toolchains/<ecosystem>/...`) and `IdeTargetKind::Ssh`
//! (probing remote `uname -s -m` and installing into `$HOME/.otto/toolchains/...`
//! via [`TargetManager`]).

mod detected;
pub(crate) mod discovery;
mod installer;
mod installer_remote;
mod installer_tools;
mod jobs;
mod local_registry;
mod remote_probe;

use self::detected::scan_detected_sdks;
use self::discovery::{
    canonical_key, collect_managed_bin_dirs, discover_local_dap_with_shell,
    discover_local_lsp_with_shell, discover_local_sdks_with_shell, empty_tool, label_builtin_dap,
    mark_active_sdk,
};
#[cfg(test)]
use self::discovery::{discover_local_dap, discover_local_lsp, discover_local_sdks};
pub(crate) use self::discovery::{
    find_executable_in, is_executable_file, read_subdirs, resolve_java_home_dir,
    system_search_dirs, system_search_dirs_with_shell,
};
pub(crate) use self::local_registry::load_registered_local_sdks_by_host;
use self::local_registry::{
    host_key_for_target, load_all_local_host_sdks, normalize_and_validate_local_sdk,
};
pub(crate) use self::remote_probe::parse_ssh_probe_output;
use self::remote_probe::REMOTE_PROBE_SCRIPT;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use sqlx::Row;
use tokio::sync::RwLock;

use crate::environments::shell_env::ShellEnvResolver;
use crate::target::{ResolvedTarget, TargetManager};
use crate::toolchains_catalog::{CatalogClient, CatalogSdkEntry, EcosystemKind, TargetPlatform};
use crate::types::{
    IdeEcosystemKind, IdeInstallJob, IdeInstalledSdk, IdeProjectEnvironment,
    IdeRegisterLocalSdkReq, IdeSshSpec, IdeTargetKind, IdeTargetSpec, IdeToolchainEcosystem,
    IdeToolchainsResp,
};

pub(super) const MAX_JOB_LOGS: usize = 400;
pub(super) const MAX_FINISHED_JOBS: usize = 16;

/// Manages SDK/LSP/DAP discovery and background installation jobs across
/// local and remote SSH targets.
#[derive(Clone)]
pub struct ToolchainManager {
    data_dir: PathBuf,
    shell_env: Arc<ShellEnvResolver>,
    catalog: Arc<CatalogClient>,
    pub(super) jobs: Arc<DashMap<String, IdeInstallJob>>,
    pub(super) finished_order: Arc<Mutex<VecDeque<String>>>,
    allow_test_loopback: Arc<AtomicBool>,
    local_sdks_cache: Arc<RwLock<Option<Vec<IdeInstalledSdk>>>>,
    local_sdks_gen: Arc<AtomicU64>,
}

impl ToolchainManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self::with_shell_env(data_dir, Arc::new(ShellEnvResolver::new()))
    }

    pub fn with_shell_env(data_dir: PathBuf, shell_env: Arc<ShellEnvResolver>) -> Self {
        Self {
            data_dir,
            shell_env,
            catalog: Arc::new(CatalogClient::new()),
            jobs: Arc::new(DashMap::new()),
            finished_order: Arc::new(Mutex::new(VecDeque::new())),
            allow_test_loopback: Arc::new(AtomicBool::new(false)),
            local_sdks_cache: Arc::new(RwLock::new(None)),
            local_sdks_gen: Arc::new(AtomicU64::new(1)),
        }
    }

    pub(crate) fn shell_env_snapshot(&self) -> BTreeMap<String, String> {
        self.shell_env.vars_snapshot()
    }

    pub(crate) fn shell_path(&self) -> Option<String> {
        self.shell_env.vars_snapshot().get("PATH").cloned()
    }

    pub(crate) fn local_sdks_generation(&self) -> u64 {
        self.local_sdks_gen.load(Ordering::Acquire)
    }

    pub(crate) async fn cached_local_sdks(&self, pool: &DbPool) -> Vec<IdeInstalledSdk> {
        if let Some(cached) = self.local_sdks_cache.read().await.as_ref() {
            return cached.clone();
        }
        let loaded = load_all_local_host_sdks(pool).await;
        let mut guard = self.local_sdks_cache.write().await;
        if let Some(existing) = guard.as_ref() {
            return existing.clone();
        }
        *guard = Some(loaded.clone());
        loaded
    }

    async fn invalidate_local_sdks_cache(&self) {
        *self.local_sdks_cache.write().await = None;
        self.local_sdks_gen.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn allows_test_loopback(&self) -> bool {
        self.allow_test_loopback.load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    pub fn seed_test_catalog_entry(
        &self,
        platform: TargetPlatform,
        ecosystem: IdeEcosystemKind,
        entry: CatalogSdkEntry,
    ) {
        self.allow_test_loopback.store(true, Ordering::Relaxed);
        self.catalog
            .seed_cached_entry(platform, EcosystemKind::from(ecosystem), entry);
    }

    pub fn toolchains_dir(&self) -> PathBuf {
        self.data_dir.join("toolchains")
    }

    pub fn extra_local_bin_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for root in self.local_toolchain_roots() {
            collect_managed_bin_dirs(&root, &mut out);
        }
        out
    }

    pub fn resolve_local_lsp_path(&self, ecosystem: IdeEcosystemKind) -> Option<PathBuf> {
        discover_local_lsp_with_shell(
            &self.local_toolchain_roots(),
            EcosystemKind::from(ecosystem),
            self.shell_path().as_deref(),
        )
        .path
        .map(PathBuf::from)
    }

    pub fn resolve_local_dap_path(&self, ecosystem: IdeEcosystemKind) -> Option<PathBuf> {
        discover_local_dap_with_shell(
            &self.local_toolchain_roots(),
            EcosystemKind::from(ecosystem),
            self.shell_path().as_deref(),
        )
        .path
        .map(PathBuf::from)
    }

    fn local_toolchain_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.toolchains_dir()];
        if !otto_core::e2e::isolated() {
            if let Some(d) = dirs::home_dir().map(|h| h.join(".otto/toolchains")) {
                if d != roots[0] {
                    roots.push(d);
                }
            }
        }
        roots
    }

    /// Discover installed & downloadable SDKs, LSPs, and DAPs on `target`.
    pub async fn list_toolchains_with_opts(
        &self,
        pool: Option<&DbPool>,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
        active_env: Option<&IdeProjectEnvironment>,
        include_catalog: bool,
    ) -> Result<IdeToolchainsResp> {
        target.validate()?;
        let custom_sdks = match pool {
            Some(db) => load_registered_local_sdks_by_host(db, &host_key_for_target(target)).await,
            None => HashMap::new(),
        };
        match target.kind {
            IdeTargetKind::Local => {
                let mut proj_roots = Vec::new();
                if !target.root.trim().is_empty() && target.root.trim() != "/" {
                    proj_roots.push(PathBuf::from(target.root.trim()));
                }
                for extra in target.merged_extra_roots() {
                    proj_roots.push(PathBuf::from(extra));
                }
                if let Some(ae) = active_env {
                    for ar in &ae.additional_roots {
                        let p = PathBuf::from(ar);
                        if !proj_roots.contains(&p) {
                            proj_roots.push(p);
                        }
                    }
                }
                self.list_local(active_env, custom_sdks, proj_roots, include_catalog)
                    .await
            }
            IdeTargetKind::Ssh => {
                let spec = ssh.ok_or_else(|| {
                    Error::Invalid("SSH connection spec required for remote target".into())
                })?;
                let resolved = targets.resolve_with_ssh(target, "ide-toolchains", spec.clone())?;
                self.list_ssh(targets, &resolved, active_env, custom_sdks, include_catalog)
                    .await
            }
        }
    }

    /// Register an existing SDK/toolchain directory or interpreter from disk
    /// (`POST /ide/toolchains/local`).
    pub async fn register_local_sdk(
        &self,
        pool: &DbPool,
        req: IdeRegisterLocalSdkReq,
    ) -> Result<IdeInstalledSdk> {
        req.target.validate()?;
        let eco = EcosystemKind::from(req.ecosystem);
        let (normalized_path, version) =
            normalize_and_validate_local_sdk(&req.target, eco, &req.path)?;
        let host_key = host_key_for_target(&req.target);
        let id = format!("{}-local-{}", eco.as_str(), new_id());
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"INSERT INTO ide_local_sdks (id, host_key, ecosystem, version, path, created_at)
               VALUES (?, ?, ?, ?, ?, ?)
               ON CONFLICT(host_key, ecosystem, path) DO UPDATE SET
                   version = excluded.version"#,
        )
        .bind(&id)
        .bind(&host_key)
        .bind(eco.as_str())
        .bind(&version)
        .bind(&normalized_path)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("persist ide_local_sdks: {e}")))?;

        let saved_id: String = sqlx::query(
            "SELECT id FROM ide_local_sdks WHERE host_key = ? AND ecosystem = ? AND path = ?",
        )
        .bind(&host_key)
        .bind(eco.as_str())
        .bind(&normalized_path)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|r| r.try_get("id").ok())
        .unwrap_or(id);

        self.invalidate_local_sdks_cache().await;

        Ok(IdeInstalledSdk {
            id: saved_id,
            version,
            path: normalized_path,
            source: "local".into(),
            active: false,
        })
    }

    /// Remove a user-registered local SDK (`DELETE /ide/toolchains/local`).
    pub async fn remove_local_sdk(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        id: Option<&str>,
        path: Option<&str>,
    ) -> Result<()> {
        let host_key = host_key_for_target(target);
        let id_clean = id.map(str::trim).filter(|s| !s.is_empty());
        let path_clean = path.map(str::trim).filter(|s| !s.is_empty());
        if id_clean.is_none() && path_clean.is_none() {
            return Err(Error::Invalid(
                "either 'id' or 'path' query parameter is required".into(),
            ));
        }

        if let Some(sdk_id) = id_clean {
            sqlx::query("DELETE FROM ide_local_sdks WHERE host_key = ? AND id = ?")
                .bind(&host_key)
                .bind(sdk_id)
                .execute(pool)
                .await
                .map_err(|e| Error::Internal(format!("delete ide_local_sdks by id: {e}")))?;
        }
        if let Some(raw_path) = path_clean {
            let trimmed = raw_path.trim_end_matches('/');
            let canon = std::fs::canonicalize(trimmed)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| trimmed.to_string());
            sqlx::query("DELETE FROM ide_local_sdks WHERE host_key = ? AND (path = ? OR path = ?)")
                .bind(&host_key)
                .bind(trimmed)
                .bind(&canon)
                .execute(pool)
                .await
                .map_err(|e| Error::Internal(format!("delete ide_local_sdks by path: {e}")))?;

            if target.kind == IdeTargetKind::Local {
                if let (Ok(tc_canon), Ok(target_canon)) = (
                    std::fs::canonicalize(self.toolchains_dir()),
                    std::fs::canonicalize(trimmed),
                ) {
                    if target_canon != tc_canon && target_canon.starts_with(&tc_canon) {
                        if target_canon.is_dir() {
                            let _ = tokio::fs::remove_dir_all(&target_canon).await;
                        } else {
                            let _ = tokio::fs::remove_file(&target_canon).await;
                        }
                        if let Some(parent) = target_canon.parent() {
                            if parent != tc_canon && parent.starts_with(&tc_canon) {
                                let _ = tokio::fs::remove_dir(parent).await;
                            }
                        }
                    }
                }
            }
        }
        self.invalidate_local_sdks_cache().await;
        Ok(())
    }

    async fn list_local(
        &self,
        active_env: Option<&IdeProjectEnvironment>,
        mut custom_sdks: HashMap<EcosystemKind, Vec<IdeInstalledSdk>>,
        project_roots: Vec<PathBuf>,
        include_catalog: bool,
    ) -> Result<IdeToolchainsResp> {
        let platform = TargetPlatform::host();
        let catalogs = if include_catalog {
            self.catalog.fetch_all(platform).await
        } else {
            self.catalog.cached_or_curated(platform)
        };
        let roots = self.local_toolchain_roots();
        let env_clone = active_env.cloned();
        let shell_path = self.shell_path();

        let (ecosystems, detected) = tokio::task::spawn_blocking(move || {
            let detected = scan_detected_sdks(&roots, &project_roots);
            let ecosystems = EcosystemKind::ALL
                .into_iter()
                .map(|eco| {
                    let mut installed = custom_sdks.remove(&eco).unwrap_or_default();
                    let mut seen: BTreeSet<String> =
                        installed.iter().map(|s| canonical_key(&s.path)).collect();
                    if eco == EcosystemKind::Python {
                        for det in &detected {
                            if det.ecosystem == IdeEcosystemKind::Python
                                && matches!(det.source.as_str(), "venv" | "poetry" | "conda")
                                && seen.insert(canonical_key(&det.bin_path))
                            {
                                installed.push(IdeInstalledSdk {
                                    id: format!("python-{}-{}", det.source, installed.len() + 1),
                                    version: det.version.clone(),
                                    path: det.bin_path.clone(),
                                    source: det.source.clone(),
                                    active: false,
                                });
                            }
                        }
                    }
                    for disc in discover_local_sdks_with_shell(&roots, eco, shell_path.as_deref()) {
                        if seen.insert(canonical_key(&disc.path)) {
                            installed.push(disc);
                        }
                    }
                    // Everything else the detector found (Homebrew JDKs, pyenv, fnm,
                    // Go toolchains…), so the SDK pickers list what "Detected" shows.
                    let api_eco: IdeEcosystemKind = eco.into();
                    for det in detected
                        .iter()
                        .filter(|d| d.ecosystem == api_eco && d.valid)
                    {
                        if seen.insert(canonical_key(&det.path)) {
                            installed.push(IdeInstalledSdk {
                                id: format!(
                                    "{}-{}-{}",
                                    eco.as_str(),
                                    det.source,
                                    installed.len() + 1
                                ),
                                version: det.version.clone(),
                                path: det.path.clone(),
                                source: det.source.clone(),
                                active: false,
                            });
                        }
                    }
                    mark_active_sdk(&mut installed, eco, env_clone.as_ref());
                    let downloadable = catalogs
                        .get(&eco)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(Into::into)
                        .collect();
                    IdeToolchainEcosystem {
                        ecosystem: eco.into(),
                        display_name: eco.display_name().to_string(),
                        installed,
                        lsp_status: discover_local_lsp_with_shell(
                            &roots,
                            eco,
                            shell_path.as_deref(),
                        ),
                        dap_status: label_builtin_dap(
                            eco,
                            discover_local_dap_with_shell(&roots, eco, shell_path.as_deref()),
                        ),
                        downloadable,
                    }
                })
                .collect::<Vec<_>>();
            (ecosystems, detected)
        })
        .await
        .map_err(|e| Error::Internal(format!("toolchain discovery task: {e}")))?;

        Ok(IdeToolchainsResp {
            target_os: platform.os.as_str().to_string(),
            target_arch: platform.arch.as_str().to_string(),
            ecosystems,
            detected,
            jobs: self.list_jobs(),
        })
    }

    async fn list_ssh(
        &self,
        targets: &TargetManager,
        resolved: &ResolvedTarget,
        active_env: Option<&IdeProjectEnvironment>,
        mut custom_sdks: HashMap<EcosystemKind, Vec<IdeInstalledSdk>>,
        include_catalog: bool,
    ) -> Result<IdeToolchainsResp> {
        let out = targets
            .exec_shell(
                resolved,
                REMOTE_PROBE_SCRIPT,
                Some("/"),
                &BTreeMap::new(),
                Duration::from_secs(20),
            )
            .await?;
        let (platform, mut eco_map) = parse_ssh_probe_output(&out.stdout);
        let catalogs = if include_catalog {
            self.catalog.fetch_all(platform).await
        } else {
            self.catalog.cached_or_curated(platform)
        };

        let ecosystems = EcosystemKind::ALL
            .into_iter()
            .map(|eco| {
                let probed = eco_map.remove(&eco).unwrap_or_default();
                let mut installed = custom_sdks.remove(&eco).unwrap_or_default();
                let mut seen: BTreeSet<String> = installed.iter().map(|s| s.path.clone()).collect();
                for disc in probed.installed {
                    if seen.insert(disc.path.clone()) {
                        installed.push(disc);
                    }
                }
                mark_active_sdk(&mut installed, eco, active_env);
                IdeToolchainEcosystem {
                    ecosystem: eco.into(),
                    display_name: eco.display_name().to_string(),
                    installed,
                    lsp_status: probed.lsp.unwrap_or_else(|| empty_tool(eco.lsp_name())),
                    dap_status: probed.dap.unwrap_or_else(|| empty_tool(eco.dap_name())),
                    downloadable: catalogs
                        .get(&eco)
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                }
            })
            .collect();

        Ok(IdeToolchainsResp {
            target_os: platform.os.as_str().to_string(),
            target_arch: platform.arch.as_str().to_string(),
            ecosystems,
            detected: Vec::new(),
            jobs: self.list_jobs(),
        })
    }
}

#[cfg(test)]
mod tests;
