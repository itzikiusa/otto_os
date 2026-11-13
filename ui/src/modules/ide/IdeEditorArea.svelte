<script lang="ts">
  import { onDestroy } from 'svelte';
  import { copyWithToast } from '../../lib/clipboard';
  import Badge from '../../lib/components/Badge.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { plural } from '../../lib/plural';
  import { toasts } from '../../lib/toast.svelte';
  import { ide, type IdeEditorHandle, type IdeOpenTab, type IdeSymbolNode } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';
  import { subscribeIdeBufferEvictions, subscribeIdeFileSaved } from './store/ideBufferPerf';
  import IdeTabBar from './editor/IdeTabBar.svelte';
  import {
    areIdeBreakpointsMuted,
    classifyIdeFile,
    fetchIdeGitBlame,
    ideEditorPrefs,
    navigateIdeBack,
    navigateIdeForward,
    performIdeDive,
    pushIdeNavLocation,
    resetIdeEditorBridgeState,
    revealInIdeProjectTree,
    toggleFormatOnSave,
  } from './editor/ideEditorStoreBridge.svelte';
  import {
    computeJsonPathAt,
    detectIdeDocKind,
    extractInstantSymbols,
    transformJsonText,
    type IdeOutlineNode,
  } from './editor/ideLangAnalyzer';

  const IdeInlinePromptBarLazy = lazyComponent(() => import('./IdeInlinePromptBar.svelte'));
  const IdeCsvViewLazy = lazyComponent(() => import('./editor/IdeCsvView.svelte'));
  const IdeBreakpointPopoverLazy = lazyComponent(() => import('./editor/IdeBreakpointPopover.svelte'));
  const IdeFileStructureModalLazy = lazyComponent(() => import('./editor/IdeFileStructureModal.svelte'));
  const IdeRenameModalLazy = lazyComponent(() => import('./editor/IdeRenameModal.svelte'));

  type ExtendedEditorRef = IdeEditorHandle & {
    evictPaths?: (paths: Iterable<string> | 'all') => void;
    notifyDidSave?: () => void;
    disposeLspClients?: (rootPath?: string) => void;
  };

  let primaryEditorRef = $state<ExtendedEditorRef | undefined>(undefined);
  let splitEditorRef = $state<ExtendedEditorRef | undefined>(undefined);
  let everMountedEditor = false;
  let linePrefix = $state('');
  let lineSuffix = $state('');
  let structureOpen = $state(false);
  let bpEditor = $state<{ path: string; line: number; x: number; y: number } | null>(null);
  let tableModeByPath = $state<Record<string, 'table' | 'text'>>({});
  let blameEnabledByPath = $state<Record<string, boolean>>({});
  let blameByPath = $state<Record<string, Record<number, string> | null>>({});

  // Stage ready tabs so `<CodeEditor>` NEVER mounts or swaps onto `content=""` while `loading: true`.
  let stagedPrimaryTab = $state<IdeOpenTab | null>(null);
  let stagedSplitTab = $state<IdeOpenTab | null>(null);

  $effect(() => {
    const cur = ide.activeTab;
    if (!cur) { stagedPrimaryTab = null; return; }
    if (!cur.loading && !cur.error) { stagedPrimaryTab = cur; everMountedEditor = true; }
    else if (stagedPrimaryTab && !ide.tabs.some((t) => t.path === stagedPrimaryTab?.path)) stagedPrimaryTab = null;
  });

  $effect(() => {
    const cur = ide.splitTab;
    if (!cur) { stagedSplitTab = null; return; }
    if (!cur.loading && !cur.error) { stagedSplitTab = cur; everMountedEditor = true; }
    else if (stagedSplitTab && !ide.tabs.some((t) => t.path === stagedSplitTab?.path)) stagedSplitTab = null;
  });

  function disposeAllPooledLsp(): void {
    primaryEditorRef?.disposeLspClients?.();
    splitEditorRef?.disposeLspClients?.();
    if (everMountedEditor) {
      everMountedEditor = false;
      void import('../../lib/components/cm-ide-lsp')
        .then((m) => m.disposePooledLspClients({ forceAll: true }))
        .catch(() => {});
    }
  }

  const unsubEvict = subscribeIdeBufferEvictions((paths) => {
    primaryEditorRef?.evictPaths?.(paths);
    splitEditorRef?.evictPaths?.(paths);
    if (paths === 'all') {
      blameEnabledByPath = {}; blameByPath = {}; tableModeByPath = {}; bpEditor = null;
      resetIdeEditorBridgeState();
      disposeAllPooledLsp();
      return;
    }
    let blameChanged = false;
    let modeChanged = false;
    const nextBlame = { ...blameByPath };
    const nextBlameOn = { ...blameEnabledByPath };
    const nextMode = { ...tableModeByPath };
    for (const p of paths) {
      if (p in nextBlame || p in nextBlameOn) { delete nextBlame[p]; delete nextBlameOn[p]; blameChanged = true; }
      if (p in nextMode) { delete nextMode[p]; modeChanged = true; }
      if (bpEditor?.path === p) bpEditor = null;
    }
    if (blameChanged) { blameByPath = nextBlame; blameEnabledByPath = nextBlameOn; }
    if (modeChanged) tableModeByPath = nextMode;
    if (ide.tabs.length === 0) disposeAllPooledLsp();
  });

  const unsubSaved = subscribeIdeFileSaved((savedPath) => {
    if (savedPath === stagedPrimaryTab?.path) primaryEditorRef?.notifyDidSave?.();
    if (savedPath === stagedSplitTab?.path) splitEditorRef?.notifyDidSave?.();
  });

  onDestroy(() => {
    unsubEvict();
    unsubSaved();
    disposeAllPooledLsp();
  });

  const hasNonEmptySelection = $derived(ide.cursorInfo.selections.some((s) => s.anchor !== s.head));
  const activeDocKind = $derived(ide.activeTab ? detectIdeDocKind(ide.activeTab.path, ide.activeTab.language) : 'other');
  const activeFileMeta = $derived(
    ide.activeTab
      ? classifyIdeFile(
          ide.activeTab,
          ide.target.root,
          (ide as { additionalRoots?: string[] }).additionalRoots ??
            (ide.target as { additional_roots?: string[] }).additional_roots ??
            [],
        )
      : { external: false, sdkLabel: null },
  );
  const effectiveViewMode = $derived.by((): 'table' | 'text' => {
    const t = ide.activeTab;
    if (!t) return 'text';
    return tableModeByPath[t.path] ?? (activeDocKind === 'csv' ? 'table' : 'text');
  });
  const effectiveSymbols = $derived.by((): IdeSymbolNode[] => {
    const t = ide.activeTab;
    if (!t) return [];
    return ide.activeSymbols.length > 0 ? ide.activeSymbols : extractInstantSymbols(t.path, t.language, t.content);
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
    walk(effectiveSymbols);
    return best;
  });
  const activeJsonPath = $derived.by((): string | null => {
    if (activeDocKind !== 'json' || !ide.activeTab) return null;
    return computeJsonPathAt(ide.activeTab.content, ide.cursorInfo.line, ide.cursorInfo.col);
  });
  const pathSegments = $derived.by((): string[] => {
    if (!ide.activeTab) return [];
    return ide.relativePath(ide.activeTab.path).split('/').filter(Boolean);
  });

  function toEditorHandle(ref: IdeEditorHandle): IdeEditorHandle {
    return {
      applyPrefixSuffix: (prefix, suffix) => ref.applyPrefixSuffix(prefix, suffix),
      splitSelectionToLines: () => ref.splitSelectionToLines(),
      goToLine: (line, col) => ref.goToLine(line, col),
      focus: () => ref.focus(),
      getSelectionRange: () => ref.getSelectionRange?.() ?? null,
      applyAgentEdit: (nextContent, range) => ref.applyAgentEdit?.(nextContent, range) ?? false,
    };
  }

  $effect(() => {
    ide.registerEditorHandle('primary', primaryEditorRef ? toEditorHandle(primaryEditorRef) : null);
    return () => ide.registerEditorHandle('primary', null);
  });
  $effect(() => {
    ide.registerEditorHandle('split', splitEditorRef ? toEditorHandle(splitEditorRef) : null);
    return () => ide.registerEditorHandle('split', null);
  });

  function focusAgentComposer(): void {
    let tries = 0;
    const step = (): void => {
      const el = document.querySelector<HTMLElement>('[data-testid="ide-agent-composer"]');
      if (el) el.focus();
      else if (++tries < 12) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  async function saveWithOptionalFormat(targetPath: string): Promise<void> {
    const storeHandlesFormat = (ide as { formatOnSave?: boolean }).formatOnSave !== undefined;
    if (!storeHandlesFormat && ideEditorPrefs.formatOnSave && targetPath === ide.activePath) {
      try { await ide.formatActiveFile(); } catch { /* ignore unsupported formatter */ }
    }
    await ide.saveFile(targetPath);
  }

  function toggleBlameFor(filePath: string): void {
    const next = !blameEnabledByPath[filePath];
    blameEnabledByPath = { ...blameEnabledByPath, [filePath]: next };
    if (!next) {
      const copy = { ...blameByPath };
      delete copy[filePath];
      blameByPath = copy;
      return;
    }
    void fetchIdeGitBlame(ide.target.root, filePath).then((res) => {
      if (!blameEnabledByPath[filePath]) return;
      if (!res) toasts.info('No git blame available for this file');
      blameByPath = { ...blameByPath, [filePath]: res };
    });
  }

  function jumpAdjacentSymbol(delta: -1 | 1): void {
    const flat: IdeSymbolNode[] = [];
    const walk = (nodes: IdeSymbolNode[]) => {
      for (const n of nodes) { flat.push(n); if (n.children?.length) walk(n.children); }
    };
    walk(effectiveSymbols);
    if (flat.length === 0) return;
    flat.sort((a, b) => a.line - b.line);
    const cur = ide.cursorInfo.line;
    const target = delta > 0
      ? (flat.find((s) => s.line > cur) ?? flat[0])
      : ([...flat].reverse().find((s) => s.line < cur) ?? flat[flat.length - 1]);
    if (target && ide.activePath) {
      pushIdeNavLocation(ide.activePath, cur, ide.cursorInfo.col);
      primaryEditorRef?.goToLine(target.line, (target as IdeOutlineNode).column || 1);
    }
  }

  function applyJsonTransform(mode: 'pretty' | 'minify' | 'sort'): void {
    const t = ide.activeTab;
    if (!t || t.readOnly) return;
    const res = transformJsonText(t.content, mode, activeDocKind === 'jsonl');
    if (!res.ok) { toasts.error('Invalid JSON', res.error); return; }
    if (!primaryEditorRef?.applyAgentEdit?.(res.text)) ide.updateBuffer(t.path, res.text);
  }

  function openBreakpointPopover(filePath: string, line: number, ev?: MouseEvent): void {
    if (!(ide.breakpointsByPath[filePath] ?? []).some((b) => b.line === line)) {
      ide.toggleBreakpoint(filePath, line);
    }
    bpEditor = { path: filePath, line, x: ev?.clientX ?? 240, y: ev?.clientY ?? 180 };
  }

  function openSymbolBreadcrumbMenu(e: MouseEvent): void {
    const anchor = e.currentTarget as HTMLElement | null;
    if (effectiveSymbols.length === 0) { structureOpen = true; return; }
    const items: MenuItem[] = effectiveSymbols.slice(0, 50).map((s) => ({
      label: s.name,
      icon: s.line === enclosingSymbol?.line ? 'check' : 'function',
      hint: `L${s.line}`,
      action: () => {
        if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
        primaryEditorRef?.goToLine(s.line, (s as IdeOutlineNode).column || 1);
      },
    }));
    items.push({ separator: true }, { label: 'Search file structure…', icon: 'search', hint: '⌘F12', action: () => (structureOpen = true) });
    ctxMenu.showAt(anchor, items, { align: 'start' });
  }

  async function openQuickFixAt(path: string, line: number, col: number, anchor?: MouseEvent): Promise<void> {
    const actions = await ide.fetchCodeActions(path, line, col);
    const items: MenuItem[] = actions.length > 0
      ? actions.map((act) => ({ label: act.title, icon: 'bulb' as const, action: () => ide.applyCodeAction(path, act) }))
      : [{ label: 'No quick fixes available on this line', disabled: true }];
    items.push(
      { separator: true },
      { label: 'Rename symbol…', icon: 'edit', hint: '⇧F6', action: () => void ide.openRenameModal(path, line, col) },
      { label: 'Find usages', icon: 'search', hint: '⌥F7', action: () => void ide.findReferences(path, line, col) },
      { label: 'Reformat code', icon: 'format', hint: '⌥⌘L', action: () => void ide.formatActiveFile() },
      {
        label: ideEditorPrefs.formatOnSave ? 'Format on save: On' : 'Format on save: Off',
        icon: ideEditorPrefs.formatOnSave ? 'check' : 'dot',
        action: () => toggleFormatOnSave(),
      },
    );
    if (anchor) { ctxMenu.show(anchor, items); return; }
    const btn = document.querySelector<HTMLElement>('[data-testid="ide-quickfix-btn"]');
    if (btn) ctxMenu.showAt(btn, items, { align: 'end' });
  }

  function applyLineWrap(): void {
    if (!linePrefix && !lineSuffix) return;
    ide.applyPrefixSuffix(linePrefix, lineSuffix);
  }
</script>

<section class="ide-editor-area" aria-label="Code editor" data-testid="ide-editor-area">
  {#if ide.tabs.length > 0}
    <IdeTabBar onsave={(p) => void saveWithOptionalFormat(p)} />
  {/if}

  {#if ide.activeTab}
    {@const activeTab = ide.activeTab}
    {@const blameActive = Boolean(blameEnabledByPath[activeTab.path])}
    <div class="ide-subbar">
      <div class="ide-crumbs" dir="ltr">
        {#each pathSegments as seg, idx (`${seg}:${idx}`)}
          {#if idx > 0}<span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>{/if}
          <button type="button" class="crumb-btn" class:last={idx === pathSegments.length - 1} onclick={() => void revealInIdeProjectTree(activeTab.path)} title="Reveal in project tree">{seg}</button>
        {/each}
        {#if enclosingSymbol}
          <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
          <button type="button" class="crumb-sym-btn" onclick={openSymbolBreadcrumbMenu} title="Jump to symbol in file (⌘F12)">
            <Icon name="function" size={12} /><span>{enclosingSymbol.name}</span>
          </button>
        {/if}
        {#if activeJsonPath && activeJsonPath !== '$'}
          <span class="crumb-sep" aria-hidden="true"><Icon name="chevronRight" size={12} noflip /></span>
          <button type="button" class="crumb-sym-btn mono" onclick={() => void copyWithToast(activeJsonPath, 'JSONPath')} title="Click to copy JSONPath" data-testid="ide-jsonpath-crumb">{activeJsonPath}</button>
        {/if}
        {#if activeFileMeta.sdkLabel}<Badge variant="outline" tone="info" label={activeFileMeta.sdkLabel} />{/if}
        {#if activeTab.readOnly}<Badge variant="outline" tone="neutral" label="Read-only" />{/if}
      </div>

      <div class="ide-subbar-tools">
        {#if activeDocKind === 'csv' || activeDocKind === 'jsonl'}
          <div class="segmented" role="group" aria-label="Editor view mode" data-testid="ide-csv-mode-toggle">
            <button type="button" aria-pressed={effectiveViewMode === 'table'} onclick={() => (tableModeByPath = { ...tableModeByPath, [activeTab.path]: 'table' })}>Table</button>
            <button type="button" aria-pressed={effectiveViewMode === 'text'} onclick={() => (tableModeByPath = { ...tableModeByPath, [activeTab.path]: 'text' })}>Text</button>
          </div>
        {/if}
        {#if (activeDocKind === 'json' || activeDocKind === 'jsonl') && !activeTab.readOnly && effectiveViewMode === 'text'}
          <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('pretty')} title="Pretty-print JSON">Pretty</button>
          <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('minify')} title="Minify JSON">Minify</button>
          <button type="button" class="btn small ghost" onclick={() => applyJsonTransform('sort')} title="Sort JSON keys alphabetically">Sort keys</button>
        {/if}

        <span class="ide-cursor-meta" dir="ltr">Ln {ide.cursorInfo.line}, Col {ide.cursorInfo.col}</span>
        {#if ide.cursorInfo.cursors > 1}<Badge variant="outline" tone="accent" label={plural(ide.cursorInfo.cursors, 'cursor')} />{/if}

        {#if hasNonEmptySelection && effectiveViewMode === 'text'}
          <div class="ide-sel-bar" role="group" aria-label="Selection actions" data-testid="ide-selection-agent-bar">
            <button type="button" class="btn small ghost ide-sel-accent" title="Add selection to AI Agent context (⌘L)" onclick={() => { ideAgent.markSelectionFromEditor({ openPanel: true }); focusAgentComposer(); }} data-testid="ide-selection-add-agent">
              <Icon name="assistant" size={12} />Add to Agent ⌘L
            </button>
            <button type="button" class="btn small ghost" disabled={activeTab.readOnly || !ide.canEditCollab} title="Inline AI edit on selection (⌘I)" onclick={() => ideAgent.openInlineEdit()} data-testid="ide-selection-inline-edit">
              <Icon name="sparkle" size={12} />Edit ⌘I
            </button>
            <button type="button" class="btn small ghost" aria-pressed={ide.prefixSuffixOpen} title="Add prefix or suffix to every selected line" onclick={() => (ide.prefixSuffixOpen = true)} data-testid="ide-prefix-suffix-toggle">
              <Icon name="edit" size={12} />Prefix / suffix
            </button>
            <button type="button" class="icon-btn" aria-label="Split selection into lines" title="Split selection into multiple cursors (⌥⇧I)" onclick={() => ide.splitSelectionToLines()}>
              <Icon name="layers" size={13} />
            </button>
          </div>
        {/if}

        <button type="button" class="icon-btn" aria-label="Previous function or symbol" title="Previous function (⌥↑)" disabled={effectiveSymbols.length === 0} onclick={() => jumpAdjacentSymbol(-1)}>
          <Icon name="arrowUp" size={13} />
        </button>
        <button type="button" class="icon-btn" aria-label="Next function or symbol" title="Next function (⌥↓)" disabled={effectiveSymbols.length === 0} onclick={() => jumpAdjacentSymbol(1)}>
          <Icon name="arrowDown" size={13} />
        </button>
        <button type="button" class="icon-btn" aria-label="File structure" title="File structure (⌘F12)" onclick={() => (structureOpen = true)} data-testid="ide-structure-btn">
          <Icon name="layers" size={13} />
        </button>
        <button type="button" class="icon-btn" aria-label="Toggle git blame annotations" title="Toggle git blame annotations" aria-pressed={blameActive} onclick={() => toggleBlameFor(activeTab.path)} data-testid="ide-blame-btn">
          <Icon name="commit" size={13} />
        </button>
        <button type="button" class="icon-btn" aria-label="Quick fixes" title="Show quick fixes and refactorings (⌥⏎)" onclick={(e) => void openQuickFixAt(activeTab.path, ide.cursorInfo.line, ide.cursorInfo.col, e)} data-testid="ide-quickfix-btn">
          <Icon name="bulb" size={13} />
        </button>
        <button type="button" class="icon-btn" aria-label="Reformat file" title="Reformat code (⌥⌘L)" disabled={activeTab.readOnly} onclick={() => void ide.formatActiveFile()}>
          <Icon name="format" size={13} />
        </button>
      </div>
    </div>

    {#if ideAgent.inlinePrompt?.open}
      <LazyMount lazy={IdeInlinePromptBarLazy} what="inline prompt" variant="compact" />
    {/if}

    {#if ide.prefixSuffixOpen}
      <div class="ide-ps-bar" role="region" aria-label="Multi-line prefix and suffix editor">
        <input class="ide-ps-input" type="text" dir="ltr" aria-label="Before each line" placeholder="Before each line…" bind:value={linePrefix} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); applyLineWrap(); } else if (e.key === 'Escape') { e.preventDefault(); ide.prefixSuffixOpen = false; } }} data-testid="ide-prefix-input" />
        <input class="ide-ps-input" type="text" dir="ltr" aria-label="After each line" placeholder="After each line…" bind:value={lineSuffix} onkeydown={(e) => { if (e.key === 'Enter') { e.preventDefault(); applyLineWrap(); } else if (e.key === 'Escape') { e.preventDefault(); ide.prefixSuffixOpen = false; } }} data-testid="ide-suffix-input" />
        <button type="button" class="btn small" disabled={(!linePrefix && !lineSuffix) || activeTab.readOnly} onclick={applyLineWrap} data-testid="ide-apply-prefix-suffix">Apply to lines</button>
        <span class="ide-ps-tip">Tip: Hold ⌥ and drag vertically for column selection</span>
        <button type="button" class="icon-btn" aria-label="Close prefix and suffix bar" title="Close" onclick={() => (ide.prefixSuffixOpen = false)}><Icon name="x" size={12} /></button>
      </div>
    {/if}

    <div class="ide-panes" class:split-down={ide.splitDirection === 'down'}>
      <div class="ide-pane">
        {#if activeTab.error || !stagedPrimaryTab}
          <LoadState what={activeTab.name} variant="panel" loading={activeTab.loading} error={activeTab.error} empty={!stagedPrimaryTab} onretry={() => void ide.openFile(activeTab.path)} />
        {:else if effectiveViewMode === 'table' && (activeDocKind === 'csv' || activeDocKind === 'jsonl') && stagedPrimaryTab.path === activeTab.path}
          <LazyMount
            lazy={IdeCsvViewLazy}
            what="table grid"
            variant="panel"
            props={{
              path: stagedPrimaryTab.path,
              content: stagedPrimaryTab.content,
              readOnly: stagedPrimaryTab.readOnly || !ide.canEditCollab,
              isJsonl: activeDocKind === 'jsonl',
              onchange: (val: string) => ide.updateBuffer(stagedPrimaryTab!.path, val),
              onswitchtext: (ln: number) => {
                tableModeByPath = { ...tableModeByPath, [stagedPrimaryTab!.path]: 'text' };
                requestAnimationFrame(() => primaryEditorRef?.goToLine(ln, 1));
              },
            }}
          />
        {:else}
          {@const pTab = stagedPrimaryTab}
          <CodeEditor
            bind:this={primaryEditorRef}
            path={pTab.path}
            content={pTab.content}
            savedContent={pTab.savedContent}
            blameByLine={blameEnabledByPath[pTab.path] ? (blameByPath[pTab.path] ?? null) : null}
            root={ide.target.root}
            language={pTab.language}
            readOnly={pTab.readOnly || !ide.canEditCollab}
            keepStates={true}
            lsp={!ide.collabIsGuest}
            ideMode={true}
            connectionId={ide.target.kind === 'ssh' ? ide.target.connection_id : null}
            breakpoints={ide.breakpointsByPath[pTab.path] ?? []}
            breakpointsMuted={areIdeBreakpointsMuted()}
            onToggleBreakpoint={(ln) => ide.toggleBreakpoint(pTab.path, ln)}
            onEditBreakpoint={(ln, ev) => openBreakpointPopover(pTab.path, ln, ev)}
            gutterItems={ide.gutterByPath[pTab.path] ?? []}
            onGutterAction={(item, mode) => ide.runFromGutter(item, mode)}
            pausedLine={ide.activeDebug?.state === 'paused' && ide.activeDebug.paused_file === pTab.path ? (ide.activeDebug.paused_line ?? null) : null}
            inlineValues={ide.activeDebug?.state === 'paused' && ide.activeDebug.paused_file === pTab.path ? ide.activeDebug.inline_values : []}
            peerCursors={ide.peerCursors.filter((p) => p.path === pTab.path)}
            keymapPreset={ide.keymapPreset}
            jumpToLine={ide.jumpRequest?.path === pTab.path ? { line: ide.jumpRequest.line, column: ide.jumpRequest.col, seq: ide.jumpRequest.seq } : null}
            onchange={(val) => ide.updateBuffer(pTab.path, val)}
            onCursorChange={(info) => ide.updateCursor(pTab.path, { line: info.line, col: info.column, cursors: info.cursorCount, selections: info.selections })}
            onDefinitionJump={(t) => void performIdeDive('definition', pTab.path, t, pTab.content)}
            onImplementationJump={(t) => void performIdeDive('implementation', pTab.path, t, pTab.content)}
            onFindUsages={(ctx) => void ide.findReferences(pTab.path, ctx.line, ctx.column, ctx.symbol)}
            onFileStructure={() => (structureOpen = true)}
            onJumpSymbol={(delta) => jumpAdjacentSymbol(delta)}
            onNavigateBack={() => void navigateIdeBack()}
            onNavigateForward={() => void navigateIdeForward()}
            onToggleBlame={() => toggleBlameFor(pTab.path)}
            onQuickFix={(ctx) => void openQuickFixAt(pTab.path, ctx.line, ctx.column)}
            onRenameSymbol={(ctx) => void ide.openRenameModal(pTab.path, ctx.line, ctx.column, ctx.symbol)}
            onFormat={() => void ide.formatActiveFile()}
            onDiagnostics={(diags) => ide.setDiagnostics(pTab.path, diags)}
            onAgentAction={(mode, sel) => {
              if (mode === 'inline') ideAgent.openInlineEdit({ path: pTab.path, ...sel });
              else { ideAgent.markSelectionFromEditor({ openPanel: true, selection: { path: pTab.path, ...sel } }); focusAgentComposer(); }
            }}
          />
        {/if}
      </div>

      {#if ide.splitTab}
        {@const splitTab = ide.splitTab}
        <div class="ide-pane split-secondary" data-testid="ide-split-pane">
          <div class="split-head" dir="ltr">
            <span class="split-title">{ide.relativePath(splitTab.path)}</span>
            <button type="button" class="icon-btn" aria-label="Close split pane" title="Close split pane" onclick={() => ide.closeSplit()}><Icon name="x" size={12} /></button>
          </div>
          <div class="split-body">
            {#if splitTab.error || !stagedSplitTab}
              <LoadState what={splitTab.name} variant="panel" loading={splitTab.loading} error={splitTab.error} empty={!stagedSplitTab} onretry={() => void ide.openFile(splitTab.path, { inSplit: true })} />
            {:else}
              {@const sTab = stagedSplitTab}
              <CodeEditor
                bind:this={splitEditorRef}
                path={sTab.path}
                content={sTab.content}
                savedContent={sTab.savedContent}
                blameByLine={blameEnabledByPath[sTab.path] ? (blameByPath[sTab.path] ?? null) : null}
                root={ide.target.root}
                language={sTab.language}
                readOnly={sTab.readOnly || !ide.canEditCollab}
                findOwner={false}
                keepStates={true}
                lsp={!ide.collabIsGuest}
                ideMode={true}
                connectionId={ide.target.kind === 'ssh' ? ide.target.connection_id : null}
                breakpoints={ide.breakpointsByPath[sTab.path] ?? []}
                breakpointsMuted={areIdeBreakpointsMuted()}
                onToggleBreakpoint={(ln) => ide.toggleBreakpoint(sTab.path, ln)}
                onEditBreakpoint={(ln, ev) => openBreakpointPopover(sTab.path, ln, ev)}
                gutterItems={ide.gutterByPath[sTab.path] ?? []}
                onGutterAction={(item, mode) => ide.runFromGutter(item, mode)}
                pausedLine={ide.activeDebug?.state === 'paused' && ide.activeDebug.paused_file === sTab.path ? (ide.activeDebug.paused_line ?? null) : null}
                inlineValues={ide.activeDebug?.state === 'paused' && ide.activeDebug.paused_file === sTab.path ? ide.activeDebug.inline_values : []}
                keymapPreset={ide.keymapPreset}
                onchange={(val) => ide.updateBuffer(sTab.path, val)}
                onDefinitionJump={(t) => void performIdeDive('definition', sTab.path, t, sTab.content)}
                onImplementationJump={(t) => void performIdeDive('implementation', sTab.path, t, sTab.content)}
                onFindUsages={(ctx) => void ide.findReferences(sTab.path, ctx.line, ctx.column, ctx.symbol)}
                onNavigateBack={() => void navigateIdeBack()}
                onNavigateForward={() => void navigateIdeForward()}
                onQuickFix={(ctx) => void openQuickFixAt(sTab.path, ctx.line, ctx.column)}
                onRenameSymbol={(ctx) => void ide.openRenameModal(sTab.path, ctx.line, ctx.column, ctx.symbol)}
                onAgentAction={(mode, sel) => {
                  if (mode === 'inline') ideAgent.openInlineEdit({ path: sTab.path, ...sel });
                  else { ideAgent.markSelectionFromEditor({ openPanel: true, selection: { path: sTab.path, ...sel } }); focusAgentComposer(); }
                }}
              />
            {/if}
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
        onaction={() => { ide.searchInitialTab = 'all'; ide.searchEverywhereOpen = true; }}
      />
    </div>
  {/if}
</section>

{#if bpEditor}
  <LazyMount
    lazy={IdeBreakpointPopoverLazy}
    what="breakpoint editor"
    quiet
    onerror={() => (bpEditor = null)}
    props={{
      path: bpEditor.path,
      line: bpEditor.line,
      anchorX: bpEditor.x,
      anchorY: bpEditor.y,
      onclose: () => (bpEditor = null),
    }}
  />
{/if}

{#if structureOpen}
  <LazyMount
    lazy={IdeFileStructureModalLazy}
    what="file structure"
    quiet
    onerror={() => (structureOpen = false)}
    props={{
      open: structureOpen,
      fileName: ide.activeTab?.name ?? '',
      symbols: effectiveSymbols,
      currentLine: ide.cursorInfo.line,
      onjump: (line: number, col: number) => {
        if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
        if (effectiveViewMode === 'table' && ide.activePath) tableModeByPath = { ...tableModeByPath, [ide.activePath]: 'text' };
        requestAnimationFrame(() => primaryEditorRef?.goToLine(line, col));
      },
      onclose: () => (structureOpen = false),
    }}
  />
{/if}

{#if ide.renameState.open}
  <LazyMount
    lazy={IdeRenameModalLazy}
    what="rename symbol dialog"
    quiet
    onerror={() => (ide.renameState.open = false)}
  />
{/if}

<style>
  .ide-editor-area { display: flex; flex-direction: column; flex: 1; min-inline-size: 0; min-block-size: 0; background: var(--bg); }
  .ide-subbar {
    display: flex; align-items: center; justify-content: space-between; gap: var(--sp-4);
    padding: var(--sp-2) 10px; background: var(--surface); border-block-end: 1px solid var(--border); min-inline-size: 0; flex-wrap: wrap;
  }
  .ide-crumbs { display: flex; align-items: center; gap: var(--sp-2); min-inline-size: 0; overflow: hidden; font-size: var(--fs-xs); color: var(--text-dim); }
  .crumb-sep { display: inline-flex; opacity: var(--disabled-opacity); }
  .crumb-btn { background: transparent; border: none; padding: 0; color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .crumb-btn:hover { color: var(--accent-text); }
  .crumb-btn.last { color: var(--text); font-weight: 500; }
  .crumb-sym-btn {
    display: inline-flex; align-items: center; gap: var(--sp-1); padding: var(--sp-1) var(--sp-3);
    border-radius: var(--radius-s); border: 1px solid var(--border); background: var(--surface-2);
    color: var(--accent-text); font-size: var(--fs-xs); cursor: pointer; max-inline-size: 220px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .crumb-sym-btn:hover { background: var(--accent-soft); }
  .ide-subbar-tools { display: flex; align-items: center; gap: var(--sp-2); flex-wrap: wrap; min-inline-size: 0; max-inline-size: 100%; }
  .ide-cursor-meta { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); padding-inline-end: var(--sp-2); }
  .ide-sel-bar {
    display: inline-flex; align-items: center; flex-wrap: wrap; gap: var(--sp-1); padding: var(--sp-1);
    min-inline-size: 0; max-inline-size: 100%; background: var(--surface-2); border: 1px solid var(--accent-line); border-radius: var(--radius-s);
  }
  .ide-sel-accent { color: var(--accent-text); }
  .ide-ps-bar {
    display: flex; align-items: center; gap: var(--sp-3); padding: var(--sp-3) 10px;
    background: var(--surface-2); border-block-end: 1px solid var(--border); flex-wrap: wrap;
  }
  .ide-ps-input {
    inline-size: 170px; max-inline-size: 100%; block-size: 26px; padding-inline: var(--sp-4);
    font-family: var(--font-mono); font-size: var(--fs-xs); background: var(--bg); color: var(--text);
    border: 1px solid var(--control-border); border-radius: var(--radius-s);
  }
  .ide-ps-tip { font-size: var(--fs-xs); color: var(--text-dim); margin-inline-start: auto; }
  .ide-panes { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; }
  .ide-panes.split-down { flex-direction: column; }
  .ide-pane { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  .split-secondary { border-inline-start: 1px solid var(--border); }
  .ide-panes.split-down .split-secondary { border-inline-start: none; border-block-start: 1px solid var(--border); }
  .split-head {
    display: flex; align-items: center; justify-content: space-between; padding: var(--sp-2) var(--sp-4);
    background: var(--surface); border-block-end: 1px solid var(--border); font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim);
  }
  .split-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .split-body { flex: 1; min-block-size: 0; display: flex; flex-direction: column; }
  .ide-empty-editor { flex: 1; display: flex; align-items: center; justify-content: center; min-block-size: 0; }
  @media (max-width: 640px) {
    .ide-ps-tip { display: none; }
    .ide-panes { flex-direction: column; }
    .split-secondary { border-inline-start: none; border-block-start: 1px solid var(--border); }
  }
</style>
