//! Hook that lets an inbound channel message **launch (or approve) a Run with
//! Otto run**. Like [`SwarmTrigger`](crate::swarm_trigger), otto-channels can't
//! depend on otto-server (where the run engine lives), so the `Bridge` holds an
//! `Arc<dyn RunTrigger>` that otto-server implements and injects.
//!
//! In `Bridge::handle`, before a normal session is created, the bridge calls
//! [`RunTrigger::handle`]. It returns `Some(ack)` when the message either launched
//! a run (`/run <ref>` or "run with otto …") OR resolved an awaiting run's
//! approval gate (`approve` / `reject` reply in the run's thread); the bridge then
//! posts `ack.reply` and stops.

use async_trait::async_trait;

/// What the bridge should post back after the trigger handled a message.
#[derive(Debug, Clone)]
pub struct RunAck {
    pub reply: String,
}

#[async_trait]
pub trait RunTrigger: Send + Sync {
    /// Returns `Some(ack)` if the message launched a run or resolved an approval
    /// (the bridge replies + skips normal session routing), else `None`.
    async fn handle(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        user: &str,
        text: &str,
    ) -> Option<RunAck>;
}
