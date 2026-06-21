<script lang="ts">
  // Attribution drilldown panel (B1): "why did this cost so much?"
  // Groups usage_events by a chosen work-graph dimension (repo/branch/PR/…) and
  // shows cost + tokens per group. Supports CSV and JSON export via exporters.ts.
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { exportCsv, downloadJson, copyAsJson } from '../../lib/components/exporters';
  import type { AttributionRow, AttributionDim } from './types';
  import { DIM_LABELS } from './types';

  // Injected from the parent (UsagePage) — the current look-back window.
  let { days = 30 }: { days?: number } = $props();

  const DIMS: AttributionDim[] = [
    'origin',
    'repo',
    'branch',
    'pr',
    'story',
    'swarm_task',
    'workflow',
    'channel',
    'review',
  ];

  let selectedDim: AttributionDim = $state('origin');
  let rows: AttributionRow[] = $state([]);
  let loading = $state(false);
  let copiedRow: string | null = $state(null);

  async function load(): Promise<void> {
    loading = true;
    try {
      rows = await api.get<AttributionRow[]>(
        `/usage/attribution?by=${selectedDim}&days=${days}`,
      );
    } catch (e) {
      toasts.error(
        'Could not load attribution',
        e instanceof Error ? e.message : String(e),
      );
      rows = [];
    } finally {
      loading = false;
    }
  }

  // Reload whenever dim or window changes.
  $effect(() => {
    void load();
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    selectedDim; days;
  });

  const totalCost = $derived(rows.reduce((s, r) => s + r.cost_usd, 0));
  const totalTokens = $derived(rows.reduce((s, r) => s + r.tokens, 0));

  function fmtCost(n: number): string {
    if (n === 0) return '$0';
    if (n < 0.01) return '<$0.01';
    return '$' + n.toFixed(2);
  }
  function fmtNum(n: number): string {
    if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + 'M';
    if (n >= 1_000) return (n / 1_000).toFixed(1) + 'k';
    return String(n);
  }
  function pct(cost: number): number {
    return totalCost > 0 ? (cost / totalCost) * 100 : 0;
  }

  function exportRowsCsv(): void {
    exportCsv(
      rows.map((r) => ({
        [DIM_LABELS[selectedDim]]: r.key,
        cost_usd: r.cost_usd,
        tokens: r.tokens,
        sessions: r.sessions,
      })),
      `otto-attribution-${selectedDim}-${days}d.csv`,
    );
  }

  function exportRowsJson(): void {
    downloadJson(rows, `otto-attribution-${selectedDim}-${days}d.json`);
  }

  async function copyRow(r: AttributionRow): Promise<void> {
    await copyAsJson(r);
    copiedRow = r.key;
    setTimeout(() => {
      copiedRow = null;
    }, 1500);
  }
</script>

