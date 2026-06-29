//! Workflow trigger scheduler: fires `schedule`-kind triggers on their cadence
//! (interval / daily / weekly) and starts a workflow run in the background.
//!
//! Modeled on [`crate::swarm_scheduler`]: 60-second tick, 500ms cancel slices,
//! DB-cursor idempotency via `last_run` stored in the trigger's `spec_json`.
//!
//! Schedule spec keys (mirrors the swarm-scheduler format):
//!   `cadence`    — "interval" | "daily" | "weekly" (default "interval")
//!   `every_min`  — minutes between fires (cadence=interval; default 60)
//!   `at`         — "HH:MM" UTC wall time to fire (daily/weekly)
//!   `weekday`    — 0-6, Mon=0 (weekly only; default Monday)
//!   `last_run`   — RFC-3339 timestamp of last fire (cursor; set by scheduler)
//!   `enabled`    — bool; missing/false → skip

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::event::Event;
use otto_state::{TriggersRepo, WorkflowsRepo};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::state::ServerCtx;

const SCAN: Duration = Duration::from_secs(60);
const SLICE: Duration = Duration::from_millis(500);

/// Start the scheduler supervisor task. Returns a cancel flag; set to `true`
/// to stop the loop (mirrors the swarm/insights/cli-update pattern).
pub fn start(ctx: ServerCtx) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: Arc<AtomicBool>) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        if let Err(e) = tick(&ctx).await {
            warn!("workflow trigger scheduler tick: {e}");
        }
        let mut waited = Duration::ZERO;
        while waited < SCAN {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(SLICE).await;
            waited += SLICE;
        }
    }
}

async fn tick(ctx: &ServerCtx) -> otto_core::Result<()> {
    let triggers_repo = TriggersRepo::new(ctx.pool.clone());
    let workflows_repo = WorkflowsRepo::new(ctx.pool.clone());
    let now = Utc::now();

    for trigger in triggers_repo.list_enabled_by_kind("schedule").await? {
        if !is_due(&trigger.spec, now) {
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

        // Advance the cursor first (idempotency: a slow/failing run can't
        // double-fire on the next tick).
        let mut spec2 = trigger.spec.clone();
        if let Some(obj) = spec2.as_object_mut() {
            obj.insert("last_run".into(), json!(now.to_rfc3339()));
        }
        if let Err(e) = triggers_repo.set_spec(&trigger.id, spec2).await {
            warn!(trigger_id = %trigger.id, "workflow scheduler: advance cursor: {e}");
            continue;
        }

        // Create the run row, then execute in a background task.
        let run = match workflows_repo
            .create_run(&wf.id, &wf.workspace_id, &json!({"trigger": "schedule"}))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(workflow_id = %wf.id, "workflow scheduler: create run: {e}");
                continue;
            }
        };

        info!(
            workflow_id = %wf.id,
            run_id = %run.id,
            "workflow trigger scheduler: firing schedule trigger"
        );

        let ctx2 = ctx.clone();
        let run_id = run.id.clone();
        tokio::spawn(async move {
            crate::workflow_engine::run_workflow(
                ctx2, ws, wf, run_id,
                json!({"trigger": "schedule"}),
                None, false,
            )
            .await;
        });
    }
    Ok(())
}

/// True when a schedule-trigger spec is due to fire at `now`.
///
/// Delegates to the shared [`crate::cadence`] engine (the same one Scheduled
/// Tasks use) so workflow schedule triggers get **cron** (`cadence:"cron"`,
/// `expr`) and **IANA timezone** (`timezone`) parity for free, while
/// interval/daily/weekly behave exactly as before. The cursor (`last_run`) is
/// read from the spec.
pub fn is_due(spec: &Value, now: DateTime<Utc>) -> bool {
    let last = spec
        .get("last_run")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    let tz = crate::cadence::task_tz(spec.get("timezone").and_then(Value::as_str).unwrap_or(""));
    crate::cadence::is_due(spec, last, now, tz)
}

// ---------------------------------------------------------------------------
// Event-trigger listener (B8): subscribes to the daemon event bus and fires
// any enabled `event`-kind triggers whose `event_kind` spec field matches the
// incoming event.  Reuses the same workflow run-start path as the webhook
// trigger and the schedule scheduler.
//
// Event → stable `event_kind` string mapping (what the user configures in
// the trigger spec's `event_kind` field):
//   ReviewChanged       → "review_changed"
//   BudgetExceeded      → "budget_exceeded"
//   ProductChanged      → "product_changed"
//   SwarmStatus         → "swarm_status"
//   ImprovementRunFinished → "improvement_run_finished"
//   InsightReady        → "insight_ready"
//   WorkflowRunUpdated  → "workflow_run_updated"
//
// Keep this mapping stable: users configure it by string in the trigger spec.
// ---------------------------------------------------------------------------

