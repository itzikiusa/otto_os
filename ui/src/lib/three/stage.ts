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
