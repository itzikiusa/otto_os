//! The collector: one cycle = sweep pods → events → metrics-server → pick
//! transport → scrape probes → classify restarts → write ClickHouse → status
//! (spec "Collector cycle"), and the per-cluster loop that repeats it every
//! `interval_secs` with exponential back-off on kubectl failures.
//!
//! Nothing here holds a lock while scraping; the loop is the only writer of
//! its cluster's status row, and the scheduler guarantees one loop per
//! cluster.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use futures_util::stream::{self, StreamExt};
use otto_core::api::AuditLogQuery;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{AuditRepo, K8sCluster, K8sMonitorRepo, K8sMonitorStatusRow};
use serde_json::{json, Value};

use super::classify::{self, ActionHint, Classified, EventHint, PodSnap, Snapshot};
use super::parse::{self, Parsed, Sample, SERIES_CAP};
use super::probes::{self, is_excluded, MonitorConfig, PodRef, ProbeFormat};
use super::schema;
use super::scrape::{self, ScrapeTarget, TransportUsed};
use crate::cli::Kubectl;
use crate::clusters::{kubectl_for, Clusters};
use crate::resources::{self, arr, s};
use crate::{K8sCtx, MonitorSink};

/// Per-pod status series written every cycle from the sweep alone.
pub const STATUS_METRICS: [&str; 6] = [
    "restarts_total",
    "ready",
    "phase_running",
    "mem_limit_bytes",
    "cpu_request_millis",
    "pod_age_seconds",
];

/// Event reasons kept from `kubectl get events` (spec step 2).
const EVENT_REASONS: [&str; 10] = [
    "OOMKilling",
    "Killing",
    "Unhealthy",
    "BackOff",
    "Evicted",
    "Preempted",
    "ScalingReplicaSet",
    "SuccessfulCreate",
    "SuccessfulDelete",
    "FailedScheduling",
];

/// Back-off ceiling after consecutive kubectl failures.
const MAX_BACKOFF: Duration = Duration::from_secs(900);
/// Sleep slice so cancel is observed promptly.
const SLICE: Duration = Duration::from_secs(1);

