// An open runtime thread's conversation: its folded projection (projection.ts)
// kept live from `agent_runtime_event` while a view holds it. Loaded with the
// view (RuntimeThread), not with the Threads page.
//
// Sync follows T3 Code's thread subscription (MIT, Copyright (c) 2026 T3
// Tools Inc.; packages/client-runtime/src/state/threads.ts): load, then fold
// live events above the applied sequence; after a reconnect or a sequence
// gap, replay `after_sequence=<applied>` (events, or a snapshot when the
// daemon's ring no longer covers it). Live events that land while a replay
// is in flight are held and folded after it.

import { api } from '../../../lib/api/client';
import { perfCount } from '../../../lib/perfCount';
import { inLane } from '../../../lib/api/lane';
import { appLive, onLive } from '../../../lib/live';
import { loadErrorText } from '../../../lib/loadError';
import { pollWhileVisible } from '../../../lib/poll';
import { startMeasurement, type Measurement } from '../../../lib/telemetry';
import type {
  RuntimeEvent,
  RuntimeReplay,
  RuntimeTurnItemDetailResponse,
  TurnItem,
} from '../../../lib/api/types';
import {
  applyEvents,
  applyReplay,
  emptyProjection,
  mergeHistoryPage,
  replaceHydratedItem,
  type RuntimeProjection,
} from './projection';
import { runtimeThreads } from './runtimeThreads.svelte';
import { runtimeDrafts } from '../../../lib/stores/runtimeDrafts.svelte';

export const TAIL_TURNS = 40;
export const MOBILE_TAIL_TURNS = 12;
export const TAIL_MAX_LINES = 10_000;

function defaultTailTurns(): number {
  if (typeof window !== 'undefined' && typeof window.innerWidth === 'number' && window.innerWidth > 0 && window.innerWidth < 768) {
    return MOBILE_TAIL_TURNS;
  }
  return TAIL_TURNS;
}

const RETRY_STEPS_MS = [1_500, 3_000, 6_000, 15_000] as const;
const ACTIVE_LIVENESS_MS = 4_000;
const IDLE_LIVENESS_MS = 15_000;
const LIVE_WS_FRESH_MS = 2_500;
const STREAM_BATCH_MS = 32;
/** Catch-up when the daemon announced a sequence its events endpoint does not
 *  deliver (a dropped event, a restart mid-turn): back off 250 ms doubling to
 *  4 s while no progress is made, then give up on events and reload one
 *  snapshot, whose sequence is taken as the truth. */
const CHASE_BASE_MS = 250;
const CHASE_MAX_MS = 4_000;
export const CHASE_MAX_ATTEMPTS = 5;

function isCoalesceableStreamEvent(ev: RuntimeEvent): boolean {
  if (ev.type === 'command_output.delta') return true;
  if (ev.type === 'turn_item.updated') {
    const it = ev.turn_item;
    return Boolean(('streaming' in it && it.streaming) || it.status === 'running');
  }
  return false;
}

interface View {
  refs: number;
  /** Live events held while a replay is in flight; null when none is. */
  held: RuntimeEvent[] | null;
  /** Set when another replay was requested while `held !== null`. */
  dirty: boolean;
  /** Highest sequence reported on the session row (`session.last_sequence`). */
  knownSequence: number;
  /** Epoch ms of the latest live WS event or idle liveness check. */
  lastLiveAt: number;
  /** Epoch ms of the latest actual WS `agent_runtime_event` (`0` before first). */
  lastWsEventAt: number;
  /** Epoch ms when live WS events were last folded into `projections[id]`. */
  lastLiveFlushAt: number;
  /** Queued streaming deltas coalesced into one ~32ms frame fold. */
  pendingLive: RuntimeEvent[];
  liveFlushTimer: ReturnType<typeof setTimeout> | null;
  /** Consecutive failed replay count for bounded backoff. */
  failures: number;
  retryTimer: ReturnType<typeof setTimeout> | null;
  /** Catch-up fetches in a row that applied nothing new. */
  chase: number;
  chaseTimer: ReturnType<typeof setTimeout> | null;
  /** The next replay asks for a snapshot (`after_sequence=0`). */
  forceSnapshot: boolean;
  openSpan: Measurement | null;
}

