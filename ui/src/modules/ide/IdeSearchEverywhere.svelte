<script lang="ts">
  // The IDE quick-open: ⇧⇧ All, ⌘P Files (an empty query lists recent files), ⌘E
  // recent files, ⌘G `:line` in the active file, ⌘O Symbols, the Actions tab (from
  // shell/ideActions.ts) and Find & Replace in Path (⌘⇧F / ⌘⇧R: Match Case, Whole
  // Word, Regex, File Mask, Scope, grouped matches, per-file Replace, Replace All).
  //
  // A top-anchored panel, not a centred Modal: its top edge never moves while results
  // arrive, and on a phone it is a full-width sheet under the status bar that keeps
  // above the keyboard. Focus handling is the shared `dialogFocus` (trap, Esc, return).
  import { onDestroy, untrack } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { dialogFocus } from '../../lib/dialogFocus';
  import { plural } from '../../lib/plural';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ide, ideBasename, type IdeSearchItem } from './ideStore.svelte';
  import { pushIdeNavLocation } from './editor/ideEditorStoreBridge.svelte';
  import { quickOpenRequest } from './search/ideQuickOpen.svelte';
  import {
    dropTextHitsShownAsSymbols,
    matchesFileMask,
    parentFolderOf,
    parseQuickOpenQuery,
    recentFileRows,
    type QuickOpenTab,
  } from './search/ideQuickOpenModel';
  import { currentIdeShellCtx, ideActionHint, ideActionsFor, type IdeAction } from './shell/ideActions';
  import { ideLayout } from './shell/ideLayout.svelte';
  import { cancelWorkspaceSearch } from './shell/ideLspAndSearchHelpers';

  type NavRow =
    | { kind: 'action'; action: IdeAction }
    | { kind: 'hit'; item: IdeSearchItem; groupStart: boolean }
    | { kind: 'recent'; path: string; groupStart: string | null }
    | { kind: 'line'; line: number | null; col: number | null };

  const SEARCH_TABS: readonly TabItem<QuickOpenTab>[] = [
    { id: 'all', label: 'All', testid: 'ide-se-tab-all' },
    { id: 'files', label: 'Files', testid: 'ide-se-tab-files' },
    { id: 'symbols', label: 'Symbols', testid: 'ide-se-tab-symbols' },
    { id: 'text', label: 'Find in path', testid: 'ide-se-tab-text' },
    { id: 'actions', label: 'Actions', testid: 'ide-se-tab-actions' },
  ];

  const ROW_CAP = 120;
  const uid = $props.id();

  let tab = $state<QuickOpenTab>('all');
  let query = $state('');
  let replacement = $state('');
  let caseSensitive = $state(false);
  let wholeWord = $state(false);
  let regex = $state(false);
  let fileGlob = $state('');
  let scopeMode = $state<'project' | 'folder' | 'custom'>('project');
  let customScopePath = $state('');
  let replaceOpen = $state(false);
  let replaceMode = $state<'preview' | 'apply' | null>(null);
  let selectedIdx = $state(0);
  let showAllRows = $state(false);
  let resultsEl = $state<HTMLDivElement | undefined>(undefined);
  let inputEl = $state<HTMLInputElement | undefined>(undefined);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let downOnBackdrop = false;
  let wasOpen = false;
  let syncedInitTab: QuickOpenTab = 'all';
  let syncedReplaceMode = false;
  let syncedSeq = -1;

  function hasActiveQuery(t: QuickOpenTab, q: string): boolean {
    return t === 'text' ? q.length > 0 : q.trim().length > 0;
  }

  const activeDirPath = $derived.by((): string => {
    const p = ide.activePath;
    if (!p) return ide.target.root;
    const idx = p.lastIndexOf('/');
    return idx > 0 ? p.slice(0, idx) : ide.target.root;
  });

  const effectiveScopePath = $derived.by((): string | undefined => {
    if (scopeMode === 'folder') return activeDirPath;
    if (scopeMode === 'custom' && customScopePath.trim()) return customScopePath.trim();
    return undefined;
  });

  // A preview only describes the query, replacement and options it ran with.
  $effect(() => {
    void [query, replacement, caseSensitive, wholeWord, regex, fileGlob, effectiveScopePath];
    ide.replacePreview = null;
  });

  const parsed = $derived(parseQuickOpenQuery(tab, query));

  // Register as an open overlay: the native browser webview hides, and the IDE's own
  // window keys stand down (except the ones that retarget this picker).
  $effect(() => {
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });

  $effect(() => {
    const open = ide.searchEverywhereOpen;
    const nextInitTab = ide.searchInitialTab;
    const nextReplace = Boolean(ide.searchReplaceMode);
    const seq = quickOpenRequest.seq;
    if (open && (!wasOpen || nextInitTab !== syncedInitTab || nextReplace !== syncedReplaceMode || seq !== syncedSeq)) {
      untrack(() => {
        tab = nextInitTab;
        replaceOpen = nextReplace;
        syncedInitTab = nextInitTab;
        syncedReplaceMode = nextReplace;
        // A request's starting query (`:` for Go to Line, '' for ⌘P / ⌘E) is taken once.
        if (!wasOpen || seq !== syncedSeq) {
          query = quickOpenRequest.query;
          quickOpenRequest.query = '';
          syncedSeq = seq;
        }
        if (ide.searchScopePath) {
          customScopePath = ide.searchScopePath;
          scopeMode = 'custom';
        } else if (!wasOpen) {
          scopeMode = 'project';
        }
        selectedIdx = 0;
        showAllRows = false;
        ide.replacePreview = null;
        if (hasActiveQuery(tab, query) && tab !== 'actions') triggerSearch(0);
        queueMicrotask(() => {
          inputEl?.focus();
          const end = inputEl?.value.length ?? 0;
          inputEl?.setSelectionRange(end, end);
        });
      });
    }
    wasOpen = open;
  });

  onDestroy(() => {
    if (debounceTimer) clearTimeout(debounceTimer);
    cancelWorkspaceSearch();
  });

  const actionCtx = $derived({ ...currentIdeShellCtx(), stack: ideLayout.stack });

  const filteredActions = $derived.by((): IdeAction[] => {
    if (tab !== 'all' && tab !== 'actions') return [];
    const q = query.trim().toLowerCase();
    if (!q) return tab === 'actions' ? ideActionsFor('search', actionCtx) : [];
    return ideActionsFor('search', actionCtx).filter((a) =>
      `${a.title} ${a.label?.(actionCtx) ?? ''} ${a.keywords ?? ''} ${a.id}`.toLowerCase().includes(q),
    );
  });

  const rows = $derived.by((): NavRow[] => {
    const out: NavRow[] = [];
    if (tab === 'actions') {
      for (const action of filteredActions) out.push({ kind: 'action', action });
      return out;
    }
    if (parsed.lineOnly) {
      if (ide.activePath) out.push({ kind: 'line', line: parsed.line, col: parsed.col });
      return out;
    }
    if ((tab === 'all' || tab === 'files') && !query.trim()) {
      let prev: string | null = null;
      for (const r of recentFileRows(ide.recentPaths, ide.tabs.map((t) => t.path), ide.activePath)) {
        out.push({ kind: 'recent', path: r.path, groupStart: r.source !== prev ? r.source : null });
        prev = r.source;
      }
      return out;
    }
    const hits = tab === 'all' ? dropTextHitsShownAsSymbols(ide.searchResults) : ide.searchResults;
    let prevPath = '';
    for (const item of hits) {
      if (tab === 'text' && !matchesFileMask(ideBasename(item.path), ide.relativePath(ide.resolvePath(item.path)), fileGlob)) continue;
      const groupStart = tab === 'text' && item.path !== prevPath;
      prevPath = item.path;
      out.push({ kind: 'hit', item, groupStart });
    }
    for (const action of filteredActions) out.push({ kind: 'action', action });
    return out;
  });

  const visibleRows = $derived(!showAllRows && rows.length > ROW_CAP ? rows.slice(0, ROW_CAP) : rows);

  const dialogLabel = $derived(
    parsed.lineOnly ? 'Go to line' : replaceOpen ? 'Replace in path' : tab === 'text' ? 'Find in path' : tab === 'files' ? 'Go to file' : 'Search everywhere',
  );

  function buildSearchOpts() {
    // The match options belong to Find in path; Files / Symbols / All search the name as typed.
    const text = tab === 'text';
    ide.searchScopePath = text ? (effectiveScopePath ?? null) : null;
    return {
      caseSensitive: text && caseSensitive,
      regex: text && regex,
      wholeWord: text && wholeWord,
      fileGlob: text ? fileGlob.trim() || undefined : undefined,
      scopePath: text ? (effectiveScopePath ?? null) : null,
    };
  }

  function close(): void {
    if (ide.replaceLoading) return;
    if (debounceTimer) clearTimeout(debounceTimer);
    cancelWorkspaceSearch();
    ide.searchLoading = false;
    ide.searchReplaceMode = false;
    ide.searchScopePath = null;
    ide.searchEverywhereOpen = false;
  }

  function triggerSearch(delayMs = 120): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const effectiveQ = tab === 'text' ? query : parsed.cleanQuery.trim();
    if (tab === 'actions' || effectiveQ.length === 0) {
      cancelWorkspaceSearch();
      ide.searchLoading = false;
      ide.searchResults = [];
      return;
    }
    const mode = tab === 'all' ? 'all' : tab;
    const run = () => void ide.searchWorkspace(effectiveQ, mode, buildSearchOpts());
    if (delayMs <= 0) run();
    else debounceTimer = setTimeout(run, delayMs);
  }

  function onQueryInput(): void {
    selectedIdx = 0;
    showAllRows = false;
    ide.replacePreview = null;
    triggerSearch(120);
  }

  function activateRow(row: NavRow | undefined): void {
    if (!row) return;
    if (row.kind === 'line' && row.line === null) return;
    close();
    if (row.kind === 'action') {
      if (!row.action.disabled?.(actionCtx)) row.action.run(actionCtx);
      return;
    }
    if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
    if (row.kind === 'line') {
      if (ide.activePath && row.line) ide.jumpTo(ide.activePath, row.line, row.col ?? 1);
      return;
    }
    if (row.kind === 'recent') {
      void ide.openFile(row.path, { preview: false });
      return;
    }
    const jumpLn = parsed.line ?? row.item.line ?? undefined;
    const jumpCol = parsed.col ?? row.item.column ?? undefined;
    void ide.openFile(row.item.path, { line: jumpLn, col: jumpCol, preview: false });
  }

  function scrollSelectedIntoView(): void {
    queueMicrotask(() => {
      resultsEl?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
    });
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (visibleRows.length > 0) {
        selectedIdx = (selectedIdx + 1) % visibleRows.length;
        scrollSelectedIntoView();
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (visibleRows.length > 0) {
        selectedIdx = (selectedIdx - 1 + visibleRows.length) % visibleRows.length;
        scrollSelectedIntoView();
      }
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      activateRow(visibleRows[selectedIdx]);
    }
  }

  async function previewReplace(): Promise<void> {
    if (query.length === 0) return;
    replaceMode = 'preview';
    try {
      await ide.replaceInWorkspace(query, replacement, true, buildSearchOpts());
    } finally {
      replaceMode = null;
    }
  }

  async function replaceSingleFile(filePath: string): Promise<void> {
    if (query.length === 0) return;
    replaceMode = 'apply';
    try {
      await ide.replaceInWorkspace(query, replacement, false, {
        caseSensitive,
        regex,
        wholeWord,
        paths: [filePath],
      });
      await previewReplace();
      triggerSearch(0);
    } finally {
      replaceMode = null;
    }
  }

  async function confirmAndReplaceAll(): Promise<void> {
    if (query.length === 0) return;
    replaceMode = 'apply';
    try {
      // Always a fresh dry run: the count we confirm is the count the disk has now.
      const preview = await ide.replaceInWorkspace(query, replacement, true, buildSearchOpts());
      if (!preview || preview.replacements === 0) return;
      const ok = await confirmer.ask(
        `Replace ${plural(preview.replacements, 'occurrence')} across ${plural(preview.files_changed, 'file')}?`,
        { title: 'Replace in path', confirmLabel: 'Replace all', danger: true },
      );
      if (!ok) return;
      await ide.replaceInWorkspace(query, replacement, false, buildSearchOpts());
      close();
    } finally {
      replaceMode = null;
    }
  }

  function placeholderFor(t: QuickOpenTab): string {
    if (t === 'files') return 'File name, file.go:42, or :42 for a line…';
    if (t === 'symbols') return 'Functions, structs, classes, types…';
    if (t === 'text') return 'Find in path across the project…';
    if (t === 'actions') return 'Type an IDE command…';
    return 'Files (file:line), symbols, text, or actions…';
  }

  function rowKey(row: NavRow, idx: number): string {
    if (row.kind === 'action') return `act:${row.action.id}`;
    if (row.kind === 'recent') return `rec:${row.path}`;
    if (row.kind === 'line') return 'line';
    return `hit:${row.item.kind}:${row.item.path}:${row.item.line ?? 0}:${idx}`;
  }
