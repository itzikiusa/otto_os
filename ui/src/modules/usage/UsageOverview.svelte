<script lang="ts">
  // The Usage page's first screen (spec E): what is left on each plan, then
  // what it cost in words — "This month ≈ $854 · on track for $1,120" — ONE
  // chart (cost by default) and ONE "Where it went" table with a switch. The
  // details card (budgets, system load, storage) floats beside the reading
  // column when there is room and follows it otherwise. Otto's own telemetry
  // is one visible click away at the bottom ("Diagnostics").
  import type { Snippet } from 'svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { formatCount, formatUsd } from '../../lib/metric-format';
  import { pluralNoun } from '../../lib/plural';
  import { usage } from '../../lib/api/usage.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import DailyChart from './DailyChart.svelte';
  import RemainingLimits from './RemainingLimits.svelte';
  import UsageDetails from './UsageDetails.svelte';
  import WhereItWent from './WhereItWent.svelte';
  import { breakdownTitle, summaryParts } from './tokenParts';
  import { budgetHeadsUp, monthOutlook } from './usageOutlook';

  /** `gate`: what shows instead of the numbers while the usage engine is
   *  loading, failed or not set up (the page's states) — Remaining stays. */
  let { admin, ondiagnostics, gate }: { admin: boolean; ondiagnostics: () => void; gate?: Snippet } = $props();

  let where = $state<WhereItWent | undefined>();
  /** The page's one Export: what "Where it went" shows. */
  export function exportShown(): string | null {
    return where?.exportShown() ?? null;
  }

  const summary = $derived(usage.summary);
  const hasUsage = $derived(Boolean(summary && (summary.total_tokens > 0 || summary.total_events > 0 || summary.sessions.length > 0)));
  const outlook = $derived(summary ? monthOutlook(summary.daily, summary.days, new Date()) : null);
  const wsName = (id: string): string => ws.workspaces.find((w) => w.id === id)?.name ?? (id === SCRATCH_WORKSPACE_ID ? (ws.scratch?.name ?? 'Scratch') : id);
  const headsUp = $derived(usage.budgets && !usage.budgets.spend_unavailable ? budgetHeadsUp(usage.budgets.rows, (r) => (r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key))) : []);
  const paceText = $derived(summary && summary.days > 0 ? formatUsd(summary.total_cost_usd / summary.days) : '');
</script>

<div class="ov-wrap">
<div class="ov">
  <div class="ov-main">
    <RemainingLimits />

    {#if gate}
      {@render gate()}
    {:else if usage.summaryError}
      <LoadState what="usage summary" loading={usage.loading} error={usage.summaryError} empty={!summary} onretry={() => void usage.loadAll()} />
    {:else if !summary && usage.loading}
      <LoadState what="usage summary" loading empty rows={4} />
    {/if}

    {#if gate}
      <!-- the gate says it -->
    {:else if summary && !hasUsage}
      <EmptyState icon="chart" title="No usage in this window" body="Choose a longer window or All sessions to include activity outside Otto. New agent activity will appear here." />
    {:else if summary}
      <section class="spend read-prose" aria-label="Spend" data-testid="usage-spend">
        {#if outlook}
          <p class="headline">This month ≈ <strong>{formatUsd(outlook.monthToDate)}</strong> · on track for <strong>{formatUsd(outlook.projected)}</strong></p>
        {:else}
          <p class="headline">Last {summary.days} days ≈ <strong>{formatUsd(summary.total_cost_usd)}</strong> · about {paceText} a day</p>
        {/if}
        <p class="sub">
          Last {summary.days} days ≈ {formatUsd(summary.total_cost_usd)} ·
          <span title={breakdownTitle(summaryParts(summary))}>{formatCount(summary.total_tokens)} tokens</span> ·
          {formatCount(summary.sessions.length)} {pluralNoun(summary.sessions.length, 'session', 'sessions')} ·
          {formatCount(summary.providers.length)} {pluralNoun(summary.providers.length, 'provider', 'providers')}
        </p>
        {#each headsUp as h (h.text)}
          <p class="heads-up" class:over={h.over}><Icon name="warning" size={14} /> {h.text}</p>
        {/each}
      </section>

      <DailyChart daily={summary.daily} days={summary.days} />
      <WhereItWent bind:this={where} {summary} />
    {/if}

    {#if admin}
      <button class="diag" onclick={ondiagnostics} data-testid="usage-diagnostics">
        <Icon name="gauge" size={14} /> Diagnostics — Otto’s own speed, errors and profiling <Icon name="chevronRight" size={12} />
      </button>
    {/if}
  </div>
  {#if !gate}
    <aside class="ov-side" aria-label="Usage details">
      <UsageDetails {admin} floating />
    </aside>
  {/if}
</div>
</div>

<style>
  /* The reading column (760 px) with the details card beside it when the pane
     has room for both; below that the card follows the column. */
  .ov-wrap {
    container-type: inline-size;
  }
  .ov {
    display: grid;
    grid-template-columns: minmax(0, var(--read-col));
    justify-content: center;
    gap: var(--sp-8);
    padding-block-end: var(--sp-9);
  }
  .ov-main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-8);
    min-inline-size: 0;
  }
  .ov-side {
    min-inline-size: 0;
  }
  @container (min-width: 1100px) {
    .ov {
      grid-template-columns: minmax(0, var(--read-col)) 300px;
    }
    .ov-side {
      position: sticky;
      inset-block-start: 0;
      align-self: start;
    }
  }
  .spend {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .spend p {
    margin: 0;
  }
  .headline {
    font-size: var(--fs-xl);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .headline strong {
    font-weight: 600;
  }
  .sub {
    font-size: var(--fs-m);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .heads-up {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    color: var(--warning);
    font-weight: 600;
  }
  .heads-up.over {
    color: var(--danger);
  }
  .diag {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    align-self: flex-start;
    min-block-size: 32px;
    padding-inline: var(--sp-4);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .diag:hover {
    background: var(--hover);
    color: var(--text);
  }
  @media (pointer: coarse) {
    .diag {
      min-block-size: 44px;
    }
  }
</style>
