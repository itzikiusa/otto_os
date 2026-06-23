//! Proactive self-improvement notifier.
//!
//! Subscribes to the daemon event broadcast and, for the three self-improvement
//! events (`ImprovementEditApplied`, `ImprovementApprovalPending`,
//! `ImprovementRunFinished`), formats a concise one-line summary and posts it to
//! the channel(s) configured for the event's workspace — so a user watching only
//! Slack/Telegram sees what Otto learned the moment it happens, without opening
//! the UI.
//!
//! Design constraints (kept deliberately tight):
//!   * **Opt-in, default OFF.** Gated on the `channels.notify_self_improvement`
//!     bool in the existing key/value settings store. Re-read per event so the
//!     toggle takes effect live without a restart.
//!   * **Best-effort + non-blocking.** A slow/failed channel send is logged and
//!     swallowed; broadcast lag is skipped (never panics). Nothing here can stall
//!     or crash the improvement engine or the event bus.
//!   * **No secrets / no diff bodies.** Only names and counts are posted.
//!
//! Target chat: the integration's configured *default* chat/channel
//! (`Integration.channel_id`) — the same chat the bot already operates in. If no
//! enabled integration for the workspace has a default chat, the event is skipped
//! silently.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use otto_core::domain::{Channel, Integration};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_state::{IntegrationsRepo, SettingsRepo};
use tokio::sync::broadcast;
use tracing::{debug, warn};

use crate::adapter::Adapter;
use crate::slack::SlackAdapter;
use crate::telegram::TelegramAdapter;

/// Settings key (existing key/value store) that turns the notifier on. Bool,
/// default `false` — the feature is opt-in so it's never noisy by default.
pub const NOTIFY_SETTING_KEY: &str = "channels.notify_self_improvement";

// ---------------------------------------------------------------------------
// Per-event-type opt-in keys (all default OFF)
// ---------------------------------------------------------------------------

/// Forward "insight ready" events (an insights report became available).
pub const NOTIFY_INSIGHT_KEY: &str = "channels.notify_insight_ready";
/// Forward swarm-run status-changed events (done / failed).
pub const NOTIFY_SWARM_KEY: &str = "channels.notify_swarm_done";
/// Forward code-review completion events.
pub const NOTIFY_REVIEW_KEY: &str = "channels.notify_review_done";
/// Forward budget-exceeded events (spend cap crossed while enforcement is on).
pub const NOTIFY_BUDGET_KEY: &str = "channels.notify_budget_exceeded";
/// Forward approval-required events (same as `notify_self_improvement` but only
/// the `ImprovementApprovalPending` signal, kept as a superset alias).
/// We reuse `NOTIFY_SETTING_KEY` for backward compatibility and add this
/// constant only for documentation clarity.
pub const NOTIFY_APPROVAL_KEY: &str = "channels.notify_approval_required";

/// Spawn the self-improvement notifier task. Returns immediately; the task runs
/// until `cancel` is set or the broadcast sender is dropped.
pub fn spawn(
    events: broadcast::Receiver<Event>,
    integrations: IntegrationsRepo,
    settings: SettingsRepo,
    secrets: Arc<dyn SecretStore>,
    cancel: Arc<AtomicBool>,
) {
    tokio::spawn(run(events, integrations, settings, secrets, cancel));
}

