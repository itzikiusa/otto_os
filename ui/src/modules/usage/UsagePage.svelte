<script lang="ts">
  // Usage: how much is left on each plan and what it cost (spec E). The page
  // is the frame — header (Refresh, the one Export, ⋯), the scope row (Otto /
  // All, window) and the states (no access, engine not set up, loading) — and
  // three views: the overview (UsageOverview), the ccusage-style Report and,
  // for root, Diagnostics (Otto's own telemetry). Report and Diagnostics are
  // lazy chunks reached from ⋯ and the overview's Diagnostics link. A non-root
  // user holding Usage:View sees their own sessions (`summary.scope === 'own'`).
  // Opening the page asks for status, summary, budgets and limits only; the
  // forecast, attribution and system metrics load when their view opens.
  import { onMount } from 'svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { announceShown, focusAppContent } from '../../lib/announce.svelte';
  import PathField from '../../lib/components/PathField.svelte';
  import { router } from '../../lib/router.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import Icon from '../../lib/components/Icon.svelte';
  import { registry } from '../../lib/commands.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { formatUsd } from '../../lib/metric-format';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { usage } from '../../lib/api/usage.svelte';
  import { budgetBus } from '../../lib/events.svelte';
  import UsageHealthBanner from './UsageHealthBanner.svelte';
  import UsageOverview from './UsageOverview.svelte';
  import { usageUi, type UsageSubView } from './usageUiState.svelte';

  /** Root sees everything and owns the engine/budget controls; anyone else
   *  with Usage:View gets a read-only view of their own sessions. */
  const admin = $derived(auth.isRoot);
  const canView = $derived(auth.isRoot || auth.can('usage', 'view'));
  $effect(() => {
    if (router.module !== 'usage') return;
    const sub = router.parts[1];
    usageUi.view = sub === 'report' ? 'report' : sub === 'otto' && admin ? 'otto' : 'overview';
  });
  const view = $derived<UsageSubView>(usageUi.view === 'otto' && !admin ? 'overview' : usageUi.view);
  let ottoUsageChunk: Promise<typeof import('./OttoUsage.svelte')> | null = null;
  const ottoUsage = () => (ottoUsageChunk ??= import('./OttoUsage.svelte'));
  let ottoLoad = $state(0);
  let reportLoad = $state(0);
  async function selectView(next: UsageSubView): Promise<void> {
    if (next === view) return;
    if (usageUi.telemetryDirty && !await confirmer.ask('You have unsaved telemetry settings. Leaving now discards them.', { title: 'Discard unsaved changes?', confirmLabel: 'Discard', cancelLabel: 'Keep editing', danger: true })) return;
    if (usageUi.telemetryDirty) usageUi.discardTelemetry();
    usageUi.view = next;
    if (router.module === 'usage') router.go(next === 'overview' ? 'usage' : `usage/${next}`);
  }
  $effect(() => guardUnsaved(() => usageUi.telemetryDirty, { what: 'your telemetry settings', allow: (to) => to === 'usage' || to.startsWith('usage/') }));

  const WINDOWS = [
    { days: 7, label: '7d' },
    { days: 30, label: '30d' },
    { days: 90, label: '90d' },
    { days: 180, label: '180d' },
  ];

  // The not-installed state's "use a binary you already have".
  let chPath = $state('');

  // Live budget-exceeded banner — the BudgetExceeded WS event via budgetBus;
  // dismissible, a "recovered" direction says so. Null: no active alert.
  type BudgetAlert = { provider: string; spendUsd: number; capUsd: number; direction: string };
  let budgetAlert: BudgetAlert | null = $state(null);
  let budgetAlertTick = $state(0);
  $effect(() => {
    const tick = budgetBus.tick;
    if (tick === 0 || tick === budgetAlertTick) return;
    budgetAlertTick = tick;
    budgetAlert = { provider: budgetBus.provider, spendUsd: budgetBus.spendUsd, capUsd: budgetBus.capUsd, direction: budgetBus.direction };
  });
  function dismissBudgetAlert(): void {
    budgetAlert = null;
    void focusAppContent();
  }

  let overview = $state<UsageOverview | undefined>();
  /** The one Export: a CSV of what "Where it went" shows. */
  function exportShown(): void {
    const file = overview?.exportShown();
    if (file) toasts.success('Exported as CSV', file);
    else toasts.info('Nothing to export in this view yet');
  }
  function exportSummary(): void {
    usage.exportSummaryJson();
    toasts.success('Exported the usage summary as JSON', `otto-usage-summary-${usage.days}d.json`);
  }
  function exportDaily(): void {
    usage.exportDailyCsv();
    toasts.success('Exported daily totals as CSV', `otto-usage-daily-${usage.days}d.csv`);
  }

  // ⋯: everything that is not the first screen, still one visible click away.
  const moreItems = (): MenuItem[] => [
    { label: 'Report…', icon: 'list', action: () => void selectView('report') },
    ...(admin ? [{ label: 'Diagnostics', icon: 'gauge', action: () => void selectView('otto') }] : []),
    { separator: true },
    { label: 'Export daily totals (CSV)', icon: 'download', disabled: !usage.summary, action: exportDaily },
    { label: 'Export everything (JSON)', icon: 'download', disabled: !usage.summary, action: exportSummary },
    { separator: true },
    { label: 'Refresh every minute', checked: usage.autoRefresh, action: () => usage.setAutoRefresh(!usage.autoRefresh) },
    ...(admin ? [{ label: 'Budgets…', icon: 'edit', disabled: !usage.budgets, action: () => (usageUi.budgetsOpen = true) }, { label: 'Storage and retention…', icon: 'gear', disabled: !usage.status, action: () => (usageUi.configOpen = true) }] : []),
  ];

  // ⌘K verbs while Usage is open.
  $effect(() =>
    registry.register('usage', [
      { id: 'usage.refresh', title: 'Refresh usage', group: 'Usage', keywords: 'reload tokens cost', run: () => void usage.loadAll() },
      { id: 'usage.view-overview', title: 'Show usage overview', group: 'Usage', keywords: 'dashboard charts', run: () => void selectView('overview') },
      { id: 'usage.view-report', title: 'Show usage report tables', group: 'Usage', keywords: 'ccusage daily monthly table', run: () => void selectView('report') },
      ...WINDOWS.map((w) => ({ id: `usage.window-${w.days}`, title: `Show the last ${w.days} days`, group: 'Usage', keywords: `usage window range ${w.label}`, run: () => usage.setDays(w.days) })),
      { id: 'usage.export-summary', title: 'Export usage summary (JSON)', group: 'Usage', keywords: 'download save json report', run: exportSummary },
      ...(admin ? [{ id: 'usage.view-otto', title: 'Show Otto diagnostics: performance and profiling', group: 'Usage', keywords: 'telemetry cpu ram slow traces', run: () => void selectView('otto') }, { id: 'usage.settings', title: 'Open usage storage and retention', group: 'Usage', keywords: 'clickhouse retention interval settings', run: () => (usageUi.configOpen = true) }] : []),
    ]),
  );

  onMount(() => {
    usage.admin = admin;
    if (canView) void usage.loadAll();
    return () => {
      if (usageUi.telemetryDirty) usageUi.discardTelemetry();
      usage.setAutoRefresh(false);
    };
  });
  // Keep the store's root flag in step (it decides whether /usage/metrics,
  // a root-only endpoint, is requested at all).
  $effect(() => {
    usage.admin = admin;
  });
  $effect(() => {
    if (usage.status && !chPath) chPath = usage.status.binary ?? '';
  });

  const crumbs = $derived(view === 'overview' ? [] : [{ label: 'Usage', onclick: () => void selectView('overview') }]);
  const title = $derived(view === 'report' ? 'Report' : view === 'otto' ? 'Diagnostics' : 'Usage');
