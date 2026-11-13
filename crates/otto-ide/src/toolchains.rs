//! OS/arch-aware SDK, Language Server (LSP), and Debug Adapter (DAP) manager
//! and background installer (`ToolchainManager`).
//!
//! Works transparently on both `IdeTargetKind::Local` (storing managed SDKs
//! under `<data_dir>/toolchains/<ecosystem>/...`) and `IdeTargetKind::Ssh`
//! (probing remote `uname -s -m` and installing into `$HOME/.otto/toolchains/...`
//! via [`TargetManager`]).

mod installer;

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::{Path, PathBuf};
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
    TargetPlatform, JAVA_DEBUG_JAR_NAME,
};
use crate::types::{
    IdeEcosystemKind, IdeInstallComponent, IdeInstallJob, IdeInstallJobStatus,
    IdeInstallToolchainReq, IdeInstalledSdk, IdeProjectEnvironment, IdeRegisterLocalSdkReq,
    IdeSshSpec, IdeTargetKind, IdeTargetSpec, IdeToolBinaryStatus, IdeToolchainEcosystem,
    IdeToolchainsResp,
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
                self.list_local(active_env, custom_sdks, include_catalog)
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

    async fn list_local(
        &self,
        active_env: Option<&IdeProjectEnvironment>,
        mut custom_sdks: HashMap<EcosystemKind, Vec<IdeInstalledSdk>>,
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

        let ecosystems = tokio::task::spawn_blocking(move || {
            EcosystemKind::ALL
                .into_iter()
                .map(|eco| {
                    let mut installed = custom_sdks.remove(&eco).unwrap_or_default();
                    let mut seen: BTreeSet<String> =
                        installed.iter().map(|s| s.path.clone()).collect();
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
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| Error::Internal(format!("toolchain discovery task: {e}")))?;

        Ok(IdeToolchainsResp {
            target_os: platform.os.as_str().to_string(),
            target_arch: platform.arch.as_str().to_string(),
            ecosystems,
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

fn empty_tool(name: &str) -> IdeToolBinaryStatus {
    IdeToolBinaryStatus {
        name: name.to_string(),
        available: false,
        path: None,
    }
}

fn host_key_for_target(target: &IdeTargetSpec) -> String {
    match target.kind {
        IdeTargetKind::Local => "local".into(),
        IdeTargetKind::Ssh => format!(
            "ssh:{}",
            target.connection_id.as_deref().unwrap_or("default")
        ),
    }
}

async fn load_registered_local_sdks(
    pool: &DbPool,
    host_key: &str,
) -> HashMap<EcosystemKind, Vec<IdeInstalledSdk>> {
    let rows = match sqlx::query(
        r#"SELECT id, ecosystem, version, path
           FROM ide_local_sdks
           WHERE host_key = ?
           ORDER BY created_at DESC"#,
    )
    .bind(host_key)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return HashMap::new(),
    };

    let mut out: HashMap<EcosystemKind, Vec<IdeInstalledSdk>> = HashMap::new();
    for row in rows {
        let eco_s: String = row.try_get("ecosystem").unwrap_or_default();
        let Some(eco) = EcosystemKind::parse(&eco_s) else {
            continue;
        };
        let path: String = row.try_get("path").unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        if host_key == "local" && !Path::new(&path).exists() {
            continue;
        }
        let id: String = row.try_get("id").unwrap_or_default();
        let version: String = row.try_get("version").unwrap_or_else(|_| "custom".into());
        out.entry(eco).or_default().push(IdeInstalledSdk {
            id,
            version,
            path,
            source: "local".into(),
            active: false,
        });
    }
    out
}

fn normalize_and_validate_local_sdk(
    target: &IdeTargetSpec,
    eco: EcosystemKind,
    raw_path: &str,
) -> Result<(String, String)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("SDK path must not be empty".into()));
    }
    if !trimmed.starts_with('/') {
        return Err(Error::Invalid("SDK path must be an absolute path".into()));
    }

    if target.kind == IdeTargetKind::Ssh {
        let clean = trimmed.trim_end_matches('/').to_string();
        let ver = dir_name_version(Path::new(&clean));
        return Ok((clean, ver));
    }

    let canon = std::fs::canonicalize(trimmed)
        .map_err(|e| Error::Invalid(format!("SDK path '{trimmed}' does not exist: {e}")))?;
    let canon_str = canon.to_string_lossy();
    if canon_str == "/" || canon_str == "/etc" || canon_str == "/private/etc" {
        return Err(Error::Forbidden(
            "refusing to register root or system configuration directory as SDK".into(),
        ));
    }
    if let Some(home) = dirs::home_dir() {
        let hs = home.to_string_lossy();
        for blocked_rel in [".ssh", ".aws", ".gnupg", ".kube"] {
            let blocked = format!("{}/{blocked_rel}", hs.trim_end_matches('/'));
            if canon_str == blocked || canon_str.starts_with(&format!("{blocked}/")) {
                return Err(Error::Forbidden(
                    "refusing to register sensitive credential directory as SDK".into(),
                ));
            }
        }
    }

    match eco {
        EcosystemKind::Java => {
            let jh = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else {
                resolve_java_home_dir(&canon).ok_or_else(|| {
                    Error::Invalid(format!(
                        "'{}' does not contain a valid JDK (expected bin/java or Contents/Home/bin/java)",
                        canon.display()
                    ))
                })?
            };
            let ver = read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&jh));
            Ok((jh.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Go => {
            let gr = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else if canon.join("go/bin/go").is_file() {
                canon.join("go")
            } else {
                canon
            };
            if !gr.join("bin/go").is_file() {
                return Err(Error::Invalid(format!(
                    "'{}' does not contain bin/go",
                    gr.display()
                )));
            }
            let ver = read_go_root_version(&gr).unwrap_or_else(|| dir_name_version(&gr));
            Ok((gr.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Rust => {
            let root = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else {
                canon
            };
            if !root.join("bin/rustc").is_file() && !root.join("bin/cargo").is_file() {
                return Err(Error::Invalid(format!(
                    "'{}' does not contain bin/rustc or bin/cargo",
                    root.display()
                )));
            }
            let ver = rust_toolchain_label(&root);
            Ok((root.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Node => {
            let root = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else {
                canon
            };
            if !root.join("bin/node").is_file() && !is_executable_file(&root) {
                return Err(Error::Invalid(format!(
                    "'{}' does not contain bin/node",
                    root.display()
                )));
            }
            let ver = dir_name_version(&root);
            Ok((root.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Python => {
            let interp = if canon.is_file() && is_executable_file(&canon) {
                canon
            } else if canon.join("bin/python3").is_file() {
                canon.join("bin/python3")
            } else if canon.join("bin/python").is_file() {
                canon.join("bin/python")
            } else {
                return Err(Error::Invalid(format!(
                    "'{}' is neither a python executable nor a virtualenv/SDK with bin/python3",
                    canon.display()
                )));
            };
            let ver = interp
                .parent()
                .and_then(|b| b.parent())
                .map(python_dir_version)
                .unwrap_or_else(|| "custom".into());
            Ok((interp.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Shell | EcosystemKind::Json => {
            if !canon.is_dir() && !is_executable_file(&canon) {
                return Err(Error::Invalid(format!(
                    "'{}' must be an executable binary or tool directory",
                    canon.display()
                )));
            }
            let ver = dir_name_version(&canon);
            Ok((canon.to_string_lossy().into_owned(), ver))
        }
    }
}

// ── Local Filesystem Discovery (runs off-worker via spawn_blocking) ──────────

fn collect_managed_bin_dirs(tc_root: &Path, out: &mut Vec<PathBuf>) {
    if !tc_root.is_dir() {
        return;
    }
    for rel in [
        "java/lsp/bin",
        "java/current/bin",
        "go/bin",
        "rust/bin",
        "rust/cargo/bin",
        "rust/dap/extension/adapter",
        "rust/codelldb/adapter",
        "node/current/bin",
        "node/lsp/node_modules/.bin",
        "python/bin",
        "python/current/bin",
        "python/dap/bin",
        "shell/bin",
        "shell/lsp/node_modules/.bin",
        "json/bin",
        "json/lsp/node_modules/.bin",
    ] {
        let c = tc_root.join(rel);
        if c.is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
    for sub in ["java/jdks", "go/sdks", "node/sdks", "python/interpreters"] {
        for p in read_subdirs(&tc_root.join(sub)) {
            for rel in ["bin", "Contents/Home/bin", "go/bin"] {
                let b = p.join(rel);
                if b.is_dir() && !out.contains(&b) {
                    out.push(b);
                }
            }
        }
    }
}

pub(crate) fn system_search_dirs() -> Vec<PathBuf> {
    let mut dirs = std::env::var_os("PATH")
        .map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .unwrap_or_default();
    if let Some(shell_path) = crate::environments::cached_shell_env_snapshot().get("PATH") {
        for p in std::env::split_paths(shell_path) {
            if !dirs.contains(&p) {
                dirs.push(p);
            }
        }
    }
    for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        let p = PathBuf::from(extra);
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for rel in [".cargo/bin", "go/bin", ".local/bin"] {
                let p = home.join(rel);
                if !dirs.contains(&p) {
                    dirs.push(p);
                }
            }
        }
    }
    dirs
}

pub(crate) fn find_executable_in(dirs: &[PathBuf], names: &[&str]) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find(|cand| is_executable_file(cand))
}

pub(crate) fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn discover_local_sdks(roots: &[PathBuf], eco: EcosystemKind) -> Vec<IdeInstalledSdk> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        scan_otto_sdks(root, eco, &mut seen, &mut out);
    }
    scan_system_sdks(eco, &mut seen, &mut out);
    out
}

fn scan_otto_sdks(
    root: &Path,
    eco: EcosystemKind,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<IdeInstalledSdk>,
) {
    match eco {
        EcosystemKind::Java => {
            for dir in read_subdirs(&root.join("java/jdks")) {
                if let Some(jh) = resolve_java_home_dir(&dir) {
                    let v =
                        read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&dir));
                    push_sdk(out, seen, "java", &v, &jh, "otto");
                }
            }
        }
        EcosystemKind::Go => {
            for dir in read_subdirs(&root.join("go/sdks")) {
                let gr = if dir.join("go/bin/go").is_file() {
                    dir.join("go")
                } else {
                    dir.clone()
                };
                if gr.join("bin/go").is_file() {
                    let v = read_go_root_version(&gr).unwrap_or_else(|| dir_name_version(&dir));
                    push_sdk(out, seen, "go", &v, &gr, "otto");
                }
            }
        }
        EcosystemKind::Rust => {
            for dir in read_subdirs(&root.join("rust/rustup/toolchains")) {
                if dir.join("bin/rustc").is_file() {
                    push_sdk(out, seen, "rust", &rust_toolchain_label(&dir), &dir, "otto");
                }
            }
        }
        EcosystemKind::Node => {
            for dir in read_subdirs(&root.join("node/sdks")) {
                if dir.join("bin/node").is_file() {
                    push_sdk(out, seen, "node", &dir_name_version(&dir), &dir, "otto");
                }
            }
        }
        EcosystemKind::Python => {
            for dir in read_subdirs(&root.join("python/interpreters")) {
                let py = dir.join("bin/python3");
                if py.is_file() {
                    push_sdk(out, seen, "python", &python_dir_version(&dir), &py, "otto");
                }
            }
        }
        EcosystemKind::Shell => {
            let shfmt = root.join("shell/bin/shfmt");
            if is_executable_file(&shfmt) {
                push_sdk(out, seen, "shell", "3.10.0", &shfmt, "otto");
            }
        }
        EcosystemKind::Json => {
            let jq = root.join("json/bin/jq");
            if is_executable_file(&jq) {
                push_sdk(out, seen, "json", "1.7.1", &jq, "otto");
            }
        }
    }
}

fn scan_system_sdks(
    eco: EcosystemKind,
    seen: &mut BTreeSet<String>,
    out: &mut Vec<IdeInstalledSdk>,
) {
    let search = system_search_dirs();
    let home = dirs::home_dir();
    match eco {
        EcosystemKind::Java => {
            let mut jvm_dirs = vec![
                PathBuf::from("/Library/Java/JavaVirtualMachines"),
                PathBuf::from("/usr/lib/jvm"),
            ];
            if let Some(h) = &home {
                jvm_dirs.push(h.join("Library/Java/JavaVirtualMachines"));
                jvm_dirs.push(h.join(".sdkman/candidates/java"));
            }
            for dir in jvm_dirs.iter().flat_map(|p| read_subdirs(p)) {
                if let Some(jh) = resolve_java_home_dir(&dir) {
                    let v =
                        read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&dir));
                    push_sdk(out, seen, "java", &v, &jh, "system");
                }
            }
            if let Some(jh) = find_executable_in(&search, &["java"])
                .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
                .filter(|jh| jh != Path::new("/usr"))
            {
                let v = read_java_release_version(&jh).unwrap_or_else(|| "system".into());
                push_sdk(out, seen, "java", &v, &jh, "system");
            }
        }
        EcosystemKind::Go => {
            for cand in ["/opt/homebrew/opt/go/libexec", "/usr/local/go"] {
                let p = PathBuf::from(cand);
                if p.join("bin/go").is_file() {
                    let v = read_go_root_version(&p).unwrap_or_else(|| "system".into());
                    push_sdk(out, seen, "go", &v, &p, "system");
                }
            }
            if let Some(gr) = find_executable_in(&search, &["go"])
                .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
            {
                let v = read_go_root_version(&gr).unwrap_or_else(|| "system".into());
                push_sdk(out, seen, "go", &v, &gr, "system");
            }
        }
        EcosystemKind::Rust => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".rustup/toolchains")) {
                    if dir.join("bin/rustc").is_file() {
                        push_sdk(
                            out,
                            seen,
                            "rust",
                            &rust_toolchain_label(&dir),
                            &dir,
                            "system",
                        );
                    }
                }
            }
            if out.is_empty() {
                if let Some(root) = find_executable_in(&search, &["rustc"])
                    .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
                {
                    push_sdk(out, seen, "rust", "system", &root, "system");
                }
            }
        }
        EcosystemKind::Node => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".nvm/versions/node")) {
                    if dir.join("bin/node").is_file() {
                        push_sdk(out, seen, "node", &dir_name_version(&dir), &dir, "system");
                    }
                }
            }
            if let Some(prefix) = find_executable_in(&search, &["node"])
                .and_then(|b| b.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
            {
                push_sdk(out, seen, "node", "system", &prefix, "system");
            }
        }
        EcosystemKind::Python => {
            if let Some(h) = &home {
                for dir in read_subdirs(&h.join(".local/share/uv/python")) {
                    let py = dir.join("bin/python3");
                    if py.is_file() {
                        push_sdk(
                            out,
                            seen,
                            "python",
                            &python_dir_version(&dir),
                            &py,
                            "system",
                        );
                    }
                }
            }
            if let Some(py_bin) = find_executable_in(&search, &["python3"]) {
                push_sdk(out, seen, "python", "3", &py_bin, "system");
            }
        }
        EcosystemKind::Shell => {
            if let Some(sh_bin) = find_executable_in(&search, &["bash", "zsh"]) {
                push_sdk(out, seen, "shell", "system", &sh_bin, "system");
            }
        }
        EcosystemKind::Json => {
            if let Some(jq_bin) = find_executable_in(&search, &["jq"]) {
                push_sdk(out, seen, "json", "system", &jq_bin, "system");
            }
        }
    }
}

fn discover_local_lsp(roots: &[PathBuf], eco: EcosystemKind) -> IdeToolBinaryStatus {
    let name = eco.lsp_name().to_string();
    let rels: &[&str] = match eco {
        EcosystemKind::Java => &["java/lsp/bin/jdtls"],
        EcosystemKind::Go => &["go/bin/gopls"],
        EcosystemKind::Rust => &["rust/bin/rust-analyzer", "rust/cargo/bin/rust-analyzer"],
        EcosystemKind::Node => &["node/lsp/node_modules/.bin/typescript-language-server"],
        EcosystemKind::Python => &["python/bin/pyright-langserver", "python/bin/pyright"],
        EcosystemKind::Shell => &["shell/lsp/node_modules/.bin/bash-language-server"],
        EcosystemKind::Json => &[
            "json/lsp/node_modules/.bin/vscode-json-language-server",
            "json/lsp/node_modules/.bin/vscode-json-languageserver",
        ],
    };
    for cand in roots
        .iter()
        .flat_map(|r| rels.iter().map(move |rel| r.join(rel)))
    {
        if cand.is_file() {
            return IdeToolBinaryStatus {
                name,
                available: true,
                path: Some(cand.to_string_lossy().into_owned()),
            };
        }
    }
    let sys_names: &[&str] = match eco {
        EcosystemKind::Java => &["jdtls"],
        EcosystemKind::Go => &["gopls"],
        EcosystemKind::Rust => &["rust-analyzer"],
        EcosystemKind::Node => &["typescript-language-server"],
        EcosystemKind::Python => &["pyright-langserver", "pyright"],
        EcosystemKind::Shell => &["bash-language-server"],
        EcosystemKind::Json => &["vscode-json-language-server", "vscode-json-languageserver"],
    };
    if let Some(sys) = find_executable_in(&system_search_dirs(), sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
        };
    }
    empty_tool(&name)
}

