// Local selection in a terminal whose app owns the mouse (Terminal.svelte).
//
// Claude's "full" mouse mode includes 1003 (any-motion): xterm reports every
// pointer MOVE to the app as user input, and xterm clears the selection on
// user input, so an ⌥-drag selection vanished on the first wiggle after the
// drag. While text is selected, a button-less move stays local (the link
// layer on the screen element still sees it first); clicks, keys and the
// wheel still reach the app and end the selection as usual.
//
// A plain drag in such an app goes to the app and selects nothing. The first
// time that happens, one calm pointer to the modifier that selects anyway
// (xterm: ⌥ on macOS, Shift elsewhere); never again.
import type { Terminal } from '@xterm/xterm';
import { isMacPlatform } from '../keys';
import { toasts } from '../toast.svelte';

export const SELECT_HINT_KEY = 'otto_term_select_hint_shown';

/** Attach both behaviours to an opened xterm; returns the undo. */
export function guardSelection(t: Terminal): () => void {
  const screen = t.element?.querySelector<HTMLElement>('.xterm-screen');
  if (!screen) return () => {};
  let from: { x: number; y: number } | null = null;
  const move = (e: MouseEvent): void => {
    if (!e.buttons && t.hasSelection()) e.stopPropagation();
  };
  const down = (e: MouseEvent): void => {
    const plain = e.button === 0 && !e.altKey && !e.shiftKey;
    from = plain && t.modes.mouseTrackingMode !== 'none' ? { x: e.clientX, y: e.clientY } : null;
  };
  const up = (e: MouseEvent): void => {
    const start = from;
    from = null;
    if (start && Math.hypot(e.clientX - start.x, e.clientY - start.y) >= 24) selectHintOnce();
  };
  screen.addEventListener('mousemove', move);
  screen.addEventListener('mousedown', down);
  screen.addEventListener('mouseup', up);
  return () => {
    screen.removeEventListener('mousemove', move);
    screen.removeEventListener('mousedown', down);
    screen.removeEventListener('mouseup', up);
  };
}

function selectHintOnce(): void {
  try {
    if (localStorage.getItem(SELECT_HINT_KEY) === '1') return;
    localStorage.setItem(SELECT_HINT_KEY, '1');
  } catch {
    return; // no storage → don't nag on every drag
  }
  const mod = isMacPlatform() ? '⌥' : 'Shift';
  toasts.push('info', `Hold ${mod} to select text`, `This app uses the mouse, so a plain drag goes to it. Hold ${mod} while you drag to select, then copy.`, 10000);
}
