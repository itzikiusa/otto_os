<script lang="ts">
  // "Where it went": ONE table with a switch — Provider · Model · Feature ·
  // Session · Work item — biggest cost first. Each view renders (and fetches)
  // only while it is open: the next-run forecast under Feature and the
  // work-item attribution are requests of their own, never made on page open.
  // `exportShown()` is the page's one Export: a CSV of exactly this view.
  import { usage, type UsageSummary } from '../../lib/api/usage.svelte';
  import { exportCsv } from '../../lib/components/exporters';
  import { formatCount } from '../../lib/metric-format';
  import { pluralNoun } from '../../lib/plural';
  import { auth } from '../../lib/stores/auth.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import CostForecastChip from './CostForecastChip.svelte';
  import CostRow from './CostRow.svelte';
  import { unmeasuredTerminalNote, usageProviderLabel } from './providerCoverage';
  import { breakdownTitle, featureLabel } from './tokenParts';
  import { DIM_LABELS, type AttributionDim, type AttributionRow } from './types';
  import { costRows } from './usageOutlook';
  import { usageUi, type WhereView } from './usageUiState.svelte';

  let { summary }: { summary: UsageSummary } = $props();

  // Views other than Provider and Feature load on first open (first paint).
  const ModelBreakdown = lazyComponent(() => import('./ModelBreakdown.svelte'));
  const SessionsTable = lazyComponent(() => import('./SessionsTable.svelte'));
  const AttributionDrilldown = lazyComponent(() => import('./AttributionDrilldown.svelte'));

  const VIEWS: { id: WhereView; label: string }[] = [
    { id: 'provider', label: 'Provider' },
    { id: 'model', label: 'Model' },
    { id: 'feature', label: 'Feature' },
    { id: 'session', label: 'Session' },
    { id: 'work', label: 'Work item' },
  ];

  const providerRows = $derived(costRows(summary.providers, (p) => p.provider, (p) => usageProviderLabel(p.provider)));
  const featureRows = $derived(costRows(summary.by_kind, (f) => f.feature, (f) => featureLabel(f.feature)));
  const installed = $derived.by(() => {
    const set = new Set<string>(summary.providers.map((p) => p.provider));
    for (const t of auth.meta?.providers ?? []) set.add(t);
    return [...set].filter(Boolean);
  });

  let attrRows = $state<AttributionRow[]>([]);
  let attrDim = $state<AttributionDim>('origin');

  /** What the one Export saves, or null when the view shows nothing. Returns
   *  the file name it wrote. */
  export function exportShown(): string | null {
    const d = usage.days;
    switch (usageUi.where) {
      case 'provider':
        if (!summary.providers.length) return null;
        usage.exportProvidersCsv();
        return `otto-usage-providers-${d}d.csv`;
      case 'model':
        if (!summary.models?.length) return null;
        usage.exportModelsCsv();
        return `otto-usage-models-${d}d.csv`;
      case 'feature':
        if (!summary.by_kind.length) return null;
        usage.exportFeaturesCsv();
        return `otto-usage-features-${d}d.csv`;
      case 'session':
        if (!summary.sessions.length) return null;
        usage.exportSessionsCsv();
        return `otto-usage-sessions-${d}d.csv`;
      case 'work': {
        if (!attrRows.length) return null;
        const file = `otto-attribution-${attrDim}-${d}d.csv`;
        exportCsv(attrRows.map((r) => ({ [DIM_LABELS[attrDim]]: r.key, cost_usd: r.cost_usd, tokens: r.tokens, sessions: r.sessions })), file);
        return file;
      }
    }
  }
</script>

<section class="where" aria-labelledby="usage-where-title" data-testid="usage-where">
  <div class="where-head">
    <h3 id="usage-where-title">Where it went</h3>
    <div class="segmented switch" role="group" aria-label="Where it went by">
      {#each VIEWS as v (v.id)}
        <button aria-pressed={usageUi.where === v.id} data-testid="usage-where-{v.id}" onclick={() => (usageUi.where = v.id)}>{v.label}</button>
      {/each}
    </div>
  </div>

  {#if usageUi.where === 'provider'}
    {#each providerRows as r (r.key)}
      <CostRow label={r.label} cost={r.item.cost_usd} share={r.share} title={breakdownTitle(r.item)} sub="{formatCount(r.item.total_tokens)} tokens · {formatCount(r.item.events)} {pluralNoun(r.item.events, 'event', 'events')}" />
    {:else}
      <p class="dim small">No usage in this window. Providers appear here as agents run.</p>
    {/each}
    {#if unmeasuredTerminalNote(installed)}<p class="dim small note" data-testid="usage-unmeasured-note">{unmeasuredTerminalNote(installed)}</p>{/if}
  {:else if usageUi.where === 'model'}
    <LazyMount lazy={ModelBreakdown} what="the models" variant="compact" props={{ models: summary.models ?? [], dailyModels: summary.daily_models ?? [], days: summary.days }} />
  {:else if usageUi.where === 'feature'}
    {#each featureRows as r (r.key)}
      <CostRow label={r.label} cost={r.item.cost_usd} share={r.share} title={breakdownTitle(r.item)} sub="{formatCount(r.item.sessions)} {pluralNoun(r.item.sessions, 'session', 'sessions')} · {formatCount(r.item.total_tokens)} tokens" />
    {:else}
      <p class="dim small">No usage in this window. Cost splits by feature (reviews, product, channels, agents…) appear here as work runs.</p>
    {/each}
    {#if summary.providers.length > 0}
      <div class="note"><CostForecastChip feature="agent" provider={summary.providers[0].provider} lead="Next agent run on {usageProviderLabel(summary.providers[0].provider)} ≈" /></div>
    {/if}
  {:else if usageUi.where === 'session'}
    <LazyMount lazy={SessionsTable} what="the sessions" variant="compact" props={{ sessions: summary.sessions }} />
  {:else}
    <LazyMount lazy={AttributionDrilldown} what="the work items" variant="compact" props={{ days: summary.days, onrows: (rows: AttributionRow[], dim: AttributionDim) => { attrRows = rows; attrDim = dim; } }} />
  {/if}
</section>

<style>
  .where {
    display: flex;
    flex-direction: column;
    min-inline-size: 0;
  }
  .where-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--sp-4) var(--sp-5);
    margin-block-end: var(--sp-4);
  }
  h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .switch {
    flex-wrap: wrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .note {
    margin-block-start: var(--sp-4);
  }
  @media (pointer: coarse) {
    .switch > button {
      min-block-size: 44px;
    }
  }
</style>
