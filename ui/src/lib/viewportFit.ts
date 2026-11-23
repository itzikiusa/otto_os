// Touch devices: keep the app inside the VISIBLE viewport. iOS (Safari and
// Chrome alike) does not shrink the layout viewport for the on-screen
// keyboard; it pans the page inside it, so focusing a bottom composer pushed
// the page's top (the Threads header) above the screen — and the pan often
// stayed after the keyboard closed. `#app` follows `visualViewport.height`
// (app.css `--app-h`) and any pan is undone, so the header stays put and the
// composer sits just above the keyboard. Pinch-zoom (scale ≠ 1) is left alone.
//
// It is also the ONE place that decides "the on-screen keyboard is open", for
// every page: `html[data-kb="open"]` (CSS — the BottomNav and the phone action
// bar hide under it), `--kb-h` (the keyboard's height in px, 0 when closed)
// and `onKeyboard()` (lib/stores/keyboard.svelte.ts makes it reactive).
//   iOS / Android Chrome ≥ 108 (`interactive-widget=resizes-visual`, index.html):
//     the layout viewport keeps its height (`innerHeight`), only the visual
//     viewport shrinks — the keyboard is innerHeight − vv.height.
//   Android "resizes-content" (older Chrome, other browsers): both shrink, so
//     the gap is measured against the tallest height seen at this WIDTH (a
//     rotation starts a new baseline).
// `--app-h` is the visual height in both cases, so nothing is subtracted
// twice. A text field must have focus too: a hardware keyboard, Safari's
// toolbars or a split-screen resize never count as the keyboard.

import { trace } from './composerTrace.ts';

type VV = Pick<VisualViewport, 'height' | 'scale' | 'offsetTop' | 'addEventListener'> & {
  offsetLeft?: number;
};

export interface KeyboardState {
  /** The on-screen keyboard is up (a text field has focus and the view shrank). */
  open: boolean;
  /** Height of the visible area in CSS px (the app's height). */
  visibleHeight: number;
  /** The keyboard's height in CSS px; 0 when closed. */
  inset: number;
}

/** Keyboard up and less than this much screen left (a phone on its side keeps ~70 px): the
 *  shell's top row and the page header step aside (`html[data-kb-short]`, app.css). */
export const KEYBOARD_SHORT_PX = 400;
/** A drop smaller than this is browser chrome (Safari's toolbars), not a keyboard. */
export const KEYBOARD_MIN_PX = 120;
const MIN_SHARE = 0.15;

const TEXT_INPUTS = new Set(['text', 'search', 'email', 'url', 'tel', 'password', 'number', 'date', 'datetime-local', 'month', 'time', 'week', '']);

/** An element that raises the on-screen keyboard when focused. */
export function isTextEntry(el: Element | null | undefined): boolean {
  if (!el) return false;
  const f = el as HTMLInputElement & { isContentEditable?: boolean };
  if (el.tagName === 'TEXTAREA') return !f.readOnly && !f.disabled;
  if (el.tagName === 'INPUT') return TEXT_INPUTS.has(String(f.type ?? '').toLowerCase()) && !f.readOnly && !f.disabled;
  return f.isContentEditable === true;
}

export interface Baseline {
  width: number;
  height: number;
}

/** The keyboard-free height at this width: the tallest seen since the width
 *  last changed (a rotation, a split-screen resize). */
export function nextBaseline(prev: Baseline | null, width: number, layoutHeight: number, visibleHeight: number): Baseline {
  const seen = Math.max(layoutHeight, visibleHeight);
  if (!prev || prev.width !== width) return { width, height: seen };
  return { width, height: Math.max(prev.height, seen) };
}

/** Keyboard state from one measurement (pure: the unit tests drive it). */
export function keyboardState(baseline: number, visibleHeight: number, textFocused: boolean): KeyboardState {
  const gap = Math.max(0, Math.round(baseline - visibleHeight));
  const open = textFocused && gap >= Math.max(KEYBOARD_MIN_PX, baseline * MIN_SHARE);
  return { open, visibleHeight: Math.round(visibleHeight), inset: open ? gap : 0 };
}

let current: KeyboardState = { open: false, visibleHeight: 0, inset: 0 };
const listeners = new Set<(s: KeyboardState) => void>();
let activeSync: (() => void) | null = null;

/** Subscribe to keyboard changes; called at once with the current state. */
export function onKeyboard(f: (s: KeyboardState) => void): () => void {
  listeners.add(f);
  f(current);
  return () => void listeners.delete(f);
}

interface ScrollBox {
  scrollLeft: number;
  scrollTop: number;
}

interface Win {
  visualViewport: VV | null;
  scrollX?: number;
  scrollY: number;
  scrollTo(x: number, y: number): void;
  innerHeight?: number;
  innerWidth?: number;
  document?: {
    activeElement: Element | null;
    documentElement?: ScrollBox | null;
    body?: ScrollBox | null;
    getElementById?: (id: string) => ScrollBox | null;
    querySelector?: (sel: string) => ScrollBox | null;
  };
  addEventListener?(type: string, f: () => void, capture?: boolean): void;
}

interface Root {
  style: { setProperty(k: string, v: string): void };
  dataset?: Record<string, string | undefined>;
}

