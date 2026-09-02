//! Row normalisation — `kubectl get … -o json` → the flat `K8sRow` the UI
//! table renders (contract §3.2, §4.2–§4.5).
//!
//! Everything here is pure (JSON in → rows out) so it is unit-tested against
//! fixture files copied from real clusters (`testdata/*.json`). The only I/O is
//! in [`list`], [`detail`], [`nodes`], [`namespaces`], [`containers`] and
//! [`metrics`], which run kubectl through [`crate::cli::Kubectl`] and then call
//! the pure functions. Secrets are the one kind whose payload never leaves the
//! daemon: rows carry `type` + key NAMES only and the detail manifest has every
//! `data` / `stringData` value replaced by `<redacted>`.
//!
//! Metrics come from the metrics-server API (`get --raw /apis/metrics.k8s.io/…`)
//! rather than `kubectl top`'s text table — same data, already JSON, and
//! `Quantity` strings are parsed here (`parse_cpu_millicores` /
//! `parse_mem_bytes`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::cli::Kubectl;

/// Resource kinds the console lists (contract §3.2) plus the two cluster-scoped
/// kinds the detail endpoint accepts (`nodes`, `namespaces`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Pods,
    Deployments,
    Statefulsets,
    Daemonsets,
    Replicasets,
    Jobs,
    Cronjobs,
    Services,
    Ingresses,
    Configmaps,
    Secrets,
    Pvcs,
    Hpas,
    Rollouts,
    Applications,
    Events,
    Nodes,
    Namespaces,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "pods" | "pod" | "po" => Kind::Pods,
            "deployments" | "deployment" | "deploy" => Kind::Deployments,
            "statefulsets" | "statefulset" | "sts" => Kind::Statefulsets,
            "daemonsets" | "daemonset" | "ds" => Kind::Daemonsets,
            "replicasets" | "replicaset" | "rs" => Kind::Replicasets,
            "jobs" | "job" => Kind::Jobs,
            "cronjobs" | "cronjob" | "cj" => Kind::Cronjobs,
            "services" | "service" | "svc" => Kind::Services,
            "ingresses" | "ingress" | "ing" => Kind::Ingresses,
            "configmaps" | "configmap" | "cm" => Kind::Configmaps,
            "secrets" | "secret" => Kind::Secrets,
            "pvcs" | "pvc" | "persistentvolumeclaims" => Kind::Pvcs,
            "hpas" | "hpa" | "horizontalpodautoscalers" => Kind::Hpas,
            "rollouts" | "rollout" | "ro" => Kind::Rollouts,
            "applications" | "application" | "app" | "apps" => Kind::Applications,
            "events" | "event" | "ev" => Kind::Events,
            "nodes" | "node" | "no" => Kind::Nodes,
            "namespaces" | "namespace" | "ns" => Kind::Namespaces,
            _ => return None,
        })
    }

    /// Canonical wire name (what `K8sRow.kind` carries).
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Pods => "pods",
            Kind::Deployments => "deployments",
            Kind::Statefulsets => "statefulsets",
            Kind::Daemonsets => "daemonsets",
            Kind::Replicasets => "replicasets",
            Kind::Jobs => "jobs",
            Kind::Cronjobs => "cronjobs",
            Kind::Services => "services",
            Kind::Ingresses => "ingresses",
            Kind::Configmaps => "configmaps",
            Kind::Secrets => "secrets",
            Kind::Pvcs => "pvcs",
            Kind::Hpas => "hpas",
            Kind::Rollouts => "rollouts",
            Kind::Applications => "applications",
            Kind::Events => "events",
            Kind::Nodes => "nodes",
            Kind::Namespaces => "namespaces",
        }
    }

    /// The resource name handed to kubectl. CRDs are fully qualified so an
    /// unrelated CRD with the same short name can never be picked up.
    pub fn kubectl_resource(&self) -> &'static str {
        match self {
            Kind::Pvcs => "persistentvolumeclaims",
            Kind::Hpas => "horizontalpodautoscalers",
            Kind::Rollouts => "rollouts.argoproj.io",
            Kind::Applications => "applications.argoproj.io",
            other => other.as_str(),
        }
    }

    pub fn namespaced(&self) -> bool {
        !matches!(self, Kind::Nodes | Kind::Namespaces)
    }

    /// The Kubernetes `kind` string (`involvedObject.kind`, Argo `resources[].kind`).
    pub fn k8s_kind(&self) -> &'static str {
        match self {
            Kind::Pods => "Pod",
            Kind::Deployments => "Deployment",
            Kind::Statefulsets => "StatefulSet",
            Kind::Daemonsets => "DaemonSet",
            Kind::Replicasets => "ReplicaSet",
            Kind::Jobs => "Job",
            Kind::Cronjobs => "CronJob",
            Kind::Services => "Service",
            Kind::Ingresses => "Ingress",
            Kind::Configmaps => "ConfigMap",
            Kind::Secrets => "Secret",
            Kind::Pvcs => "PersistentVolumeClaim",
            Kind::Hpas => "HorizontalPodAutoscaler",
            Kind::Rollouts => "Rollout",
            Kind::Applications => "Application",
            Kind::Events => "Event",
            Kind::Nodes => "Node",
            Kind::Namespaces => "Namespace",
        }
    }

    /// Reverse of [`Kind::k8s_kind`] (Argo `status.resources[].kind` → Kind).
    pub fn from_k8s_kind(s: &str) -> Option<Self> {
        match s {
            "Deployment" => Some(Kind::Deployments),
            "StatefulSet" => Some(Kind::Statefulsets),
            "DaemonSet" => Some(Kind::Daemonsets),
            "Rollout" => Some(Kind::Rollouts),
            "Pod" => Some(Kind::Pods),
            "CronJob" => Some(Kind::Cronjobs),
            _ => None,
        }
    }
}

/// Health colouring the table uses (§4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    Ok,
    Warn,
    Bad,
    Progressing,
}

/// One table row (contract §3.2 `K8sRow`). `cpu` is millicores, `mem` bytes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sRow {
    pub name: String,
    pub namespace: String,
    pub kind: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restarts: Option<i64>,
    pub age_seconds: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    pub labels: BTreeMap<String, String>,
    pub extra: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Health>,
}