fn discover_local_dap(roots: &[PathBuf], eco: EcosystemKind) -> IdeToolBinaryStatus {
    let name = eco.dap_name().to_string();
    let java_rel = format!("java/dap/{JAVA_DEBUG_JAR_NAME}");
    let rels: Vec<&str> = match eco {
        EcosystemKind::Java => vec![java_rel.as_str()],
        EcosystemKind::Go => vec!["go/bin/dlv", "dlv/dlv"],
        EcosystemKind::Rust => vec![
            "rust/dap/extension/adapter/codelldb",
            "rust/codelldb/adapter/codelldb",
        ],
        EcosystemKind::Node => vec![
            "node/dap/js-debug/src/dapDebugServer.js",
            "node/js-debug/src/dapDebugServer.js",
        ],
        EcosystemKind::Python => vec!["python/dap/bin/python"],
        EcosystemKind::Shell => vec!["shell/bin/bashdb"],
        EcosystemKind::Json => vec!["json/bin/jq"],
    };
    for root in roots {
        for rel in &rels {
            let cand = root.join(rel);
            let ok = match eco {
                EcosystemKind::Java | EcosystemKind::Node | EcosystemKind::Python => cand.is_file(),
                EcosystemKind::Go
                | EcosystemKind::Rust
                | EcosystemKind::Shell
                | EcosystemKind::Json => is_executable_file(&cand),
            };
            if ok {
                return IdeToolBinaryStatus {
                    name,
                    available: true,
                    path: Some(cand.to_string_lossy().into_owned()),
                };
            }
        }
    }
    let sys_names: &[&str] = match eco {
        EcosystemKind::Java => &["java-debug"],
        EcosystemKind::Go => &["dlv"],
        EcosystemKind::Rust => &["codelldb", "lldb-dap"],
        EcosystemKind::Node => &["js-debug-dap"],
        EcosystemKind::Python => &["debugpy", "debugpy-adapter"],
        EcosystemKind::Shell => &["bashdb", "bash"],
        EcosystemKind::Json => &["jq"],
    };
    if let Some(sys) = find_executable_in(&system_search_dirs(), sys_names) {
        return IdeToolBinaryStatus {
            name,
            available: true,
            path: Some(sys.to_string_lossy().into_owned()),
        };
    }
    empty_tool(&name)
}

