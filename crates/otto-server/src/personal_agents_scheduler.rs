//! Personal-agents supervisor: a 60-second tick that fires every enabled
//! schedule of every enabled personal agent whose cadence is due.
//!
//! Same concurrency model as `scheduled_tasks_scheduler` (the `cli_update`
//! ordering): the tick claims a per-**agent** in-flight guard (shared with the
//! manual and delegated run paths); a busy or not-due schedule is skipped
//! **without advancing its cursor**, so the occurrence is retried rather than
//! lost. The engine advances the fired schedule's `last_run_at` cursor only on
//! run completion — each schedule has its own cursor, so an agent's daily recap
//! and its 15-minute needs-attention check never race each other's cursor, and
//! the per-agent guard keeps them from running at once in the agent's shared
//! folder and memory. On startup we **reap** `running` rows left by a previous
//! daemon life.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{info, warn};

use otto_state::PersonalAgentsRepo;

use crate::cadence;
use crate::cancel_signal::CancelSignal;
use crate::personal_agents_engine::{in_flight, run_agent, spawn_proactive_run};
use crate::state::ServerCtx;
use otto_state::AgentAutonomy;

const SCAN: Duration = Duration::from_secs(60);

/// Start the supervisor. Returns its cancel signal; `cancel()` stops the loop at once
/// (mirrors the scheduled-tasks / swarm / cli-update schedulers).
pub fn start(ctx: ServerCtx) -> CancelSignal {
    let cancel = CancelSignal::new();
    tokio::spawn(supervise(ctx, cancel.clone()));
    cancel
}

async fn supervise(ctx: ServerCtx, cancel: CancelSignal) {
    let repo = PersonalAgentsRepo::new(ctx.pool.clone());
    match repo.reap_running().await {
        Ok(n) if n > 0 => info!("personal agents: reaped {n} interrupted run(s) on startup"),
        Ok(_) => {}
        Err(e) => warn!("personal agents: startup reap failed: {e}"),
    }
    loop {
        if cancel.is_cancelled() {
            return;
        }
        if let Err(e) = tick(&ctx, &repo).await {
            warn!("personal agents scheduler tick: {e}");
        }
        // One timer per scan; cancel() wakes it (no 500 ms polling slices).
        if cancel.sleep(SCAN).await {
            return;
        }
    }
}

async fn tick(ctx: &ServerCtx, repo: &PersonalAgentsRepo) -> otto_core::Result<()> {
    let now = Utc::now();
    for (schedule, agent) in repo.list_enabled_schedules().await? {
        // Not due → skip. Busy (any run of this AGENT in flight — scheduled,
        // manual or delegated) → skip WITHOUT advancing the cursor, so the
        // occurrence is retried next tick rather than lost (the engine
        // advances it only on completion).
        // Never before the arm instant (created / resumed / re-timed).
        let last = cadence::effective_cursor(
            &schedule.schedule,
            schedule.last_run_at.as_deref().and_then(parse_ts),
            schedule.armed_at.as_deref().and_then(parse_ts),
        );
        let tz = cadence::task_tz(&schedule.timezone);
        // The creation time anchors a never-run cron (first-fire catch-up).
        let created = parse_ts(&schedule.created_at);
        if !cadence::is_due_since(&schedule.schedule, last, created, now, tz) {
            continue;
        }
        let Some(guard) = in_flight().claim(&agent.id) else {
            continue;
        };

        info!(agent = %agent.id, schedule = %schedule.id, "personal agents: firing due schedule");
        let ctx2 = ctx.clone();
        tokio::spawn(async move {
            // The guard clears the in-flight entry on drop — including on panic.
            let _guard = guard;
            let _ = run_agent(&ctx2, &agent, Some(&schedule), "schedule").await;
        });
    }
    proactive_tick(ctx, repo, now).await;
    Ok(())
}

