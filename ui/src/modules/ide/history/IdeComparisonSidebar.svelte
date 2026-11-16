<script lang="ts">
  // Resizable left timeline / revision / branch picker sidebar for `IdeComparisonModal`.
  import type { CommitInfo, RefBranch } from '../../../lib/api/types';
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { BadgeTone } from '../../../lib/status';
  import type { IdeCompareMode, IdeLocalHistoryEntry } from './ideComparisonLoader';

  interface Props {
    mode: IdeCompareMode;
    isDir: boolean;
    /** The desktop list width; 0 = the stacked (phone) list step, which fills the sheet. */
    widthPx: number;
    entries: IdeLocalHistoryEntry[];
    filteredEntries: IdeLocalHistoryEntry[];
    deletedCount: number;
    filterMode: 'all' | 'deleted';
    searchInput: string;
    selectedEntryId: string | null;
    commits: CommitInfo[];
    branches: RefBranch[];
    selectedRef: string;
    refFilter: string;
    loading: boolean;
    error: string | null;
    onFilterMode: (next: 'all' | 'deleted') => void;
    onSearchInput: (next: string) => void;
    onSelectEntry: (id: string) => void;
    onPutLabel: () => void;
    onRefFilter: (next: string) => void;
    onSelectRef: (ref: string) => void;
    onRetry: () => void;
  }

  let {
    mode,
    isDir,
    widthPx,
    entries,
    filteredEntries,
    deletedCount,
    filterMode,
    searchInput,
    selectedEntryId,
    commits,
    branches,
    selectedRef,
    refFilter,
    loading,
    error,
    onFilterMode,
    onSearchInput,
    onSelectEntry,
    onPutLabel,
    onRefFilter,
    onSelectRef,
    onRetry,
  }: Props = $props();

  function kindTone(kind: string): BadgeTone {
    if (kind === 'deleted') return 'bad';
    if (kind === 'created') return 'ok';
    if (kind === 'restored' || kind === 'label') return 'accent';
    if (kind === 'renamed') return 'warn';
    return 'info';
  }

  function kindLabel(kind: string): string {
    if (kind === 'deleted') return 'Deleted';
    if (kind === 'created') return 'Created';
    if (kind === 'restored') return 'Restored';
    if (kind === 'renamed') return 'Renamed';
    if (kind === 'external') return 'External';
    return 'Change';
  }

  function formatTime(ms: number): string {
    const d = new Date(ms);
    if (Number.isNaN(d.getTime())) return '';
    return d.toLocaleString(undefined, {
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    });
  }
</script>