</script>

<div class="usage">
  <PageHeader {title} {crumbs} subtitle={view === 'overview' ? (admin ? 'What is left on your plans and what it cost' : 'What your sessions used and cost') : undefined} more={view === 'otto' ? undefined : moreItems}>
    {#snippet actions()}
      {#if usage.status?.available && view !== 'otto'}
        <button class="icon-btn" onclick={() => usage.loadAll()} disabled={usage.loading} title={usage.loading ? 'Refreshing…' : 'Refresh'} aria-label="Refresh" data-icon="refresh" data-label="Refresh">
          <Icon name="refresh" size={14} />
        </button>
        {#if view === 'overview'}
          <button class="btn small" disabled={!usage.summary} onclick={exportShown} title="Save what “Where it went” shows as CSV" data-icon="download" data-label="Export" data-testid="usage-export" data-keep>
            <Icon name="download" size={12} /> Export
          </button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody>
    {#if view === 'otto' && admin}
      {#key ottoLoad}
        {#await ottoUsage()}<LoadState what="Otto diagnostics" loading empty />{:then module}<module.default />{:catch}
          <LoadState what="Otto diagnostics" error="Otto may have been updated in the background. Retry, or reload Otto to pick up the new version." empty onretry={() => { ottoUsageChunk = null; ottoLoad++; }}>
            {#snippet errorActions()}<button class="btn" onclick={() => location.reload()}>Reload Otto</button><button class="btn" onclick={() => void selectView('overview')}>Back to usage</button>{/snippet}
          </LoadState>
        {/await}
      {/key}
    {:else}
    <!-- What the numbers cover: who and how far back (one row). -->
    {#if usage.status?.available}
      <div class="usage-bar" role="toolbar" aria-label="Usage scope">
        {#if admin}
          <div class="segmented" role="group" aria-label="Sessions to count" title="Otto: only sessions run inside Otto. All: every Claude/Codex session on this Mac.">
            <button aria-pressed={usage.ottoOnly} onclick={() => usage.setOttoOnly(true)}>Otto</button>
            <button aria-pressed={!usage.ottoOnly} onclick={() => usage.setOttoOnly(false)}>All</button>
          </div>
        {/if}
        <div class="segmented" role="group" aria-label="Time window">
          {#each WINDOWS as w (w.days)}
            <button aria-pressed={usage.days === w.days} title="Last {w.days} days" onclick={() => usage.setDays(w.days)}>{w.label}</button>
          {/each}
        </div>
        {#if usage.autoRefresh}
          <button class="chip-live" onclick={() => usage.setAutoRefresh(false)} title="Refreshing every minute — click to stop"><Icon name="clock" size={12} /> Live</button>
        {/if}
        {#if usage.summary?.scope === 'own'}<span class="scope-note"><Icon name="info" size={12} /> Your sessions only</span>{/if}
      </div>
    {/if}

    <!-- Ingestion health: tailer retrying/stopped, database down, journal filling. -->
    <UsageHealthBanner status={usage.status} />

    {#if budgetAlert}
      <div class="budget-banner" class:recovered={budgetAlert.direction === 'recovered'} use:announceShown={budgetAlert.direction === 'recovered' ? `Back under budget: ${budgetAlert.provider || 'workspace'}` : { text: `Over budget: ${budgetAlert.provider || 'workspace'} spent ${formatUsd(budgetAlert.spendUsd)} of its ${formatUsd(budgetAlert.capUsd)} cap.`, assertive: true }}>
        <Icon name={budgetAlert.direction === 'recovered' ? 'check' : 'warning'} size={14} />
        <span>
          {#if budgetAlert.direction === 'recovered'}
            Back under budget: <strong>{budgetAlert.provider || 'workspace'}</strong> spend ({formatUsd(budgetAlert.spendUsd)}) is below its {formatUsd(budgetAlert.capUsd)} cap.
          {:else}
            Over budget: <strong>{budgetAlert.provider || 'workspace'}</strong> spent {formatUsd(budgetAlert.spendUsd)} of its {formatUsd(budgetAlert.capUsd)} cap.
          {/if}
        </span>
        <button class="icon-btn" onclick={dismissBudgetAlert} title="Dismiss" aria-label="Dismiss budget alert"><Icon name="x" size={14} /></button>
      </div>
    {/if}

    {#if !canView}
      <EmptyState variant="page" icon="chart" title="You don’t have access to usage" body="Token and cost history is visible to people granted Usage. Ask whoever set up this Otto for access." />
    {:else if view === 'report' && usage.status?.available}
      {#key reportLoad}
        {#await import('./UsageReportView.svelte')}
          <LoadState what="Usage report" loading empty />
        {:then module}
          <module.default {admin} />
        {:catch error}
          <LoadState what="Usage report" error={loadErrorText(error)} empty onretry={() => reportLoad++} />
        {/await}
      {/key}
    {:else}
      <!-- Remaining shows whatever the engine's state: limits come from the CLIs'
           own files, not from ClickHouse. -->
      <UsageOverview bind:this={overview} {admin} ondiagnostics={() => void selectView('otto')} gate={usage.status?.available ? undefined : gate} />
    {/if}
    {/if}
  </PageBody>
</div>

{#snippet gate()}
      {#if !usage.status}
        <!-- Status unknown (loading, or /usage/status failed): never fall through to
             the "Install ClickHouse" prompt — a failed load is not "not installed". -->
        <LoadState what="usage" variant="page" rows={6} loading={usage.loading || !usage.statusError} error={usage.statusError} empty onretry={() => void usage.loadAll()} />
      {:else if !usage.status.available && !admin}
        <EmptyState variant="page" icon="chart" title="Usage tracking isn’t set up" body="Otto records token usage once the root account sets up usage tracking." />
      {:else if !usage.status.available}
        <!-- Engine not installed: the page's one CTA installs it; pointing at an
             existing binary is the quiet secondary path. -->
        <EmptyState
          variant="page"
          icon="chart"
          title="Set up usage tracking"
          body="Otto keeps token, cost and system-load history in an embedded ClickHouse engine (clickhouse local — no server, no port). Install it once and Otto manages it from then on."
          actionLabel={usage.installing ? 'Installing…' : 'Install ClickHouse'}
          actionIcon="download"
          onaction={() => { if (!usage.installing) void usage.install(); }}
        >
          <div class="install-alt">
            <label for="ch-path">Or use a ClickHouse binary you already have</label>
            <div class="path-input">
              <PathField bind:value={chPath} files><input dir="ltr" id="ch-path" class="input mono" placeholder="/usr/local/bin/clickhouse" bind:value={chPath} spellcheck="false" /></PathField>
              <button class="btn" disabled={usage.saving || chPath.trim() === ''} title={chPath.trim() === '' ? 'Enter the path to a clickhouse binary first' : 'Use this binary'} onclick={() => usage.saveConfig({ enabled: true, clickhouse_path: chPath.trim() })}>
                {usage.saving ? 'Saving…' : 'Use binary'}
              </button>
            </div>
            {#if usage.status?.binary}<span class="dim small">Detected: <span class="mono">{usage.status.binary}</span></span>{/if}
          </div>
        </EmptyState>
      {/if}
{/snippet}

<style>
  .usage {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  /* Scope controls (who / how far back) above the page's content. */
  .usage-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: var(--sp-4);
    margin-block-end: var(--sp-6);
  }
  .chip-live {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    block-size: 26px;
    padding-inline: var(--sp-4);
    border: 1px solid var(--accent);
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .scope-note {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  @media (pointer: coarse) {
    .usage-bar .segmented > button,
    .chip-live {
      min-block-size: 44px;
    }
  }
  /* Live WS budget banner — top of the page body. */
  .budget-banner {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    padding-block: var(--sp-3);
    padding-inline: var(--sp-5) var(--sp-4);
    margin: 0 auto var(--sp-6);
    max-inline-size: var(--read-col);
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .budget-banner > :global(svg:first-child) {
    color: var(--danger);
    flex-shrink: 0;
  }
  .budget-banner.recovered {
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 40%, transparent);
  }
  .budget-banner.recovered > :global(svg:first-child) {
    color: var(--success);
  }
  .budget-banner span {
    flex: 1;
    min-width: 0;
  }
  /* Not-installed page: the secondary "use an existing binary" path under the
     EmptyState's one CTA. */
  .install-alt {
    width: min(460px, 100%);
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    text-align: start;
    margin-top: var(--sp-5);
    padding-top: var(--sp-6);
    border-top: 1px solid var(--separator);
  }
  .install-alt label {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .path-input {
    display: flex;
    gap: var(--sp-4);
  }
  .path-input > :global(:first-child) {
    flex: 1;
    min-width: 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
</style>
