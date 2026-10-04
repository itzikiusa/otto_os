<script lang="ts">
  // The draggable + keyboard-operable divider on the INLINE-END edge of a list /
  // index pane (the "window splitter"): pointer drag, ←/→ (⇧ big step), Home/End,
  // Enter or double-click to reset, with the width remembered per `storageKey`.
  // The page owns the pane's width (`bind:width`) and applies it to the pane:
  //
  //   let listW = $state(loadPaneWidth('proof.railW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  //   <aside style:width="{listW}px">…</aside>
  //   <PaneDivider bind:width={listW} storageKey="proof.railW" label="Resize the runs list" />
  import { LIST_PANE, paneResizer, pxWide, RESIZE_TITLE, savePaneWidth } from '../paneResizer';
  import { startMouseDrag } from '../dragCursor';

  interface Props {
    width: number;
    storageKey: string;
    label: string;
    min?: number;
    max?: number;
    defaultWidth?: number;
    /** The pane sits on the inline-END side of the divider (it grows when the
     *  divider moves toward the inline start), e.g. a right-hand detail pane. */
    invert?: boolean;
  }
  let {
    width = $bindable(),
    storageKey,
    label,
    min = LIST_PANE.min,
    max = LIST_PANE.max,
    defaultWidth = LIST_PANE.default,
    invert = false,
  }: Props = $props();

  const clamp = (w: number): number => Math.max(min, Math.min(max, Math.round(w)));
  function set(w: number): void {
    width = clamp(w);
    savePaneWidth(storageKey, width);
  }
  function startDrag(e: MouseEvent): void {
    if (e.button !== 0) return;
    const startX = e.clientX;
    const startW = width;
    const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
    const dir = (rtl ? -1 : 1) * (invert ? -1 : 1);
    startMouseDrag(e, {
      cursor: 'col-resize',
      onMove: (ev) => (width = clamp(startW + dir * (ev.clientX - startX))),
      onEnd: () => savePaneWidth(storageKey, width),
    });
  }
</script>

<!-- A focusable separator is the ARIA window-splitter widget (paneResizer adds the keys). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
<div
  class="pane-divider"
  role="separator"
  tabindex="0"
  aria-orientation="vertical"
  aria-label={label}
  title={RESIZE_TITLE}
  onmousedown={startDrag}
  ondblclick={() => set(defaultWidth)}
  use:paneResizer={{ value: width, min, max, invert, step: 10, bigStep: 40, onChange: set, onReset: () => set(defaultWidth), text: pxWide }}
></div>

<style>
  /* A 1px --separator hairline in a wider hit area; it lights up on hover/focus. */
  .pane-divider {
    flex: none;
    position: relative;
    z-index: var(--z-sticky);
    inline-size: 7px;
    margin-inline: -3px;
    cursor: col-resize;
    touch-action: none;
  }
  .pane-divider::after {
    content: '';
    position: absolute;
    inset-block: 0;
    inset-inline: 3px;
    background: var(--separator);
  }
  .pane-divider:hover::after,
  .pane-divider:focus-visible::after {
    inset-inline: 2px;
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .pane-divider:focus-visible {
    outline: none;
  }
  @media (max-width: 640px) {
    .pane-divider {
      display: none;
    }
  }
</style>
