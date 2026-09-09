//! Shared agent-session run mechanics used by BOTH the product analysis runner
//! (`product_run`) and the PR-review runner (`review_session`).
//!
//! Each module still owns its own *spawn + prompt-injection* (product has extra
//! codex re-paste robustness review doesn't need) and its own *persistence*
//! (product writes a DB agent row; review mutates a shared `Vec` of states). What
//! they share — and what used to be copy-pasted — lives here:
//!
//! - [`watch_for_result`]: the watch loop. Polls the agent's result file (and the
//!   claude transcript as a fallback), and classifies exit / **stuck** (idle too
//!   long) / timeout. Emits `Waiting`/`Resumed` transitions via an async hook so
//!   each module can persist them however it likes.
//! - [`run_with_recovery`]: the bounded retry loop. Runs an attempt closure up to
//!   `max_attempts`, killing the prior (stuck/failed) session and backing off
//!   between tries, honoring an optional cancel flag (manual Stop).
//!
//! Approval prompts are handled globally by `otto_sessions::PromptGuard` (one
//! scanner on the shared `SessionManager`), so neither this module nor the
//! callers deal with trust/continue prompts.

use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::Id;
use otto_sessions::SessionManager;
use tracing::warn;

/// Poll cadence while waiting for an agent's result file.
const POLL: Duration = Duration::from_millis(1000);

/// Why an agent run failed (`None` reason ⇒ success). Stable string forms feed
/// notifications and per-agent error notes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// No output and no result for the stuck window — wedged.
    Stuck,
    /// Overall grace timeout elapsed AND the agent had gone quiet (idle past
    /// the waiting window). An agent that is still visibly working when the
    /// grace elapses is NOT timed out — see [`deadline_fires`].
    Timeout,
    /// PTY exited before writing a result.
    Exited,
    /// Session vanished from the manager.
    SessionGone,
    /// Could not create the session at all.
    CreateFailed,
    /// Cancelled by a manual Stop (must NOT be auto-retried).
    Stopped,
    /// Gave up retrying because the caller's `give_up` predicate said the work
    /// is already covered (e.g. a sibling reviewer on another provider finished
    /// the same lens). Not an error in the agent itself; never auto-retried.
    Superseded,
}

impl FailReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailReason::Stuck => "stuck",
            FailReason::Timeout => "timeout",
            FailReason::Exited => "exited",
            FailReason::SessionGone => "session-gone",
            FailReason::CreateFailed => "create-failed",
            FailReason::Stopped => "stopped",
            FailReason::Superseded => "superseded",
        }
    }
}

/// Outcome of one agent run (one attempt, or the final result after recovery).
pub struct RunOutcome {
    /// Result text the agent produced (its out-file, or an accepted claude turn).
    pub raw: Option<String>,
    /// The session id (so the agent stays openable; killed between retries).
    pub session_id: Option<Id>,
    /// `None` ⇒ success; `Some(_)` ⇒ failed with this reason.
    pub reason: Option<FailReason>,
}

impl RunOutcome {
    pub fn ok(raw: String, sid: Id) -> Self {
        Self { raw: Some(raw), session_id: Some(sid), reason: None }
    }
    pub fn failed(sid: Option<Id>, reason: FailReason) -> Self {
        Self { raw: None, session_id: sid, reason: Some(reason) }
    }
    pub fn errored(&self) -> bool {
        self.reason.is_some()
    }
}

/// Idle-state transition emitted by [`watch_for_result`] so the caller can
/// reflect it (e.g. flip the agent row to "waiting" / back to "running").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus {
    /// Quiet past `waiting_idle` with no result yet — possibly blocked on input.
    Waiting,
    /// Output resumed after a `Waiting`.
    Resumed,
}

