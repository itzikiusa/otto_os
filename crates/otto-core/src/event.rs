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
    /// A session's `title` changed — a user rename, or the background
    /// auto-namer adopting the provider's own session title. Carries the new
    /// title so clients can update their cached session in place (the
    /// `meta_updated` event does not carry the title).
    SessionRenamed {
        session_id: Id,
        workspace_id: Id,
        title: String,
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
    /// A project's board was cleared (all tasks + project-scoped feed deleted,
    /// in-flight runs stopped). Clients drop their local task/board state for
    /// the project instead of waiting for per-row updates that won't come.
    SwarmProjectCleared {
        workspace_id: Id,
        swarm_id: Id,
        project_id: Id,
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
    ///
    /// The additive fields let clients apply the change IN PLACE instead of
    /// refetching the whole run on every transition: `rev` is the run's
    /// monotonic revision after this change (stale-snapshot guard; 0 when the
    /// write failed), `node` is the changed node's full state (omitted when
    /// oversized — clients fall back to a refetch), and `nodes_done`/
    /// `nodes_total`/`waiting_approval` keep the "Running" sidebar live
    /// without a second GET.
    WorkflowRunUpdated {
        workspace_id: Id,
        run_id: Id,
        status: String,
        node_id: Option<Id>,
        #[serde(default)]
        rev: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<crate::workflows::NodeRunState>,
        #[serde(default)]
        nodes_done: u32,
        #[serde(default)]
        nodes_total: u32,
        #[serde(default)]
        waiting_approval: bool,
    },
    /// A skill-evaluation run advanced. Lets the Skill-Eval UI switch from
    /// fixed-interval polling to event-driven refresh.
    SkillEvalUpdated {
        workspace_id: Id,
        run_id: Id,
        status: String,
    },
    /// A skills **review** advanced (running | done | error | cancelled). The
    /// Skills Lab Review panel re-fetches `GET /skill-reviews/{id}` on a matching
    /// tick — the embedded agent terminals stream separately over `/ws/term/{id}`.
    SkillReviewUpdated {
        workspace_id: Id,
        review_id: Id,
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
    /// A product design artifact's source changed — emitted LIVE while the
    /// design agent edits the backing file (per-poll, mid-turn), once more with
    /// the committed result, on every `PUT /product/attachments/{aid}/content`
    /// save from the UI, and for each output a Blender render job attaches. The
    /// Product → Design arena subscribes and re-renders the viewer for the
    /// matching `attachment_id`.
    MockupUpdated {
        workspace_id: Id,
        story_id: Id,
        attachment_id: Id,
        /// A `DesignFormat` name (`html` | `mermaid` | `excalidraw` | `scene3d`)
        /// or, for uploaded binaries (`glb`/`gltf`/images), the attachment's mime.
        format: String,
        /// The new source for text formats; `None` (serialized as an explicit
        /// `null`, never omitted) for binary / oversized payloads — clients
        /// re-fetch `GET /product/attachments/{aid}` instead.
        content: Option<String>,
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
    /// A personal-agent run started, finished, or errored. The Personal Agents
    /// page subscribes and re-fetches the agent's run history on a matching tick
    /// instead of polling. `status` is the snake_case run status
    /// ("running"|"ok"|"error").
    PersonalAgentRunUpdated {
        workspace_id: Id,
        agent_id: Id,
        run_id: Id,
        status: String,
    },
    /// A message was appended to an agent room (by an agent over the room MCP
    /// tools, or by the user over REST). Carries ids only — clients re-fetch the
    /// room's messages after their cursor. `author_kind` is "agent" | "user".
    AgentRoomMessage {
        workspace_id: Id,
        room_id: Id,
        message_id: Id,
        author_kind: String,
        author_id: Id,
    },
    /// A Run with Otto run advanced a stage, errored, or finished. The Run with
    /// Otto page re-fetches the run + its timeline on a matching tick. `status` is
    /// the snake_case `RunStatus`.
    OttoRunUpdated {
        workspace_id: Id,
        run_id: Id,
        status: String,
    },
    /// A session's set of referenced Canvas scenes changed (attach/detach). The
    /// session's Canvas panel re-fetches `GET /sessions/{id}/canvas-refs` on a
    /// matching tick instead of polling.
    CanvasRefsChanged {
        workspace_id: Id,
        session_id: Id,
    },
    /// A browser tab was created, navigated, or had its mode changed. The open
    /// Browser page re-fetches (or applies in place) the matching tab. `tab` is
    /// the serialized `otto_state::browser::BrowserTab` (opaque here — otto-core
    /// can't depend on otto-state, like `CanvasUpdated`'s `doc`).
    BrowserTabUpdated {
        workspace_id: Id,
        tab: serde_json::Value,
    },
    /// A DOM annotation was added to a page. The open Browser page appends it
    /// for the matching tab/URL. `annotation` is the serialized
    /// `otto_state::browser::BrowserAnnotation`.
    BrowserAnnotationAdded {
        workspace_id: Id,
        annotation: serde_json::Value,
    },
    /// AWS console: an account row was created/updated/deleted. Accounts are a
    /// global library (no workspace axis) — delivered to everyone; the client
    /// re-lists (RBAC filtering happens on the list call).
    AwsAccountUpdated { account_id: Id, deleted: bool },
    /// AWS console: the CLI installer job changed state (`idle|running|done|failed`).
    AwsInstallUpdated { tool: String, state: String },
    /// Kubernetes console: a cluster row was created/updated/deleted (global).
    K8sClusterUpdated { cluster_id: Id, deleted: bool },
    /// Kubernetes console: the kubectl/k9s installer job changed state.
    K8sInstallUpdated { tool: String, state: String },
    /// Conversation view: the session's transcript grew. `turns` are the turns
    /// touched by the new records (each sent whole — clients replace by `id`);
    /// `cursor` is the index of the LAST folded record. Payloads over 64 KB are
    /// sent with `turns: []` so the client re-fetches instead. Session-family
    /// scoped (owner / workspace admin / root), like `trail_appended`. `turns`
    /// travels as JSON because otto-core cannot depend on otto-transcript.
    TranscriptAppended {
        workspace_id: Id,
        session_id: Id,
        cursor: String,
        turns: Vec<serde_json::Value>,
    },
    /// Conversation view: the transcript fold found a new artifact (a written
    /// file, PR link, image …). `artifact` is the serialized
    /// `otto_transcript::Artifact`. Session-family scoped.
    ArtifactAdded {
        workspace_id: Id,
        session_id: Id,
        artifact: serde_json::Value,
    },
    /// History index rescan progress (`POST /workspaces/{wid}/history/rescan`).
    /// Workspace-scoped. `done` is true on the final tick.
    HistoryIndexProgress {
        workspace_id: Id,
        scanned: u64,
        total: u64,
        done: bool,
    },
    /// Kubernetes monitoring: a collector cycle finished for `cluster_id`
    /// (dashboards refresh; `ok` = samples were written).
    K8sMonitorCycle {
        cluster_id: Id,
        ok: bool,
        pods_scraped: u32,
        pods_failed: u32,
        cycle_ms: u64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire shape the workflows UI merges in place: `rev` + the changed
    /// node ride the event; `node` is omitted (not null) when absent so older
    /// clients see the exact pre-0092 payload plus ignorable extras.
    #[test]
    fn workflow_run_updated_wire_shape() {
        let ev = Event::WorkflowRunUpdated {
            workspace_id: "ws1".into(),
            run_id: "r1".into(),
            status: "running".into(),
            node_id: Some("step".into()),
            rev: 7,
            node: Some(crate::workflows::NodeRunState {
                node_id: "step".into(),
                status: crate::workflows::NodeStatus::Running,
                output: None,
                error: None,
                logs: vec!["▶ log started".into()],
                started_at: None,
                duration_ms: None,
                attempts: None,
                sessions: vec![],
            }),
            nodes_done: 2,
            nodes_total: 5,
            waiting_approval: false,
        };
        let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "workflow_run_updated");
        assert_eq!(v["rev"], 7);
        assert_eq!(v["node"]["node_id"], "step");
        assert_eq!(v["node"]["status"], "running");
        // `started_at` is skip-if-none — a pending node stays compact.
        assert!(v["node"].get("started_at").is_none());
        assert_eq!(v["nodes_done"], 2);
        assert_eq!(v["nodes_total"], 5);

        // Without a node payload the key is omitted entirely.
        let ev = Event::WorkflowRunUpdated {
            workspace_id: "ws1".into(),
            run_id: "r1".into(),
            status: "success".into(),
            node_id: None,
            rev: 9,
            node: None,
            nodes_done: 5,
            nodes_total: 5,
            waiting_approval: false,
        };
        let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
        assert!(v.get("node").is_none());
    }
}
