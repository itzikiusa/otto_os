//! Per-workload statistics (the dashboard's workloads table) and the compact
//! `k8s_health` digest an agent can reason over (spec "`k8s_health` MCP
//! tool"). Both are assembled from a handful of ClickHouse queries plus the
//! collector's last pod snapshot (pod counts, memory limits, workload kinds
//! come from the snapshot — they are exact and free).

use std::collections::BTreeMap;

use chrono::Duration;
use otto_core::Result;
use otto_state::{K8sCluster, K8sMonitorStatusRow};
use serde::Serialize;
use serde_json::{json, Value};

use super::classify::{PodSnap, Snapshot};
use super::queries;
use crate::MonitorSink;

/// Memory ≥ this % of the limit is an outlier.
pub const MEM_PCT: f64 = 85.0;
/// Memory growth over the window ≥ this % is an outlier.
pub const MEM_TREND_PCT: f64 = 25.0;
/// Error rate ≥ `ERR_MULT` × baseline **and** ≥ `ERR_MIN_PCT` is a spike.
pub const ERR_MULT: f64 = 3.0;
pub const ERR_MIN_PCT: f64 = 1.0;
/// p95 ≥ this × baseline is a spike.
pub const P95_MULT: f64 = 3.0;
/// Lists in the digest are truncated to this many entries.
pub const LIST_CAP: usize = 20;
/// Sample lookback for "latest" gauges (covers a slow cycle at 60 s intervals).
const LATEST_SECS: i64 = 15 * 60;

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct RestartCounts {
    pub oom: u32,
    pub crash: u32,
    pub probe: u32,
    pub unknown: u32,
}

