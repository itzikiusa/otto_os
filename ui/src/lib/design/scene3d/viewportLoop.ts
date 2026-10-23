// Scene3DViewport's render-loop and keyboard rules, kept pure for tests.

export interface ViewportLoopState {
  /** A state tween owns the pose. */
  tween: boolean;
  /** A camera move to a named view is running. */
  camTween: boolean;
  /** The turntable orbit is on… */
  turntable: boolean;
  /** …but a gizmo drag pauses it. */
  dragging: boolean;
}

/** Whether the loop re-arms after a frame. Everything else (orbit, gizmo,
 *  reconcile, resize, selection) calls `invalidate()`, which requests one
 *  frame: an idle viewport schedules no rAF at all. */
export function viewportHasWork(s: ViewportLoopState): boolean {
  return s.tween || s.camTween || (s.turntable && !s.dragging);
}

/**
 * The viewport's single-key shortcuts (W/E/R, F, Delete, Escape) act only
 * while focus is in the viewport. A pointer merely resting over it does not
 * count: a key typed elsewhere must never delete the selected object (WCAG
 * 2.1.4 — speech and switch users' pointer position is incidental).
 */
export function viewportOwnsKey(host: { contains(n: Node | null): boolean } | null, target: EventTarget | null): boolean {
  return !!host && !!target && !isTypingTarget(target) && host.contains(target as Node);
}

/** A text field (typing there is never a shortcut). */
export function isTypingTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null;
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || !!el.isContentEditable;
}
