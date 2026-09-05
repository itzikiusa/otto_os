//! Restart / pod-churn classification (spec "Restart classification").
//!
//! Every cycle the collector snapshots the pod list ([`snapshot_from_pod_list`])
//! and diffs it against the previous cycle's snapshot ([`classify`]):
//!
//! * a container whose `restartCount` rose is an in-place **restart** —
//!   classed `oom` / `probe` / `crash` / `unknown` from its last terminated
//!   state + the events of the window;
//! * a pod that disappeared or appeared is **churn** — classed `planned`
//!   (rollout, scale, drain, an Otto action on the workload) or `completed`
//!   (Jobs), else `unknown`.
//!
//! Restart counters are per pod, so a rollout never inflates them; the two
//! kinds are reported as separate numbers ("restarts (unplanned)" vs "pod
//! churn (planned)").

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::resources::{arr, b, i, parse_cpu_millicores, parse_mem_bytes, s, string_map};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ContainerSnap {
    pub restarts: i64,
    /// `lastState.terminated.reason` (`OOMKilled`, `Error`, `Completed`…).
    pub last_reason: String,
    pub last_exit: i32,
    pub last_finished: Option<String>,
    /// `state.waiting.reason` (`CrashLoopBackOff`…) when not running.
    pub waiting_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PodSnap {
    pub namespace: String,
    pub name: String,
    pub phase: String,
    pub ready: bool,
    /// Direct owner (`ReplicaSet` `frb-769767cc74`, `Job`, `StatefulSet`…).
    pub owner_kind: String,
    pub owner: String,
    /// Logical workload (`deployment` / `rollout` / `statefulset` /
    /// `daemonset` / `job` / `cronjob` / `pod`) + its name.
    pub workload_kind: String,
    pub workload: String,
    pub node: String,
    pub containers: BTreeMap<String, ContainerSnap>,
    /// Sum of container memory limits (bytes); 0 = unlimited.
    pub mem_limit: i64,
    /// Sum of container CPU requests (millicores).
    pub cpu_request: i64,
    pub images: Vec<String>,
    pub labels: BTreeMap<String, String>,
    pub created: String,
    pub deleting: bool,
    /// First declared container port (probe default).
    #[serde(default)]
    pub first_port: Option<u16>,
    /// `status.reason` for the pod itself (`Evicted`, `Preempted`).
    #[serde(default)]
    pub pod_reason: String,
}

/// `"ns/name"` → pod.
pub type Snapshot = BTreeMap<String, PodSnap>;

pub fn snap_key(ns: &str, name: &str) -> String {
    format!("{ns}/{name}")
}

/// Strip the ReplicaSet pod-template hash (`frb-769767cc74` → `frb`).
fn strip_rs_hash(rs: &str) -> String {
    match rs.rsplit_once('-') {
        Some((base, hash))
            if !hash.is_empty()
                && hash.len() >= 5
                && hash.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) =>
        {
            base.to_string()
        }
        _ => rs.to_string(),
    }
}

/// Derive `(workload_kind, workload)` from the pod's owner reference.
fn workload_of(owner_kind: &str, owner: &str, pod_name: &str, labels: &BTreeMap<String, String>) -> (String, String) {
    match owner_kind {
        "ReplicaSet" => {
            // Argo Rollouts label their RS pods; Deployments do not.
            let kind = if labels.contains_key("rollouts-pod-template-hash") {
                "rollout"
            } else {
                "deployment"
            };
            (kind.into(), strip_rs_hash(owner))
        }
        "StatefulSet" => ("statefulset".into(), owner.to_string()),
        "DaemonSet" => ("daemonset".into(), owner.to_string()),
        "Job" => {
            // CronJob-spawned jobs are `<cronjob>-<unix-minutes>`.
            match owner.rsplit_once('-') {
                Some((base, suffix)) if suffix.len() >= 8 && suffix.chars().all(|c| c.is_ascii_digit()) => {
                    ("cronjob".into(), base.to_string())
                }
                _ => ("job".into(), owner.to_string()),
            }
        }
        "" => ("pod".into(), pod_name.to_string()),
        other => (other.to_ascii_lowercase(), owner.to_string()),
    }
}

