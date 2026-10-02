//! Listener health — what each Slack / Telegram inbound listener is doing
//! right now, so the Settings → Channels page can say "Connected", "Reconnecting
//! (timed out)" or "App token rejected" instead of the user finding out from a
//! silent bot.
//!
//! Hard-won: after a reboot a Socket Mode dial could park on a network flap and
//! every workspace went silent for hours with nothing but a "connecting to
//! socket mode" log line — the daemon looked healthy, the UI said "Enabled".
//! The dial is bounded now (`slack::WS_CONNECT_TIMEOUT`), and this registry is
//! the other half: the state machine is visible without reading the log.
//!
//! Process-global and in-memory (it describes live tasks, not config). Each
//! listener spawn takes a fresh *generation*; an update carrying an older
//! generation is dropped, so a cancelled listener winding down can never
//! overwrite the state of the one that replaced it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use otto_core::domain::Channel;
use serde::Serialize;

/// Longest `detail` / `last_error` kept (chars) — a reason, not a log dump.
const MAX_DETAIL_CHARS: usize = 300;

/// Where a listener is in its connect / reconnect cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerState {
    /// A token can't be read yet (not saved, or the Keychain isn't available
    /// right after login). Retried on every manager rescan (~15 s).
    WaitingForToken,
    /// First connection attempt in flight.
    Connecting,
    /// Socket Mode said `hello` / the last Telegram poll succeeded.
    Connected,
    /// The connection dropped or an attempt failed; retrying with backoff.
    Reconnecting,
    /// The platform rejected the credentials (revoked / wrong token type /
    /// Socket Mode disabled). Still retried at the backoff ceiling, but it
    /// won't fix itself — the user has to change the token or the app.
    Failing,
    /// Not started: another enabled workspace already listens with this token.
    Conflict,
}

/// One listener's live status (`GET /workspaces/{id}/integrations/status`).
#[derive(Debug, Clone, Serialize)]
pub struct ListenerStatus {
    pub workspace_id: String,
    pub channel: Channel,
    pub state: ListenerState,
    /// Human-readable reason for the current state (an error, what's missing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// When the listener entered `state`.
    pub since: DateTime<Utc>,
    /// When the current connection came up (`None` while not connected).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<DateTime<Utc>>,
    /// Last inbound event the listener received (survives reconnects).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<DateTime<Utc>>,
    /// Consecutive failed attempts since the last good connection.
    pub failures: u32,
    #[serde(skip)]
    generation: u64,
}

