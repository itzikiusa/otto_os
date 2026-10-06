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
//! | `k8s_pod_cycle` | Null | one wide row per pod per cycle (collector) | — |
//! | `k8s_wl_{1m,5m,1h}` | AggregatingMergeTree | 1 min / 5 min / 1 h per workload | 2 d / 14 d / `retention_days` |
//! | `k8s_pod_{1m,5m,1h}` | AggregatingMergeTree | 1 min / 5 min / 1 h per pod | 2 d / 14 d / `retention_days` |
//!
//! The **wide** tiers are what the dashboards read: the collector writes
//! counters as reset-aware increments (`wide`), so every column is additive
//! and the pod / label dimensions fold away — a workload-hour is ONE row.
//! They need no backfill (their source only exists from the upgrade on), so
//! [`ensure`] creates their tables and views idempotently on every start.
//! The per-series tiers below stay for the generic per-metric chart and the
//! per-path requests view.
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
//! Existing installs migrate in [`ensure`]: each missing tier is streamed
//! in bounded blocks into a staging table, atomically exchanged with its
//! target, then checkpointed by creating its materialized view. A failed
//! attempt leaves the previous target intact and retries only unfinished
//! tiers. Initialization is serialized so collectors start after every view
//! exists. Old raw partitions are then trimmed best-effort; TTL remains the
//! fallback if shutdown interrupts that final cleanup.
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

/// The collector's one-row-per-pod-per-cycle insert target (`ENGINE =
/// Null`: nothing is stored, the wide views below fold every insert).
pub const POD_CYCLE_TABLE: &str = "k8s_pod_cycle";

/// One wide tier: a workload-level and a pod-level table at the same grain,
/// both fed from [`POD_CYCLE_TABLE`]. Counters arrive as reset-aware
/// increments, so everything but memory last/max is a plain `sum` and the
/// pod dimension folds away inside ClickHouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WideTier {
    pub grain: i64,
    pub keep_days: u32,
    pub wl_table: &'static str,
    wl_view: &'static str,
    pub pod_table: &'static str,
    pod_view: &'static str,
    partition: &'static str,
    /// Coarse time prefix of the sort key: per-workload filters prune
    /// inside each prefix group instead of scanning every bucket.
    prefix: &'static str,
}

pub const WIDE_1M: WideTier = WideTier {
    grain: 60,
    keep_days: 2,
    wl_table: "k8s_wl_1m",
    wl_view: "k8s_wl_1m_mv",
    pod_table: "k8s_pod_1m",
    pod_view: "k8s_pod_1m_mv",
    partition: "toDate(t)",
    prefix: "toStartOfHour(t)",
};
pub const WIDE_5M: WideTier = WideTier {
    grain: 300,
    keep_days: 14,
    wl_table: "k8s_wl_5m",
    wl_view: "k8s_wl_5m_mv",
    pod_table: "k8s_pod_5m",
    pod_view: "k8s_pod_5m_mv",
    partition: "toMonday(t)",
    prefix: "toStartOfDay(t)",
};
pub const WIDE_1H: WideTier = WideTier {
    grain: 3600,
    keep_days: 90,
    wl_table: "k8s_wl_1h",
    wl_view: "k8s_wl_1h_mv",
    pod_table: "k8s_pod_1h",
    pod_view: "k8s_pod_1h_mv",
    partition: "toYYYYMM(t)",
    prefix: "toMonday(t)",
};
/// Finest first.
pub const WIDE_TIERS: [WideTier; 3] = [WIDE_1M, WIDE_5M, WIDE_1H];
/// Granule-level time pruning for the wide tiers. The coarse `toStartOf…(t)`
/// sort-key prefix does NOT prune a `t >= …` filter (measured: a 5-minute
/// edge read the whole day partition); a minmax index on `t` does.
const T_INDEX_NAME: &str = "t_minmax";
const T_INDEX: &str = "INDEX t_minmax t TYPE minmax GRANULARITY 1";

