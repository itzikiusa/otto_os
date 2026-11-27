use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use reqwest::Client;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::download::{self, io_error, upstream};
use crate::governor::{self, cancelled};
use crate::{Check, EngineVersion, Finding, ScanMetrics, ScanSettings};

const MIB: u64 = 1024 * 1024;
const DB_REFRESH_HOURS: i64 = 6;
const DB_REPO: &str = "aquasecurity/trivy-db";
pub(crate) const BASE_EXCLUDED_DIRS: &[&str] =
    &[".git", "node_modules", "target", ".venv", ".gradle"];

pub(crate) struct Engine {
    dir: PathBuf,
    kind: EngineKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineKind {
    Trivy,
    Opengrep,
}

impl EngineKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Trivy => "trivy",
            Self::Opengrep => "opengrep",
        }
    }

    fn repository(self) -> &'static str {
        match self {
            Self::Trivy => "aquasecurity/trivy",
            Self::Opengrep => "opengrep/opengrep",
        }
    }

    fn asset(self, version: &str) -> Result<String> {
        match (self, std::env::consts::ARCH) {
            (Self::Trivy, "aarch64") => Ok(format!("trivy_{version}_macOS-ARM64.tar.gz")),
            (Self::Trivy, "x86_64") => Ok(format!("trivy_{version}_macOS-64bit.tar.gz")),
            (Self::Opengrep, "aarch64") => Ok("opengrep_osx_arm64".into()),
            (Self::Opengrep, "x86_64") => Ok("opengrep_osx_x86".into()),
            _ => Err(Error::Invalid(
                "This scanner is unavailable on this architecture.".into(),
            )),
        }
    }
}

impl Engine {
    pub(crate) fn new(toolchains: &Path, kind: EngineKind) -> Self {
        Self {
            dir: toolchains.join(format!("security-{}", kind.name())),
            kind,
        }
    }

    pub(crate) async fn status(&self) -> Option<EngineVersion> {
        read_json(&self.dir.join("engine.json")).await
    }