/** Zero out any accidental scroll offset on non-scrolling shell ancestors
 *  (`html`, `body`, `#app`, `.shell`, `#app-content`) and the window itself.
 *  Focusing off-screen animated drawers or calling `scrollIntoView` can drift
 *  `scrollLeft`/`scrollTop` on `overflow: hidden` ancestors and leave every
 *  subsequent page shifted sideways ("half cut off"). */
export function resetShellScroll(win?: Win): void {
  const w = win ?? (typeof window !== 'undefined' ? (window as unknown as Win) : undefined);
  if (!w) return;
  const vv = w.visualViewport;
  const sx = w.scrollX ?? 0;
  const sy = w.scrollY ?? 0;
  const ox = vv?.offsetLeft ?? 0;
  const oy = vv?.offsetTop ?? 0;
  const moved: string[] = [];
  if (sx !== 0 || sy !== 0 || ox !== 0 || oy !== 0) {
    w.scrollTo(0, 0);
    moved.push(`window ${sx},${sy} vv ${ox},${oy}`);
  }
  const doc = w.document;
  const boxes: [string, ScrollBox | null | undefined][] = doc
    ? [
        ['html', doc.documentElement],
        ['body', doc.body],
        ['#app', doc.getElementById?.('app')],
        ['#app-content', doc.getElementById?.('app-content')],
        ['.shell', doc.querySelector?.('.shell')],
        ['.shell-main', doc.querySelector?.('.shell-main')],
      ]
    : [];
  for (const [name, box] of boxes) {
    if (!box || (box.scrollLeft === 0 && box.scrollTop === 0)) continue;
    moved.push(`${name} ${box.scrollLeft},${box.scrollTop}`);
    box.scrollLeft = 0;
    box.scrollTop = 0;
  }
  if (moved.length) trace('reset', { moved: moved.join('; ') });
}

/** Re-run viewport fit immediately (called after route navigation / mode switches). */
export function resyncViewportFit(): void {
  resetShellScroll();
  activeSync?.();
}

export function fitToVisualViewport(win: Win, root: Root): void {
  const vv = win.visualViewport;
  if (!vv) return;
  let base: Baseline | null = null;
  const emit = (next: KeyboardState): void => {
    root.style.setProperty('--kb-h', `${next.inset}px`);
    if (root.dataset) {
      if (next.open) root.dataset.kb = 'open';
      else delete root.dataset.kb;
      if (next.open && next.visibleHeight < KEYBOARD_SHORT_PX) root.dataset.kbShort = '';
      else delete root.dataset.kbShort;
    }
    if (next.open === current.open && next.visibleHeight === current.visibleHeight && next.inset === current.inset) return;
    // The keyboard just opened: the app shrank under the field Chrome scrolled into view
    // a moment earlier, so bring the field (and its scroll-margin) back into the visible part.
    if (next.open && !current.open) {
      setTimeout(() => {
        const el = win.document?.activeElement as Element | null | undefined;
        trace('scrollIntoView', { el, top: vv.offsetTop });
        el?.scrollIntoView?.({ block: 'nearest' });
      }, 0);
    }
    current = next;
    for (const f of listeners) f(current);
  };
  let appTop = 0;
  const sync = (): void => {
    const focused = isTextEntry(win.document?.activeElement);
    if (Math.abs(vv.scale - 1) > 0.01) {
      // Leave pinch-zoom height alone, but never leave `data-kb="open"` stuck
      // (which hides BottomNav) once focus has left the text field.
      if (current.open && !focused) {
        emit({ open: false, visibleHeight: current.visibleHeight, inset: 0 });
      }
      return;
    }
    root.style.setProperty('--app-h', `${Math.round(vv.height)}px`);
    resetShellScroll(win);
    // Android Chrome (resizes-visual) reveals a focused field by panning the VISUAL viewport
    // inside the taller layout one, and scrollTo can't undo that pan: the app then sat above a
    // blank band. With the keyboard up, #app follows the pan instead (app.css `--app-top`).
    const top = Math.max(0, Math.round(vv.offsetTop));
    trace('fit', { h: vv.height, top: vv.offsetTop, appTop: `${appTop}→${top}` });
    appTop = top;
    root.style.setProperty('--app-top', `${top}px`);
    base = nextBaseline(base, win.innerWidth ?? 0, win.innerHeight ?? vv.height, vv.height);
    const next = keyboardState(base.height, vv.height, focused);
    emit(next);
  };
  activeSync = sync;
  sync();
  vv.addEventListener('resize', sync);
  vv.addEventListener('scroll', sync);
  win.addEventListener?.('resize', sync);
  win.addEventListener?.('pageshow', sync);
  // Focus changes re-check only after a tap has finished: a tap on Send
  // blurs the field (or focuses the button) for a moment, and closing the
  // keyboard state right then brought the BottomNav back under the finger —
  // Send moved between press and release and the tap was lost. A keyboard
  // that really opens or closes resizes the visual viewport, which re-checks
  // at once.
  let timer: ReturnType<typeof setTimeout> | undefined;
  const later = (): void => {
    clearTimeout(timer);
    timer = setTimeout(() => {
      trace('recheck');
      sync();
    }, 400);
  };
  win.addEventListener?.('focusin', later, true);
  win.addEventListener?.('focusout', later, true);
  // Orientation changes on iOS/Android can finish updating visualViewport a
  // beat after the event fires: sync immediately and again once settled.
  const onOrient = (): void => {
    sync();
    clearTimeout(timer);
    timer = setTimeout(sync, 200);
  };
  win.addEventListener?.('orientationchange', onOrient);
}