fn mark_active_sdk(
    installed: &mut [IdeInstalledSdk],
    eco: EcosystemKind,
    active_env: Option<&IdeProjectEnvironment>,
) {
    let chosen = active_env.and_then(|env| match eco {
        EcosystemKind::Java => env.java_home.as_deref(),
        EcosystemKind::Go => env.go_root.as_deref(),
        EcosystemKind::Rust => env.rust_toolchain.as_deref(),
        EcosystemKind::Node => env.node_home.as_deref(),
        EcosystemKind::Python => env.python_interpreter.as_deref(),
        EcosystemKind::Shell | EcosystemKind::Json => None,
    });
    if let Some(sel) = chosen.filter(|s| !s.trim().is_empty()) {
        if let Some(item) = installed
            .iter_mut()
            .find(|i| i.path == sel || i.version == sel)
        {
            item.active = true;
            return;
        }
    }
    if let Some(first) = installed.first_mut() {
        first.active = true;
    }
}

#[allow(clippy::disallowed_methods)] // bounded depth-1 toolchains dir read off async hot path
pub(crate) fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

pub(super) fn resolve_java_home_dir(dir: &Path) -> Option<PathBuf> {
    for base in std::iter::once(dir.to_path_buf()).chain(read_subdirs(dir)) {
        for cand in [base.join("Contents/Home"), base] {
            if cand.join("bin/java").is_file() {
                return Some(cand);
            }
        }
    }
    None
}

