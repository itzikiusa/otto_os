//! Webhook adapter: delivers an agent's reply back to an external system by
//! HTTP `POST`, the outbound half of the inbound-webhook channel.
//!
//! The inbound half (turning an external `POST` into an agent session) is the
//! `Bridge` — reused verbatim. This adapter is what the `Mirror` drives to
//! deliver the result. Unlike Slack/Telegram, a webhook caller wants only the
//! final answer, not the streaming "🧠 working…" feed, so the activity-feed
//! verbs (`send`, `edit`, `typing`) are no-ops and only the final reply
//! (`send_formatted`) and file attachments (`upload`) are POSTed to the
//! configured callback URL.
//!
//! The callback URL is passed through [`otto_netguard::check_url`] before every
//! request so a key-holder can't turn Otto into an SSRF proxy (loopback /
//! private / cloud-metadata targets are refused). With no callback URL the
//! adapter is a pure no-op — the webhook becomes a fire-and-forget trigger.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::Engine as _;
use otto_core::domain::Channel;

use crate::adapter::Adapter;

/// Resolve the reply callback URL for an inbound webhook call.
///
/// If the workspace owner configured a non-empty callback URL (`integ.channel_id`),
/// lock delivery to that URL so a caller cannot redirect replies to a different
/// host. Only when no admin callback URL is configured may `req.callback_url`
/// choose the callback destination.
pub fn resolve_callback_url(configured: &str, requested: Option<&str>) -> Option<String> {
    let admin = configured.trim();
    if !admin.is_empty() {
        return Some(admin.to_string());
    }
    requested
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .map(str::to_string)
}

/// Sliding window for failed webhook auth attempts.
pub const AUTH_FAIL_WINDOW: Duration = Duration::from_secs(60);
/// Failed attempts per `(workspace, client-ip)` within [`AUTH_FAIL_WINDOW`]
/// before returning 429.
pub const AUTH_FAIL_IP_LIMIT: usize = 10;
/// Aggregate failed attempts per `workspace` within [`AUTH_FAIL_WINDOW`] so
/// rotating spoofed `X-Forwarded-For` headers cannot bypass rate limiting.
pub const AUTH_FAIL_WS_LIMIT: usize = 30;
/// Cap on tracked buckets so spoofed headers cannot grow memory unboundedly.
pub const MAX_AUTH_BUCKETS: usize = 1024;

