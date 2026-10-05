//! ClickHouse SQL builders for the dashboard + health digest. Every string
//! value goes through [`sql_str`]; every identifier-like filter (metric,
//! workload, pod, namespace, class) must pass [`ident_ok`] first — the HTTP
//! layer rejects anything else with 400 before it gets here.
//!
//! Sample reads go through a [`Span`]: the planner picks the COARSEST tier
//! (see `schema`: 1 h / 5 min / 1 min rollups, raw only for drill-downs
//! shorter than [`MIN_BUCKETS`] minutes) that still gives the window
//! [`MIN_BUCKETS`] buckets, divides the chart step, and still holds the
//! window's start. Every tier is read through the same uniform row shape
//! ([`Span::source`]: `v_min / v_max / v_sum / n / v_last` per series and
//! bucket; a raw row is a bucket of one), so each query is written once.
//! The window start snaps down to the tier grain; rates divide by the
//! seconds actually covered ([`Span::secs`]), so a snapped window never
//! inflates a rate.
//!
//! The dashboards (workloads, health, overview, sparklines, Fleet table and
//! charts) read the **wide** tiers through a [`WSpan`] ([`wide_totals`],
//! [`wide_spark_in`], [`wide_series_in`]): counters there are increments,
//! window totals are stitched from closed coarse buckets + a fine edge
//! ([`WSpan::plan_total`]) and closed spans are cached ([`query_span`]). The
//! per-series builders below serve the generic per-metric chart and the
//! per-path requests view.
//!
//! Per-series counters are stored raw; rates are `greatest(0, max − min) / seconds` per
//! (pod, label-set) inside the window, summed up. A counter reset inside the
//! window therefore under-counts that pod for the window rather than
//! producing a negative spike. `min`/`max` compose across buckets, so the
//! rollups give exactly the raw answer for the same (snapped) range.

use std::collections::BTreeMap;

use chrono::Duration;
use otto_core::{Error, Result};
use serde_json::Value;

use super::schema::{self, sql_str, Rollup, WideTier, RAW_KEEP_DAYS, ROLLUPS, WIDE_TIERS};
use crate::MonitorSink;

/// Request-counter series recognised for rps / error-rate (Go + Spring).
pub const REQUEST_COUNTERS: [&str; 2] =
    ["http_requests_total", "http_server_requests_seconds_count"];
/// Latency histogram bucket series (Go + Spring).
pub const LATENCY_BUCKETS: [&str; 2] = [
    "http_request_duration_seconds_bucket",
    "http_server_requests_seconds_bucket",
];
pub const LATENCY_SUMS: [&str; 2] = [
    "http_request_duration_seconds_sum",
    "http_server_requests_seconds_sum",
];
pub const LATENCY_COUNTS: [&str; 2] = [
    "http_request_duration_seconds_count",
    "http_server_requests_seconds_count",
];
/// Memory gauges, most authoritative first.
pub const MEMORY_GAUGES: [&str; 3] = [
    "mem_working_set_bytes",
    "mem_sys_bytes",
    "jvm_memory_used_bytes",
];

/// A window must span at least this many buckets of a tier to read it
/// (bounds the start-snap to ~4 % of the window). Below 24 minutes, raw.
pub const MIN_BUCKETS: i64 = 24;

/// Where a sample read comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Raw,
    Rollup(Rollup),
}

impl Tier {
    pub fn grain(self) -> i64 {
        match self {
            Tier::Raw => 1,
            Tier::Rollup(r) => r.grain,
        }
    }
    pub fn table(self) -> &'static str {
        match self {
            Tier::Raw => "k8s_samples",
            Tier::Rollup(r) => r.table,
        }
    }
}

/// A planned sample read: tier + `[from, to)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub tier: Tier,
    /// Inclusive lower bound, snapped down to the tier grain.
    pub from: i64,
    /// Exclusive upper bound (snapped); `None` = up to now.
    pub to: Option<i64>,
    /// Seconds the read covers — the rate denominator.
    pub secs: i64,
}

pub fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Span {
    /// The coarsest tier for `[now − back, now − until)`; `step` (a chart
    /// bucket width) must be a multiple of the tier grain. Falls back to raw
    /// for short windows, then to the finest tier still holding the start.
    pub fn plan(now: i64, back_secs: i64, until_secs: i64, step: Option<u32>) -> Span {
        let back = back_secs.max(1);
        let until = until_secs.clamp(0, back - 1);
        let width = back - until;
        let step_ok = |g: i64| step.is_none_or(|s| i64::from(s) % g == 0);
        let holds = |days: u32| back <= i64::from(days) * 86_400;
        let tier = ROLLUPS
            .iter()
            .rev()
            .find(|r| r.grain * MIN_BUCKETS <= width && step_ok(r.grain) && holds(r.keep_days))
            .map(|r| Tier::Rollup(*r))
            .or_else(|| holds(RAW_KEEP_DAYS).then_some(Tier::Raw))
            .or_else(|| {
                ROLLUPS
                    .iter()
                    .find(|r| step_ok(r.grain) && holds(r.keep_days))
                    .map(|r| Tier::Rollup(*r))
            })
            .unwrap_or(Tier::Rollup(schema::ROLLUP_1H));
        Span::on(tier, now, back, until)
    }

    /// `[now − back, now − until)` on a given tier (snapped to its grain).
    pub fn on(tier: Tier, now: i64, back_secs: i64, until_secs: i64) -> Span {
        let g = tier.grain();
        let from = (now - back_secs.max(1)).div_euclid(g) * g;
        let to = (until_secs > 0).then(|| ((now - until_secs).div_euclid(g) * g).max(from + g));
        Span::between(tier, from, to, now)
    }

    /// An explicit `[from, to)` (already aligned) — tests compare tiers on it.
    pub fn between(tier: Tier, from: i64, to: Option<i64>, now: i64) -> Span {
        Span {
            tier,
            from,
            to,
            secs: (to.unwrap_or(now) - from).max(1),
        }
    }

    pub fn is_raw(&self) -> bool {
        self.tier == Tier::Raw
    }

    fn time_filter(&self) -> String {
        let col = if self.is_raw() { "ts" } else { "t" };
        let mut s = format!("{col} >= toDateTime({})", self.from);
        if let Some(to) = self.to {
            s.push_str(&format!(" AND {col} < toDateTime({to})"));
        }
        s
    }

    /// Uniform rows `(t, cluster_id, namespace, workload, metric, pod,
    /// series, labels, v_min, v_max, v_sum, n, v_last)` for this span;
    /// `filter` is appended to the `WHERE` (leading ` AND …`) so every
    /// predicate reaches the table's primary key.
    pub fn source(&self, filter: &str) -> String {
        match self.tier {
            Tier::Raw => format!(
                "SELECT ts AS t, cluster_id, namespace, workload, metric, pod, cityHash64(labels) AS series, labels,
                        value AS v_min, value AS v_max, value AS v_sum, toUInt64(1) AS n, (ts, value) AS v_last
                 FROM k8s_samples WHERE {time}{filter}",
                time = self.time_filter(),
            ),
            Tier::Rollup(r) => format!(
                "SELECT t, cluster_id, namespace, workload, metric, pod, series, labels, v_min, v_max, v_sum, n, v_last
                 FROM {table} WHERE {time}{filter}",
                table = r.table,
                time = self.time_filter(),
            ),
        }
    }
}