/// Wide tables that already have the time index (`name` column).
pub fn wide_t_index_present_sql() -> String {
    let names = wide_tables()
        .iter()
        .map(|t| sql_str(t))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT table AS name FROM system.data_skipping_indices
         WHERE database = currentDatabase() AND name = {} AND table IN ({names})",
        sql_str(T_INDEX_NAME)
    )
}

/// Add + build the time index on a wide table created before it existed
/// (idempotent `ADD`; the `MATERIALIZE` mutation runs once, only for a
/// table that lacked it).
pub fn wide_t_index_add_sql(table: &str) -> String {
    format!(
        "ALTER TABLE {table} ADD INDEX IF NOT EXISTS {T_INDEX_NAME} t TYPE minmax GRANULARITY 1;
ALTER TABLE {table} MATERIALIZE INDEX {T_INDEX_NAME}"
    )
}

/// Small granules: a workload-tier read is a few hundred rows, and the
/// default 8192-row granule would set the floor of every read.
const WIDE_GRANULARITY: u32 = 1024;

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
    labels      SimpleAggregateFunction(any, Map(String, String)),
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

/// The wide pod-cycle table + every wide tier (`CREATE … IF NOT EXISTS`,
/// idempotent; no backfill — the views start folding with the next cycle).
pub fn wide_schema_sql(retention_days: u32) -> String {
    let ttl = retention_days.clamp(1, 90);
    let mut s = format!(
        "CREATE TABLE IF NOT EXISTS {POD_CYCLE_TABLE} (
    ts          DateTime,
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    pod         String,
    mem         Float64,
    has_mem     UInt8,
    req         Float64,
    err         Float64,
    lat_sum     Float64,
    lat_cnt     Float64,
    hist        Map(String, Float64)
) ENGINE = Null"
    );
    const AGG: &str = "
    mem_n       SimpleAggregateFunction(sum, UInt64),
    mem_sum     SimpleAggregateFunction(sum, Float64),
    mem_max     SimpleAggregateFunction(max, Float64),
    mem_last    SimpleAggregateFunction(max, Tuple(DateTime, Float64)),
    req         SimpleAggregateFunction(sum, Float64),
    err         SimpleAggregateFunction(sum, Float64),
    lat_sum     SimpleAggregateFunction(sum, Float64),
    lat_cnt     SimpleAggregateFunction(sum, Float64),
    hist        SimpleAggregateFunction(sumMap, Map(String, Float64))";
    for w in WIDE_TIERS {
        let keep = keep_days(w.keep_days, ttl);
        let drop_parts = if w.grain <= 60 {
            ", ttl_only_drop_parts = 1"
        } else {
            ""
        };
        let settings = format!("SETTINGS index_granularity = {WIDE_GRANULARITY}{drop_parts}");
        s.push_str(&format!(
            ";

CREATE TABLE IF NOT EXISTS {wl} (
    t           DateTime,
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    n           SimpleAggregateFunction(sum, UInt64),
    pods_max    SimpleAggregateFunction(max, UInt64),{AGG},
    {T_INDEX}
) ENGINE = AggregatingMergeTree
PARTITION BY (cluster_id, {part})
ORDER BY (cluster_id, {prefix}, namespace, workload, t)
TTL t + INTERVAL {keep} DAY
{settings};

CREATE TABLE IF NOT EXISTS {pod} (
    t           DateTime,
    cluster_id  LowCardinality(String),
    namespace   LowCardinality(String),
    workload    LowCardinality(String),
    pod         String,
    n           SimpleAggregateFunction(sum, UInt64),{AGG},
    {T_INDEX}
) ENGINE = AggregatingMergeTree
PARTITION BY (cluster_id, {part})
ORDER BY (cluster_id, {prefix}, namespace, workload, pod, t)
TTL t + INTERVAL {keep} DAY
{settings}",
            wl = w.wl_table,
            pod = w.pod_table,
            part = w.partition,
            prefix = w.prefix,
        ));
    }
    s
}