</script>

<div
  class="qo-backdrop"
  role="presentation"
  onpointerdown={(e) => (downOnBackdrop = e.target === e.currentTarget)}
  onclick={(e) => {
    if (downOnBackdrop && e.target === e.currentTarget) close();
    downOnBackdrop = false;
  }}
>
  <div class="qo-panel" role="dialog" aria-modal="true" aria-label={dialogLabel} use:dialogFocus={close} data-testid="ide-quick-open">
    <Tabs
      size="s"
      label="Search scope"
      idBase="ide-se"
      tabs={SEARCH_TABS}
      value={tab}
      onchange={(id) => {
        tab = id;
        selectedIdx = 0;
        showAllRows = false;
        if (id !== 'text') replaceOpen = false;
        triggerSearch(0);
      }}
    />

    <div class="se-input-row">
      <span class="se-lead-icon"><Icon name="search" size={14} /></span>
      <input
        bind:this={inputEl}
        class="se-input"
        type="search"
        dir="ltr"
        role="combobox"
        aria-expanded="true"
        aria-autocomplete="list"
        aria-controls="{uid}-list"
        aria-activedescendant={visibleRows.length ? `${uid}-opt-${selectedIdx}` : undefined}
        aria-label={dialogLabel}
        placeholder={placeholderFor(tab)}
        bind:value={query}
        oninput={onQueryInput}
        onkeydown={onKeydown}
        data-testid="ide-search-input"
      />
      {#if tab === 'text'}
        <div class="se-opts" data-testid="ide-se-find-options">
          <button type="button" class="btn small ghost se-opt" aria-label="Match case" title="Match case" aria-pressed={caseSensitive} onclick={() => { caseSensitive = !caseSensitive; triggerSearch(0); }} data-testid="ide-se-case-btn">Aa</button>
          <button type="button" class="btn small ghost se-opt" aria-label="Whole word" title="Match whole word" aria-pressed={wholeWord} onclick={() => { wholeWord = !wholeWord; triggerSearch(0); }} data-testid="ide-se-word-btn">W</button>
          <button type="button" class="btn small ghost se-opt" aria-label="Regular expression" title="Use regular expression" aria-pressed={regex} onclick={() => { regex = !regex; triggerSearch(0); }} data-testid="ide-se-regex-btn">.*</button>
          <button type="button" class="btn small ghost" aria-pressed={replaceOpen} onclick={() => (replaceOpen = !replaceOpen)} data-testid="ide-se-replace-toggle">Replace…</button>
        </div>
      {/if}
    </div>

    {#if tab === 'text'}
      <div class="se-filter-row">
        <div class="segmented" role="group" aria-label="Directory scope">
          <button type="button" aria-pressed={scopeMode === 'project'} onclick={() => { scopeMode = 'project'; triggerSearch(0); }}>In project</button>
          {#if ide.activePath}
            <button type="button" aria-pressed={scopeMode === 'folder'} onclick={() => { scopeMode = 'folder'; triggerSearch(0); }} title={activeDirPath}>Current dir</button>
          {/if}
          {#if customScopePath}
            <button type="button" aria-pressed={scopeMode === 'custom'} onclick={() => { scopeMode = 'custom'; triggerSearch(0); }} title={customScopePath}>Directory</button>
          {/if}
        </div>
        <input
          class="se-mask-input mono"
          type="text"
          dir="ltr"
          aria-label="File mask filter"
          placeholder="File mask: *.go, *.ts, !*_test.go"
          bind:value={fileGlob}
          oninput={() => triggerSearch(150)}
          data-testid="ide-se-mask-input"
        />
      </div>

      {#if replaceOpen}
        <form
          class="se-replace-row"
          onsubmit={(e) => {
            e.preventDefault();
            if (query.length > 0 && !ide.replaceLoading) void confirmAndReplaceAll();
          }}
        >
          <input class="se-input" type="text" dir="ltr" aria-label="Replacement text" placeholder="Replace with…" bind:value={replacement} data-testid="ide-replace-input" />
          <button type="button" class="btn small" disabled={query.length === 0 || ide.replaceLoading} onclick={() => void previewReplace()}>
            {ide.replaceLoading && replaceMode === 'preview' ? 'Previewing…' : 'Preview'}
          </button>
          <button type="submit" class="btn small primary" disabled={query.length === 0 || ide.replaceLoading} data-testid="ide-replace-all-btn">
            {ide.replaceLoading && replaceMode === 'apply' ? 'Replacing…' : 'Replace all'}
          </button>
        </form>

        {#if ide.replacePreview}
          <div class="se-preview-box">
            <div class="se-preview-head">
              Preview: {plural(ide.replacePreview.replacements, 'occurrence')} in {plural(ide.replacePreview.files_changed, 'file')}
            </div>
            {#each ide.replacePreview.previews.slice(0, 12) as p (p.path)}
              <div class="se-preview-item" dir="ltr">
                <div class="se-preview-top">
                  <span class="se-preview-file">{ide.relativePath(ide.resolvePath(p.path))} ({plural(p.count, 'match', 'matches')})</span>
                  <button type="button" class="btn small ghost" disabled={ide.replaceLoading} onclick={() => void replaceSingleFile(p.path)}>Replace file</button>
                </div>
                <div class="se-diff-before">- {p.sample_before}</div>
                <div class="se-diff-after">+ {p.sample_after}</div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    {/if}

    <div class="se-results" bind:this={resultsEl}>
      <LoadState
        what="search results"
        variant="compact"
        loading={ide.searchLoading && rows.length === 0 && hasActiveQuery(tab, query) && !parsed.lineOnly}
        error={hasActiveQuery(tab, query) && !parsed.lineOnly ? ide.searchError : null}
        empty={rows.length === 0 && hasActiveQuery(tab, query)}
        onretry={() => triggerSearch(0)}
      >
        {#snippet emptyView()}
          <EmptyState
            icon="search"
            title={parsed.lineOnly ? 'Open a file first' : 'No matches found'}
            body={parsed.lineOnly ? 'Go to line moves the cursor in the file you are editing.' : 'Try a shorter query, or switch to All.'}
          />
        {/snippet}

        {#if rows.length === 0 && !hasActiveQuery(tab, query)}
          <p class="se-hint">{tab === 'symbols' ? 'Type a function, type or class name.' : tab === 'text' ? 'Type text to find across the project.' : 'No recent files yet. Type to search the project.'}</p>
        {/if}
        <div class="se-list" role="listbox" id="{uid}-list" aria-label="Results">
          {#each visibleRows as row, idx (rowKey(row, idx))}
            {#if row.kind === 'hit' && row.groupStart}
              <div class="se-group-head mono" dir="ltr" role="presentation">
                <Icon name="file" size={12} />
                <span>{ide.relativePath(row.item.path)}</span>
              </div>
            {:else if row.kind === 'recent' && row.groupStart}
              <div class="se-section-head" role="presentation">{row.groupStart === 'recent' ? 'Recent files' : 'Open tabs'}</div>
            {/if}
            <button
              type="button"
              id="{uid}-opt-{idx}"
              class="se-row"
              class:selected={idx === selectedIdx}
              role="option"
              tabindex="-1"
              aria-selected={idx === selectedIdx}
              onmouseenter={() => (selectedIdx = idx)}
              onclick={() => activateRow(row)}
              data-testid="ide-qo-row"
              data-kind={row.kind}
            >
              {#if row.kind === 'action'}
                {@const hint = ideActionHint(row.action)}
                <Icon name={row.action.icon} size={13} />
                <span class="se-main">{row.action.label?.(actionCtx) ?? row.action.title}</span>
                {#if tab === 'all'}<Badge variant="outline" tone="accent" label="Action" />{/if}
                {#if hint}<kbd class="se-kbd">{hint}</kbd>{/if}
              {:else if row.kind === 'recent'}
                {@const rel = ide.relativePath(row.path)}
                <Icon name="file" size={13} />
                <span class="se-hit-body se-hit-inline" dir="ltr">
                  <span class="se-name mono">{ideBasename(row.path)}</span>
                  <span class="se-sub">{parentFolderOf(rel)}</span>
                </span>
              {:else if row.kind === 'line'}
                <Icon name="step" size={13} />
                <span class="se-hit-body" dir="ltr">
                  <span class="se-main">{row.line ? `Go to line ${row.line}${row.col ? `, column ${row.col}` : ''}` : 'Type a line number, or line:column'}</span>
                  <span class="se-sub">{ide.activePath ? ide.relativePath(ide.activePath) : ''}</span>
                </span>
              {:else}
                <Icon name={row.item.kind === 'file' ? 'file' : row.item.kind === 'symbol' ? 'function' : 'search'} size={13} />
                <span class="se-hit-body" dir="ltr">
                  <span class="se-main mono">{row.item.kind === 'file' ? ideBasename(row.item.path) : row.item.preview}</span>
                  <span class="se-sub">{ide.relativePath(row.item.path)}{row.item.line ? `:${row.item.line}` : ''}</span>
                </span>
                <Badge variant="outline" tone={row.item.kind === 'symbol' ? 'info' : 'neutral'} label={row.item.symbol_kind || row.item.kind} />
              {/if}
            </button>
          {/each}
          {#if !showAllRows && rows.length > ROW_CAP}
            <button type="button" class="btn small ghost" onclick={() => (showAllRows = true)}>
              Show all {rows.length} matches ({rows.length - ROW_CAP} more)
            </button>
          {/if}
          {#if ide.searchTruncated && tab !== 'actions'}
            <p class="se-truncated" data-testid="ide-search-truncated">There are more matches than shown. Narrow the query to see the rest.</p>
          {/if}
        </div>
      </LoadState>
    </div>
  </div>
</div>

<style>
  /* Percentages of the inset:0 backdrop (= the window), never vh: in the WKWebView vh
     is the SCREEN height (same rule as the ⌘K palette). */
  .qo-backdrop {
    position: fixed; inset: 0; z-index: var(--z-modal);
    display: flex; flex-direction: column; align-items: center;
    background: var(--scrim-soft);
  }
  .qo-backdrop::before { content: ''; flex: 0 0 12%; }
  .qo-panel {
    display: flex; flex-direction: column; gap: var(--sp-3); flex: 0 1 auto; min-block-size: 0;
    inline-size: 720px; max-inline-size: calc(100% - 48px); max-block-size: min(560px, 76%);
    padding: var(--sp-4); background: var(--surface); border: 1px solid var(--glass-border);
    border-radius: var(--radius-l); box-shadow: var(--glass-shadow);
  }
  /* Phone: a full-width sheet from the top, sized to the visible viewport (`--app-h`
     follows the on-screen keyboard), so the field and the first results stay above it.
     A short page (a phone on its side, ~380-430 px tall) gets the same sheet: the
     centred card with a 12% top gap left room for two results (review I4-10). */
  @media (max-width: 640px), (max-height: 520px) {
    .qo-backdrop::before { flex-basis: env(safe-area-inset-top, 0px); }
    .qo-panel {
      inline-size: 100%; max-inline-size: 100%; border-block-start: none;
      border-start-start-radius: 0; border-start-end-radius: 0;
      max-block-size: calc(var(--app-h, 100%) - env(safe-area-inset-top, 0px) - 8px);
    }
  }
  @media (max-height: 520px) {
    .qo-panel { gap: var(--sp-2); padding-block: var(--sp-2); }
  }
  .se-input-row, .se-replace-row, .se-filter-row { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
  .se-opts { display: flex; align-items: center; gap: var(--sp-2); }
  .se-lead-icon { display: inline-flex; color: var(--text-dim); padding-inline-start: var(--sp-2); }
  .se-input {
    flex: 1; min-inline-size: 160px; block-size: 32px; padding-inline: 10px; font-size: var(--fs-s);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--control-border); border-radius: var(--radius-s);
  }
  .se-mask-input {
    flex: 1; min-inline-size: 160px; block-size: 26px; padding-inline: var(--sp-4); font-size: var(--fs-xs);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--control-border); border-radius: var(--radius-s);
  }
  .se-opt { font-family: var(--font-mono); padding-inline: var(--sp-4); }
  .se-opt[aria-pressed='true'] { background: var(--accent-soft); color: var(--accent-text); }
  .se-preview-box {
    display: flex; flex-direction: column; gap: var(--sp-3); max-block-size: 180px; overflow-y: auto; flex-shrink: 0;
    padding: var(--sp-4) 10px; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-xs);
  }
  .se-preview-head { font-weight: 600; color: var(--text); }
  .se-preview-item { font-family: var(--font-mono); font-size: var(--fs-xs); border-block-start: 1px solid var(--border); padding-block-start: var(--sp-2); }
  .se-preview-top { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); }
  .se-preview-file { color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-diff-before { color: var(--danger); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-diff-after { color: var(--success); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-results { flex: 1 1 auto; min-block-size: 0; overflow-y: auto; border-block-start: 1px solid var(--border); padding-block-start: var(--sp-3); }
  .se-list { display: flex; flex-direction: column; gap: var(--sp-1); }
  .se-truncated { margin: 0; padding: var(--sp-2) 10px; font-size: var(--fs-xs); color: var(--text-dim); }
  .se-hint { margin: 0; padding: var(--sp-3) 10px; font-size: var(--fs-xs); color: var(--text-dim); }
  .se-section-head { padding: var(--sp-2) 10px; font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .se-group-head {
    display: flex; align-items: center; gap: var(--sp-2); padding: var(--sp-2) var(--sp-4);
    font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); background: var(--surface-2); border-radius: var(--radius-s);
  }
  .se-row {
    display: flex; align-items: center; gap: var(--sp-4); inline-size: 100%; padding: var(--sp-3) 10px;
    background: transparent; border: none; border-radius: var(--radius-s); color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .se-row.selected, .se-row:hover { background: var(--surface-2); }
  .se-row.selected { outline: 1px solid var(--accent-line); outline-offset: -1px; }
  .se-hit-body { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .se-hit-inline { flex-direction: row; align-items: baseline; gap: var(--sp-3); }
  .se-name { flex-shrink: 0; max-inline-size: 70%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-main { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-sub { min-inline-size: 0; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-kbd { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); padding: var(--sp-1) var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-s); white-space: nowrap; }
</style>
