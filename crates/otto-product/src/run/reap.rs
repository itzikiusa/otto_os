//! Orphan reaper: auto-resume analysis agents stranded by a daemon restart,
//! and the summarizer-row naming it (and the retry route) tell apart.

use tracing::warn;

use super::retry::retry_analysis_agent;
use crate::host::ProductRunHost;

/// How many times an orphaned agent may be auto-resumed across daemon restarts
/// before the reaper gives up and marks it errored.
const MAX_RESUME_ATTEMPTS: i64 = 2;

/// Summary stamped on an analysis the boot reaper finalizes (S4-10).
pub const INTERRUPTED_SUMMARY: &str =
    "Interrupted by a daemon restart before the summary — re-run the analysis to consolidate.";

/// Display-name prefix of the summarizer agent row (see `run_analysis`).
pub const SUMMARIZER_NAME_PREFIX: &str = "Summarizer \u{00b7} ";

/// True for the analysis's summarizer row (not a lens).
pub fn is_summarizer_agent(name: &str) -> bool {
    name.starts_with(SUMMARIZER_NAME_PREFIX)
}

/// Run ONCE at daemon startup. After a restart, any analysis agent still in
/// `running`/`waiting` has no surviving task driving it, so it is orphaned.
/// For each: if it hasn't exhausted its resume budget, re-run it via
/// [`retry_analysis_agent`] (which rebuilds context + prompt from the DB and runs
/// with full recovery); otherwise mark it errored and notify. Running this only at
/// startup avoids racing legitimately-in-flight agents (there are none yet).
pub async fn reap_orphaned_agents_on_startup<C: ProductRunHost>(ctx: C) {
    // The analysis rows themselves (S4-10): `run_analysis` — the only writer of
    // their final status — died with the previous daemon, so a `running` row
    // would spin the Analysis tab forever. Finalize each as `partial` (a
    // terminal status the UI understands); resumed lens agents still land
    // their findings on their own rows, and a re-run re-summarizes.
    match ctx.product_repo().list_running_analyses().await {
        Ok(running) => {
            for a in running {
                let _ = ctx
                    .product_repo()
                    .set_analysis_status(&a.id, "partial", Some(INTERRUPTED_SUMMARY), true)
                    .await;
            }
        }
        Err(e) => warn!("orphan reaper: list_running_analyses failed: {e}"),
    }
    let agents = match ctx.product_repo().list_unfinished_agents().await {
        Ok(a) => a,
        Err(e) => {
            warn!("orphan reaper: list_unfinished_agents failed: {e}");
            return;
        }
    };
    if agents.is_empty() {
        return;
    }
    warn!(
        "orphan reaper: {} unfinished analysis agent(s) after restart",
        agents.len()
    );

    for agent in agents {
        // The summarizer is not a lens: re-running it through the lens retry
        // would store lens findings on the summarizer row (S4-10).
        if is_summarizer_agent(&agent.name) {
            let _ = ctx
                .product_repo()
                .set_agent_status(
                    &agent.id,
                    "error",
                    None,
                    Some("interrupted by daemon restart — re-run the analysis to re-summarize"),
                    true,
                )
                .await;
            continue;
        }
        let analysis = match ctx.product_repo().get_analysis(&agent.analysis_id).await {
            Ok(a) => a,
            Err(_) => {
                let _ = ctx
                    .product_repo()
                    .set_agent_status(
                        &agent.id,
                        "error",
                        None,
                        Some("interrupted (restart); analysis gone"),
                        true,
                    )
                    .await;
                continue;
            }
        };
        let story = match ctx.product_repo().get_story(&analysis.story_id).await {
            Ok(s) => s,
            Err(_) => {
                let _ = ctx
                    .product_repo()
                    .set_agent_status(
                        &agent.id,
                        "error",
                        None,
                        Some("interrupted (restart); story gone"),
                        true,
                    )
                    .await;
                continue;
            }
        };

        if agent.resume_count >= MAX_RESUME_ATTEMPTS {
            let _ = ctx
                .product_repo()
                .set_agent_status(
                    &agent.id,
                    "error",
                    None,
                    Some("interrupted by daemon restart — gave up after auto-resumes"),
                    true,
                )
                .await;
            let _ = ctx.events().send(otto_core::event::Event::Notice {
                level: "warn".into(),
                title: format!("Analysis agent abandoned: {}", agent.name),
                body: "Restarted too many times to auto-resume.".into(),
            });
            continue;
        }

        let ws = match ctx.workspaces().get(&story.workspace_id).await {
            Ok(w) => w,
            Err(e) => {
                warn!(
                    "orphan reaper: workspace {} load failed: {e}",
                    story.workspace_id
                );
                continue;
            }
        };

        let _ = ctx.product_repo().bump_resume_count(&agent.id).await;
        let _ = ctx.events().send(otto_core::event::Event::Notice {
            level: "info".into(),
            title: format!("Resuming analysis agent: {}", agent.name),
            body: "Re-running after a daemon restart.".into(),
        });
        tokio::spawn(retry_analysis_agent(
            ctx.clone(),
            ws,
            story.created_by.clone(),
            analysis.id.clone(),
            agent.id.clone(),
        ));
    }
}
