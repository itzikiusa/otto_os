//! Notification-center notices for unattended automation (review 08 · N1).
//!
//! Scheduled tasks, personal agents, workflow runs and goal loops used to
//! finish only onto the event bus: a daily task could fail for a week and
//! nobody noticed. Each engine now calls in here when a run fails (or a
//! workflow waits on a human), and a notice lands in the notification center
//! with a click-through to the run ([`NoticeAction::OpenRoute`]).
//!
//! **One notice per failure streak.** A streak is keyed per entity (task /
//! agent / workflow / loop). The first failure of a streak posts the notice;
//! later failures in the same streak stay silent (a 15-minute watchdog that
//! keeps failing must not flood the bell); a success ends the streak. The
//! streak set is in-memory: after a daemon restart the next failure notifies
//! again, refreshing the SAME row (the notice's `source_key` is the streak
//! key, which the repo de-dupes on) rather than adding one.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use otto_core::domain::{NoticeAction, NoticeKind, NoticeSeverity};
use otto_core::event::Event;
use otto_state::{NewNotice, WorkflowsRepo};
use tokio::sync::broadcast::error::RecvError;
use tracing::{info, warn};

use crate::state::ServerCtx;

/// Streak keys whose failure notice has already been posted.
fn streaks() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// True when this failure STARTS a streak (so it should notify); records it.
fn begin_streak(key: &str) -> bool {
    streaks()
        .lock()
        .map(|mut s| s.insert(key.to_string()))
        .unwrap_or(true)
}

/// A success ends the entity's failure streak; the next failure notifies.
pub fn clear_streak(key: &str) {
    if let Ok(mut s) = streaks().lock() {
        s.remove(key);
    }
}

/// The streak / `source_key` for an automation entity, e.g.
/// `automation:scheduled_task:<id>`.
pub fn streak_key(kind: &str, id: &str) -> String {
    format!("automation:{kind}:{id}")
}

/// What a notice is about and where its click lands.
pub struct RunNotice {
    /// Streak key ([`streak_key`]) — also the notice's de-dupe `source_key`.
    pub key: String,
    pub severity: NoticeSeverity,
    pub title: String,
    pub body: String,
    /// In-app route the click opens (`workflows/<wf>/runs/<run>`, …).
    pub route: String,
    pub workspace_id: Option<String>,
    /// Owning user (the task/workflow/loop creator); `None` = everyone.
    pub user_id: Option<String>,
}

/// Post a failure notice — only for the first failure of a streak.
pub async fn notify_failure(ctx: &ServerCtx, n: RunNotice) {
    if !begin_streak(&n.key) {
        return;
    }
    post(ctx, n).await;
}

/// Post a notice unconditionally (de-duped on its key by the repo): used for
/// "a workflow is waiting for your approval", one per run.
pub async fn post(ctx: &ServerCtx, n: RunNotice) {
    let res = ctx
        .notifications()
        .create(NewNotice {
            kind: NoticeKind::System,
            severity: n.severity,
            title: n.title,
            body: clip(&n.body, 300),
            source_key: Some(n.key),
            action: Some(NoticeAction::OpenRoute {
                route: n.route,
                workspace_id: n.workspace_id,
            }),
            user_id: n.user_id,
        })
        .await;
    if let Err(e) = res {
        warn!("automation notice: {e}");
    }
}

/// Workflow runs and goal loops report through the event bus; this listener
/// turns their terminal / attention states into notices (scheduled tasks and
/// personal agents notify inline from their engines). Parks on the bus, so it
/// costs no idle wakeups.
pub fn spawn_listener(ctx: ServerCtx) {
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        loop {
            let ev = match rx.recv().await {
                Ok(e) => e,
                Err(RecvError::Lagged(n)) => {
                    warn!("automation notices: lagged by {n} events");
                    continue;
                }
                Err(RecvError::Closed) => {
                    info!("automation notices: event bus closed; stopping");
                    return;
                }
            };
            match ev {
                Event::WorkflowRunUpdated {
                    run_id,
                    status,
                    node_id,
                    waiting_approval,
                    ..
                } => {
                    on_workflow_run(&ctx, &run_id, &status, node_id.as_deref(), waiting_approval)
                        .await
                }
                Event::GoalLoopUpdated {
                    loop_id, status, ..
                } => on_goal_loop(&ctx, &loop_id, &status).await,
                _ => {}
            }
        }
    });
}

