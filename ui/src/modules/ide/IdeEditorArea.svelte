<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import CodeEditor, { registerIdeEditorCore } from '../../lib/components/CodeEditor.svelte';
  import type * as CmIdeDepsMod from '../../lib/components/cm-ide-deps';
  import type * as CmIdeMod from '../../lib/components/cm-ide-extensions';
  import { readIdePreferences } from '../../lib/components/cm-ide-prefs';

  let histReady = $state(false);
  void Promise.all([
    import('../../lib/components/cm-ide-theme'),
    import('../../lib/components/cm-ide-history'),
    import('../../lib/components/cm-editor-smart'),
  ]).then(([themeMod, histMod, smartMod]) => {
    registerIdeEditorCore(themeMod, histMod, smartMod);
    histReady = true;
  });
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { isIdeManifestPath } from '../../lib/components/cm-ide-manifest';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { RESIZE_TITLE, RESIZE_TITLE_VERTICAL, splitter } from '../../lib/paneResizer';
  import { lsGet, lsSet } from '../../lib/storage';
  import { toasts } from '../../lib/toast.svelte';
  import { ide, type IdeCodeAction, type IdeEditorHandle, type IdeOpenTab, type IdeSymbolNode } from './ideStore.svelte';
  import { ideAgent } from './store/ideAgentController.svelte';
  import { ideEditorDebugProps } from './editor/ideEditorDebugBridge';
  import { computeSplitRatioFromKey } from './editor/ideEditorHelpers';
  import { attachIdeEditorWindowEvents } from './editor/ideEditorWindowEvents';
  import { subscribeIdeBufferEvictions, subscribeIdeFileSaved } from './store/ideBufferPerf';
  import IdeTabBar from './editor/IdeTabBar.svelte';
  import IdePreviewPane from './editor/IdePreviewPane.svelte';
  import IdeDiskChangedBanner from './editor/IdeDiskChangedBanner.svelte';
  import IdeTouchDebugBar from './editor/IdeTouchDebugBar.svelte';
  import {
    areIdeBreakpointsMuted, classifyIdeFile, evictPersistedIdeGitBase, fetchIdeGitBlame,
    flushAutoLocalHistorySnapshot, getIdeProjectFolders, getPersistedIdeGitBaseDigest,
    getPersistedIdeGitBaseSnapshot, ideEditorPrefs, idePreviewMode, navigateIdeBack, navigateIdeForward,
    performIdeDive, pushIdeNavLocation, resolveIdeGitBaseContent,
    setPersistedIdeGitBase,
  } from './editor/ideEditorStoreBridge.svelte';
  import type { ideGit as IdeGitSingleton } from './git/ideGitController.svelte';
  import { extractGutterHunks, findGutterHunkAtLine, resolveClickedDiffGutterLine, revertHunkInText, type IdeGutterHunk } from './git/ideGutterHunks';
  import type { IdeOutlineNode } from './editor/ideLangAnalyzer';
  let langAnalyzerMod = $state<typeof import('./editor/ideLangAnalyzer') | null>(null);
  let liveGit = $state<typeof IdeGitSingleton | null>(null);
  function fastDocKind(filePath: string, language?: string): string {
    const clean = filePath.split('?')[0].toLowerCase();
    const base = clean.slice(clean.lastIndexOf('/') + 1);
    const dot = base.lastIndexOf('.');
    const ext = dot >= 0 ? base.slice(dot + 1) : '';
    if (ext === 'csv' || ext === 'tsv' || language === 'csv' || language === 'tsv') return 'csv';
    if (ext === 'jsonl' || ext === 'ndjson' || language === 'jsonl') return 'jsonl';
    if (ext === 'json' || ext === 'jsonc' || language === 'json') return 'json';
    return langAnalyzerMod ? langAnalyzerMod.detectIdeDocKind(filePath, language) : 'other';
  }

  // The subbar only renders with an open tab, whose editor already waits on the
  // histReady chunks above; warm it now so it lands in the same window. Until
  // then a 30px placeholder holds its slot (no layout shift).
  const IdeEditorSubbarLazy = lazyComponent(() => import('./editor/IdeEditorSubbar.svelte'));
  IdeEditorSubbarLazy.prefetch();
  const IdeInlinePromptBarLazy = lazyComponent(() => import('./IdeInlinePromptBar.svelte'));
  const IdeCsvViewLazy = lazyComponent(() => import('./editor/IdeCsvView.svelte'));
  const IdeBreakpointPopoverLazy = lazyComponent(() => import('./editor/IdeBreakpointPopover.svelte'));
  const IdeFileStructureModalLazy = lazyComponent(() => import('./editor/IdeFileStructureModal.svelte'));
  const IdeRenameModalLazy = lazyComponent(() => import('./editor/IdeRenameModal.svelte'));
  const IdeIntentionPopoverLazy = lazyComponent(() => import('./editor/IdeIntentionPopover.svelte'));
  const IdeDependencyAnalyzerLazy = lazyComponent(() => import('./editor/IdeDependencyAnalyzer.svelte'));
  const IdeClipboardHistoryModalLazy = lazyComponent(() => import('./editor/IdeClipboardHistoryModal.svelte'));
  const IdeGitHunkPopoverLazy = lazyComponent(() => import('./editor/IdeGitHunkPopover.svelte'));

  const SPLIT_RATIO_KEY = 'otto.ide.splitRatio.v1';
  function readSavedSplitRatio(): number {
    const v = Number(lsGet(SPLIT_RATIO_KEY) ?? '');
    return Number.isFinite(v) && v >= 0.2 && v <= 0.8 ? v : 0.5;
  }
  function persistSplitRatio(next: number): void {
    splitRatio = next;
    lsSet(SPLIT_RATIO_KEY, next.toFixed(3));
  }

  type ExtendedEditorRef = IdeEditorHandle & {
    insertText?: (text: string) => void;
    evictPaths?: (paths: Iterable<string> | 'all') => void;
    notifyDidSave?: () => void;
    reconnectLspClients?: (rootPath?: string) => void;
    disposeLspClients?: (rootPath?: string) => void;
    replaceBufferContent?: (nextContent: string, range?: { from: number; to: number }) => boolean;
    applyQuickFixEdits?: (edits: CmIdeMod.IdeConcreteTextEdit[]) => boolean;
    applyClientDiagFix?: (fix: CmIdeMod.ClientDiagQuickFix) => boolean;
    getAvailableQuickFixes?: (line?: number, symbol?: string) => {
      clientFixes: CmIdeMod.ClientDiagQuickFix[];
      deterministicFixes: CmIdeMod.IdeQuickFixAction[];
      docText: string;
    };
  };

  let primaryEditorRef = $state<ExtendedEditorRef | undefined>(undefined);
  let splitEditorRef = $state<ExtendedEditorRef | undefined>(undefined);
  let panesEl = $state<HTMLDivElement | null>(null);
  let everMountedEditor = false;
  let structureOpen = $state(false);
  let clipboardModalOpen = $state(false);
  let bpEditor = $state<{ path: string; line: number; x: number; y: number } | null>(null);
  let hunkPopover = $state<{ path: string; hunkIndex: number; x: number; y: number } | null>(null);
  let intentionState = $state<{
    path: string; line: number; col: number; x: number; y: number; symbol?: string;
    manifestDep: CmIdeDepsMod.IdeDepItem | null;
    serverActions: IdeCodeAction[];
  } | null>(null);
  let manifestModeByPath = $state<Record<string, 'text' | 'analyzer'>>({});
  let manifestDepsByPath = $state<Record<string, CmIdeDepsMod.IdeDepItem[]>>({});
  let blameEnabledByPath = $state<Record<string, boolean>>({});
  let blameByPath = $state<Record<string, Record<number, string> | null>>({});
  let gitBaseByPath = $state<Record<string, string | null | undefined>>(getPersistedIdeGitBaseSnapshot());
  const gitBaseCtlByPath = new Map<string, AbortController>();
  let splitRatio = $state(readSavedSplitRatio());
  let cancelSplitDrag: (() => void) | null = null;
  let prefs = $state(readIdePreferences());
  let stagedPrimaryTab = $state<IdeOpenTab | null>(null);
  let stagedSplitTab = $state<IdeOpenTab | null>(null);

  function refreshGitBaseForTab(tab: IdeOpenTab, force = false): void {
    const filePath = tab.path;
    if (!filePath || filePath.startsWith('untitled:') || filePath.startsWith('jdt://')) return;
    const digest = liveGit ? liveGit.fileStatusDigest(filePath) : 'pending';
    if (!force && getPersistedIdeGitBaseDigest(filePath) === digest && filePath in gitBaseByPath) return;
    gitBaseCtlByPath.get(filePath)?.abort();
    const ctl = new AbortController();
    gitBaseCtlByPath.set(filePath, ctl);
    void resolveIdeGitBaseContent(filePath, tab.savedContent, ctl.signal)
      .then((base) => {
        if (ctl.signal.aborted) return;
        setPersistedIdeGitBase(filePath, base, digest);
        if (gitBaseByPath[filePath] !== base || !(filePath in gitBaseByPath)) {
          gitBaseByPath = { ...gitBaseByPath, [filePath]: base };
        }
      })
      .catch(() => { /* ignore aborted or non-git paths */ });
  }

  /** A dependency action wrote the manifest itself: take the file from disk (an
   *  edit on top of the old sha would end in a save conflict and a dirty tab). */
  function reloadTabFromDisk(path: string): void {
    void import('./git/ideGitController.svelte').then((m) => m.ideGit.reloadIdeTabIfOpen(path));
  }

  $effect(() => {
    const cur = ide.activeTab;
    if (!cur) { stagedPrimaryTab = null; return; }
    if (!langAnalyzerMod) void import('./editor/ideLangAnalyzer').then((m) => { langAnalyzerMod = m; });
    if (!liveGit) void import('./git/ideGitController.svelte').then((m) => { liveGit = m.ideGit; });
    if (!cur.loading && !cur.error && histReady) { stagedPrimaryTab = cur; everMountedEditor = true; }
    else if (stagedPrimaryTab && !ide.tabs.some((t) => t.path === stagedPrimaryTab?.path)) stagedPrimaryTab = null;
  });

  $effect(() => {
    const cur = ide.splitTab;
    if (!cur) { stagedSplitTab = null; return; }
    if (!liveGit) void import('./git/ideGitController.svelte').then((m) => { liveGit = m.ideGit; });
    if (!cur.loading && !cur.error && histReady) { stagedSplitTab = cur; everMountedEditor = true; }
    else if (stagedSplitTab && !ide.tabs.some((t) => t.path === stagedSplitTab?.path)) stagedSplitTab = null;
  });

  $effect(() => {
    const p = stagedPrimaryTab;
    if (!p) return;
    void (liveGit ? liveGit.fileStatusDigest(p.path) : 'pending');
    untrack(() => refreshGitBaseForTab(p));
  });

  $effect(() => {
    const s = stagedSplitTab;
    if (!s) return;
    void (liveGit ? liveGit.fileStatusDigest(s.path) : 'pending');
    untrack(() => refreshGitBaseForTab(s));
  });

  // Subscribe to repo status digests even when the Git tool panel is closed so
  // external `git restore` / `git checkout` reloads clean open tabs immediately.
  $effect(() => {
    if (liveGit) liveGit.syncExternalRepoChanges();
  });

  $effect(() => attachIdeEditorWindowEvents({
    onPrefsChanged: () => { prefs = readIdePreferences(); },
    onOpenClipboardHistory: () => { clipboardModalOpen = true; },
    getPrimaryPath: () => stagedPrimaryTab?.path ?? null,
    getSplitPath: () => stagedSplitTab?.path ?? null,
    applyQuickFixEdits: (pane, edits) => (pane === 'primary' ? primaryEditorRef : splitEditorRef)?.applyQuickFixEdits?.(edits),
    reconnectLspClients: (rootPath) => { primaryEditorRef?.reconnectLspClients?.(rootPath); splitEditorRef?.reconnectLspClients?.(rootPath); },
    refreshGitBase: (tab) => refreshGitBaseForTab(tab, true),
  }));

  function disposeAllPooledLsp(): void {
    primaryEditorRef?.disposeLspClients?.();
    splitEditorRef?.disposeLspClients?.();
    if (everMountedEditor) {
      everMountedEditor = false;
      void import('../../lib/components/cm-ide-lsp').then((m) => m.disposePooledLspClients({ forceAll: true })).catch(() => {});
    }
  }

  const unsubEvict = subscribeIdeBufferEvictions((paths) => {
    const pathList = paths === 'all' ? 'all' : [...paths];
    primaryEditorRef?.evictPaths?.(pathList);
    splitEditorRef?.evictPaths?.(pathList);
    if (!primaryEditorRef) {
      void import('../../lib/components/cm-ide-history').then((m) => {
        m.evictSingletonIdeState(pathList);
        if (pathList === 'all') m.forgetIdeUndoJournal('all');
        else for (const p of pathList) m.forgetIdeUndoJournal(p);
      }).catch(() => {});
    }
    if (pathList === 'all') {
      for (const ctl of gitBaseCtlByPath.values()) ctl.abort();
      gitBaseCtlByPath.clear(); evictPersistedIdeGitBase('all'); gitBaseByPath = {};
      blameEnabledByPath = {}; blameByPath = {}; manifestModeByPath = {}; manifestDepsByPath = {};
      bpEditor = null; intentionState = null; hunkPopover = null; disposeAllPooledLsp(); return;
    }
    evictPersistedIdeGitBase(pathList);
    const nextBlame = { ...blameByPath }, nextBlameOn = { ...blameEnabledByPath }, nextGitBase = { ...gitBaseByPath };
    const nextManMode = { ...manifestModeByPath }, nextManDeps = { ...manifestDepsByPath };
    for (const p of pathList) {
      gitBaseCtlByPath.get(p)?.abort(); gitBaseCtlByPath.delete(p); delete nextGitBase[p];
      delete nextBlame[p]; delete nextBlameOn[p]; delete nextManMode[p]; delete nextManDeps[p];
      if (bpEditor?.path === p) bpEditor = null;
      if (intentionState?.path === p) intentionState = null;
      if (hunkPopover?.path === p) hunkPopover = null;
    }
    blameByPath = nextBlame; blameEnabledByPath = nextBlameOn; gitBaseByPath = nextGitBase;
    manifestModeByPath = nextManMode; manifestDepsByPath = nextManDeps;
    if (ide.tabs.length === 0) disposeAllPooledLsp();
  });

  const unsubSaved = subscribeIdeFileSaved((savedPath) => {
    flushAutoLocalHistorySnapshot(savedPath);
    if (savedPath === stagedPrimaryTab?.path) primaryEditorRef?.notifyDidSave?.();
    if (savedPath === stagedSplitTab?.path) splitEditorRef?.notifyDidSave?.();
    if (liveGit) liveGit.onIdeFileSaved(savedPath);
    else void import('./git/ideGitController.svelte').then((m) => m.ideGit.onIdeFileSaved(savedPath)).catch(() => {});
  });

  onDestroy(() => {
    cancelSplitDrag?.();
    cancelSplitDrag = null;
    for (const ctl of gitBaseCtlByPath.values()) ctl.abort();
    gitBaseCtlByPath.clear();
    flushAutoLocalHistorySnapshot();
    unsubEvict();
    unsubSaved();
    disposeAllPooledLsp();
  });

  const projectFolders = $derived(getIdeProjectFolders());
  const hasNonEmptySelection = $derived(ide.cursorInfo.selections.some((s) => s.anchor !== s.head));
  const activeDocKind = $derived(ide.activeTab ? fastDocKind(ide.activeTab.path, ide.activeTab.language) : 'other');
  const activeIsManifest = $derived(ide.activeTab ? isIdeManifestPath(ide.activeTab.path) : false);
  const effectiveManifestMode = $derived(ide.activeTab ? (manifestModeByPath[ide.activeTab.path] ?? 'text') : 'text');
  const activeFileMeta = $derived(ide.activeTab ? classifyIdeFile(ide.activeTab, ide.target.root, projectFolders.slice(1)) : { external: false, sdkLabel: null });
  const effectiveViewMode = $derived.by((): 'table' | 'text' => {
    const t = ide.activeTab;
    return t && idePreviewMode.layoutFor(t.path) === 'table' ? 'table' : 'text';
  });
  const effectiveSymbols = $derived.by((): IdeSymbolNode[] => {
    const t = ide.activeTab;
    if (!t) return [];
    if (ide.activeSymbols.length > 0) return ide.activeSymbols;
    return langAnalyzerMod ? langAnalyzerMod.extractInstantSymbols(t.path, t.language, t.content) : [];
  });
  const activeJsonPath = $derived(
    activeDocKind === 'json' && ide.activeTab && langAnalyzerMod
      ? langAnalyzerMod.computeJsonPathAt(ide.activeTab.content, ide.cursorInfo.line, ide.cursorInfo.col)
      : null,
  );
  const pathSegments = $derived(ide.activeTab ? ide.relativePath(ide.activeTab.path).split('/').filter(Boolean) : []);
  const diffBaseFor = (t: IdeOpenTab): string | null => (gitBaseByPath[t.path] !== undefined ? (gitBaseByPath[t.path] ?? null) : t.savedContent);
  const activeGitHunks = $derived(ide.activeTab ? extractGutterHunks(diffBaseFor(ide.activeTab), ide.activeTab.content, ide.relativePath(ide.activeTab.path)) : []);
  const activeCursorHunk = $derived(findGutterHunkAtLine(activeGitHunks, ide.cursorInfo.line));

  $effect(() => {
    const el = panesEl;
    if (!el) return;
    const onDown = (e: MouseEvent): void => {
      const ln = resolveClickedDiffGutterLine(e);
      if (ln == null) return;
      const tab = e.target instanceof Element && e.target.closest('.split-secondary') ? stagedSplitTab : stagedPrimaryTab;
      const hit = tab ? findGutterHunkAtLine(extractGutterHunks(diffBaseFor(tab), tab.content, ide.relativePath(tab.path)), ln) : null;
      if (!tab || !hit) return;
      e.preventDefault(); e.stopPropagation();
      hunkPopover = { path: tab.path, hunkIndex: hit.index, x: e.clientX, y: e.clientY };
    };
    el.addEventListener('mousedown', onDown, true);
    return () => el.removeEventListener('mousedown', onDown, true);
  });

  function toEditorHandle(ref: ExtendedEditorRef): IdeEditorHandle {
    const h: IdeEditorHandle & { applyQuickFixEdits?: (edits: CmIdeMod.IdeConcreteTextEdit[]) => boolean } = {
      applyPrefixSuffix: (prefix, suffix) => ref.applyPrefixSuffix(prefix, suffix),
      splitSelectionToLines: () => ref.splitSelectionToLines(),
      goToLine: (line, col) => ref.goToLine(line, col),
      focus: () => ref.focus(),
      getSelectionRange: () => ref.getSelectionRange?.() ?? null,
      replaceBufferContent: (nextContent, range) => ref.replaceBufferContent?.(nextContent, range) ?? false,
      applyAgentEdit: (nextContent, range) => ref.applyAgentEdit?.(nextContent, range) ?? false,
      applyQuickFixEdits: (edits) => ref.applyQuickFixEdits?.(edits) ?? false,
      lspRenameEdits: (line, column, newName) => ref.lspRenameEdits?.(line, column, newName) ?? Promise.resolve(null),
      lspReferences: (line, column) => ref.lspReferences?.(line, column) ?? Promise.resolve(null),
      lspQuickFixes: (line) => ref.lspQuickFixes?.(line) ?? Promise.resolve(null),
    };
    return h;
  }

  $effect(() => { ide.registerEditorHandle('primary', primaryEditorRef ? toEditorHandle(primaryEditorRef) : null); return () => ide.registerEditorHandle('primary', null); });
  $effect(() => { ide.registerEditorHandle('split', splitEditorRef ? toEditorHandle(splitEditorRef) : null); return () => ide.registerEditorHandle('split', null); });

  function focusAgentComposer(): void {
    let tries = 0;
    const step = (): void => {
      const el = document.querySelector<HTMLElement>('[data-testid="ide-agent-composer"]');
      if (el) el.focus(); else if (++tries < 12) requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  }

  async function saveWithOptionalFormat(targetPath: string): Promise<void> {
    const storeHandlesFormat = (ide as { formatOnSave?: boolean }).formatOnSave !== undefined;
    if (!storeHandlesFormat && (ideEditorPrefs.formatOnSave || prefs.formatOnSave) && targetPath === ide.activePath) {
      try { await ide.formatActiveFile(); } catch { /* ignore */ }
    }
    await ide.saveFile(targetPath);
  }

  function toggleBlameFor(filePath: string): void {
    const next = !blameEnabledByPath[filePath];
    blameEnabledByPath = { ...blameEnabledByPath, [filePath]: next };
    if (!next) { const copy = { ...blameByPath }; delete copy[filePath]; blameByPath = copy; return; }
    void fetchIdeGitBlame(ide.target.root, filePath).then((res) => {
      if (!blameEnabledByPath[filePath]) return;
      if (!res) toasts.info('No git blame available for this file');
      blameByPath = { ...blameByPath, [filePath]: res };
    });
  }

  function jumpAdjacentSymbol(delta: -1 | 1): void {
    const flat: IdeSymbolNode[] = [];
    const walk = (nodes: IdeSymbolNode[]) => { for (const n of nodes) { flat.push(n); if (n.children?.length) walk(n.children); } };
    walk(effectiveSymbols);
    if (!flat.length) return;
    flat.sort((a, b) => a.line - b.line);
    const cur = ide.cursorInfo.line;
    const target = delta > 0 ? (flat.find((s) => s.line > cur) ?? flat[0]) : ([...flat].reverse().find((s) => s.line < cur) ?? flat[flat.length - 1]);
    if (target && ide.activePath) {
      pushIdeNavLocation(ide.activePath, cur, ide.cursorInfo.col);
      primaryEditorRef?.goToLine(target.line, (target as IdeOutlineNode).column || 1);
    }
  }

  function openBreakpointPopover(filePath: string, line: number, ev?: MouseEvent): void {
    bpEditor = { path: filePath, line, x: ev?.clientX ?? 240, y: ev?.clientY ?? 180 };
  }

  function openIntentionPopoverAt(
    filePath: string, line: number, col: number,
    opts?: { x?: number; y?: number; symbol?: string; manifestDep?: CmIdeDepsMod.IdeDepItem | null },
  ): void {
    const rect = document.querySelector<HTMLElement>('[data-testid="ide-editor-more-btn"]')?.getBoundingClientRect();
    const x = opts?.x ?? (rect ? rect.left - 180 : 280);
    const y = opts?.y ?? (rect ? rect.bottom + 6 : 160);
    const dep = opts?.manifestDep ?? (manifestDepsByPath[filePath] ?? []).find((d) => line >= (d.block_start_line ?? d.line) && line <= (d.end_line ?? d.block_end_line ?? d.line)) ?? null;
    intentionState = { path: filePath, line, col, x, y, symbol: opts?.symbol, manifestDep: dep, serverActions: [] };
    void ide.fetchCodeActions(filePath, line, col).then((acts) => {
      if (intentionState?.path === filePath && intentionState.line === line) intentionState = { ...intentionState, serverActions: acts };
    }).catch(() => {});
  }

  function startSplitResize(e: MouseEvent): void {
    if (!panesEl) return;
    cancelSplitDrag?.();
    const down = ide.splitDirection === 'down', rect = panesEl.getBoundingClientRect();
    const rtl = getComputedStyle(panesEl).direction === 'rtl';
    cancelSplitDrag = startMouseDrag(e, {
      cursor: down ? 'row-resize' : 'col-resize',
      onMove: (me) => {
        const raw = down
          ? (me.clientY - rect.top) / Math.max(1, rect.height)
          : (rtl ? rect.right - me.clientX : me.clientX - rect.left) / Math.max(1, rect.width);
        splitRatio = Math.max(0.2, Math.min(0.8, raw));
      },
      onEnd: () => {
        cancelSplitDrag = null;
        lsSet(SPLIT_RATIO_KEY, splitRatio.toFixed(3));
      },
    });
  }

  function onSplitResizerKeydown(e: KeyboardEvent): void {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const down = ide.splitDirection === 'down';
    const rtl = !down && Boolean(panesEl && getComputedStyle(panesEl).direction === 'rtl');
    const next = computeSplitRatioFromKey(splitRatio, e.key, { down, rtl, shiftKey: e.shiftKey });
    if (next === null) return;
    e.preventDefault();
    persistSplitRatio(next);
  }
</script>

<section class="ide-editor-area" aria-label="Code editor" data-testid="ide-editor-area">
  {#if ide.tabs.length > 0}<IdeTabBar onsave={(p) => void saveWithOptionalFormat(p)} />{/if}

  {#if ide.activeTab}
    {@const activeTab = ide.activeTab}
    {#if IdeEditorSubbarLazy.component}
      <IdeEditorSubbarLazy.component
        {activeTab}
        {pathSegments}
        {effectiveSymbols}
        {activeJsonPath}
        sdkLabel={activeFileMeta.sdkLabel}
        {activeDocKind}
        {effectiveViewMode}
        {hasNonEmptySelection}
        blameActive={Boolean(blameEnabledByPath[activeTab.path])}
        activeGitHunk={activeCursorHunk} gitHunkCount={activeGitHunks.length}
        onapplyjson={(txt) => { if (!primaryEditorRef?.applyAgentEdit?.(txt)) ide.updateBuffer(activeTab.path, txt); }}
        onjumpsymbol={jumpAdjacentSymbol} ongotoline={(ln, col) => primaryEditorRef?.goToLine(ln, col)}
        onopenstructure={() => (structureOpen = true)} ontoggleblame={() => toggleBlameFor(activeTab.path)}
        onopenquickfix={(e) => openIntentionPopoverAt(activeTab.path, ide.cursorInfo.line, ide.cursorInfo.col, { x: e.clientX - 200, y: e.clientY + 10 })}
        onopengithunk={(e) => {
          if (!activeCursorHunk) return;
          const r = (e.currentTarget as HTMLElement | null)?.getBoundingClientRect();
          hunkPopover = { path: activeTab.path, hunkIndex: activeCursorHunk.index, x: r ? r.left : e.clientX, y: r ? r.bottom + 4 : e.clientY + 6 };
        }}
        onfocusagent={focusAgentComposer}
      />
    {:else if IdeEditorSubbarLazy.error}
      <LoadState what="the editor toolbar" variant="compact" error={IdeEditorSubbarLazy.error} onretry={() => IdeEditorSubbarLazy.retry()} />
    {:else}
      <div class="ide-subbar-slot" aria-hidden="true"></div>
    {/if}

    {#if ideAgent.inlinePrompt?.open}<LazyMount lazy={IdeInlinePromptBarLazy} what="inline prompt" variant="compact" />{/if}
    <IdeDiskChangedBanner tab={activeTab} />

    <div class="ide-panes" class:split-down={ide.splitDirection === 'down'} bind:this={panesEl}>
      <div class="ide-pane {idePreviewMode.paneClass(stagedPrimaryTab?.path === activeTab.path ? stagedPrimaryTab : null)}" style:flex={ide.splitTab ? `0 0 calc(${(splitRatio * 100).toFixed(1)}% - 3px)` : '1 1 0%'}>
        {#if activeTab.error || !stagedPrimaryTab}
          <LoadState what={activeTab.name} variant="panel" loading={activeTab.loading} error={activeTab.error} empty={!stagedPrimaryTab} onretry={() => void ide.openFile(activeTab.path)} />
        {:else if activeIsManifest && effectiveManifestMode === 'analyzer' && stagedPrimaryTab.path === activeTab.path}
          <LazyMount
            lazy={IdeDependencyAnalyzerLazy}
            what="dependency analyzer"
            variant="panel"
            props={{
              filePath: stagedPrimaryTab.path, content: stagedPrimaryTab.content, root: ide.target.root,
              connectionId: ide.target.kind === 'ssh' ? ide.target.connection_id : null, projectFolders,
              readOnly: stagedPrimaryTab.readOnly || !ide.canEditCollab,
              initialDeps: manifestDepsByPath[stagedPrimaryTab.path] ?? [],
              onfilewritten: () => reloadTabFromDisk(stagedPrimaryTab!.path),
              onjumplocation: (loc: { path: string; line: number; column: number }) => { void performIdeDive('definition', stagedPrimaryTab!.path, loc, stagedPrimaryTab!.content); },
              onswitchtext: (ln: number) => { manifestModeByPath = { ...manifestModeByPath, [stagedPrimaryTab!.path]: 'text' }; requestAnimationFrame(() => primaryEditorRef?.goToLine(ln, 1)); },
            }}
          />
        {:else if effectiveViewMode === 'table' && (activeDocKind === 'csv' || activeDocKind === 'jsonl') && stagedPrimaryTab.path === activeTab.path}
          <LazyMount
            lazy={IdeCsvViewLazy}
            what="table grid"
            variant="panel"
            props={{
              path: stagedPrimaryTab.path, content: stagedPrimaryTab.content,
              readOnly: stagedPrimaryTab.readOnly || !ide.canEditCollab, isJsonl: activeDocKind === 'jsonl',
              onchange: (val: string) => { ide.updateBuffer(stagedPrimaryTab!.path, val); },
              onswitchtext: (ln: number) => { idePreviewMode.set(stagedPrimaryTab!.path, 'source'); requestAnimationFrame(() => primaryEditorRef?.goToLine(ln, 1)); },
            }}
          />
        {:else}
          {@const pTab = stagedPrimaryTab}
          <CodeEditor
            bind:this={primaryEditorRef}
            path={pTab.path} content={pTab.content} savedContent={pTab.savedContent} gitBaseContent={gitBaseByPath[pTab.path]}
            blameByLine={blameEnabledByPath[pTab.path] ? (blameByPath[pTab.path] ?? null) : null}
            root={ide.target.root} {projectFolders} language={pTab.language}
            readOnly={pTab.readOnly || !ide.canEditCollab} keepStates={true} lsp={!ide.collabIsGuest} ideMode={true}
            connectionId={ide.target.kind === 'ssh' ? ide.target.connection_id : null}
            breakpoints={ide.breakpointsByPath[pTab.path] ?? []} breakpointsMuted={areIdeBreakpointsMuted()}
            onToggleBreakpoint={(ln) => ide.toggleBreakpoint(pTab.path, ln)}
            onEditBreakpoint={(ln, ev) => openBreakpointPopover(pTab.path, ln, ev)}
            gutterItems={ide.gutterByPath[pTab.path] ?? []}
            onGutterAction={(item, mode) => ide.runFromGutter(item, mode)}
            {...ideEditorDebugProps(pTab.path)}
            peerCursors={ide.peerCursors.filter((p) => p.path === pTab.path)}
            keymapPreset={ide.keymapPreset}
            fontLigatures={prefs.fontLigatures}
            jumpToLine={ide.jumpRequest?.path === pTab.path ? { line: ide.jumpRequest.line, column: ide.jumpRequest.col, seq: ide.jumpRequest.seq } : null}
            onchange={(val) => { ide.updateBuffer(pTab.path, val); }}
            onCursorChange={(info) => ide.updateCursor(pTab.path, { line: info.line, col: info.column, cursors: info.cursorCount, selections: info.selections })}
            onDefinitionJump={(t) => void performIdeDive('definition', pTab.path, t, pTab.content)}
            onImplementationJump={(t) => void performIdeDive('implementation', pTab.path, t, pTab.content)}
            onFindUsages={(ctx) => void ide.findReferences(pTab.path, ctx.line, ctx.column, ctx.symbol)}
            onFileStructure={() => (structureOpen = true)}
            onJumpSymbol={(delta) => jumpAdjacentSymbol(delta)}
            onNavigateBack={() => void navigateIdeBack()}
            onNavigateForward={() => void navigateIdeForward()}
            onToggleBlame={() => toggleBlameFor(pTab.path)}
            onPasteFromHistory={() => (clipboardModalOpen = true)}
            onOpenLocalHistory={() => {
              flushAutoLocalHistorySnapshot(pTab.path);
              window.dispatchEvent(new CustomEvent('otto:ide-open-compare', { detail: { mode: 'local-history', path: pTab.path } }));
            }}
            onOpenDepAnalyzer={() => { manifestModeByPath = { ...manifestModeByPath, [pTab.path]: 'analyzer' }; }}
            onOpenManifestIntention={(ln, dep) => openIntentionPopoverAt(pTab.path, ln, 1, { manifestDep: dep })}
            onManifestDepsUpdated={(items) => untrack(() => {
              const prev = manifestDepsByPath[pTab.path];
              if (!items.length && (!prev || !prev.length)) return;
              manifestDepsByPath = { ...manifestDepsByPath, [pTab.path]: items };
            })}
            onQuickFix={(ctx) => openIntentionPopoverAt(pTab.path, ctx.line, ctx.column, { x: ctx.clientX, y: ctx.clientY, symbol: ctx.symbol })}
            onRenameSymbol={(ctx) => void ide.openRenameModal(pTab.path, ctx.line, ctx.column, ctx.symbol, ctx.newName)}
            onFormat={() => void ide.formatActiveFile()}
            onDiagnostics={(diags) => ide.setDiagnostics(pTab.path, diags)}
            onAgentAction={(mode, sel) => {
              if (mode === 'inline') ideAgent.openInlineEdit({ path: pTab.path, ...sel });
              else { ideAgent.markSelectionFromEditor({ openPanel: true, selection: { path: pTab.path, ...sel } }); focusAgentComposer(); }
            }}
          />
        {/if}

        {#if stagedPrimaryTab?.path === activeTab.path}<IdePreviewPane tab={stagedPrimaryTab} />{/if}
        <IdeTouchDebugBar />
        {#if activeIsManifest}
          <div class="ide-manifest-tabs" role="group" aria-label="Manifest editor mode" data-testid="ide-dep-mode-switcher">
            <button type="button" class="ide-man-tab" class:active={effectiveManifestMode === 'text'} aria-pressed={effectiveManifestMode === 'text'} onclick={() => (manifestModeByPath = { ...manifestModeByPath, [activeTab.path]: 'text' })}>Text</button>
            <button type="button" class="ide-man-tab" class:active={effectiveManifestMode === 'analyzer'} aria-pressed={effectiveManifestMode === 'analyzer'} onclick={() => (manifestModeByPath = { ...manifestModeByPath, [activeTab.path]: 'analyzer' })} data-testid="ide-dep-analyzer-tab">Dependency Analyzer</button>
          </div>
        {/if}
      </div>

      {#if ide.splitTab}
        {@const splitTab = ide.splitTab}
        <div
          class="ide-split-resizer"
          role="separator"
          aria-label="Resize split editor panes"
          aria-orientation={ide.splitDirection === 'down' ? 'horizontal' : 'vertical'}
          aria-valuemin={20}
          aria-valuemax={80}
          aria-valuenow={Math.round(splitRatio * 100)}
          title={ide.splitDirection === 'down' ? RESIZE_TITLE_VERTICAL : RESIZE_TITLE}
          use:splitter={{
            onmousedown: startSplitResize,
            ondblclick: () => persistSplitRatio(0.5),
            onkeydown: onSplitResizerKeydown,
          }}
          data-testid="ide-editor-split-resizer"
        ></div>

        <div class="ide-pane split-secondary" data-testid="ide-split-pane">
          <div class="split-head" dir="ltr">
            <span class="split-title">{ide.relativePath(splitTab.path)}</span>
            <button type="button" class="icon-btn" aria-label="Close split pane" title="Close split pane" onclick={() => ide.closeSplit()}><Icon name="x" size={12} /></button>
          </div>
          {#if splitTab.path !== activeTab.path}<IdeDiskChangedBanner tab={splitTab} watch={false} />{/if}
          <div class="split-body">
            {#if splitTab.error || !stagedSplitTab}
              <LoadState what={splitTab.name} variant="panel" loading={splitTab.loading} error={splitTab.error} empty={!stagedSplitTab} onretry={() => void ide.openFile(splitTab.path, { inSplit: true })} />
            {:else}
              {@const sTab = stagedSplitTab}
              <CodeEditor
                bind:this={splitEditorRef}
                path={sTab.path} content={sTab.content} savedContent={sTab.savedContent} gitBaseContent={gitBaseByPath[sTab.path]}
                blameByLine={blameEnabledByPath[sTab.path] ? (blameByPath[sTab.path] ?? null) : null}
                root={ide.target.root} {projectFolders} language={sTab.language}
                readOnly={sTab.readOnly || !ide.canEditCollab} findOwner={false} keepStates={true} lsp={!ide.collabIsGuest} ideMode={true}
                connectionId={ide.target.kind === 'ssh' ? ide.target.connection_id : null}
                breakpoints={ide.breakpointsByPath[sTab.path] ?? []} breakpointsMuted={areIdeBreakpointsMuted()}
                onToggleBreakpoint={(ln) => ide.toggleBreakpoint(sTab.path, ln)}
                onEditBreakpoint={(ln, ev) => openBreakpointPopover(sTab.path, ln, ev)}
                gutterItems={ide.gutterByPath[sTab.path] ?? []}
                onGutterAction={(item, mode) => ide.runFromGutter(item, mode)}
                {...ideEditorDebugProps(sTab.path)}
                peerCursors={ide.peerCursors.filter((p) => p.path === sTab.path)}
                keymapPreset={ide.keymapPreset}
                fontLigatures={prefs.fontLigatures}
                onchange={(val) => { ide.updateBuffer(sTab.path, val); }}
                onCursorChange={(info) => ide.updateCursor(sTab.path, { line: info.line, col: info.column, cursors: info.cursorCount, selections: info.selections })}
                onDefinitionJump={(t) => void performIdeDive('definition', sTab.path, t, sTab.content)}
                onImplementationJump={(t) => void performIdeDive('implementation', sTab.path, t, sTab.content)}
                onFindUsages={(ctx) => void ide.findReferences(sTab.path, ctx.line, ctx.column, ctx.symbol)}
                onNavigateBack={() => void navigateIdeBack()}
                onNavigateForward={() => void navigateIdeForward()}
                onToggleBlame={() => toggleBlameFor(sTab.path)}
                onQuickFix={(ctx) => openIntentionPopoverAt(sTab.path, ctx.line, ctx.column, { x: ctx.clientX, y: ctx.clientY, symbol: ctx.symbol })}
                onRenameSymbol={(ctx) => void ide.openRenameModal(sTab.path, ctx.line, ctx.column, ctx.symbol, ctx.newName)}
                onFormat={() => void ide.formatActiveFile(sTab.path)}
                onDiagnostics={(diags) => ide.setDiagnostics(sTab.path, diags)}
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
        headingLevel={2} icon="ide" title="Select a file to start coding"
        body="Choose a file in the explorer or press ⇧⇧ to search everywhere."
        actionLabel="Search everywhere" actionIcon="search" actionKind="secondary"
        onaction={() => { ide.searchInitialTab = 'all'; ide.searchEverywhereOpen = true; }}
      />
    </div>
  {/if}
</section>

{#if intentionState}
  <!-- Actions apply to the pane the popover was opened from (a split-pane ⌥⏎ must not edit the primary file). -->
  {@const iRef = intentionState.path !== stagedPrimaryTab?.path && intentionState.path === stagedSplitTab?.path ? splitEditorRef : primaryEditorRef}
  {@const iTab = ide.tabs.find((t) => t.path === intentionState?.path)}
  {@const qfBundle = iRef?.getAvailableQuickFixes?.(intentionState.line, intentionState.symbol) ?? { clientFixes: [], deterministicFixes: [], docText: iTab?.content ?? '' }}
  <LazyMount
    lazy={IdeIntentionPopoverLazy} what="context actions" quiet onerror={() => (intentionState = null)}
    props={{
      filePath: intentionState.path, root: ide.target.root,
      connectionId: ide.target.kind === 'ssh' ? ide.target.connection_id : null, projectFolders,
      docText: qfBundle.docText, line: intentionState.line, col: intentionState.col,
      anchorX: intentionState.x, anchorY: intentionState.y,
      readOnly: Boolean(iTab?.readOnly || !ide.canEditCollab),
      clientFixes: qfBundle.clientFixes, deterministicFixes: qfBundle.deterministicFixes,
      serverActions: intentionState.serverActions, manifestDep: intentionState.manifestDep,
      allManifestDeps: manifestDepsByPath[intentionState.path] ?? [],
      onapplyclientfix: (fix: CmIdeMod.ClientDiagQuickFix) => { iRef?.applyClientDiagFix?.(fix); },
      onapplyedits: (edits: CmIdeMod.IdeConcreteTextEdit[]) => { iRef?.applyQuickFixEdits?.(edits); },
      onfilewritten: () => { if (intentionState) reloadTabFromDisk(intentionState.path); },
      onapplyserveraction: (act: IdeCodeAction) => { if (intentionState) ide.applyCodeAction(intentionState.path, act); },
      onrename: () => { if (intentionState) void ide.openRenameModal(intentionState.path, intentionState.line, intentionState.col, intentionState.symbol); },
      onfindusages: () => { if (intentionState) void ide.findReferences(intentionState.path, intentionState.line, intentionState.col, intentionState.symbol); },
      onformat: () => { void ide.formatActiveFile(intentionState?.path); },
      onaskagent: () => {
        const sel = iRef?.getSelectionRange?.();
        if (intentionState && sel) ideAgent.openInlineEdit({ path: intentionState.path, ...sel });
        else ideAgent.openInlineEdit();
      },
      onclose: () => (intentionState = null),
    }}
  />
{/if}

{#if clipboardModalOpen}
  <LazyMount
    lazy={IdeClipboardHistoryModalLazy} what="clipboard history" quiet onerror={() => (clipboardModalOpen = false)}
    props={{
      filePath: ide.activePath ?? '', readOnly: Boolean(!ide.activeTab || ide.activeTab.readOnly || !ide.canEditCollab),
      onpaste: (txt: string) => { primaryEditorRef?.insertText?.(txt); }, onclose: () => (clipboardModalOpen = false),
    }}
  />
{/if}

{#if bpEditor}
  <LazyMount lazy={IdeBreakpointPopoverLazy} what="breakpoint editor" quiet onerror={() => (bpEditor = null)} props={{ path: bpEditor.path, line: bpEditor.line, anchorX: bpEditor.x, anchorY: bpEditor.y, onclose: () => (bpEditor = null) }} />
{/if}

{#if hunkPopover}
  {@const hTab = ide.tabs.find((t) => t.path === hunkPopover?.path)}
  {@const hBase = hTab ? diffBaseFor(hTab) : null}
  {@const hList = hTab && hBase != null ? extractGutterHunks(hBase, hTab.content, ide.relativePath(hTab.path)) : []}
  {@const hRef = hunkPopover.path !== stagedPrimaryTab?.path && hunkPopover.path === stagedSplitTab?.path ? splitEditorRef : primaryEditorRef}
  {#if hTab && hBase != null && hList.length > 0}
    <LazyMount lazy={IdeGitHunkPopoverLazy} what="git hunk details" quiet onerror={() => (hunkPopover = null)} props={{
      path: hTab.path, hunks: hList, hunkIndex: hunkPopover.hunkIndex, anchorX: hunkPopover.x, anchorY: hunkPopover.y, readOnly: Boolean(hTab.readOnly || !ide.canEditCollab),
      onselecthunk: (nextIdx: number) => {
        if (hunkPopover) hunkPopover = { ...hunkPopover, hunkIndex: nextIdx };
        const nextH = hList[nextIdx];
        if (nextH) hRef?.goToLine(nextH.anchorLine, 1);
      },
      onrevert: (h: IdeGutterHunk) => {
        const nextText = revertHunkInText(hBase, hTab.content, h.index);
        if (nextText !== hTab.content && !hRef?.replaceBufferContent?.(nextText)) ide.updateBuffer(hTab.path, nextText);
      },
      onaskagent: (h: IdeGutterHunk) => {
        ideAgent.attachGitHunk({ path: hTab.path, startLine: h.startLine, endLine: h.endLine, anchorLine: h.anchorLine, addedCount: h.addedLines.length, removedCount: h.removedLines.length, unifiedDiff: h.unifiedDiff }, { openPanel: true });
        focusAgentComposer();
      },
      onclose: () => (hunkPopover = null),
    }} />
  {/if}
{/if}

{#if structureOpen}
  <LazyMount
    lazy={IdeFileStructureModalLazy} what="file structure" quiet onerror={() => (structureOpen = false)}
    props={{
      open: structureOpen, fileName: ide.activeTab?.name ?? '', symbols: effectiveSymbols, currentLine: ide.cursorInfo.line,
      onjump: (line: number, col: number) => {
        if (ide.activePath) pushIdeNavLocation(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col);
        if (effectiveViewMode === 'table' && ide.activePath) idePreviewMode.set(ide.activePath, 'source');
        requestAnimationFrame(() => primaryEditorRef?.goToLine(line, col));
      },
      onclose: () => (structureOpen = false),
    }}
  />
{/if}

{#if ide.renameState.open}
  <LazyMount lazy={IdeRenameModalLazy} what="rename symbol dialog" quiet onerror={() => (ide.renameState.open = false)} />
{/if}

<style>
  .ide-editor-area { display: flex; flex-direction: column; flex: 1; min-inline-size: 0; min-block-size: 0; background: var(--bg); }
  .ide-subbar-slot { block-size: 30px; flex-shrink: 0; background: var(--surface); border-block-end: 1px solid var(--border); }
  .ide-panes { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; }
  .ide-panes.split-down { flex-direction: column; }
  .ide-pane { flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  .ide-split-resizer {
    appearance: none; border: none; padding: 0; margin: 0;
    inline-size: 4px; flex-shrink: 0; cursor: col-resize; background: var(--border);
    transition: background var(--dur-fast) var(--ease-out);
  }
  .ide-panes.split-down .ide-split-resizer { inline-size: 100%; block-size: 4px; cursor: row-resize; }
  .ide-split-resizer:hover, .ide-split-resizer:focus-visible { background: var(--accent-solid); }
  .split-head {
    display: flex; align-items: center; justify-content: space-between; padding: var(--sp-1) var(--sp-4);
    block-size: 28px; flex-shrink: 0; background: var(--surface); border-block-end: 1px solid var(--border);
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim);
  }
  .split-title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .split-body { flex: 1; min-block-size: 0; display: flex; flex-direction: column; }
  .ide-manifest-tabs {
    display: flex; align-items: center; gap: 2px; block-size: 26px; padding-inline: var(--sp-3);
    background: var(--surface); border-block-start: 1px solid var(--border); flex-shrink: 0;
  }
  .ide-man-tab {
    block-size: 22px; padding-inline: var(--sp-3); background: transparent; border: none;
    border-radius: var(--radius-s); color: var(--text-dim); font-size: var(--fs-xs); cursor: pointer;
  }
  .ide-man-tab.active { background: var(--surface-2); color: var(--text); font-weight: 500; box-shadow: inset 0 -2px 0 var(--accent-solid); }
  .ide-empty-editor { flex: 1; display: flex; align-items: center; justify-content: center; min-block-size: 0; }
  @media (max-width: 640px) { .ide-panes { flex-direction: column; } }
</style>
