// The WAI-ARIA "window splitter" as one action: it makes the separator
// focusable (role + tabindex), exposes its value to assistive tech, adds the
// keys, and — given `onDragStart` — wires the pointer drag and double-click
// reset too, so the markup is a plain labelled <div> with no hand-written
// tabindex or handlers (Svelte's a11y lint cannot see that a focusable
// separator is a widget). The drag maths stay with each page:
//   ←/→ (↑/↓ for a horizontal separator) nudge by `step`, ⇧ by `bigStep`,
//   Home/End jump to the limits, Enter resets (when `onReset` is given).
//
//   <div class="side-resizer" role="separator" aria-label="Resize connections sidebar"
//        title="Drag or use ←/→ to resize · double-click or Enter to reset"
//        use:paneResizer={{ value: sideW, min: 200, max: 480, onChange: (w) => (sideW = w),
//                           onReset: resetSideW, onDragStart: startDrag }}></div>
//
// `invert` is for a separator on the LEADING edge of the pane it sizes (the pane
// grows when the divider moves toward the start, e.g. a right-hand rail); it
// flips the arrows only — Home/End stay the value's min/max.
/** One shared tooltip for every side-by-side pane separator. */
export const RESIZE_TITLE = 'Drag or use ←/→ to resize · double-click or Enter to reset';
/** The same for a horizontal bar between stacked panes (↑/↓). */
export const RESIZE_TITLE_VERTICAL = 'Drag or use ↑/↓ to resize · double-click or Enter to reset';
/** `aria-valuetext` for a pane width. */
export const pxWide = (v: number): string => `${Math.round(v)} pixels wide`;

/** The list/index pane of a list–detail page: one default + one range, so the
 *  modules agree (and each persists the user's width under its own key). */
export const LIST_PANE = { default: 280, min: 220, max: 420 } as const;

/** Persisted pane width: read `key` from localStorage, clamped to [min, max]. */
export function loadPaneWidth(key: string, fallback: number, min: number, max: number): number {
  try {
    const v = Number(localStorage.getItem(key));
    return Number.isFinite(v) && v >= min ? Math.min(max, v) : fallback;
  } catch {
    return fallback;
  }
}

export function savePaneWidth(key: string, width: number): void {
  try {
    localStorage.setItem(key, String(Math.round(width)));
  } catch {
    /* storage unavailable — the width just isn't remembered */
  }
}

export interface PaneResizerOptions {
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  onReset?: () => void;
  /** Default `vertical` — a vertical bar between side-by-side panes (←/→). */
  orientation?: 'vertical' | 'horizontal';
  step?: number;
  bigStep?: number;
  invert?: boolean;
  /** `aria-valuetext`, e.g. (v) => `${v} px`. */
  text?: (value: number) => string;
  /** The page's pointer drag, attached as `mousedown` (double-click then
   *  runs `onReset`). Leave it out when the page wires its own pointer events. */
  onDragStart?: (e: MouseEvent) => void;
}

export function paneResizer(node: HTMLElement, initial: PaneResizerOptions) {
  let o = initial;

  const sync = (): void => {
    node.setAttribute('role', 'separator');
    node.tabIndex = 0;
    node.setAttribute('aria-orientation', o.orientation ?? 'vertical');
    node.setAttribute('aria-valuemin', String(Math.round(o.min)));
    node.setAttribute('aria-valuemax', String(Math.round(o.max)));
    node.setAttribute('aria-valuenow', String(Math.round(o.value)));
    if (o.text) node.setAttribute('aria-valuetext', o.text(o.value));
    else node.removeAttribute('aria-valuetext');
  };

  function onKeyDown(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const horizontalBar = o.orientation === 'horizontal';
    const rtl = !horizontalBar && getComputedStyle(node).direction === 'rtl';
    const step = (e.shiftKey ? (o.bigStep ?? (o.step ?? 16) * 4) : (o.step ?? 16));
    let delta = 0;
    if (!horizontalBar && e.key === 'ArrowRight') delta = rtl ? -step : step;
    else if (!horizontalBar && e.key === 'ArrowLeft') delta = rtl ? step : -step;
    else if (horizontalBar && e.key === 'ArrowDown') delta = step;
    else if (horizontalBar && e.key === 'ArrowUp') delta = -step;
    // Home/End are VALUE limits (APG window splitter: Home gives the sized pane
    // its smallest size), whichever edge it sits on — `invert` only flips the
    // arrows. Swapping them for an inverted separator announced "Home" as
    // aria-valuemax.
    else if (e.key === 'Home') { e.preventDefault(); o.onChange(o.min); return; }
    else if (e.key === 'End') { e.preventDefault(); o.onChange(o.max); return; }
    else if (e.key === 'Enter' && o.onReset) { e.preventDefault(); o.onReset(); return; }
    else return;
    e.preventDefault();
    if (o.invert) delta = -delta;
    o.onChange(Math.max(o.min, Math.min(o.max, o.value + delta)));
  }

  const onMouseDown = (e: MouseEvent): void => o.onDragStart?.(e);
  const onDblClick = (): void => {
    if (o.onDragStart) o.onReset?.();
  };

  sync();
  node.addEventListener('keydown', onKeyDown);
  node.addEventListener('mousedown', onMouseDown);
  node.addEventListener('dblclick', onDblClick);
  return {
    update(next: PaneResizerOptions) {
      o = next;
      sync();
    },
    destroy() {
      node.removeEventListener('keydown', onKeyDown);
      node.removeEventListener('mousedown', onMouseDown);
      node.removeEventListener('dblclick', onDblClick);
    },
  };
}

/** Handlers of a custom window splitter (see {@link splitter}). */
export interface SplitterHandlers {
  onkeydown: (e: KeyboardEvent) => void;
  onpointerdown?: (e: PointerEvent) => void;
  onmousedown?: (e: MouseEvent) => void;
  ondblclick?: (e: MouseEvent) => void;
}

/**
 * The same window-splitter widget for separators whose value logic is their
 * own (a fraction split, a 2-D tile grid, a row/column pair): the markup keeps
 * `role="separator"`, its label and its `aria-value*`; this makes it focusable
 * and attaches the keys + pointer handlers. Use `paneResizer` when the value is
 * a plain width/height.
 *
 *   <div role="separator" aria-label="Resize tiles" aria-valuenow={pct}
 *        use:splitter={{ onkeydown: onKey, onpointerdown: startDrag, ondblclick: reset }}></div>
 */
export function splitter(node: HTMLElement, initial: SplitterHandlers) {
  let h = initial;
  node.tabIndex = 0;
  const key = (e: KeyboardEvent): void => h.onkeydown(e);
  const pointer = (e: PointerEvent): void => h.onpointerdown?.(e);
  const mouse = (e: MouseEvent): void => h.onmousedown?.(e);
  const dbl = (e: MouseEvent): void => h.ondblclick?.(e);
  node.addEventListener('keydown', key);
  node.addEventListener('pointerdown', pointer);
  node.addEventListener('mousedown', mouse);
  node.addEventListener('dblclick', dbl);
  return {
    update(next: SplitterHandlers) {
      h = next;
    },
    destroy() {
      node.removeEventListener('keydown', key);
      node.removeEventListener('pointerdown', pointer);
      node.removeEventListener('mousedown', mouse);
      node.removeEventListener('dblclick', dbl);
    },
  };
}
