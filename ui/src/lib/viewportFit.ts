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

type VV = Pick<VisualViewport, 'height' | 'scale' | 'offsetTop' | 'addEventListener'>;

export interface KeyboardState {
  /** The on-screen keyboard is up (a text field has focus and the view shrank). */
  open: boolean;
  /** Height of the visible area in CSS px (the app's height). */
  visibleHeight: number;
  /** The keyboard's height in CSS px; 0 when closed. */
  inset: number;
}

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

/** Subscribe to keyboard changes; called at once with the current state. */
export function onKeyboard(f: (s: KeyboardState) => void): () => void {
  listeners.add(f);
  f(current);
  return () => void listeners.delete(f);
}

interface Win {
  visualViewport: VV | null;
  scrollY: number;
  scrollTo(x: number, y: number): void;
  innerHeight?: number;
  innerWidth?: number;
  document?: { activeElement: Element | null };
  addEventListener?(type: string, f: () => void, capture?: boolean): void;
}

interface Root {
  style: { setProperty(k: string, v: string): void };
  dataset?: Record<string, string | undefined>;
}

export function fitToVisualViewport(win: Win, root: Root): void {
  const vv = win.visualViewport;
  if (!vv) return;
  let base: Baseline | null = null;
  const sync = (): void => {
    if (Math.abs(vv.scale - 1) > 0.01) return;
    root.style.setProperty('--app-h', `${Math.round(vv.height)}px`);
    if (win.scrollY !== 0 || vv.offsetTop !== 0) win.scrollTo(0, 0);
    base = nextBaseline(base, win.innerWidth ?? 0, win.innerHeight ?? vv.height, vv.height);
    const next = keyboardState(base.height, vv.height, isTextEntry(win.document?.activeElement));
    root.style.setProperty('--kb-h', `${next.inset}px`);
    if (root.dataset) {
      if (next.open) root.dataset.kb = 'open';
      else delete root.dataset.kb;
    }
    if (next.open === current.open && next.visibleHeight === current.visibleHeight && next.inset === current.inset) return;
    current = next;
    for (const f of listeners) f(current);
  };
  sync();
  vv.addEventListener('resize', sync);
  vv.addEventListener('scroll', sync);
  // Focus changes re-check only after a tap has finished: a tap on Send
  // blurs the field (or focuses the button) for a moment, and closing the
  // keyboard state right then brought the BottomNav back under the finger —
  // Send moved between press and release and the tap was lost. A keyboard
  // that really opens or closes resizes the visual viewport, which re-checks
  // at once.
  let timer: ReturnType<typeof setTimeout> | undefined;
  const later = (): void => {
    clearTimeout(timer);
    timer = setTimeout(sync, 400);
  };
  win.addEventListener?.('focusin', later, true);
  win.addEventListener?.('focusout', later, true);
}
