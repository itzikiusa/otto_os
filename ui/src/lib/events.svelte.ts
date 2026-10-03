// Events WS client (/ws/events) with auto-reconnect + exponential backoff.
// Feeds the workspace store (session statuses) and the toast store (notices).

import { resumeAltLoopback, suspendAltLoopback, wsConnect } from './api/client';
import { inLane } from './api/lane';
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
import { git } from './stores/git.svelte';
import { assistant } from './stores/assistant.svelte';
import { uiControl } from './stores/uiControl.svelte';
import { lazyModule } from './lazyModule';

// Page-owned stores load on their FIRST event (perf F2): statically importing
// all of them made every document — main window, pop-out, side pane — parse
// and evaluate the database / apiClient / product / k8s / … stores before the
// shell could paint. The shell-owned stores above stay static (the sidebar,
// status bar and session view read them from the first frame).
//
// Most handlers `peek()` (perf G2/H1): a page imports its store statically
// and loads its own data on mount, so a document that never opened the page
// has nothing to patch — `use()` there imported the store into every window
// and, for API history / Run with Otto / AWS, fetched per event. `use()` is
// kept only where the event's payload must survive until the page mounts
// (an agent session or proposal to attach); `unit/lazyModule.test.ts` holds
// that allow-list.
const swarmStore = lazyModule(() => import('./stores/swarm.svelte').then((m) => m.swarm), 'swarm');
const loopsStore = lazyModule(() => import('./stores/loops.svelte').then((m) => m.loops), 'loops');
const usageStore = lazyModule(() => import('./api/usage.svelte').then((m) => m.usage), 'usage');
const productStore = lazyModule(() => import('./stores/product.svelte').then((m) => m.product), 'product');
const canvasStore = lazyModule(() => import('./stores/canvas.svelte').then((m) => m.canvas));
const mockupAssistStore = lazyModule(() => import('./stores/mockup-assist.svelte').then((m) => m.mockupAssist));
const databaseStore = lazyModule(() => import('./stores/database.svelte').then((m) => m.database), 'database');
const scheduledTasksStore = lazyModule(() => import('./stores/scheduledTasks.svelte').then((m) => m.scheduledTasks), 'scheduledTasks');
const runWithOttoStore = lazyModule(() => import('./stores/runWithOtto.svelte').then((m) => m.runWithOtto), 'runWithOtto');
const browserStore = lazyModule(() => import('./stores/browser.svelte').then((m) => m.browser), 'browser');
const browserLiveStore = lazyModule(() => import('./stores/browserLive.svelte').then((m) => m.browserLive), 'browserLive');
const personalAgentsStore = lazyModule(() => import('./stores/personalAgents.svelte').then((m) => m.personalAgents), 'personalAgents');
const k8sStore = lazyModule(() => import('./stores/k8s.svelte').then((m) => m.k8s), 'k8s');
const awsStore = lazyModule(() => import('./stores/aws.svelte').then((m) => m.aws), 'aws');
const apiClientStore = lazyModule(() => import('./stores/apiClient.svelte').then((m) => m.apiClient), 'apiClient');
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

export type EventsState = 'connecting' | 'connected' | 'offline';

/** Generic subscription registry over this window's `/ws/events` stream
 *  (TRANSPORT_PLAN stage 2; `appLive` from lib/live.ts, fed here):
 *  `liveEvents.on(types, fn)` next to the
 *  hard-coded store dispatch below, plus `onResync` (reconnect / lag) and
 *  `onConnection`. `liveQuery` (lib/live.ts) uses it as its default source, so
 *  a new event-fed view never has to be added to `resyncAfterReconnect`. */
