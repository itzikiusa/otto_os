//! Settled turns → the Usage store (review I-03).
//!
//! The runtime knows each turn's token usage (`ProviderTurn.turn_token_usage`)
//! but nothing billed it, so Usage showed no tokens for opencode, pi or
//! gemini threads. When a turn settles, its final usage (a running total
//! overwritten as the turn went, so the settled value IS the turn's total) is
//! handed to otto-usage once, which records it for the providers whose
//! transcripts the tailer does not read and ignores the rest (Claude, Codex,
//! Grok, agy, Muse: their CLIs' own records are billed, attributed to the thread).
//! The row is keyed by session + turn, so a repeat never counts twice.

use std::sync::Arc;

use otto_core::agent_runtime::{ProviderTurn, RuntimeSession};

use super::{AgentRuntime, Inner, State};

pub mod resume_at;

impl AgentRuntime {
    /// Wire the daemon's usage store (once; a later call is ignored). Until
    /// then — and in tests that never call it — settled turns record nothing.
    pub fn set_usage_engine(&self, engine: Arc<otto_usage::UsageEngine>) {
        let _ = self.inner.usage.set(engine);
    }
}

/// `turn` just settled (`finish_turn`): record its usage, if any.
pub(super) fn record_settled_turn(inner: &Inner, st: &State, turn: &ProviderTurn) {
    let Some(engine) = inner.usage.get() else {
        return;
    };
    if let Some(usage) = settled_usage(&st.session, turn) {
        engine.record_runtime_turn(&usage);
    }
}

/// The hand-over for `turn` of `session`, or `None` when it reported nothing.
fn settled_usage(
    session: &RuntimeSession,
    turn: &ProviderTurn,
) -> Option<otto_usage::RuntimeTurnUsage> {
    let usage = turn.turn_token_usage.clone()?;
    let settled = turn.completed_at.unwrap_or_else(chrono::Utc::now);
    let duration_ms = turn
        .started_at
        .and_then(|s| (settled - s).num_milliseconds().try_into().ok())
        .unwrap_or(0);
    Some(otto_usage::RuntimeTurnUsage {
        session_id: session.id.clone(),
        workspace_id: session.workspace_id.clone(),
        provider: session.provider.as_str().to_string(),
        // The turn's own model (pinned, or what the agent reported when it
        // started), else the thread's.
        model: turn
            .model
            .clone()
            .or_else(|| session.model.clone())
            .unwrap_or_default(),
        turn_id: turn.id.clone(),
        settled_at: settled.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        duration_ms,
        usage,
    })
}
