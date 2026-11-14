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
mod local_registry;

use self::detected::scan_detected_sdks;
use self::discovery::{
    collect_managed_bin_dirs, discover_local_dap, discover_local_lsp, discover_local_sdks,
    empty_tool, mark_active_sdk, REMOTE_PROBE_SCRIPT,
};
pub(crate) use self::discovery::{
    find_executable_in, is_executable_file, parse_ssh_probe_output, read_subdirs,
    resolve_java_home_dir, system_search_dirs,
};
use self::local_registry::{
    host_key_for_target, load_registered_local_sdks, normalize_and_validate_local_sdk,
};

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use sqlx::Row;

use crate::target::{ResolvedTarget, TargetManager};
use crate::toolchains_catalog::{
    is_safe_version_token, resolve_sdk_download, CatalogClient, CatalogSdkEntry, EcosystemKind,
    TargetPlatform,
};
use crate::types::{
    IdeEcosystemKind, IdeInstallComponent, IdeInstallJob, IdeInstallJobStatus,
    IdeInstallToolchainReq, IdeInstalledSdk, IdeProjectEnvironment, IdePythonVenvInfo,
    IdeRegisterLocalSdkReq, IdeSshSpec, IdeTargetKind, IdeTargetSpec, IdeToolchainEcosystem,
    IdeToolchainPythonVenvReq, IdeToolchainPythonVenvResp, IdeToolchainsResp,
};
use crate::IdeCtx;

use self::installer::{
    build_remote_install_script, install_local_dap, install_local_lsp, install_local_sdk,
    run_ssh_streaming,
};

const MAX_JOB_LOGS: usize = 400;
const MAX_FINISHED_JOBS: usize = 16;

/// Manages SDK/LSP/DAP discovery and background installation jobs across
/// local and remote SSH targets.
#[derive(Clone)]
pub struct ToolchainManager {
    data_dir: PathBuf,
    catalog: Arc<CatalogClient>,
    jobs: Arc<DashMap<String, IdeInstallJob>>,
    finished_order: Arc<Mutex<VecDeque<String>>>,
}

