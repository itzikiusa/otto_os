// Contract mirror: events. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  AgentTask, TrailEvent,
} from './activity';
import type {
  BrowserChromeBuild, BrowserEngineInstallState, BrowserLiveSessionState,
} from './browser';
import type {
  Id, SessionStatus,
} from './core';
import type {
  DesignArtifactChange, DesignLinkUpdateReason, DesignSignalKind,
} from './design';
import type {
  DesignAssistMode, DesignAssistStatus,
} from './design-assist';
import type {
  GoalLoopPhase, GoalLoopStatus,
} from './goal-loops';
import type {
  ApiAutomationRun,
} from './k8s-monitor';
import type {
  Notice,
} from './notifications';
import type {
  Session,
} from './sessions';
import type {
  Artifact, Turn,
} from './transcript';
import type {
  WorkbenchDocChangedEvent,
} from './workbench';
import type {
  NodeRunState,
} from './workflows';

/** Per-connection `/ws/events` frame (ws.md "Lag resync frame"): the daemon
 *  dropped `skipped` events for this socket — refetch event-fed state. Never an
 *  `OttoEvent` variant. */
export interface EventsResyncFrame {
  type: 'resync';
  skipped: number;
}

/** Client → `/ws/events` (any socket, ws.md "Topic subscription"): deliver
 *  only these event types (an empty list clears the filter). ≤ 64 topics of
 *  ≤ 64 chars; `resync` frames are always delivered. */
export interface EventsSubscribeFrame {
  type: 'subscribe';
  topics: string[];
}

/** Server → that socket: the active filter (`null` = everything) and the
 *  daemon's boot id (a different id than before = the daemon restarted). */
export interface EventsSubscribeAckFrame {
  type: 'subscribe_ack';
  topics: string[] | null;
  boot_id: string;
}

