//! The ONE assist turn engine (architecture-4-02). Canvas "Ask AI"
//! (`otto-canvas`), Design Hall assist / variants (`otto-design-assist`) and
//! story mockups (otto-server's `mockup_assist`) each used to carry their own
//! turn job and live-preview poll; they are now [`AssistTarget`] adapters over
//! [`run_assist_job`]:
//!
//!   1. the job runs only under an [`AssistClaim`] on the target's busy key
//!      (one turn per document; the claim also carries the Stop signal),
//!   2. [`AssistTarget::load`] puts the current document on disk,
//!   3. the agent turn runs ([`TurnRunner`]: the managed-session runner in
//!      production) while the file the agent edits is polled stat-gated
//!      ([`crate::live_file`]) and each change goes to [`AssistTarget::on_live`],
//!   4. the poll stops when the turn settles or the owner stops the job; a
//!      stopped job parses and applies NOTHING,
//!   5. otherwise the reply is parsed into a proposal and applied once.
//!
//! Each step is reported through [`AssistTarget::report`] and the outcome says
//! which [`Stage`] failed, so adapters keep their own wire shapes and errors.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::cancel_signal::{
    CancelSignal, InFlightGuard, InFlightSet, RunCancelGuard, RunCancels,
};
use otto_core::{Error, Id, Result};

use crate::live_file::{Poll, StatGate};
pub use crate::session_turn::AgentTurn;
use crate::session_turn::SessionManager;

/// Live-preview poll cadence while the agent edits.
pub const LIVE_POLL: Duration = Duration::from_millis(900);

/// Runs one agent turn for a job. The production runner is the host's
/// [`SessionManager`] (via [`crate::session_turn::run_agent_turn`], which
/// also enforces resume ownership); tests pass a fake.
pub trait TurnRunner: Send + Sync {
    /// Run `turn`; `on_ready` fires the moment the session exists. Returns
    /// `(reply_text, session_id)`.
    fn run_turn<F: FnOnce(&Id) + Send>(
        &self,
        turn: AgentTurn<'_>,
        on_ready: F,
    ) -> impl Future<Output = Result<(String, Id)>> + Send;
    /// Kill a session whose turn overran [`AssistTarget::turn_cap`].
    fn kill_session(&self, sid: &Id) -> impl Future<Output = ()> + Send;
}

impl TurnRunner for Arc<SessionManager> {
    fn run_turn<F: FnOnce(&Id) + Send>(
        &self,
        turn: AgentTurn<'_>,
        on_ready: F,
    ) -> impl Future<Output = Result<(String, Id)>> + Send {
        crate::session_turn::run_agent_turn(self, turn, on_ready)
    }

    async fn kill_session(&self, sid: &Id) {
        if let Err(error) = SessionManager::kill_session(self, sid).await {
            tracing::warn!(session = %sid, "assist: kill overrun session: {error}");
        }
    }
}

/// One engine's in-flight assist turns: the exclusive per-document slot and
/// its Stop signal (the shared `otto_core::cancel_signal` bookkeeping).
#[derive(Default)]
pub struct AssistSlots {
    busy: InFlightSet,
    stops: RunCancels,
}

impl AssistSlots {
    /// Claim `key`; `None` while another job holds it.
    pub fn claim(&'static self, key: &str) -> Option<AssistClaim> {
        let slot = self.busy.claim(key)?;
        Some(AssistClaim {
            key: key.to_string(),
            stop: self.stops.register(key),
            _slot: slot,
        })
    }

    /// Ask the job holding `key` to stop. False when none is running.
    pub fn stop(&self, key: &str) -> bool {
        self.stops.cancel(key)
    }
}

/// A held [`AssistSlots`] claim; released on drop. Field order matters: the
/// stop registration is removed BEFORE the slot frees, so a later job's
/// registration is never the one dropped.
pub struct AssistClaim {
    key: String,
    stop: RunCancelGuard,
    _slot: InFlightGuard,
}

impl AssistClaim {
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The Stop signal [`AssistSlots::stop`] fires.
    pub fn signal(&self) -> &CancelSignal {
        &self.stop.signal
    }

    /// True once the owner asked this job to stop.
    pub fn stopped(&self) -> bool {
        self.stop.signal.is_cancelled()
    }
}

/// Where a job failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Load,
    Turn,
    /// The turn overran [`AssistTarget::turn_cap`]; its session was killed.
    Deadline,
    Parse,
    Apply,
}

/// A state edge of one job, for [`AssistTarget::report`].
#[derive(Debug)]
pub enum AssistState<'a> {
    /// The agent session exists (turn start).
    Running {
        session: &'a Id,
    },
    /// The turn settled with a parsed proposal; applying it.
    Applying,
    Applied,
    /// The owner stopped the job: nothing was applied.
    Stopped,
    Failed {
        stage: Stage,
        error: &'a Error,
    },
}

/// How a job ended.
#[derive(Debug)]
pub enum AssistOutcome<A> {
    Applied {
        applied: A,
        session: Id,
    },
    Stopped {
        session: Option<Id>,
    },
    Failed {
        stage: Stage,
        error: Error,
        session: Option<Id>,
    },
}

/// One document an assist turn edits. Methods take `&self`: the adapter holds
/// its prepared state (paths, base document, prompt inputs).
pub trait AssistTarget: Send + Sync {
    /// The reply, parsed into a change to persist.
    type Proposal: Send;
    /// What a successful apply hands back to the caller.
    type Applied: Send;