/// Workload view: per cycle first (pods, totals, the cycle's memory total),
/// then per bucket — exact whatever cycles one insert block carries.
fn wide_wl_select(w: &WideTier) -> String {
    format!(
        "SELECT {t} AS t, cluster_id, namespace, workload,
       count() AS n, max(pods) AS pods_max, sum(mn) AS mem_n, sum(ms) AS mem_sum, max(mx) AS mem_max,
       max(if(mn > 0, (ts, ms), (toDateTime(0), toFloat64(0)))) AS mem_last,
       sum(rq) AS req, sum(er) AS err, sum(ls) AS lat_sum, sum(lc) AS lat_cnt, sumMap(h) AS hist
FROM (
  SELECT ts, cluster_id, namespace, workload, count() AS pods, countIf(has_mem = 1) AS mn,
         sumIf(mem, has_mem = 1) AS ms, maxIf(mem, has_mem = 1) AS mx,
         sum(req) AS rq, sum(err) AS er, sum(lat_sum) AS ls, sum(lat_cnt) AS lc, sumMap(hist) AS h
  FROM {POD_CYCLE_TABLE}
  GROUP BY ts, cluster_id, namespace, workload
)
GROUP BY t, cluster_id, namespace, workload",
        t = bucket_expr("ts", w.grain),
    )
}

fn wide_pod_select(w: &WideTier) -> String {
    format!(
        "SELECT {t} AS t, cluster_id, namespace, workload, pod,
       count() AS n, countIf(has_mem = 1) AS mem_n, sumIf(mem, has_mem = 1) AS mem_sum, maxIf(mem, has_mem = 1) AS mem_max,
       max(if(has_mem = 1, (ts, mem), (toDateTime(0), toFloat64(0)))) AS mem_last,
       sum(req) AS req, sum(err) AS err, sum(lat_sum) AS lat_sum, sum(lat_cnt) AS lat_cnt, sumMap(hist) AS hist
FROM {POD_CYCLE_TABLE}
GROUP BY t, cluster_id, namespace, workload, pod",
        t = bucket_expr("ts", w.grain),
    )
}

/// `CREATE MATERIALIZED VIEW IF NOT EXISTS` for every wide tier.
pub fn wide_views_sql() -> String {
    let mut out = Vec::new();
    for w in WIDE_TIERS {
        out.push(format!(
            "CREATE MATERIALIZED VIEW IF NOT EXISTS {} TO {} AS\n{}",
            w.wl_view,
            w.wl_table,
            wide_wl_select(&w)
        ));
        out.push(format!(
            "CREATE MATERIALIZED VIEW IF NOT EXISTS {} TO {} AS\n{}",
            w.pod_view,
            w.pod_table,
            wide_pod_select(&w)
        ));
    }
    out.join(";\n\n")
}

/// Every wide target table (TTL alters, purges).
pub fn wide_tables() -> Vec<&'static str> {
    WIDE_TIERS
        .iter()
        .flat_map(|w| [w.wl_table, w.pod_table])
        .collect()
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

/// Bounded-block singleton aggregates for a raw partition. The destination
/// engines combine these values without a day-wide aggregation hash table.
pub fn backfill_sql(
    cluster_id: &str,
    date: &str,
    age_days: i64,
    retention_days: u32,
) -> Vec<String> {
    backfill::statements(cluster_id, date, age_days, retention_days)
}

