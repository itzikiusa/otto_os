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
//! Counters are stored raw; rates are `greatest(0, max − min) / seconds` per
//! (pod, label-set) inside the window, summed up. A counter reset inside the
//! window therefore under-counts that pod for the window rather than
//! producing a negative spike. `min`/`max` compose across buckets, so the
//! rollups give exactly the raw answer for the same (snapped) range.

use chrono::Duration;
use otto_core::{Error, Result};

use super::schema::{self, sql_str, Rollup, RAW_KEEP_DAYS, ROLLUPS};

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

/// Latest `version` label per pod (the last-value table) →
/// `(cluster_id, namespace, workload, pod, version)` rows (one per pod);
/// drift = workloads with >1 distinct version.
pub fn versions_sql(cluster_ids: &[String], ns: Option<&str>, lookback_secs: i64) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, argMax(labels['version'], last_ts) AS version
         FROM {latest}
         WHERE cluster_id IN ({cids}) AND labels['version'] != '' AND last_ts >= now() - INTERVAL {lookback_secs} SECOND{ns}
         GROUP BY cluster_id, namespace, workload, pod",
        latest = schema::LATEST_TABLE,
        cids = in_list_owned(cluster_ids),
        ns = ns_filter(ns),
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
pub fn chart_step(step: u32, window_secs: i64) -> u32 {
    if window_secs >= MIN_BUCKETS * 60 {
        align_step(step.max(60))
    } else {
        align_step(step)
    }
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
    let step = step_secs.max(10);
    series_in(
        &Span::plan(now_secs(), window.num_seconds(), 0, Some(step)),
        cluster_id,
        metric,
        workload,
        pod,
        step,
        is_counter,
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
    let step = step_secs.max(10);
    let pod_f = pod
        .filter(|p| !p.is_empty())
        .map(|p| format!(" AND pod = {}", sql_str(p)))
        .unwrap_or_default();
    let filter = format!(
        " AND cluster_id = {} AND metric = {}{}{}",
        sql_str(cluster_id),
        sql_str(metric),
        wl_filter(workload),
        pod_f
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

/// Classified restarts / churn (+ raw k8s events when `class` is `k8s_event`).
pub fn events_sql(
    cluster_id: &str,
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
         WHERE cluster_id = {cid} AND {range}{kind}{wl}
         ORDER BY ts DESC LIMIT {limit}",
        ts_utc = TS_UTC,
        cid = sql_str(cluster_id),
        range = ts_range(window.num_seconds(), 0),
        kind = kind_f,
        wl = wl_filter(workload),
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
    }

    #[test]
    fn latest_reads_use_the_last_value_table() {
        let m = latest_memory_sql(&["c1".into()], Some("shop"), 900);
        assert!(m.contains("FROM k8s_latest"));
        assert!(m.contains("argMax(last_value, last_ts) AS mem"));
        assert!(m.contains("namespace = 'shop'"));
        let v = versions_sql(&["c1".into()], None, 900);
        assert!(v.contains("FROM k8s_latest"));
        assert!(v.contains("labels['version'] != ''"));
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
