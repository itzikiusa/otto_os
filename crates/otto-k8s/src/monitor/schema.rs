//! ClickHouse DDL for the monitoring tables (spec "Storage").
//!
//! Raw samples land in `k8s_samples`; materialized views fold every insert
//! into pre-aggregated tiers so the dashboards never re-aggregate raw rows:
//!
//! | table | engine | grain / key | default keep |
//! |---|---|---|---|
//! | `k8s_samples` | MergeTree | one row per scraped value | 2 d (short drill-downs only) |
//! | `k8s_samples_1m` | AggregatingMergeTree | 1 min per series | 2 d |
//! | `k8s_samples_5m` | AggregatingMergeTree | 5 min per series | 14 d |
//! | `k8s_samples_1h` | AggregatingMergeTree | 1 h per series | `retention_days` (≤ 90) |
//! | `k8s_latest` | ReplacingMergeTree | last value per series | 1 d |
//! | `k8s_pods_1h` | AggregatingMergeTree | 1 h per pod (row count, last seen) | `retention_days` |
//! | `k8s_events` | MergeTree | one row per event | `retention_days` |
//!
//! A series is `(cluster, namespace, workload, metric, pod, labels)` —
//! exactly the identity the raw counter math groups by — so every rollup
//! keeps `min / max / sum / count / last` per series per bucket and stays
//! composable: a window's counter delta is `max(v_max) − min(v_min)`, a gauge
//! average `sum(v_sum) / sum(n)`, the latest value `max(v_last)` (a
//! `(ts, value)` tuple). Rollups put time right after `metric` in the sort
//! key, so a 1 h window reads ~1 h of granules instead of the whole day
//! partition the raw key forces.
//!
//! Existing installs migrate in [`ensure`]: the rollup tables are created,
//! back-filled from whatever raw rows exist (one `(cluster, day)` partition
//! per statement, two threads), and only THEN are the materialized views
//! created — while every collector loop waits on the same lock — so a row is
//! never counted twice and a crash mid-backfill simply redoes it (the
//! rollups are truncated first; nothing but the backfill writes them until
//! the views exist). Raw partitions older than the raw keep are then dropped.
//!
//! Tables are partitioned per cluster so per-cluster purges are cheap; TTLs
//! follow the largest configured retention (`alter_ttl_sql`), and clusters
//! that ask for less are trimmed by `purge_cluster_sql`.

use otto_core::Result;

use crate::MonitorSink;

/// Escape a string literal for ClickHouse (`'`, `\`, newline).
pub fn sql_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => o.push_str("\\'"),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            _ => o.push(ch),
        }
    }
    o.push('\'');
    o
}

/// Raw rows are only read for windows too short for the 1-minute tier.
pub const RAW_KEEP_DAYS: u32 = 2;
/// Last-value table: covers the 15 min "latest" lookback with a wide margin.
pub const LATEST_KEEP_DAYS: u32 = 1;

/// One pre-aggregated sample tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rollup {
    pub table: &'static str,
    pub view: &'static str,
    /// Bucket width in seconds (epoch-aligned).
    pub grain: i64,
    /// Default keep; capped by the configured retention.
    pub keep_days: u32,
    /// ClickHouse partition expression (besides `cluster_id`).
    partition: &'static str,
}

pub const ROLLUP_1M: Rollup = Rollup {
    table: "k8s_samples_1m",
    view: "k8s_samples_1m_mv",
    grain: 60,
    keep_days: 2,
    partition: "toDate(t)",
};
pub const ROLLUP_5M: Rollup = Rollup {
    table: "k8s_samples_5m",
    view: "k8s_samples_5m_mv",
    grain: 300,
    keep_days: 14,
    partition: "toMonday(t)",
};
pub const ROLLUP_1H: Rollup = Rollup {
    table: "k8s_samples_1h",
    view: "k8s_samples_1h_mv",
    grain: 3600,
    keep_days: 90,
    partition: "toYYYYMM(t)",
};
/// Finest first.
pub const ROLLUPS: [Rollup; 3] = [ROLLUP_1M, ROLLUP_5M, ROLLUP_1H];

