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
    /// A product story AI run (analysis, rewrite, plan, testcases) completed or
    /// changed section. Lets the UI drop polling for that tab and switch to
    /// event-driven refresh. `section` is one of "analysis" | "rewrite" |
    /// "plan" | "testcases". `status` mirrors the run status ("done" | "error" | "partial").
    ProductChanged {
        workspace_id: Id,
        story_id: Id,
        section: String,
        status: String,
    },
    /// A multi-agent plan generation kicked off N visible planning sessions (and,
    /// when >1 planner, a summarizer session). The Plan tab uses this to tile the
    /// sessions side-by-side so the user can watch them work (and answer questions
    /// in interactive mode). `session_ids` are the live, openable sessions in
    /// spawn order (planners first, summarizer appended when it starts).
    /// `interactive` mirrors the request: `false` ⇒ agents run unattended.
    PlanRun {
        workspace_id: Id,
        story_id: Id,
        session_ids: Vec<Id>,
        interactive: bool,
    },
    /// A PR/code-review row changed state (queued | running | done | error |
    /// cancelled). The Review Panel uses this to poll immediately instead of
    /// waiting for its back-off timer. `session_id` is the orchestrating session
    /// (may be `None` for externally-triggered reviews).
    ReviewChanged {
        workspace_id: Id,
        session_id: Option<Id>,
        review_id: Id,
        status: String,
    },
    /// A self-improvement run finished or an approval became pending. Lets the
    /// Self-Improvement settings pane refresh on the event instead of guessing.
    /// `kind` is "run_finished" | "approval_pending".
    ImprovementUpdated { kind: String, id: Option<Id> },
    /// A workflow run advanced (a node started/finished, or the run completed).
    /// `node_id` is the node that changed, when applicable.
    WorkflowRunUpdated {
        workspace_id: Id,
        run_id: Id,
        status: String,
        node_id: Option<Id>,
    },
    /// A skill-evaluation run advanced. Lets the Skill-Eval UI switch from
    /// fixed-interval polling to event-driven refresh.
    SkillEvalUpdated {
        workspace_id: Id,
        run_id: Id,
        status: String,
    },
    /// An insights report became available for a cadence period. Used by the
    /// channel notifier (opt-in) and the Insights UI to refresh without polling.
    /// `period` is the human label for the completed period ("daily 2026-06-20",
    /// "weekly 2026-W25", etc.). `session_id` is the completing session (for
    /// cross-link in the UI; may be omitted if the caller doesn't have it).
    InsightReady {
        period: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Id>,
    },
    /// A usage budget was exceeded (or recovered). Emitted by the usage-sampler
    /// when `enforce = true` and a cap is crossed. The channel notifier (opt-in)
    /// can forward this to Slack/Telegram. `direction` is "exceeded" | "recovered".
    BudgetExceeded {
        workspace_id: Id,
        provider: String,
        spend_usd: f64,
        cap_usd: f64,
        direction: String,
    },
}
