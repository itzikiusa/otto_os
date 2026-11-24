<script lang="ts" generics="M extends { id: string }">
  // The ONE "More" row under the sidebar's shown modules (lib/sidebarMore.ts):
  // it expands the rest in place, and the open state is remembered per device
  // (stores/navUsage). The host renders each row (`row`), so a module looks
  // the same here as above. `rail`: the collapsed icon rail's version.
  import type { Snippet } from 'svelte';
  import Icon from '../lib/components/Icon.svelte';
  import { navUsage } from '../lib/stores/navUsage.svelte';

  let { modules, row, rail = false }: { modules: M[]; row: Snippet<[M]>; rail?: boolean } = $props();
  const open = $derived(navUsage.moreOpen);
  const label = $derived(open ? 'Show fewer modules' : `Show ${modules.length} more modules`);
</script>

{#if modules.length > 0}
  <div class="sm" class:rail data-testid="sidebar-more">
    <button
      type="button"
      class="sm-toggle"
      aria-expanded={open}
      aria-label={label}
      title={label}
      data-testid="sidebar-more-toggle"
      onclick={() => navUsage.toggleMore()}
    >
      {#if rail}
        <Icon name={open ? 'chevronUp' : 'more'} />
      {:else}
        <Icon name="more" size={14} />
        <span class="sm-label">More</span>
        <span class="sm-count">{modules.length}</span>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
      {/if}
    </button>
    {#if open}
      {#each modules as m (m.id)}
        {@render row(m)}
      {/each}
    {/if}
  </div>
{/if}

<style>
  .sm {
    display: flex;
    flex-direction: column;
    margin-block-start: var(--sp-5);
  }
  .sm.rail {
    align-items: center;
    margin-block-start: var(--sp-2);
  }
  .sm-toggle {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    inline-size: 100%;
    block-size: 28px;
    padding-inline: var(--sp-4);
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .sm-toggle:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text);
  }
  .sm.rail .sm-toggle {
    justify-content: center;
    inline-size: 32px;
    block-size: 32px;
    padding: 0;
  }
  .sm-label {
    flex: 1;
  }
  .sm-count {
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  @media (pointer: coarse) {
    .sm-toggle {
      min-block-size: var(--hit-min);
    }
  }
</style>
