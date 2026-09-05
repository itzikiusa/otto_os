//! ClickHouse SQL builders for the dashboard + health digest. Every string
//! value goes through [`sql_str`]; every identifier-like filter (metric,
//! workload, pod, namespace, class) must pass [`ident_ok`] first — the HTTP
//! layer rejects anything else with 400 before it gets here.
//!
//! Counters are stored raw; rates are `greatest(0, max − min) / seconds` per
//! (pod, label-set) inside the window, summed up. A counter reset inside the
//! window therefore under-counts that pod for the window rather than
//! producing a negative spike.

use chrono::Duration;
use otto_core::{Error, Result};

use super::schema::sql_str;

/// Request-counter series recognised for rps / error-rate (Go + Spring).
pub const REQUEST_COUNTERS: [&str; 2] = ["http_requests_total", "http_server_requests_seconds_count"];
/// Latency histogram bucket series (Go + Spring).
pub const LATENCY_BUCKETS: [&str; 2] = ["http_request_duration_seconds_bucket", "http_server_requests_seconds_bucket"];
pub const LATENCY_SUMS: [&str; 2] = ["http_request_duration_seconds_sum", "http_server_requests_seconds_sum"];
pub const LATENCY_COUNTS: [&str; 2] = ["http_request_duration_seconds_count", "http_server_requests_seconds_count"];
/// Memory gauges, most authoritative first.
pub const MEMORY_GAUGES: [&str; 3] = ["mem_working_set_bytes", "mem_sys_bytes", "jvm_memory_used_bytes"];

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
        && s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '/' | '-'))
}

pub fn is_counter(metric: &str) -> bool {
    metric.ends_with("_total")
        || metric.ends_with("_count")
        || metric.ends_with("_sum")
        || metric.ends_with("_bucket")
        || metric == "restarts_total"
}

fn in_list(items: &[&str]) -> String {
    items.iter().map(|s| sql_str(s)).collect::<Vec<_>>().join(", ")
}

fn in_list_owned(items: &[String]) -> String {
    items.iter().map(|s| sql_str(s)).collect::<Vec<_>>().join(", ")
}

/// `ts >= now() - INTERVAL n SECOND AND ts < now() - INTERVAL m SECOND`.
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
const T_UTC: &str = "formatDateTime(t, '%Y-%m-%dT%H:%i:%SZ', 'UTC')";

/// The HTTP status label, whichever name the exporter used.
const CODE_EXPR: &str =
    "if(labels['code'] != '', labels['code'], if(labels['status'] != '', labels['status'], labels['status_code']))";

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

/// Latest memory gauge per pod (best available gauge) → `(cluster_id, namespace, workload, pod, mem)`.
pub fn latest_memory_sql(cluster_ids: &[String], ns: Option<&str>, lookback_secs: i64) -> String {
    memory_between_sql(cluster_ids, ns, lookback_secs, 0)
}

/// Last memory gauge per pod inside `[now-back, now-until)` (trend baseline).
pub fn memory_between_sql(cluster_ids: &[String], ns: Option<&str>, back_secs: i64, until_secs: i64) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, argMax(value, ts) AS mem, argMax(metric, ts) AS metric
         FROM k8s_samples
         WHERE cluster_id IN ({cids}) AND metric IN ({gauges}) AND {range}{ns}
         GROUP BY cluster_id, namespace, workload, pod",
        cids = in_list_owned(cluster_ids),
        gauges = in_list(&MEMORY_GAUGES),
        range = ts_range(back_secs, until_secs),
        ns = ns_filter(ns),
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

