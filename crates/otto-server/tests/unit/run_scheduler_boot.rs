use super::*;
use otto_core::run::{RunMode, RunOrigin, RunStatus, SourceKind};
use otto_state::runs::NewRun;

async fn live_run(ctx: &ServerCtx, workspace_id: &str) -> otto_core::run::OttoRun {
    let run = ctx
        .runs
        .create(NewRun {
            workspace_id: workspace_id.into(),
            title: "Recovery fixture".into(),
            source_kind: SourceKind::Finding,
            source_ref: "isolated-finding".into(),
            source_url: None,
            goal: "No agents are launched".into(),
            mode: RunMode::SingleAgent,
            provider: "claude".into(),
            model: String::new(),
            repo_id: None,
            origin_kind: RunOrigin::Slack,
            origin_chat: None,
            origin_thread: None,
            origin_user: None,
            callback_url: None,
            auto_open_pr: false,
            context_summary: None,
            created_by: "root".into(),
        })
        .await
        .unwrap();
    ctx.runs
        .set_status(&run.id, RunStatus::Executing)
        .await
        .unwrap();
    ctx.runs.get(&run.id).await.unwrap()
}

#[tokio::test]
async fn run_scheduler_start_waits_for_startup_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('recovery-ws', 'Recovery', '/tmp', '2026-10-08T00:00:00Z')").execute(&pool).await.unwrap();
    let old = live_run(&ctx, "recovery-ws").await;
    // Hold the sole fixture connection: the startup phase must stay pending
    // until the previous-life scan and settlement have finished.
    let connection = pool.acquire().await.unwrap();
    let starting = async {
        let mut phases = crate::boot::BootPhases::start();
        crate::boot::recover_before_serve(&ctx, &mut phases)
            .await
            .unwrap();
    };
    tokio::pin!(starting);
    tokio::select! {
        biased;
        _ = &mut starting => panic!("startup returned before recovery could run"),
        _ = tokio::time::sleep(Duration::from_millis(30)) => {}
    }
    drop(connection);
    tokio::time::timeout(Duration::from_secs(10), starting)
        .await
        .unwrap();
    assert_eq!(
        ctx.runs.get(&old.id).await.unwrap().status,
        RunStatus::Failed
    );
    let fresh = live_run(&ctx, "recovery-ws").await;
    start_run_scheduler(&ctx);
    // The detached supervisor gets its first tick after fresh work is admitted;
    // it must never classify that new live run as belonging to the old daemon.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        ctx.runs.get(&fresh.id).await.unwrap().status,
        RunStatus::Executing
    );
}

#[tokio::test]
async fn run_scheduler_recovery_failure_prevents_boot_admission() {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    pool.close().await;
    let mut phases = crate::boot::BootPhases::start();
    let error = crate::boot::recover_before_serve(&ctx, &mut phases)
        .await
        .unwrap_err();
    assert!(error.starts_with("run-with-otto recovery:"), "{error}");
}
