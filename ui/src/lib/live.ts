// Event-driven queries (TRANSPORT_PLAN stage 2). Most UI pollers re-read data
// the daemon already announces on `/ws/events`; each tick still cost a socket
// on the webview's shared per-host pool (see api/client.ts "Request lanes").
// `liveQuery` is the one sanctioned replacement:
//
//  - runs once, then again (coalesced, trailing-debounced) whenever a matching
//    event arrives, and after a reconnect / lag `resync` (events were lost);
//  - while the event socket is UP, keeps only a slow safety net (`safetyMs`,
//    default 5 min) in case an emit point was missed;
//  - while the socket is DOWN, falls back to the old poll cadence
//    (`fallbackMs`), so behaviour degrades to exactly what it was before;
//  - inherits every `pollWhileVisible` rule (in-flight guard, hidden pause,
//    backoff, jitter, abort on stop) — it IS a poller with a dynamic cadence.
//
// The event source is pluggable (`LiveSource`): the main window's EventsClient
// feeds `appLive`; the menu-bar tray feeds its own topic-filtered socket
// (lib/topicSocket.ts); tests pass their own registry.

import { pollWhileVisible, type Poller, type PollRun } from './poll';
import type { Lane } from './api/lane';

export interface LiveEvent {
  type: string;
  [key: string]: unknown;
}

export interface LiveSource {
  /** Subscribe to event types; returns the unsubscribe. */
  on(types: readonly string[], fn: (ev: LiveEvent) => void): () => void;
  /** Called when events may have been lost (reconnect, lag `resync`). */
  onResync(fn: () => void): () => void;
  /** Socket up/down transitions. */
  onConnection(fn: (connected: boolean) => void): () => void;
  connected(): boolean;
}

/** The fan-out behind a `LiveSource`: the socket owner calls `dispatch` per
 *  event, `resync()` after a gap and `setConnected()` on open/close. */
export class LiveRegistry implements LiveSource {
  private byType = new Map<string, Set<(ev: LiveEvent) => void>>();
  private resyncFns = new Set<() => void>();
  private connFns = new Set<(connected: boolean) => void>();
  private up = false;

  on(types: readonly string[], fn: (ev: LiveEvent) => void): () => void {
    for (const t of types) {
      let set = this.byType.get(t);
      if (!set) this.byType.set(t, (set = new Set()));
      set.add(fn);
    }
    return () => {
      for (const t of types) {
        const set = this.byType.get(t);
        set?.delete(fn);
        if (set && set.size === 0) this.byType.delete(t);
      }
    };
  }

  onResync(fn: () => void): () => void {
    this.resyncFns.add(fn);
    return () => this.resyncFns.delete(fn);
  }

  onConnection(fn: (connected: boolean) => void): () => void {
    this.connFns.add(fn);
    return () => this.connFns.delete(fn);
  }

  connected(): boolean {
    return this.up;
  }

  /** Event types someone listens to (a topic-filtered socket subscribes to these). */
  types(): string[] {
    return [...this.byType.keys()].sort();
  }

  dispatch(ev: LiveEvent): void {
    const set = this.byType.get(ev.type);
    if (!set) return;
    for (const fn of [...set]) {
      try {
        fn(ev);
      } catch {
        /* one listener's bug never starves the rest */
      }
    }
  }

  resync(): void {
    for (const fn of [...this.resyncFns]) {
      try {
        fn();
      } catch {
        /* ignore */
      }
    }
  }

  setConnected(up: boolean): void {
    if (up === this.up) return;
    this.up = up;
    for (const fn of [...this.connFns]) {
      try {
        fn(up);
      } catch {
        /* ignore */
      }
    }
  }
}

/** This document's app-wide source. The main EventsClient (events.svelte.ts)
 *  feeds it; it exists from module load, so stores that subscribe at import
 *  time never race the socket's creation. In a document with no EventsClient
 *  it simply never connects — every liveQuery then polls at `fallbackMs`. */
export const appLive = new LiveRegistry();

/** Listen to `appLive` directly (for views that apply an event's payload
 *  themselves instead of refetching). Returns the unsubscribe. */
export function onLive(types: readonly string[], fn: (ev: LiveEvent) => void): () => void {
  return appLive.on(types, fn);
}

