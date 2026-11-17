//! Provider-aware role turns. Role slots follow executor slots, preserving their
//! stable retry indices while retaining every role session for reload/history.
use crate::agent::RoleTurn;
use crate::AutomationCtx;
use otto_core::domain::{GoalLoop, GoalLoopRoleCfg, LoopAgentState};
use otto_core::{Error, Id, Result};
use std::sync::atomic::Ordering;
use std::time::Duration;

fn valid_result(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| {
            v.get("result")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
        })
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    ctx: &impl AutomationCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    name: &str,
    role: &GoalLoopRoleCfg,
    prompt: String,
    cwd: &str,
    budget: u64,
) -> Result<String> {
    // Bind the controller that started this role, before any await. A later
    // start replaces the registry entry; this role must keep watching the
    // handle it began with, and stop when that handle is no longer current.
    let bound = ctx.goal_loops().lock().unwrap().get(&loop_.id).cloned();
    let workspace = ctx.workspaces().get(&loop_.workspace_id).await?;
    let user = otto_state::UsersRepo::new(ctx.pool().clone())
        .get(&loop_.created_by)
        .await?;
    let provider = ctx
        .resolve_provider(Some(&workspace), Some(&role.provider))
        .await?;
    let mut state = LoopAgentState {
        name: name.into(),
        provider: provider.clone(),
        model: role.model.clone(),
        status: "pending".into(),
        note: String::new(),
        session_id: None,
        output_summary: None,
    };
    let index = ctx
        .goal_loops_repo()
        .append_iter_agent(iter_id, &state)
        .await?;
    let dir = tempfile::Builder::new()
        .prefix("otto-goal-role-")
        .tempdir()
        .map_err(|e| Error::Internal(e.to_string()))?;
    let output = dir.path().join("result.json");
    let temporary = dir.path().join("result.json.tmp");
    let prompt = format!("{prompt}\n\nExecution policy: this role is read-only except for its result file. Do not edit source, stage, commit, push, publish or send messages.\n\nFinally encode your full response as a JSON object with one string field named result. Write the complete JSON to {}, then atomically rename it to {}. A chat reply alone does not finish the turn.", temporary.display(), output.display());
    let meta = serde_json::json!({"source":"goal_loop", "loop_id":loop_.id,
        "role":name, "model":role.model});
    // Parks on the loop flag bell (perf W8) instead of a 100 ms poll.
    let loop_id = &loop_.id;
    let stop_asked = || {
        let current = ctx.goal_loops().lock().unwrap().get(loop_id).cloned();
        let replaced = match (&bound, &current) {
            (Some(old), Some(now)) => !std::sync::Arc::ptr_eq(&old.cancel, &now.cancel),
            (Some(_), None) => true,
            _ => false,
        };
        replaced
            || bound.as_ref().is_some_and(|h| {
                h.interrupted.load(Ordering::Relaxed)
                    || h.cancel.load(Ordering::Relaxed)
                    || h.paused.load(Ordering::Relaxed)
            })
    };
    let cancelled = crate::goal_loop::until_flag(stop_asked);
    let turn = RoleTurn {
        workspace: &workspace,
        user: &user,
        title: name,
        cwd,
        provider: &provider,
        meta,
        prompt: &prompt,
        budget: Duration::from_secs(budget),
        done_file: output,
        done_file_validator: valid_result,
    };
    let result = ctx
        .run_role_turn(turn, cancelled, |id| {
            state.session_id = Some(id);
            state.status = "running".into();
            let state = state.clone();
            async move {
                ctx.goal_loops_repo()
                    .set_iter_agent_at(iter_id, index, &state)
                    .await
                    .map_err(|e| e.to_string())
            }
        })
        .await;
    // Validate before the one status write: an unusable result fails the role,
    // and a person's pause/stop is "stopped", not a failure.
    let result = result.and_then(|text| role_result(&text));
    (state.status, state.note) = match &result {
        Ok(_) => ("done".into(), String::new()),
        Err(e) if stop_asked() => ("stopped".into(), e.clone()),
        Err(e) => ("error".into(), e.clone()),
    };
    ctx.goal_loops_repo()
        .set_iter_agent_at(iter_id, index, &state)
        .await?;
    result.map_err(Error::Upstream)
}

/// The `result` string of a role's JSON envelope.
fn role_result(text: &str) -> std::result::Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("invalid role result: {e}"))?;
    value["result"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "missing role result".into())
}

/// Definition happens before a loop exists, but still uses the configured
/// provider and a managed, inspectable session with bounded cleanup.
pub async fn define(
    ctx: &impl AutomationCtx,
    workspace: &otto_core::domain::Workspace,
    user: &otto_core::domain::User,
    role: &GoalLoopRoleCfg,
    prompt: &str,
    cwd: &str,
) -> Result<String> {
    let dir = tempfile::Builder::new()
        .prefix("otto-goal-define-")
        .tempdir()
        .map_err(|e| Error::Internal(e.to_string()))?;
    let output = dir.path().join("result.json");
    let prompt = format!("{prompt}\nDo not change source, commit, push, or publish. Encode your full response in a JSON object with one string field result. Write the complete JSON to {}, then atomically rename it to {}.", dir.path().join("result.tmp").display(), output.display());
    let provider = ctx
        .resolve_provider(Some(workspace), Some(&role.provider))
        .await?;
    let turn = RoleTurn {
        workspace,
        user,
        title: "Goal definer",
        cwd,
        provider: &provider,
        meta: serde_json::json!({"source":"goal_loop", "role":"definer", "model":role.model}),
        prompt: &prompt,
        budget: Duration::from_secs(300),
        done_file: output,
        done_file_validator: valid_result,
    };
    let result = ctx
        .run_role_turn(turn, std::future::pending(), |_| async { Ok(()) })
        .await
        .map_err(Error::Upstream)?;
    let value: serde_json::Value =
        serde_json::from_str(&result).map_err(|e| Error::Upstream(e.to_string()))?;
    value["result"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Upstream("missing definition result".into()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn accepts_only_complete_role_envelopes() {
        assert!(!super::valid_result(r#"{"result":"partial"#));
        assert!(!super::valid_result("{}"));
        assert!(!super::valid_result(r#"{"result":"  "}"#));
        assert!(super::valid_result(r#"{"result":"complete"}"#));
    }

    #[test]
    fn role_result_rejects_non_json_and_missing_result() {
        assert!(super::role_result("not json").is_err());
        assert!(super::role_result(r#"{"other":"x"}"#).is_err());
        assert_eq!(super::role_result(r#"{"result":"ok"}"#).unwrap(), "ok");
    }
}
