<script lang="ts">
  import Badge from '../../lib/components/Badge.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { plural } from '../../lib/plural';
  import { ide, ideBasename, type IdeSearchItem } from './ideStore.svelte';

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
    | { kind: 'hit'; item: IdeSearchItem };

  const SEARCH_TABS: readonly TabItem<SearchTab>[] = [
    { id: 'all', label: 'All', testid: 'ide-se-tab-all' },
    { id: 'files', label: 'Files', testid: 'ide-se-tab-files' },
    { id: 'symbols', label: 'Symbols', testid: 'ide-se-tab-symbols' },
    { id: 'text', label: 'Text', testid: 'ide-se-tab-text' },
    { id: 'actions', label: 'Actions', testid: 'ide-se-tab-actions' },
  ];

  let tab = $state<SearchTab>('all');
  let query = $state('');
  let replacement = $state('');
  let caseSensitive = $state(false);
  let regex = $state(false);
  let replaceOpen = $state(false);
  let replaceMode = $state<'preview' | 'apply' | null>(null);
  let selectedIdx = $state(0);
  let resultsEl = $state<HTMLDivElement | undefined>(undefined);
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let wasOpen = false;

  $effect(() => {
    const open = ide.searchEverywhereOpen;
    if (open && !wasOpen) {
      tab = ide.searchInitialTab;
      selectedIdx = 0;
      ide.replacePreview = null;
      if (query.trim() && tab !== 'actions') triggerSearch(0);
    }
    wasOpen = open;
  });

  const ACTIONS = $derived<IdeQuickAction[]>([
    { id: 'run', label: 'Run active configuration', shortcut: '⌃R', icon: 'play', run: () => void ide.startRun() },
    { id: 'debug', label: 'Debug active configuration', shortcut: '⌃D', icon: 'bug', run: () => void ide.startDebug() },
    { id: 'save', label: 'Save active file', shortcut: '⌘S', icon: 'download', run: () => void ide.saveFile() },
    { id: 'format', label: 'Reformat code', shortcut: '⌥⌘L', icon: 'format', run: () => void ide.formatActiveFile() },
    {
      id: 'prefix-suffix',
      label: 'Multi-line edit (prefix / suffix)',
      icon: 'edit',
      run: () => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen),
    },
    {
      id: 'split-lines',
      label: 'Split selection into multiple cursors',
      shortcut: '⌥⇧I',
      icon: 'layers',
      run: () => ide.splitSelectionToLines(),
    },
    {
      id: 'rename',
      label: 'Rename symbol across workspace…',
      shortcut: '⇧F6',
      icon: 'edit',
      run: () => {
        if (ide.activePath) void ide.openRenameModal(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
      },
    },
    {
      id: 'usages',
      label: 'Find usages at cursor',
      shortcut: '⌥F7',
      icon: 'search',
      run: () => {
        if (ide.activePath) void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
      },
    },
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
    for (let i = 0; i < ide.searchResults.length; i++) {
      out.push({ kind: 'hit', item: ide.searchResults[i] });
    }
    if (tab === 'all') {
      for (let i = 0; i < filteredActions.length; i++) out.push({ kind: 'action', action: filteredActions[i] });
    }
    return out;
  });

  function close(): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    ide.searchEverywhereOpen = false;
  }

  function triggerSearch(delayMs = 140): void {
    if (debounceTimer) clearTimeout(debounceTimer);
    if (tab === 'actions' || !query.trim()) {
      ide.searchResults = [];
      return;
    }
    const mode = tab === 'all' ? 'all' : tab;
    if (delayMs <= 0) {
      void ide.searchWorkspace(query, mode, { caseSensitive, regex });
      return;
    }
    debounceTimer = setTimeout(() => {
      void ide.searchWorkspace(query, mode, { caseSensitive, regex });
    }, delayMs);
  }

  function onQueryInput(): void {
    selectedIdx = 0;
    ide.replacePreview = null;
    triggerSearch(140);
  }

  function activateRow(row: NavRow | undefined): void {
    if (!row) return;
    close();
    if (row.kind === 'action') {
      row.action.run();
      return;
    }
    void ide.openFile(row.item.path, {
      line: row.item.line ?? undefined,
      col: row.item.column ?? undefined,
      preview: false,
    });
  }

  function scrollSelectedIntoView(): void {
    queueMicrotask(() => {
      resultsEl?.querySelector<HTMLElement>('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' });
    });
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (rows.length > 0) {
        selectedIdx = (selectedIdx + 1) % rows.length;
        scrollSelectedIntoView();
      }
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (rows.length > 0) {
        selectedIdx = (selectedIdx - 1 + rows.length) % rows.length;
        scrollSelectedIntoView();
      }
    } else if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      activateRow(rows[selectedIdx]);
    }
  }

  async function previewReplace(): Promise<void> {
    if (!query.trim()) return;
    replaceMode = 'preview';
    try {
      await ide.replaceInWorkspace(query, replacement, true, { caseSensitive, regex });
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
        (await ide.replaceInWorkspace(query, replacement, true, { caseSensitive, regex }));
      if (!preview || preview.replacements === 0) return;
      const ok = await confirmer.ask(
        `Replace ${plural(preview.replacements, 'occurrence')} across ${plural(preview.files_changed, 'file')}?`,
        { title: 'Replace in project', confirmLabel: 'Replace all', danger: true },
      );
      if (!ok) return;
      await ide.replaceInWorkspace(query, replacement, false, { caseSensitive, regex });
      close();
    } finally {
      replaceMode = null;
    }
  }

  function placeholderFor(t: SearchTab): string {
    if (t === 'files') return 'Search files by name…';
    if (t === 'symbols') return 'Search functions, structs, classes, types…';
    if (t === 'text') return 'Search text across project…';
    if (t === 'actions') return 'Type an IDE command…';
    return 'Search files, symbols, text, or actions…';
  }
