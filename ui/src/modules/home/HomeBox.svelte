<script lang="ts">
  // One tile on the Home grid: a header (kind icon + title, refresh, zoom,
  // menu), the kind's live body, and — in the grid — a drag handle for
  // reordering plus a corner handle for resizing in grid units. The same
  // component renders the zoomed (full-page) box, minus the grid affordances.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { home, COLS, GAP_PX, MIN_H, MAX_H, MIN_W, ROW_PX, type HomeBox } from './home.svelte';
  import { kindDef } from './kinds';
  import SessionsBox from './boxes/SessionsBox.svelte';
  import MissionControlBox from './boxes/MissionControlBox.svelte';
  import DbDashboardBox from './boxes/DbDashboardBox.svelte';
  import K8sBox from './boxes/K8sBox.svelte';
  import InsightsBox from './boxes/InsightsBox.svelte';
  import UsageBox from './boxes/UsageBox.svelte';

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed?: boolean;
    /** Index within the view (for the move-left/right menu). */
    index?: number;
    count?: number;
    /** Drag-reorder plumbing (grid only). */
    ondragbox?: (id: string) => void;
    ondropon?: (id: string) => void;
  }
  let { box, viewId, zoomed = false, index = 0, count = 1, ondragbox, ondropon }: Props = $props();

  const def = $derived(kindDef(box.kind));
  // Bumped by the header's refresh button; each box body re-fetches on change.
  let tick = $state(0);

  // ── Resize (grid units) ──────────────────────────────────────────────────
  // Pointer-capture drag on the corner handle. The column width is measured
  // from the tile's own box (cols = current span) so no grid metrics need to
  // be threaded down; rows are the fixed ROW_PX unit.
  let el = $state<HTMLElement | null>(null);
  let resizing = $state(false);
  function onResizeStart(e: PointerEvent): void {
    if (!el || viewport.isPhone) return;
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const rect = el.getBoundingClientRect();
    const colPx = (rect.width - GAP_PX * (box.w - 1)) / box.w + GAP_PX;
    const rowPx = ROW_PX + GAP_PX;
    const startX = e.clientX;
    const startY = e.clientY;
    const w0 = box.w;
    const h0 = box.h;
    resizing = true;
    home.interacting = true;
    const move = (ev: PointerEvent): void => {
      const w = Math.min(COLS, Math.max(MIN_W, Math.round(w0 + (ev.clientX - startX) / colPx)));
      const h = Math.min(MAX_H, Math.max(MIN_H, Math.round(h0 + (ev.clientY - startY) / rowPx)));
      home.resizeBox(viewId, box.id, w, h);
    };
    const up = (): void => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      resizing = false;
      home.interacting = false;
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  }

  // Keyboard resize on the handle: arrows nudge one grid unit.
  function onResizeKey(e: KeyboardEvent): void {
    const d: Record<string, [number, number]> = { ArrowRight: [1, 0], ArrowLeft: [-1, 0], ArrowDown: [0, 1], ArrowUp: [0, -1] };
    const v = d[e.key];
    if (!v) return;
    e.preventDefault();
    home.resizeBox(viewId, box.id, box.w + v[0], box.h + v[1]);
  }

  // ── Drag reorder (HTML5 DnD on the header handle) ───────────────────────
  let dragOver = $state(false);
  function onDragStart(e: DragEvent): void {
    e.dataTransfer?.setData('text/plain', box.id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
    home.interacting = true;
    ondragbox?.(box.id);
  }
  function onDragEnd(): void {
    home.interacting = false;
  }
  function onDragOver(e: DragEvent): void {
    if (!ondropon) return;
    e.preventDefault();
    dragOver = true;
  }
  function onDrop(e: DragEvent): void {
    e.preventDefault();
    dragOver = false;
    ondropon?.(box.id);
  }

  function menu(e: MouseEvent | KeyboardEvent): void {
    const others = home.views.filter((v) => v.id !== viewId);
    ctxMenu.show(e, [
      { label: 'Refresh', icon: 'refresh', action: () => (tick += 1) },
      { label: zoomed ? 'Exit zoom' : 'Zoom in', icon: zoomed ? 'minimize' : 'maximize', action: () => home.toggleZoom(box.id) },
      { label: `Open ${def.label}`, icon: 'external', action: () => router.go(def.route) },
      { separator: true },
      ...(zoomed
        ? []
        : [
            { label: 'Move left', icon: 'chevronLeft', disabled: index <= 0, action: () => home.moveBox(viewId, box.id, index - 1) },
            { label: 'Move right', icon: 'chevronRight', disabled: index >= count - 1, action: () => home.moveBox(viewId, box.id, index + 1) },
            { label: 'Fill width', icon: 'maximize', disabled: box.w === COLS, action: () => home.resizeBox(viewId, box.id, COLS, box.h) },
            { label: 'Reset size', icon: 'square', action: () => home.resizeBox(viewId, box.id, def.w, def.h) },
          ]),
      ...others.map((v) => ({
        label: `Move to “${v.name}”`,
        icon: 'grid',
        action: () => {
          home.moveBoxToView(viewId, box.id, v.id);
        },
      })),
      { separator: true },
      { label: 'Remove box', icon: 'trash', danger: true, action: () => home.removeBox(viewId, box.id) },
    ]);
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<section
  class="hbox"
  class:zoomed
  class:resizing
  class:drag-over={dragOver}
  bind:this={el}
  style:grid-column={zoomed || viewport.isPhone ? null : `span ${box.w}`}
  style:grid-row={zoomed ? null : `span ${box.h}`}
  data-kind={box.kind}
  data-box-id={box.id}
  aria-label={def.label}
  oncontextmenu={menu}
  ondragover={onDragOver}
  ondragleave={() => (dragOver = false)}
  ondrop={onDrop}
>
  <header class="hb-head" ondblclick={() => home.toggleZoom(box.id)}>
    {#if !zoomed && !viewport.isPhone}
      <span class="grip" draggable="true" ondragstart={onDragStart} ondragend={onDragEnd} title="Drag to reorder" aria-hidden="true"><Icon name="grip" size={12} /></span>
    {/if}
    <Icon name={def.icon} size={13} />
    <span class="hb-title ellipsis">{def.label}</span>
    <button class="icon-btn" onclick={() => (tick += 1)} title="Refresh" aria-label="Refresh {def.label}"><Icon name="refresh" size={12} /></button>
    <button class="icon-btn" onclick={() => home.toggleZoom(box.id)} title={zoomed ? 'Exit zoom (Esc)' : 'Zoom in'} aria-label={zoomed ? 'Exit zoom' : 'Zoom in'}>
      <Icon name={zoomed ? 'minimize' : 'maximize'} size={12} />
    </button>
    <button class="icon-btn" onclick={menu} title="More" aria-label="Box menu"><Icon name="dot" size={12} /></button>
  </header>
  <div class="hb-body">
    {#if box.kind === 'sessions'}
      <SessionsBox {box} {zoomed} {tick} />
    {:else if box.kind === 'mission-control'}
      <MissionControlBox {box} {zoomed} {tick} />
    {:else if box.kind === 'db-dashboard'}
      <DbDashboardBox {box} {viewId} {zoomed} {tick} />
    {:else if box.kind === 'k8s'}
      <K8sBox {box} {viewId} {zoomed} {tick} />
    {:else if box.kind === 'insights'}
      <InsightsBox {box} {zoomed} {tick} />
    {:else if box.kind === 'usage'}
      <UsageBox {box} {viewId} {zoomed} {tick} />
    {/if}
  </div>
  {#if !zoomed && !viewport.isPhone}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="resize"
      role="slider"
      tabindex="0"
      aria-label="Resize {def.label} ({box.w} × {box.h})"
      aria-valuetext="{box.w} columns by {box.h} rows"
      aria-valuenow={box.w}
      aria-valuemin={MIN_W}
      aria-valuemax={COLS}
      title="Drag to resize · arrows to nudge"
      onpointerdown={onResizeStart}
      onkeydown={onResizeKey}
    ></div>
  {/if}
</section>

<style>
  .hbox {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    transition: box-shadow 130ms ease-out, border-color 130ms ease-out;
  }
  .hbox.resizing {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 30%, transparent);
    user-select: none;
  }
  .hbox.drag-over {
    border-color: var(--accent);
  }
  .hbox.zoomed {
    height: 100%;
  }
  .hb-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 6px 6px 10px;
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
    flex: none;
    user-select: none;
  }
  .grip {
    display: inline-grid;
    place-items: center;
    cursor: grab;
    color: var(--text-dim);
    margin-inline-start: -4px;
  }
  .grip:active {
    cursor: grabbing;
  }
  .hb-title {
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
  }
  .hb-body {
    flex: 1;
    min-height: 0;
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .hb-body > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .resize {
    position: absolute;
    inset-inline-end: 0;
    bottom: 0;
    width: 16px;
    height: 16px;
    cursor: nwse-resize;
    touch-action: none;
  }
  :global([dir='rtl']) .resize {
    cursor: nesw-resize;
  }
  .resize::after {
    content: '';
    position: absolute;
    inset-inline-end: 3px;
    bottom: 3px;
    width: 8px;
    height: 8px;
    border-inline-end: 2px solid var(--text-dim);
    border-bottom: 2px solid var(--text-dim);
    border-end-end-radius: 2px;
    opacity: 0.45;
  }
  .hbox:hover .resize::after,
  .resize:focus-visible::after {
    opacity: 1;
  }
  .resize:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
