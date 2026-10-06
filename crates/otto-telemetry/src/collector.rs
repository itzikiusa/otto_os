//! Managed, pinned upstream Collector; no external OTLP destinations.
use crate::TelemetryConfig;
use anyhow::{bail, Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};
use sysinfo::{
    Pid, Process, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, Signal, System, Uid,
    UpdateKind,
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
};
pub const VERSION: &str = "0.162.0";
/// The collector's own stderr (export failures, rejected batches). Rotated
/// once at each start so it stays bounded across many short flush runs.
const LOG_LIMIT: u64 = 1024 * 1024;
/// Go heap soft limit, kept below the memory_limiter hard limit (192 MiB) so
/// the runtime collects before the limiter starts refusing data.
const GOMEMLIMIT: &str = "160MiB";
pub(crate) struct Collector {
    child: Child,
    _owner_lock: Option<Arc<std::fs::File>>,
    pub endpoint: String,
    pub health: String,
    /// Loopback Prometheus endpoint for the collector's own metrics.
    pub metrics: String,
}
/// Exporter health scraped from the collector's internal metrics.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct ExporterStats {
    /// Records the exporters gave up on (after retries) since this start.
    pub send_failed: u64,
    /// Records still waiting in the exporters' sending queues.
    pub queue_size: u64,
}
/// Sum the exporter counters out of a Prometheus text exposition. Unknown
/// lines and malformed values are ignored, never treated as zero failures.
pub(crate) fn parse_exporter_stats(text: &str) -> Option<ExporterStats> {
    let mut stats = ExporterStats::default();
    let mut seen = false;
    for line in text.lines() {
        if line.starts_with('#') {
            continue;
        }
        let name = line.split(['{', ' ']).next().unwrap_or_default();
        let Some(value) = line
            .rsplit(' ')
            .next()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v >= 0.0)
        else {
            continue;
        };
        if name.starts_with("otelcol_exporter_send_failed_") {
            stats.send_failed += value as u64;
            seen = true;
        } else if name == "otelcol_exporter_queue_size" {
            stats.queue_size += value as u64;
            seen = true;
        }
    }
    seen.then_some(stats)
}
impl Collector {
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    /// `None` when the metrics endpoint is unreachable or exposes no exporter
    /// series yet (they appear after the first export attempt).
    pub async fn exporter_stats(&self, client: &reqwest::Client) -> Option<ExporterStats> {
        let text = client
            .get(&self.metrics)
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()?;
        parse_exporter_stats(&text)
    }
    /// SIGTERM first so the collector drains its sending queue into
    /// ClickHouse, then SIGKILL after a bounded wait. The child is not yet
    /// reaped, so its pid cannot have been reused.
    pub async fn stop(&mut self) {
        if let Some(pid) = self.child.id().filter(|_| self.alive()) {
            let pid = Pid::from_u32(pid);
            let mut system = System::new();
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing(),
            );
            if system
                .process(pid)
                .and_then(|p| p.kill_with(Signal::Term))
                .unwrap_or(false)
            {
                let _ = tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await;
            }
        }
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
        self._owner_lock.take();
    }
    pub async fn start(dir: &Path, clickhouse: &str, config: &TelemetryConfig) -> Result<Self> {
        // A previous daemon may have died without running Child::drop. Never
        // rewrite a live owner's config or start a second exporter beside it.
        tokio::fs::create_dir_all(dir).await?;
        // OTTO_DATA_DIR may be relative. Every persisted/spawned path must use
        // the same absolute identity so a subsequent daemon can recover it.
        let absolute_dir = tokio::fs::canonicalize(dir)
            .await
            .context("resolve telemetry directory")?;
        let dir = absolute_dir.as_path();
        let owner_lock = acquire_owner_lock(dir).await?;
        recover_unlocked(dir, owner_lock.clone()).await?;
        let bin = install(dir).await?;
        recover_unlocked(dir, owner_lock.clone()).await?;
        let health_port = port()?;
        let metrics_port = port()?;
        let port = port()?;
        let endpoint = format!("http://127.0.0.1:{port}");
        let mut exporters = serde_json::Map::new();
        for (signal, days) in [
            ("traces", config.traces_days),
            ("logs", config.logs_days),
            ("metrics", config.metrics_days),
        ] {
            exporters.insert(format!("clickhouse/{signal}"),json!({
                "endpoint":clickhouse,"database":"otto_telemetry","create_schema":true,
                // The daemon hands over minutes of buffered records at once;
                // server-side async inserts + large batches keep part count low.
                "ttl":format!("{}h",days*24),"async_insert":true,"compress":"none","timeout":"10s",
                "connection_params":{"max_open_conns":"2","max_idle_conns":"1"},
                "sending_queue":{"enabled":true,"num_consumers":1,"queue_size":16384,"sizer":"items","batch":{"min_size":2048,"max_size":8192,"flush_timeout":"10s","sizer":"items"}},
                "retry_on_failure":{"enabled":true,"initial_interval":"1s","max_interval":"10s","max_elapsed_time":"30s"}
            }));
        }
        let config = json!({
            "receivers":{"otlp":{"protocols":{"http":{"endpoint":format!("127.0.0.1:{port}"),"max_request_body_size":1048576}}}},
            "processors":{"memory_limiter":{"check_interval":"1s","limit_mib":192,"spike_limit_mib":32}},
            "exporters":exporters,
            "extensions":{"health_check":{"endpoint":format!("127.0.0.1:{health_port}")}},
            "service":{"extensions":["health_check"],"telemetry":{"logs":{"level":"warn","encoding":"json"},"metrics":{"level":"normal","readers":[{"pull":{"exporter":{"prometheus":{"host":"127.0.0.1","port":metrics_port}}}}]}},"pipelines":{
                "traces":{"receivers":["otlp"],"processors":["memory_limiter"],"exporters":["clickhouse/traces"]},
                "logs":{"receivers":["otlp"],"processors":["memory_limiter"],"exporters":["clickhouse/logs"]},
                "metrics":{"receivers":["otlp"],"processors":["memory_limiter"],"exporters":["clickhouse/metrics"]}
            }}
        });
        let path = dir.join("collector.json");
        tokio::fs::write(&path, serde_json::to_vec(&config)?).await?;
        let log = open_log(dir).await?;
        let child = Command::new(bin)
            .arg("--config")
            .arg(path)
            .env("GOMAXPROCS", "2")
            .env("GOMEMLIMIT", GOMEMLIMIT)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .context("start telemetry collector")?;
        let mut collector = Self {
            child,
            _owner_lock: Some(owner_lock),
            endpoint,
            health: format!("http://127.0.0.1:{health_port}"),
            metrics: format!("http://127.0.0.1:{metrics_port}/metrics"),
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(1))
            .build()?;
        for _ in 0..60 {
            if !collector.alive() {
                bail!("collector exited before readiness; verify the local ClickHouse engine is available");
            }
            if client
                .get(&collector.health)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                return Ok(collector);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        collector.stop().await;
        bail!("collector readiness timed out")
    }
}
/// `telemetry/collector.log`, rotated to `collector.log.1` once it exceeds
/// [`LOG_LIMIT`]; at most two bounded files ever exist.
async fn open_log(dir: &Path) -> Result<std::fs::File> {
    let path = dir.join("collector.log");
    if tokio::fs::metadata(&path)
        .await
        .is_ok_and(|m| m.len() > LOG_LIMIT)
    {
        tokio::fs::rename(&path, dir.join("collector.log.1")).await?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).context("open collector log")
}
fn port() -> Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?
        .local_addr()?
        .port())
}
fn distribution() -> Result<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok((
            "darwin_arm64",
            "d5e11974d2e4adac3cc001a25137aad52f6e77ec3034ba7735cac7c219a5f97a",
        )),
        ("macos", "x86_64") => Ok((
            "darwin_amd64",
            "91a6e7a0f5e1981986c897db4535de5ba8d7fc45d4205809aecb9ebebf68c71c",
        )),
        ("linux", "x86_64") => Ok((
            "linux_amd64",
            "fcc063749f730f8c21fe29f2d340ff174f5f1c5885bd3156fb6c985a3036fcc3",
        )),
        ("linux", "aarch64") => Ok((
            "linux_arm64",
            "ecf6a917e2b53beb1703b19a84bc4e112a2fcaba549639cf6b0ba58446bfd7d2",
        )),
        _ => bail!("managed collector is unavailable on this platform"),
    }
}
/// Hashes are pinned from the matching release's per-archive SHA256 assets.
async fn install(dir: &Path) -> Result<std::path::PathBuf> {
    let (platform, checksum) = distribution()?;
    let install_dir = dir.join(format!("collector-{VERSION}"));
    tokio::fs::create_dir_all(&install_dir).await?;
    let bin = install_dir.join("otelcol-contrib");
    let stamp = install_dir.join("binary.sha256");
    if bin.is_file() && stamp.is_file() {
        let path = bin.clone();
        let digest = tokio::task::spawn_blocking(move || hash_file(&path)).await??;
        if tokio::fs::read_to_string(&stamp).await? == digest {
            return Ok(bin);
        }
    }
    let archive = install_dir.join("download.tar.gz");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(300))
        .build()?;
    let url=format!("https://github.com/open-telemetry/opentelemetry-collector-releases/releases/download/v{VERSION}/otelcol-contrib_{VERSION}_{platform}.tar.gz");
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut digest = Sha256::new();
    let mut bytes = 0usize;
    while let Some(chunk) = response.chunk().await? {
        bytes += chunk.len();
        if bytes > 400 * 1024 * 1024 {
            bail!("collector archive exceeds download limit");
        }
        digest.update(&chunk);
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    if hex::encode(digest.finalize()) != checksum {
        bail!("collector checksum mismatch; refusing to execute download");
    }
    // Extract exactly the signed distribution binary, never arbitrary archive paths.
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&install_dir)
        .arg("otelcol-contrib")
        .status()
        .await?;
    if !status.success() {
        bail!("collector archive extraction failed");
    }
    let path = bin.clone();
    let digest = tokio::task::spawn_blocking(move || hash_file(&path)).await??;
    tokio::fs::write(stamp, digest).await?;
    tokio::fs::remove_file(archive).await?;
    Ok(bin)
}
fn hash_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    Ok(hex::encode(digest.finalize()))
}