</script>

{#if ide.searchEverywhereOpen}
  <Modal title="Search everywhere" width={680} dismissable={!ide.replaceLoading} onclose={close}>
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
            onclick={() => {
              caseSensitive = !caseSensitive;
              triggerSearch(0);
            }}
          >
            Aa
          </button>
          <button
            type="button"
            class="btn small ghost se-opt"
            aria-label="Regular expression"
            title="Use regular expression"
            aria-pressed={regex}
            onclick={() => {
              regex = !regex;
              triggerSearch(0);
            }}
          >
            .*
          </button>
          <button
            type="button"
            class="btn small ghost"
            aria-pressed={replaceOpen}
            onclick={() => {
              replaceOpen = !replaceOpen;
              if (replaceOpen && tab === 'files') tab = 'text';
            }}
          >
            Replace…
          </button>
        {/if}
      </div>

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
          >
            {ide.replaceLoading && replaceMode === 'apply' ? 'Replacing…' : 'Replace all'}
          </button>
        </div>

        {#if ide.replacePreview}
          <div class="se-preview-box">
            <div class="se-preview-head">
              Preview: {plural(ide.replacePreview.replacements, 'occurrence')} in {plural(ide.replacePreview.files_changed, 'file')}
            </div>
            {#each ide.replacePreview.previews.slice(0, 6) as p (p.path)}
              <div class="se-preview-item" dir="ltr">
                <div class="se-preview-file">{ide.relativePath(p.path)} ({plural(p.count, 'match')})</div>
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
              body="Try broadening your query or switching to All."
            />
          {/snippet}

          <div class="se-list" role="listbox" aria-label="Search results">
            {#each rows as row, idx (row.kind === 'action' ? `act:${row.action.id}` : `hit:${row.item.kind}:${row.item.path}:${row.item.line ?? 0}:${idx}`)}
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
                  {#if row.action.shortcut}
                    <kbd class="se-kbd">{row.action.shortcut}</kbd>
                  {/if}
                {:else}
                  <Icon
                    name={row.item.kind === 'file' ? 'file' : row.item.kind === 'symbol' ? 'layers' : 'search'}
                    size={13}
                  />
                  <span class="se-hit-body" dir="ltr">
                    <span class="se-main">
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
          </div>
        </LoadState>
      </div>
    </div>
  </Modal>
{/if}

<style>
  .se-wrap {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .se-input-row,
  .se-replace-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .se-lead-icon {
    display: inline-flex;
    color: var(--text-dim);
    padding-inline-start: 4px;
  }
  .se-input {
    flex: 1;
    min-width: 0;
    height: 32px;
    padding-inline: 10px;
    font-size: var(--fs-s);
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .se-opt {
    font-family: var(--font-mono);
    padding-inline: 8px;
  }
  .se-opt[aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .se-preview-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 160px;
    overflow-y: auto;
    padding: 8px 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
  }
  .se-preview-head {
    font-weight: 600;
    color: var(--text);
  }
  .se-preview-item {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    border-top: 1px solid var(--border);
    padding-top: 4px;
  }
  .se-preview-file {
    color: var(--text-dim);
  }
  .se-diff-before {
    color: var(--danger);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-diff-after {
    color: var(--success);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-results {
    display: flex;
    flex-direction: column;
    max-height: 340px;
    overflow-y: auto;
    border-top: 1px solid var(--border);
    padding-top: 6px;
  }
  .se-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .se-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    background: transparent;
    border: none;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .se-row.selected,
  .se-row:hover {
    background: var(--surface-2);
  }
  .se-hit-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .se-main {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-sub {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-kbd {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
</style>
