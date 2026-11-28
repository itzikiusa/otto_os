<script lang="ts">
  // Saved Queries + Query History sidebar body extracted from DatabasePage.svelte.
  import { focusOnMount } from '../../lib/focusOnMount';
  import { rowMenu } from '../../lib/rowMenu';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import HistoryAll from './HistoryAll.svelte';
  import { stmtPreview } from '../../lib/database/sql-util';
  import { database } from '../../lib/stores/database.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { dbSidebarState } from './dbSidebarState.svelte';
  import type { DbSavedQuery } from '../../lib/api/types';

  interface Props {
    mode: 'saved' | 'history';
  }
  let { mode }: Props = $props();

  let renamingId = $state<string | null>(null);
  let renameDraft = $state('');

  // Memoize lowercased text per row so keystrokes don't re-lowercase whole scripts.
  const searchText = new WeakMap<object, { src: string; lower: string }>();
  function lowered(row: object, text: string): string {
    const hit = searchText.get(row);
    if (hit && hit.src === text) return hit.lower;
    const lower = text.toLowerCase();
    searchText.set(row, { src: text, lower });
    return lower;
  }

  const filteredSaved = $derived.by(() => {
    const s = dbSidebarState.savedSearch.trim().toLowerCase();
    if (!s) return database.savedQueries;
    return database.savedQueries.filter(
      (q) => q.name.toLowerCase().includes(s) || lowered(q, q.statement).includes(s),
    );
  });

  const filteredHistory = $derived.by(() => {
    const s = dbSidebarState.historySearch.trim().toLowerCase();
    if (!s) return database.history;
    return database.history.filter((h) =>
      lowered(h, h.error ? `${h.statement}\n${h.error}` : h.statement).includes(s),
    );
  });

  function startRename(q: DbSavedQuery): void {
    renamingId = q.id;
    renameDraft = q.name;
  }
  async function commitRename(): Promise<void> {
    const id = renamingId;
    if (id && renameDraft.trim()) await database.renameSavedQuery(id, renameDraft);
    renamingId = null;
  }
  function cancelRename(): void {
    renamingId = null;
  }
  function savedMenu(e: MouseEvent, q: DbSavedQuery): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'file', action: () => database.openSavedQuery(q) },
      { label: 'Rename', icon: 'edit', action: () => startRename(q) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteSaved(q) },
    ]);
  }
  async function deleteSaved(q: DbSavedQuery): Promise<void> {
    const ok = await confirmer.ask(
      `Delete saved query “${q.name}”? Open tabs keep their text but are no longer linked to it.`,
      { title: 'Delete saved query', confirmLabel: 'Delete query', danger: true },
    );
    if (ok) void database.deleteSavedQuery(q.id);
  }
</script>

