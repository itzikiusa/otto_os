//! Monitoring REST routes (contract `docs/contracts/api.md` "Kubernetes
//! monitoring"). Authorization is the server's policy table: everything
//! under `/k8s/clusters/{id}/monitor*` is View on GET / Edit otherwise, and
//! `/k8s/monitor/overview` is View. Handlers validate input, talk to the
//! SQLite repo + the `MonitorSink`, and audit config writes.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use otto_core::auth::AuthUser;
use otto_core::{Error, Id};
use otto_state::{K8sCluster, K8sMonitorRepo, K8sMonitorStatusRow};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::classify::{self, Snapshot};
use super::collector;
use super::health::{self, WorkloadStat};
use super::parse;
use super::probes::{self, is_excluded, MonitorConfig, PodRef, ProbeFormat};
use super::queries;
use super::scrape::{self, ScrapeTarget};
use crate::clusters::{kubectl_for, Clusters};
use crate::http::{audit, ApiErr};
use crate::resources;
use crate::K8sCtx;

type ApiResult<T> = std::result::Result<T, ApiErr>;

pub fn routes<S: K8sCtx>() -> Router<S> {
    Router::new()
        .route("/k8s/monitor/overview", get(overview::<S>))
        .route(
            "/k8s/clusters/{id}/monitor",
            get(get_monitor::<S>).put(put_monitor::<S>),
        )
        .route("/k8s/clusters/{id}/monitor/test", post(test_probes::<S>))
        .route("/k8s/clusters/{id}/monitor/run", post(run_now::<S>))
        .route("/k8s/clusters/{id}/monitor/workloads", get(workloads::<S>))
        .route("/k8s/clusters/{id}/monitor/series", get(series::<S>))
        .route("/k8s/clusters/{id}/monitor/events", get(events::<S>))
        .route("/k8s/clusters/{id}/monitor/health", get(health_digest::<S>))
}

#[derive(Serialize)]
struct PresetOut {
    id: String,
    title: String,
    probes: Vec<probes::Probe>,
}

fn presets_out() -> Vec<PresetOut> {
    probes::presets()
        .into_iter()
        .map(|(id, title, probes)| PresetOut { id, title, probes })
        .collect()
}

async fn load<S: K8sCtx>(ctx: &S, id: &Id) -> ApiResult<(K8sCluster, MonitorConfig, Option<K8sMonitorStatusRow>)> {
    let cluster = Clusters::new(ctx).get(id).await?;
    let repo = K8sMonitorRepo::new(ctx.pool());
    let cfg = repo
        .get_config(id.as_str())
        .await?
        .map(|r| probes::from_row(&r))
        .unwrap_or_default();
    let status = repo.get_status(id.as_str()).await?;
    Ok((cluster, cfg, status))
}

fn monitor_resp(cfg: &MonitorConfig, status: Option<&K8sMonitorStatusRow>) -> Value {
    json!({ "config": cfg, "status": status, "presets": presets_out() })
}

async fn get_monitor<S: K8sCtx>(State(ctx): State<S>, Path(id): Path<Id>) -> ApiResult<Json<Value>> {
    let (_, cfg, status) = load(&ctx, &id).await?;
    Ok(Json(monitor_resp(&cfg, status.as_ref())))
}

async fn put_monitor<S: K8sCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(cfg): Json<MonitorConfig>,
) -> ApiResult<Json<Value>> {
    let cluster = Clusters::new(&ctx).get(&id).await?;
    cfg.validate(cluster.default_namespace.as_deref())?;
    if cfg.enabled {
        let available = ctx.monitor_sink().map(|s| s.available()).unwrap_or(false);
        if !available {
            return Err(Error::Conflict(
                "monitoring needs the usage engine (embedded ClickHouse); enable it under Settings → Usage first".into(),
            )
            .into());
        }
    }
    let repo = K8sMonitorRepo::new(ctx.pool());
    let saved = repo.upsert_config(&probes::to_row(id.as_str(), &cfg)).await?;
    audit(
        &ctx,
        &user,
        "k8s.monitor.update",
        &id,
        json!({
            "cluster": cluster.name, "enabled": cfg.enabled, "interval_secs": cfg.interval_secs,
            "probes": cfg.probes.len(), "exclusions": cfg.exclusions.len(), "transport": cfg.transport.as_str(),
        }),
    )
    .await;
    let status = repo.get_status(id.as_str()).await?;
    Ok(Json(monitor_resp(&probes::from_row(&saved), status.as_ref())))
}

