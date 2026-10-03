//! The wide pod row: ONE `k8s_pod_cycle` row per pod per cycle, built in
//! Rust from the samples the cycle already has (memory, request / 5xx
//! counts, latency sum / count, the latency histogram as a `le → count`
//! map). The wide views (`schema::wide_views_sql`) fold it into workload-
//! and pod-level 1m / 5m / 1h tiers, which is what the dashboards read.
//!
//! Counters are stored as **increments**, not raw values: the collector
//! keeps the last value of every counter series per cluster ([`Counters`])
//! and writes `v − prev`, or `v` when the value dropped (a counter reset —
//! Prometheus `increase()` semantics). Increments are additive, so the pod
//! and label dimensions fold away inside ClickHouse, and a reset no longer
//! under-counts the window the way the per-series `max − min` did. A series
//! seen for the first time contributes 0 (its history is unknown); after a
//! daemon restart the map is re-seeded once from `k8s_latest` ([`seed_sql`])
//! so a restart costs no interval.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use super::classify::PodSnap;
use super::parse::Sample;
use super::queries::{
    in_list, LATENCY_BUCKETS, LATENCY_COUNTS, LATENCY_SUMS, MEMORY_GAUGES, REQUEST_COUNTERS,
};
use super::schema::{sql_str, LATEST_TABLE};

/// A counter the wide row folds (requests, latency buckets / sum / count).
pub fn is_wide_counter(metric: &str) -> bool {
    REQUEST_COUNTERS.contains(&metric)
        || LATENCY_BUCKETS.contains(&metric)
        || LATENCY_SUMS.contains(&metric)
        || LATENCY_COUNTS.contains(&metric)
}

/// Identity of one counter series — the same `(pod, metric, label set)` the
/// raw table's series are (labels as written, extra pod labels included).
pub fn series_key(ns: &str, pod: &str, metric: &str, labels: &BTreeMap<String, String>) -> String {
    let mut k = format!("{ns}/{pod}\u{1f}{metric}");
    for (a, b) in labels {
        k.push('\u{1f}');
        k.push_str(a);
        k.push('=');
        k.push_str(b);
    }
    k
}

/// Last value per counter series for one cluster.
#[derive(Debug, Default)]
pub struct Counters {
    last: HashMap<String, f64>,
    /// `k8s_latest` was consulted once (or tried) since the daemon started.
    pub seeded: bool,
}

impl Counters {
    /// Reset-aware increment: first sighting 0, a drop = reset → the new value.
    pub fn increment(&mut self, key: String, v: f64) -> f64 {
        let inc = match self.last.get(&key) {
            None => 0.0,
            Some(&p) if v >= p => v - p,
            Some(_) => v,
        };
        self.last.insert(key, v);
        inc
    }

    /// Seed a series' previous value (never overrides a live one).
    pub fn seed(&mut self, key: String, v: f64) {
        self.last.entry(key).or_insert(v);
    }

    /// Forget series of pods that left the snapshot (a pod that merely
    /// failed one scrape keeps its values: the next success spans both
    /// intervals, which is exactly right for an increment).
    pub fn retain_pods(&mut self, live: &std::collections::HashSet<String>) {
        self.last.retain(|k, _| {
            k.split_once('\u{1f}')
                .map(|(pod, _)| live.contains(pod))
                .unwrap_or(false)
        });
    }

    pub fn len(&self) -> usize {
        self.last.len()
    }

    pub fn is_empty(&self) -> bool {
        self.last.is_empty()
    }
}

static STATE: OnceLock<Mutex<HashMap<String, Counters>>> = OnceLock::new();

/// Run `f` on a cluster's counter state (shared by the loop and run-now).
pub fn with_counters<R>(cluster_id: &str, f: impl FnOnce(&mut Counters) -> R) -> R {
    let mut m = STATE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    f(m.entry(cluster_id.to_string()).or_default())
}

/// Last value of every wide counter series still alive for a cluster
/// (`k8s_latest`, counters only) → `(namespace, pod, metric, labels, v)`.
pub fn seed_sql(cluster_id: &str) -> String {
    let mut metrics: Vec<&str> = Vec::new();
    for m in REQUEST_COUNTERS
        .iter()
        .chain(LATENCY_BUCKETS.iter())
        .chain(LATENCY_SUMS.iter())
        .chain(LATENCY_COUNTS.iter())
    {
        if !metrics.contains(m) {
            metrics.push(m);
        }
    }
    format!(
        "SELECT namespace, pod, metric, labels, argMax(last_value, last_ts) AS v
         FROM {LATEST_TABLE}
         WHERE cluster_id = {cid} AND metric IN ({m}) AND last_ts >= now() - INTERVAL 15 MINUTE
         GROUP BY namespace, pod, metric, labels",
        cid = sql_str(cluster_id),
        m = in_list(&metrics),
    )
}

