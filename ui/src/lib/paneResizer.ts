// The keyboard + ARIA half of a pane resizer — the WAI-ARIA "window splitter".
// Pointer dragging stays with each page (their drag maths differ); this action
// makes the same separator reachable and operable without a mouse and exposes
// its value to assistive tech, matching shell/SplitDivider.svelte:
//   ←/→ (↑/↓ for a horizontal separator) nudge by `step`, ⇧ by `bigStep`,
//   Home/End jump to the limits, Enter resets (when `onReset` is given).
//
//   <div class="side-resizer" role="separator" aria-label="Resize connections sidebar"
//        title="Drag or use ←/→ to resize · double-click or Enter to reset"
//        use:paneResizer={{ value: sideW, min: 200, max: 480, onChange: (w) => (sideW = w), onReset: resetSideW }}></div>
//
// `invert` is for a separator on the LEADING edge of the pane it sizes (the pane
// grows when the divider moves toward the start, e.g. a right-hand rail).
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
    else if (e.key === 'Home') { e.preventDefault(); o.onChange(o.invert ? o.max : o.min); return; }
    else if (e.key === 'End') { e.preventDefault(); o.onChange(o.invert ? o.min : o.max); return; }
    else if (e.key === 'Enter' && o.onReset) { e.preventDefault(); o.onReset(); return; }
    else return;
    e.preventDefault();
    if (o.invert) delta = -delta;
    o.onChange(Math.max(o.min, Math.min(o.max, o.value + delta)));
  }

  sync();
  node.addEventListener('keydown', onKeyDown);
  return {
    update(next: PaneResizerOptions) {
      o = next;
      sync();
    },
    destroy() {
      node.removeEventListener('keydown', onKeyDown);
    },
  };
}
