<script lang="ts">
  // Split layout: 1 pane full, 2 panes split on ws.splitAxis, 3–4 panes in a
  // 2×2 grid. Gutters are draggable (adjust col/row fractions).
  import SessionView from './SessionView.svelte';
  import DatabasePage from '../database/DatabasePage.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';

  let host: HTMLDivElement;

  const gridStyle = $derived.by(() => {
    const n = ws.panes.length;
    const c = Math.round(ws.colFrac * 1000) / 10;
    const r = Math.round(ws.rowFrac * 1000) / 10;
    if (n <= 1) return 'grid-template-columns: 1fr; grid-template-rows: 1fr;';
    if (n === 2) {
      return ws.splitAxis === 'col'
        ? `grid-template-columns: ${c}% ${100 - c}%; grid-template-rows: 1fr;`
        : `grid-template-columns: 1fr; grid-template-rows: ${r}% ${100 - r}%;`;
    }
    return `grid-template-columns: ${c}% ${100 - c}%; grid-template-rows: ${r}% ${100 - r}%;`;
  });

  function startDrag(axis: 'col' | 'row', e: PointerEvent): void {
    e.preventDefault();
    const rect = host.getBoundingClientRect();
    const move = (ev: PointerEvent) => {
      if (axis === 'col') {
        ws.colFrac = Math.min(0.8, Math.max(0.2, (ev.clientX - rect.left) / rect.width));
      } else {
        ws.rowFrac = Math.min(0.8, Math.max(0.2, (ev.clientY - rect.top) / rect.height));
      }
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  const showColGutter = $derived(
    ws.panes.length >= 3 || (ws.panes.length === 2 && ws.splitAxis === 'col'),
  );
  const showRowGutter = $derived(
    ws.panes.length >= 3 || (ws.panes.length === 2 && ws.splitAxis === 'row'),
  );

  const colGutterPos = $derived(`left: calc(${ws.colFrac * 100}% - 3px);`);
  const rowGutterPos = $derived(`top: calc(${ws.rowFrac * 100}% - 3px);`);
</script>

<div class="splits" bind:this={host}>
  <div class="grid" style={gridStyle}>
    {#each ws.panes as paneId, i (i)}
      {#if paneId === DB_PANE_ID}
        <div class="db-pane" role="group" aria-label="Database" class:focused={ws.focusedPane === i && ws.panes.length > 1} onpointerdown={() => ws.focusPane(i)}>
          {#if ws.panes.length > 1}
            <button class="db-pane-close" title="Close pane" aria-label="Close pane" onclick={() => ws.closePane(i)}>✕</button>
          {/if}
          <DatabasePage />
        </div>
      {:else}
        <SessionView
          sessionId={paneId}
          focused={ws.focusedPane === i && ws.panes.length > 1}
          showClose={ws.panes.length > 1}
          onfocus={() => ws.focusPane(i)}
          onclosepane={() => ws.closePane(i)}
        />
      {/if}
    {/each}
  </div>

  {#if showColGutter}
    <div
      class="gutter col"
      style={colGutterPos}
      onpointerdown={(e) => startDrag('col', e)}
      role="separator"
      aria-orientation="vertical"
    ></div>
  {/if}
  {#if showRowGutter}
    <div
      class="gutter row"
      style={rowGutterPos}
      onpointerdown={(e) => startDrag('row', e)}
      role="separator"
      aria-orientation="horizontal"
    ></div>
  {/if}
</div>

<style>
  .splits {
    position: relative;
    height: 100%;
    padding: 8px;
  }
  .grid {
    display: grid;
    gap: 8px;
    height: 100%;
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
    right: 8px;
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
  .gutter {
    position: absolute;
    z-index: 10;
  }
  .gutter.col {
    top: 8px;
    bottom: 8px;
    width: 8px;
    cursor: col-resize;
  }
  .gutter.row {
    left: 8px;
    right: 8px;
    height: 8px;
    cursor: row-resize;
  }
  .gutter:hover::after {
    content: '';
    position: absolute;
    inset: 0;
    margin: auto;
    background: color-mix(in srgb, var(--accent) 45%, transparent);
    border-radius: 2px;
  }
  .gutter.col:hover::after {
    width: 2px;
  }
  .gutter.row:hover::after {
    height: 2px;
  }
</style>