#[derive(Default)]
pub struct AuthRateLimiter {
    buckets: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl AuthRateLimiter {
    fn prune(q: &mut VecDeque<Instant>, now: Instant) {
        while let Some(&front) = q.front() {
            if now.saturating_duration_since(front) >= AUTH_FAIL_WINDOW {
                q.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn is_limited_at(&self, ws_id: &str, ip: &str, now: Instant) -> bool {
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        let ip_key = format!("ip:{ws_id}:{ip}");
        if let Some(q) = map.get_mut(&ip_key) {
            Self::prune(q, now);
            if q.len() >= AUTH_FAIL_IP_LIMIT {
                return true;
            }
        }
        let ws_key = format!("ws:{ws_id}");
        if let Some(q) = map.get_mut(&ws_key) {
            Self::prune(q, now);
            if q.len() >= AUTH_FAIL_WS_LIMIT {
                return true;
            }
        }
        // Fail closed when the bucket table is saturated with active failure
        // buckets so flooding distinct keys can never evict existing lockouts.
        if map.len() >= MAX_AUTH_BUCKETS {
            map.retain(|_, q| {
                Self::prune(q, now);
                !q.is_empty()
            });
            if map.len() >= MAX_AUTH_BUCKETS {
                return true;
            }
        }
        false
    }

    pub fn record_failure_at(&self, ws_id: &str, ip: &str, now: Instant) {
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= MAX_AUTH_BUCKETS {
            map.retain(|_, q| {
                Self::prune(q, now);
                !q.is_empty()
            });
        }
        for (key, cap) in [
            (format!("ip:{ws_id}:{ip}"), AUTH_FAIL_IP_LIMIT),
            (format!("ws:{ws_id}"), AUTH_FAIL_WS_LIMIT),
        ] {
            if let Some(q) = map.get_mut(&key) {
                Self::prune(q, now);
                if q.len() < cap {
                    q.push_back(now);
                }
            } else if map.len() < MAX_AUTH_BUCKETS {
                let mut q = VecDeque::new();
                q.push_back(now);
                map.insert(key, q);
            }
        }
    }

    pub fn clear_ip(&self, ws_id: &str, ip: &str) {
        let mut map = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(&format!("ip:{ws_id}:{ip}"));
    }
}

/// Process-wide webhook auth rate limiter shared across requests.
pub fn auth_rate_limiter() -> &'static AuthRateLimiter {
    static LIMITER: OnceLock<AuthRateLimiter> = OnceLock::new();
    LIMITER.get_or_init(AuthRateLimiter::default)
}

/// How long a single callback POST may take before we give up.
const CALLBACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// An `Adapter` that POSTs the agent's reply to an external callback URL.
pub struct WebhookAdapter {
    /// Where replies are POSTed. `None` → every send is a no-op (the caller only
    /// wanted to trigger a session and isn't collecting the reply here).
    callback_url: Option<String>,
    http: reqwest::Client,
}

impl WebhookAdapter {
    /// Build an adapter that delivers replies to `callback_url` (if any).
    pub fn new(callback_url: Option<String>) -> Self {
        // SSRF guard end to end: the guarded resolver re-vets the address at
        // connect time (no DNS rebinding after `check_url`), and the guarded
        // redirect policy stops a 3xx bouncing the POST to an internal address.
        // One process-wide client (a new adapter per request clones it — a
        // cheap `Arc` bump sharing the connection pool).
        static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let http = CLIENT
            .get_or_init(|| {
                otto_netguard::guarded_client_builder()
                    .timeout(CALLBACK_TIMEOUT)
                    .build()
                    .unwrap_or_default()
            })
            .clone();
        Self {
            callback_url: callback_url.filter(|u| !u.trim().is_empty()),
            http,
        }
    }

    /// POST one JSON payload to the callback URL, after the SSRF check. Best
    /// effort: a missing callback URL is a silent no-op; a blocked/failed
    /// request surfaces as an `Err` the `Mirror` logs (never fatal).
    async fn post(&self, payload: serde_json::Value) -> anyhow::Result<()> {
        let Some(url) = self.callback_url.as_deref() else {
            return Ok(());
        };
        if let Err(reason) = otto_netguard::check_url(url).await {
            return Err(anyhow::anyhow!("webhook callback blocked: {reason}"));
        }
        self.http
            .post(url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl Adapter for WebhookAdapter {
    /// Activity-feed message — suppressed for webhooks (callers want the result,
    /// not the streaming feed). Returns an empty message id.
    async fn send(
        &self,
        _chat: &str,
        _thread: Option<&str>,
        _text: &str,
    ) -> anyhow::Result<String> {
        Ok(String::new())
    }

    /// The agent's final reply. `chat` is the conversation key (echoed so the
    /// caller can correlate); the destination is the configured callback URL.
    async fn send_formatted(
        &self,
        chat: &str,
        thread: Option<&str>,
        text: &str,
    ) -> anyhow::Result<String> {
        self.post(serde_json::json!({
            "kind": "reply",
            "conversation": chat,
            "thread": thread,
            "text": text,
        }))
        .await?;
        Ok(String::new())
    }

    /// A bridge-side failure (delivery / session creation): POSTed as
    /// `{kind:"error", conversation, thread, text}` so the caller learns its
    /// message never reached an agent.
    async fn send_notice(
        &self,
        chat: &str,
        thread: Option<&str>,
        text: &str,
    ) -> anyhow::Result<String> {
        self.post(serde_json::json!({
            "kind": "error",
            "conversation": chat,
            "thread": thread,
            "text": text,
        }))
        .await?;
        Ok(String::new())
    }

    /// Activity-feed edit — suppressed (see `send`).
    async fn edit(&self, _chat: &str, _message_id: &str, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn channel(&self) -> Channel {
        Channel::Webhook
    }

    /// A file the agent attached (e.g. the full `investigation.md` for a long
    /// reply). POSTed base64-encoded so binary attachments survive intact.
    async fn upload(
        &self,
        chat: &str,
        thread: Option<&str>,
        filename: &str,
        content: &[u8],
    ) -> anyhow::Result<()> {
        let b64 = base64::engine::general_purpose::STANDARD.encode(content);
        self.post(serde_json::json!({
            "kind": "file",
            "conversation": chat,
            "thread": thread,
            "filename": filename,
            "content_base64": b64,
        }))
        .await
    }

    // `typing` keeps the trait default (no-op) — no progress pings for webhooks.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_is_webhook() {
        assert_eq!(WebhookAdapter::new(None).channel(), Channel::Webhook);
    }

    #[test]
    fn blank_callback_url_is_treated_as_none() {
        // Whitespace-only URL collapses to None so we don't try to POST to "".
        let a = WebhookAdapter::new(Some("   ".to_string()));
        assert!(a.callback_url.is_none());
    }

    #[tokio::test]
    async fn no_callback_url_sends_are_noops() {
        let a = WebhookAdapter::new(None);
        // All verbs succeed without any network call.
        assert_eq!(a.send("c", None, "x").await.unwrap(), "");
        assert_eq!(a.send_formatted("c", None, "x").await.unwrap(), "");
        a.edit("c", "1", "x").await.unwrap();
        a.upload("c", None, "f.md", b"data").await.unwrap();
    }

    #[tokio::test]
    async fn ssrf_guard_blocks_loopback_callback() {
        // A key-holder must not be able to aim the callback at internal hosts.
        let a = WebhookAdapter::new(Some("http://127.0.0.1/collect".to_string()));
        let err = a.send_formatted("c", None, "hi").await.unwrap_err();
        assert!(err.to_string().contains("blocked"), "got: {err}");
        // A failure notice is POSTed too (S5-19: `send` is a no-op, so the
        // bridge's notices used to vanish) — here it reaches the same guard.
        let err = a
            .send_notice("c", None, "couldn't deliver")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("blocked"), "got: {err}");
    }

    #[test]
    fn callback_url_is_locked_when_admin_configured() {
        // Admin-configured URL always wins over caller-supplied callback_url.
        assert_eq!(
            resolve_callback_url(
                "https://hooks.internal.example/cb",
                Some("https://evil.example/steal"),
            )
            .as_deref(),
            Some("https://hooks.internal.example/cb")
        );
        // When no admin URL is configured, caller may supply one (or omit it).
        assert_eq!(
            resolve_callback_url("  ", Some("https://caller.example/cb")).as_deref(),
            Some("https://caller.example/cb")
        );
        assert_eq!(resolve_callback_url("", None), None);
        assert_eq!(resolve_callback_url("", Some("   ")), None);
    }

    #[test]
    fn auth_rate_limiter_blocks_after_limit_and_recovers_after_window() {
        let limiter = AuthRateLimiter::default();
        let t0 = Instant::now();

        for _ in 0..AUTH_FAIL_IP_LIMIT {
            assert!(!limiter.is_limited_at("ws1", "10.0.0.1", t0));
            limiter.record_failure_at("ws1", "10.0.0.1", t0);
        }
        assert!(limiter.is_limited_at("ws1", "10.0.0.1", t0));
        // Another IP in the same workspace is not blocked yet.
        assert!(!limiter.is_limited_at("ws1", "10.0.0.2", t0));
        // Another workspace with the same IP is isolated.
        assert!(!limiter.is_limited_at("ws2", "10.0.0.1", t0));

        // Window expiry lifts the block.
        assert!(!limiter.is_limited_at("ws1", "10.0.0.1", t0 + AUTH_FAIL_WINDOW));

        // Rotating spoofed IPs still trips the per-workspace aggregate limit.
        let t1 = t0 + Duration::from_secs(120);
        for i in 0..AUTH_FAIL_WS_LIMIT {
            let ip = format!("192.0.2.{i}");
            assert!(!limiter.is_limited_at("ws3", &ip, t1));
            limiter.record_failure_at("ws3", &ip, t1);
        }
        assert!(limiter.is_limited_at("ws3", "198.51.100.1", t1));
    }

    #[test]
    fn auth_rate_limiter_fails_closed_at_capacity_without_clearing_lockouts() {
        let limiter = AuthRateLimiter::default();
        let t0 = Instant::now();

        // Lock out a victim workspace + IP first.
        for _ in 0..AUTH_FAIL_IP_LIMIT {
            limiter.record_failure_at("ws-victim", "203.0.113.5", t0);
        }
        assert!(limiter.is_limited_at("ws-victim", "203.0.113.5", t0));

        // Flood more than MAX_AUTH_BUCKETS distinct (workspace, ip) pairs.
        for i in 0..(MAX_AUTH_BUCKETS + 64) {
            limiter.record_failure_at(&format!("ws-flood-{i}"), &format!("198.51.100.{i}"), t0);
        }

        // The original lockout MUST survive (never cleared) AND new callers
        // fail closed while the table is saturated.
        assert!(limiter.is_limited_at("ws-victim", "203.0.113.5", t0));
        assert!(limiter.is_limited_at("ws-new", "192.0.2.99", t0));

        // Once the window expires, pruning frees capacity and new callers pass again.
        assert!(!limiter.is_limited_at("ws-new", "192.0.2.99", t0 + AUTH_FAIL_WINDOW));
    }
}
