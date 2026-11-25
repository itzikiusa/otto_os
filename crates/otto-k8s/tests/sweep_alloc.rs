//! P2-016: the monitor sweep's pod-list parse peak. Its own test binary:
//! the counting global allocator must not see other tests' allocations.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use otto_k8s::monitor::classify::snapshot_from_pod_json;
use serde_json::json;

struct Counting;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grow(n: usize) {
    let c = CUR.fetch_add(n, Relaxed) + n;
    PEAK.fetch_max(c, Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            grow(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        CUR.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            if n > l.size() {
                grow(n - l.size());
            } else {
                CUR.fetch_sub(l.size() - n, Relaxed);
            }
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

/// A pod as `kubectl get -o json` prints it (no managedFields): env,
/// volumes, mounts and probes are most of the bytes.
fn pod(i: usize) -> serde_json::Value {
    let env: Vec<_> = (0..60)
        .map(|e| json!({"name": format!("ENV_VAR_NUMBER_{e}"), "value": format!("some-config-value-{e}-{i}")}))
        .collect();
    let vols: Vec<_> = (0..8)
        .map(|v| json!({"name": format!("vol-{v}"), "configMap": {"name": format!("cm-{v}"), "defaultMode": 420}}))
        .collect();
    let mounts: Vec<_> = (0..8)
        .map(|v| json!({"name": format!("vol-{v}"), "mountPath": format!("/etc/app/conf-{v}"), "readOnly": true}))
        .collect();
    let probe = json!({"httpGet": {"path": "/actuator/health", "port": 9000}, "periodSeconds": 10, "failureThreshold": 3});
    json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": format!("api-{:03}-5d4c-{i:05}", i / 10), "namespace": "shop",
            "uid": format!("u-{i}"), "creationTimestamp": "2026-09-01T10:00:00Z",
            "labels": {"app": format!("api-{:03}", i / 10), "pod-template-hash": "5d4c"},
            "ownerReferences": [{"kind": "ReplicaSet", "name": format!("api-{:03}-5d4c", i / 10), "controller": true}]
        },
        "spec": {
            "nodeName": "ip-10-0-1-5", "volumes": vols,
            "containers": [{"name": "api", "image": "registry/api:1.0.0", "env": env, "volumeMounts": mounts,
                            "livenessProbe": probe.clone(), "readinessProbe": probe,
                            "ports": [{"containerPort": 9000}],
                            "resources": {"limits": {"memory": "512Mi"}, "requests": {"cpu": "100m"}}}]
        },
        "status": {
            "phase": "Running",
            "conditions": [{"type": "Ready", "status": "True", "lastTransitionTime": "2026-09-01T10:00:05Z"}],
            "containerStatuses": [{"name": "api", "ready": true, "restartCount": 0,
                                   "state": {"running": {"startedAt": "2026-09-01T10:00:05Z"}}}]
        }
    })
}

/// A whole-list `serde_json::Value` was ~11x the JSON text (0.7 MB of pods
/// → 7.8 MB DOM), times up to 4 namespaces parsed at once; the typed parse
/// reads only what the snapshot needs.
#[test]
fn pod_list_parse_peak_stays_near_the_json_size() {
    let text = json!({"items": (0..100).map(pod).collect::<Vec<_>>()}).to_string();
    let base = CUR.load(Relaxed);
    PEAK.store(base, Relaxed);
    let snap = snapshot_from_pod_json(&text).unwrap();
    let peak = PEAK.load(Relaxed) - base;
    assert_eq!(snap.len(), 100);
    assert_eq!(snap["shop/api-000-5d4c-00000"].mem_limit, 512 << 20);
    eprintln!("pod list {} bytes, parse peak {peak} bytes", text.len());
    assert!(
        peak < text.len() * 2,
        "parse peak {peak} B for {} B of JSON",
        text.len()
    );
}