/// Proactive mode: each enabled agent with proactive on works its standing
/// goals round-robin, at most `runs_per_day` runs in any rolling 24 h, spaced
/// evenly (24 h / runs_per_day apart). A busy agent is skipped (its scheduled
/// and directed work comes first); the next tick tries again.
async fn proactive_tick(ctx: &ServerCtx, repo: &PersonalAgentsRepo, now: DateTime<Utc>) {
    let agents = match repo.list_proactive().await {
        Ok(a) => a,
        Err(e) => {
            warn!("personal agents: proactive scan failed: {e}");
            return;
        }
    };
    let since = (now - chrono::Duration::hours(24)).to_rfc3339();
    for (agent, cfg) in agents {
        let used = repo
            .count_runs_since(&agent.id, "proactive", &since)
            .await
            .unwrap_or(i64::MAX);
        let Some(goal) = next_proactive_goal(&cfg, used, now) else {
            continue;
        };
        // A busy agent answers Conflict — skipped quietly, retried next tick.
        match spawn_proactive_run(ctx, &agent, &goal).await {
            Ok(_) => info!(agent = %agent.id, goal = %goal, "personal agents: proactive run"),
            Err(otto_core::Error::Conflict(_)) => {}
            Err(e) => warn!(agent = %agent.id, "personal agents: proactive run: {e}"),
        }
    }
}

/// The goal a proactive run should work on now, or `None` (off, budget used,
/// too soon after the last proactive run, or no enabled goal). Pure.
pub fn next_proactive_goal(
    cfg: &AgentAutonomy,
    runs_last_24h: i64,
    now: DateTime<Utc>,
) -> Option<String> {
    if !cfg.proactive.enabled {
        return None;
    }
    let per_day = cfg.proactive.runs_per_day.clamp(1, 24);
    if runs_last_24h >= i64::from(per_day) {
        return None;
    }
    let goals: Vec<_> = cfg
        .goals
        .iter()
        .filter(|g| g.enabled && !g.text.trim().is_empty())
        .collect();
    let last_any = goals
        .iter()
        .filter_map(|g| g.last_run_at.as_deref().and_then(parse_ts))
        .max();
    let spacing = chrono::Duration::minutes(i64::from(24 * 60 / per_day));
    if last_any.is_some_and(|t| now - t < spacing) {
        return None;
    }
    // Oldest-worked goal first; a never-worked goal before any worked one.
    goals
        .into_iter()
        .min_by_key(|g| g.last_run_at.as_deref().and_then(parse_ts))
        .map(|g| g.id.clone())
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn goal(id: &str, last: Option<&str>) -> otto_state::StandingGoal {
        otto_state::StandingGoal {
            id: id.into(),
            text: format!("goal {id}"),
            enabled: true,
            last_run_at: last.map(str::to_string),
        }
    }

    #[test]
    fn proactive_goal_respects_switch_budget_spacing_and_round_robin() {
        let now = parse_ts("2026-10-03T12:00:00+00:00").unwrap();
        let mut cfg = AgentAutonomy::default();
        cfg.goals = vec![
            goal("a", Some("2026-10-02T06:00:00+00:00")),
            goal("b", Some("2026-10-02T01:00:00+00:00")),
        ];
        // Off by default.
        assert_eq!(next_proactive_goal(&cfg, 0, now), None);
        cfg.proactive.enabled = true;
        cfg.proactive.runs_per_day = 4;
        // Oldest-worked goal first.
        assert_eq!(next_proactive_goal(&cfg, 0, now).as_deref(), Some("b"));
        // A never-worked goal wins.
        cfg.goals.push(goal("c", None));
        assert_eq!(next_proactive_goal(&cfg, 0, now).as_deref(), Some("c"));
        // Budget used up.
        assert_eq!(next_proactive_goal(&cfg, 4, now), None);
        // Too soon: 4/day ⇒ 6 h spacing; a goal was worked 1 h ago.
        cfg.goals[0].last_run_at = Some("2026-10-03T11:00:00+00:00".into());
        assert_eq!(next_proactive_goal(&cfg, 1, now), None);
        // Disabled / empty goals are never picked.
        cfg.goals = vec![otto_state::StandingGoal {
            enabled: false,
            ..goal("x", None)
        }];
        assert_eq!(next_proactive_goal(&cfg, 0, now), None);
    }

    #[test]
    fn parse_ts_roundtrips() {
        assert!(parse_ts("2026-09-01T10:00:00+00:00").is_some());
        assert!(parse_ts("not-a-time").is_none());
    }
}
