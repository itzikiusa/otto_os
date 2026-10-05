//! Scheduled-task engine tests that need a full [`ServerCtx`] (the engine itself
//! lives in `otto-automation`, which cannot build one): the admission boundary
//! under concurrent schedule edits, and the HTTP edit → settlement fence.

#![allow(clippy::disallowed_methods)] // tests: plain sync fs / process / secret store is fine

use chrono::Utc;
use otto_core::domain::ScheduledTask;
use serde_json::json;

use crate::cadence;
use crate::scheduled_tasks_engine::{open_run, settle_schedule};
use crate::state::ServerCtx;

async fn admission_fixture() -> (tempfile::TempDir, ServerCtx, ScheduledTask) {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "snapshot-ws").await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let mut new =
        otto_state::NewScheduledTask::defaults("snapshot-ws".into(), "Captured occurrence".into());
    new.schedule = json!({"cadence":"once","run_at":"2000-01-01T10:00:00Z"});
    ctx.scheduled_tasks.create(new).await.unwrap();
    // The scheduler actually captures these rows before spawning run_task.
    let captured = ctx.scheduled_tasks.list_enabled().await.unwrap().remove(0);
    assert!(cadence::is_due_since(
        &captured.schedule,
        None,
        None,
        Utc::now(),
        chrono_tz::UTC,
    ));
    (tmp, ctx, captured)
}

async fn stale_schedule_admission(edits: Vec<otto_state::ScheduledTaskPatch>) {
    let (_tmp, ctx, captured) = admission_fixture().await;
    let mut events = ctx.events.subscribe();
    let (release, wait) = tokio::sync::oneshot::channel();
    let pending_ctx = ctx.clone();
    let pending_task = captured.clone();
    let pending = tokio::spawn(async move {
        wait.await.unwrap();
        // This is the actual run_task / spawn_run admission boundary, before
        // complete_run can execute a provider or deliver output.
        open_run(&pending_ctx, &pending_task, "schedule").await
    });
    for edit in edits {
        ctx.scheduled_tasks
            .update(&captured.id, edit)
            .await
            .unwrap();
    }
    let edited = ctx.scheduled_tasks.get(&captured.id).await.unwrap();
    release.send(()).unwrap();
    let result = pending.await.unwrap();
    let rows = ctx
        .scheduled_tasks
        .list_runs(&captured.id, 10)
        .await
        .unwrap();
    assert!(rows.is_empty(), "stale scheduled snapshot opened a run row");
    assert!(
        result.is_err(),
        "stale admission must stop before execution"
    );
    assert!(
        events.try_recv().is_err(),
        "rejected admission announced a run"
    );
    let after = ctx.scheduled_tasks.get(&captured.id).await.unwrap();
    assert_eq!(after.schedule, edited.schedule);
    assert_eq!(after.enabled, edited.enabled);
    assert_eq!(after.last_run_at, edited.last_run_at);
    assert_eq!(after.schedule_generation, edited.schedule_generation);
}

#[tokio::test]
async fn review5_scheduled_admission_rejects_disabled_snapshot() {
    stale_schedule_admission(vec![otto_state::ScheduledTaskPatch {
        enabled: Some(false),
        ..Default::default()
    }])
    .await;
}

#[tokio::test]
async fn review5_scheduled_admission_rejects_retimed_snapshot() {
    stale_schedule_admission(vec![otto_state::ScheduledTaskPatch {
        schedule: Some(json!({"cadence":"once","run_at":"2099-01-01T10:00:00Z"})),
        ..Default::default()
    }])
    .await;
}

#[tokio::test]
async fn review5_scheduled_admission_rejects_disable_enable_aba() {
    stale_schedule_admission(vec![
        otto_state::ScheduledTaskPatch {
            enabled: Some(false),
            ..Default::default()
        },
        otto_state::ScheduledTaskPatch {
            enabled: Some(true),
            ..Default::default()
        },
    ])
    .await;
}

#[tokio::test]
async fn review5_scheduled_admission_rejects_retime_aba() {
    stale_schedule_admission(vec![
        otto_state::ScheduledTaskPatch {
            schedule: Some(json!({"cadence":"once","run_at":"2099-01-01T10:00:00Z"})),
            ..Default::default()
        },
        otto_state::ScheduledTaskPatch {
            schedule: Some(json!({"cadence":"once","run_at":"2000-01-01T10:00:00Z"})),
            ..Default::default()
        },
    ])
    .await;
}

