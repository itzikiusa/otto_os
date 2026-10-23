/**
 * The WebGL lifecycle every three.js surface shares: the classroom
 * (`modules/home/school`), the room games (`rooms/games/common/view.ts`) and
 * the Design Hall studio embed. Each used to hand-roll it and each had a
 * different gap (the games had no context-loss path, the studio none either).
 *
 * Deliberately three-free (structural types only), so importing it never
 * pulls the three.js chunk into a page that has not loaded it yet.
 */

/** The renderer surface these helpers touch (a `THREE.WebGLRenderer`). */
export interface StageRenderer {
  readonly domElement: HTMLCanvasElement;
  dispose(): void;
  forceContextLoss(): void;
}

/** `devicePixelRatio`, capped (a 3× phone renders 9× the pixels of 1×, for
 *  little visible gain); non-finite or missing ratios count as 1. */
export function cappedPixelRatio(cap: number, ratio: number = typeof window === 'undefined' ? 1 : window.devicePixelRatio): number {
  const r = Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
  return Math.min(cap, r);
}

/**
 * Calls `lost` when the browser reclaims the canvas's GPU context (too many
 * live 3D views, a GPU reset, a sleeping laptop). The default is prevented so
 * the context may be restored; the caller remounts (rebuilds its renderer)
 * rather than leaving a black canvas. Returns the unsubscribe.
 */
export function onContextLost(canvas: HTMLCanvasElement, lost: () => void): () => void {
  const handler = (e: Event): void => {
    e.preventDefault();
    lost();
  };
  canvas.addEventListener('webglcontextlost', handler);
  return () => canvas.removeEventListener('webglcontextlost', handler);
}

/**
 * Reports whether `el` is worth drawing: in (or near) the viewport and the
 * document is visible. `change` fires on every flip; the caller pauses its
 * render loop while false. Returns the unsubscribe.
 */
export function watchVisibility(el: Element, change: (visible: boolean) => void, rootMargin = '0px'): () => void {
  let inView = true;
  const doc = typeof document === 'undefined' ? null : document;
  let last: boolean | null = null;
  const emit = (): void => {
    const v = inView && !doc?.hidden;
    if (v === last) return;
    last = v;
    change(v);
  };
  const io =
    typeof IntersectionObserver === 'undefined'
      ? null
      : new IntersectionObserver((entries) => {
          inView = entries.some((e) => e.isIntersecting);
          emit();
        }, { rootMargin });
  io?.observe(el);
  doc?.addEventListener('visibilitychange', emit);
  emit();
  return () => {
    io?.disconnect();
    doc?.removeEventListener('visibilitychange', emit);
  };
}

/** An on-demand render loop (see {@link demandLoop}). */
export interface DemandLoop {
  /** Schedule a frame unless one is pending or the loop may not run. */
  kick(): void;
  /** Cancel the pending frame (pause; a later `kick` resumes). */
  stop(): void;
  /** A frame is scheduled or running (false = the loop is idle). */
  readonly pending: boolean;
}

/**
 * The render-loop discipline every 3D surface shares: frames are requested
 * only while there is work. `frame(now)` runs one frame and returns whether
 * more work remains (an active tween, held keys, a turntable, damping…);
 * true re-arms the next frame, false leaves the loop idle with NO rAF
 * scheduled — an idle scene must not wake the main thread 60×/s (Otto's
 * webviews disable WebKit's background throttling, so nothing else clamps
 * it). Anything that changes the picture calls `kick()` to restart it.
 * `canRun` (visible, active…) is checked on `kick` and at every frame.
 */
export function demandLoop(
  frame: (now: number) => boolean,
  opts: {
    canRun?: () => boolean;
    raf?: (cb: (now: number) => void) => number;
    caf?: (id: number) => void;
  } = {},
): DemandLoop {
  const raf = opts.raf ?? ((cb: (now: number) => void) => requestAnimationFrame(cb));
  const caf = opts.caf ?? ((id: number) => cancelAnimationFrame(id));
  const may = (): boolean => !opts.canRun || opts.canRun();
  let id = 0;
  let inFrame = false;
  const run = (now: number): void => {
    id = 0;
    if (!may()) return;
    inFrame = true;
    let more = false;
    try {
      more = frame(now);
    } finally {
      inFrame = false;
    }
    // `frame` may itself kick (a callback that changed something): one frame.
    if (more && !id && may()) id = raf(run);
  };
  return {
    kick() {
      if (id || !may()) return;
      id = raf(run);
    },
    stop() {
      if (id) caf(id);
      id = 0;
    },
    get pending() {
      return id !== 0 || inFrame;
    },
  };
}

/** Tears a renderer down for good: frees its GL resources, hands the context
 *  back at once (browsers cap live contexts) and removes the canvas. */
export function releaseRenderer(renderer: StageRenderer): void {
  renderer.dispose();
  renderer.forceContextLoss();
  renderer.domElement.remove();
}

/**
 * A remount budget for context losses: `allow()` is true for at most `max`
 * losses inside `windowMs`, after which the surface should ask for Retry
 * instead of looping (the classroom's "twice a minute" rule).
 */
export function lossBudget(max = 2, windowMs = 60_000, clock: () => number = Date.now): { allow(): boolean } {
  let at: number[] = [];
  return {
    allow() {
      const t = clock();
      at = at.filter((x) => t - x < windowMs).concat(t);
      return at.length <= max;
    },
  };
}
