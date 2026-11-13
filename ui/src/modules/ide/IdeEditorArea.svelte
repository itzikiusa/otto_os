<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { plural } from '../../lib/plural';
  import { onTabKey } from '../../lib/tabKeys';
  import {
    ide,
    ideBasename,
    type IdeEditorHandle,
    type IdeOpenTab,
    type IdeSymbolNode,
  } from './ideStore.svelte';

  let primaryEditor = $state<IdeEditorHandle | undefined>(undefined);
  let splitEditor = $state<IdeEditorHandle | undefined>(undefined);

  let linePrefix = $state('');
  let lineSuffix = $state('');
  let renameMode = $state<'preview' | 'apply' | null>(null);

  async function runRenameAction(mode: 'preview' | 'apply'): Promise<void> {
    renameMode = mode;
    try {
      if (mode === 'preview') await ide.previewRename();
      else await ide.applyRename();
    } finally {
      renameMode = null;
    }
  }

  $effect(() => {
    ide.registerEditorHandle('primary', primaryEditor ?? null);
    return () => ide.registerEditorHandle('primary', null);
  });

  $effect(() => {
    ide.registerEditorHandle('split', splitEditor ?? null);
    return () => ide.registerEditorHandle('split', null);
  });

  const enclosingSymbol = $derived.by((): IdeSymbolNode | null => {
    const ln = ide.cursorInfo.line;
    let best: IdeSymbolNode | null = null;
    const walk = (nodes: IdeSymbolNode[]): void => {
      for (let i = 0; i < nodes.length; i++) {
        const s = nodes[i];
        if (ln >= s.line && ln <= Math.max(s.line, s.end_line)) {
          best = s;
          if (s.children?.length) walk(s.children);
        }
      }
    };
    walk(ide.activeSymbols);
    return best;
  });

  const pathSegments = $derived.by((): string[] => {
    if (!ide.activeTab) return [];
    return ide.relativePath(ide.activeTab.path).split('/').filter(Boolean);
  });

  function openTabMenu(e: MouseEvent, tab: IdeOpenTab): void {
    const items: MenuItem[] = [
      {
        label: 'Save',
        icon: 'download',
        hint: '⌘S',
        disabled: tab.readOnly || tab.content === tab.savedContent,
        action: () => void ide.saveFile(tab.path),
      },
      { separator: true },
      { label: 'Split right', icon: 'split', action: () => ide.openInSplit(tab.path, 'right') },
      { label: 'Split down', icon: 'unified', action: () => ide.openInSplit(tab.path, 'down') },
      { separator: true },
      {
        label: 'Close others',
        disabled: ide.tabs.length <= 1,
        action: () => ide.closeOtherTabs(tab.path),
      },
      { label: 'Close', icon: 'x', action: () => void ide.closeTab(tab.path) },
    ];
    ctxMenu.show(e, items);
  }

  async function promptBreakpointCondition(path: string, line: number): Promise<void> {
    const existing = (ide.breakpointsByPath[path] ?? []).find((b) => b.line === line);
    const cond = await confirmer.promptText(
      `Condition for ${ideBasename(path)}:${line} (leave empty for unconditional breakpoint)`,
      {
        title: 'Edit breakpoint',
        confirmLabel: 'Save',
        initial: existing?.condition ?? '',
        placeholder: 'x > 10 && user != nil',
      },
    );
    if (cond === null) return;
    ide.updateBreakpoint(path, line, { condition: cond.trim() || null, enabled: true });
  }

  async function handleDefinitionJump(
    fromPath: string,
    target: { path?: string; line: number; column: number; symbol?: string },
  ): Promise<void> {
    if (target.path) {
      await ide.openFile(target.path, { line: target.line, col: target.column, preview: false });
      return;
    }
    await ide.findReferences(fromPath, target.line, target.column, target.symbol);
  }

  async function openQuickFixAt(path: string, line: number, col: number, anchor?: MouseEvent): Promise<void> {
    const actions = await ide.fetchCodeActions(path, line, col);
    const items: MenuItem[] =
      actions.length > 0
        ? actions.map((act) => ({
            label: act.title,
            icon: 'bulb',
            action: () => ide.applyCodeAction(path, act),
          }))
        : [{ label: 'No quick fixes available on this line', disabled: true }];
    items.push(
      { separator: true },
      {
        label: 'Rename symbol…',
        icon: 'edit',
        hint: '⇧F6',
        action: () => void ide.openRenameModal(path, line, col),
      },
      {
        label: 'Reformat code',
        icon: 'format',
        hint: '⌥⌘L',
        action: () => void ide.formatActiveFile(),
      },
    );
    if (anchor) {
      ctxMenu.show(anchor, items);
      return;
    }
    const btn = document.querySelector<HTMLElement>('[data-testid="ide-quickfix-btn"]');
    if (btn) ctxMenu.showAt(btn, items, { align: 'end' });
  }

  function applyLineWrap(): void {
    if (!linePrefix && !lineSuffix) return;
    ide.applyPrefixSuffix(linePrefix, lineSuffix);
  }

  function onPrefixSuffixKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      applyLineWrap();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      ide.prefixSuffixOpen = false;
    }
  }