/// Watch a freshly-spawned, already-prompted session for its result.
///
/// Returns success when the out-file appears (or — for claude — a transcript
/// turn the caller deems acceptable via `transcript_ok`). Classifies exit /
/// stuck (`stuck_idle`) / timeout otherwise. `on_status` is an async hook invoked
/// on each Waiting/Resumed transition (`waiting_idle` < `stuck_idle`).
#[allow(clippy::too_many_arguments)]
pub async fn watch_for_result<F, Fut>(
    manager: &Arc<SessionManager>,
    sid: &Id,
    provider: &str,
    provider_session_id: Option<&str>,
    cwd: &str,
    out_path: &Path,
    timeout: Duration,
    waiting_idle: Duration,
    stuck_idle: Duration,
    transcript_ok: fn(&str) -> bool,
    mut on_status: F,
) -> RunOutcome
where
    F: FnMut(WatchStatus) -> Fut,
    Fut: Future<Output = ()>,
{
    let deadline = Instant::now() + timeout;
    let mut flagged_waiting = false;
    let mut flagged_over_budget = false;
    loop {
        if let Ok(text) = std::fs::read_to_string(out_path) {
            let _ = std::fs::remove_file(out_path);
            return RunOutcome::ok(text, sid.clone());
        }

        // claude writes a JSONL transcript; codex/agy don't, so the out-file is
        // their only signal. The caller decides what counts as a complete turn.
        if provider == "claude" {
            if let Some(psid) = provider_session_id {
                let jsonl = otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid);
                if let Ok(raw) = std::fs::read_to_string(&jsonl) {
                    if let Some(turn) = otto_orchestrator::claude_pty::completed_turn_text(&raw) {
                        if transcript_ok(&turn) {
                            return RunOutcome::ok(turn, sid.clone());
                        }
                    }
                }
            }
        }

        match manager.live_handle(sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    return RunOutcome::failed(Some(sid.clone()), FailReason::Exited);
                }
                let idle = handle.last_output_at().elapsed();
                // Fail fast once truly silent for stuck_idle so recovery can retry.
                if idle >= stuck_idle {
                    warn!("agent_run: session ({provider}) stuck — no output for {}s", stuck_idle.as_secs());
                    return RunOutcome::failed(Some(sid.clone()), FailReason::Stuck);
                }
                if idle >= waiting_idle && !flagged_waiting {
                    flagged_waiting = true;
                    on_status(WatchStatus::Waiting).await;
                } else if idle < waiting_idle && flagged_waiting {
                    flagged_waiting = false;
                    on_status(WatchStatus::Resumed).await;
                }
            }
            None => return RunOutcome::failed(Some(sid.clone()), FailReason::SessionGone),
        }

        if Instant::now() >= deadline {
            // The grace period is a budget for the WORK, not a hard kill switch:
            // a reviewer 40 minutes into a 170-file diff (12 siblings sharing one
            // CPU) is still working, and killing it to start over from zero
            // only guarantees the next attempt runs out of budget too — which is
            // exactly the "kills and respawns forever" loop observed. So the
            // deadline only fires once the agent has ALSO gone quiet for the
            // waiting window; a wedged one is caught by `stuck_idle` either way.
            let idle = manager
                .live_handle(sid)
                .map(|h| h.last_output_at().elapsed())
                .unwrap_or(Duration::MAX);
            if deadline_fires(idle, waiting_idle) {
                warn!("agent_run: session ({provider}) timed out (idle {}s past grace)", idle.as_secs());
                return RunOutcome::failed(Some(sid.clone()), FailReason::Timeout);
            }
            if !flagged_over_budget {
                flagged_over_budget = true;
                warn!(
                    "agent_run: session ({provider}) is past its {}s grace but still active — letting it finish",
                    timeout.as_secs()
                );
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Whether an elapsed grace deadline should fail the run: only when the agent
/// has been silent for at least the waiting window (i.e. it is not visibly
/// working). Pure so the policy is unit-testable without a PTY.
pub fn deadline_fires(idle: Duration, waiting_idle: Duration) -> bool {
    idle >= waiting_idle
}

/// Run `attempt` up to `max_attempts` times with auto-recovery: between tries it
/// kills the prior (stuck/failed) session and waits `backoff[min(i, last)]`. An
/// optional `cancel` flag (manual Stop) short-circuits with `Stopped` and is NOT
/// retried. Returns the first success, or the last failure.
pub async fn run_with_recovery<F, Fut>(
    manager: &Arc<SessionManager>,
    max_attempts: u32,
    backoff: &[Duration],
    cancel: Option<&Arc<AtomicBool>>,
    attempt: F,
) -> RunOutcome
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = RunOutcome>,
{
    run_with_recovery_until(manager, max_attempts, backoff, cancel, || async { false }, attempt).await
}

/// [`run_with_recovery`] with a `give_up` predicate consulted before every
/// RETRY (never before the first attempt): when it returns true the loop stops
/// with [`FailReason::Superseded`] instead of spawning another session. Lets a
/// caller skip retries whose result nobody needs any more — a review lens
/// already covered by a sibling provider, for instance — while the failed
/// attempt's session is still killed like any other retry would.
pub async fn run_with_recovery_until<F, Fut, G, GFut>(
    manager: &Arc<SessionManager>,
    max_attempts: u32,
    backoff: &[Duration],
    cancel: Option<&Arc<AtomicBool>>,
    give_up: G,
    mut attempt: F,
) -> RunOutcome
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = RunOutcome>,
    G: Fn() -> GFut,
    GFut: Future<Output = bool>,
{
    let cancelled = |c: Option<&Arc<AtomicBool>>| c.is_some_and(|f| f.load(Ordering::Relaxed));

    let mut last = RunOutcome::failed(None, FailReason::Exited);
    for i in 0..max_attempts {
        if cancelled(cancel) {
            last.reason = Some(FailReason::Stopped);
            return last;
        }
        if i > 0 {
            if give_up().await {
                if let Some(sid) = last.session_id.clone() {
                    let _ = manager.kill_session(&sid).await;
                }
                warn!(
                    "agent_run: not retrying after attempt {i}/{max_attempts} (prev: {}) — superseded",
                    last.reason.map(|r| r.as_str()).unwrap_or("?")
                );
                last.reason = Some(FailReason::Superseded);
                return last;
            }
            if let Some(sid) = last.session_id.clone() {
                let _ = manager.kill_session(&sid).await;
            }
            let idx = ((i - 1) as usize).min(backoff.len().saturating_sub(1));
            if let Some(d) = backoff.get(idx) {
                tokio::time::sleep(*d).await;
            }
            if cancelled(cancel) {
                last.reason = Some(FailReason::Stopped);
                return last;
            }
            warn!("agent_run: retry attempt {}/{} (prev: {})", i + 1, max_attempts, last.reason.map(|r| r.as_str()).unwrap_or("?"));
        }

        let res = attempt(i).await;
        if !res.errored() {
            return res;
        }
        last = res;
    }
    // A Stop that lands during the FINAL attempt has no next iteration to
    // catch it — without this, the kill surfaces as SessionGone/Exited and the
    // caller persists the wrong terminal note.
    if cancelled(cancel) {
        last.reason = Some(FailReason::Stopped);
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_only_fires_on_a_quiet_agent() {
        let waiting = Duration::from_secs(120);
        // Still producing output past its grace → keep waiting.
        assert!(!deadline_fires(Duration::from_secs(3), waiting));
        assert!(!deadline_fires(Duration::from_secs(119), waiting));
        // Quiet for the whole waiting window → the grace really elapsed.
        assert!(deadline_fires(Duration::from_secs(120), waiting));
        // A vanished PTY handle reports MAX idle → fires.
        assert!(deadline_fires(Duration::MAX, waiting));
    }

    #[test]
    fn superseded_has_a_stable_string_form() {
        assert_eq!(FailReason::Superseded.as_str(), "superseded");
    }
}
