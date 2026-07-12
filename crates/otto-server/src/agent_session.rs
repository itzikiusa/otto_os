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

use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::api::CreateSessionReq;
use otto_core::domain::{SessionKind, User, Workspace};
use otto_core::{Error, Id};
use otto_sessions::SessionManager;
use serde_json::Value;

use crate::error::{ApiError, ApiResult};
use crate::review_session::{bracketed_paste, dispatched, wait_for_tui, PASTE_TO_ENTER};
use crate::state::ServerCtx;

/// Absolute cap on one turn (cold claude spawn + a long reply). Deliberately very
/// generous (10h): a workflow agent step (e.g. "write tests", a long refactor) can
/// legitimately run for hours, and the operator stops a run manually rather than
/// having it killed out from under them. A genuine claude API error still fails
/// FAST (see below) — this cap only bounds *legitimate* long work.
const TURN_TIMEOUT: Duration = Duration::from_secs(10 * 60 * 60);
/// Default no-output idle backstop (10h) — the session's max lifespan floor. This
/// is the value non-workflow callers pass for `stuck_after`, so a step that's
/// quietly working — compiling, running a long test suite — is never mistaken for
/// a hung session. Workflow steps pass a much shorter `stuck_after` (see
/// `run_session_turn`) as an EARLIER, additional trip; this 10h backstop and
/// `TURN_TIMEOUT` are never reduced.
pub const STUCK_IDLE: Duration = Duration::from_secs(10 * 60 * 60);
/// Watch poll cadence.
const POLL: Duration = Duration::from_millis(1000);
/// How long we keep trying to land the prompt on the CLI's real input box before
/// giving up with a visible error. A cold claude spawn + a promo/onboarding screen
/// can delay the input box; we re-submit within this window rather than silently
/// polling a no-op turn for 10h.
const SUBMIT_CONFIRM: Duration = Duration::from_secs(45);
/// Re-send the prompt this often, within `SUBMIT_CONFIRM`, until claude records it
/// as a user turn (a banner can swallow the first paste/Enter).
const RESUBMIT_EVERY: Duration = Duration::from_secs(8);
/// Cap on paste attempts (first + re-sends) so a genuinely broken session can't
/// spin forever inside the confirm window.
const MAX_SUBMIT_ATTEMPTS: u32 = 4;
/// How many times to nudge an agent that echoed the CLI's injected reminder
/// (see [`is_injected_reminder_echo`]) instead of doing the work, before giving
/// up and returning whatever it produced.
const MAX_REMINDER_NUDGES: u32 = 2;
/// Sent when an agent no-ops by parroting the injected skill/agentic-loop
/// reminder — tells it to actually perform the task and write its handoff.
const REMINDER_NUDGE: &str = "You have not done the task — you only restated the instructions. \
Read the referenced context files, DO the actual work now, and write your handoff summary to the \
required file. Do not repeat or restate these instructions.";

/// Extra turn-completion channels for providers WITHOUT a pollable transcript
/// (codex/agy/grok/custom). Turn completion is transcript-based and only
/// claude writes one — without these, a non-claude agent that finished its
/// work sits "running" until the 10h idle backstop (the live "codex/grok done
/// but shown RUNNING" bug).
#[derive(Default)]
pub struct TurnOpts {
    /// Completion marker: the turn is complete the moment this file exists
    /// with non-empty content — the (trimmed) content IS the turn text. The
    /// caller appends a "FINALLY write your one-line summary to <path>"
    /// instruction to the prompt. Checked for every provider (a compliant
    /// claude just completes via whichever channel fires first).
    pub done_file: Option<std::path::PathBuf>,
    /// Quiet fallback for non-transcript providers only: once the prompt was
    /// dispatched, this much PTY silence counts as turn-complete (empty turn
    /// text). Working TUIs repaint (spinners/tool output), so silence this
    /// long means the agent is sitting at its input box. Ignored for claude.
    pub quiet_done: Option<Duration>,
}

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
    stuck_after: Duration,
    on_ready: impl FnOnce(&Id),
) -> ApiResult<(String, Id)> {
    run_session_turn_with(
        ctx,
        ws,
        user,
        existing,
        title,
        cwd,
        provider,
        meta,
        prompt,
        stuck_after,
        TurnOpts::default(),
        on_ready,
    )
    .await
}

