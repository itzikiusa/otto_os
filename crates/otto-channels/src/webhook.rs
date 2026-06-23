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

use base64::Engine as _;
use otto_core::domain::Channel;

use crate::adapter::Adapter;

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
        let http = reqwest::Client::builder()
            .timeout(CALLBACK_TIMEOUT)
            // Reuse the SSRF guard's redirect policy so a 3xx can't bounce the
            // POST to a blocked internal address.
            .redirect(otto_netguard::redirect_policy())
            .build()
            .unwrap_or_default();
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
    }
}
