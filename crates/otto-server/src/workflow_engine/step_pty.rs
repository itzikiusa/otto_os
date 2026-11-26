//! The PTY step runner (engine.md §4.3): one agent turn as a visible,
//! openable terminal session through the shared `run_session_turn` flow and
//! the turn oracle. Frozen — moved here unchanged when steps moved to runtime
//! threads; used when the run latched `wf_step_runner = "pty"` and for
//! providers the runtime has no driver for. Removed one release later (Q4).

use super::*;
use otto_agent_run::phase_feed::{e2e_subagent_sequence, e2e_subagents, PhaseFeed};

/// Default no-REAL-progress trip for agent-backed workflow steps (overridable
/// via the `wf_step_stall_secs` setting; 0 disables). Progress means the
/// provider's transcript/rollout growing or child processes burning CPU — not
/// PTY repaints, which a stuck TUI spinner keeps emitting forever. A tripped
/// step is retried per `resolve_retry`. Separate from — and never longer
/// than — the 10h max session lifespan backstop. See design R5.
const WF_STEP_STALL_DEFAULT: Duration = Duration::from_secs(5 * 60);

/// Run one agent turn as a visible, openable session (reusing the shared
/// `run_session_turn` flow), reporting the session id over `session_tx` the
/// moment the session exists so the run view can open it live. Returns
/// `(reply, session_id)`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn run(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    node: &WorkflowNode,
    wf_name: &str,
    run_id: &Id,
    provider: &str,
    // Optional model alias (e.g. "opus"/"haiku"). Empty/None = provider default.
    // Forwarded via `meta.model` so `SessionManager::model_args` adds `--model`.
    model: Option<&str>,
    prompt: &str,
    cwd: &str,
    // The step's handoff `.md` (named by the preamble) as a completion MARKER:
    // for providers without a pollable transcript (codex/agy) it is the ONLY
    // positive "the step finished" signal — without it they sit at their input
    // box until the stall trip errors the step. `None` for steps whose reply
    // must come from the transcript (canvas/goals/PR-draft agents).
    done_file: Option<std::path::PathBuf>,
    session_tx: &tokio::sync::mpsc::UnboundedSender<AgentAssociation>,
    // The node's LIVE log channel: the oracle's phase lines stream here as the
    // turn runs and are kept in the step's logs afterwards (R1.4/R5.5).
    log_tx: &tokio::sync::mpsc::UnboundedSender<String>,
    // Live sub-agent snapshot for the run view's chip + nested rows.
    activity_tx: &tokio::sync::mpsc::UnboundedSender<NodeActivity>,
) -> Result<(String, Id)> {
    // Title carries the workflow name, the step, and a short run id so two
    // concurrent runs of the same workflow are distinguishable in the Agents
    // list (they'd otherwise both read "Workflow: Write tests").
    let label = if node.name.is_empty() {
        node.kind.as_str()
    } else {
        node.name.as_str()
    };
    let title = format!("WF {wf_name} · {label} · run {}", short_id(run_id));
    // Stamp the FULL run id (+ workflow name) into meta so every workflow-spawned
    // session is provably attributable to its specific run — the UI/global Agents
    // list can filter + group sessions by exact run.
    let mut meta = json!({
        "source": "workflow",
        "run_id": run_id,
        "wf_name": wf_name,
        "node_id": node.id,
        "node_kind": node.kind,
        "cwd": cwd,
    });
    // Forward the model alias so the spawn adds `--model` (providers without the
    // flag ignore it). Empty string = provider default → omitted.
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        meta["model"] = json!(m);
    }
    // R5.5: the step's phases are visible from the first line. The engine logs
    // the boot line itself (the oracle's `Phase::Booting` is therefore never
    // re-logged by the forwarder below).
    let _ = log_tx.send(format!("⏳ starting {provider} session"));
    // E2E: the offline daemon never spawns a CLI, so a spec that needs a step
    // with sub-agents drives a synthetic sequence off a prompt sentinel.
    if otto_e2e() {
        if let Some((n, hold)) = e2e_subagents(prompt) {
            e2e_subagent_sequence(n, hold, log_tx, activity_tx).await;
        }
    }
    let tx = session_tx.clone();
    // The session id the turn creates — the phase forwarder needs it to ask the
    // oracle for this step's sub-agent rows (it only exists once `on_ready`
    // fires, a few seconds in).
    let live_sid: std::sync::Arc<std::sync::Mutex<Option<Id>>> = Default::default();
    let sid_slot = live_sid.clone();
    // R1.3: sub-agents ARE allowed; the appended block is a completion protocol
    // (finish every task you launched, handoff LAST) and the turn oracle holds
    // the step open until the parent's pending set is empty.
    let guarded = format!("{prompt}{WF_STEP_RULES}");
    // R5: EVERY agent-backed step gets the no-real-progress trip (retryable via
    // resolve_retry). Heavy kinds are safe under a short threshold because the
    // clock only counts genuine stalls — transcript/rollout growth and child
    // CPU reset it (see agent_session), so a long review or quiet test run
    // never false-trips while a codex spinner can no longer mask a hang.
    let stall = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("wf_step_stall_secs")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_u64())
        .map_or(WF_STEP_STALL_DEFAULT, Duration::from_secs);
    let stuck_after = if stall.is_zero() {
        crate::agent_session::STUCK_IDLE
    } else {
        stall
    };
    // A leftover handoff from a PRIOR attempt/run of this step would complete
    // the new turn instantly with stale content — clear it before submitting.
    if let Some(df) = &done_file {
        let _ = std::fs::remove_file(df);
    }
    // Steps whose reply must come from the transcript (goals eval, PR draft,
    // canvas) never name a handoff file — the oracle completes them on the turn
    // ending with nothing pending, and the "handoff missing" ⚠ line would be a
    // lie for them.
    let expects_handoff = done_file.is_some();
    // R1: the turn ORACLE decides completion for workflow steps — a claude
    // `end_turn` while sub-agents are still working is not "done". Phases stream
    // back on `phase_tx`; the accepted rule rides `outcome_tx`.
    let (phase_tx, mut phase_rx) = tokio::sync::mpsc::unbounded_channel::<Phase>();
    let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel::<CompleteVia>();
    let turn = crate::agent_session::run_session_turn_with(
        ctx,
        ws,
        user,
        None,
        &title,
        cwd,
        provider,
        meta,
        &guarded,
        stuck_after,
        crate::agent_session::TurnOpts {
            done_file,
            done_file_validator: None,
            transcript_fallback: false,
            cancel: None,
            // Silence is not evidence that delegated work has completed.
            quiet_done: None,
            // The retry spawns a fresh session; don't leave the stuck one
            // alive (its spinner defeats the idle-suspend sweep).
            kill_on_stall: true,
            oracle: true,
            phase_tx: Some(phase_tx),
            outcome_tx: Some(outcome_tx),
        },
        move |id| {
            let _ = tx.send(AgentAssociation::Session(id.to_string()));
            if let Ok(mut slot) = sid_slot.lock() {
                *slot = Some(id.clone());
            }
        },
    );
    tokio::pin!(turn);
    // Phase → log/activity forwarder. Runs BESIDE the turn (not spawned) so it
    // can read the live session without cloning the whole context.
    let mut feed = PhaseFeed::new(provider);
    let result = loop {
        tokio::select! {
            biased;
            Some(p) = phase_rx.recv() => {
                let sid = live_sid.lock().ok().and_then(|g| g.clone());
                feed.on_phase(&ctx.manager, sid.as_ref(), p, log_tx, activity_tx).await;
            }
            r = &mut turn => break r,
        }
    };
    while let Ok(p) = phase_rx.try_recv() {
        let sid = live_sid.lock().ok().and_then(|g| g.clone());
        feed.on_phase(&ctx.manager, sid.as_ref(), p, log_tx, activity_tx)
            .await;
    }
    let out = result.map_err(|e| e.0)?;
    // How the oracle accepted the turn — `Err` means the sender was dropped
    // without a send (E2E short-circuit / legacy path): nothing to report.
    if let Ok(via) = outcome_rx.await {
        if expects_handoff || via != CompleteVia::IdleTurnNoHandoff {
            let _ = log_tx.send(feed.completion_line(via));
        }
    }
    Ok(out)
}