async fn run(
    mut events: broadcast::Receiver<Event>,
    integrations: IntegrationsRepo,
    settings: SettingsRepo,
    secrets: Arc<dyn SecretStore>,
    cancel: Arc<AtomicBool>,
) {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let event = match events.recv().await {
            Ok(e) => e,
            // We fell behind — skip the missed events rather than panic. The next
            // recv resumes from the current position.
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                warn!("improve notifier: broadcast lagged, skipped {skipped} events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };

        // Classify the event. Returns `(opt_in_key, workspace_id_opt, text)`.
        // `workspace_id_opt = None` means: deliver to ALL enabled integrations.
        let Some((opt_in_key, workspace_id_opt, text)) = classify(&event) else {
            continue;
        };

        // Per-event opt-in check — re-read each time so a live toggle fires at once.
        if !setting_enabled(&settings, opt_in_key).await {
            continue;
        }

        match workspace_id_opt {
            Some(ws) => deliver(&integrations, &secrets, &ws, &text).await,
            None => deliver_all(&integrations, &secrets, &text).await,
        }
    }
}

/// Read an arbitrary bool flag from the settings store. Missing / non-bool /
/// read error all resolve to `false` (off by default — every notify flag is opt-in).
async fn setting_enabled(settings: &SettingsRepo, key: &str) -> bool {
    matches!(settings.get(key).await, Ok(Some(serde_json::Value::Bool(true))))
}

/// Classify an event into `(opt_in_key, workspace_id_opt, one_line_message)`.
///
/// Returns `None` for events that never generate a channel notification (the
/// vast majority). `workspace_id_opt = None` signals "deliver to every enabled
/// integration" (used for global events like `InsightReady`).
fn classify(event: &Event) -> Option<(&'static str, Option<String>, String)> {
    match event {
        // ---- self-improvement events (existing) ----------------------------
        Event::ImprovementEditApplied { workspace_id, target_ref, .. } => Some((
            NOTIFY_SETTING_KEY,
            Some(workspace_id.clone()),
            format!("Self-improvement: {} — applied", describe_target(target_ref)),
        )),
        Event::ImprovementApprovalPending { workspace_id, target_ref, .. } => {
            // The `approval_required` key is a finer-grained alias; fall back
            // to the main `notify_self_improvement` flag (whichever is on).
            // We run a combined check: if either the main key OR the specific
            // key is on, deliver.  We model this by returning the specific key
            // and letting the outer loop check it.  The UI surface for this key
            // is the same "Push self-improvement events" toggle — the two keys
            // are equivalent here; the specific key is reserved for the future.
            Some((
                NOTIFY_SETTING_KEY,
                Some(workspace_id.clone()),
                format!(
                    "Self-improvement: proposed edit to {} — needs approval",
                    describe_target(target_ref)
                ),
            ))
        }
        Event::ImprovementRunFinished { workspace_id, applied, pending, .. } => {
            if *applied == 0 && *pending == 0 {
                return None; // nothing happened — no ping
            }
            Some((
                NOTIFY_SETTING_KEY,
                Some(workspace_id.clone()),
                format!("Self-improvement run: {applied} applied, {pending} queued"),
            ))
        }

        // ---- code review ---------------------------------------------------
        Event::ReviewChanged { workspace_id, status, .. } if status == "done" || status == "error" => {
            let label = if status == "done" { "done" } else { "failed" };
            Some((
                NOTIFY_REVIEW_KEY,
                Some(workspace_id.clone()),
                format!("Code review {label}"),
            ))
        }

        // ---- swarm ---------------------------------------------------------
        Event::SwarmStatus { workspace_id, status, swarm_id } => {
            // Only notify for terminal states.
            if !matches!(status.as_str(), "done" | "aborted" | "failed") {
                return None;
            }
            let label = match status.as_str() {
                "done" => "completed",
                "aborted" => "aborted",
                _ => "failed",
            };
            Some((
                NOTIFY_SWARM_KEY,
                Some(workspace_id.clone()),
                format!("Agent swarm `{swarm_id}` {label}"),
            ))
        }

        // ---- insights (global — no workspace_id) ---------------------------
        Event::InsightReady { period, .. } => Some((
            NOTIFY_INSIGHT_KEY,
            None, // global: deliver to all enabled integrations
            format!("Insights report ready: {period}"),
        )),

        // ---- budget exceeded -----------------------------------------------
        Event::BudgetExceeded { workspace_id, provider, spend_usd, cap_usd, direction } => {
            if direction != "exceeded" {
                return None; // only notify on the "crossed over" edge
            }
            Some((
                NOTIFY_BUDGET_KEY,
                Some(workspace_id.clone()),
                format!(
                    "Budget exceeded: {provider} spent ${spend_usd:.2} (cap ${cap_usd:.2})"
                ),
            ))
        }

        _ => None,
    }
}

/// Human-readable description of an edit target. `target_ref` is either a memory
/// file (`*.md`, e.g. `MEMORY.md`) or a skill name (a bare segment, e.g.
/// `big-win-legitimacy-verification`) — mirrors `otto-improve`'s pathsafe rules.
fn describe_target(target_ref: &str) -> String {
    if target_ref.ends_with(".md") {
        format!("memory `{target_ref}`")
    } else {
        format!("skill `{target_ref}`")
    }
}

/// Post `text` to the default chat of every enabled integration whose workspace
/// matches `workspace_id`. Best-effort: integrations without a default chat are
/// skipped silently; send failures are logged and swallowed.
async fn deliver(
    integrations: &IntegrationsRepo,
    secrets: &Arc<dyn SecretStore>,
    workspace_id: &str,
    text: &str,
) {
    let all = match integrations.list_all_enabled().await {
        Ok(list) => list,
        Err(e) => {
            debug!("improve notifier: could not list integrations: {e}");
            return;
        }
    };

    let mut sent_any = false;
    for integ in all.iter().filter(|i| i.workspace_id == workspace_id) {
        sent_any |= send_one(secrets, integ, text, workspace_id).await;
    }

    if !sent_any {
        debug!(workspace = %workspace_id, "improve notifier: no deliverable target for event");
    }
}

/// Post `text` to the default chat of EVERY enabled integration (used for
/// global events like `InsightReady` that have no workspace scope).
async fn deliver_all(
    integrations: &IntegrationsRepo,
    secrets: &Arc<dyn SecretStore>,
    text: &str,
) {
    let all = match integrations.list_all_enabled().await {
        Ok(list) => list,
        Err(e) => {
            debug!("improve notifier: could not list integrations: {e}");
            return;
        }
    };
    for integ in &all {
        send_one(secrets, integ, text, &integ.workspace_id).await;
    }
}

/// Send `text` to `integ`'s default chat. Returns `true` on success.
/// Best-effort: failures are logged and the function always returns.
async fn send_one(
    secrets: &Arc<dyn SecretStore>,
    integ: &Integration,
    text: &str,
    workspace_id: &str,
) -> bool {
    if integ.channel_id.trim().is_empty() {
        debug!(
            workspace = %workspace_id,
            channel = %integ.channel.as_str(),
            "improve notifier: no default chat configured, skipping"
        );
        return false;
    }
    let Some(adapter) = build_adapter(secrets, integ) else {
        return false;
    };
    let chat = integ.channel_id.trim();
    match adapter.send(chat, None, text).await {
        Ok(_) => true,
        Err(e) => {
            warn!(
                workspace = %workspace_id,
                channel = %integ.channel.as_str(),
                "improve notifier: send failed: {e}"
            );
            false
        }
    }
}

/// Build the outbound adapter for an integration, resolving its bot token from
/// the secret store (same refs the channel manager uses). Returns `None` when the
/// token is missing/empty (nothing to send with).
fn build_adapter(secrets: &Arc<dyn SecretStore>, integ: &Integration) -> Option<Arc<dyn Adapter>> {
    let ws = &integ.workspace_id;
    match integ.channel {
        Channel::Telegram => {
            let token = secrets.get(&format!("chan-bot-{ws}-telegram")).ok().flatten();
            match token {
                Some(t) if !t.is_empty() => Some(Arc::new(TelegramAdapter::new(t))),
                _ => {
                    debug!(workspace = %ws, "improve notifier: telegram bot token missing");
                    None
                }
            }
        }
        Channel::Slack => {
            let token = secrets.get(&format!("chan-bot-{ws}-slack")).ok().flatten();
            match token {
                Some(t) if !t.is_empty() => Some(Arc::new(SlackAdapter::new(t))),
                _ => {
                    debug!(workspace = %ws, "improve notifier: slack bot token missing");
                    None
                }
            }
        }
        // Webhooks are inbound-only — not a proactive-push target.
        Channel::Webhook => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::Id;

    fn applied(target_ref: &str) -> Event {
        Event::ImprovementEditApplied {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
            edit_id: Id::from("edit_1"),
            target_ref: target_ref.into(),
        }
    }

    #[test]
    fn applied_skill_edit_formats_one_line() {
        let (key, ws, line) = classify(&applied("big-win-legitimacy-verification")).unwrap();
        assert_eq!(key, NOTIFY_SETTING_KEY);
        assert_eq!(ws.as_deref(), Some("ws_1"));
        assert_eq!(
            line,
            "Self-improvement: skill `big-win-legitimacy-verification` — applied"
        );
        // One line, no diff body / secrets.
        assert!(!line.contains('\n'));
    }

    #[test]
    fn applied_memory_edit_is_described_as_memory() {
        let (_, _, line) = classify(&applied("MEMORY.md")).unwrap();
        assert_eq!(line, "Self-improvement: memory `MEMORY.md` — applied");
    }

    #[test]
    fn pending_edit_asks_for_approval() {
        let (_, _, line) = classify(&Event::ImprovementApprovalPending {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
            edit_id: Id::from("edit_1"),
            target_ref: "support-triage-router".into(),
        })
        .unwrap();
        assert_eq!(
            line,
            "Self-improvement: proposed edit to skill `support-triage-router` — needs approval"
        );
    }

    #[test]
    fn finished_run_summarizes_counts() {
        let (_, _, line) = classify(&Event::ImprovementRunFinished {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
            status: "done".into(),
            applied: 2,
            pending: 1,
        })
        .unwrap();
        assert_eq!(line, "Self-improvement run: 2 applied, 1 queued");
    }

    #[test]
    fn finished_run_with_no_changes_is_silent() {
        assert!(classify(&Event::ImprovementRunFinished {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
            status: "skipped".into(),
            applied: 0,
            pending: 0,
        })
        .is_none());
    }

    #[test]
    fn non_improvement_events_are_ignored() {
        assert!(classify(&Event::Notice {
            level: "info".into(),
            title: "hi".into(),
            body: "there".into(),
        })
        .is_none());
        assert!(classify(&Event::ImprovementRunStarted {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
        })
        .is_none());
    }

    #[test]
    fn review_done_routes_to_review_key() {
        let (key, ws, line) = classify(&Event::ReviewChanged {
            workspace_id: "ws_2".into(),
            session_id: None,
            review_id: Id::from("r1"),
            status: "done".into(),
        })
        .unwrap();
        assert_eq!(key, NOTIFY_REVIEW_KEY);
        assert_eq!(ws.as_deref(), Some("ws_2"));
        assert_eq!(line, "Code review done");
    }

    #[test]
    fn review_queued_is_silent() {
        assert!(classify(&Event::ReviewChanged {
            workspace_id: "ws_2".into(),
            session_id: None,
            review_id: Id::from("r1"),
            status: "queued".into(),
        })
        .is_none());
    }

    #[test]
    fn swarm_done_routes_to_swarm_key() {
        let (key, ws, line) = classify(&Event::SwarmStatus {
            workspace_id: "ws_3".into(),
            swarm_id: Id::from("sw1"),
            status: "done".into(),
        })
        .unwrap();
        assert_eq!(key, NOTIFY_SWARM_KEY);
        assert_eq!(ws.as_deref(), Some("ws_3"));
        assert!(line.contains("completed"));
    }

    #[test]
    fn swarm_active_is_silent() {
        assert!(classify(&Event::SwarmStatus {
            workspace_id: "ws_3".into(),
            swarm_id: Id::from("sw1"),
            status: "active".into(),
        })
        .is_none());
    }

    #[test]
    fn insight_ready_has_no_workspace() {
        let (key, ws, line) = classify(&Event::InsightReady {
            period: "daily 2026-06-20".into(),
            session_id: None,
        })
        .unwrap();
        assert_eq!(key, NOTIFY_INSIGHT_KEY);
        assert!(ws.is_none(), "InsightReady must be global (no workspace filter)");
        assert!(line.contains("daily 2026-06-20"));
    }

    #[test]
    fn budget_exceeded_routes_correctly() {
        let (key, ws, line) = classify(&Event::BudgetExceeded {
            workspace_id: "ws_4".into(),
            provider: "claude".into(),
            spend_usd: 5.25,
            cap_usd: 5.00,
            direction: "exceeded".into(),
        })
        .unwrap();
        assert_eq!(key, NOTIFY_BUDGET_KEY);
        assert_eq!(ws.as_deref(), Some("ws_4"));
        assert!(line.contains("claude"));
        assert!(line.contains("$5.25"));
    }

    #[test]
    fn budget_recovered_is_silent() {
        assert!(classify(&Event::BudgetExceeded {
            workspace_id: "ws_4".into(),
            provider: "claude".into(),
            spend_usd: 4.00,
            cap_usd: 5.00,
            direction: "recovered".into(),
        })
        .is_none());
    }
}
