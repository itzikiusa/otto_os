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
import { inLane } from '../../../lib/api/lane';
import { appLive, onLive } from '../../../lib/live';
import { loadErrorText } from '../../../lib/loadError';
import { pollWhileVisible } from '../../../lib/poll';
import type { RuntimeEvent, RuntimeReplay } from '../../../lib/api/types';
import { applyEvents, applyReplay, emptyProjection, type RuntimeProjection } from './projection';
import { runtimeThreads } from './runtimeThreads.svelte';

const RETRY_STEPS_MS = [1_500, 3_000, 6_000, 15_000] as const;
const ACTIVE_LIVENESS_MS = 4_000;
const IDLE_LIVENESS_MS = 15_000;

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
  /** Consecutive failed replay count for bounded backoff. */
  failures: number;
  retryTimer: ReturnType<typeof setTimeout> | null;
}

export class RuntimeViews {
  /** Open threads' projections, by session id. */
  projections = $state<Record<string, RuntimeProjection>>({});
  /** A failed first load of an open thread, by session id. */
  loadError = $state<Record<string, string>>({});
  /** A failed later resync (after a reconnect or a gap): the projection
   *  shown may be out of date, by session id. */
  staleError = $state<Record<string, string>>({});

  private views = new Map<string, View>();
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
    const applied = this.projections[id]?.lastSequence ?? 0;
    if (lastSequence > applied) void this.replay(id, true);
  }

  /** Keep `id`'s projection live while open; returns the release. */
  open(id: string): () => void {
    if (!this.stopLive) this.startListeners();
    const view = this.views.get(id);
    if (view) view.refs++;
    else {
      this.views.set(id, {
        refs: 1,
        held: null,
        dirty: false,
        knownSequence: 0,
        lastLiveAt: Date.now(),
        failures: 0,
        retryTimer: null,
      });
      void this.replay(id, false);
    }
    return () => {
      const v = this.views.get(id);
      if (!v || --v.refs > 0) return;
      if (v.retryTimer) clearTimeout(v.retryTimer);
      this.views.delete(id);
      const { [id]: _drop, ...rest } = this.projections;
      this.projections = rest;
      if (this.loadError[id]) {
        const { [id]: _err, ...errs } = this.loadError;
        this.loadError = errs;
      }
      if (this.staleError[id]) {
        const { [id]: _gone, ...stale } = this.staleError;
        this.staleError = stale;
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
    const onWake = () => this.replayAll(true);
    if (typeof document !== 'undefined') document.addEventListener('visibilitychange', onVisible);
    if (typeof window !== 'undefined') {
      window.addEventListener('focus', onWake);
      window.addEventListener('online', onWake);
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
        void this.replay(id, true);
      } else if (now - view.lastLiveAt >= IDLE_LIVENESS_MS) {
        view.lastLiveAt = now;
        void this.replay(id, true);
      }
    }
  }

  private onEvent(ev: RuntimeEvent): void {
    const view = this.views.get(ev.session_id);
    if (!view) return;
    view.lastLiveAt = Date.now();
    if (ev.sequence > view.knownSequence) view.knownSequence = ev.sequence;
    if (view.held) {
      view.held.push(ev);
      return;
    }
    const { projection, gap } = applyEvents(this.projections[ev.session_id] ?? emptyProjection(), [ev]);
    this.projections[ev.session_id] = projection;
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

  private async replay(id: string, bg = false): Promise<void> {
    const view = this.views.get(id);
    if (!view) return;
    if (view.retryTimer) {
      clearTimeout(view.retryTimer);
      view.retryTimer = null;
    }
    if (view.held) {
      view.dirty = true;
      return;
    }
    view.held = [];
    view.dirty = false;
    const after = this.projections[id]?.lastSequence ?? 0;
    const path = `/runtime-sessions/${id}/events?after_sequence=${after}`;
    try {
      const replay = await (bg ? inLane('bg', () => api.get<RuntimeReplay>(path)) : api.get<RuntimeReplay>(path));
      if (this.views.get(id) !== view) return;
      const base = applyReplay(this.projections[id] ?? emptyProjection(), replay);
      const { projection, gap } = applyEvents(base, view.held);
      this.projections[id] = projection;
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
      const rerun = gap || view.dirty || view.knownSequence > projection.lastSequence;
      view.held = null;
      view.dirty = false;
      if (rerun) void this.replay(id, true);
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
      if (!this.projections[id]) this.loadError = { ...this.loadError, [id]: loadErrorText(e) };
      else this.staleError = { ...this.staleError, [id]: loadErrorText(e) };
      this.scheduleRetry(id, view);
    }
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