class ProjectionBox {
  v = $state.raw<RuntimeProjection | undefined>(undefined);
}

/** `Record<id, projection>` over one raw signal per thread. */
function projectionRecord(): Record<string, RuntimeProjection> {
  const boxes = new Map<string, ProjectionBox>();
  // A thread's box is made by the first write, never by a read: a signal made
  // inside a reader (a $derived) would not be one of its dependencies. A
  // reader of a thread with no box yet waits on `added` instead.
  const added = new ProjectionBox();
  let n = 0;
  const box = (id: string): ProjectionBox => {
    let b = boxes.get(id);
    if (!b) {
      boxes.set(id, (b = new ProjectionBox()));
      added.v = { n: ++n } as unknown as RuntimeProjection;
    }
    return b;
  };
  const live = (k: string | symbol): k is string => typeof k === 'string' && boxes.get(k)?.v !== undefined;
  return new Proxy({} as Record<string, RuntimeProjection>, {
    get: (_t, k) => {
      if (typeof k !== 'string') return undefined;
      const b = boxes.get(k);
      if (!b) {
        perfCount('projectionMissingRead');
        void added.v;
        return undefined;
      }
      return b.v;
    },
    set: (_t, k, v: RuntimeProjection | undefined) => {
      if (typeof k === 'string') box(k).v = v;
      return true;
    },
    deleteProperty: (_t, k) => {
      if (typeof k === 'string' && boxes.has(k)) box(k).v = undefined;
      return true;
    },
    has: (_t, k) => live(k),
    ownKeys: () => [...boxes.keys()].filter(live),
    getOwnPropertyDescriptor: (_t, k) => (live(k) ? { enumerable: true, configurable: true, writable: true, value: boxes.get(k)!.v } : undefined),
  });
}

export class RuntimeViews {
  /** Open threads' projections, by session id. Each thread's projection is
   *  its own raw signal (ProjectionBox): a streamed chunk wakes only what
   *  reads THAT thread, and no deep proxy wraps every item a reader touches
   *  (projections are replaced, never mutated in place — projection.ts). */
  projections: Record<string, RuntimeProjection> = projectionRecord();
  /** A failed first load of an open thread, by session id. */
  loadError = $state<Record<string, string>>({});
  /** A failed later resync (after a reconnect or a gap): the projection
   *  shown may be out of date, by session id. */
  staleError = $state<Record<string, string>>({});
  /** In-flight older-history pagination requests, by session id. */
  loadingOlder = $state<Record<string, boolean>>({});
  /** Failed older-history pagination error text, by session id. */
  olderError = $state<Record<string, string>>({});
  /** In-flight single-item detail hydration (`${sessionId}:${itemId}`). */
  loadingItem = $state<Record<string, boolean>>({});
  /** Failed single-item detail hydration error text (`${sessionId}:${itemId}`). */
  itemError = $state<Record<string, string>>({});

  private views = new Map<string, View>();
  private inFlightItems = new Map<string, Promise<TurnItem | null>>();
  private stopLive: (() => void) | null = null;
  private activeProbe: ((id: string) => boolean) | null = null;

  /** Optional hook from runtimeThreads so active/background-waiting sessions
   *  get 4 s liveness catch-up without introducing a module cycle. */
  setSessionActiveProbe(probe: (id: string) => boolean): void {
    this.activeProbe = probe;
  }

  /** Cross-check `session.last_sequence` whenever session rows load or update:
   *  any unapplied sequence immediately triggers a catch-up replay. */
  noteSessionSequence(id: string, lastSequence: number | null | undefined): void {
    if (typeof lastSequence !== 'number' || lastSequence <= 0) return;
    const view = this.views.get(id);
    if (!view) return;
    if (lastSequence > view.knownSequence) view.knownSequence = lastSequence;
    this.flushPendingLive(id, view);
    const applied = this.projections[id]?.lastSequence ?? 0;
    if (lastSequence > applied) void this.replay(id, true);
  }