/// Feed [`seed_sql`] rows into the state.
pub fn seed_from_rows(c: &mut Counters, rows: &[Value]) {
    for r in rows {
        let st = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let labels: BTreeMap<String, String> = r
            .get("labels")
            .and_then(Value::as_object)
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let v = r.get("v").and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        });
        if let Some(v) = v {
            c.seed(
                series_key(&st("namespace"), &st("pod"), &st("metric"), &labels),
                v,
            );
        }
    }
}

/// The HTTP status of a label set, whichever name the exporter used
/// (mirrors `queries::code_of`).
pub fn code_of(labels: &BTreeMap<String, String>) -> &str {
    for k in ["code", "status", "status_code"] {
        if let Some(v) = labels.get(k).filter(|v| !v.is_empty()) {
            return v;
        }
    }
    ""
}

/// One pod's wide row for a cycle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PodRow {
    pub mem: f64,
    pub has_mem: bool,
    pub req: f64,
    pub err: f64,
    pub lat_sum: f64,
    pub lat_cnt: f64,
    pub hist: BTreeMap<String, f64>,
    /// Any wide counter series was present this cycle.
    pub has_counters: bool,
}

/// Fold one pod's samples (labels already merged as written to raw) into its
/// wide row, turning counters into increments against `c`.
///
/// Memory: the most authoritative gauge present (`MEMORY_GAUGES` order),
/// SUMMED over its series — a pod's containers (metrics-server) or memory
/// pools (JVM) add up to the pod total.
pub fn pod_row(c: &mut Counters, pod: &PodSnap, samples: &[Sample]) -> PodRow {
    let mut row = PodRow::default();
    let mut best: Option<(usize, f64)> = None;
    for s in samples {
        if let Some(rank) = MEMORY_GAUGES.iter().position(|g| *g == s.metric) {
            best = match best {
                Some((r, v)) if r == rank => Some((r, v + s.value)),
                Some((r, v)) if r < rank => Some((r, v)),
                _ => Some((rank, s.value)),
            };
            continue;
        }
        if !is_wide_counter(&s.metric) || !s.value.is_finite() {
            continue;
        }
        row.has_counters = true;
        let inc = c.increment(
            series_key(&pod.namespace, &pod.name, &s.metric, &s.labels),
            s.value,
        );
        let m = s.metric.as_str();
        if REQUEST_COUNTERS.contains(&m) {
            row.req += inc;
            if code_of(&s.labels).starts_with('5') {
                row.err += inc;
            }
        }
        if LATENCY_SUMS.contains(&m) {
            row.lat_sum += inc;
        }
        if LATENCY_COUNTS.contains(&m) {
            row.lat_cnt += inc;
        }
        if LATENCY_BUCKETS.contains(&m) {
            if let Some(le) = s.labels.get("le").filter(|l| !l.is_empty()) {
                *row.hist.entry(le.clone()).or_default() += inc;
            }
        }
    }
    if let Some((_, v)) = best {
        row.mem = v;
        row.has_mem = true;
    }
    row
}

/// The NDJSON line for one pod's row (`ts` in whole seconds).
pub fn row_ndjson(cluster_id: &str, at: DateTime<Utc>, pod: &PodSnap, row: &PodRow) -> String {
    let mut line = json!({
        "ts": at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "cluster_id": cluster_id,
        "namespace": pod.namespace,
        "workload": pod.workload,
        "pod": pod.name,
        "mem": row.mem,
        "has_mem": u8::from(row.has_mem),
        "req": row.req,
        "err": row.err,
        "lat_sum": row.lat_sum,
        "lat_cnt": row.lat_cnt,
        "hist": row.hist,
    })
    .to_string();
    line.push('\n');
    line
}