pub struct CycleOutcome {
    pub status: K8sMonitorStatusRow,
    pub samples_written: usize,
    pub events_written: usize,
    /// The cluster could not be reached at all (kubectl failed on the sweep);
    /// the loop backs off on this.
    pub unreachable: bool,
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// One NDJSON line per sample for `k8s_samples`.
pub fn samples_ndjson(
    cluster_id: &str,
    at: DateTime<Utc>,
    pod: &PodSnap,
    container: &str,
    samples: &[Sample],
    extra_labels: &BTreeMap<String, String>,
) -> String {
    let mut out = String::with_capacity(samples.len() * 160);
    let at = ts(at);
    for smp in samples {
        let mut labels = extra_labels.clone();
        labels.extend(smp.labels.iter().map(|(k, v)| (k.clone(), v.clone())));
        let row = json!({
            "ts": at,
            "cluster_id": cluster_id,
            "namespace": pod.namespace,
            "workload_kind": pod.workload_kind,
            "workload": pod.workload,
            "pod": pod.name,
            "container": container,
            "metric": smp.metric,
            "labels": labels,
            "value": smp.value,
        });
        out.push_str(&row.to_string());
        out.push('\n');
    }
    out
}

/// NDJSON for `k8s_events`: classified restarts / churn + raw k8s events.
pub fn events_ndjson(
    cluster_id: &str,
    at: DateTime<Utc>,
    rows: &[Classified],
    k8s_events: &[EventHint],
    snaps: &Snapshot,
) -> String {
    let mut out = String::new();
    for r in rows {
        let row = json!({
            "ts": r.at,
            "cluster_id": cluster_id,
            "namespace": r.namespace,
            "workload": r.workload,
            "pod": r.pod,
            "container": r.container,
            "kind": r.kind,
            "class": r.class.as_str(),
            "reason": r.reason,
            "exit_code": r.exit_code,
            "detail": json!({
                "planned_by": r.planned_by,
                "workload_kind": r.workload_kind,
                "prev_restarts": r.prev_restarts,
                "next_restarts": r.next_restarts,
            }).to_string(),
            "actor": r.planned_by.strip_prefix("otto:").unwrap_or(""),
        });
        out.push_str(&row.to_string());
        out.push('\n');
    }
    let _ = at;
    for e in k8s_events {
        let workload = if e.involved_kind == "Pod" {
            snaps
                .get(&classify::snap_key(&e.namespace, &e.involved_name))
                .map(|p| p.workload.clone())
                .unwrap_or_default()
        } else {
            e.involved_name.clone()
        };
        let row = json!({
            "ts": ts(e.at),
            "cluster_id": cluster_id,
            "namespace": e.namespace,
            "workload": workload,
            "pod": if e.involved_kind == "Pod" { e.involved_name.as_str() } else { "" },
            "container": "",
            "kind": "k8s_event",
            "class": "",
            "reason": e.reason,
            "exit_code": 0,
            "detail": json!({"message": e.message, "involved_kind": e.involved_kind, "involved_name": e.involved_name}).to_string(),
            "actor": "",
        });
        out.push_str(&row.to_string());
        out.push('\n');
    }
    out
}

/// Status series for one pod (from the sweep alone).
pub fn status_samples(p: &PodSnap, now: DateTime<Utc>) -> Vec<Sample> {
    let restarts: i64 = p.containers.values().map(|c| c.restarts).sum();
    let age = DateTime::parse_from_rfc3339(&p.created)
        .map(|t| (now - t.with_timezone(&Utc)).num_seconds().max(0))
        .unwrap_or(0);
    let mk = |m: &str, v: f64| Sample {
        metric: m.into(),
        labels: BTreeMap::new(),
        value: v,
    };
    vec![
        mk("restarts_total", restarts as f64),
        mk("ready", if p.ready { 1.0 } else { 0.0 }),
        mk("phase_running", if p.phase == "Running" { 1.0 } else { 0.0 }),
        mk("mem_limit_bytes", p.mem_limit as f64),
        mk("cpu_request_millis", p.cpu_request as f64),
        mk("pod_age_seconds", age as f64),
    ]
}

/// `kubectl get events -o json` → hints newer than `since`.
pub fn parse_event_hints(list: &Value, since: Option<DateTime<Utc>>) -> Vec<EventHint> {
    arr(list, "/items")
        .iter()
        .filter_map(|e| {
            let reason = s(e, "/reason")?;
            if !EVENT_REASONS.contains(&reason) {
                return None;
            }
            let at = s(e, "/lastTimestamp")
                .or_else(|| s(e, "/eventTime"))
                .or_else(|| s(e, "/firstTimestamp"))
                .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                .map(|t| t.with_timezone(&Utc))?;
            if let Some(since) = since {
                if at < since {
                    return None;
                }
            }
            Some(EventHint {
                namespace: s(e, "/metadata/namespace")
                    .or_else(|| s(e, "/involvedObject/namespace"))
                    .unwrap_or("")
                    .to_string(),
                reason: reason.to_string(),
                message: s(e, "/message").unwrap_or("").to_string(),
                at,
                involved_kind: s(e, "/involvedObject/kind").unwrap_or("").to_string(),
                involved_name: s(e, "/involvedObject/name").unwrap_or("").to_string(),
            })
        })
        .collect()
}

/// Otto's own `k8s.action.*` audit rows on this cluster in the last 5 min.
async fn action_hints<S: K8sCtx>(ctx: &S, cluster_id: &Id, now: DateTime<Utc>) -> Vec<ActionHint> {
    let q = AuditLogQuery {
        from: Some(now - chrono::Duration::minutes(5)),
        to: None,
        action: None,
        user_id: None,
        limit: Some(200),
        offset: None,
    };
    let rows = match AuditRepo::new(ctx.pool()).list(&q).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!("k8s monitor: audit lookup failed: {e}");
            return vec![];
        }
    };
    rows.into_iter()
        .filter(|r| r.action.starts_with("k8s.action.") && r.target.as_deref() == Some(cluster_id.as_str()))
        .filter_map(|r| {
            let d = r.detail?;
            let name = d.get("name")?.as_str()?.to_string();
            let kind = d.get("kind").and_then(Value::as_str).unwrap_or("");
            // Pod-level actions name the pod; strip the RS/pod suffixes so
            // they land on the workload like everything else.
            let workload = if kind == "pods" || kind == "pod" {
                let base = name.rsplitn(3, '-').last().unwrap_or(&name).to_string();
                base
            } else {
                name
            };
            Some(ActionHint {
                namespace: d.get("ns").and_then(Value::as_str).unwrap_or("").to_string(),
                workload,
                at: r.ts,
                actor: r.user_id.map(|u| u.to_string()).unwrap_or_else(|| "otto".into()),
            })
        })
        .collect()
}

