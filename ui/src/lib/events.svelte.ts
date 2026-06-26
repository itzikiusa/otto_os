// Events WS client (/ws/events) with auto-reconnect + exponential backoff.
// Feeds the workspace store (session statuses) and the toast store (notices).

import { wsConnect } from './api/client';
import type { OttoEvent } from './api/types';
import { ws } from './stores/workspace.svelte';
import { notifications } from './stores/notifications.svelte';
import { activity } from './stores/activity.svelte';
import { swarm } from './stores/swarm.svelte';
import { loops } from './stores/loops.svelte';
import { usage } from './api/usage.svelte';
import { product } from './stores/product.svelte';
import { canvas } from './stores/canvas.svelte';
import { mockupAssist } from './stores/mockup-assist.svelte';
import { database } from './stores/database.svelte';
import { proof } from './stores/proof.svelte';
import { scheduledTasks } from './stores/scheduledTasks.svelte';

// ---------------------------------------------------------------------------
// improvement_updated — simple reactive counter so subscribed pages refresh.
// ---------------------------------------------------------------------------

/** Incremented each time an `improvement_updated` WS event arrives.
 *  Self-Improvement page subscribes to this value instead of polling. */
export class ImprovementUpdateBus {
  /** Tick counter — consumers react to its change, not the value. */
  tick: number = $state(0);
  /** Kind of the most-recent update ("run_finished" | "approval_pending"). */
  lastKind: string = $state('');
  /** Id of the updated run/edit, if the server sent one. */
  lastId: string | null = $state(null);

  apply(kind: string, id?: string | null): void {
    this.lastKind = kind;
    this.lastId = id ?? null;
    this.tick += 1;
  }
}

export const improvementBus = new ImprovementUpdateBus();

// ---------------------------------------------------------------------------
// workflow_run_updated / skill_eval_updated — reactive buses for the Workflows
// and Skill-Eval pages. Both pages subscribe to the relevant bus and trigger a
// single GET when their run_id matches, replacing fixed-interval polling.
// ---------------------------------------------------------------------------

/** Incremented each time a `workflow_run_updated` WS event arrives.
 *  WorkflowsPage subscribes and re-fetches the matching run immediately. */
export class WorkflowRunBus {
  tick: number = $state(0);
  runId: string = $state('');
  workspaceId: string = $state('');
  status: string = $state('');
  nodeId: string | null = $state(null);

  apply(workspaceId: string, runId: string, status: string, nodeId?: string | null): void {
    this.workspaceId = workspaceId;
    this.runId = runId;
    this.status = status;
    this.nodeId = nodeId ?? null;
    this.tick += 1;
  }
}

export const workflowRunBus = new WorkflowRunBus();

/** Incremented each time a `skill_eval_updated` WS event arrives.
 *  Skill-Eval pages subscribe and stop polling once a terminal status lands. */
export class SkillEvalBus {
  tick: number = $state(0);
  runId: string = $state('');
  workspaceId: string = $state('');
  status: string = $state('');

  apply(workspaceId: string, runId: string, status: string): void {
    this.workspaceId = workspaceId;
    this.runId = runId;
    this.status = status;
    this.tick += 1;
  }
}

export const skillEvalBus = new SkillEvalBus();

// ---------------------------------------------------------------------------
// review_changed / budget_exceeded — reactive buses for the Review panel and a
// budget banner. The Review panel subscribes to reviewBus and re-fetches the
// matching review/findings/merge-readiness on the event instead of polling.
// ---------------------------------------------------------------------------

/** Incremented each time a `review_changed` WS event arrives. */
export class ReviewBus {
  tick: number = $state(0);
  reviewId: string = $state('');
  workspaceId: string = $state('');
  status: string = $state('');

  apply(workspaceId: string, reviewId: string, status: string): void {
    this.workspaceId = workspaceId;
    this.reviewId = reviewId;
    this.status = status;
    this.tick += 1;
  }
}

export const reviewBus = new ReviewBus();

/** Incremented each time a `finding_updated` / `finding_action_started` WS event
 *  arrives. The Findings board subscribes (keyed by review_id) and refetches the
 *  matching review's findings — the same pattern reviewBus drives the panel. */
export class FindingBus {
  tick: number = $state(0);
  reviewId: string = $state('');
  workspaceId: string = $state('');
  findingId: string = $state('');
  /** New status (finding_updated) — empty for finding_action_started. */
  status: string = $state('');
  /** "fix" | "verify" | "regression_test" (finding_action_started) — empty otherwise. */
  action: string = $state('');
  /** The spawned agent session id (finding_action_started), if any. */
  sessionId: string | null = $state(null);

  apply(
    workspaceId: string,
    reviewId: string,
    findingId: string,
    status: string,
    action: string,
    sessionId?: string | null,
  ): void {
    this.workspaceId = workspaceId;
    this.reviewId = reviewId;
    this.findingId = findingId;
    this.status = status;
    this.action = action;
    this.sessionId = sessionId ?? null;
    this.tick += 1;
  }
}

export const findingBus = new FindingBus();