/// `GET /k8s/clusters/{id}/nodes` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRow {
    pub name: String,
    pub status: String,
    pub roles: String,
    pub version: String,
    /// Millicores.
    pub cpu_capacity: i64,
    /// Bytes.
    pub mem_capacity: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_usage: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_usage: Option<i64>,
    pub age_seconds: i64,
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamespaceRow {
    pub name: String,
    pub status: String,
    pub age_seconds: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerRow {
    pub name: String,
    pub image: String,
    pub ready: bool,
    pub state: String,
    pub restarts: i64,
    pub init: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRow {
    #[serde(rename = "type")]
    pub kind: String,
    pub reason: String,
    pub message: String,
    pub count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerMetrics {
    pub name: String,
    pub cpu_millicores: i64,
    pub mem_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodMetrics {
    pub name: String,
    pub namespace: String,
    pub cpu_millicores: i64,
    pub mem_bytes: i64,
    pub containers: Vec<ContainerMetrics>,
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn s<'a>(v: &'a Value, ptr: &str) -> Option<&'a str> {
    v.pointer(ptr).and_then(Value::as_str)
}

fn i(v: &Value, ptr: &str) -> Option<i64> {
    v.pointer(ptr)
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
}

fn b(v: &Value, ptr: &str) -> Option<bool> {
    v.pointer(ptr).and_then(Value::as_bool)
}

fn arr<'a>(v: &'a Value, ptr: &str) -> &'a [Value] {
    v.pointer(ptr)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn string_map(v: Option<&Value>) -> BTreeMap<String, String> {
    v.and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn age_seconds(v: &Value, ptr: &str, now: DateTime<Utc>) -> i64 {
    s(v, ptr)
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| (now - t.with_timezone(&Utc)).num_seconds().max(0))
        .unwrap_or(0)
}

fn images(v: &Value) -> Option<Vec<String>> {
    let mut out: Vec<String> = arr(v, "/spec/containers")
        .iter()
        .chain(arr(v, "/spec/template/spec/containers"))
        .chain(arr(v, "/spec/jobTemplate/spec/template/spec/containers"))
        .filter_map(|c| s(c, "/image").map(str::to_string))
        .collect();
    out.dedup();
    (!out.is_empty()).then_some(out)
}

fn base_row(kind: Kind, item: &Value, now: DateTime<Utc>) -> K8sRow {
    K8sRow {
        name: s(item, "/metadata/name").unwrap_or("").to_string(),
        namespace: s(item, "/metadata/namespace").unwrap_or("").to_string(),
        kind: kind.as_str().to_string(),
        status: String::new(),
        ready: None,
        restarts: None,
        age_seconds: age_seconds(item, "/metadata/creationTimestamp", now),
        node: None,
        ip: None,
        cpu: None,
        mem: None,
        images: None,
        labels: string_map(item.pointer("/metadata/labels")),
        extra: BTreeMap::new(),
        health: None,
    }
}

fn put(extra: &mut BTreeMap<String, String>, k: &str, v: impl ToString) {
    extra.insert(k.to_string(), v.to_string());
}

fn put_opt(extra: &mut BTreeMap<String, String>, k: &str, v: Option<impl ToString>) {
    if let Some(v) = v {
        extra.insert(k.to_string(), v.to_string());
    }
}

// ---------------------------------------------------------------------------
// Quantities
// ---------------------------------------------------------------------------

/// Parse a Kubernetes CPU quantity (`250m`, `1`, `1.5`, `1500u`, `12345678n`)
/// into millicores (rounded to nearest).
pub fn parse_cpu_millicores(q: &str) -> Option<i64> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    let (num, suffix) = split_quantity(q);
    let n: f64 = num.parse().ok()?;
    let millis = match suffix {
        "" => n * 1000.0,
        "m" => n,
        "u" => n / 1_000.0,
        "n" => n / 1_000_000.0,
        "k" => n * 1_000_000.0,
        _ => return None,
    };
    Some(millis.round() as i64)
}

/// Parse a Kubernetes memory quantity (`128Mi`, `1Gi`, `20480Ki`, `1000000`,
/// `1G`, `1e3`) into bytes.
pub fn parse_mem_bytes(q: &str) -> Option<i64> {
    let q = q.trim();
    if q.is_empty() {
        return None;
    }
    let (num, suffix) = split_quantity(q);
    let n: f64 = num.parse().ok()?;
    let mult: f64 = match suffix {
        "" => 1.0,
        "Ki" => 1024.0,
        "Mi" => 1024f64.powi(2),
        "Gi" => 1024f64.powi(3),
        "Ti" => 1024f64.powi(4),
        "Pi" => 1024f64.powi(5),
        "Ei" => 1024f64.powi(6),
        "k" => 1e3,
        "M" => 1e6,
        "G" => 1e9,
        "T" => 1e12,
        "P" => 1e15,
        "E" => 1e18,
        "m" => 1e-3,
        _ => return None,
    };
    Some((n * mult).round() as i64)
}

/// Split `12.5Gi` → (`12.5`, `Gi`); exponent forms (`1e3`) keep the exponent in
/// the numeric part.
fn split_quantity(q: &str) -> (&str, &str) {
    let bytes = q.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        let c = bytes[end] as char;
        let numeric = c.is_ascii_digit() || c == '.' || c == '-' || c == '+';
        let exponent = (c == 'e' || c == 'E')
            && end + 1 < bytes.len()
            && (bytes[end + 1].is_ascii_digit()
                || bytes[end + 1] == b'-'
                || bytes[end + 1] == b'+')
            && bytes[end + 1..]
                .iter()
                .all(|b| b.is_ascii_digit() || *b == b'-' || *b == b'+');
        if !(numeric || exponent) {
            break;
        }
        end += 1;
    }
    (&q[..end], &q[end..])
}

// ---------------------------------------------------------------------------
// Per-kind normalisation
// ---------------------------------------------------------------------------

/// Normalise one `kubectl get -o json` item of `kind` into a row.
pub fn normalize(kind: Kind, item: &Value, now: DateTime<Utc>) -> K8sRow {
    let mut row = base_row(kind, item, now);
    match kind {
        Kind::Pods => pod(&mut row, item),
        Kind::Deployments => deployment(&mut row, item),
        Kind::Statefulsets => statefulset(&mut row, item),
        Kind::Daemonsets => daemonset(&mut row, item),
        Kind::Replicasets => replicaset(&mut row, item),
        Kind::Jobs => job(&mut row, item),
        Kind::Cronjobs => cronjob(&mut row, item),
        Kind::Services => service(&mut row, item),
        Kind::Ingresses => ingress(&mut row, item),
        Kind::Configmaps => configmap(&mut row, item),
        Kind::Secrets => secret(&mut row, item),
        Kind::Pvcs => pvc(&mut row, item),
        Kind::Hpas => hpa(&mut row, item),
        Kind::Rollouts => rollout(&mut row, item),
        Kind::Applications => application(&mut row, item),
        Kind::Events => event(&mut row, item, now),
        Kind::Nodes => {
            let n = node(item, now);
            row.status = n.status;
            row.health = Some(if row.status.starts_with("Ready") {
                Health::Ok
            } else {
                Health::Bad
            });
            put(&mut row.extra, "roles", n.roles);
            put(&mut row.extra, "version", n.version);
        }
        Kind::Namespaces => {
            row.status = s(item, "/status/phase").unwrap_or("").to_string();
            row.health = Some(if row.status == "Active" {
                Health::Ok
            } else {
                Health::Warn
            });
        }
    }
    row
}

/// Waiting/terminated reasons that mean "this pod is broken" (§4.2 `bad`).
const BAD_REASONS: &[&str] = &[
    "CrashLoopBackOff",
    "ImagePullBackOff",
    "ErrImagePull",
    "InvalidImageName",
    "CreateContainerConfigError",
    "CreateContainerError",
    "RunContainerError",
    "Error",
    "OOMKilled",
    "Evicted",
    "Failed",
    "DeadlineExceeded",
];

fn container_reason(status: &Value) -> Option<String> {
    s(status, "/state/waiting/reason")
        .or_else(|| s(status, "/state/terminated/reason"))
        .map(str::to_string)
}

fn pod(row: &mut K8sRow, item: &Value) {
    let phase = s(item, "/status/phase").unwrap_or("Unknown").to_string();
    let statuses = arr(item, "/status/containerStatuses");
    let init_statuses = arr(item, "/status/initContainerStatuses");
    let total = arr(item, "/spec/containers").len().max(statuses.len());
    let ready = statuses
        .iter()
        .filter(|c| b(c, "/ready") == Some(true))
        .count();
    let restarts: i64 = statuses
        .iter()
        .chain(init_statuses)
        .filter_map(|c| i(c, "/restartCount"))
        .sum();

    // Reason precedence: bad container reason > init progress > waiting reason > phase.
    let bad = statuses
        .iter()
        .filter_map(container_reason)
        .find(|r| BAD_REASONS.contains(&r.as_str()));
    let init_bad = init_statuses
        .iter()
        .filter_map(container_reason)
        .find(|r| BAD_REASONS.contains(&r.as_str()));
    let init_pending = init_statuses
        .iter()
        .any(|c| c.pointer("/state/terminated").is_none());
    let waiting = statuses
        .iter()
        .filter_map(|c| s(c, "/state/waiting/reason"))
        .next();

    let mut status = if item.pointer("/metadata/deletionTimestamp").is_some() {
        "Terminating".to_string()
    } else if let Some(r) = bad {
        r
    } else if let Some(r) = init_bad {
        format!("Init:{r}")
    } else if init_pending && phase == "Pending" {
        let done = init_statuses
            .iter()
            .filter(|c| c.pointer("/state/terminated").is_some())
            .count();
        format!("Init:{done}/{}", init_statuses.len())
    } else if let Some(r) = waiting.filter(|_| phase == "Pending" || phase == "Running") {
        r.to_string()
    } else if phase == "Succeeded" {
        "Completed".to_string()
    } else {
        phase.clone()
    };
    if phase == "Failed" {
        if let Some(r) = s(item, "/status/reason") {
            status = r.to_string();
        }
    }

    let all_ready = total > 0 && ready == total;
    let health = if status == "Completed" || (status == "Running" && all_ready) {
        Health::Ok
    } else if BAD_REASONS.contains(&status.as_str())
        || status.starts_with("Init:") && BAD_REASONS.iter().any(|r| status.ends_with(r))
        || phase == "Failed"
    {
        Health::Bad
    } else if status == "Pending"
        || status == "ContainerCreating"
        || status == "PodInitializing"
        || status.starts_with("Init:")
    {
        Health::Progressing
    } else {
        Health::Warn
    };

    row.status = status;
    row.ready = Some(format!("{ready}/{total}"));
    row.restarts = Some(restarts);
    row.node = s(item, "/spec/nodeName").map(str::to_string);
    row.ip = s(item, "/status/podIP").map(str::to_string);
    row.images = images(item);
    row.health = Some(health);
    put(&mut row.extra, "phase", phase);
    put_opt(&mut row.extra, "qos", s(item, "/status/qosClass"));
    put(&mut row.extra, "containers", total);
    if let Some(r) = statuses
        .iter()
        .filter_map(|c| s(c, "/state/waiting/message"))
        .next()
    {
        put(&mut row.extra, "message", r);
    }
}