/// `toDateTime` of the `step`-second bucket of `col` (epoch-aligned, so it
/// matches the rollups' own buckets whenever `step` is a multiple of them).
pub fn bucket(col: &str, step: u32) -> String {
    schema::bucket_expr(col, i64::from(step.max(1)))
}

/// Per-series counter increase inside the rows being grouped.
const DELTA: &str = "greatest(0, max(v_max) - min(v_min))";
/// Gauge mean over the rows being grouped.
const MEAN: &str = "sum(v_sum) / sum(n)";

/// `1h` | `6h` | `24h` | `7d` | `<n>m|h|d`; max 90 d.
pub fn parse_window(s: &str) -> Result<Duration> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(Duration::hours(24));
    }
    let (num, unit) = t.split_at(t.len() - 1);
    let n: i64 = num
        .parse()
        .map_err(|_| Error::Invalid(format!("bad window '{s}'")))?;
    if n <= 0 {
        return Err(Error::Invalid(format!("bad window '{s}'")));
    }
    let d = match unit {
        "m" => Duration::minutes(n),
        "h" => Duration::hours(n),
        "d" => Duration::days(n),
        _ => return Err(Error::Invalid(format!("bad window '{s}' (use m/h/d)"))),
    };
    if d > Duration::days(90) {
        return Err(Error::Invalid("window must be at most 90d".into()));
    }
    Ok(d)
}

/// `^[A-Za-z0-9_.:/-]{1,128}$`
pub fn ident_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-'))
}

pub fn is_counter(metric: &str) -> bool {
    metric.ends_with("_total")
        || metric.ends_with("_count")
        || metric.ends_with("_sum")
        || metric.ends_with("_bucket")
        || metric == "restarts_total"
}

pub fn in_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ")
}

fn in_list_owned(items: &[String]) -> String {
    items
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `ts >= now() - INTERVAL n SECOND AND ts < now() - INTERVAL m SECOND`
/// (events — small enough to read raw).
fn ts_range(back_secs: i64, until_secs: i64) -> String {
    let mut s = format!("ts >= now() - INTERVAL {back_secs} SECOND");
    if until_secs > 0 {
        s.push_str(&format!(" AND ts < now() - INTERVAL {until_secs} SECOND"));
    }
    s
}

/// Timestamps leave ClickHouse as explicit UTC ISO strings. The embedded
/// server formats `DateTime` in ITS session timezone (the machine's), so a
/// bare `toString(ts)` would be off by the local offset once the UI appends
/// `Z`.
const TS_UTC: &str = "formatDateTime(ts, '%Y-%m-%dT%H:%i:%SZ', 'UTC')";
pub const B_UTC: &str = "formatDateTime(b, '%Y-%m-%dT%H:%i:%SZ', 'UTC')";

/// The HTTP status label of the label map `col`, whichever name the
/// exporter used.
pub fn code_of(col: &str) -> String {
    format!(
        "if({col}['code'] != '', {col}['code'], if({col}['status'] != '', {col}['status'], {col}['status_code']))"
    )
}

fn ns_filter(ns: Option<&str>) -> String {
    ns.filter(|n| !n.is_empty())
        .map(|n| format!(" AND namespace = {}", sql_str(n)))
        .unwrap_or_default()
}

/// `AND namespace IN (…)` for a caller limited to some namespaces (S6-05);
/// `None` = no limit, an empty list matches nothing.
pub fn ns_in_filter(namespaces: Option<&[String]>) -> String {
    match namespaces {
        None => String::new(),
        Some([]) => " AND 0".to_string(),
        Some(list) => format!(" AND namespace IN ({})", in_list_owned(list)),
    }
}

fn wl_filter(workload: Option<&str>) -> String {
    workload
        .filter(|w| !w.is_empty())
        .map(|w| format!(" AND workload = {}", sql_str(w)))
        .unwrap_or_default()
}

fn cids_filter(cluster_ids: &[String]) -> String {
    format!(" AND cluster_id IN ({})", in_list_owned(cluster_ids))
}

/// Latest memory gauge per pod (the last-value table) →
/// `(cluster_id, namespace, workload, pod, mem)`.
pub fn latest_memory_sql(cluster_ids: &[String], ns: Option<&str>, lookback_secs: i64) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, argMax(last_value, last_ts) AS mem, argMax(metric, last_ts) AS gauge
         FROM {latest}
         WHERE cluster_id IN ({cids}) AND metric IN ({gauges}) AND last_ts >= now() - INTERVAL {lookback_secs} SECOND{ns}
         GROUP BY cluster_id, namespace, workload, pod",
        latest = schema::LATEST_TABLE,
        cids = in_list_owned(cluster_ids),
        gauges = in_list(&MEMORY_GAUGES),
        ns = ns_filter(ns),
    )
}

/// Last memory gauge per pod inside `[now-back, now-until)` (trend baseline).
pub fn memory_between_sql(
    cluster_ids: &[String],
    ns: Option<&str>,
    back_secs: i64,
    until_secs: i64,
) -> String {
    memory_between_in(
        &Span::plan(now_secs(), back_secs, until_secs, None),
        cluster_ids,
        ns,
    )
}

pub fn memory_between_in(span: &Span, cluster_ids: &[String], ns: Option<&str>) -> String {
    let filter = format!(
        "{} AND metric IN ({}){}",
        cids_filter(cluster_ids),
        in_list(&MEMORY_GAUGES),
        ns_filter(ns)
    );
    format!(
        "SELECT cluster_id, namespace, workload, pod, tupleElement(max(v_last), 2) AS mem, argMax(metric, v_last) AS gauge
         FROM ({src})
         GROUP BY cluster_id, namespace, workload, pod",
        src = span.source(&filter),
    )
}

/// Restart / churn counts per class → `(cluster_id, namespace, workload, pod, kind, class, n)`.
pub fn restart_counts_sql(cluster_ids: &[String], ns: Option<&str>, window: Duration) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, kind, class, count() AS n
         FROM k8s_events
         WHERE cluster_id IN ({cids}) AND kind IN ('restart', 'churn') AND {range}{ns}
         GROUP BY cluster_id, namespace, workload, pod, kind, class",
        cids = in_list_owned(cluster_ids),
        range = ts_range(window.num_seconds(), 0),
        ns = ns_filter(ns),
    )
}