/// Request rate + 5xx rate per workload over `[now-back, now-until)` →
/// `(cluster_id, namespace, workload, rps, err_rps)`.
pub fn request_rates_sql(cluster_ids: &[String], ns: Option<&str>, back_secs: i64, until_secs: i64) -> String {
    let secs = (back_secs - until_secs).max(1);
    format!(
        "SELECT cluster_id, namespace, workload, sum(delta) / {secs} AS rps, sumIf(delta, is5xx) / {secs} AS err_rps
         FROM (
           SELECT cluster_id, namespace, workload, pod, labels, startsWith({code}, '5') AS is5xx,
                  greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE cluster_id IN ({cids}) AND metric IN ({counters}) AND {range}{ns}
           GROUP BY cluster_id, namespace, workload, pod, labels
         ) GROUP BY cluster_id, namespace, workload",
        code = CODE_EXPR,
        cids = in_list_owned(cluster_ids),
        counters = in_list(&REQUEST_COUNTERS),
        range = ts_range(back_secs, until_secs),
        ns = ns_filter(ns),
    )
}

/// Histogram bucket deltas per workload → `(cluster_id, namespace, workload, le, delta)`;
/// p95 is derived in Rust ([`p95_from_buckets`]).
pub fn latency_buckets_sql(cluster_ids: &[String], ns: Option<&str>, back_secs: i64, until_secs: i64) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, le, sum(delta) AS delta
         FROM (
           SELECT cluster_id, namespace, workload, pod, labels['le'] AS le, labels, greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE cluster_id IN ({cids}) AND metric IN ({buckets}) AND {range}{ns}
           GROUP BY cluster_id, namespace, workload, pod, labels
         ) GROUP BY cluster_id, namespace, workload, le",
        cids = in_list_owned(cluster_ids),
        buckets = in_list(&LATENCY_BUCKETS),
        range = ts_range(back_secs, until_secs),
        ns = ns_filter(ns),
    )
}

/// Mean latency fallback (`_sum` / `_count` deltas) → `(cluster_id, namespace, workload, avg_ms)`.
pub fn latency_avg_sql(cluster_ids: &[String], ns: Option<&str>, back_secs: i64, until_secs: i64) -> String {
    format!(
        "SELECT cluster_id, namespace, workload,
                if(sumIf(delta, is_count) > 0, 1000 * sumIf(delta, NOT is_count) / sumIf(delta, is_count), 0) AS avg_ms
         FROM (
           SELECT cluster_id, namespace, workload, pod, labels, metric IN ({counts}) AS is_count,
                  greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE cluster_id IN ({cids}) AND metric IN ({sums}, {counts}) AND {range}{ns}
           GROUP BY cluster_id, namespace, workload, pod, labels, metric
         ) GROUP BY cluster_id, namespace, workload",
        counts = in_list(&LATENCY_COUNTS),
        sums = in_list(&LATENCY_SUMS),
        cids = in_list_owned(cluster_ids),
        range = ts_range(back_secs, until_secs),
        ns = ns_filter(ns),
    )
}

/// Latest `version` label per pod → `(cluster_id, namespace, workload, version)` rows
/// (one per pod); drift = workloads with >1 distinct version.
pub fn versions_sql(cluster_ids: &[String], ns: Option<&str>, lookback_secs: i64) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, argMax(labels['version'], ts) AS version
         FROM k8s_samples
         WHERE cluster_id IN ({cids}) AND labels['version'] != '' AND {range}{ns}
         GROUP BY cluster_id, namespace, workload, pod",
        cids = in_list_owned(cluster_ids),
        range = ts_range(lookback_secs, 0),
        ns = ns_filter(ns),
    )
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
    let pod_f = pod
        .filter(|p| !p.is_empty())
        .map(|p| format!(" AND pod = {}", sql_str(p)))
        .unwrap_or_default();
    let inner_agg = if is_counter {
        format!("greatest(0, max(value) - min(value)) / {step}")
    } else {
        "avg(value)".to_string()
    };
    format!(
        "SELECT {t_utc} AS t, sum(v) AS v FROM (
           SELECT toStartOfInterval(ts, INTERVAL {step} SECOND) AS t, pod, labels, {inner_agg} AS v
           FROM k8s_samples
           WHERE cluster_id = {cid} AND metric = {metric} AND {range}{wl}{pod}
           GROUP BY t, pod, labels
         ) GROUP BY t ORDER BY t",
        t_utc = T_UTC,
        cid = sql_str(cluster_id),
        metric = sql_str(metric),
        range = ts_range(window.num_seconds(), 0),
        wl = wl_filter(workload),
        pod = pod_f,
    )
}

