//! ChannelManager — starts per-integration listener tasks and keeps them in
//! sync with the stored config.
//!
//! `start()` spawns a supervisor task that scans all enabled integrations,
//! resolves each token from the secret store, and spawns the appropriate
//! adapter loop. Every ~15s it re-scans; when the enabled set changes (a
//! channel toggled / added / removed in the UI) it cancels the current
//! generation of adapters and respawns — so config edits apply without a
//! daemon restart. A top-level `cancel` flag stops everything on shutdown.
//!
//! An integration whose token can't be read yet (a Keychain read failing
//! right after login/boot, or a token not saved yet) is kept PENDING and
//! retried on every rescan, without restarting the listeners that did start.
//! It used to be skipped for the whole generation — a failed read at daemon
//! start left that channel dead until the integration was edited.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::domain::Channel;
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_sessions::SessionManager;
use otto_state::{IntegrationsRepo, SettingsRepo, WorkspacesRepo};
use tokio::sync::{broadcast, Notify};
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

/// Tokens an inbound listener needs, or why it can't start yet.
#[derive(Debug, Clone, PartialEq)]
enum ListenerTokens {
    Telegram {
        token: String,
    },
    Slack {
        bot_token: String,
        app_token: String,
    },
    /// Request-driven channel (webhook): nothing to spawn.
    None,
}

/// Read one secret; a read error and "not saved" are both "not ready yet"
/// (retried on the next rescan), but logged differently. The `Err` is the
/// user-facing reason shown on the Channels page (`health::waiting_for_token`).
fn read_secret(
    secrets: &dyn SecretStore,
    key: &str,
    ws: &str,
    what: &str,
) -> Result<String, String> {
    match secrets.get(key) {
        Ok(Some(t)) if !t.is_empty() => Ok(t),
        Ok(_) => {
            warn!(workspace = %ws, "{what} missing — will retry on the next rescan");
            Err(format!("{what} is not saved"))
        }
        Err(e) => {
            warn!(workspace = %ws, "{what} could not be read ({e}) — will retry on the next rescan");
            Err(format!(
                "{what} could not be read from the Keychain yet — retrying every {}s",
                RESCAN_INTERVAL.as_secs()
            ))
        }
    }
}

/// Resolve an integration's listener tokens. `Err` = not ready yet (why).
fn resolve_tokens(
    secrets: &dyn SecretStore,
    integ: &otto_core::domain::Integration,
) -> Result<ListenerTokens, String> {
    let ws = integ.workspace_id.as_str();
    match integ.channel {
        Channel::Telegram => {
            let token = read_secret(
                secrets,
                &format!("chan-bot-{ws}-telegram"),
                ws,
                "Telegram bot token",
            )?;
            Ok(ListenerTokens::Telegram { token })
        }
        Channel::Slack => {
            let bot_token = read_secret(
                secrets,
                &format!("chan-bot-{ws}-slack"),
                ws,
                "Slack bot token (xoxb-…)",
            )?;
            let app_token = read_secret(
                secrets,
                &format!("chan-app-{ws}-slack"),
                ws,
                "Slack app token (xapp-…, needed for Socket Mode)",
            )?;
            Ok(ListenerTokens::Slack {
                bot_token,
                app_token,
            })
        }
        Channel::Webhook => Ok(ListenerTokens::None),
    }
}

/// [`resolve_tokens`] for every integration, off the runtime: Keychain reads
/// can block (a locked keychain, a prompt), so the whole batch runs as ONE
/// blocking-pool task instead of parking a tokio worker per secret.
async fn resolve_all_tokens(
    secrets: &Arc<dyn SecretStore>,
    integrations: &[otto_core::domain::Integration],
) -> Vec<Result<ListenerTokens, String>> {
    let secrets = Arc::clone(secrets);
    let list = integrations.to_vec();
    let n = list.len();
    tokio::task::spawn_blocking(move || {
        list.iter()
            .map(|i| resolve_tokens(secrets.as_ref(), i))
            .collect()
    })
    .await
    .unwrap_or_else(|e| vec![Err(format!("token read task failed: {e}")); n])
}