/// Shared readiness math for replica-managed workloads (§4.3).
fn workload(
    row: &mut K8sRow,
    desired: i64,
    updated: i64,
    ready: i64,
    available: i64,
    paused: bool,
) {
    row.ready = Some(format!("{ready}/{desired}"));
    put(&mut row.extra, "desired", desired);
    put(&mut row.extra, "updated", updated);
    put(&mut row.extra, "ready", ready);
    put(&mut row.extra, "available", available);
    if paused {
        put(&mut row.extra, "paused", "true");
    }
    let (status, health) = if desired == 0 {
        ("ScaledDown", Health::Warn)
    } else if paused {
        ("Paused", Health::Warn)
    } else if available >= desired && updated >= desired && ready >= desired {
        ("Available", Health::Ok)
    } else if available == 0 {
        ("Unavailable", Health::Bad)
    } else if updated < desired {
        ("Updating", Health::Progressing)
    } else {
        ("Progressing", Health::Progressing)
    };
    row.status = status.to_string();
    row.health = Some(health);
}

fn deployment(row: &mut K8sRow, item: &Value) {
    let desired = i(item, "/spec/replicas").unwrap_or(1);
    workload(
        row,
        desired,
        i(item, "/status/updatedReplicas").unwrap_or(0),
        i(item, "/status/readyReplicas").unwrap_or(0),
        i(item, "/status/availableReplicas").unwrap_or(0),
        b(item, "/spec/paused").unwrap_or(false),
    );
    // A Progressing=False condition (ProgressDeadlineExceeded) is a hard failure
    // even when some replicas are still available.
    let stalled = arr(item, "/status/conditions")
        .iter()
        .any(|c| s(c, "/type") == Some("Progressing") && s(c, "/status") == Some("False"));
    if stalled && desired > 0 {
        row.status = "Failed".into();
        row.health = Some(Health::Bad);
        if let Some(r) = arr(item, "/status/conditions")
            .iter()
            .find(|c| s(c, "/type") == Some("Progressing"))
            .and_then(|c| s(c, "/reason"))
        {
            put(&mut row.extra, "reason", r);
        }
    }
    row.images = images(item);
}

fn statefulset(row: &mut K8sRow, item: &Value) {
    let desired = i(item, "/spec/replicas").unwrap_or(1);
    let ready = i(item, "/status/readyReplicas").unwrap_or(0);
    workload(
        row,
        desired,
        i(item, "/status/updatedReplicas").unwrap_or(0),
        ready,
        i(item, "/status/availableReplicas").unwrap_or(ready),
        false,
    );
    row.images = images(item);
}

fn daemonset(row: &mut K8sRow, item: &Value) {
    workload(
        row,
        i(item, "/status/desiredNumberScheduled").unwrap_or(0),
        i(item, "/status/updatedNumberScheduled").unwrap_or(0),
        i(item, "/status/numberReady").unwrap_or(0),
        i(item, "/status/numberAvailable").unwrap_or(0),
        false,
    );
    if row.status == "ScaledDown" {
        row.status = "NoNodes".into();
    }
    put_opt(
        &mut row.extra,
        "misscheduled",
        i(item, "/status/numberMisscheduled"),
    );
    row.images = images(item);
}

fn replicaset(row: &mut K8sRow, item: &Value) {
    let desired = i(item, "/spec/replicas").unwrap_or(0);
    let ready = i(item, "/status/readyReplicas").unwrap_or(0);
    workload(
        row,
        desired,
        i(item, "/status/replicas").unwrap_or(0),
        ready,
        i(item, "/status/availableReplicas").unwrap_or(0),
        false,
    );
    if desired == 0 {
        // Old ReplicaSets kept for rollback history are not a warning.
        row.status = "Inactive".into();
        row.health = Some(Health::Ok);
    }
    row.images = images(item);
}

fn job(row: &mut K8sRow, item: &Value) {
    let completions = i(item, "/spec/completions").unwrap_or(1);
    let succeeded = i(item, "/status/succeeded").unwrap_or(0);
    let failed = i(item, "/status/failed").unwrap_or(0);
    let active = i(item, "/status/active").unwrap_or(0);
    let cond = |t: &str| {
        arr(item, "/status/conditions")
            .iter()
            .any(|c| s(c, "/type") == Some(t) && s(c, "/status") == Some("True"))
    };
    let (status, health) = if cond("Complete") {
        ("Complete", Health::Ok)
    } else if cond("Failed") {
        ("Failed", Health::Bad)
    } else if active > 0 {
        ("Running", Health::Progressing)
    } else if b(item, "/spec/suspend") == Some(true) {
        ("Suspended", Health::Warn)
    } else {
        ("Pending", Health::Progressing)
    };
    row.status = status.into();
    row.health = Some(health);
    row.ready = Some(format!("{succeeded}/{completions}"));
    put(
        &mut row.extra,
        "completions",
        format!("{succeeded}/{completions}"),
    );
    put(&mut row.extra, "active", active);
    put(&mut row.extra, "failed", failed);
    if let (Some(start), Some(end)) = (
        s(item, "/status/startTime").and_then(|t| DateTime::parse_from_rfc3339(t).ok()),
        s(item, "/status/completionTime").and_then(|t| DateTime::parse_from_rfc3339(t).ok()),
    ) {
        put(
            &mut row.extra,
            "duration_seconds",
            (end - start).num_seconds(),
        );
    }
    row.images = images(item);
}

fn cronjob(row: &mut K8sRow, item: &Value) {
    let suspended = b(item, "/spec/suspend").unwrap_or(false);
    let active = arr(item, "/status/active").len();
    row.status = if suspended {
        "Suspended".into()
    } else if active > 0 {
        "Active".into()
    } else {
        "Scheduled".into()
    };
    row.health = Some(if suspended { Health::Warn } else { Health::Ok });
    put_opt(&mut row.extra, "schedule", s(item, "/spec/schedule"));
    put(&mut row.extra, "suspend", suspended);
    put(&mut row.extra, "active", active);
    put_opt(
        &mut row.extra,
        "last_schedule",
        s(item, "/status/lastScheduleTime"),
    );
    put_opt(
        &mut row.extra,
        "last_successful",
        s(item, "/status/lastSuccessfulTime"),
    );
    row.images = images(item);
}

fn service(row: &mut K8sRow, item: &Value) {
    let ty = s(item, "/spec/type").unwrap_or("ClusterIP").to_string();
    let ports: Vec<String> = arr(item, "/spec/ports")
        .iter()
        .map(|p| {
            let port = i(p, "/port").unwrap_or(0);
            let proto = s(p, "/protocol").unwrap_or("TCP");
            match i(p, "/nodePort") {
                Some(np) => format!("{port}:{np}/{proto}"),
                None => format!("{port}/{proto}"),
            }
        })
        .collect();
    let external: Vec<String> = arr(item, "/status/loadBalancer/ingress")
        .iter()
        .filter_map(|e| s(e, "/ip").or_else(|| s(e, "/hostname")))
        .map(str::to_string)
        .chain(
            arr(item, "/spec/externalIPs")
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string),
        )
        .collect();
    row.status = ty.clone();
    row.ip = s(item, "/spec/clusterIP").map(str::to_string);
    row.health = Some(if ty == "LoadBalancer" && external.is_empty() {
        Health::Progressing
    } else {
        Health::Ok
    });
    put(&mut row.extra, "type", ty);
    put(&mut row.extra, "ports", ports.join(","));
    if !external.is_empty() {
        put(&mut row.extra, "external_ip", external.join(","));
    }
    let selector = string_map(item.pointer("/spec/selector"));
    if !selector.is_empty() {
        let sel: Vec<String> = selector.iter().map(|(k, v)| format!("{k}={v}")).collect();
        put(&mut row.extra, "selector", sel.join(","));
    }
}

fn ingress(row: &mut K8sRow, item: &Value) {
    let hosts: Vec<&str> = arr(item, "/spec/rules")
        .iter()
        .filter_map(|r| s(r, "/host"))
        .collect();
    let address: Vec<&str> = arr(item, "/status/loadBalancer/ingress")
        .iter()
        .filter_map(|e| s(e, "/ip").or_else(|| s(e, "/hostname")))
        .collect();
    row.status = if address.is_empty() {
        "Pending".into()
    } else {
        "Ready".into()
    };
    row.health = Some(if address.is_empty() {
        Health::Progressing
    } else {
        Health::Ok
    });
    put_opt(&mut row.extra, "class", s(item, "/spec/ingressClassName"));
    put(
        &mut row.extra,
        "hosts",
        if hosts.is_empty() {
            "*".to_string()
        } else {
            hosts.join(",")
        },
    );
    put(&mut row.extra, "address", address.join(","));
    put(&mut row.extra, "tls", !arr(item, "/spec/tls").is_empty());
}