/// Per-series counter increase rows `(cols…, pod, series, [metric,] lb, delta)`
/// over `span` — the inner half of every rate / latency query. `lb` is the
/// series' label map.
pub fn series_deltas(span: &Span, cols: &str, by_metric: bool, filter: &str) -> String {
    let m = if by_metric { ", metric" } else { "" };
    format!(
        "SELECT {cols}, pod, series{m}, any(labels) AS lb, {DELTA} AS delta
           FROM ({src})
           GROUP BY {cols}, pod, series{m}",
        src = span.source(filter),
    )
}

/// Request rate + 5xx rate per workload over `[now-back, now-until)` →
/// `(cluster_id, namespace, workload, rps, err_rps)`.
pub fn request_rates_sql(
    cluster_ids: &[String],
    ns: Option<&str>,
    back_secs: i64,
    until_secs: i64,
) -> String {
    request_rates_in(
        &Span::plan(now_secs(), back_secs, until_secs, None),
        cluster_ids,
        ns,
    )
}

pub fn request_rates_in(span: &Span, cluster_ids: &[String], ns: Option<&str>) -> String {
    let cols = "cluster_id, namespace, workload";
    let filter = format!(
        "{} AND metric IN ({}){}",
        cids_filter(cluster_ids),
        in_list(&REQUEST_COUNTERS),
        ns_filter(ns)
    );
    format!(
        "SELECT {cols}, sum(delta) / {secs} AS rps, sumIf(delta, startsWith({code}, '5')) / {secs} AS err_rps
         FROM ({inner}) GROUP BY {cols}",
        secs = span.secs,
        code = code_of("lb"),
        inner = series_deltas(span, cols, false, &filter),
    )
}

/// Histogram bucket deltas per workload → `(cluster_id, namespace, workload, le, delta)`;
/// p95 is derived in Rust ([`p95_from_buckets`]).
pub fn latency_buckets_sql(
    cluster_ids: &[String],
    ns: Option<&str>,
    back_secs: i64,
    until_secs: i64,
) -> String {
    latency_buckets_in(
        &Span::plan(now_secs(), back_secs, until_secs, None),
        cluster_ids,
        ns,
    )
}

pub fn latency_buckets_in(span: &Span, cluster_ids: &[String], ns: Option<&str>) -> String {
    let cols = "cluster_id, namespace, workload";
    let filter = format!(
        "{} AND metric IN ({}){}",
        cids_filter(cluster_ids),
        in_list(&LATENCY_BUCKETS),
        ns_filter(ns)
    );
    format!(
        "SELECT {cols}, lb['le'] AS le, sum(delta) AS delta
         FROM ({inner}) GROUP BY {cols}, le",
        inner = series_deltas(span, cols, false, &filter),
    )
}

/// Mean latency fallback (`_sum` / `_count` deltas) → `(cluster_id, namespace, workload, avg_ms)`.
pub fn latency_avg_sql(
    cluster_ids: &[String],
    ns: Option<&str>,
    back_secs: i64,
    until_secs: i64,
) -> String {
    latency_avg_in(
        &Span::plan(now_secs(), back_secs, until_secs, None),
        cluster_ids,
        ns,
    )
}

pub fn latency_avg_in(span: &Span, cluster_ids: &[String], ns: Option<&str>) -> String {
    let cols = "cluster_id, namespace, workload";
    let filter = format!(
        "{} AND metric IN ({}, {}){}",
        cids_filter(cluster_ids),
        in_list(&LATENCY_SUMS),
        in_list(&LATENCY_COUNTS),
        ns_filter(ns)
    );
    format!(
        "SELECT {cols},
                if(sumIf(delta, is_count) > 0, 1000 * sumIf(delta, NOT is_count) / sumIf(delta, is_count), 0) AS avg_ms
         FROM (
           SELECT {cols}, metric IN ({counts}) AS is_count, delta FROM ({inner})
         ) GROUP BY {cols}",
        counts = in_list(&LATENCY_COUNTS),
        inner = series_deltas(span, cols, true, &filter),
    )
}

/// Normalise a chart step so it lines up with a tier: whole minutes from a
/// minute up, whole 5 minutes from 5 minutes, whole hours from an hour.
pub fn align_step(step: u32) -> u32 {
    let up = |s: u32, g: u32| s.div_ceil(g) * g;
    match step {
        s if s >= 3600 => up(s, 3600),
        s if s >= 300 => up(s, 300),
        s if s >= 60 => up(s, 60),
        s => s.max(10),
    }
}

/// A chart step for a window: [`align_step`], and never below a minute once
/// the window is long enough for the minute tier (a sub-minute step would
/// force a raw read).
///
/// Windows of a day or more step in whole hours (the hour tier: 24 rows per
/// series per day instead of 288 on the 5-minute one).
pub fn chart_step(step: u32, window_secs: i64) -> u32 {
    if window_secs >= 86_400 {
        align_step(step.max(3600))
    } else if window_secs >= MIN_BUCKETS * 60 {
        align_step(step.max(60))
    } else {
        align_step(step)
    }
}

/// [`chart_step`] for the wide tiers, which start at a minute.
pub fn wide_step(step: u32, window_secs: i64) -> u32 {
    chart_step(step.max(60), window_secs.max(MIN_BUCKETS * 60))
}

/// Per-bucket value per (pod, series): gauge mean or counter rate.
fn bucket_value(is_counter: bool, step: u32) -> String {
    if is_counter {
        format!("{DELTA} / {step}")
    } else {
        MEAN.to_string()
    }
}

/// Time series for one metric. Gauges: per-bucket avg per pod, summed across
/// the selected pods. Counters: per-bucket rate (Δ/step) summed across pods.
pub fn series_sql(
    cluster_id: &str,
    metric: &str,
    workload: Option<&str>,
    pod: Option<&str>,
    window: Duration,
    step_secs: u32,
    is_counter: bool,
) -> String {
    series_sql_in_namespaces(
        cluster_id, None, metric, workload, pod, window, step_secs, is_counter,
    )
}

/// [`series_sql`] limited to `namespaces` (see [`ns_in_filter`]).
#[allow(clippy::too_many_arguments)]
pub fn series_sql_in_namespaces(
    cluster_id: &str,
    namespaces: Option<&[String]>,
    metric: &str,
    workload: Option<&str>,
    pod: Option<&str>,
    window: Duration,
    step_secs: u32,
    is_counter: bool,
) -> String {
    let step = step_secs.max(10);
    series_in_filtered(
        &Span::plan(now_secs(), window.num_seconds(), 0, Some(step)),
        cluster_id,
        metric,
        workload,
        pod,
        step,
        is_counter,
        &ns_in_filter(namespaces),
    )
}

pub fn series_in(
    span: &Span,
    cluster_id: &str,
    metric: &str,
    workload: Option<&str>,
    pod: Option<&str>,
    step_secs: u32,
    is_counter: bool,
) -> String {
    series_in_filtered(
        span, cluster_id, metric, workload, pod, step_secs, is_counter, "",
    )
}

