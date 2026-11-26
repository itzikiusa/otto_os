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
    },
    Upload {
        chat: String,
        thread: Option<String>,
        name: String,
        content: Vec<u8>,
    },
}

/// A channel that records instead of posting. Message ids are `m1`, `m2`, …
pub struct RecordingAdapter {
    channel: Channel,
    workspace_id: String,
    chat: String,
    thread: Option<String>,
    log: Mutex<Vec<Recorded>>,
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
        })
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
        Ok(self.send_inner(chat, thread, text, true))
    }

    async fn edit(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()> {
        self.push(Recorded::Edit {
            chat: chat.into(),
            message_id: message_id.into(),
            text: text.into(),
        });
        Ok(())
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
        self.push(Recorded::Upload {
            chat: chat.into(),
            thread: thread.map(str::to_owned),
            name: filename.into(),
            content: content.to_vec(),
        });
        Ok(())
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
}
