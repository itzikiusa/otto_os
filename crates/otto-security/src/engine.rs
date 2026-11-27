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

const RELEASE_API: &str = "https://api.github.com/repos/aquasecurity/trivy/releases/latest";
const MIB: u64 = 1024 * 1024;
const DB_REFRESH_HOURS: i64 = 6;
const DB_REPO: &str = "aquasecurity/trivy-db";
const BASE_EXCLUDED_DIRS: &[&str] = &[".git", "node_modules", "target", ".venv", ".gradle"];

pub(crate) struct Engine {
    dir: PathBuf,
}

impl Engine {
    pub(crate) fn new(toolchains: &Path) -> Self {
        Self {
            dir: toolchains.join("security-trivy"),
        }
    }

    pub(crate) async fn status(&self) -> Option<EngineVersion> {
        read_json(&self.dir.join("engine.json")).await
    }

    pub(crate) async fn execute(
        &self,
        roots: &[PathBuf],
        settings: &ScanSettings,
        cancel: &CancellationToken,
    ) -> Result<(EngineVersion, ScanMetrics, Vec<Finding>)> {
        if !cfg!(target_os = "macos") {
            return Err(Error::Invalid(
                "Security scans require macOS Seatbelt.".into(),
            ));
        }
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(io_error)?;
        let client = download::client()?;
        let timeout = Duration::from_secs(u64::from(settings.timeout_secs));
        let updates = cancel.child_token();
        let prepared = async {
            let version = self.refresh_release(&client, &updates).await?;
            let cache = if settings.checks.contains(&Check::Vulnerability) {
                self.refresh_database(&client, &updates).await?
            } else {
                self.dir.join("offline-cache")
            };
            Ok::<_, Error>((version, cache))
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
        let (version, cache) = result.expect("preparation completed")?;
        tokio::fs::create_dir_all(&cache).await.map_err(io_error)?;
        let program = self.binary(&version.version)?;
        let mut metrics = ScanMetrics::default();
        let mut findings = Vec::new();
        for root in roots {
            let (run, mut results) = self
                .scan_root(&program, &cache, root, settings, cancel)
                .await?;
            metrics.wall_ms += run.wall_ms;
            metrics.peak_cpu_percent = metrics.peak_cpu_percent.max(run.peak_cpu_percent);
            metrics.peak_rss_bytes = metrics.peak_rss_bytes.max(run.peak_rss_bytes);
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
        Ok((version, metrics, findings))
    }

    async fn scan_root(
        &self,
        program: &Path,
        cache: &Path,
        root: &Path,
        settings: &ScanSettings,
        cancel: &CancellationToken,
    ) -> Result<(ScanMetrics, Vec<Finding>)> {
        let scratch = tempfile::tempdir_in(&self.dir).map_err(io_error)?;
        let output = tokio::fs::canonicalize(scratch.path())
            .await
            .map_err(io_error)?;
        let config = output.join("config.yaml");
        tokio::fs::write(&config, "{}\n").await.map_err(io_error)?;
        let report = output.join("report.json");
        let args = trivy_args(root, cache, &report, &config, settings);
        let metrics = governor::run(program, &args, root, cache, &output, settings, cancel).await?;
        let path = root.to_path_buf();
        let data = tokio::fs::read(&report).await.map_err(io_error)?;
        let findings =
            tokio::task::spawn_blocking(move || crate::report::parse_report(&data, &path))
                .await
                .map_err(|_| Error::Internal("security report worker stopped".into()))??;
        Ok((metrics, findings))
    }

    fn binary(&self, version: &str) -> Result<PathBuf> {
        if !version_valid(version) {
            return Err(upstream("Trivy returned an invalid release version."));
        }
        Ok(self.dir.join("releases").join(version).join("trivy"))
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
        if last.as_deref().is_some_and(|stamp| fresh(stamp, 24)) {
            return installed.filter(|_| usable).ok_or_else(|| {
                upstream("Trivy installation failed today. Otto will check for a release again tomorrow.")
            });
        }
        write_json(&attempt, &crate::store::now()).await?;
        let update =
            async {
                let request =
                    async {
                        download::json::<Release>(client.get(RELEASE_API).send().await.map_err(
                            |_| upstream("The Trivy release service could not be reached."),
                        )?)
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
        match update {
            Err(error) if usable && !cancel.is_cancelled() => {
                tracing::warn!(%error, "Trivy release refresh failed; keeping installed version");
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
        let asset_platform = match std::env::consts::ARCH {
            "aarch64" => "macOS-ARM64",
            "x86_64" => "macOS-64bit",
            _ => {
                return Err(Error::Invalid(
                    "Trivy is unavailable on this architecture.".into(),
                ))
            }
        };
        let version = release.tag_name.trim_start_matches('v');
        let name = format!("trivy_{version}_{asset_platform}.tar.gz");
        let asset = release
            .assets
            .iter()
            .find(|asset| asset.name == name)
            .ok_or_else(|| upstream("The current Trivy release has no asset for this Mac."))?;
        let digest = download::sha256(&asset.digest)?;
        let url = format!(
            "https://github.com/aquasecurity/trivy/releases/download/{}/{name}",
            release.tag_name
        );
        let dir = binary.parent().expect("release directory");
        tokio::fs::create_dir_all(dir).await.map_err(io_error)?;
        let stage = tempfile::tempdir_in(dir).map_err(io_error)?;
        let archive = stage.path().join("release.tgz");
        fetch(client.get(url), &archive, digest, 160 * MIB, cancel).await?;
        let stage = unpack(stage, archive, &["trivy"], 512 * MIB, cancel).await?;
        let executable = stage.path().join("trivy");
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

fn fresh(at: &str, hours: i64) -> bool {
    DateTime::parse_from_rfc3339(at).is_ok_and(|stamp| {
        let age = Utc::now().signed_duration_since(stamp);
        age >= chrono::Duration::zero() && age < chrono::Duration::hours(hours)
    })
}

async fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
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

    /// Local acceptance check. Uses only temporary tool state and prints aggregate metrics.
    #[tokio::test]
    #[ignore = "downloads Trivy and its public vulnerability DB"]
    async fn live_trivy_on_otto_worktree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let tools = tempfile::tempdir().unwrap();
        let engine = Engine::new(&tools.path().canonicalize().unwrap());
        let cancel = CancellationToken::new();
        let settings = ScanSettings {
            timeout_secs: 300,
            ..Default::default()
        };
        let (version, metrics, findings) =
            engine.execute(&[root], &settings, &cancel).await.unwrap();
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
}