#[tokio::test]
async fn review5_scheduled_admission_fresh_resumed_snapshot_runs() {
    let (_tmp, ctx, captured) = admission_fixture().await;
    for enabled in [false, true] {
        ctx.scheduled_tasks
            .update(
                &captured.id,
                otto_state::ScheduledTaskPatch {
                    enabled: Some(enabled),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let fresh = ctx.scheduled_tasks.get(&captured.id).await.unwrap();
    // Eligibility changes must not invalidate an already-admitted run's
    // settlement identity (the review4_once_pause tests cover completion).
    assert_eq!(fresh.schedule_generation, captured.schedule_generation);
    let run = open_run(&ctx, &fresh, "schedule").await.unwrap();
    assert_eq!(run.trigger, "schedule");
    assert_eq!(
        ctx.scheduled_tasks
            .list_runs(&fresh.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review5_scheduled_admission_content_edit_preserves_eligibility() {
    let (_tmp, ctx, captured) = admission_fixture().await;
    ctx.scheduled_tasks
        .update(
            &captured.id,
            otto_state::ScheduledTaskPatch {
                name: Some("Renamed".into()),
                prompt: Some("New prompt".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let run = open_run(&ctx, &captured, "schedule").await.unwrap();
    assert_eq!(run.task_id, captured.id);
}

#[tokio::test]
async fn review5_scheduled_admission_manual_run_allows_disabled_task() {
    let (_tmp, ctx, captured) = admission_fixture().await;
    let disabled = ctx
        .scheduled_tasks
        .update(
            &captured.id,
            otto_state::ScheduledTaskPatch {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let run = open_run(&ctx, &disabled, "manual").await.unwrap();
    assert_eq!(run.trigger, "manual");
    assert_eq!(
        ctx.scheduled_tasks
            .list_runs(&disabled.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review5_scheduled_admission_same_snapshot_has_one_running_row() {
    let (_tmp, ctx, captured) = admission_fixture().await;
    let (first, second) = tokio::join!(
        open_run(&ctx, &captured, "schedule"),
        open_run(&ctx, &captured, "schedule"),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        ctx.scheduled_tasks
            .list_runs(&captured.id, 10)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn review4_schedule_http_edit_and_away_back_fence_actual_settlement() {
    use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
    use tower::ServiceExt;
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "ws").await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let app = crate::routes::scheduled_tasks::routes().with_state(ctx.clone());
    for status in ["ok", "error", "canceled"] {
        for old_schedule in [
            json!({"cadence":"once","run_at":"2099-10-05T10:00:00Z"}),
            json!({"cadence":"interval","every_min":60}),
        ] {
            let mut new = otto_state::NewScheduledTask::defaults("ws".into(), status.into());
            new.schedule = old_schedule;
            let dispatched = ctx.scheduled_tasks.create(new).await.unwrap();
            let run = ctx
                .scheduled_tasks
                .create_run(NewScheduledRun {
                    task_id: dispatched.id.clone(),
                    workspace_id: "ws".into(),
                    trigger: "schedule".into(),
                })
                .await
                .unwrap();
            for time in ["11:00:00", "10:00:00"] {
                let body =
                    json!({"schedule":{"cadence":"once","run_at":format!("2099-10-05T{time}Z")}});
                let mut request = axum::http::Request::builder()
                    .method("PATCH")
                    .uri(format!("/scheduled-tasks/{}", dispatched.id))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap();
                request
                    .extensions_mut()
                    .insert(otto_core::auth::AuthUser(root_user()));
                let response = app.clone().oneshot(request).await.unwrap();
                assert_eq!(response.status(), axum::http::StatusCode::OK);
            }
            let retimed = ctx.scheduled_tasks.get(&dispatched.id).await.unwrap();
            assert_eq!(
                retimed.schedule_generation,
                dispatched.schedule_generation + 2
            );
            ctx.scheduled_tasks
                .finish_run(
                    &run.id,
                    FinishRun {
                        status: status.into(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            settle_schedule(
                &ctx.scheduled_tasks,
                &dispatched,
                status,
                "2099-10-05T10:02:00Z".parse().unwrap(),
            )
            .await
            .unwrap();
            let after = ctx.scheduled_tasks.get(&dispatched.id).await.unwrap();
            assert_eq!(after.next_run_at, retimed.next_run_at);
            assert!(after.last_run_at.is_none());
            assert!(cadence::is_due_since(
                &after.schedule,
                None,
                None,
                "2099-10-05T10:00:00Z".parse().unwrap(),
                chrono_tz::UTC
            ));
            assert_eq!(
                ctx.scheduled_tasks.get_run(&run.id).await.unwrap().status,
                status
            );
            // Ordinary content changes preserve this generation; its own
            // completion still consumes exactly this new occurrence.
            let edited = ctx
                .scheduled_tasks
                .update(
                    &after.id,
                    otto_state::ScheduledTaskPatch {
                        name: Some("renamed".into()),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(edited.schedule_generation, after.schedule_generation);
            settle_schedule(
                &ctx.scheduled_tasks,
                &after,
                "ok",
                "2099-10-05T10:03:00Z".parse().unwrap(),
            )
            .await
            .unwrap();
            let done = ctx.scheduled_tasks.get(&after.id).await.unwrap();
            let cursor = done
                .last_run_at
                .as_deref()
                .map(|s| s.parse::<DateTime<Utc>>().unwrap());
            assert!(!cadence::is_due_since(
                &done.schedule,
                cursor,
                None,
                "2099-10-05T11:00:00Z".parse().unwrap(),
                chrono_tz::UTC
            ));
        }
    }
}
