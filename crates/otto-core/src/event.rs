//! Events broadcast on the daemon event bus and streamed to `/ws/events`.

use serde::{Deserialize, Serialize};

use crate::domain::{AgentTask, Notice, Session, SessionStatus, TrailEvent};
use crate::Id;
use std::sync::Arc;

/// Most worktree paths one `RepoStatusChanged` lists before it falls back to
/// "unknown" (`paths: None`).
pub const REPO_CHANGED_MAX_PATHS: usize = 64;

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
    /// Authoritative row after either archive transition.
    SessionArchiveChanged { session: Session },
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
    /// An API-client history row was written (a human Send or an agent tool run).
    /// Carries ids only — never the request or response.
    ApiHistoryAppended {
        workspace_id: Id,
        entry_id: Id,
        source: String,
        session_id: Option<Id>,
        request_id: Option<Id>,
    },
    /// An API automation run completed a step or finished (perf F2): the
    /// running view fetches the new steps on this instead of polling blind.
    /// Counts and status only — never a step result.
    ApiRunProgress {
        workspace_id: Id,
        automation_id: Id,
        run_id: Id,
        status: String,
        steps_done: usize,
    },
    /// A saved API-client object changed (perf N4): a request, collection,
    /// environment or automation was created, updated, deleted or imported —
    /// by a person or an agent tool. The UI drops its 60 s list cache for the
    /// workspace on this. `kind` is `request` | `collection` | `environment` |
    /// `automation`; `id` is `None` for bulk changes (imports). Ids only.
    ApiClientChanged {
        workspace_id: Id,
        kind: String,
        id: Option<Id>,
        deleted: bool,
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
        /// The changed node's SUMMARY — exactly the node shape
        /// `GET /workflows/runs/{id}/progress` returns (`logs: []` +
        /// `log_count`, `output: null` + `has_output`, `detail_version`), so
        /// a client applies it in place without refetching (perf W5). Was the
        /// full `NodeRunState` (≤ 32 KiB), which clients refetched anyway.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<serde_json::Value>,
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
    /// Design Hall: an artifact in the design graph changed — a new committed
    /// version (content save, named commit, legacy import/sync) or a
    /// metadata-only change (title/status/tags/approve/archive). Supersedes
    /// `MockupUpdated`/`CanvasUpdated` for graph-aware clients; both legacy
    /// events keep firing for their own routes.
    DesignArtifactUpdated {
        workspace_id: Id,
        artifact_id: Id,
        /// The artifact's format (`html` | `scene3d` | `otto-canvas` | `png` …).
        format: String,
        /// `created` | `content` | `meta` | `approved` | `archived` | `deleted`,
        /// or `live` — an UNCOMMITTED, validated edit a design-assist agent
        /// just saved mid-turn (`version_id: null`; the turn's commit follows).
        change: String,
        /// The newly committed version, when the change created one.
        version_id: Option<Id>,
        /// The new source for text formats ≤ 4 MB; an explicit `null` (never
        /// omitted) for binaries, oversized payloads and metadata-only changes —
        /// clients re-fetch `GET /design/artifacts/{id}/content` then.
        content: Option<String>,
    },
    /// Design Hall: a link touching `artifact_id` changed — an explicit link was
    /// created/deleted, the document's extracted links were rebuilt, or the
    /// link's TARGET moved (a new approved version for `follow_approved`
    /// consumers, a new head for `follow_latest` ones). Consumers show a pulse +
    /// "now vN" badge and re-fetch `GET /design/artifacts/{id}/links`.
    DesignLinkUpdated {
        workspace_id: Id,
        /// The consumer (link source) artifact.
        artifact_id: Id,
        link_id: Option<Id>,
        target_artifact_id: Option<Id>,
        target_version_id: Option<Id>,
        /// `created` | `deleted` | `extracted` | `target_approved` | `target_updated`.
        reason: String,
    },
    /// Design Hall learning loop: a design signal was captured (Phase 0 only
    /// captures; later phases also emit this when a learned rule is proposed).
    DesignLearningUpdate {
        workspace_id: Id,
        /// The signal kind (`variant_chosen`, `edit_after_draft`, …).
        kind: String,
        signal_id: Option<Id>,
        artifact_id: Option<Id>,
    },
    /// Design Hall: a unified design-assist agent turn (`POST
    /// /design/artifacts/{id}/assist`, or one variant of `…/variants`) changed
    /// state — `running` once its agent session is live (attach the shell),
    /// then exactly one terminal state. A committed turn ALSO produces the
    /// usual `design_artifact_updated` (main branch only; variants never touch
    /// the head).
    DesignAssistUpdated {
        workspace_id: Id,
        artifact_id: Id,
        turn_id: Id,
        /// `starting` (accepted, session not live yet) | `running` | `done` (a
        /// version was committed) | `unchanged` (the agent changed nothing —
        /// e.g. a critique) | `conflict` (the head moved meanwhile: the draft
        /// was kept as a side version `variant/<turn>/1`) | `failed`.
        status: String,
        /// `generate` | `refine` | `critique` | `a11y` | `variant`.
        mode: String,
        /// `main`, or `variant/<run>/<k>` for a variant turn.
        branch: String,
        session_id: Option<Id>,
        /// The version this turn committed, when it committed one.
        version_id: Option<Id>,
        error: Option<String>,
    },
    /// Design Hall: every turn of a variants run (`POST
    /// /design/artifacts/{id}/variants`) finished. `version_ids` are the
    /// committed variant versions (branch `variant/<run_id>/<k>`, head
    /// untouched); `failed` turns produced nothing. Accept one with `POST
    /// …/variants/{version}/accept`.
    DesignVariantsReady {
        workspace_id: Id,
        artifact_id: Id,
        run_id: Id,
        base_version_id: Option<Id>,
        version_ids: Vec<Id>,
        failed: usize,
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
        /// The pack's badges after this recompute, so a listener can patch its
        /// summary row in place instead of refetching (absent from older
        /// emitters).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        badges: Option<Vec<String>>,
        /// Evidence count after this recompute.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        artifact_count: Option<u32>,
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
    /// A personal agent's live activity changed: one of its sessions made a
    /// governed tool call (allowed / blocked / needs approval) or is waiting on
    /// an approval. Ids only — the agent page re-fetches
    /// `GET /personal-agents/{id}/activity`. `kind` is the entry kind.
    PersonalAgentActivity {
        workspace_id: Id,
        agent_id: Id,
        kind: String,
    },
    /// A message was appended to an agent room (by an agent over the room MCP
    /// tools, or by the user over REST). Carries the whole message (`text`,
    /// `created_at`) so an open room appends it without a GET; a client only
    /// re-fetches after its cursor when it detects a gap. `author_kind` is
    /// "agent" | "user".
    AgentRoomMessage {
        workspace_id: Id,
        room_id: Id,
        message_id: Id,
        author_kind: String,
        author_id: Id,
        text: String,
        created_at: String,
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
    CanvasRefsChanged { workspace_id: Id, session_id: Id },
    /// A watched repository's working tree, index or refs changed on disk (an
    /// editor save, a CLI `git add`/commit/checkout). The Git page re-reads
    /// the repo's local status instead of waiting for the next auto-fetch.
    /// Debounced per repo; only repos a client recently asked about are
    /// watched (`otto-git` `watch.rs`).
    ///
    /// `paths`: the repo-relative worktree paths (files or directories) the
    /// burst touched, so an open diff re-reads only when ITS file changed.
    /// Absent means "unknown — assume anything": more than
    /// [`REPO_CHANGED_MAX_PATHS`] paths, a dropped-events rescan, or a change
    /// inside `.git` (index / HEAD / branch refs alter every file's diff).
    /// Present and empty: only status-level state moved (e.g. remote refs).
    RepoStatusChanged {
        workspace_id: Id,
        repo_id: Id,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        paths: Option<Vec<String>>,
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
    /// A tab's remote live session (daemon Chromium) opened / became ready /
    /// crashed / closed (`state` ∈ `starting|ready|crashed|closed`). Carries no
    /// URL or title — the session is private to its owner; the Browser page
    /// only badges the tab and re-fetches `GET /browser/tabs/{id}/live`.
    BrowserLiveSessionUpdated {
        workspace_id: Id,
        tab_id: Id,
        owner_id: Id,
        state: String,
    },
    /// The Chromium download job ticked (`state` ∈
    /// `downloading|verifying|extracting|installed|failed`). Machine-wide.
    BrowserEngineInstallUpdated {
        build: String,
        version: String,
        state: String,
        received_bytes: u64,
        total_bytes: Option<u64>,
        error: Option<String>,
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
    /// `turns` is an `Arc<[_]>`: every bus subscriber's `recv` clones the
    /// event, and a refcount bump beats deep-copying a 64 KB JSON tree a dozen
    /// times per frame (serializes exactly like a `Vec`).
    TranscriptAppended {
        workspace_id: Id,
        session_id: Id,
        cursor: String,
        turns: Arc<[serde_json::Value]>,
    },
    /// Conversation view: the agent's in-progress response as currently drawn
    /// on the session's terminal screen (plain text, ≤ 16 KB), pushed by the
    /// live tail at most once per poll while the text changes. The provider
    /// writes a transcript record only when a block COMPLETES, so this is the
    /// only sub-turn signal; clients show it as a draft below the last folded
    /// turn and drop it once the real turn lands. `text` is empty when nothing
    /// is streaming. Session-family scoped. The text fields are `Arc<str>`
    /// (cheap per-subscriber clones, same JSON string on the wire) — this is
    /// pushed every ~700 ms per streaming session.
    TranscriptLive {
        workspace_id: Id,
        session_id: Id,
        text: Arc<str>,
        /// Text currently typed (unsent) in the terminal's input box — the
        /// chat shows it so a message sent from the chat is known to be
        /// appended to it (the CLI submits both as ONE message).
        input: Arc<str>,
        /// The terminal's status rows below the input box (the CLI's own
        /// status line: model, context %, plan limits, mode …), joined by
        /// " · ".
        status: Arc<str>,
        /// Git branch of the session cwd (from `.git/HEAD`), if any.
        branch: Option<String>,
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
    /// Otto Assistant: a turn was indexed into a thread (a user send echo, a
    /// landed reply with its provider badge, or a system line — memory chip,
    /// delegation, reminder, route/limit notice). `turn` / `thread` are the
    /// serialized `otto_state::AssistantTurn` / `AssistantThread` (otto-core
    /// can't depend on otto-state); `thread` is `null` when it didn't change.
    /// OWNER-scoped: delivered only to `user_id`'s connections.
    AssistantTurn {
        user_id: Id,
        thread_id: Id,
        turn: serde_json::Value,
        thread: Option<serde_json::Value>,
    },
    /// Otto Assistant: a task was created or changed state. Owner-scoped.
    AssistantTaskUpdate {
        user_id: Id,
        task: serde_json::Value,
    },
    /// Otto Assistant: a task entered or left the needs-you queue;
    /// `open_count` is the queue size after the change. Owner-scoped.
    AssistantNeedsYou {
        user_id: Id,
        task: serde_json::Value,
        open_count: i64,
    },
    /// Otto Assistant: a provider usage limit was detected on a thread's
    /// route. `suggestion` is the route to continue on (null when none);
    /// `task_id` the "continue on X?" needs-you item; `auto_switched` means
    /// auto-failover already moved the thread. Owner-scoped.
    AssistantLimit {
        user_id: Id,
        thread_id: Option<Id>,
        limit: serde_json::Value,
        suggestion: Option<serde_json::Value>,
        task_id: Option<Id>,
        auto_switched: bool,
    },
    /// Agent UI control: an agent in `session_id` called an `otto.ui_*` tool
    /// but the session has no "Allow UI control" grant yet. The UI shows an
    /// inline Allow / Deny prompt beside the session (the grant itself is
    /// `POST /sessions/{id}/ui-control`, human credentials only). Delivered
    /// ONLY to the session owner's connections (owner-scoped, like the
    /// Assistant events) — never to workspace admins or root.
    UiControlRequested {
        user_id: Id,
        workspace_id: Id,
        session_id: Id,
        session_title: String,
        /// The paneKey the command targets (`connections`, `shell`, …).
        module: String,
        /// The catalog command name (`db_run_query`), no `ui_` prefix.
        command: String,
    },
    /// An MCP approval row changed: created (`pending`), decided
    /// (`approved`/`denied`), `consumed`, or `expired`. An invalidation
    /// signal only — no title/tool/args ride it; clients refetch
    /// `GET /mcp/approvals`, which applies visibility. `approval_id` is absent
    /// for a bulk expiry sweep; `workspace_id` is absent for workspace-less
    /// approvals and for `consumed`/`expired` (the row is not re-read).
    McpApprovalChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        approval_id: Option<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_id: Option<Id>,
        status: String,
    },
    /// Effective access to governed resources may have changed (a resource
    /// policy, an access group/role, a user's grants, or a workspace role was
    /// written). `kind` + `resource_id` name the one resource when the change
    /// was a resource policy; both absent = "anything may have changed".
    /// Clients re-check their cached `/access/{kind}/{id}/capabilities`.
    ResourceAccessChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resource_id: Option<Id>,
    },
    /// The caller's notice list changed without a new notice (mark-read,
    /// read-all, dismiss, clear). Owner-only; clients refetch
    /// `GET /notifications` (the tray's "needs you" glyph).
    NotificationsChanged { user_id: Id },
    /// A Workbench doc of `user_id` changed (create / content or metadata
    /// update / trash / restore / permanent delete). Owner-only; an
    /// invalidation cue — clients refetch the list and, unless `client_id` is
    /// their own (self-echo), the open doc.
    WorkbenchDocChanged {
        workspace_id: Id,
        user_id: Id,
        doc_id: Id,
        /// `created` | `updated` | `trashed` | `restored` | `deleted`.
        action: String,
        rev: i64,
        updated_at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
    },
}

impl Event {
    /// High-rate variants only WebSocket clients consume (perf2/03 N6):
    /// producers publish them with `otto_server::ws_fanout::publish_stream`,
    /// which skips the internal bus and its ~13 subscribers.
    pub fn is_streaming(&self) -> bool {
        matches!(
            self,
            Event::TranscriptLive { .. } | Event::TranscriptAppended { .. }
        )
    }

    /// The wire `type` tag (`session_status`, …) without serializing — used to
    /// filter per-connection topic subscriptions (`/ws/events` `subscribe`)
    /// before the authorization check and serialization.
    pub fn type_name(&self) -> &'static str {
        match self {
            Event::SessionStatus { .. } => "session_status",
            Event::SessionCreated { .. } => "session_created",
            Event::SessionArchiveChanged { .. } => "session_archive_changed",
            Event::SessionMetaUpdated { .. } => "session_meta_updated",
            Event::SessionRenamed { .. } => "session_renamed",
            Event::SessionRemoved { .. } => "session_removed",
            Event::Notice { .. } => "notice",
            Event::Notification { .. } => "notification",
            Event::ImprovementRunStarted { .. } => "improvement_run_started",
            Event::ImprovementRunFinished { .. } => "improvement_run_finished",
            Event::ImprovementEditApplied { .. } => "improvement_edit_applied",
            Event::ImprovementApprovalPending { .. } => "improvement_approval_pending",
            Event::TrailAppended { .. } => "trail_appended",
            Event::TasksUpdated { .. } => "tasks_updated",
            Event::ApiHistoryAppended { .. } => "api_history_appended",
            Event::ApiRunProgress { .. } => "api_run_progress",
            Event::ApiClientChanged { .. } => "api_client_changed",
            Event::SwarmRunUpdated { .. } => "swarm_run_updated",
            Event::SwarmTaskUpdated { .. } => "swarm_task_updated",
            Event::SwarmProjectCleared { .. } => "swarm_project_cleared",
            Event::SwarmMessagePosted { .. } => "swarm_message_posted",
            Event::SwarmStatus { .. } => "swarm_status",
            Event::SwarmGoalUpdated { .. } => "swarm_goal_updated",
            Event::UsageMetricsTick { .. } => "usage_metrics_tick",
            Event::ProductChanged { .. } => "product_changed",
            Event::PlanRun { .. } => "plan_run",
            Event::ReviewChanged { .. } => "review_changed",
            Event::GoalLoopUpdated { .. } => "goal_loop_updated",
            Event::ImprovementUpdated { .. } => "improvement_updated",
            Event::WorkflowRunUpdated { .. } => "workflow_run_updated",
            Event::SkillEvalUpdated { .. } => "skill_eval_updated",
            Event::SkillReviewUpdated { .. } => "skill_review_updated",
            Event::InsightReady { .. } => "insight_ready",
            Event::BudgetExceeded { .. } => "budget_exceeded",
            Event::CanvasUpdated { .. } => "canvas_updated",
            Event::CanvasSessionStarted { .. } => "canvas_session_started",
            Event::MockupUpdated { .. } => "mockup_updated",
            Event::DesignArtifactUpdated { .. } => "design_artifact_updated",
            Event::DesignLinkUpdated { .. } => "design_link_updated",
            Event::DesignLearningUpdate { .. } => "design_learning_update",
            Event::DesignAssistUpdated { .. } => "design_assist_updated",
            Event::DesignVariantsReady { .. } => "design_variants_ready",
            Event::MockupSessionStarted { .. } => "mockup_session_started",
            Event::DbAssistSessionStarted { .. } => "db_assist_session_started",
            Event::DbAssistUpdated { .. } => "db_assist_updated",
            Event::WorkGraphUpdated { .. } => "work_graph_updated",
            Event::FindingUpdated { .. } => "finding_updated",
            Event::FindingActionStarted { .. } => "finding_action_started",
            Event::ProofPackExported { .. } => "proof_pack_exported",
            Event::ProofPackUpdated { .. } => "proof_pack_updated",
            Event::ScheduledTaskRunUpdated { .. } => "scheduled_task_run_updated",
            Event::PersonalAgentRunUpdated { .. } => "personal_agent_run_updated",
            Event::PersonalAgentActivity { .. } => "personal_agent_activity",
            Event::AgentRoomMessage { .. } => "agent_room_message",
            Event::OttoRunUpdated { .. } => "otto_run_updated",
            Event::CanvasRefsChanged { .. } => "canvas_refs_changed",
            Event::RepoStatusChanged { .. } => "repo_status_changed",
            Event::BrowserTabUpdated { .. } => "browser_tab_updated",
            Event::BrowserAnnotationAdded { .. } => "browser_annotation_added",
            Event::BrowserLiveSessionUpdated { .. } => "browser_live_session_updated",
            Event::BrowserEngineInstallUpdated { .. } => "browser_engine_install_updated",
            Event::AwsAccountUpdated { .. } => "aws_account_updated",
            Event::AwsInstallUpdated { .. } => "aws_install_updated",
            Event::K8sClusterUpdated { .. } => "k8s_cluster_updated",
            Event::K8sInstallUpdated { .. } => "k8s_install_updated",
            Event::TranscriptAppended { .. } => "transcript_appended",
            Event::TranscriptLive { .. } => "transcript_live",
            Event::ArtifactAdded { .. } => "artifact_added",
            Event::HistoryIndexProgress { .. } => "history_index_progress",
            Event::K8sMonitorCycle { .. } => "k8s_monitor_cycle",
            Event::AssistantTurn { .. } => "assistant_turn",
            Event::AssistantTaskUpdate { .. } => "assistant_task_update",
            Event::AssistantNeedsYou { .. } => "assistant_needs_you",
            Event::AssistantLimit { .. } => "assistant_limit",
            Event::UiControlRequested { .. } => "ui_control_requested",
            Event::McpApprovalChanged { .. } => "mcp_approval_changed",
            Event::ResourceAccessChanged { .. } => "resource_access_changed",
            Event::NotificationsChanged { .. } => "notifications_changed",
            Event::WorkbenchDocChanged { .. } => "workbench_doc_changed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every bus subscriber deep-clones the `Event` it receives (tokio
    /// broadcast semantics), so the enum must stay small and big payloads must
    /// live behind an `Arc`/`Box`. A new variant that inlines a large struct
    /// trips this; box its payload instead of raising the bound.
    #[test]
    fn event_stays_small_for_cheap_bus_clones() {
        let size = std::mem::size_of::<Event>();
        assert!(size <= EVENT_SIZE_BUDGET, "size_of::<Event>() = {size}");
    }

    /// Bytes; see `event_stays_small_for_cheap_bus_clones`.
    const EVENT_SIZE_BUDGET: usize = 512;

    /// The `Arc` payloads serialize byte-identically to the old `Vec`/`String`
    /// shape and round-trip through serde.
    #[test]
    fn transcript_events_keep_their_wire_shape() {
        let ev = Event::TranscriptAppended {
            workspace_id: "w".into(),
            session_id: "s".into(),
            cursor: "3".into(),
            turns: vec![serde_json::json!({"id": "t1", "text": "hi"})].into(),
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"type": "transcript_appended", "workspace_id": "w",
                "session_id": "s", "cursor": "3", "turns": [{"id": "t1", "text": "hi"}]})
        );
        let back: Event = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), v);

        let ev = Event::TranscriptLive {
            workspace_id: "w".into(),
            session_id: "s".into(),
            text: "draft".into(),
            input: "".into(),
            status: "opus · 12%".into(),
            branch: None,
        };
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"type": "transcript_live", "workspace_id": "w",
                "session_id": "s", "text": "draft", "input": "",
                "status": "opus · 12%", "branch": null})
        );
        let back: Event = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), v);
        // A clone shares the payload instead of copying it.
        if let (Event::TranscriptLive { text: a, .. }, Event::TranscriptLive { text: b, .. }) =
            (&ev, &ev.clone())
        {
            assert!(Arc::ptr_eq(a, b));
        }
    }

    /// `type_name()` must equal the serde tag for every variant (it is
    /// generated from the variant names; this pins the snake_case rule incl.
    /// digits and acronyms, and the two transport events' optional fields).
    #[test]
    fn type_name_matches_the_wire_tag() {
        let events = vec![
            Event::SessionRemoved {
                session_id: "s".into(),
                workspace_id: "w".into(),
            },
            Event::Notice {
                level: "info".into(),
                title: "t".into(),
                body: "b".into(),
            },
            Event::UsageMetricsTick { ts: "now".into() },
            Event::K8sClusterUpdated {
                cluster_id: "c".into(),
                deleted: false,
            },
            Event::AwsInstallUpdated {
                tool: "aws".into(),
                state: "done".into(),
            },
            Event::CanvasRefsChanged {
                workspace_id: "w".into(),
                session_id: "s".into(),
            },
            Event::RepoStatusChanged {
                workspace_id: "w".into(),
                repo_id: "r".into(),
                paths: Some(vec!["src/a.rs".into()]),
            },
            Event::McpApprovalChanged {
                approval_id: None,
                workspace_id: None,
                status: "expired".into(),
            },
            Event::ResourceAccessChanged {
                kind: Some("connection".into()),
                resource_id: Some("r".into()),
            },
            Event::NotificationsChanged {
                user_id: "u".into(),
            },
            Event::WorkbenchDocChanged {
                workspace_id: "w".into(),
                user_id: "u".into(),
                doc_id: "d".into(),
                action: "updated".into(),
                rev: 2,
                updated_at: "2026-10-03T00:00:00Z".into(),
                client_id: None,
            },
        ];
        for e in events {
            let v = serde_json::to_value(&e).unwrap();
            assert_eq!(v["type"], e.type_name(), "{v}");
        }
        let v = serde_json::to_value(Event::McpApprovalChanged {
            approval_id: None,
            workspace_id: None,
            status: "expired".into(),
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({"type": "mcp_approval_changed", "status": "expired"})
        );
    }

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
            node: Some(serde_json::json!({
                "node_id": "step",
                "status": "running",
                "logs": [],
                "log_count": 1,
                "detail_version": "abc",
            })),
            nodes_done: 2,
            nodes_total: 5,
            waiting_approval: false,
        };
        let v: serde_json::Value = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "workflow_run_updated");
        assert_eq!(v["rev"], 7);
        assert_eq!(v["node"]["node_id"], "step");
        assert_eq!(v["node"]["status"], "running");
        assert_eq!(v["node"]["log_count"], 1);
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

    /// Otto Assistant events: snake_case tags; `thread` / `thread_id` /
    /// `suggestion` / `task_id` travel as explicit `null` (the contract's
    /// `| null`), never omitted.
    #[test]
    fn assistant_event_wire_shapes() {
        let v = serde_json::to_value(Event::AssistantTurn {
            user_id: "u1".into(),
            thread_id: "t1".into(),
            turn: serde_json::json!({"id":"x","role":"assistant","provider":"claude"}),
            thread: None,
        })
        .unwrap();
        assert_eq!(v["type"], "assistant_turn");
        assert_eq!(v["user_id"], "u1");
        assert_eq!(v["turn"]["provider"], "claude");
        assert!(v.get("thread").is_some_and(|t| t.is_null()));

        let v = serde_json::to_value(Event::AssistantTaskUpdate {
            user_id: "u1".into(),
            task: serde_json::json!({"id":"k1","state":"running"}),
        })
        .unwrap();
        assert_eq!(v["type"], "assistant_task_update");
        assert_eq!(v["task"]["state"], "running");

        let v = serde_json::to_value(Event::AssistantNeedsYou {
            user_id: "u1".into(),
            task: serde_json::json!({"id":"k1","state":"needs_you"}),
            open_count: 2,
        })
        .unwrap();
        assert_eq!(v["type"], "assistant_needs_you");
        assert_eq!(v["open_count"], 2);

        let v = serde_json::to_value(Event::AssistantLimit {
            user_id: "u1".into(),
            thread_id: Some("t1".into()),
            limit: serde_json::json!({"provider":"claude","limited":true}),
            suggestion: None,
            task_id: None,
            auto_switched: false,
        })
        .unwrap();
        assert_eq!(v["type"], "assistant_limit");

        assert_eq!(v["limit"]["provider"], "claude");
        assert!(v.get("suggestion").is_some_and(|s| s.is_null()));
        assert!(v.get("task_id").is_some_and(|s| s.is_null()));
        assert_eq!(v["auto_switched"], false);

        let v = serde_json::to_value(Event::UiControlRequested {
            user_id: "u1".into(),
            workspace_id: "w1".into(),
            session_id: "s1".into(),
            session_title: "Fix it".into(),
            module: "connections".into(),
            command: "db_run_query".into(),
        })
        .unwrap();
        assert_eq!(v["type"], "ui_control_requested");
        assert_eq!(v["session_title"], "Fix it");
        assert_eq!(v["command"], "db_run_query");
        assert_eq!(v["user_id"], "u1");
        assert_eq!(v["module"], "connections");
    }

    /// Design Hall events: snake_case tags, and the optional ids / `content`
    /// travel as explicit `null` (never omitted), like `mockup_updated`.
    #[test]
    fn design_event_wire_shapes() {
        let v = serde_json::to_value(Event::DesignArtifactUpdated {
            workspace_id: "ws1".into(),
            artifact_id: "a1".into(),
            format: "png".into(),
            change: "meta".into(),
            version_id: None,
            content: None,
        })
        .unwrap();
        assert_eq!(v["type"], "design_artifact_updated");
        assert!(v.get("content").is_some_and(|c| c.is_null()));
        assert!(v.get("version_id").is_some_and(|c| c.is_null()));

        let v = serde_json::to_value(Event::DesignLinkUpdated {
            workspace_id: "ws1".into(),
            artifact_id: "a1".into(),
            link_id: Some("l1".into()),
            target_artifact_id: Some("a2".into()),
            target_version_id: None,
            reason: "target_approved".into(),
        })
        .unwrap();
        assert_eq!(v["type"], "design_link_updated");
        assert_eq!(v["target_artifact_id"], "a2");

        let v = serde_json::to_value(Event::DesignLearningUpdate {
            workspace_id: "ws1".into(),
            kind: "variant_chosen".into(),
            signal_id: Some("s1".into()),
            artifact_id: Some("a1".into()),
        })
        .unwrap();
        assert_eq!(v["type"], "design_learning_update");
        assert_eq!(v["kind"], "variant_chosen");

        let v = serde_json::to_value(Event::DesignAssistUpdated {
            workspace_id: "ws1".into(),
            artifact_id: "a1".into(),
            turn_id: "t1".into(),
            status: "running".into(),
            mode: "refine".into(),
            branch: "main".into(),
            session_id: Some("s1".into()),
            version_id: None,
            error: None,
        })
        .unwrap();
        assert_eq!(v["type"], "design_assist_updated");
        assert_eq!(v["session_id"], "s1");
        assert!(v.get("version_id").is_some_and(|c| c.is_null()));
        assert!(v.get("error").is_some_and(|c| c.is_null()));

        let v = serde_json::to_value(Event::DesignVariantsReady {
            workspace_id: "ws1".into(),
            artifact_id: "a1".into(),
            run_id: "r1".into(),
            base_version_id: Some("v1".into()),
            version_ids: vec!["v2".into(), "v3".into()],
            failed: 1,
        })
        .unwrap();
        assert_eq!(v["type"], "design_variants_ready");
        assert_eq!(v["version_ids"][1], "v3");
        assert_eq!(v["failed"], 1);
    }
}
