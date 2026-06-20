//! otto-usage — embedded ClickHouse usage & metrics store.
//!
//! Otto ships a *local* ClickHouse engine (the same `clickhouse` binary the
//! user installs via `curl https://clickhouse.com/ | sh`) and drives it in
//! `clickhouse local --path <dir>` mode: no separate server, no port, data
//! persisted on disk under the daemon's data directory. All access is
//! serialized through a single [`clickhouse::ClickHouse`] handle (a per-path
//! lock — `clickhouse local` refuses concurrent openers of the same path).
//!
//! Two tables, both `MergeTree` with a configurable `TTL` (the retention
//! window, default 180 days, adjustable live from the dashboard):
//!   * `usage_events`  — one row per agent turn / tool call / session action,
//!     attributed to a provider, model, session, workspace and day, with token
//!     counts + an (optional) USD cost.
//!   * `system_metrics` — periodic CPU / RAM / load / process samples
//!     (Prometheus-style host telemetry, our own schema).
//!
//! The engine degrades gracefully: if the binary can't be found or the schema
//! can't be created it becomes a no-op recorder that still reports its status,
//! so the rest of the daemon is unaffected.

pub mod budget_dedup;
mod clickhouse;
mod engine;
mod metrics;
mod pricing;
mod schema;
pub mod tailer;
mod types;

/// `workspace_id` assigned to usage recorded from transcripts that don't map to
/// any Otto session (the user's own Claude/codex runs). Recorded for a complete
/// machine-wide picture, but excludable from the dashboard via the "Otto only"
/// view filter (see `UsageEngine::summary(_, otto_only)`).
pub const EXTERNAL_WORKSPACE: &str = "external";

pub use budget_dedup::{BudgetDedup, BudgetSignal};
pub use clickhouse::ClickHouse;
pub use engine::UsageEngine;
pub use metrics::{Metric, MetricsSampler};
pub use pricing::{estimate_cost, is_priced, PRICED_AS_OF};
pub use tailer::{
    parse_claude_line, parse_codex_line, parse_codex_session_meta, CodexMeta, CursorStore,
    ParsedUsage,
};
pub use types::{
    AttributionDimension, AttributionRow, DailyUsage, FeatureUsage, ForecastReq, ForecastResp,
    MetricPoint, ProviderUsage, SessionTotals, SessionUsage, UsageConfig, UsageEvent, UsageStatus,
    UsageSummary,
};
