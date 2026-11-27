//! Test-only recording channel adapter (feature `test-util`).
//!
//! [`RecordingAdapter`] stands at the [`Adapter`] boundary: it records every
//! send, edit (with the message id it targets) and upload (name and bytes) in
//! order, and builds the [`Inbound`] a named user would send in its thread.
//! [`RecordingAdapter::factory`] is the [`OutboundAdapterFactory`] a test
//! daemon posts its workflow chat through (progress lines, final result,
//! `channel_notify`, queue announcements); inbound goes through the real
//! `Bridge` with this adapter as the channel.

use std::sync::{Arc, Mutex};

use otto_core::domain::{Channel, Integration};

use crate::adapter::{Adapter, Inbound};
use crate::improve_notify::OutboundAdapterFactory;

/// One thing the engine did on the channel, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    Send {
        id: String,
        chat: String,
        thread: Option<String>,
        text: String,
        formatted: bool,
    },
    Edit {
        chat: String,
        message_id: String,
        text: String,
        /// `edit_formatted` (GFM) rather than `edit`.
        formatted: bool,
    },
    Upload {
        chat: String,
        thread: Option<String>,
        name: String,
        content: Vec<u8>,
        /// `upload_with_caption`'s caption; `None` for a plain `upload`.
        caption: Option<String>,
    },
}

/// A channel that records instead of posting. Message ids are `m1`, `m2`, …
pub struct RecordingAdapter {
    channel: Channel,
    workspace_id: String,
    chat: String,
    thread: Option<String>,
    log: Mutex<Vec<Recorded>>,
    /// Errors the next edits / uploads fail with, oldest first
    /// ([`Self::fail_next_edit`], [`Self::fail_next_upload`]).
    edit_errors: Mutex<std::collections::VecDeque<String>>,
    upload_errors: Mutex<std::collections::VecDeque<String>>,
    /// Delay before `send_formatted` records — lets tests simulate chat API
    /// latency on progress-pump posts without sleeping in production code.
    formatted_delay: Mutex<std::time::Duration>,
}

impl RecordingAdapter {
    pub fn new(
        channel: Channel,
        workspace_id: &str,
        chat: &str,
        thread: Option<&str>,
    ) -> Arc<Self> {
        Arc::new(Self {
            channel,
            workspace_id: workspace_id.into(),
            chat: chat.into(),
            thread: thread.map(str::to_owned),
            log: Mutex::new(vec![]),
            edit_errors: Mutex::default(),
            upload_errors: Mutex::default(),
            formatted_delay: Mutex::new(std::time::Duration::ZERO),
        })
    }

    /// Delay every `send_formatted` call by `delay` before recording it.
    pub fn delay_formatted(&self, delay: std::time::Duration) {
        *self.formatted_delay.lock().unwrap() = delay;
    }

    /// The next edit fails with `error` (e.g. Slack's
    /// `slack chat.update: message_not_found`) and is not recorded.
    pub fn fail_next_edit(&self, error: &str) {
        self.edit_errors.lock().unwrap().push_back(error.into());
    }

    /// The next upload fails with `error` (e.g.
    /// `slack upload PUT: ratelimited (retry after 1s)`) and is not recorded.
    pub fn fail_next_upload(&self, error: &str) {
        self.upload_errors.lock().unwrap().push_back(error.into());
    }

    fn edit_inner(
        &self,
        chat: &str,
        message_id: &str,
        text: &str,
        formatted: bool,
    ) -> anyhow::Result<()> {
        if let Some(e) = self.edit_errors.lock().unwrap().pop_front() {
            anyhow::bail!(e);
        }
        self.push(Recorded::Edit {
            chat: chat.into(),
            message_id: message_id.into(),
            text: text.into(),
            formatted,
        });
        Ok(())
    }

    fn upload_inner(
        &self,
        chat: &str,
        thread: Option<&str>,
        name: &str,
        content: &[u8],
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        if let Some(e) = self.upload_errors.lock().unwrap().pop_front() {
            anyhow::bail!(e);
        }
        self.push(Recorded::Upload {
            chat: chat.into(),
            thread: thread.map(str::to_owned),
            name: name.into(),
            content: content.to_vec(),
            caption: caption.map(str::to_owned),
        });
        Ok(())
    }

    /// The message `user` would send in this adapter's chat and thread.
    pub fn inbound(&self, user: &str, text: &str) -> Inbound {
        Inbound {
            workspace_id: self.workspace_id.clone(),
            chat: self.chat.clone(),
            thread: self.thread.clone(),
            user: user.into(),
            user_name: None,
            text: text.into(),
            edited: false,
            attachments: vec![],
        }
    }

    /// Everything recorded so far, in order.
    pub fn recorded(&self) -> Vec<Recorded> {
        self.log.lock().unwrap().clone()
    }

    fn push(&self, r: Recorded) {
        self.log.lock().unwrap().push(r);
    }

    fn next_id(&self) -> String {
        let n = self
            .log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, Recorded::Send { .. }))
            .count();
        format!("m{}", n + 1)
    }

    fn send_inner(&self, chat: &str, thread: Option<&str>, text: &str, formatted: bool) -> String {
        let id = self.next_id();
        self.push(Recorded::Send {
            id: id.clone(),
            chat: chat.into(),
            thread: thread.map(str::to_owned),
            text: text.into(),
            formatted,
        });
        id
    }
}

#[async_trait::async_trait]
impl Adapter for RecordingAdapter {
    async fn send(&self, chat: &str, thread: Option<&str>, text: &str) -> anyhow::Result<String> {
        Ok(self.send_inner(chat, thread, text, false))
    }