impl RestartCounts {
    pub fn total(&self) -> u32 {
        self.oom + self.crash + self.probe + self.unknown
    }
    fn bump(&mut self, class: &str, n: u32) {
        match class {
            "oom" => self.oom += n,
            "crash" => self.crash += n,
            "probe" => self.probe += n,
            _ => self.unknown += n,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct WorkloadStat {
    pub namespace: String,
    pub workload: String,
    pub kind: String,
    pub pods: u32,
    pub ready: u32,
    pub mem_bytes: f64,
    pub mem_limit: f64,
    /// 0 when no limit.
    pub mem_pct: f64,
    /// Δ% of memory across the window (`None` = no baseline sample).
    pub mem_trend_pct: Option<f64>,
    pub restarts: RestartCounts,
    pub churn_planned: u32,
    pub churn_unknown: u32,
    pub rps: f64,
    pub err_pct: f64,
    pub err_pct_baseline: f64,
    pub rps_baseline: f64,
    /// `p95` | `avg` | `` (no latency data).
    pub latency_kind: String,
    pub latency_ms: f64,
    pub latency_baseline_ms: f64,
    pub versions: Vec<String>,
    pub crashloop: u32,
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0.0)
}
fn st<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn key(ns: &str, wl: &str) -> String {
    format!("{ns}/{wl}")
}

/// Seed one stat per workload from the pod snapshot.
fn seed_from_snapshot(snap: &Snapshot, ns: Option<&str>) -> BTreeMap<String, WorkloadStat> {
    let mut m: BTreeMap<String, WorkloadStat> = BTreeMap::new();
    for p in snap.values() {
        if let Some(n) = ns {
            if !n.is_empty() && p.namespace != n {
                continue;
            }
        }
        let e = m.entry(key(&p.namespace, &p.workload)).or_insert_with(|| WorkloadStat {
            namespace: p.namespace.clone(),
            workload: p.workload.clone(),
            kind: p.workload_kind.clone(),
            ..WorkloadStat::default()
        });
        e.pods += 1;
        if p.ready {
            e.ready += 1;
        }
        e.mem_limit += p.mem_limit as f64;
        if p.containers.values().any(|c| c.waiting_reason == "CrashLoopBackOff") {
            e.crashloop += 1;
        }
    }
    m
}

/// Everything the workloads table / digest needs, one row per workload.
pub async fn workload_stats(
    sink: &dyn MonitorSink,
    cluster_id: &str,
    snap: &Snapshot,
    ns: Option<&str>,
    window: Duration,
) -> Result<Vec<WorkloadStat>> {
    let cids = vec![cluster_id.to_string()];
    let secs = window.num_seconds();
    let mut stats = seed_from_snapshot(snap, ns);

    // Memory now + at the start of the window (trend).
    let mem_now = sink.query_rows(&queries::latest_memory_sql(&cids, ns, LATEST_SECS)).await?;
    let mut mem_then_by_wl: BTreeMap<String, f64> = BTreeMap::new();
    if secs > LATEST_SECS {
        let then = sink
            .query_rows(&queries::memory_between_sql(&cids, ns, secs + LATEST_SECS, secs - LATEST_SECS))
            .await
            .unwrap_or_default();
        for r in then {
            *mem_then_by_wl.entry(key(st(&r, "namespace"), st(&r, "workload"))).or_default() += f(&r, "mem");
        }
    }
    for r in mem_now {
        let k = key(st(&r, "namespace"), st(&r, "workload"));
        if let Some(s) = stats.get_mut(&k) {
            s.mem_bytes += f(&r, "mem");
        }
    }
    for (k, s) in stats.iter_mut() {
        if s.mem_limit > 0.0 {
            s.mem_pct = 100.0 * s.mem_bytes / s.mem_limit;
        }
        if let Some(then) = mem_then_by_wl.get(k) {
            if *then > 0.0 {
                s.mem_trend_pct = Some(100.0 * (s.mem_bytes - then) / then);
            }
        }
    }

    // Restarts / churn.
    for r in sink.query_rows(&queries::restart_counts_sql(&cids, ns, window)).await? {
        // The events table has no namespace filter on workload key; match by workload name within ns.
        let wl = st(&r, "workload");
        let n = f(&r, "n") as u32;
        let class = st(&r, "class").to_string();
        let kind = st(&r, "kind").to_string();
        for s in stats.values_mut().filter(|s| s.workload == wl) {
            match kind.as_str() {
                "restart" => s.restarts.bump(&class, n),
                "churn" => {
                    if class == "planned" || class == "completed" {
                        s.churn_planned += n;
                    } else {
                        s.churn_unknown += n;
                    }
                }
                _ => {}
            }
        }
    }

    // Request rates now vs 24 h baseline (the 24 h preceding the window).
    let rates_now = sink.query_rows(&queries::request_rates_sql(&cids, ns, secs, 0)).await?;
    let rates_then = sink
        .query_rows(&queries::request_rates_sql(&cids, ns, secs + 86_400, secs))
        .await
        .unwrap_or_default();
    for r in rates_now {
        if let Some(s) = stats.get_mut(&key(st(&r, "namespace"), st(&r, "workload"))) {
            s.rps = f(&r, "rps");
            s.err_pct = if s.rps > 0.0 { 100.0 * f(&r, "err_rps") / s.rps } else { 0.0 };
        }
    }
    for r in rates_then {
        if let Some(s) = stats.get_mut(&key(st(&r, "namespace"), st(&r, "workload"))) {
            let rps = f(&r, "rps");
            s.rps_baseline = rps;
            s.err_pct_baseline = if rps > 0.0 { 100.0 * f(&r, "err_rps") / rps } else { 0.0 };
        }
    }

    // Latency: p95 from buckets, else avg from sum/count.
    let mut had_p95 = false;
    for (rows, baseline) in [
        (sink.query_rows(&queries::latency_buckets_sql(&cids, ns, secs, 0)).await?, false),
        (
            sink.query_rows(&queries::latency_buckets_sql(&cids, ns, secs + 86_400, secs))
                .await
                .unwrap_or_default(),
            true,
        ),
    ] {
        let mut by_wl: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
        for r in rows {
            by_wl
                .entry(st(&r, "workload").to_string())
                .or_default()
                .push((st(&r, "le").to_string(), f(&r, "delta")));
        }
        for (wl, buckets) in by_wl {
            if let Some(p95) = queries::p95_from_buckets(&buckets) {
                for s in stats.values_mut().filter(|s| s.workload == wl) {
                    s.latency_kind = "p95".into();
                    if baseline {
                        s.latency_baseline_ms = p95;
                    } else {
                        s.latency_ms = p95;
                        had_p95 = true;
                    }
                }
            }
        }
    }
    if !had_p95 {
        for (rows, baseline) in [
            (sink.query_rows(&queries::latency_avg_sql(&cids, ns, secs, 0)).await.unwrap_or_default(), false),
            (
                sink.query_rows(&queries::latency_avg_sql(&cids, ns, secs + 86_400, secs))
                    .await
                    .unwrap_or_default(),
                true,
            ),
        ] {
            for r in rows {
                let wl = st(&r, "workload");
                let v = f(&r, "avg_ms");
                if v <= 0.0 {
                    continue;
                }
                for s in stats.values_mut().filter(|s| s.workload == wl) {
                    s.latency_kind = "avg".into();
                    if baseline {
                        s.latency_baseline_ms = v;
                    } else {
                        s.latency_ms = v;
                    }
                }
            }
        }
    }

    // Versions.
    for r in sink.query_rows(&queries::versions_sql(&cids, ns, LATEST_SECS)).await.unwrap_or_default() {
        let wl = st(&r, "workload");
        let v = st(&r, "version").to_string();
        if v.is_empty() {
            continue;
        }
        for s in stats.values_mut().filter(|s| s.workload == wl) {
            if !s.versions.contains(&v) {
                s.versions.push(v.clone());
            }
        }
    }

    Ok(stats.into_values().collect())
}

/// Outlier lists (memory, error rate, latency) — pure, threshold-driven.
pub fn outliers(stats: &[WorkloadStat]) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
    let mut mem: Vec<(f64, Value)> = Vec::new();
    let mut err: Vec<(f64, Value)> = Vec::new();
    let mut lat: Vec<(f64, Value)> = Vec::new();
    for s in stats {
        let trend_hit = s.mem_trend_pct.map(|t| t >= MEM_TREND_PCT).unwrap_or(false);
        if (s.mem_limit > 0.0 && s.mem_pct >= MEM_PCT) || (trend_hit && s.mem_bytes > 0.0) {
            mem.push((
                s.mem_pct.max(s.mem_trend_pct.unwrap_or(0.0)),
                json!({
                    "namespace": s.namespace, "workload": s.workload,
                    "mem": human_bytes(s.mem_bytes), "limit": if s.mem_limit > 0.0 { human_bytes(s.mem_limit) } else { "none".into() },
                    "pct": round1(s.mem_pct),
                    "trend": s.mem_trend_pct.map(|t| format!("{:+.0}%", t)),
                }),
            ));
        }
        if s.rps > 0.0 && s.err_pct >= ERR_MIN_PCT && s.err_pct >= ERR_MULT * s.err_pct_baseline.max(0.1) {
            err.push((
                s.err_pct,
                json!({
                    "namespace": s.namespace, "workload": s.workload,
                    "err_pct": round1(s.err_pct), "baseline_pct": round1(s.err_pct_baseline), "rps": round1(s.rps),
                }),
            ));
        }
        if s.latency_ms > 0.0 && s.latency_baseline_ms > 0.0 && s.latency_ms >= P95_MULT * s.latency_baseline_ms {
            lat.push((
                s.latency_ms,
                json!({
                    "namespace": s.namespace, "workload": s.workload, "kind": s.latency_kind,
                    "ms": round1(s.latency_ms), "baseline_ms": round1(s.latency_baseline_ms),
                }),
            ));
        }
    }
    let take = |mut v: Vec<(f64, Value)>| -> Vec<Value> {
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        v.into_iter().take(LIST_CAP).map(|(_, j)| j).collect()
    };
    (take(mem), take(err), take(lat))
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn human_bytes(b: f64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}{}", U[i])
    } else {
        format!("{v:.1}{}", U[i])
    }
}

