<script lang="ts">
  // Tags panel (left sidebar mode): every tag with its count; click → search.
  import { vault } from './vault.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  let filter = $state('');
  const shown = $derived(
    vault.tags.filter((t) => !filter || t.tag.toLowerCase().includes(filter.toLowerCase())),
  );
</script>

<div class="tags">
  <input type="search" bind:value={filter} placeholder="Filter tags…" aria-label="Filter tags" />
  {#if vault.tags.length === 0 && vault.loading}
    <Skeleton rows={4} height={24} label="tags" />
  {:else if vault.tags.length === 0}
    <div class="dim">No tags yet. Add #tags in a note, or tags: in its frontmatter.</div>
  {:else if shown.length === 0}
    <div class="dim">No tags match “{filter}”.</div>
  {/if}
  <div class="list">
    {#each shown as t (t.tag)}
      <button class="tag-row" onclick={() => vault.searchTag(t.tag)}>
        <span class="tag">#{t.tag}</span>
        <span class="count">{t.count}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .tags {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-height: 0;
    flex: 1;
    padding: 8px;
  }
  input {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 6px 10px;
    width: 100%;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    padding: 4px 2px;
  }
  .list {
    overflow-y: auto;
    min-height: 0;
  }
  .tag-row {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    padding: 5px 8px;
    cursor: pointer;
  }
  .tag-row:hover {
    background: var(--hover);
  }
  .tag {
    color: var(--accent-text);
    font-size: var(--fs-s);
  }
  .count {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
