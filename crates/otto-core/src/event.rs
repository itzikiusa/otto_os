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
    /// A swarm goal was created/changed (verification progress). `goal` is the
    /// serialized SwarmGoal row (otto-core can't depend on otto-state).
    SwarmGoalUpdated {
        workspace_id: Id,
        swarm_id: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<Id>,
        goal: serde_json::Value,
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
    /// A goal loop advanced (status/phase/iteration change, after each
    /// evaluation, or when an executor's live state flips — e.g. → waiting).
    /// The Loops UI re-fetches `GET /goal-loops/{id}` on a matching tick and
    /// updates the list row directly from these fields.
    GoalLoopUpdated {
        workspace_id: Id,
        loop_id: Id,
        /// `GoalLoopStatus` as snake_case.
        status: String,
        /// `GoalLoopPhase` as snake_case.
        phase: String,
        current_iteration: u32,
        progress_pct: u32,
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
    /// A canvas scene's source document changed — emitted LIVE while an agent
    /// edits the scene's backing file (per-poll, mid-turn) and once more with the
    /// committed result. `doc` is the opaque canvas document
    /// (`{type:"otto-canvas",format,source,...}`) so the open editor can re-render
    /// in place without a refetch. The Canvas page subscribes and renders the
    /// `doc` for the matching `scene_id`.
    CanvasUpdated {
        workspace_id: Id,
        scene_id: Id,
        doc: serde_json::Value,
    },
    /// A canvas scene's Ask-AI agent session just became live (at the START of a
    /// turn). Lets the Canvas Assistant panel attach the agent's shell/Terminal
    /// immediately, instead of only after the turn finishes.
    CanvasSessionStarted {
        workspace_id: Id,
        scene_id: Id,
        session_id: Id,
    },
    /// A product mockup's source changed — emitted LIVE while the mockup agent
    /// edits the backing file (per-poll, mid-turn) and once more with the committed
    /// result. `content` is the raw mockup source (a self-contained HTML page or a
    /// Mermaid diagram); `format` is `html` | `mermaid`. The Product → Mockups
    /// Assistant panel subscribes and re-renders the live preview for the matching
    /// `attachment_id`.
    MockupUpdated {
        workspace_id: Id,
        story_id: Id,
        attachment_id: Id,
        format: String,
        content: String,
    },
    /// A mockup agent session just became live (at the START of a turn). Lets the
    /// Mockups Assistant panel attach the agent's shell/Terminal immediately,
    /// instead of only after the turn finishes.
    MockupSessionStarted {
        workspace_id: Id,
        story_id: Id,
        attachment_id: Id,
        session_id: Id,
    },
    /// A DB Explorer "assistant" agent session just became live (at the START of a
    /// turn). Lets the Database page attach the agent's live shell/Terminal
    /// immediately. The session is hidden from the Agents list via `meta.source`.
    DbAssistSessionStarted {
        workspace_id: Id,
        connection_id: Id,
        assist_id: Id,
        session_id: Id,
    },
    /// The DB assistant's working answer changed — emitted LIVE while the agent
    /// writes its `ANSWER.sql` (per-poll, mid-turn) and once with the final result.
    /// `sql` is the current proposed query; `note` is a short status line. The
    /// Database page renders this in the assistant panel as the agent works.
    DbAssistUpdated {
        workspace_id: Id,
        connection_id: Id,
        assist_id: Id,
        sql: String,
        note: String,
    },
    /// A work-graph item was created or changed — Mission Control's live signal.
    /// The Mission Control page re-fetches the matching workspace's summary/list
    /// on a matching tick instead of polling. `kind`/`status` are the normalized
    /// snake_case strings (otto-core stays free of otto-state types).
    WorkGraphUpdated {
        workspace_id: Id,
        item_id: Id,
        kind: String,
        status: String,
    },
    /// A review finding's workflow `status` (or a tracked field) changed — emitted
    /// after every triage action / transition. The Findings board subscribes and
    /// refetches the matching finding (like `review_changed` drives the panel).
    /// `status` is the new `FindingStatus` as snake_case.
    FindingUpdated {
        workspace_id: Id,
        review_id: Id,
        finding_id: Id,
        status: String,
    },
    /// An agent-backed finding action just spawned a live, openable session (fix /
    /// verify / regression-test). Lets the board attach the agent's shell so the
    /// user can watch it close the loop. `action` is "fix" | "verify" | "regression_test".
    FindingActionStarted {
        workspace_id: Id,
        review_id: Id,
        finding_id: Id,
        action: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<Id>,
    },
    /// A review's Proof Pack was exported (a snapshot persisted + verified findings
    /// ingested into memory). The Review panel can surface the new evidence bundle.
    ProofPackExported {
        workspace_id: Id,
        review_id: Id,
        proof_pack_id: Id,
    },
    /// A proof pack was created, (re)assembled, had an artifact added, or was
    /// waived — its derived status / risk may have changed. The UI re-fetches the
    /// affected pack and refreshes the workspace proof summary.
    ProofPackUpdated {
        workspace_id: Id,
        proof_pack_id: Id,
        work_item_kind: String,
        work_item_id: String,
        status: String,
        risk_score: u8,
        /// Done-contract readiness 0..100 (see `proof::compute_done_contract`).
        #[serde(default)]
        done_score: u8,
    },
    /// A scheduled-task run started, finished, or errored. The Scheduled Tasks page
    /// subscribes and re-fetches the task's run history on a matching tick instead
    /// of polling. `status` is the snake_case run status ("running"|"ok"|"error").
    ScheduledTaskRunUpdated {
        workspace_id: Id,
        task_id: Id,
        run_id: Id,
        status: String,
    },
    /// A Run with Otto run advanced a stage, errored, or finished. The Run with
    /// Otto page re-fetches the run + its timeline on a matching tick. `status` is
    /// the snake_case `RunStatus`.
    OttoRunUpdated {
        workspace_id: Id,
        run_id: Id,
        status: String,
    },
}
