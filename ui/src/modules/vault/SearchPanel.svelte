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
    type="search"
    placeholder="Search… (tag:x path:y type:z)"
    aria-label="Search this vault"
    title="Full-text search. Narrow with tag:, path: or type:. Press Enter to search."
    onkeydown={(e) => e.key === 'Enter' && void vault.runSearch()}
  />
  {#if vault.searching}
    <div class="dim" role="status">Searching…</div>
  {:else if vault.searchError}
    <div class="err" role="alert">
      <span>Search failed: {vault.searchError}</span>
      <button class="btn small" onclick={() => void vault.runSearch()}>Retry</button>
    </div>
  {:else if vault.searchQuery.trim() && vault.searchQuery.trim() !== vault.searchedQuery}
    <!-- Search runs on Enter: "No results" before it ran was a lie. -->
    <div class="dim">Press Enter to search</div>
  {:else if vault.searchQuery.trim() && vault.searchHits.length === 0}
    <div class="dim">No notes match “{vault.searchQuery.trim()}”.</div>
  {/if}
  <div class="hits">
    {#each vault.searchHits as h (h.path)}
      <button class="hit" class:reserved={h.reserved} onclick={() => void vault.open(h.path)}>
        <div class="t">{h.title}</div>
        <div class="p" title={h.path}>{h.path}</div>
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
    border-radius: var(--radius-s);
    padding: 6px 8px;
    cursor: pointer;
    color: var(--text);
  }
  .hit:hover {
    background: var(--hover);
  }
  .hit.reserved .t {
    color: var(--text-dim);
    font-style: italic;
  }
  .err {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--text);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .t {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .p {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .s {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-top: 2px;
  }
  .s :global(mark) {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--text);
    border-radius: 3px;
    padding: 0 1px;
  }
</style>
