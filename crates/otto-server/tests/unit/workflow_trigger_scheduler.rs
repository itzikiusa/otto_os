use super::*;

#[tokio::test]
async fn cancelled_schedule_tick_preserves_unclaimed_occurrence() {
    let (_tmp, ctx, captured, _workspace) = admission_fixture().await;
    let repo = TriggersRepo::new(ctx.pool.clone());
    let mut due = captured.spec.clone();
    due["run_at"] = json!("2000-01-01T00:00:00Z");
    repo.update(&captured.id, Some(due), None).await.unwrap();
    let before = repo.get(&captured.id).await.unwrap();
    let cancel = CancelSignal::new();
    cancel.cancel();
    tick(&ctx, &cancel).await.unwrap();
    let after = repo.get(&captured.id).await.unwrap();
    assert_eq!(after.spec, before.spec);
    assert!(WorkflowsRepo::new(ctx.pool.clone())
        .list_runs(&captured.workflow_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn schedule_tick_cancelled_during_database_wait_does_not_admit() {
    let (_tmp, ctx, captured, _workspace) = admission_fixture().await;
    let repo = TriggersRepo::new(ctx.pool.clone());
    let mut due = captured.spec.clone();
    due["run_at"] = json!("2000-01-01T00:00:00Z");
    repo.update(&captured.id, Some(due), None).await.unwrap();
    let before = repo.get(&captured.id).await.unwrap();
    assert!(is_due_armed(
        &before.spec,
        Some(before.created_at),
        before.armed_at,
        Utc::now()
    ));
    // This fixture has one SQLite connection. Poll the production tick
    // into its first DB await, then cancel before releasing that await.
    let connection = ctx.pool.acquire().await.unwrap();
    let cancel = CancelSignal::new();
    let ticking = tick(&ctx, &cancel);
    tokio::pin!(ticking);
    let first = std::future::poll_fn(|cx| {
        std::task::Poll::Ready(std::future::Future::poll(ticking.as_mut(), cx))
    })
    .await;
    assert!(first.is_pending());
    cancel.cancel();
    drop(connection);
    tokio::time::timeout(Duration::from_secs(5), ticking)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repo.get(&captured.id).await.unwrap().spec, before.spec);
    assert!(WorkflowsRepo::new(ctx.pool.clone())
        .list_runs(&captured.workflow_id)
        .await
        .unwrap()
        .is_empty());
}

async fn admission_fixture() -> (
    tempfile::TempDir,
    ServerCtx,
    otto_state::WorkflowTrigger,
    otto_core::Id,
) {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "admission-ws").await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('admission-user','admission-user','x','User',1,?)")
        .bind(Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let wf = WorkflowsRepo::new(pool.clone())
        .create(
            &"admission-ws".into(),
            "Scheduled",
            "",
            "standing instruction",
            &otto_core::workflows::WorkflowGraph::default(),
            &"admission-user".into(),
        )
        .await
        .unwrap();
    let trigger=TriggersRepo::new(pool).create(otto_state::NewWorkflowTrigger {
        workflow_id:wf.id,kind:"schedule".into(),enabled:true,
        spec:json!({"cadence":"once","run_at":"2099-10-05T10:00:00Z","prompt":"original prompt"}),
    }).await.unwrap();
    (tmp, ctx, trigger, wf.workspace_id)
}

async fn stale_admission_after_edit(change: &str) {
    let (_tmp, ctx, captured, workspace) = admission_fixture().await;
    let now = "2099-10-05T10:01:00Z".parse().unwrap();
    assert!(is_due_armed(
        &captured.spec,
        Some(captured.created_at),
        captured.armed_at,
        now
    ));
    let repo = TriggersRepo::new(ctx.pool.clone());
    match change {
        "retime" | "retime-away-back" => {
            let mut future = captured.spec.clone();
            future["run_at"] = json!("2099-10-05T11:00:00Z");
            repo.update(&captured.id, Some(future), None).await.unwrap();
            if change == "retime-away-back" {
                repo.update(&captured.id, Some(captured.spec.clone()), None)
                    .await
                    .unwrap();
            }
        }
        "disable" | "disable-enable" => {
            repo.update(&captured.id, None, Some(false)).await.unwrap();
            if change == "disable-enable" {
                repo.update(&captured.id, None, Some(true)).await.unwrap();
            }
        }
        _ => panic!("unknown fixture edit"),
    }
    let edited = repo.get(&captured.id).await.unwrap();
    // This is the production boundary reached after the tick's workflow,
    // workspace and active-run awaits, carrying its already captured row.
    let admitted = admit_schedule_run(
        &ctx,
        &captured,
        &workspace,
        &json!({"trigger":"schedule"}),
        now,
    )
    .await
    .unwrap();
    assert!(
        admitted.is_none(),
        "stale {change} tick must not create a queued run"
    );
    assert!(WorkflowsRepo::new(ctx.pool.clone())
        .list_runs(&captured.workflow_id)
        .await
        .unwrap()
        .is_empty());
    let after = repo.get(&captured.id).await.unwrap();
    assert_eq!(after.spec, edited.spec);
    assert_eq!(after.enabled, edited.enabled);
}

#[tokio::test]
async fn review4_trigger_admission_rejects_retime() {
    stale_admission_after_edit("retime").await;
}

#[tokio::test]
async fn review4_trigger_admission_rejects_disable() {
    stale_admission_after_edit("disable").await;
}

#[tokio::test]
async fn review4_trigger_admission_rejects_retime_away_back() {
    stale_admission_after_edit("retime-away-back").await;
}

#[tokio::test]
async fn review4_trigger_admission_rejects_disable_enable() {
    stale_admission_after_edit("disable-enable").await;
}

#[tokio::test]
async fn review4_trigger_admission_failure_does_not_consume_occurrence() {
    let (_tmp, ctx, captured, workspace) = admission_fixture().await;
    sqlx::query("CREATE TRIGGER reject_scheduled_queue BEFORE INSERT ON workflow_runs BEGIN SELECT RAISE(ABORT,'injected queue failure'); END;").execute(&ctx.pool).await.unwrap();
    assert!(admit_schedule_run(
        &ctx,
        &captured,
        &workspace,
        &json!({"trigger":"schedule"}),
        "2099-10-05T10:01:00Z".parse().unwrap()
    )
    .await
    .is_err());
    let current = TriggersRepo::new(ctx.pool.clone())
        .get(&captured.id)
        .await
        .unwrap();
    assert!(
        current.spec.get("last_run").is_none(),
        "failed queue insertion must roll back claiming the occurrence"
    );
    assert!(WorkflowsRepo::new(ctx.pool.clone())
        .list_runs(&captured.workflow_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn review4_trigger_same_snapshot_concurrent_admission_queues_once() {
    let (_tmp, ctx, captured, workspace) = admission_fixture().await;
    let input = json!({"trigger":"schedule"});
    let now = "2099-10-05T10:01:00Z".parse().unwrap();
    let (a, b) = tokio::join!(
        admit_schedule_run(&ctx, &captured, &workspace, &input, now),
        admit_schedule_run(&ctx, &captured, &workspace, &input, now)
    );
    let count = usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some());
    assert_eq!(count, 1, "one captured occurrence admits exactly one run");
    assert_eq!(
        WorkflowsRepo::new(ctx.pool.clone())
            .list_runs(&captured.workflow_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review4_trigger_current_due_admission_preserves_pinned_definition() {
    let (_tmp, ctx, captured, workspace) = admission_fixture().await;
    let now = "2099-10-05T10:01:00Z".parse().unwrap();
    assert!(is_due_armed(
        &captured.spec,
        Some(captured.created_at),
        captured.armed_at,
        now
    ));
    let run = admit_schedule_run(
        &ctx,
        &captured,
        &workspace,
        &json!({"trigger":"schedule"}),
        now,
    )
    .await
    .unwrap()
    .expect("current due tick admitted");
    assert_eq!(run.workflow_version, Some(1));
    let definition = WorkflowsRepo::new(ctx.pool.clone())
        .definition_for_run(&run)
        .await
        .unwrap();
    assert_eq!(definition.instructions, "standing instruction");
    let claimed = TriggersRepo::new(ctx.pool.clone())
        .get(&captured.id)
        .await
        .unwrap();
    assert!(!is_due_armed(
        &claimed.spec,
        Some(claimed.created_at),
        claimed.armed_at,
        now
    ));
    assert_eq!(
        WorkflowsRepo::new(ctx.pool.clone())
            .list_runs(&captured.workflow_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review4_trigger_overlap_is_checked_inside_admission_transaction() {
    let (_tmp, ctx, first, workspace) = admission_fixture().await;
    let repo = TriggersRepo::new(ctx.pool.clone());
    let second = repo
        .create(otto_state::NewWorkflowTrigger {
            workflow_id: first.workflow_id.clone(),
            kind: "schedule".into(),
            enabled: true,
            spec: first.spec.clone(),
        })
        .await
        .unwrap();
    let input = json!({"trigger":"schedule"});
    let now = "2099-10-05T10:01:00Z".parse().unwrap();
    let (a, b) = tokio::join!(
        admit_schedule_run(&ctx, &first, &workspace, &input, now),
        admit_schedule_run(&ctx, &second, &workspace, &input, now)
    );
    assert_eq!(
        usize::from(a.unwrap().is_some()) + usize::from(b.unwrap().is_some()),
        1
    );
    let a = repo.get(&first.id).await.unwrap();
    let b = repo.get(&second.id).await.unwrap();
    assert_eq!(
        usize::from(a.spec.get("last_run").is_some())
            + usize::from(b.spec.get("last_run").is_some()),
        1,
        "skipped overlapping trigger must remain due"
    );
    assert_eq!(
        WorkflowsRepo::new(ctx.pool.clone())
            .list_runs(&first.workflow_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review4_trigger_captured_cursor_stays_stale_after_run_finishes() {
    let (_tmp, ctx, captured, workspace) = admission_fixture().await;
    let input = json!({"trigger":"schedule"});
    let now = "2099-10-05T10:01:00Z".parse().unwrap();
    let run = admit_schedule_run(&ctx, &captured, &workspace, &input, now)
        .await
        .unwrap()
        .unwrap();
    WorkflowsRepo::new(ctx.pool.clone())
        .update_run(
            &run.id,
            otto_core::workflows::RunStatus::Success,
            &[],
            None,
            true,
        )
        .await
        .unwrap();
    assert!(
        admit_schedule_run(&ctx, &captured, &workspace, &input, now)
            .await
            .unwrap()
            .is_none(),
        "cursor comparison must reject an old tick even when no run remains active"
    );
    assert_eq!(
        WorkflowsRepo::new(ctx.pool.clone())
            .list_runs(&captured.workflow_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn interval_due_when_never_run() {
    let s = json!({"cadence": "interval", "every_min": 30, "enabled": true});
    assert!(is_due(&s, Utc::now()));
}

#[test]
fn interval_not_due_within_window() {
    let now = Utc::now();
    let s = json!({"cadence":"interval","every_min":60,"last_run": now.to_rfc3339()});
    assert!(!is_due(&s, now));
}

#[test]
fn interval_due_after_window() {
    let now = Utc::now();
    let past = now - chrono::Duration::minutes(90);
    let s = json!({"cadence":"interval","every_min":60,"last_run": past.to_rfc3339()});
    assert!(is_due(&s, now));
}

#[test]
fn unknown_cadence_is_never_due() {
    let s = json!({"cadence": "monthly", "enabled": true});
    assert!(!is_due(&s, Utc::now()));
}

#[test]
fn cron_cadence_supported_via_shared_engine() {
    use chrono::TimeZone;
    // "every minute" cron, never run → due now (proves cron parity).
    let now = Utc.with_ymd_and_hms(2026, 6, 29, 12, 0, 0).unwrap();
    let s = json!({ "cadence": "cron", "expr": "* * * * *" });
    assert!(is_due(&s, now), "every-minute cron should be due");
    // A daily cron at 09:00 with a cursor already past today's fire is not due
    // again at noon.
    let s2 = json!({
        "cadence": "cron", "expr": "0 9 * * *",
        "last_run": Utc.with_ymd_and_hms(2026, 6, 29, 9, 0, 0).unwrap().to_rfc3339(),
    });
    assert!(!is_due(&s2, now), "already fired today's 09:00 cron");
}

#[test]
fn never_run_cron_catches_up_its_first_fire_from_creation() {
    use chrono::TimeZone;
    let s = json!({ "cadence": "cron", "expr": "0 9 * * *" });
    let created = Utc.with_ymd_and_hms(2026, 6, 29, 8, 0, 0).unwrap();
    // The daemon was down at 09:00; at 09:20 the first fire is still owed.
    let now = Utc.with_ymd_and_hms(2026, 6, 29, 9, 20, 0).unwrap();
    assert!(!is_due(&s, now), "without the anchor the fire is lost");
    assert!(is_due_since(&s, Some(created), now));
    // Created after today's 09:00 → nothing owed until tomorrow.
    let late = Utc.with_ymd_and_hms(2026, 6, 29, 9, 10, 0).unwrap();
    assert!(!is_due_since(&s, Some(late), now));
}

#[test]
fn resumed_trigger_does_not_fire_its_missed_run() {
    use chrono::TimeZone;
    let now = Utc.with_ymd_and_hms(2026, 6, 29, 15, 0, 0).unwrap();
    let week_ago = now - chrono::Duration::days(7);
    // Interval trigger paused a week, re-enabled a minute ago.
    let s = json!({"cadence": "interval", "every_min": 60, "last_run": week_ago.to_rfc3339()});
    assert!(is_due_since(&s, Some(week_ago), now), "old behaviour fired");
    let armed = now - chrono::Duration::minutes(1);
    assert!(!is_due_armed(&s, Some(week_ago), Some(armed), now));
    assert!(is_due_armed(
        &s,
        Some(week_ago),
        Some(armed),
        now + chrono::Duration::minutes(60)
    ));
    // A daily 09:00 trigger created/enabled at 15:00 waits for tomorrow.
    let d = json!({"cadence": "daily", "at": "09:00", "timezone": "UTC"});
    assert!(!is_due_armed(&d, Some(now), Some(now), now));
    let tomorrow = Utc.with_ymd_and_hms(2026, 6, 30, 9, 1, 0).unwrap();
    assert!(is_due_armed(&d, Some(now), Some(now), tomorrow));
}

#[test]
fn timezone_is_threaded_through() {
    // A daily 09:00 trigger in a +/- tz is interpreted in that tz, not UTC.
    // Just assert it doesn't panic and respects the spec shape.
    let s = json!({ "cadence": "daily", "at": "09:00", "timezone": "America/New_York" });
    let _ = is_due(&s, Utc::now());
}