fn read_java_release_version(java_home: &Path) -> Option<String> {
    std::fs::read_to_string(java_home.join("release"))
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("JAVA_VERSION="))
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

fn read_go_root_version(goroot: &Path) -> Option<String> {
    let text = std::fs::read_to_string(goroot.join("VERSION")).ok()?;
    let first = text.lines().next()?.trim().trim_start_matches("go");
    (!first.is_empty()).then(|| first.to_string())
}

fn dir_name_version(dir: &Path) -> String {
    dir.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .trim_start_matches("jdk-")
        .trim_start_matches("go")
        .trim_start_matches('v')
        .to_string()
}

fn rust_toolchain_label(dir: &Path) -> String {
    let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or("stable");
    ["stable", "beta", "nightly"]
        .into_iter()
        .find(|ch| raw.starts_with(ch))
        .unwrap_or_else(|| raw.split('-').next().unwrap_or(raw))
        .to_string()
}

fn python_dir_version(dir: &Path) -> String {
    let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or("3");
    let stripped = raw.trim_start_matches("cpython-");
    stripped.split('-').next().unwrap_or(stripped).to_string()
}

fn push_sdk(
    out: &mut Vec<IdeInstalledSdk>,
    seen: &mut BTreeSet<String>,
    prefix: &str,
    version: &str,
    path: &Path,
    source: &str,
) {
    let path_str = path.to_string_lossy().into_owned();
    if seen.insert(path_str.clone()) {
        out.push(IdeInstalledSdk {
            id: format!("{prefix}-{source}-{}", out.len() + 1),
            version: version.to_string(),
            path: path_str,
            source: source.to_string(),
            active: false,
        });
    }
}