export interface LiveQueryOptions {
  /** Load + apply. Same contract as a `pollWhileVisible` run: resolve `false`
   *  or throw to count a failure; the signal aborts on stop. */
  run: PollRun;
  /** Event types that invalidate this query. */
  on: readonly string[];
  /** Narrow `on` (e.g. to the open workspace/run); default: every event. */
  match?: (ev: LiveEvent) => boolean;
  /** Cadence while the event socket is DOWN — the poll this replaces. */
  fallbackMs: number;
  /** Safety-net cadence while the socket is UP (default 5 min). */
  safetyMs?: number;
  /** Trailing debounce for bursts of events (default 250 ms)… */
  debounceMs?: number;
  /** …but never deferred longer than this under a continuous stream (2 s). */
  maxWaitMs?: number;
  /** Event-driven refetches at most this often (default 0 = no floor): a
   *  view that reloads a lot per refetch caps its rate under event churn. */
  minIntervalMs?: number;
  hidden?: 'pause' | number;
  floorMs?: number;
  jitter?: number;
  immediate?: boolean;
  signal?: AbortSignal;
  /** Force a request lane for every run (default: event / resync / cadence
   *  refetches on `bg`, the first load and a manual `now()` interactive). */
  lane?: Lane;
  /** Spread a reconnect / lag resync over this window (default 1500 ms), so
   *  every mounted query of every document doesn't refetch in one burst. */
  resyncSpreadMs?: number;
}

export const LIVE_SAFETY_MS = 300_000;

/** An event-driven query; see the module comment. `source` defaults to this
 *  document's {@link appLive}; `null` makes it a plain poll at `fallbackMs`. */
export function liveQuery(opts: LiveQueryOptions, source: LiveSource | null = appLive): Poller {
  const safety = Math.max(opts.safetyMs ?? LIVE_SAFETY_MS, opts.fallbackMs);
  const debounceMs = opts.debounceMs ?? 250;
  const maxWaitMs = opts.maxWaitMs ?? 2_000;
  const minIntervalMs = opts.minIntervalMs ?? 0;
  let lastFireAt = -Infinity;
  const offs: (() => void)[] = [];
  let timer: ReturnType<typeof setTimeout> | undefined;
  let firstPendingAt = 0;
  let stopped = false;

  const poller = pollWhileVisible(opts.run, {
    // Read on every scheduling decision: slow while events flow, the old
    // cadence while they cannot.
    get ms() {
      return source?.connected() ? safety : opts.fallbackMs;
    },
    // A numeric hidden cadence (a window whose result shows outside the
    // webview, e.g. the tray glyph) also relaxes to the safety net while
    // events flow.
    get hidden() {
      const h = opts.hidden;
      return typeof h === 'number' && source?.connected() ? Math.max(h, safety) : h;
    },
    floorMs: opts.floorMs,
    jitter: opts.jitter,
    immediate: opts.immediate,
    signal: opts.signal,
    lane: opts.lane,
  });

  const fire = (): void => {
    timer = undefined;
    firstPendingAt = 0;
    lastFireAt = Date.now();
    // An event / resync refetch is background work (lane "bg").
    if (!stopped) poller.now({ background: true });
  };
  const bump = (): void => {
    if (stopped) return;
    const now = Date.now();
    if (timer === undefined) {
      firstPendingAt = now;
    } else if (now - firstPendingAt >= maxWaitMs) {
      return; // a continuous stream: let the armed timer fire
    } else {
      clearTimeout(timer);
    }
    timer = setTimeout(fire, Math.max(debounceMs, lastFireAt + minIntervalMs - now));
  };

  if (source) {
    offs.push(
      source.on(opts.on, (ev) => {
        if (!opts.match || opts.match(ev)) bump();
      }),
    );
    // A reconnect / lag resync reaches every query in every document at the
    // same instant: stagger each one by a random share of the spread window.
    const spread = Math.max(0, opts.resyncSpreadMs ?? 1500);
    let resyncTimer: ReturnType<typeof setTimeout> | undefined;
    offs.push(
      source.onResync(() => {
        if (stopped || resyncTimer !== undefined) return;
        resyncTimer = setTimeout(() => {
          resyncTimer = undefined;
          bump();
        }, Math.round(Math.random() * spread));
      }),
    );
    offs.push(() => {
      if (resyncTimer !== undefined) clearTimeout(resyncTimer);
    });
    // Socket lost → catch up now and continue at the fallback cadence; the
    // reconnect's resync then refreshes once more. Socket up → the pending
    // fallback tick is pushed out to the safety net (no fetch).
    offs.push(
      source.onConnection((up) => {
        if (up) poller.reschedule?.();
        else bump();
      }),
    );
  }

  const stop = (): void => {
    if (stopped) return;
    stopped = true;
    if (timer !== undefined) clearTimeout(timer);
    for (const off of offs) off();
    poller.stop();
  };
  opts.signal?.addEventListener('abort', stop, { once: true });

  return {
    stop,
    now: () => {
      if (!stopped) poller.now();
    },
    reschedule: () => {
      if (!stopped) poller.reschedule?.();
    },
  };
}
