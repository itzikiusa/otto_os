<script lang="ts">
  // Per-tool aggregates derived from the audit ledger: call count, error count +
  // rate, average / max latency, total bytes (the cost proxy — true USD is not
  // metered), and the last-called time. Read-only.
  import Icon from '../../lib/components/Icon.svelte';
  import { mcpCpApi } from '../../lib/api/mcp';
  import { toasts } from '../../lib/toast.svelte';
  import type { McpToolStats } from '../../lib/api/types';

  let stats = $state<McpToolStats[]>([]);
  let loading = $state(false);

  async function load(): Promise<void> {
    loading = true;
    try {
      stats = await mcpCpApi.cpStats();
    } catch (e) {
      toasts.error('Failed to load stats', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });

  function fmtBytes(b: number): string {
    if (b < 1024) return `${b} B`;
    if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
    return `${(b / 1024 / 1024).toFixed(1)} MB`;
  }
  function errClass(rate: number): string {
    return rate >= 0.25 ? 'bad' : rate > 0 ? 'warn' : 'ok';
  }
</script>

<div class="stats">
  <div class="bar">
    <span class="count">{stats.length} tool{stats.length === 1 ? '' : 's'}</span>
    <span class="muted small">Cost in USD is not metered — bytes are the available proxy.</span>
    <span class="grow"></span>
    <button class="btn small" onclick={() => void load()} title="Refresh"><Icon name="refresh" size={13} /></button>
  </div>

  {#if loading && stats.length === 0}
    <p class="muted pad">Loading…</p>
  {:else if stats.length === 0}
    <div class="empty">
      <Icon name="chart" size={22} />
      <p>No tool calls recorded yet. Stats build up from the audit ledger.</p>
    </div>
  {:else}
    <div class="grid">
      <div class="thead">
        <span>Tool</span>
        <span>Server</span>
        <span class="num">Calls</span>
        <span class="num">Errors</span>
        <span class="num">Err rate</span>
        <span class="num">Avg lat</span>
        <span class="num">Max lat</span>
        <span class="num">Avg bytes</span>
        <span class="num">Total bytes</span>
        <span>Last called</span>
      </div>
      {#each stats as s (`${s.server_id ?? ''}:${s.tool}`)}
        <div class="srow">
          <span class="cell mono">{s.tool}</span>
          <span class="cell">{s.server_name ?? '—'}</span>
          <span class="cell num">{s.calls}</span>
          <span class="cell num">{s.errors}</span>
          <span class="cell num {errClass(s.error_rate)}">{(s.error_rate * 100).toFixed(1)}%</span>
          <span class="cell num">{Math.round(s.avg_latency_ms)}ms</span>
          <span class="cell num">{s.max_latency_ms}ms</span>
          <span class="cell num">{fmtBytes(Math.round(s.avg_bytes))}</span>
          <span class="cell num">{fmtBytes(s.total_bytes)}</span>
          <span class="cell when">{s.last_called_at ? new Date(s.last_called_at).toLocaleString() : '—'}</span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .stats {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .count {
    font-size: 12px;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .grid {
    overflow: auto;
  }
  .thead,
  .srow {
    display: grid;
    grid-template-columns: minmax(140px, 1.4fr) minmax(100px, 1fr) 60px 60px 70px 70px 70px 80px 90px 160px;
    align-items: center;
    gap: 8px;
    padding: 7px 14px;
  }
  .thead {
    position: sticky;
    top: 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    z-index: 1;
  }
  .srow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: 12.5px;
  }
  .srow:hover {
    background: color-mix(in srgb, var(--text-dim) 5%, transparent);
  }
  .cell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    text-align: right;
  }
  .num.ok {
    color: var(--status-working, #28c840);
  }
  .num.warn {
    color: #e0a000;
  }
  .num.bad {
    color: var(--status-exited, #ff5f57);
  }
  .when {
    font-size: 11px;
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .pad {
    padding: 16px;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    color: var(--text-dim);
    text-align: center;
    padding: 36px 24px;
  }

  @media (max-width: 900px) {
    .thead {
      display: none;
    }
    .srow {
      grid-template-columns: 1fr 1fr 1fr;
      gap: 4px 8px;
    }
  }
</style>