/** Incremented each time a `budget_exceeded` WS event arrives. A budget banner
 *  subscribes to surface the most-recent cap crossing (or recovery). */
export class BudgetBus {
  tick: number = $state(0);
  provider: string = $state('');
  spendUsd: number = $state(0);
  capUsd: number = $state(0);
  direction: string = $state('');

  apply(provider: string, spendUsd: number, capUsd: number, direction: string): void {
    this.provider = provider;
    this.spendUsd = spendUsd;
    this.capUsd = capUsd;
    this.direction = direction;
    this.tick += 1;
  }
}

export const budgetBus = new BudgetBus();

/** Incremented each time a `work_graph_updated` WS event arrives. The Mission
 *  Control page subscribes and re-fetches the workspace summary/list when the
 *  event's workspace matches the open one — replacing any polling. */
export class MissionControlBus {
  tick: number = $state(0);
  workspaceId: string = $state('');
  itemId: string = $state('');
  status: string = $state('');

  apply(workspaceId: string, itemId: string, status: string): void {
    this.workspaceId = workspaceId;
    this.itemId = itemId;
    this.status = status;
    this.tick += 1;
  }
}

export const missionControlBus = new MissionControlBus();

// ---------------------------------------------------------------------------
// canvas_updated — live canvas-document push. The server broadcasts the scene's
// source doc on every file change while an agent edits (and once committed); the
// open Canvas editor subscribes and re-renders the matching scene in place.
// ---------------------------------------------------------------------------

/** Holds the most-recent canvas-document update. The Canvas editor subscribes,
 *  renders `doc` when `sceneId` matches the open scene, and ignores the rest. */
export class CanvasDocBus {
  tick: number = $state(0);
  sceneId: string = $state('');
  /** The opaque canvas doc (`{type:'otto-canvas',format,source,…}`). */
  doc: unknown = $state(null);

  apply(sceneId: string, doc: unknown): void {
    this.sceneId = sceneId;
    this.doc = doc;
    this.tick += 1;
  }
}

export const canvasDocBus = new CanvasDocBus();

export type EventsState = 'connecting' | 'connected' | 'offline';

class EventsClient {
  state: EventsState = $state('offline');