<div class="attribution-panel">
  <div class="attr-header">
    <h3 class="attr-title">Cost Attribution</h3>
    <span class="attr-subtitle">Why did this cost so much?</span>
    <div class="attr-controls">
      <label class="dim-label" for="attr-dim-select">Group by</label>
      <select
        id="attr-dim-select"
        class="dim-select"
        bind:value={selectedDim}
      >
        {#each DIMS as dim}
          <option value={dim}>{DIM_LABELS[dim]}</option>
        {/each}
      </select>
      {#if rows.length > 0}
        <button class="export-btn" onclick={exportRowsCsv} title="Export CSV">CSV</button>
        <button class="export-btn" onclick={exportRowsJson} title="Export JSON">JSON</button>
      {/if}
    </div>
  </div>

  {#if loading}
    <div class="attr-loading">Loading attribution…</div>
  {:else if rows.length === 0}
    <div class="attr-empty">
      No attributed usage for this dimension in the selected window.
      Sessions need a work reference stamped at creation time (review/product/swarm
      runners do this automatically; manual sessions carry origin="manual").
    </div>
  {:else}
    <div class="attr-table-wrap">
      <table class="attr-table">
        <thead>
          <tr>
            <th class="col-key">{DIM_LABELS[selectedDim]}</th>
            <th class="col-bar"></th>
            <th class="col-cost">Cost</th>
            <th class="col-pct">%</th>
            <th class="col-tokens">Tokens</th>
            <th class="col-sessions">Sessions</th>
            <th class="col-copy"></th>
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.key)}
            <tr class="attr-row">
              <td class="col-key attr-key" title={r.key}>{r.key}</td>
              <td class="col-bar">
                <div class="bar-track">
                  <div class="bar-fill" style="width: {pct(r.cost_usd).toFixed(1)}%"></div>
                </div>
              </td>
              <td class="col-cost">{fmtCost(r.cost_usd)}</td>
              <td class="col-pct dim-pct">{pct(r.cost_usd).toFixed(1)}%</td>
              <td class="col-tokens">{fmtNum(r.tokens)}</td>
              <td class="col-sessions">{r.sessions}</td>
              <td class="col-copy">
                <button
                  class="copy-btn"
                  onclick={() => copyRow(r)}
                  title="Copy row as JSON"
                >
                  {copiedRow === r.key ? '✓' : '⧉'}
                </button>
              </td>
            </tr>
          {/each}
        </tbody>
        <tfoot>
          <tr class="attr-total">
            <td class="col-key">Total</td>
            <td class="col-bar"></td>
            <td class="col-cost">{fmtCost(totalCost)}</td>
            <td class="col-pct">100%</td>
            <td class="col-tokens">{fmtNum(totalTokens)}</td>
            <td class="col-sessions"></td>
            <td class="col-copy"></td>
          </tr>
        </tfoot>
      </table>
    </div>
  {/if}
</div>

<style>
  .attribution-panel {
    border: 1px solid var(--border, #30363d);
    border-radius: 8px;
    padding: 16px;
    background: var(--surface-2, #161b22);
    margin-bottom: 16px;
  }

  .attr-header {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
    margin-bottom: 14px;
  }
  .attr-title {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--fg, #e6edf3);
  }
  .attr-subtitle {
    font-size: 12px;
    color: var(--fg-muted, #8b949e);
    flex: 1;
  }
  .attr-controls {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dim-label {
    font-size: 12px;
    color: var(--fg-muted, #8b949e);
  }
  .dim-select {
    font-size: 12px;
    padding: 3px 6px;
    background: var(--surface-3, #0d1117);
    color: var(--fg, #e6edf3);
    border: 1px solid var(--border, #30363d);
    border-radius: 4px;
  }
  .export-btn {
    font-size: 11px;
    padding: 3px 8px;
    background: var(--surface-3, #0d1117);
    color: var(--fg-muted, #8b949e);
    border: 1px solid var(--border, #30363d);
    border-radius: 4px;
    cursor: pointer;
  }
  .export-btn:hover {
    color: var(--fg, #e6edf3);
    border-color: var(--fg-muted, #8b949e);
  }

  .attr-loading,
  .attr-empty {
    font-size: 12px;
    color: var(--fg-muted, #8b949e);
    padding: 24px 0;
    text-align: center;
  }

  .attr-table-wrap {
    overflow-x: auto;
  }
  .attr-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .attr-table thead th {
    text-align: start;
    font-weight: 600;
    color: var(--fg-muted, #8b949e);
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--border, #30363d);
    white-space: nowrap;
  }
  .attr-row td {
    padding: 5px 8px;
    border-bottom: 1px solid var(--border-subtle, #21262d);
    color: var(--fg, #e6edf3);
    vertical-align: middle;
  }
  .attr-row:last-child td {
    border-bottom: none;
  }
  .attr-total td {
    padding: 5px 8px;
    border-top: 1px solid var(--border, #30363d);
    font-weight: 600;
    color: var(--fg-muted, #8b949e);
  }

  .col-key { max-width: 200px; }
  .attr-key {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono, monospace);
    font-size: 11px;
  }
  .col-bar { width: 120px; }
  .bar-track {
    height: 6px;
    background: var(--surface-3, #0d1117);
    border-radius: 3px;
    overflow: hidden;
  }
  .bar-fill {
    height: 100%;
    background: var(--accent, #388bfd);
    border-radius: 3px;
    transition: width 0.2s;
  }
  .col-cost, .col-pct, .col-tokens, .col-sessions { text-align: end; }
  .dim-pct { color: var(--fg-muted, #8b949e); }
  .col-copy { width: 32px; text-align: center; }
  .copy-btn {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--fg-muted, #8b949e);
    font-size: 13px;
    padding: 2px 4px;
    border-radius: 3px;
    line-height: 1;
  }
  .copy-btn:hover { color: var(--fg, #e6edf3); background: var(--surface-3, #0d1117); }
</style>