#[derive(Debug, Default, Deserialize)]
pub struct TestReq {
    pub ns: Option<String>,
    pub pod: Option<String>,
}

/// `POST /k8s/clusters/{id}/monitor/test` — one pod, every probe, parsed.
async fn test_probes<S: K8sCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    body: Option<Json<TestReq>>,
) -> ApiResult<Json<Value>> {
    let req = body.map(|Json(b)| b).unwrap_or_default();
    let (cluster, cfg, _) = load(&ctx, &id).await?;
    if cfg.probes.is_empty() {
        return Err(Error::Invalid("no probes configured".into()).into());
    }
    let namespaces = cfg.effective_namespaces(cluster.default_namespace.as_deref());
    let ns = req
        .ns
        .filter(|n| !n.trim().is_empty())
        .or_else(|| namespaces.first().cloned())
        .ok_or_else(|| Error::Invalid("namespace is required".into()))?;
    let k = kubectl_for(&ctx, &cluster).await?;
    let list = k.json(["get", "pods", "-n", ns.as_str(), "-o", "json"]).await?;
    let snap = classify::snapshot_from_pod_list(&list);
    let pod = match req.pod.filter(|p| !p.trim().is_empty()) {
        Some(name) => snap
            .get(&classify::snap_key(&ns, &name))
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("pod {ns}/{name}")))?,
        None => snap
            .values()
            .filter(|p| p.phase == "Running" && !p.deleting)
            .find(|p| {
                !is_excluded(
                    &cfg.exclusions,
                    &PodRef {
                        namespace: &p.namespace,
                        name: &p.name,
                        workload_kind: &p.workload_kind,
                        workload: &p.workload,
                        labels: &p.labels,
                    },
                )
            })
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("no running, non-excluded pod in {ns}")))?,
    };

    let metrics_server = if !cfg.metrics_server {
        "disabled".to_string()
    } else {
        match resources::pod_metrics(&k, Some(&ns)).await {
            Ok(_) => "ok".to_string(),
            Err(Error::Forbidden(m)) => format!("forbidden: {m}"),
            Err(_) => "absent".to_string(),
        }
    };

    let probe0 = &cfg.probes[0];
    let sample = ScrapeTarget {
        namespace: pod.namespace.clone(),
        pod: pod.name.clone(),
        port: probe0.port.or(pod.first_port).unwrap_or(80),
    };
    let transport = scrape::pick_transport(&k, cfg.transport, &sample, &probe0.path).await;

    let (by_port, unresolved) = scrape::group_by_port(&cfg.probes, pod.first_port);
    let mut out: Vec<Value> = Vec::new();
    for name in unresolved {
        out.push(json!({"name": name, "ok": false, "error": "no port: the probe has none and the container declares none"}));
    }
    for (port, probes) in by_port {
        let target = ScrapeTarget {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            port,
        };
        let results = scrape::fetch(&k, transport, &target, &probes).await;
        for (probe, res) in probes.iter().zip(results) {
            match res {
                Ok(r) => {
                    let parsed = match probe.format {
                        ProbeFormat::Prometheus => parse::parse_prometheus(&r.body, &probe.include, &probe.exclude, cfg.series_cap as usize),
                        ProbeFormat::Json => parse::parse_json(&r.body, &probe.mappings),
                        ProbeFormat::Health => parse::parse_health(r.status),
                    };
                    let samples: Vec<Value> = parsed
                        .samples
                        .iter()
                        .take(50)
                        .map(|s| json!({"metric": s.metric, "labels": s.labels, "value": s.value}))
                        .collect();
                    out.push(json!({
                        "name": probe.name, "ok": (200..300).contains(&r.status), "status": r.status, "ms": r.ms, "port": port,
                        "samples": samples, "sample_count": parsed.samples.len(), "labels": parsed.labels,
                        "parse_errors": parsed.parse_errors, "capped": parsed.capped,
                        "body_preview": r.body.chars().take(400).collect::<String>(),
                    }));
                }
                Err(e) => out.push(json!({"name": probe.name, "ok": false, "port": port, "error": e.to_string()})),
            }
        }
    }
    Ok(Json(json!({
        "namespace": pod.namespace, "pod": pod.name, "workload": pod.workload,
        "transport": transport.as_str(), "metrics_server": metrics_server, "probes": out,
    })))
}

