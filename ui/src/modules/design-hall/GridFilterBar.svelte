<script lang="ts">
  // The filter row over "All designs": Group by (Recent · Story · Project)
  // and the chips — studio ▾, status ▾, Mine, Archived. Every chip is a real
  // button whose state is in its text or aria-pressed, never colour alone.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { DesignStatus } from '../../lib/api/types';
  import { STUDIOS, statusLabel, studioInfo } from './model';
  import { NO_FILTERS, activeFilterCount, type GridFilters, type GroupBy } from './designGrid';

  interface Props {
    filters: GridFilters;
    groupBy: GroupBy;
  }
  let { filters = $bindable(), groupBy = $bindable() }: Props = $props();

  const GROUPS: { id: GroupBy; label: string }[] = [
    { id: 'recent', label: 'Recent' },
    { id: 'story', label: 'Story' },
    { id: 'project', label: 'Project' },
  ];
  const STATUSES: Exclude<DesignStatus, 'archived'>[] = ['draft', 'review', 'approved', 'shipped'];

  function studioMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'All studios', checked: !filters.studio, action: () => (filters = { ...filters, studio: '' }) },
      { separator: true },
      ...STUDIOS.map((s) => ({ label: s.name, checked: filters.studio === s.id, action: () => (filters = { ...filters, studio: s.id }) })),
    ]);
  }
  function statusMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Any status', checked: !filters.status, action: () => (filters = { ...filters, status: '' }) },
      { separator: true },
      ...STATUSES.map((s) => ({ label: statusLabel(s), checked: filters.status === s, action: () => (filters = { ...filters, status: s }) })),
    ]);
  }
</script>

<div class="bar" data-testid="design-grid-filters">
  <div class="grp">
    <span class="lbl" aria-hidden="true">Group by</span>
    <div class="segmented" role="group" aria-label="Group by">
      {#each GROUPS as g (g.id)}
        <button aria-pressed={groupBy === g.id} class:active={groupBy === g.id} onclick={() => (groupBy = g.id)}
          data-testid={`design-group-${g.id}`}>{g.label}</button>
      {/each}
    </div>
  </div>
  <div class="chips" role="group" aria-label="Filter designs">
    <button class="pill-toggle" class:on={!!filters.studio} onclick={studioMenu} aria-haspopup="menu" data-testid="design-filter-studio">
      {filters.studio ? studioInfo(filters.studio).name : 'Studio'} <Icon name="chevronDown" size={12} />
    </button>
    <button class="pill-toggle" class:on={!!filters.status} onclick={statusMenu} aria-haspopup="menu" disabled={filters.archived}
      title={filters.archived ? 'Archived designs have no other status' : undefined} data-testid="design-filter-status">
      {filters.status ? statusLabel(filters.status) : 'Status'} <Icon name="chevronDown" size={12} />
    </button>
    <button class="pill-toggle" class:on={filters.mine} aria-pressed={filters.mine} onclick={() => (filters = { ...filters, mine: !filters.mine })}
      title="Designs you created" data-testid="design-filter-mine">
      {#if filters.mine}<Icon name="check" size={12} />{/if} Mine
    </button>
    <button class="pill-toggle" class:on={filters.archived} aria-pressed={filters.archived}
      onclick={() => (filters = { ...filters, archived: !filters.archived, status: '' })}
      title="Show only archived designs, to find or restore one" data-testid="design-filter-archived">
      {#if filters.archived}<Icon name="check" size={12} />{/if} Archived
    </button>
    {#if activeFilterCount(filters)}
      <button class="linkbtn" onclick={() => (filters = { ...NO_FILTERS })} data-testid="design-filter-clear">Clear</button>
    {/if}
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 12px;
    margin-block-end: 14px;
    min-width: 0;
  }
  .grp {
    display: inline-flex;
    align-items: center;
  }
  .lbl {
    font-size: var(--fs-s);
    color: var(--text-dim);
    padding-inline: 4px 6px;
  }
  .chips {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pill-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0 4px;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
  @media (pointer: coarse), (max-width: 640px) {
    .pill-toggle,
    .segmented button {
      min-block-size: var(--hit-min);
    }
  }
</style>