/// Reconcile only this directory's exact managed executable/config pair. The
/// scan and bounded waits run off the async executor, including when collection
/// is disabled after a crash. No PID file is needed between spawn and shutdown.
pub(crate) async fn recover_orphans(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let absolute_dir = tokio::fs::canonicalize(dir)
        .await
        .context("resolve telemetry directory")?;
    let owner_lock = acquire_owner_lock(&absolute_dir).await?;
    recover_unlocked(&absolute_dir, owner_lock).await
}
async fn recover_unlocked(dir: &Path, owner_lock: Arc<std::fs::File>) -> Result<()> {
    let dir = dir.to_owned();
    // spawn_blocking cannot be canceled once running. Its closure owns the
    // lock until all identity checks/signals finish, even if this future drops.
    tokio::task::spawn_blocking(move || {
        let _owner_lock = owner_lock;
        recover_orphans_blocking(&dir)
    })
    .await?
}
async fn acquire_owner_lock(dir: &Path) -> Result<Arc<std::fs::File>> {
    let path = dir.join("collector-owner.lock");
    tokio::task::spawn_blocking(move || {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .context("open collector ownership lock")?;
        file.try_lock().map_err(|_| {
            anyhow::anyhow!("managed collector belongs to a live owner; refusing duplicate startup")
        })?;
        Ok(Arc::new(file))
    })
    .await?
}

