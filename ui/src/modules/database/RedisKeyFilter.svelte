<script lang="ts">
  // Inline prefix-filter for a Redis keyspace node. Typing a prefix + Enter
  // re-scans the keyspace with `MATCH <prefix>*` (bounded server-side) so huge
  // databases load a narrow, responsive set instead of every key.
  import Icon from '../../lib/components/Icon.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type { SchemaNode } from '../../lib/api/types';

  let { node, depth }: { node: SchemaNode; depth: number } = $props();

  // Local edit buffer for the filter input. Seeded from the node's current
  // filter and re-synced whenever the node (or its stored filter) changes, so a
  // reused component never shows a stale prefix.
  let draft = $state('');
  const active = $derived(database.nodeFilter(node.id));
  $effect(() => {
    draft = active;
  });

  function apply(): void {
    void database.applyNodeFilter(node, draft);
  }
  function clear(): void {
    draft = '';
    void database.applyNodeFilter(node, '');
  }
</script>

<div class="kf" style="padding-left: {(depth + 1) * 13 + 4}px">
  <Icon name="search" size={11} />
  <input
    class="kf-input"
    placeholder="filter by prefix…"
    bind:value={draft}
    onkeydown={(e) => {
      if (e.key === 'Enter') apply();
      else if (e.key === 'Escape') clear();
    }}
    onblur={apply}
  />
  {#if active}
    <button class="kf-clear" title="Clear filter" aria-label="Clear filter" onclick={clear}>
      <Icon name="x" size={10} />
    </button>
  {/if}
</div>

<style>
  .kf {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding-right: 6px;
    color: var(--text-dim);
  }
  .kf-input {
    flex: 1;
    min-width: 0;
    height: 19px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11px;
    padding: 0 6px;
    outline: none;
  }
  .kf-input:focus {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .kf-clear {
    display: grid;
    place-items: center;
    width: 17px;
    height: 17px;
    flex-shrink: 0;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .kf-clear:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--text-dim) 20%, transparent);
  }
</style>
