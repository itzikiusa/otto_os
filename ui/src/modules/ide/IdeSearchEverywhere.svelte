<script lang="ts">
  // Search Everywhere (⇧⇧ All, ⌘⇧O Files, ⌘O Symbols, Actions) +
  // Find & Replace in Path (⌘⇧F / ⌘⇧R: Match Case, Whole Word, Regex, File Mask,
  // Scope Directory, grouped match preview, per-file Replace, and Replace All).
  import { onDestroy } from 'svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { plural } from '../../lib/plural';
  import { ide, ideBasename, type IdeSearchItem } from './ideStore.svelte';
  import {
    areIdeBreakpointsMuted,
    closeAllIdeTabs,
    closeUnmodifiedIdeTabs,
    navigateIdeBack,
    navigateIdeForward,
    pushIdeNavLocation,
    toggleFormatOnSave,
    toggleIdeBreakpointsMuted,
  } from './editor/ideEditorStoreBridge.svelte';

  type SearchTab = 'all' | 'files' | 'symbols' | 'text' | 'actions';

  interface IdeQuickAction {
    id: string;
    label: string;
    shortcut?: string;
    icon: IconName;
    run: () => void;
  }

  type NavRow =
    | { kind: 'action'; action: IdeQuickAction }
    | { kind: 'hit'; item: IdeSearchItem; groupStart: boolean };

  interface ExtendedSearchStore {
    searchScopePath?: string | null;
    searchReplaceMode?: boolean;
    searchWorkspace: (
      query: string,
      mode: 'all' | 'files' | 'symbols' | 'text',
      opts?: {
        caseSensitive?: boolean;
        regex?: boolean;
        wholeWord?: boolean;
        fileGlob?: string;
        scopePath?: string | null;
      },
    ) => Promise<void>;
    replaceInWorkspace: (
      query: string,
      replacement: string,
      dryRun: boolean,
      opts?: {
        caseSensitive?: boolean;
        regex?: boolean;
        wholeWord?: boolean;
        fileGlob?: string;
        scopePath?: string | null;
        paths?: string[];
      },
    ) => Promise<{ files_changed: number; replacements: number; previews: Array<{ path: string; count: number; sample_before: string; sample_after: string }> } | null>;
  }

  const extSearch = (): ExtendedSearchStore => ide as unknown as ExtendedSearchStore;

  const SEARCH_TABS: readonly TabItem<SearchTab>[] = [
    { id: 'all', label: 'All', testid: 'ide-se-tab-all' },
    { id: 'files', label: 'Files', testid: 'ide-se-tab-files' },
    { id: 'symbols', label: 'Symbols', testid: 'ide-se-tab-symbols' },
    { id: 'text', label: 'Find in path', testid: 'ide-se-tab-text' },
    { id: 'actions', label: 'Actions', testid: 'ide-se-tab-actions' },
  ];

  const ROW_CAP = 120;

  let tab = $state<SearchTab>('all');
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
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let wasOpen = false;

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

  // Parse `file.go:42` or `file.go:42:8` syntax on All / Files tabs.
  const parsedFileLineQuery = $derived.by((): { cleanQuery: string; line: number | null; col: number | null } => {
    if (tab !== 'all' && tab !== 'files') return { cleanQuery: query, line: null, col: null };
    const m = /^(.+?):(\d+)(?::(\d+))?$/.exec(query.trim());
    if (!m) return { cleanQuery: query, line: null, col: null };
    return {
      cleanQuery: m[1].trim(),
      line: Number(m[2]) || null,
      col: m[3] ? Number(m[3]) || null : null,
    };
  });

  $effect(() => {
    const open = ide.searchEverywhereOpen;
    if (open && !wasOpen) {
      const s = extSearch();
      tab = ide.searchInitialTab;
      replaceOpen = Boolean(s.searchReplaceMode);
      if (s.searchScopePath) {
        customScopePath = s.searchScopePath;
        scopeMode = 'custom';
      } else {
        scopeMode = 'project';
      }
      selectedIdx = 0;
      showAllRows = false;
      ide.replacePreview = null;
      if (query.trim() && tab !== 'actions') triggerSearch(0);
    }
    wasOpen = open;
  });

  onDestroy(() => {
    if (debounceTimer) clearTimeout(debounceTimer);
  });

  const ACTIONS = $derived<IdeQuickAction[]>([
    { id: 'run', label: 'Run active configuration', shortcut: '⌃R', icon: 'play', run: () => void ide.startRun() },
    { id: 'debug', label: 'Debug active configuration', shortcut: '⌃D', icon: 'bug', run: () => void ide.startDebug() },
    { id: 'save', label: 'Save active file', shortcut: '⌘S', icon: 'download', run: () => void ide.saveFile() },
    { id: 'format', label: 'Reformat code', shortcut: '⌥⌘L', icon: 'format', run: () => void ide.formatActiveFile() },
    { id: 'format-save', label: 'Toggle format on save', icon: 'check', run: () => toggleFormatOnSave() },
    { id: 'back', label: 'Navigate back', shortcut: '⌘[', icon: 'chevronLeft', run: () => void navigateIdeBack() },
    { id: 'forward', label: 'Navigate forward', shortcut: '⌘]', icon: 'chevronRight', run: () => void navigateIdeForward() },
    { id: 'mute-bp', label: areIdeBreakpointsMuted() ? 'Unmute breakpoints' : 'Mute all breakpoints', icon: 'shield', run: () => toggleIdeBreakpointsMuted() },
    { id: 'prefix-suffix', label: 'Multi-line edit (prefix / suffix)', icon: 'edit', run: () => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen) },
    { id: 'split-lines', label: 'Split selection into multiple cursors', shortcut: '⌥⇧I', icon: 'layers', run: () => ide.splitSelectionToLines() },
    {
      id: 'rename',
      label: 'Rename symbol across workspace…',
      shortcut: '⇧F6',
      icon: 'edit',
      run: () => { if (ide.activePath) void ide.openRenameModal(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); },
    },
    {
      id: 'usages',
      label: 'Find usages at cursor',
      shortcut: '⌥F7',
      icon: 'search',
      run: () => { if (ide.activePath) void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); },
    },
    { id: 'close-unmod', label: 'Close unmodified tabs', icon: 'x', run: () => void closeUnmodifiedIdeTabs() },
    { id: 'close-all', label: 'Close all tabs', icon: 'x', run: () => void closeAllIdeTabs() },
    { id: 'toolchains', label: 'Toolchains & SDKs…', icon: 'box', run: () => (ide.toolchainsOpen = true) },
    { id: 'envs', label: 'Environments & run configurations…', icon: 'gear', run: () => (ide.environmentsOpen = true) },
    { id: 'ssh', label: 'Connect remote SSH project…', icon: 'server', run: () => (ide.sshOpen = true) },
    { id: 'share', label: 'Share live IDE session…', icon: 'share', run: () => (ide.shareOpen = true) },
    { id: 'plugins', label: 'Manage IDE plugins…', icon: 'plug', run: () => (ide.pluginsOpen = true) },
    {
      id: 'keymap',
      label: ide.keymapPreset === 'jetbrains' ? 'Switch keymap to Standard' : 'Switch keymap to JetBrains',
      icon: 'command',
      run: () => ide.setKeymapPreset(ide.keymapPreset === 'jetbrains' ? 'standard' : 'jetbrains'),
    },
  ]);

  function matchesGlobMask(filePath: string, rawMask: string): boolean {
    const mask = rawMask.trim();
    if (!mask) return true;
    const base = ideBasename(filePath);
    const patterns = mask.split(',').map((p) => p.trim()).filter(Boolean);
    if (patterns.length === 0) return true;
    let included = false;
    let hasPositive = false;
    for (const pat of patterns) {
      const neg = pat.startsWith('!');
      const clean = neg ? pat.slice(1).trim() : pat;
      if (!clean) continue;
      if (!neg) hasPositive = true;
      const escaped = clean.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
      const re = new RegExp(`^${escaped}$`, 'i');
      const target = clean.includes('/') ? ide.relativePath(ide.resolvePath(filePath)) : base;
      const ok = re.test(target);
      if (neg && ok) return false;
      if (!neg && ok) included = true;
    }
    return !hasPositive || included;
  }

  const filteredActions = $derived.by((): IdeQuickAction[] => {
    if (tab !== 'all' && tab !== 'actions') return [];
    const q = query.trim().toLowerCase();
    if (!q) return tab === 'actions' ? ACTIONS : ACTIONS.slice(0, 5);
    return ACTIONS.filter((a) => a.label.toLowerCase().includes(q) || a.id.includes(q));
  });

  const rows = $derived.by((): NavRow[] => {
    const out: NavRow[] = [];
    if (tab === 'actions') {
      for (let i = 0; i < filteredActions.length; i++) out.push({ kind: 'action', action: filteredActions[i] });
      return out;
    }
    if (tab === 'all' && !query.trim()) {
      for (let i = 0; i < filteredActions.length; i++) out.push({ kind: 'action', action: filteredActions[i] });
      return out;
    }
    let prevPath = '';
    for (let i = 0; i < ide.searchResults.length; i++) {
      const item = ide.searchResults[i];
      if (!matchesGlobMask(item.path, fileGlob)) continue;
      const groupStart = tab === 'text' && item.path !== prevPath;
      prevPath = item.path;
      out.push({ kind: 'hit', item, groupStart });
    }
    if (tab === 'all') {
      for (let i = 0; i < filteredActions.length; i++) out.push({ kind: 'action', action: filteredActions[i] });
    }
    return out;
  });

  const visibleRows = $derived(!showAllRows && rows.length > ROW_CAP ? rows.slice(0, ROW_CAP) : rows);

  function buildSearchOpts() {
    extSearch().searchScopePath = effectiveScopePath ?? null;
    return {
      caseSensitive,
      regex,
      wholeWord,
      fileGlob: fileGlob.trim() || undefined,
      scopePath: effectiveScopePath ?? null,
    };
  }

  function close(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    extSearch().searchReplaceMode = false;
    extSearch().searchScopePath = null;
    ide.searchEverywhereOpen = false;
  }

  function triggerSearch(delayMs = 120): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    const effectiveQ = parsedFileLineQuery.cleanQuery.trim();
    if (tab === 'actions' || !effectiveQ) {
      ide.searchResults = [];
      return;
    }
    const mode = tab === 'all' ? 'all' : tab;
    const run = () => void extSearch().searchWorkspace(effectiveQ, mode, buildSearchOpts());
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
    close();
    if (row.kind === 'action') {
      row.action.run();
      return;
    }
    if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
    const jumpLn = parsedFileLineQuery.line ?? row.item.line ?? undefined;
    const jumpCol = parsedFileLineQuery.col ?? row.item.column ?? undefined;
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
    if (!query.trim()) return;
    replaceMode = 'preview';
    try {
      await extSearch().replaceInWorkspace(query, replacement, true, buildSearchOpts());
    } finally {
      replaceMode = null;
    }
  }

  async function replaceSingleFile(filePath: string): Promise<void> {
    if (!query.trim()) return;
    replaceMode = 'apply';
    try {
      await extSearch().replaceInWorkspace(query, replacement, false, {
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
    if (!query.trim()) return;
    replaceMode = 'apply';
    try {
      const preview =
        ide.replacePreview ??
        (await extSearch().replaceInWorkspace(query, replacement, true, buildSearchOpts()));
      if (!preview || preview.replacements === 0) return;
      const ok = await confirmer.ask(
        `Replace ${plural(preview.replacements, 'occurrence')} across ${plural(preview.files_changed, 'file')}?`,
        { title: 'Replace in path', confirmLabel: 'Replace all', danger: true },
      );
      if (!ok) return;
      await extSearch().replaceInWorkspace(query, replacement, false, buildSearchOpts());
      close();
    } finally {
      replaceMode = null;
    }
  }

  function placeholderFor(t: SearchTab): string {
    if (t === 'files') return 'Search files by name or file.go:42…';
    if (t === 'symbols') return 'Search functions, structs, classes, types…';
    if (t === 'text') return 'Find in path across workspace…';
    if (t === 'actions') return 'Type an IDE command…';
    return 'Search files (file:line), symbols, text, or actions…';
  }
</script>

{#if ide.searchEverywhereOpen}
  <Modal
    title={replaceOpen ? 'Replace in path' : tab === 'text' ? 'Find in path' : 'Search everywhere'}
    width={720}
    dismissable={!ide.replaceLoading}
    onclose={close}
  >
    <div class="se-wrap">
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
          triggerSearch(0);
        }}
      />

      <div class="se-input-row">
        <span class="se-lead-icon"><Icon name="search" size={14} /></span>
        <input
          class="se-input"
          type="search"
          dir="ltr"
          data-autofocus
          aria-label="Search query"
          placeholder={placeholderFor(tab)}
          bind:value={query}
          oninput={onQueryInput}
          onkeydown={onKeydown}
          data-testid="ide-search-input"
        />
        {#if tab !== 'actions'}
          <button
            type="button"
            class="btn small ghost se-opt"
            aria-label="Match case"
            title="Match case"
            aria-pressed={caseSensitive}
            onclick={() => { caseSensitive = !caseSensitive; triggerSearch(0); }}
            data-testid="ide-se-case-btn"
          >Aa</button>
          <button
            type="button"
            class="btn small ghost se-opt"
            aria-label="Whole word"
            title="Match whole word"
            aria-pressed={wholeWord}
            onclick={() => { wholeWord = !wholeWord; triggerSearch(0); }}
            data-testid="ide-se-word-btn"
          >W</button>
          <button
            type="button"
            class="btn small ghost se-opt"
            aria-label="Regular expression"
            title="Use regular expression"
            aria-pressed={regex}
            onclick={() => { regex = !regex; triggerSearch(0); }}
            data-testid="ide-se-regex-btn"
          >.*</button>
          <button
            type="button"
            class="btn small ghost"
            aria-pressed={replaceOpen}
            onclick={() => {
              replaceOpen = !replaceOpen;
              if (replaceOpen && tab !== 'text') tab = 'text';
            }}
            data-testid="ide-se-replace-toggle"
          >Replace…</button>
        {/if}
      </div>

      {#if tab !== 'actions'}
        <div class="se-filter-row">
          <div class="segmented" role="group" aria-label="Directory scope">
            <button
              type="button"
              aria-pressed={scopeMode === 'project'}
              onclick={() => { scopeMode = 'project'; triggerSearch(0); }}
            >In project</button>
            {#if ide.activePath}
              <button
                type="button"
                aria-pressed={scopeMode === 'folder'}
                onclick={() => { scopeMode = 'folder'; triggerSearch(0); }}
                title={activeDirPath}
              >Current dir</button>
            {/if}
            {#if customScopePath}
              <button
                type="button"
                aria-pressed={scopeMode === 'custom'}
                onclick={() => { scopeMode = 'custom'; triggerSearch(0); }}
                title={customScopePath}
              >Directory</button>
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
      {/if}

      {#if replaceOpen && tab !== 'actions'}
        <div class="se-replace-row">
          <input
            class="se-input"
            type="text"
            dir="ltr"
            aria-label="Replacement text"
            placeholder="Replace with…"
            bind:value={replacement}
            data-testid="ide-replace-input"
          />
          <button
            type="button"
            class="btn small"
            disabled={!query.trim() || ide.replaceLoading}
            onclick={() => void previewReplace()}
          >
            {ide.replaceLoading && replaceMode === 'preview' ? 'Previewing…' : 'Preview'}
          </button>
          <button
            type="button"
            class="btn small primary"
            disabled={!query.trim() || ide.replaceLoading}
            onclick={() => void confirmAndReplaceAll()}
            data-testid="ide-replace-all-btn"
          >
            {ide.replaceLoading && replaceMode === 'apply' ? 'Replacing…' : 'Replace all'}
          </button>
        </div>

        {#if ide.replacePreview}
          <div class="se-preview-box">
            <div class="se-preview-head">
              Preview: {plural(ide.replacePreview.replacements, 'occurrence')} in {plural(ide.replacePreview.files_changed, 'file')}
            </div>
            {#each ide.replacePreview.previews.slice(0, 12) as p (p.path)}
              <div class="se-preview-item" dir="ltr">
                <div class="se-preview-top">
                  <span class="se-preview-file">{ide.relativePath(ide.resolvePath(p.path))} ({plural(p.count, 'match')})</span>
                  <button
                    type="button"
                    class="btn small ghost"
                    disabled={ide.replaceLoading}
                    onclick={() => void replaceSingleFile(p.path)}
                  >Replace file</button>
                </div>
                <div class="se-diff-before">- {p.sample_before}</div>
                <div class="se-diff-after">+ {p.sample_after}</div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}

      <div class="se-results" bind:this={resultsEl}>
        <LoadState
          what="search results"
          variant="compact"
          loading={ide.searchLoading && rows.length === 0}
          error={ide.searchError}
          empty={rows.length === 0 && Boolean(query.trim())}
          onretry={() => triggerSearch(0)}
        >
          {#snippet emptyView()}
            <EmptyState
              icon="search"
              title="No matches found"
              body="Try broadening your query, clearing the file mask, or switching to All."
            />
          {/snippet}

          <div class="se-list" role="listbox" aria-label="Search results">
            {#each visibleRows as row, idx (row.kind === 'action' ? `act:${row.action.id}` : `hit:${row.item.kind}:${row.item.path}:${row.item.line ?? 0}:${idx}`)}
              {#if row.kind === 'hit' && row.groupStart}
                <div class="se-group-head mono" dir="ltr">
                  <Icon name="file" size={12} />
                  <span>{ide.relativePath(row.item.path)}</span>
                </div>
              {/if}
              <button
                type="button"
                class="se-row"
                class:selected={idx === selectedIdx}
                role="option"
                aria-selected={idx === selectedIdx}
                onmouseenter={() => (selectedIdx = idx)}
                onclick={() => activateRow(row)}
              >
                {#if row.kind === 'action'}
                  <Icon name={row.action.icon} size={13} />
                  <span class="se-main">{row.action.label}</span>
                  <Badge variant="outline" tone="accent" label="Action" />
                  {#if row.action.shortcut}<kbd class="se-kbd">{row.action.shortcut}</kbd>{/if}
                {:else}
                  <Icon
                    name={row.item.kind === 'file' ? 'file' : row.item.kind === 'symbol' ? 'layers' : 'search'}
                    size={13}
                  />
                  <span class="se-hit-body" dir="ltr">
                    <span class="se-main mono">
                      {row.item.kind === 'file' ? ideBasename(row.item.path) : row.item.preview}
                    </span>
                    <span class="se-sub">
                      {ide.relativePath(row.item.path)}{row.item.line ? `:${row.item.line}` : ''}
                    </span>
                  </span>
                  <Badge
                    variant="outline"
                    tone={row.item.kind === 'symbol' ? 'info' : 'neutral'}
                    label={row.item.symbol_kind || row.item.kind}
                  />
                {/if}
              </button>
            {/each}
            {#if !showAllRows && rows.length > ROW_CAP}
              <button type="button" class="btn small ghost" onclick={() => (showAllRows = true)}>
                Show all {rows.length} matches ({rows.length - ROW_CAP} more)
              </button>
            {/if}
          </div>
        </LoadState>
      </div>
    </div>
  </Modal>
{/if}

<style>
  .se-wrap { display: flex; flex-direction: column; gap: var(--sp-3); min-inline-size: 0; }
  .se-input-row, .se-replace-row, .se-filter-row { display: flex; align-items: center; gap: var(--sp-3); flex-wrap: wrap; }
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
    display: flex; flex-direction: column; gap: var(--sp-3); max-block-size: 180px; overflow-y: auto;
    padding: var(--sp-4) 10px; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-xs);
  }
  .se-preview-head { font-weight: 600; color: var(--text); }
  .se-preview-item { font-family: var(--font-mono); font-size: var(--fs-xs); border-block-start: 1px solid var(--border); padding-block-start: var(--sp-2); }
  .se-preview-top { display: flex; align-items: center; justify-content: space-between; gap: var(--sp-3); }
  .se-preview-file { color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-diff-before { color: var(--danger); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-diff-after { color: var(--success); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-results { display: flex; flex-direction: column; max-block-size: 340px; overflow-y: auto; border-block-start: 1px solid var(--border); padding-block-start: var(--sp-3); }
  .se-list { display: flex; flex-direction: column; gap: var(--sp-1); }
  .se-group-head {
    display: flex; align-items: center; gap: var(--sp-2); padding: var(--sp-2) var(--sp-4);
    font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); background: var(--surface-2); border-radius: var(--radius-s);
  }
  .se-row {
    display: flex; align-items: center; gap: var(--sp-4); inline-size: 100%; padding: var(--sp-3) 10px;
    background: transparent; border: none; border-radius: var(--radius-s); color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .se-row.selected, .se-row:hover { background: var(--surface-2); }
  .se-hit-body { flex: 1; min-inline-size: 0; display: flex; flex-direction: column; gap: var(--sp-1); }
  .se-main { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-sub { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .se-kbd { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); padding: var(--sp-1) var(--sp-3); border: 1px solid var(--border); border-radius: var(--radius-s); }
</style>
