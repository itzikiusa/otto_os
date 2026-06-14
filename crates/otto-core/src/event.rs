//! Events broadcast on the daemon event bus and streamed to `/ws/events`.

use serde::{Deserialize, Serialize};

use crate::domain::{Notice, Session, SessionStatus};
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
    Notification { notice: Notice },
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
}