/// `POST /k8s/clusters/{id}/monitor/run` — one cycle inline.
async fn run_now<S: K8sCtx>(State(ctx): State<S>, Path(id): Path<Id>) -> ApiResult<Json<K8sMonitorStatusRow>> {
    let (cluster, cfg, status) = load(&ctx, &id).await?;
    cfg.validate(cluster.default_namespace.as_deref())?;
    let sink = ctx
        .monitor_sink()
        .filter(|s| s.available())
        .ok_or_else(|| Error::Conflict("usage engine (ClickHouse) is not available".into()))?;
    sink.exec(&super::schema::schema_sql(cfg.retention_days)).await?;
    let prev: Snapshot = status
        .as_ref()
        .map(|s| serde_json::from_value(s.snapshot.clone()).unwrap_or_default())
        .unwrap_or_default();
    let prev_at = status
        .as_ref()
        .and_then(|s| s.last_cycle_at.as_deref())
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc));
    let out = collector::run_cycle(&ctx, &cluster, &cfg, &prev, prev_at, sink.as_ref()).await;
    K8sMonitorRepo::new(ctx.pool()).upsert_status(&out.status).await?;
    Ok(Json(out.status))
}

// ---------------------------------------------------------------------------
// Dashboard reads
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
pub struct WindowQuery {
    pub window: Option<String>,
    pub ns: Option<String>,
}

fn snapshot_of(status: Option<&K8sMonitorStatusRow>) -> Snapshot {
    status
        .map(|s| serde_json::from_value(s.snapshot.clone()).unwrap_or_default())
        .unwrap_or_default()
}

fn check_ident(name: &str, v: Option<&str>) -> ApiResult<()> {
    if let Some(v) = v.filter(|v| !v.is_empty()) {
        if !queries::ident_ok(v) {
            return Err(Error::Invalid(format!("bad {name}")).into());
        }
    }
    Ok(())
}

/// `healthy` | `degraded` | `incident` | `off` | `unknown`.
pub fn health_badge(enabled: bool, status: Option<&K8sMonitorStatusRow>, stats: &[WorkloadStat], pods: &Value) -> &'static str {
    if !enabled {
        return "off";
    }
    let Some(st) = status else { return "unknown" };
    if st.last_ok_at.is_none() {
        return if st.last_error.is_empty() { "unknown" } else { "degraded" };
    }
    let unplanned: u32 = stats.iter().map(|s| s.restarts.total()).sum();
    let oom_crash: u32 = stats.iter().map(|s| s.restarts.oom + s.restarts.crash).sum();
    let (mem, err, lat) = health::outliers(stats);
    let failed = pods.get("failed").and_then(Value::as_u64).unwrap_or(0);
    let crashloop = pods.get("crashloop").and_then(Value::as_u64).unwrap_or(0);
    if crashloop > 0 || failed > 0 || (oom_crash > 0 && !err.is_empty()) {
        return "incident";
    }
    let scrape_fail = st.pods_failed > 0 && st.pods_failed * 5 >= (st.pods_scraped + st.pods_failed).max(1);
    if unplanned > 0 || !mem.is_empty() || !err.is_empty() || !lat.is_empty() || scrape_fail {
        return "degraded";
    }
    "healthy"
}