fn container_snap(st: &Value) -> ContainerSnap {
    ContainerSnap {
        restarts: i(st, "/restartCount").unwrap_or(0),
        last_reason: s(st, "/lastState/terminated/reason").unwrap_or("").to_string(),
        last_exit: i(st, "/lastState/terminated/exitCode").unwrap_or(0) as i32,
        last_finished: s(st, "/lastState/terminated/finishedAt").map(str::to_string),
        waiting_reason: s(st, "/state/waiting/reason").unwrap_or("").to_string(),
    }
}

/// One `PodSnap` from a `v1.Pod` JSON object.
pub fn snapshot_pod(item: &Value) -> PodSnap {
    let labels = string_map(item.pointer("/metadata/labels"));
    let owners = arr(item, "/metadata/ownerReferences");
    let (owner_kind, owner) = owners
        .iter()
        .find(|o| b(o, "/controller") == Some(true))
        .or_else(|| owners.first())
        .map(|o| {
            (
                s(o, "/kind").unwrap_or("").to_string(),
                s(o, "/name").unwrap_or("").to_string(),
            )
        })
        .unwrap_or_default();
    let name = s(item, "/metadata/name").unwrap_or("").to_string();
    let (workload_kind, workload) = workload_of(&owner_kind, &owner, &name, &labels);
    let mut containers = BTreeMap::new();
    for st in arr(item, "/status/containerStatuses") {
        containers.insert(
            s(st, "/name").unwrap_or("").to_string(),
            container_snap(st),
        );
    }
    let specs = arr(item, "/spec/containers");
    let mem_limit: i64 = specs
        .iter()
        .filter_map(|c| s(c, "/resources/limits/memory").and_then(parse_mem_bytes))
        .sum();
    let cpu_request: i64 = specs
        .iter()
        .filter_map(|c| s(c, "/resources/requests/cpu").and_then(parse_cpu_millicores))
        .sum();
    let first_port = specs
        .iter()
        .flat_map(|c| arr(c, "/ports"))
        .find_map(|p| i(p, "/containerPort"))
        .and_then(|p| u16::try_from(p).ok());
    let ready = arr(item, "/status/conditions")
        .iter()
        .any(|c| s(c, "/type") == Some("Ready") && s(c, "/status") == Some("True"));
    PodSnap {
        namespace: s(item, "/metadata/namespace").unwrap_or("").to_string(),
        name,
        phase: s(item, "/status/phase").unwrap_or("Unknown").to_string(),
        ready,
        owner_kind,
        owner,
        workload_kind,
        workload,
        node: s(item, "/spec/nodeName").unwrap_or("").to_string(),
        containers,
        mem_limit,
        cpu_request,
        images: specs
            .iter()
            .filter_map(|c| s(c, "/image").map(str::to_string))
            .collect(),
        labels,
        created: s(item, "/metadata/creationTimestamp").unwrap_or("").to_string(),
        deleting: item.pointer("/metadata/deletionTimestamp").is_some(),
        first_port,
        pod_reason: s(item, "/status/reason").unwrap_or("").to_string(),
    }
}

/// `kubectl get pods -o json` → snapshot (walks `/items`).
pub fn snapshot_from_pod_list(list: &Value) -> Snapshot {
    arr(list, "/items")
        .iter()
        .map(snapshot_pod)
        .map(|p| (snap_key(&p.namespace, &p.name), p))
        .collect()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Class {
    Oom,
    Crash,
    Probe,
    Planned,
    Completed,
    Unknown,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Oom => "oom",
            Class::Crash => "crash",
            Class::Probe => "probe",
            Class::Planned => "planned",
            Class::Completed => "completed",
            Class::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Classified {
    /// `restart` (in place, counter rose) or `churn` (pod replaced/removed).
    pub kind: &'static str,
    pub class: Class,
    pub namespace: String,
    pub workload_kind: String,
    pub workload: String,
    pub pod: String,
    pub container: String,
    pub reason: String,
    pub exit_code: i32,
    /// `rollout` | `scale` | `drain` | `otto:<user>` | `` for planned churn.
    pub planned_by: String,
    pub prev_restarts: i64,
    pub next_restarts: i64,
    pub at: String,
}

/// A `v1.Event` reduced to what classification needs.
#[derive(Debug, Clone, PartialEq)]
pub struct EventHint {
    pub namespace: String,
    pub reason: String,
    pub message: String,
    pub at: DateTime<Utc>,
    pub involved_kind: String,
    pub involved_name: String,
}

/// An Otto `k8s.action.*` audit row (scale / restart / rollout…) on a workload.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionHint {
    pub namespace: String,
    pub workload: String,
    pub at: DateTime<Utc>,
    pub actor: String,
}

/// Events on a pod within `window` before `at`.
fn events_for<'a>(
    events: &'a [EventHint],
    ns: &str,
    name: &str,
    at: DateTime<Utc>,
    window: Duration,
) -> impl Iterator<Item = &'a EventHint> + 'a {
    let (ns, name) = (ns.to_string(), name.to_string());
    events.iter().filter(move |e| {
        e.namespace == ns
            && e.involved_kind == "Pod"
            && e.involved_name == name
            && e.at <= at + Duration::minutes(1)
            && e.at >= at - window
    })
}

