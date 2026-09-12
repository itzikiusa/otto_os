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
//! - [`watch_for_result_guarded`]: the same loop with the R1 pending-task guard
//!   armed — for agents that delegate to sub-agents (the orchestrator reviewer),
//!   where the out-file appearing does NOT mean the work is finished.
//! - [`run_with_recovery`]: the bounded retry loop. Runs an attempt closure up to
//!   `max_attempts`, killing the prior (stuck/failed) session and backing off
//!   between tries, honoring an optional cancel flag (manual Stop).
//!
//! Approval prompts are handled globally by `otto_sessions::PromptGuard` (one
//! scanner on the shared `SessionManager`), so neither this module nor the
//! callers deal with trust/continue prompts.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use otto_core::Id;
use otto_sessions::SessionManager;
use tracing::warn;

use crate::turn_oracle::{self, ClaudeScan};

/// Poll cadence while waiting for an agent's result file.
const POLL: Duration = Duration::from_millis(1000);
/// How long a written-but-held out-file may be held before it is adopted
/// anyway. Bounded like every other R1 hold: a sub-agent that never reports
/// back must not park the reviewer forever.
pub const HOLD_LINGER_CAP: Duration = Duration::from_secs(15 * 60);
/// Floor between two `on_note` calls (the orchestrator row's progress note is
/// persisted, so it must not write once a second).
const NOTE_MIN_INTERVAL: Duration = Duration::from_secs(5);

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
        Self {
            raw: Some(raw),
            session_id: Some(sid),
            reason: None,
        }
    }
    pub fn failed(sid: Option<Id>, reason: FailReason) -> Self {
        Self {
            raw: None,
            session_id: sid,
            reason: Some(reason),
        }
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

/// Extra R1 awareness for [`watch_for_result_guarded`]. `Default` (every field
/// off) is the LEGACY behaviour byte-for-byte — [`watch_for_result`] passes it.
#[derive(Debug, Clone, Default)]
pub struct WatchGuard {
    /// Hold the out-file (and any accepted transcript turn) while the claude
    /// parent has launched/resumed tasks that have not reported back.
    pub pending_aware: bool,
    /// `(lens_slug, per-lens findings path)` — for the progress note only.
    pub lens_files: Vec<(String, PathBuf)>,
}

/// What the watch loop does with an out-file that already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardDecision {
    /// Read + remove it and finish (today's behaviour).
    Adopt,
    /// The parent still has launched/resumed tasks in flight — the file is a
    /// PARTIAL result; leave it on disk and keep watching.
    Hold,
    /// Held past [`HOLD_LINGER_CAP`]; adopt it anyway and say so.
    AdoptAfterCap,
}

/// Decide what to do with an existing out-file this tick. Pure so the
/// hold/cap policy is testable without a PTY or a live transcript.
pub(crate) fn guard_decision(
    out_exists: bool,
    scan: &ClaudeScan,
    held_since: Option<SystemTime>,
    now: SystemTime,
) -> GuardDecision {
    if !out_exists || scan.pending.is_empty() {
        return GuardDecision::Adopt;
    }
    let held = held_since
        .and_then(|s| now.duration_since(s).ok())
        .unwrap_or_default();
    if held >= HOLD_LINGER_CAP {
        GuardDecision::AdoptAfterCap
    } else {
        GuardDecision::Hold
    }
}

/// Whether the stuck trip fires. A result already sitting on disk is never
/// "stuck" — the agent did the work; whatever it is still doing (merging its
/// sub-agents' files) is progress the PTY clock cannot see.
pub(crate) fn stuck_fires(idle: Duration, stuck_idle: Duration, out_exists: bool) -> bool {
    !out_exists && idle >= stuck_idle
}

/// The orchestrator row's progress note: which lenses have written their file,
/// and how many sub-agents the parent is still waiting on.
pub(crate) fn lens_progress_note(done: &[String], total: usize, running: usize) -> String {
    let mut s = format!("lenses {}/{total} done", done.len());
    if !done.is_empty() {
        let ticks: Vec<String> = done.iter().map(|d| format!("{d} \u{2713}")).collect();
        s.push_str(&format!(" \u{00b7} {}", ticks.join(" ")));
    }
    s.push_str(&format!(" \u{00b7} {running} sub-agents running"));
    s
}

/// Idle measured from the newest artifact the agent (or any of its sub-agents)
/// touched, rather than from its own PTY: a parent waiting on four sub-agents
/// prints nothing while they write megabytes. `None` ⇒ no signal, caller falls
/// back to the PTY clock.
fn progress_idle(cwd: &str, provider_session_id: Option<&str>) -> Option<Duration> {
    let psid = provider_session_id?;
    let jsonl = otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid);
    let subagents = otto_orchestrator::claude_pty::project_dir(cwd)
        .join(psid)
        .join("subagents");
    let stamp = turn_oracle::progress_stamp(&jsonl, Some(&subagents), None)?;
    SystemTime::now().duration_since(stamp).ok()
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
    on_status: F,
) -> RunOutcome
where
    F: FnMut(WatchStatus) -> Fut,
    Fut: Future<Output = ()>,
{
    watch_for_result_guarded(
        manager,
        sid,
        provider,
        provider_session_id,
        cwd,
        out_path,
        timeout,
        waiting_idle,
        stuck_idle,
        transcript_ok,
        WatchGuard::default(),
        on_status,
        |_note: String| async {},
    )
    .await
}