mod backfill;

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
    // Wide tiers: tables, then views (no history to back-fill — the source
    // is a Null table only the collector writes, after this returns).
    sink.exec(&wide_schema_sql(retention_days)).await?;
    sink.exec(&wide_views_sql()).await?;
    if let Ok(rows) = sink.query_rows(&wide_t_index_present_sql()).await {
        let have: Vec<&str> = rows
            .iter()
            .filter_map(|r| r.get("name").and_then(|v| v.as_str()))
            .collect();
        for t in wide_tables() {
            if !have.contains(&t) {
                if let Err(e) = sink.exec(&wide_t_index_add_sql(t)).await {
                    tracing::debug!("k8s monitor: time index on {t}: {e}");
                }
            }
        }
    }
    if !backfill::migrate(sink, retention_days).await? {
        return Ok(());
    }
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
    tracing::info!("k8s monitor: rollup migration complete");
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
    for w in WIDE_TIERS {
        for table in [w.wl_table, w.pod_table] {
            v.push(format!(
                "ALTER TABLE {table} MODIFY TTL t + INTERVAL {} DAY{NO_REWRITE}",
                keep_days(w.keep_days, ttl)
            ));
        }
    }
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
    for t in wide_tables() {
        v.push(mk(t, "toDate(t)"));
    }
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
    fn wide_tiers_are_small_granule_and_fed_from_the_cycle_table() {
        let s = wide_schema_sql(30);
        assert!(s.contains("CREATE TABLE IF NOT EXISTS k8s_pod_cycle ("));
        assert!(s.contains(") ENGINE = Null"));
        for t in wide_tables() {
            assert!(
                s.contains(&format!("CREATE TABLE IF NOT EXISTS {t} (")),
                "{t}"
            );
        }
        assert_eq!(s.matches("index_granularity = 1024").count(), 6);
        assert_eq!(
            s.matches("INDEX t_minmax t TYPE minmax GRANULARITY 1")
                .count(),
            6
        );
        let add = wide_t_index_add_sql("k8s_wl_1m");
        assert!(add.contains("ADD INDEX IF NOT EXISTS t_minmax t TYPE minmax"));
        assert!(add.contains("MATERIALIZE INDEX t_minmax"));
        assert!(wide_t_index_present_sql().contains("'k8s_pod_1h'"));
        assert!(s.contains("ORDER BY (cluster_id, toMonday(t), namespace, workload, t)"));
        assert!(s.contains("ORDER BY (cluster_id, toStartOfHour(t), namespace, workload, pod, t)"));
        assert!(s.contains("hist        SimpleAggregateFunction(sumMap, Map(String, Float64))"));
        // 1m keeps 2 d, 5m 14 d, 1h the retention.
        assert_eq!(s.matches("TTL t + INTERVAL 30 DAY").count(), 2);
        for stmt in s.split(';') {
            assert!(!stmt.contains('\''), "no string literals in DDL: {stmt}");
        }
        let v = wide_views_sql();
        assert_eq!(
            v.matches("CREATE MATERIALIZED VIEW IF NOT EXISTS").count(),
            6
        );
        assert!(v.contains("k8s_wl_1h_mv TO k8s_wl_1h"));
        assert!(
            v.contains("GROUP BY ts, cluster_id, namespace, workload"),
            "per cycle first"
        );
        assert_eq!(v.matches("FROM k8s_pod_cycle").count(), 6);
        assert!(
            !views().contains(&"k8s_wl_1m_mv"),
            "wide views never trigger the backfill"
        );
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
            .all(|q| q.contains("max_threads = 1") && q.contains("max_memory_usage = 402653184")));
        let old = backfill_sql("c1", "2026-09-20", 12, 14);
        let tables: Vec<&str> = old
            .iter()
            .map(|q| q.split_whitespace().nth(2).unwrap())
            .collect();
        assert_eq!(
            tables,
            vec!["k8s_samples_5m", "k8s_samples_1h", "k8s_pods_1h"]
        );
        assert!(fresh.iter().all(|q| !q.contains("GROUP BY")));
        assert!(fresh.iter().all(|q| q.contains("max_insert_threads = 1")));
        assert!(expired_raw_partitions_sql(14).contains("today() - 2"));
        assert_eq!(
            drop_raw_partition_sql("a'b"),
            "ALTER TABLE k8s_samples DROP PARTITION ID 'a\\'b'"
        );
    }

    #[test]
    fn purge_targets_one_cluster_only() {
        let v = purge_cluster_sql("c'1", Some("2026-09-01"));
        assert_eq!(v.len(), 13, "7 per-series + 6 wide tables");
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
            12
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
