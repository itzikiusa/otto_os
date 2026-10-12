//! The shared agent retry loop (`run_with_recovery_until`) against a real
//! session manager (sweep 2026-10-11 S-3 / S-4): a result that already
//! finished survives a Stop that lands after it, and the last failed attempt's
//! session stays openable unless a person stopped the run.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use otto_core::domain::{SessionKind, SessionStatus};
use otto_core::Id;
use otto_server::agent_run::{run_with_recovery_until, FailReason, RunOutcome};
use otto_server::ServerCtx;
use otto_state::{DbPool, SessionsRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

/// A live `/bin/sh` agent session standing in for a reviewer / executor.
async fn live_session(ctx: &ServerCtx, pool: &DbPool, dir: &std::path::Path) -> Id {
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create(&otto_core::new_id(), "unused", "Owner", false)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("Recovery", dir.to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    ctx.manager
        .create(
            &ws,
            &owner.id,
            otto_core::api::CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("reviewer".into()),
                cwd: None,
                connection_id: None,
                meta: None,
                model: None,
            },
            Some(otto_pty::CommandSpec {
                program: "/bin/sh".into(),
                args: vec![],
                cwd: Some(dir.to_string_lossy().into()),
                env: vec![],
            }),
        )
        .await
        .unwrap()
        .id
}

async fn exited(pool: &DbPool, id: &Id) -> bool {
    SessionsRepo::new(pool.clone())
        .get(id)
        .await
        .unwrap()
        .status
        == SessionStatus::Exited
}

#[tokio::test]
async fn a_finished_result_survives_a_stop_that_lands_after_it() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let sid = live_session(&ctx, &pool, tmp.path()).await;
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = run_with_recovery_until(
        &ctx.manager,
        2,
        &[],
        Some(&cancel),
        || async { false },
        |_| {
            let (cancel, sid) = (Arc::clone(&cancel), sid.clone());
            async move {
                // The agent finished; the Stop arrives just after.
                cancel.store(true, Ordering::Relaxed);
                RunOutcome::ok("[]".into(), sid)
            }
        },
    )
    .await;
    assert!(!outcome.errored(), "{:?}", outcome.reason);
    assert_eq!(outcome.raw.as_deref(), Some("[]"));
    assert!(!exited(&pool, &sid).await, "completed work is not killed");
}

#[tokio::test]
async fn the_last_failed_attempt_stays_openable_unless_stopped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let sid = live_session(&ctx, &pool, tmp.path()).await;
    let outcome = run_with_recovery_until(
        &ctx.manager,
        1,
        &[],
        None,
        || async { false },
        |_| {
            let sid = sid.clone();
            async move { RunOutcome::failed(Some(sid), FailReason::Stuck) }
        },
    )
    .await;
    assert_eq!(outcome.reason, Some(FailReason::Stuck));
    assert!(
        !exited(&pool, &sid).await,
        "a stuck reviewer stays open to inspect"
    );
    assert!(ctx.manager.is_live(&sid));

    // A person's Stop during the final attempt still ends the session.
    let cancel = Arc::new(AtomicBool::new(false));
    let outcome = run_with_recovery_until(
        &ctx.manager,
        1,
        &[],
        Some(&cancel),
        || async { false },
        |_| {
            let (cancel, sid) = (Arc::clone(&cancel), sid.clone());
            async move {
                cancel.store(true, Ordering::Relaxed);
                RunOutcome::failed(Some(sid), FailReason::Exited)
            }
        },
    )
    .await;
    assert_eq!(outcome.reason, Some(FailReason::Stopped));
    assert!(exited(&pool, &sid).await, "Stop retires the session");
}