pub const LATEST_TABLE: &str = "k8s_latest";
const LATEST_VIEW: &str = "k8s_latest_mv";
pub const PODS_TABLE: &str = "k8s_pods_1h";
const PODS_VIEW: &str = "k8s_pods_1h_mv";

/// Every materialized view, in creation order.
pub fn views() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ROLLUPS.iter().map(|r| r.view).collect();
    v.push(LATEST_VIEW);
    v.push(PODS_VIEW);
    v
}

/// Keep for a tier under a configured retention.
pub fn keep_days(default_days: u32, retention_days: u32) -> u32 {
    default_days.min(retention_days.clamp(1, 90)).max(1)
}

/// `t` for a `grain`-second bucket of `ts`, epoch-aligned (timezone-free, so
/// it matches the planner's integer math in `queries`).
pub fn bucket_expr(col: &str, grain: i64) -> String {
    format!("toDateTime(intDiv(toUInt32(toUnixTimestamp({col})), {grain}) * {grain})")
}

/// `CREATE TABLE IF NOT EXISTS` for the raw + event tables and every rollup
/// target, with `{retention_days}` TTLs. Idempotent — run at every collector
/// start (no views: [`ensure`] adds those after the one-time backfill).
pub fn schema_sql(retention_days: u32) -> String {
    let ttl = retention_days.clamp(1, 90);
    let raw = keep_days(RAW_KEEP_DAYS, ttl);
    let mut s = format!(
        "CREATE TABLE IF NOT EXISTS k8s_samples (
    ts            DateTime64(3),
    sample_date   Date DEFAULT toDate(ts),
    cluster_id    LowCardinality(String),
    namespace     LowCardinality(String),
    workload_kind LowCardinality(String),
    workload      LowCardinality(String),
    pod           String,
    container     LowCardinality(String),
    metric        LowCardinality(String),
    labels        Map(LowCardinality(String), String),
    value         Float64
) ENGINE = MergeTree
PARTITION BY (cluster_id, sample_date)
ORDER BY (cluster_id, namespace, workload, metric, pod, ts)
TTL sample_date + INTERVAL {raw} DAY
SETTINGS ttl_only_drop_parts = 1;

ALTER TABLE k8s_samples ADD INDEX IF NOT EXISTS ts_minmax ts TYPE minmax GRANULARITY 1;

ALTER TABLE k8s_samples MODIFY SETTING ttl_only_drop_parts = 1;

CREATE TABLE IF NOT EXISTS k8s_events (
    ts          DateTime64(3),
    event_date  Date DEFAULT toDate(ts),
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    pod         String,
    container   LowCardinality(String),
    kind        LowCardinality(String),
    class       LowCardinality(String),
    reason      String,
    exit_code   Int32,
    detail      String,
    actor       String
) ENGINE = MergeTree
PARTITION BY (cluster_id, event_date)
ORDER BY (cluster_id, ts)
TTL event_date + INTERVAL {ttl} DAY;"
    );
    for r in ROLLUPS {
        let keep = keep_days(r.keep_days, ttl);
        // Daily partitions drop whole; coarser ones keep exact retention.
        let drop_parts = if r.grain <= 60 {
            "\nSETTINGS ttl_only_drop_parts = 1"
        } else {
            ""
        };
        s.push_str(&format!(
            "

CREATE TABLE IF NOT EXISTS {table} (
    t           DateTime,
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    metric      LowCardinality(String),
    pod         String,
    series      UInt64,
    labels      Map(LowCardinality(String), String),
    v_min       SimpleAggregateFunction(min, Float64),
    v_max       SimpleAggregateFunction(max, Float64),
    v_sum       SimpleAggregateFunction(sum, Float64),
    n           SimpleAggregateFunction(sum, UInt64),
    v_last      SimpleAggregateFunction(max, Tuple(DateTime64(3), Float64))
) ENGINE = AggregatingMergeTree
PARTITION BY (cluster_id, {part})
ORDER BY (cluster_id, metric, toStartOfHour(t), namespace, workload, pod, series, t)
TTL t + INTERVAL {keep} DAY{drop_parts};",
            table = r.table,
            part = r.partition,
        ));
    }
    s.push_str(&format!(
        "

CREATE TABLE IF NOT EXISTS {LATEST_TABLE} (
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    metric      LowCardinality(String),
    pod         String,
    series      UInt64,
    labels      Map(LowCardinality(String), String),
    last_ts     DateTime64(3),
    last_value  Float64
) ENGINE = ReplacingMergeTree(last_ts)
PARTITION BY cluster_id
ORDER BY (cluster_id, metric, namespace, workload, pod, series)
TTL toDateTime(last_ts) + INTERVAL {latest} DAY;

CREATE TABLE IF NOT EXISTS {PODS_TABLE} (
    t           DateTime,
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    pod         String,
    n           SimpleAggregateFunction(sum, UInt64),
    last        SimpleAggregateFunction(max, DateTime64(3))
) ENGINE = AggregatingMergeTree
PARTITION BY (cluster_id, toYYYYMM(t))
ORDER BY (cluster_id, t, namespace, workload, pod)
TTL t + INTERVAL {ttl} DAY;",
        latest = LATEST_KEEP_DAYS,
    ));
    s
}

