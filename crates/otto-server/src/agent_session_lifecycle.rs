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
    /// Whether a failed turn retires (kills) the session. Only a session this
    /// turn CREATED is the turn's to kill; a resumed one belongs to its thread
    /// (Discovery chat, DB Assist, a design or vault thread) and is released
    /// alive, so the next message resumes it and the user can open it.
    retire: bool,
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
    /// End a failed turn. A created session is killed. A resumed one is
    /// released alive, except that an owner's Stop (`cancelled`) suspends it:
    /// its work stops and the conversation stays resumable.
    pub async fn stop(&mut self, cancelled: bool) {
        let owned = self.slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(owner) = owned {
            let manager = Arc::clone(&self.manager);
            let id = owner.id.clone();
            let (kill, suspend) = (owner.retire, !owner.retire && cancelled);
            // If this waiter is dropped, the cleanup task still owns the lease.
            let cleanup = tokio::spawn(retire(owner, async move {
                if kill {
                    let _ = manager.kill_session(&id).await;
                } else if suspend {
                    let _ = manager.suspend(&id).await;
                }
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
        // A resumed session is only released (dropping `owner` frees the lease).
        let owned = owned.filter(|owner| owner.retire);
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
                    retire: true,
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

/// A dropped resume future releases its reservation once the resume settles.
/// The resumed session itself is never retired: it belongs to its thread.
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
            retire: false,
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

/// Only caller-cancelled turns use tracked startup. A queued resume does not
/// register a task until it has acquired the session lease, so cancelling a
/// waiter neither waits for nor stops its predecessor.
#[derive(Default)]
pub(super) struct Startup(Mutex<Option<tokio::task::JoinHandle<()>>>);
impl Startup {
    async fn run<F>(
        &self,
        owner: OwnershipSlot,
        cancelled: Arc<AtomicBool>,
        work: F,
    ) -> otto_core::Result<Session>
    where
        F: std::future::Future<Output = otto_core::Result<(Session, SessionGuard)>>
            + Send
            + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            match work.await {
                Ok((session, mut guard)) => {
                    // Publish ownership before the reply can be accepted OR
                    // dropped. No queued-result guard can escape the drain.
                    guard.transfer_to(&owner);
                    guard.slot = owner;
                    guard.armed = true;
                    if tx.send(Ok(session)).is_err() {
                        guard.stop(cancelled.load(Ordering::Acquire)).await;
                    } else {
                        guard.disarm();
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                }
            }
        });
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
        rx.await
            .map_err(|_| Error::Internal("tracked session startup interrupted".into()))?
    }
    pub async fn drain(&self) {
        let task = self.0.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
}

pub(super) async fn create_tracked(
    manager: Arc<SessionManager>,
    ws: Workspace,
    user: Id,
    req: CreateSessionReq,
    startup: &Startup,
    owner: OwnershipSlot,
    cancelled: Arc<AtomicBool>,
) -> otto_core::Result<Session> {
    startup
        .run(owner, cancelled, async move {
            create(manager, ws, user, req).await
        })
        .await
}

pub(super) async fn resume_tracked(
    manager: Arc<SessionManager>,
    sid: Id,
    startup: &Startup,
    owner: OwnershipSlot,
    cancelled: Arc<AtomicBool>,
) -> otto_core::Result<Session> {
    let lease = acquire_lease(&sid).await;
    if cancelled.load(Ordering::Acquire) {
        return Err(Error::Conflict("agent turn stopped before resume".into()));
    }
    startup
        .run(owner, cancelled, async move {
            let guard = SessionGuard::new(Arc::clone(&manager), true);
            *guard.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(SessionOwnership {
                id: sid.clone(),
                _lease: lease,
                retire: false,
            });
            manager.ensure_live(&sid).await?;
            let session = manager.get(&sid).await?;
            Ok((session, guard))
        })
        .await
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
            retire: true,
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

    async fn live_shell(
        ctx: &crate::state::ServerCtx,
        ws: &Workspace,
        user: &Id,
        dir: &std::path::Path,
    ) -> Id {
        let req = CreateSessionReq {
            kind: otto_core::domain::SessionKind::Agent,
            provider: Some("shell".into()),
            title: Some("thread".into()),
            cwd: None,
            connection_id: None,
            model: None,
            meta: None,
        };
        let spec = otto_pty::CommandSpec {
            program: "/bin/sh".into(),
            args: vec![],
            cwd: Some(dir.to_string_lossy().into()),
            env: vec![],
        };
        ctx.manager
            .create(ws, user, req, Some(spec))
            .await
            .unwrap()
            .id
    }

    /// S-1 (sweep 2026-10-11): a failed turn on a RESUMED session (Discovery
    /// chat, DB Assist) used to kill the user's session; only a session the
    /// turn created is its to retire.
    #[tokio::test]
    async fn failed_turn_releases_a_resumed_session_and_retires_only_its_own() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = crate::state::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("u", "x", "U", false)
            .await
            .unwrap();
        let ws = ctx
            .workspaces
            .create("w", tmp.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let sessions = otto_state::SessionsRepo::new(pool.clone());
        let exited = |id: Id| {
            let sessions = sessions.clone();
            async move {
                sessions.get(&id).await.unwrap().status == otto_core::domain::SessionStatus::Exited
            }
        };

        let thread = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
        for cancelled in [false, true] {
            ctx.manager.ensure_live(&thread).await.unwrap();
            let (_, mut resumed) = resume(Arc::clone(&ctx.manager), thread.clone())
                .await
                .unwrap();
            let mut turn = SessionGuard::new(Arc::clone(&ctx.manager), true);
            resumed.transfer_to(&turn.slot);
            turn.stop(cancelled).await;
            assert!(!exited(thread.clone()).await, "cancelled={cancelled}");
            if !cancelled {
                assert!(ctx.manager.is_live(&thread), "a failed turn leaves it open");
            }
            // The reservation is released: the next message resumes at once.
            assert!(
                tokio::time::timeout(Duration::from_secs(1), acquire_lease(&thread))
                    .await
                    .is_ok()
            );
        }

        let created = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
        let mut turn = SessionGuard::new(Arc::clone(&ctx.manager), true);
        *turn.slot.lock().unwrap() = Some(SessionOwnership {
            id: created.clone(),
            _lease: acquire_lease(&created).await,
            retire: true,
        });
        turn.stop(false).await;
        assert!(
            exited(created).await,
            "a session the turn created is retired"
        );
    }

    #[tokio::test]
    async fn tracked_startup_cancellation_drains_created_and_resumed_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = crate::state::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("startup", "x", "U", false)
            .await
            .unwrap();
        let ws = ctx
            .workspaces
            .create("w", tmp.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        for retire in [true, false] {
            let id = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
            let session = ctx.manager.get(&id).await.unwrap();
            let manager = Arc::clone(&ctx.manager);
            let lease = acquire_lease(&id).await;
            let work_id = id.clone();
            let flag = Arc::new(AtomicBool::new(false));
            let startup = Startup::default();
            let mut owner = SessionGuard::new(Arc::clone(&ctx.manager), true);
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            let (finish_tx, finish_rx) = tokio::sync::oneshot::channel();
            let mut admission =
                Box::pin(startup.run(owner.slot.clone(), flag.clone(), async move {
                    let guard = SessionGuard::new(manager, true);
                    *guard.slot.lock().unwrap() = Some(SessionOwnership {
                        id: work_id,
                        _lease: lease,
                        retire,
                    });
                    ready_tx.send(()).unwrap();
                    // Force cancellation while startup owns the process, before
                    // its result/ownership reaches the caller.
                    finish_rx.await.unwrap();
                    Ok((session, guard))
                }));
            tokio::select! {
                _ = &mut admission => panic!("startup must wait at the barrier"),
                _ = ready_rx => {}
            }
            flag.store(true, Ordering::Release);
            drop(admission);
            let mut drain = Box::pin(startup.drain());
            assert!(tokio::time::timeout(Duration::from_millis(5), &mut drain)
                .await
                .is_err());
            finish_tx.send(()).unwrap();
            drain.await;
            owner.stop(true).await;
            assert!(
                !ctx.manager.is_live(&id),
                "no live process after cancellation drain: retire={retire}"
            );
            assert!(owner.slot.lock().unwrap().is_none());
            assert!(
                startup.0.lock().unwrap().is_none(),
                "no background startup remains"
            );
            assert!(
                tokio::time::timeout(Duration::from_secs(1), acquire_lease(&id))
                    .await
                    .is_ok()
            );
        }
    }

    #[tokio::test]
    async fn queued_tracked_resume_cancellation_does_not_stop_the_owner() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = crate::state::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("queued", "x", "U", false)
            .await
            .unwrap();
        let ws = ctx
            .workspaces
            .create("w", tmp.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let id = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
        let predecessor = acquire_lease(&id).await;
        let startup = Startup::default();
        let mut owner = SessionGuard::new(Arc::clone(&ctx.manager), true);
        let flag = Arc::new(AtomicBool::new(false));
        let queued = resume_tracked(
            Arc::clone(&ctx.manager),
            id.clone(),
            &startup,
            owner.slot.clone(),
            flag.clone(),
        );
        assert!(tokio::time::timeout(Duration::from_millis(5), queued)
            .await
            .is_err());
        flag.store(true, Ordering::Release);
        startup.drain().await;
        owner.stop(true).await;
        assert!(ctx.manager.is_live(&id));
        assert!(
            tokio::time::timeout(Duration::from_millis(5), acquire_lease(&id))
                .await
                .is_err()
        );
        drop(predecessor);
        ctx.manager.kill_session(&id).await.unwrap();
    }

    #[tokio::test]
    async fn cancellable_chat_wrapper_stops_its_resumed_turn_before_returning() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = crate::state::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("wrapper", "x", "U", false)
            .await
            .unwrap();
        let ws = ctx
            .workspaces
            .create("w", tmp.path().to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let id = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
        let flag = Arc::new(AtomicBool::new(false));
        let stop = flag.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            super::super::run_session_turn_cancellable(
                &ctx,
                &ws,
                &user,
                Some(&id),
                "test",
                tmp.path().to_str().unwrap(),
                "shell",
                serde_json::json!({}),
                "printf test",
                Duration::from_secs(30),
                Some(flag),
                move |_| stop.store(true, Ordering::Release),
            ),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        assert!(!ctx.manager.is_live(&id));
        assert!(
            tokio::time::timeout(Duration::from_secs(1), acquire_lease(&id))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn product_studio_stop_and_story_delete_drain_the_owned_session() {
        use otto_product::ProductStudioHost;
        for delete_story in [false, true] {
            let tmp = tempfile::tempdir().unwrap();
            let pool = crate::test_support::mem_pool().await;
            let ctx = crate::state::ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
            let user = otto_state::UsersRepo::new(pool.clone())
                .create("studio-stop", "x", "U", false)
                .await
                .unwrap();
            let ws = ctx
                .workspaces
                .create("w", tmp.path().to_str().unwrap(), &user.id)
                .await
                .unwrap();
            let id = live_shell(&ctx, &ws, &user.id, tmp.path()).await;
            let story = otto_core::new_id();
            let ownership = otto_product::story_lifecycle::claim(&story).await.unwrap();
            let cancel = otto_core::cancel_signal::CancelSignal::new();
            let running_ctx = ctx.clone();
            let running_id = id.clone();
            let running_story = story.clone();
            let running_cancel = cancel.clone();
            let cwd = tmp.path().to_str().unwrap().to_owned();
            let running = tokio::spawn(async move {
                running_ctx
                    .run_session_turn(
                        &ws,
                        &user,
                        Some(&running_id),
                        "studio stop",
                        &cwd,
                        "shell",
                        serde_json::json!({"story_id": running_story}),
                        "sleep 30",
                        Duration::from_secs(30),
                        running_cancel,
                    )
                    .await
            });
            // Observe real ownership, not a delay that assumes startup finished.
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match tokio::time::timeout(Duration::from_millis(10), acquire_lease(&id)).await
                    {
                        Err(_) => break,
                        Ok(lease) => drop(lease),
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let deletion = if delete_story {
                Some(tokio::spawn(async move {
                    otto_product::story_lifecycle::begin_delete(&story).await
                }))
            } else {
                cancel.cancel();
                None
            };
            assert!(tokio::time::timeout(Duration::from_secs(5), running)
                .await
                .unwrap()
                .unwrap()
                .is_err());
            assert!(!ctx.manager.is_live(&id), "delete_story={delete_story}");
            assert!(
                tokio::time::timeout(Duration::from_secs(1), acquire_lease(&id))
                    .await
                    .is_ok()
            );
            drop(ownership);
            if let Some(deletion) = deletion {
                drop(deletion.await.unwrap());
            }
        }
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