    /// The key the job's [`AssistClaim`] must hold (one turn per document).
    fn busy_key(&self) -> &str;
    /// Put the current document on disk for the agent.
    fn load(&self) -> impl Future<Output = Result<()>> + Send;
    /// The full turn prompt.
    fn prompt(&self) -> String;
    /// The session parameters of the turn running `prompt`.
    fn turn<'a>(&'a self, prompt: &'a str) -> AgentTurn<'a>;
    /// Bound on the turn; past it the session is killed ([`Stage::Deadline`]).
    fn turn_cap(&self) -> Option<Duration> {
        None
    }
    /// The file the agent edits, polled for the live preview (`None`: no poll).
    fn live_path(&self) -> Option<PathBuf> {
        None
    }
    /// Read cap for one live poll (larger files are skipped, with a warning).
    fn live_cap(&self) -> usize;
    /// The bytes the poll starts from (an unchanged file is not re-sent).
    fn live_base(&self) -> Vec<u8> {
        Vec::new()
    }
    fn live_every(&self) -> Duration {
        LIVE_POLL
    }
    /// One live change: non-blank and different from the previous one.
    fn on_live(&self, _bytes: &[u8]) {}
    /// After the turn, before the stop check: keep `session` resumable.
    /// `turn_ok` says whether the turn itself succeeded.
    fn remember(&self, _session: &Id, _turn_ok: bool) -> impl Future<Output = ()> + Send {
        async {}
    }
    /// Turn the reply (and the agent's file) into a proposal.
    fn parse(&self, reply: &str) -> impl Future<Output = Result<Self::Proposal>> + Send;
    /// Persist the proposal; called at most once per job.
    fn apply(
        &self,
        proposal: Self::Proposal,
        session: &Id,
    ) -> impl Future<Output = Result<Self::Applied>> + Send;
    fn report(&self, _state: AssistState<'_>) {}
}

/// Run one assist job for `target` under `claim` (see the module docs).
pub async fn run_assist_job<R: TurnRunner, T: AssistTarget>(
    runner: &R,
    target: &T,
    claim: &AssistClaim,
) -> AssistOutcome<T::Applied> {
    let fail = |stage, error: Error, session| {
        target.report(AssistState::Failed {
            stage,
            error: &error,
        });
        AssistOutcome::Failed {
            stage,
            error,
            session,
        }
    };
    if claim.key() != target.busy_key() {
        let error = Error::Internal("assist job run without its document's claim".into());
        return fail(Stage::Load, error, None);
    }
    if let Err(error) = target.load().await {
        return fail(Stage::Load, error, None);
    }
    let prompt = target.prompt();
    let started: Mutex<Option<Id>> = Mutex::new(None);
    let on_ready = |sid: &Id| {
        *started.lock().unwrap_or_else(|e| e.into_inner()) = Some(sid.clone());
        target.report(AssistState::Running { session: sid });
    };
    let turn = runner.run_turn(target.turn(&prompt), on_ready);
    let cap = target.turn_cap();
    let capped = async {
        match cap {
            None => Ok(turn.await),
            Some(cap) => tokio::time::timeout(cap, turn).await.map_err(|_| cap),
        }
    };
    tokio::pin!(capped);
    // The poll lives inside this future: it ends when the turn settles (or the
    // whole job is dropped), and stops early when the owner stops the job.
    let result = tokio::select! {
        r = &mut capped => r,
        () = live_poll(target, claim.signal()) => capped.await,
    };
    let known = started.lock().unwrap_or_else(|e| e.into_inner()).take();
    let result = match result {
        Ok(turn) => turn.map_err(|e| (Stage::Turn, e)),
        Err(cap) => {
            if let Some(sid) = &known {
                runner.kill_session(sid).await;
            }
            Err((
                Stage::Deadline,
                Error::Upstream(format!(
                    "the agent turn exceeded {} minutes and was stopped",
                    cap.as_secs() / 60
                )),
            ))
        }
    };
    let session = result.as_ref().ok().map(|(_, s)| s.clone()).or(known);
    if let Some(sid) = &session {
        target.remember(sid, result.is_ok()).await;
    }
    if claim.stopped() {
        target.report(AssistState::Stopped);
        return AssistOutcome::Stopped { session };
    }
    let (reply, sid) = match result {
        Ok(v) => v,
        Err((stage, error)) => return fail(stage, error, session),
    };
    let proposal = match target.parse(&reply).await {
        Ok(p) => p,
        Err(error) => return fail(Stage::Parse, error, Some(sid)),
    };
    target.report(AssistState::Applying);
    match target.apply(proposal, &sid).await {
        Ok(applied) => {
            target.report(AssistState::Applied);
            AssistOutcome::Applied {
                applied,
                session: sid,
            }
        }
        Err(error) => fail(Stage::Apply, error, Some(sid)),
    }
}

/// Poll the target's live file until `stop` fires (forever when it never
/// does; the caller drops this future when the turn settles).
async fn live_poll<T: AssistTarget>(target: &T, stop: &CancelSignal) {
    let Some(path) = target.live_path() else {
        return stop.cancelled().await;
    };
    let (cap, every) = (target.live_cap(), target.live_every());
    let mut last = target.live_base();
    // Stat-gated: an unchanged (len, mtime) skips the read entirely.
    let mut gate = StatGate::default();
    while !stop.sleep(every).await {
        let bytes = match gate.poll(&path, cap).await {
            Poll::Changed(bytes) => bytes,
            Poll::TooLarge(len) => {
                tracing::warn!(
                    "assist: {} is {len} bytes, over {cap}; ignored",
                    path.display()
                );
                continue;
            }
            Poll::Unchanged | Poll::Missing => continue,
        };
        if bytes == last || bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        last = bytes;
        target.on_live(&last);
    }
}

#[cfg(test)]
mod tests;
