//! Kubernetes-monitor supervisor: keeps exactly one collector loop
//! (`otto_k8s::monitor::collector::run_loop`) alive per **enabled** cluster.
//!
//! Every `SCAN` it reconciles the running set against
//! `k8s_monitor_configs.enabled = 1`: a newly enabled cluster is started, a
//! disabled/deleted one is cancelled, and a cluster whose config `updated_at`
//! changed is restarted so the new probes / interval take effect immediately.
//! A `k8s_cluster_updated { deleted: true }` event cancels without waiting for
//! the next scan. Loops observe their own cancel flag every second.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::event::Event;
use otto_core::Id;
use otto_state::K8sMonitorRepo;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::state::ServerCtx;

const SCAN: Duration = Duration::from_secs(15);
const SLICE: Duration = Duration::from_millis(500);

struct Running {
    handle: JoinHandle<()>,
    cancel: Arc<AtomicBool>,
    updated_at: String,
}

/// What a scan should do — pure so it can be unit-tested.
#[derive(Debug, Default, PartialEq)]
pub struct Reconcile {
    pub stop: Vec<String>,
    pub start: Vec<(String, String)>,
}

/// `running`: cluster_id → config `updated_at` of the loop we spawned.
/// `enabled`: (cluster_id, updated_at) rows that should be running.
pub fn reconcile(running: &HashMap<String, String>, enabled: &[(String, String)]) -> Reconcile {
    let mut r = Reconcile::default();
    let want: HashMap<&str, &str> = enabled.iter().map(|(id, u)| (id.as_str(), u.as_str())).collect();
    let mut stop: Vec<String> = running
        .iter()
        .filter(|(id, u)| want.get(id.as_str()).map(|w| *w != u.as_str()).unwrap_or(true))
        .map(|(id, _)| id.clone())
        .collect();
    stop.sort();
    r.stop = stop;
    r.start = enabled
        .iter()
        .filter(|(id, u)| running.get(id).map(|cur| cur != u).unwrap_or(true))
        .cloned()
        .collect();
    r
}

pub fn start(ctx: ServerCtx) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: Arc<AtomicBool>) {
    let repo = K8sMonitorRepo::new(ctx.pool.clone());
    let mut running: HashMap<String, Running> = HashMap::new();
    let mut events = ctx.events.subscribe();
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // Reap finished loops (config disabled from inside, cluster gone).
        running.retain(|id, r| {
            if r.handle.is_finished() {
                info!(cluster = %id, "k8s monitor: loop ended");
                false
            } else {
                true
            }
        });
        match repo.list_enabled().await {
            Ok(rows) => {
                let enabled: Vec<(String, String)> = rows
                    .into_iter()
                    .map(|r| (r.cluster_id, r.updated_at))
                    .collect();
                let snapshot: HashMap<String, String> = running
                    .iter()
                    .map(|(id, r)| (id.clone(), r.updated_at.clone()))
                    .collect();
                let plan = reconcile(&snapshot, &enabled);
                for id in plan.stop {
                    if let Some(r) = running.remove(&id) {
                        info!(cluster = %id, "k8s monitor: stopping loop");
                        r.cancel.store(true, Ordering::Relaxed);
                        r.handle.abort();
                    }
                }
                for (id, updated_at) in plan.start {
                    info!(cluster = %id, "k8s monitor: starting loop");
                    let flag = Arc::new(AtomicBool::new(false));
                    let cluster_id: Id = id.clone();
                    let handle = tokio::spawn(otto_k8s::monitor::collector::run_loop(
                        ctx.clone(),
                        cluster_id,
                        flag.clone(),
                    ));
                    running.insert(
                        id,
                        Running {
                            handle,
                            cancel: flag,
                            updated_at,
                        },
                    );
                }
            }
            Err(e) => warn!("k8s monitor scheduler: {e}"),
        }

        // Sleep in slices; react early to cluster deletions.
        let mut waited = Duration::ZERO;
        while waited < SCAN {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(SLICE) => { waited += SLICE; }
                ev = events.recv() => {
                    if let Ok(Event::K8sClusterUpdated { cluster_id, deleted: true }) = ev {
                        if let Some(r) = running.remove(cluster_id.as_str()) {
                            info!(cluster = %cluster_id, "k8s monitor: cluster deleted; stopping loop");
                            r.cancel.store(true, Ordering::Relaxed);
                            r.handle.abort();
                        }
                    }
                }
            }
        }
    }
    for (_, r) in running.drain() {
        r.cancel.store(true, Ordering::Relaxed);
        r.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_starts_stops_and_restarts_on_change() {
        let running: HashMap<String, String> =
            [("a".to_string(), "t1".to_string()), ("b".to_string(), "t1".to_string())].into();
        let enabled = vec![("a".to_string(), "t2".to_string()), ("c".to_string(), "t1".to_string())];
        let d = reconcile(&running, &enabled);
        assert_eq!(d.stop, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(
            d.start,
            vec![("a".to_string(), "t2".to_string()), ("c".to_string(), "t1".to_string())]
        );
    }

    #[test]
    fn reconcile_is_a_noop_when_in_sync() {
        let running: HashMap<String, String> = [("a".to_string(), "t1".to_string())].into();
        let d = reconcile(&running, &[("a".to_string(), "t1".to_string())]);
        assert_eq!(d, Reconcile::default());
    }
}