fn key_names(item: &Value) -> Vec<String> {
    let mut keys: Vec<String> = item
        .pointer("/data")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    if let Some(bd) = item.pointer("/binaryData").and_then(Value::as_object) {
        keys.extend(bd.keys().cloned());
    }
    keys.sort();
    keys
}

fn configmap(row: &mut K8sRow, item: &Value) {
    let keys = key_names(item);
    row.status = format!("{} keys", keys.len());
    row.health = Some(Health::Ok);
    put(&mut row.extra, "keys", keys.join(","));
    put(&mut row.extra, "key_count", keys.len());
}

/// Secrets: `type` + key NAMES only — values never leave the daemon.
fn secret(row: &mut K8sRow, item: &Value) {
    let keys = key_names(item);
    row.status = s(item, "/type").unwrap_or("Opaque").to_string();
    row.health = Some(Health::Ok);
    put(&mut row.extra, "type", &row.status);
    put(&mut row.extra, "keys", keys.join(","));
    put(&mut row.extra, "key_count", keys.len());
}

fn pvc(row: &mut K8sRow, item: &Value) {
    let phase = s(item, "/status/phase").unwrap_or("Unknown").to_string();
    row.health = Some(match phase.as_str() {
        "Bound" => Health::Ok,
        "Pending" => Health::Progressing,
        "Lost" => Health::Bad,
        _ => Health::Warn,
    });
    row.status = phase;
    put_opt(
        &mut row.extra,
        "storage_class",
        s(item, "/spec/storageClassName"),
    );
    put_opt(&mut row.extra, "volume", s(item, "/spec/volumeName"));
    put_opt(
        &mut row.extra,
        "capacity",
        s(item, "/status/capacity/storage").or_else(|| s(item, "/spec/resources/requests/storage")),
    );
    let modes: Vec<&str> = arr(item, "/spec/accessModes")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    put(&mut row.extra, "access_modes", modes.join(","));
}

fn hpa(row: &mut K8sRow, item: &Value) {
    let min = i(item, "/spec/minReplicas").unwrap_or(1);
    let max = i(item, "/spec/maxReplicas").unwrap_or(0);
    let current = i(item, "/status/currentReplicas").unwrap_or(0);
    let desired = i(item, "/status/desiredReplicas").unwrap_or(current);
    let reference = format!(
        "{}/{}",
        s(item, "/spec/scaleTargetRef/kind").unwrap_or(""),
        s(item, "/spec/scaleTargetRef/name").unwrap_or("")
    );
    // Targets: "cpu: 45%/80%" pairs (current from status.currentMetrics, target
    // from spec.metrics), autoscaling/v2 shapes; v1 falls back to the CPU fields.
    let mut targets: Vec<String> = Vec::new();
    let currents: Vec<&Value> = arr(item, "/status/currentMetrics").iter().collect();
    for (idx, m) in arr(item, "/spec/metrics").iter().enumerate() {
        let name = s(m, "/resource/name")
            .or_else(|| s(m, "/pods/metric/name"))
            .or_else(|| s(m, "/external/metric/name"))
            .or_else(|| s(m, "/object/metric/name"))
            .unwrap_or("metric");
        let target = i(m, "/resource/target/averageUtilization")
            .map(|t| format!("{t}%"))
            .or_else(|| s(m, "/resource/target/averageValue").map(str::to_string))
            .or_else(|| s(m, "/pods/target/averageValue").map(str::to_string))
            .or_else(|| s(m, "/external/target/averageValue").map(str::to_string))
            .or_else(|| s(m, "/external/target/value").map(str::to_string))
            .or_else(|| s(m, "/object/target/value").map(str::to_string))
            .unwrap_or_else(|| "?".into());
        let cur = currents
            .get(idx)
            .and_then(|c| {
                i(c, "/resource/current/averageUtilization")
                    .map(|t| format!("{t}%"))
                    .or_else(|| s(c, "/resource/current/averageValue").map(str::to_string))
                    .or_else(|| s(c, "/pods/current/averageValue").map(str::to_string))
                    .or_else(|| s(c, "/external/current/averageValue").map(str::to_string))
                    .or_else(|| s(c, "/external/current/value").map(str::to_string))
                    .or_else(|| s(c, "/object/current/value").map(str::to_string))
            })
            .unwrap_or_else(|| "<unknown>".into());
        targets.push(format!("{name}: {cur}/{target}"));
    }
    if targets.is_empty() {
        if let Some(t) = i(item, "/spec/targetCPUUtilizationPercentage") {
            let cur = i(item, "/status/currentCPUUtilizationPercentage")
                .map(|c| format!("{c}%"))
                .unwrap_or_else(|| "<unknown>".into());
            targets.push(format!("cpu: {cur}/{t}%"));
        }
    }
    row.status = format!("{current}/{max}");
    row.ready = Some(format!("{current}/{max}"));
    row.health = Some(if max > 0 && current >= max {
        Health::Warn
    } else if desired != current {
        Health::Progressing
    } else {
        Health::Ok
    });
    put(&mut row.extra, "reference", reference);
    put(&mut row.extra, "min", min);
    put(&mut row.extra, "max", max);
    put(&mut row.extra, "replicas", current);
    put(&mut row.extra, "desired", desired);
    put(&mut row.extra, "targets", targets.join(", "));
}

/// §4.4 Argo Rollouts.
fn rollout(row: &mut K8sRow, item: &Value) {
    let desired = i(item, "/spec/replicas").unwrap_or(1);
    let ready = i(item, "/status/readyReplicas").unwrap_or(0);
    let updated = i(item, "/status/updatedReplicas").unwrap_or(0);
    let available = i(item, "/status/availableReplicas").unwrap_or(0);
    let paused = b(item, "/spec/paused").unwrap_or(false);
    let phase = s(item, "/status/phase").unwrap_or("").to_string();
    let strategy = if item.pointer("/spec/strategy/canary").is_some() {
        "canary"
    } else if item.pointer("/spec/strategy/blueGreen").is_some() {
        "blueGreen"
    } else {
        "unknown"
    };
    let steps = arr(item, "/spec/strategy/canary/steps").len();
    row.ready = Some(format!("{ready}/{desired}"));
    row.status = if phase.is_empty() {
        if paused {
            "Paused".into()
        } else {
            "Unknown".into()
        }
    } else {
        phase.clone()
    };
    row.health = Some(match row.status.as_str() {
        "Healthy" => Health::Ok,
        "Degraded" => Health::Bad,
        "Paused" => Health::Warn,
        "Progressing" => Health::Progressing,
        _ => Health::Warn,
    });
    put(&mut row.extra, "strategy", strategy);
    put(&mut row.extra, "phase", phase);
    if strategy == "canary" {
        let idx = i(item, "/status/currentStepIndex").unwrap_or(0);
        put(&mut row.extra, "step", format!("{idx}/{steps}"));
    }
    put_opt(
        &mut row.extra,
        "weight",
        i(item, "/status/canary/weights/canary/weight"),
    );
    put(&mut row.extra, "paused", paused);
    put_opt(&mut row.extra, "message", s(item, "/status/message"));
    put(&mut row.extra, "desired", desired);
    put(&mut row.extra, "updated", updated);
    put(&mut row.extra, "ready", ready);
    put(&mut row.extra, "available", available);
    row.images = images(item);
}

/// §4.5 ArgoCD Applications.
fn application(row: &mut K8sRow, item: &Value) {
    let sync = s(item, "/status/sync/status")
        .unwrap_or("Unknown")
        .to_string();
    let health = s(item, "/status/health/status")
        .unwrap_or("Unknown")
        .to_string();
    let operation = s(item, "/status/operationState/phase").map(str::to_string);
    let revision = s(item, "/status/sync/revision")
        .map(|r| r.chars().take(8).collect::<String>())
        .unwrap_or_default();
    row.status = health.clone();
    row.health = Some(
        match (health.as_str(), sync.as_str(), operation.as_deref()) {
            (_, _, Some("Running")) => Health::Progressing,
            ("Healthy", "Synced", _) => Health::Ok,
            ("Degraded", _, _) | ("Missing", _, _) => Health::Bad,
            ("Progressing", _, _) => Health::Progressing,
            _ => Health::Warn,
        },
    );
    put(&mut row.extra, "sync", sync);
    put(&mut row.extra, "health", health);
    put(&mut row.extra, "revision", revision);
    put_opt(&mut row.extra, "repo", s(item, "/spec/source/repoURL"));
    put_opt(&mut row.extra, "path", s(item, "/spec/source/path"));
    put_opt(
        &mut row.extra,
        "target_revision",
        s(item, "/spec/source/targetRevision"),
    );
    put_opt(
        &mut row.extra,
        "dest_ns",
        s(item, "/spec/destination/namespace"),
    );
    put_opt(&mut row.extra, "project", s(item, "/spec/project"));
    put_opt(&mut row.extra, "operation", operation);
}