/// Sparkline buckets for every workload at once → `(workload, t, v)`.
pub fn workload_spark_sql(cluster_id: &str, ns: Option<&str>, metrics: &[&str], window: Duration, step_secs: u32, is_counter: bool) -> String {
    let step = step_secs.max(10);
    let inner_agg = if is_counter {
        format!("greatest(0, max(value) - min(value)) / {step}")
    } else {
        "avg(value)".to_string()
    };
    format!(
        "SELECT workload, t, sum(v) AS v FROM (
           SELECT workload, toStartOfInterval(ts, INTERVAL {step} SECOND) AS t, pod, labels, {inner_agg} AS v
           FROM k8s_samples
           WHERE cluster_id = {cid} AND metric IN ({metrics}) AND {range}{ns}
           GROUP BY workload, t, pod, labels
         ) GROUP BY workload, t ORDER BY workload, t",
        cid = sql_str(cluster_id),
        metrics = in_list(metrics),
        range = ts_range(window.num_seconds(), 0),
        ns = ns_filter(ns),
    )
}

/// Classified restarts / churn (+ raw k8s events when `class` is `k8s_event`).
pub fn events_sql(cluster_id: &str, window: Duration, class: Option<&str>, workload: Option<&str>, limit: u32) -> String {
    let kind_f = match class.filter(|c| !c.is_empty()) {
        Some("k8s_event") => " AND kind = 'k8s_event'".to_string(),
        Some("version") => " AND kind = 'version'".to_string(),
        Some(c) => format!(" AND kind IN ('restart', 'churn') AND class = {}", sql_str(c)),
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
            let frac = if span > 0.0 { (target - prev_count) / span } else { 1.0 };
            return Some((prev_bound + (bound - prev_bound) * frac) * 1000.0);
        }
        prev_bound = *bound;
        prev_count = *cum;
    }
    b.last().map(|(bound, _)| if bound.is_infinite() { prev_bound * 1000.0 } else { bound * 1000.0 })
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
        let q = series_sql("c1", "http_requests_total", Some("auditlog"), None, Duration::hours(1), 60, true);
        assert!(q.contains("greatest(0, max(value) - min(value)) / 60"));
        assert!(q.contains("cluster_id = 'c1'"));
        assert!(q.contains("workload = 'auditlog'"));
        assert!(q.contains("toStartOfInterval(ts, INTERVAL 60 SECOND)"));
        assert!(!q.contains("pod ="));
        let g = series_sql("c1", "mem_sys_bytes", None, Some("p-1"), Duration::hours(1), 60, false);
        assert!(g.contains("avg(value)"));
        assert!(g.contains("pod = 'p-1'"));
    }

    #[test]
    fn events_sql_filters() {
        let q = events_sql("c1", Duration::hours(24), Some("oom"), None, 100);
        assert!(q.contains("class = 'oom'"));
        assert!(q.contains("kind IN ('restart', 'churn')"));
        assert!(events_sql("c1", Duration::hours(1), None, None, 10).contains("'version'"));
        assert!(events_sql("c1", Duration::hours(1), Some("version"), None, 10).contains("kind = 'version'"));
        assert!(q.contains("LIMIT 100"));
        let raw = events_sql("c1", Duration::hours(1), Some("k8s_event"), Some("frb"), 5000);
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
        assert!(q.contains("/ 3600"));
        assert!(q.contains("startsWith("));
        let b = request_rates_sql(&["c1".into()], None, 90000, 3600);
        assert!(b.contains("ts < now() - INTERVAL 3600 SECOND"));
        assert!(b.contains("/ 86400"));
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
