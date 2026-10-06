use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub native_profiling: bool,
    pub traces_days: u32,
    pub logs_days: u32,
    pub metrics_days: u32,
    pub analysis_interval_hours: u32,
    pub analysis_window_hours: u32,
    pub suggestion_limit: u32,
    pub slow_threshold_ms: f64,
    pub min_samples: u32,
    pub sample_interval_secs: u32,
    pub cpu_spike_percent: f64,
    pub rss_spike_mb: f64,
}
impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            native_profiling: false,
            traces_days: 1,
            logs_days: 2,
            metrics_days: 7,
            analysis_interval_hours: 24,
            analysis_window_hours: 24,
            suggestion_limit: 100,
            slow_threshold_ms: 100.0,
            min_samples: 3,
            sample_interval_secs: 5,
            cpu_spike_percent: 80.0,
            rss_spike_mb: 256.0,
        }
    }
}
impl TelemetryConfig {
    pub fn validate(&self) -> Result<()> {
        if !(1..=30).contains(&self.traces_days)
            || !(1..=30).contains(&self.logs_days)
            || !(1..=90).contains(&self.metrics_days)
        {
            bail!("retention must be 1–30 days for traces/logs and 1–90 days for metrics");
        }
        if !(1..=168).contains(&self.analysis_interval_hours)
            || !(1..=self.traces_days * 24).contains(&self.analysis_window_hours)
            || !(1..=100).contains(&self.suggestion_limit)
            || !(1..=10000).contains(&self.min_samples)
            || !(2..=60).contains(&self.sample_interval_secs)
        {
            bail!("analysis interval, window, limit, sample count or sampling interval is out of range");
        }
        if !self.slow_threshold_ms.is_finite()
            || !(1.0..=60000.0).contains(&self.slow_threshold_ms)
            || !self.cpu_spike_percent.is_finite()
            || !(1.0..=10000.0).contains(&self.cpu_spike_percent)
            || !self.rss_spike_mb.is_finite()
            || !(16.0..=65536.0).contains(&self.rss_spike_mb)
        {
            bail!("telemetry thresholds are out of range");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanRecord {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default)]
    pub parent_span_id: Option<String>,
    pub name: String,
    pub component: String,
    pub kind: String,
    #[serde(deserialize_with = "deserialize_nano")]
    pub start_unix_nano: u64,
    pub duration_ms: f64,
    pub status: String,
    #[serde(default)]
    pub attributes: BTreeMap<String, Value>,
}
fn deserialize_nano<'de, D: serde::Deserializer<'de>>(de: D) -> std::result::Result<u64, D::Error> {
    let v = Value::deserialize(de)?;
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| {
            serde::de::Error::custom("nanoseconds must be a nonnegative integer or decimal string")
        })
}
impl SpanRecord {
    pub fn new(name: impl Into<String>, component: impl Into<String>, duration_ms: f64) -> Self {
        Self {
            trace_id: uuid::Uuid::new_v4().simple().to_string(),
            span_id: uuid::Uuid::new_v4().simple().to_string()[..16].to_string(),
            parent_span_id: None,
            name: name.into(),
            component: component.into(),
            kind: "internal".into(),
            start_unix_nano: now_nanos().saturating_sub((duration_ms * 1e6) as u64),
            duration_ms,
            status: "ok".into(),
            attributes: BTreeMap::new(),
        }
    }
    pub fn validate(&self) -> Result<()> {
        if !valid_id(&self.trace_id, 32)
            || !valid_id(&self.span_id, 16)
            || self
                .parent_span_id
                .as_ref()
                .is_some_and(|s| !valid_id(s, 16))
        {
            bail!("invalid trace/span ID");
        }
        if !safe_name(&self.name)
            || !safe_name(&self.component)
            || !matches!(
                self.kind.as_str(),
                "client" | "server" | "internal" | "producer" | "consumer"
            )
            || !matches!(self.status.as_str(), "ok" | "error" | "unset" | "cancelled")
        {
            bail!("invalid operation metadata");
        }
        if !self.duration_ms.is_finite() || !(0.0..=3_600_000.0).contains(&self.duration_ms) {
            bail!("invalid span duration");
        }
        let now = now_nanos();
        if self.start_unix_nano < now.saturating_sub(86_400_000_000_000)
            || self.start_unix_nano > now.saturating_add(60_000_000_000)
        {
            bail!("span timestamp outside ingestion window");
        }
        if self.attributes.len() > 12 {
            bail!("too many attributes");
        }
        for (key, val) in &self.attributes {
            let valid = match key.as_str() {
                "http.request.method" => val.as_str().is_some_and(|s| {
                    matches!(
                        s,
                        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
                    )
                }),
                "http.response.status_code" => {
                    val.as_u64().is_some_and(|n| (100..=599).contains(&n))
                }
                "http.route" => val.as_str().is_some_and(safe_route),
                "otto.lane" => val.as_str().is_some_and(|s| {
                    matches!(
                        s,
                        "interactive"
                            | "background"
                            | "default"
                            | "foreground"
                            | "read"
                            | "write"
                            | "int"
                            | "bg"
                            | "long"
                    )
                }),
                "otto.phase" => val.as_str().is_some_and(|s| {
                    matches!(
                        s,
                        "queue"
                            | "request"
                            | "decode"
                            | "render"
                            | "import"
                            | "navigation"
                            | "work"
                    )
                }),
                "otto.cpu.percent" | "otto.rss.mb" | "otto.items" => val
                    .as_f64()
                    .is_some_and(|v| v.is_finite() && (0.0..=1e9).contains(&v)),
                _ => false,
            };
            if !valid {
                bail!("attribute is not in the telemetry allowlist: {key}");
            }
        }
        Ok(())
    }
}
pub fn valid_id(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && s.bytes().any(|c| c != b'0')
}
pub fn safe_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 96
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
}
fn safe_route(s: &str) -> bool {
    s.starts_with('/')
        && s.len() <= 192
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/_-{}:*".contains(&c))
}
pub fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
pub fn now_seconds() -> i64 {
    (now_nanos() / 1_000_000_000) as i64
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryStatus {
    pub enabled: bool,
    pub collector_ready: bool,
    pub collector_version: String,
    pub queued: usize,
    /// Per-minute resource samples + spike/profile logs awaiting the next flush.
    #[serde(default)]
    pub buffered_samples: usize,
    pub dropped: u64,
    pub exported: u64,
    /// Records the collector's ClickHouse exporters gave up on (cumulative).
    #[serde(default)]
    pub collector_send_failed: u64,
    /// Exporter sending-queue depth at the end of the last flush.
    #[serde(default)]
    pub collector_queue_size: u64,
    /// When buffered telemetry was last written (unix seconds).
    #[serde(default)]
    pub last_flush_at: Option<i64>,
    pub last_error: Option<String>,
    pub last_analysis_at: Option<i64>,
    pub next_analysis_at: Option<i64>,
    pub native_profiling_supported: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OperationSummary {
    pub component: String,
    pub name: String,
    pub count: u64,
    pub errors: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
    pub trace_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourcePoint {
    pub timestamp: i64,
    pub process: String,
    pub cpu_percent: Option<f64>,
    pub rss_mb: Option<f64>,
    pub host_load: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryOverview {
    pub hours: u32,
    pub operations: Vec<OperationSummary>,
    pub resources: Vec<ResourcePoint>,
    pub status: TelemetryStatus,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub kind: String,
    pub component: String,
    pub name: String,
    pub count: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub total_ms: f64,
    /// Exclusive time (inclusive minus direct children) over the window; the
    /// ranking key, so a parent is not blamed for its children's latency.
    #[serde(default)]
    pub self_ms: Option<f64>,
    pub threshold_ms: f64,
    pub window_hours: u32,
    pub observed_at: i64,
    pub action: String,
    pub trace_id: String,
    pub dismissed: bool,
    pub peak_cpu_percent: Option<f64>,
    pub peak_rss_mb: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeProfile {
    pub captured_at: i64,
    pub duration_seconds: u32,
    pub format: String,
    pub frames: Vec<String>,
}
