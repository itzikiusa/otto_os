<script lang="ts">
  // Per-tool aggregates derived from the audit ledger: call count, error count +
  // rate, average / max latency, total bytes (the cost proxy — true USD is not
  // metered), and the last-called time. Read-only.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { plural } from '../../lib/plural';
  import { loadErrorText } from '../../lib/loadError';
  import { mcpCpApi } from '../../lib/api/mcp';
  import type { McpToolStats } from '../../lib/api/types';

  let stats = $state<McpToolStats[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);

  async function load(): Promise<void> {
    loading = true;
    try {
      stats = await mcpCpApi.cpStats();
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
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

<div class="stats" data-testid="mcp-stats">
  <div class="bar">
    <span class="count">{plural(stats.length, 'tool')}</span>
    <span class="muted small">Cost in USD is not metered — bytes are the available proxy.</span>
    <span class="grow"></span>
    <button class="icon-btn" onclick={() => void load()} disabled={loading} title="Refresh stats" aria-label="Refresh stats"><Icon name="refresh" size={13} /></button>
  </div>

  <LoadState what="tool stats" {loading} error={loadError} empty={stats.length === 0} onretry={() => void load()}>
    {#snippet emptyView()}
      <EmptyState icon="chart" title="No tool calls recorded yet" body="Per-tool stats build up from the audit ledger as governed tools are called." />
    {/snippet}
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
          <!-- Each cell carries its column name (`.cl`): visually hidden while the
               header row shows, inline once rows stack on tablet/phone. -->
          <span class="cell mono tool" title={s.tool}><span class="cl">Tool</span>{s.tool}</span>
          <span class="cell" title={s.server_name ?? undefined}><span class="cl">Server</span>{s.server_name ?? '—'}</span>
          <span class="cell num"><span class="cl">Calls</span>{s.calls}</span>
          <span class="cell num"><span class="cl">Errors</span>{s.errors}</span>
          <span class="cell num {errClass(s.error_rate)}"><span class="cl">Err rate</span>{(s.error_rate * 100).toFixed(1)}%</span>
          <span class="cell num"><span class="cl">Avg lat</span>{Math.round(s.avg_latency_ms)}ms</span>
          <span class="cell num"><span class="cl">Max lat</span>{s.max_latency_ms}ms</span>
          <span class="cell num"><span class="cl">Avg bytes</span>{fmtBytes(Math.round(s.avg_bytes))}</span>
          <span class="cell num"><span class="cl">Total bytes</span>{fmtBytes(s.total_bytes)}</span>
          <span class="cell when"><span class="cl">Last called</span>{s.last_called_at ? new Date(s.last_called_at).toLocaleString() : '—'}</span>
        </div>
      {/each}
    </div>
  </LoadState>
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
    font-size: var(--fs-s);
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
    padding: 6px 14px;
  }
  .thead {
    position: sticky;
    top: 0;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    z-index: 1;
  }
  .srow {
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    font-size: var(--fs-m);
  }
  .srow:hover {
    background: var(--hover);
  }
  .cell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .num {
    text-align: end;
  }
  .num.ok {
    color: var(--success);
  }
  .num.warn {
    color: var(--warning);
  }
  .num.bad {
    color: var(--danger);
  }
  .when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  /* The cell's column name: hidden (but announced) while the header row shows. */
  .cl {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }

  /* Tablet/phone: the header row goes and each row stacks into labelled cells
     (the tool name spans the row as its title). */
  @media (max-width: 1024px) {
    .thead {
      display: none;
    }
    .srow {
      grid-template-columns: repeat(3, minmax(0, 1fr));
      gap: 4px 12px;
      padding-block: 8px;
    }
    .srow > .tool {
      grid-column: 1 / -1;
      font-weight: 600;
    }
    .srow > .tool .cl {
      display: none;
    }
    .cell.num {
      text-align: start;
    }
    .cl {
      position: static;
      inline-size: auto;
      block-size: auto;
      overflow: visible;
      clip-path: none;
      margin-inline-end: 4px;
      font-size: var(--fs-xs);
      color: var(--text-dim);
    }
  }
</style>
