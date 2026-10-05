<script lang="ts">
  // The side-by-side split's divider: a 1px hairline (NSSplitView's thin
  // divider) with a wider invisible grab area. Drag it, or focus it and use
  // ←/→ (⇧ for a big step), Home/End for the limits and Enter — or a
  // double-click — for 50/50. While dragging, the side pane's iframe stops
  // taking pointer events (sidePane.dragging) so it can't swallow the moves.
  import { sidePane } from '../lib/stores/sidePane.svelte';
  import { leadingFromPointer, nudgeLeading, SPLIT_MAX, SPLIT_MIN } from '../lib/sidePane';
  import { rafCoalesce, showDragOverlay } from '../lib/dragCursor';
  import { splitter } from '../lib/paneResizer';

  interface Props {
    /** Accessible name of the two panes, e.g. "Agents and Connections". */
    label: string;
  }
  let { label }: Props = $props();

  let el: HTMLDivElement | undefined = $state();

  const isRtl = (): boolean => !!el && getComputedStyle(el).direction === 'rtl';

  function onPointerDown(e: PointerEvent): void {
    if (e.button !== 0) return;
    const split = el?.parentElement;
    if (!split) return;
    e.preventDefault();
    el?.setPointerCapture(e.pointerId);
    sidePane.dragging = true;
    const rtl = isRtl();
    // Overlay cursor (not body.style — an inherited write restyles the whole
    // app) and one split write per frame (lib/dragCursor.ts).
    const hideOverlay = showDragOverlay('col-resize');
    const moves = rafCoalesce((ev: PointerEvent): void => {
      const r = split.getBoundingClientRect();
      sidePane.setLeading(leadingFromPointer(ev.clientX, r.left, r.right, rtl), false);
    });
    const move = (ev: PointerEvent): void => moves.push(ev);
    const up = (ev: PointerEvent): void => {
      moves.flush();
      el?.releasePointerCapture(ev.pointerId);
      el?.removeEventListener('pointermove', move);
      el?.removeEventListener('pointerup', up);
      el?.removeEventListener('pointercancel', up);
      hideOverlay();
      sidePane.dragging = false;
      sidePane.setLeading(sidePane.leading, true);
    };
    el?.addEventListener('pointermove', move);
    el?.addEventListener('pointerup', up);
    el?.addEventListener('pointercancel', up);
  }

  function onKeyDown(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const next = nudgeLeading(sidePane.leading, e.key, { shift: e.shiftKey, rtl: isRtl() });
    if (next === null) return;
    e.preventDefault();
    if (e.key === 'Enter') sidePane.resetSplit();
    else sidePane.setLeading(next);
  }

  const pct = $derived(Math.round(sidePane.leading * 100));
</script>

<!-- The ARIA "window splitter": `splitter` makes it focusable and wires the
     keys, the pointer drag and the double-click 50/50 reset. -->
<div
  bind:this={el}
  class="split-divider"
  class:dragging={sidePane.dragging}
  role="separator"
  aria-orientation="vertical"
  aria-label={`Resize ${label}`}
  aria-valuemin={Math.round(SPLIT_MIN * 100)}
  aria-valuemax={Math.round(SPLIT_MAX * 100)}
  aria-valuenow={pct}
  aria-valuetext={`${pct}% · ${100 - pct}%`}
  title="Drag to resize · double-click for 50/50"
  use:splitter={{ onkeydown: onKeyDown, onpointerdown: onPointerDown, ondblclick: () => sidePane.resetSplit() }}
  data-testid="split-divider"
></div>

<style>
  .split-divider {
    position: relative;
    flex: 0 0 1px;
    align-self: stretch;
    /* The hairline over an opaque base: the window behind the content column
       can be see-through (native vibrancy). */
    background-color: var(--bg);
    background-image: linear-gradient(var(--separator), var(--separator));
    cursor: col-resize;
    z-index: 5;
    touch-action: none;
    outline: none;
  }
  /* A 9px grab area centred on the hairline. */
  .split-divider::before {
    content: '';
    position: absolute;
    inset-block: 0;
    inset-inline-start: -4px;
    width: 9px;
  }
  /* Drag / keyboard focus: the line firms up into the accent, like a focus
     indicator — the only time the divider draws attention to itself. */
  .split-divider.dragging,
  .split-divider:focus-visible {
    background-image: linear-gradient(var(--accent), var(--accent));
  }
  .split-divider:focus-visible::after {
    content: '';
    position: absolute;
    inset-block: 0;
    inset-inline-start: -1px;
    width: 3px;
    background: var(--accent-line-strong);
    border-radius: var(--radius-s);
  }
</style>
