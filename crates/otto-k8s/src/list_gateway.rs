//! The console's per-cluster list gateway (perf R3).
//!
//! A resources list was a `kubectl get <kind> -o json` per cache miss: a
//! process spawn, kubectl decoding the API server's JSON and re-encoding it
//! (~42 MB for 5k pods), then our parse. While a console is open the cluster
//! now gets ONE long-lived, GET-only `kubectl proxy` (collection paths only —
//! see `ACCEPT_PATHS_LIST` in the monitor gateway) and lists are paged
//! straight off the API server (`limit=500&continue=`), each page parsed on
//! the blocking pool. Nobody listing for [`IDLE`] ⇒ the proxy is killed (a
//! console that closed costs nothing). A proxy that cannot start is not
//! retried for [`RETRY_AFTER`]; any gateway failure falls back to kubectl, so
//! errors (RBAC, unknown CRD) keep kubectl's exact mapping.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::cli::Kubectl;
use crate::monitor::gateway::KubeProxy;

/// A gateway nobody listed through for this long is stopped.
pub const IDLE: Duration = Duration::from_secs(120);
/// After a failed start, use kubectl for this long before trying again.
pub const RETRY_AFTER: Duration = Duration::from_secs(300);
/// Items per page (the API server's chunked list).
pub const PAGE: usize = 500;
/// One page's body cap (500 fat pods is ~4 MB).
pub const MAX_PAGE_BYTES: usize = 64 << 20;
/// One page's timeout.
pub const PAGE_TIMEOUT: Duration = Duration::from_secs(30);

enum Slot {
    Up { gw: Arc<KubeProxy>, used: Instant },
    Failed { at: Instant },
}

fn map() -> &'static Mutex<HashMap<String, Slot>> {
    static M: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Drop idle gateways (their children are `kill_on_drop`) and stale
/// failure marks. Returns how many gateways were stopped.
pub fn reap(now: Instant) -> usize {
    let Ok(mut m) = map().lock() else { return 0 };
    let before = m.values().filter(|s| matches!(s, Slot::Up { .. })).count();
    m.retain(|_, s| match s {
        Slot::Up { used, .. } => now.duration_since(*used) < IDLE,
        Slot::Failed { at } => now.duration_since(*at) < RETRY_AFTER,
    });
    before - m.values().filter(|s| matches!(s, Slot::Up { .. })).count()
}

/// Gateways running right now (tests / diagnostics).
pub fn running() -> usize {
    map()
        .lock()
        .map(|m| m.values().filter(|s| matches!(s, Slot::Up { .. })).count())
        .unwrap_or(0)
}

/// One reaper task for the process, started with the first gateway.
fn ensure_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        tokio::spawn(async {
            loop {
                tokio::time::sleep(IDLE / 2).await;
                reap(Instant::now());
            }
        });
    });
}

/// The cluster's list gateway, started on first use; `None` = use kubectl
/// (it failed to start recently, or just now).
pub async fn get(cluster: &str, k: &Kubectl) -> Option<Arc<KubeProxy>> {
    let lookup = |m: &mut HashMap<String, Slot>| -> Option<Option<Arc<KubeProxy>>> {
        match m.get_mut(cluster) {
            Some(Slot::Up { gw, used }) if gw.usable_for(k) => {
                *used = Instant::now();
                Some(Some(gw.clone()))
            }
            Some(Slot::Failed { at }) if at.elapsed() < RETRY_AFTER => Some(None),
            _ => None,
        }
    };
    if let Some(hit) = lookup(&mut *map().lock().ok()?) {
        return hit;
    }
    // One start per cluster at a time; the others wait and reuse it.
    let _flight = crate::monitor::cache::flight(&format!("listgw:{cluster}")).await;
    if let Some(hit) = lookup(&mut *map().lock().ok()?) {
        return hit;
    }
    let started = KubeProxy::start_lists(k).await;
    let mut m = map().lock().ok()?;
    match started {
        Ok(gw) => {
            let gw = Arc::new(gw);
            m.insert(
                cluster.to_string(),
                Slot::Up {
                    gw: gw.clone(),
                    used: Instant::now(),
                },
            );
            drop(m);
            ensure_reaper();
            Some(gw)
        }
        Err(e) => {
            tracing::debug!(cluster, "k8s list gateway unavailable, using kubectl: {e}");
            m.insert(cluster.to_string(), Slot::Failed { at: Instant::now() });
            None
        }
    }
}

/// Stop `cluster`'s gateway (it answered nonsense / the cluster changed).
pub fn forget(cluster: &str) {
    if let Ok(mut m) = map().lock() {
        m.remove(cluster);
    }
}

/// Percent-encode a query value (RFC 3986 unreserved kept).
pub fn encode_query(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_selectors_are_encoded() {
        assert_eq!(encode_query("app=web"), "app%3Dweb");
        assert_eq!(
            encode_query("tier in (a,b),!x"),
            "tier%20in%20%28a%2Cb%29%2C%21x"
        );
        assert_eq!(encode_query("a.b-c_d~"), "a.b-c_d~");
    }

    #[test]
    fn reap_drops_idle_gateways_and_stale_failures() {
        let now = Instant::now();
        map().lock().unwrap().insert(
            "t-reap".into(),
            Slot::Failed {
                at: now - RETRY_AFTER - Duration::from_secs(1),
            },
        );
        reap(now);
        assert!(!map().lock().unwrap().contains_key("t-reap"));
    }
}
