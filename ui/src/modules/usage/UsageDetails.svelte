<script lang="ts">
  // The Usage details card (patterns.md "Details card"): budgets, system load
  // and storage — the facts around the numbers, one tap from editing. System
  // load fetches its samples only while it is open; budgets and storage come
  // from what the page already loaded (no request of their own).
  import DetailsCard from '../../lib/components/DetailsCard.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { formatBytes, formatCount, formatUsd } from '../../lib/metric-format';
  import { pluralNoun } from '../../lib/plural';
  import { usage } from '../../lib/api/usage.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import { computeBudgetForecast } from './budgetForecast';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { usageUi } from './usageUiState.svelte';

  let { admin, floating = false }: { admin: boolean; floating?: boolean } = $props();

  // Opened rarely: each loads on first use.
  const BudgetsEditor = lazyComponent(() => import('./BudgetsEditor.svelte'));
  const StorageSettings = lazyComponent(() => import('./StorageSettings.svelte'));
  const SystemLoad = lazyComponent(() => import('./SystemLoad.svelte'));

  const wsName = (id: string): string => ws.workspaces.find((w) => w.id === id)?.name ?? (id === SCRATCH_WORKSPACE_ID ? (ws.scratch?.name ?? 'Scratch') : id);
  const budgets = $derived(usage.budgets);
  // Spend couldn't be read: rows carry 0, not a reading.
  const spendUnavailable = $derived(budgets?.spend_unavailable === true);
  const status = $derived(usage.status);
</script>

<DetailsCard label="Usage details" {floating} testid="usage-details">
  <div class="dc-group" aria-label="Budgets">
    <span class="dc-cap">Budgets</span>
    {#if budgets && budgets.rows.length > 0}
      {#if spendUnavailable}<p class="dc-note dc-warn">Spend unavailable: the usage engine couldn’t report spend for this window.</p>{/if}
      {#each budgets.rows as r (r.scope + ':' + r.key)}
        {@const name = r.scope === 'workspace' ? wsName(r.key) : (r.label ?? r.key)}
        {@const fc = spendUnavailable ? null : computeBudgetForecast(r, { windowDays: budgets.config.window_days, daily: usage.summary?.daily, dailyModels: usage.summary?.daily_models })}
        <div class="dc-row dc-static budget" class:warn={r.warning && !r.exceeded} class:over={r.exceeded}>
          <span class="dc-label" title="{r.scope === 'workspace' ? 'Workspace' : 'Provider'}: {name}">{name}</span>
          <span class="dc-end">{spendUnavailable ? '—' : formatUsd(r.spent_usd)} of {formatUsd(r.limit_usd)}{#if !spendUnavailable && (r.exceeded || r.warning)} · {r.exceeded ? 'over' : `${Math.round(r.used_fraction * 100)} %`}{/if}</span>
        </div>
        <div class="meter" role="meter" aria-label="{name} spend" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(Math.min(100, r.used_fraction * 100))}>
          <span class:warn={r.warning && !r.exceeded} class:over={r.exceeded} style="inline-size: {Math.min(100, r.used_fraction * 100)}%"></span>
        </div>
        {#if fc}<p class="dc-note fc-{fc.tone}" data-testid="budget-forecast">{fc.label}</p>{/if}
      {/each}
      {#if budgets.config.enforce}
        <p class="dc-note">{budgets.config.block_on_exceed ? 'Enforced: new work in an over-budget scope is refused.' : 'Enforced as a warning: over-budget scopes are flagged, not blocked.'}</p>
      {/if}
      {#if admin}<button class="dc-row" onclick={() => (usageUi.budgetsOpen = true)}><Icon name="edit" size={13} /><span class="dc-label">Edit budgets</span></button>{/if}
    {:else if usage.budgetsError}
      <LoadState what="budgets" variant="compact" error={usage.budgetsError} empty onretry={() => void usage.loadBudgets()} />
    {:else if !budgets}
      <LoadState what="budgets" variant="compact" loading empty rows={1} />
    {:else}
      <p class="dc-note">No spend caps yet.</p>
      {#if admin}<button class="dc-row" onclick={() => (usageUi.budgetsOpen = true)}><Icon name="plus" size={13} /><span class="dc-label">Edit budgets</span></button>{/if}
    {/if}
  </div>

  {#if admin && status}
    <div class="dc-group" aria-label="System load">
      <button class="dc-row" aria-expanded={usageUi.systemOpen} data-testid="usage-system-toggle" onclick={() => (usageUi.systemOpen = !usageUi.systemOpen)}>
        <Icon name={usageUi.systemOpen ? 'chevronDown' : 'chevronRight'} size={13} /><span class="dc-label">System load</span>
      </button>
      {#if usageUi.systemOpen}<LazyMount lazy={SystemLoad} what="system load" variant="compact" props={{ intervalSecs: status.metrics_interval_secs }} />{/if}
    </div>
    <div class="dc-group" aria-label="Storage">
      <button class="dc-row" onclick={() => (usageUi.configOpen = true)} title="Storage and retention settings">
        <Icon name="gear" size={13} /><span class="dc-label">Storage</span>
        <span class="dc-end">{formatBytes(status.disk_bytes)} · {formatCount(status.retention_days)} {pluralNoun(status.retention_days, 'day', 'days')}</span>
      </button>
      {#if status.priced_as_of}<p class="dc-note">Costs are estimates at published rates as of {status.priced_as_of}; unknown models are priced at the Opus tier and marked “Est.”.</p>{/if}
    </div>
  {:else if status?.priced_as_of}
    <div class="dc-group"><p class="dc-note">Costs are estimates at published rates as of {status.priced_as_of}.</p></div>
  {/if}
</DetailsCard>

{#if usageUi.budgetsOpen && admin}<LazyMount lazy={BudgetsEditor} what="the budgets editor" quiet onerror={() => (usageUi.budgetsOpen = false)} props={{ onclose: () => (usageUi.budgetsOpen = false) }} />{/if}
{#if usageUi.configOpen && admin && status}<LazyMount lazy={StorageSettings} what="storage settings" quiet onerror={() => (usageUi.configOpen = false)} props={{ status, onclose: () => (usageUi.configOpen = false) }} />{/if}

<style>
  .budget .dc-end {
    color: var(--text);
  }
  .budget.warn .dc-end {
    color: var(--warning);
  }
  .budget.over .dc-end {
    color: var(--danger);
  }
  .meter {
    block-size: 4px;
    margin-inline: var(--sp-6);
    margin-block-end: var(--sp-2);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .meter > span {
    display: block;
    block-size: 100%;
    background: var(--accent-solid);
  }
  .meter > span.warn {
    background: var(--warning);
  }
  .meter > span.over {
    background: var(--danger);
  }
  .fc-warn {
    color: var(--warning);
  }
  .fc-bad {
    color: var(--danger);
  }
</style>
