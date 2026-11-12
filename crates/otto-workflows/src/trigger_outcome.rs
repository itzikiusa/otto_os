//! What a trigger's latest attempt came to — "Last fired 2 h ago · started",
//! "Last attempt rejected: <reason>" — stamped on the trigger row
//! (`TriggersRepo::record_outcome`, one UPDATE per fire) where a schedule,
//! chat-binding or webhook trigger fires or is refused. A trigger used to show
//! nothing but `armed_at`, so one that silently never worked looked exactly
//! like one that did. Stamping is best-effort: a failed write is logged and
//! never stops the run it describes.

use otto_state::{DbPool, TriggersRepo, WorkflowsRepo};

use crate::chat_resolve::SlotOutlook;
use crate::run_queue::{max_parallel_runs, slot_outlook_now};

/// Cap on a stored rejection reason — the panel shows one line of it.
const DETAIL_CAP: usize = 200;

/// The outcome of one trigger attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOutcome {
    /// A run was admitted and got a slot at once.
    Started,
    /// A run was admitted and waits behind the parallel-run cap.
    Queued,
    /// No run: the reason, in a few words.
    Rejected(String),
}

impl TriggerOutcome {
    /// The stored `last_outcome` value.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Queued => "queued",
            Self::Rejected(_) => "rejected",
        }
    }

    /// The stored `last_detail`: the rejection reason, clipped to one line of
    /// at most [`DETAIL_CAP`] chars; `None` for an admitted run.
    pub fn detail(&self) -> Option<String> {
        let Self::Rejected(reason) = self else {
            return None;
        };
        let line = reason.lines().next().unwrap_or("").trim();
        Some(if line.chars().count() > DETAIL_CAP {
            format!("{}…", line.chars().take(DETAIL_CAP).collect::<String>())
        } else {
            line.to_string()
        })
    }
}

/// An admitted run's outcome from where it landed (`None` — the snapshot
/// failed — reads as started, the same rule the chat acknowledgement uses).
pub fn admitted(outlook: Option<SlotOutlook>) -> TriggerOutcome {
    match outlook {
        Some(SlotOutlook::Queued { .. }) => TriggerOutcome::Queued,
        _ => TriggerOutcome::Started,
    }
}

/// Stamp `outcome` on trigger `trigger_id`. Best-effort.
pub async fn stamp(pool: &DbPool, trigger_id: &str, outcome: &TriggerOutcome) {
    let detail = outcome.detail();
    if let Err(e) = TriggersRepo::new(pool.clone())
        .record_outcome(&trigger_id.to_string(), outcome.kind(), detail.as_deref())
        .await
    {
        tracing::warn!(
            trigger_id,
            "workflow trigger: recording its outcome failed: {e}"
        );
    }
}

/// Stamp an admitted run: `started` or `queued`, from the run queue as it is
/// now, not counting `run_id` itself (its row already exists).
pub async fn stamp_admitted(pool: &DbPool, trigger_id: &str, run_id: &str) {
    let repo = WorkflowsRepo::new(pool.clone());
    let outlook = slot_outlook_now(&repo, max_parallel_runs(), Some(run_id)).await;
    stamp(pool, trigger_id, &admitted(outlook)).await;
}

/// Stamp a refused attempt with its reason. Best-effort.
pub async fn stamp_rejected(pool: &DbPool, trigger_id: &str, reason: &str) {
    stamp(
        pool,
        trigger_id,
        &TriggerOutcome::Rejected(reason.to_string()),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_store_as_their_wire_names() {
        assert_eq!(TriggerOutcome::Started.kind(), "started");
        assert_eq!(TriggerOutcome::Queued.kind(), "queued");
        assert_eq!(TriggerOutcome::Rejected("x".into()).kind(), "rejected");
        assert_eq!(TriggerOutcome::Started.detail(), None);
    }

    #[test]
    fn a_rejection_reason_is_one_clipped_line() {
        let r = TriggerOutcome::Rejected("  path is outside every repo \nsecond line".into());
        assert_eq!(r.detail().as_deref(), Some("path is outside every repo"));
        let long = TriggerOutcome::Rejected("x".repeat(500));
        let d = long.detail().unwrap();
        assert_eq!(d.chars().count(), DETAIL_CAP + 1);
        assert!(d.ends_with('…'));
    }

    #[test]
    fn where_a_run_lands_decides_started_or_queued() {
        assert_eq!(
            admitted(Some(SlotOutlook::StartsNow)),
            TriggerOutcome::Started
        );
        assert_eq!(
            admitted(Some(SlotOutlook::Queued { ahead: 2 })),
            TriggerOutcome::Queued
        );
        assert_eq!(admitted(None), TriggerOutcome::Started);
    }
}
