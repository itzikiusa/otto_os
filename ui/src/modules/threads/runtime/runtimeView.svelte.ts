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
import { appLive, onLive } from '../../../lib/live';
import { loadErrorText } from '../../../lib/loadError';
import type { RuntimeEvent, RuntimeReplay } from '../../../lib/api/types';
import { applyEvents, applyReplay, emptyProjection, type RuntimeProjection } from './projection';

interface View {
  refs: number;
  /** Live events held while a replay is in flight; null when none is. */
  held: RuntimeEvent[] | null;
}

class RuntimeViews {
  /** Open threads' projections, by session id. */
  projections = $state<Record<string, RuntimeProjection>>({});
  /** A failed first load of an open thread, by session id. */
  loadError = $state<Record<string, string>>({});
  /** A failed later resync (after a reconnect or a gap): the projection
   *  shown may be out of date, by session id. */
  staleError = $state<Record<string, string>>({});

  private views = new Map<string, View>();
  private stopLive: (() => void) | null = null;

  /** Keep `id`'s projection live while open; returns the release. */
  open(id: string): () => void {
    if (!this.stopLive) {
      const offEvent = onLive(['agent_runtime_event'], (ev) => this.onEvent(ev.event as RuntimeEvent));
      const offResync = appLive.onResync(() => {
        for (const v of this.views.keys()) void this.replay(v);
      });
      this.stopLive = () => {
        offEvent();
        offResync();
      };
    }
    const view = this.views.get(id);
    if (view) view.refs++;
    else {
      this.views.set(id, { refs: 1, held: null });
      void this.replay(id);
    }
    return () => {
      const v = this.views.get(id);
      if (!v || --v.refs > 0) return;
      this.views.delete(id);
      const { [id]: _drop, ...rest } = this.projections;
      this.projections = rest;
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

  private onEvent(ev: RuntimeEvent): void {
    const view = this.views.get(ev.session_id);
    if (!view) return;
    if (view.held) {
      view.held.push(ev);
      return;
    }
    const { projection, gap } = applyEvents(this.projections[ev.session_id] ?? emptyProjection(), [ev]);
    this.projections[ev.session_id] = projection;
    if (gap) void this.replay(ev.session_id);
  }

  private async replay(id: string): Promise<void> {
    const view = this.views.get(id);
    if (!view || view.held) return;
    view.held = [];
    const after = this.projections[id]?.lastSequence ?? 0;
    try {
      const replay = await api.get<RuntimeReplay>(`/runtime-sessions/${id}/events?after_sequence=${after}`);
      if (this.views.get(id) !== view) return;
      const base = applyReplay(this.projections[id] ?? emptyProjection(), replay);
      const { projection, gap } = applyEvents(base, view.held);
      this.projections[id] = projection;
      const { [id]: _ok, ...rest } = this.loadError;
      this.loadError = rest;
      if (this.staleError[id]) {
        const { [id]: _fresh, ...stale } = this.staleError;
        this.staleError = stale;
      }
      view.held = null;
      if (gap) void this.replay(id);
    } catch (e) {
      view.held = null;
      if (!this.projections[id]) this.loadError = { ...this.loadError, [id]: loadErrorText(e) };
      else this.staleError = { ...this.staleError, [id]: loadErrorText(e) };
    }
  }

  /** Reload a thread whose first load (or a later resync) failed. */
  retry(id: string): void {
    void this.replay(id);
  }
}

export const runtimeViews = new RuntimeViews();
