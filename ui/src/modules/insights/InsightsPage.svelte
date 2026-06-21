<script lang="ts">
  // Insights view — two tabs:
  //   • Reports: generated HTML insight reports (filterable by kind)
  //   • Health:  Capability & Health Registry (B3, GET /capabilities)
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
  import { downloadText } from '../../lib/components/exporters';
  import CapabilitiesPage from './CapabilitiesPage.svelte';

  // Tab: 'reports' (default) or 'health' (sub-route `#/insights/health`).
  const tab = $derived(router.parts[1] === 'health' ? 'health' : 'reports');

  let reports: InsightReport[] = $state([]);
  let loading = $state(true);
  let running = $state(false);
  /** Reason the last run did not start (e.g. skill not installed). */
  let runFailReason: string | null = $state(null);
  /** Whether we are polling for the in-flight run to complete. */
  let pollRunId: string | null = $state(null);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  /** Maximum times to poll before giving up (avoid infinite loops). */
  const POLL_MAX = 20;
  let pollCount = $state(0);

  // Filter: 'all' | kind
  let filter: 'all' | InsightKind = $state('all');

  // Run-now period + offset picker. Offset = 0 → most recent complete period;
  // offset = 1 → the one before that, etc. (mirrors --offset in the skill CLI).
  let runPeriod: InsightRunPeriod = $state('day');
  let runOffset: number = $state(1);

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
    return () => {
      if (pollTimer) clearTimeout(pollTimer);
    };
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
  // Run now — with real run_id polling
  // ---------------------------------------------------------------------------

  async function runNow(): Promise<void> {
    if (running) return;
    running = true;
    runFailReason = null;
    pollRunId = null;
    pollCount = 0;
    try {
      const resp = await insightsApi.run({ period: runPeriod, offset: runOffset });
      if (!resp.started) {
        // skill not installed or no workspace available
        runFailReason = resp.reason ?? 'Could not start a run — check that the insights skill is installed.';
        return;
      }
      // We have a run_id: poll /insights/reports until the new report appears
      // (the backend writes it when the session finishes). Fall back to the
      // old blind 2.5 s wait if run_id was not returned (shouldn't happen).
      if (resp.run_id) {
        pollRunId = resp.run_id;
        schedulePoll();
      } else {
        setTimeout(() => void load(), 2500);
        toasts.success('Insights run started', 'The report will appear shortly.');
      }
    } catch (e) {
      toasts.error('Run failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

  /** Poll /insights/reports every 3 s to detect when the new report lands. */
  function schedulePoll(): void {
    if (pollCount >= POLL_MAX || !pollRunId) {
      pollRunId = null;
      // Reload one final time in case we hit the cap.
      void load();
      return;
    }
    pollTimer = setTimeout(async () => {
      pollCount += 1;
      const prev = reports.length;
      await load();
      if (reports.length > prev) {
        // A new report appeared — done.
        pollRunId = null;
        toasts.success('Insights report ready', 'Open it from the list below.');
      } else {
        schedulePoll();
      }
    }, 3_000);
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

  /** Download the currently-open report HTML as a file. */
  async function downloadReport(r: InsightReport): Promise<void> {
    try {
      const url = await insightsApi.reportUrl(r.html_path);
      // Fetch the blob behind the object URL so we can re-download it.
      const res = await fetch(url);
      const text = await res.text();
      URL.revokeObjectURL(url);
      const filename = r.html_path.split('/').at(-1) ?? `insight-${r.kind}-${r.period_start}.html`;
      downloadText(text, filename, 'text/html');
    } catch (e) {
      toasts.error('Download failed', e instanceof Error ? e.message : String(e));
    }
  }

  /** Open the report in a new browser tab (Tauri webview). */
  async function openInTab(r: InsightReport): Promise<void> {
    try {
      const url = await insightsApi.reportUrl(r.html_path);
      window.open(url, '_blank', 'noopener');
      // Don't revoke — the new tab needs the URL. It'll be GC'd on close.
    } catch (e) {
      toasts.error('Could not open tab', e instanceof Error ? e.message : String(e));
    }
  }

  // Revoke any live object URL on unmount.
  $effect(() => {
    return () => {
      if (openUrl) URL.revokeObjectURL(openUrl);
      if (pollTimer) clearTimeout(pollTimer);
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
  <!-- Tab switcher: Reports | Health -->
  <div class="tab-bar">
    <button class="tab" class:active={tab === 'reports'} onclick={() => router.go('insights')}>
      <Icon name="gauge" size={13} />
      Reports
    </button>
    <button class="tab" class:active={tab === 'health'} onclick={() => router.go('insights/health')}>
      <Icon name="check" size={13} />
      Health
    </button>
  </div>

  {#if tab === 'health'}
    <CapabilitiesPage />
  {:else}
  <!-- ---- Reports tab (original content follows) ---- -->
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
      <select
        class="input run-offset"
        bind:value={runOffset}
        disabled={running}
        aria-label="How many periods back"
        title="0 = most recent complete period; 1 = the one before that, etc."
      >
        <option value={1}>Previous</option>
        <option value={2}>2 periods ago</option>
        <option value={3}>3 periods ago</option>
        <option value={4}>4 periods ago</option>
      </select>
      <button class="btn primary" disabled={running || !!pollRunId} onclick={runNow}>
        <Icon name="play" size={13} />
        {running ? 'Starting…' : pollRunId ? 'Running…' : 'Run now'}
      </button>
    </div>
  </div>

  {#if runFailReason}
    <!-- Skill not installed or no workspace available -->
    <div class="skill-missing card">
      <Icon name="alert" size={20} />
      <div class="skill-missing-body">
        <strong>Insights skill not available</strong>
        <p>{runFailReason}</p>
        <button class="btn" onclick={() => router.go('settings/skills')}>
          <Icon name="gear" size={13} />
          Open Settings → Skills
        </button>
      </div>
    </div>
  {/if}

  {#if pollRunId}
    <div class="run-progress dim">
      <Icon name="refresh" size={13} />
      Running insights… checking every 3 s (attempt {pollCount}/{20})
    </div>
  {/if}

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
  {/if}<!-- end tab === reports -->
</div>

<!-- Full report overlay (outside tab guard — not rendered in health tab) -->
{#if openReport}
  {@const rep = openReport}
  <div class="overlay" role="dialog" aria-modal="true" aria-label="Insight report">
    <div class="overlay-bar">
      <span class="chip kind {rep.kind}">{kindLabel(rep.kind)}</span>
      <span class="overlay-title">{periodLabel(rep)}</span>
      <span class="grow"></span>
      <button class="btn" onclick={() => openInTab(rep)} aria-label="Open in new tab" title="Open in new tab">
        <Icon name="external" size={13} />
        New tab
      </button>
      <button class="btn" onclick={() => downloadReport(rep)} aria-label="Download report" title="Download HTML">
        <Icon name="download" size={13} />
        Download
      </button>
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
  /* Tab bar (Reports / Health) */
  .tab-bar {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border, #e2e8f0);
    margin-bottom: 20px;
    padding: 0 0 0 0;
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 8px 14px;
    border: none;
    background: transparent;
    font: inherit;
    font-size: 13px;
    color: var(--text-muted);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    transition: color 120ms ease-out, border-color 120ms ease-out;
  }
  .tab:hover { color: var(--text); }
  .tab.active { color: var(--accent); border-bottom-color: var(--accent); font-weight: 500; }

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
  .run-period,
  .run-offset {
    width: auto;
    height: 30px;
  }
  .btn :global(svg) {
    margin-inline-end: 2px;
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
    text-align: start;
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

  /* Skill-not-installed empty state */
  .skill-missing {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 16px 18px;
    margin: 10px 0;
    border-inline-start: 3px solid var(--warn, #d08a18);
    background: color-mix(in srgb, var(--warn, #d08a18) 10%, transparent);
    border-radius: var(--radius-m, 8px);
    color: var(--text);
    font-size: 13px;
  }
  .skill-missing-body {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .skill-missing-body p {
    margin: 0;
    color: var(--text-dim);
    font-size: 12px;
  }

  /* Run-in-progress poll indicator */
  .run-progress {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    padding: 6px 0;
  }
</style>
