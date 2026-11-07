//! Bounded repository-level fanout. Reviewer-level concurrency remains owned
//! by otto-review; this only avoids serial idle waits between repositories.
use futures_util::future::BoxFuture;
use futures_util::{stream, StreamExt};

pub(super) async fn collect<'a, T: Send + 'a>(tasks: Vec<BoxFuture<'a, T>>) -> Vec<T> {
    let mut results = stream::iter(
        tasks
            .into_iter()
            .enumerate()
            .map(|(index, task)| async move { (index, task.await) }),
    )
    .buffer_unordered(2)
    .collect::<Vec<_>>()
    .await;
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, output)| output).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn bounds_overlap_preserves_order_and_releases_canceled_work() {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        struct Running(Arc<AtomicUsize>);
        impl Drop for Running {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let task = |i, delay| {
            let active = active.clone();
            let peak = peak.clone();
            Box::pin(async move {
                let n = active.fetch_add(1, Ordering::SeqCst) + 1;
                let _running = Running(active);
                peak.fetch_max(n, Ordering::SeqCst);
                tokio::time::sleep(delay).await;
                i
            }) as BoxFuture<'static, i32>
        };
        let start = Instant::now();
        let results = collect((0..6).map(|i| task(i, Duration::from_millis(20))).collect()).await;
        let parallel = start.elapsed();
        assert_eq!(results, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        let start = Instant::now();
        for i in 0..6 {
            task(i, Duration::from_millis(20)).await;
        }
        eprintln!(
            "six fixture reviews: bounded={parallel:?}; serial={:?}",
            start.elapsed()
        );
        let cancel = collect((0..6).map(|i| task(i, Duration::from_secs(60))).collect());
        assert!(tokio::time::timeout(Duration::from_millis(10), cancel)
            .await
            .is_err());
        assert_eq!(
            active.load(Ordering::SeqCst),
            0,
            "cancellation drops both active waits"
        );
    }
}