  private sock: WebSocket | null = null;
  private backoff = 1000;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private stopped = false;

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    this.sock?.close();
    this.sock = null;
    this.state = 'offline';
  }

  /** Force an immediate reconnect: cancel any pending backoff timer, drop the
   *  current socket, reset backoff, and connect now. Used by the StatusBar
   *  "reconnect" affordance so a wedged stream is recoverable without an app
   *  restart. No-op while already connecting. */
  reconnectNow(): void {
    if (this.stopped) this.stopped = false;
    if (this.state === 'connecting') return;
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.backoff = 1000;
    // Detach handlers first so the old socket's onclose can't schedule a
    // competing reconnect after we've already started a fresh one.
    if (this.sock) {
      this.sock.onopen = null;
      this.sock.onmessage = null;
      this.sock.onclose = null;
      this.sock.onerror = null;
      this.sock.close();
    }
    this.sock = null;
    this.connect();
  }

  private connect(): void {
    if (this.stopped) return;
    this.state = 'connecting';
    try {
      // Bearer token travels in Sec-WebSocket-Protocol, not the URL query.
      this.sock = wsConnect('/ws/events');
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.sock.onopen = () => {
      this.state = 'connected';
      this.backoff = 1000;
    };
    this.sock.onmessage = (ev: MessageEvent) => {
      if (typeof ev.data !== 'string') return;
      try {
        const parsed = JSON.parse(ev.data) as OttoEvent;
        if (parsed.type === 'notification') {
          notifications.ingest(parsed.notice);
          // A "waiting"/blocked notice (Claude's Notification hook) means a
          // session is blocked on the operator — raise the sticky "needs you"
          // flag, distinct from plain idle. Keyed off the stable source_key.
          const n = parsed.notice;
          if (
            n.source_key?.endsWith(':waiting') &&
            n.action?.type === 'open_session'
          ) {
            ws.markNeedsYou(n.action.session_id);
          }
        } else if (parsed.type === 'trail_appended' || parsed.type === 'tasks_updated') {
          activity.applyEvent(parsed);
        } else if (
          parsed.type === 'swarm_run_updated' ||
          parsed.type === 'swarm_task_updated' ||
          parsed.type === 'swarm_message_posted' ||
          parsed.type === 'swarm_goal_updated' ||
          parsed.type === 'swarm_status'
        ) {
          swarm.applyEvent(parsed);
        } else if (parsed.type === 'usage_metrics_tick') {
          // Drive a near-real-time metrics sparkline refresh without polling.
          usage.applyMetricsTick();
        } else if (parsed.type === 'product_changed') {
          // Let product section tabs know a run completed (kills a poll cycle).
          product.applyEvent(parsed);
        } else if (parsed.type === 'plan_run') {
          // Multi-agent plan kickoff: the Plan tab tiles the live sessions.
          product.applyPlanRun(parsed);
        } else if (parsed.type === 'improvement_updated') {
          // Let the Self-Improvement pane refresh without waiting for its poll.
          improvementBus.apply(parsed.kind, parsed.id);
        } else if (parsed.type === 'workflow_run_updated') {
          // Workflow execution progress: node-start / node-finish / run complete.
          // The Workflows page subscribes to workflowRunBus and re-fetches only
          // the run whose id matches, replacing the 700ms interval poll.
          workflowRunBus.apply(parsed.workspace_id, parsed.run_id, parsed.status, parsed.node_id);
        } else if (parsed.type === 'skill_eval_updated') {
          // Skill-Eval terminal notification (done/error/cancelled).
          skillEvalBus.apply(parsed.workspace_id, parsed.run_id, parsed.status);
        } else if (parsed.type === 'review_changed') {
          // Review panel: refresh the matching review + findings + merge-readiness
          // on the event instead of waiting for its visibility-gated poll.
          reviewBus.apply(parsed.workspace_id, parsed.review_id, parsed.status);
        } else if (parsed.type === 'finding_updated') {
          // Findings board: refetch the matching review's findings on every triage
          // action / transition (status changed).
          findingBus.apply(
            parsed.workspace_id,
            parsed.review_id,
            parsed.finding_id,
            parsed.status,
            '',
            null,
          );
        } else if (parsed.type === 'finding_action_started') {
          // An agent-backed action (fix/verify/regression-test) spawned a live
          // session — let the board reflect the in-flight action.
          findingBus.apply(
            parsed.workspace_id,
            parsed.review_id,
            parsed.finding_id,
            '',
            parsed.action,
            parsed.session_id,
          );
        } else if (parsed.type === 'proof_pack_exported') {
          // A Proof Pack snapshot was exported; the board can refresh if open. We
          // route it through the finding bus tick so subscribers re-render.
          findingBus.apply(parsed.workspace_id, parsed.review_id, '', '', 'proof_pack_exported', null);
        } else if (parsed.type === 'budget_exceeded') {
          // Surface a budget cap crossing/recovery to any subscribed banner.
          budgetBus.apply(parsed.provider, parsed.spend_usd, parsed.cap_usd, parsed.direction);
        } else if (parsed.type === 'work_graph_updated') {
          // Mission Control: a work item was created or changed status. The page
          // re-fetches the matching workspace's summary/list on the event.
          missionControlBus.apply(parsed.workspace_id, parsed.item_id, parsed.status);
        } else if (parsed.type === 'goal_loop_updated') {
          // Goal Loops: update the list row + bump the open detail's re-fetch tick.
          loops.applyEvent(parsed);
        } else if (parsed.type === 'canvas_updated') {
          // Live canvas edits: the open Canvas editor re-renders the matching scene.
          canvasDocBus.apply(parsed.scene_id, parsed.doc);
        } else if (parsed.type === 'canvas_session_started') {
          // The agent session is live (turn start) → attach its shell immediately
          // by setting the open scene's session id.
          if (parsed.scene_id === canvas.currentId) canvas.sessionId = parsed.session_id;
        } else if (parsed.type === 'mockup_updated') {
          // Live mockup edits: the Mockups Assistant panel re-renders the preview.
          mockupAssist.ingestLive(
            parsed.attachment_id,
            parsed.story_id,
            parsed.format,
            parsed.content,
          );
        } else if (parsed.type === 'mockup_session_started') {
          // The mockup agent session is live (turn start) → attach its shell.
          mockupAssist.setSession(parsed.attachment_id, parsed.story_id, parsed.session_id);
        } else if (parsed.type === 'db_assist_session_started') {
          // The DB Assistant agent session is live (turn start) → attach its shell
          // in the embedded DB Assistant panel (beside the query editor).
          database.setAssistSession(parsed.assist_id, parsed.connection_id, parsed.session_id);
        } else if (parsed.type === 'db_assist_updated') {
          // Live proposed SQL/note from the DB Assistant agent → the panel's
          // read-only SQL block (Insert into editor / Run).
          database.applyAssistUpdate(
            parsed.assist_id,
            parsed.connection_id,
            parsed.sql,
            parsed.note,
          );
        } else if (parsed.type === 'proof_pack_updated') {
          // Proof page list/detail + sidebar proof chips refresh on the event.
          proof.applyEvent(parsed);
        } else if (parsed.type === 'scheduled_task_run_updated') {
          // Scheduled Tasks page refreshes the affected task's runs + list status.
          scheduledTasks.applyEvent(parsed);
        } else {
          if (parsed.type === 'session_removed') activity.forget(parsed.session_id);
          ws.applyEvent(parsed);
        }
      } catch {
        /* malformed frame — ignore */
      }
    };
    this.sock.onclose = () => {
      this.state = 'offline';
      this.scheduleReconnect();
    };
    this.sock.onerror = () => {
      this.sock?.close();
    };
  }

  private scheduleReconnect(): void {
    if (this.stopped) return;
    if (this.timer) clearTimeout(this.timer);
    this.timer = setTimeout(() => this.connect(), this.backoff);
    this.backoff = Math.min(this.backoff * 2, 30_000);
  }
}

export const events = new EventsClient();
