//! ClickHouse DDL for the usage + metrics tables. Retention is expressed as a
//! `MergeTree` `TTL` on the partition date column, so old data is dropped
//! automatically during background merges — and the window can be changed live
//! with `ALTER TABLE ... MODIFY TTL` (see [`alter_ttl_sql`]).
//!
//! ## Work-attribution columns (B1)
//!
//! Nine nullable `LowCardinality(String)` columns were added to `usage_events` to
//! carry work-graph dimensions (repo / branch / PR / story / swarm-task /
//! workflow / channel / review / origin). The `CREATE TABLE IF NOT EXISTS` DDL
//! includes them from now on so fresh installs get them automatically.
//!
//! Existing installs that already have the table but lack those columns are
//! upgraded by [`add_workref_columns_sql`] — a sequence of `ALTER TABLE ... ADD
//! COLUMN IF NOT EXISTS` statements run at startup after schema init.

/// `CREATE TABLE IF NOT EXISTS` for both tables, with a `{retention_days}` TTL.
pub fn schema_sql(retention_days: u32) -> String {
    let ttl = retention_days.max(1);
    format!(
        "CREATE TABLE IF NOT EXISTS usage_events (
    ts                 DateTime64(3) DEFAULT now64(3),
    event_date         Date DEFAULT toDate(ts),
    workspace_id       String,
    session_id         String,
    provider           LowCardinality(String),
    model              LowCardinality(String),
    kind               LowCardinality(String),
    input_tokens       UInt64,
    output_tokens      UInt64,
    cache_read_tokens  UInt64,
    cache_write_tokens UInt64,
    cost_usd           Float64,
    duration_ms        UInt64,
    -- work-graph attribution (B1); nullable, empty on old rows
    repo_id            LowCardinality(String) DEFAULT '',
    branch             LowCardinality(String) DEFAULT '',
    pr_number          LowCardinality(String) DEFAULT '',
    story_id           LowCardinality(String) DEFAULT '',
    swarm_task_id      LowCardinality(String) DEFAULT '',
    workflow_id        LowCardinality(String) DEFAULT '',
    channel            LowCardinality(String) DEFAULT '',
    review_id          LowCardinality(String) DEFAULT '',
    origin             LowCardinality(String) DEFAULT ''
) ENGINE = MergeTree
PARTITION BY toYYYYMM(event_date)
ORDER BY (event_date, provider, session_id, ts)
TTL event_date + INTERVAL {ttl} DAY;

CREATE TABLE IF NOT EXISTS system_metrics (
    ts              DateTime64(3) DEFAULT now64(3),
    metric_date     Date DEFAULT toDate(ts),
    host            LowCardinality(String),
    cpu_pct         Float64,
    mem_used_mb     Float64,
    mem_total_mb    Float64,
    mem_pct         Float64,
    load_avg_1      Float64,
    process_rss_mb  Float64,
    process_cpu_pct Float64,
    active_sessions UInt32
) ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (metric_date, ts)
TTL metric_date + INTERVAL {ttl} DAY;"
    )
}

/// `ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS …` for each work-ref
/// dimension. Run once on startup after `schema_sql` so that tables created
/// before B1 gain the new columns automatically. Existing rows backfill the
/// column's `DEFAULT ''` (empty string = "not set"). Safe to re-run.
pub fn add_workref_columns_sql() -> String {
    // ClickHouse `ALTER TABLE … ADD COLUMN IF NOT EXISTS` is idempotent and
    // non-blocking on local mode — columns with a `DEFAULT` expression cause
    // no write amplification on existing data (the default is evaluated at
    // query time from metadata, not materialised).
    "ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS repo_id       LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS branch        LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS pr_number     LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS story_id      LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS swarm_task_id LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS workflow_id   LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS channel       LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS review_id     LowCardinality(String) DEFAULT '';
ALTER TABLE usage_events ADD COLUMN IF NOT EXISTS origin        LowCardinality(String) DEFAULT '';"
    .to_string()
}

/// `ALTER TABLE ... MODIFY TTL` for both tables — applies a new retention
/// window to existing tables without recreating them.
pub fn alter_ttl_sql(retention_days: u32) -> String {
    let ttl = retention_days.max(1);
    format!(
        "ALTER TABLE usage_events MODIFY TTL event_date + INTERVAL {ttl} DAY;
ALTER TABLE system_metrics MODIFY TTL metric_date + INTERVAL {ttl} DAY;"
    )
}