fn pods_json(snap: &Snapshot) -> Value {
    let mut running = 0u64;
    let mut pending = 0u64;
    let mut failed = 0u64;
    let mut crashloop = 0u64;
    for p in snap.values() {
        match p.phase.as_str() {
            "Running" => running += 1,
            "Pending" => pending += 1,
            "Failed" => failed += 1,
            _ => {}
        }
        if p.containers.values().any(|c| c.waiting_reason == "CrashLoopBackOff") {
            crashloop += 1;
        }
    }
    json!({"running": running, "pending": pending, "failed": failed, "crashloop": crashloop, "total": snap.len()})
}

/// `GET /k8s/monitor/overview?window=` — one row per registered cluster.
async fn overview<S: K8sCtx>(State(ctx): State<S>, Query(q): Query<WindowQuery>) -> ApiResult<Json<Vec<Value>>> {
    let window_label = q.window.clone().unwrap_or_else(|| "24h".into());
    let window = queries::parse_window(&window_label)?;
    let repo = K8sMonitorRepo::new(ctx.pool());
    let sink = ctx.monitor_sink();
    let mut rows = Vec::new();
    for cluster in Clusters::new(&ctx).list().await? {
        let cfg = repo
            .get_config(cluster.id.as_str())
            .await?
            .map(|r| probes::from_row(&r))
            .unwrap_or_default();
        let status = repo.get_status(cluster.id.as_str()).await?;
        let snap = snapshot_of(status.as_ref());
        let pods = pods_json(&snap);
        let stats: Vec<WorkloadStat> = match (&sink, cfg.enabled, status.as_ref()) {
            (Some(s), true, Some(_)) if s.available() => health::workload_stats(s.as_ref(), cluster.id.as_str(), &snap, None, window)
                .await
                .unwrap_or_default(),
            _ => vec![],
        };
        let mut restarts = health::RestartCounts::default();
        let mut churn = 0u32;
        let mut mem_used = 0.0;
        let mut mem_limit = 0.0;
        let mut rps = 0.0;
        let mut err_rps = 0.0;
        let mut drift = Vec::new();
        for s in &stats {
            restarts.oom += s.restarts.oom;
            restarts.crash += s.restarts.crash;
            restarts.probe += s.restarts.probe;
            restarts.unknown += s.restarts.unknown;
            churn += s.churn_planned;
            mem_used += s.mem_bytes;
            mem_limit += s.mem_limit;
            rps += s.rps;
            err_rps += s.rps * s.err_pct / 100.0;
            if s.versions.len() > 1 {
                drift.push(json!({"workload": s.workload, "versions": s.versions}));
            }
        }
        let badge = health_badge(cfg.enabled, status.as_ref(), &stats, &pods);
        rows.push(json!({
            "cluster": {"id": cluster.id, "name": cluster.name, "environment": cluster.environment, "color": cluster.color},
            "enabled": cfg.enabled,
            "interval_secs": cfg.interval_secs,
            "status": status,
            "health": badge,
            "window": window_label,
            "pods": pods,
            "restarts": restarts,
            "churn": churn,
            "mem": {"used": mem_used, "limit": mem_limit, "pct": if mem_limit > 0.0 { 100.0 * mem_used / mem_limit } else { 0.0 }},
            "rps": rps,
            "err_pct": if rps > 0.0 { 100.0 * err_rps / rps } else { 0.0 },
            "drift": drift,
            "workloads": stats.len(),
        }));
    }
    Ok(Json(rows))
}