  /** Keep `id`'s projection live while open; returns the release. The
   *  thread's composer draft syncs only while a view holds it too. */
  open(id: string): () => void {
    if (!this.stopLive) this.startListeners();
    const releaseDraft = runtimeDrafts.hold(id);
    const view = this.views.get(id);
    if (view) view.refs++;
    else {
      this.views.set(id, {
        refs: 1,
        held: null,
        dirty: false,
        knownSequence: 0,
        lastLiveAt: Date.now(),
        lastWsEventAt: 0,
        lastLiveFlushAt: 0,
        pendingLive: [],
        liveFlushTimer: null,
        failures: 0,
        retryTimer: null,
        chase: 0,
        chaseTimer: null,
        forceSnapshot: false,
        openSpan: startMeasurement('ui.thread.open', 'threads'),
      });
      void this.replay(id, false);
    }
    return () => {
      releaseDraft();
      const v = this.views.get(id);
      if (!v || --v.refs > 0) return;
      if (v.openSpan) {
        v.openSpan.finish('cancelled');
        v.openSpan = null;
      }
      if (v.retryTimer) clearTimeout(v.retryTimer);
      if (v.chaseTimer) clearTimeout(v.chaseTimer);
      if (v.liveFlushTimer) clearTimeout(v.liveFlushTimer);
      v.pendingLive.length = 0;
      this.views.delete(id);
      delete this.projections[id];
      if (this.loadError[id]) {
        const { [id]: _err, ...errs } = this.loadError;
        this.loadError = errs;
      }
      if (this.staleError[id]) {
        const { [id]: _gone, ...stale } = this.staleError;
        this.staleError = stale;
      }
      if (this.loadingOlder[id]) {
        const { [id]: _old, ...older } = this.loadingOlder;
        this.loadingOlder = older;
      }
      if (this.olderError[id]) {
        const { [id]: _oerr, ...olderErrs } = this.olderError;
        this.olderError = olderErrs;
      }
      const prefix = `${id}:`;
      for (const k of this.inFlightItems.keys()) {
        if (k.startsWith(prefix)) this.inFlightItems.delete(k);
      }
      if (Object.keys(this.loadingItem).some((k) => k.startsWith(prefix))) {
        const next: Record<string, boolean> = {};
        for (const [k, val] of Object.entries(this.loadingItem)) {
          if (!k.startsWith(prefix)) next[k] = val;
        }
        this.loadingItem = next;
      }
      if (Object.keys(this.itemError).some((k) => k.startsWith(prefix))) {
        const nextErr: Record<string, string> = {};
        for (const [k, val] of Object.entries(this.itemError)) {
          if (!k.startsWith(prefix)) nextErr[k] = val;
        }
        this.itemError = nextErr;
      }
      if (this.views.size === 0) {
        this.stopLive?.();
        this.stopLive = null;
      }
    };
  }

