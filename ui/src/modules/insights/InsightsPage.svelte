<script lang="ts">
  // Insights view — lists generated HTML insight reports (filterable by kind),
  // opens a report in an in-app iframe, and runs an ad-hoc report on demand.
  // Scheduled reports are opt-in (Settings → Insights) and catch-up.
  import { insightsApi } from '../../lib/api/insights';
  import type {
    InsightKind,
    InsightReport,
    InsightRunPeriod,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';

  let reports: InsightReport[] = $state([]);
  let loading = $state(true);
  let running = $state(false);

  // Filter: 'all' | kind
  let filter: 'all' | InsightKind = $state('all');

  // Run-now period picker
  let runPeriod: InsightRunPeriod = $state('day');

  // Currently-open report (rendered in the iframe overlay).
  let openReport: InsightReport | null = $state(null);
  let openUrl: string | null = $state(null);
  let openLoading = $state(false);

  const kinds: { id: 'all' | InsightKind; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'daily', label: 'Daily' },
    { id: 'weekly', label: 'Weekly' },
    { id: 'monthly', label: 'Monthly' },
    { id: 'adhoc', label: 'Ad-hoc' },
  ];

  const filtered = $derived(
    filter === 'all' ? reports : reports.filter((r) => r.kind === filter),
  );

  // ---------------------------------------------------------------------------
  // Load on mount
  // ---------------------------------------------------------------------------

  let loaded = false;
  $effect(() => {
    if (loaded) return;
    loaded = true;
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      reports = await insightsApi.listReports();
    } catch (e) {
      toasts.error('Could not load reports', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Run now
  // ---------------------------------------------------------------------------

  async function runNow(): Promise<void> {
    if (running) return;
    running = true;
    try {
      await insightsApi.run({ period: runPeriod });
      toasts.success('Insights run started', 'The report will appear shortly.');
      // Refresh after a short delay so a fast-completing report shows up.
      setTimeout(() => void load(), 2500);
    } catch (e) {
      toasts.error('Run failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Open / close a report's HTML in the iframe overlay
  // ---------------------------------------------------------------------------

  async function open(r: InsightReport): Promise<void> {
    openReport = r;
    openLoading = true;
    openUrl = null;
    try {
      openUrl = await insightsApi.reportUrl(r.html_path);
    } catch (e) {
      toasts.error('Could not open report', e instanceof Error ? e.message : String(e));
      close();
    } finally {
      openLoading = false;
    }
  }

  function close(): void {
    if (openUrl) URL.revokeObjectURL(openUrl);
    openUrl = null;
    openReport = null;
    openLoading = false;
  }

  // Revoke any live object URL on unmount.
  $effect(() => {
    return () => {
      if (openUrl) URL.revokeObjectURL(openUrl);
    };
  });

  // ---------------------------------------------------------------------------
  // Display helpers
  // ---------------------------------------------------------------------------

  function kindLabel(k: InsightKind): string {
    return k === 'adhoc' ? 'Ad-hoc' : k[0].toUpperCase() + k.slice(1);
  }

  function fmtDate(s: string): string {
    const d = new Date(s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleDateString();
  }

  function fmtDateTime(s: string): string {
    const d = new Date(s);
    return Number.isNaN(d.getTime()) ? s : d.toLocaleString();
  }

  function periodLabel(r: InsightReport): string {
    const start = fmtDate(r.period_start);
    const end = fmtDate(r.period_end);
    return start === end ? start : `${start} – ${end}`;
  }
</script>

<div class="page">
  <div class="page-header head-row">
    <div>
      <h1>Insights</h1>
      <div class="sub">
        Generated reports about your Otto activity. Scheduled reports are opt-in
        (<button class="link" onclick={() => router.go('settings/insights')}>Settings → Insights</button>);
        you can also run one on demand below.
      </div>
    </div>
    <div class="run-now">
      <select class="input run-period" bind:value={runPeriod} disabled={running} aria-label="Run period">
        <option value="day">Yesterday (day)</option>
        <option value="week">Last week</option>
        <option value="month">Last month</option>
      </select>
      <button class="btn primary" disabled={running} onclick={runNow}>
        <Icon name="play" size={13} />
        {running ? 'Starting…' : 'Run now'}
      </button>
    </div>
  </div>

  {#if loading && reports.length === 0}
    <Skeleton rows={3} height={72} />
  {:else if reports.length === 0}
    <EmptyState
      icon="gauge"
      title="No insight reports yet"
      body="Scheduled insights are opt-in and off by default. Turn on daily, weekly, or monthly reports in Settings → Insights, or run one now."
      actionLabel="Run now"
      onaction={runNow}
    >
      <button class="btn settings-link" onclick={() => router.go('settings/insights')}>
        <Icon name="gear" size={13} />
        Open Settings → Insights
      </button>
    </EmptyState>
  {:else}
    <!-- Kind filter -->
    <div class="filters">
      {#each kinds as k (k.id)}
        <button class="chip filter" class:active={filter === k.id} onclick={() => (filter = k.id)}>
          {k.label}
        </button>
      {/each}
    </div>

    {#if filtered.length === 0}
      <div class="card empty-filter dim">No {filter} reports yet.</div>
    {:else}
      <div class="report-list">
        {#each filtered as r (r.html_path + r.created_at)}
          <button class="report-card card" onclick={() => open(r)}>
            <div class="report-head">
              <span class="chip kind {r.kind}">{kindLabel(r.kind)}</span>
              <span class="report-period">{periodLabel(r)}</span>
              <span class="grow"></span>
              <span class="report-created dim">{fmtDateTime(r.created_at)}</span>
              <Icon name="external" size={13} />
            </div>
            {#if r.summary}
              <div class="report-summary">{r.summary}</div>
            {/if}
          </button>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<!-- Full report overlay -->
{#if openReport}
  <div class="overlay" role="dialog" aria-modal="true" aria-label="Insight report">
    <div class="overlay-bar">
      <span class="chip kind {openReport.kind}">{kindLabel(openReport.kind)}</span>
      <span class="overlay-title">{periodLabel(openReport)}</span>
      <span class="grow"></span>
      <button class="btn" onclick={close} aria-label="Close report">
        <Icon name="x" size={13} />
        Close
      </button>
    </div>
    <div class="overlay-body">
      {#if openLoading}
        <div class="overlay-loading dim">Loading report…</div>
      {:else if openUrl}
        <iframe class="report-frame" src={openUrl} title="Insight report"></iframe>
      {/if}
    </div>
  </div>
{/if}

<style>
  .head-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }

  .run-now {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .run-period {
    width: auto;
    height: 30px;
  }
  .btn :global(svg) {
    margin-right: 2px;
  }

  .link {
    border: none;
    background: none;
    padding: 0;
    font: inherit;
    color: var(--accent);
    cursor: pointer;
    text-decoration: underline;
  }
  .settings-link {
    margin-top: 4px;
  }

  /* Filters */
  .filters {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin: 4px 0 14px;
  }
  .chip.filter {
    cursor: pointer;
    background: transparent;
    transition: background 120ms ease-out, color 120ms ease-out, border-color 120ms ease-out;
  }
  .chip.filter:hover {
    background: var(--surface-2);
  }
  .chip.filter.active {
    border-color: var(--accent);
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }

  .empty-filter {
    padding: 18px;
    font-size: 12.5px;
  }

  /* Report list */
  .report-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    max-width: min(820px, 92vw);
  }
  .report-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 14px 16px;
    text-align: left;
    cursor: pointer;
    transition: border-color 120ms ease-out, background 120ms ease-out;
  }
  .report-card:hover {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 6%, transparent);
  }

  .report-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .report-period {
    font-size: 13px;
    font-weight: 600;
  }
  .report-created {
    font-size: 11px;
  }
  .report-summary {
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--text-dim);
  }

  .chip.kind {
    text-transform: none;
  }
  .chip.kind.daily {
    color: var(--accent);
    border-color: currentColor;
  }
  .chip.kind.weekly {
    color: var(--status-working, #2d8);
    border-color: currentColor;
  }
  .chip.kind.monthly {
    color: var(--status-info, #4a9eff);
    border-color: currentColor;
  }

  /* Overlay */
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .overlay-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-shrink: 0;
    padding: 10px 16px;
    border-bottom: 1px solid var(--border);
  }
  .overlay-title {
    font-size: 13px;
    font-weight: 600;
  }
  .overlay-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .overlay-loading {
    margin: auto;
    font-size: 13px;
  }
  .report-frame {
    flex: 1;
    width: 100%;
    height: 100%;
    border: none;
    background: #fff;
  }
</style>