/// `GET /k8s/clusters/{id}/monitor/workloads?window=&ns=`.
async fn workloads<S: K8sCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<WindowQuery>,
) -> ApiResult<Json<Value>> {
    let window_label = q.window.clone().unwrap_or_else(|| "1h".into());
    let window = queries::parse_window(&window_label)?;
    check_ident("ns", q.ns.as_deref())?;
    let (cluster, cfg, status) = load(&ctx, &id).await?;
    let sink = ctx
        .monitor_sink()
        .filter(|s| s.available())
        .ok_or_else(|| Error::Conflict("usage engine (ClickHouse) is not available".into()))?;
    let snap = snapshot_of(status.as_ref());
    // Every namespace in the snapshot — independent of the `ns` filter, so the
    // picker keeps its full list while one namespace is selected.
    let mut all_namespaces: Vec<String> = snap.values().map(|p| p.namespace.clone()).collect();
    all_namespaces.sort();
    all_namespaces.dedup();
    let ns = q.ns.as_deref().filter(|n| !n.is_empty());
    let mut stats = health::workload_stats(sink.as_ref(), cluster.id.as_str(), &snap, ns, window).await?;
    stats.sort_by(|a, b| a.workload.cmp(&b.workload));

    // Sparklines: memory (gauge) + rps (counter) per workload, ~40 buckets —
    // but never finer than 3 collection cycles, or a counter bucket flips
    // between one sample (Δ = 0) and two (Δ > 0) and draws a sawtooth.
    let step = ((window.num_seconds() / 40).clamp(30, 3600) as u32).max(cfg.interval_secs.saturating_mul(3));
    let mut spark_mem: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    let mut spark_rps: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
    let mem_rows = sink
        .query_rows(&queries::workload_spark_sql(cluster.id.as_str(), ns, &queries::MEMORY_GAUGES, window, step, false))
        .await
        .unwrap_or_default();
    for r in mem_rows {
        let wl = r.get("workload").and_then(Value::as_str).unwrap_or("").to_string();
        spark_mem.entry(wl).or_default().push(num(&r, "v"));
    }
    let rps_rows = sink
        .query_rows(&queries::workload_spark_sql(cluster.id.as_str(), ns, &queries::REQUEST_COUNTERS, window, step, true))
        .await
        .unwrap_or_default();
    for r in rps_rows {
        let wl = r.get("workload").and_then(Value::as_str).unwrap_or("").to_string();
        spark_rps.entry(wl).or_default().push(num(&r, "v"));
    }
    let rows: Vec<Value> = stats
        .into_iter()
        .map(|s| {
            let mut v = serde_json::to_value(&s).unwrap_or_default();
            v["spark"] = json!({
                "mem": spark_mem.get(&s.workload).cloned().unwrap_or_default(),
                "rps": spark_rps.get(&s.workload).cloned().unwrap_or_default(),
            });
            v
        })
        .collect();
    Ok(Json(json!({
        "window": window_label, "step_secs": step, "enabled": cfg.enabled, "status": status,
        "namespaces": all_namespaces, "workloads": rows,
    })))
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k)
        .and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0.0)
}

#[derive(Debug, Default, Deserialize)]
pub struct SeriesQuery {
    pub metric: String,
    pub workload: Option<String>,
    pub pod: Option<String>,
    pub window: Option<String>,
    pub step: Option<u32>,
}

