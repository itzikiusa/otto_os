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

        // Only the three self-improvement events carry a notification; everything
        // else is ignored cheaply before any I/O.
        let Some((workspace_id, text)) = render(&event) else {
            continue;
        };

        // Opt-in flag, re-read per event so a live toggle takes effect at once.
        if !notify_enabled(&settings).await {
            continue;
        }

        deliver(&integrations, &secrets, &workspace_id, &text).await;
    }
}

/// Read the opt-in flag from the existing settings store. Missing / non-bool /
/// read error all resolve to `false` (off by default).
async fn notify_enabled(settings: &SettingsRepo) -> bool {
    matches!(
        settings.get(NOTIFY_SETTING_KEY).await,
        Ok(Some(serde_json::Value::Bool(true)))
    )
}

/// Render a self-improvement event into `(workspace_id, line)`. Returns `None`
/// for any other event variant (and for run-finished runs that did nothing).
fn render(event: &Event) -> Option<(String, String)> {
    match event {
        Event::ImprovementEditApplied {
            workspace_id,
            target_ref,
            ..
        } => Some((
            workspace_id.clone(),
            format!("💾 Self-improvement: {} — applied", describe_target(target_ref)),
        )),
        Event::ImprovementApprovalPending {
            workspace_id,
            target_ref,
            ..
        } => Some((
            workspace_id.clone(),
            format!(
                "📝 Self-improvement: proposed edit to {} — needs approval",
                describe_target(target_ref)
            ),
        )),
        Event::ImprovementRunFinished {
            workspace_id,
            applied,
            pending,
            ..
        } => {
            // A run that neither applied nor queued anything isn't worth a ping.
            if *applied == 0 && *pending == 0 {
                return None;
            }
            Some((
                workspace_id.clone(),
                format!("🧠 Self-improvement run: {applied} applied, {pending} queued"),
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
        // No configured "home" chat → nothing we can safely post to.
        if integ.channel_id.trim().is_empty() {
            debug!(
                workspace = %workspace_id,
                channel = %integ.channel.as_str(),
                "improve notifier: no default chat configured, skipping"
            );
            continue;
        }
        let Some(adapter) = build_adapter(secrets, integ) else {
            continue;
        };
        let chat = integ.channel_id.trim();
        match adapter.send(chat, None, text).await {
            Ok(_) => sent_any = true,
            Err(e) => warn!(
                workspace = %workspace_id,
                channel = %integ.channel.as_str(),
                "improve notifier: send failed: {e}"
            ),
        }
    }

    if !sent_any {
        debug!(workspace = %workspace_id, "improve notifier: no deliverable target for event");
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
        let (ws, line) = render(&applied("big-win-legitimacy-verification")).unwrap();
        assert_eq!(ws, "ws_1");
        assert_eq!(
            line,
            "💾 Self-improvement: skill `big-win-legitimacy-verification` — applied"
        );
        // One line, no diff body / secrets.
        assert!(!line.contains('\n'));
    }

    #[test]
    fn applied_memory_edit_is_described_as_memory() {
        let (_, line) = render(&applied("MEMORY.md")).unwrap();
        assert_eq!(line, "💾 Self-improvement: memory `MEMORY.md` — applied");
    }

    #[test]
    fn pending_edit_asks_for_approval() {
        let (_, line) = render(&Event::ImprovementApprovalPending {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
            edit_id: Id::from("edit_1"),
            target_ref: "support-triage-router".into(),
        })
        .unwrap();
        assert_eq!(
            line,
            "📝 Self-improvement: proposed edit to skill `support-triage-router` — needs approval"
        );
    }

    #[test]
    fn finished_run_summarizes_counts() {
        let (_, line) = render(&Event::ImprovementRunFinished {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
            status: "done".into(),
            applied: 2,
            pending: 1,
        })
        .unwrap();
        assert_eq!(line, "🧠 Self-improvement run: 2 applied, 1 queued");
    }

    #[test]
    fn finished_run_with_no_changes_is_silent() {
        assert!(render(&Event::ImprovementRunFinished {
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
        assert!(render(&Event::Notice {
            level: "info".into(),
            title: "hi".into(),
            body: "there".into(),
        })
        .is_none());
        assert!(render(&Event::ImprovementRunStarted {
            workspace_id: "ws_1".into(),
            run_id: Id::from("run_1"),
        })
        .is_none());
    }
}
