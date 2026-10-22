//! The end of a workflow run, after the node loop: stop a canceled / timed-out
//! run's agents, drain the chat progress pump, then write the terminal status
//! (CAS on in-flight), deliver the result, emit the final event and reap the
//! run's worktrees. Moved verbatim out of `run_workflow_with_budget`.

use super::super::*;

/// Finish a run whose node loop has exited. Takes ownership of the driver's
/// state: nothing runs after it.
#[allow(clippy::too_many_arguments)]
pub(in crate::workflow_engine) async fn finish(
    ctx: ServerCtx,
    repo: WorkflowsRepo,
    workflow: Workflow,
    run_id: Id,
    mut states: Vec<NodeRunState>,
    input: Value,
    files: std::sync::Arc<crate::workflow_context::RunContextFiles>,
    progress: ProgressSink,
    progress_pump: Option<tokio::task::JoinHandle<()>>,
    canceled: bool,
    timed_out: bool,
) {
    // A stopped run stops its agents and reviewers NOW: the drain below can
    // take minutes (each queued chat post has a 30 s timeout on a slow or
    // offline Slack), and the agents kept editing the worktree meanwhile.
    // The finalize paths repeat this (idempotent).
    if canceled || timed_out {
        stop_run_reviews(&ctx, &repo, &workflow.workspace_id, &run_id, &states).await;
        kill_run_sessions(&ctx, &states).await;
    }
    // Flush all streamed progress lines (close the channel, await the pump) so the
    // per-step updates land in the thread BEFORE the final summary is delivered.
    drop(progress);
    if let Some(h) = progress_pump {
        let _ = h.await;
    }

    if canceled {
        finalize_canceled_run(&ctx, &repo, &workflow, &run_id, &mut states, &input).await;
        return;
    }

    if timed_out {
        stop_run_reviews(&ctx, &repo, &workflow.workspace_id, &run_id, &states).await;
        kill_run_sessions(&ctx, &states).await;
        // Keep the interrupted step retryable; only unreached work is skipped.
        for s in states.iter_mut() {
            if s.status == NodeStatus::Running {
                s.status = NodeStatus::Error;
                s.error = Some("workflow active time budget exhausted".into());
            } else if s.status == NodeStatus::Pending {
                s.status = NodeStatus::Skipped;
            }
        }
        let msg = format!(
            "run exceeded the {}-hour time limit",
            RUN_WALL_CLOCK_TIMEOUT.as_secs() / 3600
        );
        // Terminal writes are CAS on in-flight: a cancel that landed after the
        // last boundary check wins, and is finalized as a cancel.
        let rev = match write_terminal(
            &repo,
            &run_id,
            &[RunStatus::Pending, RunStatus::Running],
            RunStatus::Error,
            &states,
            Some(&msg),
        )
        .await
        {
            Ok(Some(rev)) => rev,
            Ok(None) => {
                finalize_canceled_run(&ctx, &repo, &workflow, &run_id, &mut states, &input).await;
                return;
            }
            // The row is still live: delivering a result now would disagree
            // with the `error` the orphan sweep writes once this driver exits
            // — the sweep delivers and cleans up instead (S3-305).
            Err(e) => {
                tracing::warn!(%run_id, "workflow run finalize write failed (left to the orphan sweep): {e}");
                return;
            }
        };
        deliver_run_result(
            &ctx,
            &workflow,
            &states,
            RunStatus::Error,
            None,
            &input,
            None,
        )
        .await;
        emit_run_updated(
            &ctx,
            &workflow.workspace_id,
            &run_id,
            "error",
            None,
            rev,
            None,
            &states,
            false,
        );
        clear_skip_markers(&ctx, &run_id);
        reap_run_worktrees(&ctx, &run_id).await;
        return;
    }

    let any_error = states.iter().any(|s| s.status == NodeStatus::Error);
    let final_status = if any_error {
        RunStatus::Error
    } else {
        RunStatus::Success
    };
    let err_msg = if any_error {
        Some("one or more nodes failed".to_string())
    } else {
        None
    };
    let rev = match write_terminal(
        &repo,
        &run_id,
        &[RunStatus::Pending, RunStatus::Running],
        final_status,
        &states,
        err_msg.as_deref(),
    )
    .await
    {
        Ok(Some(rev)) => rev,
        // Canceled after the last node boundary: the cancel wins (it used to
        // be silently overwritten with success).
        Ok(None) => {
            finalize_canceled_run(&ctx, &repo, &workflow, &run_id, &mut states, &input).await;
            return;
        }
        // The row is still live: delivering a result now would disagree with
        // the `error` the orphan sweep writes once this driver exits — the
        // sweep delivers and cleans up instead (S3-305).
        Err(e) => {
            tracing::warn!(%run_id, "workflow run finalize write failed (left to the orphan sweep): {e}");
            return;
        }
    };
    // The run's deliverable: a copy of the last content-bearing step's handoff
    // file, only on outright success — an errored run has no coherent "answer"
    // to hand back, so delivery falls back to the per-step summary.md instead.
    let final_output = if final_status == RunStatus::Success {
        files.write_final_output()
    } else {
        None
    };
    // Proof pack: package the run's node outputs, human approvals, and budget
    // gate into inspectable evidence; link the pack to the run. Best-effort.
    let pack_id = assemble_workflow_proof(&ctx, &workflow, &run_id, &states).await;
    if let Some(pid) = &pack_id {
        let _ = repo.set_run_proof_pack(&run_id, pid).await;
    }
    // Report the result back to wherever the run was triggered from (Slack
    // thread / webhook): a brief status + the run's deliverable (final-output.md
    // when the run produced one, else the generated summary.md). Best-effort.
    deliver_run_result(
        &ctx,
        &workflow,
        &states,
        final_status,
        pack_id.as_deref(),
        &input,
        final_output.as_deref(),
    )
    .await;
    // Final event: run complete.
    emit_run_updated(
        &ctx,
        &workflow.workspace_id,
        &run_id,
        final_status.as_str(),
        None,
        rev,
        None,
        &states,
        false,
    );
    // Free the run's provisioned worktrees (+ safe branch cleanup) — repeat
    // automations must not accumulate one worktree/branch per run.
    clear_skip_markers(&ctx, &run_id);
    reap_run_worktrees(&ctx, &run_id).await;
    // Reap earlier runs whose worktrees were kept for a reopenable session
    // that has since gone (or aged past the reopen TTL).
    sweep_stale_run_worktrees(&ctx).await;
}
