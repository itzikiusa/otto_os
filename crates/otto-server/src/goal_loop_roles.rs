//! Provider-aware role turns. Role slots follow executor slots, preserving their
//! stable retry indices while retaining every role session for reload/history.
use crate::state::ServerCtx;
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
    ctx: &ServerCtx,
    loop_: &GoalLoop,
    iter_id: &Id,
    name: &str,
    role: &GoalLoopRoleCfg,
    prompt: String,
    cwd: &str,
    budget: u64,
) -> Result<String> {
    let workspace = ctx.workspaces.get(&loop_.workspace_id).await?;
    let user = otto_state::UsersRepo::new(ctx.pool.clone())
        .get(&loop_.created_by)
        .await?;
    let global = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let provider = otto_core::provider::resolve_provider(&[
        role.provider.trim(),
        otto_core::provider::workspace_default(&workspace.settings),
        otto_core::provider::global_default(global.as_ref()),
    ]);
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
        .goal_loops_repo
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
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let opts = crate::agent_session::TurnOpts {
        done_file: Some(output),
        done_file_validator: Some(valid_result),
        ..Default::default()
    };
    let handle = ctx.goal_loops.lock().unwrap().get(&loop_.id).cloned();
    let cancelled = async {
        loop {
            if handle.as_ref().is_some_and(|h| {
                h.cancel.load(Ordering::Relaxed) || h.paused.load(Ordering::Relaxed)
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    };
    let turn = async {
        crate::agent_session::run_session_turn_with(
            ctx,
            &workspace,
            &user,
            None,
            name,
            cwd,
            &provider,
            meta,
            &prompt,
            Duration::from_secs(budget),
            opts,
            |id| {
                let _ = ready_tx.send(id.clone());
            },
        )
        .await
        .map(|(text, _)| text)
        .map_err(|e| format!("{e:?}"))
    };
    let result = crate::review_summarizer::drive(
        turn,
        ready_rx,
        Duration::from_secs(budget),
        cancelled,
        |id| {
            state.session_id = Some(id);
            state.status = "running".into();
            let state = state.clone();
            async move {
                ctx.goal_loops_repo
                    .set_iter_agent_at(iter_id, index, &state)
                    .await
                    .map_err(|e| e.to_string())
            }
        },
        |id| async move {
            crate::review_session::stop_review_sessions(&ctx.manager, &[id]).await;
        },
    )
    .await;
    state.status = if result.is_ok() { "done" } else { "error" }.into();
    state.note = result.as_ref().err().cloned().unwrap_or_default();
    ctx.goal_loops_repo
        .set_iter_agent_at(iter_id, index, &state)
        .await?;
    let text = result.map_err(Error::Upstream)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Upstream(format!("invalid role result: {e}")))?;
    value["result"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Error::Upstream("missing role result".into()))
}

/// Definition happens before a loop exists, but still uses the configured
/// provider and a managed, inspectable session with bounded cleanup.
pub async fn define(
    ctx: &ServerCtx,
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
    let global = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    let provider = otto_core::provider::resolve_provider(&[
        role.provider.trim(),
        otto_core::provider::workspace_default(&workspace.settings),
        otto_core::provider::global_default(global.as_ref()),
    ]);
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let opts = crate::agent_session::TurnOpts {
        done_file: Some(output),
        done_file_validator: Some(valid_result),
        ..Default::default()
    };
    let turn = async {
        crate::agent_session::run_session_turn_with(
            ctx,
            workspace,
            user,
            None,
            "Goal definer",
            cwd,
            &provider,
            serde_json::json!({"source":"goal_loop", "role":"definer", "model":role.model}),
            &prompt,
            Duration::from_secs(300),
            opts,
            |id| {
                let _ = ready_tx.send(id.clone());
            },
        )
        .await
        .map(|(text, _)| text)
        .map_err(|e| format!("{e:?}"))
    };
    let result = crate::review_summarizer::drive(
        turn,
        ready_rx,
        Duration::from_secs(300),
        std::future::pending(),
        |_| async { Ok(()) },
        |id| async {
            crate::review_session::stop_review_sessions(&ctx.manager, &[id]).await;
        },
    )
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
}