type Key = (String, &'static str);

fn registry() -> &'static Mutex<HashMap<Key, ListenerStatus>> {
    static REG: OnceLock<Mutex<HashMap<Key, ListenerStatus>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_generation() -> u64 {
    static GEN: AtomicU64 = AtomicU64::new(1);
    GEN.fetch_add(1, Ordering::Relaxed)
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= MAX_DETAIL_CHARS {
        return s.to_string();
    }
    let mut out: String = s.chars().take(MAX_DETAIL_CHARS).collect();
    out.push('…');
    out
}

/// Start a new generation for `(ws, channel)` in `state`, keeping the last
/// event time from any previous entry. Returns the generation.
fn reset(ws: &str, channel: Channel, state: ListenerState, detail: Option<&str>) -> u64 {
    let generation = next_generation();
    let now = Utc::now();
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let key = (ws.to_string(), channel.as_str());
    let last_event_at = reg.get(&key).and_then(|s| s.last_event_at);
    reg.insert(
        key,
        ListenerStatus {
            workspace_id: ws.to_string(),
            channel,
            state,
            detail: detail.map(clip),
            since: now,
            connected_at: None,
            last_event_at,
            last_error: None,
            last_error_at: None,
            failures: 0,
            generation,
        },
    );
    generation
}

/// Record that an integration is enabled but its token isn't readable yet.
pub fn waiting_for_token(ws: &str, channel: Channel, what: &str) {
    reset(ws, channel, ListenerState::WaitingForToken, Some(what));
}

/// Record that an integration was not started because its token clashes.
pub fn conflict(ws: &str, channel: Channel, detail: &str) {
    reset(ws, channel, ListenerState::Conflict, Some(detail));
}

/// Drop the entry of `(ws, channel)` — an integration that was disabled or
/// removed has no listener left to describe.
pub fn remove(ws: &str, channel: Channel) {
    let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    reg.remove(&(ws.to_string(), channel.as_str()));
}

/// Every listener status of workspace `ws` (Slack before Telegram).
pub fn snapshot(ws: &str) -> Vec<ListenerStatus> {
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<ListenerStatus> = reg
        .values()
        .filter(|s| s.workspace_id == ws)
        .cloned()
        .collect();
    out.sort_by_key(|s| s.channel.as_str());
    out
}

/// A listener task's handle on its registry entry. Cheap to clone; every
/// update is a no-op once a newer generation owns the entry.
#[derive(Debug, Clone)]
pub struct Health {
    ws: String,
    channel: Channel,
    generation: u64,
}

impl Health {
    /// Begin a new listener generation in `Connecting`.
    pub fn begin(ws: &str, channel: Channel) -> Self {
        let generation = reset(ws, channel, ListenerState::Connecting, None);
        Self {
            ws: ws.to_string(),
            channel,
            generation,
        }
    }

    /// Apply `f` to this generation's entry (skipped when superseded).
    fn update(&self, f: impl FnOnce(&mut ListenerStatus)) {
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = reg.get_mut(&(self.ws.clone(), self.channel.as_str())) {
            if s.generation == self.generation {
                f(s);
            }
        }
    }

    fn enter(s: &mut ListenerStatus, state: ListenerState) {
        if s.state != state {
            s.state = state;
            s.since = Utc::now();
        }
    }

    /// The connection is up (Socket Mode `hello`, a good Telegram poll).
    pub fn connected(&self) {
        self.update(|s| {
            if s.state != ListenerState::Connected {
                s.connected_at = Some(Utc::now());
            }
            Self::enter(s, ListenerState::Connected);
            s.detail = None;
            s.failures = 0;
        });
    }

    /// An attempt failed or the connection dropped; the listener will retry.
    /// `permanent` = the platform rejected the credentials/config, which
    /// retrying won't fix (shown as `Failing`, not `Reconnecting`).
    pub fn failed(&self, reason: &str, permanent: bool) {
        let reason = clip(reason);
        self.update(|s| {
            let now = Utc::now();
            Self::enter(
                s,
                if permanent {
                    ListenerState::Failing
                } else {
                    ListenerState::Reconnecting
                },
            );
            s.connected_at = None;
            s.failures = s.failures.saturating_add(1);
            s.detail = Some(reason.clone());
            s.last_error = Some(reason);
            s.last_error_at = Some(now);
        });
    }

    /// The connection ended cleanly (server-requested refresh); reconnecting.
    pub fn reconnecting(&self, reason: &str) {
        let reason = clip(reason);
        self.update(|s| {
            Self::enter(s, ListenerState::Reconnecting);
            s.connected_at = None;
            s.detail = Some(reason);
        });
    }

    /// An inbound event arrived.
    pub fn event(&self) {
        self.update(|s| s.last_event_at = Some(Utc::now()));
    }

    #[cfg(test)]
    fn state(&self) -> Option<ListenerStatus> {
        registry()
            .lock()
            .unwrap()
            .get(&(self.ws.clone(), self.channel.as_str()))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The registry is process-global: every test uses its own workspace ids.

    #[test]
    fn a_listener_walks_connecting_connected_reconnecting() {
        let h = Health::begin("ws_health_walk", Channel::Slack);
        assert_eq!(h.state().unwrap().state, ListenerState::Connecting);

        h.failed("websocket connect timed out after 20s", false);
        let s = h.state().unwrap();
        assert_eq!(s.state, ListenerState::Reconnecting);
        assert_eq!(s.failures, 1);
        assert_eq!(
            s.last_error.as_deref(),
            Some("websocket connect timed out after 20s")
        );

        h.connected();
        let s = h.state().unwrap();
        assert_eq!(s.state, ListenerState::Connected);
        assert_eq!(s.failures, 0, "a good connection clears the failure run");
        assert!(s.connected_at.is_some());
        assert!(s.detail.is_none());
        // The last error stays visible after recovery (when it last broke).
        assert!(s.last_error.is_some());

        h.event();
        assert!(h.state().unwrap().last_event_at.is_some());

        h.failed("apps.connections.open: invalid_auth", true);
        let s = h.state().unwrap();
        assert_eq!(s.state, ListenerState::Failing);
        assert!(s.connected_at.is_none());
    }

    #[test]
    fn a_superseded_listener_cannot_overwrite_its_replacement() {
        let old = Health::begin("ws_health_gen", Channel::Telegram);
        old.event();
        let new = Health::begin("ws_health_gen", Channel::Telegram);
        // The cancelled generation winds down and reports a failure…
        old.failed("stream ended", false);
        let s = new.state().unwrap();
        assert_eq!(s.state, ListenerState::Connecting, "old gen ignored");
        // …while the last-event time carried over to the new generation.
        assert!(s.last_event_at.is_some());
        new.connected();
        assert_eq!(new.state().unwrap().state, ListenerState::Connected);
    }

    #[test]
    fn waiting_conflict_snapshot_and_remove() {
        waiting_for_token("ws_health_snap", Channel::Slack, "app token missing");
        conflict(
            "ws_health_snap",
            Channel::Telegram,
            "bot token already used by workspace 'Other'",
        );
        let snap = snapshot("ws_health_snap");
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].channel, Channel::Slack);
        assert_eq!(snap[0].state, ListenerState::WaitingForToken);
        assert_eq!(snap[1].state, ListenerState::Conflict);
        let json = serde_json::to_value(&snap[0]).unwrap();
        assert_eq!(json["state"], "waiting_for_token");
        assert_eq!(json["channel"], "slack");
        assert!(
            json.get("generation").is_none(),
            "internal field not exposed"
        );

        // Telegram disabled → its entry goes; Slack stays.
        remove("ws_health_snap", Channel::Telegram);
        let snap = snapshot("ws_health_snap");
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].channel, Channel::Slack);
    }

    #[test]
    fn long_reasons_are_clipped() {
        let h = Health::begin("ws_health_clip", Channel::Slack);
        h.failed(&"x".repeat(1000), false);
        let s = h.state().unwrap();
        assert_eq!(s.detail.unwrap().chars().count(), MAX_DETAIL_CHARS + 1);
    }
}