#[allow(clippy::too_many_arguments)]
fn series_in_filtered(
    span: &Span,
    cluster_id: &str,
    metric: &str,
    workload: Option<&str>,
    pod: Option<&str>,
    step_secs: u32,
    is_counter: bool,
    extra: &str,
) -> String {
    let step = step_secs.max(10);
    let pod_f = pod
        .filter(|p| !p.is_empty())
        .map(|p| format!(" AND pod = {}", sql_str(p)))
        .unwrap_or_default();
    let filter = format!(
        " AND cluster_id = {} AND metric = {}{}{}{}",
        sql_str(cluster_id),
        sql_str(metric),
        wl_filter(workload),
        pod_f,
        extra
    );
    format!(
        "SELECT {b_utc} AS t, sum(v) AS v FROM (
           SELECT {b} AS b, pod, series, {agg} AS v
           FROM ({src})
           GROUP BY b, pod, series
         ) GROUP BY b ORDER BY b",
        b_utc = B_UTC,
        b = bucket("t", step),
        agg = bucket_value(is_counter, step),
        src = span.source(&filter),
    )
}

/// Sparkline buckets for every workload at once → `(workload, t, v)`.
pub fn workload_spark_sql(
    cluster_id: &str,
    ns: Option<&str>,
    metrics: &[&str],
    window: Duration,
    step_secs: u32,
    is_counter: bool,
) -> String {
    let step = step_secs.max(10);
    workload_spark_in(
        &Span::plan(now_secs(), window.num_seconds(), 0, Some(step)),
        cluster_id,
        ns,
        metrics,
        step,
        is_counter,
    )
}

pub fn workload_spark_in(
    span: &Span,
    cluster_id: &str,
    ns: Option<&str>,
    metrics: &[&str],
    step_secs: u32,
    is_counter: bool,
) -> String {
    let step = step_secs.max(10);
    let filter = format!(
        " AND cluster_id = {} AND metric IN ({}){}",
        sql_str(cluster_id),
        in_list(metrics),
        ns_filter(ns)
    );
    format!(
        "SELECT workload, {b_utc} AS t, sum(v) AS v FROM (
           SELECT workload, {b} AS b, pod, series, {agg} AS v
           FROM ({src})
           GROUP BY workload, b, pod, series
         ) GROUP BY workload, b ORDER BY workload, b",
        b_utc = B_UTC,
        b = bucket("t", step),
        agg = bucket_value(is_counter, step),
        src = span.source(&filter),
    )
}

// ---------------------------------------------------------------------------
// Wide tiers (workload / pod rows of reset-aware increments — `wide`)
// ---------------------------------------------------------------------------

/// Which wide table a read uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Workload,
    Pod,
}

impl Level {
    pub fn cols(self) -> &'static str {
        match self {
            Level::Workload => "cluster_id, namespace, workload",
            Level::Pod => "cluster_id, namespace, workload, pod",
        }
    }
}

/// A planned wide read: tier + `[from, to)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WSpan {
    pub tier: WideTier,
    pub from: i64,
    /// Exclusive upper bound (snapped); `None` = up to now.
    pub to: Option<i64>,
    /// Seconds the read covers — the rate denominator.
    pub secs: i64,
}

impl WSpan {
    /// The coarsest wide tier giving `[now − back, now − until)` at least
    /// [`MIN_BUCKETS`] buckets whose grain divides `step` and whose keep
    /// still holds the start; else the finest one that fits.
    pub fn plan(now: i64, back_secs: i64, until_secs: i64, step: Option<u32>) -> WSpan {
        let back = back_secs.max(1);
        let until = until_secs.clamp(0, back - 1);
        let width = back - until;
        let step_ok = |g: i64| step.is_none_or(|s| i64::from(s) % g == 0);
        let holds = |days: u32| back <= i64::from(days) * 86_400;
        let tier = WIDE_TIERS
            .iter()
            .rev()
            .find(|w| w.grain * MIN_BUCKETS <= width && step_ok(w.grain) && holds(w.keep_days))
            .or_else(|| {
                WIDE_TIERS
                    .iter()
                    .find(|w| step_ok(w.grain) && holds(w.keep_days))
            })
            .copied()
            .unwrap_or(schema::WIDE_1H);
        WSpan::on(tier, now, back, until)
    }

    /// `[now − back, now − until)` on a given tier (snapped to its grain).
    pub fn on(tier: WideTier, now: i64, back_secs: i64, until_secs: i64) -> WSpan {
        let g = tier.grain;
        let from = (now - back_secs.max(1)).div_euclid(g) * g;
        let to = (until_secs > 0).then(|| ((now - until_secs).div_euclid(g) * g).max(from + g));
        WSpan::between(tier, from, to, now)
    }

    pub fn between(tier: WideTier, from: i64, to: Option<i64>, now: i64) -> WSpan {
        WSpan {
            tier,
            from,
            to,
            secs: (to.unwrap_or(now) - from).max(1),
        }
    }

    /// A window TOTAL as parts: when the plan lands on the minute tier, the
    /// whole 5-minute buckets inside it come from the 5-minute tier and only
    /// the ragged head and the open edge from the minute tier (≤ 5 + 12 + 5
    /// rows per workload for an hour instead of 60). Increments are
    /// additive, so the parts sum to exactly the single-tier answer.
    pub fn plan_total(now: i64, back_secs: i64, until_secs: i64) -> Vec<WSpan> {
        let base = WSpan::plan(now, back_secs, until_secs, None);
        let c = schema::WIDE_5M;
        if base.tier != schema::WIDE_1M || back_secs > i64::from(c.keep_days) * 86_400 {
            return vec![base];
        }
        let g = c.grain;
        let end = base.to.unwrap_or(now);
        let mid_from = (base.from + g - 1).div_euclid(g) * g;
        let mid_to = end.div_euclid(g) * g;
        if mid_to - mid_from < 2 * g {
            return vec![base];
        }
        let mut parts = Vec::with_capacity(3);
        if base.from < mid_from {
            parts.push(WSpan::between(base.tier, base.from, Some(mid_from), now));
        }
        parts.push(WSpan::between(c, mid_from, Some(mid_to), now));
        if base.to.is_none() || mid_to < end {
            parts.push(WSpan::between(base.tier, mid_to, base.to, now));
        }
        parts
    }

    pub fn table(&self, level: Level) -> &'static str {
        match level {
            Level::Workload => self.tier.wl_table,
            Level::Pod => self.tier.pod_table,
        }
    }

    /// Immutable: the span ends at or before the start of the tier's current
    /// bucket, so nothing the collector writes from now on lands in it.
    pub fn closed(&self, now: i64) -> bool {
        let g = self.tier.grain;
        self.to.is_some_and(|to| to <= now.div_euclid(g) * g)
    }

    fn time_filter(&self) -> String {
        let mut s = format!("t >= toDateTime({})", self.from);
        if let Some(to) = self.to {
            s.push_str(&format!(" AND t < toDateTime({to})"));
        }
        s
    }
}