<aside class="ics-left" class:fill={widthPx <= 0} style:inline-size={widthPx > 0 ? `${widthPx}px` : undefined} data-testid="ide-local-history-modal">
  {#if mode === 'local-history'}
    <div class="ics-controls">
      <div class="ics-top-row">
        <div class="segmented ics-seg" role="group" aria-label="Local history filter">
          <button
            type="button"
            class:active={filterMode === 'all'}
            aria-pressed={filterMode === 'all'}
            onclick={() => onFilterMode('all')}
            data-testid="ide-local-history-filter-all"
          ><span>All</span>{#if entries.length > 0}<Badge tone="neutral" variant="outline" label={String(entries.length)} />{/if}</button>
          <button
            type="button"
            class:active={filterMode === 'deleted'}
            aria-pressed={filterMode === 'deleted'}
            onclick={() => onFilterMode('deleted')}
            data-testid="ide-local-history-filter-deleted"
          ><span>Deleted</span>{#if deletedCount > 0}<Badge tone="bad" label={String(deletedCount)} />{/if}</button>
        </div>
        <button
          type="button"
          class="btn small ghost ics-label-btn"
          title="Save a named Local History label"
          onclick={onPutLabel}
          data-testid="ide-local-history-put-label"
        ><Icon name="tag" size={12} /><span>Label…</span></button>
      </div>
      <div class="input-group ics-search">
        <Icon name="search" size={12} />
        <input
          type="search"
          dir="ltr"
          aria-label="Search Local History by content or file"
          placeholder="Search by content…"
          value={searchInput}
          oninput={(e) => onSearchInput((e.currentTarget as HTMLInputElement).value)}
          data-testid="ide-local-history-search"
        />
      </div>
    </div>

    <div class="ics-list">
      <LoadState what="local history" {loading} {error} empty={filteredEntries.length === 0} onretry={onRetry} variant="compact">
        {#snippet emptyView()}
          <EmptyState
            icon="clock"
            title={searchInput.trim() ? 'No matching snapshots' : filterMode === 'deleted' ? 'No deleted files recorded' : 'No local snapshots yet'}
            body={searchInput.trim() ? 'No revision content matched your search query.' : 'Snapshots are saved automatically on edits, saves, renames, and deletions.'}
            headingLevel={3}
          />
        {/snippet}
        {#each filteredEntries as item (item.id)}
          <button
            type="button"
            class="ics-row"
            class:active={selectedEntryId === item.id}
            onclick={() => onSelectEntry(item.id)}
            data-testid="ide-local-history-row"
          >
            <div class="ics-row-top">
              <Badge tone={kindTone(item.event_kind)} label={item.label || kindLabel(item.event_kind)} />
              {#if isDir}<span class="ics-path mono ellipsis" dir="ltr">{item.rel_path}</span>{/if}
            </div>
            <div class="ics-row-meta">
              <span class="mono">{formatTime(item.timestamp_ms)}</span>
              <span class="mono">{item.size_bytes < 1024 ? `${item.size_bytes} B` : `${(item.size_bytes / 1024).toFixed(1)} KB`}</span>
            </div>
          </button>
        {/each}
      </LoadState>
    </div>
  {:else}
    <div class="ics-controls">
      <div class="input-group ics-search">
        <Icon name="search" size={12} />
        <input
          type="search"
          dir="ltr"
          aria-label={mode === 'git-branch' ? 'Filter branches' : 'Filter revisions'}
          placeholder={mode === 'git-branch' ? 'Filter branches…' : 'Filter commits…'}
          value={refFilter}
          oninput={(e) => onRefFilter((e.currentTarget as HTMLInputElement).value)}
        />
      </div>
    </div>
    <div class="ics-list">
      <LoadState what={mode === 'git-branch' ? 'branches' : 'revisions'} {loading} {error} empty={mode === 'git-branch' ? branches.length === 0 : commits.length === 0} variant="compact">
        {#if mode === 'git-branch'}
          {#each branches as br (br.remote ? `r:${br.name}` : `l:${br.name}`)}
            <button type="button" class="ics-row" class:active={selectedRef === br.name} onclick={() => onSelectRef(br.name)} data-testid="ide-compare-branch-row">
              <div class="ics-row-top">
                <Icon name={br.remote ? 'globe' : 'branch'} size={12} />
                <span class="ics-path mono ellipsis" dir="ltr">{br.name}</span>
                {#if br.is_current}<Badge tone="accent" variant="outline" label="HEAD" />{/if}
              </div>
            </button>
          {/each}
        {:else}
          {#each commits as c (c.sha)}
            <button type="button" class="ics-row" class:active={selectedRef === c.sha} onclick={() => onSelectRef(c.sha)} data-testid="ide-compare-revision-row">
              <div class="ics-row-top">
                <span class="ics-path ellipsis" dir="auto">{c.subject || c.short_sha}</span>
              </div>
              <div class="ics-row-meta">
                <span class="ellipsis">{c.author}</span>
                <span class="mono">{c.short_sha}</span>
              </div>
            </button>
          {/each}
        {/if}
      </LoadState>
    </div>
  {/if}
</aside>

<style>
  .ics-left { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; border-inline-end: 1px solid var(--border); background: var(--surface); }
  .ics-controls { display: flex; flex-direction: column; gap: 6px; padding: 8px; border-block-end: 1px solid var(--border); flex-shrink: 0; }
  .ics-top-row { display: flex; align-items: center; gap: 6px; }
  .ics-seg { flex: 1; display: flex; }
  .ics-seg button { flex: 1; display: inline-flex; align-items: center; justify-content: center; gap: 4px; block-size: 24px; font-size: var(--fs-xs); }
  .ics-label-btn { block-size: 24px; padding-block: 0; flex-shrink: 0; }
  .ics-search { block-size: 26px; font-size: var(--fs-xs); }
  .ics-list { flex: 1; min-block-size: 0; overflow-y: auto; }
  .ics-row {
    inline-size: 100%; display: flex; flex-direction: column; gap: 4px;
    padding-block: 6px; padding-inline: 10px; border: 0;
    border-block-end: 1px solid var(--separator); background: transparent;
    color: var(--text); text-align: start; cursor: pointer;
  }
  .ics-row:hover { background: var(--hover); }
  .ics-left.fill { flex: 1; min-inline-size: 0; border-inline-end: 0; }
  /* Phone list step: rows and the filter are touch-sized. */
  .ics-left.fill .ics-row { min-block-size: 44px; padding-block: 8px; }
  .ics-left.fill .ics-search { block-size: 36px; }
  .ics-left.fill .ics-seg button, .ics-left.fill .ics-label-btn { block-size: 32px; }
  .ics-row.active { background: var(--accent-soft); }
  .ics-row-top { display: flex; align-items: center; gap: 6px; min-inline-size: 0; font-size: var(--fs-xs); }
  .ics-path { flex: 1; font-size: var(--fs-xs); font-weight: 500; }
  .ics-row-meta { display: flex; align-items: center; justify-content: space-between; gap: 8px; font-size: var(--fs-xs); color: var(--text-dim); }
</style>