/// [`watch_for_result`] with the R1 guard. With `guard.pending_aware` the loop
/// reads the claude transcript once per tick and:
///
/// - HOLDS an existing out-file while the parent has launched/resumed tasks
///   that never reported back (the orchestrator reviewer writes its merged file
///   last, but a sub-agent that writes the parent's path early would otherwise
///   end the watch with a partial result), bounded by [`HOLD_LINGER_CAP`];
/// - accepts a transcript turn only when the tail really is a native end-turn
///   with nothing pending (an `end_turn` emitted right after launching
///   sub-agents is not a finished turn);
/// - measures the stuck clock from the newest transcript/sub-agent artifact
///   instead of the parent's own silent PTY, and suspends it entirely while a
///   result is already on disk.
///
/// `on_note` carries the human progress note (rate-limited to one per
/// [`NOTE_MIN_INTERVAL`], and only when it changed).
#[allow(clippy::too_many_arguments)]
pub async fn watch_for_result_guarded<F, Fut, G, GFut>(
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
    guard: WatchGuard,
    mut on_status: F,
    mut on_note: G,
) -> RunOutcome
where
    F: FnMut(WatchStatus) -> Fut,
    Fut: Future<Output = ()>,
    G: FnMut(String) -> GFut,
    GFut: Future<Output = ()>,
{
    let deadline = Instant::now() + timeout;
    let guarded = guard.pending_aware && provider == "claude";
    let mut flagged_waiting = false;
    let mut flagged_over_budget = false;
    // When the out-file first appeared while tasks were still pending (its own
    // mtime, so a restarted watch inherits the real age).
    let mut held_since: Option<SystemTime> = None;
    let mut last_note: Option<String> = None;
    let mut last_note_at: Option<Instant> = None;
    loop {
        // One transcript read per tick feeds BOTH guards below. Unguarded, this
        // stays exactly where it was — after the out-file check.
        let scan = if guarded {
            provider_session_id
                .map(|psid| otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|raw| turn_oracle::scan_claude(&raw))
        } else {
            None
        };
        // Unguarded this stat never happens — the legacy path is untouched.
        let out_exists = guarded && out_path.exists();
        let pending = scan.as_ref().map(|s| s.pending.len()).unwrap_or(0);
        let decision = match scan.as_ref() {
            Some(s) if out_exists => {
                if !s.pending.is_empty() && held_since.is_none() {
                    held_since = std::fs::metadata(out_path)
                        .and_then(|m| m.modified())
                        .ok()
                        .or_else(|| Some(SystemTime::now()));
                }
                guard_decision(true, s, held_since, SystemTime::now())
            }
            _ => GuardDecision::Adopt,
        };

        if decision != GuardDecision::Hold {
            if let Ok(text) = std::fs::read_to_string(out_path) {
                let _ = std::fs::remove_file(out_path);
                if decision == GuardDecision::AdoptAfterCap {
                    warn!(
                        "agent_run: adopting findings after {}m with {pending} task(s) still pending",
                        HOLD_LINGER_CAP.as_secs() / 60
                    );
                    on_note(format!(
                        "\u{26a0} merged after {}m with {pending} sub-agents still pending",
                        HOLD_LINGER_CAP.as_secs() / 60
                    ))
                    .await;
                }
                return RunOutcome::ok(text, sid.clone());
            }
        }

        // claude writes a JSONL transcript; codex/agy don't, so the out-file is
        // their only signal. The caller decides what counts as a complete turn.
        if provider == "claude" && decision != GuardDecision::Hold {
            if let Some(psid) = provider_session_id {
                // Guarded: an end-turn is only a FINISHED turn when nothing the
                // parent launched is still outstanding (it ends one every time a
                // sub-agent reports back, and one right after launching them).
                let settled = scan
                    .as_ref()
                    .map(|s| s.tail_is_assistant_end_turn && s.pending.is_empty())
                    .unwrap_or(true);
                if settled {
                    let jsonl = otto_orchestrator::claude_pty::session_jsonl_path(cwd, psid);
                    if let Ok(raw) = std::fs::read_to_string(&jsonl) {
                        if let Some(turn) = otto_orchestrator::claude_pty::completed_turn_text(&raw)
                        {
                            if transcript_ok(&turn) {
                                return RunOutcome::ok(turn, sid.clone());
                            }
                        }
                    }
                }
            }
        }

        // Progress note (orchestrator rows / a held result). Rate-limited and
        // only on change, since the caller persists it.
        let note = if decision == GuardDecision::Hold {
            Some(format!(
                "findings written — {pending} sub-agents still running"
            ))
        } else if !guard.lens_files.is_empty() {
            let done: Vec<String> = guard
                .lens_files
                .iter()
                .filter(|(_, p)| p.exists())
                .map(|(slug, _)| slug.clone())
                .collect();
            Some(lens_progress_note(&done, guard.lens_files.len(), pending))
        } else {
            None
        };
        if let Some(note) = note {
            let changed = last_note.as_deref() != Some(note.as_str());
            let due = match last_note_at {
                Some(t) => t.elapsed() >= NOTE_MIN_INTERVAL,
                None => true,
            };
            if changed && due {
                on_note(note.clone()).await;
                last_note = Some(note);
                last_note_at = Some(Instant::now());
            }
        }

        match manager.live_handle(sid) {
            Some(handle) => {
                if handle.on_exit().borrow().is_some() {
                    return RunOutcome::failed(Some(sid.clone()), FailReason::Exited);
                }
                let idle = handle.last_output_at().elapsed();
                // Guarded, the stuck clock follows the artifacts (sub-agents
                // write their own transcripts while the parent's PTY is mute).
                let quiet_for = if guarded {
                    progress_idle(cwd, provider_session_id).unwrap_or(idle)
                } else {
                    idle
                };
                // Fail fast once truly silent for stuck_idle so recovery can retry.
                if stuck_fires(quiet_for, stuck_idle, out_exists) {
                    warn!(
                        "agent_run: session ({provider}) stuck — no output for {}s",
                        stuck_idle.as_secs()
                    );
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
                warn!(
                    "agent_run: session ({provider}) timed out (idle {}s past grace)",
                    idle.as_secs()
                );
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
    run_with_recovery_until(
        manager,
        max_attempts,
        backoff,
        cancel,
        || async { false },
        attempt,
    )
    .await
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
            warn!(
                "agent_run: retry attempt {}/{} (prev: {})",
                i + 1,
                max_attempts,
                last.reason.map(|r| r.as_str()).unwrap_or("?")
            );
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

    /// A scan with `n` outstanding `Agent` launches. Built field-by-field on
    /// top of `Default` — `ClaudeScan` grows fields, and an exhaustive literal
    /// here would break every time it does.
    fn scan_with_pending(n: usize) -> ClaudeScan {
        ClaudeScan {
            pending: (0..n)
                .map(|i| turn_oracle::PendingTask {
                    id: format!("task-{i}"),
                    kind: turn_oracle::TaskKind::Agent,
                    description: format!("sweep {i}"),
                    since_line: i,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn findings_file_held_while_async_children_pending() {
        let now = SystemTime::now();
        let held = now - Duration::from_secs(60);
        // Written while two sub-agents are still running ⇒ partial, hold it.
        assert_eq!(
            guard_decision(true, &scan_with_pending(2), Some(held), now),
            GuardDecision::Hold
        );
        // The last notification landed ⇒ the merged file is final, adopt it.
        assert_eq!(
            guard_decision(true, &scan_with_pending(0), Some(held), now),
            GuardDecision::Adopt
        );
        // Held past the cap ⇒ adopt anyway rather than park the reviewer.
        let stale = now - (HOLD_LINGER_CAP + Duration::from_secs(1));
        assert_eq!(
            guard_decision(true, &scan_with_pending(1), Some(stale), now),
            GuardDecision::AdoptAfterCap
        );
        // No file yet ⇒ nothing to decide, pending or not.
        assert_eq!(
            guard_decision(false, &scan_with_pending(3), None, now),
            GuardDecision::Adopt
        );
    }

    #[test]
    fn stuck_clock_suspended_while_out_file_exists() {
        let stuck = Duration::from_secs(900);
        // No result on disk: the trip works exactly as before.
        assert!(stuck_fires(Duration::from_secs(900), stuck, false));
        assert!(!stuck_fires(Duration::from_secs(899), stuck, false));
        // A result IS on disk (held for its sub-agents) — never stuck, however
        // long the parent's own PTY has been silent.
        assert!(!stuck_fires(Duration::from_secs(10_000), stuck, true));
    }

    #[test]
    fn watch_for_result_wrapper_has_no_guard() {
        // `watch_for_result` forwards `WatchGuard::default()`, so the legacy
        // callers keep the pre-R1 behaviour: no transcript pre-read, no hold,
        // no progress notes.
        let g = WatchGuard::default();
        assert!(!g.pending_aware);
        assert!(g.lens_files.is_empty());
    }

    #[test]
    fn lens_progress_note_reads_like_the_design_row() {
        assert_eq!(
            lens_progress_note(
                &["correctness".into(), "security".into(), "test".into()],
                6,
                2
            ),
            "lenses 3/6 done \u{00b7} correctness \u{2713} security \u{2713} test \u{2713} \u{00b7} 2 sub-agents running"
        );
        // Nothing done yet ⇒ no empty tick list in the middle.
        assert_eq!(
            lens_progress_note(&[], 4, 4),
            "lenses 0/4 done \u{00b7} 4 sub-agents running"
        );
    }
}
