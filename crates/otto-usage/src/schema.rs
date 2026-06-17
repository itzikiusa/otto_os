//! ClickHouse DDL for the usage + metrics tables. Retention is expressed as a
//! `MergeTree` `TTL` on the partition date column, so old data is dropped
//! automatically during background merges — and the window can be changed live
//! with `ALTER TABLE ... MODIFY TTL` (see [`alter_ttl_sql`]).

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
    duration_ms        UInt64
) ENGINE = MergeTree
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
ORDER BY (metric_date, ts)
TTL metric_date + INTERVAL {ttl} DAY;"
    )
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
