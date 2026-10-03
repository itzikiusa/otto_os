//! The newest typed pod snapshot per cluster, stamped with the cycle that
//! produced it (perf K3/K4). The collector publishes its `Arc<Snapshot>`
//! after every cycle, so a dashboard cache miss reads it from memory instead
//! of re-parsing the multi-MB `snapshot_json` column; after a daemon restart
//! (nothing published yet) the first miss loads the column once, parses it on
//! the blocking pool and publishes that.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use otto_state::K8sMonitorRepo;

use super::classify::Snapshot;

type Entry = (Option<String>, Arc<Snapshot>);

fn map() -> &'static Mutex<HashMap<String, Entry>> {
    static M: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Publish `snap` as `cluster_id`'s snapshot for the cycle at `stamp`.
pub fn publish(cluster_id: &str, stamp: Option<String>, snap: Arc<Snapshot>) {
    if let Ok(mut m) = map().lock() {
        m.insert(cluster_id.to_string(), (stamp, snap));
    }
}

/// The published snapshot when it belongs to the cycle at `stamp`.
pub fn get(cluster_id: &str, stamp: Option<&str>) -> Option<Arc<Snapshot>> {
    let m = map().lock().ok()?;
    let (at, snap) = m.get(cluster_id)?;
    (at.as_deref() == stamp).then(|| snap.clone())
}

/// Drop a cluster's entry (monitoring reset / cluster removed).
pub fn forget(cluster_id: &str) {
    if let Ok(mut m) = map().lock() {
        m.remove(cluster_id);
    }
}

/// Parse a stored `snapshot_json` (empty on garbage — the collector rebuilds
/// it next cycle).
pub fn parse(raw: &str) -> Snapshot {
    serde_json::from_str(raw).unwrap_or_default()
}

/// `cluster_id`'s snapshot for the cycle at `stamp`: memory first, else the
/// SQLite column parsed off the runtime (and published for the next caller).
pub async fn load(repo: &K8sMonitorRepo, cluster_id: &str, stamp: Option<&str>) -> Arc<Snapshot> {
    if let Some(s) = get(cluster_id, stamp) {
        return s;
    }
    let Ok(Some((raw, at))) = repo.get_snapshot_json(cluster_id).await else {
        return Arc::new(Snapshot::new());
    };
    let snap = tokio::task::spawn_blocking(move || parse(&raw))
        .await
        .map(Arc::new)
        .unwrap_or_default();
    publish(cluster_id, at, snap.clone());
    snap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_must_match() {
        let mut snap = Snapshot::new();
        snap.insert(
            "ns/p".into(),
            super::super::classify::PodSnap {
                namespace: "ns".into(),
                name: "p".into(),
                ..Default::default()
            },
        );
        // What the collector stores parses back to the same snapshot.
        assert_eq!(parse(&serde_json::to_string(&snap).unwrap()), snap);
        publish("latest-t1", Some("a".into()), Arc::new(snap));
        assert!(get("latest-t1", Some("a")).is_some_and(|s| s.len() == 1));
        assert!(
            get("latest-t1", Some("b")).is_none(),
            "an older cycle's snapshot is never served"
        );
        assert!(get("latest-t1", None).is_none());
        forget("latest-t1");
        assert!(get("latest-t1", Some("a")).is_none());
        assert!(parse("not json").is_empty());
    }
}
