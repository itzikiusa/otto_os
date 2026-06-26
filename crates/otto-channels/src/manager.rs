//! ChannelManager — starts per-integration listener tasks and keeps them in
//! sync with the stored config.
//!
//! `start()` spawns a supervisor task that scans all enabled integrations,
//! resolves each token from the secret store, and spawns the appropriate
//! adapter loop. Every ~15s it re-scans; when the enabled set changes (a
//! channel toggled / added / removed in the UI) it cancels the current
//! generation of adapters and respawns — so config edits apply without a
//! daemon restart. A top-level `cancel` flag stops everything on shutdown.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::domain::Channel;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_sessions::SessionManager;
use otto_state::{IntegrationsRepo, SettingsRepo, WorkspacesRepo};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bridge::Bridge;
use crate::mirror::Mirror;

const RESCAN_INTERVAL: Duration = Duration::from_secs(15);

type GenerationSignature = Vec<(String, String, String)>;

fn generation_signature(integrations: &[otto_core::domain::Integration]) -> GenerationSignature {
    let mut sig: GenerationSignature = integrations
        .iter()
        .map(|i| {
            (
                i.workspace_id.clone(),
                i.channel.as_str().to_string(),
                i.updated_at.to_rfc3339(),
            )
        })
        .collect();
    sig.sort();
    sig
}

/// Handle returned by `ChannelManager::start`. Keep it alive for the process
/// lifetime; dropping it sets the cancel flag and stops the supervisor.
pub struct ChannelHandle {
    cancel: Arc<AtomicBool>,
    _supervisor: JoinHandle<()>,
}

impl ChannelHandle {
    /// Signal the supervisor + all listener tasks to stop (best-effort).
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for ChannelHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Wires together the repos, secrets store and session manager to drive
/// channel integrations.
pub struct ChannelManager {
    pub manager: Arc<SessionManager>,
    pub workspaces: WorkspacesRepo,
    pub integrations: IntegrationsRepo,
    pub settings: SettingsRepo,
    pub secrets: Arc<dyn SecretStore>,
    pub root_user_id: String,
    /// Daemon event bus (the same one the WS subscribes to). The proactive
    /// self-improvement notifier subscribes to this; `None` disables it.
    pub events: Option<broadcast::Sender<Event>>,
    /// Optional hook: an inbound message on a swarm-bound channel launches that
    /// swarm instead of starting a normal session. Injected by otto-server.
    pub swarm_trigger: Option<Arc<dyn crate::swarm_trigger::SwarmTrigger>>,
    /// Optional hook: after a channel interaction finishes, run self-improvement
    /// on it and reply in-thread. Injected by otto-server (owns the engine).
    pub improver: Option<Arc<dyn crate::mirror::InteractionImprover>>,
}

impl ChannelManager {
    pub fn new(
        manager: Arc<SessionManager>,
        workspaces: WorkspacesRepo,
        integrations: IntegrationsRepo,
        settings: SettingsRepo,
        secrets: Arc<dyn SecretStore>,
        root_user_id: String,
        events: Option<broadcast::Sender<Event>>,
    ) -> Self {
        Self {
            manager,
            workspaces,
            integrations,
            settings,
            secrets,
            root_user_id,
            events,
            swarm_trigger: None,
            improver: None,
        }
    }

    /// Wire the swarm-launch hook (otto-server provides the implementation).
    pub fn with_swarm_trigger(mut self, trigger: Arc<dyn crate::swarm_trigger::SwarmTrigger>) -> Self {
        self.swarm_trigger = Some(trigger);
        self
    }

    /// Wire the self-improvement-on-interaction hook (otto-server provides the
    /// implementation; `None` leaves the mirror unchanged).
    pub fn with_improver(mut self, improver: Arc<dyn crate::mirror::InteractionImprover>) -> Self {
        self.improver = Some(improver);
        self
    }

    /// Start the supervisor. Returns immediately; adapters run in the
    /// background and stay in sync with the config until the handle is dropped.
    pub async fn start(self) -> ChannelHandle {
        let cancel = Arc::new(AtomicBool::new(false));

        // Spawn the proactive self-improvement notifier alongside the adapter
        // supervisor (opt-in, gated inside on `channels.notify_self_improvement`).
        // It shares the top-level cancel flag so it stops on shutdown.
        if let Some(events) = &self.events {
            crate::improve_notify::spawn(
                events.subscribe(),
                self.integrations.clone(),
                self.settings.clone(),
                Arc::clone(&self.secrets),
                Arc::clone(&cancel),
            );
            info!("channel manager: self-improvement notifier started (opt-in)");
        }

        let supervisor = tokio::spawn(self.supervise(Arc::clone(&cancel)));
        ChannelHandle {
            cancel,
            _supervisor: supervisor,
        }
    }

