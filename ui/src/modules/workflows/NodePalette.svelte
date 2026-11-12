<script lang="ts">
  // The canvas "Add node" popover: a filter box over the node catalog, grouped
  // by category, each entry with its description — so a node can be found by
  // what it does, not only by its name. The page owns opening, closing (Esc /
  // outside click on `.wf-pop`) and where it sits.
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import type { NodeTypeSpec } from '../../lib/api/types';
  import { groupNodeTypes } from './nodePalette';

  interface Props {
    types: NodeTypeSpec[];
    /** Inline-end offset inside the editor pane, under the opening button. */
    right: number;
    onpick: (t: NodeTypeSpec) => void;
  }
  let { types, right, onpick }: Props = $props();

  let query = $state('');
  const groups = $derived(groupNodeTypes(types, query));

  /** Enter in the filter adds the first match. */
  function onKey(e: KeyboardEvent): void {
    const first = groups[0]?.types[0];
    if (e.key === 'Enter' && first) {
      e.preventDefault();
      onpick(first);
    }
  }
</script>

<div class="np wf-pop" role="dialog" aria-label="Add a node" style:inset-inline-end="{right}px">
  <input
    class="pal-filter"
    type="search"
    dir="auto"
    placeholder="Filter nodes…"
    aria-label="Filter nodes"
    bind:value={query}
    onkeydown={onKey}
  />
  <div class="np-list">
    {#each groups as g (g.category)}
      <div class="np-cat">{g.category}</div>
      {#each g.types as t (t.kind)}
        <button class="pal-item" title={t.description} onclick={() => onpick(t)}>
          <span class="pal-ic"><Icon name={asIcon(t.icon, 'box')} size={12} /></span>
          <span class="pal-body">
            <span class="pal-name">{t.label}</span>
            {#if t.description}<span class="pal-desc">{t.description}</span>{/if}
          </span>
        </button>
      {/each}
    {:else}
      <p class="np-empty">{types.length ? `No node matches “${query.trim()}”.` : 'Loading nodes…'}</p>
    {/each}
  </div>
</div>

<style>
  .np {
    position: absolute;
    top: 6px;
    z-index: var(--z-sticky);
    width: 280px;
    max-width: calc(100% - 16px);
    max-height: min(420px, calc(100% - 12px)); /* the pane, not the window */
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-l);
    box-shadow: var(--glass-shadow);
    padding: 4px;
  }
  .pal-filter {
    font: inherit;
    font-size: var(--fs-s);
    margin: 2px 2px 4px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .np-list {
    overflow-y: auto;
    min-height: 0;
  }
  .np-cat {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    padding: 6px 8px 2px;
  }
  .pal-item {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .pal-item:hover,
  .pal-item:focus-visible {
    background: var(--accent-soft);
  }
  .pal-ic {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .pal-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .pal-name {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .pal-desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .np-empty {
    margin: 6px 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
