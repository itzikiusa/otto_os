<script lang="ts">
  // The Threads sidebar's folded groups for threads the person did not start
  // here (threadOrigins.ts): "Agent runs" (a personal agent's routine runs)
  // and the helpers other modules embed ("Pull requests", "Connections", …).
  // Folded by default with a count; open, the newest 20, then "Show more".
  import Icon from '../../lib/components/Icon.svelte';
  import type { Session } from '../../lib/api/types';
  import ThreadSidebarRow from './ThreadSidebarRow.svelte';
  import type { OriginGroup } from './threadOrigins';

  interface Props {
    groups: OriginGroup[];
    onrowclick: (e: MouseEvent, s: Session) => void;
    onreopen: (s: Session, beside: boolean) => void;
    onmenu: (e: MouseEvent, s: Session) => void;
    onselect?: () => void;
  }
  let { groups, onrowclick, onreopen, onmenu, onselect }: Props = $props();

  const PAGE = 20;
  let open = $state<Record<string, boolean>>({});
  let limit = $state<Record<string, number>>({});
</script>

{#each groups as g (g.key)}
  {@const expanded = open[g.key] === true}
  {@const max = limit[g.key] ?? PAGE}
  <section class="og" aria-label={g.label} data-testid="threads-group-{g.key}">
    <button type="button" class="og-toggle" aria-expanded={expanded} onclick={() => (open = { ...open, [g.key]: !expanded })}>
      <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} />
      <span class="ellipsis">{g.label}</span>
      <span class="og-count" aria-label="{g.items.length} threads">{g.items.length}</span>
    </button>
    {#if expanded}
      <div class="og-list">
        {#each g.items.slice(0, max) as s (s.id)}
          <ThreadSidebarRow session={s} variant={s.archived ? 'history' : 'active'} {onrowclick} {onreopen} {onmenu} {onselect} />
        {/each}
        {#if g.items.length > max}
          <button type="button" class="btn small ghost og-more" onclick={() => (limit = { ...limit, [g.key]: max + PAGE })}>
            Show more ({g.items.length - max})
          </button>
        {/if}
      </div>
    {/if}
  </section>
{/each}

<style>
  .og {
    display: flex;
    flex-direction: column;
  }
  .og-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    font-weight: 500;
    text-align: start;
    cursor: pointer;
  }
  .og-toggle:hover {
    background: var(--hover);
  }
  .og-toggle :global(svg) {
    flex-shrink: 0;
  }
  .og-count {
    margin-inline-start: auto;
    padding-inline: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .og-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .og-more {
    margin: 6px 8px;
  }
</style>