#[derive(Clone)]
struct ManagedIdentity {
    executable: PathBuf,
    config: PathBuf,
    uid: Uid,
}
#[derive(Clone, Copy)]
struct Candidate {
    pid: Pid,
    start_time: u64,
}
#[derive(Debug, PartialEq)]
enum Ownership {
    Unrelated,
    Managed,
    Ambiguous,
}

fn refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::Always)
        .with_cmd(UpdateKind::Always)
        .with_user(UpdateKind::Always)
        .without_tasks()
}
fn canonical_argument(path: &Path) -> Option<PathBuf> {
    // Config can have been removed after a crash. Resolve its parent then
    // append exactly one filename, never a relative path from another process.
    if !path.is_absolute() {
        return None;
    }
    path.canonicalize()
        .ok()
        .or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?)))
}
fn ownership(process: &Process, identity: &ManagedIdentity) -> Ownership {
    if matches!(process.status(), ProcessStatus::Zombie) {
        return Ownership::Unrelated;
    }
    let Some(executable) = process.exe().and_then(|path| path.canonicalize().ok()) else {
        return Ownership::Unrelated;
    };
    if executable != identity.executable {
        return Ownership::Unrelated;
    }
    let Some(uid) = process.user_id() else {
        return Ownership::Ambiguous;
    };
    if uid != &identity.uid {
        return Ownership::Unrelated;
    }
    let args = process.cmd();
    if args.len() != 3 || args[1] != "--config" {
        return Ownership::Ambiguous;
    }
    match canonical_argument(Path::new(&args[2])) {
        Some(config) if config == identity.config => Ownership::Managed,
        Some(_) => Ownership::Unrelated,
        None => Ownership::Ambiguous,
    }
}
fn orphaned(process: &Process, _system: &System) -> bool {
    // Both supported Unix platforms reparent an orphan to init/launchd. A
    // missing parent in a process scan alone can also mean denied inspection;
    // never treat that ambiguity (or an unknown subreaper) as permission to kill.
    process.parent().is_some_and(|parent| parent.as_u32() == 1)
}

