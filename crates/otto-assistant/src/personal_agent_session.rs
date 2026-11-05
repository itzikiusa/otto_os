//! A personal agent's run as a **terminal (PTY) session** — the path for the
//! providers the structured runtime does not drive (`agy`, custom
//! providers); the runtime ones run as Threads threads
//! (`personal_agent_threads`). Moved out of `otto-server`'s
//! `assistant_host`: the agent writes its report to a scratch file the
//! watcher returns, retried per `agent_run::run_with_recovery`.

use std::sync::{Arc, Mutex};

use otto_agent_run::agent_run::{run_with_recovery, watch_for_result, FailReason, RunOutcome};
use otto_core::api::CreateSessionReq;
use otto_core::domain::SessionKind;
use otto_core::{Error, Result};
use otto_sessions::submit::SubmitOpts;
use otto_state::PersonalAgent;
use tracing::warn;

use crate::ctx::{AgentSessionOutcome, AgentSessionRun};
use crate::personal_agents_engine::{
    self as pa_engine, RunPlan, MAX_ATTEMPTS, RETRY_BACKOFF, RUN_NO_PROGRESS, STUCK_IDLE,
    WAITING_IDLE,
};
use crate::AssistantCtx;

/// The visible-session attempt loop for one run.
pub(crate) async fn run_pty_session<C: AssistantCtx>(
    ctx: &C,
    run: AgentSessionRun<'_>,
) -> Result<AgentSessionOutcome> {
    let AgentSessionRun {
        agent,
        run_id,
        owner,
        cwd,
        prompt,
        plan,
        cancel,
    } = run;
    let ws = ctx.workspaces().get(&agent.workspace_id).await?;

    // The agent writes its report here; the watcher returns its contents.
    let out_path = crate::personal_agent_threads::report_path(ctx, agent, run_id);
    if let Some(p) = out_path.parent() {
        let _ = tokio::fs::create_dir_all(p).await;
    }
    let _ = std::fs::remove_file(&out_path);
    let augmented = C::augment_report_prompt(prompt, &out_path.to_string_lossy());

    // Pre-trust so the session doesn't stall on the "trust this folder?" prompt.
    otto_sessions::trust::ensure_trusted(&agent.provider, cwd);

    let captured_sid: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let attempts = Arc::new(std::sync::atomic::AtomicI64::new(0));

    let outcome = run_with_recovery(ctx.manager(), MAX_ATTEMPTS, &RETRY_BACKOFF, None, |_| {
        let captured = captured_sid.clone();
        let attempts = attempts.clone();
        let ws = ws.clone();
        let augmented = augmented.clone();
        let out_path = out_path.clone();
        async move {
            attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let one = OneSession {
                ws: &ws,
                owner,
                agent,
                run_id,
                cwd,
                prompt: &augmented,
                out_path: &out_path,
                plan,
                cancel,
            };
            run_one_session(ctx, one, &captured).await
        }
    })
    .await;
    // The watcher already read the report into `outcome`; the scratch file
    // would otherwise pile up one per run (for a personal agent, inside the
    // folder its next runs work in — where they could read stale reports).
    let _ = std::fs::remove_file(&out_path);

    let session_id = captured_sid
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let failure = outcome.errored().then(|| {
        outcome
            .reason
            .map(|r| r.as_str())
            .unwrap_or("unknown")
            .to_string()
    });
    Ok(AgentSessionOutcome {
        report: outcome.raw,
        session_id,
        attempts: attempts.load(std::sync::atomic::Ordering::Relaxed),
        failure,
    })
}

struct OneSession<'a> {
    ws: &'a otto_core::domain::Workspace,
    owner: &'a str,
    agent: &'a PersonalAgent,
    run_id: &'a str,
    cwd: &'a str,
    prompt: &'a str,
    out_path: &'a std::path::Path,
    plan: &'a RunPlan,
    cancel: &'a otto_core::cancel_signal::CancelSignal,
}

/// One attempt: create a fresh visible session of the agent's provider, inject
/// the prompt, and watch for the report file. Mirrors
/// `scheduled_tasks_engine::run_one_agent_session`.
async fn run_one_session<C: AssistantCtx>(
    ctx: &C,
    one: OneSession<'_>,
    captured_sid: &Arc<Mutex<Option<String>>>,
) -> RunOutcome {
    let OneSession {
        ws,
        owner,
        agent,
        run_id,
        cwd,
        prompt,
        out_path,
        plan,
        cancel,
    } = one;
    let _ = std::fs::remove_file(out_path);
    let mut meta = crate::personal_agent_threads::run_meta(agent, run_id, plan);
    if !agent.model.trim().is_empty() {
        meta["model"] = serde_json::json!(agent.model.trim());
    }
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(agent.provider.clone()),
        title: Some(format!("Agent: {}", agent.name)),
        cwd: Some(cwd.to_string()),
        connection_id: None,
        model: None,
        meta: Some(meta),
    };
    // Session creation must finish even if the run future is canceled. Its
    // completion publishes the exact session ID, then kills it when Stop won
    // before publication. No prompt can be submitted by this detached setup.
    let (ctx2, ws2, owner2, run2, stopped) = (
        ctx.clone(),
        ws.clone(),
        owner.to_string(),
        run_id.to_string(),
        cancel.clone(),
    );
    let creation = tokio::spawn(async move {
        let session = ctx2.manager().create(&ws2, &owner2, req, None).await?;
        if let Err(e) = pa_engine::repo(&ctx2)
            .set_run_session(&run2, &session.id)
            .await
        {
            let _ = ctx2.manager().kill_session(&session.id).await;
            return Err(e);
        }
        if stopped.is_cancelled() {
            ctx2.manager().kill_session(&session.id).await?;
            return Err(Error::Conflict(
                "run stopped during session creation".into(),
            ));
        }
        Ok(session)
    });
    let session = match creation
        .await
        .unwrap_or_else(|e| Err(Error::Internal(e.to_string())))
    {
        Ok(s) => s,
        Err(e) => {
            warn!(agent = %agent.id, "personal agent: create session ({}): {e}", agent.provider);
            return RunOutcome::failed(None, FailReason::CreateFailed);
        }
    };
    let sid = session.id.clone();
    *captured_sid.lock().unwrap_or_else(|e| e.into_inner()) = Some(sid.clone());

    // The one locked submit; the watch below catches a prompt that never landed.
    ctx.manager()
        .submit_prompt(&sid, prompt, SubmitOpts::default())
        .await;

    watch_for_result(
        ctx.manager(),
        &sid,
        &agent.provider,
        session.provider_session_id.as_deref(),
        cwd,
        out_path,
        RUN_NO_PROGRESS,
        WAITING_IDLE,
        STUCK_IDLE,
        Some(|t| !t.trim().is_empty()),
        |_st| async {},
    )
    .await
}
