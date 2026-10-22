// Events WS client (/ws/events) with auto-reconnect + exponential backoff.
// Feeds the workspace store (session statuses) and the toast store (notices).

import { api, getToken, resumeAltLoopback, suspendAltLoopback, wsConnect } from './api/client';
import { inLane } from './api/lane';
import { invalidateMissionSummary } from './api/missionControl';
import { auth } from './stores/auth.svelte';
import { appLive, type LiveEvent } from './live';
import type { EventsResyncFrame, NodeRunState, OttoEvent, UiHelloAckFrame } from './api/types';
import { ws } from './stores/workspace.svelte';
import { toasts } from './toast.svelte';
import { restartSummaryText } from './status';

/** localStorage key holding the boot id the restart toast was shown for. */
const RESTART_TOAST_KEY = 'otto_restart_toast_boot';
import { notifications } from './stores/notifications.svelte';
import { activity } from './stores/activity.svelte';
import { proof } from './stores/proof.svelte';
import { transcript } from './stores/transcript.svelte';
import { assistant } from './stores/assistant.svelte';

// Store routes live per feature in `events/routes/` (architecture-2-04), each
// with its own lazy loader; the four loaders the reconnect / restart paths
// below also need are re-exported from there.
import {
  featureRoutes,
  swarmStore,
  databaseStore,
  personalAgentsStore,
  type EventRoutes,
} from './events/routes';
import {
  handleUiFrame,
  helloFrame,
  onUiCapabilitiesChanged,
  presenceFrame,
  uiSocketClosed,
} from './uiCommands';

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
 *  WorkflowsPage subscribes and applies the change to the viewed run — in
 *  place when the event carries the changed node + a contiguous `rev`, else
 *  via a rev-guarded refetch of `GET /workflow-runs/{id}`. */
export class WorkflowRunBus {
  tick: number = $state(0);
  runId: string = $state('');
  workspaceId: string = $state('');
  status: string = $state('');
  nodeId: string | null = $state(null);
  /** Run revision after this change (0 = unknown → refetch path). */
  rev: number = $state(0);
  /** The changed node's full state, when the event carried it. */
  node: NodeRunState | null = $state(null);
  waitingApproval: boolean = $state(false);

