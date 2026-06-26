//! Hook that lets an inbound channel message **launch an agent swarm** instead of
//! starting a normal session. otto-channels can't depend on otto-server (where the
//! swarm runtime lives), so the `Bridge` holds an `Arc<dyn SwarmTrigger>` that
//! otto-server implements and injects (mirrors the `PreSpawnHook` boundary).
//!
//! In `Bridge::handle`, after the workspace is resolved and before a session is
//! created, the bridge calls `try_launch`. If it returns `Some(ack)`, the bridge
//! posts `ack.reply` back to the channel and stops — the swarm takes over.

use async_trait::async_trait;

/// What the bridge should tell the channel after a swarm was launched.
#[derive(Debug, Clone)]
pub struct LaunchAck {
    /// Message to post back to the originating chat (the launch acknowledgement).
    pub reply: String,
}

/// Implemented by otto-server: decide whether an inbound message matches a
/// configured swarm trigger and, if so, launch that swarm.
#[async_trait]
pub trait SwarmTrigger: Send + Sync {
    /// Returns `Some(ack)` if the message launched a swarm (the bridge then
    /// replies + skips normal session creation), else `None` (normal routing).
    async fn try_launch(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        user: &str,
        text: &str,
    ) -> Option<LaunchAck>;
}
