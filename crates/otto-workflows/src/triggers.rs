//! Workflow trigger plumbing that needs no server internals: the event-trigger
//! listener (event bus -> matching `event` triggers -> admitted runs) and the
//! run-input helper shared with the schedule scheduler. Moved out of
//! otto-server's `workflow_trigger_scheduler`; the cadence-driven schedule tick
//! stays there (it shares `cadence` with Scheduled Tasks).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use otto_core::event::Event;
use otto_state::{TriggersRepo, WorkflowsRepo};
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::WorkflowCtx;

/// Copy a trigger spec's result-delivery destinations into a run input map.
/// `deliver_run_result` reads these exact keys from the input; without them a
/// scheduled/event/webhook run completes with no notification anywhere.
pub fn copy_result_destinations(spec: &Value, input: &mut serde_json::Map<String, Value>) {
    for key in [
        "result_channel",
        "result_chat",
        "result_thread",
        "result_webhook",
    ] {
        if let Some(v) = spec
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            input.insert(key.into(), json!(v));
        }
    }
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
        Event::ReviewChanged { .. } => Some("review_changed"),
        Event::BudgetExceeded { .. } => Some("budget_exceeded"),
        Event::ProductChanged { .. } => Some("product_changed"),
        Event::SwarmStatus { .. } => Some("swarm_status"),
        Event::ImprovementRunFinished { .. } => Some("improvement_run_finished"),
        Event::InsightReady { .. } => Some("insight_ready"),
        // `WorkflowRunUpdated` is deliberately NOT triggerable: the engine
        // emits it on EVERY node transition of EVERY run, so a trigger on it
        // recursively spawns runs that emit more of it — an unbounded run
        // explosion. Trigger create/update rejects the kind too; this guard
        // also silences any pre-existing rows.
        //
        // Session, metric, notice, trail, task, swarm-run, improvement-edit,
        // skill-eval, swarm-message, swarm-task, meta-updated events are
        // deliberately excluded — too noisy or not useful as macro triggers.
        _ => None,
    }
}

/// The workspace an event belongs to, for scoping event triggers: a trigger
/// must only fire for events in ITS workflow's workspace, not every workspace
/// on the daemon. `None` (e.g. `InsightReady`, which is daemon-global) matches
/// any workspace.
fn event_workspace(event: &Event) -> Option<&otto_core::Id> {
    match event {
        Event::ReviewChanged { workspace_id, .. }
        | Event::BudgetExceeded { workspace_id, .. }
        | Event::ProductChanged { workspace_id, .. }
        | Event::SwarmStatus { workspace_id, .. }
        | Event::ImprovementRunFinished { workspace_id, .. } => Some(workspace_id),
        _ => None,
    }
}

/// Apply a trigger's optional `filter_json` (a FLAT object of
/// `field: expected` equality checks) against the event's serialized payload.
/// Absent/empty/non-object filters match everything; a field missing from the
/// payload fails the match.
fn filter_matches(filter: Option<&Value>, event_payload: &Value) -> bool {
    let Some(Value::Object(map)) = filter else {
        return true;
    };
    map.iter()
        .all(|(k, expected)| event_payload.get(k) == Some(expected))
}

