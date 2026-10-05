//! Local, explicitly opted-in application telemetry. All exports use OTLP through
//! a managed loopback collector, and all persisted signals use owned CH tables.
mod analysis;
mod collector;
pub mod context;
mod otlp;
mod resource;
mod schema;
#[cfg(test)]
mod tests;
mod types;
use anyhow::{bail, Context, Result};
use otto_usage::{ClickHouse, ClickHouseLease, UsageEngine};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;
pub use types::*;
const QUEUE_LIMIT: usize = 4096;
const BATCH_LIMIT: usize = 256;
/// Canceled futures still account for drained spans. An unacknowledged request
/// may have reached the collector, so this is not a claim of end-to-end loss.
struct PendingExport<'a> {
    discarded: &'a AtomicU64,
    count: u64,
    accepted: bool,
}
impl Drop for PendingExport<'_> {
    fn drop(&mut self) {
        if !self.accepted {
            self.discarded.fetch_add(self.count, Ordering::Relaxed);
        }
    }
}
#[derive(Default, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    schedule_started_at: Option<i64>,
    #[serde(default)]
    latest_profile: Option<NativeProfile>,
    #[serde(default)]
    dismissed_ids: std::collections::BTreeSet<String>,
    last_analysis_at: Option<i64>,
    suggestions: Vec<Suggestion>,
}
#[derive(Default)]
struct Runtime {
    collector: Option<collector::Collector>,
    ch: Option<Arc<ClickHouse>>,
    lease: Option<ClickHouseLease>,
    revision: u64,
    next_start: Option<Instant>,
}
impl Runtime {
    async fn stop(&mut self) {
        if let Some(mut c) = self.collector.take() {
            c.stop().await;
        }
        self.lease = None;
        self.ch = None;
    }
}
pub struct TelemetryService {
    usage: Arc<UsageEngine>,
    dir: PathBuf,
    config: RwLock<TelemetryConfig>,
    enabled: AtomicBool,
    revision: AtomicU64,
    queue: Mutex<VecDeque<SpanRecord>>,
    dropped: AtomicU64,
    exported: AtomicU64,
    ready: AtomicBool,
    error: RwLock<Option<String>>,
    persisted: RwLock<Persisted>,
    runtime: tokio::sync::Mutex<Runtime>,
    analysis_lock: tokio::sync::Mutex<()>,
    changed: Notify,
    stopped: AtomicBool,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
    sampler: Arc<Mutex<resource::Sampler>>,
    spikes: Mutex<resource::SpikeDetector>,
    last_sample: Mutex<Option<Instant>>,
    last_profile: AtomicU64,
    profile_lock: tokio::sync::Mutex<()>,
    profile_launch: Mutex<()>,
    profile_cancel: Notify,
    #[cfg(test)]
    profile_setup_hook: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
    http: reqwest::Client,
}
impl TelemetryService {
    pub async fn start(
        usage: Arc<UsageEngine>,
        dir: PathBuf,
        config: TelemetryConfig,
    ) -> Arc<Self> {
        let dir = dir.join("telemetry");
        // Recover a collector orphaned by an unclean daemon exit even when the
        // saved consent switch is off. Failures remain visible in status.
        let recovery_error=collector::recover_orphans(&dir).await.err().map(|_|"Could not recover managed telemetry collector: another owner is active or ownership could not be verified".to_owned());
        let config = if config.validate().is_ok() {
            config
        } else {
            TelemetryConfig::default()
        };
        let mut persisted: Persisted =
            match read_bounded(&dir.join("analysis.json"), 2 * 1024 * 1024).await {
                Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
                Err(_) => Persisted::default(),
            };
        if config.enabled && persisted.schedule_started_at.is_none() {
            persisted.schedule_started_at = Some(now_seconds());
        }
        let service = Arc::new(Self {
            usage,
            dir,
            enabled: AtomicBool::new(config.enabled),
            config: RwLock::new(config),
            revision: AtomicU64::new(1),
            queue: Mutex::new(VecDeque::new()),
            dropped: AtomicU64::new(0),
            exported: AtomicU64::new(0),
            ready: AtomicBool::new(false),
            error: RwLock::new(recovery_error),
            persisted: RwLock::new(persisted),
            runtime: tokio::sync::Mutex::new(Runtime::default()),
            analysis_lock: tokio::sync::Mutex::new(()),
            changed: Notify::new(),
            stopped: AtomicBool::new(false),
            worker: Mutex::new(None),
            sampler: Arc::new(Mutex::new(resource::Sampler::new())),
            spikes: Mutex::new(resource::SpikeDetector::default()),
            last_sample: Mutex::new(None),
            last_profile: AtomicU64::new(0),
            profile_lock: tokio::sync::Mutex::new(()),
            profile_launch: Mutex::new(()),
            profile_cancel: Notify::new(),
            #[cfg(test)]
            profile_setup_hook: Mutex::new(None),
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()
                .expect("local telemetry HTTP client"),
        });
        if service.enabled() {
            let _lock = service.analysis_lock.lock().await;
            if service.save().await.is_err() {
                *service.error.write().unwrap() =
                    Some("Could not persist telemetry analysis schedule".into());
            }
        }
        let worker = service.clone();
        *service.worker.lock().unwrap() = Some(tokio::spawn(async move {
            worker.run().await;
        }));
        service
    }
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
    pub fn config(&self) -> TelemetryConfig {
        self.config.read().unwrap().clone()
    }
    pub fn status(&self) -> TelemetryStatus {
        let c = self.config();
        let last = self.persisted.read().unwrap().last_analysis_at;
        TelemetryStatus {
            enabled: self.enabled(),
            collector_ready: self.ready.load(Ordering::Relaxed),
            collector_version: collector::VERSION.into(),
            queued: self.queue.lock().unwrap().len(),
            dropped: self.dropped.load(Ordering::Relaxed),
            exported: self.exported.load(Ordering::Relaxed),
            last_error: self.error.read().unwrap().clone(),
            last_analysis_at: last,
            next_analysis_at: c.enabled.then(|| {
                last.or(self.persisted.read().unwrap().schedule_started_at)
                    .unwrap_or_else(now_seconds)
                    + i64::from(c.analysis_interval_hours) * 3600
            }),
            native_profiling_supported: cfg!(target_os = "macos"),
        }
    }
    pub async fn configure(&self, config: TelemetryConfig) -> Result<()> {
        config.validate()?;
        if config.enabled {
            let _lock = self.analysis_lock.lock().await;
            if self.persisted.read().unwrap().schedule_started_at.is_none() {
                self.persisted.write().unwrap().schedule_started_at = Some(now_seconds());
                self.save().await?;
            }
        }
        {
            let _launch = self.profile_launch.lock().unwrap();
            self.enabled.store(config.enabled, Ordering::SeqCst);
            *self.config.write().unwrap() = config.clone();
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
        if !config.enabled {
            let mut q = self.queue.lock().unwrap();
            self.dropped.fetch_add(q.len() as u64, Ordering::Relaxed);
            q.clear();
        }
        self.changed.notify_one();
        if !config.enabled || !config.native_profiling {
            self.profile_cancel.notify_waiters();
        }
        if !config.enabled {
            self.runtime.lock().await.stop().await;
            self.ready.store(false, Ordering::Relaxed);
            *self.last_sample.lock().unwrap() = None;
            *self.sampler.lock().unwrap() = resource::Sampler::new();
            *self.spikes.lock().unwrap() = resource::SpikeDetector::default();
        }
        Ok(())
    }
    pub fn record(&self, span: SpanRecord) -> bool {
        if !self.enabled() {
            return false;
        }
        if span.validate().is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        let mut queue = self.queue.lock().unwrap();
        if !self.enabled() {
            return false;
        }
        if queue.len() == QUEUE_LIMIT {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        queue.push_back(span);
        true
    }
    pub fn suggestions(&self) -> Vec<Suggestion> {
        self.persisted
            .read()
            .unwrap()
            .suggestions
            .iter()
            .filter(|s| !s.dismissed)
            .cloned()
            .collect()
    }
    async fn save(&self) -> Result<()> {
        self.ensure_private_dir().await?;
        let bytes = serde_json::to_vec(&*self.persisted.read().unwrap())?;
        let pending = self.dir.join("analysis.pending.json");
        tokio::fs::write(&pending, bytes).await?;
        tokio::fs::rename(pending, self.dir.join("analysis.json")).await?;
        Ok(())
    }
    pub async fn dismiss(&self, id: &str) -> Result<()> {
        let _lock = self.analysis_lock.lock().await;
        {
            let mut state = self.persisted.write().unwrap();
            if state.dismissed_ids.len() >= 4096 && !state.dismissed_ids.contains(id) {
                bail!("telemetry dismissal history is full");
            }
            let Some(s) = state.suggestions.iter_mut().find(|s| s.id == id) else {
                bail!("recommendation not found")
            };
            s.dismissed = true;
            state.dismissed_ids.insert(id.to_owned());
        }
        self.save().await
    }
    async fn ensure_private_dir(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700)).await?;
        }
        Ok(())
    }
    pub fn latest_profile(&self) -> Option<NativeProfile> {
        self.persisted.read().unwrap().latest_profile.clone()
    }
    pub async fn overview(&self, hours: u32) -> Result<TelemetryOverview> {
        let hours = hours.clamp(1, self.config().metrics_days * 24);
        if !self.enabled() {
            let exists=self.usage.query_rows("SELECT count() n FROM system.tables WHERE database='otto_telemetry' AND name='operation_minutes'").await;
            if !exists.is_ok_and(|rows| rows.first().is_some_and(|row| number(row, "n") > 0.0)) {
                return Ok(TelemetryOverview {
                    hours,
                    operations: Vec::new(),
                    resources: Vec::new(),
                    status: self.status(),
                });
            }
        }
        let operations = self
            .operation_rows(hours.min(self.config().traces_days * 24))
            .await?;
        let rows = self.usage.query_rows(&schema::resources(hours)).await?;
        let mut points: BTreeMap<(i64, String), ResourcePoint> = BTreeMap::new();
        for row in rows {
            let time = number(&row, "timestamp") as i64;
            let process = string(&row, "process");
            let point = points
                .entry((time, process.clone()))
                .or_insert(ResourcePoint {
                    timestamp: time,
                    process,
                    cpu_percent: None,
                    rss_mb: None,
                    host_load: None,
                });
            match string(&row, "metric").as_str() {
                "otto.process.cpu" => point.cpu_percent = Some(number(&row, "value")),
                "otto.process.rss" => point.rss_mb = Some(number(&row, "value")),
                "otto.host.load" => point.host_load = Some(number(&row, "value")),
                _ => {}
            }
        }
        Ok(TelemetryOverview {
            hours,
            operations,
            resources: points.into_values().collect(),
            status: self.status(),
        })
    }
    async fn operation_rows(&self, hours: u32) -> Result<Vec<OperationSummary>> {
        self.operation_query(&schema::operations(hours)).await
    }
    async fn operation_query(&self, sql: &str) -> Result<Vec<OperationSummary>> {
        Ok(self
            .usage
            .query_rows(sql)
            .await?
            .into_iter()
            .map(|r| OperationSummary {
                component: string(&r, "component"),
                name: string(&r, "name"),
                count: number(&r, "count") as u64,
                errors: number(&r, "errors") as u64,
                p50_ms: number(&r, "p50_ms"),
                p95_ms: number(&r, "p95_ms"),
                max_ms: number(&r, "max_ms"),
                total_ms: number(&r, "total_ms"),
                trace_id: string(&r, "trace_id"),
            })
            .collect())
    }
    pub async fn analyze(&self) -> Result<Vec<Suggestion>> {
        if !self.enabled() {
            bail!("enable telemetry before analyzing");
        }
        let _lock = self.analysis_lock.lock().await;
        let c = self.config();
        let now = now_seconds();
        let operations = self.operation_query(&schema::slow_operations(&c)).await?;
        let mut suggestions = analysis::rank(&operations, &c, now);
        let spike_rows=self.usage.query_rows(&format!("SELECT process,countMerge(calls) count,maxMerge(cpu) cpu,maxMerge(rss) rss FROM otto_telemetry.spike_minutes WHERE minute>=now()-INTERVAL {} HOUR GROUP BY process LIMIT 8 SETTINGS max_execution_time=5,max_memory_usage=67108864",c.analysis_window_hours.min(c.logs_days*24))).await?;
        for row in spike_rows {
            let component = string(&row, "process");
            suggestions.push(Suggestion{id:analysis::id("resource",&component,"resource.spike"),kind:"resource".into(),component,name:"resource.spike".into(),count:number(&row,"count")as u64,p50_ms:0.0,p95_ms:0.0,max_ms:0.0,total_ms:0.0,threshold_ms:0.0,window_hours:c.analysis_window_hours.min(c.logs_days*24),observed_at:now,action:"Compare measured process CPU and RSS around this spike. If it recurs in the daemon, capture a native profile and inspect the sampled functions.".into(),trace_id:String::new(),dismissed:false,peak_cpu_percent:Some(number(&row,"cpu")),peak_rss_mb:Some(number(&row,"rss"))});
        }
        {
            let mut state = self.persisted.write().unwrap();
            for suggestion in &mut suggestions {
                suggestion.dismissed = state.dismissed_ids.contains(&suggestion.id)
                    || state
                        .suggestions
                        .iter()
                        .any(|old| old.id == suggestion.id && old.dismissed);
            }
            state.suggestions = suggestions;
            state.last_analysis_at = Some(now);
        }
        self.save().await?;
        Ok(self.suggestions())
    }
    pub async fn trace(&self, id: &str) -> Result<Vec<SpanRecord>> {
        if !valid_id(id, 32) {
            bail!("invalid trace ID");
        }
        let rows=self.usage.query_rows(&format!("SELECT TraceId,SpanId,ParentSpanId,SpanName,SpanKind,SpanAttributes,StatusCode,toUnixTimestamp64Nano(Timestamp) started,Duration FROM otto_telemetry.otel_traces WHERE TraceId='{id}' AND Timestamp>=now()-INTERVAL {} DAY ORDER BY Timestamp LIMIT 1000 SETTINGS max_execution_time=5,max_memory_usage=67108864",self.config().traces_days)).await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let attrs = r["SpanAttributes"].as_object();
                SpanRecord {
                    trace_id: string(&r, "TraceId"),
                    span_id: string(&r, "SpanId"),
                    parent_span_id: (!string(&r, "ParentSpanId").is_empty())
                        .then(|| string(&r, "ParentSpanId")),
                    name: string(&r, "SpanName"),
                    component: attrs
                        .and_then(|a| a.get("otto.component"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    kind: string(&r, "SpanKind").to_lowercase(),
                    start_unix_nano: r["started"]
                        .as_u64()
                        .or_else(|| r["started"].as_str().and_then(|s| s.parse().ok()))
                        .unwrap_or_default(),
                    duration_ms: number(&r, "Duration") / 1e6,
                    status: if string(&r, "StatusCode") == "Error" {
                        "error"
                    } else {
                        "ok"
                    }
                    .into(),
                    attributes: attrs
                        .map(|a| {
                            a.iter()
                                .filter(|(k, _)| k.as_str() != "otto.component")
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            })
            .collect())
    }
    pub async fn profile(&self) -> Result<NativeProfile> {
        let _lock = self
            .profile_lock
            .try_lock()
            .context("a native profile is already running")?;
        if !self.enabled() || !self.config().native_profiling {
            bail!("enable telemetry and native profiling first");
        }
        if !cfg!(target_os = "macos") {
            bail!("native profiles are supported on macOS only");
        }
        let now = now_seconds() as u64;
        let last = self.last_profile.load(Ordering::Relaxed);
        if last > 0 && now.saturating_sub(last) < 60 {
            bail!("wait 60 seconds between profiles");
        }
        let revision = self.revision.load(Ordering::SeqCst);
        let cancelled = self.profile_cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        self.last_profile.store(now, Ordering::Relaxed);
        #[cfg(test)]
        {
            let hook = self.profile_setup_hook.lock().unwrap().clone();
            if let Some((arrived, resume)) = hook {
                arrived.notify_one();
                resume.notified().await;
            }
        }
        self.ensure_private_dir().await?;
        // NamedTempFile creates mode 0600 and unlinks on every error/cancellation path.
        let raw = tempfile::NamedTempFile::new_in(&self.dir)?;
        let mut child = {
            let _launch = self.profile_launch.lock().unwrap();
            if !self.enabled()
                || !self.config().native_profiling
                || self.revision.load(Ordering::SeqCst) != revision
            {
                bail!("profiling was disabled during setup");
            }
            tokio::process::Command::new("/usr/bin/sample")
                .args([
                    std::process::id().to_string(),
                    "3".into(),
                    "10".into(),
                    "-file".into(),
                ])
                .arg(raw.path())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()?
        };
        let result = tokio::select! {
            biased;
            _=&mut cancelled=>None,
            result=tokio::time::timeout(Duration::from_secs(10),child.wait())=>Some(result),
        };
        if !matches!(result,Some(Ok(Ok(status)))if status.success()) {
            let _ = child.kill().await;
            bail!("native sampling failed or timed out");
        }
        let contents = if tokio::fs::metadata(raw.path()).await?.len() <= 2 * 1024 * 1024 {
            tokio::fs::read_to_string(raw.path()).await?
        } else {
            String::new()
        };
        drop(raw);
        if !self.enabled()
            || !self.config().native_profiling
            || self.revision.load(Ordering::SeqCst) != revision
        {
            bail!("profiling was disabled during capture");
        }
        let profile = NativeProfile {
            captured_at: now as i64,
            duration_seconds: 3,
            format: "macos-sampled-stacks".into(),
            frames: resource::sanitize_profile(&contents),
        };
        if profile.frames.is_empty() {
            bail!("native sampler returned no usable function frames");
        }
        {
            let _lock = self.analysis_lock.lock().await;
            if !self.enabled() || self.revision.load(Ordering::SeqCst) != revision {
                bail!("profiling was disabled before saving");
            }
            self.persisted.write().unwrap().latest_profile = Some(profile.clone());
            self.save().await?;
        }
        // A safe metadata span links the local sampled-stack artifact to telemetry.
        self.record(SpanRecord::new("native.profile", "daemon", 3000.0));
        {
            let runtime = self.runtime.lock().await;
            if let Some(collector) = &runtime.collector {
                self.send(&collector.endpoint, "logs", otlp::profile(&profile))
                    .await?;
            }
        }
        Ok(profile)
    }
    pub async fn shutdown(&self) {
        {
            let _launch = self.profile_launch.lock().unwrap();
            self.enabled.store(false, Ordering::SeqCst);
            self.stopped.store(true, Ordering::SeqCst);
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
        self.profile_cancel.notify_waiters();
        self.changed.notify_one();
        let worker = self.worker.lock().unwrap().take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
        self.runtime.lock().await.stop().await;
        self.queue.lock().unwrap().clear();
        self.ready.store(false, Ordering::Relaxed);
    }
    async fn run(self: Arc<Self>) {
        loop {
            if self.stopped.load(Ordering::Relaxed) {
                break;
            }
            if !self.enabled() {
                self.changed.notified().await;
                continue;
            }
            tokio::select! {
                _=self.changed.notified()=>{},
                result=self.tick()=>{if result.is_err() && self.enabled(){
                    #[cfg(test)] eprintln!("telemetry tick error: {result:?}");*self.error.write().unwrap()=Some("Local telemetry pipeline unavailable; retrying automatically. Check the embedded ClickHouse engine and collector installation.".into());self.ready.store(false,Ordering::Relaxed);let mut runtime=self.runtime.lock().await;runtime.stop().await;runtime.next_start=Some(Instant::now()+Duration::from_secs(30));}}
            }
            tokio::select! {_=self.changed.notified()=>{},_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
        self.runtime.lock().await.stop().await;
    }
    async fn tick(&self) -> Result<()> {
        let c = self.config();
        if !c.enabled {
            return Ok(());
        }
        let revision = self.revision.load(Ordering::Relaxed);
        let mut runtime = self.runtime.lock().await;
        let ch = self.usage.telemetry_clickhouse()?;
        let same = runtime.ch.as_ref().is_some_and(|old| Arc::ptr_eq(old, &ch));
        let alive = runtime
            .collector
            .as_mut()
            .is_some_and(|collector| collector.alive());
        if !same || !alive || runtime.revision != revision {
            if runtime.next_start.is_some_and(|when| when > Instant::now()) {
                return Ok(());
            }
            runtime.stop().await;
            self.ready.store(false, Ordering::Relaxed);
            let lease = ch.keep_awake().await?;
            self.ensure_private_dir().await?;
            let collector = collector::Collector::start(&self.dir, &lease.endpoint, &c).await?;
            schema::ensure(&ch, &c).await?;
            runtime.collector = Some(collector);
            runtime.ch = Some(ch.clone());
            runtime.lease = Some(lease);
            runtime.revision = revision;
            runtime.next_start = None;
            self.ready.store(true, Ordering::Relaxed);
            *self.error.write().unwrap() = None;
        }
        if !self.enabled() {
            return Ok(());
        }
        let collector = runtime
            .collector
            .as_ref()
            .context("collector unavailable")?;
        let endpoint = collector.endpoint.clone();
        let spans: Vec<_> = {
            let mut queue = self.queue.lock().unwrap();
            let count = queue.len().min(BATCH_LIMIT);
            queue.drain(..count).collect()
        };
        if !spans.is_empty() {
            let mut pending = PendingExport {
                discarded: &self.dropped,
                count: spans.len() as u64,
                accepted: false,
            };
            self.send(&endpoint, "traces", otlp::traces(&spans)).await?;
            self.exported
                .fetch_add(spans.len() as u64, Ordering::Relaxed);
            pending.accepted = true;
            if spans.iter().any(|s| s.status == "error") {
                self.send(&endpoint, "logs", otlp::errors(&spans)).await?;
            }
        }
        let due = self.last_sample.lock().unwrap().is_none_or(|last| {
            last.elapsed() >= Duration::from_secs(c.sample_interval_secs.into())
        });
        if due {
            *self.last_sample.lock().unwrap() = Some(Instant::now());
            let mut processes = vec![("daemon".into(), std::process::id())];
            if let Some(pid) = collector.pid() {
                processes.push(("collector".into(), pid));
            }
            if let Some(pid) = ch.server_pid() {
                processes.push(("clickhouse".into(), pid));
            }
            let sampler = self.sampler.clone();
            let points =
                tokio::task::spawn_blocking(move || sampler.lock().unwrap().sample(&processes))
                    .await?;
            if !self.enabled() {
                return Ok(());
            }
            self.send(&endpoint, "metrics", otlp::metrics(&points))
                .await?;
            for point in &points {
                let spike = self.spikes.lock().unwrap().observe(point, &c);
                if spike {
                    self.send(&endpoint, "logs", otlp::spike(point)).await?;
                }
            }
        }
        drop(runtime);
        let last = {
            let state = self.persisted.read().unwrap();
            state.last_analysis_at.or(state.schedule_started_at)
        };
        if last
            .is_none_or(|last| now_seconds() - last >= i64::from(c.analysis_interval_hours) * 3600)
        {
            self.analyze().await?;
        }
        Ok(())
    }
    async fn send(&self, endpoint: &str, signal: &str, body: Value) -> Result<()> {
        if !self.enabled() {
            bail!("telemetry export canceled by opt-out");
        }
        let response = self
            .http
            .post(format!("{endpoint}/v1/{signal}"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let bytes = response.bytes().await?;
        if !bytes.is_empty() {
            let value: Value = serde_json::from_slice(&bytes)?;
            if let Some(partial) = value.get("partialSuccess") {
                if partial.as_object().is_some_and(|p| {
                    p.values().any(|v| {
                        v.as_u64().is_some_and(|n| n > 0)
                            || v.as_str().is_some_and(|s| !s.is_empty() && s != "0")
                    })
                }) {
                    bail!("collector rejected telemetry records");
                }
            }
        }
        Ok(())
    }
}
fn number(row: &Value, key: &str) -> f64 {
    row[key]
        .as_f64()
        .or_else(|| row[key].as_str().and_then(|s| s.parse().ok()))
        .filter(|n| n.is_finite())
        .unwrap_or_default()
}
fn string(row: &Value, key: &str) -> String {
    row[key].as_str().unwrap_or_default().to_owned()
}

async fn read_bounded(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).await?;
    if bytes.len() > limit {
        bail!("local telemetry file exceeds size limit");
    }
    Ok(bytes)
}
