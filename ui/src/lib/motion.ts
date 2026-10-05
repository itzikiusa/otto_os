// Reduced-motion helpers for motion started from SCRIPT. The global
// `@media (prefers-reduced-motion: reduce)` override in app.css only reaches
// CSS animations/transitions — a JS `scrollIntoView({ behavior: 'smooth' })` or
// a Svelte `in:fly` ignores it. Route those through here (foundations §8;
// `ui-guards` ratchets `behavior: 'smooth'` literals as `smooth-scroll`).

/** True when the user asked the OS to reduce motion. Never throws (SSR / old
 *  webviews without matchMedia read as "no preference"). */
export function reducedMotion(): boolean {
  try {
    return typeof window !== 'undefined' && !!window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}

/** `behavior` for scrollIntoView / scrollTo / scrollBy: smooth unless the user
 *  reduces motion, then an instant jump. */
export function scrollBehavior(): ScrollBehavior {
  return reducedMotion() ? 'auto' : 'smooth';
}

/** A transition duration that collapses to 0 under reduced motion
 *  (`in:fly={{ y: 8, duration: motionMs(160) }}`). */
export function motionMs(ms: number): number {
  return reducedMotion() ? 0 : ms;
}