export const liveEvents = appLive;

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

  start(): void {
    this.stopped = false;
    this.connect();
  }

  stop(): void {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    uiSocketClosed();
    this.sock?.close();
    this.sock = null;
    this.state = 'offline';
    liveEvents.setConnected(false);
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

  private resyncStores(): void {
    // Every liveQuery refetches (coalesced, staggered) — registered views
    // resync for free.
    liveEvents.resync();
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
    try {
      // Bearer token travels in Sec-WebSocket-Protocol, not the URL query.
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
      if (reconnected) this.resyncAfterReconnect();
      // The Assistant's needs-you badge lives in the sidebar, so it loads on
      // first connect too (quietly: an older daemon without the route → no badge).
      else void assistant.loadNeedsYou();
    };
    this.sock.onmessage = (ev: MessageEvent) => {
      if (typeof ev.data !== 'string') return;
      try {
        const data: unknown = JSON.parse(ev.data);
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
        // Generic subscribers first (liveQuery views); the store dispatch
        // below is unchanged.
        liveEvents.dispatch(parsed as unknown as LiveEvent);
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
        } else if (parsed.type === 'api_history_appended') {
          // API Client history is shared across human sends and agent MCP runs.
          // Refresh once after a burst so the current workspace stays live.
          // Automation runs emit one per step / row: only a loaded store
          // with a loaded history list refetches.
          apiClientStore.peek()?.noteHistoryAppended(parsed.workspace_id, parsed.entry_id);
        } else if (parsed.type === 'api_run_progress') {
          // A running automation finished a step: fetch its delta now
          // instead of waiting for the fallback poll.
          apiClientStore.peek()?.noteRunProgress(parsed.run_id, parsed.status);
        } else if (parsed.type === 'api_client_changed') {
          // Requests / collections / envs / automations changed (maybe by an
          // agent): keep the API store's 60 s reuse window honest. A store
          // that never loaded has nothing stale — don't load the chunk for it.
          apiClientStore.peek()?.noteClientChanged(parsed);
        } else if (
          parsed.type === 'swarm_run_updated' ||
          parsed.type === 'swarm_task_updated' ||
          parsed.type === 'swarm_project_cleared' ||
          parsed.type === 'swarm_message_posted' ||
          parsed.type === 'swarm_goal_updated' ||
          parsed.type === 'swarm_status'
        ) {
          // The Swarm page loads its swarms on mount: a document without the
          // store has nothing to patch.
          swarmStore.peek()?.applyEvent(parsed);
        } else if (parsed.type === 'usage_metrics_tick') {
          // Drive a near-real-time metrics sparkline refresh without polling.
          // Broadcast on every sampler tick; only an open metrics view wants it.
          usageStore.peek()?.applyMetricsTick();
        } else if (parsed.type === 'product_changed') {
          // Let product section tabs know a run completed (kills a poll cycle).
          productStore.peek()?.applyEvent(parsed);
        } else if (parsed.type === 'plan_run') {
          // Multi-agent plan kickoff: the Plan tab tiles the live sessions.
          productStore.peek()?.applyPlanRun(parsed);
        } else if (parsed.type === 'improvement_updated') {
          // Let the Self-Improvement pane refresh without waiting for its poll.
          improvementBus.apply(parsed.kind, parsed.id);
        } else if (parsed.type === 'workflow_run_updated') {
          // Workflow execution progress: node-start / node-finish / approval
          // pause/resume / run complete. The Workflows page subscribes to
          // workflowRunBus and applies the change to the viewed run (in place
          // when the event carries the changed node, else a rev-guarded GET).
          workflowRunBus.apply(parsed);
          // Keep the "Running" sidebar list + nav count live — in place from
          // the event's progress fields; a full refetch only for runs the list
          // doesn't know yet (or events from an older daemon).
          ws.applyWorkflowRunEvent(parsed);
        } else if (parsed.type === 'skill_eval_updated') {
          // Skill-Eval terminal notification (done/error/cancelled).
          skillEvalBus.apply(parsed.workspace_id, parsed.run_id, parsed.status);
        } else if (parsed.type === 'skill_review_updated') {
          // Skills Lab review advanced — refresh the matching review.
          skillReviewBus.apply(parsed.workspace_id, parsed.review_id, parsed.status);
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
          loopsStore.peek()?.applyEvent(parsed);
        } else if (parsed.type === 'canvas_updated') {
          // Live canvas edits: the open Canvas editor re-renders the matching scene.
          canvasDocBus.apply(parsed.scene_id, parsed.doc);
        } else if (parsed.type === 'canvas_session_started') {
          // The agent session is live (turn start) → attach its shell immediately
          // by setting the open scene's session id.
          canvasStore.use((canvas) => {
            if (parsed.scene_id === canvas.currentId) canvas.sessionId = parsed.session_id;
          });
        } else if (parsed.type === 'canvas_refs_changed') {
          // A scene was attached/detached to a session — the session's Canvas
          // panel refetches when its session id matches.
          canvasRefsBus.apply(parsed.session_id);
        } else if (parsed.type === 'repo_status_changed') {
          // A watched repo changed on disk: the Git page re-reads its status.
          git.applyRepoChanged(parsed.repo_id, parsed.paths);
        } else if (parsed.type === 'mockup_updated') {
          // Live design edits: the arena's Assistant preview + the open artifact
          // reload. `content` is an explicit null for binary / oversized payloads
          // (a glb, a Blender render) — the store then re-fetches the bytes rather
          // than treating "null" as an empty document.
          mockupAssistStore.use((mockupAssist) =>
            mockupAssist.ingestLive(parsed.attachment_id, parsed.story_id, parsed.format, parsed.content ?? null),
          );
        } else if (
          parsed.type === 'design_artifact_updated' ||
          parsed.type === 'design_link_updated' ||
          parsed.type === 'design_learning_update'
        ) {
          // Design Hall graph: the lobby, the open artifact, its Links panel and
          // the learning log each re-fetch what the event touches.
          designBus.apply(parsed);
        } else if (parsed.type === 'design_assist_updated' || parsed.type === 'design_variants_ready') {
          // Design assist: the Otto panel's turn states + the variants tray.
          designAssistBus.apply(parsed);
        } else if (parsed.type === 'mockup_session_started') {
          // The mockup agent session is live (turn start) → attach its shell.
          mockupAssistStore.use((mockupAssist) =>
            mockupAssist.setSession(parsed.attachment_id, parsed.story_id, parsed.session_id),
          );
        } else if (parsed.type === 'db_assist_session_started') {
          // The DB Assistant agent session is live (turn start) → attach its shell
          // in the embedded DB Assistant panel (beside the query editor).
          databaseStore.use((database) =>
            database.setAssistSession(parsed.assist_id, parsed.connection_id, parsed.session_id),
          );
        } else if (parsed.type === 'db_assist_updated') {
          // Live proposed SQL/note from the DB Assistant agent → the panel's
          // read-only SQL block (Insert into editor / Run).
          databaseStore.use((database) =>
            database.applyAssistUpdate(parsed.assist_id, parsed.connection_id, parsed.sql, parsed.note),
          );
        } else if (parsed.type === 'proof_pack_updated') {
          // Proof page list/detail + sidebar proof chips refresh on the event.
          proof.applyEvent(parsed);
        } else if (parsed.type === 'scheduled_task_run_updated') {
          // Scheduled Tasks page refreshes the affected task's runs + list status.
          scheduledTasksStore.peek()?.applyEvent(parsed);
        } else if (parsed.type === 'otto_run_updated') {
          // Run with Otto page refreshes the affected run + the workspace list.
          runWithOttoStore.peek()?.applyEvent(parsed);
        } else if (
          parsed.type === 'browser_tab_updated' ||
          parsed.type === 'browser_annotation_added'
        ) {
          // Browser page: tab strip / annotation list refresh in place.
          browserStore.peek()?.applyEvent(parsed);
        } else if (parsed.type === 'browser_engine_install_updated') {
          // Browser page / Settings → Browser: Chromium download progress.
          browserLiveStore.peek()?.applyEvent(parsed);
        } else if (
          parsed.type === 'assistant_turn' ||
          parsed.type === 'assistant_task_update' ||
          parsed.type === 'assistant_needs_you' ||
          parsed.type === 'assistant_limit'
        ) {
          // Assistant: thread turns/cards, task board, needs-you badge, limits.
          assistant.applyEvent(parsed);
        } else if (parsed.type === 'personal_agent_run_updated') {
          // Personal Agents page refreshes the agent's runs + schedule cursors.
          personalAgentsStore.peek()?.applyRunEvent(parsed);
        } else if (parsed.type === 'agent_room_message') {
          // Agent-room feeds append the event's message (open Rooms view only).
          personalAgentsStore.peek()?.applyRoomEvent(parsed);
        } else if (
          parsed.type === 'k8s_cluster_updated' ||
          parsed.type === 'k8s_install_updated' ||
          parsed.type === 'k8s_monitor_cycle'
        ) {
          // Kubernetes console: cluster list refetch / installer state tick.
          // The console and the Home box load both on mount: a document
          // without the store has nothing to refresh.
          k8sStore.peek()?.applyEvent(parsed);
        } else if (parsed.type === 'aws_account_updated' || parsed.type === 'aws_install_updated') {
          // AWS console: account rows changed / the CLI installer advanced.
          awsStore.peek()?.applyEvent(parsed);
        } else if (
          parsed.type === 'transcript_appended' ||
          parsed.type === 'transcript_live' ||
          parsed.type === 'artifact_added' ||
          parsed.type === 'history_index_progress'
        ) {
          // Conversation view: live tail deltas (+ artifact chips) for the open
          // session's chat; History page index progress. The activity store
          // also takes the artifact / index events (Outputs panel + rescan bar).
          transcript.applyEvent(parsed);
          if (parsed.type !== 'transcript_appended' && parsed.type !== 'transcript_live') activity.applyEvent(parsed);
        } else if (parsed.type === 'ui_control_requested') {
          // An agent asked to drive the UI: the session's pane shows the prompt.
          uiControl.applyEvent(parsed);
        } else {
          if (parsed.type === 'session_removed') activity.forget(parsed.session_id);
          // The grant lives in session meta; a removed session drops its prompt.
          if (parsed.type === 'session_meta_updated' || parsed.type === 'session_removed') uiControl.applyEvent(parsed);
          ws.applyEvent(parsed);
        }
      } catch {
        /* malformed frame — ignore */
      }
    };
    this.sock.onclose = () => {
      this.state = 'offline';
      liveEvents.setConnected(false);
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
