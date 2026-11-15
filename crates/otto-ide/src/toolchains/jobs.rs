//! Background toolchain install job tracking and runner methods for [`ToolchainManager`].

use otto_core::{new_id, Error, Result};

use crate::target::TargetManager;
use crate::toolchains_catalog::{
    is_safe_version_token, resolve_sdk_download, CatalogSdkEntry, EcosystemKind, TargetPlatform,
};
use crate::types::{
    IdeInstallComponent, IdeInstallJob, IdeInstallJobStatus, IdeInstallToolchainReq, IdeSshSpec,
    IdeTargetKind,
};

use super::installer::install_local_sdk;
use super::installer_remote::{build_remote_install_script, run_ssh_streaming};
use super::installer_tools::{install_local_dap, install_local_lsp};
use super::{ToolchainManager, MAX_FINISHED_JOBS, MAX_JOB_LOGS};

impl ToolchainManager {
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

    pub(super) fn finish_job(&self, job_id: &str, outcome: std::result::Result<String, String>) {
        if let Some(mut entry) = self.jobs.get_mut(job_id) {
            match outcome {
                Ok(path) => {
                    entry.status = IdeInstallJobStatus::Done;
                    entry.progress_pct = 100;
                    entry.installed_path = Some(path.clone());
                    entry.log_lines.push(format!("Completed cleanly: {path}"));
                }
                Err(err) => {
                    tracing::warn!(job_id, error = %err, "IDE toolchain install job failed");
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
        crate::lsp_host::servers::invalidate_detect_cache();
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