/// The `SELECT` that folds raw rows into one tier (shared by the view and
/// the backfill, so both aggregate identically). `filter` is a full `WHERE …`
/// clause or empty.
fn rollup_select(r: &Rollup, filter: &str) -> String {
    format!(
        "SELECT {t} AS t, cluster_id, namespace, workload, metric, pod, cityHash64(labels) AS series, labels,
       min(value) AS v_min, max(value) AS v_max, sum(value) AS v_sum, count() AS n, max((ts, value)) AS v_last
FROM k8s_samples{filter}
GROUP BY t, cluster_id, namespace, workload, metric, pod, labels",
        t = bucket_expr("ts", r.grain),
    )
}

fn latest_select(filter: &str) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, metric, pod, cityHash64(labels) AS series, labels,
       max(ts) AS last_ts, argMax(value, ts) AS last_value
FROM k8s_samples{filter}
GROUP BY cluster_id, namespace, workload, metric, pod, labels"
    )
}

fn pods_select(filter: &str) -> String {
    format!(
        "SELECT {t} AS t, cluster_id, namespace, workload, pod, count() AS n, max(ts) AS last
FROM k8s_samples{filter}
GROUP BY t, cluster_id, namespace, workload, pod",
        t = bucket_expr("ts", 3600),
    )
}

/// `CREATE MATERIALIZED VIEW IF NOT EXISTS` for every tier (raw → tier, so
/// no tier depends on another's view).
pub fn views_sql() -> String {
    let mut out: Vec<String> = ROLLUPS
        .iter()
        .map(|r| {
            format!(
                "CREATE MATERIALIZED VIEW IF NOT EXISTS {} TO {} AS\n{}",
                r.view,
                r.table,
                rollup_select(r, "")
            )
        })
        .collect();
    out.push(format!(
        "CREATE MATERIALIZED VIEW IF NOT EXISTS {LATEST_VIEW} TO {LATEST_TABLE} AS\n{}",
        latest_select("")
    ));
    out.push(format!(
        "CREATE MATERIALIZED VIEW IF NOT EXISTS {PODS_VIEW} TO {PODS_TABLE} AS\n{}",
        pods_select("")
    ));
    out.join(";\n\n")
}

/// How many of our views exist (all of them ⇒ migrated).
pub fn views_present_sql() -> String {
    let names = views()
        .iter()
        .map(|v| sql_str(v))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT count() AS n FROM system.tables WHERE database = currentDatabase() AND name IN ({names})"
    )
}

/// Raw `(cluster, day)` partitions to back-fill, oldest first.
pub fn backfill_partitions_sql() -> &'static str {
    "SELECT cluster_id, toString(sample_date) AS d FROM k8s_samples GROUP BY cluster_id, sample_date ORDER BY d, cluster_id"
}