fn managed_identity(dir: &Path) -> Result<Option<ManagedIdentity>> {
    let executable = dir
        .join(format!("collector-{VERSION}"))
        .join("otelcol-contrib");
    if !executable.exists() {
        return Ok(None);
    }
    let managed_dir = dir
        .canonicalize()
        .context("resolve managed telemetry directory")?;
    let executable = executable
        .canonicalize()
        .context("resolve managed collector executable")?;
    if !executable.starts_with(&managed_dir) {
        bail!("managed collector executable escapes its telemetry directory");
    }
    let config = managed_dir.join("collector.json");
    let config = canonical_argument(&config).context("resolve managed collector configuration")?;
    let mut system = System::new();
    let me = Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[me]), true, refresh_kind());
    let uid = system
        .process(me)
        .and_then(|process| process.user_id())
        .cloned()
        .context("cannot verify collector process ownership")?;
    Ok(Some(ManagedIdentity {
        executable,
        config,
        uid,
    }))
}
fn snapshot(identity: &ManagedIdentity) -> Result<Vec<Candidate>> {
    let mut system = System::new();
    // Inspect command arguments only after UID and executable match. Unrelated
    // agent processes may carry prompts or credentials in their arguments.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_exe(UpdateKind::Always)
            .with_user(UpdateKind::Always)
            .without_tasks(),
    );
    let possible: Vec<_> = system
        .processes()
        .iter()
        .filter_map(|(pid, process)| {
            ((process.user_id().is_none() || process.user_id() == Some(&identity.uid))
                && process
                    .exe()
                    .and_then(|path| path.canonicalize().ok())
                    .is_some_and(|path| path == identity.executable))
            .then_some(*pid)
        })
        .collect();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&possible), true, refresh_kind());
    let mut candidates = Vec::new();
    for (pid, process) in system.processes() {
        match ownership(process, identity) {
            Ownership::Unrelated => {}
            Ownership::Ambiguous => {
                bail!("managed collector ownership is ambiguous; refusing automatic recovery")
            }
            Ownership::Managed => {
                if !orphaned(process, &system) {
                    bail!("managed collector belongs to a live owner; refusing duplicate startup");
                }
                if candidates.len() >= 8 {
                    bail!("too many orphaned collectors for bounded automatic recovery");
                }
                candidates.push(Candidate {
                    pid: *pid,
                    start_time: process.start_time(),
                });
            }
        }
    }
    Ok(candidates)
}
/// Return true only when this exact process incarnation was still an orphan
/// immediately before the signal. A reused PID or changed command is untouched.
fn revalidate_and_signal(
    identity: &ManagedIdentity,
    candidate: Candidate,
    signal: Option<Signal>,
) -> Result<bool> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[candidate.pid]),
        true,
        refresh_kind(),
    );
    let Some(process) = system.process(candidate.pid) else {
        return Ok(false);
    };
    if process.start_time() != candidate.start_time
        || ownership(process, identity) != Ownership::Managed
    {
        return Ok(false);
    }
    if !orphaned(process, &system) {
        bail!("collector ownership changed during recovery; no signal sent");
    }
    if let Some(signal) = signal {
        if process.kill_with(signal) != Some(true) {
            bail!("could not stop orphaned managed collector");
        }
    }
    Ok(true)
}
fn recover_orphans_blocking(dir: &Path) -> Result<()> {
    let Some(identity) = managed_identity(dir)? else {
        return Ok(());
    };
    // Validate every candidate before touching any process. A live owner makes
    // the entire startup ambiguous, even if another orphan also exists.
    let candidates = snapshot(&identity)?;
    for candidate in candidates {
        if !revalidate_and_signal(&identity, candidate, Some(Signal::Term))? {
            continue;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && revalidate_and_signal(&identity, candidate, None)? {
            std::thread::sleep(Duration::from_millis(50));
        }
        if revalidate_and_signal(&identity, candidate, Some(Signal::Kill))? {
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline && revalidate_and_signal(&identity, candidate, None)? {
                std::thread::sleep(Duration::from_millis(50));
            }
            if revalidate_and_signal(&identity, candidate, None)? {
                bail!("orphaned collector did not exit within recovery deadline");
            }
        }
    }
    // Detect another active daemon starting while the cleanup was in progress.
    if !snapshot(&identity)?.is_empty() {
        bail!("collector appeared during recovery; retry startup");
    }
    Ok(())
}

#[cfg(test)]
#[path = "collector_recovery_tests.rs"]
mod recovery_tests;
