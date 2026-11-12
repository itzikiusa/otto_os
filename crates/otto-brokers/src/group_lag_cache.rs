//! Total lag per group on the Groups LIST. Each group's lag is an OffsetFetch
//! through a group-scoped client plus a watermark pass, so a cluster with
//! hundreds of groups must not compute them inline: one background job per
//! cluster runs at most [`LAG_WORKERS`] groups at a time, results are cached
//! for [`LAG_TTL`], and the list waits at most a short budget for them. A
//! group that is not done yet comes back `lag_pending` (the UI re-fetches); a
//! group whose lag could not be computed is `None` ("—"), never 0.

use crate::types::GroupSummary;
use dashmap::DashMap;
use otto_core::{Id, Result};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

/// How long a computed lag (or "unknown") is served before recomputing.
pub(crate) const LAG_TTL: Duration = Duration::from_secs(30);
/// Groups computed concurrently per cluster (each holds a librdkafka client).
pub(crate) const LAG_WORKERS: usize = 4;
/// How long a list request waits for the lag job before answering.
pub(crate) const LAG_WAIT: Duration = Duration::from_secs(3);

type Key = (Id, String);

#[derive(Default)]
pub(crate) struct GroupLagCache {
    entries: DashMap<Key, (Instant, Option<i64>)>,
    /// Clusters with a lag job in flight (single-flight per cluster).
    running: DashMap<Id, ()>,
}

/// Only a consumer group (or a simple `assign()` group, whose protocol type is
/// empty) has committed offsets to measure; Connect / other protocols don't.
fn has_consumer_offsets(g: &GroupSummary) -> bool {
    matches!(g.protocol_type.as_str(), "consumer" | "")
}

impl GroupLagCache {
    /// Drop a cluster's cached lags (edit/delete: other credentials or brokers).
    pub(crate) fn forget_cluster(&self, id: &Id) {
        self.entries.retain(|(cid, _), _| cid != id);
    }

    fn fresh(&self, id: &Id, group: &str, now: Instant) -> Option<Option<i64>> {
        self.entries
            .get(&(id.clone(), group.to_string()))
            .filter(|e| now.saturating_duration_since(e.0) < LAG_TTL)
            .map(|e| e.1)
    }

    /// Fill `total_lag` / `lag_pending` on `groups`. Stale groups start one
    /// background job (unless one is already running for the cluster) that
    /// keeps going after `wait`, so the next list load finds its results.
    /// `partitions` runs once per job; `lag` once per group, on blocking threads.
    pub(crate) async fn fill<P, L>(
        self: &Arc<Self>,
        id: &Id,
        groups: &mut [GroupSummary],
        partitions: P,
        lag: L,
        wait: Duration,
    ) where
        P: FnOnce() -> Result<Vec<(String, i32)>> + Send + 'static,
        L: Fn(&str, &[(String, i32)]) -> Result<Option<i64>> + Send + Sync + 'static,
    {
        let now = Instant::now();
        let stale: Vec<String> = groups
            .iter()
            .filter(|g| has_consumer_offsets(g) && self.fresh(id, &g.group_id, now).is_none())
            .map(|g| g.group_id.clone())
            .collect();
        if !stale.is_empty() && self.running.insert(id.clone(), ()).is_none() {
            let job = tokio::spawn(self.clone().compute(id.clone(), stale, partitions, lag));
            // On timeout the job is detached, not cancelled.
            let _ = tokio::time::timeout(wait, job).await;
        }
        let running = self.running.contains_key(id);
        let now = Instant::now();
        for g in groups.iter_mut().filter(|g| has_consumer_offsets(g)) {
            match self.fresh(id, &g.group_id, now) {
                Some(v) => g.total_lag = v,
                None => g.lag_pending = running,
            }
        }
    }

