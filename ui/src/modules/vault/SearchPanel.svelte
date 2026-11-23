<script lang="ts">
  // Full-text search panel (left sidebar mode). FTS5 with `tag:` / `path:` /
  // `type:` operators; snippets come highlighted with ‹› markers from bm25.
  import { vault } from './vault.svelte';
  import { escapeHtml } from '../../lib/html';
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { searchIndexNotice } from './searchIndex';
  import {
    AUDIT_PREDICATES,
    PREFIX_OPERATORS,
    activePredicates,
    clearPredicates,
    hasPredicate,
    insertOperatorPrefix,
    togglePredicate,
  } from './searchPredicates';

  let input = $state<HTMLInputElement | undefined>();
  // While the full-text index rebuilds, results come from the indexed part:
  // say so, and search again once it completes so the list is whole.
  const partial = $derived(vault.status?.search_index ?? null);
  let wasPartial = false;
  $effect(() => {
    const now = !!partial;
    if (wasPartial && !now && vault.searchedQuery && !vault.searching) void vault.runSearch();
    wasPartial = now;
  });

  $effect(() => {
    if (vault.leftMode === 'search') input?.focus();
  });

  const activeCount = $derived(activePredicates(vault.searchQuery).length);

  function onTogglePredicate(token: string, exclusivePrefix?: string): void {
    vault.searchQuery = togglePredicate(vault.searchQuery, token, exclusivePrefix);
    void vault.runSearch();
  }

  function onInsertPrefix(prefix: string): void {
    vault.searchQuery = insertOperatorPrefix(vault.searchQuery, prefix);
    input?.focus();
    const len = vault.searchQuery.length;
    input?.setSelectionRange?.(len, len);
  }

  function onClearPredicates(): void {
    vault.searchQuery = clearPredicates(vault.searchQuery);
    void vault.runSearch();
  }

  function renderSnippet(s: string): string {
    // Server marks matches with ‹ › — escape everything else. Only a
    // balanced ‹…› pair (no nested ‹) becomes a <mark>: a stray literal ‹ or
    // › in the note stays text instead of opening/closing tags (S18-26).
    return escapeHtml(s).replace(/‹([^‹›]*)›/g, '<mark>$1</mark>');
  }
</script>

<div class="search">
  <input dir="auto"
    bind:this={input}
    bind:value={vault.searchQuery}
    type="search"
    placeholder="Search… (status: stale:true orphan:true tag:)"
    aria-label="Search this vault"
    title="Full-text search. Operators: status:, type:, tag:, path:, stale:true, orphan:true, links-to:. Press Enter to search."
    oninput={() => {
      if (!vault.searchQuery.trim()) void vault.runSearch();
    }}
    onkeydown={(e) => e.key === 'Enter' && void vault.runSearch()}
  />
  <div class="predicates" role="group" aria-label="Search filters" data-testid="vault-search-predicates">
    {#each AUDIT_PREDICATES as p (p.id)}
      {@const on = hasPredicate(vault.searchQuery, p.token)}
      <button
        type="button"
        class="pred-chip"
        class:on
        aria-pressed={on}
        title={p.title}
        data-testid="vault-pred-{p.id}"
        onclick={() => onTogglePredicate(p.token, p.exclusivePrefix)}
      >{p.label}</button>
    {/each}
    {#each PREFIX_OPERATORS as op (op.id)}
      <button
        type="button"
        class="pred-chip op"
        title={op.title}
        onclick={() => onInsertPrefix(op.prefix)}
      >{op.label}</button>
    {/each}
    {#if activeCount > 0}
      <button
        type="button"
        class="pred-chip clear"
        title="Clear structured filters"
        onclick={onClearPredicates}
      >Clear</button>
    {/if}
  </div>
  {#if vault.searching}
    <div class="dim">Searching…</div>
  {:else if vault.searchError}
    <LoadState variant="compact" what="search results" error={vault.searchError} empty onretry={() => void vault.runSearch()} />
  {:else if vault.searchQuery.trim() && vault.searchQuery.trim() !== vault.searchedQuery}
    <!-- Search runs on Enter: "No results" before it ran was a lie. -->
    <div class="dim">Press Enter to search</div>
  {:else if vault.searchQuery.trim() && vault.searchHits.length === 0}
    <div class="dim">No notes match “{vault.searchQuery.trim()}”.</div>
  {/if}
  {#if partial && vault.searchedQuery && !vault.searchError}
    <div class="partial" use:announceShown={searchIndexNotice(partial)}><Icon name="info" size={12} /> {searchIndexNotice(partial)}</div>
  {/if}
  {#if !vault.searching && !vault.searchError && vault.searchedQuery && vault.searchQuery.trim() === vault.searchedQuery}
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
      {#if vault.searchHits.length >= 50}
        <div class="dim cap-note">Showing the first 50 matches. Narrow with status:, type:, tag:, path:, stale:true, orphan:true, or links-to:.</div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .partial {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin-block: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-xs);
    line-height: 1.4;
  }
  .partial :global(svg) {
    flex: none;
    margin-block-start: 2px;
    color: var(--info);
  }
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
  .predicates {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .pred-chip {
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 8px;
    cursor: pointer;
    line-height: 1.4;
  }
  .pred-chip:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .pred-chip.on {
    background: var(--accent-soft);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 500;
  }
  .pred-chip.op {
    font-family: var(--font-mono);
  }
  .pred-chip.clear {
    background: transparent;
    border-style: dashed;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    padding: 4px 2px;
  }
  .cap-note {
    font-size: var(--fs-xs);
    padding: 6px 8px;
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
    min-height: var(--hit-min);
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
    background: var(--accent-line);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 0 1px;
  }
</style>