{#if mode === 'saved'}
  <div class="list-search">
    <Icon name="search" size={12} />
    <input
      dir="ltr"
      class="list-search-input"
      placeholder="Filter saved queries…"
      bind:value={dbSidebarState.savedSearch}
      aria-label="Filter saved queries"
    />
    {#if dbSidebarState.savedSearch}
      <button class="icon-btn" onclick={() => (dbSidebarState.savedSearch = '')} aria-label="Clear filter" title="Clear filter">
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>
  <LoadState
    what="saved queries"
    variant="compact"
    loading={database.savedQueriesLoading}
    error={database.savedQueriesError}
    empty={database.savedQueries.length === 0}
    onretry={() => database.loadSavedQueries()}
  />
  {#if database.savedQueries.length === 0 && (database.savedQueriesLoading || database.savedQueriesError)}
    <!-- Loading and failed loads are rendered above, never as an empty list. -->
  {:else if database.savedQueries.length === 0}
    <EmptyState icon="file" title="No saved queries" body="Save one from the Query tab." />
  {:else if filteredSaved.length === 0}
    <EmptyState
      icon="search"
      title="No matches"
      body={`No saved queries match “${dbSidebarState.savedSearch}”.`}
      actionLabel="Clear filter"
      actionKind="secondary"
      onaction={() => (dbSidebarState.savedSearch = '')}
    />
  {:else}
    {#each filteredSaved as q (q.id)}
      <div class="side-row saved-row" role="presentation" oncontextmenu={(e) => { e.preventDefault(); savedMenu(e, q); }}>
        {#if renamingId === q.id}
          <input
            dir="auto"
            class="rename-input"
            bind:value={renameDraft}
            use:focusOnMount
            onkeydown={(e) => {
              if (e.key === 'Enter') void commitRename();
              else if (e.key === 'Escape') cancelRename();
            }}
            onblur={() => void commitRename()}
            aria-label="Rename saved query"
          />
        {:else}
          <button class="side-row-main saved-open" data-side-row onclick={() => database.openSavedQuery(q)} title={stmtPreview(q.statement, 1000)} use:rowMenu>
            <Icon name="file" size={14} />
            <span class="side-row-title ellipsis" dir="auto">{q.name}</span>
          </button>
          <button
            class="icon-btn side-row-more row-del"
            onclick={(e) => savedMenu(e, q)}
            aria-haspopup="menu"
            aria-label="Actions for saved query {q.name}"
            title="Rename or delete"
          >
            <Icon name="more" size={12} />
          </button>
        {/if}
      </div>
    {/each}
  {/if}
{:else}
  <div class="list-search">
    <Icon name="search" size={12} />
    <input
      dir="ltr"
      class="list-search-input"
      placeholder="Filter history…"
      bind:value={dbSidebarState.historySearch}
      aria-label="Filter query history"
    />
    {#if dbSidebarState.historySearch}
      <button class="icon-btn" onclick={() => (dbSidebarState.historySearch = '')} aria-label="Clear filter" title="Clear filter">
        <Icon name="x" size={12} />
      </button>
    {/if}
    <select
      class="hist-scope"
      bind:value={dbSidebarState.historyAll}
      aria-label="Whose history"
      title="This connection’s history, or every connection’s"
    >
      <option value={false}>This connection</option>
      <option value={true}>All connections</option>
    </select>
  </div>
  {#if dbSidebarState.historyAll}
    <HistoryAll search={dbSidebarState.historySearch} />
  {:else}
    <LoadState
      what="query history"
      variant="compact"
      loading={database.historyLoading}
      error={database.historyError}
      empty={database.history.length === 0}
      onretry={() => database.loadHistory()}
    />
    {#if database.history.length === 0 && (database.historyLoading || database.historyError)}
      <!-- Loading and failed loads are rendered above, never as an empty list. -->
    {:else if database.history.length === 0}
      <EmptyState icon="clock" title="No query history yet" body="Queries you run show up here." />
    {:else if filteredHistory.length === 0}
      <EmptyState
        icon="search"
        title="No matches"
        body={`No history matches “${dbSidebarState.historySearch}”.`}
        actionLabel="Clear filter"
        actionKind="secondary"
        onaction={() => (dbSidebarState.historySearch = '')}
      />
    {:else}
      {#each filteredHistory as h (h.id)}
        <div class="side-row">
          <button
            class="side-row-main hist-row"
            class:bad={!h.ok}
            data-side-row
            onclick={() => void database.openHistory(h)}
            title={h.error ? stmtPreview(h.error, 1000) : stmtPreview(h.statement, 1000)}
          >
            <span class="hist-dot" class:ok={h.ok}></span>
            <span class="side-row-title hist-stmt ellipsis mono">{stmtPreview(h.statement)}</span>
            <span class="side-row-trailing hist-meta">{h.ok ? `${h.row_count}r` : 'err'} · {rel(h.created_at, { compact: true })}</span>
          </button>
        </div>
      {/each}
      {#if database.canLoadMoreHistory}
        <button class="load-more" onclick={() => database.loadMoreHistory()} disabled={database.historyLoadingMore}>
          {database.historyLoadingMore ? 'Loading more history…' : 'Load more'}
        </button>
      {/if}
    {/if}
  {/if}
{/if}

<style>
  .list-search {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px 6px;
    color: var(--text-dim);
  }
  .list-search-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    font-size: var(--fs-s);
  }
  .list-search-input:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .hist-scope {
    max-width: 45%;
    font-size: var(--fs-xs);
  }
  .rename-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--accent);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    padding: 4px 6px;
    margin: 0 2px;
    font-size: var(--fs-s);
  }
  .rename-input:focus {
    outline: none;
  }
  .load-more {
    width: 100%;
    border: none;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    font-size: var(--fs-s);
    padding: 8px 6px;
    text-align: center;
  }
  .load-more:hover:not(:disabled) {
    text-decoration: underline;
  }
  .load-more:disabled {
    color: var(--text-dim);
    cursor: default;
  }
  /* Rows: the shared .side-row look (app.css); the ⋯ is the row's .side-row-more. */
  .hist-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--status-exited);
    flex-shrink: 0;
  }
  .hist-dot.ok {
    background: var(--status-working);
  }
  .hist-stmt {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .hist-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
