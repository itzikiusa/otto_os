//! Transport abstraction: each messaging channel (Telegram, Slack, …) implements
//! `Adapter` so the mirror/bridge layer is channel-agnostic.

use otto_core::domain::Channel;

/// A message received from a channel that needs to be forwarded to an agent.
#[derive(Debug, Clone)]
pub struct Inbound {
    /// The Otto workspace this integration belongs to.
    pub workspace_id: String,
    /// Channel-native chat/conversation id (e.g. Telegram chat_id, Slack channel id).
    pub chat: String,
    /// Thread/topic id within the chat, if applicable.
    pub thread: Option<String>,
    /// Channel-native user identifier (numeric id string).
    pub user: String,
    /// The user's message text.
    pub text: String,
}

/// Sending/editing abstraction for a messaging channel.
#[async_trait::async_trait]
pub trait Adapter: Send + Sync {
    /// Post a new message to `chat` (optionally in `thread`).
    /// Returns the channel-native message id of the new message.
    async fn send(&self, chat: &str, thread: Option<&str>, text: &str) -> anyhow::Result<String>;

    /// Edit a previously sent message in-place (used for the rolling activity feed).
    async fn edit(&self, chat: &str, message_id: &str, text: &str) -> anyhow::Result<()>;

    /// Which channel this adapter is for.
    fn channel(&self) -> Channel;

    /// Upload a file attachment to the conversation.
    ///
    /// `content` is the file's raw bytes — passed verbatim so binary files
    /// (images, PDFs, …) are uploaded intact. Default implementation is a no-op
    /// (returns `Ok(())`).  Adapters that support file uploads override this.
    async fn upload(
        &self,
        _chat: &str,
        _thread: Option<&str>,
        _filename: &str,
        _content: &[u8],
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Send a "typing…" indicator to the chat.
    ///
    /// Default implementation is a no-op.  Telegram overrides this.
    async fn typing(&self, _chat: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