/// Scrape one pod: every probe on every port, parsed into NDJSON.
async fn scrape_pod(
    k: &Kubectl,
    cluster_id: &str,
    transport: TransportUsed,
    cfg: &MonitorConfig,
    pod: &PodSnap,
    now: DateTime<Utc>,
) -> (bool, u32, u32, String) {
    let (by_port, unresolved) = scrape::group_by_port(&cfg.probes, pod.first_port);
    let mut any_ok = false;
    let mut parse_errors = unresolved.len() as u32;
    let mut capped = 0u32;
    let mut ndjson = String::new();
    let mut pod_labels: BTreeMap<String, String> = BTreeMap::new();
    let mut per_probe: Vec<(String, Parsed)> = Vec::new();
    for (port, probes) in by_port {
        let target = ScrapeTarget {
            namespace: pod.namespace.clone(),
            pod: pod.name.clone(),
            port,
        };
        let results = scrape::fetch(k, transport, &target, &probes).await;
        for (probe, res) in probes.iter().zip(results) {
            match res {
                Ok(r) => {
                    let parsed = match probe.format {
                        ProbeFormat::Prometheus => {
                            if (200..300).contains(&r.status) {
                                parse::parse_prometheus(&r.body, &probe.include, &probe.exclude, SERIES_CAP)
                            } else {
                                Parsed {
                                    parse_errors: 1,
                                    ..Parsed::default()
                                }
                            }
                        }
                        ProbeFormat::Json => {
                            if (200..300).contains(&r.status) {
                                parse::parse_json(&r.body, &probe.mappings)
                            } else {
                                Parsed {
                                    parse_errors: 1,
                                    ..Parsed::default()
                                }
                            }
                        }
                        ProbeFormat::Health => parse::parse_health(r.status),
                    };
                    any_ok = true;
                    parse_errors += parsed.parse_errors;
                    if parsed.capped {
                        capped += 1;
                    }
                    pod_labels.extend(parsed.labels.clone());
                    per_probe.push((probe.name.clone(), parsed));
                }
                Err(e) => {
                    tracing::debug!(pod = %pod.name, probe = %probe.name, "k8s monitor scrape failed: {e}");
                    parse_errors += 1;
                    // A failed health probe is still a signal: up=0.
                    if probe.format == ProbeFormat::Health {
                        per_probe.push((probe.name.clone(), parse::parse_health(0)));
                    }
                }
            }
        }
    }
    let container = pod.containers.keys().next().cloned().unwrap_or_default();
    for (_, parsed) in &per_probe {
        ndjson.push_str(&samples_ndjson(
            cluster_id,
            now,
            pod,
            &container,
            &parsed.samples,
            &pod_labels,
        ));
    }
    (any_ok, parse_errors, capped, ndjson)
}