    pub(crate) async fn execute<Fut>(
        &self,
        roots: &[PathBuf],
        settings: &ScanSettings,
        cancel: &CancellationToken,
        busy: &(impl Fn() -> Fut + Sync),
    ) -> Result<(crate::EngineRun, Vec<Finding>)>
    where
        Fut: Future<Output = bool> + Send,
    {
        if !cfg!(target_os = "macos") {
            return Err(Error::Invalid(
                "Security scans require macOS Seatbelt.".into(),
            ));
        }
        crate::wait_for_idle(busy, cancel).await?;
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(io_error)?;
        let client = download::client()?;
        let timeout = Duration::from_secs(u64::from(settings.timeout_secs));
        let updates = cancel.child_token();
        let prepared = async {
            let version = self.refresh_release(&client, &updates).await?;
            let cache = if self.kind == EngineKind::Trivy
                && settings.checks.contains(&Check::Vulnerability)
            {
                self.refresh_database(&client, &updates).await?
            } else {
                self.dir.join("offline-cache")
            };
            let rules = if self.kind == EngineKind::Opengrep {
                Some(crate::sast::refresh_rules(&client, &self.dir, &updates).await?)
            } else {
                None
            };
            Ok::<_, Error>((version, cache, rules))
        };
        tokio::pin!(prepared);
        let mut interrupted = None;
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                interrupted = Some(cancelled());
                None
            }
            _ = tokio::time::sleep(timeout) => {
                updates.cancel();
                interrupted = Some(Error::internal_public("Scanner preparation reached the time limit."));
                None
            }
            result = &mut prepared => Some(result),
        };
        if let Some(error) = interrupted {
            // Let a started extraction observe cancellation and release its staging directory.
            let _ = prepared.await;
            return Err(error);
        }
        let (version, cache, rules) = result.expect("preparation completed")?;
        tokio::fs::create_dir_all(&cache).await.map_err(io_error)?;
        let program = self.binary(&version.version)?;
        let mut engine_run = crate::EngineRun {
            name: self.kind.name().into(),
            version: version.clone(),
            metrics: ScanMetrics::default(),
            mode: crate::ScanMode::Cached,
        };
        let key = crate::store::encode(&(
            "2",
            &version.version,
            &cache,
            rules.as_ref().map(|rules| &rules.digest),
            settings,
        ))?;
        let cache_context = crate::incremental::CacheContext::new(&self.dir, &key, settings);
        let mut findings = Vec::new();
        for root in roots {
            crate::wait_for_idle(busy, cancel).await?;
            let plan = cache_context.plan(root, self.kind, cancel).await?;
            let (run, results) = match &plan.selection {
                crate::incremental::Selection::Files(files) if files.is_empty() => {
                    (ScanMetrics::default(), Vec::new())
                }
                selection => {
                    crate::wait_for_idle(busy, cancel).await?;
                    self.scan_root(
                        &program,
                        &cache,
                        root,
                        settings,
                        rules.as_ref(),
                        selection,
                        cancel,
                    )
                    .await?
                }
            };
            let mut results = cache_context
                .finish(root, plan, &mut engine_run.mode, results, cancel)
                .await?;
            engine_run.metrics.wall_ms += run.wall_ms;
            engine_run.metrics.peak_cpu_percent = engine_run
                .metrics
                .peak_cpu_percent
                .max(run.peak_cpu_percent);
            engine_run.metrics.peak_rss_bytes =
                engine_run.metrics.peak_rss_bytes.max(run.peak_rss_bytes);
            findings.append(&mut results);
            if findings.len() > crate::report::MAX_FINDINGS {
                return Err(Error::internal_public(
                    "This scan produced too many findings. Narrow its scope and retry.",
                ));
            }
        }
        findings.sort_unstable_by(|a, b| {
            a.severity
                .cmp(&b.severity)
                .then_with(|| a.fingerprint.cmp(&b.fingerprint))
        });
        findings.dedup_by(|a, b| a.fingerprint == b.fingerprint);
        Ok((engine_run, findings))
    }

    #[allow(clippy::too_many_arguments)] // Already-prepared engine inputs and the run cancellation token.
    async fn scan_root(
        &self,
        program: &Path,
        cache: &Path,
        root: &Path,
        settings: &ScanSettings,
        rules: Option<&crate::sast::Rules>,
        selection: &crate::incremental::Selection,
        cancel: &CancellationToken,
    ) -> Result<(ScanMetrics, Vec<Finding>)> {
        let scratch = tempfile::tempdir_in(&self.dir).map_err(io_error)?;
        let output = tokio::fs::canonicalize(scratch.path())
            .await
            .map_err(io_error)?;
        let report = output.join("report.json");
        let args = if let Some(rules) = rules {
            crate::sast::args(root, &report, &output, rules, settings, selection).await?
        } else {
            let config = output.join("config.yaml");
            tokio::fs::write(&config, "{}\n").await.map_err(io_error)?;
            trivy_args(root, cache, &report, &config, settings)
        };
        let metrics = governor::run(program, &args, root, cache, &output, settings, cancel).await?;
        let path = root.to_path_buf();
        let data = tokio::fs::read(&report).await.map_err(io_error)?;
        let kind = self.kind;
        let rules = rules.cloned();
        let findings = tokio::task::spawn_blocking(move || match kind {
            EngineKind::Trivy => crate::report::parse_report(&data, &path),
            EngineKind::Opengrep => crate::sast::parse(
                &data,
                &path,
                rules.as_ref().expect("prepared Opengrep rules"),
            ),
        })
        .await
        .map_err(|_| Error::Internal("security report worker stopped".into()))??;
        Ok((metrics, findings))
    }

    fn binary(&self, version: &str) -> Result<PathBuf> {
        if !version_valid(version) {
            return Err(upstream("The scanner returned an invalid release version."));
        }
        Ok(self
            .dir
            .join("releases")
            .join(version)
            .join(self.kind.name()))
    }

    async fn refresh_release(
        &self,
        client: &Client,
        cancel: &CancellationToken,
    ) -> Result<EngineVersion> {
        let installed = self.status().await;
        let attempt = self.dir.join("release-attempt.json");
        let last: Option<String> = read_json(&attempt).await;
        let usable = match &installed {
            Some(record) => tokio::fs::try_exists(self.binary(&record.version)?)
                .await
                .unwrap_or(false),
            None => false,
        };
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if usable && last.as_deref().is_some_and(|stamp| fresh(stamp, 24)) {
            return Ok(installed.expect("installed release"));
        }
        let update = async {
            let request = async {
                download::json::<Release>(
                    client
                        .get(format!(
                            "https://api.github.com/repos/{}/releases/latest",
                            self.kind.repository()
                        ))
                        .send()
                        .await
                        .map_err(|_| {
                            upstream("The scanner release service could not be reached.")
                        })?,
                )
                .await
            };
            let release = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(cancelled()),
                result = request => result?,
            };
            let version = release
                .tag_name
                .strip_prefix('v')
                .unwrap_or(&release.tag_name)
                .to_string();
            let binary = self.binary(&version)?;
            let changed = installed
                .as_ref()
                .is_none_or(|current| current.version != version)
                || !usable;
            if changed {
                self.install_release(client, release, &binary, cancel)
                    .await?;
            }
            let checked_at = crate::store::now();
            let updated_at = if changed {
                checked_at.clone()
            } else {
                installed
                    .as_ref()
                    .expect("installed release")
                    .updated_at
                    .clone()
            };
            let record = EngineVersion {
                version,
                updated_at,
                checked_at,
            };
            write_json(&self.dir.join("engine.json"), &record).await?;
            Ok(record)
        }
        .await;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        if usable || update.is_ok() {
            write_json(&attempt, &crate::store::now()).await?;
        }
        match update {
            Err(error) if usable => {
                tracing::warn!(%error, "Scanner release refresh failed; keeping installed version");
                Ok(installed.expect("installed release"))
            }
            result => result,
        }
    }

    async fn install_release(
        &self,
        client: &Client,
        release: Release,
        binary: &Path,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let version = release.tag_name.trim_start_matches('v');
        let name = self.kind.asset(version)?;
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .ok_or_else(|| upstream("The scanner release has no asset for this Mac."))?;
        let digest = download::sha256(&asset.digest)?;
        let url = format!(
            "https://github.com/{}/releases/download/{}/{name}",
            self.kind.repository(),
            release.tag_name
        );
        let dir = binary.parent().expect("release directory");
        tokio::fs::create_dir_all(dir).await.map_err(io_error)?;
        let stage = tempfile::tempdir_in(dir).map_err(io_error)?;
        let payload = stage.path().join("release");
        fetch(client.get(url), &payload, digest, 256 * MIB, cancel).await?;
        let stage = if self.kind == EngineKind::Trivy {
            unpack(stage, payload, &["trivy"], 512 * MIB, cancel).await?
        } else {
            stage
        };
        let executable = stage.path().join(if self.kind == EngineKind::Trivy {
            "trivy"
        } else {
            "release"
        });
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
                .await
                .map_err(io_error)?;
        }
        tokio::fs::rename(executable, binary)
            .await
            .map_err(io_error)
    }

    async fn refresh_database(
        &self,
        client: &Client,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        let record_path = self.dir.join("database.json");
        let existing: Option<Database> = read_json(&record_path).await;
        let old_cache = existing
            .as_ref()
            .and_then(|db| download::sha256(&db.digest).ok())
            .map(|digest| self.dir.join(format!("cache-{digest}")));
        let old_cache = match old_cache {
            Some(path)
                if tokio::fs::try_exists(path.join("db/trivy.db"))
                    .await
                    .unwrap_or(false) =>
            {
                Some(path)
            }
            _ => None,
        };
        if let (Some(record), Some(cache)) = (&existing, &old_cache) {
            if fresh(&record.checked_at, DB_REFRESH_HOURS) {
                return Ok(cache.clone());
            }
        }
        let result = self.update_database(client, cancel).await;
        match (result, old_cache) {
            (Ok(cache), old) => {
                if let Some(old) = old.filter(|old| old != &cache) {
                    let _ = tokio::fs::remove_dir_all(old).await;
                }
                Ok(cache)
            }
            (Err(error), Some(cache)) if !cancel.is_cancelled() => {
                tracing::warn!(%error, "Trivy database refresh failed; keeping cached database");
                Ok(cache)
            }
            (Err(error), _) => Err(error),
        }
    }

    async fn update_database(
        &self,
        client: &Client,
        cancel: &CancellationToken,
    ) -> Result<PathBuf> {
        #[derive(Deserialize)]
        struct Token {
            token: String,
        }
        #[derive(Deserialize)]
        struct Layer {
            digest: String,
        }
        #[derive(Deserialize)]
        struct Manifest {
            layers: Vec<Layer>,
        }
        let service_error =
            |_| upstream("The Trivy vulnerability database service could not be reached.");
        let request = async {
            let token: Token = download::json(
                client
                    .get(format!(
                        "https://ghcr.io/token?scope=repository:{DB_REPO}:pull&service=ghcr.io"
                    ))
                    .send()
                    .await
                    .map_err(service_error)?,
            )
            .await?;
            let manifest: Manifest = download::json(
                client
                    .get(format!("https://ghcr.io/v2/{DB_REPO}/manifests/2"))
                    .header("Accept", "application/vnd.oci.image.manifest.v1+json")
                    .bearer_auth(&token.token)
                    .send()
                    .await
                    .map_err(service_error)?,
            )
            .await?;
            Ok::<_, Error>((token, manifest))
        };
        let (token, manifest) = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(cancelled()),
            result = request => result?,
        };
        let [layer] = manifest.layers.as_slice() else {
            return Err(upstream("The Trivy database has an unexpected manifest."));
        };
        let digest = download::sha256(&layer.digest)?;
        let cache = self.dir.join(format!("cache-{digest}"));
        if !tokio::fs::try_exists(cache.join("db/trivy.db"))
            .await
            .map_err(io_error)?
        {
            let stage = tempfile::tempdir_in(&self.dir).map_err(io_error)?;
            let archive = stage.path().join("db.tgz");
            let request = client
                .get(format!(
                    "https://ghcr.io/v2/{DB_REPO}/blobs/{}",
                    layer.digest
                ))
                .bearer_auth(token.token);
            fetch(request, &archive, digest, 384 * MIB, cancel).await?;
            let stage = unpack(
                stage,
                archive,
                &["trivy.db", "metadata.json"],
                3072 * MIB,
                cancel,
            )
            .await?;
            tokio::fs::remove_file(stage.path().join("db.tgz"))
                .await
                .map_err(io_error)?;
            tokio::fs::create_dir_all(&cache).await.map_err(io_error)?;
            tokio::fs::rename(stage.path(), cache.join("db"))
                .await
                .map_err(io_error)?;
        }
        write_json(
            &self.dir.join("database.json"),
            &Database {
                digest: layer.digest.clone(),
                checked_at: crate::store::now(),
            },
        )
        .await?;
        Ok(cache)
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    #[serde(default)]
    digest: String,
}

