//! Final independent review and completion admission for legacy and graph loops.
use super::*;
use otto_core::domain::GoalLoopLedger;

pub(super) fn ready(loop_: &GoalLoop, ledger: &GoalLoopLedger, eval: &GoalLoopEvaluation) -> bool {
    eval.verdict == "achieved"
        && !eval.criteria.is_empty()
        && eval.criteria.iter().all(|c| c.met)
        && loop_
            .config
            .graph
            .as_ref()
            .is_none_or(|g| crate::goal_loop_graph::all_passed(g, ledger))
}

/// False means interrupted or already blocked/released. The controller retains
/// ownership of stop/pause settlement, just as for its other phase boundaries.
pub(super) async fn review(
    ctx: &impl AutomationCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    wt: &str,
    ledger: &mut GoalLoopLedger,
    handle: &LoopHandle,
) -> bool {
    if loop_.config.require_review {
        let review = run_role(ctx, loop_, iter_id, "Completion reviewer", &loop_.config.evaluator,
                    format!("{}\nIndependently review the final work against the goal. Report concrete unresolved defects. Return JSON {{\"approved\":true|false,\"findings\":[string]}}. Approve only when no material issue remains.", goal_context(loop_)),
                    wt, loop_.limits.per_phase_timeout_secs).await;
        let result = review
            .as_ref()
            .ok()
            .and_then(|text| crate::goal_loop_parse::parse_review(text));
        ledger.review_passed = result.as_ref().is_some_and(|v| {
            v["approved"] == true && v["findings"].as_array().is_some_and(Vec::is_empty)
        });
        ledger.review_summary = review.unwrap_or_else(|e| e.to_string());
        if !ledger.review_passed {
            if let Some(graph) = &loop_.config.graph {
                crate::goal_loop_graph::reopen_completion(graph, ledger);
            }
        }
        warn_on_err(
            &loop_.id,
            "set_ledger",
            ctx.goal_loops_repo().set_ledger(&loop_.id, ledger).await,
        );
        if stop_requested(handle) {
            return false;
        }
        if !ledger.review_passed {
            clear_evaluation_continuation(ctx, &loop_.id).await;
            let _ = block(ctx, &loop_.id, "Completion review has unresolved findings. Inspect the review evidence before resuming.").await; // logged + noticed inside
            release_honouring_stop(ctx, &loop_.id, handle).await;
            return false;
        }
    }
    true
}
