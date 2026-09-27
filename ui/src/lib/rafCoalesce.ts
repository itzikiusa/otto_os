// Coalesce a high-rate callback (mousemove, pointermove, ResizeObserver) to
// the latest argument once per animation frame. DOM-lib-free on purpose so
// `node --test` can exercise it (the frame functions are read off globalThis).

type FrameApi = {
  requestAnimationFrame: (cb: () => void) => number;
  cancelAnimationFrame: (id: number) => void;
};
const frames = (): FrameApi => globalThis as unknown as FrameApi;

export function rafCoalesce<A>(fn: (arg: A) => void): { push(arg: A): void; flush(): void; cancel(): void } {
  let pending: { arg: A } | null = null;
  let raf = 0;
  const run = (): void => {
    raf = 0;
    const p = pending;
    pending = null;
    if (p) fn(p.arg);
  };
  return {
    push(arg: A): void {
      pending = { arg };
      if (!raf) raf = frames().requestAnimationFrame(run);
    },
    /** Apply a pending call now (e.g. on mouseup, before persisting). */
    flush(): void {
      if (raf) frames().cancelAnimationFrame(raf);
      run();
    },
    cancel(): void {
      if (raf) frames().cancelAnimationFrame(raf);
      raf = 0;
      pending = null;
    },
  };
}