/// Seconds a list of total parts covers (the rate denominator).
pub fn parts_secs(parts: &[WSpan], now: i64) -> i64 {
    match (parts.first(), parts.last()) {
        (Some(a), Some(b)) => (b.to.unwrap_or(now) - a.from).max(1),
        _ => 1,
    }
}

/// Additive totals per `group` row over one wide span, read from the `src`
/// level → `(group cols…, n, pods, mem_n, mem_sum, mem_max, last_mem_ts,
/// last_mem, req, err, lat_sum, lat_cnt, hist)` — `last_mem*` is the latest
/// cycle's memory (an alias must not shadow the `mem_last` column it reads).
/// `filter` appends to the `WHERE` (leading ` AND …`).
pub fn wide_totals_in(span: &WSpan, src: Level, group: Level, filter: &str) -> String {
    let pods = match src {
        Level::Workload => "max(pods_max)",
        Level::Pod => "uniqExact(pod)",
    };
    let cols = group.cols();
    format!(
        "SELECT {cols}, sum(n) AS n, {pods} AS pods, sum(mem_n) AS mem_n, sum(mem_sum) AS mem_sum,
                max(mem_max) AS mem_max, toUnixTimestamp(tupleElement(max(mem_last), 1)) AS last_mem_ts,
                tupleElement(max(mem_last), 2) AS last_mem, sum(req) AS req, sum(err) AS err,
                sum(lat_sum) AS lat_sum, sum(lat_cnt) AS lat_cnt, sumMap(hist) AS hist
         FROM {table} WHERE {time}{filter}
         GROUP BY {cols}",
        table = span.table(src),
        time = span.time_filter(),
    )
}

/// One row of wide totals (summed across parts).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub n: f64,
    pub pods: f64,
    pub mem_n: f64,
    pub mem_sum: f64,
    pub mem_max: f64,
    pub mem_last_ts: f64,
    pub mem_last: f64,
    pub req: f64,
    pub err: f64,
    pub lat_sum: f64,
    pub lat_cnt: f64,
    pub hist: BTreeMap<String, f64>,
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0)
}

impl Totals {
    pub fn from_row(r: &Value) -> Totals {
        let hist = r
            .get("hist")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| {
                        let v = v
                            .as_f64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                            .unwrap_or(0.0);
                        (k.clone(), v)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Totals {
            n: num(r, "n"),
            pods: num(r, "pods"),
            mem_n: num(r, "mem_n"),
            mem_sum: num(r, "mem_sum"),
            mem_max: num(r, "mem_max"),
            mem_last_ts: num(r, "last_mem_ts"),
            mem_last: num(r, "last_mem"),
            req: num(r, "req"),
            err: num(r, "err"),
            lat_sum: num(r, "lat_sum"),
            lat_cnt: num(r, "lat_cnt"),
            hist,
        }
    }

    pub fn merge(&mut self, o: &Totals) {
        self.n += o.n;
        self.pods = self.pods.max(o.pods);
        self.mem_n += o.mem_n;
        self.mem_sum += o.mem_sum;
        self.mem_max = self.mem_max.max(o.mem_max);
        if o.mem_last_ts > self.mem_last_ts {
            self.mem_last_ts = o.mem_last_ts;
            self.mem_last = o.mem_last;
        }
        self.req += o.req;
        self.err += o.err;
        self.lat_sum += o.lat_sum;
        self.lat_cnt += o.lat_cnt;
        for (le, v) in &o.hist {
            *self.hist.entry(le.clone()).or_default() += v;
        }
    }

    /// Mean memory per sample (a pod's mean; for a workload row, per pod).
    pub fn mem_avg_per_pod(&self) -> f64 {
        if self.mem_n > 0.0 {
            self.mem_sum / self.mem_n
        } else {
            0.0
        }
    }

    /// p95 (ms) from the histogram, else the `sum / count` mean →
    /// `(kind, ms)`; `("", 0)` without latency data.
    pub fn latency(&self) -> (&'static str, f64) {
        let rows: Vec<(String, f64)> = self.hist.iter().map(|(k, v)| (k.clone(), *v)).collect();
        if let Some(p) = p95_from_buckets(&rows) {
            return ("p95", p);
        }
        if self.lat_cnt > 0.0 {
            let ms = 1000.0 * self.lat_sum / self.lat_cnt;
            if ms > 0.0 {
                return ("avg", ms);
            }
        }
        ("", 0.0)
    }
}

/// `(cluster_id, namespace, workload, pod)` — pod empty for workload rows.
pub type RowKey = (String, String, String, String);

/// Run one SELECT; a CLOSED span's answer is immutable, so it is served from
/// the closed-span cache (keyed by the SQL, which carries the snapped
/// bounds) until the tier's next bucket boundary.
pub async fn query_span(
    sink: &dyn MonitorSink,
    span: &WSpan,
    now: i64,
    sql: &str,
) -> Result<Vec<Value>> {
    if !span.closed(now) {
        return sink.query_rows(sql).await;
    }
    if let Some(Value::Array(rows)) = super::cache::get_closed(sql) {
        return Ok(rows);
    }
    let rows = sink.query_rows(sql).await?;
    let g = span.tier.grain;
    let ttl = (now.div_euclid(g) + 1) * g - now;
    super::cache::put_closed(
        sql.to_string(),
        &Value::Array(rows.clone()),
        std::time::Duration::from_secs(ttl.max(1) as u64),
    );
    Ok(rows)
}

/// Wide totals per row over `parts` (see [`WSpan::plan_total`]), merged in
/// Rust; closed parts come from the closed-span cache.
pub async fn wide_totals(
    sink: &dyn MonitorSink,
    parts: &[WSpan],
    now: i64,
    src: Level,
    group: Level,
    filter: &str,
) -> Result<BTreeMap<RowKey, Totals>> {
    let sqls: Vec<String> = parts
        .iter()
        .map(|p| wide_totals_in(p, src, group, filter))
        .collect();
    let results = futures_util::future::join_all(
        parts
            .iter()
            .zip(&sqls)
            .map(|(p, sql)| query_span(sink, p, now, sql)),
    )
    .await;
    let mut out: BTreeMap<RowKey, Totals> = BTreeMap::new();
    for rows in results {
        for r in rows? {
            let st = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let key = (
                st("cluster_id"),
                st("namespace"),
                st("workload"),
                if group == Level::Pod {
                    st("pod")
                } else {
                    String::new()
                },
            );
            out.entry(key).or_default().merge(&Totals::from_row(&r));
        }
    }
    Ok(out)
}

/// Per-workload sparkline buckets (memory: mean workload total per cycle;
/// rps) for one cluster → `(namespace, workload, t, mem, rps)` — keyed by
/// namespace AND workload, so the same name in two namespaces never sums.
pub fn wide_spark_in(span: &WSpan, cluster_id: &str, ns: Option<&str>, step_secs: u32) -> String {
    let step = step_secs.max(60);
    format!(
        "SELECT namespace, workload, {b_utc} AS t, if(sum(n) > 0, sum(mem_sum) / sum(n), 0) AS mem, sum(req) / {step} AS rps
         FROM (
           SELECT {b} AS b, namespace, workload, n, mem_sum, req
           FROM {table} WHERE {time} AND cluster_id = {cid}{ns}
         ) GROUP BY namespace, workload, b ORDER BY namespace, workload, b",
        b_utc = B_UTC,
        b = bucket("t", step),
        table = span.table(Level::Workload),
        time = span.time_filter(),
        cid = sql_str(cluster_id),
        ns = ns_filter(ns),
    )
}

/// What a wide chart line plots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WideMetric {
    Mem,
    Rps,
    Err,
    Latency,
}

