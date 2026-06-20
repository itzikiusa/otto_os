// Events WS client (/ws/events) with auto-reconnect + exponential backoff.
// Feeds the workspace store (session statuses) and the toast store (notices).

import { wsConnect } from './api/client';
import type { OttoEvent } from './api/types';
import { ws } from './stores/workspace.svelte';
import { notifications } from './stores/notifications.svelte';
import { activity } from './stores/activity.svelte';
import { swarm } from './stores/swarm.svelte';
import { usage } from './api/usage.svelte';
import { product } from './stores/product.svelte';

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
          parsed.type === 'swarm_status'
        ) {
          swarm.applyEvent(parsed);
        } else if (parsed.type === 'usage_metrics_tick') {
          // Drive a near-real-time metrics sparkline refresh without polling.
          usage.applyMetricsTick();
        } else if (parsed.type === 'product_changed') {
          // Let product section tabs know a run completed (kills a poll cycle).
          product.applyEvent(parsed);
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