fn event(row: &mut K8sRow, item: &Value, now: DateTime<Utc>) {
    let ty = s(item, "/type").unwrap_or("Normal").to_string();
    let last = s(item, "/lastTimestamp")
        .or_else(|| s(item, "/eventTime"))
        .or_else(|| s(item, "/series/lastObservedTime"))
        .or_else(|| s(item, "/metadata/creationTimestamp"));
    if let Some(t) = last.and_then(|t| DateTime::parse_from_rfc3339(t).ok()) {
        row.age_seconds = (now - t.with_timezone(&Utc)).num_seconds().max(0);
    }
    row.status = ty.clone();
    row.health = Some(if ty == "Warning" {
        Health::Warn
    } else {
        Health::Ok
    });
    put_opt(&mut row.extra, "reason", s(item, "/reason"));
    put_opt(&mut row.extra, "message", s(item, "/message"));
    put(
        &mut row.extra,
        "object",
        format!(
            "{}/{}",
            s(item, "/involvedObject/kind").unwrap_or(""),
            s(item, "/involvedObject/name").unwrap_or("")
        ),
    );
    put(
        &mut row.extra,
        "count",
        i(item, "/count")
            .or_else(|| i(item, "/series/count"))
            .unwrap_or(1),
    );
    put_opt(&mut row.extra, "last_seen", last);
    put_opt(
        &mut row.extra,
        "source",
        s(item, "/source/component").or_else(|| s(item, "/reportingComponent")),
    );
}

/// Node row from a `Node` item (usage merged later by [`merge_node_metrics`]).
pub fn node(item: &Value, now: DateTime<Utc>) -> NodeRow {
    let ready = arr(item, "/status/conditions")
        .iter()
        .any(|c| s(c, "/type") == Some("Ready") && s(c, "/status") == Some("True"));
    let mut status = if ready {
        "Ready".to_string()
    } else {
        "NotReady".to_string()
    };
    if b(item, "/spec/unschedulable") == Some(true) {
        status.push_str(",SchedulingDisabled");
    }
    let labels = string_map(item.pointer("/metadata/labels"));
    let mut roles: Vec<String> = labels
        .keys()
        .filter_map(|k| k.strip_prefix("node-role.kubernetes.io/"))
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .collect();
    if let Some(r) = labels.get("kubernetes.io/role") {
        if !roles.contains(r) {
            roles.push(r.clone());
        }
    }
    roles.sort();
    NodeRow {
        name: s(item, "/metadata/name").unwrap_or("").to_string(),
        status,
        roles: if roles.is_empty() {
            "<none>".into()
        } else {
            roles.join(",")
        },
        version: s(item, "/status/nodeInfo/kubeletVersion")
            .unwrap_or("")
            .to_string(),
        cpu_capacity: s(item, "/status/capacity/cpu")
            .and_then(parse_cpu_millicores)
            .unwrap_or(0),
        mem_capacity: s(item, "/status/capacity/memory")
            .and_then(parse_mem_bytes)
            .unwrap_or(0),
        cpu_usage: None,
        mem_usage: None,
        age_seconds: age_seconds(item, "/metadata/creationTimestamp", now),
        labels,
    }
}

/// Parse a `PodMetricsList` (metrics.k8s.io) into per-pod totals.
pub fn parse_pod_metrics(list: &Value) -> Vec<PodMetrics> {
    arr(list, "/items")
        .iter()
        .map(|m| {
            let containers: Vec<ContainerMetrics> = arr(m, "/containers")
                .iter()
                .map(|c| ContainerMetrics {
                    name: s(c, "/name").unwrap_or("").to_string(),
                    cpu_millicores: s(c, "/usage/cpu")
                        .and_then(parse_cpu_millicores)
                        .unwrap_or(0),
                    mem_bytes: s(c, "/usage/memory").and_then(parse_mem_bytes).unwrap_or(0),
                })
                .collect();
            PodMetrics {
                name: s(m, "/metadata/name").unwrap_or("").to_string(),
                namespace: s(m, "/metadata/namespace").unwrap_or("").to_string(),
                cpu_millicores: containers.iter().map(|c| c.cpu_millicores).sum(),
                mem_bytes: containers.iter().map(|c| c.mem_bytes).sum(),
                containers,
            }
        })
        .collect()
}

/// Fill `cpu` / `mem` on pod rows from a metrics list (matched by ns + name).
pub fn merge_pod_metrics(rows: &mut [K8sRow], metrics: &[PodMetrics]) {
    for row in rows.iter_mut() {
        if let Some(m) = metrics
            .iter()
            .find(|m| m.name == row.name && m.namespace == row.namespace)
        {
            row.cpu = Some(m.cpu_millicores);
            row.mem = Some(m.mem_bytes);
        }
    }
}

/// Fill node usage from a `NodeMetricsList`.
pub fn merge_node_metrics(rows: &mut [NodeRow], list: &Value) {
    for m in arr(list, "/items") {
        let name = s(m, "/metadata/name").unwrap_or("");
        if let Some(row) = rows.iter_mut().find(|r| r.name == name) {
            row.cpu_usage = s(m, "/usage/cpu").and_then(parse_cpu_millicores);
            row.mem_usage = s(m, "/usage/memory").and_then(parse_mem_bytes);
        }
    }
}

/// Container table for the pod drawer.
pub fn pod_containers(pod: &Value) -> Vec<ContainerRow> {
    let mut out = Vec::new();
    for (spec_ptr, status_ptr, init) in [
        (
            "/spec/initContainers",
            "/status/initContainerStatuses",
            true,
        ),
        ("/spec/containers", "/status/containerStatuses", false),
    ] {
        let statuses = arr(pod, status_ptr);
        for c in arr(pod, spec_ptr) {
            let name = s(c, "/name").unwrap_or("").to_string();
            let st = statuses
                .iter()
                .find(|st| s(st, "/name") == Some(name.as_str()));
            let state = st
                .and_then(|st| {
                    st.pointer("/state")
                        .and_then(Value::as_object)
                        .and_then(|m| {
                            m.iter().next().map(|(k, v)| match s(v, "/reason") {
                                Some(r) => format!("{k}:{r}"),
                                None => k.clone(),
                            })
                        })
                })
                .unwrap_or_else(|| "unknown".into());
            out.push(ContainerRow {
                image: s(c, "/image").unwrap_or("").to_string(),
                name,
                ready: st.and_then(|st| b(st, "/ready")).unwrap_or(false),
                state,
                restarts: st.and_then(|st| i(st, "/restartCount")).unwrap_or(0),
                init,
            });
        }
    }
    out
}

/// Strip `managedFields` and, for Secrets, blank every data value (§3.2).
pub fn sanitize_manifest(mut manifest: Value) -> Value {
    if let Some(meta) = manifest
        .pointer_mut("/metadata")
        .and_then(Value::as_object_mut)
    {
        meta.remove("managedFields");
    }
    if manifest.get("kind").and_then(Value::as_str) == Some("Secret") {
        for field in ["data", "stringData"] {
            if let Some(data) = manifest.get_mut(field).and_then(Value::as_object_mut) {
                let redacted: Map<String, Value> = data
                    .keys()
                    .map(|k| (k.clone(), Value::String("<redacted>".into())))
                    .collect();
                *data = redacted;
            }
        }
        // `last-applied-configuration` embeds the full secret too.
        if let Some(ann) = manifest
            .pointer_mut("/metadata/annotations")
            .and_then(Value::as_object_mut)
        {
            if ann.contains_key("kubectl.kubernetes.io/last-applied-configuration") {
                ann.insert(
                    "kubectl.kubernetes.io/last-applied-configuration".into(),
                    Value::String("<redacted>".into()),
                );
            }
        }
    }
    manifest
}

/// Events for one object, newest first.
pub fn parse_events(list: &Value) -> Vec<EventRow> {
    let mut evs: Vec<(String, EventRow)> = arr(list, "/items")
        .iter()
        .map(|e| {
            let last = s(e, "/lastTimestamp")
                .or_else(|| s(e, "/eventTime"))
                .or_else(|| s(e, "/series/lastObservedTime"))
                .or_else(|| s(e, "/metadata/creationTimestamp"))
                .map(str::to_string);
            (
                last.clone().unwrap_or_default(),
                EventRow {
                    kind: s(e, "/type").unwrap_or("Normal").to_string(),
                    reason: s(e, "/reason").unwrap_or("").to_string(),
                    message: s(e, "/message").unwrap_or("").to_string(),
                    count: i(e, "/count")
                        .or_else(|| i(e, "/series/count"))
                        .unwrap_or(1),
                    last_seen: last,
                },
            )
        })
        .collect();
    evs.sort_by(|a, b| b.0.cmp(&a.0));
    evs.into_iter().map(|(_, e)| e).collect()
}