/// `GET /k8s/clusters/{id}/monitor/series?metric=&workload=&pod=&window=&step=`.
async fn series<S: K8sCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<SeriesQuery>,
) -> ApiResult<Json<Value>> {
    let window = queries::parse_window(q.window.as_deref().unwrap_or("1h"))?;
    check_ident("metric", Some(q.metric.as_str()))?;
    if q.metric.is_empty() {
        return Err(Error::Invalid("metric is required".into()).into());
    }
    check_ident("workload", q.workload.as_deref())?;
    check_ident("pod", q.pod.as_deref())?;
    let (cluster, cfg, _) = load(&ctx, &id).await?;
    let sink = ctx
        .monitor_sink()
        .filter(|s| s.available())
        .ok_or_else(|| Error::Conflict("usage engine (ClickHouse) is not available".into()))?;
    // Same rule as the sparklines: a bucket must span ≥ 3 cycles.
    let step = q
        .step
        .unwrap_or_else(|| (window.num_seconds() / 120).clamp(10, 3600) as u32)
        .max(cfg.interval_secs.saturating_mul(3))
        .clamp(10, 86_400);
    let counter = queries::is_counter(&q.metric);
    let sql = queries::series_sql(
        cluster.id.as_str(),
        &q.metric,
        q.workload.as_deref(),
        q.pod.as_deref(),
        window,
        step,
        counter,
    );
    let points: Vec<Value> = sink
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(|r| json!({"t": r.get("t").cloned().unwrap_or(Value::Null), "v": num(&r, "v")}))
        .collect();
    Ok(Json(json!({
        "metric": q.metric, "kind": if counter { "rate" } else { "gauge" }, "step_secs": step, "points": points,
    })))
}

#[derive(Debug, Default, Deserialize)]
pub struct EventsQuery {
    pub window: Option<String>,
    pub class: Option<String>,
    pub workload: Option<String>,
    pub limit: Option<u32>,
}

/// `GET /k8s/clusters/{id}/monitor/events?window=&class=&workload=&limit=`.
async fn events<S: K8sCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<EventsQuery>,
) -> ApiResult<Json<Vec<Value>>> {
    let window = queries::parse_window(q.window.as_deref().unwrap_or("24h"))?;
    check_ident("class", q.class.as_deref())?;
    check_ident("workload", q.workload.as_deref())?;
    let cluster = Clusters::new(&ctx).get(&id).await?;
    let sink = ctx
        .monitor_sink()
        .filter(|s| s.available())
        .ok_or_else(|| Error::Conflict("usage engine (ClickHouse) is not available".into()))?;
    let sql = queries::events_sql(
        cluster.id.as_str(),
        window,
        q.class.as_deref(),
        q.workload.as_deref(),
        q.limit.unwrap_or(200),
    );
    let rows: Vec<Value> = sink
        .query_rows(&sql)
        .await?
        .into_iter()
        .map(|mut r| {
            if let Some(d) = r.get("detail").and_then(Value::as_str) {
                let parsed: Value = serde_json::from_str(d).unwrap_or(Value::Null);
                r["detail"] = parsed;
            }
            r
        })
        .collect();
    Ok(Json(rows))
}

/// `GET /k8s/clusters/{id}/monitor/health?window=` — the `k8s_health` payload.
async fn health_digest<S: K8sCtx>(
    State(ctx): State<S>,
    Path(id): Path<Id>,
    Query(q): Query<WindowQuery>,
) -> ApiResult<Json<Value>> {
    let label = q.window.clone().unwrap_or_else(|| "1h".into());
    let window = queries::parse_window(&label)?;
    let (cluster, cfg, status) = load(&ctx, &id).await?;
    let status = status.unwrap_or_else(|| K8sMonitorStatusRow::empty(cluster.id.as_str()));
    let Some(sink) = ctx.monitor_sink().filter(|s| s.available()) else {
        return Ok(Json(json!({
            "cluster": cluster.name, "cluster_id": cluster.id, "window": label,
            "collector": {"enabled": cfg.enabled, "ok": false, "error": "usage engine (ClickHouse) is not available"},
        })));
    };
    if !cfg.enabled {
        return Ok(Json(json!({
            "cluster": cluster.name, "cluster_id": cluster.id, "window": label,
            "collector": {"enabled": false, "ok": false, "error": "monitoring is disabled for this cluster"},
        })));
    }
    let v = health::health(sink.as_ref(), &cluster, &status, cfg.enabled, window, &label).await?;
    Ok(Json(v))
}