    async fn send_formatted(
        &self,
        chat: &str,
        thread: Option<&str>,
        text: &str,
    ) -> anyhow::Result<String> {
        let delay = *self.formatted_delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        Ok(self.send_inner(chat, thread, text, true))
    }

    async fn edit(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()> {
        self.edit_inner(chat, message_id, text, false)
    }

    async fn edit_formatted(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()> {
        self.edit_inner(chat, message_id, text, true)
    }

    fn channel(&self) -> Channel {
        self.channel
    }

    async fn upload(
        &self,
        chat: &str,
        thread: Option<&str>,
        filename: &str,
        content: &[u8],
    ) -> anyhow::Result<()> {
        self.upload_inner(chat, thread, filename, content, None)
    }

    async fn upload_with_caption(
        &self,
        chat: &str,
        thread: Option<&str>,
        filename: &str,
        content: &[u8],
        caption: &str,
    ) -> anyhow::Result<()> {
        self.upload_inner(chat, thread, filename, content, Some(caption))
    }
}

/// Hands `adapter` out for every integration (see [`RecordingAdapter::factory`]).
struct RecordingFactory(Arc<RecordingAdapter>);

#[async_trait::async_trait]
impl OutboundAdapterFactory for RecordingFactory {
    async fn adapter(&self, _integ: &Integration) -> Option<Arc<dyn Adapter>> {
        Some(self.0.clone())
    }
}

impl RecordingAdapter {
    /// An [`OutboundAdapterFactory`] that returns this adapter: set it as a
    /// test daemon's `ServerCtx::outbound` and every outbound workflow
    /// message lands here.
    pub fn factory(self: &Arc<Self>) -> Arc<dyn OutboundAdapterFactory> {
        Arc::new(RecordingFactory(self.clone()))
    }
}

/// A Slack message event as the listener hands it to the bridge: the
/// [`Inbound`] it builds, with its attachments downloaded through the real
/// path (a `saved to:` line per file). `None` when there is nothing to act on.
pub async fn slack_inbound(
    event: &serde_json::Value,
    integ: &Integration,
    bot_token: &str,
) -> Option<Inbound> {
    let inbound = crate::slack::slack_inbound(event, integ);
    crate::slack::with_attachments(event, inbound, bot_token).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_in_order_and_is_what_the_factory_hands_out() {
        let rec = RecordingAdapter::new(Channel::Slack, "ws-rec", "chat-1", Some("t-1"));
        let integ = Integration {
            workspace_id: "ws-rec".into(),
            channel: Channel::Slack,
            enabled: true,
            allowed_users: String::new(),
            open_to_all: false,
            agent_reply: false,
            reply_instructions: String::new(),
            channel_id: "chat-1".into(),
            preferred_cli: String::new(),
            has_bot_token: true,
            has_app_token: false,
            updated_at: chrono::Utc::now(),
        };
        let a = rec.factory().adapter(&integ).await.expect("the recorder");
        let id = a
            .send_formatted("chat-1", Some("t-1"), "hello")
            .await
            .unwrap();
        a.edit("chat-1", &id, "hello again").await.unwrap();
        a.upload("chat-1", Some("t-1"), "step1.md", b"body")
            .await
            .unwrap();
        assert_eq!(a.send("chat-1", None, "plain").await.unwrap(), "m2");
        let log = rec.recorded();
        assert_eq!(log.len(), 4);
        assert!(
            matches!(&log[0], Recorded::Send { id, formatted: true, text, .. } if id == "m1" && text == "hello")
        );
        assert!(matches!(&log[1], Recorded::Edit { message_id, .. } if message_id == "m1"));
        assert!(
            matches!(&log[2], Recorded::Upload { name, content, .. } if name == "step1.md" && content == b"body")
        );
        let msg = rec.inbound("U1", "status");
        assert_eq!(
            (
                msg.workspace_id.as_str(),
                msg.chat.as_str(),
                msg.thread.as_deref(),
                msg.user.as_str()
            ),
            ("ws-rec", "chat-1", Some("t-1"), "U1")
        );
    }

    #[tokio::test]
    async fn records_formatted_edits_captions_and_injected_failures() {
        let rec = RecordingAdapter::new(Channel::Slack, "ws-rec2", "chat-1", Some("t-1"));
        let id = rec
            .send_formatted("chat-1", Some("t-1"), "a")
            .await
            .unwrap();
        rec.fail_next_edit("slack chat.update: message_not_found");
        let lost = rec.edit_formatted("chat-1", &id, "b").await.unwrap_err();
        assert!(crate::adapter::edit_needs_new_post(&lost));
        rec.edit_formatted("chat-1", &id, "c").await.unwrap();
        rec.fail_next_upload("slack upload PUT: ratelimited (retry after 1s)");
        assert!(rec
            .upload_with_caption("chat-1", Some("t-1"), "s.md", b"x", "✅ S · 1s")
            .await
            .is_err());
        rec.upload_with_caption("chat-1", Some("t-1"), "s.md", b"x", "✅ S · 1s")
            .await
            .unwrap();
        let log = rec.recorded();
        assert_eq!(log.len(), 3, "failed calls are not recorded: {log:?}");
        assert!(matches!(&log[1], Recorded::Edit { text, formatted: true, .. } if text == "c"));
        assert!(matches!(&log[2], Recorded::Upload { caption: Some(c), .. } if c == "✅ S · 1s"));
    }
}