/// Truncate every rollup target (backfill restart point).
pub fn truncate_rollups_sql() -> String {
    let mut v: Vec<String> = ROLLUPS
        .iter()
        .map(|r| format!("TRUNCATE TABLE IF EXISTS {}", r.table))
        .collect();
    v.push(format!("TRUNCATE TABLE IF EXISTS {LATEST_TABLE}"));
    v.push(format!("TRUNCATE TABLE IF EXISTS {PODS_TABLE}"));
    v.join(";\n")
}

/// `INSERT … SELECT` statements that back-fill one raw partition into the
/// tiers that still keep that day. `age_days` = today − `date`. Capped at two
/// threads so a large backfill never pins the CPU.
pub fn backfill_sql(
    cluster_id: &str,
    date: &str,
    age_days: i64,
    retention_days: u32,
) -> Vec<String> {
    let filter = format!(
        "\nWHERE cluster_id = {} AND sample_date = {}",
        sql_str(cluster_id),
        sql_str(date)
    );
    const LIMITS: &str = "\nSETTINGS max_threads = 2";
    let mut out = Vec::new();
    for r in ROLLUPS {
        if age_days <= i64::from(keep_days(r.keep_days, retention_days)) {
            out.push(format!(
                "INSERT INTO {}\n{}{LIMITS}",
                r.table,
                rollup_select(&r, &filter)
            ));
        }
    }
    if age_days <= i64::from(LATEST_KEEP_DAYS) {
        out.push(format!(
            "INSERT INTO {LATEST_TABLE}\n{}{LIMITS}",
            latest_select(&filter)
        ));
    }
    out.push(format!(
        "INSERT INTO {PODS_TABLE}\n{}{LIMITS}",
        pods_select(&filter)
    ));
    out
}

/// Raw partitions past the raw keep (dropped once, right after migrating —
/// the old TTL would otherwise keep them up to two weeks).
pub fn expired_raw_partitions_sql(retention_days: u32) -> String {
    format!(
        "SELECT DISTINCT _partition_id AS pid FROM k8s_samples WHERE sample_date < today() - {}",
        keep_days(RAW_KEEP_DAYS, retention_days)
    )
}

pub fn drop_raw_partition_sql(partition_id: &str) -> String {
    format!(
        "ALTER TABLE k8s_samples DROP PARTITION ID {}",
        sql_str(partition_id)
    )
}