// ── Remote SSH Batched Probe ─────────────────────────────────────────────────

const REMOTE_PROBE_SCRIPT: &str = r#"
echo "UNAME:$(uname -s) $(uname -m)"
export PATH="$HOME/.otto/toolchains/go/bin:$HOME/.otto/toolchains/rust/bin:$HOME/.otto/toolchains/rust/cargo/bin:$HOME/.otto/toolchains/python/bin:$HOME/.otto/toolchains/shell/bin:$HOME/.otto/toolchains/json/bin:$HOME/.cargo/bin:$HOME/go/bin:/opt/homebrew/bin:/usr/local/go/bin:/usr/local/bin:$PATH"
for d in "$HOME"/.otto/toolchains/java/jdks/*; do [ -d "$d" ] && echo "SDK:java:otto:$d"; done
for d in "$HOME"/.otto/toolchains/go/sdks/*; do [ -d "$d" ] && echo "SDK:go:otto:$d"; done
for d in "$HOME"/.otto/toolchains/rust/rustup/toolchains/*; do [ -d "$d" ] && echo "SDK:rust:otto:$d"; done
for d in "$HOME"/.otto/toolchains/node/sdks/*; do [ -d "$d" ] && echo "SDK:node:otto:$d"; done
for d in "$HOME"/.otto/toolchains/python/interpreters/*; do [ -x "$d/bin/python3" ] && echo "SDK:python:otto:$d/bin/python3"; done
[ -x "$HOME/.otto/toolchains/shell/bin/shfmt" ] && echo "SDK:shell:otto:$HOME/.otto/toolchains/shell/bin/shfmt" || true
[ -x "$HOME/.otto/toolchains/json/bin/jq" ] && echo "SDK:json:otto:$HOME/.otto/toolchains/json/bin/jq" || true
for pair in "java:java" "go:go" "rust:rustc" "node:node" "python:python3" "shell:bash" "json:jq"; do
  e="${pair%%:*}"; b="${pair#*:}"
  command -v "$b" >/dev/null 2>&1 && echo "SDK:$e:system:$(command -v "$b")" || true
done
for pair in "java:jdtls:$HOME/.otto/toolchains/java/lsp/bin/jdtls" "go:gopls:$HOME/.otto/toolchains/go/bin/gopls" "rust:rust-analyzer:$HOME/.otto/toolchains/rust/bin/rust-analyzer" "node:typescript-language-server:$HOME/.otto/toolchains/node/lsp/node_modules/.bin/typescript-language-server" "python:pyright-langserver:$HOME/.otto/toolchains/python/bin/pyright-langserver" "shell:bash-language-server:$HOME/.otto/toolchains/shell/lsp/node_modules/.bin/bash-language-server" "json:vscode-json-language-server:$HOME/.otto/toolchains/json/lsp/node_modules/.bin/vscode-json-language-server"; do
  e="${pair%%:*}"; r="${pair#*:}"; b="${r%%:*}"; m="${r#*:}"
  if [ -x "$m" ]; then echo "LSP:$e:$m"; elif command -v "$b" >/dev/null 2>&1; then echo "LSP:$e:$(command -v "$b")"; fi
done
[ -f "$HOME/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar" ] && echo "DAP:java:$HOME/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar" || true
command -v dlv >/dev/null 2>&1 && echo "DAP:go:$(command -v dlv)" || true
if [ -x "$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb" ]; then echo "DAP:rust:$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb"; elif command -v codelldb >/dev/null 2>&1; then echo "DAP:rust:$(command -v codelldb)"; fi
[ -f "$HOME/.otto/toolchains/node/dap/js-debug/src/dapDebugServer.js" ] && echo "DAP:node:$HOME/.otto/toolchains/node/dap/js-debug/src/dapDebugServer.js" || true
[ -x "$HOME/.otto/toolchains/python/dap/bin/python" ] && echo "DAP:python:$HOME/.otto/toolchains/python/dap/bin/python" || true
if [ -x "$HOME/.otto/toolchains/shell/bin/bashdb" ]; then echo "DAP:shell:$HOME/.otto/toolchains/shell/bin/bashdb"; elif command -v bashdb >/dev/null 2>&1; then echo "DAP:shell:$(command -v bashdb)"; fi
if [ -x "$HOME/.otto/toolchains/json/bin/jq" ]; then echo "DAP:json:$HOME/.otto/toolchains/json/bin/jq"; elif command -v jq >/dev/null 2>&1; then echo "DAP:json:$(command -v jq)"; fi
"#;

#[derive(Default)]
pub(crate) struct ProbedRemoteEcosystem {
    pub installed: Vec<IdeInstalledSdk>,
    pub lsp: Option<IdeToolBinaryStatus>,
    pub dap: Option<IdeToolBinaryStatus>,
}

pub(crate) fn parse_ssh_probe_output(
    raw: &str,
) -> (
    TargetPlatform,
    HashMap<EcosystemKind, ProbedRemoteEcosystem>,
) {
    let mut platform = TargetPlatform::host();
    let mut map: HashMap<EcosystemKind, ProbedRemoteEcosystem> = HashMap::new();
    for line in raw.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("UNAME:") {
            if let Some(p) = TargetPlatform::from_combined_uname(rest) {
                platform = p;
            }
        } else if let Some(rest) = line.strip_prefix("SDK:") {
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            if parts.len() == 3 {
                if let Some(eco) = EcosystemKind::parse(parts[0]) {
                    let entry = map.entry(eco).or_default();
                    entry.installed.push(IdeInstalledSdk {
                        id: format!(
                            "{}-{}-{}",
                            eco.as_str(),
                            parts[1],
                            entry.installed.len() + 1
                        ),
                        version: dir_name_version(Path::new(parts[2])),
                        path: parts[2].to_string(),
                        source: parts[1].to_string(),
                        active: false,
                    });
                }
            }
        } else if let Some((eco_s, path)) =
            line.strip_prefix("LSP:").and_then(|r| r.split_once(':'))
        {
            if let Some(eco) = EcosystemKind::parse(eco_s) {
                map.entry(eco).or_default().lsp = Some(IdeToolBinaryStatus {
                    name: eco.lsp_name().to_string(),
                    available: true,
                    path: Some(path.to_string()),
                });
            }
        } else if let Some((eco_s, path)) =
            line.strip_prefix("DAP:").and_then(|r| r.split_once(':'))
        {
            if let Some(eco) = EcosystemKind::parse(eco_s) {
                map.entry(eco).or_default().dap = Some(IdeToolBinaryStatus {
                    name: eco.dap_name().to_string(),
                    available: true,
                    path: Some(path.to_string()),
                });
            }
        }
    }
    (platform, map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use installer::make_executable;

    #[test]
    fn discovers_otto_managed_sdks_lsps_daps_and_marks_active_selection() {
        let tmp = tempfile::tempdir().unwrap();
        let tc = tmp.path().join("toolchains");

        let jdk_home = tc.join("java/jdks/jdk-21/Contents/Home");
        std::fs::create_dir_all(jdk_home.join("bin")).unwrap();
        std::fs::write(jdk_home.join("bin/java"), "#!/bin/sh\n").unwrap();
        make_executable(&jdk_home.join("bin/java")).unwrap();
        std::fs::write(jdk_home.join("release"), "JAVA_VERSION=\"21.0.6\"\n").unwrap();

        let jdtls = tc.join("java/lsp/bin/jdtls");
        std::fs::create_dir_all(jdtls.parent().unwrap()).unwrap();
        std::fs::write(&jdtls, "#!/bin/sh\n").unwrap();
        make_executable(&jdtls).unwrap();

        let java_dbg = tc.join("java/dap").join(JAVA_DEBUG_JAR_NAME);
        std::fs::create_dir_all(java_dbg.parent().unwrap()).unwrap();
        std::fs::write(&java_dbg, b"jar").unwrap();

        let goroot = tc.join("go/sdks/1.24.1/go");
        std::fs::create_dir_all(goroot.join("bin")).unwrap();
        std::fs::write(goroot.join("bin/go"), "#!/bin/sh\n").unwrap();
        make_executable(&goroot.join("bin/go")).unwrap();
        std::fs::write(goroot.join("VERSION"), "go1.24.1\n").unwrap();

        let dlv = tc.join("go/bin/dlv");
        std::fs::create_dir_all(dlv.parent().unwrap()).unwrap();
        std::fs::write(&dlv, "#!/bin/sh\n").unwrap();
        make_executable(&dlv).unwrap();

        let shfmt = tc.join("shell/bin/shfmt");
        std::fs::create_dir_all(shfmt.parent().unwrap()).unwrap();
        std::fs::write(&shfmt, "#!/bin/sh\n").unwrap();
        make_executable(&shfmt).unwrap();

        let jq = tc.join("json/bin/jq");
        std::fs::create_dir_all(jq.parent().unwrap()).unwrap();
        std::fs::write(&jq, "#!/bin/sh\n").unwrap();
        make_executable(&jq).unwrap();

        let roots = vec![tc];
        let mut java_sdks = discover_local_sdks(&roots, EcosystemKind::Java);
        let env = IdeProjectEnvironment {
            java_home: Some(jdk_home.to_string_lossy().into_owned()),
            ..Default::default()
        };
        mark_active_sdk(&mut java_sdks, EcosystemKind::Java, Some(&env));
        let otto_java = java_sdks.iter().find(|s| s.source == "otto").unwrap();
        assert_eq!(otto_java.version, "21.0.6");
        assert!(otto_java.active);
        assert!(discover_local_lsp(&roots, EcosystemKind::Java).available);
        assert!(discover_local_dap(&roots, EcosystemKind::Java).available);

        let go_sdks = discover_local_sdks(&roots, EcosystemKind::Go);
        assert!(go_sdks
            .iter()
            .any(|s| s.source == "otto" && s.version == "1.24.1"));
        assert!(discover_local_dap(&roots, EcosystemKind::Go).available);

        let shell_sdks = discover_local_sdks(&roots, EcosystemKind::Shell);
        assert!(shell_sdks.iter().any(|s| s.source == "otto"));
        let json_sdks = discover_local_sdks(&roots, EcosystemKind::Json);
        assert!(json_sdks.iter().any(|s| s.source == "otto"));
    }

    #[test]
    fn parses_batched_ssh_probe_for_remote_linux_aarch64() {
        let sample = "UNAME:Linux aarch64\nSDK:go:otto:/home/dev/.otto/toolchains/go/sdks/1.24.1\nLSP:go:/home/dev/.otto/toolchains/go/bin/gopls\nDAP:go:/usr/local/bin/dlv\nLSP:shell:/usr/local/bin/bash-language-server\n";
        let (platform, map) = parse_ssh_probe_output(sample);
        assert_eq!(platform.os.as_str(), "linux");
        assert_eq!(platform.arch.as_str(), "aarch64");
        let go = map.get(&EcosystemKind::Go).unwrap();
        assert_eq!(go.installed[0].source, "otto");
        assert!(go.lsp.as_ref().unwrap().available);
        assert!(go.dap.as_ref().unwrap().available);
        let sh = map.get(&EcosystemKind::Shell).unwrap();
        assert!(sh.lsp.as_ref().unwrap().available);
    }

    #[tokio::test]
    async fn register_and_remove_local_sdk_round_trip_persists_and_surfaces_in_fast_list() {
        let tmp = tempfile::tempdir().unwrap();
        let db = otto_state::open(&tmp.path().join("ide_tc.db"))
            .await
            .unwrap();
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();

        let custom_go = tmp.path().join("custom-go-1.23.7");
        std::fs::create_dir_all(custom_go.join("bin")).unwrap();
        std::fs::write(custom_go.join("bin/go"), "#!/bin/sh\n").unwrap();
        make_executable(&custom_go.join("bin/go")).unwrap();
        std::fs::write(custom_go.join("VERSION"), "go1.23.7\n").unwrap();

        let mgr = ToolchainManager::new(tmp.path().join("data"));
        let targets = TargetManager::new();
        let target = IdeTargetSpec::local(ws.to_string_lossy().into_owned());

        let registered = mgr
            .register_local_sdk(
                &db,
                IdeRegisterLocalSdkReq {
                    target: target.clone(),
                    ecosystem: IdeEcosystemKind::Go,
                    path: custom_go.to_string_lossy().into_owned(),
                },
            )
            .await
            .unwrap();
        assert_eq!(registered.source, "local");
        assert_eq!(registered.version, "1.23.7");

        let t0 = std::time::Instant::now();
        let resp = mgr
            .list_toolchains_with_opts(Some(&db), &targets, &target, None, None, false)
            .await
            .unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "non-catalog list_toolchains must not make network requests (took {elapsed:?})"
        );
        assert_eq!(resp.ecosystems.len(), 7);
        let go_eco = resp
            .ecosystems
            .iter()
            .find(|e| e.ecosystem == IdeEcosystemKind::Go)
            .unwrap();
        assert!(go_eco
            .installed
            .iter()
            .any(|s| s.source == "local" && s.version == "1.23.7"));

        mgr.remove_local_sdk(&db, &target, None, Some(&registered.path))
            .await
            .unwrap();
        let resp_after = mgr
            .list_toolchains_with_opts(Some(&db), &targets, &target, None, None, false)
            .await
            .unwrap();
        let go_after = resp_after
            .ecosystems
            .iter()
            .find(|e| e.ecosystem == IdeEcosystemKind::Go)
            .unwrap();
        assert!(!go_after
            .installed
            .iter()
            .any(|s| s.source == "local" && s.version == "1.23.7"));
    }

    #[test]
    fn finished_install_jobs_are_bounded_to_prevent_memory_leak() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = ToolchainManager::new(tmp.path().to_path_buf());
        for i in 0..30 {
            let id = format!("tcjob_{i:02}");
            mgr.jobs.insert(
                id.clone(),
                IdeInstallJob {
                    id: id.clone(),
                    ecosystem: IdeEcosystemKind::Go,
                    component: IdeInstallComponent::Lsp,
                    version: None,
                    target_os: "darwin".into(),
                    target_arch: "aarch64".into(),
                    status: IdeInstallJobStatus::Running,
                    progress_pct: 50,
                    log_lines: vec!["working".into()],
                    installed_path: None,
                    error: None,
                },
            );
            mgr.finish_job(&id, Ok("/tmp/gopls".into()));
        }
        assert!(mgr.jobs.len() <= MAX_FINISHED_JOBS);
    }
}