/// [`run_session_turn`] with extra completion channels (see [`TurnOpts`]).
#[allow(clippy::too_many_arguments)]
pub async fn run_session_turn_with(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    existing: Option<&Id>,
    title: &str,
    cwd: &str,
    provider: &str,
    meta: Value,
    prompt: &str,
    // No-output idle trip. Workflow steps pass a short value (e.g. 3 min) as an
    // early "stuck" signal; interactive callers pass STUCK_IDLE (10h) to keep the
    // long backstop. Never lengthens past TURN_TIMEOUT.
    stuck_after: Duration,
    opts: TurnOpts,
    on_ready: impl FnOnce(&Id),
) -> ApiResult<(String, Id)> {
    // 1. E2E short-circuit: the offline test daemon points CLAUDE_BIN at a
    //    nonexistent path, so a real session can't spawn. Return the deterministic
    //    canned reply (routed by an OTTO_TASK: sentinel in the prompt).
    if matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true")) {
        let reply = otto_orchestrator::e2e_stub::canned_reply(prompt);
        let sid = existing.cloned().unwrap_or_else(otto_core::new_id);
        on_ready(&sid);
        return Ok((reply, sid));
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

    // Session exists now — let the caller surface its id (e.g. attach the live
    // shell in the Canvas panel) BEFORE the long turn runs.
    on_ready(&sid);

    // 4. Submit the prompt AND confirm it actually landed on the CLI's input box.
    //    A startup/promo/onboarding banner is quiescent output, so wait_for_tui /
    //    dispatched can report "ready"/"sent" while the real input box isn't up
    //    yet — swallowing the paste and leaving the step a no-op. Only claude
    //    writes a pollable transcript, so only there can we verify; other providers
    //    keep the best-effort single submit.
    let can_confirm = transcript_path(provider, &cwd_canon, psid.as_deref()).is_some();
    let needle = confirm_needle(prompt);

    submit_once(&ctx.manager, &sid, prompt).await;
    ctx.manager.record_user_message(&sid, prompt).await;

    // Baseline of completed assistant turns. For a resumed/non-claude session we
    // seed it from the pre-submit count; when we can confirm (claude), we RE-baseline
    // at the exact instant our prompt is seen as the latest user turn (below), so a
    // stray reply to an empty submit is already counted and can't be mistaken for
    // THIS turn's result.
    let mut baseline = transcript_path(provider, &cwd_canon, psid.as_deref())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|c| otto_orchestrator::claude_pty::completed_turn_count(&c))
        .unwrap_or(0);

    if can_confirm {
        let confirm_deadline = Instant::now() + SUBMIT_CONFIRM;
        let mut next_resubmit = Instant::now() + RESUBMIT_EVERY;
        let mut attempts: u32 = 1;
        let mut entered = false;
        loop {
            if let Some(path) = transcript_path(provider, &cwd_canon, psid.as_deref()) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Some(err) = otto_orchestrator::claude_pty::transcript_api_error(&content) {
                        return Err(ApiError(Error::Upstream(format!("agent error: {err}"))));
                    }
                    if prompt_entered(&content, &needle) {
                        baseline = otto_orchestrator::claude_pty::completed_turn_count(&content);
                        entered = true;
                        break;
                    }
                }
            }
            match ctx.manager.live_handle(&sid) {
                Some(h) if h.on_exit().borrow().is_some() => {
                    return Err(ApiError(Error::Upstream(
                        "agent session exited before accepting the prompt".into(),
                    )));
                }
                None => return Err(ApiError(Error::Upstream("agent session vanished".into()))),
                _ => {}
            }
            if Instant::now() >= confirm_deadline {
                break;
            }
            if Instant::now() >= next_resubmit && attempts < MAX_SUBMIT_ATTEMPTS {
                submit_once(&ctx.manager, &sid, prompt).await;
                attempts += 1;
                next_resubmit = Instant::now() + RESUBMIT_EVERY;
            }
            tokio::time::sleep(POLL).await;
        }
        if !entered {
            // The prompt never became a user turn — the step is a no-op. Fail
            // LOUD (and retryable) instead of polling a dead turn for 10h or
            // mistaking a stray greeting for completion.
            return Err(ApiError(Error::Upstream(
                "agent never accepted the prompt (stuck on a startup screen?)".into(),
            )));
        }
    }

    // 5. Watch for the NEW completed turn (count > baseline). Fail fast on a
    //    claude API error / no progress for `stuck_after` / exit / timeout. Leave
    //    the session OPEN.
    let deadline = Instant::now() + TURN_TIMEOUT;
    let mut reminder_nudges: u32 = 0;
    loop {
        // Marker channel (provider-agnostic): the agent wrote its done-file —
        // the turn is complete regardless of transcript availability.
        if let Some(df) = &opts.done_file {
            if let Ok(s) = tokio::fs::read_to_string(df).await {
                let t = s.trim();
                if !t.is_empty() {
                    return Ok((t.to_string(), sid));
                }
            }
        }
        if let Some(path) = transcript_path(provider, &cwd_canon, psid.as_deref()) {
            if let Ok(content) = tokio::fs::read_to_string(&path).await {
                if let Some(err) = otto_orchestrator::claude_pty::transcript_api_error(&content) {
                    return Err(ApiError(Error::Upstream(format!("agent error: {err}"))));
                }
                if otto_orchestrator::claude_pty::completed_turn_count(&content) > baseline {
                    let text = otto_orchestrator::claude_pty::completed_turn_text(&content)
                        .unwrap_or_default();
                    // Guard against a DEGENERATE turn: the agent sometimes just
                    // parrots the CLI's injected skill/agentic-loop reminder ("Now
                    // write a response to the user. Keep going… Remember to use
                    // skills…") and ends its turn without doing any work. That is
                    // never a real reply — nudge it to actually act (bounded), rather
                    // than accept the echo as the step's output and silently no-op.
                    if is_injected_reminder_echo(&text) && reminder_nudges < MAX_REMINDER_NUDGES {
                        reminder_nudges += 1;
                        // Advance the baseline past this echo so we wait for the
                        // NEXT (hopefully real) turn instead of re-tripping on it.
                        baseline = otto_orchestrator::claude_pty::completed_turn_count(&content);
                        submit_once(&ctx.manager, &sid, REMINDER_NUDGE).await;
                        tokio::time::sleep(POLL).await;
                        continue;
                    }
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
                // Quiet fallback (non-transcript providers only): a TUI that's
                // WORKING keeps painting; this much silence means it's idle at
                // its input box — the turn is over even if the agent never
                // wrote the done-file. Claude keeps transcript-only detection.
                if !can_confirm {
                    if let Some(q) = opts.quiet_done {
                        if h.last_output_at().elapsed() >= q {
                            return Ok((String::new(), sid));
                        }
                    }
                }
                if h.last_output_at().elapsed() >= stuck_after {
                    return Err(ApiError(Error::Upstream(format!(
                        "step made no progress for {}m",
                        stuck_after.as_secs() / 60
                    ))));
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

/// One paste + Enter into the session, with a single re-`\r` when the first didn't
/// visibly dispatch. Bracketed paste keeps a multi-line prompt atomic.
async fn submit_once(manager: &Arc<SessionManager>, sid: &Id, prompt: &str) {
    if wait_for_tui(manager, sid).await {
        let _ = manager.input(sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        let before = manager.live_handle(sid).map(|h| h.last_output_at());
        let _ = manager.input(sid, b"\r").await;
        if !dispatched(manager, sid, before).await {
            let _ = manager.input(sid, b"\r").await;
        }
    }
}

/// Collapse every run of whitespace to a single space, so a paste reflow / newline
/// differences don't defeat a substring match.
fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A normalized, distinctive slice of `prompt` to look for in claude's recorded
/// user turn. Sampled from ~30% in to skip any shared leading boilerplate (the
/// workflow context preamble), so it's specific to THIS prompt.
fn confirm_needle(prompt: &str) -> String {
    let norm = normalize_ws(prompt);
    let chars: Vec<char> = norm.chars().collect();
    if chars.len() <= 24 {
        return norm;
    }
    let start = chars.len() * 3 / 10;
    let end = (start + 60).min(chars.len());
    chars[start..end].iter().collect()
}

/// Whether claude's latest USER turn in `transcript` contains our prompt slice —
/// i.e. the prompt was actually entered, not lost to a startup/promo banner.
fn prompt_entered(transcript: &str, needle: &str) -> bool {
    match otto_orchestrator::claude_pty::last_user_text(transcript) {
        Some(t) => normalize_ws(&t).contains(needle),
        None => false,
    }
}

/// True when `text` is a DEGENERATE turn that merely echoes the CLI's injected
/// skill/agentic-loop reminder ("…write a response to the user. Keep going until
/// the task is fully resolved… Remember to use skills…") instead of doing the
/// work. A real agent reply never instructs ITSELF to "write a response to the
/// user", so ≥2 of these signatures in a short reply is a reliable tell. The
/// length cap keeps a genuine (long) reply that merely mentions "skills" from
/// tripping it.
fn is_injected_reminder_echo(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.len() > 800 {
        return false;
    }
    const SIGS: [&str; 4] = [
        "write a response to the user",
        "Keep going until the task is fully resolved",
        "Remember to use skills",
        "invoke it with the Skill tool",
    ];
    SIGS.iter().filter(|s| t.contains(*s)).count() >= 2
}

/// The claude JSONL transcript path for this session, or `None` for non-claude
/// providers (codex/agy don't write a JSONL transcript we can poll).
fn transcript_path(provider: &str, cwd: &str, psid: Option<&str>) -> Option<std::path::PathBuf> {
    match (provider, psid) {
        ("claude", Some(p)) => Some(otto_orchestrator::claude_pty::session_jsonl_path(cwd, p)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_ws_collapses_all_whitespace() {
        assert_eq!(normalize_ws("a\n\n  b\tc  "), "a b c");
    }

    #[test]
    fn confirm_needle_samples_a_distinctive_slice() {
        assert_eq!(confirm_needle("do it"), "do it"); // short prompts use the whole thing
        let long = "[workflow context] read the files. \
             Now write the leaderboard tests for the tournament feature in module X.";
        let needle = confirm_needle(long);
        assert!(!needle.is_empty());
        // The needle is a normalized slice OF the prompt…
        assert!(normalize_ws(long).contains(&needle), "needle: {needle}");
        // …sampled past the shared preamble, so it carries step-specific text.
        assert!(!needle.starts_with("[workflow context]"));
    }

    #[test]
    fn prompt_entered_matches_only_our_user_turn() {
        let prompt =
            "[workflow context] read files. Implement the deposit-bonus template creation flow now.";
        let needle = confirm_needle(prompt);
        // A transcript whose latest user turn IS our prompt → entered.
        let mine = format!(
            r#"{{"message":{{"role":"user","content":{}}}}}"#,
            serde_json::to_string(prompt).unwrap()
        );
        assert!(prompt_entered(&mine, &needle));
        // A DIFFERENT user turn (a prior resumed prompt / a stray empty submit) → not entered.
        let other = r#"{"message":{"role":"user","content":"totally unrelated earlier prompt"}}"#;
        assert!(!prompt_entered(other, &needle));
        // Empty transcript → not entered (so we keep re-submitting, then fail loud).
        assert!(!prompt_entered("", &needle));
    }

    #[test]
    fn detects_injected_reminder_echo() {
        // The exact degenerate echo we saw in a workflow transcript.
        let echo = "When a skill applies to your task, invoke it with the Skill tool before you begin. \
            When multiple skills apply, invoke them together.\n\nNow write a response to the user. \
            Keep going until the task is fully resolved. Only end your turn when you're confident \
            everything is working.\n\nRemember to use skills when they are relevant to the given task!";
        assert!(is_injected_reminder_echo(echo));
        // A real reply that merely mentions skills once is NOT an echo.
        assert!(!is_injected_reminder_echo(
            "I rewrote the legacy test suite onto the new framework; the relevant skills helped."
        ));
        // Empty / whitespace is not an echo.
        assert!(!is_injected_reminder_echo("   "));
    }
}