    async fn compute<P, L>(self: Arc<Self>, id: Id, groups: Vec<String>, partitions: P, lag: L)
    where
        P: FnOnce() -> Result<Vec<(String, i32)>> + Send + 'static,
        L: Fn(&str, &[(String, i32)]) -> Result<Option<i64>> + Send + Sync + 'static,
    {
        let parts = match tokio::task::spawn_blocking(partitions).await {
            Ok(Ok(p)) => Arc::new(p),
            // No partition list: every group is unknown for this TTL.
            _ => {
                let now = Instant::now();
                for g in groups {
                    self.entries.insert((id.clone(), g), (now, None));
                }
                self.running.remove(&id);
                return;
            }
        };
        let lag = Arc::new(lag);
        let sem = Arc::new(Semaphore::new(LAG_WORKERS));
        let mut set = tokio::task::JoinSet::new();
        for g in groups {
            let (me, id, sem, lag, parts) = (
                self.clone(),
                id.clone(),
                sem.clone(),
                lag.clone(),
                parts.clone(),
            );
            set.spawn(async move {
                let Ok(_permit) = sem.acquire_owned().await else {
                    return;
                };
                let name = g.clone();
                let v =
                    tokio::task::spawn_blocking(move || (*lag)(name.as_str(), parts.as_slice()))
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .flatten();
                me.entries.insert((id, g), (Instant::now(), v));
            });
        }
        while set.join_next().await.is_some() {}
        self.running.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::Error;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn group(id: &str, protocol_type: &str) -> GroupSummary {
        GroupSummary {
            group_id: id.into(),
            state: "Stable".into(),
            protocol_type: protocol_type.into(),
            members: 1,
            total_lag: None,
            lag_pending: false,
        }
    }

    fn parts() -> Result<Vec<(String, i32)>> {
        Ok(vec![("t".into(), 0)])
    }

    #[tokio::test]
    async fn list_lag_is_filled_and_failures_read_unknown_not_zero() {
        let cache = Arc::new(GroupLagCache::default());
        let id = Id::from("c1");
        let mut groups = vec![
            group("ok", "consumer"),
            group("simple", ""),
            group("broken", "consumer"),
            group("connect-cluster", "connect"),
        ];
        let lag = |g: &str, _: &[(String, i32)]| match g {
            "ok" => Ok(Some(42)),
            "simple" => Ok(Some(0)),
            _ => Err(Error::Upstream("offsets unavailable".into())),
        };
        cache
            .fill(&id, &mut groups, parts, lag, Duration::from_secs(10))
            .await;
        let got: Vec<_> = groups
            .iter()
            .map(|g| (g.group_id.as_str(), g.total_lag, g.lag_pending))
            .collect();
        assert_eq!(
            got,
            vec![
                ("ok", Some(42), false),
                ("simple", Some(0), false),
                ("broken", None, false),
                ("connect-cluster", None, false),
            ]
        );
    }

    #[tokio::test]
    async fn cached_lag_is_reused_within_the_ttl() {
        let cache = Arc::new(GroupLagCache::default());
        let id = Id::from("c1");
        let calls = Arc::new(AtomicUsize::new(0));
        for _ in 0..3 {
            let mut groups = vec![group("g", "consumer")];
            let c = calls.clone();
            let lag = move |_: &str, _: &[(String, i32)]| {
                c.fetch_add(1, Ordering::SeqCst);
                Ok(Some(7))
            };
            cache
                .fill(&id, &mut groups, parts, lag, Duration::from_secs(10))
                .await;
            assert_eq!(groups[0].total_lag, Some(7));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        cache.forget_cluster(&id);
        assert!(cache.fresh(&id, "g", Instant::now()).is_none());
    }

    #[tokio::test]
    async fn many_groups_run_with_bounded_concurrency() {
        let cache = Arc::new(GroupLagCache::default());
        let id = Id::from("c1");
        let mut groups: Vec<_> = (0..16)
            .map(|i| group(&format!("g{i}"), "consumer"))
            .collect();
        let (now_in, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (n, p) = (now_in.clone(), peak.clone());
        let lag = move |_: &str, _: &[(String, i32)]| {
            let cur = n.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(cur, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(20));
            n.fetch_sub(1, Ordering::SeqCst);
            Ok(Some(1))
        };
        cache
            .fill(&id, &mut groups, parts, lag, Duration::from_secs(10))
            .await;
        assert!(groups.iter().all(|g| g.total_lag == Some(1)));
        let peak = peak.load(Ordering::SeqCst);
        assert!((1..=LAG_WORKERS).contains(&peak), "peak concurrency {peak}");
    }

    #[tokio::test]
    async fn slow_lag_answers_pending_then_lands_for_the_next_load() {
        let cache = Arc::new(GroupLagCache::default());
        let id = Id::from("c1");
        let mut groups = vec![group("slow", "consumer")];
        let lag = |_: &str, _: &[(String, i32)]| {
            std::thread::sleep(Duration::from_millis(200));
            Ok(Some(5))
        };
        cache
            .fill(&id, &mut groups, parts, lag, Duration::from_millis(10))
            .await;
        assert_eq!((groups[0].total_lag, groups[0].lag_pending), (None, true));

        // The detached job keeps going; a later load serves its result.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let mut again = vec![group("slow", "consumer")];
        let unused = |_: &str, _: &[(String, i32)]| -> Result<Option<i64>> {
            panic!("cached lag must not be recomputed")
        };
        cache
            .fill(&id, &mut again, parts, unused, Duration::from_millis(10))
            .await;
        assert_eq!((again[0].total_lag, again[0].lag_pending), (Some(5), false));
    }

    #[tokio::test]
    async fn unreadable_partition_list_marks_groups_unknown() {
        let cache = Arc::new(GroupLagCache::default());
        let id = Id::from("c1");
        let mut groups = vec![group("g", "consumer")];
        let no_parts = || -> Result<Vec<(String, i32)>> { Err(Error::Upstream("metadata".into())) };
        let lag = |_: &str, _: &[(String, i32)]| Ok(Some(1));
        cache
            .fill(&id, &mut groups, no_parts, lag, Duration::from_secs(10))
            .await;
        assert_eq!((groups[0].total_lag, groups[0].lag_pending), (None, false));
    }
}
