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
use otto_usage::{ClickHouse, UsageEngine};
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
const BATCH_LIMIT: usize = 512;
/// Per-minute resource maxima held between flushes (3 processes × ~22 h).
const POINT_LIMIT: usize = 4096;
/// Spike / profile log records held between flushes.
const LOG_LIMIT: usize = 256;
/// Buffered signals are handed to a short-lived collector this often, under a
/// BACKGROUND ClickHouse lease (no wake, no idle-clock reset), so telemetry
/// never keeps the idle-stopped engine resident on its own.
const FLUSH_INTERVAL: Duration = Duration::from_secs(5 * 60);
/// While ClickHouse is idle-stopped, data waits at most this long (or until
/// the span queue is under pressure) before one flush wakes the engine.
const MAX_DEFER: Duration = Duration::from_secs(30 * 60);
/// Retry a failed flush after this long instead of hot-looping.
/// Minimum age of the last flush before a telemetry read requests another.
/// Every flush spawns a collector (a 363 MB Go binary living ≥ DRAIN_WAIT),
/// so a Telemetry page polling every 15 s must not pull one per ~45 s
/// (S9-04): two minutes keeps the view reasonably fresh at a fraction of it.
const READ_REFRESH_AFTER: Duration = Duration::from_secs(120);
const FLUSH_BACKOFF: Duration = Duration::from_secs(60);
/// A failed scheduled analysis is retried after this long.
const ANALYSIS_BACKOFF: Duration = Duration::from_secs(15 * 60);
/// The first analysis runs this soon after telemetry is enabled (then on the
/// configured interval), so a new user sees results the same session.
const FIRST_ANALYSIS_AFTER: i64 = 3600;
/// Attempts per OTLP POST before the flush gives up and re-queues its data.
const SEND_ATTEMPTS: u32 = 3;
/// Exporter batch flush_timeout + slack: how long a flush waits for the
/// collector to write its last partial batch before it is stopped.
const DRAIN_WAIT: Duration = Duration::from_secs(11);
const DRAIN_DEADLINE: Duration = Duration::from_secs(25);
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
/// Resource points taken out of the buffer for one export. Whatever was not
/// acknowledged (`points[sent..]`) is merged back on drop — failure or
/// cancellation alike (S9-08).
struct PendingPoints<'a> {
    buffer: &'a Mutex<BTreeMap<(i64, String), ResourcePoint>>,
    dropped: &'a AtomicU64,
    points: Vec<ResourcePoint>,
    sent: usize,
}
impl Drop for PendingPoints<'_> {
    fn drop(&mut self) {
        if self.sent >= self.points.len() {
            return;
        }
        if let Ok(mut buffer) = self.buffer.lock() {
            for point in &self.points[self.sent..] {
                merge_point(&mut buffer, point, self.dropped);
            }
        }
    }
}
/// How one flush's records ended up, judged from the collector's exporter
/// counters at the end of the drain (S9-03).
#[derive(Debug, PartialEq)]
struct Settlement {
    /// Accepted spans confirmed written.
    exported: u64,
    /// Records the collector gave up on or still held when it was stopped.
    failed: u64,
    error: Option<String>,
}
/// A drain is only CONFIRMED when the exporter counters were readable and
/// every sending queue was empty: records still queued at the deadline die
/// with the collector's SIGKILL and never reach `send_failed`, and missing
/// counters prove nothing. Either way nothing accepted counts as exported.
fn settle(stats: Option<collector::ExporterStats>, accepted: u64) -> Settlement {
    match stats {
        None => Settlement {
            exported: 0,
            failed: accepted,
            error: Some(
                "Telemetry collector stopped before confirming its export; buffered records may be lost. See telemetry/collector.log.".into(),
            ),
        },
        Some(s) if s.queue_size > 0 => Settlement {
            exported: 0,
            failed: s.send_failed + s.queue_size,
            error: Some(format!(
                "Collector stopped with {} telemetry records still queued for ClickHouse (slow or waking engine); see telemetry/collector.log.",
                s.queue_size
            )),
        },
        // Queues empty but not every accepted span was written or given up
        // on (S9-307): a batch was still mid-retry when the collector stopped.
        Some(s) if s.sent_spans + s.failed_spans < accepted => {
            let unsettled = accepted - s.sent_spans - s.failed_spans;
            Settlement {
                exported: s.sent_spans.min(accepted),
                failed: accepted - s.sent_spans.min(accepted),
                error: Some(format!(
                    "Collector stopped with {unsettled} telemetry records still being retried against ClickHouse; see telemetry/collector.log."
                )),
            }
        }
        Some(s) if s.send_failed > 0 => Settlement {
            exported: accepted.saturating_sub(s.send_failed),
            failed: s.send_failed,
            error: Some(format!(
                "Collector could not write {} telemetry records to ClickHouse; see telemetry/collector.log.",
                s.send_failed
            )),
        },
        Some(_) => Settlement {
            exported: accepted,
            failed: 0,
            error: None,
        },
    }
}
/// Whether a drain may stop: nothing queued and every accepted span settled.
fn drained(stats: &collector::ExporterStats, accepted: u64) -> bool {
    stats.queue_size == 0 && stats.sent_spans + stats.failed_spans >= accepted
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
/// The collector and the ClickHouse lease live only inside one flush (see
/// [`FLUSH_INTERVAL`]); between flushes nothing telemetry-owned is resident.
#[derive(Default)]
struct Runtime {
    /// Engine + config revision the owned rollup schema was last ensured for.
    schema: Option<(Arc<ClickHouse>, u64)>,
    next_start: Option<Instant>,
    analysis_retry: Option<Instant>,
}
impl Runtime {
    async fn stop(&mut self) {
        self.schema = None;
        self.next_start = None;
    }
}
pub struct TelemetryService {
    usage: Arc<UsageEngine>,
    dir: PathBuf,
    config: RwLock<TelemetryConfig>,
    enabled: AtomicBool,
    revision: AtomicU64,
    queue: Mutex<VecDeque<SpanRecord>>,
    /// Per-minute, per-process resource maxima awaiting the next flush (the
    /// rollup keeps per-minute maxima, so nothing it reads is lost).
    points: Mutex<BTreeMap<(i64, String), ResourcePoint>>,
    /// Pre-encoded OTLP log bodies (resource spikes, profile markers).
    pending_logs: Mutex<VecDeque<Value>>,
    last_flush: Mutex<Option<Instant>>,
    last_flush_at: Mutex<Option<i64>>,
    flush_requested: AtomicBool,
    collector_failed: AtomicU64,
    collector_queue: AtomicU64,
    /// Set once an orphan recovery succeeded (at start, or by the first
    /// collector start after a failed one) — later starts skip the scans.
    orphans_recovered: AtomicBool,
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
            points: Mutex::new(BTreeMap::new()),
            pending_logs: Mutex::new(VecDeque::new()),
            last_flush: Mutex::new(None),
            last_flush_at: Mutex::new(None),
            flush_requested: AtomicBool::new(false),
            collector_failed: AtomicU64::new(0),
            collector_queue: AtomicU64::new(0),
            orphans_recovered: AtomicBool::new(recovery_error.is_none()),
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
            buffered_samples: self.points.lock().unwrap().len()
                + self.pending_logs.lock().unwrap().len(),
            dropped: self.dropped.load(Ordering::Relaxed),
            exported: self.exported.load(Ordering::Relaxed),
            collector_send_failed: self.collector_failed.load(Ordering::Relaxed),
            collector_queue_size: self.collector_queue.load(Ordering::Relaxed),
            last_flush_at: *self.last_flush_at.lock().unwrap(),
            last_error: self.error.read().unwrap().clone(),
            last_analysis_at: last,
            next_analysis_at: c.enabled.then(|| self.next_analysis_at(&c)),
            native_profiling_supported: cfg!(target_os = "macos"),
        }
    }
    /// The first pass runs [`FIRST_ANALYSIS_AFTER`] after enabling (capped by
    /// the interval); every later pass follows the configured interval.
    fn next_analysis_at(&self, c: &TelemetryConfig) -> i64 {
        let state = self.persisted.read().unwrap();
        let interval = i64::from(c.analysis_interval_hours) * 3600;
        match state.last_analysis_at {
            Some(last) => last + interval,
            None => {
                state.schedule_started_at.unwrap_or_else(now_seconds)
                    + FIRST_ANALYSIS_AFTER.min(interval)
            }
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
            self.clear_buffers();
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
    fn clear_buffers(&self) {
        let mut q = self.queue.lock().unwrap();
        self.dropped.fetch_add(q.len() as u64, Ordering::Relaxed);
        q.clear();
        self.points.lock().unwrap().clear();
        self.pending_logs.lock().unwrap().clear();
    }
    /// Ask the worker to flush on its next tick, waking ClickHouse if needed.
    pub fn request_flush(&self) {
        self.flush_requested.store(true, Ordering::SeqCst);
    }
    /// A telemetry READ (which queries, so wakes ClickHouse anyway) pulls the
    /// buffered data forward, so the view catches up within seconds instead
    /// of the next scheduled flush. Rate-limited so a polling page cannot
    /// spawn a collector per poll.
    fn refresh_on_read(&self) {
        if self.enabled()
            && self
                .last_flush
                .lock()
                .unwrap()
                .is_none_or(|t| t.elapsed() >= READ_REFRESH_AFTER)
        {
            self.request_flush();
        }
    }
    fn push_log(&self, body: Value) {
        let mut logs = self.pending_logs.lock().unwrap();
        if logs.len() >= LOG_LIMIT {
            logs.pop_front();
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        logs.push_back(body);
    }
    /// Keep the per-minute maximum of each resource gauge (what the
    /// `resource_minutes` rollup stores), bounded by [`POINT_LIMIT`].
    fn buffer_point(&self, point: &ResourcePoint) {
        merge_point(&mut self.points.lock().unwrap(), point, &self.dropped);
    }
    /// Put spans that a failed flush could not deliver back at the front.
    fn requeue(&self, spans: Vec<SpanRecord>) {
        let mut queue = self.queue.lock().unwrap();
        if !self.enabled() {
            self.dropped
                .fetch_add(spans.len() as u64, Ordering::Relaxed);
            return;
        }
        for span in spans.into_iter().rev() {
            if queue.len() >= QUEUE_LIMIT {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            queue.push_front(span);
        }
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
    /// Open the ingestion gate without starting a collector (tests in
    /// dependent crates; never downloads or spawns anything).
    #[doc(hidden)]
    pub fn enable_ingestion_for_tests(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
    #[doc(hidden)]
    pub fn queued_spans_for_tests(&self) -> Vec<SpanRecord> {
        self.queue.lock().unwrap().iter().cloned().collect()
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
        self.refresh_on_read();
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
        // Self time is best effort: without it ranking falls back to the
        // inclusive total (still collapsing candidates that share a trace).
        let self_ms = self
            .usage
            .query_rows(&schema::self_times(&c))
            .await
            .map(|rows| {
                rows.iter()
                    .map(|r| {
                        (
                            (string(r, "component"), string(r, "name")),
                            number(r, "self_ms"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut suggestions = analysis::rank(&operations, &self_ms, &c, now);
        let spike_rows=self.usage.query_rows(&format!("SELECT process,countMerge(calls) count,maxMerge(cpu) cpu,maxMerge(rss) rss FROM otto_telemetry.spike_minutes WHERE minute>=now()-INTERVAL {} HOUR GROUP BY process LIMIT 8 SETTINGS max_execution_time=5,max_memory_usage=67108864",c.analysis_window_hours.min(c.logs_days*24))).await?;
        for row in spike_rows {
            let component = string(&row, "process");
            suggestions.push(Suggestion{id:analysis::id("resource",&component,"resource.spike"),kind:"resource".into(),component,name:"resource.spike".into(),count:number(&row,"count")as u64,p50_ms:0.0,p95_ms:0.0,max_ms:0.0,total_ms:0.0,self_ms:None,threshold_ms:0.0,window_hours:c.analysis_window_hours.min(c.logs_days*24),observed_at:now,action:"Compare measured process CPU and RSS around this spike. If it recurs in the daemon, capture a native profile and inspect the sampled functions.".into(),trace_id:String::new(),dismissed:false,peak_cpu_percent:Some(number(&row,"cpu")),peak_rss_mb:Some(number(&row,"rss"))});
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
        self.refresh_on_read();
        let rows = self
            .usage
            .query_rows(&schema::trace(id, self.config().traces_days))
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let attrs = r["SpanAttributes"].as_object();
                SpanRecord {
                    trace_id: string(&r, "TraceId"),
                    span_id: string(&r, "SpanId"),
                    parent_span_id: (!string(&r, "ParentSpanId").is_empty())
                        .then(|| string(&r, "ParentSpanId")),
                    name: string(&r, "SpanName")
                        .trim_end_matches(otlp::CANCELLED_SUFFIX)
                        .to_owned(),
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
                    status: span_status(
                        &string(&r, "StatusCode"),
                        attrs
                            .and_then(|a| a.get("otto.status"))
                            .and_then(Value::as_str),
                    )
                    .into(),
                    attributes: attrs
                        .map(|a| {
                            a.iter()
                                .filter(|(k, _)| {
                                    !matches!(k.as_str(), "otto.component" | "otto.status")
                                })
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
        self.push_log(otlp::profile(&profile));
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
        self.clear_buffers();
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
            // A config change cancels an in-flight flush: its collector is
            // killed on drop and the lease released with it.
            tokio::select! {
                _=self.changed.notified()=>{},
                result=self.tick()=>{
                    if let Err(error) = result {
                        if self.enabled() {
                            #[cfg(test)] eprintln!("telemetry tick error: {error:?}");
                            #[cfg(not(test))] let _ = error;
                            *self.error.write().unwrap()=Some("Local telemetry export failed; buffered data is kept and retried automatically. Check the embedded ClickHouse engine and telemetry/collector.log.".into());
                            self.ready.store(false,Ordering::Relaxed);
                            self.runtime.lock().await.next_start=Some(Instant::now()+FLUSH_BACKOFF);
                        }
                    }
                }
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
        self.sample(&c).await?;
        if !self.enabled() {
            return Ok(());
        }
        let (due, must_wake) = self.flush_due();
        let backoff = self
            .runtime
            .lock()
            .await
            .next_start
            .is_some_and(|when| when > Instant::now());
        if due && !backoff {
            self.flush(&c, must_wake).await?;
        }
        let retry = self.runtime.lock().await.analysis_retry;
        if now_seconds() >= self.next_analysis_at(&c)
            && retry.is_none_or(|when| when <= Instant::now())
        {
            // A failed analysis is not an export failure: never restart the
            // pipeline for it, retry later and surface it in status.
            if self.analyze().await.is_err() && self.enabled() {
                self.runtime.lock().await.analysis_retry = Some(Instant::now() + ANALYSIS_BACKOFF);
                *self.error.write().unwrap() =
                    Some("Scheduled telemetry analysis failed; retrying later.".into());
            }
        }
        Ok(())
    }
    /// Sample process resources into the per-minute buffer. Never touches
    /// ClickHouse: a parked engine has no pid and is simply not sampled.
    async fn sample(&self, c: &TelemetryConfig) -> Result<()> {
        let due = self.last_sample.lock().unwrap().is_none_or(|last| {
            last.elapsed() >= Duration::from_secs(c.sample_interval_secs.into())
        });
        if !due {
            return Ok(());
        }
        *self.last_sample.lock().unwrap() = Some(Instant::now());
        let mut processes = vec![("daemon".into(), std::process::id())];
        if let Some(pid) = self.usage.clickhouse().and_then(|ch| ch.server_pid()) {
            processes.push(("clickhouse".into(), pid));
        }
        let sampler = self.sampler.clone();
        let points =
            tokio::task::spawn_blocking(move || sampler.lock().unwrap().sample(&processes)).await?;
        if !self.enabled() {
            return Ok(());
        }
        for point in &points {
            self.buffer_point(point);
            if self.spikes.lock().unwrap().observe(point, c) {
                self.push_log(otlp::spike(point));
            }
        }
        Ok(())
    }
    /// `(due, must_wake)`: flush every [`FLUSH_INTERVAL`] when anything is
    /// buffered; wake a parked engine only on request, on the first flush
    /// after enabling, after [`MAX_DEFER`], or under span-queue pressure.
    fn flush_due(&self) -> (bool, bool) {
        let queued = self.queue.lock().unwrap().len();
        let has_data = queued > 0
            || !self.points.lock().unwrap().is_empty()
            || !self.pending_logs.lock().unwrap().is_empty();
        let requested = self.flush_requested.load(Ordering::SeqCst);
        let since = self.last_flush.lock().unwrap().map(|t| t.elapsed());
        flush_decision(has_data, requested, since, queued)
    }
    async fn flush(&self, c: &TelemetryConfig, must_wake: bool) -> Result<()> {
        let ch = self.usage.telemetry_clickhouse()?;
        let lease = match ch.try_keep_awake_background() {
            Some(lease) => lease,
            None if must_wake => ch.wake_background().await?,
            // Parked and nothing urgent: keep buffering, let it stay stopped.
            None => return Ok(()),
        };
        self.flush_requested.store(false, Ordering::SeqCst);
        let revision = self.revision.load(Ordering::SeqCst);
        self.ensure_private_dir().await?;
        let recover = !self.orphans_recovered.load(Ordering::Relaxed);
        let mut collector =
            collector::Collector::start(&self.dir, &lease.endpoint, c, recover).await?;
        self.orphans_recovered.store(true, Ordering::Relaxed);
        let fresh = self
            .runtime
            .lock()
            .await
            .schema
            .as_ref()
            .is_some_and(|(old, rev)| Arc::ptr_eq(old, &ch) && *rev == revision);
        // Spans the collector ACCEPTED this run. They count as exported only
        // once the drain confirms the collector wrote them (S9-03).
        let mut accepted = 0u64;
        let result = async {
            if !fresh {
                schema::ensure(&ch, c).await?;
                self.runtime.lock().await.schema = Some((ch.clone(), revision));
            }
            self.export(&collector.endpoint, &mut accepted).await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        let stats = self.drain(&mut collector, accepted).await;
        collector.stop().await;
        drop(lease);
        let settled = settle(stats, accepted);
        self.exported.fetch_add(settled.exported, Ordering::Relaxed);
        self.collector_failed
            .fetch_add(settled.failed, Ordering::Relaxed);
        result?;
        *self.last_flush.lock().unwrap() = Some(Instant::now());
        *self.last_flush_at.lock().unwrap() = Some(now_seconds());
        self.ready.store(settled.error.is_none(), Ordering::Relaxed);
        match settled.error {
            // A clean, confirmed run clears an earlier error; a partial one
            // never does (it replaces it with its own).
            None => *self.error.write().unwrap() = None,
            Some(error) => *self.error.write().unwrap() = Some(error),
        }
        Ok(())
    }
    /// Hand every buffered signal to the collector. Undelivered data is put
    /// back so the next flush retries it.
    async fn export(&self, endpoint: &str, accepted: &mut u64) -> Result<()> {
        loop {
            let spans: Vec<_> = {
                let mut queue = self.queue.lock().unwrap();
                let count = queue.len().min(BATCH_LIMIT);
                queue.drain(..count).collect()
            };
            if spans.is_empty() {
                break;
            }
            let mut pending = PendingExport {
                discarded: &self.dropped,
                count: spans.len() as u64,
                accepted: false,
            };
            if let Err(error) = self
                .send_retry(endpoint, "traces", otlp::traces(&spans))
                .await
            {
                self.requeue(spans);
                pending.accepted = true;
                return Err(error);
            }
            *accepted += spans.len() as u64;
            pending.accepted = true;
            if spans.iter().any(|s| s.status == "error") {
                // The spans are delivered; a lost error-log copy is not retried.
                let _ = self
                    .send_retry(endpoint, "logs", otlp::errors(&spans))
                    .await;
            }
        }
        // Points not yet acknowledged go back on failure AND on cancellation
        // (a config change drops this future mid-send, S9-08). Re-sending a
        // chunk is harmless: the rollup keeps per-minute maxima.
        let mut points = PendingPoints {
            buffer: &self.points,
            dropped: &self.dropped,
            points: std::mem::take(&mut *self.points.lock().unwrap())
                .into_values()
                .collect(),
            sent: 0,
        };
        while points.sent < points.points.len() {
            let end = (points.sent + 1024).min(points.points.len());
            let body = otlp::metrics(&points.points[points.sent..end]);
            self.send_retry(endpoint, "metrics", body).await?;
            points.sent = end;
        }
        loop {
            let Some(body) = self.pending_logs.lock().unwrap().pop_front() else {
                break;
            };
            // A popped record lost to cancellation is counted, like a span.
            let mut pending = PendingExport {
                discarded: &self.dropped,
                count: 1,
                accepted: false,
            };
            if let Err(error) = self.send_retry(endpoint, "logs", body.clone()).await {
                self.pending_logs.lock().unwrap().push_front(body);
                pending.accepted = true;
                return Err(error);
            }
            pending.accepted = true;
        }
        Ok(())
    }
    /// Wait for the exporter queues to empty, the last partial batch to be
    /// written and every `accepted` span to be settled (sent or failed —
    /// S9-307: an empty queue alone does not cover a batch mid-retry), then
    /// return the run's exporter counters (if exposed).
    async fn drain(
        &self,
        collector: &mut collector::Collector,
        accepted: u64,
    ) -> Option<collector::ExporterStats> {
        let started = Instant::now();
        let mut last = None;
        while started.elapsed() < DRAIN_DEADLINE && collector.alive() {
            last = collector.exporter_stats(&self.http).await.or(last);
            if let Some(stats) = last {
                self.collector_queue
                    .store(stats.queue_size, Ordering::Relaxed);
                if drained(&stats, accepted) && started.elapsed() >= DRAIN_WAIT {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        last
    }
    /// One OTLP POST with bounded retries; a single transient failure no
    /// longer tears the pipeline down.
    async fn send_retry(&self, endpoint: &str, signal: &str, body: Value) -> Result<()> {
        let mut delay = Duration::from_millis(250);
        let mut attempt = 1;
        loop {
            match self.send(endpoint, signal, &body).await {
                Ok(()) => return Ok(()),
                Err(error) if attempt >= SEND_ATTEMPTS || !self.enabled() => return Err(error),
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay *= 4;
                    attempt += 1;
                }
            }
        }
    }
    async fn send(&self, endpoint: &str, signal: &str, body: &Value) -> Result<()> {
        if !self.enabled() {
            bail!("telemetry export canceled by opt-out");
        }
        let response = self
            .http
            .post(format!("{endpoint}/v1/{signal}"))
            .json(body)
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
/// See [`TelemetryService::flush_due`].
fn flush_decision(
    has_data: bool,
    requested: bool,
    since_last: Option<Duration>,
    queued: usize,
) -> (bool, bool) {
    let pressure = queued >= QUEUE_LIMIT * 3 / 4;
    let must_wake = requested || pressure || since_last.is_none_or(|e| e >= MAX_DEFER);
    let due = has_data
        && (requested
            || queued >= QUEUE_LIMIT / 2
            || since_last.is_none_or(|e| e >= FLUSH_INTERVAL));
    (due, must_wake)
}
fn merge_point(
    buffer: &mut BTreeMap<(i64, String), ResourcePoint>,
    point: &ResourcePoint,
    dropped: &AtomicU64,
) {
    let minute = point.timestamp - point.timestamp.rem_euclid(60);
    let entry = buffer
        .entry((minute, point.process.clone()))
        .or_insert_with(|| ResourcePoint {
            timestamp: minute,
            process: point.process.clone(),
            cpu_percent: None,
            rss_mb: None,
            host_load: None,
        });
    let max = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    entry.cpu_percent = max(entry.cpu_percent, point.cpu_percent);
    entry.rss_mb = max(entry.rss_mb, point.rss_mb);
    entry.host_load = max(entry.host_load, point.host_load);
    while buffer.len() > POINT_LIMIT {
        buffer.pop_first();
        dropped.fetch_add(1, Ordering::Relaxed);
    }
}
/// Otto status of a stored span: the exported `otto.status` attribute when
/// present, else the OTLP code (`Unset` is not "ok" — S9-05).
fn span_status(code: &str, otto: Option<&str>) -> &'static str {
    match otto {
        Some("ok") => "ok",
        Some("error") => "error",
        Some("cancelled") => "cancelled",
        Some("unset") => "unset",
        _ => match code {
            "Error" | "STATUS_CODE_ERROR" => "error",
            "Ok" | "STATUS_CODE_OK" => "ok",
            _ => "unset",
        },
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
