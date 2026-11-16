<script lang="ts">
  // Top stats, search filter, collapse-all, and layout mode bar for `DiffViewer.svelte`.
  import { plural } from '../plural';

  interface Props {
    fileCount: number;
    matchCount: number;
    totalAdded: number;
    totalDeleted: number;
    showLargeDiffNote: boolean;
    rawSearch: string;
    search: string;
    allCollapsed: boolean;
    isMobile: boolean;
    mode: 'unified' | 'split';
    onsearchchange: (next: string) => void;
    ontoggleallcollapsed: () => void;
    onmodechange: (next: 'unified' | 'split') => void;
  }

  let {
    fileCount,
    matchCount,
    totalAdded,
    totalDeleted,
    showLargeDiffNote,
    rawSearch,
    search,
    allCollapsed,
    isMobile,
    mode,
    onsearchchange,
    ontoggleallcollapsed,
    onmodechange,
  }: Props = $props();
</script>

<div class="diff-toolbar">
  <span class="diff-stats">
    {#if search}
      <span>{matchCount} / {plural(fileCount, 'file')}</span>
    {:else}
      {plural(fileCount, 'file')}
    {/if}
    <span class="add">+{totalAdded}</span>
    <span class="del">−{totalDeleted}</span>
    {#if showLargeDiffNote}
      <span
        class="diff-note"
        title="The diff is too large to send at once; open a file to load its lines."
      >
        Large diff — some files load on demand
      </span>
    {/if}
  </span>
  <span class="grow"></span>
  <div class="toolbar-search-wrap">
    <input
      dir="auto"
      class="toolbar-search input"
      type="text"
      placeholder="Search files & diff…"
      value={rawSearch}
      oninput={(e) => onsearchchange(e.currentTarget.value)}
      aria-label="Search files and diff content"
    />
    {#if search}
      <span class="toolbar-match-count">{matchCount} match{matchCount === 1 ? '' : 'es'}</span>
    {/if}
  </div>
  {#if fileCount > 1}
    <button
      type="button"
      class="btn small ghost"
      onclick={ontoggleallcollapsed}
      title={allCollapsed
        ? 'Expand every file (only the rows on screen render)'
        : 'Collapse every file'}
    >{allCollapsed ? 'Expand all' : 'Collapse all'}</button>
  {/if}
  {#if !isMobile}
    <div class="segmented" role="group" aria-label="Diff layout">
      <button
        type="button"
        class:active={mode === 'unified'}
        aria-pressed={mode === 'unified'}
        onclick={() => onmodechange('unified')}
      >Unified</button>
      <button
        type="button"
        class:active={mode === 'split'}
        aria-pressed={mode === 'split'}
        onclick={() => onmodechange('split')}
      >Side by side</button>
    </div>
  {/if}
</div>

<style>
  .diff-toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .diff-stats {
    font-size: var(--fs-s);
    color: var(--text-dim);
    display: inline-flex;
    gap: 8px;
    align-items: center;
  }
  .diff-stats .add {
    color: var(--success);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .diff-stats .del {
    color: var(--danger);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .diff-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .toolbar-search-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }
  .toolbar-search {
    font-size: var(--fs-xs);
    block-size: 26px;
    padding-block: 0;
    padding-inline: 8px;
    inline-size: 180px;
  }
  .toolbar-match-count {
    position: absolute;
    inset-inline-end: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    pointer-events: none;
    white-space: nowrap;
  }
  @media (max-width: 1024px) {
    .diff-toolbar {
      flex-wrap: wrap;
      gap: 8px;
    }
    .toolbar-search-wrap {
      flex: 1;
      min-inline-size: 120px;
    }
    .diff-stats {
      font-size: var(--fs-m);
    }
    .diff-stats :is(.add, .del) {
      font-size: var(--fs-m);
    }
    .toolbar-search {
      font-size: var(--fs-m);
      block-size: 32px;
      inline-size: 100%;
    }
  }
</style>
