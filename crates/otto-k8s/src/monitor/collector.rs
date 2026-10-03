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
use super::gateway::{self, KubeProxy};
use super::parse::{self, Parsed, Sample};
use super::probes::{self, is_excluded, MonitorConfig, PodRef, ProbeFormat, Transport};
use super::schema;
use super::scrape::{self, ScrapeTarget, TransportUsed};
use super::wide;
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

/// kubectl lists in flight at once per cycle (pods, then events).
const SWEEP_CONCURRENCY: usize = 4;

/// Parse kubectl JSON and reduce it with `f` on the blocking pool (a 5k-pod
/// list is ~120 ms of `serde_json::Value` parsing — never on a runtime worker).
async fn parse_off_runtime<T: Send + 'static>(
    stdout: String,
    f: impl FnOnce(Value) -> T + Send + 'static,
) -> otto_core::Result<T> {
    tokio::task::spawn_blocking(move || crate::cli::parse_json(&stdout).map(f))
        .await
        .map_err(|e| Error::Internal(format!("k8s monitor parse task: {e}")))?
}

/// Persist a cycle's status, rewriting the (large) snapshot column only when
/// `cur` (this cycle's typed snapshot; `None` = the cycle never got one, keep
/// what is stored) differs from `baseline` — what this loop last stored. The
/// comparison is typed and the serialisation runs on the blocking pool
/// straight to text: no `Value` round-trip per cycle (perf K4). Returns
/// whether the snapshot was written.
async fn store_status(
    repo: &K8sMonitorRepo,
    status: &K8sMonitorStatusRow,
    cur: Option<&Arc<Snapshot>>,
    baseline: Option<&Arc<Snapshot>>,
) -> otto_core::Result<bool> {
    let changed = match (cur, baseline) {
        (None, _) => None,
        (Some(c), Some(b)) if Arc::ptr_eq(c, b) || **c == **b => None,
        (Some(c), _) => Some(c.clone()),
    };
    match changed {
        None => {
            repo.upsert_status_keep_snapshot(status).await?;
            Ok(false)
        }
        Some(snap) => {
            let raw = tokio::task::spawn_blocking(move || {
                serde_json::to_string(&*snap).unwrap_or_else(|_| "{}".into())
            })
            .await
            .map_err(|e| Error::Internal(format!("k8s monitor snapshot task: {e}")))?;
            repo.upsert_status_with_snapshot_str(status, &raw).await?;
            Ok(true)
        }
    }
}

/// Persist a one-off cycle's outcome (the "Run now" route) and publish its
/// snapshot for the dashboards.
pub async fn persist_outcome(repo: &K8sMonitorRepo, out: &CycleOutcome) -> otto_core::Result<()> {
    let cur = out.snapshot.clone().map(Arc::new);
    store_status(repo, &out.status, cur.as_ref(), None).await?;
    if let Some(snap) = cur {
        super::latest::publish(
            &out.status.cluster_id,
            out.status.last_cycle_at.clone(),
            snap,
        );
    }
    Ok(())
}

/// Re-sniff a cached `Auto` transport decision at least this often.
const TRANSPORT_TTL: Duration = Duration::from_secs(3600);
/// Status series (restarts, ready, phase, limits) are written when they change
/// and for every pod at least this often (perf K8: they were 6 rows per pod
/// per cycle into the narrow table, read only by the generic series picker).
const STATUS_HEARTBEAT: Duration = Duration::from_secs(15 * 60);
/// A watch-based incremental events read holds the request this long.
const EVENTS_WATCH_SECS: u64 = 1;

/// A cached transport decision.
#[derive(Debug, Clone, Copy)]
struct CachedTransport {
    used: TransportUsed,
    want: Transport,
    at: Instant,
}

/// What the per-cluster loop keeps across cycles: the `kubectl proxy`
/// gateway, the transport decision (perf K1), the port-forward pool, the
/// per-namespace events `resourceVersion` (incremental reads, K5) and when
/// status series were last written in full (K8).
#[derive(Default)]
pub struct LoopState {
    pub gateway: Option<KubeProxy>,
    transport: Option<CachedTransport>,
    pub forwards: scrape::ForwardPool,
    event_rv: std::collections::HashMap<String, String>,
    status_full_at: Option<Instant>,
}

/// The cached `Auto` decision when it is still valid for `want` (field-level
/// helpers: the gateway borrow of the same `LoopState` is live meanwhile).
fn cached_transport(
    c: &Option<CachedTransport>,
    want: Transport,
    now: Instant,
) -> Option<TransportUsed> {
    c.filter(|c| c.want == want && now.duration_since(c.at) < TRANSPORT_TTL)
        .map(|c| c.used)
}

