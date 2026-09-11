//! Fleet dashboard: ONE view over every monitored cluster, read **only from
//! ClickHouse** (`k8s_samples` + `k8s_events`). Nothing here touches a
//! cluster or the collector's SQLite snapshot — the point is the general
//! picture over a window (restarts / OOMs, memory, req/s, latency), not the
//! live pod status, so the pages keep working for a cluster that is currently
//! unreachable, and every question is one aggregation away.
//!
//! Routes (`/k8s/monitor/fleet/*`, `kubernetes:View`):
//!
//! | route | answers |
//! |---|---|
//! | `filters` | which clusters / namespaces / workloads / pods have data in the window |
//! | `table` | one row per workload (or pod) — restarts by class, churn, memory, rps, 5xx %, latency; sortable |
//! | `series` | time buckets for one metric, one series per cluster / namespace / workload / pod (restarts: per class) |
//! | `events` | the cross-cluster restart / churn timeline, sortable, paged |
//! | `requests` | per (path, method) rps / 5xx % / avg latency — needs `request_labels` on the cluster |
//!
//! Every filter value passes [`queries::ident_ok`] (400 otherwise) and every
//! literal goes through [`sql_str`]; sort keys are allow-listed. Cluster names
//! come from the SQLite registry only to label rows — a cluster whose row is
//! gone still shows under its id, so history never silently disappears.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Duration;
use otto_core::Error;
use serde::Deserialize;
use serde_json::{json, Value};

use super::health::RestartCounts;
use super::queries::{
    self, ident_ok, is_counter, p95_from_buckets, LATENCY_BUCKETS, LATENCY_COUNTS, LATENCY_SUMS,
    MEMORY_GAUGES, REQUEST_COUNTERS,
};
use super::schema::sql_str;
use crate::clusters::Clusters;
use crate::http::ApiErr;
use crate::{K8sCtx, MonitorSink};

type ApiResult<T> = std::result::Result<T, ApiErr>;
/// `(cluster_id, namespace, workload, pod)` — pod is empty in workload grouping.
type RowKey = (String, String, String, String);

pub fn routes<S: K8sCtx>() -> Router<S> {
    Router::new()
        .route("/k8s/monitor/fleet/filters", get(filters::<S>))
        .route("/k8s/monitor/fleet/table", get(table::<S>))
        .route("/k8s/monitor/fleet/series", get(series::<S>))
        .route("/k8s/monitor/fleet/events", get(events::<S>))
        .route("/k8s/monitor/fleet/requests", get(requests::<S>))
}

// ---------------------------------------------------------------------------
// Filter model
// ---------------------------------------------------------------------------

/// The common `?cluster=a,b&ns=&workload=&pod=&window=` selection.
#[derive(Debug, Default, Clone)]
pub struct FleetFilter {
    /// Empty = every cluster that has rows.
    pub clusters: Vec<String>,
    pub namespace: Option<String>,
    pub workload: Option<String>,
    pub pod: Option<String>,
}

impl FleetFilter {
    /// Parse + validate the raw query fields (400 on a bad identifier).
    pub fn parse(
        cluster: Option<&str>,
        ns: Option<&str>,
        workload: Option<&str>,
        pod: Option<&str>,
    ) -> Result<Self, Error> {
        let clusters: Vec<String> = cluster
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for c in &clusters {
            if !ident_ok(c) {
                return Err(Error::Invalid("bad cluster".into()));
            }
        }
        let opt = |name: &str, v: Option<&str>| -> Result<Option<String>, Error> {
            match v.map(str::trim).filter(|s| !s.is_empty()) {
                None => Ok(None),
                Some(s) if ident_ok(s) => Ok(Some(s.to_string())),
                Some(_) => Err(Error::Invalid(format!("bad {name}"))),
            }
        };
        Ok(Self {
            clusters,
            namespace: opt("ns", ns)?,
            workload: opt("workload", workload)?,
            pod: opt("pod", pod)?,
        })
    }

    /// `AND cluster_id IN (…) AND namespace = … AND workload = … AND pod = …`
    /// (each part only when set). Leading `AND` so it appends to a `WHERE`.
    pub fn sql(&self) -> String {
        let mut s = String::new();
        if !self.clusters.is_empty() {
            let list = self
                .clusters
                .iter()
                .map(|c| sql_str(c))
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!(" AND cluster_id IN ({list})"));
        }
        if let Some(n) = &self.namespace {
            s.push_str(&format!(" AND namespace = {}", sql_str(n)));
        }
        if let Some(w) = &self.workload {
            s.push_str(&format!(" AND workload = {}", sql_str(w)));
        }
        if let Some(p) = &self.pod {
            s.push_str(&format!(" AND pod = {}", sql_str(p)));
        }
        s
    }
}

fn range(window: Duration) -> String {
    format!("ts >= now() - INTERVAL {} SECOND", window.num_seconds())
}

fn in_list(items: &[&str]) -> String {
    items
        .iter()
        .map(|s| sql_str(s))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Rank expression so the most authoritative memory gauge wins per pod.
fn mem_rank_expr() -> String {
    let mut parts = Vec::new();
    for (i, g) in MEMORY_GAUGES.iter().enumerate() {
        parts.push(format!("metric = {}, {i}", sql_str(g)));
    }
    format!("multiIf({}, {})", parts.join(", "), MEMORY_GAUGES.len())
}

const CODE_EXPR: &str =
    "if(labels['code'] != '', labels['code'], if(labels['status'] != '', labels['status'], labels['status_code']))";
const T_UTC: &str = "formatDateTime(t, '%Y-%m-%dT%H:%i:%SZ', 'UTC')";
const TS_UTC: &str = "formatDateTime(ts, '%Y-%m-%dT%H:%i:%SZ', 'UTC')";

/// `workload` | `pod` — the table's row identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Workload,
    Pod,
}

