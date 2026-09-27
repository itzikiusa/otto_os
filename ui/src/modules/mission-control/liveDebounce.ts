// Trailing debounce with a max wait (A7). Mission Control refetches summary,
// items and the graph on `work_graph_updated`; a plain trailing debounce
// starves under a steady event stream (a busy swarm ticks every few hundred
// ms), so the page never refreshed while work was actually moving. `trigger()`
// re-arms the quiet timer, but never pushes the run past `maxWaitMs` after the
// first trigger of a burst.

export interface LiveDebounce {
  trigger(): void;
  cancel(): void;
  readonly pending: boolean;
}

export function liveDebounce(
  run: () => void,
  waitMs: number,
  maxWaitMs: number,
  now: () => number = () => Date.now(),
  timers: {
    set: (fn: () => void, ms: number) => unknown;
    clear: (h: unknown) => void;
  } = {
    set: (fn, ms) => setTimeout(fn, ms),
    clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  },
): LiveDebounce {
  let handle: unknown = null;
  let firstAt = 0;
  const fire = (): void => {
    handle = null;
    run();
  };
  return {
    trigger(): void {
      const t = now();
      if (handle === null) firstAt = t;
      else timers.clear(handle);
      const delay = Math.max(0, Math.min(waitMs, firstAt + maxWaitMs - t));
      handle = timers.set(fire, delay);
    },
    cancel(): void {
      if (handle !== null) timers.clear(handle);
      handle = null;
    },
    get pending(): boolean {
      return handle !== null;
    },
  };
}