  private startListeners(): void {
    const offEvent = onLive(['agent_runtime_event'], (ev) => this.onEvent(ev.event as RuntimeEvent));
    const offResync = appLive.onResync(() => this.replayAll(true));
    // Catch events emitted before initial `/ws/events` `onopen` (when `reconnected === false`).
    const offConn = appLive.onConnection((up) => {
      if (up) this.replayAll(true);
    });
    const onVisible = () => {
      if (typeof document === 'undefined' || !document.hidden) this.replayAll(true);
    };
    let lastInputWakeAt = 0;
    const onWake = () => this.replayAll(true);
    const onInputWake = () => {
      const now = Date.now();
      if (now - lastInputWakeAt < ACTIVE_LIVENESS_MS) return;
      lastInputWakeAt = now;
      for (const [id, view] of this.views.entries()) {
        if (!view.held && now - view.lastLiveAt >= ACTIVE_LIVENESS_MS) {
          void this.replay(id, true);
        }
      }
    };
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisible);
    if (typeof window !== 'undefined') {
      window.addEventListener('focus', onWake);
      window.addEventListener('online', onWake);
      window.addEventListener('pointerdown', onInputWake, { passive: true });
    }
    const liveness = pollWhileVisible(() => this.livenessTick(), {
      ms: ACTIVE_LIVENESS_MS,
      immediate: false,
      jitter: 0,
      lane: 'bg',
    });
    this.stopLive = () => {
      offEvent();
      offResync();
      offConn();
      liveness.stop();
      if (typeof document !== 'undefined') document.removeEventListener('visibilitychange', onVisible);
      if (typeof window !== 'undefined') {
        window.removeEventListener('focus', onWake);
        window.removeEventListener('online', onWake);
        window.removeEventListener('pointerdown', onInputWake);
      }
    };
  }

  private replayAll(bg: boolean): void {
    for (const id of this.views.keys()) void this.replay(id, bg);
  }

  private isViewActive(id: string): boolean {
    if (this.activeProbe?.(id)) return true;
    const p = this.projections[id];
    if (!p) return true;
    const running = p.turns.some((t) => t.status === 'pending' || t.status === 'running');
    return running || (p.session?.background_tasks?.length ?? 0) > 0;
  }

  private livenessTick(): void {
    if (this.views.size === 0) return;
    if (typeof document !== 'undefined' && document.hidden) return;
    const now = Date.now();
    for (const [id, view] of this.views.entries()) {
      if (view.held) continue;
      if (this.isViewActive(id)) {
        if (now - view.lastWsEventAt >= LIVE_WS_FRESH_MS) {
          void this.replay(id, true);
        }
      } else if (now - view.lastLiveAt >= IDLE_LIVENESS_MS) {
        view.lastLiveAt = now;
        void this.replay(id, true);
      }
    }
  }

  private flushPendingLive(id: string, view: View): void {
    if (view.liveFlushTimer) {
      clearTimeout(view.liveFlushTimer);
      view.liveFlushTimer = null;
    }
    if (view.pendingLive.length === 0) return;
    const batch = view.pendingLive.splice(0, view.pendingLive.length);
    if (view.held) {
      view.held.push(...batch);
      return;
    }
    const prev = this.projections[id];
    const { projection, gap } = applyEvents(prev ?? emptyProjection(), batch);
    view.lastLiveFlushAt = Date.now();
    if (!prev || projection !== prev) this.projections[id] = projection;
    if (gap) void this.replay(id, true);
  }

  private onEvent(ev: RuntimeEvent): void {
    const view = this.views.get(ev.session_id);
    if (!view) return;
    const now = Date.now();
    view.lastLiveAt = now;
    view.lastWsEventAt = now;
    if (ev.sequence > view.knownSequence) view.knownSequence = ev.sequence;
    if (view.held) {
      view.held.push(ev);
      return;
    }
    if (typeof requestAnimationFrame === 'function' && isCoalesceableStreamEvent(ev)) {
      if (view.pendingLive.length === 0 && now - view.lastLiveFlushAt >= STREAM_BATCH_MS) {
        const prev = this.projections[ev.session_id];
        const { projection, gap } = applyEvents(prev ?? emptyProjection(), [ev]);
        view.lastLiveFlushAt = now;
        if (!prev || projection !== prev) this.projections[ev.session_id] = projection;
        if (gap) void this.replay(ev.session_id, true);
        return;
      }
      view.pendingLive.push(ev);
      if (!view.liveFlushTimer) {
        const waitMs = Math.max(8, STREAM_BATCH_MS - (now - view.lastLiveFlushAt));
        view.liveFlushTimer = setTimeout(() => {
          view.liveFlushTimer = null;
          if (this.views.get(ev.session_id) === view) this.flushPendingLive(ev.session_id, view);
        }, waitMs);
      }
      return;
    }
    this.flushPendingLive(ev.session_id, view);
    const prev = this.projections[ev.session_id];
    const { projection, gap } = applyEvents(prev ?? emptyProjection(), [ev]);
    view.lastLiveFlushAt = now;
    if (!prev || projection !== prev) this.projections[ev.session_id] = projection;
    if (gap) void this.replay(ev.session_id, true);
  }

  private scheduleRetry(id: string, view: View): void {
    if (view.retryTimer || this.views.get(id) !== view) return;
    const delay = RETRY_STEPS_MS[Math.min(Math.max(view.failures - 1, 0), RETRY_STEPS_MS.length - 1)];
    view.retryTimer = setTimeout(() => {
      view.retryTimer = null;
      if (this.views.get(id) === view) void this.replay(id, true);
    }, delay);
  }

  /** Still behind `knownSequence` after a replay that applied nothing:
   *  retry with backoff, then once as a snapshot. */
  private scheduleChase(id: string, view: View): void {
    if (view.chaseTimer || this.views.get(id) !== view) return;
    view.chase += 1;
    if (view.chase > CHASE_MAX_ATTEMPTS) view.forceSnapshot = true;
    const n = Math.min(view.chase, CHASE_MAX_ATTEMPTS);
    const delay = Math.min(CHASE_BASE_MS * 2 ** (n - 1), CHASE_MAX_MS);
    view.chaseTimer = setTimeout(() => {
      view.chaseTimer = null;
      if (this.views.get(id) === view) void this.replay(id, true);
    }, delay);
  }

  private async replay(id: string, bg = false): Promise<void> {
    const view = this.views.get(id);
    if (!view) return;
    if (view.retryTimer) {
      clearTimeout(view.retryTimer);
      view.retryTimer = null;
    }
    if (view.chaseTimer && !view.held) {
      clearTimeout(view.chaseTimer);
      view.chaseTimer = null;
    }
    if (view.held) {
      view.dirty = true;
      return;
    }
    this.flushPendingLive(id, view);
    view.held = [];
    view.dirty = false;
    const applied = this.projections[id]?.lastSequence ?? 0;
    const snapshotReload = view.forceSnapshot;
    view.forceSnapshot = false;
    const after = snapshotReload ? 0 : applied;
    const path = `/runtime-sessions/${id}/events?after_sequence=${after}&tail_turns=${defaultTailTurns()}&max_lines=${TAIL_MAX_LINES}&compact=true`;
    try {
      const replay = await (bg ? inLane('bg', () => api.get<RuntimeReplay>(path)) : api.get<RuntimeReplay>(path));
      if (this.views.get(id) !== view) return;
      const prev = this.projections[id];
      const base = applyReplay(prev ?? emptyProjection(), replay);
      const { projection, gap } = applyEvents(base, view.held);
      if (!prev || projection !== prev) this.projections[id] = projection;
      if (view.openSpan) {
        const span = view.openSpan;
        view.openSpan = null;
        if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => span.finish('ok'));
        else span.finish('ok');
      }
      if (projection.lastSequence > view.knownSequence) view.knownSequence = projection.lastSequence;
      view.lastLiveAt = Date.now();
      view.failures = 0;
      if (this.loadError[id]) {
        const { [id]: _ok, ...rest } = this.loadError;
        this.loadError = rest;
      }
      if (this.staleError[id]) {
        const { [id]: _fresh, ...stale } = this.staleError;
        this.staleError = stale;
      }
      if (snapshotReload) {
        // The daemon's own snapshot is the truth: stop chasing what it never had.
        if (view.knownSequence > projection.lastSequence) view.knownSequence = projection.lastSequence;
        view.chase = 0;
      } else if (projection.lastSequence > applied) {
        view.chase = 0; // progress (a replay page): keep going at once
      }
      const behind = gap || view.knownSequence > projection.lastSequence;
      const again = view.dirty;
      view.held = null;
      view.dirty = false;
      if (again || (behind && view.chase === 0 && projection.lastSequence > applied)) void this.replay(id, true);
      else if (behind) this.scheduleChase(id, view);
    } catch (e) {
      if (this.views.get(id) !== view) return;
      const held = view.held ?? [];
      view.held = null;
      view.dirty = false;
      if (held.length > 0 && this.projections[id]) {
        const folded = applyEvents(this.projections[id], held);
        if (!folded.gap) this.projections[id] = folded.projection;
      }
      view.failures += 1;
      if (view.openSpan) {
        view.openSpan.finish('error');
        view.openSpan = null;
      }
      if (!this.projections[id]) this.loadError = { ...this.loadError, [id]: loadErrorText(e) };
      else this.staleError = { ...this.staleError, [id]: loadErrorText(e) };
      this.scheduleRetry(id, view);
    }
  }

  /** Fetch and prepend the next page of older turns before `oldestTurnOrdinal`. */
  async loadOlder(id: string, limitTurns = defaultTailTurns()): Promise<boolean> {
    const view = this.views.get(id);
    if (view) this.flushPendingLive(id, view);
    const cur = this.projections[id];
    if (!cur || !cur.hasOlder || cur.oldestTurnOrdinal == null || this.loadingOlder[id]) return false;
    if (this.olderError[id]) {
      const { [id]: _clr, ...restErr } = this.olderError;
      this.olderError = restErr;
    }
    this.loadingOlder = { ...this.loadingOlder, [id]: true };
    const path = `/runtime-sessions/${id}/events?before_turn_ordinal=${cur.oldestTurnOrdinal}&limit_turns=${limitTurns}&max_lines=${TAIL_MAX_LINES}&compact=true`;
    try {
      const replay = await api.get<RuntimeReplay>(path);
      if (!this.views.has(id) || replay.kind !== 'snapshot') return false;
      const latest = this.projections[id];
      if (!latest) return false;
      this.projections[id] = mergeHistoryPage(latest, replay.snapshot);
      return true;
    } catch (e) {
      if (this.views.has(id)) {
        this.olderError = { ...this.olderError, [id]: loadErrorText(e) };
      }
      return false;
    } finally {
      if (this.loadingOlder[id]) {
        const { [id]: _done, ...rest } = this.loadingOlder;
        this.loadingOlder = rest;
      }
    }
  }

  /** Lazily fetch full output/diff/input for a compacted historical `TurnItem`. */
  ensureItemDetail(id: string, itemId: string, opts?: { force?: boolean }): Promise<TurnItem | null> {
    const cur = this.projections[id]?.items.find((it) => it.id === itemId);
    if (cur && !cur.detail_omitted && !opts?.force) return Promise.resolve(cur);
    const key = `${id}:${itemId}`;
    const existing = this.inFlightItems.get(key);
    if (existing) return existing;
    if (this.itemError[key]) {
      const { [key]: _clr, ...restErr } = this.itemError;
      this.itemError = restErr;
    }
    this.loadingItem = { ...this.loadingItem, [key]: true };
    const req = api
      .get<RuntimeTurnItemDetailResponse>(`/runtime-sessions/${id}/items/${encodeURIComponent(itemId)}`)
      .then((res) => {
        const item =
          res && typeof res === 'object' && 'item' in res
            ? (res as { item: TurnItem }).item
            : res;
        const latest = this.projections[id];
        if (this.views.has(id) && latest) {
          this.projections[id] = replaceHydratedItem(latest, item);
        }
        return item;
      })
      .catch((e) => {
        if (this.views.has(id)) {
          this.itemError = { ...this.itemError, [key]: loadErrorText(e) };
        }
        return null;
      })
      .finally(() => {
        this.inFlightItems.delete(key);
        if (this.loadingItem[key]) {
          const { [key]: _done, ...rest } = this.loadingItem;
          this.loadingItem = rest;
        }
      });
    this.inFlightItems.set(key, req);
    return req;
  }

  /** Reload a thread whose first load (or a later resync) failed. */
  retry(id: string): void {
    const view = this.views.get(id);
    if (view) view.failures = 0;
    void this.replay(id, false);
  }
}

export const runtimeViews = new RuntimeViews();
runtimeViews.setSessionActiveProbe((id) => runtimeThreads.isActiveOrWaiting(id));
runtimeThreads.onSequence((id, seq) => runtimeViews.noteSessionSequence(id, seq));
