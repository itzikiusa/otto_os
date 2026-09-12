<script lang="ts" module>
  // Shared by the pane ⋯ menu (SessionView) and the Database pane's ✕
  // right-click (below) — one list, so the five rows can't drift apart.
  import { layout as layoutStore, type Preset as PresetOp } from '../../lib/stores/splitLayout.svelte';
  import type { MenuItem as PresetMenuItem } from '../../lib/contextmenu.svelte';

  /** The five layout presets as flat ctxMenu rows (the menu has no submenus). */
  export function presetItems(): PresetMenuItem[] {
    const rows: [PresetOp, string][] = [
      ['cols', 'Layout: Equal columns'],
      ['rows', 'Layout: Equal rows'],
      ['one-two-below', 'Layout: One above two'],
      ['one-two-beside', 'Layout: One beside two'],
      ['grid', 'Layout: Grid'],
    ];
    return rows.map(([p, label]) => ({ label, icon: 'split', action: () => layoutStore.applyPreset(p) }));
  }
</script>

<script lang="ts">
  // One node of the split TREE (C2). A `split` lays its two children out with an
  // 8px draggable gutter between them; a `leaf` renders the pane itself plus the
  // C3a drop veil. The component mounts itself for a/b — the tree is arbitrarily
  // deep, so the recursion lives here rather than in Splits.svelte.
  //
  // Tracks are `fr`, never `%`: `f% 8px (100−f)%` sums to 100 % + 8 px and would
  // overflow 8 px per nesting level (the flat grid this replaced kept the gutter
  // outside the tracks via `gap`, which a tree cannot do).
  import Self from './SplitNode.svelte';
  import SessionView from './SessionView.svelte';
  import DatabasePage from '../database/DatabasePage.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import {
    layout,
    MIN_PANE_PX,
    type LayoutNode,
    type Side,
  } from '../../lib/stores/splitLayout.svelte';

  interface Props {
    node: LayoutNode;
    /** Nesting depth — informational (styling hook), the tree recurses on `node`. */
    depth?: number;
  }
  let { node, depth = 0 }: Props = $props();

  // ── split: gutter drag + keyboard resize ─────────────────────────────────
  let el = $state<HTMLDivElement | null>(null);

  /** Pointer drag on THIS node's gutter: the fraction is measured against the
   *  node's own rect, then guarded so neither side drops below MIN_PANE_PX. The
   *  guard is skipped when the node itself is narrower than two minimum panes —
   *  it cannot satisfy it, and clamping there would pin the gutter mid-drag. */
  function startDrag(e: PointerEvent): void {
    if (node.kind !== 'split' || !el) return;
    e.preventDefault();
    const rect = el.getBoundingClientRect();
    const { key, axis } = node;
    const rtl = getComputedStyle(el).direction === 'rtl';
    const move = (ev: PointerEvent): void => {
      const size = axis === 'col' ? rect.width : rect.height;
      if (size <= 0) return;
      const along =
        axis === 'col' ? (rtl ? rect.right - ev.clientX : ev.clientX - rect.left) : ev.clientY - rect.top;
      let frac = along / size;
      if (size >= 2 * MIN_PANE_PX) {
        frac = Math.min(1 - MIN_PANE_PX / size, Math.max(MIN_PANE_PX / size, frac));
      }
      layout.setFrac(key, frac);
    };
    const up = (): void => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  /** Keyboard resize (a11y — pointer-only otherwise): arrows nudge 2 %, Home/End
   *  jump to the 10/90 bounds clampFrac allows. */
  function gutterKeydown(e: KeyboardEvent): void {
    if (node.kind !== 'split') return;
    const horizontal = node.axis === 'col';
    const dec = horizontal ? e.key === 'ArrowLeft' : e.key === 'ArrowUp';
    const inc = horizontal ? e.key === 'ArrowRight' : e.key === 'ArrowDown';
    let next: number | null = null;
    if (dec) next = node.frac - 0.02;
    else if (inc) next = node.frac + 0.02;
    else if (e.key === 'Home') next = 0.1;
    else if (e.key === 'End') next = 0.9;
    if (next === null) return;
    e.preventDefault();
    layout.setFrac(node.key, next);
  }

  // ── leaf: index-based workspace calls + the C3a drop veil ─────────────────
  const idx = $derived(node.kind === 'leaf' ? layout.leaves.findIndex((l) => l.key === node.key) : -1);
  const paneCount = $derived(ws.panes.length);

  /** Drop zone under the cursor: the outer quarter on each side moves the pane
   *  there, the middle swaps the two sessions. */
  let zone = $state<Side | 'centre' | null>(null);
  const ZONE_LABEL: Record<Side | 'centre', string> = {
    centre: 'Swap',
    left: 'Move left',
    right: 'Move right',
    up: 'Move up',
    down: 'Move down',
  };

  function zoneAt(e: DragEvent): Side | 'centre' {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const px = r.width > 0 ? (e.clientX - r.left) / r.width : 0.5;
    const py = r.height > 0 ? (e.clientY - r.top) / r.height : 0.5;
    const edge = Math.min(px, 1 - px, py, 1 - py);
    if (edge > 0.25) return 'centre';
    if (edge === px) return 'left';
    if (edge === 1 - px) return 'right';
    if (edge === py) return 'up';
    return 'down';
  }

  function onVeilOver(e: DragEvent): void {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    zone = zoneAt(e);
  }

  function onVeilDrop(e: DragEvent): void {
    e.preventDefault();
    const from = layout.dragKey;
    const z = zone;
    zone = null;
    layout.dragKey = null;
    if (!from || node.kind !== 'leaf' || from === node.key || !z) return;
    if (z === 'centre') layout.swap(from, node.key);
    else layout.move(from, node.key, z);
    // `activeSessionId` follows the focused leaf and only focusPane routes to
    // navigateToSession — keep the URL + navigator highlight in step.
    ws.focusPane(layout.focusedIndex);
  }

</script>

{#if node.kind === 'split'}
  <div
    class="split-node"
    bind:this={el}
    data-axis={node.axis}
    data-key={node.key}
    data-depth={depth}
    style={node.axis === 'col'
      ? `grid-template-columns: ${node.frac}fr 8px ${1 - node.frac}fr; grid-template-rows: 1fr;`
      : `grid-template-rows: ${node.frac}fr 8px ${1 - node.frac}fr; grid-template-columns: 1fr;`}
  >
    <Self node={node.a} depth={depth + 1} />
    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      class="gutter"
      role="separator"
      tabindex="0"
      aria-orientation={node.axis === 'col' ? 'vertical' : 'horizontal'}
      aria-label="Resize split (arrow keys)"
      aria-valuenow={Math.round(node.frac * 100)}
      aria-valuemin={10}
      aria-valuemax={90}
      onpointerdown={startDrag}
      onkeydown={gutterKeydown}
    ></div>
    <Self node={node.b} depth={depth + 1} />
  </div>
{:else}
  <div class="leaf" data-pane-key={node.key} data-session={node.session}>
    {#if node.session === DB_PANE_ID}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="db-pane"
        role="group"
        aria-label="Database"
        class:focused={ws.focusedPane === idx && paneCount > 1}
        onpointerdown={() => ws.focusPane(idx)}
      >
        {#if paneCount > 1}
          <button
            class="db-pane-close"
            title="Close pane — right-click for layout presets"
            aria-label="Close pane"
            onclick={() => ws.closePane(idx)}
            oncontextmenu={(e) => ctxMenu.show(e, presetItems())}
          >✕</button>
        {/if}
        <DatabasePage />
      </div>
    {:else}
      <SessionView
        sessionId={node.session}
        focused={ws.focusedPane === idx && paneCount > 1}
        showClose={paneCount > 1}
        showGrip={paneCount > 1}
        dragKey={node.key}
        ondragpane={(phase) => (layout.dragKey = phase === 'start' ? node.key : null)}
        onfocus={() => ws.focusPane(idx)}
        closeTitle="Close session (⌘W)"
        onclosepane={() => void ws.requestCloseTab(node.session)}
      />
    {/if}
    <!-- Drop veil: hit-testable only while a pane drag is in flight, so the
         terminal below never sees the drag. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="drop-veil"
      class:armed={zone !== null}
      data-zone={zone ?? ''}
      hidden={!layout.dragKey || layout.dragKey === node.key}
      ondragover={onVeilOver}
      ondragleave={() => (zone = null)}
      ondrop={onVeilDrop}
    >
      {#if zone}
        <span class="drop-ind" data-ind={zone}></span>
        <span class="drop-cap">{ZONE_LABEL[zone]}</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .split-node {
    display: grid;
    min-width: 0;
    min-height: 0;
  }
  .split-node > :global(*) {
    min-width: 0;
    min-height: 0;
  }
  .leaf {
    position: relative;
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .leaf > :global(*) {
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .db-pane {
    position: relative;
    min-width: 0;
    min-height: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .db-pane.focused {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .db-pane-close {
    position: absolute;
    top: 6px;
    inset-inline-end: 8px;
    z-index: 25;
    width: 20px;
    height: 20px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
    font-size: 11px;
    line-height: 1;
  }
  .db-pane-close:hover {
    color: var(--text);
  }
  /* The gutter is its own 8px grid track, so it never overlaps a pane. */
  .gutter {
    position: relative;
    z-index: 10;
  }
  .split-node[data-axis='col'] > .gutter {
    cursor: col-resize;
  }
  .split-node[data-axis='row'] > .gutter {
    cursor: row-resize;
  }
  .gutter:focus-visible {
    outline: none;
  }
  /* Always drawn, faintly, so the resize handle is discoverable without
     hovering; hover / focus brighten it. */
  .gutter::after {
    content: '';
    position: absolute;
    inset: 0;
    margin: auto;
    background: var(--border);
    border-radius: 2px;
    transition: background 120ms ease-out;
  }
  .gutter:hover::after,
  .gutter:focus-visible::after {
    background: color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .gutter:focus-visible::after {
    background: color-mix(in srgb, var(--accent) 65%, transparent);
  }
  .split-node[data-axis='col'] > .gutter::after {
    width: 2px;
  }
  .split-node[data-axis='row'] > .gutter::after {
    height: 2px;
  }
  /* C3a drop target. `hidden` keeps it out of the hit-test entirely unless a
     pane drag is in flight (a permanently-mounted overlay would eat every
     terminal click). */
  .drop-veil {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-m);
  }
  .drop-veil[hidden] {
    display: none;
  }
  .drop-veil.armed {
    background: color-mix(in srgb, var(--accent) 8%, transparent);
    outline: 1px dashed color-mix(in srgb, var(--accent) 60%, transparent);
    outline-offset: -2px;
  }
  .drop-ind {
    position: absolute;
    background: color-mix(in srgb, var(--accent) 26%, transparent);
    border: 1px solid color-mix(in srgb, var(--accent) 70%, transparent);
    border-radius: var(--radius-s);
    pointer-events: none;
  }
  .drop-ind[data-ind='centre'] {
    inset: 6px;
  }
  .drop-ind[data-ind='left'] {
    inset: 6px 50% 6px 6px;
  }
  .drop-ind[data-ind='right'] {
    inset: 6px 6px 6px 50%;
  }
  .drop-ind[data-ind='up'] {
    inset: 6px 6px 50% 6px;
  }
  .drop-ind[data-ind='down'] {
    inset: 50% 6px 6px 6px;
  }
  .drop-cap {
    position: relative;
    padding: 2px 10px;
    border-radius: 99px;
    font-size: 11px;
    font-weight: 600;
    color: var(--accent-contrast, #fff);
    background: var(--accent);
    box-shadow: var(--shadow);
    pointer-events: none;
  }
</style>