#[derive(serde::Serialize, Deserialize)]
struct Database {
    digest: String,
    checked_at: String,
}

fn version_valid(version: &str) -> bool {
    let parts: Vec<_> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| p.parse::<u32>().is_ok())
}

pub(crate) fn fresh(at: &str, hours: i64) -> bool {
    DateTime::parse_from_rfc3339(at).is_ok_and(|stamp| {
        let age = Utc::now().signed_duration_since(stamp);
        age >= chrono::Duration::zero() && age < chrono::Duration::hours(hours)
    })
}

pub(crate) async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) async fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let contents = crate::store::encode(value)?;
    let stage =
        tempfile::tempdir_in(path.parent().expect("metadata directory")).map_err(io_error)?;
    let file = stage.path().join("metadata");
    tokio::fs::write(&file, contents).await.map_err(io_error)?;
    tokio::fs::rename(file, path).await.map_err(io_error)
}

async fn fetch(
    request: reqwest::RequestBuilder,
    path: &Path,
    digest: &str,
    limit: u64,
    cancel: &CancellationToken,
) -> Result<()> {
    let transfer = async {
        let response = request
            .send()
            .await
            .map_err(|_| upstream("The scanner download service could not be reached."))?;
        download::verified_file(response, path, digest, limit).await
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(cancelled()),
        result = transfer => result,
    }
}

