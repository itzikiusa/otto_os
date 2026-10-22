//! The `human_approval` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx,
        ws,
        node,
        input,
        env,
        ..
    } = call;
    let run_id: &Id = &env.run_id;
    let p = &node.params;
    product_publication::expose_pending(ctx, run_id, node, &input).await?;
    let prompt = p
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("Please review and approve to continue");

    // Mark the run as paused-for-approval.  The resume handler sets
    // `waiting_approval = 0` and records the decision. Bump `rev` so
    // clients treat the pause as a fresh state, and announce it over
    // WS immediately — the approval banner/badge must not wait on a poll.
    // Subscribe BEFORE the gate opens so a decision announced the moment
    // it does is never missed.
    let mut decisions = ctx.events.subscribe();
    let pool = &ctx.pool;
    let rev: i64 = sqlx::query_scalar(
        "UPDATE workflow_runs
                 SET waiting_approval = 1, approval_node_id = ?,
                     approved_by = NULL, approval_note = NULL, approved_at = NULL,
                     rev = rev + 1
                 WHERE id = ?
                 RETURNING rev",
    )
    .bind(&node.id)
    .bind(run_id)
    .fetch_one(pool)
    .await
    .map_err(|e| otto_core::Error::Internal(format!("human_approval mark: {e}")))?;
    emit_run_updated(
        ctx,
        &ws.id,
        run_id,
        "running",
        Some(&node.id),
        rev,
        None,
        &[],
        true,
    );

    // Poll for the operator's decision. Operators are HUMANS: the
    // deadline defaults to 24h and is node-tunable via
    // `params.timeout_s` (min 60s) — this arm previously reused the
    // 120-second agent-turn budget, which no PO can meet.
    let timeout_s = p
        .get("timeout_s")
        .and_then(Value::as_u64)
        .unwrap_or(86_400)
        .max(60);
    let deadline = Instant::now() + Duration::from_secs(timeout_s);
    // A parked run hands its run-gate slot back while a human decides
    // (up to 24h) and re-takes it — FIFO — once decided.
    let released = release_run_permit();
    let approval_pause = env.budget.pause();
    let tokio_deadline = tokio::time::Instant::from_std(deadline);
    let decision = loop {
        // Wake on THIS gate's decision (`approve_run` emits a
        // non-waiting update for the gate node), else re-read on the
        // safety cadence. It used to read the row every 2 s for up to
        // 24 h (~43 k reads per parked run).
        let wake_at = (tokio::time::Instant::now() + APPROVAL_SAFETY_RECHECK).min(tokio_deadline);
        wake_on_event(
            &mut decisions,
            |ev| is_gate_decision(ev, run_id, &node.id),
            wake_at,
        )
        .await;
        if Instant::now() >= deadline {
            // Clear the pause flag before erroring so the run doesn't
            // appear stuck after it errors out.
            let _ = sqlx::query("UPDATE workflow_runs SET waiting_approval = 0 WHERE id = ?")
                .bind(run_id)
                .execute(pool)
                .await;
            break Err(otto_core::Error::Upstream(
                "human_approval: timed out waiting for operator decision".into(),
            ));
        }
        // Read the current state of the run row.
        let row = match sqlx::query(
            "SELECT status, waiting_approval, approved_by, approval_note
                     FROM workflow_runs WHERE id = ?",
        )
        .bind(run_id)
        .fetch_optional(pool)
        .await
        {
            Ok(row) => row,
            Err(e) => {
                break Err(otto_core::Error::Internal(format!(
                    "human_approval poll: {e}"
                )))
            }
        };

        let Some(row) = row else {
            break Err(otto_core::Error::Internal(
                "human_approval: run row disappeared".into(),
            ));
        };

        use sqlx::Row as _;
        // A cancel clears `waiting_approval` too — it is NOT a
        // decision. Reading it as one recorded "approved by <the
        // previous approver>" (or "rejected") for a canceled run; keep
        // waiting and let the run's cancel poll stop this step.
        let status: String = row.get("status");
        if status == "canceled" {
            continue;
        }
        let still_waiting: i64 = row.get("waiting_approval");
        if still_waiting == 0 {
            // The resume handler cleared the flag; read the decision.
            // We look for `approved_by` being non-null as "approved".
            let approved_by: Option<String> = row.get("approved_by");
            let note: Option<String> = row.get("approval_note");
            // A null `approved_by` after the wait means the operator
            // explicitly rejected (the resume handler only clears
            // `approved_by` on rejection, leaving it NULL).  Check
            // the `approved_at` column for the "approved" path.
            match approved_by {
                None => {
                    break Err(otto_core::Error::Upstream(format!(
                        "human_approval: rejected — {}",
                        note.as_deref().unwrap_or("no note")
                    )));
                }
                Some(by) => {
                    let mut output =
                        json!({"approved":true,"approved_by":by,"note":note,"prompt":prompt});
                    if let Some(preview) = input.get("publication_preview") {
                        output["publication_preview"] = preview.clone();
                        output["approval_node_id"] = json!(node.id);
                        output["approval_run_id"] = json!(run_id);
                    }
                    break Ok((output, vec![format!("human_approval: approved by {by}")]));
                }
            }
        }
    };
    reacquire_run_permit(released).await;
    drop(approval_pause);
    decision
}
