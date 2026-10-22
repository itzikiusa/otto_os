//! Review lifecycle: status transitions (persist + broadcast in one call) and
//! the per-agent cancel registry.

use super::*;

/// Persist a review's `status` (+ `error` note) and broadcast
/// `Event::ReviewChanged` in ONE call, so no exit path can update the row and
/// forget the broadcast (which leaves open panels on a stale "running"). A
/// failed write is logged and still broadcast (subscribers re-read the row,
/// so they see whatever actually persisted) and returned to the caller.
pub(crate) async fn transition(
    ctx: &ServerCtx,
    workspace_id: &str,
    review_id: &str,
    status: ReviewStatus,
    error: Option<&str>,
) -> Result<()> {
    let persisted = ctx
        .reviews_store
        .set_status(&review_id.to_string(), status, error)
        .await;
    if let Err(e) = &persisted {
        tracing::error!(review = %review_id, status = status.as_str(), "set review status: {e}");
    }
    announce(ctx, workspace_id, review_id, status);
    persisted
}

/// Broadcast `Event::ReviewChanged` for a status that is already persisted
/// (a freshly created `running` row, a per-agent row change) — the one place
/// the review-level event is built.
pub(crate) fn announce(ctx: &ServerCtx, workspace_id: &str, review_id: &str, status: ReviewStatus) {
    let _ = ctx.events.send(Event::ReviewChanged {
        workspace_id: workspace_id.to_string(),
        session_id: None,
        review_id: review_id.to_string(),
        status: status.as_str().to_string(),
    });
}

/// Register (or replace) the cancel flag for one review agent. The guard
/// holds a fresh, un-tripped flag — replacing matters on retry, where a stale
/// tripped flag would short-circuit the new run instantly — and unregisters it
/// on drop only while the registry still holds it (a newer retry's flag of the
/// same agent survives).
pub(crate) fn register_review_agent_cancel(
    reg: &crate::skill_eval::CancelRegistry,
    review_id: &str,
    index: usize,
) -> CancelGuard {
    reg.register(&review_agent_cancel_key(review_id, index))
}

/// Trip one review agent's cancel flag, if registered (no-op otherwise — e.g.
/// a seeded/orphaned row with no live recovery loop).
pub(crate) fn signal_review_agent_cancel(
    reg: &crate::skill_eval::CancelRegistry,
    review_id: &str,
    index: usize,
) {
    reg.signal(&review_agent_cancel_key(review_id, index));
}

/// Remove the review's `otto-review-<id>*` temp files when NO agent of it is
/// registered any more (no original run, no other retry in flight). Checked
/// and swept under the registry lock, so a retry registering concurrently
/// either sees its files kept or writes them after the sweep. Sync: run it on
/// the blocking pool (the temp-dir walk blocks).
pub(crate) fn sweep_review_temp_files_if_idle(
    reg: &crate::skill_eval::CancelRegistry,
    review_id: &str,
) {
    let map = reg.lock_map();
    let mine = format!("{review_id}:");
    if map.keys().any(|k| k.starts_with(&mine)) {
        return;
    }
    remove_review_temp_files_in(&std::env::temp_dir(), &format!("otto-review-{review_id}"));
}

/// Shared cancellation for the review endpoint and workflows that own reviews.
/// Signal the review itself before killing PTYs: a killed summarizer is otherwise
/// indistinguishable from a provider failure eligible for deterministic fallback.
pub(crate) async fn cancel_running_review(ctx: &ServerCtx, review: &Review, workspace_id: &Id) {
    if review.status != ReviewStatus::Running {
        return;
    }
    let review_id = review.id.clone();
    // 1. Signal the cancel flags so each agent's recovery loop short-circuits and
    //    run_review_core skips the summarizer / finding persistence. The review-
    //    level flag gates the post-join summarizer skip; the recovery loops watch
    //    their per-agent flags (per-agent Stop), so trip those too.
    //    Only an attempt that is actually running has a flag; trip it, never
    //    insert one — a tripped orphan would cancel a later rerun of this id
    //    (the durable `cancelled` status below covers flagless paths).
    ctx.review_cancels.signal(review_id.as_str());
    for i in 0..review.agents.len() {
        signal_review_agent_cancel(&ctx.review_agent_cancels, &review_id, i);
    }

    // 2. Kill the live agent sessions — PtyHandle has no Drop-kill, so this
    //    explicit reap is what actually stops the in-flight agents.
    for agent in &review.agents {
        if let Some(sid) = &agent.session_id {
            let _ = ctx.manager.kill_session(sid).await;
        }
    }

    // 3. Best-effort cleanup: temp files (diff + per-agent prompt/json) AND the
    //    durable prompt/diff rows (0100) — same lifecycle, cancelled runs are
    //    not retryable.
    remove_review_temp_files(&review_id).await;
    let _ = ctx.reviews_store.delete_run_artifacts(&review_id).await;

    // 4. Mark the run cancelled (the background task sees the flag and won't
    //    overwrite this with done/error) and broadcast the terminal status.
    let _ = transition(ctx, workspace_id, &review_id, ReviewStatus::Cancelled, None).await;
}