/// Wide rows for every pod of a cycle with memory or counter data, under
/// the cluster's counter state (series of vanished pods are dropped).
/// `samples` maps `classify::snap_key` → that pod's merged samples.
pub fn cycle_ndjson(
    cluster_id: &str,
    at: DateTime<Utc>,
    pods: &BTreeMap<String, PodSnap>,
    samples: &HashMap<String, Vec<Sample>>,
) -> String {
    with_counters(cluster_id, |c| {
        let mut out = String::new();
        for (key, pod) in pods {
            let Some(smp) = samples.get(key) else {
                continue;
            };
            let row = pod_row(c, pod, smp);
            if row.has_mem || row.has_counters {
                out.push_str(&row_ndjson(cluster_id, at, pod, &row));
            }
        }
        let live: std::collections::HashSet<String> = pods
            .values()
            .map(|p| format!("{}/{}", p.namespace, p.name))
            .collect();
        c.retain_pods(&live);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(metric: &str, labels: &[(&str, &str)], value: f64) -> Sample {
        Sample {
            metric: metric.into(),
            labels: labels
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            value,
        }
    }

    fn pod() -> PodSnap {
        PodSnap {
            namespace: "shop".into(),
            name: "web-a".into(),
            workload: "web".into(),
            ..PodSnap::default()
        }
    }

    #[test]
    fn increments_are_reset_aware() {
        let mut c = Counters::default();
        assert_eq!(c.increment("k".into(), 100.0), 0.0, "first sighting");
        assert_eq!(c.increment("k".into(), 130.0), 30.0);
        // Reset: the process restarted and counted 7 since.
        assert_eq!(c.increment("k".into(), 7.0), 7.0);
        assert_eq!(c.increment("k".into(), 7.0), 0.0);
        c.seed("k".into(), 1.0);
        assert_eq!(c.increment("k".into(), 9.0), 2.0, "seed never overrides");
    }

    #[test]
    fn pod_row_folds_counters_memory_and_histogram() {
        let mut c = Counters::default();
        let p = pod();
        let cycle = |n: f64| {
            vec![
                s("mem_working_set_bytes", &[], 100.0),
                s("mem_working_set_bytes", &[], 50.0), // a sidecar container
                s("mem_sys_bytes", &[], 999.0),        // less authoritative
                s("http_requests_total", &[("code", "200")], 10.0 * n),
                s("http_requests_total", &[("status", "503")], n),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "0.1")],
                    4.0 * n,
                ),
                s(
                    "http_request_duration_seconds_bucket",
                    &[("le", "+Inf")],
                    11.0 * n,
                ),
                s("http_request_duration_seconds_sum", &[], 0.5 * n),
                s("http_request_duration_seconds_count", &[], 11.0 * n),
                s("restarts_total", &[], 3.0), // not a wide counter
            ]
        };
        let first = pod_row(&mut c, &p, &cycle(1.0));
        assert_eq!(
            (first.mem, first.has_mem),
            (150.0, true),
            "containers summed"
        );
        assert_eq!((first.req, first.err), (0.0, 0.0), "first sighting");
        assert!(first.has_counters);
        let second = pod_row(&mut c, &p, &cycle(3.0));
        assert_eq!(second.req, 22.0);
        assert_eq!(second.err, 2.0);
        assert_eq!(second.lat_sum, 1.0);
        assert_eq!(second.lat_cnt, 22.0);
        assert_eq!(second.hist["0.1"], 8.0);
        assert_eq!(second.hist["+Inf"], 22.0);
        let nd = row_ndjson("c1", Utc::now(), &p, &second);
        let v: Value = serde_json::from_str(nd.trim()).unwrap();
        assert_eq!(v["hist"]["+Inf"], 22.0);
        assert_eq!(v["has_mem"], 1);
        assert!(!v["ts"].as_str().unwrap().contains('.'), "whole seconds");
    }

    #[test]
    fn jvm_pools_sum_and_gauge_rank_wins() {
        let mut c = Counters::default();
        let r = pod_row(
            &mut c,
            &pod(),
            &[
                s("jvm_memory_used_bytes", &[("area", "heap")], 10.0),
                s("jvm_memory_used_bytes", &[("area", "nonheap")], 5.0),
            ],
        );
        assert_eq!(r.mem, 15.0);
        let r = pod_row(
            &mut c,
            &pod(),
            &[
                s("jvm_memory_used_bytes", &[], 10.0),
                s("mem_sys_bytes", &[], 7.0),
            ],
        );
        assert_eq!(r.mem, 7.0, "mem_sys_bytes outranks the JVM pools");
    }

    #[test]
    fn cycle_rows_skip_empty_pods_and_forget_vanished_series() {
        let cid = "wide-test-cycle";
        let p = pod();
        let mut pods = BTreeMap::new();
        pods.insert("shop/web-a".to_string(), p.clone());
        let mut other = p.clone();
        other.name = "web-b".into();
        pods.insert("shop/web-b".to_string(), other);
        let mut samples = HashMap::new();
        samples.insert(
            "shop/web-a".to_string(),
            vec![s("http_requests_total", &[("code", "200")], 5.0)],
        );
        let nd = cycle_ndjson(cid, Utc::now(), &pods, &samples);
        assert_eq!(nd.lines().count(), 1, "web-b has no data");
        assert_eq!(with_counters(cid, |c| c.len()), 1);
        pods.remove("shop/web-a");
        cycle_ndjson(cid, Utc::now(), &pods, &samples);
        assert_eq!(with_counters(cid, |c| c.len()), 0, "web-a left");
    }

    #[test]
    fn seed_reads_counters_only_and_matches_series_keys() {
        let q = seed_sql("c'1");
        assert!(q.contains("cluster_id = 'c\\'1'"));
        assert!(q.contains("'http_requests_total'"));
        assert!(!q.contains("mem_"));
        let mut c = Counters::default();
        seed_from_rows(
            &mut c,
            &[
                json!({"namespace": "shop", "pod": "web-a", "metric": "http_requests_total",
                      "labels": {"code": "200"}, "v": 40.0}),
            ],
        );
        let p = pod();
        let r = pod_row(
            &mut c,
            &p,
            &[s("http_requests_total", &[("code", "200")], 45.0)],
        );
        assert_eq!(r.req, 5.0, "seeded series continues across a restart");
    }
}