/// Bucketed wide series → rows `(g, t, v)` ordered by `g, t`; `g_expr` keys
/// the line (must only use columns of the `src` level), `filter` appends to
/// the `WHERE`.
pub fn wide_series_in(
    span: &WSpan,
    src: Level,
    g_expr: &str,
    filter: &str,
    metric: WideMetric,
    step_secs: u32,
) -> String {
    let step = step_secs.max(60);
    let b = bucket("t", step);
    let table = span.table(src);
    let time = span.time_filter();
    match metric {
        // Memory: per row the mean per cycle (workload total / pod value),
        // then summed into the line.
        WideMetric::Mem => {
            let (cols, v) = match src {
                Level::Workload => (
                    Level::Workload.cols(),
                    "if(sum(n) > 0, sum(mem_sum) / sum(n), 0)",
                ),
                Level::Pod => (
                    Level::Pod.cols(),
                    "if(sum(mem_n) > 0, sum(mem_sum) / sum(mem_n), 0)",
                ),
            };
            format!(
                "SELECT g, {b_utc} AS t, sum(v) AS v FROM (
                   SELECT {g_expr} AS g, {b} AS b, {cols}, {v} AS v
                   FROM {table} WHERE {time}{filter}
                   GROUP BY g, b, {cols}
                 ) GROUP BY g, b ORDER BY g, b",
                b_utc = B_UTC,
            )
        }
        _ => {
            let v = match metric {
                WideMetric::Rps => format!("sum(req) / {step}"),
                WideMetric::Err => "if(sum(req) > 0, 100 * sum(err) / sum(req), 0)".into(),
                _ => "if(sum(lat_cnt) > 0, 1000 * sum(lat_sum) / sum(lat_cnt), 0)".into(),
            };
            format!(
                "SELECT g, {b_utc} AS t, {v} AS v FROM (
                   SELECT {g_expr} AS g, {b} AS b, req, err, lat_sum, lat_cnt
                   FROM {table} WHERE {time}{filter}
                 ) GROUP BY g, b ORDER BY g, b",
                b_utc = B_UTC,
            )
        }
    }
}

/// Classified restarts / churn (+ raw k8s events when `class` is `k8s_event`).
pub fn events_sql(
    cluster_id: &str,
    window: Duration,
    class: Option<&str>,
    workload: Option<&str>,
    limit: u32,
) -> String {
    events_sql_in_namespaces(cluster_id, None, window, class, workload, limit)
}

/// [`events_sql`] limited to `namespaces` (see [`ns_in_filter`]).
pub fn events_sql_in_namespaces(
    cluster_id: &str,
    namespaces: Option<&[String]>,
    window: Duration,
    class: Option<&str>,
    workload: Option<&str>,
    limit: u32,
) -> String {
    let kind_f = match class.filter(|c| !c.is_empty()) {
        Some("k8s_event") => " AND kind = 'k8s_event'".to_string(),
        Some("version") => " AND kind = 'version'".to_string(),
        Some(c) => format!(
            " AND kind IN ('restart', 'churn') AND class = {}",
            sql_str(c)
        ),
        None => " AND kind IN ('restart', 'churn', 'version')".to_string(),
    };
    format!(
        "SELECT {ts_utc} AS ts, namespace, workload, pod, container, kind, class, reason, exit_code, detail, actor
         FROM k8s_events
         WHERE cluster_id = {cid} AND {range}{kind}{wl}{ns}
         ORDER BY ts DESC LIMIT {limit}",
        ts_utc = TS_UTC,
        cid = sql_str(cluster_id),
        range = ts_range(window.num_seconds(), 0),
        kind = kind_f,
        wl = wl_filter(workload),
        ns = ns_in_filter(namespaces),
        limit = limit.clamp(1, 1000),
    )
}

/// Recent restart rows with detail (the health digest's per-restart list).
pub fn recent_restarts_sql(cluster_id: &str, window: Duration, limit: u32) -> String {
    events_sql(cluster_id, window, None, None, limit)
}