export type OttoEvent =
  | { type: 'session_status'; session_id: Id; workspace_id: Id; status: SessionStatus }
  | { type: 'session_created'; session: Session }
  | { type: 'session_archive_changed'; session: Session }
  | {
      type: 'session_meta_updated';
      session_id: Id;
      workspace_id: Id;
      meta: Record<string, unknown>;
    }
  | { type: 'session_renamed'; session_id: Id; workspace_id: Id; title: string }
  | { type: 'session_removed'; session_id: Id; workspace_id: Id }
  /** An agent session needs the operator: its turn ended (confirmed idle) or
   *  it raised a prompt / permission request. Sent whatever the notification
   *  settings say — it drives the session list's "needs you" flag. */
  | { type: 'session_attention'; session_id: Id; workspace_id: Id; reason: 'turn_end' | 'prompt' }
  | { type: 'notice'; level: 'info' | 'warn' | 'error'; title: string; body: string }
  | { type: 'notification'; notice: Notice; user_id?: string | null }
  // Self-improvement engine (otto-improve) — see docs/contracts/ws.md.
  | { type: 'improvement_run_started'; workspace_id: Id; run_id: Id }
  | {
      type: 'improvement_run_finished';
      workspace_id: Id;
      run_id: Id;
      status: 'done' | 'skipped' | 'failed';
      applied: number;
      pending: number;
    }
  | { type: 'improvement_edit_applied'; workspace_id: Id; run_id: Id; edit_id: Id; target_ref: string }
  | {
      type: 'improvement_approval_pending';
      workspace_id: Id;
      run_id: Id;
      edit_id: Id;
      target_ref: string;
    }
  | { type: 'trail_appended'; workspace_id: Id; session_id: Id; event: TrailEvent }
  | {
      type: 'api_history_appended';
      workspace_id: Id;
      entry_id: Id;
      source: 'agent' | 'human';
      session_id: Id | null;
      request_id: Id | null;
    }
  | {
      type: 'api_run_progress';
      workspace_id: Id;
      automation_id: Id;
      run_id: Id;
      status: ApiAutomationRun['status'];
      steps_done: number;
    }
  | {
      /** A saved request / collection / environment / automation changed —
       *  by the UI, an agent's MCP tool or another user (perf2 N4). `id` is
       *  null for a bulk change; an import emits one event per row. */
      type: 'api_client_changed';
      workspace_id: Id;
      kind: 'request' | 'collection' | 'environment' | 'automation';
      id: Id | null;
      deleted: boolean;
    }
  | { type: 'tasks_updated'; workspace_id: Id; session_id: Id; tasks: AgentTask[] }
  /** Emitted after each metrics-sampler tick so the dashboard can refresh
   *  sparklines in near-real-time. `ts` is the sample timestamp (UTC ISO-8601).
   *  A9 — Usage/Insights cluster. */
  | { type: 'usage_metrics_tick'; ts: string }
  /** A PR review run changed status (started, agent-done, finished, error).
   *  A2 — Git/Review cluster. Supplement for visibility-gated poll fallback. */
  | {
      type: 'review_changed';
      workspace_id: Id;
      session_id?: Id | null;
      review_id: Id;
      status: string;
    }
  /** A goal loop advanced (status/phase/iteration change, after each evaluation,
   *  or when an executor's live state flips). Drives the Loops UI re-fetch. */
  | {
      type: 'goal_loop_updated';
      workspace_id: Id;
      loop_id: Id;
      status: GoalLoopStatus;
      phase: GoalLoopPhase;
      current_iteration: number;
      progress_pct: number;
    }
  /** A product story AI run (analysis/rewrite/plan/testcases) completed or
   *  changed. `section` ∈ "analysis" | "rewrite" | "plan" | "testcases".
   *  `status` mirrors run status ("done" | "error" | "partial").
   *  A3 — Product cluster. */
  | {
      type: 'product_changed';
      workspace_id: Id;
      story_id: Id;
      section: string;
      status: string;
    }
  /** A multi-agent plan generation kicked off N visible planning sessions (and,
   *  with >1 planner, a summarizer). The Plan tab tiles `session_ids`
   *  side-by-side so the user can watch them (and answer questions when
   *  `interactive`). Ids are in spawn order; the summarizer is appended when it
   *  starts. A3 — Product cluster. */
  | {
      type: 'plan_run';
      workspace_id: Id;
      story_id: Id;
      session_ids: Id[];
      interactive: boolean;
    }
  /** A self-improvement run finished or an approval became pending. Lets the
   *  Self-Improvement settings pane refresh on the event instead of guessing.
   *  `kind` is "run_finished" | "approval_pending".
   *  A8 — Skills/Improve/Channels cluster. */
  | { type: 'improvement_updated'; kind: string; id?: string | null }
  /** A workflow run advanced: a node started, finished (or was served from
   *  cache), the run paused/resumed at a human approval, or it reached a
   *  terminal status. `node_id` is present when a specific node changed.
   *
   *  The additive fields let the UI apply the change IN PLACE instead of
   *  refetching the whole run per event: `rev` is the run's monotonic revision
   *  after this change (stale-snapshot guard; 0/absent → fall back to refetch),
   *  `node` is the changed node's full state (omitted when oversized), and
   *  `nodes_done`/`nodes_total`/`waiting_approval` keep the "Running" sidebar
   *  live without a second GET (`nodes_total` 0 = unknown, keep last counts).
   *  A11 — Workflows cluster. A 2.5s fallback poll remains for missed events. */
  | {
      type: 'workflow_run_updated';
      workspace_id: Id;
      run_id: Id;
      status: string;
      node_id?: Id | null;
      rev?: number;
      /** The changed node's SUMMARY — the `/progress` node shape (`logs: []` +
       *  `log_count`, `output: null` + `has_output`, `detail_version`); applied
       *  in place on a contiguous `rev` (perf W5). */
      node?: NodeRunState | null;
      nodes_done?: number;
      nodes_total?: number;
      waiting_approval?: boolean;
    }
  /** A skill-evaluation run reached a terminal state (done/error/cancelled).
   *  Lets the Skill-Eval UI replace its 2s×600 fixed poll with event-driven
   *  refresh. A11 — Workflows cluster. */
  | { type: 'skill_eval_updated'; workspace_id: Id; run_id: Id; status: string }
  /** A Skills Lab review advanced (running|done|error|cancelled). The Review
   *  panel re-fetches the matching review on the tick. */
  | { type: 'skill_review_updated'; workspace_id: Id; review_id: Id; status: string }
  /** An insights report became available for a cadence period.
   *  `period` is a human label ("daily 2026-06-20", "weekly 2026-W25").
   *  B8 — Skills/Improve/Channels. */
  | { type: 'insight_ready'; period: string; session_id?: Id | null }
  /** A usage budget cap was crossed (or recovered).
   *  Only emitted when `enforce = true`. `direction` is "exceeded"|"recovered".
   *  B8 — Skills/Improve/Channels. */
  | {
      type: 'budget_exceeded';
      workspace_id: Id;
      provider: string;
      spend_usd: number;
      cap_usd: number;
      direction: string;
    }
  | {
      /** A scheduled-task run started/finished/errored — the Scheduled Tasks page
       *  re-fetches the task's run history on a matching tick. */
      type: 'scheduled_task_run_updated';
      workspace_id: Id;
      task_id: Id;
      run_id: Id;
      status: string;
    }
  | {
      /** A personal-agent run started/finished/errored — the Personal Agents
       *  page re-fetches the agent's run history on a matching tick. */
      type: 'personal_agent_run_updated';
      workspace_id: Id;
      agent_id: Id;
      run_id: Id;
      status: string;
    }
  | {
      /** A personal agent's live activity changed (tool call allowed/blocked/
       *  needing approval, or an approval it waits on) — the Activity tab
       *  re-fetches `GET /personal-agents/{id}/activity`. */
      type: 'personal_agent_activity';
      workspace_id: Id;
      agent_id: Id;
      kind: string;
    }
  | {
      /** A message was appended to an agent room (agent via the room MCP tools,
       *  or the user over REST). Carries the whole message (`text`,
       *  `created_at`), so an open room appends it with no GET; a client
       *  re-reads the room only after a gap (reconnect / lag). */
      type: 'agent_room_message';
      workspace_id: Id;
      room_id: Id;
      message_id: Id;
      author_kind: string;
      author_id: Id;
      text: string;
      created_at: string;
    }
  | {
      /** A Kubernetes cluster row was created/updated/deleted — the Kubernetes
       *  console re-fetches its cluster list (or drops the deleted one). */
      type: 'k8s_cluster_updated';
      cluster_id: Id;
      deleted: boolean;
    }
  | {
      /** The kubectl / k9s background installer changed state. */
      type: 'k8s_install_updated';
      tool: string;
      state: string;
    }
  | {
      /** Kubernetes monitoring: a collector cycle finished for `cluster_id`.
       *  The Monitor dashboard re-fetches its overview / workloads. */
      type: 'k8s_monitor_cycle';
      cluster_id: Id;
      ok: boolean;
      pods_scraped: number;
      pods_failed: number;
      cycle_ms: number;
    }
  | {
      /** A canvas scene's source doc changed — pushed LIVE while an agent edits
       *  the backing file (per-poll) and once with the committed result. The
       *  Canvas page re-renders `doc` for the matching `scene_id`. */
      type: 'canvas_updated';
      workspace_id: Id;
      scene_id: Id;
      doc: unknown;
    }
  | {
      /** The canvas Ask-AI agent session became live (turn start) — the Canvas
       *  Assistant panel attaches its shell immediately for the matching scene. */
      type: 'canvas_session_started';
      workspace_id: Id;
      scene_id: Id;
      session_id: Id;
    }
  | {
      /** A product mockup's source changed — pushed LIVE while the mockup agent
       *  edits the backing file (per-poll) and once with the committed result. The
       *  Mockups Assistant panel re-renders the live preview for `attachment_id`. */
      type: 'mockup_updated';
      workspace_id: Id;
      story_id: Id;
      attachment_id: Id;
      format: string;
      /** The new source for text formats; an explicit `null` for binary/large
       *  payloads (glb, oversized) — clients re-fetch the attachment bytes. */
      content: string | null;
    }
  | {
      /** The mockup agent session became live (turn start) — the Mockups Assistant
       *  panel attaches its shell immediately for the matching attachment. */
      type: 'mockup_session_started';
      workspace_id: Id;
      story_id: Id;
      attachment_id: Id;
      session_id: Id;
    }
  | {
      /** Design Hall: an artifact changed — a new committed version (save,
       *  named commit, legacy import/sync) or a metadata-only change. Text
       *  content ≤ 4 MB rides along; `null` → re-fetch
       *  `GET /design/artifacts/{id}/content`. */
      type: 'design_artifact_updated';
      workspace_id: Id;
      artifact_id: Id;
      format: string;
      change: DesignArtifactChange;
      version_id: Id | null;
      content: string | null;
    }
  | {
      /** Design Hall: a link touching `artifact_id` (the consumer) changed —
       *  explicit create/delete, re-extraction on save, or its target moved
       *  (new approved version / new head / deleted). Re-fetch
       *  `GET /design/artifacts/{artifact_id}/links`. */
      type: 'design_link_updated';
      workspace_id: Id;
      artifact_id: Id;
      link_id: Id | null;
      target_artifact_id: Id | null;
      target_version_id: Id | null;
      reason: DesignLinkUpdateReason;
    }
  | {
      /** Design Hall learning loop: a design signal was captured, or
       *  (`kind: 'rule_proposed'`) new team rules await approval. */
      type: 'design_learning_update';
      workspace_id: Id;
      kind: DesignSignalKind | 'rule_proposed';
      signal_id: Id | null;
      artifact_id: Id | null;
    }
  | {
      /** Design Hall: a design-assist turn changed state (`running` once its
       *  session is live — attach the shell — then one terminal state). A
       *  committed main turn also emits `design_artifact_updated`. Fetch the
       *  full turn from `GET /design/artifacts/{artifact_id}/assist`. */
      type: 'design_assist_updated';
      workspace_id: Id;
      artifact_id: Id;
      turn_id: Id;
      status: DesignAssistStatus;
      mode: DesignAssistMode | 'variant';
      /** `main` or `variant/<run>/<k>`. */
      branch: string;
      session_id: Id | null;
      version_id: Id | null;
      error: string | null;
    }
  | {
      /** Design Hall: every turn of a variants run finished; `version_ids` are
       *  the committed variant versions (head untouched). */
      type: 'design_variants_ready';
      workspace_id: Id;
      artifact_id: Id;
      run_id: Id;
      base_version_id: Id | null;
      version_ids: Id[];
      failed: number;
    }
  | {
      /** The DB Assistant agent session became live (turn start) — the embedded
       *  DB Assistant panel (beside the query editor) attaches its live shell for
       *  the matching `assist_id`. The session is hidden from the Agents list
       *  (meta.source = 'db_assist'). */
      type: 'db_assist_session_started';
      workspace_id: Id;
      connection_id: Id;
      assist_id: Id;
      session_id: Id;
    }
  | {
      /** The DB Assistant's proposed SQL/note changed — pushed LIVE while the agent
       *  writes its ANSWER.sql, and once with the committed result. The panel shows
       *  `sql` in a read-only block with Insert/Run for the matching `assist_id`. */
      type: 'db_assist_updated';
      workspace_id: Id;
      connection_id: Id;
      assist_id: Id;
      sql: string;
      note: string;
    }
  | {
      /** A review finding's workflow `status` (or a tracked field) changed —
       *  emitted after every triage action / transition. The Findings board
       *  subscribes and refetches the matching review's findings.
       *  `status` is the new `FindingStatus` (snake_case). */
      type: 'finding_updated';
      workspace_id: Id;
      review_id: Id;
      finding_id: Id;
      status: string;
    }
  | {
      /** An agent-backed finding action spawned a live, openable session
       *  (fix / verify / regression-test). `action` is
       *  "fix" | "verify" | "regression_test". */
      type: 'finding_action_started';
      workspace_id: Id;
      review_id: Id;
      finding_id: Id;
      action: string;
      session_id?: Id | null;
    }
  | {
      /** A session's set of referenced Canvas scenes changed (attach/detach).
       *  The session's Canvas panel re-fetches `GET /sessions/{id}/canvas-refs`
       *  on a matching tick instead of polling. */
      type: 'canvas_refs_changed';
      workspace_id: Id;
      session_id: Id;
    }
  | {
      /** A watched repository's working tree, index or refs changed on disk.
       *  The Git page re-reads `GET /repos/{id}/status` (local, no fetch). */
      type: 'repo_status_changed';
      workspace_id: Id;
      repo_id: Id;
      /** Repo-relative worktree paths (files or directories) the change burst
       *  touched, so an open diff re-reads only for its own file. Absent =
       *  unknown (index/HEAD/branch-ref move, a rescan, > 64 paths): assume
       *  anything changed. Present and empty: only remote refs/tags moved. */
      paths?: string[];
    }
  | {
      /** A browser tab was created or navigated (a reader-mode navigation
       *  adopts the fetched page's title). `tab` is the full `BrowserTab`. */
      type: 'browser_tab_updated';
      workspace_id: Id;
      tab: unknown;
    }
  | {
      /** A DOM annotation was added to a page. `annotation` is the full
       *  `BrowserAnnotation`. */
      type: 'browser_annotation_added';
      workspace_id: Id;
      annotation: unknown;
    }
  | {
      /** A tab's remote live session opened / became ready / crashed / closed.
       *  Carries no URL/title (sessions are owner-private) — fetch
       *  `GET /browser/tabs/{id}/live` for details. Workspace-scoped. */
      type: 'browser_live_session_updated';
      workspace_id: Id;
      tab_id: Id;
      owner_id: Id;
      state: BrowserLiveSessionState;
    }
  | {
      /** The Chromium download job changed state (machine-wide). Progress
       *  ticks ≤ 4/s while downloading. */
      type: 'browser_engine_install_updated';
      build: BrowserChromeBuild;
      version: string;
      state: BrowserEngineInstallState;
      received_bytes: number;
      total_bytes: number | null;
      error: string | null;
    }
  | {
      /** New turns folded from a live session's provider transcript on disk
       *  (Scope::Session — carries both ids). `cursor` = index of the LAST folded
       *  record; `turns` are the turns appended since the previous push. Frames are
       *  capped at 64 KB — a bigger delta, or a gap vs the client's cursor, means
       *  the client re-fetches `GET /sessions/{id}/transcript`. */
      type: 'transcript_appended';
      workspace_id: Id;
      session_id: Id;
      cursor: string;
      turns: Turn[];
    }
  | {
      /** The agent's in-progress response as drawn on the session's terminal
       *  screen (plain text, ≤ 16 KB) — pushed by the live tail whenever it changes
       *  (≤ 1 frame per 700 ms). The provider writes a transcript record only when
       *  a block completes, so this is the sub-turn streaming signal; the chat shows
       *  it as a draft under the last turn and drops it once the real turn lands.
       *  Empty `text` = nothing streaming. Scope::Session. */
      type: 'transcript_live';
      workspace_id: Id;
      session_id: Id;
      text: string;
      /** Unsent text in the terminal's input box (a chat send is appended to it → ONE message). */
      input: string;
      /** The CLI's own status rows under the input box (model, context %, limits, mode …), " · "-joined. */
      status: string;
      /** Git branch of the session cwd, if any. */
      branch: string | null;
    }
  | {
      /** The transcript folder registered a new artifact (file / PR / image / …)
       *  for a live session — the Outputs panel and the chat's artifact chips update. */
      type: 'artifact_added';
      workspace_id: Id;
      session_id: Id;
      artifact: Artifact;
    }
  | {
      /** An AWS account row was created / updated / deleted (global scope) —
       *  the AWS console refreshes its account list. */
      type: 'aws_account_updated';
      account_id: Id;
      deleted: boolean;
    }
  | {
      /** The AWS-CLI installer changed state — the first-run panel applies the
       *  job in place instead of waiting for its 1.5 s poll. */
      type: 'aws_install_updated';
      tool: string;
      state: string;
    }
  | {
      /** Agent UI control: an agent session called an `otto.ui_*` tool but the
       *  user hasn't allowed UI control for it (`session.meta.ui_control`). Owner
       *  scope. The session's pane shows an inline "Allow for this session /
       *  Deny" prompt (stores/uiControl.svelte.ts); the tool returns
       *  `pending_grant` and the agent retries once the user allows it. */
      type: 'ui_control_requested';
      /** The session owner — the only user this event is delivered to. */
      user_id: Id;
      workspace_id: Id;
      session_id: Id;
      session_title: string;
      /** The paneKey of the module the command drives (`connections`, `shell`…). */
      module: string;
      /** The catalog command name, e.g. `db_run_query` (tool `otto.ui_db_run_query`). */
      command: string;
    }
  | {
      /** An MCP approval was created (`pending`), decided, `consumed` or
       *  `expired` — an invalidation cue only (no title/tool/args): refetch
       *  `GET /mcp/approvals`. `approval_id` is absent for a bulk expiry;
       *  `workspace_id` for workspace-less rows and consume/expire. */
      type: 'mcp_approval_changed';
      approval_id?: Id;
      workspace_id?: Id;
      status: string;
    }
  | {
      /** Effective resource access may have changed (resource policy, access
       *  group/role, user grants, workspace membership). With `kind` +
       *  `resource_id`: that one resource; without: re-check everything. */
      type: 'resource_access_changed';
      kind?: string;
      resource_id?: Id;
    }
  | {
      /** The caller's notice list changed without a new notice (read,
       *  read-all, dismiss, clear; one per bulk read / dismiss batch) —
       *  owner-only; refetch `/notifications`. */
      type: 'notifications_changed';
      user_id: Id;
    }
  | {
      type: 'db_mcp_activity';
      workspace_id: Id;
      connection_id: Id;
      session_id: Id | null;
      session_title: string | null;
      activity_id: string;
      tool: string;
      sql: string | null;
      node: string | null;
      status: 'started' | 'finished' | 'error';
      row_count: number | null;
      duration_ms: number | null;
      columns: string[];
      rows_preview: unknown[][];
      error: string | null;
      ts: string;
    }
  | WorkbenchDocChangedEvent;
