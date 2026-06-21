<script lang="ts">
  // Recent executions: method + url + status + time. Click a row to reload it
  // into the builder; clear-all empties the workspace history.
  import Icon from '../../lib/components/Icon.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import type { ApiHistoryEntry } from '../../lib/api/types';

  function statusClass(status: number | null): string {
    if (status == null) return 'none';
    if (status >= 200 && status < 300) return 'ok';
    if (status >= 300 && status < 400) return 'redirect';
    if (status >= 400 && status < 500) return 'client';
    if (status >= 500) return 'server';
    return 'none';
  }

  function fmtTime(iso: string): string {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return iso;
    return d.toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  }

  function reload(h: ApiHistoryEntry): void {
    apiClient.loadHistoryIntoDraft(h);
  }

  async function clear(): Promise<void> {
    if (!(await confirmer.ask('Clear all request history for this workspace?', { title: 'Clear history', confirmLabel: 'Clear' }))) return;
    await apiClient.clearHistory();
  }
</script>

<div class="hist-wrap">
  <div class="hist-head">
    <span class="hist-title">History</span>
    {#if apiClient.history.length > 0}
      <button class="icon-btn" title="Clear history" aria-label="Clear history" onclick={clear}><Icon name="trash" size={13} /></button>
    {/if}
  </div>

  {#if apiClient.history.length === 0}
    <div class="empty-mini">No requests yet.</div>
  {:else}
    <VirtualList items={apiClient.history} estimateHeight={28} class="hist-vlist">
      {#snippet row(h: ApiHistoryEntry)}
        <button class="hist-row" onclick={() => reload(h)} title={h.url}>
          <span class="rm rm-{h.method.toLowerCase()}">{h.method}</span>
          <span class="hurl mono ellipsis grow">{h.url}</span>
          {#if h.status != null}
            <span class="status-dot {statusClass(h.status)}">{h.status}</span>
          {/if}
          <span class="htime">{fmtTime(h.executed_at)}</span>
        </button>
      {/snippet}
    </VirtualList>
  {/if}
</div>

<style>
  .hist-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    flex: 1;
  }
  .hist-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px 6px;
  }
  .hist-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  /* VirtualList container replaces .hist-list direct flex; keep for fallback reference */
  :global(.hist-vlist) {
    flex: 1;
    min-height: 0;
    max-height: 100%;
  }
  .hist-row {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 6px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    border-radius: var(--radius-s);
  }
  .hist-row:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .rm {
    font-size: 9.5px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-dim);
    width: 40px;
    flex-shrink: 0;
  }
  .rm-get { color: var(--status-working); }
  .rm-post { color: var(--accent); }
  .rm-put,
  .rm-patch { color: #d2691e; }
  .rm-delete { color: var(--status-exited); }
  .hurl {
    font-size: 11.5px;
    min-width: 0;
  }
  .status-dot {
    font-size: 10px;
    font-weight: 700;
    padding: 0 6px;
    height: 16px;
    line-height: 16px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .status-dot.ok { color: var(--status-working); background: color-mix(in srgb, var(--status-working) 16%, transparent); }
  .status-dot.redirect { color: var(--accent); background: color-mix(in srgb, var(--accent) 16%, transparent); }
  .status-dot.client { color: #d2691e; background: color-mix(in srgb, #d2691e 18%, transparent); }
  .status-dot.server { color: var(--status-exited); background: color-mix(in srgb, var(--status-exited) 16%, transparent); }
  .htime {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .empty-mini {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 2px;
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
