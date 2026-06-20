//! Events broadcast on the daemon event bus and streamed to `/ws/events`.

use serde::{Deserialize, Serialize};

use crate::domain::{AgentTask, Notice, Session, SessionStatus, TrailEvent};
use crate::Id;

/// Daemon-wide event. Serialized as JSON with a `type` tag, one per WS message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A session's live status changed.
    SessionStatus {
        session_id: Id,
        workspace_id: Id,
        status: SessionStatus,
    },
    /// A session was created (by any client or by the orchestrator).
    SessionCreated { session: Session },
    /// A session's `meta` changed. Carries the full merged meta so clients can
    /// update their cached session in place (e.g. live handover-progress flags).
    SessionMetaUpdated {
        session_id: Id,
        workspace_id: Id,
        meta: serde_json::Value,
    },
    /// A session was removed.
    SessionRemoved { session_id: Id, workspace_id: Id },
    /// Free-form notice surfaced as a toast/notification.
    /// `level` is one of "info" | "warn" | "error".
    Notice {
        level: String,
        title: String,
        body: String,
    },
    /// A persisted notification was created (credential expiry, session event,
    /// …). The SPA appends it to the notification center and may raise a native
    /// OS notification for warn/error severities.
    ///
    /// `user_id` is the notice's owner: `None` = a global / system notice
    /// delivered to every authenticated client; `Some(id)` is delivered only to
    /// that user's WS connections (see `ws_events::allowed`).
    Notification {
        notice: Notice,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user_id: Option<Id>,
    },
    /// A self-reflection run started.
    ImprovementRunStarted { workspace_id: Id, run_id: Id },
    /// A self-reflection run finished. `status` is one of
    /// "done" | "skipped" | "failed".
    ImprovementRunFinished {
        workspace_id: Id,
        run_id: Id,
        status: String,
        applied: i64,
        pending: i64,
    },
    /// An edit was auto-applied to a skill/memory file.
    ImprovementEditApplied {
        workspace_id: Id,
        run_id: Id,
        edit_id: Id,
        target_ref: String,
    },
    /// An edit is awaiting human approval.
    ImprovementApprovalPending {
        workspace_id: Id,
        run_id: Id,
        edit_id: Id,
        target_ref: String,
    },
    /// A new entry was appended to a session's activity trail.
    TrailAppended {
        workspace_id: Id,
        session_id: Id,
        event: TrailEvent,
    },
    /// A session's task tracker changed; carries the full current task list.
    TasksUpdated {
        workspace_id: Id,
        session_id: Id,
        tasks: Vec<AgentTask>,
    },
    /// A swarm run was created or changed. `run` is the serialized SwarmRun row
    /// (otto-core can't depend on otto-state, so it travels as JSON).
    SwarmRunUpdated {
        workspace_id: Id,
        swarm_id: Id,
        run: serde_json::Value,
    },
    /// A swarm task was created or changed. `task` is the serialized SwarmTask row.
    SwarmTaskUpdated {
        workspace_id: Id,
        swarm_id: Id,
        project_id: Id,
        task: serde_json::Value,
    },
    /// A new message was posted to a swarm's shared board. `message` is the
    /// serialized SwarmMessage row.
    SwarmMessagePosted {
        workspace_id: Id,
        swarm_id: Id,
        message: serde_json::Value,
    },
    /// A swarm's lifecycle status changed (active | paused | aborted).
    SwarmStatus {
        workspace_id: Id,
        swarm_id: Id,
        status: String,
    },
    /// Throttle marker emitted after each metrics-sampler tick. The UI can
    /// subscribe to refresh the `/usage/metrics` sparklines in near-real-time
    /// instead of polling blindly. `ts` is the sample timestamp (UTC ISO-8601).
    UsageMetricsTick { ts: String },
}
