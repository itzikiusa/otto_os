//! The `self_improve` workflow node, moved verbatim out of
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
        progress,
        ..
    } = call;
    let p = &node.params;
    use otto_core::domain::{Autonomy, ImprovementTrigger};
    let eng = &ctx.improve_engine;
    // Provider picker on the node: `providers` (array) OR `provider`
    // (single string) overrides the workspace's Self-Improvement providers
    // for THIS run. Empty ⇒ fall back to the configured set.
    let providers: Vec<String> = match p.get("providers").and_then(Value::as_array) {
        Some(arr) => arr
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
            .collect(),
        None => p
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
            .into_iter()
            .collect(),
    };
    let run = eng
        .improvements
        .create_run(&ws.id, ImprovementTrigger::Manual)
        .await
        .map_err(|e| otto_core::Error::Internal(format!("self_improve: create run: {e}")))?;
    eng.execute_run_with_autonomy_providers(
        &run.id,
        &ws.id,
        ImprovementTrigger::Manual,
        Autonomy::Propose,
        providers,
    )
    .await
    .map_err(|e| otto_core::Error::Upstream(format!("self_improve: {e}")))?;
    let final_run = eng.improvements.get_run(&run.id).await.ok();
    let edits = eng
        .improvements
        .list_edits_by_run(&run.id)
        .await
        .unwrap_or_default();
    let summary = final_run
        .as_ref()
        .map(|r| r.summary.clone())
        .unwrap_or_default();
    let offered: Vec<Value> = edits
        .iter()
        .map(|e| {
            json!({
                "id": e.id,
                "target": e.target.as_str(),
                "target_ref": e.target_ref,
                "kind": e.kind.as_str(),
                "risk": e.risk.as_str(),
                "rationale": e.rationale,
            })
        })
        .collect();
    let mut logs = vec![format!(
        "self_improve: offered {} improvement(s) — none applied (run {})",
        offered.len(),
        run.id
    )];
    for e in &edits {
        logs.push(format!(
            "  offer: [{}] {} — {}",
            e.risk.as_str(),
            e.target_ref,
            truncate(&e.rationale, 160)
        ));
    }
    if progress.enabled() {
        let mut msg = format!(
                    "💡 *Self-improvement* — {} improvement(s) *offered* (queued for approval; none auto-applied).",
                    offered.len()
                );
        for e in edits.iter().take(8) {
            msg.push_str(&format!(
                "\n • [{}] `{}` — {}",
                e.risk.as_str(),
                e.target_ref,
                truncate(&e.rationale, 140)
            ));
        }
        if !summary.trim().is_empty() {
            msg.push_str(&format!("\n_Summary:_ {}", truncate(&summary, 280)));
        }
        progress.post(msg);
    }
    Ok((
        json!({
            "run_id": run.id,
            "summary": summary,
            "offered": offered.len(),
            "edits": offered,
        }),
        logs,
    ))
}