async fn unpack(
    stage: tempfile::TempDir,
    archive: PathBuf,
    names: &'static [&'static str],
    limit: u64,
    cancel: &CancellationToken,
) -> Result<tempfile::TempDir> {
    let cancel = cancel.clone();
    tokio::task::spawn_blocking(move || {
        download::extract(&archive, stage.path(), names, limit, &cancel)?;
        Ok(stage)
    })
    .await
    .map_err(|_| Error::Internal("security download worker stopped".into()))?
}

fn trivy_args(
    root: &Path,
    cache: &Path,
    report: &Path,
    config: &Path,
    settings: &ScanSettings,
) -> Vec<String> {
    let checks = settings
        .checks
        .iter()
        .map(|check| match check {
            Check::Vulnerability => "vuln",
            Check::Secret => "secret",
            Check::Misconfiguration => "misconfig",
            Check::License => "license",
            Check::Sast | Check::Dast => unreachable!("Trivy receives only Trivy checks"),
        })
        .collect::<Vec<_>>()
        .join(",");
    let mut args = vec![
        "filesystem".into(),
        "--cache-dir".into(),
        cache.to_string_lossy().into_owned(),
        "--cache-backend".into(),
        "memory".into(),
        "--config".into(),
        config.to_string_lossy().into_owned(),
        "--secret-config".into(),
        config.to_string_lossy().into_owned(),
        "--ignorefile=".into(),
        "--offline-scan".into(),
        "--skip-db-update".into(),
        "--skip-java-db-update".into(),
        "--skip-check-update".into(),
        "--skip-vex-repo-update".into(),
        "--skip-version-check".into(),
        "--disable-telemetry".into(),
        "--no-progress".into(),
        "--scanners".into(),
        checks,
        "--parallel".into(),
        settings.parallelism.to_string(),
        "--timeout".into(),
        format!("{}s", settings.timeout_secs),
        "--format".into(),
        "json".into(),
        "--output".into(),
        report.to_string_lossy().into_owned(),
    ];
    // Archive analysis requires Trivy's separate 1 GB Java index. Keep lockfile and
    // manifest scanning available without making every project download that index.
    for extension in ["jar", "war", "ear", "par"] {
        args.extend(["--skip-files".into(), format!("**/*.{extension}")]);
    }
    for entry in BASE_EXCLUDED_DIRS {
        args.extend(["--skip-dirs".into(), format!("**/{entry}")]);
    }
    for entry in &settings.excludes {
        args.extend([
            "--skip-dirs".into(),
            entry.clone(),
            "--skip-files".into(),
            entry.clone(),
        ]);
    }
    args.push(root.to_string_lossy().into_owned());
    args
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refresh_release_retries_a_missing_binary_and_leaves_no_attempt_on_cancel() {
        let tools = tempfile::tempdir().unwrap();
        let engine = Engine::new(tools.path(), EngineKind::Trivy);
        tokio::fs::create_dir_all(&engine.dir).await.unwrap();
        let client = download::client().unwrap();
        let attempt = engine.dir.join("release-attempt.json");
        let cancel = CancellationToken::new();
        cancel.cancel();

        let error = engine.refresh_release(&client, &cancel).await.unwrap_err();
        assert_eq!(error.to_string(), cancelled().to_string());
        assert!(
            !attempt.exists(),
            "a cancelled install recorded a release check"
        );

        write_json(&attempt, &crate::store::now()).await.unwrap();
        let error = engine.refresh_release(&client, &cancel).await.unwrap_err();
        assert_eq!(error.to_string(), cancelled().to_string());
    }

    #[tokio::test]
    async fn refresh_release_retries_offline_installs_and_throttles_installed_fallback() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Default)]
        struct Offline(AtomicUsize);
        impl reqwest::dns::Resolve for Offline {
            fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
                self.0.fetch_add(1, Ordering::Relaxed);
                Box::pin(async { Err(std::io::Error::other("offline fixture").into()) })
            }
        }

        let tools = tempfile::tempdir().unwrap();
        let engine = Engine::new(tools.path(), EngineKind::Trivy);
        tokio::fs::create_dir_all(&engine.dir).await.unwrap();
        let offline = std::sync::Arc::new(Offline::default());
        let client = Client::builder()
            .no_proxy()
            .dns_resolver(offline.clone())
            .build()
            .unwrap();
        let cancel = CancellationToken::new();
        for _ in 0..2 {
            let error = engine.refresh_release(&client, &cancel).await.unwrap_err();
            assert!(error.to_string().contains("could not be reached"));
        }
        assert_eq!(offline.0.load(Ordering::Relaxed), 2);

        let version = EngineVersion {
            version: "1.2.3".into(),
            checked_at: crate::store::now(),
            updated_at: crate::store::now(),
        };
        let binary = engine.binary(&version.version).unwrap();
        tokio::fs::create_dir_all(binary.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(binary, "installed fixture").await.unwrap();
        write_json(&engine.dir.join("engine.json"), &version)
            .await
            .unwrap();
        for _ in 0..2 {
            let result = engine.refresh_release(&client, &cancel).await.unwrap();
            assert_eq!(result.version, version.version);
        }
        assert_eq!(offline.0.load(Ordering::Relaxed), 3);
    }

    /// Local acceptance check. Uses only temporary tool state and prints aggregate metrics.
    #[tokio::test]
    #[ignore = "downloads Trivy and its public vulnerability DB"]
    async fn live_trivy_on_otto_worktree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let tools = tempfile::tempdir().unwrap();
        let engine = Engine::new(&tools.path().canonicalize().unwrap(), EngineKind::Trivy);
        let cancel = CancellationToken::new();
        let settings = ScanSettings {
            checks: vec![Check::Vulnerability, Check::Secret, Check::Misconfiguration],
            timeout_secs: 300,
            ..Default::default()
        };
        let (run, findings) = engine
            .execute(&[root], &settings, &cancel, &|| async { false })
            .await
            .unwrap();
        let (version, metrics) = (run.version, run.metrics);
        println!(
            "Trivy {} full scan of Otto: {} findings, wall={} ms, sampled peak CPU={:.1}%, peak RSS={} MiB",
            version.version,
            findings.len(),
            metrics.wall_ms,
            metrics.peak_cpu_percent,
            metrics.peak_rss_bytes / MIB
        );
        assert!(metrics.peak_rss_bytes > 0);
        assert!(metrics.peak_cpu_percent > 0.0);
    }

    #[tokio::test]
    #[ignore = "downloads Opengrep and community rules, then scans the Otto worktree"]
    async fn live_incremental_engines_on_otto_worktree() {
        use std::time::Instant;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let tools = tempfile::tempdir().unwrap();
        let path = tools.path().canonicalize().unwrap();
        let sample = tempfile::Builder::new()
            .prefix("otto-secscan-")
            .suffix(".zig")
            .tempfile_in(&root)
            .unwrap();
        tokio::fs::write(sample.path(), "pub fn probe() void {}\n")
            .await
            .unwrap();
        let name = sample.path().file_name().unwrap().to_str().unwrap();
        let cancel = CancellationToken::new();
        for kind in [EngineKind::Opengrep, EngineKind::Trivy] {
            let engine = Engine::new(&path, kind);
            let settings = ScanSettings {
                checks: if kind == EngineKind::Opengrep {
                    vec![Check::Sast]
                } else {
                    vec![Check::Vulnerability, Check::Secret, Check::Misconfiguration]
                },
                timeout_secs: 300,
                ..Default::default()
            };
            let start = Instant::now();
            let (full, _) = engine
                .execute(std::slice::from_ref(&root), &settings, &cancel, &|| async {
                    false
                })
                .await
                .unwrap();
            assert_eq!(full.mode, crate::ScanMode::Full);
            print_metrics(&full, start);
            if kind == EngineKind::Opengrep {
                tokio::fs::write(
                    sample.path(),
                    "pub fn probe() void { @setRuntimeSafety(false); }\n",
                )
                .await
                .unwrap();
                let start = Instant::now();
                let (incremental, findings) = engine
                    .execute(std::slice::from_ref(&root), &settings, &cancel, &|| async {
                        false
                    })
                    .await
                    .unwrap();
                assert_eq!(incremental.mode, crate::ScanMode::Incremental);
                print_metrics(&incremental, start);
                assert!(findings.iter().any(|finding| finding.path == name
                    && finding.rule == "otto.zig.runtime-safety-disabled"));
            }
            let start = Instant::now();
            let (cached, _) = engine
                .execute(std::slice::from_ref(&root), &settings, &cancel, &|| async {
                    false
                })
                .await
                .unwrap();
            assert_eq!(cached.mode, crate::ScanMode::Cached);
            print_metrics(&cached, start);
        }
    }

    fn print_metrics(run: &crate::EngineRun, start: std::time::Instant) {
        println!(
            "{} {} {:?}: engine={} ms total={} ms peak CPU={:.1}% peak RSS={} MiB",
            run.name,
            run.version.version,
            run.mode,
            run.metrics.wall_ms,
            start.elapsed().as_millis(),
            run.metrics.peak_cpu_percent,
            run.metrics.peak_rss_bytes / MIB
        );
    }
}