</script>

<section class="ide-editor-area" aria-label="Code editor" data-testid="ide-editor-area">
  {#if ide.tabs.length > 0}
    <div class="ide-tabs-row">
      <div class="ide-tabs-strip" role="toolbar" aria-label="Open files">
        {#each ide.tabs as tab (tab.path)}
          {@const isDirty = tab.content !== tab.savedContent}
          {@const isActive = tab.path === ide.activePath}
          <div
            class="ide-tab"
            class:active={isActive}
            class:preview={tab.preview}
            oncontextmenu={(e) => openTabMenu(e, tab)}
            role="presentation"
          >
            <button
              type="button"
              class="ide-tab-btn"
              aria-pressed={isActive}
              onkeydown={onTabKey}
              onclick={() => (ide.activePath = tab.path)}
              ondblclick={() => ide.pinTab(tab.path)}
              title={`${ide.relativePath(tab.path)}${isDirty ? ' (unsaved)' : ''}`}
            >
              <Icon name="file" size={12} />
              <span class="ide-tab-label" dir="ltr">{tab.name}</span>
              {#if isDirty}
                <span class="ide-dirty-dot" aria-hidden="true"></span>
              {/if}
            </button>
            <button
              type="button"
              class="icon-btn ide-tab-close"
              aria-label="Close {tab.name}"
              title="Close tab"
              onclick={() => void ide.closeTab(tab.path)}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>

      <div class="ide-tabs-end">
        {#if ide.activePath}
          <button
            type="button"
            class="icon-btn"
            aria-label="Split editor right"
            title="Split editor right"
            onclick={() => ide.activePath && ide.openInSplit(ide.activePath, 'right')}
          >
            <Icon name="split" size={13} />
          </button>
        {/if}
        {#if ide.splitPath}
          <button
            type="button"
            class="icon-btn"
            aria-label="Close split editor"
            title="Close split editor"
            onclick={() => ide.closeSplit()}
          >
            <Icon name="minimize" size={13} />
          </button>
        {/if}
      </div>
    </div>
  {/if}

  {#if ide.activeTab}
    {@const activeTab = ide.activeTab}
    <div class="ide-subbar">
      <div class="ide-crumbs" dir="ltr">
        {#each pathSegments as seg, idx (`${seg}:${idx}`)}
          {#if idx > 0}
            <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
          {/if}
          <span class="crumb-seg" class:last={idx === pathSegments.length - 1}>{seg}</span>
        {/each}
        {#if enclosingSymbol}
          <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
          <Badge variant="outline" tone="info" label={enclosingSymbol.name} />
        {/if}
        {#if activeTab.readOnly}
          <Badge variant="outline" tone="neutral" label="Read-only" />
        {/if}
      </div>

      <div class="ide-subbar-tools">
        <span class="ide-cursor-meta" dir="ltr">
          Ln {ide.cursorInfo.line}, Col {ide.cursorInfo.col}
        </span>
        {#if ide.cursorInfo.cursors > 1}
          <Badge variant="outline" tone="accent" label={plural(ide.cursorInfo.cursors, 'cursor')} />
        {/if}
        <button
          type="button"
          class="btn small ghost"
          aria-pressed={ide.prefixSuffixOpen}
          title="Add prefix or suffix to every selected line"
          onclick={() => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen)}
          data-testid="ide-prefix-suffix-toggle"
        >
          <Icon name="edit" size={12} />
          Prefix / suffix
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Split selection into lines"
          title="Split selection into multiple cursors (⌥⇧I)"
          onclick={() => ide.splitSelectionToLines()}
        >
          <Icon name="layers" size={13} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Quick fixes"
          title="Show quick fixes and refactorings (⌥⏎)"
          onclick={(e) => void openQuickFixAt(activeTab.path, ide.cursorInfo.line, ide.cursorInfo.col, e)}
          data-testid="ide-quickfix-btn"
        >
          <Icon name="bulb" size={13} />
        </button>
        <button
          type="button"
          class="icon-btn"
          aria-label="Reformat file"
          title="Reformat code (⌥⌘L)"
          disabled={activeTab.readOnly}
          onclick={() => void ide.formatActiveFile()}
        >
          <Icon name="format" size={13} />
        </button>
      </div>
    </div>

    {#if ide.prefixSuffixOpen}
      <div class="ide-ps-bar" role="region" aria-label="Multi-line prefix and suffix editor">
        <input
          class="ide-ps-input"
          type="text"
          dir="ltr"
          aria-label="Before each line"
          placeholder="Before each line…"
          bind:value={linePrefix}
          onkeydown={onPrefixSuffixKey}
          data-testid="ide-prefix-input"
        />
        <input
          class="ide-ps-input"
          type="text"
          dir="ltr"
          aria-label="After each line"
          placeholder="After each line…"
          bind:value={lineSuffix}
          onkeydown={onPrefixSuffixKey}
          data-testid="ide-suffix-input"
        />
        <button
          type="button"
          class="btn small"
          disabled={(!linePrefix && !lineSuffix) || activeTab.readOnly}
          onclick={applyLineWrap}
          data-testid="ide-apply-prefix-suffix"
        >
          Apply to lines
        </button>
        <span class="ide-ps-tip">Tip: Hold ⌥ and drag vertically for column selection</span>
        <button
          type="button"
          class="icon-btn"
          aria-label="Close prefix and suffix bar"
          title="Close"
          onclick={() => (ide.prefixSuffixOpen = false)}
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    {/if}

    <div class="ide-panes" class:split-down={ide.splitDirection === 'down'}>
      <div class="ide-pane">
        <LoadState
          what={activeTab.name}
          variant="panel"
          loading={activeTab.loading}
          error={activeTab.error}
          empty={false}
          onretry={() => void ide.openFile(activeTab.path)}
        >
          <CodeEditor
            bind:this={primaryEditor}
            path={activeTab.path}
            content={activeTab.content}
            root={ide.target.root}
            language={activeTab.language}
            readOnly={activeTab.readOnly || !ide.canEditCollab}
            keepStates={true}
            lsp={!ide.collabIsGuest}
            ideMode={true}
            connectionId={ide.target.kind === 'ssh' ? ide.target.connection_id : null}
            breakpoints={ide.breakpointsByPath[activeTab.path] ?? []}
            onToggleBreakpoint={(ln) => ide.toggleBreakpoint(activeTab.path, ln)}
            onEditBreakpoint={(ln) => void promptBreakpointCondition(activeTab.path, ln)}
            gutterItems={ide.gutterByPath[activeTab.path] ?? []}
            onGutterAction={(item, mode) => ide.runFromGutter(item, mode)}
            pausedLine={ide.activeDebug?.state === 'paused' && ide.activeDebug.paused_file === activeTab.path
              ? (ide.activeDebug.paused_line ?? null)
              : null}
            inlineValues={ide.activeDebug?.state === 'paused' && ide.activeDebug.paused_file === activeTab.path
              ? ide.activeDebug.inline_values
              : []}
            peerCursors={ide.peerCursors.filter((p) => p.path === activeTab.path)}
            keymapPreset={ide.keymapPreset}
            jumpToLine={ide.jumpRequest?.path === activeTab.path
              ? { line: ide.jumpRequest.line, column: ide.jumpRequest.col, seq: ide.jumpRequest.seq }
              : null}
            onchange={(val) => ide.updateBuffer(activeTab.path, val)}
            onCursorChange={(info) =>
              ide.updateCursor(activeTab.path, {
                line: info.line,
                col: info.column,
                cursors: info.cursorCount,
                selections: info.selections,
              })}
            onDefinitionJump={(t) => void handleDefinitionJump(activeTab.path, t)}
            onQuickFix={(ctx) => void openQuickFixAt(activeTab.path, ctx.line, ctx.column)}
            onRenameSymbol={(ctx) =>
              void ide.openRenameModal(activeTab.path, ctx.line, ctx.column, ctx.symbol)}
            onFormat={() => void ide.formatActiveFile()}
            onDiagnostics={(diags) => ide.setDiagnostics(activeTab.path, diags)}
          />
        </LoadState>
      </div>

      {#if ide.splitTab}
        {@const splitTab = ide.splitTab}
        <div class="ide-pane split-secondary">
          <div class="split-head" dir="ltr">
            <span class="split-title">{ide.relativePath(splitTab.path)}</span>
            <button
              type="button"
              class="icon-btn"
              aria-label="Close split pane"
              title="Close split pane"
              onclick={() => ide.closeSplit()}
            >
              <Icon name="x" size={12} />
            </button>
          </div>
          <div class="split-body">
            <CodeEditor
              bind:this={splitEditor}
              path={splitTab.path}
              content={splitTab.content}
              root={ide.target.root}
              language={splitTab.language}
              readOnly={splitTab.readOnly || !ide.canEditCollab}
              findOwner={false}
              keepStates={true}
              lsp={!ide.collabIsGuest}
              ideMode={true}
              connectionId={ide.target.kind === 'ssh' ? ide.target.connection_id : null}
              breakpoints={ide.breakpointsByPath[splitTab.path] ?? []}
              onToggleBreakpoint={(ln) => ide.toggleBreakpoint(splitTab.path, ln)}
              onEditBreakpoint={(ln) => void promptBreakpointCondition(splitTab.path, ln)}
              gutterItems={ide.gutterByPath[splitTab.path] ?? []}
              onGutterAction={(item, mode) => ide.runFromGutter(item, mode)}
              keymapPreset={ide.keymapPreset}
              onchange={(val) => ide.updateBuffer(splitTab.path, val)}
            />
          </div>
        </div>
      {/if}
    </div>
  {:else}
    <div class="ide-empty-editor">
      <EmptyState
        headingLevel={2}
        icon="ide"
        title="Select a file to start coding"
        body="Choose a file in the explorer, press ⇧⇧ to search everywhere, or hold ⌥ and drag in any file for multi-cursor editing"
        actionLabel="Search everywhere"
        actionIcon="search"
        actionKind="secondary"
        onaction={() => {
          ide.searchInitialTab = 'all';
          ide.searchEverywhereOpen = true;
        }}
      />
    </div>
  {/if}
</section>

{#if ide.renameState.open}
  <Modal
    title="Rename symbol across workspace"
    width={580}
    dismissable={!ide.renameState.loading}
    onclose={() => (ide.renameState.open = false)}
  >
    <div class="rename-modal">
      <div class="rename-row">
        <input
          class="rename-input"
          type="text"
          dir="ltr"
          data-autofocus
          aria-label="New symbol name"
          placeholder="New identifier name…"
          bind:value={ide.renameState.newName}
          onkeydown={(e) => {
            if (e.key === 'Enter' && !ide.renameState.loading) {
              e.preventDefault();
              void runRenameAction('apply');
            }
          }}
        />
        <button
          type="button"
          class="btn small"
          disabled={!ide.renameState.newName.trim() || ide.renameState.loading}
          onclick={() => void runRenameAction('preview')}
        >
          {ide.renameState.loading && renameMode === 'preview' ? 'Previewing…' : 'Preview'}
        </button>
      </div>

      {#if ide.renameState.error}
        <p class="rename-err" use:announceShown={{ text: ide.renameState.error, assertive: true }}>
          {ide.renameState.error}
        </p>
      {/if}

      {#if ide.renameState.edits.length > 0}
        <div class="rename-preview">
          <div class="rename-summary">
            Updates across {plural(ide.renameState.edits.length, 'file')}
          </div>
          {#each ide.renameState.edits as edit (edit.path)}
            <div class="rename-file" dir="ltr">
              <div class="rename-file-head">
                {ide.relativePath(edit.path)} ({plural(edit.occurrences, 'occurrence')})
              </div>
              {#each edit.preview_lines.slice(0, 3) as pl (`${edit.path}:${pl.line}`)}
                <div class="rename-before">- L{pl.line}: {pl.before}</div>
                <div class="rename-after">+ L{pl.line}: {pl.after}</div>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </div>

    {#snippet footer()}
      <button
        type="button"
        class="btn"
        disabled={ide.renameState.loading}
        onclick={() => (ide.renameState.open = false)}
      >
        Cancel
      </button>
      <button
        type="button"
        class="btn primary"
        disabled={!ide.renameState.newName.trim() || ide.renameState.loading}
        onclick={() => void runRenameAction('apply')}
      >
        {ide.renameState.loading && renameMode === 'apply' ? 'Renaming…' : 'Rename'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ide-editor-area {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--bg);
  }
  .ide-tabs-row { display: flex; align-items: center; background: var(--surface); border-bottom: 1px solid var(--border); min-width: 0; }
  .ide-tabs-strip { flex: 1; min-width: 0; display: flex; align-items: stretch; overflow-x: auto; scrollbar-width: none; }
  .ide-tabs-strip::-webkit-scrollbar { display: none; }
  .ide-tab {
    display: inline-flex; align-items: center; padding-inline-end: 4px;
    border-inline-end: 1px solid var(--border); background: var(--surface); flex-shrink: 0; max-width: 220px;
  }
  .ide-tab.active { background: var(--bg); box-shadow: inset 0 -2px 0 var(--accent-solid); }
  .ide-tab.preview .ide-tab-label { font-style: italic; }
  .ide-tab-btn {
    display: inline-flex; align-items: center; gap: 6px; padding-block: 6px; padding-inline: 10px 6px;
    background: transparent; border: none; color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; min-width: 0;
  }
  .ide-tab.active .ide-tab-btn { color: var(--text); font-weight: 500; }
  .ide-tab-label, .crumb-seg { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ide-dirty-dot { inline-size: 6px; block-size: 6px; border-radius: 50%; background: var(--accent-solid); flex-shrink: 0; }
  .ide-tab-close { opacity: var(--disabled-opacity); }
  .ide-tab:hover .ide-tab-close, .ide-tab.active .ide-tab-close { opacity: 1; }
  .ide-tabs-end { display: flex; align-items: center; gap: 2px; padding-inline: 6px; flex-shrink: 0; }
  .ide-subbar {
    display: flex; align-items: center; justify-content: space-between; gap: 8px;
    padding: 4px 10px; background: var(--surface); border-bottom: 1px solid var(--border); min-width: 0; flex-wrap: wrap;
  }
  .ide-crumbs { display: flex; align-items: center; gap: 4px; min-width: 0; overflow: hidden; font-size: var(--fs-xs); color: var(--text-dim); }
  .crumb-sep { display: inline-flex; opacity: var(--disabled-opacity); }
  .crumb-seg.last { color: var(--text); font-weight: 500; }
  .ide-subbar-tools { display: flex; align-items: center; gap: 4px; flex-shrink: 0; }
  .ide-cursor-meta { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); padding-inline-end: 4px; }
  .ide-ps-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .ide-ps-input {
    width: 170px;
    max-width: 100%;
    height: 26px;
    padding-inline: 8px;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .ide-ps-tip { font-size: var(--fs-xs); color: var(--text-dim); margin-inline-start: auto; }
  .ide-panes { flex: 1; min-height: 0; min-width: 0; display: flex; }
  .ide-panes.split-down { flex-direction: column; }
  .ide-pane { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; }
  .split-secondary { border-inline-start: 1px solid var(--border); }
  .ide-panes.split-down .split-secondary { border-inline-start: none; border-top: 1px solid var(--border); }
  .split-head {
    display: flex; align-items: center; justify-content: space-between; padding: 4px 8px;
    background: var(--surface); border-bottom: 1px solid var(--border); font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim);
  }
  .split-title, .rename-before, .rename-after { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .split-body { flex: 1; min-height: 0; }
  .ide-empty-editor { flex: 1; display: flex; align-items: center; justify-content: center; min-height: 0; }
  .rename-modal { display: flex; flex-direction: column; gap: 10px; }
  .rename-row { display: flex; gap: 6px; }
  .rename-input {
    flex: 1; height: 32px; padding-inline: 10px; font-family: var(--font-mono); font-size: var(--fs-s);
    background: var(--surface-2); color: var(--text); border: 1px solid var(--border); border-radius: var(--radius-s);
  }
  .rename-err { margin: 0; color: var(--danger); font-size: var(--fs-xs); }
  .rename-preview {
    display: flex; flex-direction: column; gap: 6px; max-height: 240px; overflow-y: auto;
    padding: 8px 10px; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-s); font-size: var(--fs-xs);
  }
  .rename-summary { font-weight: 600; }
  .rename-file { font-family: var(--font-mono); font-size: var(--fs-xs); border-top: 1px solid var(--border); padding-top: 4px; }
  .rename-file-head { color: var(--text-dim); }
  .rename-before { color: var(--danger); }
  .rename-after { color: var(--success); }
  @media (max-width: 640px) {
    .ide-ps-tip {
      display: none;
    }
    .ide-panes {
      flex-direction: column;
    }
    .split-secondary {
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
  }
</style>
