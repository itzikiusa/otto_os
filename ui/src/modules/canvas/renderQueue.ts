// Serial render queue with a latest-wins escape hatch, shared by the Mermaid
// and D2 bridges (SD-23). Both libraries must run one job at a time (mermaid's
// `initialize` is global; the D2 worker bridge has a single resolve slot), so a
// burst of edits used to queue N full renders and run every one of them — the
// callers only discarded the stale results AFTER paying for the render (D2's
// fallback frame runs Go-WASM on the main thread).
//
// `isStale` is checked at DEQUEUE: a job whose caller has already moved on
// (newer token, element detached, file switched) resolves to `STALE` without
// touching the renderer, so only the newest job per caller actually renders.
// Failures never wedge the chain.

/** Resolution of a job skipped because its caller superseded it. */
export const STALE = Object.freeze({ stale: true as const });
export type Stale = typeof STALE;

export interface RenderQueue {
  run<T>(job: () => Promise<T>, isStale?: () => boolean): Promise<T | Stale>;
}

export function createRenderQueue(): RenderQueue {
  let tail: Promise<unknown> = Promise.resolve();
  return {
    run<T>(job: () => Promise<T>, isStale?: () => boolean): Promise<T | Stale> {
      const step = async (): Promise<T | Stale> => {
        let stale = false;
        try { stale = !!isStale?.(); } catch { stale = false; }
        if (stale) return STALE;
        return job();
      };
      const next = tail.then(step, step);
      tail = next.catch(() => undefined);
      return next;
    },
  };
}

export function isStaleResult(v: unknown): v is Stale {
  return v === STALE;
}