/// What a workflow-run event means for the notice center.
#[derive(Debug, PartialEq, Eq)]
enum RunVerdict {
    /// Failed: notice once per streak.
    Failed,
    /// Succeeded: end the streak.
    Succeeded,
    /// Parked at a human-approval node: one notice per (run, node).
    NeedsApproval,
    Nothing,
}

fn run_verdict(status: &str, waiting_approval: bool) -> RunVerdict {
    match status {
        "error" => RunVerdict::Failed,
        "success" => RunVerdict::Succeeded,
        "running" if waiting_approval => RunVerdict::NeedsApproval,
        _ => RunVerdict::Nothing,
    }
}

/// Prefix of a run's approval-gate dedupe keys (`…:{run}:{node}`).
fn approval_gate_prefix(run_id: &str) -> String {
    format!("automation:workflow_approval:{run_id}:")
}

/// Drop a settled run's approval-gate keys (perf W11): they were inserted per
/// (run, node) and never removed, so the streak set grew for the daemon's
/// whole life.
fn clear_approval_gates(run_id: &str) {
    let prefix = approval_gate_prefix(run_id);
    if let Ok(mut s) = streaks().lock() {
        s.retain(|k| !k.starts_with(&prefix));
    }
}

async fn on_workflow_run(
    ctx: &ServerCtx,
    run_id: &str,
    status: &str,
    node_id: Option<&str>,
    waiting_approval: bool,
) {
    if matches!(status, "success" | "error" | "canceled") {
        clear_approval_gates(run_id);
    }
    let verdict = run_verdict(status, waiting_approval);
    if verdict == RunVerdict::Nothing {
        return;
    }
    // Once per approval gate, not per event the parked run emits — and
    // decided BEFORE any DB read (perf W11: every event a parked run emitted
    // used to pay a full run-row read first).
    let gate = (verdict == RunVerdict::NeedsApproval)
        .then(|| format!("{}{}", approval_gate_prefix(run_id), node_id.unwrap_or("")));
    if let Some(g) = &gate {
        if !begin_streak(g) {
            return;
        }
    }
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    // Status/ids/error only — never the 50–200 KB `nodes_json` (perf W11).
    let Ok(Some(run)) = repo.run_head(&run_id.to_string(), false).await else {
        return;
    };
    let key = streak_key("workflow", &run.workflow_id);
    if verdict == RunVerdict::Succeeded {
        clear_streak(&key);
        return;
    }
    let Ok(wf) = repo.get(&run.workflow_id).await else {
        return;
    };
    let route = format!("workflows/{}/runs/{}", wf.id, run.id);
    let base = RunNotice {
        key: key.clone(),
        severity: NoticeSeverity::Error,
        title: String::new(),
        body: String::new(),
        route,
        workspace_id: Some(run.workspace_id.clone()),
        user_id: Some(wf.created_by.clone()),
    };
    if let Some(gate) = gate {
        post(
            ctx,
            RunNotice {
                key: gate,
                severity: NoticeSeverity::Warn,
                title: format!("Workflow “{}” is waiting for your approval", wf.name),
                body: "A human-approval step paused the run. Open it to approve or reject.".into(),
                ..base
            },
        )
        .await;
        return;
    }
    notify_failure(
        ctx,
        RunNotice {
            title: format!("Workflow “{}” failed", wf.name),
            body: run
                .error
                .clone()
                .unwrap_or_else(|| "The run ended with an error.".into()),
            ..base
        },
    )
    .await;
}