/// Map a daemon `Event` to the stable `event_kind` string a user puts in
/// their trigger's spec.  Returns `None` for events that are not useful as
/// automation triggers (session churn, low-level ticks, etc.).
fn event_to_kind(event: &Event) -> Option<&'static str> {
    match event {
        Event::ReviewChanged { .. }         => Some("review_changed"),
        Event::BudgetExceeded { .. }        => Some("budget_exceeded"),
        Event::ProductChanged { .. }        => Some("product_changed"),
        Event::SwarmStatus { .. }           => Some("swarm_status"),
        Event::ImprovementRunFinished { .. } => Some("improvement_run_finished"),
        Event::InsightReady { .. }          => Some("insight_ready"),
        Event::WorkflowRunUpdated { .. }    => Some("workflow_run_updated"),
        // Session, metric, notice, trail, task, swarm-run, improvement-edit,
        // skill-eval, swarm-message, swarm-task, meta-updated events are
        // deliberately excluded — too noisy or not useful as macro triggers.
        _ => None,
    }
}

/// Start the event-trigger listener task. Returns a cancel flag; set to `true`
/// to stop the loop (mirrors the schedule scheduler pattern).
pub fn spawn_workflow_event_trigger_listener(ctx: ServerCtx) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel2 = Arc::clone(&cancel);
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        loop {
            if cancel2.load(Ordering::Relaxed) {
                return;
            }
            let event = match rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!("workflow event-trigger listener: lagged by {n} events; continuing");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("workflow event-trigger listener: event bus closed; stopping");
                    return;
                }
            };

            let Some(kind_str) = event_to_kind(&event) else {
                continue;
            };

            // Load enabled event triggers whose spec declares this kind.
            let triggers_repo = TriggersRepo::new(ctx.pool.clone());
            let triggers = match triggers_repo.list_enabled_by_kind("event").await {
                Ok(t) => t,
                Err(e) => {
                    warn!("workflow event-trigger listener: list triggers: {e}");
                    continue;
                }
            };

            let matching: Vec<_> = triggers
                .into_iter()
                .filter(|t| {
                    t.spec
                        .get("event_kind")
                        .and_then(Value::as_str)
                        == Some(kind_str)
                })
                .collect();

            if matching.is_empty() {
                continue;
            }

            let workflows_repo = WorkflowsRepo::new(ctx.pool.clone());
            for trigger in matching {
                // Resolve the workflow; skip silently when it was deleted.
                let wf = match workflows_repo.get(&trigger.workflow_id).await {
                    Ok(w) => w,
                    Err(_) => continue,
                };
                let ws = match ctx.workspaces.get(&wf.workspace_id).await {
                    Ok(w) => w,
                    Err(_) => continue,
                };

                // Build the run input: include the trigger kind so the workflow
                // graph can branch or log on it.
                let input = json!({
                    "trigger": "event",
                    "event_kind": kind_str,
                });

                let run = match workflows_repo
                    .create_run(&wf.id, &wf.workspace_id, &input)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(
                            workflow_id = %wf.id,
                            event_kind = kind_str,
                            "workflow event-trigger listener: create run: {e}"
                        );
                        continue;
                    }
                };

                info!(
                    workflow_id = %wf.id,
                    run_id = %run.id,
                    event_kind = kind_str,
                    "workflow event-trigger listener: firing event trigger"
                );

                let ctx2 = ctx.clone();
                let run_id = run.id.clone();
                tokio::spawn(async move {
                    crate::workflow_engine::run_workflow(
                        ctx2, ws, wf, run_id,
                        input,
                        None, false,
                    )
                    .await;
                });
            }
        }
    });
    cancel
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn timezone_is_threaded_through() {
        // A daily 09:00 trigger in a +/- tz is interpreted in that tz, not UTC.
        // Just assert it doesn't panic and respects the spec shape.
        let s = json!({ "cadence": "daily", "at": "09:00", "timezone": "America/New_York" });
        let _ = is_due(&s, Utc::now());
    }
}