/// p95 (in ms) from `(le, delta)` cumulative-bucket rows of one workload;
/// `None` when the histogram is empty.
pub fn p95_from_buckets(rows: &[(String, f64)]) -> Option<f64> {
    let mut b: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|(le, d)| {
            let bound = match le.as_str() {
                "+Inf" | "Inf" => f64::INFINITY,
                s => s.parse::<f64>().ok()?,
            };
            Some((bound, *d))
        })
        .collect();
    if b.is_empty() {
        return None;
    }
    b.sort_by(|a, c| a.0.partial_cmp(&c.0).unwrap_or(std::cmp::Ordering::Equal));
    let total = b.iter().map(|x| x.1).fold(0.0_f64, f64::max);
    if total <= 0.0 {
        return None;
    }
    let target = 0.95 * total;
    let mut prev_bound = 0.0;
    let mut prev_count = 0.0;
    for (bound, cum) in &b {
        if *cum >= target {
            if bound.is_infinite() {
                return Some(prev_bound * 1000.0);
            }
            // Linear interpolation inside the bucket.
            let span = cum - prev_count;
            let frac = if span > 0.0 {
                (target - prev_count) / span
            } else {
                1.0
            };
            return Some((prev_bound + (bound - prev_bound) * frac) * 1000.0);
        }
        prev_bound = *bound;
        prev_count = *cum;
    }
    b.last().map(|(bound, _)| {
        if bound.is_infinite() {
            prev_bound * 1000.0
        } else {
            bound * 1000.0
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_ident() {
        assert_eq!(parse_window("6h").unwrap(), Duration::hours(6));
        assert_eq!(parse_window("7d").unwrap(), Duration::days(7));
        assert_eq!(parse_window("30m").unwrap(), Duration::minutes(30));
        assert_eq!(parse_window("").unwrap(), Duration::hours(24));
        assert!(parse_window("1y").is_err());
        assert!(parse_window("0h").is_err());
        assert!(parse_window("91d").is_err());
        assert!(ident_ok("http_requests_total"));
        assert!(ident_ok("gowithdrawal-confsrv"));
        assert!(!ident_ok("x; DROP"));
        assert!(!ident_ok(""));
        assert!(!ident_ok("a'b"));
    }

    #[test]
    fn series_counter_uses_rate() {
        let q = series_sql(
            "c1",
            "http_requests_total",
            Some("auditlog"),
            None,
            Duration::hours(1),
            60,
            true,
        );
        assert!(q.contains("greatest(0, max(v_max) - min(v_min)) / 60"));
        assert!(q.contains("cluster_id = 'c1'"));
        assert!(q.contains("workload = 'auditlog'"));
        assert!(q.contains("intDiv(toUInt32(toUnixTimestamp(t)), 60) * 60"));
        assert!(
            q.contains("FROM k8s_samples_1m"),
            "1 h at a 60 s step → minute tier"
        );
        assert!(!q.contains("pod ="));
        let g = series_sql(
            "c1",
            "mem_sys_bytes",
            None,
            Some("p-1"),
            Duration::hours(1),
            60,
            false,
        );
        assert!(g.contains("sum(v_sum) / sum(n)"));
        assert!(g.contains("pod = 'p-1'"));
        // A sub-minute step can only be served raw.
        let raw = series_sql("c1", "m", None, None, Duration::hours(1), 30, false);
        assert!(raw.contains("FROM k8s_samples WHERE ts >="));
    }

    const NOW: i64 = 1_790_000_000;

    #[test]
    fn planner_picks_the_coarsest_tier_that_fits() {
        let tier =
            |back: i64, until: i64, step: Option<u32>| Span::plan(NOW, back, until, step).tier;
        let r = |t: Rollup| Tier::Rollup(t);
        // Drill-downs shorter than 24 buckets of a minute stay raw.
        assert_eq!(tier(600, 0, None), Tier::Raw);
        assert_eq!(tier(23 * 60, 0, None), Tier::Raw);
        assert_eq!(tier(24 * 60, 0, None), r(schema::ROLLUP_1M));
        assert_eq!(tier(3600, 0, None), r(schema::ROLLUP_1M));
        assert_eq!(tier(6 * 3600, 0, None), r(schema::ROLLUP_5M));
        assert_eq!(tier(86_400, 0, None), r(schema::ROLLUP_1H));
        assert_eq!(tier(7 * 86_400, 0, None), r(schema::ROLLUP_1H));
        // The step must be a multiple of the grain.
        assert_eq!(tier(86_400, 0, Some(1500)), r(schema::ROLLUP_5M));
        assert_eq!(tier(6 * 3600, 0, Some(180)), r(schema::ROLLUP_1M));
        assert_eq!(tier(3600, 0, Some(30)), Tier::Raw);
        // 24 h baseline before a 1 h window → hour tier.
        assert_eq!(tier(25 * 3600, 3600, None), r(schema::ROLLUP_1H));
        // A 30-min slice 7 days back: past the minute + raw keep → 5 min tier.
        assert_eq!(
            tier(7 * 86_400 + 900, 7 * 86_400 - 900, None),
            r(schema::ROLLUP_5M)
        );
        // ... and 30 days back: only the hour tier still has it.
        assert_eq!(
            tier(30 * 86_400 + 900, 30 * 86_400 - 900, None),
            r(schema::ROLLUP_1H)
        );
    }

    #[test]
    fn spans_snap_to_the_grain_and_count_covered_seconds() {
        let now = 1_790_001_234;
        let s = Span::plan(now, 86_400, 0, None);
        assert_eq!(s.from % 3600, 0);
        assert!(s.from <= now - 86_400 && s.from > now - 86_400 - 3600);
        assert_eq!(s.to, None);
        assert_eq!(s.secs, now - s.from);
        let b = Span::plan(now, 25 * 3600, 3600, None);
        assert_eq!(b.to.unwrap() - b.from, 86_400);
        assert_eq!(b.secs, 86_400);
        // Raw keeps the exact window (and the old denominator).
        let raw = Span::plan(now, 600, 0, None);
        assert_eq!((raw.from, raw.secs), (now - 600, 600));
        // A slice narrower than one grain still covers one bucket.
        let thin = Span::on(Tier::Rollup(schema::ROLLUP_1H), now, 1800, 900);
        assert_eq!(thin.to.unwrap() - thin.from, 3600);
        assert!(raw
            .source("")
            .contains("FROM k8s_samples WHERE ts >= toDateTime("));
        assert!(s
            .source(" AND metric = 'x'")
            .contains("FROM k8s_samples_1h WHERE t >= toDateTime("));
        assert!(b.source("").contains(" AND t < toDateTime("));
    }

    #[test]
    fn steps_align_to_rollup_grains() {
        assert_eq!(align_step(30), 30);
        assert_eq!(align_step(90), 120);
        assert_eq!(align_step(1440), 1500);
        assert_eq!(align_step(3601), 7200);
        assert_eq!(chart_step(30, 3600), 60);
        assert_eq!(chart_step(30, 600), 30);
        // A day or more steps in whole hours (the hour tier).
        assert_eq!(chart_step(1440, 86_400), 3600);
        assert_eq!(chart_step(2160, 86_400), 3600);
        assert_eq!(chart_step(10_080, 7 * 86_400), 10_800);
        assert_eq!(chart_step(540, 6 * 3600), 600);
        assert_eq!(wide_step(30, 600), 60);
        assert_eq!(
            WSpan::plan(NOW, 86_400, 0, Some(chart_step(1440, 86_400))).tier,
            schema::WIDE_1H
        );
    }

    #[test]
    fn wide_planner_and_stitched_totals() {
        let tier = |back: i64, until: i64| WSpan::plan(NOW, back, until, None).tier;
        assert_eq!(tier(600, 0), schema::WIDE_1M, "short windows: finest");
        assert_eq!(tier(3600, 0), schema::WIDE_1M);
        assert_eq!(tier(6 * 3600, 0), schema::WIDE_5M);
        assert_eq!(tier(86_400, 0), schema::WIDE_1H);
        assert_eq!(tier(25 * 3600, 3600), schema::WIDE_1H);
        assert_eq!(tier(30 * 86_400, 0), schema::WIDE_1H);
        // 1 h total: minute head + whole 5-minute buckets + open minute edge,
        // contiguous and covering exactly the single-tier plan.
        let now = 1_790_001_234;
        let base = WSpan::plan(now, 3600, 0, None);
        let parts = WSpan::plan_total(now, 3600, 0);
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0].tier, schema::WIDE_1M);
        assert_eq!(parts[1].tier, schema::WIDE_5M);
        assert_eq!(parts[2].tier, schema::WIDE_1M);
        assert_eq!(parts[0].from, base.from);
        assert_eq!(parts[0].to, Some(parts[1].from));
        assert_eq!(parts[1].to, Some(parts[2].from));
        assert_eq!(parts[2].to, None);
        assert_eq!(parts[1].from % 300, 0);
        assert_eq!(parts_secs(&parts, now), base.secs);
        assert!(parts[0].closed(now) && parts[1].closed(now) && !parts[2].closed(now));
        // A 24 h baseline is one closed hour-tier part.
        let b = WSpan::plan_total(now, 25 * 3600, 3600);
        assert_eq!(b.len(), 1);
        assert!(b[0].closed(now));
        assert_eq!(b[0].secs, 86_400);
        // A grain-aligned start has no head.
        let aligned = 1_790_001_000; // 5-minute aligned
        assert_eq!(WSpan::plan_total(aligned + 30, 3600 + 30, 0).len(), 2);
    }

    #[test]
    fn wide_builders_read_the_wide_tables() {
        let now = 1_790_001_234;
        let s = WSpan::plan(now, 86_400, 0, None);
        let q = wide_totals_in(
            &s,
            Level::Workload,
            Level::Workload,
            " AND cluster_id IN ('c1')",
        );
        assert!(q.contains("FROM k8s_wl_1h WHERE t >= toDateTime("));
        assert!(q.contains("sumMap(hist) AS hist"));
        assert!(q.contains("max(pods_max) AS pods"));
        assert!(
            q.contains("GROUP BY cluster_id, namespace, workload\n")
                || q.ends_with("GROUP BY cluster_id, namespace, workload")
        );
        let p = wide_totals_in(&s, Level::Pod, Level::Pod, "");
        assert!(p.contains("FROM k8s_pod_1h"));
        assert!(p.contains("uniqExact(pod) AS pods"));
        let sp = wide_spark_in(&s, "c'1", Some("shop"), 3600);
        assert!(sp.contains("cluster_id = 'c\\'1'"));
        assert!(sp.contains("GROUP BY namespace, workload, b"));
        assert!(sp.contains("sum(req) / 3600 AS rps"));
        let m = wide_series_in(&s, Level::Workload, "cluster_id", "", WideMetric::Mem, 3600);
        assert!(m.contains("GROUP BY g, b, cluster_id, namespace, workload"));
        let e = wide_series_in(&s, Level::Pod, "pod", "", WideMetric::Err, 30);
        assert!(e.contains("FROM k8s_pod_1h"));
        assert!(
            e.contains("intDiv(toUInt32(toUnixTimestamp(t)), 60) * 60"),
            "step floors at a minute"
        );
        let mut t = Totals::default();
        t.merge(&Totals {
            req: 2.0,
            mem_last_ts: 5.0,
            mem_last: 7.0,
            hist: [("+Inf".to_string(), 3.0)].into(),
            ..Totals::default()
        });
        t.merge(&Totals {
            req: 3.0,
            mem_last_ts: 4.0,
            mem_last: 1.0,
            hist: [("+Inf".to_string(), 1.0)].into(),
            ..Totals::default()
        });
        assert_eq!((t.req, t.mem_last, t.hist["+Inf"]), (5.0, 7.0, 4.0));
        assert_eq!(
            Totals {
                lat_sum: 1.0,
                lat_cnt: 4.0,
                ..Totals::default()
            }
            .latency(),
            ("avg", 250.0)
        );
    }

    #[test]
    fn latest_reads_use_the_last_value_table() {
        let m = latest_memory_sql(&["c1".into()], Some("shop"), 900);
        assert!(m.contains("FROM k8s_latest"));
        assert!(m.contains("argMax(last_value, last_ts) AS mem"));
        assert!(m.contains("namespace = 'shop'"));
    }

    #[test]
    fn namespace_scope_reaches_series_and_events_sql() {
        // S6-05: a namespace-scoped caller's reads carry its grant.
        let shop = vec!["shop".to_string(), "o'k".to_string()];
        let e = events_sql_in_namespaces("c1", Some(&shop), Duration::hours(1), None, None, 10);
        assert!(e.contains("AND namespace IN ('shop', 'o\\'k')"), "{e}");
        let s = series_sql_in_namespaces(
            "c1",
            Some(&shop[..1]),
            "http_requests_total",
            None,
            None,
            Duration::hours(1),
            60,
            true,
        );
        assert!(s.contains("AND namespace IN ('shop')"), "{s}");
        assert!(
            events_sql_in_namespaces("c1", Some(&[]), Duration::hours(1), None, None, 10)
                .contains(" AND 0")
        );
        assert_eq!(
            events_sql("c1", Duration::hours(1), None, None, 10),
            events_sql_in_namespaces("c1", None, Duration::hours(1), None, None, 10)
        );
    }

    #[test]
    fn events_sql_filters() {
        let q = events_sql("c1", Duration::hours(24), Some("oom"), None, 100);
        assert!(q.contains("class = 'oom'"));
        assert!(q.contains("kind IN ('restart', 'churn')"));
        assert!(events_sql("c1", Duration::hours(1), None, None, 10).contains("'version'"));
        assert!(
            events_sql("c1", Duration::hours(1), Some("version"), None, 10)
                .contains("kind = 'version'")
        );
        assert!(q.contains("LIMIT 100"));
        let raw = events_sql(
            "c1",
            Duration::hours(1),
            Some("k8s_event"),
            Some("frb"),
            5000,
        );
        assert!(raw.contains("kind = 'k8s_event'"));
        assert!(raw.contains("workload = 'frb'"));
        assert!(raw.contains("LIMIT 1000"));
    }

    #[test]
    fn counters() {
        assert!(is_counter("http_requests_total"));
        assert!(is_counter("restarts_total"));
        assert!(!is_counter("mem_sys_bytes"));
    }

    #[test]
    fn request_rates_escapes_and_ranges() {
        let q = request_rates_sql(&["c'1".into()], Some("mscasino"), 3600, 0);
        assert!(q.contains("cluster_id IN ('c\\'1')"));
        assert!(q.contains("namespace = 'mscasino'"));
        assert!(q.contains("FROM k8s_samples_1m"));
        assert!(q.contains("startsWith("));
        assert!(q.contains("GROUP BY cluster_id, namespace, workload, pod, series"));
        let b = request_rates_sql(&["c1".into()], None, 90000, 3600);
        assert!(b.contains("FROM k8s_samples_1h"));
        assert!(b.contains(" AND t < toDateTime("));
        assert!(b.contains("/ 86400 AS rps"));
    }

    #[test]
    fn p95_interpolates_and_handles_inf() {
        let rows = vec![
            ("0.1".to_string(), 50.0),
            ("0.5".to_string(), 90.0),
            ("1".to_string(), 96.0),
            ("+Inf".to_string(), 100.0),
        ];
        let p = p95_from_buckets(&rows).unwrap();
        // target 95 lies in the (0.5, 1] bucket: 0.5 + 0.5 * (95-90)/(96-90)
        assert!((p - (0.5 + 0.5 * 5.0 / 6.0) * 1000.0).abs() < 0.01);
        assert_eq!(p95_from_buckets(&[]), None);
        assert_eq!(p95_from_buckets(&[("+Inf".into(), 0.0)]), None);
        let only_inf = vec![("0.25".to_string(), 10.0), ("+Inf".to_string(), 100.0)];
        assert_eq!(p95_from_buckets(&only_inf), Some(250.0));
    }
}