/// Goal-loop status → (severity, title suffix) for statuses worth a notice;
/// `None` for the rest. A running loop ends the streak.
fn loop_notice(status: &str) -> Option<(NoticeSeverity, &'static str)> {
    match status {
        "exhausted" => Some((NoticeSeverity::Warn, "ran out of budget")),
        "blocked" => Some((NoticeSeverity::Warn, "is blocked and needs you")),
        "failed" => Some((NoticeSeverity::Error, "failed")),
        "succeeded" => Some((NoticeSeverity::Info, "reached its goal")),
        _ => None,
    }
}

async fn on_goal_loop(ctx: &ServerCtx, loop_id: &str, status: &str) {
    let key = streak_key("goal_loop", loop_id);
    let Some((severity, what)) = loop_notice(status) else {
        if status == "running" {
            clear_streak(&key);
        }
        return;
    };
    if !begin_streak(&key) {
        return;
    }
    let Ok(l) = ctx.goal_loops_repo.get(&loop_id.to_string()).await else {
        return;
    };
    post(
        ctx,
        RunNotice {
            key,
            severity,
            title: format!("Goal loop “{}” {what}", l.name),
            body: l.error.clone().or(l.summary.clone()).unwrap_or_default(),
            route: format!("loops/{}", l.id),
            workspace_id: Some(l.workspace_id.clone()),
            user_id: Some(l.created_by.clone()),
        },
    )
    .await;
}

/// First `max` chars of a (possibly long, multi-line) error, on one line.
fn clip(s: &str, max: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        return one;
    }
    let mut out: String = one.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_notice_per_failure_streak() {
        let k = streak_key("scheduled_task", "t-streak-test");
        assert!(begin_streak(&k), "first failure notifies");
        assert!(!begin_streak(&k), "repeat failures stay silent");
        assert!(!begin_streak(&k));
        clear_streak(&k);
        assert!(begin_streak(&k), "a success ends the streak");
        clear_streak(&k);
    }

    #[test]
    fn streaks_are_per_entity() {
        let a = streak_key("workflow", "wf-a-test");
        let b = streak_key("workflow", "wf-b-test");
        assert!(begin_streak(&a));
        assert!(begin_streak(&b));
        clear_streak(&a);
        clear_streak(&b);
    }

    #[test]
    fn workflow_events_map_to_verdicts() {
        assert_eq!(run_verdict("error", false), RunVerdict::Failed);
        assert_eq!(run_verdict("success", false), RunVerdict::Succeeded);
        assert_eq!(run_verdict("running", true), RunVerdict::NeedsApproval);
        assert_eq!(run_verdict("running", false), RunVerdict::Nothing);
        // A user's cancel is not a failure to tell them about.
        assert_eq!(run_verdict("canceled", false), RunVerdict::Nothing);
    }

    #[test]
    fn a_settled_run_drops_its_approval_gate_keys() {
        let g1 = format!("{}n1", approval_gate_prefix("run-gate-test"));
        let g2 = format!("{}n2", approval_gate_prefix("run-gate-test"));
        let other = format!("{}n1", approval_gate_prefix("run-gate-other"));
        assert!(begin_streak(&g1) && begin_streak(&g2) && begin_streak(&other));
        clear_approval_gates("run-gate-test");
        assert!(begin_streak(&g1), "gate key removed");
        assert!(begin_streak(&g2), "gate key removed");
        assert!(!begin_streak(&other), "other runs' gates untouched");
        clear_approval_gates("run-gate-test");
        clear_approval_gates("run-gate-other");
    }

    #[test]
    fn goal_loop_statuses_worth_a_notice() {
        assert!(loop_notice("exhausted").is_some());
        assert!(loop_notice("blocked").is_some());
        assert!(loop_notice("failed").is_some());
        assert!(loop_notice("succeeded").is_some());
        assert!(loop_notice("running").is_none());
        assert!(loop_notice("paused").is_none());
        assert!(loop_notice("stopped").is_none());
    }

    #[test]
    fn clip_flattens_and_truncates() {
        assert_eq!(clip("a\n  b\tc", 10), "a b c");
        assert_eq!(clip("abcdef", 3), "abc…");
    }
}
