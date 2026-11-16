// The keyboard interrupt for a running turn (Claude Code / Codex: Esc). In
// the composer, Esc stops the turn when the composer is empty; with text in
// it, a second Esc within ESC_WINDOW_MS does (one stray Esc never throws a
// turn away). ⌘. (Ctrl+. elsewhere) stops from anywhere in the focused pane.
// Whatever else uses Esc first — the mention menu, the slash popup, a find
// bar, an open overlay — handles it before this is asked. Pure
// (unit/threadsAttentionDock.test.ts).

export const ESC_WINDOW_MS = 600;

export interface StopKeyState {
  /** `performance.now()` of the last lone Esc in the composer, or -Infinity. */
  lastEscAt: number;
}

/** Whether this key press in the composer stops the running turn. Updates
 *  `state.lastEscAt` for the double-press rule. */
export function composerStopKey(
  e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; isComposing?: boolean },
  ctx: { running: boolean; composerEmpty: boolean; now: number },
  state: StopKeyState,
): boolean {
  if (!ctx.running || e.isComposing) return false;
  if (isStopChord(e)) return true;
  if (e.key !== 'Escape' || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return false;
  if (ctx.composerEmpty || ctx.now - state.lastEscAt <= ESC_WINDOW_MS) {
    state.lastEscAt = -Infinity;
    return true;
  }
  state.lastEscAt = ctx.now;
  return false;
}

/** ⌘. / Ctrl+. — the pane-wide stop chord. */
export function isStopChord(e: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }): boolean {
  return e.key === '.' && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey;
}
