//! A daemon restart kills every loop controller. Boot recovery
//! (`GoalLoopsRepo::fail_running`) parks the loops that were running as
//! Paused, with [`RESTART_PAUSE_NOTE`] as their `error`. That used to be the
//! whole story: nothing told the person, so an unattended loop sat paused
//! until someone happened to open it. This posts one bell notice per loop the
//! restart paused and announces the status to open views.
use super::*;

/// The `error` boot recovery stores on a loop it paused. The UI recognises a
/// restart pause by this text on a Paused loop (a person's Pause never sets
/// an error, and Resume clears it).
pub const RESTART_PAUSE_NOTE: &str =
    "Interrupted by a daemon restart — Resume to continue with preserved work.";

/// The notice streak kind. Separate from `goal_loop` (blocked / exhausted /
/// …) so a restart notice neither swallows nor is swallowed by those.
const RESTART_NOTICE_KIND: &str = "goal_loop_restart";

/// Release the swept loops' sessions; for each loop the restart actually
/// paused, emit its Paused status and post a notice that opens it.
/// `swept` is what `fail_running` returned (their pre-sweep rows).
pub async fn announce_restart_pause(ctx: &impl AutomationCtx, swept: &[GoalLoop]) {
    for l in swept {
        cleanup_executor_sessions(ctx, &l.workspace_id, &l.id).await;
        if !paused_by_restart(l.status) {
            continue;
        }
        emit(
            ctx,
            &l.workspace_id,
            &l.id,
            GoalLoopStatus::Paused,
            GoalLoopPhase::Done,
            l.current_iteration,
            l.progress_pct,
        );
        let (title, body) = restart_notice(l);
        ctx.run_notice(
            RESTART_NOTICE_KIND,
            &l.id,
            Some((title, body)),
            format!("loops/{}", l.id),
            Some(l.workspace_id.clone()),
            Some(l.created_by.clone()),
        )
        .await;
    }
}

/// The sweep keeps a Blocked loop Blocked (its question already raised a
/// notice); everything else it returns was running and is now Paused.
fn paused_by_restart(pre_sweep: GoalLoopStatus) -> bool {
    pre_sweep != GoalLoopStatus::Blocked
}

fn restart_notice(l: &GoalLoop) -> (String, String) {
    (
        format!("Goal loop “{}” was paused by a restart", l.name),
        format!(
            "Otto restarted while this loop was running (iteration {}, {}% done). \
             Its work is kept. Open it and Resume to continue.",
            l.current_iteration, l.progress_pct
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swept(status: &str) -> GoalLoop {
        serde_json::from_value(serde_json::json!({
            "id":"g", "workspace_id":"w", "name":"Nightly fix", "repo_path":"/tmp/isolated",
            "definition":{"title":"Goal","acceptance_criteria":[]},
            "config":otto_core::domain::GoalLoopConfig::default(),
            "limits":otto_core::domain::GoalLoopLimits::default(),
            "status":status, "phase":"executing", "iterations_started":2, "current_iteration":2,
            "progress_pct":40, "elapsed_secs":0, "cost_usd":0, "created_by":"u",
            "created_at":Utc::now(), "updated_at":Utc::now()
        }))
        .unwrap()
    }

    #[test]
    fn only_loops_that_were_running_are_announced_as_paused_by_restart() {
        assert!(paused_by_restart(swept("running").status));
        assert!(paused_by_restart(swept("paused").status));
        assert!(!paused_by_restart(swept("blocked").status));
    }

    #[test]
    fn the_restart_notice_names_the_loop_and_says_how_to_continue() {
        let (title, body) = restart_notice(&swept("running"));
        assert!(title.contains("Nightly fix"), "{title}");
        assert!(title.contains("paused by a restart"), "{title}");
        assert!(body.contains("iteration 2, 40% done"), "{body}");
        assert!(body.contains("Resume"), "{body}");
        assert!(RESTART_PAUSE_NOTE.contains("daemon restart"));
    }
}
