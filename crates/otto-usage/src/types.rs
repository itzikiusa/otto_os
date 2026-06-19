//! Public data types: the recorded event, engine config, the dashboard query
//! results and the engine status report. All serde-serializable so they double
//! as the HTTP API DTOs and as ClickHouse `JSONEachRow` rows.

use serde::{Deserialize, Serialize};

/// One usage row. Field names match the `usage_events` columns exactly so the
/// struct serializes straight to `JSONEachRow` for insertion (the `ts` /
/// `event_date` columns are omitted and default to "now" in ClickHouse).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageEvent {
    pub workspace_id: String,
    pub session_id: String,
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// `prompt` | `completion` | `tool` | `session` | `other`.
    pub kind: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub duration_ms: u64,
}

/// Engine configuration, persisted in the daemon `settings` table under the
/// `usage` key and editable from the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageConfig {
    /// Master switch — when false the engine is a no-op recorder.
    pub enabled: bool,
    /// Days of history to keep (the `MergeTree` TTL window).
    pub retention_days: u32,
    /// How often to sample CPU/RAM into `system_metrics`.
    pub metrics_interval_secs: u64,
    /// Optional explicit path to the `clickhouse` binary (overrides discovery).
    #[serde(default)]
    pub clickhouse_path: Option<String>,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 180,
            metrics_interval_secs: 60,
            clickhouse_path: None,
        }
    }
}

impl UsageConfig {
    /// Parse from the raw `settings.usage` JSON, falling back to defaults for
    /// any missing/invalid field.
    pub fn from_json(value: Option<&serde_json::Value>) -> Self {
        match value {
            Some(v) => serde_json::from_value(v.clone()).unwrap_or_default(),
            None => Self::default(),
        }
    }
}

/// Per-provider rollup over the window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider: String,
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cached (cache-read) input tokens — the "cached" hits.
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// Cache-creation (cache-write) tokens.
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Per-day rollup over the window (one point per calendar day).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyUsage {
    pub day: String,
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// Per-session rollup over the window (top N by tokens).
///
/// The first fields come straight from ClickHouse. `title`/`kind`/`workspace_name`
/// are enriched server-side from the SQLite `sessions`/`workspaces` tables for
/// Otto-owned sessions (and stay `None` for external ones), so they are
/// `#[serde(default)]` — ClickHouse never supplies them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionUsage {
    pub session_id: String,
    pub workspace_id: String,
    pub provider: String,
    pub events: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub last_active: String,
    /// Otto session title (pane name).
    #[serde(default)]
    pub title: Option<String>,
    /// What kind of Otto work this was: "review", "product", "channel", or the
    /// session kind ("agent", "shell", …).
    #[serde(default)]
    pub kind: Option<String>,
    /// Human-readable workspace name (not the id).
    #[serde(default)]
    pub workspace_name: Option<String>,
}

/// Per-feature rollup over the window — usage grouped by the kind of Otto work
/// (review / product / channel / agent / shell / connection / external …)
/// rather than by provider. The `feature` label is derived server-side from each
/// session's SQLite metadata (see `routes::usage`), so the engine never produces
/// these directly; it only supplies the per-session sums the server folds in.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureUsage {
    /// Short feature label: "review" | "product" | "channel" | "agent" |
    /// "connection" | "external" | … (matches the session-row `kind`).
    pub feature: String,
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    /// Distinct sessions that contributed to this feature bucket.
    #[serde(default)]
    pub sessions: u64,
}

/// Per-session raw sums over the window — the unenriched, unlimited rollup the
/// server uses to build the per-feature breakdown. Same numbers as
/// [`SessionUsage`] but without the SQLite enrichment or top-N cap.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTotals {
    pub session_id: String,
    pub workspace_id: String,
    pub provider: String,
    pub events: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
}

/// One system-metrics sample, as returned to the dashboard time-series.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MetricPoint {
    pub ts: String,
    pub cpu_pct: f64,
    pub mem_used_mb: f64,
    pub mem_total_mb: f64,
    pub mem_pct: f64,
    pub load_avg_1: f64,
    pub process_rss_mb: f64,
    pub process_cpu_pct: f64,
    pub active_sessions: u32,
}

/// The full dashboard payload for a time window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageSummary {
    pub days: u32,
    pub total_events: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    #[serde(default)]
    pub total_cache_read_tokens: u64,
    #[serde(default)]
    pub total_cache_write_tokens: u64,
    pub total_tokens: u64,
    pub total_cost_usd: f64,
    pub providers: Vec<ProviderUsage>,
    pub daily: Vec<DailyUsage>,
    pub sessions: Vec<SessionUsage>,
    /// Per-feature (by-kind) rollup — review / product / channel / agent / …
    /// Built server-side by classifying sessions, so it's `#[serde(default)]`
    /// (the engine itself returns it empty until the server fills it in).
    #[serde(default)]
    pub by_kind: Vec<FeatureUsage>,
}

/// Engine + ClickHouse health/status, for the settings panel and the wizard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStatus {
    /// True when the binary was found and the schema is live.
    pub available: bool,
    pub enabled: bool,
    /// Absolute path to the resolved `clickhouse` binary, if any.
    pub binary: Option<String>,
    /// `clickhouse local --version` first line, if available.
    pub version: Option<String>,
    /// On-disk data directory.
    pub data_dir: String,
    pub retention_days: u32,
    pub metrics_interval_secs: u64,
    pub usage_rows: u64,
    pub metric_rows: u64,
    /// On-disk size of the data directory in bytes.
    pub disk_bytes: u64,
    /// Date the cost-estimation rate table was last reconciled against published
    /// list prices (see `pricing::PRICED_AS_OF`). Lets the UI flag estimates as
    /// "priced as of <date>" rather than implying live pricing.
    pub priced_as_of: String,
}