fn remember_transport(
    c: &mut Option<CachedTransport>,
    want: Transport,
    used: TransportUsed,
    now: Instant,
) {
    *c = Some(CachedTransport {
        used,
        want,
        at: now,
    });
}

/// Back-off ceiling after consecutive kubectl failures.
const MAX_BACKOFF: Duration = Duration::from_secs(900);
/// How often a cluster whose retention is shorter than the table TTL trims its
/// old rows (a lightweight DELETE is a ClickHouse mutation; per cycle it was
/// 2 × 1,440 mutations a day per cluster — r3-08-03).
const PURGE_EVERY: Duration = Duration::from_secs(24 * 3600);

pub struct CycleOutcome {
    pub status: K8sMonitorStatusRow,
    /// This cycle's typed pod snapshot; `None` when the cycle failed before
    /// the sweep finished (the stored one stays the baseline).
    pub snapshot: Option<Snapshot>,
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
        mk(
            "phase_running",
            if p.phase == "Running" { 1.0 } else { 0.0 },
        ),
        mk("mem_limit_bytes", p.mem_limit as f64),
        mk("cpu_request_millis", p.cpu_request as f64),
        mk("pod_age_seconds", age as f64),
    ]
}

/// metrics-server's per-container `(cpu millicores, memory bytes)` as ONE
/// pod-total sample pair.
pub fn metrics_server_samples(containers: impl Iterator<Item = (i64, i64)>) -> Vec<Sample> {
    let (cpu, mem) = containers.fold((0i64, 0i64), |(c, m), (cc, mm)| (c + cc, m + mm));
    vec![
        Sample {
            metric: "cpu_millis".into(),
            labels: BTreeMap::new(),
            value: cpu as f64,
        },
        Sample {
            metric: "mem_working_set_bytes".into(),
            labels: BTreeMap::new(),
            value: mem as f64,
        },
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
        .filter(|r| {
            r.action.starts_with("k8s.action.") && r.target.as_deref() == Some(cluster_id.as_str())
        })
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
                namespace: d
                    .get("ns")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                workload,
                at: r.ts,
                actor: r
                    .user_id
                    .map(|u| u.to_string())
                    .unwrap_or_else(|| "otto".into()),
            })
        })
        .collect()
}

/// What one pod's scrape produced.
struct Scraped {
    ok: bool,
    parse_errors: u32,
    capped: u32,
    ndjson: String,
    version: String,
    /// Every sample with its labels merged exactly as written to raw (the
    /// wide row's input).
    samples: Vec<Sample>,
}

