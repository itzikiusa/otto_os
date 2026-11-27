use std::collections::HashMap;
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use otto_state::DbPool;
use tokio::sync::{Mutex, OnceCell, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::engine::{Engine, EngineKind};
use crate::{
    governor::cancelled, store, EngineRun, Finding, ProjectScans, ScanDetail, ScanSettings,
    ScanSummary,
};

pub struct SecurityScans {
    engine: Engine,
    sast: Engine,
    queue: Semaphore,
    shutdown: CancellationToken,
    init: OnceCell<()>,
    jobs: Mutex<HashMap<String, Job>>,
}

struct Job {
    project: String,
    cancel: CancellationToken,
}

impl SecurityScans {
    pub fn new(toolchains: &Path) -> Self {
        Self {
            engine: Engine::new(toolchains, EngineKind::Trivy),
            sast: Engine::new(toolchains, EngineKind::Opengrep),
            queue: Semaphore::new(1),
            shutdown: CancellationToken::new(),
            init: OnceCell::new(),
            jobs: Mutex::new(HashMap::new()),
        }
    }

    async fn recover(&self, pool: &DbPool) -> Result<()> {
        self.init.get_or_try_init(|| store::recover(pool)).await?;
        Ok(())
    }

    pub async fn project(&self, pool: &DbPool, project: &str) -> Result<ProjectScans> {
        self.recover(pool).await?;
        let mut status = store::list(pool, project).await?;
        status.engine = self.engine.status().await;
        status.opengrep = self.sast.status().await;
        Ok(status)
    }

    pub async fn configure(
        &self,
        pool: &DbPool,
        project: &str,
        mut settings: ScanSettings,
    ) -> Result<ScanSettings> {
        self.recover(pool).await?;
        validate(&mut settings)?;
        store::save_settings(pool, project, &settings).await?;
        Ok(settings)
    }

    pub async fn detail(&self, pool: &DbPool, project: &str, id: &str) -> Result<ScanDetail> {
        self.recover(pool).await?;
        store::detail(pool, project, id).await
    }

    pub async fn enqueue<F, Fut, R, Roots>(
        self: &Arc<Self>,
        pool: DbPool,
        project: String,
        resolve_roots: R,
        busy: F,
    ) -> Result<ScanSummary>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send,
        R: FnOnce() -> Roots + Send + 'static,
        Roots: Future<Output = Result<Vec<PathBuf>>> + Send,
    {
        let service = self.clone();
        self.enqueue_job(pool, project, busy, move |settings, cancel| async move {
            let roots = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(cancelled()),
                result = resolve_roots() => result?,
            };
            service.scan_sources(&roots, &settings, &cancel).await
        })
        .await
    }

    pub async fn scan_sources(
        &self,
        roots: &[PathBuf],
        settings: &ScanSettings,
        cancel: &CancellationToken,
    ) -> Result<(Vec<EngineRun>, Vec<Finding>)> {
        let mut runs = Vec::new();
        let mut findings = Vec::new();
        for (engine, only_sast) in [(&self.engine, false), (&self.sast, true)] {
            let mut config = settings.clone();
            config
                .checks
                .retain(|check| (*check == crate::Check::Sast) == only_sast);
            if !config.checks.is_empty() {
                let (run, mut result) = engine.execute(roots, &config, cancel).await?;
                runs.push(run);
                findings.append(&mut result);
            }
        }
        Ok((runs, findings))
    }

    pub async fn enqueue_job<F, Fut, J, JobFuture>(
        self: &Arc<Self>,
        pool: DbPool,
        project: String,
        busy: F,
        job: J,
    ) -> Result<ScanSummary>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send,
        J: FnOnce(ScanSettings, CancellationToken) -> JobFuture + Send + 'static,
        JobFuture: Future<Output = Result<(Vec<EngineRun>, Vec<Finding>)>> + Send,
    {
        self.recover(&pool).await?;
        let settings = store::settings(&pool, &project).await?;
        let mut jobs = self.jobs.lock().await;
        if self.shutdown.is_cancelled() {
            return Err(Error::Conflict("Otto is shutting down.".into()));
        }
        let scan = store::insert(&pool, &project).await?;
        let cancel = self.shutdown.child_token();
        jobs.insert(
            scan.id.clone(),
            Job {
                project: project.clone(),
                cancel: cancel.clone(),
            },
        );
        drop(jobs);

        let id = scan.id.clone();
        let service = self.clone();
        tokio::spawn(async move {
            let run = async {
                let _turn = tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(cancelled()),
                    permit = service.queue.acquire() => permit.expect("scan queue never closes"),
                };
                let wait_for_idle = async {
                    while busy().await {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                };
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(cancelled()),
                    () = wait_for_idle => {}
                }
                if cancel.is_cancelled() || !store::start(&pool, &id).await? {
                    return Err(cancelled());
                }
                let (runs, mut findings) = job(settings, cancel.clone()).await?;
                findings.sort_unstable_by(|a, b| {
                    a.severity
                        .cmp(&b.severity)
                        .then_with(|| a.fingerprint.cmp(&b.fingerprint))
                });
                if findings.len() > crate::report::MAX_FINDINGS {
                    return Err(Error::internal_public(
                        "This scan produced too many findings. Narrow its scope and retry.",
                    ));
                }
                Ok((runs, findings))
            };
            // Cancel queued scans without waiting for the active scan. Once the
            // engine starts, let its deadline/kill-and-reap path finish under the permit.
            let output = run.await;
            let mut jobs = service.jobs.lock().await;
            let stored = if cancel.is_cancelled() {
                store::cancel(&pool, &project, &id).await.map(|_| ())
            } else {
                store::finish(&pool, &id, output).await
            };
            if let Err(error) = stored {
                tracing::error!(scan_id = %id, %error, "could not record security scan result");
            }
            jobs.remove(&id);
        });
        Ok(scan)
    }

    pub async fn cancel(&self, pool: &DbPool, project: &str, id: &str) -> Result<ScanSummary> {
        self.recover(pool).await?;
        let jobs = self.jobs.lock().await;
        if let Some(job) = jobs.get(id).filter(|job| job.project == project) {
            job.cancel.cancel();
        }
        drop(jobs);
        store::cancel(pool, project, id).await
    }

    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        // The running engine sees cancellation, kills the group and reaps it
        // before releasing its permit.
        let _ = self.queue.acquire().await;
    }
}

fn validate(settings: &mut ScanSettings) -> Result<()> {
    if settings.checks.is_empty()
        || settings.checks.contains(&crate::Check::Dast)
        || !(1..=2).contains(&settings.parallelism)
        || !(256..=4096).contains(&settings.memory_mib)
        || !(10..=900).contains(&settings.timeout_secs)
    {
        return Err(Error::Invalid(
            "Select at least one check and use 1–2 workers, 256–4096 MiB, and 10–900 seconds."
                .into(),
        ));
    }
    if settings.excludes.len() > 64
        || settings.excludes.iter().any(|path| {
            path.is_empty()
                || path.len() > 240
                || path.contains(',')
                || path.chars().any(char::is_control)
                || Path::new(path)
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
        })
    {
        return Err(Error::Invalid(
            "Use up to 64 relative exclude patterns without commas or parent directories.".into(),
        ));
    }
    crate::incremental::exclude_globs(settings)?;
    settings.checks.sort_unstable();
    settings.checks.dedup();
    Ok(())
}
