<script lang="ts">
  // Usage report: daily / monthly / per-model / per-session tables for the
  // page's window, tokens first and cost as the last (secondary) column — in
  // the style of ccusage. "Download HTML" saves the same tables as one
  // self-contained file (reportHtml.ts). Root also gets the opt-in ccusage
  // cross-check below the tables.
  import { onMount } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { downloadText } from '../../lib/components/exporters';
  import { usage } from '../../lib/api/usage.svelte';
  import { buildReportHtml, reportFileName, tokenCells, TOKEN_HEADERS } from './reportHtml';
  import CcusagePanel from './CcusagePanel.svelte';

  let { admin }: { admin: boolean } = $props();

  const SESSION_PAGE = 100;
  let sessionLimit = $state(SESSION_PAGE);

  onMount(() => {
    void usage.loadReport();
  });

  const report = $derived(usage.report);
  const sessions = $derived(report ? report.sessions.slice(0, sessionLimit) : []);

  function download(): void {
    if (!report) return;
    const file = reportFileName(report);
    downloadText(buildReportHtml(report), file, 'text/html');
    toasts.success('Saved the usage report', file);
  }

  function scopeLabel(): string {
    if (!report) return '';
    if (report.scope === 'own') return 'Your sessions';
    return report.otto_only ? 'Otto sessions' : 'All sessions on this Mac';
  }
</script>

<div class="report" data-testid="usage-report">
  <div class="report-bar">
    <span class="dim small">
      {#if report}{scopeLabel()} · last {report.days} days · cost estimated at rates as of {report.priced_as_of}{/if}
    </span>
    <button class="btn small" disabled={!report} onclick={download} title={report ? 'Save this report as one HTML file' : 'Load the report first'}>
      <Icon name="download" size={12} /> Download HTML
    </button>
  </div>

  {#if usage.reportError && !report}
    <LoadState what="the usage report" error={usage.reportError} empty onretry={() => void usage.loadReport()} loading={usage.reportLoading} />
  {:else if !report}
    <LoadState what="the usage report" loading empty rows={6} />
  {:else if report.totals.total_tokens === 0 && report.daily.length === 0}
    <EmptyState icon="chart" title="No usage in this window" body="Pick a longer window, or switch to All to include sessions run outside Otto." />
  {:else}
    {#if usage.reportError}
      <LoadState what="the usage report" error={usage.reportError} onretry={() => void usage.loadReport()} loading={usage.reportLoading} />
    {/if}
    {#snippet tokenTable(caption: string, lead: string[], rows: { key: string; lead: string[]; r: Parameters<typeof tokenCells>[0] }[], total?: Parameters<typeof tokenCells>[0])}
      <section class="panel card">
        <h3>{caption}</h3>
        {#if rows.length === 0}
          <p class="dim small">No usage in this window.</p>
        {:else}
          <div class="tbl-scroll">
            <table>
              <thead>
                <tr>
                  {#each lead as h (h)}<th scope="col">{h}</th>{/each}
                  {#each TOKEN_HEADERS as h (h)}<th scope="col" class="num">{h}</th>{/each}
                </tr>
              </thead>
              <tbody>
                {#each rows as row (row.key)}
                  <tr>
                    {#each row.lead as c, i (i)}<td class:lead-cell={i === 0}>{c}</td>{/each}
                    {#each tokenCells(row.r) as c, i (i)}<td class="num" class:dim={i === 5} class:strong={i === 4}>{c}</td>{/each}
                  </tr>
                {/each}
              </tbody>
              {#if total}
                <tfoot>
                  <tr>
                    <th scope="row" colspan={lead.length}>Total</th>
                    {#each tokenCells(total) as c, i (i)}<td class="num" class:dim={i === 5}>{c}</td>{/each}
                  </tr>
                </tfoot>
              {/if}
            </table>
          </div>
        {/if}
      </section>
    {/snippet}

    {@render tokenTable('Daily', ['Date'], report.daily.map((d) => ({ key: d.day, lead: [d.day], r: d })), report.totals)}
    {@render tokenTable('Monthly', ['Month'], report.monthly.map((m) => ({ key: m.month, lead: [m.month], r: m })), report.totals)}
    {@render tokenTable(
      'By model',
      ['Model', 'Provider'],
      report.models.map((m) => ({ key: m.provider + '\u0000' + m.model, lead: [m.model || 'unknown', m.provider], r: m })),
      report.totals,
    )}
    {@render tokenTable(
      'By session',
      ['Session', 'Workspace', 'Model', 'Last active'],
      sessions.map((s) => ({
        key: s.session_id,
        lead: [
          s.title ?? (s.kind != null ? s.session_id.slice(0, 12) : 'Session outside Otto'),
          s.workspace_name ?? '—',
          `${s.provider} · ${s.model || 'unknown'}`,
          s.last_active.slice(0, 16),
        ],
        r: s,
      })),
    )}
    {#if report.sessions.length > sessionLimit}
      <div class="more">
        <button class="btn small" onclick={() => (sessionLimit += SESSION_PAGE * 4)}>
          Show more sessions ({report.sessions.length - sessionLimit} hidden)
        </button>
      </div>
    {/if}
  {/if}

  {#if admin}
    <CcusagePanel />
  {/if}
</div>

<style>
  .report {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  .report-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
  }
  .panel {
    padding: 14px 16px;
    min-width: 0;
  }
  h3 {
    font-size: var(--fs-m);
    font-weight: 600;
    margin: 0 0 10px;
    color: var(--text);
  }
  .tbl-scroll {
    overflow-x: auto;
    max-width: 100%;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  th,
  td {
    padding: 5px 8px;
    border-bottom: 1px solid var(--separator);
    text-align: start;
    white-space: nowrap;
  }
  thead th {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  tfoot th,
  tfoot td {
    font-weight: 600;
    color: var(--text);
    border-bottom: none;
    border-top: 1px solid var(--border);
  }
  .num {
    text-align: end;
  }
  .lead-cell {
    color: var(--text);
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .strong {
    font-weight: 600;
    color: var(--text);
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  p.small {
    margin: 0;
  }
  .more {
    display: flex;
    justify-content: center;
  }
</style>