/// Case-insensitive free-text filter over the visible columns.
pub fn matches_query(row: &K8sRow, q: &str) -> bool {
    let q = q.trim().to_ascii_lowercase();
    if q.is_empty() {
        return true;
    }
    let hay = |s: &str| s.to_ascii_lowercase().contains(&q);
    hay(&row.name)
        || hay(&row.namespace)
        || hay(&row.status)
        || row.node.as_deref().is_some_and(hay)
        || row.ip.as_deref().is_some_and(hay)
        || row.images.iter().flatten().any(|s| hay(s))
        || row.extra.values().any(|v| hay(v))
        || row.labels.iter().any(|(k, v)| hay(k) || hay(v))
}

// ---------------------------------------------------------------------------
// kubectl-backed operations
// ---------------------------------------------------------------------------

/// Namespace selector flags: empty ⇒ `-A` (all namespaces) for namespaced kinds.
fn ns_flags(kind: Kind, ns: Option<&str>) -> Vec<String> {
    if !kind.namespaced() {
        return vec![];
    }
    match ns.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => vec!["-n".into(), n.to_string()],
        None => vec!["-A".into()],
    }
}

/// `GET /k8s/clusters/{id}/resources` — list + normalise + filter (+ metrics).
pub async fn list(
    k: &Kubectl,
    kind: Kind,
    ns: Option<&str>,
    label: Option<&str>,
    q: Option<&str>,
    metrics_server: bool,
) -> Result<(Vec<K8sRow>, bool)> {
    let mut args: Vec<String> = vec![
        "get".into(),
        kind.kubectl_resource().into(),
        "-o".into(),
        "json".into(),
    ];
    args.extend(ns_flags(kind, ns));
    if let Some(l) = label.map(str::trim).filter(|l| !l.is_empty()) {
        args.push("-l".into());
        args.push(l.to_string());
    }
    let list = k.json(args).await?;
    let now = Utc::now();
    let mut rows: Vec<K8sRow> = arr(&list, "/items")
        .iter()
        .map(|item| normalize(kind, item, now))
        .filter(|r| q.is_none_or(|q| matches_query(r, q)))
        .collect();
    let mut has_metrics = false;
    if kind == Kind::Pods && metrics_server && !rows.is_empty() {
        // Best-effort: a metrics hiccup must not blank the pod table.
        if let Ok(m) = pod_metrics(k, ns).await {
            merge_pod_metrics(&mut rows, &m);
            has_metrics = true;
        }
    }
    Ok((rows, has_metrics))
}

/// Raw metrics-server pod list for `ns` (or all).
pub async fn pod_metrics(k: &Kubectl, ns: Option<&str>) -> Result<Vec<PodMetrics>> {
    let path = match ns.map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => format!("/apis/metrics.k8s.io/v1beta1/namespaces/{n}/pods"),
        None => "/apis/metrics.k8s.io/v1beta1/pods".to_string(),
    };
    let v = k.json(["get", "--raw", &path]).await?;
    Ok(parse_pod_metrics(&v))
}

/// `GET /k8s/clusters/{id}/metrics` — pods + `available` flag.
pub async fn metrics(k: &Kubectl, ns: Option<&str>) -> Result<(Vec<PodMetrics>, bool)> {
    match pod_metrics(k, ns).await {
        Ok(p) => Ok((p, true)),
        Err(Error::Forbidden(m)) => Err(Error::Forbidden(m)),
        Err(e) => {
            tracing::debug!("metrics unavailable: {e}");
            Ok((vec![], false))
        }
    }
}

/// `GET /k8s/clusters/{id}/nodes` — `get nodes` merged with node metrics.
pub async fn nodes(k: &Kubectl, metrics_server: bool) -> Result<Vec<NodeRow>> {
    let list = k.json(["get", "nodes", "-o", "json"]).await?;
    let now = Utc::now();
    let mut rows: Vec<NodeRow> = arr(&list, "/items").iter().map(|n| node(n, now)).collect();
    if metrics_server {
        if let Ok(m) = k
            .json(["get", "--raw", "/apis/metrics.k8s.io/v1beta1/nodes"])
            .await
        {
            merge_node_metrics(&mut rows, &m);
        }
    }
    Ok(rows)
}

/// `GET /k8s/clusters/{id}/namespaces`.
pub async fn namespaces(k: &Kubectl) -> Result<Vec<NamespaceRow>> {
    let list = k.json(["get", "namespaces", "-o", "json"]).await?;
    let now = Utc::now();
    Ok(arr(&list, "/items")
        .iter()
        .map(|n| NamespaceRow {
            name: s(n, "/metadata/name").unwrap_or("").to_string(),
            status: s(n, "/status/phase").unwrap_or("").to_string(),
            age_seconds: age_seconds(n, "/metadata/creationTimestamp", now),
        })
        .collect())
}

/// Fetch one object's JSON.
pub async fn get_one(k: &Kubectl, kind: Kind, ns: Option<&str>, name: &str) -> Result<Value> {
    let mut args: Vec<String> = vec![
        "get".into(),
        kind.kubectl_resource().into(),
        name.into(),
        "-o".into(),
        "json".into(),
    ];
    if kind.namespaced() {
        if let Some(n) = ns.map(str::trim).filter(|n| !n.is_empty()) {
            args.push("-n".into());
            args.push(n.to_string());
        }
    }
    k.json(args).await
}

/// `GET /k8s/clusters/{id}/pods/{ns}/{name}/containers`.
pub async fn containers(k: &Kubectl, ns: &str, name: &str) -> Result<Vec<ContainerRow>> {
    let pod = get_one(k, Kind::Pods, Some(ns), name).await?;
    Ok(pod_containers(&pod))
}

/// `GET /k8s/clusters/{id}/resource` — sanitized manifest + describe + events.
pub async fn detail(k: &Kubectl, kind: Kind, ns: Option<&str>, name: &str) -> Result<Value> {
    let manifest = get_one(k, kind, ns, name).await?;
    let uid = s(&manifest, "/metadata/uid").map(str::to_string);
    let ns_args: Vec<String> = if kind.namespaced() {
        ns.map(str::trim)
            .filter(|n| !n.is_empty())
            .map(|n| vec!["-n".to_string(), n.to_string()])
            .unwrap_or_default()
    } else {
        vec![]
    };
    let mut describe_args: Vec<String> = vec![
        "describe".into(),
        kind.kubectl_resource().into(),
        name.into(),
    ];
    describe_args.extend(ns_args.iter().cloned());
    let mut events_args: Vec<String> =
        vec!["get".into(), "events".into(), "-o".into(), "json".into()];
    events_args.extend(ns_args.iter().cloned());
    let selector = match &uid {
        Some(u) => format!("involvedObject.uid={u}"),
        None => format!("involvedObject.name={name}"),
    };
    events_args.push("--field-selector".into());
    events_args.push(selector);
    let (describe, events) = tokio::join!(k.run(describe_args), k.json(events_args));
    let describe = match describe {
        Ok(o) => sanitize_describe(kind, &o.stdout),
        Err(e) => format!("(describe failed: {e})"),
    };
    let events = events.map(|v| parse_events(&v)).unwrap_or_default();
    Ok(json!({
        "manifest": sanitize_manifest(manifest),
        "describe": describe,
        "events": events,
    }))
}