fn classify_restart(c: &ContainerSnap, events: &[EventHint], ns: &str, pod: &str, now: DateTime<Utc>) -> Class {
    if c.last_reason == "OOMKilled"
        || events_for(events, ns, pod, now, Duration::minutes(10)).any(|e| e.reason == "OOMKilling")
    {
        return Class::Oom;
    }
    let at = c
        .last_finished
        .as_deref()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or(now);
    let mut unhealthy = false;
    let mut killing = false;
    for e in events_for(events, ns, pod, at, Duration::minutes(2)) {
        if e.reason == "Unhealthy" && e.message.contains("Liveness") {
            unhealthy = true;
        }
        if e.reason == "Killing" {
            killing = true;
        }
    }
    if unhealthy && killing {
        return Class::Probe;
    }
    if c.last_reason == "Error"
        || c.last_reason == "ContainerCannotRun"
        || c.last_reason == "StartError"
        || c.last_exit != 0
        || c.waiting_reason == "CrashLoopBackOff"
    {
        return Class::Crash;
    }
    Class::Unknown
}

/// Diff `prev` → `cur` and classify (see module doc).
pub fn classify(
    prev: &Snapshot,
    cur: &Snapshot,
    events: &[EventHint],
    actions: &[ActionHint],
    now: DateTime<Utc>,
) -> Vec<Classified> {
    let mut out = Vec::new();
    let now_s = now.to_rfc3339();

    // First cycle after enabling: no baseline, nothing to diff against. A
    // burst of "unknown churn" for every existing pod would be noise.
    if prev.is_empty() {
        return out;
    }

    // In-place restarts.
    for (key, p) in cur {
        let Some(old) = prev.get(key) else { continue };
        for (cname, c) in &p.containers {
            let before = old.containers.get(cname).map(|o| o.restarts).unwrap_or(0);
            if c.restarts <= before {
                continue;
            }
            let class = classify_restart(c, events, &p.namespace, &p.name, now);
            out.push(Classified {
                kind: "restart",
                class,
                namespace: p.namespace.clone(),
                workload_kind: p.workload_kind.clone(),
                workload: p.workload.clone(),
                pod: p.name.clone(),
                container: cname.clone(),
                reason: c.last_reason.clone(),
                exit_code: c.last_exit,
                planned_by: String::new(),
                prev_restarts: before,
                next_restarts: c.restarts,
                at: c.last_finished.clone().unwrap_or_else(|| now_s.clone()),
            });
        }
    }

    // Churn: pods gone / new, grouped per (ns, workload).
    let gone: Vec<&PodSnap> = prev.iter().filter(|(k, _)| !cur.contains_key(*k)).map(|(_, p)| p).collect();
    let new: Vec<&PodSnap> = cur.iter().filter(|(k, _)| !prev.contains_key(*k)).map(|(_, p)| p).collect();
    let mut groups: BTreeSet<(String, String)> = BTreeSet::new();
    for p in gone.iter().chain(new.iter()) {
        groups.insert((p.namespace.clone(), p.workload.clone()));
    }
    for (ns, wl) in groups {
        let g: Vec<&PodSnap> = gone.iter().copied().filter(|p| p.namespace == ns && p.workload == wl).collect();
        let n: Vec<&PodSnap> = new.iter().copied().filter(|p| p.namespace == ns && p.workload == wl).collect();
        let sample = g.first().or(n.first()).copied();
        let Some(sample) = sample else { continue };
        let wl_kind = sample.workload_kind.clone();

        // Jobs finishing are completions, not churn.
        if matches!(wl_kind.as_str(), "job" | "cronjob")
            && g.iter().all(|p| p.phase == "Succeeded" || p.phase == "Running")
            && n.is_empty()
        {
            for p in &g {
                out.push(churn(p, Class::Completed, "", &now_s));
            }
            continue;
        }

        let gone_owners: BTreeSet<&str> = g.iter().map(|p| p.owner.as_str()).collect();
        let new_owners: BTreeSet<&str> = n.iter().map(|p| p.owner.as_str()).collect();
        let rollout = !g.is_empty() && !n.is_empty() && gone_owners.is_disjoint(&new_owners);
        let otto = actions
            .iter()
            .filter(|a| a.namespace == ns && a.workload == wl && (now - a.at) <= Duration::minutes(5))
            .max_by_key(|a| a.at);
        let scaled = events.iter().any(|e| {
            e.namespace == ns
                && e.reason == "ScalingReplicaSet"
                && e.at >= now - Duration::minutes(10)
                && (e.involved_name == wl || e.message.contains(&format!(" {wl}-")) || e.message.contains(&format!(" {wl} ")))
        });

        for p in g.iter().chain(n.iter()) {
            let drained = matches!(p.pod_reason.as_str(), "Evicted" | "Preempted")
                || events_for(events, &p.namespace, &p.name, now, Duration::minutes(10))
                    .any(|e| e.reason == "Evicted" || e.reason == "Preempted");
            let (class, by) = if let Some(a) = otto {
                (Class::Planned, format!("otto:{}", a.actor))
            } else if rollout {
                (Class::Planned, "rollout".to_string())
            } else if drained {
                (Class::Planned, "drain".to_string())
            } else if scaled {
                (Class::Planned, "scale".to_string())
            } else if p.deleting && g.iter().any(|x| std::ptr::eq(*x, *p)) {
                // Was already terminating last cycle with nothing replacing it: a scale-down.
                (Class::Planned, "scale".to_string())
            } else {
                (Class::Unknown, String::new())
            };
            out.push(churn(p, class, &by, &now_s));
        }
    }
    out
}

