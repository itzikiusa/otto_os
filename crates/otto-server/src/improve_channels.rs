//! Channel ↔ self-improvement glue: run Otto's improvement engine on a finished
//! Slack/Telegram interaction and return a one-line summary the mirror posts back
//! in the originating thread.
//!
//! This implements [`otto_channels::InteractionImprover`], injected into the
//! channel `Mirror` (mirrors the `SwarmTrigger` pattern). Keeping it here means
//! otto-channels needs no dependency on otto-improve.
//!
//! Behaviour:
//!   * **Gated** on the workspace's `self_improvement.enabled` toggle — returns
//!     `None` (silent) when off. The daemon additionally only wires this hook at
//!     all when `OTTO_SELF_IMPROVE` is on.
//!   * **Deduped** per session by transcript turn count, so a single turn never
//!     evolves twice and a trivial (empty) transcript is skipped.
//!   * **Quiet when nothing changed** — returns `None` unless the run applied or
//!     queued at least one edit, so the thread isn't pinged for no-ops.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use otto_channels::InteractionImprover;
use otto_core::domain::ImprovementEdit;
use otto_core::domain::ImprovementEditStatus;
use otto_core::Id;
use otto_improve::config::effective_config;
use otto_improve::digest::build_digest;
use otto_improve::ImprovementEngine;
use otto_state::{ImprovementsRepo, SessionsRepo, WorkspacesRepo};
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Runs single-interaction self-improvement and summarises the result.
pub struct InteractionImproverImpl {
    engine: Arc<ImprovementEngine>,
    workspaces: WorkspacesRepo,
    sessions: SessionsRepo,
    improvements: ImprovementsRepo,
    /// Per-session transcript turn count last evolved — skip when it hasn't grown
    /// (the same grown-check the in-loop `LiveEvolver` uses).
    seen: Mutex<HashMap<Id, usize>>,
}

impl InteractionImproverImpl {
    pub fn new(
        engine: Arc<ImprovementEngine>,
        workspaces: WorkspacesRepo,
        sessions: SessionsRepo,
        improvements: ImprovementsRepo,
    ) -> Self {
        Self {
            engine,
            workspaces,
            sessions,
            improvements,
            seen: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl InteractionImprover for InteractionImproverImpl {
    async fn evolve_interaction(&self, session_id: &Id) -> Option<String> {
        let session = self.sessions.get(session_id).await.ok()?;
        let ws = self.workspaces.get(&session.workspace_id).await.ok()?;

        // Gate: only when this workspace has self-improvement turned on.
        if !effective_config(&ws.settings).enabled {
            return None;
        }

        // Grown-check / dedup: skip an empty transcript and any turn we've already
        // evolved. Record BEFORE the (slow) run so a duplicate Final can't double-run.
        let turns = build_digest(&session).map(|d| d.turns).unwrap_or(0);
        if turns == 0 {
            return None;
        }
        {
            let mut seen = self.seen.lock().await;
            if seen.get(session_id).copied().unwrap_or(0) >= turns {
                return None;
            }
            seen.insert(session_id.clone(), turns);
        }

        info!(session = %session_id, "channel self-improvement: evolving interaction");
        let run_id = match self.engine.evolve_session(session_id).await {
            Ok(id) => id,
            Err(e) => {
                warn!(session = %session_id, "channel self-improvement run failed: {e}");
                return None;
            }
        };

        let edits = self.improvements.list_edits_by_run(&run_id).await.ok()?;
        summarize_edits(&edits)
    }
}

/// Build a short, thread-friendly summary of a run's edits, or `None` when the
/// run changed nothing (so the thread isn't pinged for a no-op review).
fn summarize_edits(edits: &[ImprovementEdit]) -> Option<String> {
    let applied = unique_targets(edits, ImprovementEditStatus::Applied);
    let pending = edits
        .iter()
        .filter(|e| e.status == ImprovementEditStatus::Pending)
        .count();

    if applied.is_empty() && pending == 0 {
        return None;
    }

    let mut msg = String::from("🛠️ Self-improvement: ");
    if applied.is_empty() {
        msg.push_str(&format!(
            "{pending} change{} proposed, awaiting approval",
            if pending == 1 { "" } else { "s" }
        ));
    } else {
        msg.push_str(&format!("updated {}", applied.join(", ")));
        if pending > 0 {
            msg.push_str(&format!(
                " · {pending} more awaiting approval"
            ));
        }
    }
    Some(msg)
}

/// Distinct edit targets (`skill X` / `memory Y.md`) with `status`, in first-seen
/// order — so two edits to the same skill collapse to one mention.
fn unique_targets(edits: &[ImprovementEdit], status: ImprovementEditStatus) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in edits.iter().filter(|e| e.status == status) {
        let described = describe_target(&e.target_ref);
        if !out.contains(&described) {
            out.push(described);
        }
    }
    out
}

/// Human-readable target: a `*.md` is a memory file; anything else is a skill
/// name (mirrors otto-improve's pathsafe convention + the existing notifier).
fn describe_target(target_ref: &str) -> String {
    if target_ref.ends_with(".md") {
        format!("memory `{target_ref}`")
    } else {
        format!("skill `{target_ref}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::domain::{ImprovementEditKind, ImprovementRisk, ImprovementTarget};

    fn edit(target_ref: &str, status: ImprovementEditStatus) -> ImprovementEdit {
        ImprovementEdit {
            id: "e1".into(),
            run_id: "r1".into(),
            workspace_id: "ws1".into(),
            target: ImprovementTarget::Skill,
            target_ref: target_ref.into(),
            target_path: format!("/{target_ref}"),
            kind: ImprovementEditKind::Modify,
            risk: ImprovementRisk::Low,
            status,
            rationale: String::new(),
            evidence: Vec::new(),
            before_content: None,
            after_content: String::new(),
            applied_at: None,
            actor: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn nothing_changed_is_silent() {
        assert_eq!(summarize_edits(&[]), None);
        assert_eq!(
            summarize_edits(&[edit("x", ImprovementEditStatus::Rejected)]),
            None
        );
    }

    #[test]
    fn applied_edits_collapse_duplicate_targets() {
        let edits = vec![
            edit("frb-grant-failure", ImprovementEditStatus::Applied),
            edit("frb-grant-failure", ImprovementEditStatus::Applied),
            edit("MEMORY.md", ImprovementEditStatus::Applied),
        ];
        assert_eq!(
            summarize_edits(&edits).unwrap(),
            "🛠️ Self-improvement: updated skill `frb-grant-failure`, memory `MEMORY.md`"
        );
    }

    #[test]
    fn pending_only_says_awaiting_approval() {
        let edits = vec![
            edit("a", ImprovementEditStatus::Pending),
            edit("b", ImprovementEditStatus::Pending),
        ];
        assert_eq!(
            summarize_edits(&edits).unwrap(),
            "🛠️ Self-improvement: 2 changes proposed, awaiting approval"
        );
    }

    #[test]
    fn applied_plus_pending_notes_both() {
        let edits = vec![
            edit("router", ImprovementEditStatus::Applied),
            edit("risky", ImprovementEditStatus::Pending),
        ];
        assert_eq!(
            summarize_edits(&edits).unwrap(),
            "🛠️ Self-improvement: updated skill `router` · 1 more awaiting approval"
        );
    }
}
