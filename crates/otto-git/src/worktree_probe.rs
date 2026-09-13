//! Optional status hints: bounded admission, response time and cleanup ownership.
use futures_util::{stream::FuturesUnordered, StreamExt};
use otto_core::api::WorktreeInfo;
use std::{future::Future, sync::Arc, time::Duration};
use tokio::{sync::Semaphore, task::JoinHandle};

pub(crate) async fn probe<F, Fut>(
    rows: &mut [WorktreeInfo],
    admission: Arc<Semaphore>,
    budget: Duration,
    check: F,
) where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Option<bool>> + Send + 'static,
{
    let pending: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter_map(|(i, w)| (!w.prunable).then_some(i))
        .collect();
    let mut next = 0;
    let mut running: FuturesUnordered<JoinHandle<(usize, Option<bool>)>> = FuturesUnordered::new();
    let deadline = tokio::time::Instant::now() + budget;
    while next < pending.len() || !running.is_empty() {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => break,
            result = running.next(), if !running.is_empty() => {
                if let Some(Ok((index, Some(dirty)))) = result {
                    rows[index].dirty = dirty;
                    rows[index].dirty_known = true;
                }
            }
            permit = admission.clone().acquire_owned(), if next < pending.len() && running.len() < 4 => {
                let Ok(permit) = permit else { break };
                let index = pending[next]; next += 1;
                let path = rows[index].path.clone(); let check = check.clone();
                // Dropping the response drops these JoinHandles, detaching rather
                // than aborting them. Each bounded command retains admission
                // through the shared runner's process-group cleanup.
                running.push(tokio::spawn(async move {
                    let _permit = permit;
                    (index, check(path).await)
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn rows(count: usize) -> Vec<WorktreeInfo> {
        (0..count)
            .map(|i| WorktreeInfo {
                path: format!("/fixture/{i}"),
                head: String::new(),
                branch: None,
                is_main: i == 0,
                locked: false,
                lock_reason: None,
                prunable: false,
                dirty: false,
                dirty_known: false,
            })
            .collect()
    }

    #[tokio::test]
    async fn worktree_probe_cancellation_keeps_admission_with_actual_work() {
        let admission = Arc::new(Semaphore::new(8));
        let release = Arc::new(Semaphore::new(0));
        let started = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let task = {
            let (admission, release, started, done) = (
                admission.clone(),
                release.clone(),
                started.clone(),
                done.clone(),
            );
            tokio::spawn(async move {
                let mut entries = rows(100);
                probe(
                    &mut entries,
                    admission,
                    Duration::from_secs(10),
                    move |_| {
                        let (release, started, done) =
                            (release.clone(), started.clone(), done.clone());
                        async move {
                            started.fetch_add(1, Ordering::SeqCst);
                            release.acquire().await.unwrap().forget();
                            done.fetch_add(1, Ordering::SeqCst);
                            Some(false)
                        }
                    },
                )
                .await;
            })
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            while started.load(Ordering::SeqCst) < 4 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        assert_eq!(
            started.load(Ordering::SeqCst),
            4,
            "do not spawn one task per worktree"
        );
        assert_eq!(
            admission.available_permits(),
            4,
            "caller cancellation cannot release active work"
        );
        release.add_permits(4);
        tokio::time::timeout(Duration::from_secs(3), async {
            while admission.available_permits() != 8 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(done.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn worktree_probe_queue_wait_counts_against_phase_budget() {
        let mut entries = rows(20);
        probe(
            &mut entries,
            Arc::new(Semaphore::new(0)),
            Duration::from_millis(10),
            |_| async { panic!("no admission") },
        )
        .await;
        assert!(entries.iter().all(|row| !row.dirty_known && !row.dirty));
    }

    #[tokio::test]
    async fn worktree_probe_preserves_order_and_unknown_failures() {
        let mut entries = rows(12);
        probe(
            &mut entries,
            Arc::new(Semaphore::new(8)),
            Duration::from_secs(3),
            |path| async move {
                let i: usize = path.rsplit('/').next().unwrap().parse().unwrap();
                tokio::task::yield_now().await;
                (i != 5).then_some(i % 2 == 1)
            },
        )
        .await;
        for (i, row) in entries.iter().enumerate() {
            assert_eq!(row.path, format!("/fixture/{i}"));
            assert_eq!(row.dirty_known, i != 5);
            assert_eq!(row.dirty, i != 5 && i % 2 == 1);
        }
    }
}