fn churn(p: &PodSnap, class: Class, by: &str, at: &str) -> Classified {
    Classified {
        kind: "churn",
        class,
        namespace: p.namespace.clone(),
        workload_kind: p.workload_kind.clone(),
        workload: p.workload.clone(),
        pod: p.name.clone(),
        container: String::new(),
        reason: p.pod_reason.clone(),
        exit_code: 0,
        planned_by: by.to_string(),
        prev_restarts: 0,
        next_restarts: 0,
        at: at.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snap(name: &str, owner_rs: &str, restarts: i64, last_reason: &str, exit: i32) -> PodSnap {
        let mut containers = BTreeMap::new();
        containers.insert(
            "main".to_string(),
            ContainerSnap {
                restarts,
                last_reason: last_reason.into(),
                last_exit: exit,
                last_finished: None,
                waiting_reason: String::new(),
            },
        );
        PodSnap {
            namespace: "ns".into(),
            name: name.into(),
            phase: "Running".into(),
            ready: true,
            owner_kind: "ReplicaSet".into(),
            owner: owner_rs.into(),
            workload_kind: "deployment".into(),
            workload: strip_rs_hash(owner_rs),
            containers,
            ..PodSnap::default()
        }
    }
    fn snaps(v: Vec<PodSnap>) -> Snapshot {
        v.into_iter().map(|p| (snap_key(&p.namespace, &p.name), p)).collect()
    }
    fn ev(name: &str, reason: &str, message: &str, at: DateTime<Utc>) -> EventHint {
        EventHint {
            namespace: "ns".into(),
            reason: reason.into(),
            message: message.into(),
            at,
            involved_kind: "Pod".into(),
            involved_name: name.into(),
        }
    }

    #[test]
    fn oom_from_last_state() {
        let prev = snaps(vec![snap("frb-a", "frb-769767cc74", 0, "", 0)]);
        let cur = snaps(vec![snap("frb-a", "frb-769767cc74", 1, "OOMKilled", 137)]);
        let out = classify(&prev, &cur, &[], &[], Utc::now());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].class, Class::Oom);
        assert_eq!(out[0].kind, "restart");
        assert_eq!(out[0].exit_code, 137);
        assert_eq!(out[0].workload, "frb");
        assert_eq!((out[0].prev_restarts, out[0].next_restarts), (0, 1));
    }

    #[test]
    fn crash_on_error_exit() {
        let prev = snaps(vec![snap("gba-a", "gobalanceadjustment-549b8bd7b6", 0, "", 0)]);
        let cur = snaps(vec![snap("gba-a", "gobalanceadjustment-549b8bd7b6", 1, "Error", 2)]);
        let out = classify(&prev, &cur, &[], &[], Utc::now());
        assert_eq!(out[0].class, Class::Crash);
        assert_eq!(out[0].workload, "gobalanceadjustment");
    }

    #[test]
    fn probe_kill_from_events() {
        let now = Utc::now();
        let prev = snaps(vec![snap("p-a", "p-1234567890", 0, "", 0)]);
        let cur = snaps(vec![snap("p-a", "p-1234567890", 1, "Error", 137)]);
        let events = vec![
            ev("p-a", "Unhealthy", "Liveness probe failed: HTTP 503", now - Duration::seconds(50)),
            ev("p-a", "Killing", "Container main failed liveness probe, will be restarted", now - Duration::seconds(40)),
        ];
        let out = classify(&prev, &cur, &events, &[], now);
        assert_eq!(out[0].class, Class::Probe);
    }

    #[test]
    fn rollout_is_planned_churn() {
        let prev = snaps(vec![snap("adm-old", "admission-1111111111", 0, "", 0)]);
        let cur = snaps(vec![snap("adm-new", "admission-2222222222", 0, "", 0)]);
        let out = classify(&prev, &cur, &[], &[], Utc::now());
        assert_eq!(out.len(), 2, "gone + new");
        assert!(out.iter().all(|c| c.kind == "churn" && c.class == Class::Planned && c.planned_by == "rollout"));
    }

    #[test]
    fn otto_action_marks_planned() {
        let now = Utc::now();
        let prev = snaps(vec![snap("adm-old", "admission-1111111111", 0, "", 0)]);
        let cur = snaps(vec![snap("adm-new", "admission-1111111111", 0, "", 0)]);
        let actions = vec![ActionHint {
            namespace: "ns".into(),
            workload: "admission".into(),
            at: now - Duration::minutes(1),
            actor: "itzik".into(),
        }];
        let out = classify(&prev, &cur, &[], &actions, now);
        assert!(out.iter().all(|c| c.class == Class::Planned && c.planned_by == "otto:itzik"));
    }

    #[test]
    fn evicted_is_drain() {
        let prev = snaps(vec![snap("x-a", "x-1111111111", 0, "", 0)]);
        let mut gone = snap("x-a", "x-1111111111", 0, "", 0);
        gone.pod_reason = "Evicted".into();
        let prev2 = snaps(vec![gone]);
        // pod present in prev with Evicted reason, absent now
        let _ = prev;
        let cur = Snapshot::new();
        let out = classify(&prev2, &cur, &[], &[], Utc::now());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].class, Class::Planned);
        assert_eq!(out[0].planned_by, "drain");
    }

    #[test]
    fn job_completion_is_completed_not_restart() {
        let mut j = snap("cleanup-29312345-abc", "cleanup-29312345", 0, "Completed", 0);
        j.owner_kind = "Job".into();
        j.workload_kind = "cronjob".into();
        j.workload = "cleanup".into();
        j.phase = "Succeeded".into();
        let prev = snaps(vec![j]);
        let out = classify(&prev, &Snapshot::new(), &[], &[], Utc::now());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].class, Class::Completed);
        assert_eq!(out[0].kind, "churn");
    }

    #[test]
    fn unknown_keeps_raw_reason() {
        let prev = snaps(vec![snap("u-a", "u-1111111111", 0, "", 0)]);
        let cur = snaps(vec![snap("u-a", "u-1111111111", 1, "", 0)]);
        let out = classify(&prev, &cur, &[], &[], Utc::now());
        assert_eq!(out[0].class, Class::Unknown);
        assert_eq!(out[0].reason, "");
    }

    #[test]
    fn first_cycle_without_baseline_emits_nothing() {
        let cur = snaps(vec![snap("a-1", "a-1111111111", 3, "Error", 1), snap("b-1", "b-1111111111", 0, "", 0)]);
        assert!(classify(&Snapshot::new(), &cur, &[], &[], Utc::now()).is_empty());
    }

    #[test]
    fn restart_count_reset_on_recreate_is_not_negative() {
        let prev = snaps(vec![snap("r-a", "r-1111111111", 5, "Error", 1)]);
        let cur = snaps(vec![snap("r-a", "r-1111111111", 0, "", 0)]);
        let out = classify(&prev, &cur, &[], &[], Utc::now());
        assert!(out.is_empty());
    }

    #[test]
    fn scaling_event_marks_scale() {
        let now = Utc::now();
        let prev = snaps(vec![snap("s-a", "s-1111111111", 0, "", 0), snap("s-b", "s-1111111111", 0, "", 0)]);
        let cur = snaps(vec![snap("s-a", "s-1111111111", 0, "", 0)]);
        let events = vec![EventHint {
            namespace: "ns".into(),
            reason: "ScalingReplicaSet".into(),
            message: "Scaled down replica set s-1111111111 to 1".into(),
            at: now - Duration::seconds(30),
            involved_kind: "Deployment".into(),
            involved_name: "s".into(),
        }];
        let out = classify(&prev, &cur, &events, &[], now);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].planned_by, "scale");
    }

    #[test]
    fn snapshot_from_pod_json() {
        let list = json!({"items":[{
            "metadata":{"name":"frb-769767cc74-s4ckn","namespace":"mscasino","creationTimestamp":"2026-09-01T00:00:00Z",
                        "labels":{"app":"frb"},
                        "ownerReferences":[{"kind":"ReplicaSet","name":"frb-769767cc74","controller":true}]},
            "spec":{"nodeName":"n1","containers":[{"name":"frb","image":"reg/frb:5.02","ports":[{"containerPort":9000}],
                    "resources":{"limits":{"memory":"512Mi"},"requests":{"cpu":"100m"}}}]},
            "status":{"phase":"Running","conditions":[{"type":"Ready","status":"True"}],
                      "containerStatuses":[{"name":"frb","restartCount":1,"state":{"running":{}},
                        "lastState":{"terminated":{"reason":"OOMKilled","exitCode":137,"finishedAt":"2026-09-04T10:00:00Z"}}}]}
        }]});
        let snap = snapshot_from_pod_list(&list);
        let p = &snap["mscasino/frb-769767cc74-s4ckn"];
        assert_eq!(p.workload, "frb");
        assert_eq!(p.workload_kind, "deployment");
        assert_eq!(p.mem_limit, 512 * 1024 * 1024);
        assert_eq!(p.cpu_request, 100);
        assert_eq!(p.first_port, Some(9000));
        assert!(p.ready);
        let c = &p.containers["frb"];
        assert_eq!(c.restarts, 1);
        assert_eq!(c.last_reason, "OOMKilled");
        assert_eq!(c.last_exit, 137);
    }

    #[test]
    fn workload_derivation() {
        let l = BTreeMap::new();
        assert_eq!(workload_of("ReplicaSet", "auditlog-7c8dc556fb", "x", &l), ("deployment".into(), "auditlog".into()));
        assert_eq!(workload_of("Job", "cleanup-29312345", "x", &l), ("cronjob".into(), "cleanup".into()));
        assert_eq!(workload_of("Job", "one-off", "x", &l), ("job".into(), "one-off".into()));
        assert_eq!(workload_of("StatefulSet", "redis", "x", &l), ("statefulset".into(), "redis".into()));
        assert_eq!(workload_of("", "", "lonely", &l), ("pod".into(), "lonely".into()));
        let mut rl = BTreeMap::new();
        rl.insert("rollouts-pod-template-hash".to_string(), "abc".to_string());
        assert_eq!(workload_of("ReplicaSet", "api-5d8f9c7b6", "x", &rl).0, "rollout");
    }
}
