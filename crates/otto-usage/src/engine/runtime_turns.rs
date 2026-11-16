//! Usage of structured-runtime (Threads) turns whose CLI leaves no transcript
//! the tailer reads — gemini, opencode, pi (review I-03).
//!
//! The runtime settles each turn with the agent's own token counts
//! (`ProviderTurn.turn_token_usage`: the prompt response's `usage`, pi's
//! summed `message_end`s, an ACP `usage_update` cost). The settled value is
//! the turn's TOTAL — drivers that report running totals overwrite it as the
//! turn goes — so one row per turn, written once it settles, is exact.
//!
//! Claude, Codex, Grok, Antigravity and Muse are NOT recorded here: their CLIs
//! write their usual transcripts / stores in runtime mode too, and the tailer
//! bills those (attributing them to the thread through its native id). A row
//! from both paths would count the turn twice.
//!
//! Each row goes through the durable outbox under the id
//! `runtime-turn:<session>:<turn>` and is dated at the turn's settle time, so
//! a repeated hand-over (a retry, a re-settled turn after a restart) lands on
//! the same `usage_events_durable` key and the FINAL read counts it once.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use otto_core::agent_runtime::TurnTokenUsage;
use otto_core::Result;

use super::{journal_error, stamp_ts, unix_now, UsageEngine};
use crate::ingestion::DurableEvent;
use crate::UsageEvent;

/// Providers whose own on-disk records the tailer already bills
/// (`tailer/driver.rs`: Claude, Codex; `tailer/driver/native.rs`: Grok, agy,
/// Muse).
pub const TRANSCRIPT_TAILED_PROVIDERS: [&str; 5] = ["claude", "codex", "grok", "agy", "muse"];

/// Whether `provider`'s usage reaches the store through the transcript
/// tailer (so a runtime turn on it must not be recorded again).
pub fn transcript_tailed(provider: &str) -> bool {
    TRANSCRIPT_TAILED_PROVIDERS.contains(&provider)
}

/// `origin` of a settled runtime turn's row.
pub const RUNTIME_TURN_ORIGIN: &str = "thread";

/// One settled runtime turn, as the runtime hands it over.
#[derive(Debug, Clone, Default)]
pub struct RuntimeTurnUsage {
    /// The runtime thread's id (what the Threads page and the usage rows key on).
    pub session_id: String,
    pub workspace_id: String,
    /// `RuntimeProvider::as_str` (`gemini`, `opencode`, `pi`, …).
    pub provider: String,
    /// The model the turn ran on (empty = the provider's unreported default).
    pub model: String,
    pub turn_id: String,
    /// When the turn settled (RFC3339). Also the row's `ts`: a replay of the
    /// same turn must land on the same `event_date` to collapse.
    pub settled_at: String,
    pub duration_ms: u64,
    /// The runtime's normalized counts: `input_tokens` INCLUDES the cache reads
    /// (`cached_input_tokens`); `reasoning_tokens` is reported apart from
    /// `output_tokens` by the ACP agents (`thoughtTokens`) and opencode.
    pub usage: TurnTokenUsage,
}

/// The durable id and row for `turn`, or `None` when the tailer owns the
/// provider or the turn reported nothing (no tokens and no cost).
pub fn runtime_turn_event(turn: &RuntimeTurnUsage) -> Option<(String, UsageEvent)> {
    if transcript_tailed(&turn.provider) || turn.session_id.is_empty() || turn.turn_id.is_empty() {
        return None;
    }
    let u = &turn.usage;
    // The store keeps fresh input and cache reads apart (the Claude/Codex
    // rows do); a cache figure above the input total is clamped, not trusted.
    let cache_read = u.cached_input_tokens.unwrap_or(0).min(u.input_tokens);
    let input = u.input_tokens - cache_read;
    // Thinking is billed as output; the agents report it apart from it.
    let output = u
        .output_tokens
        .saturating_add(u.reasoning_tokens.unwrap_or(0));
    let reported = u.cost_usd.filter(|c| c.is_finite() && *c >= 0.0);
    if input == 0 && output == 0 && cache_read == 0 && reported.unwrap_or(0.0) == 0.0 {
        return None;
    }
    // The agent's own figure wins. Otherwise estimate only a model on the
    // rate card: the unknown-model fallback is Opus-tier and would invent
    // spend for a local or free model.
    let cost_usd = reported.unwrap_or_else(|| {
        if crate::is_priced(&turn.model) {
            crate::estimate_cost(&turn.model, input, output, cache_read, 0)
        } else {
            0.0
        }
    });
    let id = format!("runtime-turn:{}:{}", turn.session_id, turn.turn_id);
    Some((
        id,
        UsageEvent {
            ts: (!turn.settled_at.is_empty()).then(|| turn.settled_at.clone()),
            workspace_id: turn.workspace_id.clone(),
            session_id: turn.session_id.clone(),
            provider: turn.provider.clone(),
            model: turn.model.clone(),
            kind: "completion".into(),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_write_tokens: 0,
            cost_usd,
            duration_ms: turn.duration_ms,
            origin: RUNTIME_TURN_ORIGIN.into(),
            ..Default::default()
        },
    ))
}

