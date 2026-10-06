//! Workflow trigger scheduler: fires `schedule`-kind triggers on their cadence
//! (interval / daily / weekly) and starts a workflow run in the background.
//!
//! Modeled on [`otto_swarm::runtime::scheduler`]: 60-second tick (one timer, woken by `CancelSignal`),
//! Cursor/eligibility claim and queued run are committed together; a captured
//! tick cannot admit after retiming or disabling its trigger.
//!
//! Schedule spec keys (mirrors the swarm-scheduler format):
//!   `cadence`    — "interval" | "daily" | "weekly" (default "interval")
//!   `every_min`  — minutes between fires (cadence=interval; default 60)
//!   `at`         — "HH:MM" UTC wall time to fire (daily/weekly)
//!   `weekday`    — 0-6, Mon=0 (weekly only; default Monday)
//!   `last_run`   — RFC-3339 timestamp of last fire (cursor; set by scheduler)
//!   `enabled`    — bool; missing/false → skip

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_state::{TriggersRepo, WorkflowsRepo};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::cancel_signal::CancelSignal;
use crate::state::ServerCtx;
use otto_workflows::triggers::copy_result_destinations;

const SCAN: Duration = Duration::from_secs(60);

/// Start the scheduler supervisor task. Returns its cancel signal; `cancel()`
/// stops the loop at once (mirrors the swarm/insights/cli-update pattern).
pub fn start(ctx: ServerCtx) -> CancelSignal {
    let cancel = CancelSignal::new();
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: CancelSignal) {
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx).await {
            warn!("workflow trigger scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

async fn tick(ctx: &ServerCtx) -> otto_core::Result<()> {
    let triggers_repo = TriggersRepo::new(ctx.pool.clone());
    let workflows_repo = WorkflowsRepo::new(ctx.pool.clone());
    let now = Utc::now();

    for trigger in triggers_repo.list_enabled_by_kind("schedule").await? {
        // The cursor never predates the arm instant (created / resumed /
        // re-timed), so a resumed trigger doesn't fire the run it missed
        // while off — the scheduled-tasks fix from PR #70.
        if !is_due_armed(
            &trigger.spec,
            Some(trigger.created_at),
            trigger.armed_at,
            now,
        ) {
            continue;
        }

        // Resolve the workflow; skip silently if it was deleted.
        let wf = match workflows_repo.get(&trigger.workflow_id).await {
            Ok(w) => w,
            Err(_) => continue,
        };
        let ws = match ctx.workspaces.get(&wf.workspace_id).await {
            Ok(w) => w,
            Err(_) => continue,
        };

        // Overlap guard: a run longer than the cadence must not stack a second
        // concurrent copy (each provisions its own worktrees). The cursor is
        // NOT advanced, so the missed tick fires once the run finishes.
        match workflows_repo.has_active_run(&wf.id).await {
            Ok(true) => {
                info!(workflow_id = %wf.id, "workflow scheduler: previous run still active — skipping tick");
                continue;
            }
            Ok(false) => {}
            Err(e) => {
                warn!(workflow_id = %wf.id, "workflow scheduler: active-run check: {e}");
                continue;
            }
        }

        // Build the run input once, shared by `create_run` and the spawned
        // `run_workflow` call. `spec.prompt` (if set) threads a fixed prompt
        // through to the engine's `normalize_prompt`, same as a chat-started run.
        let mut input = serde_json::Map::new();
        input.insert("trigger".into(), json!("schedule"));
        if let Some(p) = trigger
            .spec
            .get("prompt")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            input.insert("prompt".into(), json!(p));
        }
        // Result delivery: thread the trigger's result_* destinations into the
        // run input — `deliver_run_result` reads exactly these keys, so a
        // scheduled run can report to a chat/webhook instead of finishing
        // silently with no notification.
        copy_result_destinations(&trigger.spec, &mut input);
        let input = Value::Object(input);

        // Revalidate eligibility and the overlap guard while atomically claiming
        // the cursor and creating the run, then execute in a background task.
        let run = match admit_schedule_run(ctx, &trigger, &wf.workspace_id, &input, now).await {
            Ok(Some(run)) => run,
            Ok(None) => continue,
            Err(e) => {
                warn!(workflow_id = %wf.id, "workflow scheduler: admit run: {e}");
                continue;
            }
        };

        info!(
            workflow_id = %wf.id,
            run_id = %run.id,
            "workflow trigger scheduler: firing schedule trigger"
        );

        crate::workflow_engine::spawn_run(
            ctx.clone(),
            ws,
            wf,
            run.id.clone(),
            input,
            otto_core::workflows::RunScope::default(),
            None,
        );
    }
    Ok(())
}

/// The scheduler's durable admission boundary, separated from background engine
/// execution so stale captured ticks can be exercised against the real database.
async fn admit_schedule_run(
    ctx: &ServerCtx,
    trigger: &otto_state::WorkflowTrigger,
    workspace_id: &otto_core::Id,
    input: &Value,
    now: DateTime<Utc>,
) -> otto_core::Result<Option<otto_core::workflows::WorkflowRun>> {
    TriggersRepo::new(ctx.pool.clone())
        .claim_scheduled_run(trigger, workspace_id, input, &now.to_rfc3339())
        .await
}

/// True when a schedule-trigger spec is due to fire at `now`.
///
/// Delegates to the shared [`crate::cadence`] engine (the same one Scheduled
/// Tasks use) so workflow schedule triggers get **cron** (`cadence:"cron"`,
/// `expr`) and **IANA timezone** (`timezone`) parity for free, while
/// interval/daily/weekly behave exactly as before. The cursor (`last_run`) is
/// read from the spec.
pub fn is_due(spec: &Value, now: DateTime<Utc>) -> bool {
    is_due_since(spec, None, now)
}

/// [`is_due`] anchored on the trigger's creation time: a never-run CRON only
/// looked one minute back, so its first fire was lost (for a whole day/week)
/// when the Mac slept or the daemon was down at that minute — the scheduled
/// tasks / personal agents ticks already anchor on creation.
pub fn is_due_since(spec: &Value, created: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    is_due_armed(spec, created, None, now)
}

/// [`is_due_since`] with the trigger's arm instant as a floor on the cursor
/// (see `WorkflowTrigger::armed_at`): `max(last_run, armed_at)`.
pub fn is_due_armed(
    spec: &Value,
    created: Option<DateTime<Utc>>,
    armed: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    let last = spec
        .get("last_run")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let last = crate::cadence::effective_cursor(spec, last, armed);
    let tz = crate::cadence::task_tz(spec.get("timezone").and_then(Value::as_str).unwrap_or(""));
    crate::cadence::is_due_since(spec, last, created, now, tz)
}

/// Start the event-trigger listener task. Returns a cancel flag; set to `true`
/// to stop the loop. The listener lives in `otto_workflows::triggers`, driven
/// through this daemon's [`otto_workflows::WorkflowCtx`] impl.
pub fn spawn_workflow_event_trigger_listener(ctx: ServerCtx) -> Arc<AtomicBool> {
    otto_workflows::triggers::spawn_event_trigger_listener(ctx)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