/// The `(workspace, channel)` keys of the listener-backed integrations in
/// `integrations` (webhooks have no listener, so no health entry).
fn listener_keys(integrations: &[otto_core::domain::Integration]) -> Vec<(String, Channel)> {
    integrations
        .iter()
        .filter(|i| i.channel != Channel::Webhook)
        .map(|i| (i.workspace_id.clone(), i.channel))
        .collect()
}

/// Handle returned by `ChannelManager::start`. Keep it alive for the process
/// lifetime; dropping it sets the cancel flag and stops the supervisor.
pub struct ChannelHandle {
    cancel: Arc<AtomicBool>,
    /// Wakes the sleeping supervisor on shutdown (`notify_one` keeps a permit,
    /// so a shutdown racing the flag check is never lost).
    wake: Arc<Notify>,
    _supervisor: JoinHandle<()>,
}

impl ChannelHandle {
    /// Signal the supervisor + all listener tasks to stop (best-effort).
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.wake.notify_one();
    }
}

impl Drop for ChannelHandle {
    fn drop(&mut self) {
        self.shutdown();
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
    admission: crate::admission::InboundAdmission,
    /// Daemon event bus (the same one the WS subscribes to). The proactive
    /// self-improvement notifier subscribes to this; `None` disables it.
    pub events: Option<broadcast::Sender<Event>>,
    /// Optional hook: an inbound message on a swarm-bound channel launches that
    /// swarm instead of starting a normal session. Injected by otto-server.
    pub swarm_trigger: Option<Arc<dyn crate::swarm_trigger::SwarmTrigger>>,
    /// Optional hook: after a channel interaction finishes, run self-improvement
    /// on it and reply in-thread. Injected by otto-server (owns the engine).
    pub improver: Option<Arc<dyn crate::mirror::InteractionImprover>>,
    /// Optional hook: an inbound `/run <ref>` (or `approve`/`reject` reply)
    /// launches/advances a Run with Otto run. Injected by otto-server.
    pub run_trigger: Option<Arc<dyn crate::run_trigger::RunTrigger>>,
    /// Optional hook: a structured `Action: Workflow` message starts a workflow
    /// run. Injected by otto-server (owns the workflow engine).
    pub workflow_trigger: Option<Arc<dyn crate::workflow_trigger::WorkflowChatTrigger>>,
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
            admission: Default::default(),
            events,
            swarm_trigger: None,
            improver: None,
            run_trigger: None,
            workflow_trigger: None,
        }
    }

    /// Share the daemon budget with the separately constructed webhook bridge.
    pub fn with_admission(mut self, admission: crate::admission::InboundAdmission) -> Self {
        self.admission = admission;
        self
    }

    /// Wire the Run with Otto launch/approval hook (otto-server provides it).
    pub fn with_run_trigger(mut self, trigger: Arc<dyn crate::run_trigger::RunTrigger>) -> Self {
        self.run_trigger = Some(trigger);
        self
    }

    /// Wire the workflow-command hook (otto-server provides the implementation).
    pub fn with_workflow_trigger(
        mut self,
        trigger: Arc<dyn crate::workflow_trigger::WorkflowChatTrigger>,
    ) -> Self {
        self.workflow_trigger = Some(trigger);
        self
    }

    /// Wire the swarm-launch hook (otto-server provides the implementation).
    pub fn with_swarm_trigger(
        mut self,
        trigger: Arc<dyn crate::swarm_trigger::SwarmTrigger>,
    ) -> Self {
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

        let wake = Arc::new(Notify::new());
        let supervisor = tokio::spawn(self.supervise(Arc::clone(&cancel), Arc::clone(&wake)));
        ChannelHandle {
            cancel,
            wake,
            _supervisor: supervisor,
        }
    }

    /// Re-scan loop: (re)spawn adapters whenever the enabled set changes.
    async fn supervise(self, cancel: Arc<AtomicBool>, wake: Arc<Notify>) {
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
            self.run_trigger.clone(),
            self.workflow_trigger.clone(),
            self.admission.clone(),
        );

        let mut gen_cancel: Option<Arc<AtomicBool>> = None;
        let mut last_sig: Option<GenerationSignature> = None;
        // Integrations of the current generation still waiting for a token,
        // and the tokens already listening in it (see `spawn_generation`).
        let mut pending: Vec<otto_core::domain::Integration> = Vec::new();
        let mut listening: HashSet<String> = HashSet::new();
        // Listener keys of the running generation, so a disabled/removed
        // integration's health entry is dropped on the next generation.
        let mut live_keys: Vec<(String, Channel)> = Vec::new();

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
                    warn!("channel manager: failed to load integrations; retaining current listeners: {e}");
                    // A failed read is not an explicit empty configuration.
                    // Preserve healthy listeners and retry on the usual timer.
                    tokio::select! {
                        _ = tokio::time::sleep(RESCAN_INTERVAL) => {}
                        _ = wake.notified() => {}
                    }
                    continue;
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
                listening.clear();
                let keys = listener_keys(&integrations);
                for (ws, ch) in live_keys.iter().filter(|k| !keys.contains(k)) {
                    crate::health::remove(ws, *ch);
                }
                live_keys = keys;
                let tokens = resolve_all_tokens(&self.secrets, &integrations).await;
                let (count, waiting) =
                    self.spawn_generation(&integrations, tokens, &bridge, &g, &mut listening);
                info!("channel manager: {count} adapter(s) active");
                pending = waiting;
                gen_cancel = Some(g);
                last_sig = Some(sig);
            } else if !pending.is_empty() {
                // Same config: start only what was waiting for its token,
                // under the running generation (no restart of live listeners).
                if let Some(g) = gen_cancel.clone() {
                    let retry = std::mem::take(&mut pending);
                    let tokens = resolve_all_tokens(&self.secrets, &retry).await;
                    let (count, waiting) =
                        self.spawn_generation(&retry, tokens, &bridge, &g, &mut listening);
                    if count > 0 {
                        info!("channel manager: {count} waiting adapter(s) started");
                    }
                    pending = waiting;
                }
            }

            // One timer per rescan; shutdown wakes it (was 500 ms slices).
            // The loop head then stops the adapters and returns.
            tokio::select! {
                _ = tokio::time::sleep(RESCAN_INTERVAL) => {}
                _ = wake.notified() => {}
            }
        }
    }

    /// Spawn one adapter task per enabled integration under `gen_cancel`.
    /// Returns how many were started, and the integrations that could not
    /// start yet (token not readable) — the supervisor retries those.
    ///
    /// `listening` holds the inbound tokens already started in this
    /// generation. The upsert API refuses to enable a second integration on
    /// the same token, but state from before that check (or a hand-edited DB)
    /// can still hold two — running both would split every bot's events
    /// randomly between the workspaces, so only the first (by workspace id)
    /// listens.
    fn spawn_generation(
        &self,
        integrations: &[otto_core::domain::Integration],
        resolved: Vec<Result<ListenerTokens, String>>,
        bridge: &Arc<Bridge>,
        gen_cancel: &Arc<AtomicBool>,
        listening: &mut HashSet<String>,
    ) -> (usize, Vec<otto_core::domain::Integration>) {
        let mut count = 0;
        let mut waiting = Vec::new();
        for (integ, tokens) in integrations.iter().zip(resolved) {
            let ws_id = integ.workspace_id.clone();
            let tokens = match tokens {
                Ok(t) => t,
                Err(why) => {
                    crate::health::waiting_for_token(&ws_id, integ.channel, &why);
                    waiting.push(integ.clone());
                    continue;
                }
            };
            let integ = integ.clone();
            if crate::bridge::open_to_everyone(&integ) {
                // Loud on every start: anyone who can message this bot drives
                // an agent session as the owner (review S5-02). Kept only for
                // integrations configured before the allow-list was required.
                warn!(
                    workspace = %ws_id,
                    channel = integ.channel.as_str(),
                    "channel OPEN TO EVERYONE: blank allowed_users with open_to_all — any sender \
                     can drive an agent session; set an allow-list in Settings → Channels"
                );
            }
            match tokens {
                ListenerTokens::Telegram { token } => {
                    if !listening.insert(token.clone()) {
                        warn!(
                            workspace = %ws_id,
                            "telegram: bot token already polled by another enabled workspace, skipping (disable one of them)"
                        );
                        crate::health::conflict(
                            &ws_id,
                            Channel::Telegram,
                            "This bot token is already polled by another enabled workspace — disable one of them.",
                        );
                        continue;
                    }
                    info!(workspace = %ws_id, "starting Telegram listener");
                    count += 1;
                    let c = Arc::clone(gen_cancel);
                    let b = Arc::clone(bridge);
                    let h = crate::health::Health::begin(&ws_id, Channel::Telegram);
                    tokio::spawn(async move {
                        crate::telegram::run(integ, token, b, c, h).await;
                    });
                }
                ListenerTokens::Slack {
                    bot_token,
                    app_token,
                } => {
                    if !listening.insert(app_token.clone()) {
                        warn!(
                            workspace = %ws_id,
                            "slack: app token already connected for another enabled workspace, skipping (Slack would split events between them; disable one)"
                        );
                        crate::health::conflict(
                            &ws_id,
                            Channel::Slack,
                            "This Slack app token is already connected for another enabled workspace — Slack would split messages between them. Disable one of them.",
                        );
                        continue;
                    }
                    info!(workspace = %ws_id, "starting Slack Socket Mode listener");
                    count += 1;
                    let c = Arc::clone(gen_cancel);
                    let b = Arc::clone(bridge);
                    let h = crate::health::Health::begin(&ws_id, Channel::Slack);
                    tokio::spawn(async move {
                        crate::slack::run(integ, bot_token, app_token, b, c, h).await;
                    });
                }
                // Webhooks are request-driven (the inbound HTTP route calls the
                // bridge directly), so the supervisor spawns no listener for them.
                ListenerTokens::None => {}
            }
        }
        (count, waiting)
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
            open_to_all: false,
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

    /// Fails every read until `ready` is set, then serves the tokens — a
    /// Keychain that isn't available yet right after login/boot.
    struct FlakyStore {
        ready: AtomicBool,
    }
    impl SecretStore for FlakyStore {
        fn get(&self, key: &str) -> otto_core::Result<Option<String>> {
            if !self.ready.load(Ordering::Relaxed) {
                return Err(otto_core::Error::Internal("keychain not available".into()));
            }
            Ok(Some(format!("tok-{key}")))
        }
        fn put(&self, _key: &str, _value: &str) -> otto_core::Result<()> {
            Ok(())
        }
        fn delete(&self, _key: &str) -> otto_core::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_unreadable_token_is_not_ready_then_resolves_once_readable() {
        let store = FlakyStore {
            ready: AtomicBool::new(false),
        };
        let slack = integration(
            Channel::Slack,
            Utc.with_ymd_and_hms(2026, 9, 29, 8, 0, 0).unwrap(),
        );
        // First scan (daemon start, Keychain not ready): not ready → pending,
        // with a reason the Channels page can show.
        let why = resolve_tokens(&store, &slack).unwrap_err();
        assert!(why.contains("Keychain"), "{why}");
        // A later rescan: the same integration now resolves.
        store.ready.store(true, Ordering::Relaxed);
        assert_eq!(
            resolve_tokens(&store, &slack),
            Ok(ListenerTokens::Slack {
                bot_token: "tok-chan-bot-ws_1-slack".into(),
                app_token: "tok-chan-app-ws_1-slack".into(),
            })
        );
        let hook = integration(
            Channel::Webhook,
            Utc.with_ymd_and_hms(2026, 9, 29, 8, 0, 0).unwrap(),
        );
        assert_eq!(resolve_tokens(&store, &hook), Ok(ListenerTokens::None));
        assert_eq!(
            listener_keys(&[slack.clone(), hook]),
            vec![("ws_1".to_string(), Channel::Slack)],
            "webhooks have no listener health"
        );
    }
}