/// Serialises [`ensure`] across collector loops + the run-now route: the
/// backfill must finish before any loop inserts again.
static ENSURE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Create / migrate everything. Idempotent and cheap once migrated (the
/// `CREATE … IF NOT EXISTS` batch + one `system.tables` count).
pub async fn ensure(sink: &dyn MonitorSink, retention_days: u32) -> Result<()> {
    let _guard = ENSURE_LOCK.lock().await;
    sink.exec(&schema_sql(retention_days)).await?;
    let present = sink
        .query_rows(&views_present_sql())
        .await?
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|n| {
            n.as_u64()
                .or_else(|| n.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0);
    if present as usize >= views().len() {
        return Ok(());
    }
    tracing::info!("k8s monitor: migrating to rollup tables (one-time backfill)");
    sink.exec(&truncate_rollups_sql()).await?;
    let today = chrono::Utc::now().date_naive();
    let parts = sink.query_rows(backfill_partitions_sql()).await?;
    for p in &parts {
        let cid = p.get("cluster_id").and_then(|v| v.as_str()).unwrap_or("");
        let d = p.get("d").and_then(|v| v.as_str()).unwrap_or("");
        let Ok(date) = chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d") else {
            continue;
        };
        let age = (today - date).num_days();
        for q in backfill_sql(cid, d, age, retention_days) {
            sink.exec(&q).await?;
        }
        // Let merges and the dashboards breathe between partitions.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    sink.exec(&views_sql()).await?;
    // Old raw days are now in the rollups: drop them instead of waiting for
    // the old (longer) TTL.
    if let Ok(rows) = sink
        .query_rows(&expired_raw_partitions_sql(retention_days))
        .await
    {
        for r in rows {
            if let Some(pid) = r.get("pid").and_then(|v| v.as_str()) {
                if let Err(e) = sink.exec(&drop_raw_partition_sql(pid)).await {
                    tracing::debug!("k8s monitor: drop raw partition {pid}: {e}");
                }
            }
        }
    }
    tracing::info!(
        partitions = parts.len(),
        "k8s monitor: rollup migration complete"
    );
    Ok(())
}

/// `ALTER TABLE … MODIFY TTL` for every table, each tier capped by its keep.
/// `materialize_ttl_after_modify = 0`: never rewrite existing parts for a TTL
/// change (old parts age out on their own schedule / are dropped by
/// [`ensure`]).
pub fn alter_ttl_sql(retention_days: u32) -> String {
    let ttl = retention_days.clamp(1, 90);
    const NO_REWRITE: &str = " SETTINGS materialize_ttl_after_modify = 0";
    let mut v = vec![
        format!(
            "ALTER TABLE k8s_samples MODIFY TTL sample_date + INTERVAL {} DAY{NO_REWRITE}",
            keep_days(RAW_KEEP_DAYS, ttl)
        ),
        format!("ALTER TABLE k8s_events MODIFY TTL event_date + INTERVAL {ttl} DAY{NO_REWRITE}"),
    ];
    for r in ROLLUPS {
        v.push(format!(
            "ALTER TABLE {} MODIFY TTL t + INTERVAL {} DAY{NO_REWRITE}",
            r.table,
            keep_days(r.keep_days, ttl)
        ));
    }
    v.push(format!(
        "ALTER TABLE {PODS_TABLE} MODIFY TTL t + INTERVAL {ttl} DAY{NO_REWRITE}"
    ));
    v.join(";\n")
}

/// `DELETE` statements for one cluster (every table); `before_date`
/// (`YYYY-MM-DD`) limits to rows older than that day, `None` = everything
/// (cluster removed).
pub fn purge_cluster_sql(cluster_id: &str, before_date: Option<&str>) -> Vec<String> {
    let cid = sql_str(cluster_id);
    let mk = |table: &str, date_expr: &str| {
        let mut q = format!("DELETE FROM {table} WHERE cluster_id = {cid}");
        if let Some(d) = before_date {
            q.push_str(&format!(" AND {date_expr} < {}", sql_str(d)));
        }
        q
    };
    let mut v = vec![
        mk("k8s_samples", "sample_date"),
        mk("k8s_events", "event_date"),
    ];
    for r in ROLLUPS {
        v.push(mk(r.table, "toDate(t)"));
    }
    v.push(mk(PODS_TABLE, "toDate(t)"));
    v.push(mk(LATEST_TABLE, "toDate(last_ts)"));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_has_every_table_and_tiered_ttls() {
        let s = schema_sql(14);
        for t in [
            "k8s_samples",
            "k8s_events",
            "k8s_samples_1m",
            "k8s_samples_5m",
            "k8s_samples_1h",
            "k8s_latest",
            "k8s_pods_1h",
        ] {
            assert!(
                s.contains(&format!("CREATE TABLE IF NOT EXISTS {t} (")),
                "{t}"
            );
        }
        assert!(s.contains("labels        Map(LowCardinality(String), String)"));
        // raw 2 d, events 14 d, 1m 2 d, 5m 14 d, 1h 14 d (capped by retention)
        assert!(s.contains("TTL sample_date + INTERVAL 2 DAY"));
        assert!(s.contains("TTL event_date + INTERVAL 14 DAY"));
        assert_eq!(
            s.matches("TTL t + INTERVAL 14 DAY").count(),
            3,
            "5m, 1h, pods"
        );
        assert_eq!(s.matches("TTL t + INTERVAL 2 DAY").count(), 1, "1m");
        assert!(s.contains("ENGINE = AggregatingMergeTree"));
        assert!(s.contains("ENGINE = ReplacingMergeTree(last_ts)"));
        // Time sits right after metric in the rollup key.
        assert!(s.contains("ORDER BY (cluster_id, metric, toStartOfHour(t),"));
        // No statement embeds a `;` (the sink splits on it).
        for stmt in s.split(';') {
            assert!(!stmt.contains('\''), "no string literals in DDL: {stmt}");
        }
        // A 90-day retention lets the hour tier keep 90 days.
        assert!(schema_sql(90).contains("TTL t + INTERVAL 90 DAY"));
    }

    #[test]
    fn views_feed_every_tier_from_raw() {
        let v = views_sql();
        assert_eq!(
            v.matches("CREATE MATERIALIZED VIEW IF NOT EXISTS").count(),
            5
        );
        assert!(v.contains("k8s_samples_1m_mv TO k8s_samples_1m"));
        assert!(v.contains("intDiv(toUInt32(toUnixTimestamp(ts)), 300) * 300"));
        assert!(v.contains("max((ts, value)) AS v_last"));
        assert!(v.contains("argMax(value, ts) AS last_value"));
        assert_eq!(v.matches("FROM k8s_samples\n").count(), 5);
        assert!(views_present_sql().contains("'k8s_pods_1h_mv'"));
    }

    #[test]
    fn backfill_is_per_partition_and_respects_tier_keep() {
        let fresh = backfill_sql("c'1", "2026-10-01", 1, 14);
        assert_eq!(fresh.len(), 5, "1m, 5m, 1h, latest, pods");
        assert!(fresh[0].starts_with("INSERT INTO k8s_samples_1m"));
        assert!(fresh[0].contains("cluster_id = 'c\\'1' AND sample_date = '2026-10-01'"));
        assert!(fresh
            .iter()
            .all(|q| q.ends_with("SETTINGS max_threads = 2")));
        let old = backfill_sql("c1", "2026-09-20", 12, 14);
        let tables: Vec<&str> = old
            .iter()
            .map(|q| q.split_whitespace().nth(2).unwrap())
            .collect();
        assert_eq!(
            tables,
            vec!["k8s_samples_5m", "k8s_samples_1h", "k8s_pods_1h"]
        );
        assert!(truncate_rollups_sql().contains("TRUNCATE TABLE IF EXISTS k8s_latest"));
        assert!(!truncate_rollups_sql().contains("k8s_samples;"));
        assert!(expired_raw_partitions_sql(14).contains("today() - 2"));
        assert_eq!(
            drop_raw_partition_sql("a'b"),
            "ALTER TABLE k8s_samples DROP PARTITION ID 'a\\'b'"
        );
    }

    #[test]
    fn purge_targets_one_cluster_only() {
        let v = purge_cluster_sql("c'1", Some("2026-09-01"));
        assert_eq!(v.len(), 7);
        assert!(v.iter().all(|q| q.contains("cluster_id = 'c\\'1'")));
        assert!(v[0].contains("sample_date < '2026-09-01'"));
        assert!(v[2].contains("toDate(t) < '2026-09-01'"));
        let all = purge_cluster_sql("c1", None);
        assert!(!all[0].contains("sample_date <"));
    }

    #[test]
    fn ttl_alters_never_rewrite_and_floor_is_one_day() {
        let a = alter_ttl_sql(0);
        assert!(a.contains("k8s_events MODIFY TTL event_date + INTERVAL 1 DAY"));
        assert!(a.contains("k8s_samples_1h MODIFY TTL t + INTERVAL 1 DAY"));
        assert_eq!(
            a.matches("SETTINGS materialize_ttl_after_modify = 0")
                .count(),
            6
        );
        let b = alter_ttl_sql(30);
        assert!(b.contains("k8s_samples MODIFY TTL sample_date + INTERVAL 2 DAY"));
        assert!(b.contains("k8s_samples_5m MODIFY TTL t + INTERVAL 14 DAY"));
        assert!(b.contains("k8s_samples_1h MODIFY TTL t + INTERVAL 30 DAY"));
    }

    #[test]
    fn sql_str_escapes() {
        assert_eq!(sql_str("a'b\\c\nd"), "'a\\'b\\\\c\\nd'");
    }
}
