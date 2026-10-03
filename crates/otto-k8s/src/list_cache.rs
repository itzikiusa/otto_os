//! Resources-list cache + single-flight + version (perf K2).
//!
//! `GET /k8s/clusters/{id}/resources` was a fresh `kubectl get <kind> -o json`
//! per call — ~42 MB for 5k pods — paid again by every viewer: the console's
//! 10 s poll, a second window, WorkloadPods, an agent's `k8s_get_resources`.
//! Now identical lists share one kubectl call for [`TTL`] (the caller's access
//! is checked before the cache is consulted; the key carries everything the
//! answer depends on, including whether the caller may see metrics), and the
//! answer carries a content `version` the UI echoes as `If-None-Match`, so an
//! unchanged list is a body-less 304.
//!
//! Also here: [`touch_throttled`] — `last_used_at` written at most once a
//! minute per cluster instead of an SQLite UPDATE on every read.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::Id;
use otto_state::K8sClustersRepo;

use crate::resources::K8sRow;

/// How long one kubectl list answers every identical request. At the UI's
/// poll floor (10 s; big lists poll every ≥ 30 s), so even a single viewer's
/// next poll can be served from here instead of a fresh kubectl list (R3).
pub const TTL: Duration = Duration::from_secs(10);
/// Entries kept at most (expired ones are dropped on every insert).
const CAP: usize = 256;
/// `last_used_at` is written at most this often per cluster.
pub const TOUCH_EVERY: Duration = Duration::from_secs(60);

/// A serialised list answer.
#[derive(Debug)]
pub struct Listed {
    /// Content hash of the items (+ metrics flag), 16 hex chars.
    pub version: String,
    /// The full JSON response body.
    pub body: String,
}

type Entries = HashMap<String, (Instant, Arc<Listed>)>;

fn map() -> &'static Mutex<Entries> {
    static M: OnceLock<Mutex<Entries>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// The cache key of one list request.
pub fn key(
    cluster: &str,
    kind: &str,
    ns: Option<&str>,
    label: Option<&str>,
    q: Option<&str>,
    metrics: bool,
) -> String {
    let t = |s: Option<&str>| s.map(str::trim).unwrap_or("").to_string();
    format!(
        "res:{cluster}\u{1f}{kind}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{metrics}",
        t(ns),
        t(label),
        t(q)
    )
}

/// A fresh cached answer for `key`.
pub fn get(key: &str) -> Option<Arc<Listed>> {
    let m = map().lock().ok()?;
    let (at, v) = m.get(key)?;
    (at.elapsed() < TTL).then(|| v.clone())
}

pub fn put(key: String, v: Arc<Listed>) {
    if let Ok(mut m) = map().lock() {
        m.retain(|_, (at, _)| at.elapsed() < TTL);
        if m.len() >= CAP {
            m.clear();
        }
        m.insert(key, (Instant::now(), v));
    }
}

/// Drop every cached list of `cluster` — after a write (an action changed
/// what the next list would show, so a poll must not be served the old one).
pub fn forget_cluster(cluster: &str) {
    let prefix = format!("res:{cluster}\u{1f}");
    if let Ok(mut m) = map().lock() {
        m.retain(|k, _| !k.starts_with(&prefix));
    }
}

/// Serialise a list answer and stamp its content version (blocking-pool
/// work for a big list: call it inside `spawn_blocking`).
pub fn build(kind: &str, items: &[K8sRow], has_metrics: bool) -> Listed {
    let items_json = serde_json::to_string(items).unwrap_or_else(|_| "[]".into());
    let version = version_of(kind, items, has_metrics);
    let body = format!(
        r#"{{"kind":{},"version":"{version}","has_metrics":{has_metrics},"items":{items_json}}}"#,
        serde_json::Value::from(kind)
    );
    Listed { version, body }
}

/// The content version of a list (perf R1): what the API server says about
/// each object — `uid` + `resourceVersion`, in list order — plus pod metrics
/// QUANTISED (cpu to 10 m, memory to 1 MiB) so metrics-server jitter doesn't
/// churn it. Never the rendered rows: those carry `age_seconds`, which made
/// every rebuild a new version and the 304 path dead. A row without a
/// `resourceVersion` (never from kubectl) falls back to its visible state.
pub fn version_of(kind: &str, items: &[K8sRow], has_metrics: bool) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut h);
    has_metrics.hash(&mut h);
    items.len().hash(&mut h);
    for r in items {
        if r.uid.is_empty() {
            (&r.namespace, &r.name).hash(&mut h);
        } else {
            r.uid.hash(&mut h);
        }
        r.resource_version.hash(&mut h);
        if r.resource_version.is_empty() {
            (&r.status, &r.ready, r.restarts, &r.extra, r.created_at).hash(&mut h);
        }
        r.cpu.map(|c| (c + 5) / 10).hash(&mut h);
        r.mem.map(|m| (m + (1 << 19)) >> 20).hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// Does an `If-None-Match` header value name `version`? Accepts the quoted
/// form, a weak `W/` prefix, a comma list and `*`.
pub fn matches(if_none_match: &str, version: &str) -> bool {
    if_none_match
        .split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.strip_prefix("W/").unwrap_or(t).trim_matches('"') == version)
}