/// Run one full cycle (spec steps 1–10). Never panics on cluster errors: an
/// unreachable cluster yields `unreachable = true` + `last_error`.
pub async fn run_cycle<S: K8sCtx>(
    ctx: &S,
    cluster: &K8sCluster,
    cfg: &MonitorConfig,
    prev: &Snapshot,
    prev_cycle_at: Option<DateTime<Utc>>,
    sink: &dyn MonitorSink,
) -> CycleOutcome {
    let started = Instant::now();
    let now = Utc::now();
    let cid = cluster.id.to_string();
    let mut status = K8sMonitorStatusRow::empty(&cid);
    status.last_cycle_at = Some(ts(now));
    status.snapshot = serde_json::to_value(prev).unwrap_or_default();

    let k = match kubectl_for(ctx, cluster).await {
        Ok(k) => k,
        Err(e) => {
            status.last_error = e.to_string();
            status.cycle_ms = started.elapsed().as_millis() as i64;
            return CycleOutcome {
                status,
                samples_written: 0,
                events_written: 0,
                unreachable: true,
            };
        }
    };
    let namespaces = cfg.effective_namespaces(cluster.default_namespace.as_deref());

    // 1. Sweep.
    let mut cur = Snapshot::new();
    for ns in &namespaces {
        match k.json(["get", "pods", "-n", ns.as_str(), "-o", "json"]).await {
            Ok(list) => cur.extend(classify::snapshot_from_pod_list(&list)),
            Err(e) => {
                status.last_error = format!("list pods in {ns}: {e}");
                status.cycle_ms = started.elapsed().as_millis() as i64;
                return CycleOutcome {
                    status,
                    samples_written: 0,
                    events_written: 0,
                    unreachable: true,
                };
            }
        }
    }
    status.pods_seen = cur.len() as i64;
    let mut samples_nd = String::new();
    for p in cur.values() {
        let container = p.containers.keys().next().cloned().unwrap_or_default();
        samples_nd.push_str(&samples_ndjson(&cid, now, p, &container, &status_samples(p, now), &BTreeMap::new()));
    }

    // 2. Events.
    let mut events: Vec<EventHint> = Vec::new();
    for ns in &namespaces {
        match k.json(["get", "events", "-n", ns.as_str(), "-o", "json"]).await {
            Ok(list) => events.extend(parse_event_hints(&list, prev_cycle_at)),
            Err(e) => tracing::debug!("k8s monitor: events in {ns}: {e}"),
        }
    }

    // 3. Metrics-server (re-probed every cycle, never cached).
    let mut ms_state = "absent".to_string();
    for ns in &namespaces {
        match resources::pod_metrics(&k, Some(ns)).await {
            Ok(pods) => {
                ms_state = "ok".into();
                for pm in pods {
                    let Some(p) = cur.get(&classify::snap_key(&pm.namespace, &pm.name)) else {
                        continue;
                    };
                    for c in &pm.containers {
                        let smp = vec![
                            Sample {
                                metric: "cpu_millis".into(),
                                labels: BTreeMap::new(),
                                value: c.cpu_millicores as f64,
                            },
                            Sample {
                                metric: "mem_working_set_bytes".into(),
                                labels: BTreeMap::new(),
                                value: c.mem_bytes as f64,
                            },
                        ];
                        samples_nd.push_str(&samples_ndjson(&cid, now, p, &c.name, &smp, &BTreeMap::new()));
                    }
                }
            }
            Err(Error::Forbidden(m)) => {
                ms_state = format!("forbidden: {m}");
                break;
            }
            Err(e) => {
                tracing::debug!("k8s monitor: metrics-server: {e}");
            }
        }
    }
    status.metrics_server = ms_state;

    // 4. Targets + transport.
    let targets: Vec<&PodSnap> = cur
        .values()
        .filter(|p| p.phase == "Running" && !p.deleting)
        .filter(|p| {
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
        .collect();
    let mut parse_errors = 0u32;
    let mut capped = 0u32;
    if !cfg.probes.is_empty() {
        if let Some(first) = targets.first() {
            let probe0 = &cfg.probes[0];
            let sample = ScrapeTarget {
                namespace: first.namespace.clone(),
                pod: first.name.clone(),
                port: probe0.port.or(first.first_port).unwrap_or(80),
            };
            let transport = scrape::pick_transport(&k, cfg.transport, &sample, &probe0.path).await;
            status.transport_used = transport.as_str().into();

            // 5. Scrape with bounded concurrency.
            let concurrency = cfg.concurrency.clamp(1, probes::MAX_CONCURRENCY) as usize;
            let k_ref = &k;
            let cid_ref = cid.as_str();
            let results: Vec<(bool, u32, u32, String)> = stream::iter(targets.iter().copied())
                .map(|p| async move { scrape_pod(k_ref, cid_ref, transport, cfg, p, now).await })
                .buffer_unordered(concurrency)
                .collect()
                .await;
            for (ok, pe, cp, nd) in results {
                if ok {
                    status.pods_scraped += 1;
                } else {
                    status.pods_failed += 1;
                }
                parse_errors += pe;
                capped += cp;
                samples_nd.push_str(&nd);
            }
        }
    }

    // 6–7. Classify.
    let actions = action_hints(ctx, &cluster.id, now).await;
    let classified = classify::classify(prev, &cur, &events, &actions, now);
    let events_nd = events_ndjson(&cid, now, &classified, &events, &cur);

    // 8. Write.
    let samples_written = samples_nd.lines().count();
    let events_written = events_nd.lines().count();
    let mut write_err = None;
    if samples_written > 0 {
        if let Err(e) = sink.insert_ndjson("k8s_samples", &samples_nd).await {
            write_err = Some(format!("write samples: {e}"));
        }
    }
    if events_written > 0 {
        if let Err(e) = sink.insert_ndjson("k8s_events", &events_nd).await {
            write_err = Some(format!("write events: {e}"));
        }
    }

    // 9. Status.
    status.snapshot = serde_json::to_value(&cur).unwrap_or_default();
    status.cycle_ms = started.elapsed().as_millis() as i64;
    match write_err {
        Some(e) => status.last_error = e,
        None => {
            status.last_ok_at = Some(ts(now));
            let mut notes = Vec::new();
            if parse_errors > 0 {
                notes.push(format!("{parse_errors} parse error(s)"));
            }
            if capped > 0 {
                notes.push(format!("series_capped on {capped} probe(s)"));
            }
            status.last_error = notes.join("; ");
        }
    }
    CycleOutcome {
        status,
        samples_written,
        events_written,
        unreachable: false,
    }
}

/// The per-cluster loop. Returns when cancelled, when the config is removed
/// or disabled, or when the cluster row is gone.
pub async fn run_loop<S: K8sCtx>(ctx: S, cluster_id: Id, cancel: Arc<AtomicBool>) {
    let repo = K8sMonitorRepo::new(ctx.pool());
    let Some(sink) = ctx.monitor_sink() else {
        tracing::warn!(cluster = %cluster_id, "k8s monitor: no sink; loop not started");
        return;
    };
    let mut schema_ready = false;
    let mut failures: u32 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(Some(row)) = repo.get_config(cluster_id.as_str()).await else {
            return;
        };
        if !row.enabled {
            return;
        }
        let cfg = probes::from_row(&row);
        let cluster = match Clusters::new(&ctx).get(&cluster_id).await {
            Ok(c) => c,
            Err(_) => return,
        };
        let interval = Duration::from_secs(u64::from(cfg.interval_secs.max(probes::MIN_INTERVAL)));

        if !schema_ready {
            if !sink.available() {
                let mut st = repo
                    .get_status(cluster_id.as_str())
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| K8sMonitorStatusRow::empty(cluster_id.as_str()));
                st.last_error = "usage engine (ClickHouse) is not available".into();
                let _ = repo.upsert_status(&st).await;
                sleep_slices(interval, &cancel).await;
                continue;
            }
            match sink.exec(&schema::schema_sql(cfg.retention_days)).await {
                Ok(()) => {
                    let _ = sink.exec(&schema::alter_ttl_sql(cfg.retention_days.max(largest_retention(&repo).await))).await;
                    schema_ready = true;
                }
                Err(e) => {
                    tracing::warn!(cluster = %cluster_id, "k8s monitor: schema init failed: {e}");
                    sleep_slices(interval, &cancel).await;
                    continue;
                }
            }
        }

        let prev_status = repo.get_status(cluster_id.as_str()).await.ok().flatten();
        let prev: Snapshot = prev_status
            .as_ref()
            .map(|s| serde_json::from_value(s.snapshot.clone()).unwrap_or_default())
            .unwrap_or_default();
        let prev_at = prev_status
            .as_ref()
            .and_then(|s| s.last_cycle_at.as_deref())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));

        let out = run_cycle(&ctx, &cluster, &cfg, &prev, prev_at, sink.as_ref()).await;
        let ok = out.status.last_ok_at.is_some();
        if let Err(e) = repo.upsert_status(&out.status).await {
            tracing::warn!(cluster = %cluster_id, "k8s monitor: status write failed: {e}");
        }
        let _ = ctx.events().send(Event::K8sMonitorCycle {
            cluster_id: cluster_id.clone(),
            ok,
            pods_scraped: out.status.pods_scraped.max(0) as u32,
            pods_failed: out.status.pods_failed.max(0) as u32,
            cycle_ms: out.status.cycle_ms.max(0) as u64,
        });
        tracing::debug!(
            cluster = %cluster_id, ok, samples = out.samples_written, events = out.events_written,
            ms = out.status.cycle_ms, "k8s monitor cycle"
        );

        // Trim clusters that keep less than the table TTL.
        if ok {
            let cutoff = (Utc::now() - chrono::Duration::days(i64::from(cfg.retention_days)))
                .format("%Y-%m-%d")
                .to_string();
            for q in schema::purge_cluster_sql(cluster_id.as_str(), Some(&cutoff)) {
                if let Err(e) = sink.exec(&q).await {
                    tracing::debug!("k8s monitor purge: {e}");
                }
            }
        }

        let wait = if out.unreachable {
            failures = failures.saturating_add(1);
            let mult = 2u32.saturating_pow(failures.min(10));
            interval.saturating_mul(mult).min(MAX_BACKOFF).max(interval)
        } else {
            failures = 0;
            let elapsed = Duration::from_millis(out.status.cycle_ms.max(0) as u64);
            interval.saturating_sub(elapsed)
        };
        sleep_slices(wait, &cancel).await;
    }
}

