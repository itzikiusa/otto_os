//! Ownership of managed sessions across dropped creation and turn futures.
use otto_core::{
    api::CreateSessionReq,
    domain::{Session, Workspace},
    Error, Id,
};
use otto_sessions::SessionManager;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock, Weak,
};
use std::time::Duration;

type Lease = tokio::sync::OwnedMutexGuard<()>;

/// Keep a session reserved through asynchronous teardown. A cancelled resume
/// may finish after its caller returns; the successor must wait for its cleanup.
async fn acquire_lease(sid: &str) -> Lease {
    static LEASES: OnceLock<Mutex<HashMap<Id, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let lock = {
        let mut leases = LEASES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        leases.retain(|_, lock| lock.strong_count() > 0);
        let lock = leases
            .get(sid)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| Arc::new(tokio::sync::Mutex::new(())));
        leases.insert(sid.to_owned(), Arc::downgrade(&lock));
        lock
    };
    lock.lock_owned().await
}

pub(super) struct SessionOwnership {
    id: Id,
    _lease: Lease,
}
pub(super) type OwnershipSlot = Arc<Mutex<Option<SessionOwnership>>>;

/// Await cleanup before releasing the reservation, including the detached Drop path.
async fn retire(owner: SessionOwnership, cleanup: impl std::future::Future<Output = ()>) {
    cleanup.await;
    drop(owner);
}

pub(super) struct SessionGuard {
    manager: Arc<SessionManager>,
    pub slot: OwnershipSlot,
    armed: bool,
}
impl SessionGuard {
    pub fn new(manager: Arc<SessionManager>, armed: bool) -> Self {
        Self {
            manager,
            slot: Arc::new(Mutex::new(None)),
            armed,
        }
    }
    pub fn disarm(&mut self) {
        self.armed = false;
    }
    pub fn transfer_to(&mut self, target: &OwnershipSlot) {
        let owned = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        *target.lock().unwrap_or_else(|e| e.into_inner()) = owned;
        self.disarm();
    }
    pub async fn stop(&mut self) {
        let owned = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(owner) = owned {
            let manager = Arc::clone(&self.manager);
            let id = owner.id.clone();
            // If this waiter is dropped, the cleanup task still owns the lease.
            let cleanup = tokio::spawn(retire(owner, async move {
                let _ = manager.kill_session(&id).await;
            }));
            let _ = cleanup.await;
        }
        self.disarm();
    }
}
impl Drop for SessionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let owned = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let (Some(owner), Ok(rt)) = (owned, tokio::runtime::Handle::try_current()) {
            let manager = Arc::clone(&self.manager);
            let id = owner.id.clone();
            rt.spawn(retire(owner, async move {
                let _ = manager.kill_session(&id).await;
            }));
        }
    }
}

/// The queued reply owns cleanup too: send succeeding does not mean a dropped
/// receiver ever accepted the session. Ownership transfers only after on_ready.
pub(super) async fn create(
    manager: Arc<SessionManager>,
    ws: Workspace,
    user: Id,
    req: CreateSessionReq,
) -> otto_core::Result<(Session, SessionGuard)> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = match manager.create(&ws, &user, req, None).await {
            Ok(session) => {
                let lease = acquire_lease(&session.id).await;
                let guard = SessionGuard::new(Arc::clone(&manager), true);
                *guard.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(SessionOwnership {
                    id: session.id.clone(),
                    _lease: lease,
                });
                Ok((session, guard))
            }
            Err(error) => Err(error),
        };
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| Error::Internal("session creation interrupted".into()))?
}

/// A dropped resume future must retire a provider which becomes live later.
pub(super) async fn resume(
    manager: Arc<SessionManager>,
    sid: Id,
) -> otto_core::Result<(Session, SessionGuard)> {
    // Acquire before spawning. Dropping a waiter owns no session and cannot
    // kill the turn currently holding the lease.
    let lease = acquire_lease(&sid).await;
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let guard = SessionGuard::new(Arc::clone(&manager), true);
        *guard.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(SessionOwnership {
            id: sid.clone(),
            _lease: lease,
        });
        let result = async {
            manager.ensure_live(&sid).await?;
            manager.get(&sid).await
        }
        .await
        .map(|session| (session, guard));
        let _ = tx.send(result);
    });
    rx.await
        .map_err(|_| Error::Internal("session resume interrupted".into()))?
}

pub(super) async fn cancelled(flag: Option<Arc<AtomicBool>>) {
    let Some(flag) = flag else {
        std::future::pending::<()>().await;
        return;
    };
    while !flag.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_resume_cleanup_finishes_before_successor_admission() {
        let id = otto_core::new_id();
        let owner = SessionOwnership {
            id: id.clone(),
            _lease: acquire_lease(&id).await,
        };
        let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
        let cleanup = tokio::spawn(retire(owner, async {
            let _ = finish_rx.await;
        }));
        let mut successor = Box::pin(acquire_lease(&id));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut successor)
                .await
                .is_err(),
            "a successor cannot resume while the previous generation can still kill its session"
        );
        finish_tx.send(()).unwrap();
        cleanup.await.unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(1), successor)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn cancelled_admission_waiter_does_not_release_predecessor() {
        let id = otto_core::new_id();
        let predecessor = acquire_lease(&id).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), acquire_lease(&id))
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(20), acquire_lease(&id))
                .await
                .is_err()
        );
        drop(predecessor);
        assert!(
            tokio::time::timeout(Duration::from_secs(1), acquire_lease(&id))
                .await
                .is_ok()
        );
    }
}