impl Group {
    fn parse(s: Option<&str>) -> Result<Self, Error> {
        match s.unwrap_or("workload") {
            "workload" | "" => Ok(Group::Workload),
            "pod" => Ok(Group::Pod),
            other => Err(Error::Invalid(format!(
                "bad group '{other}' (workload|pod)"
            ))),
        }
    }
    /// Columns that identify a row.
    fn cols(self) -> &'static str {
        match self {
            Group::Workload => "cluster_id, namespace, workload",
            Group::Pod => "cluster_id, namespace, workload, pod",
        }
    }
}

// ---------------------------------------------------------------------------
// SQL builders (pure, unit-tested)
// ---------------------------------------------------------------------------

/// Distinct (cluster, namespace, workload) with data in the window, from both
/// tables, plus the sample count per cluster so the picker can show which
/// clusters are actually reporting.
pub fn filters_sql(window: Duration) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, count() AS n FROM (
           SELECT cluster_id, namespace, workload FROM k8s_samples WHERE {r}
           UNION ALL
           SELECT cluster_id, namespace, workload FROM k8s_events WHERE {r}
         ) GROUP BY cluster_id, namespace, workload ORDER BY cluster_id, namespace, workload",
        r = range(window)
    )
}

/// Distinct pods for the current (cluster, ns, workload) selection — fetched
/// separately and capped, since a fleet can have thousands.
pub fn pods_sql(f: &FleetFilter, window: Duration, limit: u32) -> String {
    format!(
        "SELECT cluster_id, namespace, workload, pod, max(ts) AS last FROM k8s_samples
         WHERE {r}{f}
         GROUP BY cluster_id, namespace, workload, pod ORDER BY last DESC LIMIT {limit}",
        r = range(window),
        f = f.sql(),
        limit = limit.clamp(1, 5000),
    )
}

/// Memory per (row, pod, gauge): last / avg / max in the window. The Rust side
/// keeps the best gauge per pod (rank) and sums pods into the row.
pub fn memory_sql(f: &FleetFilter, window: Duration, g: Group) -> String {
    format!(
        "SELECT {cols}, pod, metric, {rank} AS rank, argMax(value, ts) AS mem_last, avg(value) AS mem_avg, max(value) AS mem_max
         FROM k8s_samples
         WHERE metric IN ({gauges}) AND {r}{f}
         GROUP BY {cols}, pod, metric",
        cols = g.cols(),
        rank = mem_rank_expr(),
        gauges = in_list(&MEMORY_GAUGES),
        r = range(window),
        f = f.sql(),
    )
}

/// Restart / churn counts per (row, kind, class).
pub fn restarts_sql(f: &FleetFilter, window: Duration, g: Group) -> String {
    format!(
        "SELECT {cols}, kind, class, count() AS n FROM k8s_events
         WHERE kind IN ('restart', 'churn') AND {r}{f}
         GROUP BY {cols}, kind, class",
        cols = g.cols(),
        r = range(window),
        f = f.sql(),
    )
}

/// Request rate + 5xx rate per row (counter deltas per pod × label-set).
pub fn rates_sql(f: &FleetFilter, window: Duration, g: Group) -> String {
    let secs = window.num_seconds().max(1);
    format!(
        "SELECT {cols}, sum(delta) / {secs} AS rps, sumIf(delta, is5xx) / {secs} AS err_rps FROM (
           SELECT {cols}, pod, labels, startsWith({code}, '5') AS is5xx, greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE metric IN ({counters}) AND {r}{f}
           GROUP BY {cols}, pod, labels
         ) GROUP BY {cols}",
        cols = g.cols(),
        code = CODE_EXPR,
        counters = in_list(&REQUEST_COUNTERS),
        r = range(window),
        f = f.sql(),
    )
}

/// Histogram bucket deltas per row → p95 in Rust.
pub fn latency_buckets_sql(f: &FleetFilter, window: Duration, g: Group) -> String {
    format!(
        "SELECT {cols}, le, sum(delta) AS delta FROM (
           SELECT {cols}, pod, labels['le'] AS le, labels, greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE metric IN ({buckets}) AND {r}{f}
           GROUP BY {cols}, pod, labels
         ) GROUP BY {cols}, le",
        cols = g.cols(),
        buckets = in_list(&LATENCY_BUCKETS),
        r = range(window),
        f = f.sql(),
    )
}

/// Mean latency per row (`_sum` / `_count` deltas) — the fallback when no histogram.
pub fn latency_avg_sql(f: &FleetFilter, window: Duration, g: Group) -> String {
    format!(
        "SELECT {cols}, if(sumIf(delta, is_count) > 0, 1000 * sumIf(delta, NOT is_count) / sumIf(delta, is_count), 0) AS avg_ms FROM (
           SELECT {cols}, pod, labels, metric IN ({counts}) AS is_count, greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE metric IN ({sums}, {counts}) AND {r}{f}
           GROUP BY {cols}, pod, labels, metric
         ) GROUP BY {cols}",
        cols = g.cols(),
        counts = in_list(&LATENCY_COUNTS),
        sums = in_list(&LATENCY_SUMS),
        r = range(window),
        f = f.sql(),
    )
}

/// The series metrics the fleet chart understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesMetric {
    Restarts,
    Mem,
    Rps,
    Err,
    Latency,
}