impl ToolchainManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            catalog: Arc::new(CatalogClient::new()),
            jobs: Arc::new(DashMap::new()),
            finished_order: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn toolchains_dir(&self) -> PathBuf {
        self.data_dir.join("toolchains")
    }

    pub fn list_jobs(&self) -> Vec<IdeInstallJob> {
        let mut jobs: Vec<IdeInstallJob> = self.jobs.iter().map(|e| e.value().clone()).collect();
        jobs.sort_by(|a, b| b.id.cmp(&a.id));
        jobs
    }

    pub fn get_job(&self, job_id: &str) -> Result<IdeInstallJob> {
        self.jobs
            .get(job_id)
            .map(|e| e.value().clone())
            .ok_or_else(|| Error::NotFound(format!("toolchain install job '{job_id}'")))
    }

    /// Return all existing Otto-managed `bin` directories on the local host.
    pub fn extra_local_bin_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for root in self.local_toolchain_roots() {
            collect_managed_bin_dirs(&root, &mut out);
        }
        out
    }

    /// Resolve the best available local LSP executable path for `ecosystem`.
    pub fn resolve_local_lsp_path(&self, ecosystem: IdeEcosystemKind) -> Option<PathBuf> {
        discover_local_lsp(
            &self.local_toolchain_roots(),
            EcosystemKind::from(ecosystem),
        )
        .path
        .map(PathBuf::from)
    }

    /// Resolve the best available local DAP adapter path for `ecosystem`.
    pub fn resolve_local_dap_path(&self, ecosystem: IdeEcosystemKind) -> Option<PathBuf> {
        discover_local_dap(
            &self.local_toolchain_roots(),
            EcosystemKind::from(ecosystem),
        )
        .path
        .map(PathBuf::from)
    }

    fn local_toolchain_roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.toolchains_dir()];
        if !otto_core::e2e::isolated() {
            if let Some(dot_otto) = dirs::home_dir().map(|h| h.join(".otto/toolchains")) {
                if dot_otto != roots[0] {
                    roots.push(dot_otto);
                }
            }
        }
        roots
    }

    /// Host-context helper for `GET /ide/toolchains` (fast zero-HTTPS default).
    pub async fn list_for_ctx<C: IdeCtx>(
        &self,
        ctx: &C,
        user: &User,
        target: &IdeTargetSpec,
    ) -> Result<IdeToolchainsResp> {
        let ssh = resolve_optional_ssh(ctx, user, target).await?;
        let active_env = ctx
            .ide()
            .environments
            .get_project_env(&ctx.pool(), target)
            .await
            .ok();
        self.list_toolchains_with_opts(
            Some(&ctx.pool()),
            &ctx.ide().targets,
            target,
            ssh.as_ref(),
            active_env.as_ref(),
            false,
        )
        .await
    }

    /// Host-context helper for `POST /ide/toolchains/install`.
    pub async fn install_for_ctx<C: IdeCtx>(
        &self,
        ctx: &C,
        user: &User,
        req: IdeInstallToolchainReq,
    ) -> Result<IdeInstallJob> {
        let ssh = resolve_optional_ssh(ctx, user, &req.target).await?;
        self.start_install(&ctx.ide().targets, req, ssh).await
    }

    /// Discover installed & curated/cached downloadable SDKs, LSPs, and DAPs on `target`
    /// without making outbound HTTPS catalog calls (`<5ms`).
    pub async fn list_toolchains(
        &self,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
        active_env: Option<&IdeProjectEnvironment>,
    ) -> Result<IdeToolchainsResp> {
        self.list_toolchains_with_opts(None, targets, target, ssh, active_env, false)
            .await
    }

    /// Discover installed & downloadable SDKs, LSPs, and DAPs on `target`.
    ///
    /// When `include_catalog == false`, returns curated + warm-cached catalog entries
    /// immediately (`<5ms`, zero network I/O). When `include_catalog == true`, fetches
    /// official upstream release catalogs (`go.dev/dl/?mode=json&include=all`, Temurin,
    /// Node.js, Python, Rust) with a bounded timeout and offline fallback.
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
            Some(db) => load_registered_local_sdks(db, &host_key_for_target(target)).await,
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
        }
        Ok(())
    }

    /// Create a per-project or per-module Python virtualenv on demand (`POST /ide/toolchains/python-venv`).
    pub async fn create_python_venv(
        &self,
        req: &IdeToolchainPythonVenvReq,
        is_root_user: bool,
    ) -> Result<IdeToolchainPythonVenvResp> {
        let root_str = req
            .root
            .as_deref()
            .or_else(|| req.target.as_ref().map(|t| t.root.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Invalid("root or target.root is required".into()))?;
        let canon_root = crate::target::canonicalize_local_root_dir(root_str, is_root_user)?;
        let rel_or_abs = req
            .path
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".venv");
        if rel_or_abs.contains("..") {
            return Err(Error::Invalid("venv path must not contain '..'".into()));
        }
        let raw_venv_dir = if std::path::Path::new(rel_or_abs).is_absolute() {
            PathBuf::from(rel_or_abs)
        } else {
            canon_root.join(rel_or_abs)
        };
        let venv_dir = otto_core::secret_paths::canonicalize_prospective(&raw_venv_dir)
            .ok_or_else(|| Error::Invalid("invalid venv path".into()))?;
        if !venv_dir.starts_with(&canon_root) {
            return Err(Error::Invalid("venv path must stay within root".into()));
        }
        if !is_root_user {
            crate::target::enforce_non_root_path(&canon_root)?;
            crate::target::enforce_non_root_path(&venv_dir)?;
        }
        let base_py = req
            .base_python
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| find_executable_in(&system_search_dirs(), &["python3", "python"]))
            .ok_or_else(|| Error::NotFound("python3 not found on host".into()))?;

        let out = tokio::process::Command::new(&base_py)
            .arg("-m")
            .arg("venv")
            .arg(&venv_dir)
            .current_dir(&canon_root)
            .output()
            .await
            .map_err(|e| Error::Internal(format!("spawn python3 -m venv: {e}")))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Invalid(format!(
                "python3 -m venv failed: {}",
                stderr.trim()
            )));
        }
        let interp = if venv_dir.join("bin/python3").is_file() {
            venv_dir.join("bin/python3")
        } else {
            venv_dir.join("bin/python")
        };
        let version =
            detected::read_pyvenv_cfg_version(&venv_dir).unwrap_or_else(|| "3".to_string());
        Ok(IdeToolchainPythonVenvResp {
            ok: true,
            venv: IdePythonVenvInfo {
                path: venv_dir.to_string_lossy().into_owned(),
                version,
                kind: "venv".into(),
            },
            interpreter_path: interp.to_string_lossy().into_owned(),
        })
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

        let (ecosystems, detected) = tokio::task::spawn_blocking(move || {
            let detected = scan_detected_sdks(&roots, &project_roots);
            let ecosystems = EcosystemKind::ALL
                .into_iter()
                .map(|eco| {
                    let mut installed = custom_sdks.remove(&eco).unwrap_or_default();
                    let mut seen: BTreeSet<String> =
                        installed.iter().map(|s| s.path.clone()).collect();
                    if eco == EcosystemKind::Python {
                        for det in &detected {
                            if det.ecosystem == IdeEcosystemKind::Python
                                && matches!(det.source.as_str(), "venv" | "poetry" | "conda")
                                && seen.insert(det.bin_path.clone())
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
                    for disc in discover_local_sdks(&roots, eco) {
                        if seen.insert(disc.path.clone()) {
                            installed.push(disc);
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
                        lsp_status: discover_local_lsp(&roots, eco),
                        dap_status: discover_local_dap(&roots, eco),
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

    /// Spawn a background SDK / LSP / DAP installation job for `req.target`.
    pub async fn start_install(
        &self,
        targets: &TargetManager,
        req: IdeInstallToolchainReq,
        ssh: Option<IdeSshSpec>,
    ) -> Result<IdeInstallJob> {
        req.target.validate()?;
        if let Some(v) = req.version.as_deref().filter(|s| !s.trim().is_empty()) {
            if !is_safe_version_token(v) {
                return Err(Error::Invalid(format!(
                    "invalid toolchain version '{}'",
                    v.trim()
                )));
            }
        }
        let platform = match req.target.kind {
            IdeTargetKind::Local => TargetPlatform::host(),
            IdeTargetKind::Ssh => {
                let spec = ssh.as_ref().ok_or_else(|| {
                    Error::Invalid("SSH connection spec required for remote install".into())
                })?;
                let resolved =
                    targets.resolve_with_ssh(&req.target, "ide-toolchains", spec.clone())?;
                let info = targets.detect_platform(&resolved).await?;
                TargetPlatform::from_uname(&info.os, &info.arch)
                    .unwrap_or_else(TargetPlatform::host)
            }
        };

        let eco = EcosystemKind::from(req.ecosystem);
        let catalogs = self.catalog.fetch_all(platform).await;
        let eco_cat = catalogs.get(&eco).cloned().unwrap_or_default();
        let sdk_entry = if matches!(
            req.component,
            IdeInstallComponent::Sdk | IdeInstallComponent::All
        ) {
            Some(
                resolve_sdk_download(eco, req.version.as_deref(), platform, &eco_cat)
                    .map_err(Error::Invalid)?,
            )
        } else {
            None
        };

        let job_id = format!("tcjob_{}", new_id());
        let job = IdeInstallJob {
            id: job_id.clone(),
            ecosystem: req.ecosystem,
            component: req.component,
            version: sdk_entry
                .as_ref()
                .map(|e| e.version.clone())
                .or_else(|| req.version.clone()),
            target_os: platform.os.as_str().to_string(),
            target_arch: platform.arch.as_str().to_string(),
            status: IdeInstallJobStatus::Running,
            progress_pct: 5,
            log_lines: vec![format!(
                "Starting {} ({:?}) install for {}-{}...",
                eco.display_name(),
                req.component,
                platform.os.as_str(),
                platform.arch.as_str()
            )],
            installed_path: None,
            error: None,
        };
        self.jobs.insert(job_id.clone(), job.clone());

        let mgr = self.clone();
        tokio::spawn(async move {
            let outcome = match req.target.kind {
                IdeTargetKind::Local => {
                    mgr.run_local_install(&job_id, eco, req.component, platform, sdk_entry)
                        .await
                }
                IdeTargetKind::Ssh => match ssh {
                    Some(spec) => {
                        mgr.run_ssh_install(&job_id, &spec, eco, req.component, platform, sdk_entry)
                            .await
                    }
                    None => Err("missing SSH connection specification".to_string()),
                },
            };
            mgr.finish_job(&job_id, outcome);
        });

        Ok(job)
    }

    pub(super) fn log_progress(&self, job_id: &str, pct: u8, line: impl Into<String>) {
        if let Some(mut entry) = self.jobs.get_mut(job_id) {
            entry.progress_pct = pct.min(99);
            let text = line.into();
            if !text.trim().is_empty() {
                if entry.log_lines.len() >= MAX_JOB_LOGS {
                    entry.log_lines.remove(0);
                }
                entry.log_lines.push(text);
            }
        }
    }

    fn finish_job(&self, job_id: &str, outcome: std::result::Result<String, String>) {
        if let Some(mut entry) = self.jobs.get_mut(job_id) {
            match outcome {
                Ok(path) => {
                    entry.status = IdeInstallJobStatus::Done;
                    entry.progress_pct = 100;
                    entry.installed_path = Some(path.clone());
                    entry.log_lines.push(format!("Completed cleanly: {path}"));
                }
                Err(err) => {
                    entry.status = IdeInstallJobStatus::Error;
                    entry.error = Some(err.clone());
                    entry.log_lines.push(format!("ERROR: {err}"));
                }
            }
            entry.log_lines.shrink_to_fit();
        }
        let mut evicted = Vec::new();
        {
            let mut order = self
                .finished_order
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            order.push_back(job_id.to_string());
            while order.len() > MAX_FINISHED_JOBS {
                if let Some(old_id) = order.pop_front() {
                    evicted.push(old_id);
                }
            }
        }
        for old_id in evicted {
            self.jobs.remove(&old_id);
        }
    }

    async fn run_local_install(
        &self,
        job_id: &str,
        eco: EcosystemKind,
        component: IdeInstallComponent,
        platform: TargetPlatform,
        sdk_entry: Option<CatalogSdkEntry>,
    ) -> std::result::Result<String, String> {
        let root = self.toolchains_dir();
        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| format!("create toolchains dir: {e}"))?;
        let mut final_path = root.join(eco.as_str()).to_string_lossy().into_owned();
        if matches!(
            component,
            IdeInstallComponent::Sdk | IdeInstallComponent::All
        ) {
            let entry = sdk_entry.ok_or_else(|| "missing SDK catalog entry".to_string())?;
            self.log_progress(job_id, 15, format!("Downloading {}...", entry.label));
            final_path = install_local_sdk(self, job_id, &root, eco, &entry).await?;
        }
        if matches!(
            component,
            IdeInstallComponent::Lsp | IdeInstallComponent::All
        ) {
            self.log_progress(
                job_id,
                65,
                format!("Installing LSP ({})...", eco.lsp_name()),
            );
            let p = install_local_lsp(self, job_id, &root, eco, platform).await?;
            if component == IdeInstallComponent::Lsp {
                final_path = p;
            }
        }
        if matches!(
            component,
            IdeInstallComponent::Dap | IdeInstallComponent::All
        ) {
            self.log_progress(
                job_id,
                85,
                format!("Installing DAP ({})...", eco.dap_name()),
            );
            let p = install_local_dap(self, job_id, &root, eco, platform).await?;
            if component == IdeInstallComponent::Dap {
                final_path = p;
            }
        }
        Ok(final_path)
    }

    async fn run_ssh_install(
        &self,
        job_id: &str,
        ssh: &IdeSshSpec,
        eco: EcosystemKind,
        component: IdeInstallComponent,
        platform: TargetPlatform,
        sdk_entry: Option<CatalogSdkEntry>,
    ) -> std::result::Result<String, String> {
        let script = build_remote_install_script(eco, component, platform, sdk_entry.as_ref())?;
        self.log_progress(
            job_id,
            20,
            format!("Running remote installer on {}...", ssh.host),
        );
        run_ssh_streaming(ssh, &script, |line| self.log_progress(job_id, 60, line)).await
    }
}

pub(crate) async fn resolve_optional_ssh<C: IdeCtx>(
    ctx: &C,
    user: &User,
    target: &IdeTargetSpec,
) -> Result<Option<IdeSshSpec>> {
    if target.kind == IdeTargetKind::Ssh {
        let conn_id = target
            .connection_id
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::Invalid("SSH target requires non-empty connection_id".into()))?;
        Ok(Some(ctx.resolve_ssh_connection(user, conn_id).await?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
