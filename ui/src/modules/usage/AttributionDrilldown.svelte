<script lang="ts">
  // Attribution drilldown panel (B1): "why did this cost so much?"
  // Groups usage_events by a chosen work-graph dimension (repo/branch/PR/…) and
  // shows cost + tokens per group. Supports CSV and JSON export via exporters.ts.
  import { api } from '../../lib/api/client';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
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
  /** Last load failure — shown inline with Retry (an empty table would read
   *  as "nothing attributed", which is not what happened). */
  let error = $state('');
  let copiedRow: string | null = $state(null);

  // Request token: switching dimension/window while a query is in flight must
  // not let the older (often slower) response land under the new labels.
  let seq = 0;

  // The dimension/window the current `rows` belong to: a new grouping clears
  // them so the old dimension's keys never sit under the new column label.
  let rowsKey = '';

  async function load(): Promise<void> {
    const mine = ++seq;
    const key = `${selectedDim}:${days}`;
    if (key !== rowsKey) {
      rows = [];
      rowsKey = key;
    }
    loading = true;
    try {
      const next = await api.get<AttributionRow[]>(
        `/usage/attribution?by=${selectedDim}&days=${days}`,
      );
      if (mine === seq) {
        rows = next;
        error = '';
      }
    } catch (e) {
      if (mine !== seq) return;
      error = loadErrorText(e);
      rows = [];
    } finally {
      if (mine === seq) loading = false;
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
    toasts.success('Exported attribution as CSV', `otto-attribution-${selectedDim}-${days}d.csv`);
  }

  function exportRowsJson(): void {
    downloadJson(rows, `otto-attribution-${selectedDim}-${days}d.json`);
    toasts.success('Exported attribution as JSON', `otto-attribution-${selectedDim}-${days}d.json`);
  }

  async function copyRow(r: AttributionRow): Promise<void> {
    // A blocked clipboard must not flash the "copied" tick — swallow it here
    // and leave the row unmarked so the failure is visible, not silent.
    try {
      await copyAsJson(r);
    } catch {
      toasts.error("Couldn't copy the row", 'Clipboard access was blocked.');
      return;
    }
    copiedRow = r.key;
    setTimeout(() => {
      copiedRow = null;
    }, 1500);
  }
</script>

<section class="card attribution-panel" aria-labelledby="attr-title">
  <div class="attr-header">
    <div class="attr-heading">
      <h3 class="attr-title" id="attr-title">Cost attribution</h3>
      <span class="attr-subtitle">Where the spend went, grouped by the work it was for</span>
    </div>
    <div class="attr-controls">
      <label class="dim-label" for="attr-dim-select">Group by</label>
      <select id="attr-dim-select" class="input dim-select" bind:value={selectedDim}>
        {#each DIMS as dim}
          <option value={dim}>{DIM_LABELS[dim]}</option>
        {/each}
      </select>
      {#if rows.length > 0}
        <button class="btn small ghost" onclick={exportRowsCsv} title="Download as CSV" aria-label="Download attribution as CSV">
          <Icon name="download" size={12} /> CSV
        </button>
        <button class="btn small ghost" onclick={exportRowsJson} title="Download as JSON" aria-label="Download attribution as JSON">
          <Icon name="download" size={12} /> JSON
        </button>
      {/if}
    </div>
  </div>

  <LoadState
    what="cost attribution"
    {loading}
    {error}
    empty={rows.length === 0}
    rows={3}
    onretry={() => void load()}
  >
    {#snippet emptyView()}
      <EmptyState
        icon="chart"
        title="Nothing attributed by {DIM_LABELS[selectedDim].toLowerCase()} in the last {days} days"
        body={selectedDim === 'origin'
          ? 'Usage shows up here once agents run in this window.'
          : 'Reviews, product stories, swarms and workflows tag the sessions they start. Ad-hoc sessions only appear under Origin.'}
      />
    {/snippet}
    <div class="attr-table-wrap">
      <table class="attr-table">
        <thead>
          <tr>
            <th class="col-key">{DIM_LABELS[selectedDim]}</th>
            <th class="col-bar"><span class="sr-only">Share of cost</span></th>
            <th class="col-cost">Cost</th>
            <th class="col-pct">Share</th>
            <th class="col-tokens">Tokens</th>
            <th class="col-sessions">Sessions</th>
            <th class="col-copy"><span class="sr-only">Actions</span></th>
          </tr>
        </thead>
        <tbody>
          {#each rows as r (r.key)}
            <tr class="attr-row">
              <td class="col-key"><span class="attr-key" title={r.key}>{r.key}</span></td>
              <td class="col-bar">
                <div class="bar-track">
                  <div class="bar-fill" style="width: {pct(r.cost_usd).toFixed(1)}%"></div>
                </div>
              </td>
              <td class="col-cost">{fmtCost(r.cost_usd)}</td>
              <td class="col-pct dim-pct">{pct(r.cost_usd).toFixed(1)}%</td>
              <td class="col-tokens" title="{r.tokens.toLocaleString()} tokens">{fmtNum(r.tokens)}</td>
              <td class="col-sessions">{r.sessions}</td>
              <td class="col-copy">
                <button
                  class="icon-btn"
                  onclick={() => copyRow(r)}
                  title={copiedRow === r.key ? 'Copied' : 'Copy row as JSON'}
                  aria-label={copiedRow === r.key ? 'Copied' : 'Copy row as JSON'}
                >
                  <Icon name={copiedRow === r.key ? 'check' : 'copy'} size={14} />
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
  </LoadState>
</section>

<style>
  /* Same card + heading treatment as the other Usage panels (global .card). */
  .attribution-panel {
    padding: 14px 16px;
    min-width: 0;
  }

  .attr-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px 12px;
    flex-wrap: wrap;
    margin-bottom: 12px;
  }
  .attr-heading {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 2px 10px;
    min-width: 0;
  }
  .attr-title {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .attr-subtitle {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .attr-controls {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .dim-label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .dim-select {
    width: auto;
  }

  .attr-table-wrap {
    overflow-x: auto;
  }
  .attr-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .attr-table thead th {
    text-align: start;
    font-weight: 600;
    color: var(--text-dim);
    padding: 4px 8px 6px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
  }
  .attr-table thead th.col-cost,
  .attr-table thead th.col-pct,
  .attr-table thead th.col-tokens,
  .attr-table thead th.col-sessions {
    text-align: end;
  }
  .attr-row td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--separator);
    color: var(--text);
    vertical-align: middle;
  }
  .attr-row:hover td {
    background: var(--hover);
  }
  .attr-row:last-child td {
    border-bottom: none;
  }
  .attr-total td {
    padding: 6px 8px;
    border-top: 1px solid var(--border);
    font-weight: 600;
    color: var(--text-dim);
  }

  .col-key {
    max-width: 260px;
  }
  .attr-key {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .col-bar {
    width: 120px;
  }
  .bar-track {
    height: 6px;
    background: var(--surface-2);
    border-radius: 3px;
    overflow: hidden;
  }
  .bar-fill {
    height: 100%;
    background: var(--accent-solid);
    border-radius: 3px;
    transition: width 0.2s;
  }
  @media (prefers-reduced-motion: reduce) {
    .bar-fill {
      transition: none;
    }
  }
  .col-cost,
  .col-pct,
  .col-tokens,
  .col-sessions {
    text-align: end;
    white-space: nowrap;
  }
  .dim-pct {
    color: var(--text-dim);
  }
  .col-copy {
    width: 32px;
    text-align: center;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }
</style>