impl SeriesMetric {
    fn parse(s: &str) -> Result<Self, Error> {
        Ok(match s {
            "restarts" => Self::Restarts,
            "mem" => Self::Mem,
            "rps" => Self::Rps,
            "err" => Self::Err,
            "latency" => Self::Latency,
            other => {
                return Err(Error::Invalid(format!(
                    "bad metric '{other}' (restarts|mem|rps|err|latency)"
                )))
            }
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Restarts => "restarts",
            Self::Mem => "mem",
            Self::Rps => "rps",
            Self::Err => "err",
            Self::Latency => "latency",
        }
    }
    fn unit(self) -> &'static str {
        match self {
            Self::Restarts => "count",
            Self::Mem => "bytes",
            Self::Rps => "rate",
            Self::Err => "percent",
            Self::Latency => "ms",
        }
    }
}

/// What one series line stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesBy {
    Cluster,
    Namespace,
    Workload,
    Pod,
}

impl SeriesBy {
    fn parse(s: Option<&str>) -> Result<Self, Error> {
        Ok(match s.unwrap_or("cluster") {
            "cluster" | "" => Self::Cluster,
            "namespace" => Self::Namespace,
            "workload" => Self::Workload,
            "pod" => Self::Pod,
            other => {
                return Err(Error::Invalid(format!(
                    "bad by '{other}' (cluster|namespace|workload|pod)"
                )))
            }
        })
    }
    /// Column (or expression) that keys the series.
    fn expr(self) -> &'static str {
        match self {
            Self::Cluster => "cluster_id",
            Self::Namespace => "concat(cluster_id, '/', namespace)",
            Self::Workload => "concat(cluster_id, '/', namespace, '/', workload)",
            Self::Pod => "concat(cluster_id, '/', namespace, '/', pod)",
        }
    }
}

/// Bucketed series for one metric → rows `(g, t, v)` ordered by `g, t`.
/// Restarts are ALWAYS keyed by class (that is the stacked chart people want).
pub fn series_sql(
    f: &FleetFilter,
    window: Duration,
    metric: SeriesMetric,
    by: SeriesBy,
    step_secs: u32,
) -> String {
    let step = step_secs.max(10);
    let r = range(window);
    let fs = f.sql();
    let g = by.expr();
    match metric {
        SeriesMetric::Restarts => format!(
            "SELECT class AS g, {t_utc} AS t, count() AS v FROM (
               SELECT class, toStartOfInterval(ts, INTERVAL {step} SECOND) AS t FROM k8s_events
               WHERE kind = 'restart' AND {r}{fs}
             ) GROUP BY g, t ORDER BY g, t",
            t_utc = T_UTC
        ),
        SeriesMetric::Mem => format!(
            "SELECT g, {t_utc} AS t, sum(v) AS v FROM (
               SELECT g, t, pod, argMin(v, rank) AS v FROM (
                 SELECT {g} AS g, toStartOfInterval(ts, INTERVAL {step} SECOND) AS t, pod, metric, {rank} AS rank, avg(value) AS v
                 FROM k8s_samples
                 WHERE metric IN ({gauges}) AND {r}{fs}
                 GROUP BY g, t, pod, metric, rank
               ) GROUP BY g, t, pod
             ) GROUP BY g, t ORDER BY g, t",
            t_utc = T_UTC,
            rank = mem_rank_expr(),
            gauges = in_list(&MEMORY_GAUGES),
        ),
        SeriesMetric::Rps => format!(
            "SELECT g, {t_utc} AS t, sum(v) AS v FROM (
               SELECT {g} AS g, toStartOfInterval(ts, INTERVAL {step} SECOND) AS t, pod, labels, greatest(0, max(value) - min(value)) / {step} AS v
               FROM k8s_samples
               WHERE metric IN ({counters}) AND {r}{fs}
               GROUP BY g, t, pod, labels
             ) GROUP BY g, t ORDER BY g, t",
            t_utc = T_UTC,
            counters = in_list(&REQUEST_COUNTERS),
        ),
        SeriesMetric::Err => format!(
            "SELECT g, {t_utc} AS t, if(sum(d) > 0, 100 * sumIf(d, is5xx) / sum(d), 0) AS v FROM (
               SELECT {g} AS g, toStartOfInterval(ts, INTERVAL {step} SECOND) AS t, pod, labels, startsWith({code}, '5') AS is5xx, greatest(0, max(value) - min(value)) AS d
               FROM k8s_samples
               WHERE metric IN ({counters}) AND {r}{fs}
               GROUP BY g, t, pod, labels, is5xx
             ) GROUP BY g, t ORDER BY g, t",
            t_utc = T_UTC,
            code = CODE_EXPR,
            counters = in_list(&REQUEST_COUNTERS),
        ),
        SeriesMetric::Latency => format!(
            "SELECT g, {t_utc} AS t, if(sumIf(d, is_count) > 0, 1000 * sumIf(d, NOT is_count) / sumIf(d, is_count), 0) AS v FROM (
               SELECT {g} AS g, toStartOfInterval(ts, INTERVAL {step} SECOND) AS t, pod, labels, metric IN ({counts}) AS is_count, greatest(0, max(value) - min(value)) AS d
               FROM k8s_samples
               WHERE metric IN ({sums}, {counts}) AND {r}{fs}
               GROUP BY g, t, pod, labels, metric, is_count
             ) GROUP BY g, t ORDER BY g, t",
            t_utc = T_UTC,
            counts = in_list(&LATENCY_COUNTS),
            sums = in_list(&LATENCY_SUMS),
        ),
    }
}

