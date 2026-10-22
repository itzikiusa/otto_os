//! The `budget_gate` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx, ws, node, env, ..
    } = call;
    let p = &node.params;
    let provider = p
        .get("provider")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(env.default_provider.as_str());
    let workspace_id_override = p
        .get("workspace_id")
        .and_then(Value::as_str)
        .unwrap_or(&ws.id);
    let verdict = crate::budget::check_budget(ctx, workspace_id_override, provider).await;
    if verdict.blocked {
        return Err(otto_core::Error::Upstream(
            verdict
                .reason
                .unwrap_or_else(|| "budget blocked".to_string()),
        ));
    }
    Ok((
        json!({
            "exceeded": verdict.exceeded,
            "blocked": false,
            "reason": verdict.reason,
        }),
        vec![if verdict.exceeded {
            format!(
                "budget_gate: exceeded (warn-only) — {}",
                verdict.reason.as_deref().unwrap_or("")
            )
        } else {
            "budget_gate: under budget".into()
        }],
    ))
}