impl UsageEngine {
    /// Record a settled runtime turn (fire-and-forget, safe to call under a
    /// lock): nothing for a tailed provider or an empty turn, else one durable
    /// row keyed by session + turn (see the module docs).
    pub fn record_runtime_turn(self: &Arc<Self>, turn: &RuntimeTurnUsage) {
        let Some((id, event)) = runtime_turn_event(turn) else {
            return;
        };
        if !self.config().enabled {
            return;
        }
        let Ok(rt) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("usage: runtime turn {id} settled off a Tokio runtime; not recorded");
            return;
        };
        let engine = Arc::clone(self);
        rt.spawn(async move {
            if let Err(e) = engine.enqueue_runtime_turn(id.clone(), event).await {
                tracing::warn!("usage: runtime turn {id} not recorded: {e}");
            }
        });
    }

    /// [`Self::record_runtime_turn`]'s write, awaited (tests and callers that
    /// must know the row is queued). Returns whether a new row was queued.
    pub async fn enqueue_runtime_turn(&self, id: String, mut event: UsageEvent) -> Result<bool> {
        stamp_ts(&mut event);
        let added = self
            .ingestion()
            .await?
            .enqueue_once(&DurableEvent {
                event_id: id,
                event,
            })
            .await
            .map_err(journal_error)?;
        if added {
            self.last_record.store(unix_now(), Ordering::Relaxed);
            self.usage_gen.fetch_add(1, Ordering::SeqCst);
        }
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(provider: &str) -> RuntimeTurnUsage {
        RuntimeTurnUsage {
            session_id: "thread-1".into(),
            workspace_id: "w1".into(),
            provider: provider.into(),
            model: "gemini-2.5-pro".into(),
            turn_id: "turn-1".into(),
            settled_at: "2026-11-16T10:00:00.000Z".into(),
            duration_ms: 1200,
            usage: TurnTokenUsage {
                input_tokens: 7,
                output_tokens: 3,
                cached_input_tokens: Some(2),
                reasoning_tokens: Some(1),
                cost_usd: None,
            },
        }
    }

    #[test]
    fn runtime_usage_row_splits_cache_adds_thinking_and_keys_by_turn() {
        let (id, ev) = runtime_turn_event(&turn("gemini")).unwrap();
        assert_eq!(id, "runtime-turn:thread-1:turn-1");
        assert_eq!(
            (
                ev.input_tokens,
                ev.cache_read_tokens,
                ev.output_tokens,
                ev.cache_write_tokens
            ),
            (5, 2, 4, 0)
        );
        assert_eq!(
            (
                ev.session_id.as_str(),
                ev.workspace_id.as_str(),
                ev.provider.as_str(),
                ev.model.as_str(),
                ev.origin.as_str(),
                ev.kind.as_str()
            ),
            (
                "thread-1",
                "w1",
                "gemini",
                "gemini-2.5-pro",
                RUNTIME_TURN_ORIGIN,
                "completion"
            )
        );
        assert_eq!(ev.ts.as_deref(), Some("2026-11-16T10:00:00.000Z"));
        assert_eq!(ev.duration_ms, 1200);
    }

    #[test]
    fn runtime_usage_skips_tailed_providers_and_empty_turns() {
        for p in TRANSCRIPT_TAILED_PROVIDERS {
            assert!(runtime_turn_event(&turn(p)).is_none(), "{p}");
        }
        for p in ["gemini", "opencode", "pi"] {
            assert!(runtime_turn_event(&turn(p)).is_some(), "{p}");
        }
        let mut empty = turn("pi");
        empty.usage = TurnTokenUsage::default();
        assert!(runtime_turn_event(&empty).is_none());
        // A cost-only turn (an ACP `usage_update` without a `usage` object)
        // is still spend.
        empty.usage.cost_usd = Some(0.25);
        let (_, ev) = runtime_turn_event(&empty).unwrap();
        assert_eq!((ev.input_tokens, ev.output_tokens), (0, 0));
        assert_eq!(ev.cost_usd, 0.25);
    }

    #[test]
    fn runtime_usage_cost_is_reported_else_rate_card_else_zero() {
        let mut t = turn("pi");
        t.usage.cost_usd = Some(0.5);
        assert_eq!(runtime_turn_event(&t).unwrap().1.cost_usd, 0.5);
        t.usage.cost_usd = Some(f64::NAN);
        t.model = "some-local-model".into();
        assert_eq!(runtime_turn_event(&t).unwrap().1.cost_usd, 0.0);
        // A cache figure above the input total is clamped.
        t.usage.cached_input_tokens = Some(99);
        let (_, ev) = runtime_turn_event(&t).unwrap();
        assert_eq!((ev.input_tokens, ev.cache_read_tokens), (0, 7));
    }

    #[tokio::test]
    async fn runtime_usage_turn_recorded_twice_queues_one_row() {
        let root = tempfile::tempdir().unwrap();
        let usage = UsageEngine::start(
            crate::UsageConfig {
                enabled: true,
                clickhouse_path: Some(
                    root.path()
                        .join("missing-clickhouse")
                        .to_string_lossy()
                        .into_owned(),
                ),
                ..Default::default()
            },
            root.path().join("usage"),
        )
        .await;
        let (id, ev) = runtime_turn_event(&turn("pi")).unwrap();
        assert!(usage
            .enqueue_runtime_turn(id.clone(), ev.clone())
            .await
            .unwrap());
        assert!(!usage.enqueue_runtime_turn(id, ev).await.unwrap());
        let journal = usage.ingestion().await.unwrap();
        let rows = journal.pending(100).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_id, "runtime-turn:thread-1:turn-1");
        assert_eq!(rows[0].event.provider, "pi");
    }
}
