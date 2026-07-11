<script lang="ts">
  // Tags panel (left sidebar mode): every tag with its count; click → search.
  import { vault } from './vault.svelte';

  let filter = $state('');
  const shown = $derived(
    vault.tags.filter((t) => !filter || t.tag.toLowerCase().includes(filter.toLowerCase())),
  );
</script>

<div class="tags">
  <input bind:value={filter} placeholder="Filter tags…" />
  {#if vault.tags.length === 0}
    <div class="dim">No tags yet</div>
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
    background: var(--panel-2, #222);
    border: 1px solid var(--border);
    border-radius: 7px;
    color: var(--text);
    font-size: 12.5px;
    padding: 6px 10px;
    width: 100%;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
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
    border-radius: 6px;
    padding: 5px 8px;
    cursor: pointer;
  }
  .tag-row:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .tag {
    color: var(--accent, #9ab4ff);
    font-size: 12.5px;
  }
  .count {
    color: var(--text-dim);
    font-size: 11px;
  }
</style>