/// Update `last_used_at` at most once per [`TOUCH_EVERY`] per cluster.
pub async fn touch_throttled(repo: &K8sClustersRepo, id: &Id) {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let due = {
        let Ok(mut m) = LAST.get_or_init(Default::default).lock() else {
            return;
        };
        match m.get(id.as_str()) {
            Some(t) if t.elapsed() < TOUCH_EVERY => false,
            _ => {
                m.insert(id.to_string(), Instant::now());
                true
            }
        }
    };
    if due {
        let _ = repo.touch(id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, rv: &str) -> K8sRow {
        let item = serde_json::json!({
            "metadata": {
                "name": name, "namespace": "shop", "uid": format!("uid-{name}"),
                "resourceVersion": rv, "creationTimestamp": "2026-10-01T10:00:00Z"
            },
            "status": { "phase": "Running" }
        });
        crate::resources::normalize(crate::resources::Kind::Pods, &item, chrono::Utc::now())
    }

    #[test]
    fn version_follows_content_and_body_is_valid_json() {
        let a = build("pods", &[row("a", "1")], true);
        let b = build("pods", &[row("a", "1")], true);
        let c = build("pods", &[row("a", "2")], true);
        assert_eq!(a.version, b.version);
        assert_ne!(a.version, c.version, "resourceVersion moves the version");
        assert_ne!(a.version, build("pods", &[row("a", "1")], false).version);
        assert_ne!(
            a.version,
            build("pods", &[row("a", "1"), row("b", "1")], true).version
        );
        let v: serde_json::Value = serde_json::from_str(&a.body).unwrap();
        assert_eq!(v["kind"], "pods");
        assert_eq!(v["version"], a.version.as_str());
        assert_eq!(v["has_metrics"], true);
        assert_eq!(v["items"][0]["name"], "a");
        assert!(
            v["items"][0].get("uid").is_none(),
            "identity stays server-side"
        );
        assert_eq!(v["items"][0]["created_at"], 1_790_848_800);
    }

    /// perf R1 — the live bug: rows normalised seconds apart differ in
    /// `age_seconds`, so a content hash of the rendered rows never matched.
    #[test]
    fn version_is_stable_across_ages_and_quantises_metrics() {
        let item = serde_json::json!({
            "metadata": { "name": "a", "namespace": "shop", "uid": "u1", "resourceVersion": "7",
                          "creationTimestamp": "2026-10-01T10:00:00Z" },
            "status": { "phase": "Running" }
        });
        let now = chrono::Utc::now();
        let kind = crate::resources::Kind::Pods;
        let mut r1 = crate::resources::normalize(kind, &item, now);
        let mut r2 = crate::resources::normalize(kind, &item, now + chrono::Duration::seconds(2));
        assert_ne!(r1.age_seconds, r2.age_seconds);
        assert_eq!(
            build("pods", &[r1.clone()], false).version,
            build("pods", &[r2.clone()], false).version
        );
        // Metrics jitter inside a quantum keeps the version; a real move doesn't.
        r1.cpu = Some(121);
        r1.mem = Some(100 << 20);
        r2.cpu = Some(124);
        r2.mem = Some((100 << 20) + 4096);
        assert_eq!(
            build("pods", &[r1.clone()], true).version,
            build("pods", &[r2.clone()], true).version
        );
        r2.cpu = Some(180);
        assert_ne!(
            build("pods", &[r1], true).version,
            build("pods", &[r2], true).version
        );
    }

    #[test]
    fn if_none_match_forms() {
        assert!(matches("\"abc\"", "abc"));
        assert!(matches("W/\"abc\"", "abc"));
        assert!(matches("\"x\", \"abc\"", "abc"));
        assert!(matches("*", "abc"));
        assert!(!matches("\"abd\"", "abc"));
        assert!(!matches("", "abc"));
    }

    #[test]
    fn entries_expire_and_keys_separate_params() {
        let k1 = key("c", "pods", Some("a"), None, None, true);
        let k2 = key("c", "pods", Some("a"), None, None, false);
        assert_ne!(k1, k2, "metrics visibility is part of the key");
        put(k1.clone(), Arc::new(build("pods", &[], true)));
        assert!(get(&k1).is_some());
        assert!(get(&k2).is_none());
    }
}