/// `kubectl describe secret` only prints sizes, but keep the invariant explicit:
/// any `Data` section of a Secret is dropped.
fn sanitize_describe(kind: Kind, text: &str) -> String {
    if kind != Kind::Secrets {
        return text.to_string();
    }
    let mut out = String::new();
    let mut in_data = false;
    for line in text.lines() {
        if line.starts_with("Data") {
            in_data = true;
            out.push_str("Data\n====\n<redacted>\n");
            continue;
        }
        if in_data
            && !line.starts_with(' ')
            && !line.starts_with('=')
            && !line.trim().is_empty()
            && line.contains(':')
            && !line.contains("bytes")
        {
            in_data = false;
        }
        if !in_data {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn fixture(name: &str) -> Value {
        let text = match name {
            "pods" => include_str!("../testdata/pods.json"),
            "deployments" => include_str!("../testdata/deployments.json"),
            "rollouts" => include_str!("../testdata/rollouts.json"),
            "applications" => include_str!("../testdata/applications.json"),
            "secrets" => include_str!("../testdata/secret_list.json"),
            "pod_metrics" => include_str!("../testdata/pod_metrics.json"),
            "nodes" => include_str!("../testdata/nodes.json"),
            "node_metrics" => include_str!("../testdata/node_metrics.json"),
            _ => panic!("no fixture {name}"),
        };
        serde_json::from_str(text).unwrap()
    }

    fn rows(kind: Kind, name: &str) -> Vec<K8sRow> {
        arr(&fixture(name), "/items")
            .iter()
            .map(|i| normalize(kind, i, now()))
            .collect()
    }

    fn by_name<'a>(rows: &'a [K8sRow], name: &str) -> &'a K8sRow {
        rows.iter()
            .find(|r| r.name == name)
            .unwrap_or_else(|| panic!("row {name}"))
    }

    #[test]
    fn pod_status_and_health_rules() {
        let rows = rows(Kind::Pods, "pods");
        let running = by_name(&rows, "web-5d4c-abcde");
        assert_eq!(running.status, "Running");
        assert_eq!(running.ready.as_deref(), Some("2/2"));
        assert_eq!(running.restarts, Some(3));
        assert_eq!(running.health, Some(Health::Ok));
        assert_eq!(running.node.as_deref(), Some("ip-10-0-1-5"));
        assert_eq!(running.ip.as_deref(), Some("10.244.1.7"));
        assert_eq!(
            running.images.as_ref().unwrap(),
            &["registry/web:1.2.3", "registry/sidecar:0.9"]
        );
        assert_eq!(running.age_seconds, 7200);
        assert_eq!(running.labels["app"], "web");
        assert_eq!(running.extra["qos"], "Burstable");

        let crash = by_name(&rows, "worker-crash");
        assert_eq!(crash.status, "CrashLoopBackOff");
        assert_eq!(crash.health, Some(Health::Bad));
        assert_eq!(crash.restarts, Some(17));
        assert_eq!(crash.ready.as_deref(), Some("0/1"));
        assert!(crash.extra["message"].contains("back-off"));

        let pending = by_name(&rows, "api-pending");
        assert_eq!(pending.status, "Pending");
        assert_eq!(pending.health, Some(Health::Progressing));
        assert_eq!(pending.ready.as_deref(), Some("0/1"));
        assert!(pending.node.is_none());

        let term = by_name(&rows, "old-terminating");
        assert_eq!(term.status, "Terminating");
        assert_eq!(term.health, Some(Health::Warn));

        let done = by_name(&rows, "report-1-xyz");
        assert_eq!(done.status, "Completed");
        assert_eq!(done.health, Some(Health::Ok));

        let img = by_name(&rows, "img-broken");
        assert_eq!(img.status, "ImagePullBackOff");
        assert_eq!(img.health, Some(Health::Bad));
        assert_eq!(img.restarts, Some(3), "init container restarts count too");

        let init = by_name(&rows, "init-wait");
        assert_eq!(init.status, "Init:0/1");
        assert_eq!(init.health, Some(Health::Progressing));
    }

    #[test]
    fn deployment_math() {
        let rows = rows(Kind::Deployments, "deployments");
        let web = by_name(&rows, "web");
        assert_eq!(web.status, "Available");
        assert_eq!(web.health, Some(Health::Ok));
        assert_eq!(web.ready.as_deref(), Some("3/3"));
        assert_eq!(web.extra["desired"], "3");
        assert_eq!(web.extra["updated"], "3");
        assert_eq!(web.extra["available"], "3");
        assert_eq!(web.images.as_ref().unwrap()[0], "registry/web:1.2.3");

        let api = by_name(&rows, "api");
        assert_eq!(api.status, "Updating");
        assert_eq!(api.health, Some(Health::Progressing));
        assert_eq!(api.ready.as_deref(), Some("2/4"));

        let broken = by_name(&rows, "broken");
        assert_eq!(broken.status, "Failed");
        assert_eq!(broken.health, Some(Health::Bad));
        assert_eq!(broken.extra["reason"], "ProgressDeadlineExceeded");
        assert_eq!(broken.extra["available"], "0");

        let parked = by_name(&rows, "parked");
        assert_eq!(parked.status, "ScaledDown");
        assert_eq!(parked.extra["paused"], "true");
        assert_eq!(parked.health, Some(Health::Warn));
    }

    #[test]
    fn rollout_extras() {
        let rows = rows(Kind::Rollouts, "rollouts");
        let c = by_name(&rows, "checkout");
        assert_eq!(c.status, "Paused");
        assert_eq!(c.health, Some(Health::Warn));
        assert_eq!(c.extra["strategy"], "canary");
        assert_eq!(c.extra["step"], "1/4");
        assert_eq!(c.extra["weight"], "20");
        assert_eq!(c.extra["paused"], "true");
        assert_eq!(c.extra["message"], "CanaryPauseStep");
        assert_eq!(c.ready.as_deref(), Some("5/5"));
        let p = by_name(&rows, "payments");
        assert_eq!(p.extra["strategy"], "blueGreen");
        assert!(!p.extra.contains_key("step"));
        assert_eq!(p.health, Some(Health::Ok));
    }

    #[test]
    fn application_extras() {
        let rows = rows(Kind::Applications, "applications");
        let a = &rows[0];
        assert_eq!(a.namespace, "argocd");
        assert_eq!(a.status, "Healthy");
        assert_eq!(a.health, Some(Health::Warn), "OutOfSync but healthy ⇒ warn");
        assert_eq!(a.extra["sync"], "OutOfSync");
        assert_eq!(a.extra["health"], "Healthy");
        assert_eq!(a.extra["revision"], "01234567");
        assert_eq!(a.extra["repo"], "https://github.com/acme/deploy.git");
        assert_eq!(a.extra["path"], "apps/shop/overlays/prod");
        assert_eq!(a.extra["dest_ns"], "shop");
        assert_eq!(a.extra["operation"], "Succeeded");
        assert_eq!(a.extra["target_revision"], "main");
    }

    #[test]
    fn secrets_never_expose_values() {
        let rows = rows(Kind::Secrets, "secrets");
        let r = &rows[0];
        assert_eq!(r.status, "Opaque");
        assert_eq!(r.extra["type"], "Opaque");
        assert_eq!(r.extra["keys"], "password,username");
        let ser = serde_json::to_string(r).unwrap();
        assert!(!ser.contains("aHVudGVyMg=="), "{ser}");
        assert!(!ser.contains("YWRtaW4="));

        let item = fixture("secrets")["items"][0].clone();
        let m = sanitize_manifest(item);
        assert_eq!(m["data"]["password"], "<redacted>");
        assert_eq!(m["data"]["username"], "<redacted>");
        assert!(m["metadata"].get("managedFields").is_none());
        assert!(!m.to_string().contains("aHVudGVyMg=="));
    }

    #[test]
    fn quantities() {
        assert_eq!(parse_cpu_millicores("250m"), Some(250));
        assert_eq!(parse_cpu_millicores("1"), Some(1000));
        assert_eq!(parse_cpu_millicores("1.5"), Some(1500));
        assert_eq!(parse_cpu_millicores("1500u"), Some(2));
        assert_eq!(parse_cpu_millicores("12345678n"), Some(12));
        assert_eq!(parse_cpu_millicores("x"), None);
        assert_eq!(parse_mem_bytes("128Mi"), Some(128 * 1024 * 1024));
        assert_eq!(parse_mem_bytes("1Gi"), Some(1 << 30));
        assert_eq!(parse_mem_bytes("20480Ki"), Some(20480 * 1024));
        assert_eq!(parse_mem_bytes("1000000"), Some(1_000_000));
        assert_eq!(parse_mem_bytes("1G"), Some(1_000_000_000));
        assert_eq!(parse_mem_bytes("1e3"), Some(1000));
        assert_eq!(parse_mem_bytes("8000000Ki"), Some(8_000_000 * 1024));
    }

    #[test]
    fn top_merge_into_pod_rows() {
        let mut rows = rows(Kind::Pods, "pods");
        let m = parse_pod_metrics(&fixture("pod_metrics"));
        assert_eq!(m.len(), 2);
        let web = m.iter().find(|p| p.name == "web-5d4c-abcde").unwrap();
        assert_eq!(web.cpu_millicores, 252);
        assert_eq!(web.mem_bytes, 128 * 1024 * 1024 + 20480 * 1024);
        assert_eq!(web.containers.len(), 2);
        merge_pod_metrics(&mut rows, &m);
        assert_eq!(by_name(&rows, "web-5d4c-abcde").cpu, Some(252));
        assert_eq!(by_name(&rows, "worker-crash").mem, Some(1 << 30));
        assert_eq!(by_name(&rows, "api-pending").cpu, None);
    }

    #[test]
    fn nodes_and_top_nodes() {
        let list = fixture("nodes");
        let mut rows: Vec<NodeRow> = arr(&list, "/items")
            .iter()
            .map(|n| node(n, now()))
            .collect();
        assert_eq!(rows[0].status, "Ready");
        assert_eq!(rows[0].roles, "control-plane");
        assert_eq!(rows[0].version, "v1.30.2");
        assert_eq!(rows[0].cpu_capacity, 8000);
        assert_eq!(rows[0].mem_capacity, 16 << 30);
        assert_eq!(rows[1].status, "NotReady,SchedulingDisabled");
        assert_eq!(rows[1].roles, "<none>");
        merge_node_metrics(&mut rows, &fixture("node_metrics"));
        assert_eq!(rows[0].cpu_usage, Some(1200));
        assert_eq!(rows[0].mem_usage, Some(4 << 30));
        assert_eq!(rows[1].cpu_usage, None);
    }

    #[test]
    fn containers_table_includes_init_containers() {
        let pod = fixture("pods")["items"][5].clone();
        let c = pod_containers(&pod);
        assert_eq!(c.len(), 2);
        assert!(c[0].init);
        assert_eq!(c[0].name, "init-db");
        assert_eq!(c[0].state, "terminated:Completed");
        assert_eq!(c[0].restarts, 3);
        assert!(!c[1].init);
        assert_eq!(c[1].state, "waiting:ImagePullBackOff");
        assert!(!c[1].ready);
    }

    #[test]
    fn events_sorted_newest_first() {
        let list = json!({"items": [
            {"type": "Normal", "reason": "Pulled", "message": "ok", "count": 1, "lastTimestamp": "2026-09-01T10:00:00Z"},
            {"type": "Warning", "reason": "BackOff", "message": "back-off", "count": 9, "lastTimestamp": "2026-09-01T11:00:00Z"},
            {"type": "Normal", "reason": "Scheduled", "message": "s", "eventTime": "2026-09-01T09:00:00Z"}
        ]});
        let e = parse_events(&list);
        assert_eq!(e[0].reason, "BackOff");
        assert_eq!(e[0].kind, "Warning");
        assert_eq!(e[0].count, 9);
        assert_eq!(e[2].reason, "Scheduled");
        assert_eq!(e[2].count, 1);
        let ser = serde_json::to_value(&e[0]).unwrap();
        assert_eq!(ser["type"], "Warning");
    }

    #[test]
    fn kind_parsing_and_kubectl_names() {
        assert_eq!(Kind::parse("PVC"), Some(Kind::Pvcs));
        assert_eq!(Kind::Pvcs.kubectl_resource(), "persistentvolumeclaims");
        assert_eq!(Kind::Hpas.kubectl_resource(), "horizontalpodautoscalers");
        assert_eq!(Kind::Rollouts.kubectl_resource(), "rollouts.argoproj.io");
        assert_eq!(
            Kind::Applications.kubectl_resource(),
            "applications.argoproj.io"
        );
        assert_eq!(Kind::parse("nope"), None);
        assert!(!Kind::Nodes.namespaced());
        assert_eq!(Kind::from_k8s_kind("StatefulSet"), Some(Kind::Statefulsets));
        assert_eq!(ns_flags(Kind::Pods, None), vec!["-A"]);
        assert_eq!(ns_flags(Kind::Pods, Some("")), vec!["-A"]);
        assert_eq!(ns_flags(Kind::Pods, Some("shop")), vec!["-n", "shop"]);
        assert!(ns_flags(Kind::Nodes, Some("shop")).is_empty());
    }

    #[test]
    fn query_filter() {
        let rows = rows(Kind::Pods, "pods");
        assert!(matches_query(by_name(&rows, "worker-crash"), "crashloop"));
        assert!(matches_query(
            by_name(&rows, "web-5d4c-abcde"),
            "sidecar:0.9"
        ));
        assert!(!matches_query(by_name(&rows, "web-5d4c-abcde"), "app=web"));
        assert!(matches_query(by_name(&rows, "web-5d4c-abcde"), "web"));
        assert!(!matches_query(by_name(&rows, "api-pending"), "zzz"));
        assert!(matches_query(by_name(&rows, "api-pending"), "  "));
    }

    #[test]
    fn other_kinds_smoke() {
        let svc = json!({"metadata": {"name": "web", "namespace": "shop", "creationTimestamp": "2026-09-01T00:00:00Z"},
            "spec": {"type": "LoadBalancer", "clusterIP": "10.0.0.5", "ports": [{"port": 80, "protocol": "TCP", "nodePort": 30080}], "selector": {"app": "web"}},
            "status": {"loadBalancer": {"ingress": [{"hostname": "abc.elb.amazonaws.com"}]}}});
        let r = normalize(Kind::Services, &svc, now());
        assert_eq!(r.status, "LoadBalancer");
        assert_eq!(r.extra["ports"], "80:30080/TCP");
        assert_eq!(r.extra["external_ip"], "abc.elb.amazonaws.com");
        assert_eq!(r.extra["selector"], "app=web");
        assert_eq!(r.ip.as_deref(), Some("10.0.0.5"));

        let cj = json!({"metadata": {"name": "nightly", "namespace": "batch"}, "spec": {"schedule": "0 2 * * *", "suspend": true}, "status": {}});
        let r = normalize(Kind::Cronjobs, &cj, now());
        assert_eq!(r.status, "Suspended");
        assert_eq!(r.extra["schedule"], "0 2 * * *");
        assert_eq!(r.health, Some(Health::Warn));

        let job = json!({"metadata": {"name": "j"}, "spec": {"completions": 2}, "status": {"succeeded": 2, "startTime": "2026-09-01T00:00:00Z", "completionTime": "2026-09-01T00:01:30Z", "conditions": [{"type": "Complete", "status": "True"}]}});
        let r = normalize(Kind::Jobs, &job, now());
        assert_eq!(r.status, "Complete");
        assert_eq!(r.extra["completions"], "2/2");
        assert_eq!(r.extra["duration_seconds"], "90");

        let pvc = json!({"metadata": {"name": "data"}, "spec": {"storageClassName": "gp3", "accessModes": ["ReadWriteOnce"], "resources": {"requests": {"storage": "10Gi"}}}, "status": {"phase": "Bound", "capacity": {"storage": "10Gi"}}});
        let r = normalize(Kind::Pvcs, &pvc, now());
        assert_eq!(r.status, "Bound");
        assert_eq!(r.extra["capacity"], "10Gi");
        assert_eq!(r.extra["access_modes"], "ReadWriteOnce");

        let hpa = json!({"metadata": {"name": "web"}, "spec": {"minReplicas": 2, "maxReplicas": 10, "scaleTargetRef": {"kind": "Deployment", "name": "web"},
            "metrics": [{"type": "Resource", "resource": {"name": "cpu", "target": {"type": "Utilization", "averageUtilization": 80}}}]},
            "status": {"currentReplicas": 10, "desiredReplicas": 10, "currentMetrics": [{"type": "Resource", "resource": {"name": "cpu", "current": {"averageUtilization": 93}}}]}});
        let r = normalize(Kind::Hpas, &hpa, now());
        assert_eq!(r.extra["reference"], "Deployment/web");
        assert_eq!(r.extra["targets"], "cpu: 93%/80%");
        assert_eq!(r.health, Some(Health::Warn), "pinned at max");

        let ing = json!({"metadata": {"name": "web"}, "spec": {"ingressClassName": "nginx", "rules": [{"host": "shop.example.com"}], "tls": [{"hosts": ["shop.example.com"]}]}, "status": {"loadBalancer": {"ingress": [{"ip": "1.2.3.4"}]}}});
        let r = normalize(Kind::Ingresses, &ing, now());
        assert_eq!(r.extra["hosts"], "shop.example.com");
        assert_eq!(r.extra["address"], "1.2.3.4");
        assert_eq!(r.extra["tls"], "true");
        assert_eq!(r.status, "Ready");

        let cm = json!({"metadata": {"name": "cfg"}, "data": {"a": "1", "b": "2"}, "binaryData": {"c": "AA=="}});
        let r = normalize(Kind::Configmaps, &cm, now());
        assert_eq!(r.extra["keys"], "a,b,c");
        assert_eq!(r.status, "3 keys");

        let ds = json!({"metadata": {"name": "fluentd"}, "status": {"desiredNumberScheduled": 3, "updatedNumberScheduled": 3, "numberReady": 2, "numberAvailable": 2}});
        let r = normalize(Kind::Daemonsets, &ds, now());
        assert_eq!(r.status, "Progressing");
        assert_eq!(r.ready.as_deref(), Some("2/3"));

        let rs = json!({"metadata": {"name": "web-old"}, "spec": {"replicas": 0}, "status": {"replicas": 0}});
        let r = normalize(Kind::Replicasets, &rs, now());
        assert_eq!(r.status, "Inactive");
        assert_eq!(r.health, Some(Health::Ok));

        let ev = json!({"metadata": {"name": "web.1", "namespace": "shop", "creationTimestamp": "2026-09-01T11:00:00Z"}, "type": "Warning", "reason": "BackOff", "message": "m", "count": 4, "lastTimestamp": "2026-09-01T11:59:00Z", "involvedObject": {"kind": "Pod", "name": "web-1"}});
        let r = normalize(Kind::Events, &ev, now());
        assert_eq!(r.status, "Warning");
        assert_eq!(r.extra["object"], "Pod/web-1");
        assert_eq!(r.age_seconds, 60);
        assert_eq!(r.extra["count"], "4");
    }
}