  apply(ev: {
    workspace_id: string;
    run_id: string;
    status: string;
    node_id?: string | null;
    rev?: number;
    node?: NodeRunState | null;
    waiting_approval?: boolean;
  }): void {
    this.workspaceId = ev.workspace_id;
    this.runId = ev.run_id;
    this.status = ev.status;
    this.nodeId = ev.node_id ?? null;
    this.rev = ev.rev ?? 0;
    this.node = ev.node ?? null;
    this.waitingApproval = ev.waiting_approval ?? false;
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

/** Incremented each time a `skill_review_updated` WS event arrives. The Skills
 *  Lab Review panel subscribes and re-fetches the matching review on the tick. */
export class SkillReviewBus {
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

export const skillReviewBus = new SkillReviewBus();

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
 *  event's workspace matches the open one — replacing any polling. A tick with
 *  an EMPTY `workspaceId` is a resync (events were lost while the socket was
 *  down) that every open view honours. */
export class MissionControlBus {
  tick: number = $state(0);
  workspaceId: string = $state('');
  itemId: string = $state('');
  status: string = $state('');

  apply(workspaceId: string, itemId: string, status: string): void {
    invalidateMissionSummary(workspaceId || undefined);
    this.workspaceId = workspaceId;
    this.itemId = itemId;
    this.status = status;
    this.tick += 1;
  }

  /** Events were missed (WS reconnect): every open Mission Control view reloads. */
  resync(): void {
    this.apply('', '', '');
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

// ---------------------------------------------------------------------------
// canvas_refs_changed — a session's referenced Canvas scenes changed. The
// session's Canvas panel subscribes and re-fetches when `sessionId` matches
// the open session.
// ---------------------------------------------------------------------------

export class CanvasRefsBus {
  tick: number = $state(0);
  sessionId: string = $state('');

  apply(sessionId: string): void {
    this.sessionId = sessionId;
    this.tick += 1;
  }
}

export const canvasRefsBus = new CanvasRefsBus();

// ---------------------------------------------------------------------------
// design_artifact_updated / design_link_updated / design_learning_update —
// the Design Hall graph. Events land in a short, sequenced log (several can
// arrive in one tick — a save emits artifact + link events together) so every
// open view processes each one exactly once: it remembers the last `seq` it
// handled and reads `since(lastSeq)` when `seq` changes. A WS reconnect bumps
// `resyncTick` (events were lost → views reload).
// ---------------------------------------------------------------------------

export type DesignBusEvent = Extract<
  OttoEvent,
  { type: 'design_artifact_updated' } | { type: 'design_link_updated' } | { type: 'design_learning_update' }
>;

export class DesignBus {
  /** Sequence number of the newest event (0 = none yet). */
  seq: number = $state(0);
  /** Bumped when events may have been missed (WS reconnect). */
  resyncTick: number = $state(0);
  private log: { seq: number; ev: DesignBusEvent }[] = [];

  apply(ev: DesignBusEvent): void {
    const seq = this.seq + 1;
    this.log.push({ seq, ev });
    if (this.log.length > 200) this.log.splice(0, this.log.length - 200);
    // Only the newest few entries keep their inline `content` (views consume
    // events within a tick); older ones drop it so the log can't pin
    // 200 document bodies — a late reader falls back to re-fetching.
    const old = this.log.length - 1 - DesignBus.KEEP_CONTENT;
    if (old >= 0) {
      const e = this.log[old];
      if (e.ev.type === 'design_artifact_updated' && e.ev.content != null) {
        e.ev = { ...e.ev, content: null };
      }
    }
    this.seq = seq;
  }
  private static readonly KEEP_CONTENT = 16;

  /** Events newer than `after`, oldest first. */
  since(after: number): DesignBusEvent[] {
    return this.log.filter((e) => e.seq > after).map((e) => e.ev);
  }

  resync(): void {
    this.resyncTick += 1;
  }
}

export const designBus = new DesignBus();

// design_assist_updated / design_variants_ready — the design-assist pipeline
// (agent turns + variant runs). Same sequenced-log shape as `designBus`, kept
// separate so graph views don't refresh on every turn state change; the Otto
// panel and the lobby hand-off read it.
export type DesignAssistBusEvent = Extract<
  OttoEvent,
  { type: 'design_assist_updated' } | { type: 'design_variants_ready' }
>;

export class DesignAssistBus {
  seq: number = $state(0);
  resyncTick: number = $state(0);
  private log: { seq: number; ev: DesignAssistBusEvent }[] = [];

  apply(ev: DesignAssistBusEvent): void {
    const seq = this.seq + 1;
    this.log.push({ seq, ev });
    if (this.log.length > 200) this.log.splice(0, this.log.length - 200);
    this.seq = seq;
  }

  since(after: number): DesignAssistBusEvent[] {
    return this.log.filter((e) => e.seq > after).map((e) => e.ev);
  }

  resync(): void {
    this.resyncTick += 1;
  }
}

export const designAssistBus = new DesignAssistBus();

// ---------------------------------------------------------------------------
// The route table: one typed handler per event type (architecture-2-04). The
// feature tables (events/routes/) own the store routes; the buses above route
// here. A type no table lists falls through to `ws.applyEvent` (a no-op for
// the liveEvents-only types) — `unit/eventRoutes.test.ts` holds that list.
// ---------------------------------------------------------------------------

const busRoutes: EventRoutes = {
  // Let the Self-Improvement pane refresh without waiting for its poll.
  improvement_updated: (parsed) => improvementBus.apply(parsed.kind, parsed.id),
  workflow_run_updated: (parsed) => {
    // Workflow execution progress: node-start / node-finish / approval
    // pause/resume / run complete. The Workflows page subscribes to
    // workflowRunBus and applies the change to the viewed run (in place when
    // the event carries the changed node, else a rev-guarded GET).
    workflowRunBus.apply(parsed);
    // Keep the "Running" sidebar list + nav count live — in place from the
    // event's progress fields; a full refetch only for runs the list doesn't
    // know yet (or events from an older daemon).
    ws.applyWorkflowRunEvent(parsed);
  },
  // Skill-Eval terminal notification (done/error/cancelled).
  skill_eval_updated: (parsed) => skillEvalBus.apply(parsed.workspace_id, parsed.run_id, parsed.status),
  // Skills Lab review advanced — refresh the matching review.
  skill_review_updated: (parsed) => skillReviewBus.apply(parsed.workspace_id, parsed.review_id, parsed.status),
  // Review panel: refresh the matching review + findings + merge-readiness on
  // the event instead of waiting for its visibility-gated poll.
  review_changed: (parsed) => reviewBus.apply(parsed.workspace_id, parsed.review_id, parsed.status),
  // Findings board: refetch the matching review's findings on every triage
  // action / transition (status changed).
  finding_updated: (parsed) =>
    findingBus.apply(parsed.workspace_id, parsed.review_id, parsed.finding_id, parsed.status, '', null),
  // An agent-backed action (fix/verify/regression-test) spawned a live
  // session — let the board reflect the in-flight action.
  finding_action_started: (parsed) =>
    findingBus.apply(parsed.workspace_id, parsed.review_id, parsed.finding_id, '', parsed.action, parsed.session_id),
  // A Proof Pack snapshot was exported; the board can refresh if open. We
  // route it through the finding bus tick so subscribers re-render.
  proof_pack_exported: (parsed) =>
    findingBus.apply(parsed.workspace_id, parsed.review_id, '', '', 'proof_pack_exported', null),
  // Surface a budget cap crossing/recovery to any subscribed banner.
  budget_exceeded: (parsed) => budgetBus.apply(parsed.provider, parsed.spend_usd, parsed.cap_usd, parsed.direction),
  // Mission Control: a work item was created or changed status. The page
  // re-fetches the matching workspace's summary/list on the event.
  work_graph_updated: (parsed) => missionControlBus.apply(parsed.workspace_id, parsed.item_id, parsed.status),
  // Live canvas edits: the open Canvas editor re-renders the matching scene.
  canvas_updated: (parsed) => canvasDocBus.apply(parsed.scene_id, parsed.doc),
  // A scene was attached/detached to a session — the session's Canvas panel
  // refetches when its session id matches.
  canvas_refs_changed: (parsed) => canvasRefsBus.apply(parsed.session_id),
  // Design Hall graph: the lobby, the open artifact, its Links panel and the
  // learning log each re-fetch what the event touches.
  design_artifact_updated: (parsed) => designBus.apply(parsed),
  design_link_updated: (parsed) => designBus.apply(parsed),
  design_learning_update: (parsed) => designBus.apply(parsed),
  // Design assist: the Otto panel's turn states + the variants tray.
  design_assist_updated: (parsed) => designAssistBus.apply(parsed),
  design_variants_ready: (parsed) => designAssistBus.apply(parsed),
};

/** Every routed event type → its handler. */
export const eventRoutes: EventRoutes = { ...featureRoutes, ...busRoutes };

/** Route one event to its store / bus (after the generic `liveEvents`). */
function dispatchEvent(ev: OttoEvent): void {
  const route = eventRoutes[ev.type] as ((e: OttoEvent) => void) | undefined;
  if (route) route(ev);
  else ws.applyEvent(ev);
}

export type EventsState = 'connecting' | 'connected' | 'offline';

/** Generic subscription registry over this window's `/ws/events` stream
 *  (TRANSPORT_PLAN stage 2; `appLive` from lib/live.ts, fed here):
 *  `liveEvents.on(types, fn)` next to the
 *  hard-coded store dispatch below, plus `onResync` (reconnect / lag) and
 *  `onConnection`. `liveQuery` (lib/live.ts) uses it as its default source, so
 *  a new event-fed view never has to be added to `resyncAfterReconnect`. */
export const liveEvents = appLive;

/** Event types whose handler failure was already reported this page life. */
const reportedHandlerErrors = new Set<string>();

/** A WS event handler threw: console + one `/client/errors` report per type. */
function reportHandlerError(type: unknown, e: unknown): void {
  const kind = typeof type === 'string' ? type : 'unknown';
  console.error('[otto] /ws/events handler for "%s" failed', kind, e);
  if (reportedHandlerErrors.has(kind)) return;
  reportedHandlerErrors.add(kind);
  const err = e as { message?: unknown; stack?: unknown } | null;
  void api.bg
    .post('/client/errors', {
      kind: 'ws_event_handler',
      message: `${kind}: ${String(err?.message ?? e)}`.slice(0, 2000),
      stack: String(err?.stack ?? '').slice(0, 4000),
      route: typeof location !== 'undefined' ? location.hash : '',
      action: 'none',
    })
    .catch(() => {});
}

class EventsClient {
  state: EventsState = $state('offline');

  private sock: WebSocket | null = null;
  private backoff = 1000;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private stopped = false;
  // True once any connection has opened — distinguishes a RE-connect (which
  // must resync event-driven stores; events were lost) from the first connect.
  private everConnected = false;
  // Pending `resync` frame (server lagged → dropped events for this socket).
  // Trailing-debounced so a burst of lag frames costs one refetch.
  private lagResyncTimer: ReturnType<typeof setTimeout> | null = null;
  // Agent UI control (ws.md §2): this socket is no longer receive-only — it
  // introduces the document (`hello`) and keeps the daemon's picture of it
  // current (`presence`, debounced), so an agent's `ui_command` reaches the
  // pane the user can see. Listeners live for the client's lifetime.
  private presenceTimer: ReturnType<typeof setTimeout> | null = null;
  private presenceWired = false;
  private lastPresence = '';
  // The bearer the open socket authenticated with. The daemon binds the
  // identity at upgrade, so after impersonate / stop-impersonating / a
  // re-login the socket kept streaming the OLD user's events until it
  // happened to drop — reconnect as soon as the stored token changes.
  private socketToken: string | null = null;
  private authWired = false;
  // Set when the token changed (S13-02): the next socket open resets the
  // identity-scoped caches and resyncs every store, instead of the plain
  // first-connect path that left the previous identity's notices, needs-you
  // items, activity and proof packs on screen.
  private identityPending = false;

  /** Send a client frame on the open socket (dropped while disconnected —
   *  the next `hello` carries the current state anyway). */
  private sendFrame(frame: object): void {
    if (this.sock?.readyState !== WebSocket.OPEN) return;
    try {
      this.sock.send(JSON.stringify(frame));
    } catch {
      /* closing */
    }
  }

  private sendHello(): void {
    const hello = helloFrame();
    this.lastPresence = JSON.stringify({ r: hello.route, f: hello.focused, v: hello.visible });
    this.sendFrame(hello);
  }

  /** Route / focus / visibility changed: one `presence` 250 ms later, and only
   *  when something the daemon ranks by actually changed. */
  private schedulePresence = (): void => {
    if (this.presenceTimer) clearTimeout(this.presenceTimer);
    this.presenceTimer = setTimeout(() => {
      this.presenceTimer = null;
      const p = presenceFrame();
      const sig = JSON.stringify({ r: p.route, f: p.focused, v: p.visible });
      if (sig === this.lastPresence) return;
      this.lastPresence = sig;
      this.sendFrame(p);
    }, 250);
  };

  private wirePresence(): void {
    if (this.presenceWired || typeof window === 'undefined') return;
    this.presenceWired = true;
    window.addEventListener('hashchange', this.schedulePresence);
    window.addEventListener('focus', this.schedulePresence);
    window.addEventListener('blur', this.schedulePresence);
    // Focus moving between this document and the side pane's iframe.
    document.addEventListener('focusin', this.schedulePresence);
    document.addEventListener('visibilitychange', this.schedulePresence);
    // A module that registers its handlers late: re-introduce the document.
    let t: ReturnType<typeof setTimeout> | null = null;
    onUiCapabilitiesChanged(() => {
      if (t) clearTimeout(t);
      t = setTimeout(() => this.sendHello(), 100);
    });
  }

  private wireAuth(): void {
    if (this.authWired || typeof window === 'undefined') return;
    this.authWired = true;
    window.addEventListener('otto:auth-changed', () => this.onAuthChanged());
  }

  /** The stored token changed (login / logout / impersonation): a socket of
   *  the previous identity must not keep feeding this document. The next
   *  open is a FIRST connect for the new identity (pages reload their own
   *  data), not a resync of the old one. */
  private onAuthChanged(): void {
    if (this.stopped || getToken() === this.socketToken) return;
    this.everConnected = false;
    this.identityPending = true;
    // Between sockets (backoff): the next connect already sends the new token.
    if (this.sock === null) return;
    // reconnectNow() is a no-op while connecting — that handshake carries the
    // old token too, so drop it first.
    if (this.state === 'connecting') this.state = 'offline';
    this.reconnectNow();
  }

  start(): void {
    this.stopped = false;
    this.wireAuth();
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    // Pending debounced work belongs to the stopped stream.
    if (this.lagResyncTimer) clearTimeout(this.lagResyncTimer);
    this.lagResyncTimer = null;
    if (this.presenceTimer) clearTimeout(this.presenceTimer);
    this.presenceTimer = null;
    uiSocketClosed();
    // Detach first (as reconnectNow does, S13-08): a quick stop→start would
    // otherwise let this socket's late onclose schedule a SECOND connection,
    // and every event / toast / notice would then be dispatched twice.
    this.detach(this.sock);
    this.sock?.close();
    this.sock = null;
    this.state = 'offline';
    liveEvents.setConnected(false);
  }

  private detach(sock: WebSocket | null): void {
    if (!sock) return;
    sock.onopen = null;
    sock.onmessage = null;
    sock.onclose = null;
    sock.onerror = null;
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
      this.detach(this.sock);
      this.sock.close();
    }
    this.sock = null;
    uiSocketClosed();
    this.connect();
  }

  /** Refetch everything the always-mounted shell caches from events. A daemon
   *  restart kills sessions and finishes runs without a single event reaching
   *  this client, so the sidebar kept showing them "working", with stale
   *  badges and unread counts, until a full reload. Page-scoped stores reload
   *  on mount; swarm + open transcripts resync themselves. */
  private resyncAfterReconnect(): void {
    // Background work by definition (TRANSPORT_PLAN §4): every fetch issued
    // here goes to the bg lane — capped, and on the alias host — instead of
    // a dozen-request burst per document on the six interactive sockets.
    inLane('bg', () => this.resyncStores());
  }

  /** First open after an identity change: drop what the previous identity
   *  read, then refetch everything like a reconnect. */
  private resyncForIdentity(): void {
    notifications.resetForIdentity();
    activity.reset();
    proof.identityChanged();
    this.resyncAfterReconnect();
  }

  private resyncStores(): void {
    // Every liveQuery refetches (coalesced, staggered) — registered views
    // resync for free.
    liveEvents.resync();
    void activity.resync();
    if (ws.currentId) void activity.loadSummary(ws.currentId);
    // Nothing to resync in a document that never opened the Swarm page.
    void swarmStore.peek()?.resync();
    transcript.resyncVisible();
    missionControlBus.resync();
    designBus.resync();
    designAssistBus.resync();
    void ws.refreshSessions().catch(() => {
      /* transient — the next reconnect or workspace switch retries */
    });
    void ws.refreshActiveWorkflowRuns();
    // refreshOtherSessions only SEEDS statuses (it must not clobber fresher
    // event-fed values on a normal refresh); after a gap the fetched rows are
    // the freshest truth, so apply them.
    void ws.refreshOtherSessions().then(() => {
      for (const s of ws.otherWsSessions) ws.statusMap[s.id] = s.status;
    });
    void notifications.load();
    assistant.resync();
    // Live room messages append straight from events — a gap needs a re-read.
    // Only when the store is loaded: nothing to resync if no room view opened.
    personalAgentsStore.peek()?.resyncRooms();
  }

  /** The daemon's boot id from `hello_ack`: a different one than before
   *  means it restarted — re-read /meta so the alias host (and the rest of
   *  `auth.meta`) reflects the NEW daemon (r3-04-01 / r3-10-09). */
  private bootId: string | null = null;
  private noteBoot(boot: string | undefined, restore?: UiHelloAckFrame['boot_restore']): void {
    if (!boot) return;
    const changed = this.bootId !== null && this.bootId !== boot;
    this.bootId = boot;
    if (!changed) return;
    void auth.refreshMeta();
    // Every DB pool/tunnel died with the old daemon: mark open connections
    // stale and re-warm them (selected first) instead of showing "ready".
    // Only a document that loaded the DB store has connections to re-warm.
    databaseStore.peek()?.onDaemonRestart();
    // "Otto restarted — N kept running · M suspended" (A4), once per boot id
    // across every window (the key is shared; storage may be unavailable).
    const text = restartSummaryText(restore);
    if (!text) return;
    try {
      if (localStorage.getItem(RESTART_TOAST_KEY) === boot) return;
      localStorage.setItem(RESTART_TOAST_KEY, boot);
    } catch {
      /* storage blocked: this window still says it once */
    }
    toasts.info('Otto restarted', text);
  }

  private scheduleLagResync(): void {
    if (this.lagResyncTimer) clearTimeout(this.lagResyncTimer);
    this.lagResyncTimer = setTimeout(() => {
      this.lagResyncTimer = null;
      this.resyncAfterReconnect();
    }, 500);
  }

  private connect(): void {
    if (this.stopped) return;
    this.wirePresence();
    this.state = 'connecting';
    // The bearer THIS socket authenticated with (its 4401 is about that one).
    let sockToken: string | null = null;
    try {
      // Bearer token travels in Sec-WebSocket-Protocol, not the URL query.
      this.socketToken = getToken();
      sockToken = this.socketToken;
      this.sock = wsConnect('/ws/events');
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.sock.onopen = () => {
      // Events that fired while the socket was down are gone for good — on a
      // RE-connect (daemon restart, sleep/wake, network blip) resync the
      // event-driven stores so boards/counts don't stay frozen at the last
      // delivered event. First connect skips it: pages load their own data.
      const reconnected = this.everConnected;
      this.everConnected = true;
      this.state = 'connected';
      this.backoff = 1000;
      liveEvents.setConnected(true);
      this.sendHello();
      // The daemon answers again: probe the alias host now (it was suspended
      // on close). A RESTARTED daemon is re-read from /meta on `hello_ack`.
      if (reconnected) resumeAltLoopback();
      const identityChanged = this.identityPending;
      this.identityPending = false;
      if (identityChanged) this.resyncForIdentity();
      else if (reconnected) this.resyncAfterReconnect();
      // The Assistant's needs-you badge lives in the sidebar, so it loads on
      // first connect too (quietly: an older daemon without the route → no badge)
      // — unless a page's own first load (Home's Today) already has it.
      else if (!['loading', 'ready'].includes(assistant.needsState)) void assistant.loadNeedsYou();
    };
    this.sock.onmessage = (ev: MessageEvent) => {
      if (typeof ev.data !== 'string') return;
      let data: unknown;
      try {
        data = JSON.parse(ev.data);
      } catch {
        return; // malformed frame — ignore
      }
      try {
        // Per-connection `resync` (ws.md): the daemon's bounded bus dropped
        // events for this socket — refetch like after a reconnect.
        if ((data as Partial<EventsResyncFrame> | null)?.type === 'resync') {
          this.scheduleLagResync();
          return;
        }
        if ((data as Partial<UiHelloAckFrame> | null)?.type === 'hello_ack') {
          this.noteBoot((data as UiHelloAckFrame).boot_id, (data as UiHelloAckFrame).boot_restore);
        }
        // Per-connection UI-control frames (hello_ack / ui_command /
        // ui_command_cancel) never reach the event stores.
        if (handleUiFrame(data)) return;
        const parsed = data as OttoEvent;
        // Generic subscribers first (liveQuery views), then the store route.
        liveEvents.dispatch(parsed as unknown as LiveEvent);
        dispatchEvent(parsed);
      } catch (e) {
        // A store handler threw (S13-06): this used to vanish with the
        // "malformed frame" catch, hiding contract drift (a field the TS
        // union calls required, missing on the wire). Log it, and report it
        // once per event type to the daemon log.
        reportHandlerError((data as { type?: unknown } | null)?.type, e);
      }
    };
    this.sock.onclose = (ev?: CloseEvent) => {
      this.state = 'offline';
      liveEvents.setConnected(false);
      // 4401 = the daemon re-validated this socket's credential and it no
      // longer verifies (logout elsewhere, "revoke all", expired
      // impersonation — ws.md). Hand it to the 401 path WITH this socket's
      // token (S13-302): an expired impersonation falls back to the admin's
      // session; any other rejected token lands on sign-in. A plain re-boot
      // signed the admin out instead.
      if (ev?.code === 4401) {
        if (sockToken && auth.phase === 'ready') void auth.handleUnauthorized(sockToken);
        else void auth.boot(true);
      }
      // The daemon may be restarting, and the next one may not hold the
      // alias: stop using it until the socket is back (resume / re-arm).
      suspendAltLoopback();
      uiSocketClosed();
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
