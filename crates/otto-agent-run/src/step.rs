//! The one interface every engine drives an unattended agent step through
//! (workflow steps, review agents): [`StepRunner`]. Two implementations —
//! `otto_agent_protocol::step_runner::ThreadStepRunner` (a structured
//! runtime thread, the default) and otto-server's `PtyStepRunner` (a terminal
//! session, frozen, removed one release later). No runtime dependency here,
//! so otto-review and the workflow engine compile against the trait alone.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_core::agent_runtime::{
    Attention, SubThreadUse, ThreadTokenUsage, TranscriptDigest, TurnTokenUsage, WaitReason,
};
use otto_core::cancel_signal::CancelSignal;
use otto_core::workflows::NodeActivity;
use otto_core::{Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

/// One step's turn: what to run, where, and on what.
#[derive(Debug, Clone)]
pub struct StepTurn {
    pub user_id: Id,
    pub workspace_id: Id,
    pub provider: String,
    /// `None` / `""` = the provider's default (no flag); validated before the call.
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cwd: PathBuf,
    /// `WF <workflow> · <step> · run <last6>`.
    pub title: String,
    /// Engine-owned: `{source, run_id, node_id, node_kind, wf_name, cwd, attempt, work}`.
    pub meta: Value,
    pub prompt: String,
    /// Continue this earlier step's thread instead of opening a fresh one.
    pub continue_thread: Option<Id>,
    /// The step file the agent must write (the engine deletes it before the call).
    pub handoff: Option<PathBuf>,
    pub read_only: bool,
    /// The one hard per-step budget: a slow agent and an unanswered wait alike.
    pub timeout: Option<Duration>,
    /// No event for this long ends the step `stalled` (review agents' grace;
    /// workflow steps `None`: the runtime's own stall watchdog only).
    pub no_progress: Option<Duration>,
}

/// Progress, in order: `Waiting`* → `Started` → `Working` →
/// (`NeedsYou` → `Resumed`)* → `Finishing`.
#[derive(Debug, Clone, PartialEq)]
pub enum StepEvent {
    /// The machine's admission gate holds the step (cancel-aware).
    Waiting(WaitReason),
    /// The thread exists; `start_ordinal` is its turn ordinal before the
    /// step's first turn (0 for a fresh thread) — the engine keeps it on
    /// `StepThread.start_ordinal` and hands it back to [`StepRunner::resume`].
    Started {
        thread_id: Id,
        start_ordinal: u32,
    },
    Working,
    /// A live wait on a person: an approval or a question, or a usage limit
    /// with a known reset the runner waits out.
    NeedsYou(Attention),
    /// The wait cleared; `answer` / `by` say what a person (or a parent
    /// agent, `thread:<id>`) answered.
    Resumed {
        request_id: Option<Id>,
        answer: Option<String>,
        by: Option<Id>,
    },
    /// The turn ended; background work, sub-threads or the handoff nudge remain.
    Finishing,
    Activity(NodeActivity),
    /// The PTY runner's phase lines.
    Log(String),
}

/// The class of a failed step, from the runner (the engine adds `script`,
/// `outcome_unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepErrorClass {
    /// A plan limit or billing failure (never retried by backoff).
    UsageLimit,
    /// A per-minute 429: retryable with the engine's backoff.
    RateLimit,
    Auth,
    Provider,
    Stalled,
    Timeout,
    Spawn,
    ServerRestart,
    Canceled,
}

impl StepErrorClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsageLimit => "usage_limit",
            Self::RateLimit => "rate_limit",
            Self::Auth => "auth",
            Self::Provider => "provider",
            Self::Stalled => "stalled",
            Self::Timeout => "timeout",
            Self::Spawn => "spawn",
            Self::ServerRestart => "server_restart",
            Self::Canceled => "canceled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepError {
    pub class: StepErrorClass,
    pub message: String,
    /// The provider's own code (`ProviderFailure.code`, e.g. `contextWindowExceeded`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl StepError {
    pub fn new(class: StepErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
            code: None,
        }
    }
}

/// What a settled step leaves: everything the run record keeps after the
/// thread is gone.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutcome {
    pub thread_id: Id,
    pub reply: String,
    pub handoff_written: bool,
    /// `completed` · `failed` · `interrupted` · `canceled`.
    pub turn_status: String,
    pub error: Option<StepError>,
    /// The thread's context meter at settle.
    pub context_usage: Option<ThreadTokenUsage>,
    /// Turns of this step (the handoff nudge and a restart's turns included).
    pub turns: u32,
    pub usage: TurnTokenUsage,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Threads the agent opened itself, removed ones included.
    pub sub_threads: Vec<SubThreadUse>,
    pub digest: Option<TranscriptDigest>,
}

#[async_trait]
pub trait StepRunner: Send + Sync {
    /// Run the step to settle. `Err` is reserved for Otto's own faults (the
    /// store); every agent-side failure is `Ok` with [`StepOutcome::error`].
    async fn run(
        &self,
        t: StepTurn,
        ev: UnboundedSender<StepEvent>,
        cancel: CancelSignal,
    ) -> Result<StepOutcome>;
    /// After a daemon restart: a held thread is awaited as if nothing happened
    /// (usage counted from `start_ordinal`); an unheld one is told Otto
    /// restarted and finishes on the same conversation, or returns
    /// `server_restart` when that is impossible.
    async fn resume(
        &self,
        thread_id: &Id,
        start_ordinal: u32,
        t: &StepTurn,
        ev: UnboundedSender<StepEvent>,
        cancel: CancelSignal,
    ) -> Result<StepOutcome>;
    /// Stop the thread and its open sub-threads and write `meta.retain_until`;
    /// the runtime's sweep deletes it after that.
    async fn close(&self, thread_id: &Id, retain_until: DateTime<Utc>);
    /// Rewrite `retain_until` on each id (open or closed; deleted ids are skipped).
    async fn set_retain_until(&self, thread_ids: &[Id], retain_until: DateTime<Utc>);
    /// Stop a kept thread's process (a later `continue_thread` respawns it with resume).
    async fn release_process(&self, thread_id: &Id);
    /// Delete now.
    async fn delete(&self, thread_id: &Id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_classes_serialize_as_the_agreed_words() {
        for (class, word) in [
            (StepErrorClass::UsageLimit, "usage_limit"),
            (StepErrorClass::RateLimit, "rate_limit"),
            (StepErrorClass::ServerRestart, "server_restart"),
            (StepErrorClass::Canceled, "canceled"),
        ] {
            assert_eq!(class.as_str(), word);
            assert_eq!(serde_json::to_value(class).unwrap(), word);
        }
        let e = StepError::new(StepErrorClass::Timeout, "waited on a question");
        assert!(serde_json::to_value(&e).unwrap().get("code").is_none());
    }
}