fn pod_counts(snap: &Snapshot) -> Value {
    let mut running = 0;
    let mut pending = 0;
    let mut failed = 0;
    let mut succeeded = 0;
    let mut crashloop = 0;
    for p in snap.values() {
        match p.phase.as_str() {
            "Running" => running += 1,
            "Pending" => pending += 1,
            "Failed" => failed += 1,
            "Succeeded" => succeeded += 1,
            _ => {}
        }
        if p.containers.values().any(|c| c.waiting_reason == "CrashLoopBackOff") {
            crashloop += 1;
        }
    }
    json!({"running": running, "pending": pending, "failed": failed, "succeeded": succeeded, "crashloop": crashloop, "total": snap.len()})
}

fn snapshot_of(status: &K8sMonitorStatusRow) -> Snapshot {
    serde_json::from_value(status.snapshot.clone()).unwrap_or_default()
}

fn limit_of<'a>(snap: &'a Snapshot, ns: &str, pod: &str) -> Option<&'a PodSnap> {
    snap.get(&super::classify::snap_key(ns, pod))
}

/// The `k8s_health` payload (spec shape). Always small: every list is capped
/// at [`LIST_CAP`] entries.
pub async fn health(
    sink: &dyn MonitorSink,
    cluster: &K8sCluster,
    status: &K8sMonitorStatusRow,
    enabled: bool,
    window: Duration,
    window_label: &str,
) -> Result<Value> {
    let snap = snapshot_of(status);
    let cid = cluster.id.to_string();
    let stats = workload_stats(sink, &cid, &snap, None, window).await?;
    let (mem, err, lat) = outliers(&stats);

    // Per-restart detail lists by class + churn summary.
    let mut restarts: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for c in ["oom", "crash", "probe", "unknown"] {
        restarts.insert(c.to_string(), vec![]);
    }
    let mut churn: BTreeMap<(String, String), (u32, String)> = BTreeMap::new();
    for r in sink.query_rows(&queries::recent_restarts_sql(&cid, window, 500)).await? {
        let kind = st(&r, "kind");
        let class = st(&r, "class").to_string();
        if kind == "restart" {
            let ns = st(&r, "namespace");
            let pod = st(&r, "pod");
            let limit = limit_of(&snap, ns, pod).map(|p| p.mem_limit).unwrap_or(0);
            let mut item = json!({
                "workload": st(&r, "workload"), "pod": pod, "container": st(&r, "container"),
                "at": st(&r, "ts"), "reason": st(&r, "reason"), "exit_code": f(&r, "exit_code") as i64,
            });
            if class == "oom" {
                item["mem_limit"] = json!(if limit > 0 { human_bytes(limit as f64) } else { "none".into() });
            }
            let list = restarts.entry(class.clone()).or_default();
            if list.len() < LIST_CAP {
                list.push(item);
            }
        } else if kind == "churn" {
            let detail: Value = serde_json::from_str(st(&r, "detail")).unwrap_or(Value::Null);
            let by = detail.get("planned_by").and_then(Value::as_str).unwrap_or("").to_string();
            let e = churn
                .entry((st(&r, "workload").to_string(), class.clone()))
                .or_insert((0, by));
            e.0 += 1;
        }
    }
    let churn_list: Vec<Value> = churn
        .into_iter()
        .take(LIST_CAP)
        .map(|((wl, class), (n, by))| json!({"workload": wl, "class": class, "pods": n, "by": by}))
        .collect();
    let drift: Vec<Value> = stats
        .iter()
        .filter(|s| s.versions.len() > 1)
        .take(LIST_CAP)
        .map(|s| json!({"namespace": s.namespace, "workload": s.workload, "versions": s.versions}))
        .collect();
    let unplanned: u32 = stats.iter().map(|s| s.restarts.total()).sum();

    Ok(json!({
        "cluster": cluster.name,
        "cluster_id": cluster.id,
        "environment": cluster.environment,
        "window": window_label,
        "collected_at": status.last_cycle_at,
        "collector": {
            "enabled": enabled,
            "ok": status.last_ok_at.is_some() && status.last_error.is_empty(),
            "last_ok_at": status.last_ok_at,
            "error": status.last_error,
            "transport": status.transport_used,
            "metrics_server": status.metrics_server,
            "pods_seen": status.pods_seen,
            "pods_scraped": status.pods_scraped,
            "pods_failed": status.pods_failed,
            "cycle_ms": status.cycle_ms,
        },
        "pods": pod_counts(&snap),
        "unplanned_restarts": unplanned,
        "restarts": restarts,
        "churn": churn_list,
        "memory_outliers": mem,
        "error_rate": err,
        "latency": lat,
        "drift": drift,
        "thresholds": {
            "mem_pct": MEM_PCT, "mem_trend_pct": MEM_TREND_PCT,
            "err_mult": ERR_MULT, "err_min_pct": ERR_MIN_PCT, "p95_mult": P95_MULT,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(wl: &str) -> WorkloadStat {
        WorkloadStat {
            namespace: "ns".into(),
            workload: wl.into(),
            kind: "deployment".into(),
            pods: 1,
            ready: 1,
            ..WorkloadStat::default()
        }
    }

    #[test]
    fn outliers_pick_each_kind_once() {
        let mut mem = stat("gofinancial");
        mem.mem_bytes = 1.9 * 1073741824.0;
        mem.mem_limit = 2.0 * 1073741824.0;
        mem.mem_pct = 95.0;
        let mut err = stat("onlineplayer");
        err.rps = 42.0;
        err.err_pct = 4.1;
        err.err_pct_baseline = 0.3;
        let mut lat = stat("slow");
        lat.latency_kind = "p95".into();
        lat.latency_ms = 1800.0;
        lat.latency_baseline_ms = 220.0;
        let mut fine = stat("fine");
        fine.mem_bytes = 100.0;
        fine.mem_limit = 1000.0;
        fine.mem_pct = 10.0;
        fine.rps = 10.0;
        fine.err_pct = 0.5;
        fine.err_pct_baseline = 0.4;
        let (m, e, l) = outliers(&[mem, err, lat, fine]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["workload"], "gofinancial");
        assert_eq!(m[0]["limit"], "2.0GiB");
        assert_eq!(e.len(), 1);
        assert_eq!(e[0]["workload"], "onlineplayer");
        assert_eq!(l.len(), 1);
        assert_eq!(l[0]["workload"], "slow");
    }

    #[test]
    fn memory_trend_alone_is_an_outlier() {
        let mut s = stat("leaky");
        s.mem_bytes = 500.0;
        s.mem_trend_pct = Some(40.0);
        let (m, _, _) = outliers(&[s]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0]["trend"], "+40%");
    }

    #[test]
    fn error_spike_needs_min_pct() {
        let mut s = stat("tiny");
        s.rps = 5.0;
        s.err_pct = 0.6;
        s.err_pct_baseline = 0.1;
        let (_, e, _) = outliers(&[s]);
        assert!(e.is_empty(), "0.6% is below ERR_MIN_PCT");
    }

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(512.0), "512B");
        assert_eq!(human_bytes(27.0 * 1048576.0), "27.0MiB");
        assert_eq!(human_bytes(1.5 * 1073741824.0), "1.5GiB");
    }

    #[test]
    fn seed_counts_pods_ready_limits_and_crashloop() {
        use crate::monitor::classify::{snap_key, ContainerSnap};
        let mut snap = Snapshot::new();
        for (name, ready, wait) in [("a-1", true, ""), ("a-2", false, "CrashLoopBackOff")] {
            let mut containers = BTreeMap::new();
            containers.insert(
                "c".to_string(),
                ContainerSnap {
                    waiting_reason: wait.into(),
                    ..ContainerSnap::default()
                },
            );
            snap.insert(
                snap_key("ns", name),
                PodSnap {
                    namespace: "ns".into(),
                    name: name.into(),
                    phase: "Running".into(),
                    ready,
                    workload_kind: "deployment".into(),
                    workload: "a".into(),
                    containers,
                    mem_limit: 100,
                    ..PodSnap::default()
                },
            );
        }
        let m = seed_from_snapshot(&snap, None);
        let s = &m["ns/a"];
        assert_eq!((s.pods, s.ready, s.crashloop), (2, 1, 1));
        assert_eq!(s.mem_limit, 200.0);
        assert!(seed_from_snapshot(&snap, Some("other")).is_empty());
    }
}