async fn largest_retention(repo: &K8sMonitorRepo) -> u32 {
    repo.list_enabled()
        .await
        .map(|rows| rows.iter().map(|r| r.retention_days.clamp(1, 90) as u32).max().unwrap_or(14))
        .unwrap_or(14)
}

async fn sleep_slices(total: Duration, cancel: &AtomicBool) {
    let mut waited = Duration::ZERO;
    while waited < total {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let step = SLICE.min(total - waited);
        tokio::time::sleep(step).await;
        waited += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::classify::{Class, ContainerSnap};

    fn fixture_pod() -> PodSnap {
        let mut containers = BTreeMap::new();
        containers.insert(
            "auditlog".to_string(),
            ContainerSnap {
                restarts: 2,
                ..ContainerSnap::default()
            },
        );
        PodSnap {
            namespace: "mscasino".into(),
            name: "auditlog-7c8dc556fb-fpzfd".into(),
            phase: "Running".into(),
            ready: true,
            workload_kind: "deployment".into(),
            workload: "auditlog".into(),
            containers,
            mem_limit: 256 * 1024 * 1024,
            cpu_request: 100,
            created: "2026-09-01T00:00:00Z".into(),
            ..PodSnap::default()
        }
    }

    #[test]
    fn samples_ndjson_rows_carry_workload_and_labels() {
        let pod = fixture_pod();
        let s = vec![Sample {
            metric: "mem_sys_bytes".into(),
            labels: BTreeMap::new(),
            value: 1.0,
        }];
        let mut extra = BTreeMap::new();
        extra.insert("version".to_string(), "5.02.25".to_string());
        let nd = samples_ndjson("c1", Utc::now(), &pod, "auditlog", &s, &extra);
        let row: Value = serde_json::from_str(nd.lines().next().unwrap()).unwrap();
        assert_eq!(row["workload"], "auditlog");
        assert_eq!(row["workload_kind"], "deployment");
        assert_eq!(row["labels"]["version"], "5.02.25");
        assert_eq!(row["metric"], "mem_sys_bytes");
        assert_eq!(row["cluster_id"], "c1");
        assert!(row["ts"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn status_samples_cover_the_six_metrics() {
        let s = status_samples(&fixture_pod(), Utc::now());
        let names: Vec<&str> = s.iter().map(|x| x.metric.as_str()).collect();
        assert_eq!(names, STATUS_METRICS.to_vec());
        assert_eq!(s[0].value, 2.0);
        assert_eq!(s[3].value, (256 * 1024 * 1024) as f64);
        assert!(s[5].value > 0.0);
    }

    #[test]
    fn events_ndjson_encodes_class_and_detail() {
        let c = Classified {
            kind: "restart",
            class: Class::Oom,
            namespace: "ns".into(),
            workload_kind: "deployment".into(),
            workload: "frb".into(),
            pod: "frb-1".into(),
            container: "frb".into(),
            reason: "OOMKilled".into(),
            exit_code: 137,
            planned_by: String::new(),
            prev_restarts: 0,
            next_restarts: 1,
            at: "2026-09-05T08:00:00Z".into(),
        };
        let ev = EventHint {
            namespace: "ns".into(),
            reason: "OOMKilling".into(),
            message: "Memory cgroup out of memory".into(),
            at: Utc::now(),
            involved_kind: "Pod".into(),
            involved_name: "frb-1".into(),
        };
        let nd = events_ndjson("c1", Utc::now(), &[c], &[ev], &Snapshot::new());
        let lines: Vec<Value> = nd.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["kind"], "restart");
        assert_eq!(lines[0]["class"], "oom");
        assert_eq!(lines[0]["exit_code"], 137);
        let detail: Value = serde_json::from_str(lines[0]["detail"].as_str().unwrap()).unwrap();
        assert_eq!(detail["next_restarts"], 1);
        assert_eq!(lines[1]["kind"], "k8s_event");
        assert_eq!(lines[1]["reason"], "OOMKilling");
    }

    #[test]
    fn event_hints_filter_reasons_and_since() {
        let list = json!({"items":[
            {"reason":"OOMKilling","message":"m","lastTimestamp":"2026-09-05T08:00:00Z","involvedObject":{"kind":"Pod","name":"p","namespace":"ns"},"metadata":{"namespace":"ns"}},
            {"reason":"Scheduled","message":"m","lastTimestamp":"2026-09-05T08:00:00Z","involvedObject":{"kind":"Pod","name":"p","namespace":"ns"}},
            {"reason":"Killing","message":"old","lastTimestamp":"2026-09-05T07:00:00Z","involvedObject":{"kind":"Pod","name":"p","namespace":"ns"}}
        ]});
        let since = DateTime::parse_from_rfc3339("2026-09-05T07:30:00Z").unwrap().with_timezone(&Utc);
        let h = parse_event_hints(&list, Some(since));
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].reason, "OOMKilling");
        assert_eq!(h[0].namespace, "ns");
        assert_eq!(parse_event_hints(&list, None).len(), 2);
    }
}