    /// Re-scan loop: (re)spawn adapters whenever the enabled set changes.
    async fn supervise(self, cancel: Arc<AtomicBool>) {
        // Shared mirror + bridge survive across generations so an in-flight
        // session keeps its channel mapping when adapters are respawned.
        let mirror = Mirror::new_with_improver(Arc::clone(&self.manager), self.improver.clone());
        let bridge = Bridge::new_with_swarm_trigger(
            Arc::clone(&self.manager),
            self.workspaces.clone(),
            self.settings.clone(),
            Arc::clone(&mirror),
            self.root_user_id.clone(),
            self.swarm_trigger.clone(),
        );

        let mut gen_cancel: Option<Arc<AtomicBool>> = None;
        let mut last_sig: Option<GenerationSignature> = None;

        loop {
            if cancel.load(Ordering::Relaxed) {
                if let Some(g) = &gen_cancel {
                    g.store(true, Ordering::Relaxed);
                }
                return;
            }

            let integrations = match self.integrations.list_all_enabled().await {
                Ok(list) => list,
                Err(e) => {
                    warn!("channel manager: failed to load integrations: {e}");
                    Vec::new()
                }
            };

            // Signature of the desired set: sorted (workspace, channel, updated_at)
            // tuples. Including updated_at makes token/config edits respawn a
            // listener even when the enabled channel set is unchanged.
            let sig = generation_signature(&integrations);

            if last_sig.as_ref() != Some(&sig) {
                // Stop the previous generation, then spawn a fresh one.
                if let Some(g) = gen_cancel.take() {
                    g.store(true, Ordering::Relaxed);
                }
                let g = Arc::new(AtomicBool::new(false));
                let count = self.spawn_generation(&integrations, &bridge, &g);
                info!("channel manager: {count} adapter(s) active");
                gen_cancel = Some(g);
                last_sig = Some(sig);
            }

            // Sleep in short slices so shutdown is responsive.
            let mut waited = Duration::ZERO;
            while waited < RESCAN_INTERVAL {
                if cancel.load(Ordering::Relaxed) {
                    if let Some(g) = &gen_cancel {
                        g.store(true, Ordering::Relaxed);
                    }
                    return;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                waited += Duration::from_millis(500);
            }
        }
    }

    /// Spawn one adapter task per enabled integration under `gen_cancel`.
    /// Returns how many were started.
    fn spawn_generation(
        &self,
        integrations: &[otto_core::domain::Integration],
        bridge: &Arc<Bridge>,
        gen_cancel: &Arc<AtomicBool>,
    ) -> usize {
        let mut count = 0;
        for integ in integrations {
            let integ = integ.clone();
            let ws_id = integ.workspace_id.clone();
            match integ.channel {
                Channel::Telegram => {
                    let bot_ref = format!("chan-bot-{}-telegram", ws_id);
                    let token = match self.secrets.get(&bot_ref) {
                        Ok(Some(t)) => t,
                        _ => {
                            warn!(workspace = %ws_id, "telegram: bot token missing, skipping");
                            continue;
                        }
                    };
                    info!(workspace = %ws_id, "starting Telegram listener");
                    count += 1;
                    let c = Arc::clone(gen_cancel);
                    let b = Arc::clone(bridge);
                    tokio::spawn(async move {
                        crate::telegram::run(integ, token, b, c).await;
                    });
                }
                Channel::Slack => {
                    let bot_token = match self.secrets.get(&format!("chan-bot-{}-slack", ws_id)) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            warn!(workspace = %ws_id, "slack: bot token missing, skipping");
                            continue;
                        }
                    };
                    let app_token = match self.secrets.get(&format!("chan-app-{}-slack", ws_id)) {
                        Ok(Some(t)) if !t.is_empty() => t,
                        _ => {
                            warn!(workspace = %ws_id, "slack: app token missing (needed for Socket Mode), skipping");
                            continue;
                        }
                    };
                    info!(workspace = %ws_id, "starting Slack Socket Mode listener");
                    count += 1;
                    let c = Arc::clone(gen_cancel);
                    let b = Arc::clone(bridge);
                    tokio::spawn(async move {
                        crate::slack::run(integ, bot_token, app_token, b, c).await;
                    });
                }
                // Webhooks are request-driven (the inbound HTTP route calls the
                // bridge directly), so the supervisor spawns no listener for them.
                Channel::Webhook => {}
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use otto_core::domain::Integration;

    fn integration(channel: Channel, updated_at: chrono::DateTime<Utc>) -> Integration {
        Integration {
            workspace_id: "ws_1".to_string(),
            channel,
            enabled: true,
            allowed_users: String::new(),
            agent_reply: true,
            reply_instructions: String::new(),
            channel_id: String::new(),
            preferred_cli: String::new(),
            has_bot_token: true,
            has_app_token: channel == Channel::Slack,
            updated_at,
        }
    }

    #[test]
    fn generation_signature_changes_when_integration_is_updated() {
        let old = vec![integration(
            Channel::Slack,
            Utc.with_ymd_and_hms(2026, 6, 13, 8, 0, 0).unwrap(),
        )];
        let new = vec![integration(
            Channel::Slack,
            Utc.with_ymd_and_hms(2026, 6, 13, 8, 1, 0).unwrap(),
        )];

        assert_ne!(generation_signature(&old), generation_signature(&new));
    }
}
