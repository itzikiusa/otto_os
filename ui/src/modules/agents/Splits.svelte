<script lang="ts">
  // Split layout: 1 pane full, 2 panes split on ws.splitAxis, 3–4 panes in a
  // 2×2 grid. Gutters are draggable (adjust col/row fractions).
  import SessionView from './SessionView.svelte';
  import DatabasePage from '../database/DatabasePage.svelte';
  import { ws, DB_PANE_ID } from '../../lib/stores/workspace.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { BroadcastResp } from '../../lib/api/types';

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

  // ── Broadcast-input mode ────────────────────────────────────────────────
  // When on (only available with ≥2 panes), a compose bar appears below the
  // grid; pressing Enter relays the text to all visible session panes via
  // the existing `POST /workspaces/{id}/broadcast` endpoint (targets only
  // the pane session ids, not every live session in the workspace).
  let broadcastMode = $state(false);
  let broadcastText = $state('');
  let broadcastBusy = $state(false);

  const broadcastTargets = $derived(
    ws.panes.filter((id) => id !== DB_PANE_ID),
  );

  // Auto-disable broadcast mode when panes collapse to 1 or 0.
  $effect(() => {
    if (ws.panes.length < 2) broadcastMode = false;
  });

  async function sendBroadcast(): Promise<void> {
    const text = broadcastText.trim();
    if (!text || broadcastBusy || !ws.currentId) return;
    broadcastBusy = true;
    try {
      const resp = await api.post<BroadcastResp>(
        `/workspaces/${ws.currentId}/broadcast`,
        { text, session_ids: broadcastTargets },
      );
      broadcastText = '';
      toasts.info('Broadcast sent', `Delivered to ${resp.session_ids.length} session(s).`);
    } catch (e) {
      toasts.error('Broadcast failed', e instanceof Error ? e.message : String(e));
    } finally {
      broadcastBusy = false;
    }
  }

  function onBroadcastKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void sendBroadcast();
    }
    if (e.key === 'Escape') {
      broadcastMode = false;
      broadcastText = '';
    }
  }
</script>

<div class="splits" bind:this={host} class:has-broadcast={broadcastMode}>
  {#if ws.panes.length >= 2 && broadcastTargets.length >= 2}
    <div class="broadcast-bar-wrap">
      <button
        class="broadcast-toggle"
        class:active={broadcastMode}
        onclick={() => { broadcastMode = !broadcastMode; }}
        title={broadcastMode ? 'Exit broadcast mode' : 'Broadcast input to all visible sessions'}
        aria-pressed={broadcastMode}
      >{broadcastMode ? '↗ exit broadcast' : '↗ broadcast'}</button>
      {#if broadcastMode}
        <!-- svelte-ignore a11y_autofocus -->
        <input
          class="broadcast-input"
          bind:value={broadcastText}
          placeholder="Send to all visible sessions — Enter to send"
          disabled={broadcastBusy}
          autofocus
          onkeydown={onBroadcastKeydown}
        />
        <button
          class="broadcast-send"
          disabled={broadcastBusy || broadcastText.trim() === ''}
          onclick={() => void sendBroadcast()}
        >{broadcastBusy ? '…' : 'Send'}</button>
      {/if}
    </div>
  {/if}
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
    display: flex;
    flex-direction: column;
  }
  .splits.has-broadcast {
    padding-bottom: 0;
  }
  .broadcast-bar-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 0 4px 0;
    flex-shrink: 0;
  }
  .broadcast-toggle {
    flex-shrink: 0;
    font-size: 11px;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: pointer;
    transition: background 120ms, color 120ms, border-color 120ms;
  }
  .broadcast-toggle.active {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
  }
  .broadcast-toggle:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .broadcast-input {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 8px;
    font-size: 12px;
    background: var(--surface);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    outline: none;
  }
  .broadcast-send {
    flex-shrink: 0;
    height: 26px;
    padding: 0 12px;
    font-size: 12px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: #fff;
    cursor: pointer;
    transition: opacity 120ms;
  }
  .broadcast-send:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .grid {
    display: grid;
    gap: 8px;
    flex: 1;
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
    inset-inline-start: 8px;
    inset-inline-end: 8px;
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
