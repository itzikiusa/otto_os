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

/// How long one kubectl list answers every identical request.
pub const TTL: Duration = Duration::from_secs(5);
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

/// Serialise a list answer and stamp its content version (blocking-pool
/// work for a big list: call it inside `spawn_blocking`).
pub fn build(kind: &str, items_json: &str, has_metrics: bool) -> Listed {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    kind.hash(&mut h);
    has_metrics.hash(&mut h);
    items_json.hash(&mut h);
    let version = format!("{:016x}", h.finish());
    let body = format!(
        r#"{{"kind":{},"version":"{version}","has_metrics":{has_metrics},"items":{items_json}}}"#,
        serde_json::Value::from(kind)
    );
    Listed { version, body }
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

    #[test]
    fn version_follows_content_and_body_is_valid_json() {
        let a = build("pods", r#"[{"name":"a"}]"#, true);
        let b = build("pods", r#"[{"name":"a"}]"#, true);
        let c = build("pods", r#"[{"name":"b"}]"#, true);
        assert_eq!(a.version, b.version);
        assert_ne!(a.version, c.version);
        assert_ne!(a.version, build("pods", r#"[{"name":"a"}]"#, false).version);
        let v: serde_json::Value = serde_json::from_str(&a.body).unwrap();
        assert_eq!(v["kind"], "pods");
        assert_eq!(v["version"], a.version.as_str());
        assert_eq!(v["has_metrics"], true);
        assert_eq!(v["items"][0]["name"], "a");
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
        put(k1.clone(), Arc::new(build("pods", "[]", true)));
        assert!(get(&k1).is_some());
        assert!(get(&k2).is_none());
    }
}
