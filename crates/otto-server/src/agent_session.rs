//! Run ONE agent turn against a managed Otto **session**, creating it on first
//! use and RESUMING it across turns. This is the session-backed replacement for
//! the throwaway `orchestrator.run_agent` PTY: the agent runs as a real,
//! visible-in-Agents, resumable session that auto-gets the workspace's MCP tools.
//!
//! Used by Discovery Chat (one session per chat thread) and Canvas "Ask AI" (one
//! session per scene). It mirrors `swarm_agent_run` but RESUMES the same session
//! across turns and detects the NEW turn via a transcript baseline — a resumed
//! transcript already holds the prior turns, so we must wait for the completed
//! turn count to GROW rather than accepting any `end_turn` (which would echo the
//! previous reply instantly). It also fails FAST on a claude API error (wrong
//! model / auth / rate-limit) with the real message instead of a "stuck" timeout.

use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, User, Workspace};
use otto_core::{Error, Id};
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
use crate::state::ServerCtx;

/// Absolute cap on one turn (cold claude spawn + a long reply). The first turn
/// pays the ~25-30s cold start; later turns are warm.
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
/// Treat the session as wedged once it produces no output for this long.
const STUCK_IDLE: Duration = Duration::from_secs(240);
/// Watch poll cadence.
const POLL: Duration = Duration::from_millis(1000);

/// Run one turn. Returns `(reply_text, session_id)`. Persist the returned
/// `session_id` so the next turn resumes the SAME session.
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    existing: Option<&Id>,
    title: &str,
    cwd: &str,
    provider: &str,
    meta: Value,
    prompt: &str,
) -> ApiResult<(String, Id)> {
    // 1. E2E short-circuit: the offline test daemon points CLAUDE_BIN at a
    //    nonexistent path, so a real session can't spawn. Return the deterministic
    //    canned reply (routed by an OTTO_TASK: sentinel in the prompt).
    if matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true")) {
        let reply = otto_orchestrator::e2e_stub::canned_reply(prompt);
        return Ok((reply, existing.cloned().unwrap_or_else(otto_core::new_id)));
    }

    // 2. Canonicalize cwd — claude symlink-resolves it for the transcript dir, so
    //    the spawn cwd and the JSONL path we poll MUST be the same resolved path.
    let cwd_canon = std::fs::canonicalize(cwd)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| cwd.to_string());

    // 3. Resolve the session: resume an existing one (ensure_live restarts it with
    //    --resume when suspended/dead, guarding supports_resume), else create.
    let (sid, psid) = match existing {
        Some(id) if ctx.manager.get(id).await.is_ok() => {
            ctx.manager.ensure_live(id).await.map_err(ApiError)?;
            let session = ctx.manager.get(id).await.map_err(ApiError)?;
            (id.clone(), session.provider_session_id.clone())
        }
        _ => {
            let req = CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some(provider.to_string()),
                title: Some(title.to_string()),
                cwd: Some(cwd_canon.clone()),
                connection_id: None,
                meta: Some(meta),
            };
            let session = ctx
                .manager
                .create(ws, &user.id, req, None)
                .await
                .map_err(ApiError)?;
            (session.id.clone(), session.provider_session_id.clone())
        }
    };

    // 4. Baseline the transcript so a resumed session's PRIOR reply isn't
    //    mistaken for this turn (claude appends to the same <psid>.jsonl on resume).
    let baseline = transcript_path(provider, &cwd_canon, psid.as_deref())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|c| otto_orchestrator::claude_pty::completed_turn_count(&c))
        .unwrap_or(0);

    // 5. Submit the prompt once the TUI has settled (bracketed paste keeps a
    //    multi-line prompt atomic), confirming dispatch with a re-`\r` retry.
    if wait_for_tui(&ctx.manager, &sid).await {
        let _ = ctx.manager.input(&sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = ctx.manager.live_handle(&sid).map(|h| h.last_output_at());
        let _ = ctx.manager.input(&sid, b"\r").await;
        if !dispatched(&ctx.manager, &sid, before).await {
            let _ = ctx.manager.input(&sid, b"\r").await;
        }
    }
    ctx.manager.record_user_message(&sid, prompt).await;

    // 6. Watch for the NEW completed turn (count > baseline). Fail fast on a
    //    claude API error / stuck / exit / timeout. Leave the session OPEN.
    let deadline = Instant::now() + TURN_TIMEOUT;
    loop {
        if let Some(path) = transcript_path(provider, &cwd_canon, psid.as_deref()) {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if let Some(err) = otto_orchestrator::claude_pty::transcript_api_error(&content) {
                    return Err(ApiError(Error::Upstream(format!("agent error: {err}"))));
                }
                if otto_orchestrator::claude_pty::completed_turn_count(&content) > baseline {
                    let text = otto_orchestrator::claude_pty::completed_turn_text(&content)
                        .unwrap_or_default();
                    return Ok((text, sid));
                }
            }
        }
        match ctx.manager.live_handle(&sid) {
            Some(h) => {
                if h.on_exit().borrow().is_some() {
                    return Err(ApiError(Error::Upstream(
                        "agent session exited before replying".into(),
                    )));
                }
                if h.last_output_at().elapsed() >= STUCK_IDLE {
                    return Err(ApiError(Error::Upstream(
                        "agent session stuck — no output".into(),
                    )));
                }
            }
            None => {
                return Err(ApiError(Error::Upstream("agent session vanished".into())));
            }
        }
        if Instant::now() >= deadline {
            return Err(ApiError(Error::Upstream("agent turn timed out".into())));
        }
        tokio::time::sleep(POLL).await;
    }
}

/// The claude JSONL transcript path for this session, or `None` for non-claude
/// providers (codex/agy don't write a JSONL transcript we can poll).
fn transcript_path(provider: &str, cwd: &str, psid: Option<&str>) -> Option<std::path::PathBuf> {
    match (provider, psid) {
        ("claude", Some(p)) => Some(otto_orchestrator::claude_pty::session_jsonl_path(cwd, p)),
        _ => None,
    }
}