/// Allow-listed event sort keys → column.
fn event_sort_col(key: &str) -> Option<&'static str> {
    Some(match key {
        "ts" | "" => "ts",
        "cluster" => "cluster_id",
        "namespace" => "namespace",
        "workload" => "workload",
        "pod" => "pod",
        "kind" => "kind",
        "class" => "class",
        "reason" => "reason",
        _ => return None,
    })
}

/// Cross-cluster events page + the total for the same filter.
pub fn events_sql(
    f: &FleetFilter,
    window: Duration,
    class: Option<&str>,
    sort: &str,
    desc: bool,
    limit: u32,
    offset: u32,
) -> Result<(String, String), Error> {
    let col = event_sort_col(sort).ok_or_else(|| Error::Invalid(format!("bad sort '{sort}'")))?;
    let kind_f = match class.filter(|c| !c.is_empty()) {
        Some("k8s_event") => " AND kind = 'k8s_event'".to_string(),
        Some("version") => " AND kind = 'version'".to_string(),
        Some("churn") => " AND kind = 'churn'".to_string(),
        Some(c) => format!(
            " AND kind IN ('restart', 'churn') AND class = {}",
            sql_str(c)
        ),
        None => " AND kind IN ('restart', 'churn', 'version')".to_string(),
    };
    let where_ = format!(
        "{r}{kind}{f}",
        r = range(window),
        kind = kind_f,
        f = f.sql()
    );
    let dir = if desc { "DESC" } else { "ASC" };
    let page = format!(
        "SELECT {ts_utc} AS ts, cluster_id, namespace, workload, pod, container, kind, class, reason, exit_code, detail, actor
         FROM k8s_events WHERE {where_}
         ORDER BY {col} {dir}, ts DESC LIMIT {limit} OFFSET {offset}",
        ts_utc = TS_UTC,
        limit = limit.clamp(1, 1000),
    );
    let total = format!("SELECT count() AS n FROM k8s_events WHERE {where_}");
    Ok((page, total))
}

