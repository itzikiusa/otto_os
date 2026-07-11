<script lang="ts">
  // Full-text search panel (left sidebar mode). FTS5 with `tag:` / `path:` /
  // `type:` operators; snippets come highlighted with ‹› markers from bm25.
  import { vault } from './vault.svelte';

  let input = $state<HTMLInputElement | undefined>();

  $effect(() => {
    if (vault.leftMode === 'search') input?.focus();
  });

  function renderSnippet(s: string): string {
    // Server marks matches with ‹ › — escape everything else.
    const esc = s
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    return esc.replace(/‹/g, '<mark>').replace(/›/g, '</mark>');
  }
</script>

<div class="search">
  <input
    bind:this={input}
    bind:value={vault.searchQuery}
    placeholder="Search… (tag:x path:y type:z)"
    onkeydown={(e) => e.key === 'Enter' && void vault.runSearch()}
  />
  {#if vault.searching}
    <div class="dim">Searching…</div>
  {:else if vault.searchQuery && vault.searchHits.length === 0}
    <div class="dim">No results</div>
  {/if}
  <div class="hits">
    {#each vault.searchHits as h (h.path)}
      <button class="hit" class:reserved={h.reserved} onclick={() => void vault.open(h.path)}>
        <div class="t">{h.title}</div>
        <div class="p">{h.path}</div>
        {#if h.snippet}
          <!-- Escaped above; only <mark> tags are injected. -->
          <div class="s">{@html renderSnippet(h.snippet)}</div>
        {/if}
      </button>
    {/each}
  </div>
</div>

<style>
  .search {
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
  .hits {
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .hit {
    text-align: start;
    background: none;
    border: none;
    border-radius: 7px;
    padding: 6px 8px;
    cursor: pointer;
    color: var(--text);
  }
  .hit:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .hit.reserved {
    opacity: 0.7;
  }
  .t {
    font-size: 12.5px;
    font-weight: 600;
  }
  .p {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .s {
    font-size: 11.5px;
    color: var(--text-dim);
    margin-top: 2px;
  }
  .s :global(mark) {
    background: var(--accent-dim, rgba(90, 120, 255, 0.3));
    color: var(--text);
    border-radius: 3px;
    padding: 0 1px;
  }
</style>
