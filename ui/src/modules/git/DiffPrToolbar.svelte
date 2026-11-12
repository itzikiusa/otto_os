<script lang="ts">
  // DiffViewer's PR-mode toolbar (the T3-style Code tab): ONE compact row —
  // the host's lead control (the "All commits ▾" picker), file / viewed /
  // hidden-tests counts, the inline search, then icon toggles: file tree,
  // unified / side by side, wrap, expand / collapse all, hide whitespace,
  // hide test files. Wraps instead of overflowing ≤1024px, where the file
  // stepper (DiffFileStepper) gets a row of its own. State lives in DiffViewer.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { plural } from '../../lib/plural';
  import DiffFileStepper from './DiffFileStepper.svelte';

  interface Props {
    lead?: Snippet;
    totalFiles: number;
    /** Files matching the active search (shown only while `searching`). */
    matchCount: number;
    searching: boolean;
    viewedCount: number;
    hideTests: boolean;
    hiddenTests: number;
    add: number;
    del: number;
    /** Some files load on demand (a truncated / summary diff without a loader). */
    largeNote: boolean;
    rawSearch: string;
    /** The search lives here while the file tree (which has its own) is hidden. */
    showSearch: boolean;
    /** The host offers a file tree at all. */
    canTree: boolean;
    treeOpen: boolean;
    isMobile: boolean;
    mode: 'unified' | 'split';
    wrapLines: boolean;
    canFold: boolean;
    allCollapsed: boolean;
    ontoggletree: () => void;
    ontogglewrap: () => void;
    ontoggletests: () => void;
    onfold: () => void;
    /** The PR files feed (prFilesBridge) the phone's file stepper drives. */
    feedKey?: string | null;
    /** The host can ask the forge for a whitespace-ignoring diff. */
    canWs?: boolean;
    ignoreWs?: boolean;
    /** Asked, but this forge can't (the response's `whitespace_ignored: false`). */
    wsUnsupported?: boolean;
    ontogglews?: () => void;
    /** A content search still covers only this many of `searchOf` files
     *  (null = all of them, or no search). */
    searchDone?: number | null;
    searchOf?: number;
    /** Search the files past the search load cap too. */
    onsearchall?: () => void;
  }
  let {
    lead,
    totalFiles,
    matchCount,
    searching,
    viewedCount,
    hideTests,
    hiddenTests,
    add,
    del,
    largeNote,
    rawSearch = $bindable(),
    showSearch,
    canTree,
    treeOpen,
    isMobile,
    mode = $bindable(),
    wrapLines,
    canFold,
    allCollapsed,
    ontoggletree,
    ontogglewrap,
    ontoggletests,
    onfold,
    feedKey = null,
    canWs = false,
    ignoreWs = false,
    wsUnsupported = false,
    ontogglews,
    searchDone = null,
    searchOf = 0,
    onsearchall,
  }: Props = $props();

  // Its height, for the diff's file headers to land BELOW it on a jump
  // (diff-pr.css: `scroll-margin-top`) rather than under the pinned toolbar.
  let el: HTMLDivElement | undefined = $state();
  let height = $state(0);
  $effect(() => {
    el?.parentElement?.style.setProperty('--pr-toolbar-h', `${height}px`);
  });
</script>

