<script lang="ts">
  // The left timeline / revision / branch list of `IdeComparisonModal` (a list pane).
  import type { CommitInfo, RefBranch } from '../../../lib/api/types';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import SideRow from '../../../lib/components/side/SideRow.svelte';
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

<!-- The revisions are a list (the shared pane, rows and keyboard walk); in the
     phone sheet the list step fills it (widthPx 0). -->
<div class="ics-left" class:fill={widthPx <= 0} style:inline-size={widthPx > 0 ? `${widthPx}px` : undefined} data-testid="ide-local-history-modal">
  {#if mode === 'local-history'}
    <ListPane name="Local history" count={filteredEntries.length} primary={{ icon: 'tag', label: 'Save a Local History label…', onclick: onPutLabel, testid: 'ide-local-history-put-label' }}>
      {#snippet filterRow()}
        <div class="segmented ics-seg" role="group" aria-label="Local history filter">
          <button type="button" class:active={filterMode === 'all'} aria-pressed={filterMode === 'all'} onclick={() => onFilterMode('all')} data-testid="ide-local-history-filter-all">All{entries.length > 0 ? ` (${entries.length})` : ''}</button>
          <button type="button" class:active={filterMode === 'deleted'} aria-pressed={filterMode === 'deleted'} onclick={() => onFilterMode('deleted')} data-testid="ide-local-history-filter-deleted">Deleted{deletedCount > 0 ? ` (${deletedCount})` : ''}</button>
        </div>
      {/snippet}
      {#snippet searchRow()}
        <div class="input-group">
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
      {/snippet}
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
          {@const kind = item.label || kindLabel(item.event_kind)}
          {@const when = `${formatTime(item.timestamp_ms)} · ${item.size_bytes < 1024 ? `${item.size_bytes} B` : `${(item.size_bytes / 1024).toFixed(1)} KB`}`}
          <div class="side-row two-line" class:selected={selectedEntryId === item.id}>
            <button type="button" class="side-row-main" data-side-row aria-current={selectedEntryId === item.id ? 'true' : undefined} onclick={() => onSelectEntry(item.id)} data-testid="ide-local-history-row">
              <span class="side-row-text">
                <span class="side-row-title" dir={isDir ? 'ltr' : 'auto'}>{isDir ? item.rel_path : kind}</span>
                <span class="side-row-meta">{isDir ? `${kind} · ${when}` : when}</span>
              </span>
            </button>
          </div>
        {/each}
      </LoadState>
    </ListPane>
  {:else}
    <ListPane name={mode === 'git-branch' ? 'Branches' : 'Revisions'} count={mode === 'git-branch' ? branches.length : commits.length}>
      {#snippet searchRow()}
        <div class="input-group">
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
      {/snippet}
      <LoadState what={mode === 'git-branch' ? 'branches' : 'revisions'} {loading} {error} empty={mode === 'git-branch' ? branches.length === 0 : commits.length === 0} variant="compact">
        {#if mode === 'git-branch'}
          {#each branches as br (br.remote ? `r:${br.name}` : `l:${br.name}`)}
            <SideRow icon={br.remote ? 'globe' : 'branch'} title={br.name} selected={selectedRef === br.name} onclick={() => onSelectRef(br.name)} testid="ide-compare-branch-row">
              {#snippet trailing()}{#if br.is_current}HEAD{/if}{/snippet}
            </SideRow>
          {/each}
        {:else}
          {#each commits as c (c.sha)}
            <SideRow title={c.subject || c.short_sha} meta={`${c.author} · ${c.short_sha}`} selected={selectedRef === c.sha} onclick={() => onSelectRef(c.sha)} testid="ide-compare-revision-row" />
          {/each}
        {/if}
      </LoadState>
    </ListPane>
  {/if}
</div>

<style>
  .ics-left { display: flex; flex-direction: column; flex-shrink: 0; min-block-size: 0; }
  .ics-left.fill { flex: 1; min-inline-size: 0; }
  .ics-left.fill :global(.side-list) { border-inline-end: 0; }
  .ics-seg { display: flex; }
  .ics-seg button { flex: 1; }
</style>
