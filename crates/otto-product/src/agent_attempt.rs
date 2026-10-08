//! Exclusive ownership of an analysis agent through final result publication.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

fn lock(agent: &str) -> Arc<AsyncMutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap();
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(agent).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(agent.to_owned(), Arc::downgrade(&lock));
    lock
}

/// Claim before spawning, and keep the guard until all row writes finish.
pub(crate) fn claim(agent: &str) -> otto_core::Result<OwnedMutexGuard<()>> {
    lock(agent).try_lock_owned().map_err(|_| {
        otto_core::Error::Conflict("this analysis agent is still running or stopping".into())
    })
}

/// Stop first signals cancellation, then drains the owner before publishing.
pub(crate) async fn drain(agent: &str) -> OwnedMutexGuard<()> {
    lock(agent).lock_owned().await
}

/// Registration can happen after Stop arrives (before provider startup). Keep
/// signalling while the current owner drains so that admission gap cannot run
/// an uncancelled turn. The queued mutex waiter also bars new retries.
pub(crate) async fn stop<F, Fut>(
    reg: &crate::run::CancelRegistry,
    agent: &str,
    mut stop_session: F,
) -> OwnedMutexGuard<()>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let drain = drain(agent);
    tokio::pin!(drain);
    loop {
        crate::run::signal_cancel(reg, agent);
        tokio::select! {
            biased;
            owner = &mut drain => return owner,
            _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                // The host observes cancellation between attempts; interrupt its
                // active watcher too, including sessions published after Stop.
                stop_session().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stop_interrupts_active_work_before_waiting_for_publication() {
        let id = otto_core::new_id();
        let owner = claim(&id).unwrap();
        let reg = crate::run::new_cancel_registry();
        let killed = Arc::new(tokio::sync::Notify::new());
        let active = killed.clone();
        let task = tokio::spawn(async move {
            let _owner = owner;
            active.notified().await;
            // The watcher exits only after its session is killed.
        });
        let _stopped = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            stop(&reg, &id, || async {
                killed.notify_one();
            }),
        )
        .await
        .unwrap();
        task.await.unwrap();
        assert!(claim(&id).is_err());
    }

    #[tokio::test]
    async fn stop_cancels_a_provider_registered_after_stop_arrived() {
        let id = otto_core::new_id();
        let owner = claim(&id).unwrap();
        let reg = crate::run::new_cancel_registry();
        let mut stopping = Box::pin(stop(&reg, &id, || async {}));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut stopping)
                .await
                .is_err()
        );
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        reg.lock().unwrap().insert(id.clone(), flag.clone());
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(60), &mut stopping)
                .await
                .is_err()
        );
        assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
        drop(owner);
        let _stopped = stopping.await;
        assert!(claim(&id).is_err());
    }

    #[tokio::test]
    async fn retry_stays_exclusive_through_result_publication_and_stop() {
        let id = otto_core::new_id();
        let owner = claim(&id).unwrap();
        // The provider has returned, but the caller is awaiting its final DB write.
        tokio::task::yield_now().await;
        assert!(claim(&id).is_err());
        let mut stop = Box::pin(drain(&id));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(5), &mut stop)
                .await
                .is_err()
        );
        drop(owner);
        // Queued Stop takes priority over a new Retry, including during its writes.
        assert!(claim(&id).is_err());
        let stop = stop.await;
        assert!(claim(&id).is_err());
        drop(stop);
        assert!(claim(&id).is_ok());
    }

    #[tokio::test]
    async fn aborted_owner_releases_admission_and_other_agents_are_independent() {
        let id = otto_core::new_id();
        let guard = claim(&id).unwrap();
        let _other = claim(&otto_core::new_id()).unwrap();
        let (ready, wait) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = guard;
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        wait.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(claim(&id).is_ok());
    }
}
