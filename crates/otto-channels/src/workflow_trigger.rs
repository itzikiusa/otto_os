//! Hook that lets a structured inbound channel message **start a workflow run**
//! instead of starting a normal session. otto-channels can't depend on
//! otto-server (where the workflow engine lives), so the `Bridge` holds an
//! `Arc<dyn WorkflowChatTrigger>` that otto-server implements and injects
//! (mirrors [`crate::swarm_trigger::SwarmTrigger`]).
//!
//! The recognized message shape (case-insensitive field labels) is:
//! ```text
//! Action: Workflow
//! Name: <workflow name>
//! Msg: please do x y z, follow all relevant rules
//! Jira ticket: GS-1111
//! Working Directory: ~/path
//! Relevant Info: ~/a, ~/b
//! Goals:
//!   - 100% test coverage
//!   - under 2 minutes runtime
//! ```

use async_trait::async_trait;

/// What the bridge should tell the channel after a workflow run was started.
#[derive(Debug, Clone)]
pub struct WorkflowChatAck {
    /// Message to post back to the originating chat.
    pub reply: String,
}

/// Implemented by otto-server: parse a structured `Action: Workflow` message,
/// resolve the workflow by name in the workspace, and start a run.
#[async_trait]
pub trait WorkflowChatTrigger: Send + Sync {
    /// Returns `Some(ack)` if the message started a workflow run (the bridge then
    /// replies + skips normal session creation), else `None` (normal routing).
    async fn try_start(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        user: &str,
        text: &str,
    ) -> Option<WorkflowChatAck>;
}
