//! Public data types: the recorded event, engine config, the dashboard query
//! results and the engine status report. All serde-serializable so they double
//! as the HTTP API DTOs and as ClickHouse `JSONEachRow` rows.

use serde::{Deserialize, Serialize};

/// One usage row. Field names match the `usage_events` columns exactly so the
/// struct serializes straight to `JSONEachRow` for insertion (the `ts` /
/// `event_date` columns are omitted and default to "now" in ClickHouse).
///
/// The nine `work_*` fields carry work-graph attribution sourced from the
/// session's `meta_json["work"]` (a [`otto_core::workref::WorkRef`]). They are
/// `skip_serializing_if = "String::is_empty"` so that events without a work ref
/// don't write unnecessary empty columns (ClickHouse fills the `DEFAULT ''`).
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
    // ── Work-graph attribution (B1) ──────────────────────────────────────────
    // Sourced from the session's `meta_json["work"]` WorkRef. Empty string =
    // "not set" (matches the column DEFAULT, so old rows and un-attributed
    // events read identically). The `skip_serializing_if` keeps the JSONEachRow
    // payload lean: ClickHouse fills the DEFAULT for absent keys.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub repo_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub pr_number: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub story_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub swarm_task_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub channel: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub review_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,
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
    /// Most-common model used by this session (from `any(model)` in ClickHouse).
    /// Used to detect fallback-priced sessions: when the model is unrecognised
    /// the engine applies the conservative Opus-tier fallback, so the UI tags
    /// the row as "estimated".
    #[serde(default)]
    pub model: String,
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
    /// True when this session's cost was estimated using the conservative Opus-tier
    /// fallback (unrecognised model). UI should render the cost as "estimated".
    #[serde(default)]
    pub fallback_priced: bool,
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

// ---------------------------------------------------------------------------
// Work-graph attribution + cost forecast (B1)
// ---------------------------------------------------------------------------

/// One row in a work-graph attribution GROUP BY response. `key` is the value of
/// the grouped dimension (e.g. the repo id, branch name, origin tag). `sessions`
/// is the distinct-session count for that group.
///
/// ClickHouse returns `cost` (not `cost_usd`) because
/// `prefer_column_name_to_alias=1` would otherwise resolve `round(sum(cost_usd)
/// , 6) AS cost_usd` as the raw unaggregated column. The serde `alias` lets the
/// struct deserialize from either key, so it works from both ClickHouse rows and
/// round-tripped HTTP JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AttributionRow {
    /// Dimension value (e.g. "feature/my-branch", "review", "01J…" story id).
    pub key: String,
    /// Deserialized from the `cost` ClickHouse alias; serialized as `cost_usd`
    /// in the HTTP response. Serde `alias` accepts both keys.
    #[serde(alias = "cost")]
    pub cost_usd: f64,
    pub tokens: u64,
    pub sessions: u64,
}

/// Which work-graph dimension to group by in `GET /usage/attribution?by=`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionDimension {
    Repo,
    Branch,
    Pr,
    Story,
    SwarmTask,
    Workflow,
    Channel,
    Review,
    Origin,
}

impl AttributionDimension {
    /// The ClickHouse column name that corresponds to this dimension.
    pub fn column(&self) -> &'static str {
        match self {
            Self::Repo => "repo_id",
            Self::Branch => "branch",
            Self::Pr => "pr_number",
            Self::Story => "story_id",
            Self::SwarmTask => "swarm_task_id",
            Self::Workflow => "workflow_id",
            Self::Channel => "channel",
            Self::Review => "review_id",
            Self::Origin => "origin",
        }
    }

    /// Parse from the `by=` query-string value (matches the contract key names).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "repo" => Some(Self::Repo),
            "branch" => Some(Self::Branch),
            "pr" => Some(Self::Pr),
            "story" => Some(Self::Story),
            "swarm_task" => Some(Self::SwarmTask),
            "workflow" => Some(Self::Workflow),
            "channel" => Some(Self::Channel),
            "review" => Some(Self::Review),
            "origin" => Some(Self::Origin),
            _ => None,
        }
    }
}

/// Request body for `POST /usage/forecast`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForecastReq {
    /// Otto feature label ("review" | "product" | "channel" | "agent" | …).
    pub feature: String,
    /// Provider ("claude" | "codex" | "shell" | …).
    pub provider: String,
    /// Optional explicit token estimate — when provided the cost is priced
    /// directly from this count rather than from recent-run averages.
    pub est_tokens: Option<u64>,
}

/// Response for `POST /usage/forecast`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForecastResp {
    /// Projected cost in USD.
    pub projected_cost_usd: f64,
    /// Human-readable explanation of how the estimate was derived.
    pub basis: String,
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