/// Scrape one pod: every probe on every port, parsed into NDJSON.
#[allow(clippy::too_many_arguments)]
async fn scrape_pod(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    pool: &scrape::ForwardPool,
    cluster_id: &str,
    transport: TransportUsed,
    cfg: &MonitorConfig,
    pod: &PodSnap,
    now: DateTime<Utc>,
) -> Scraped {
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
        let results = scrape::fetch_pooled(k, gw, Some(pool), transport, &target, &probes).await;
        for (probe, res) in probes.iter().zip(results) {
            match res {
                Ok(r) => {
                    let parsed = match probe.format {
                        ProbeFormat::Prometheus => {
                            if (200..300).contains(&r.status) {
                                let mut parsed = parse::parse_prometheus(
                                    &r.body,
                                    &probe.include,
                                    &probe.exclude,
                                    cfg.series_cap as usize,
                                );
                                parsed.samples = parse::collapse_labels_with(
                                    std::mem::take(&mut parsed.samples),
                                    cfg.request_labels,
                                );
                                parsed
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
    let version = pod_labels.get("version").cloned().unwrap_or_default();
    let samples = per_probe
        .into_iter()
        .flat_map(|(_, parsed)| parsed.samples)
        .map(|mut smp| {
            let mut labels = pod_labels.clone();
            labels.append(&mut smp.labels);
            smp.labels = labels;
            smp
        })
        .collect();
    Scraped {
        ok: any_ok,
        parse_errors,
        capped,
        ndjson,
        version,
        samples,
    }
}

/// Run one full cycle (spec steps 1–10). Never panics on cluster errors: an
/// unreachable cluster yields `unreachable = true` + `last_error`.
///
/// A one-off cycle: the `kubectl proxy` gateway / port-forwards it may start
/// die with it. The per-cluster loop uses [`run_cycle_with`] to keep them.
pub async fn run_cycle<S: K8sCtx>(
    ctx: &S,
    cluster: &K8sCluster,
    cfg: &MonitorConfig,
    prev: &Snapshot,
    prev_cycle_at: Option<DateTime<Utc>>,
    sink: &dyn MonitorSink,
) -> CycleOutcome {
    let mut state = LoopState::default();
    run_cycle_with(ctx, cluster, cfg, prev, prev_cycle_at, sink, &mut state).await
}

/// Did the status series of `p` change since `old` (restarts, readiness,
/// phase, limits — `pod_age_seconds` is derivable and never written)?
fn status_changed(p: &PodSnap, old: Option<&PodSnap>) -> bool {
    let Some(o) = old else { return true };
    let restarts = |x: &PodSnap| x.containers.values().map(|c| c.restarts).sum::<i64>();
    restarts(p) != restarts(o)
        || p.ready != o.ready
        || (p.phase == "Running") != (o.phase == "Running")
        || p.mem_limit != o.mem_limit
        || p.cpu_request != o.cpu_request
}

/// A namespace's event hints + the `resourceVersion` to resume from.
type NamespaceEvents = (Vec<EventHint>, Option<String>);

/// One namespace's event hints newer than `since` (perf K5). With a gateway
/// and a known `resourceVersion` it is an incremental watch read — only the
/// events after that version, the API server closes it after
/// [`EVENTS_WATCH_SECS`]; otherwise (first cycle, expired version, no
/// gateway) a full list. Returns the hints and the version to resume from.
async fn namespace_events(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    ns: &str,
    rv: Option<String>,
    since: Option<DateTime<Utc>>,
) -> otto_core::Result<NamespaceEvents> {
    if let (Some(gw), Some(rv)) = (gw, rv.as_deref()) {
        let path = format!(
            "/api/v1/namespaces/{ns}/events?watch=1&resourceVersion={rv}&timeoutSeconds={EVENTS_WATCH_SECS}&allowWatchBookmarks=false"
        );
        if let Ok(r) = gw
            .get(
                &path,
                Duration::from_secs(EVENTS_WATCH_SECS + 5),
                32 * 1024 * 1024,
            )
            .await
        {
            if r.status == 200 {
                let rv = rv.to_string();
                let parsed = tokio::task::spawn_blocking(move || parse_event_watch(&r.body, since))
                    .await
                    .map_err(|e| Error::Internal(format!("k8s monitor parse task: {e}")))?;
                if let Some((hints, new_rv)) = parsed {
                    return Ok((hints, Some(new_rv.unwrap_or(rv))));
                }
                // Version expired (410): fall through to a full relist.
            }
        }
    }
    if let Some(gw) = gw {
        if let Ok(r) = gw
            .get(
                &format!("/api/v1/namespaces/{ns}/events"),
                Duration::from_secs(30),
                256 * 1024 * 1024,
            )
            .await
        {
            if r.status == 200 {
                return parse_off_runtime(r.body, move |list| {
                    let rv = s(&list, "/metadata/resourceVersion")
                        .filter(|v| !v.is_empty())
                        .map(str::to_string);
                    (parse_event_hints(&list, since), rv)
                })
                .await;
            }
        }
    }
    let out = k.run(["get", "events", "-n", ns, "-o", "json"]).await?;
    parse_off_runtime(out.stdout, move |list| {
        (parse_event_hints(&list, since), None)
    })
    .await
}

/// A watch response (one JSON event per line) → hints + the newest
/// `resourceVersion` seen. `None` when the watch reports an `ERROR` (the
/// version expired: relist).
pub fn parse_event_watch(
    body: &str,
    since: Option<DateTime<Utc>>,
) -> Option<(Vec<EventHint>, Option<String>)> {
    let mut items = Vec::new();
    let mut rv = None;
    for line in body.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(ev) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match ev.get("type").and_then(Value::as_str) {
            Some("ERROR") => return None,
            Some("ADDED") | Some("MODIFIED") => {
                if let Some(obj) = ev.get("object") {
                    if let Some(v) = s(obj, "/metadata/resourceVersion") {
                        rv = Some(v.to_string());
                    }
                    items.push(obj.clone());
                }
            }
            _ => {}
        }
    }
    let hints = parse_event_hints(&json!({ "items": items }), since);
    Some((hints, rv))
}

/// metrics-server for one namespace — through the gateway when there is one
/// (an HTTP request, not a process), parsed off the runtime either way.
async fn namespace_metrics(
    k: &Kubectl,
    gw: Option<&KubeProxy>,
    ns: &str,
) -> otto_core::Result<Vec<resources::PodMetrics>> {
    let path = format!("/apis/metrics.k8s.io/v1beta1/namespaces/{ns}/pods");
    if let Some(gw) = gw {
        if let Ok(r) = gw
            .get(&path, Duration::from_secs(30), 64 * 1024 * 1024)
            .await
        {
            match r.status {
                200 => {
                    return parse_off_runtime(r.body, |v| resources::parse_pod_metrics(&v)).await
                }
                401 | 403 => {
                    return Err(Error::Forbidden(format!(
                        "cluster RBAC: metrics-server answered HTTP {}",
                        r.status
                    )))
                }
                _ => {}
            }
        }
    }
    let out = k.run(["get", "--raw", path.as_str()]).await?;
    parse_off_runtime(out.stdout, |v| resources::parse_pod_metrics(&v)).await
}

/// [`run_cycle`] reusing the loop's [`LoopState`]: the cluster's `kubectl
/// proxy` gateway (probe scrapes, events and metrics are pooled HTTP requests
/// instead of processes — r3-08-02), the cached transport decision and the
/// long-lived port-forwards (perf K1).
pub async fn run_cycle_with<S: K8sCtx>(
    ctx: &S,
    cluster: &K8sCluster,
    cfg: &MonitorConfig,
    prev: &Snapshot,
    prev_cycle_at: Option<DateTime<Utc>>,
    sink: &dyn MonitorSink,
    state: &mut LoopState,
) -> CycleOutcome {
    let started = Instant::now();
    let now = Utc::now();
    let cid = cluster.id.to_string();
    let mut status = K8sMonitorStatusRow::empty(&cid);
    status.last_cycle_at = Some(ts(now));
    status.snapshot = Value::Null;

    let k = match kubectl_for(ctx, cluster).await {
        Ok(k) => k,
        Err(e) => {
            status.last_error = e.to_string();
            status.cycle_ms = started.elapsed().as_millis() as i64;
            return CycleOutcome {
                status,
                snapshot: None,
                samples_written: 0,
                events_written: 0,
                unreachable: true,
            };
        }
    };
    let namespaces = cfg.effective_namespaces(cluster.default_namespace.as_deref());

    // 1. Sweep. Up to SWEEP_CONCURRENCY namespaces at once (was one at a
    // time); each pod list — tens of MB of JSON on a big namespace — is parsed
    // and reduced to its snapshot on the blocking pool, not a runtime worker.
    let swept: Vec<(String, otto_core::Result<Snapshot>)> =
        stream::iter(namespaces.iter().cloned())
            .map(|ns| {
                let k = &k;
                async move {
                    let res = match k
                        .run(["get", "pods", "-n", ns.as_str(), "-o", "json"])
                        .await
                    {
                        Ok(out) => {
                            parse_off_runtime(out.stdout, |list| {
                                classify::snapshot_from_pod_list(&list)
                            })
                            .await
                        }
                        Err(e) => Err(e),
                    };
                    (ns, res)
                }
            })
            .buffered(SWEEP_CONCURRENCY)
            .collect()
            .await;
    let mut cur = Snapshot::new();
    for (ns, res) in swept {
        match res {
            Ok(snap) => cur.extend(snap),
            Err(e) => {
                status.last_error = format!("list pods in {ns}: {e}");
                status.cycle_ms = started.elapsed().as_millis() as i64;
                return CycleOutcome {
                    status,
                    snapshot: None,
                    samples_written: 0,
                    events_written: 0,
                    unreachable: true,
                };
            }
        }
    }
    status.pods_seen = cur.len() as i64;

    // Status series: only pods whose values changed, plus everyone every
    // STATUS_HEARTBEAT (and on the loop's first cycle) — perf K8.
    let full_status = prev.is_empty()
        || state
            .status_full_at
            .is_none_or(|t| t.elapsed() >= STATUS_HEARTBEAT);
    if full_status {
        state.status_full_at = Some(Instant::now());
    }
    let mut samples_nd = String::new();
    for (key, p) in cur.iter() {
        if !full_status && !status_changed(p, prev.get(key)) {
            continue;
        }
        let container = p.containers.keys().next().cloned().unwrap_or_default();
        samples_nd.push_str(&samples_ndjson(
            &cid,
            now,
            p,
            &container,
            &status_samples(p, now),
            &BTreeMap::new(),
        ));
    }

    // The gateway serves events + metrics (read-only paths) and, unless the
    // config pins port-forward, the proxy transport.
    let gw: Option<&KubeProxy> = gateway::ensure(&mut state.gateway, &k).await;

    // 2. Events: incremental per namespace (same fan-out; parsed off the
    // runtime).
    let listed: Vec<(String, otto_core::Result<NamespaceEvents>)> =
        stream::iter(namespaces.iter().cloned())
            .map(|ns| {
                let k = &k;
                let rv = state.event_rv.get(&ns).cloned();
                async move {
                    let res = namespace_events(k, gw, &ns, rv, prev_cycle_at).await;
                    (ns, res)
                }
            })
            .buffered(SWEEP_CONCURRENCY)
            .collect()
            .await;
    let mut events: Vec<EventHint> = Vec::new();
    let mut event_rv = std::collections::HashMap::new();
    for (ns, res) in listed {
        match res {
            Ok((hints, rv)) => {
                events.extend(hints);
                if let Some(rv) = rv {
                    event_rv.insert(ns, rv);
                }
            }
            Err(e) => tracing::debug!("k8s monitor: events in {ns}: {e}"),
        }
    }

    // 3. Metrics-server (re-probed every cycle, never cached) — unless the
    // config turns it off (RBAC that will never be granted = a wasted call).
    // Namespaces in parallel, parsed off the runtime (perf K5).
    let mut ms_state = if cfg.metrics_server {
        "absent".to_string()
    } else {
        "disabled".to_string()
    };
    let ms_namespaces: &[String] = if cfg.metrics_server { &namespaces } else { &[] };
    let metrics: Vec<(String, otto_core::Result<Vec<resources::PodMetrics>>)> =
        stream::iter(ms_namespaces.iter().cloned())
            .map(|ns| {
                let k = &k;
                async move {
                    let res = namespace_metrics(k, gw, &ns).await;
                    (ns, res)
                }
            })
            .buffered(SWEEP_CONCURRENCY)
            .collect()
            .await;
    // Per pod (snap key): the samples the wide row is built from.
    let mut wide_samples: std::collections::HashMap<String, Vec<Sample>> = Default::default();
    let mut forbidden = None;
    for (ns, res) in metrics {
        match res {
            Ok(pods) => {
                ms_state = "ok".into();
                for pm in pods {
                    let Some(p) = cur.get(&classify::snap_key(&pm.namespace, &pm.name)) else {
                        continue;
                    };
                    // metrics-server reports per CONTAINER; the series
                    // identity has no container, so write the pod total
                    // (container "") — a sidecar no longer turns the pod's
                    // memory into an average of its containers.
                    let smp = metrics_server_samples(
                        pm.containers
                            .iter()
                            .map(|c| (c.cpu_millicores, c.mem_bytes)),
                    );
                    samples_nd.push_str(&samples_ndjson(&cid, now, p, "", &smp, &BTreeMap::new()));
                    wide_samples
                        .entry(classify::snap_key(&p.namespace, &p.name))
                        .or_default()
                        .extend(smp);
                }
            }
            Err(Error::Forbidden(m)) => {
                forbidden.get_or_insert(m);
            }
            Err(e) => {
                tracing::debug!("k8s monitor: metrics-server in {ns}: {e}");
            }
        }
    }
    if let Some(m) = forbidden {
        if ms_state != "ok" {
            ms_state = format!("forbidden: {m}");
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
    let mut scraped_versions: Vec<(String, String)> = Vec::new();
    let mut transport_used = None;
    let target_count = targets.len();
    if !cfg.probes.is_empty() && !targets.is_empty() {
        let probe0 = &cfg.probes[0];
        // The gateway only serves the proxy transport when the config does
        // not pin port-forward.
        let proxy_gw = if cfg.transport == Transport::PortForward {
            None
        } else {
            gw
        };
        let transport = match cached_transport(&state.transport, cfg.transport, Instant::now()) {
            Some(t) => t,
            None => {
                let samples: Vec<ScrapeTarget> = scrape::sniff_sample(&targets)
                    .into_iter()
                    .map(|p| ScrapeTarget {
                        namespace: p.namespace.clone(),
                        pod: p.name.clone(),
                        port: probe0.port.or(p.first_port).unwrap_or(80),
                    })
                    .collect();
                let (t, conclusive) = scrape::pick_transport_multi(
                    &k,
                    proxy_gw,
                    cfg.transport,
                    &samples,
                    &probe0.path,
                )
                .await;
                if conclusive {
                    remember_transport(&mut state.transport, cfg.transport, t, Instant::now());
                }
                t
            }
        };
        transport_used = Some(transport);
        status.transport_used = transport.as_str().into();

        // 5. Scrape with bounded concurrency. The futures are built up
        // front (owned pod copies) so no closure borrows across the
        // `'static` boundary tokio::spawn demands of the enclosing loop.
        let concurrency = cfg.concurrency.clamp(1, probes::MAX_CONCURRENCY) as usize;
        let pool = &state.forwards;
        if transport == TransportUsed::PortForward {
            pool.begin_cycle().await;
        }
        let futs: Vec<_> = targets
            .iter()
            .map(|p| {
                let pod: PodSnap = (*p).clone();
                let (k, cid, cfg) = (&k, cid.as_str(), cfg);
                async move { scrape_pod(k, proxy_gw, pool, cid, transport, cfg, &pod, now).await }
            })
            .collect();
        let keys: Vec<String> = targets
            .iter()
            .map(|p| classify::snap_key(&p.namespace, &p.name))
            .collect();
        let results: Vec<Scraped> = stream::iter(futs).buffered(concurrency).collect().await;
        for (key, r) in keys.into_iter().zip(results) {
            if r.ok {
                status.pods_scraped += 1;
            } else {
                status.pods_failed += 1;
            }
            parse_errors += r.parse_errors;
            capped += r.capped;
            samples_nd.push_str(&r.ndjson);
            wide_samples
                .entry(key.clone())
                .or_default()
                .extend(r.samples);
            scraped_versions.push((key, r.version));
        }
        // Every pod unreachable over the cached transport: decide again next
        // cycle (RBAC or the proxy may have changed).
        if status.pods_scraped == 0 && status.pods_failed > 0 {
            state.transport = None;
        }
    }
    // Port-forwards live only while their pod is still being scraped over
    // port-forward.
    if transport_used == Some(TransportUsed::PortForward) {
        let live: std::collections::HashSet<(&str, &str)> = targets
            .iter()
            .map(|p| (p.namespace.as_str(), p.name.as_str()))
            .collect();
        state
            .forwards
            .retain(|ns, pod, _| live.contains(&(ns, pod)))
            .await;
        // A forward that missed two cycles is idle: kill it (R2).
        let interval = Duration::from_secs(u64::from(cfg.interval_secs.max(probes::MIN_INTERVAL)));
        state.forwards.reap_idle(interval * 2).await;
    } else {
        state.forwards.clear().await;
    }
    state.event_rv = event_rv;
    // Versions: what the probes reported this cycle, else what we knew last
    // cycle for the same pod (so an unscraped pod does not look downgraded).
    for (key, version) in scraped_versions {
        if let Some(p) = cur.get_mut(&key) {
            p.version = version;
        }
    }
    for (key, p) in cur.iter_mut() {
        if p.version.is_empty() {
            if let Some(old) = prev.get(key) {
                p.version = old.version.clone();
            }
        }
    }

    // 6–7. Classify.
    let actions = action_hints(ctx, &cluster.id, now).await;
    let mut classified = classify::classify(prev, &cur, &events, &actions, now);
    classified.extend(classify::version_changes(prev, &cur, now));
    let events_nd = events_ndjson(&cid, now, &classified, &events, &cur);

    // 8. Write. The wide row first needs the counter state: seeded once per
    // cluster from `k8s_latest` after a daemon restart.
    if !wide::with_counters(&cid, |c| c.seeded) {
        let rows = sink
            .query_rows(&wide::seed_sql(&cid))
            .await
            .unwrap_or_default();
        wide::with_counters(&cid, |c| {
            wide::seed_from_rows(c, &rows);
            c.seeded = true;
        });
    }
    let wide_nd = wide::cycle_ndjson(&cid, now, &cur, &wide_samples);
    let samples_written = samples_nd.lines().count();
    let events_written = events_nd.lines().count();
    let mut write_err = None;
    if samples_written > 0 {
        if let Err(e) = sink.insert_ndjson("k8s_samples", &samples_nd).await {
            write_err = Some(format!("write samples: {e}"));
        }
    }
    if !wide_nd.is_empty() {
        if let Err(e) = sink.insert_ndjson(schema::POD_CYCLE_TABLE, &wide_nd).await {
            write_err = Some(format!("write pod rows: {e}"));
        }
    }
    if events_written > 0 {
        if let Err(e) = sink.insert_ndjson("k8s_events", &events_nd).await {
            write_err = Some(format!("write events: {e}"));
        }
    }
    if samples_written + events_written > 0 || !wide_nd.is_empty() {
        // Fleet answers computed before this write are now stale.
        super::cache::bump_generation();
    }

    // 9. Status (the typed snapshot travels in the outcome; the loop decides
    // whether the stored column needs rewriting).
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
            let interval_ms = i64::from(cfg.interval_secs.max(probes::MIN_INTERVAL)) * 1000;
            let pool_cap = state.forwards.cap();
            if transport_used == Some(TransportUsed::PortForward) && target_count > pool_cap {
                notes.push(format!(
                    "port-forward: {target_count} pods rotate through a pool of {pool_cap}"
                ));
            }
            if transport_used == Some(TransportUsed::PortForward) && status.cycle_ms > interval_ms {
                notes.push(format!(
                    "port-forward transport: cycle took {} s, longer than the {} s interval",
                    status.cycle_ms / 1000,
                    interval_ms / 1000
                ));
            }
            status.last_error = notes.join("; ");
        }
    }
    CycleOutcome {
        status,
        snapshot: Some(cur),
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
    // The cluster's long-lived `kubectl proxy`, transport decision and
    // port-forwards, reused across cycles; dropped (children killed, socket
    // dir removed) when the loop ends or is aborted by the supervisor.
    let mut state = LoopState::default();
    // When this loop last considered trimming rows past the retention.
    let mut last_purge: Option<Instant> = None;
    // What this loop last stored: the typed snapshot (the write-on-change
    // baseline and the next cycle's `prev`, so the row isn't re-read and
    // re-parsed every cycle) and that cycle's time. `None` = read the row.
    let mut last: Option<(Arc<Snapshot>, Option<String>)> = None;
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
                sleep_or_cancel(interval, &cancel).await;
                continue;
            }
            match schema::ensure(sink.as_ref(), cfg.retention_days).await {
                Ok(()) => {
                    let _ = sink
                        .exec(&schema::alter_ttl_sql(
                            cfg.retention_days.max(largest_retention(&repo).await),
                        ))
                        .await;
                    schema_ready = true;
                }
                Err(e) => {
                    tracing::warn!(cluster = %cluster_id, "k8s monitor: schema init failed: {e}");
                    sleep_or_cancel(interval, &cancel).await;
                    continue;
                }
            }
        }

        if last.is_none() {
            if let Ok(Some(meta)) = repo.get_status_meta(cluster_id.as_str()).await {
                let snap =
                    super::latest::load(&repo, cluster_id.as_str(), meta.last_cycle_at.as_deref())
                        .await;
                last = Some((snap, meta.last_cycle_at));
            }
        }
        let empty = Snapshot::new();
        let prev: &Snapshot = last.as_ref().map(|(snap, _)| &**snap).unwrap_or(&empty);
        let prev_at = last
            .as_ref()
            .and_then(|(_, at)| at.as_deref())
            .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.with_timezone(&Utc));

        let out = run_cycle_with(
            &ctx,
            &cluster,
            &cfg,
            prev,
            prev_at,
            sink.as_ref(),
            &mut state,
        )
        .await;
        let ok = out.status.last_ok_at.is_some();
        let cur: Option<Arc<Snapshot>> = out.snapshot.map(Arc::new);
        let baseline = last.as_ref().map(|(snap, _)| snap.clone());
        match store_status(&repo, &out.status, cur.as_ref(), baseline.as_ref()).await {
            Ok(_) => {
                // A failed sweep keeps the stored snapshot as the baseline;
                // the cycle time still moves on (as the stored row does).
                let snap = cur.or(baseline).unwrap_or_default();
                super::latest::publish(
                    cluster_id.as_str(),
                    out.status.last_cycle_at.clone(),
                    snap.clone(),
                );
                last = Some((snap, out.status.last_cycle_at.clone()));
            }
            Err(e) => {
                tracing::warn!(cluster = %cluster_id, "k8s monitor: status write failed: {e}");
                // Unknown what's stored now: re-read next cycle.
                last = None;
            }
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

        // Trim clusters that keep less than the table TTL — at most daily,
        // and only when the TTL (the largest enabled retention) is longer than
        // this cluster's retention; otherwise the TTL already drops the rows.
        if ok && last_purge.is_none_or(|t| t.elapsed() >= PURGE_EVERY) {
            last_purge = Some(Instant::now());
            if needs_purge(cfg.retention_days, largest_retention(&repo).await) {
                let cutoff = (Utc::now() - chrono::Duration::days(i64::from(cfg.retention_days)))
                    .format("%Y-%m-%d")
                    .to_string();
                for q in schema::purge_cluster_sql(cluster_id.as_str(), Some(&cutoff)) {
                    if let Err(e) = sink.exec(&q).await {
                        tracing::debug!("k8s monitor purge: {e}");
                    }
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
        sleep_or_cancel(wait, &cancel).await;
    }
}

/// A per-cluster purge is only needed when the cluster keeps fewer days than
/// the shared table TTL.
fn needs_purge(retention_days: u32, table_ttl_days: u32) -> bool {
    retention_days.clamp(1, 90) < table_ttl_days
}

async fn largest_retention(repo: &K8sMonitorRepo) -> u32 {
    repo.list_enabled()
        .await
        .map(|rows| {
            rows.iter()
                .map(|r| r.retention_days.clamp(1, 90) as u32)
                .max()
                .unwrap_or(14)
        })
        .unwrap_or(14)
}

/// One timer for the whole wait (was 1 s slices: a wake-up a second per
/// monitored cluster, r3-08-07). The supervisor both sets `cancel` and aborts
/// the task, so an abort ends the wait at once; the flag is re-checked at the
/// top of the loop for any other canceller.
async fn sleep_or_cancel(total: Duration, cancel: &AtomicBool) {
    if !cancel.load(Ordering::Relaxed) {
        tokio::time::sleep(total).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn status_rewrites_the_snapshot_only_when_it_changed() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query(
            "INSERT INTO k8s_clusters (id, name, source, context_name, environment, created_at, updated_at)
             VALUES ('c1', 'C', 'imported', 'ctx', 'dev', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repo = K8sMonitorRepo::new(pool);
        let mut st = K8sMonitorStatusRow::empty("c1");
        let pod = |phase: &str| PodSnap {
            namespace: "ns".into(),
            name: "p".into(),
            phase: phase.into(),
            ..PodSnap::default()
        };
        let snap = |phase: &str| {
            let mut m = Snapshot::new();
            m.insert("ns/p".into(), pod(phase));
            Arc::new(m)
        };
        let running = snap("Running");
        // First write (no baseline): snapshot stored.
        assert!(store_status(&repo, &st, Some(&running), None)
            .await
            .unwrap());
        // Same snapshot again (a fresh, equal value): the status fields move,
        // the snapshot isn't rewritten.
        st.cycle_ms = 77;
        assert!(
            !store_status(&repo, &st, Some(&snap("Running")), Some(&running))
                .await
                .unwrap()
        );
        assert_eq!(repo.get_status("c1").await.unwrap().unwrap().cycle_ms, 77);
        // A failed cycle (no snapshot) keeps the stored one.
        assert!(!store_status(&repo, &st, None, Some(&running))
            .await
            .unwrap());
        // A changed snapshot is written.
        assert!(
            store_status(&repo, &st, Some(&snap("Failed")), Some(&running))
                .await
                .unwrap()
        );
        assert_eq!(
            repo.get_status("c1").await.unwrap().unwrap().snapshot["ns/p"]["phase"],
            "Failed"
        );
    }

    #[test]
    fn transport_cache_expires_and_follows_the_config() {
        let mut st = LoopState::default();
        let t0 = Instant::now();
        assert_eq!(cached_transport(&st.transport, Transport::Auto, t0), None);
        remember_transport(&mut st.transport, Transport::Auto, TransportUsed::Proxy, t0);
        assert_eq!(
            cached_transport(
                &st.transport,
                Transport::Auto,
                t0 + Duration::from_secs(59 * 60)
            ),
            Some(TransportUsed::Proxy)
        );
        assert_eq!(
            cached_transport(&st.transport, Transport::Auto, t0 + TRANSPORT_TTL),
            None,
            "re-sniffed hourly"
        );
        assert_eq!(
            cached_transport(&st.transport, Transport::PortForward, t0),
            None,
            "a config change re-decides"
        );
    }

    #[test]
    fn status_rows_only_for_changed_pods() {
        let mut a = PodSnap {
            phase: "Running".into(),
            ready: true,
            mem_limit: 100,
            ..PodSnap::default()
        };
        assert!(status_changed(&a, None), "new pod");
        let b = a.clone();
        assert!(!status_changed(&a, Some(&b)));
        a.ready = false;
        assert!(status_changed(&a, Some(&b)));
        a.ready = true;
        a.created = "2026-01-01T00:00:00Z".into();
        assert!(
            !status_changed(&a, Some(&b)),
            "age is derivable, never a change"
        );
    }

    #[test]
    fn event_watch_lines_parse_incrementally() {
        let now = Utc::now();
        let t = now.to_rfc3339();
        let body = format!(
            "{}\n{}\n",
            json!({"type":"ADDED","object":{"reason":"BackOff","lastTimestamp":t,"metadata":{"namespace":"ns","resourceVersion":"41"},"involvedObject":{"kind":"Pod","name":"p"}}}),
            json!({"type":"MODIFIED","object":{"reason":"Pulled","lastTimestamp":t,"metadata":{"namespace":"ns","resourceVersion":"42"}}}),
        );
        let (hints, rv) = parse_event_watch(&body, None).unwrap();
        assert_eq!(hints.len(), 1, "only kept reasons");
        assert_eq!(hints[0].reason, "BackOff");
        assert_eq!(rv.as_deref(), Some("42"));
        assert_eq!(parse_event_watch("", None).unwrap().1, None);
        let expired = json!({"type":"ERROR","object":{"kind":"Status","code":410}}).to_string();
        assert!(parse_event_watch(&expired, None).is_none(), "410 → relist");
    }

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
    fn purge_only_when_retention_is_below_the_table_ttl() {
        assert!(!needs_purge(14, 14), "TTL already trims");
        assert!(!needs_purge(30, 14), "longer than the TTL: nothing to trim");
        assert!(needs_purge(7, 14));
        assert!(needs_purge(0, 14), "floor of one day");
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
    fn metrics_server_containers_sum_into_one_pod_sample() {
        let s = metrics_server_samples([(100, 300), (20, 50)].into_iter());
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].metric.as_str(), s[0].value), ("cpu_millis", 120.0));
        assert_eq!(
            (s[1].metric.as_str(), s[1].value),
            ("mem_working_set_bytes", 350.0)
        );
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
        let lines: Vec<Value> = nd
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
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
        let since = DateTime::parse_from_rfc3339("2026-09-05T07:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let h = parse_event_hints(&list, Some(since));
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].reason, "OOMKilling");
        assert_eq!(h[0].namespace, "ns");
        assert_eq!(parse_event_hints(&list, None).len(), 2);
    }
}