/// Per (path, method) request rate / 5xx / avg latency. Only clusters with
/// `request_labels` enabled carry the `path` label; everything else groups
/// under an empty path and is dropped by the handler.
pub fn requests_sql(f: &FleetFilter, window: Duration) -> String {
    let secs = window.num_seconds().max(1);
    format!(
        "SELECT path, method,
                sumIf(delta, is_req) / {secs} AS rps,
                sumIf(delta, is_req AND is5xx) / {secs} AS err_rps,
                if(sumIf(delta, is_count) > 0, 1000 * sumIf(delta, is_sum) / sumIf(delta, is_count), 0) AS avg_ms
         FROM (
           SELECT labels['path'] AS path, labels['method'] AS method, pod, labels, metric,
                  metric IN ({counters}) AS is_req, metric IN ({sums}) AS is_sum, metric IN ({counts}) AS is_count,
                  startsWith({code}, '5') AS is5xx, greatest(0, max(value) - min(value)) AS delta
           FROM k8s_samples
           WHERE labels['path'] != '' AND metric IN ({counters}, {sums}, {counts}) AND {r}{fs}
           GROUP BY path, method, pod, labels, metric
         ) GROUP BY path, method ORDER BY rps DESC LIMIT 500",
        counters = in_list(&REQUEST_COUNTERS),
        sums = in_list(&LATENCY_SUMS),
        counts = in_list(&LATENCY_COUNTS),
        code = CODE_EXPR,
        r = range(window),
        fs = f.sql(),
    )
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct FleetQuery {
    pub window: Option<String>,
    pub cluster: Option<String>,
    pub ns: Option<String>,
    pub workload: Option<String>,
    pub pod: Option<String>,
    // table
    pub group: Option<String>,
    pub sort: Option<String>,
    pub dir: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    // series
    pub metric: Option<String>,
    pub by: Option<String>,
    pub step: Option<u32>,
    // events
    pub class: Option<String>,
}

fn f64_of(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(|x| {
            x.as_f64()
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0.0)
}
fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn sink_of<S: K8sCtx>(ctx: &S) -> ApiResult<std::sync::Arc<dyn MonitorSink>> {
    ctx.monitor_sink()
        .filter(|s| s.available())
        .ok_or_else(|| Error::Conflict("usage engine (ClickHouse) is not available".into()).into())
}

async fn cluster_names<S: K8sCtx>(ctx: &S) -> HashMap<String, Value> {
    let mut m = HashMap::new();
    if let Ok(list) = Clusters::new(ctx).list().await {
        for c in list {
            m.insert(
                c.id.to_string(),
                json!({"id": c.id, "name": c.name, "environment": c.environment, "color": c.color}),
            );
        }
    }
    m
}

fn cluster_json(names: &HashMap<String, Value>, id: &str) -> Value {
    names.get(id).cloned().unwrap_or_else(
        || json!({"id": id, "name": id, "environment": "dev", "color": Value::Null}),
    )
}

fn parse_common(
    q: &FleetQuery,
    default_window: &str,
) -> ApiResult<(String, Duration, FleetFilter)> {
    let label = q
        .window
        .clone()
        .unwrap_or_else(|| default_window.to_string());
    let window = queries::parse_window(&label)?;
    let f = FleetFilter::parse(
        q.cluster.as_deref(),
        q.ns.as_deref(),
        q.workload.as_deref(),
        q.pod.as_deref(),
    )?;
    Ok((label, window, f))
}

/// `GET /k8s/monitor/fleet/filters?window=&cluster=&ns=&workload=`.
async fn filters<S: K8sCtx>(
    State(ctx): State<S>,
    Query(q): Query<FleetQuery>,
) -> ApiResult<Json<Value>> {
    let (label, window, f) = parse_common(&q, "24h")?;
    let sink = sink_of(&ctx)?;
    let names = cluster_names(&ctx).await;
    let rows = sink.query_rows(&filters_sql(window)).await?;
    let mut clusters: BTreeMap<String, u64> = BTreeMap::new();
    let mut namespaces: BTreeSet<(String, String)> = BTreeSet::new();
    let mut workloads: BTreeSet<(String, String, String)> = BTreeSet::new();
    for r in &rows {
        let c = str_of(r, "cluster_id").to_string();
        let n = str_of(r, "namespace").to_string();
        let w = str_of(r, "workload").to_string();
        *clusters.entry(c.clone()).or_default() += f64_of(r, "n") as u64;
        namespaces.insert((c.clone(), n.clone()));
        workloads.insert((c, n, w));
    }
    // Registered clusters with no rows still appear (rows: 0) so the picker
    // explains "nothing collected" instead of hiding the cluster.
    for id in names.keys() {
        clusters.entry(id.clone()).or_default();
    }
    let pods_rows = if f.workload.is_some()
        || f.pod.is_some()
        || (f.clusters.len() == 1 && f.namespace.is_some())
    {
        sink.query_rows(&pods_sql(&f, window, 2000))
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };
    let pods: Vec<Value> = pods_rows
        .iter()
        .map(|r| json!({"cluster_id": str_of(r, "cluster_id"), "namespace": str_of(r, "namespace"), "workload": str_of(r, "workload"), "pod": str_of(r, "pod")}))
        .collect();
    Ok(Json(json!({
        "window": label,
        "clusters": clusters.iter().map(|(id, n)| { let mut c = cluster_json(&names, id); c["rows"] = json!(n); c }).collect::<Vec<_>>(),
        "namespaces": namespaces.iter().map(|(c, n)| json!({"cluster_id": c, "namespace": n})).collect::<Vec<_>>(),
        "workloads": workloads.iter().map(|(c, n, w)| json!({"cluster_id": c, "namespace": n, "workload": w})).collect::<Vec<_>>(),
        "pods": pods,
    })))
}

#[derive(Default)]
struct Row {
    cluster_id: String,
    namespace: String,
    workload: String,
    pod: String,
    pods: BTreeSet<String>,
    restarts: RestartCounts,
    churn: u32,
    mem_last: f64,
    mem_avg: f64,
    mem_max: f64,
    rps: f64,
    err_rps: f64,
    latency_kind: &'static str,
    latency_ms: f64,
}

fn row_key(v: &Value, g: Group) -> RowKey {
    (
        str_of(v, "cluster_id").to_string(),
        str_of(v, "namespace").to_string(),
        str_of(v, "workload").to_string(),
        if g == Group::Pod {
            str_of(v, "pod").to_string()
        } else {
            String::new()
        },
    )
}

/// Allow-listed table sort keys.
const TABLE_SORT_KEYS: [&str; 16] = [
    "cluster",
    "namespace",
    "workload",
    "pod",
    "pods",
    "restarts",
    "oom",
    "crash",
    "probe",
    "churn",
    "mem_last",
    "mem_avg",
    "mem_max",
    "rps",
    "err_pct",
    "latency_ms",
];

/// `GET /k8s/monitor/fleet/table?window=&cluster=&ns=&workload=&pod=&group=&sort=&dir=&limit=&offset=`.
async fn table<S: K8sCtx>(
    State(ctx): State<S>,
    Query(q): Query<FleetQuery>,
) -> ApiResult<Json<Value>> {
    let (label, window, f) = parse_common(&q, "24h")?;
    let g = Group::parse(q.group.as_deref())?;
    let sort = q.sort.clone().unwrap_or_else(|| "restarts".into());
    if !TABLE_SORT_KEYS.contains(&sort.as_str()) {
        return Err(Error::Invalid(format!("bad sort '{sort}'")).into());
    }
    let desc = q.dir.as_deref().unwrap_or("desc") != "asc";
    let sink = sink_of(&ctx)?;
    let names = cluster_names(&ctx).await;

    let (q_mem, q_rst, q_rates, q_buckets, q_avgs) = (
        memory_sql(&f, window, g),
        restarts_sql(&f, window, g),
        rates_sql(&f, window, g),
        latency_buckets_sql(&f, window, g),
        latency_avg_sql(&f, window, g),
    );
    let (mem, rst, rates, buckets, avgs) = tokio::join!(
        sink.query_rows(&q_mem),
        sink.query_rows(&q_rst),
        sink.query_rows(&q_rates),
        sink.query_rows(&q_buckets),
        sink.query_rows(&q_avgs),
    );
    let (mem, rst, rates, buckets, avgs) = (mem?, rst?, rates?, buckets?, avgs?);

    let mut rows: BTreeMap<RowKey, Row> = BTreeMap::new();
    let ensure = |rows: &mut BTreeMap<RowKey, Row>, k: RowKey| {
        rows.entry(k.clone()).or_insert_with(|| Row {
            cluster_id: k.0.clone(),
            namespace: k.1.clone(),
            workload: k.2.clone(),
            pod: k.3.clone(),
            ..Default::default()
        });
    };

    // Memory: best gauge per pod (lowest rank), then summed into the row.
    let mut best: HashMap<(RowKey, String), (i64, f64, f64, f64)> = HashMap::new();
    for r in &mem {
        let k = row_key(r, g);
        let pod = str_of(r, "pod").to_string();
        let rank = f64_of(r, "rank") as i64;
        let e = best.entry((k, pod)).or_insert((i64::MAX, 0.0, 0.0, 0.0));
        if rank < e.0 {
            *e = (
                rank,
                f64_of(r, "mem_last"),
                f64_of(r, "mem_avg"),
                f64_of(r, "mem_max"),
            );
        }
    }
    for ((k, pod), (_, last, avg, max)) in best {
        ensure(&mut rows, k.clone());
        let row = rows.get_mut(&k).expect("ensured");
        row.pods.insert(pod);
        row.mem_last += last;
        row.mem_avg += avg;
        row.mem_max = row.mem_max.max(max);
    }
    for r in &rst {
        let k = row_key(r, g);
        ensure(&mut rows, k.clone());
        let row = rows.get_mut(&k).expect("ensured");
        row.pods.insert(str_of(r, "pod").to_string());
        let n = f64_of(r, "n") as u32;
        match (str_of(r, "kind"), str_of(r, "class")) {
            ("restart", "oom") => row.restarts.oom += n,
            ("restart", "crash") => row.restarts.crash += n,
            ("restart", "probe") => row.restarts.probe += n,
            ("restart", _) => row.restarts.unknown += n,
            ("churn", _) => row.churn += n,
            _ => {}
        }
    }
    for r in &rates {
        let k = row_key(r, g);
        ensure(&mut rows, k.clone());
        let row = rows.get_mut(&k).expect("ensured");
        row.rps += f64_of(r, "rps");
        row.err_rps += f64_of(r, "err_rps");
    }
    let mut by_row_buckets: HashMap<RowKey, Vec<(String, f64)>> = HashMap::new();
    for r in &buckets {
        by_row_buckets
            .entry(row_key(r, g))
            .or_default()
            .push((str_of(r, "le").to_string(), f64_of(r, "delta")));
    }
    for (k, b) in by_row_buckets {
        if let Some(p95) = p95_from_buckets(&b) {
            ensure(&mut rows, k.clone());
            let row = rows.get_mut(&k).expect("ensured");
            row.latency_kind = "p95";
            row.latency_ms = p95;
        }
    }
    for r in &avgs {
        let k = row_key(r, g);
        ensure(&mut rows, k.clone());
        let row = rows.get_mut(&k).expect("ensured");
        if row.latency_kind.is_empty() && f64_of(r, "avg_ms") > 0.0 {
            row.latency_kind = "avg";
            row.latency_ms = f64_of(r, "avg_ms");
        }
    }
    // Row 'pods' for the workload grouping counts distinct pods seen in any
    // source; in pod grouping it is the pod itself.
    for row in rows.values_mut() {
        if g == Group::Pod {
            row.pods.clear();
            row.pods.insert(row.pod.clone());
        }
        row.pods.remove("");
    }

    let mut list: Vec<Row> = rows.into_values().collect();
    let total = list.len();
    let num = |r: &Row| -> f64 {
        match sort.as_str() {
            "pods" => r.pods.len() as f64,
            "restarts" => f64::from(r.restarts.total()),
            "oom" => f64::from(r.restarts.oom),
            "crash" => f64::from(r.restarts.crash),
            "probe" => f64::from(r.restarts.probe),
            "churn" => f64::from(r.churn),
            "mem_last" => r.mem_last,
            "mem_avg" => r.mem_avg,
            "mem_max" => r.mem_max,
            "rps" => r.rps,
            "err_pct" => {
                if r.rps > 0.0 {
                    100.0 * r.err_rps / r.rps
                } else {
                    0.0
                }
            }
            "latency_ms" => r.latency_ms,
            _ => 0.0,
        }
    };
    let text = |r: &Row| -> String {
        match sort.as_str() {
            "cluster" => names
                .get(&r.cluster_id)
                .and_then(|c| c.get("name"))
                .and_then(Value::as_str)
                .unwrap_or(&r.cluster_id)
                .to_lowercase(),
            "namespace" => r.namespace.clone(),
            "workload" => r.workload.clone(),
            "pod" => r.pod.clone(),
            _ => String::new(),
        }
    };
    let textual = matches!(sort.as_str(), "cluster" | "namespace" | "workload" | "pod");
    list.sort_by(|a, b| {
        let o = if textual {
            text(a).cmp(&text(b))
        } else {
            num(a)
                .partial_cmp(&num(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        // Deterministic tiebreak so paging never duplicates a row.
        let o = o.then_with(|| {
            (&a.cluster_id, &a.namespace, &a.workload, &a.pod).cmp(&(
                &b.cluster_id,
                &b.namespace,
                &b.workload,
                &b.pod,
            ))
        });
        if desc {
            o.reverse()
        } else {
            o
        }
    });
    let offset = q.offset.unwrap_or(0) as usize;
    let limit = q.limit.unwrap_or(200).clamp(1, 2000) as usize;
    let page: Vec<Value> = list
        .iter()
        .skip(offset)
        .take(limit)
        .map(|r| {
            json!({
                "cluster": cluster_json(&names, &r.cluster_id),
                "cluster_id": r.cluster_id, "namespace": r.namespace, "workload": r.workload, "pod": r.pod,
                "pods": r.pods.len(),
                "restarts": r.restarts, "churn": r.churn,
                "mem_last": r.mem_last, "mem_avg": r.mem_avg, "mem_max": r.mem_max,
                "rps": r.rps, "err_pct": if r.rps > 0.0 { 100.0 * r.err_rps / r.rps } else { 0.0 },
                "latency_kind": r.latency_kind, "latency_ms": r.latency_ms,
            })
        })
        .collect();
    Ok(Json(json!({
        "window": label, "group": if g == Group::Pod { "pod" } else { "workload" },
        "sort": sort, "dir": if desc { "desc" } else { "asc" },
        "total": total, "offset": offset, "rows": page,
    })))
}

/// `GET /k8s/monitor/fleet/series?metric=&by=&window=&step=&cluster=&ns=&workload=&pod=`.
async fn series<S: K8sCtx>(
    State(ctx): State<S>,
    Query(q): Query<FleetQuery>,
) -> ApiResult<Json<Value>> {
    let (label, window, f) = parse_common(&q, "24h")?;
    let metric = SeriesMetric::parse(q.metric.as_deref().unwrap_or("restarts"))?;
    let by = SeriesBy::parse(q.by.as_deref())?;
    // ~60 buckets by default; never finer than a minute (collector cadence).
    let step = q
        .step
        .unwrap_or_else(|| (window.num_seconds() / 60).clamp(60, 3600) as u32)
        .clamp(60, 86_400);
    let sink = sink_of(&ctx)?;
    let names = cluster_names(&ctx).await;
    let rows = sink
        .query_rows(&series_sql(&f, window, metric, by, step))
        .await?;
    let mut series: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for r in &rows {
        let g = str_of(r, "g").to_string();
        series
            .entry(g)
            .or_default()
            .push(json!({"t": r.get("t").cloned().unwrap_or(Value::Null), "v": f64_of(r, "v")}));
    }
    let out: Vec<Value> = series
        .into_iter()
        .map(|(key, points)| {
            // Label: cluster ids → names when the key starts with one.
            let label = match metric {
                SeriesMetric::Restarts => key.clone(),
                _ => {
                    let (cid, rest) = key
                        .split_once('/')
                        .map(|(a, b)| (a, Some(b)))
                        .unwrap_or((key.as_str(), None));
                    let name = names
                        .get(cid)
                        .and_then(|c| c.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or(cid);
                    match rest {
                        Some(r) => format!("{name}/{r}"),
                        None => name.to_string(),
                    }
                }
            };
            json!({"key": key, "label": label, "points": points})
        })
        .collect();
    Ok(Json(json!({
        "window": label, "metric": metric.as_str(), "unit": metric.unit(),
        "by": if metric == SeriesMetric::Restarts { "class" } else { q.by.as_deref().unwrap_or("cluster") },
        "step_secs": step, "series": out,
    })))
}

/// `GET /k8s/monitor/fleet/events?window=&class=&sort=&dir=&limit=&offset=&cluster=&ns=&workload=&pod=`.
async fn events<S: K8sCtx>(
    State(ctx): State<S>,
    Query(q): Query<FleetQuery>,
) -> ApiResult<Json<Value>> {
    let (label, window, f) = parse_common(&q, "24h")?;
    if let Some(c) = q.class.as_deref().filter(|c| !c.is_empty()) {
        if !ident_ok(c) {
            return Err(Error::Invalid("bad class".into()).into());
        }
    }
    let sort = q.sort.clone().unwrap_or_default();
    let desc = q.dir.as_deref().unwrap_or("desc") != "asc";
    let limit = q.limit.unwrap_or(200);
    let offset = q.offset.unwrap_or(0);
    let (page_sql, total_sql) =
        events_sql(&f, window, q.class.as_deref(), &sort, desc, limit, offset)?;
    let sink = sink_of(&ctx)?;
    let names = cluster_names(&ctx).await;
    let (rows, total) = tokio::join!(sink.query_rows(&page_sql), sink.query_rows(&total_sql));
    let rows: Vec<Value> = rows?
        .into_iter()
        .map(|mut r| {
            if let Some(d) = r.get("detail").and_then(Value::as_str) {
                r["detail"] = serde_json::from_str(d).unwrap_or(Value::Null);
            }
            let cid = str_of(&r, "cluster_id").to_string();
            r["cluster"] = cluster_json(&names, &cid);
            r
        })
        .collect();
    let total = total?.first().map(|r| f64_of(r, "n") as u64).unwrap_or(0);
    Ok(Json(json!({
        "window": label, "sort": if sort.is_empty() { "ts" } else { sort.as_str() }, "dir": if desc { "desc" } else { "asc" },
        "total": total, "offset": offset, "rows": rows,
    })))
}

/// `GET /k8s/monitor/fleet/requests?window=&cluster=&ns=&workload=&pod=`.
async fn requests<S: K8sCtx>(
    State(ctx): State<S>,
    Query(q): Query<FleetQuery>,
) -> ApiResult<Json<Value>> {
    let (label, window, f) = parse_common(&q, "24h")?;
    let sink = sink_of(&ctx)?;
    // Which registered clusters keep request labels — so the page can say
    // "enable it on X" instead of showing an inexplicable empty table.
    let repo = otto_state::K8sMonitorRepo::new(ctx.pool());
    let mut enabled_on: Vec<Value> = Vec::new();
    let mut disabled_on: Vec<Value> = Vec::new();
    if let Ok(list) = Clusters::new(&ctx).list().await {
        for c in list {
            let on = repo
                .get_config(c.id.as_str())
                .await
                .ok()
                .flatten()
                .map(|r| r.request_labels)
                .unwrap_or(false);
            let v = json!({"id": c.id, "name": c.name});
            if on {
                enabled_on.push(v);
            } else {
                disabled_on.push(v);
            }
        }
    }
    let rows: Vec<Value> = sink
        .query_rows(&requests_sql(&f, window))
        .await?
        .into_iter()
        .filter(|r| !str_of(r, "path").is_empty())
        .map(|r| {
            let rps = f64_of(&r, "rps");
            let err = f64_of(&r, "err_rps");
            json!({
                "path": str_of(&r, "path"), "method": str_of(&r, "method"),
                "rps": rps, "err_pct": if rps > 0.0 { 100.0 * err / rps } else { 0.0 },
                "avg_ms": f64_of(&r, "avg_ms"),
            })
        })
        .collect();
    Ok(Json(json!({
        "window": label, "enabled_on": enabled_on, "disabled_on": disabled_on, "rows": rows,
    })))
}

/// `true` when the metric's per-path labels are worth keeping (requests +
/// latency sums/counts — never histogram buckets, they are the volume).
pub fn keeps_request_labels(metric: &str) -> bool {
    is_counter(metric)
        && !metric.ends_with("_bucket")
        && (REQUEST_COUNTERS.contains(&metric)
            || LATENCY_SUMS.contains(&metric)
            || LATENCY_COUNTS.contains(&metric))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f() -> FleetFilter {
        FleetFilter::parse(Some("c1,c2"), Some("shop"), Some("web"), None).unwrap()
    }

    #[test]
    fn filter_parses_and_rejects() {
        let x = f();
        assert_eq!(x.clusters, vec!["c1".to_string(), "c2".to_string()]);
        assert_eq!(
            x.sql(),
            " AND cluster_id IN ('c1', 'c2') AND namespace = 'shop' AND workload = 'web'"
        );
        assert!(FleetFilter::parse(Some("c1;drop"), None, None, None).is_err());
        assert!(FleetFilter::parse(None, Some("a b"), None, None).is_err());
        assert!(FleetFilter::parse(None, None, None, Some("p'1")).is_err());
        let empty = FleetFilter::parse(Some(" , "), None, None, None).unwrap();
        assert_eq!(empty.sql(), "");
    }

    #[test]
    fn table_queries_group_and_filter() {
        let w = Duration::hours(1);
        let m = memory_sql(&f(), w, Group::Workload);
        assert!(m.contains("GROUP BY cluster_id, namespace, workload, pod, metric"));
        assert!(m.contains("multiIf(metric = 'mem_working_set_bytes', 0"));
        assert!(m.contains("cluster_id IN ('c1', 'c2')"));
        let p = memory_sql(&f(), w, Group::Pod);
        assert!(p.contains("GROUP BY cluster_id, namespace, workload, pod, pod, metric"));
        let r = restarts_sql(&f(), w, Group::Workload);
        assert!(r.contains("kind IN ('restart', 'churn')"));
        let rates = rates_sql(&f(), w, Group::Workload);
        assert!(rates.contains("/ 3600 AS rps"));
        assert!(rates.contains("startsWith("));
        assert!(latency_buckets_sql(&f(), w, Group::Workload).contains("labels['le'] AS le"));
        assert!(latency_avg_sql(&f(), w, Group::Workload).contains("1000 * sumIf"));
    }

    #[test]
    fn series_variants() {
        let w = Duration::hours(24);
        let s = series_sql(&f(), w, SeriesMetric::Restarts, SeriesBy::Cluster, 600);
        assert!(s.contains("class AS g"));
        assert!(s.contains("kind = 'restart'"));
        let m = series_sql(&f(), w, SeriesMetric::Mem, SeriesBy::Workload, 600);
        assert!(m.contains("concat(cluster_id, '/', namespace, '/', workload) AS g"));
        assert!(m.contains("argMin(v, rank)"));
        let r = series_sql(&f(), w, SeriesMetric::Rps, SeriesBy::Namespace, 5);
        assert!(r.contains("/ 10 AS v"), "step floors at 10s");
        assert!(series_sql(&f(), w, SeriesMetric::Err, SeriesBy::Pod, 60)
            .contains("100 * sumIf(d, is5xx)"));
        assert!(
            series_sql(&f(), w, SeriesMetric::Latency, SeriesBy::Cluster, 60).contains("is_count")
        );
        assert!(SeriesMetric::parse("cpu").is_err());
        assert!(SeriesBy::parse(Some("node")).is_err());
    }

    #[test]
    fn events_sort_allowlist_and_paging() {
        let w = Duration::hours(1);
        let (page, total) = events_sql(&f(), w, Some("oom"), "class", false, 50, 100).unwrap();
        assert!(page.contains("ORDER BY class ASC, ts DESC LIMIT 50 OFFSET 100"));
        assert!(page.contains("class = 'oom'"));
        assert!(total.starts_with("SELECT count() AS n"));
        assert!(total.contains("class = 'oom'"));
        assert!(events_sql(&f(), w, None, "detail; DROP", true, 1, 0).is_err());
        let (churn, _) = events_sql(&f(), w, Some("churn"), "", true, 5000, 0).unwrap();
        assert!(churn.contains("kind = 'churn'"));
        assert!(churn.contains("LIMIT 1000"));
    }

    #[test]
    fn requests_query_needs_path_label() {
        let q = requests_sql(&f(), Duration::hours(1));
        assert!(q.contains("labels['path'] != ''"));
        assert!(q.contains("labels['method'] AS method"));
        assert!(q.contains("LIMIT 500"));
        assert!(keeps_request_labels("http_requests_total"));
        assert!(keeps_request_labels("http_server_requests_seconds_count"));
        assert!(!keeps_request_labels(
            "http_request_duration_seconds_bucket"
        ));
        assert!(!keeps_request_labels("mem_sys_bytes"));
    }

    #[test]
    fn filters_and_pods() {
        let q = filters_sql(Duration::hours(6));
        assert!(q.contains("UNION ALL"));
        assert!(q.contains("INTERVAL 21600 SECOND"));
        let p = pods_sql(&f(), Duration::hours(1), 99_999);
        assert!(p.contains("LIMIT 5000"));
        assert!(p.contains("workload = 'web'"));
    }
}