/// Start the event-trigger listener task. Returns a cancel flag; set to `true`
/// to stop the loop (it parks on the event bus, so it costs no idle wakeups).
pub fn spawn_event_trigger_listener<C: WorkflowCtx>(ctx: C) -> Arc<AtomicBool> {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel2 = Arc::clone(&cancel);
    let mut rx = ctx.events().subscribe();
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
            // Serialized once for `filter_json` matching against payload fields.
            let event_payload = serde_json::to_value(&event).unwrap_or(Value::Null);

            // Load enabled event triggers whose spec declares this kind.
            let triggers_repo = TriggersRepo::new(ctx.pool().clone());
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
                    t.spec.get("event_kind").and_then(Value::as_str) == Some(kind_str)
                        && filter_matches(t.spec.get("filter_json"), &event_payload)
                })
                .collect();

            if matching.is_empty() {
                continue;
            }

            let workflows_repo = WorkflowsRepo::new(ctx.pool().clone());
            for trigger in matching {
                // Resolve the workflow; skip silently when it was deleted.
                let wf = match workflows_repo.get(&trigger.workflow_id).await {
                    Ok(w) => w,
                    Err(_) => continue,
                };
                // Workspace scoping: the event must belong to THIS workflow's
                // workspace (a review in workspace A must not fire workspace
                // B's triggers). Workspace-less events match anywhere.
                if let Some(ev_ws) = event_workspace(&event) {
                    if ev_ws != &wf.workspace_id {
                        continue;
                    }
                }
                // In-flight cap: one live run per workflow — an event storm
                // queues nothing and cannot stack concurrent runs.
                match workflows_repo.has_active_run(&wf.id).await {
                    Ok(false) => {}
                    Ok(true) => {
                        info!(workflow_id = %wf.id, event_kind = kind_str,
                              "workflow event-trigger listener: run already active — skipping");
                        continue;
                    }
                    Err(e) => {
                        warn!(workflow_id = %wf.id, "workflow event-trigger listener: active-run check: {e}");
                        continue;
                    }
                }
                let ws = match ctx.workspaces().get(&wf.workspace_id).await {
                    Ok(w) => w,
                    Err(_) => continue,
                };

                // Build the run input: include the trigger kind so the workflow
                // graph can branch or log on it, plus the trigger's result_*
                // destinations so the run's outcome is delivered somewhere.
                let mut input_map = serde_json::Map::new();
                input_map.insert("trigger".into(), json!("event"));
                input_map.insert("event_kind".into(), json!(kind_str));
                if let Some(p) = trigger
                    .spec
                    .get("prompt")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                {
                    input_map.insert("prompt".into(), json!(p));
                }
                copy_result_destinations(&trigger.spec, &mut input_map);
                let input = Value::Object(input_map);

                // Re-checked atomically with the insert: the early check above
                // only spares the input build; a scheduled task or schedule
                // trigger may admit a run in between.
                let run = match workflows_repo
                    .admit_run_if_idle(&wf.id, &wf.workspace_id, &input, None)
                    .await
                {
                    Ok(Some(r)) => r,
                    Ok(None) => {
                        info!(workflow_id = %wf.id, event_kind = kind_str,
                              "workflow event-trigger listener: run already active — skipping");
                        continue;
                    }
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

                ctx.spawn_run(
                    ws,
                    wf,
                    run.id.clone(),
                    input,
                    otto_core::workflows::RunScope::default(),
                );
            }
        }
    });
    cancel
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_run_updated_is_not_a_fireable_event_kind() {
        // Regression: triggering on the engine's own per-node event recursively
        // spawns runs (N² explosion). The mapping must never expose it.
        let ev = Event::WorkflowRunUpdated {
            workspace_id: "w1".into(),
            run_id: "r1".into(),
            status: "running".into(),
            node_id: None,
            rev: 1,
            node: None,
            nodes_done: 0,
            nodes_total: 0,
            waiting_approval: false,
        };
        assert_eq!(event_to_kind(&ev), None);
    }

    #[test]
    fn filter_json_matches_flat_fields() {
        let payload = json!({"status": "done", "workspace_id": "w1", "n": 3});
        assert!(filter_matches(None, &payload));
        assert!(filter_matches(Some(&json!({})), &payload));
        assert!(filter_matches(Some(&json!({"status": "done"})), &payload));
        assert!(filter_matches(
            Some(&json!({"status": "done", "n": 3})),
            &payload
        ));
        assert!(!filter_matches(
            Some(&json!({"status": "failed"})),
            &payload
        ));
        assert!(!filter_matches(Some(&json!({"missing": "x"})), &payload));
        // Non-object filters are treated as match-all (defensive).
        assert!(filter_matches(Some(&json!("garbage")), &payload));
    }

    #[test]
    fn result_destinations_copy_only_nonempty_strings() {
        let spec = json!({
            "result_channel": "slack",
            "result_chat": "C123",
            "result_thread": "",
            "prompt": "irrelevant",
        });
        let mut input = serde_json::Map::new();
        copy_result_destinations(&spec, &mut input);
        assert_eq!(input.get("result_channel"), Some(&json!("slack")));
        assert_eq!(input.get("result_chat"), Some(&json!("C123")));
        assert!(input.get("result_thread").is_none(), "empty string skipped");
        assert!(input.get("prompt").is_none(), "unrelated keys not copied");
    }
}