<div class="diff-toolbar pr-toolbar" bind:this={el} bind:offsetHeight={height}>
  {#if lead}{@render lead()}{/if}
  <span class="pt-stats">
    {#if searching}
      <span>{matchCount} / {plural(totalFiles, 'file')}</span>
    {:else}
      <span>{plural(totalFiles, 'file')}</span>
    {/if}
    <span class="pt-dot" aria-hidden="true">·</span>
    <span class="pt-viewed">{viewedCount} / {totalFiles} viewed</span>
    {#if hideTests && hiddenTests > 0}
      <span class="pt-dot" aria-hidden="true">·</span>
      <span class="pt-hidden">{plural(hiddenTests, 'test file')} hidden</span>
    {/if}
    <span class="pt-add">+{add}</span>
    <span class="pt-del">−{del}</span>
    {#if largeNote}
      <span class="pt-note" title="The diff is too large to send at once; open a file to load its lines.">
        Large diff — some files load on demand
      </span>
    {/if}
    {#if wsUnsupported}
      <span class="pt-note" title="This forge’s API has no whitespace-ignoring diff; the normal diff is shown.">Whitespace changes can’t be hidden here</span>
    {/if}
  </span>
  <span class="grow"></span>
  {#if showSearch}
    <div class="pt-search-wrap">
      <input dir="auto"
        class="toolbar-search input"
        type="text"
        placeholder="Search files & diff…"
        bind:value={rawSearch}
        aria-label="Search files and diff content"
      />
      {#if searching}
        <span class="pt-match">{matchCount} match{matchCount === 1 ? '' : 'es'}</span>
      {/if}
    </div>
  {/if}
  {#if searching && searchDone !== null}
    <!-- Content search covers files whose lines are loaded; the rest load in
         the background (diff-prefetch.ts) up to a cap. -->
    <span class="pt-searchnote">
      Searched {searchDone} of {plural(searchOf, 'file')}
      {#if onsearchall}<button class="pt-link" onclick={onsearchall}>Load the rest</button>{/if}
    </span>
  {/if}
  <div class="pt-icons" role="group" aria-label="Diff view options">
    {#if canTree}
      <button
        class="icon-btn pt-tog pt-tree"
        aria-pressed={treeOpen}
        aria-label={treeOpen ? 'Hide file tree' : 'Show file tree'}
        title={treeOpen ? 'Hide file tree' : 'Show file tree'}
        onclick={ontoggletree}
      ><Icon name="sidebar" size={14} /></button>
    {/if}
    {#if !isMobile}
      <!-- Side by side is desktop-only; ≤1024 always renders unified. -->
      <div class="segmented pt-seg" role="group" aria-label="Diff layout">
        <button class:active={mode === 'unified'} aria-pressed={mode === 'unified'} aria-label="Unified" title="Unified" onclick={() => (mode = 'unified')}><Icon name="unified" size={14} /></button>
        <button class:active={mode === 'split'} aria-pressed={mode === 'split'} aria-label="Side by side" title="Side by side" onclick={() => (mode = 'split')}><Icon name="columns" size={14} /></button>
      </div>
      <button
        class="icon-btn pt-tog"
        aria-pressed={wrapLines}
        aria-label="Wrap long lines"
        title={wrapLines ? 'Long lines wrap — click to scroll them sideways instead' : 'Long lines scroll sideways — click to wrap them'}
        onclick={ontogglewrap}
      ><Icon name="wrap" size={14} /></button>
    {/if}
    {#if canWs}
      <button
        class="icon-btn pt-tog"
        aria-pressed={ignoreWs}
        aria-label="Hide whitespace changes"
        title={ignoreWs ? 'Whitespace-only changes hidden — click to show them' : 'Hide whitespace-only changes'}
        onclick={ontogglews}
      ><Icon name="format" size={14} /></button>
    {/if}
    {#if canFold}
      <button
        class="icon-btn pt-tog"
        aria-label={allCollapsed ? 'Expand all' : 'Collapse all'}
        title={allCollapsed ? 'Expand every file (only the rows on screen render)' : 'Collapse every file'}
        onclick={onfold}
      ><Icon name={allCollapsed ? 'unfold' : 'fold'} size={14} /></button>
    {/if}
    <button
      class="icon-btn pt-tog"
      aria-pressed={hideTests}
      aria-label="Hide test files"
      title={hideTests
        ? `Code only — ${plural(hiddenTests, 'test file')} hidden. Click to show tests.`
        : 'Hide test files (*.spec.ts, *_test.go, test_*.py, src/test/, …)'}
      onclick={ontoggletests}
    ><Icon name="flask" size={14} /></button>
  </div>
  {#if isMobile && feedKey}<DiffFileStepper {feedKey} />{/if}
</div>

<style>
  /* Pinned to the top of whatever scrolls the diff (the PR pane on desktop,
     the diff itself on a phone), so commits / viewed / search / view toggles
     stay at hand on a long PR. `flex: none`: in the phone's flex-column
     scroller a shrinkable toolbar collapsed to one row and its wrapped
     search row slid under the first file. */
  .pr-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 30px;
    flex: none;
    position: sticky;
    top: 0;
    z-index: var(--z-sticky);
    background: var(--surface);
  }
  .pt-stats {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .pt-dot {
    opacity: 0.6;
  }
  .pt-hidden {
    color: var(--warning);
  }
  .pt-add {
    color: var(--success);
    font-weight: 600;
  }
  .pt-del {
    color: var(--danger);
    font-weight: 600;
  }
  .pt-note {
    color: var(--text-dim);
  }
  .pt-searchnote {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .pt-link {
    padding: 0;
    border: none;
    background: none;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    text-decoration: underline;
  }
  .pt-search-wrap {
    position: relative;
    display: flex;
    align-items: center;
  }
  .toolbar-search {
    font-size: var(--fs-xs);
    height: 26px;
    padding: 0 8px;
    width: 200px;
  }
  .pt-match {
    position: absolute;
    inset-inline-end: 8px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    pointer-events: none;
    white-space: nowrap;
  }
  .pt-icons {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .pt-seg {
    margin-inline: 2px;
  }
  .pt-seg > button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .pt-tog[aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .pt-tog:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }

  @media (max-width: 1024px) {
    .pr-toolbar {
      flex-wrap: wrap;
    }
    .pt-stats {
      flex-wrap: wrap;
      white-space: normal;
      font-size: var(--fs-s);
    }
    .pt-search-wrap {
      flex: 1;
      min-width: 120px;
    }
    .toolbar-search {
      font-size: var(--fs-m);
      height: 32px;
      width: 100%;
    }
    .pt-tog {
      width: 36px;
      height: 36px;
    }
    .pt-searchnote {
      font-size: var(--fs-s);
      white-space: normal;
    }
    .pt-link {
      min-height: 36px;
    }
  }
  /* A phone in landscape (~380 px tall, most of it app and PR chrome): ONE
     pinned row, so the code keeps the height. The stepper sits inline (its
     counter keeps "12 / 54"; the file name shows in the file header), and
     the counts row gives way — the PR header carries files and +/−, the
     stepper the position, the picker the Viewed marks. */
  @media (max-width: 1024px) and (max-height: 500px) {
    .pr-toolbar {
      gap: 4px;
      min-height: 0;
    }
    /* …and the tree toggle: there the tree is the stepper's file picker. */
    .pt-stats,
    .pr-toolbar :global(.pt-tree) {
      display: none;
    }
    .pt-search-wrap {
      flex: 1 1 0;
      min-width: 64px;
    }
    .toolbar-search {
      height: 36px;
    }
    .pt-icons {
      gap: 0;
    }
  }
</style>
