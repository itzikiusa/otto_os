<script module lang="ts">
  import type * as CmIdeThemeType from './cm-ide-theme';
  import type * as CmIdeHistType from './cm-ide-history';
  import type * as CmEditorSmartType from './cm-editor-smart';
  export type { CursorChangeInfo } from './cm-editor-helpers';

  let coreThemeMod: typeof CmIdeThemeType | null = null;
  let coreHistMod: typeof CmIdeHistType | null = null;
  let coreSmartMod: typeof CmEditorSmartType | null = null;
  export function registerIdeEditorCore(
    themeMod: typeof CmIdeThemeType,
    histMod?: typeof CmIdeHistType | null,
    smartMod?: typeof CmEditorSmartType | null,
  ): void {
    coreThemeMod = themeMod;
    if (histMod) coreHistMod = histMod;
    if (smartMod) coreSmartMod = smartMod;
  }
</script>

<script lang="ts">
  // CodeMirror 6 editor with LSP, multi-cursor, 1-click gutter breakpoints, 6 IDE color schemes & durable ⌘Z undo.
  import { onDestroy, untrack } from 'svelte';
  import { EditorView, lineNumbers, keymap, drawSelection, placeholder as cmPlaceholder } from '@codemirror/view';
  import { EditorSelection, EditorState, Compartment, Prec, Text, type StateEffect, type Extension } from '@codemirror/state';
  import { defaultKeymap, selectAll } from '@codemirror/commands';
  import { registerLiveParker, PERSIST_MAX_BYTES } from '../editor-history';
  import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
  import { completionKeymap, type CompletionSource } from '@codemirror/autocomplete';
  import { lintGutter, lintKeymap } from '@codemirror/lint';
  import { indentOnInput, bracketMatching, foldKeymap, defaultHighlightStyle, syntaxHighlighting } from '@codemirror/language';
  import { oneDark, oneDarkTheme } from '@codemirror/theme-one-dark';
  import { cmLangNow, cmLangPending, loadCmLang } from './cm-langs';
  import type { SqlDialectName } from '../sql-dialects';
  import { createChangeEmitter } from './changeEmitter';
  import type { IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem, IdeInlineValue, IdePeerCursor } from '../api/types';
  import type * as CmIdeMod from './cm-ide-extensions';
  import type * as CmIdeDepsMod from './cm-ide-deps';
  import {
    applyLinePrefixSuffixSync, buildEditorFindProviderSpec,
    buildReadOnlyExtension, clampSelectionToDoc, computeCursorChangeInfo, computeMinimalTextUpdate,
    createCompletionExtension, createCopyRecorder, createEditorStateCache, createIdeColdGutterPlaceholders, createPlainLineWatch,
    foldMarkerGutter, hasLongLine, langExtOf,
    lightEditorTheme, parkEditorHistory, placeholderEditorTheme,
    restoreOrCreateEditorState, revealEditorLine,
    splitSelectionIntoLinesSync, type CursorChangeInfo,
  } from './cm-editor-helpers';
  import { isIdeManifestPath } from './cm-ide-manifest';
  import { ui } from '../stores/ui.svelte';
  import { keyContext } from '../keys';
  import { registerFindProvider } from '../findProviders';

  const themeCompartment = new Compartment();
  type JumpTarget = { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number };

  interface Props {
    path: string; content: string; root: string; language?: string; readOnly?: boolean;
    onchange?: (value: string) => void; completionSource?: CompletionSource | null;
    autoTriggerGate?: ((state: EditorState, pos: number) => boolean) | null;
    minimal?: boolean; onsubmit?: () => void; onselect?: (s: { text: string; cursor: number }) => void;
    gotoLine?: number | null; gotoCol?: number | null; findOwner?: boolean; placeholder?: string;
    wrap?: boolean; keepStates?: boolean; historyKey?: ((path: string) => { key: string; persist: boolean } | null) | null;
    oncopy?: ((text: string) => void) | null; lsp?: boolean; sqlDialect?: SqlDialectName; highlightLineLimit?: number;
    ideMode?: boolean; connectionId?: string | null; projectFolders?: string[];
    savedContent?: string | null; gitBaseContent?: string | null;
    breakpoints?: IdeBreakpoint[]; breakpointsMuted?: boolean; blameByLine?: Record<number, string> | null;
    onToggleBreakpoint?: ((line: number) => void) | null; onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
    gutterItems?: IdeGutterItem[]; onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug' | 'run_keep_tc') => void) | null;
    pausedLine?: number | null; executionLine?: number | null; inlineValues?: IdeInlineValue[];
    peerCursors?: (IdePeerCursor | IdeCollabPeer)[]; keymapPreset?: 'jetbrains' | 'standard'; fontLigatures?: boolean;
    jumpToLine?: { line: number; column?: number | null; seq?: number } | number | null;
    onCursorChange?: ((info: CursorChangeInfo) => void) | null;
    onDefinitionJump?: ((target: JumpTarget) => void) | null; onImplementationJump?: ((target: JumpTarget) => void) | null;
    onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
    onFileStructure?: (() => void) | null; onJumpSymbol?: ((delta: -1 | 1) => void) | null;
    onNavigateBack?: (() => void) | null; onNavigateForward?: (() => void) | null;
    onQuickFix?: ((ctx: { line: number; column: number; symbol?: string; clientX?: number; clientY?: number }) => void) | null;
    onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string; newName?: string }) => void) | null;
    onFormat?: (() => void) | null; onToggleBlame?: (() => void) | null;
    onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
    onAgentAction?: ((mode: 'chat' | 'inline', sel: CmIdeMod.IdeEditorSelectionRange) => void) | null;
    onGoToLine?: (() => void) | null; onPasteFromHistory?: (() => void) | null;
    onOpenLocalHistory?: (() => void) | null; onOpenDepAnalyzer?: (() => void) | null;
    onOpenManifestIntention?: ((line: number, dep: CmIdeDepsMod.IdeDepItem | null) => void) | null;
    onManifestDepsUpdated?: ((items: CmIdeDepsMod.IdeDepItem[]) => void) | null;
  }

  let {
    path, content, root, language, readOnly = true, onchange, completionSource = null, autoTriggerGate = null,
    minimal = false, onsubmit, onselect, gotoLine = null, gotoCol = null, findOwner = true, placeholder = '',
    wrap = false, keepStates = false, historyKey = null, oncopy = null, lsp = false, sqlDialect = 'standard',
    highlightLineLimit = 0, ideMode = false, connectionId = null, projectFolders = [], savedContent = null, gitBaseContent = undefined,
    breakpoints = [], breakpointsMuted = false, blameByLine = null,
    onToggleBreakpoint = null, onEditBreakpoint = null, gutterItems = [], onGutterAction = null,
    pausedLine = null, executionLine = null, inlineValues = [], peerCursors = [], keymapPreset = 'jetbrains', fontLigatures, jumpToLine = null,
    onCursorChange = null, onDefinitionJump = null, onImplementationJump = null,
    onFindUsages = null, onFileStructure = null, onJumpSymbol = null, onNavigateBack = null, onNavigateForward = null,
    onQuickFix = null, onRenameSymbol = null, onFormat = null, onToggleBlame = null,
    onDiagnostics = null, onAgentAction = null, onGoToLine = null, onPasteFromHistory = null,
    onOpenLocalHistory = null, onOpenDepAnalyzer = null, onOpenManifestIntention = null, onManifestDepsUpdated = null,
  }: Props = $props();

  let idePrefs = $state<CmIdeThemeType.IdeEditorPreferences | null>(coreThemeMod ? coreThemeMod.readIdeEditorPreferences() : null);
  const effectivePausedLine = $derived(executionLine ?? pausedLine ?? null);
  const effectiveIdeScheme = $derived(
    coreThemeMod && idePrefs
      ? coreThemeMod.resolveEffectiveIdeScheme(idePrefs, ui.resolvedScheme)
      : (ui.resolvedScheme === 'dark' ? 'jetbrains-islands-dark' : 'jetbrains-islands-light'),
  );

  function themeExt(appScheme: 'light' | 'dark', plain = false): Extension {
    if (ideMode && coreThemeMod) {
      const prefs = idePrefs ?? coreThemeMod.readIdeEditorPreferences();
      const scheme = coreThemeMod.resolveEffectiveIdeScheme(prefs, appScheme);
      return coreThemeMod.createIdeSyntaxTheme({ dark: coreThemeMod.isDarkIdeScheme(scheme), plain, ideMode: true, scheme });
    }
    if (plain) return appScheme === 'dark' ? oneDarkTheme : lightEditorTheme;
    return appScheme === 'dark' ? oneDark : [lightEditorTheme, syntaxHighlighting(defaultHighlightStyle)];
  }

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let detachScrollTracker: (() => void) | null = null;
  let lspCompartment = new Compartment(), ideBpCompartment = new Compartment(), indentCompartment = new Compartment();
  let ideCompartment = new Compartment(), ideDepsCompartment = new Compartment();
  let readOnlyCompartment = new Compartment(), completionCompartment = new Compartment();
  let placeholderCompartment = new Compartment(), wrapCompartment = new Compartment(), langCompartment = new Compartment();
  let appliedReadOnly = true, appliedPlaceholder = '', appliedWrap = false, appliedLangReady = true;
  let appliedDialect: SqlDialectName = 'standard';
  let appliedLanguage: string | undefined = undefined;
  let livePath = '', liveSavedContent: string | null = null;
  let pendingScrollTop: number | null = null;
  let historySaveTimer: ReturnType<typeof setTimeout> | null = null;

  const stateCache = createEditorStateCache();
  const wrapExt = (on: boolean): Extension => (on ? EditorView.lineWrapping : []);
  const placeholderExt = (text: string): Extension => (text ? cmPlaceholder(text) : []);
  const indentExt = (forPath?: string): Extension =>
    minimal || !coreSmartMod ? [] : coreSmartMod.createSmartIndentExtension(
      () => coreSmartMod!.resolveIndentUnit(forPath ?? (livePath || path), language, (idePrefs ?? coreThemeMod?.readIdeEditorPreferences())?.tabSize ?? 4),
      () => langExtOf(forPath ?? (livePath || path), language),
    );

  let ideMod: typeof CmIdeMod | null = null;
  let ideModPromise: Promise<typeof CmIdeMod> | null = null;
  let ideMounted = false, appliedIdePath = '', appliedManifestDepsPath = '';
  let appliedKeymapPreset: 'jetbrains' | 'standard' = 'jetbrains';

  const ensureIdeMod = (): Promise<typeof CmIdeMod> =>
    ideMod ? Promise.resolve(ideMod) : (ideModPromise ??= import('./cm-ide-extensions').then((m) => ((ideMod = m), m)));

  function isIdeActive(): boolean {
    return Boolean(
      ideMode || savedContent != null || gitBaseContent != null || blameByLine != null || onAgentAction || breakpoints.length > 0 ||
      gutterItems.length > 0 || effectivePausedLine != null || inlineValues.length > 0 || peerCursors.length > 0 ||
      onToggleBreakpoint || onEditBreakpoint || onGutterAction || onDefinitionJump || onImplementationJump ||
      onFindUsages || onFileStructure || onQuickFix || onRenameSymbol || onFormat,
    );
  }

  const getIdeCallbacks = (): CmIdeMod.IdeEditorCallbacks => ({
    onToggleBreakpoint, onEditBreakpoint, onGutterAction, onDefinitionJump, onImplementationJump,
    onFindUsages, onFileStructure, onJumpSymbol, onNavigateBack, onNavigateForward,
    onQuickFix, onRenameSymbol, onFormat, onToggleBlame, onDiagnostics, onAgentAction,
    onGoToLine, onPasteFromHistory,
  });

  const effectiveDiffBaseline = (): string | null =>
    gitBaseContent !== undefined ? gitBaseContent : savedContent;

  const effectiveDiffBaselineSource = (): 'git' | 'saved' =>
    gitBaseContent !== undefined ? 'git' : 'saved';

  const buildIdeOpts = (forPath?: string): CmIdeMod.CreateIdeExtensionsOptions => ({
    filePath: forPath ?? (livePath || path), language, root, connectionId, projectFolders,
    breakpoints, breakpointsMuted, gutterItems, pausedLine: effectivePausedLine, inlineValues, peerCursors,
    diskSavedContent: savedContent, savedContent: effectiveDiffBaseline(), baselineSource: effectiveDiffBaselineSource(), blameByLine, keymapPreset, getCallbacks: getIdeCallbacks,
  });

  const coldGutters = createIdeColdGutterPlaceholders();
  const buildIdeBpExtIfLoaded = (forPath?: string): Extension =>
    !isIdeActive() ? [] : ideMod ? ideMod.createIdeGutterExtensions(buildIdeOpts(forPath)) : coldGutters.bp;

  function buildIdeExtIfLoaded(forPath?: string): Extension {
    if (!isIdeActive()) { ideMounted = false; return []; }
    if (!ideMod) { ideMounted = false; return coldGutters.diff; }
    const p = forPath ?? (livePath || path);
    ideMounted = true; appliedIdePath = p; appliedKeymapPreset = keymapPreset;
    return ideMod.createIdeExtensions(buildIdeOpts(p));
  }

  function mountIdeExtensions(target: EditorView, forPath?: string): void {
    if (!isIdeActive()) return;
    const p = forPath ?? (livePath || path);
    const apply = () => {
      if (view === target && isIdeActive() && (!ideMounted || appliedIdePath !== p)) {
        target.dispatch({ effects: [ideBpCompartment.reconfigure(buildIdeBpExtIfLoaded(p)), ideCompartment.reconfigure(buildIdeExtIfLoaded(p))] });
      }
    };
    if (ideMod) apply(); else void ensureIdeMod().then(apply);
    if (ideMode || appliedManifestDepsPath) {
      void import('./cm-editor-ondemand').then((m) => m.mountIdeManifestDeps({
        target, forPath: p, ideMode, appliedPath: appliedManifestDepsPath,
        setAppliedPath: (next) => { appliedManifestDepsPath = next; },
        isCurrent: () => view === target && livePath === p,
        ideDepsCompartment, root, connectionId, projectFolders,
        onApplyContent: (nextContent) => { applyAgentEdit(nextContent); },
        onOpenIntentionMenu: (ln, dep) => onOpenManifestIntention?.(ln, dep),
        onJumpLocation: (loc) => onDefinitionJump?.(loc),
        onAddAgentContext: () => { const r = getSelectionRange(); if (r) onAgentAction?.('chat', r); },
        onOpenLocalHistory: () => onOpenLocalHistory?.(),
        onOpenDepAnalyzer: () => onOpenDepAnalyzer?.(),
        onToggleBlame,
        onDepsUpdated: (items) => untrack(() => onManifestDepsUpdated?.(items)),
      }));
    }
  }

  const fallbackBpGutterExt = (): Extension =>
    coreHistMod && (ideMode || onToggleBreakpoint || onEditBreakpoint)
      ? coreHistMod.createFallbackBpGutterHandlers({
          isIdeMounted: () => ideMounted, getToggle: () => onToggleBreakpoint, getEdit: () => onEditBreakpoint,
        })
      : [];

  let appliedPlain = false;
  const plainWatcher = createPlainLineWatch({
    getLimit: () => highlightLineLimit,
    isPlain: () => appliedPlain,
    setPlain: (target, on) => {
      if (on === appliedPlain) return;
      appliedPlain = on;
      queueMicrotask(() => {
        if (view === target) target.dispatch({ effects: themeCompartment.reconfigure(themeExt(untrack(() => ui.resolvedScheme), appliedPlain)) });
      });
    },
  });

  let sel: { from: number; to: number; startLine: number; endLine: number } | null = $state(null);

  function emitSelection(state: EditorState): void {
    const { from, to, head } = state.selection.main;
    sel = from === to ? null : { from, to, startLine: state.doc.lineAt(from).number, endLine: state.doc.lineAt(to).number };
    if (onCursorChange) onCursorChange(computeCursorChangeInfo(state));
    if (!onselect) return;
    let text: string | undefined = from === to ? '' : undefined;
    onselect({ get text() { return (text ??= state.sliceDoc(from, to)); }, cursor: head });
  }

  const selectionListener = EditorView.updateListener.of((u) => {
    if (u.selectionSet || u.docChanged) {
      emitSelection(u.state);
      if (u.selectionSet && livePath && coreSmartMod) {
        const main = u.state.selection.main;
        coreSmartMod.saveIdeViewPosition(root, livePath, { anchor: main.anchor, head: main.head, scrollTop: u.view.scrollDOM.scrollTop });
      }
    }
  });
  let lastEmitted: string | null = null;
  const changes = createChangeEmitter({
    threshold: 32 * 1024, delayMs: 120,
    read: () => view?.state.doc.toString() ?? null,
    emit: (value) => { lastEmitted = value; onchange?.(value); },
  });
  const changeListener = EditorView.updateListener.of((u) => { if (u.docChanged && onchange) changes.changed(u.state.doc.length); });
  const changeFlushHandlers = EditorView.domEventHandlers({
    blur: () => { changes.flush(); parkLive(); return false; },
    keydown: (e) => { if ((e.metaKey || e.ctrlKey) && changes.pending) changes.flush(); return false; },
  });

  async function sendToAgent(): Promise<void> {
    if (!sel || !view) return;
    const text = view.state.sliceDoc(sel.from, sel.to), { startLine, endLine } = sel;
    const m = await import('./cm-editor-ondemand');
    await m.sendSelectionToAgentSession(path, language, startLine, endLine, text);
  }

  const cmLangFor = (filePath: string, hint?: string): Extension | null => cmLangNow(langExtOf(filePath, hint), appliedDialect);

  function ensureLang(filePath: string): void {
    if (appliedLangReady) return;
    const target = view;
    loadCmLang(langExtOf(filePath, language)).then(() => {
      if (!view || view !== target || livePath !== filePath || appliedLangReady) return;
      const langExt = cmLangFor(filePath, language);
      if (!langExt) return;
      appliedLangReady = true;
      view.dispatch({ effects: langCompartment.reconfigure(langExt) });
    }, () => {});
  }

  function handleLspJump(targetPath: string, line: number, column: number, absLivePath: string): void {
    if (targetPath === livePath || (absLivePath && targetPath === absLivePath)) revealLine(line, column);
    else onDefinitionJump?.({ path: targetPath, line, column });
  }

  let attachedLspRoot = '', attachedLspPath = '', lspAttachSeq = 0;

  async function attachLsp(editorView: EditorView, filePath: string, rootPath: string, connId?: string | null): Promise<void> {
    if (!rootPath && !connId) return;
    const seq = ++lspAttachSeq;
    const m = await import('./cm-editor-ondemand');
    if (view !== editorView || livePath !== filePath || seq !== lspAttachSeq) return;
    const curFolders = projectFolders;
    attachedLspPath = filePath;
    attachedLspRoot = m.resolveIdeLspModuleRoot(rootPath, filePath, curFolders);
    return m.attachCodeEditorLsp({
      editorView, filePath, rootPath, connId, projectFolders: curFolders, ideMode, language, lspCompartment,
      isCurrent: () => view === editorView && livePath === filePath && seq === lspAttachSeq,
      ensureIdeMod, onLocationJump: handleLspJump, onDiagnostics,
    });
  }

  function teardownEditor(): void {
    changes.cancel(); plainWatcher.cancel();
    detachScrollTracker?.(); detachScrollTracker = null;
    ideMounted = false; appliedIdePath = ''; appliedManifestDepsPath = '';
    attachedLspRoot = ''; attachedLspPath = ''; ++lspAttachSeq;
    try { view?.destroy(); } catch { /* ignore */ }
    view = null;
    if (keyContext.openFind === openEditorSearch) keyContext.openFind = null;
  }

  const completionExt = (): Extension => createCompletionExtension(completionSource, autoTriggerGate, () => langExtOf(path, language));
  const submitKeymap = keymap.of([{ key: 'Mod-Enter', run: () => { if (!onsubmit) return false; changes.flush(); onsubmit(); return true; } }]);
  function openEditorSearch(): void { if (view) openSearchPanel(view); }

  export function openSearch(): void { openEditorSearch(); }
  export function focus(): void { view?.focus(); }
  export function goToLine(line: number, col?: number): void { revealLine(line, col); }

  export function insertText(text: string): void {
    if (!view || readOnly) return;
    view.dispatch(view.state.replaceSelection(text), { scrollIntoView: true, userEvent: 'input.paste' });
    changes.flush(); parkLive(); view.focus();
  }

  export function getSelectionRange(): CmIdeMod.IdeEditorSelectionRange | null {
    if (!view) return null;
    return ideMod ? ideMod.selectionOrCurrentLine(view.state) : (coreHistMod ? coreHistMod.fallbackSelectionRange(view.state) : null);
  }

  export function replaceBufferContent(nextContent: string, range?: { from: number; to: number }): boolean {
    if (!view) return false;
    const normInsert = nextContent.indexOf('\r') >= 0 ? nextContent.replace(/\r\n?/g, '\n') : nextContent;
    const docLen = view.state.doc.length;
    const from = range ? Math.max(0, Math.min(docLen, range.from)) : 0;
    const to = range ? Math.max(from, Math.min(docLen, range.to)) : docLen;
    const newDocLen = Math.max(0, docLen - (to - from) + normInsert.length);
    const main = view.state.selection.main;
    const nextCursor = range
      ? Math.max(0, Math.min(newDocLen, from + normInsert.length))
      : Math.max(0, Math.min(newDocLen, main.head));
    view.dispatch({
      changes: { from, to, insert: normInsert },
      selection: EditorSelection.cursor(nextCursor),
      scrollIntoView: Boolean(range),
      userEvent: 'input.complete',
    });
    changes.flush(); parkLive();
    return true;
  }

  export function applyAgentEdit(nextContent: string, range?: { from: number; to: number }): boolean {
    if (!view || readOnly) return false;
    return replaceBufferContent(nextContent, range);
  }

  export function applyQuickFixEdits(edits: CmIdeMod.IdeConcreteTextEdit[]): boolean {
    if (!view || readOnly || !edits.length) return false;
    let ok = true;
    if (ideMod) ok = ideMod.applyConcreteEditsToView(view, edits, livePath || path);
    else coreHistMod?.applyFallbackQuickFixEdits(view, edits);
    if (ok) { changes.flush(); parkLive(); view.focus(); }
    return ok;
  }

  export function getAvailableQuickFixes(line?: number, symbol?: string): {
    clientFixes: CmIdeMod.ClientDiagQuickFix[]; deterministicFixes: CmIdeMod.IdeQuickFixAction[]; docText: string;
  } {
    if (!view || !ideMod) return { clientFixes: [], deterministicFixes: [], docText: view?.state.doc.toString() ?? '' };
    const docText = view.state.doc.toString(), ln = line ?? view.state.doc.lineAt(view.state.selection.main.head).number;
    return {
      clientFixes: ideMod.getIdeQuickFixesAtLine(view.state, ln),
      deterministicFixes: ideMod.computeDeterministicQuickFixes(docText, livePath || path, language, ln, symbol),
      docText,
    };
  }

  export function applyClientDiagFix(fix: CmIdeMod.ClientDiagQuickFix): boolean {
    if (!view || readOnly || !ideMod) return false;
    ideMod.applyClientQuickFix(view, fix);
    changes.flush(); parkLive(); view.focus();
    return true;
  }

  export function applyPrefixSuffix(prefix: string, suffix: string): void {
    if (!view || readOnly) return;
    if (applyLinePrefixSuffixSync(view, prefix, suffix)) { changes.flush(); parkLive(); }
    view.focus();
  }

  export function splitSelectionToLines(): void {
    if (!view) return;
    splitSelectionIntoLinesSync(view);
    view.focus();
  }

  let skipDestroyPark = false;

  export function evictPaths(paths: Iterable<string> | 'all'): void {
    stateCache.evict(paths, historyKey);
    if (paths === 'all') {
      skipDestroyPark = true;
      coreHistMod?.evictSingletonIdeState('all');
      coreHistMod?.forgetIdeUndoJournal('all');
      ideMod?.disposePooledLspClients({ forceAll: true });
      return;
    }
    for (const p of paths) {
      if (p === livePath) skipDestroyPark = true;
      coreHistMod?.evictSingletonIdeState([p]);
      coreHistMod?.forgetIdeUndoJournal(p);
    }
  }

  export function notifyDidSave(): void {
    if (view && ideMod) ideMod.notifyLspDidSave(view);
    if (view && ideMode && !readOnly && livePath) {
      const savedDoc = view.state.doc.toString();
      liveSavedContent = savedDoc;
      coreHistMod?.persistIdeUndoJournal(livePath, view.state, savedDoc, view.scrollDOM.scrollTop);
    }
  }

  export function disposeLspClients(rootPath?: string): void {
    ideMod?.disposePooledLspClients({ rootPath, forceAll: true });
  }

  export function reconnectLspClients(rootPath?: string): void {
    ideMod?.disposePooledLspClients({ rootPath, forceAll: true });
    if (!view || !isIdeActive() || !ideMod) return;
    const p = livePath || path;
    const normFilter = rootPath ? rootPath.replace(/\\/g, '/').replace(/\/+$/, '') : '';
    const normPath = p.replace(/\\/g, '/');
    if (!normFilter || normPath === normFilter || normPath.startsWith(`${normFilter}/`)) {
      view.dispatch({ effects: ideCompartment.reconfigure(buildIdeExtIfLoaded(p)) });
    }
  }

  $effect(() => registerFindProvider(buildEditorFindProviderSpec(() => view)));

  const readOnlyExt = (ro: boolean, hk?: { key: string; persist: boolean } | null): Extension =>
    buildReadOnlyExtension(ro, ideMode, hk, splitSelectionToLines);

  function createState(filePath: string, fileContent: string): EditorState {
    appliedDialect = sqlDialect; appliedReadOnly = readOnly;
    const hk = readOnly ? null : (historyKey?.(filePath) ?? null), limit = highlightLineLimit;
    const doc = limit > 0 ? Text.of(fileContent.split(/\r\n?|\n/)) : null;
    appliedPlain = !!doc && hasLongLine(doc, limit);
    appliedLanguage = language;
    const langExt = cmLangFor(filePath, language);
    appliedLangReady = langExt != null || !cmLangPending(langExtOf(filePath, language));

    const baseExtensions: Extension[] = [
      ...(minimal ? [] : [lineNumbers(), ideBpCompartment.of(buildIdeBpExtIfLoaded(filePath)), foldMarkerGutter()]),
      indentOnInput(), indentCompartment.of(indentExt(filePath)), bracketMatching(), lintGutter(),
      ideCompartment.of(buildIdeExtIfLoaded(filePath)),
      ideDepsCompartment.of(ideMode && isIdeManifestPath(filePath) ? coldGutters.deps : []),
      drawSelection(),
      readOnlyCompartment.of(readOnlyExt(readOnly, hk)), completionCompartment.of(completionExt()),
      placeholderCompartment.of(placeholderExt((appliedPlaceholder = placeholder))),
      wrapCompartment.of(wrapExt((appliedWrap = wrap || Boolean(ideMode && idePrefs?.wordWrap)))),
      placeholderEditorTheme, search({ top: false }),
      themeCompartment.of(themeExt(ui.resolvedScheme, appliedPlain)),
      lspCompartment.of([]), selectionListener, changeListener,
      ...(limit > 0 ? [plainWatcher.extension] : []),
      changeFlushHandlers, submitKeymap,
      ...(findOwner
        ? [EditorView.domEventHandlers({
            focus: () => { keyContext.openFind = openEditorSearch; return false; },
            blur: () => { if (keyContext.openFind === openEditorSearch) keyContext.openFind = null; return false; },
          })]
        : []),
      Prec.highest(keymap.of([{ key: 'Mod-a', run: selectAll }])),
      keymap.of([...defaultKeymap, ...searchKeymap, ...lintKeymap, ...completionKeymap, ...foldKeymap]),
      langCompartment.of(langExt ?? []), fallbackBpGutterExt(),
      ...(hk || (ideMode && !readOnly) ? [historySaver] : []),
      ...(oncopy || ideMode ? [copyRecorder] : []),
    ];

    const restored = restoreOrCreateEditorState({
      filePath, fileContent, doc, savedContent, baseExtensions, ideMode, readOnly, hk, coreHistMod,
      onRestoredDraft: (draft) => queueMicrotask(() => { lastEmitted = draft; onchange?.(draft); }),
    });
    if (restored.scrollTop != null) pendingScrollTop = restored.scrollTop;
    return restored.state;
  }

  function parkHistory(forPath: string, state: EditorState, scrollTop: number, diskSaved?: string | null): void {
    const resolvedSaved = diskSaved !== undefined ? diskSaved : (forPath === livePath ? liveSavedContent : null);
    parkEditorHistory({
      forPath, state, scrollTop, readOnly, ideMode,
      diskSaved: resolvedSaved, langReady: appliedLangReady, language: appliedLanguage,
      coreHistMod, historyKey,
    });
  }

  function parkLive(): void {
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    if (!view) return;
    const main = view.state.selection.main, top = view.scrollDOM.scrollTop;
    coreSmartMod?.saveIdeViewPosition(root, livePath, { anchor: main.anchor, head: main.head, scrollTop: top }, { immediate: true });
    parkHistory(livePath, view.state, top, liveSavedContent);
  }

  function applyPendingScroll(): void {
    const top = pendingScrollTop;
    pendingScrollTop = null;
    if (top != null && top > 0) {
      requestAnimationFrame(() => {
        if (!view) return;
        const maxScroll = Math.max(0, view.scrollDOM.scrollHeight - view.scrollDOM.clientHeight);
        view.scrollDOM.scrollTop = maxScroll > 0 ? Math.min(top, maxScroll) : top;
      });
    }
  }

  let prevScrolledExecKey = '';
  function scrollToPausedLineIfNeeded(forPath: string): void {
    const p = effectivePausedLine;
    if (!view || p == null || p < 1 || view.state.doc.lines === 0) return;
    prevScrolledExecKey = `${forPath}:${p}`;
    const ln = Math.max(1, Math.min(view.state.doc.lines, p));
    view.dispatch({ effects: EditorView.scrollIntoView(view.state.doc.line(ln).from, { y: 'center' }) });
  }

  const historySaver = EditorView.updateListener.of((u) => {
    if (!u.docChanged) return;
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    if (u.state.doc.length > PERSIST_MAX_BYTES) return;
    historySaveTimer = setTimeout(() => {
      historySaveTimer = null;
      if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop, liveSavedContent);
    }, ideMode ? 600 : 1500);
  });

  const copyRecorder = createCopyRecorder((text) => {
    if (ideMode) coreHistMod?.recordIdeClipboardEntry(text, livePath || path);
    oncopy?.(text);
  });

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string, connId?: string | null): void {
    parkLive();
    stateCache.drainAndClear((p, st, evSaved) => parkHistory(p, st, 0, evSaved));
    teardownEditor();
    lspCompartment = new Compartment(); ideBpCompartment = new Compartment(); indentCompartment = new Compartment();
    ideCompartment = new Compartment(); ideDepsCompartment = new Compartment();
    readOnlyCompartment = new Compartment(); completionCompartment = new Compartment();
    placeholderCompartment = new Compartment(); wrapCompartment = new Compartment();
    langCompartment = new Compartment();
    sel = null; skipDestroyPark = false; livePath = filePath; liveSavedContent = savedContent;

    view = new EditorView({ state: createState(filePath, fileContent), parent: el });
    detachScrollTracker = coreSmartMod ? coreSmartMod.attachScrollPositionTracker(view, () => ({ root: rootPath, filePath: livePath })) : null;
    if (gotoLine == null) {
      const savedPos = coreSmartMod?.getIdeViewPosition(rootPath, filePath) ?? null;
      if (savedPos) {
        if (pendingScrollTop == null) pendingScrollTop = savedPos.scrollTop;
        const curSel = view.state.selection.main;
        if (curSel.anchor === 0 && curSel.head === 0 && (savedPos.anchor > 0 || savedPos.head > 0)) {
          view.dispatch({ selection: clampSelectionToDoc(view.state.doc.length, savedPos.anchor, savedPos.head) });
        }
      }
    }
    applyPendingScroll();
    ensureLang(filePath);
    mountIdeExtensions(view, filePath);
    if (!completionSource && lsp) void attachLsp(view, filePath, rootPath, connId);
    if (gotoLine != null) revealLine(gotoLine, gotoCol);
    else scrollToPausedLineIfNeeded(filePath);
    const built = view;
    untrack(() => emitSelection(built.state));
  }

  function swapState(fromPath: string, toPath: string, toContent: string): void {
    if (!view) return;
    changes.flush();
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;

    const prevScrollTop = view.scrollDOM.scrollTop, prevSel = view.state.selection.main, outgoingSaved = liveSavedContent;
    stateCache.saveLightMeta(fromPath, prevScrollTop, prevSel.anchor, prevSel.head);
    coreSmartMod?.saveIdeViewPosition(root, fromPath, { anchor: prevSel.anchor, head: prevSel.head, scrollTop: prevScrollTop }, { immediate: true });
    attachedLspRoot = ''; attachedLspPath = ''; appliedManifestDepsPath = ''; ++lspAttachSeq;
    const nextColdDeps = ideMode && isIdeManifestPath(toPath) ? coldGutters.deps : [];
    view.dispatch({ effects: [lspCompartment.reconfigure([]), ideDepsCompartment.reconfigure(nextColdDeps)] });

    parkHistory(fromPath, view.state, prevScrollTop, outgoingSaved);
    stateCache.stashOutgoing(
      fromPath,
      { state: view.state, scroll: view.scrollSnapshot(), chars: view.state.doc.length, dialect: appliedDialect, langReady: appliedLangReady, language: appliedLanguage, savedContent: outgoingSaved },
      ideMode,
      (evPath, evState, evSaved) => parkHistory(evPath, evState, 0, evSaved),
    );

    skipDestroyPark = false; livePath = toPath; liveSavedContent = savedContent;
    const kept = stateCache.takeKept(toPath);
    if (!kept) {
      const st = createState(toPath, toContent);
      const savedPos = coreSmartMod?.getIdeViewPosition(root, toPath) ?? stateCache.lightTabMeta.get(toPath) ?? null;
      if (savedPos) {
        if (pendingScrollTop == null) pendingScrollTop = savedPos.scrollTop;
        view.setState(st.update({ selection: clampSelectionToDoc(st.doc.length, savedPos.anchor, savedPos.head) }).state);
      } else {
        view.setState(st);
      }
      applyPendingScroll();
      ensureLang(toPath);
      mountIdeExtensions(view, toPath);
    } else {
      view.setState(kept.state);
      plainWatcher.cancel();
      const keptDoc = kept.state.doc;
      const stale = keptDoc.length !== toContent.length || keptDoc.toString() !== toContent;
      appliedPlain = highlightLineLimit > 0 && (stale ? hasLongLine(Text.of(toContent.split(/\r\n?|\n/)), highlightLineLimit) : hasLongLine(keptDoc, highlightLineLimit));
      const effects: StateEffect<unknown>[] = [
        readOnlyCompartment.reconfigure(readOnlyExt((appliedReadOnly = readOnly), readOnly ? null : (historyKey?.(toPath) ?? null))),
        indentCompartment.reconfigure(indentExt(toPath)),
        completionCompartment.reconfigure(completionExt()),
        placeholderCompartment.reconfigure(placeholderExt((appliedPlaceholder = placeholder))),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap || Boolean(ideMode && idePrefs?.wordWrap)))),
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        ideDepsCompartment.reconfigure(nextColdDeps),
        ...(!stale ? [kept.scroll] : []),
      ];
      if (ideMod && isIdeActive()) {
        effects.push(ideBpCompartment.reconfigure(buildIdeBpExtIfLoaded(toPath)), ideCompartment.reconfigure(buildIdeExtIfLoaded(toPath)));
      }
      appliedDialect = kept.dialect; appliedLangReady = kept.langReady; appliedLanguage = kept.language;
      if (sqlDialect !== kept.dialect || !kept.langReady || language !== kept.language) {
        appliedDialect = sqlDialect; appliedLanguage = language;
        const langExt = cmLangFor(toPath, language);
        appliedLangReady = langExt != null || !cmLangPending(langExtOf(toPath, language));
        effects.push(langCompartment.reconfigure(langExt ?? []));
      }
      if (stale) {
        const upd = computeMinimalTextUpdate(keptDoc.toString(), toContent, kept.state.selection.main.anchor, kept.state.selection.main.head);
        view.dispatch({ effects, ...(upd ? { changes: { from: upd.from, to: upd.to, insert: upd.insert }, selection: EditorSelection.range(upd.anchor, upd.head) } : {}) });
        pendingScrollTop = (coreSmartMod?.getIdeViewPosition(root, toPath) ?? stateCache.lightTabMeta.get(toPath))?.scrollTop ?? 0;
        applyPendingScroll();
      } else {
        view.dispatch({ effects });
      }
      ensureLang(toPath);
      mountIdeExtensions(view, toPath);
    }
    scrollToPausedLineIfNeeded(toPath);
    if (!completionSource && lsp) void attachLsp(view, toPath, root, connectionId);
    prevCompletion = completionSource;
    emitSelection(view.state);
  }

  const revealLine = (line: number, col?: number | null): void => revealEditorLine(() => view, line, col);
  let prevPath = '', prevRoot = '', prevConnId: string | null = null, prevContent = '';

  $effect(() => {
    const el = container, curPath = path, curContent = content, curRoot = root, curConn = connectionId ?? null;
    if (!el) return;
    const contentEchoed = curContent === lastEmitted;
    if (view && keepStates && curPath !== prevPath && curRoot === prevRoot && curConn === prevConnId) {
      const fromPath = prevPath;
      prevPath = curPath; prevContent = curContent;
      untrack(() => swapState(fromPath, curPath, curContent));
    } else if (!view || curPath !== prevPath || curRoot !== prevRoot || curConn !== prevConnId) {
      prevPath = curPath; prevRoot = curRoot; prevConnId = curConn; prevContent = curContent;
      buildEditor(el, curPath, curContent, curRoot, curConn);
    } else if (curContent !== prevContent && !changes.pending && (!contentEchoed || view.state.doc.toString() !== curContent)) {
      prevContent = curContent;
      const upd = computeMinimalTextUpdate(view.state.doc.toString(), curContent, view.state.selection.main.anchor, view.state.selection.main.head);
      if (upd) view.dispatch({ changes: { from: upd.from, to: upd.to, insert: upd.insert }, selection: EditorSelection.range(upd.anchor, upd.head) });
    } else if (curContent !== prevContent) {
      prevContent = curContent;
    }
  });

  $effect(() => {
    if (!ideMode || typeof window === 'undefined') return;
    if (!idePrefs && coreThemeMod) idePrefs = coreThemeMod.readIdeEditorPreferences();
    const onPrefs = () => {
      if (coreThemeMod) idePrefs = coreThemeMod.readIdeEditorPreferences();
      if (!view) return;
      const effects: StateEffect<unknown>[] = [
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        indentCompartment.reconfigure(indentExt()),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap || Boolean(ideMode && idePrefs?.wordWrap)))),
      ];
      if (ideMod && ideMounted && isIdeActive()) effects.push(ideCompartment.reconfigure(buildIdeExtIfLoaded()));
      view.dispatch({ effects });
    };
    window.addEventListener('otto:ide-prefs-changed', onPrefs);
    return () => window.removeEventListener('otto:ide-prefs-changed', onPrefs);
  });

  function syncIdeEffect(
    needMount: boolean,
    makeEffect: (m: typeof CmIdeMod) => StateEffect<unknown> | StateEffect<unknown>[],
  ): void {
    untrack(() => {
      if (!view) return;
      if (!ideMounted && needMount) mountIdeExtensions(view);
      else if (ideMod && ideMounted) view.dispatch({ effects: makeEffect(ideMod) });
    });
  }
  $effect(() => {
    const diskSaved = savedContent;
    const diffBase = gitBaseContent !== undefined ? gitBaseContent : diskSaved;
    const baseSrc: 'git' | 'saved' = gitBaseContent !== undefined ? 'git' : 'saved';
    if (livePath === path) liveSavedContent = diskSaved;
    syncIdeEffect(
      diffBase != null || diskSaved != null,
      (m) => [
        m.setSavedContentEffect.of(diffBase),
        m.setDiskSavedContentEffect.of(diskSaved),
        m.setBaselineSourceEffect.of(baseSrc),
      ],
    );
  });
  $effect(() => { const v = blameByLine; syncIdeEffect(v != null, (m) => m.setBlameLinesEffect.of(v)); });
  $effect(() => { const b = breakpoints, mu = breakpointsMuted; syncIdeEffect(b.length > 0 || !!onToggleBreakpoint, (m) => m.setBreakpointsEffect.of({ breakpoints: b, muted: mu })); });
  $effect(() => { const v = gutterItems; syncIdeEffect(v.length > 0, (m) => m.setGutterItemsEffect.of(v)); });
  $effect(() => {
    const p = effectivePausedLine, iv = inlineValues, curPath = path;
    syncIdeEffect(p != null || iv.length > 0, (m) => m.setDebugDecorationsEffect.of({ pausedLine: p, inlineValues: iv }));
    const execKey = p != null && p >= 1 ? `${curPath}:${p}` : '';
    if (!execKey) { prevScrolledExecKey = ''; return; }
    if (execKey === prevScrolledExecKey) return;
    untrack(() => { if (view && livePath === curPath) scrollToPausedLineIfNeeded(curPath); });
  });
  $effect(() => { const v = peerCursors; syncIdeEffect(v.length > 0, (m) => m.setPeerCursorsEffect.of(v)); });
  $effect(() => {
    const preset = keymapPreset;
    if (!view || !ideMod || !ideMounted || preset === appliedKeymapPreset) return;
    appliedKeymapPreset = preset;
    untrack(() => { if (view) view.dispatch({ effects: ideCompartment.reconfigure(buildIdeExtIfLoaded()) }); });
  });
  $effect(() => {
    const folders = projectFolders, curRoot = root, curPath = path, curConn = connectionId ?? null;
    if (!view || !lsp || completionSource || !folders.length || (!curRoot && !curConn)) return;
    const target = view;
    void import('./cm-editor-ondemand').then((m) => {
      if (view !== target || livePath !== curPath) return;
      if (attachedLspPath === curPath && m.resolveIdeLspModuleRoot(curRoot, curPath, folders) !== attachedLspRoot) {
        if (ideMod && ideMounted && isIdeActive()) target.dispatch({ effects: ideCompartment.reconfigure(buildIdeExtIfLoaded(curPath)) });
        void attachLsp(target, curPath, curRoot, curConn);
      }
    });
  });
  let prevCompletion: CompletionSource | null = null;
  $effect(() => {
    const src = completionSource;
    if (!view || src === prevCompletion) return;
    prevCompletion = src; view.dispatch({ effects: completionCompartment.reconfigure(completionExt()) });
  });
  $effect(() => {
    const ro = readOnly;
    if (!view || ro === appliedReadOnly) return;
    appliedReadOnly = ro;
    view.dispatch({ effects: readOnlyCompartment.reconfigure(readOnlyExt(ro, ro ? null : untrack(() => historyKey?.(livePath) ?? null))) });
  });
  $effect(() => {
    const on = wrap || Boolean(ideMode && idePrefs?.wordWrap);
    if (!view || on === appliedWrap) return;
    appliedWrap = on; view.dispatch({ effects: wrapCompartment.reconfigure(wrapExt(on)) });
  });
  $effect(() => {
    const d = sqlDialect;
    if (!view || d === appliedDialect) return;
    appliedDialect = d;
    const langExt = untrack(() => cmLangFor(path, language));
    if (langExt) view.dispatch({ effects: langCompartment.reconfigure(langExt) });
  });
  $effect(() => {
    const l = language;
    if (!view || l === appliedLanguage) return;
    untrack(() => {
      if (!view) return;
      appliedLanguage = l;
      const langExt = cmLangFor(livePath, l);
      appliedLangReady = langExt != null || !cmLangPending(langExtOf(livePath, l));
      view.dispatch({ effects: [langCompartment.reconfigure(langExt ?? []), indentCompartment.reconfigure(indentExt(livePath))] });
      ensureLang(livePath);
    });
  });
  $effect(() => {
    const text = placeholder;
    if (!view || text === appliedPlaceholder) return;
    appliedPlaceholder = text; view.dispatch({ effects: placeholderCompartment.reconfigure(placeholderExt(text)) });
  });
  $effect(() => {
    const scheme = ui.resolvedScheme, _eff = effectiveIdeScheme; void _eff;
    if (view) view.dispatch({ effects: themeCompartment.reconfigure(themeExt(scheme, appliedPlain)) });
  });
  let prevGoto: string | null = null;
  $effect(() => {
    const line = gotoLine, col = gotoCol, key = `${line ?? ''}:${col ?? ''}`;
    if (!view || line == null || key === prevGoto) return;
    prevGoto = key; revealLine(line, col);
  });
  let prevJumpKey: string | null = null;
  $effect(() => {
    const j = jumpToLine;
    if (!view || j == null) return;
    const line = typeof j === 'number' ? j : j.line, col = typeof j === 'number' ? null : (j.column ?? null);
    const key = `${line}:${col ?? ''}:${typeof j === 'number' ? '' : String(j.seq ?? '')}`;
    if (key === prevJumpKey) return;
    prevJumpKey = key; revealLine(line, col);
  });

  const unregisterParker = registerLiveParker(() => { if (historySaveTimer !== null) parkLive(); });
  onDestroy(() => {
    unregisterParker(); changes.flush();
    if (!skipDestroyPark) {
      parkLive();
      stateCache.drainAndClear((p, st, evSaved) => parkHistory(p, st, 0, evSaved));
    } else {
      if (historySaveTimer !== null) { clearTimeout(historySaveTimer); historySaveTimer = null; }
      stateCache.evict('all', historyKey);
    }
    teardownEditor();
  });
</script>

<div class="code-editor-outer" class:is-ide={ideMode} data-lang={language ?? ''}
  data-ide-scheme={ideMode ? effectiveIdeScheme : undefined}
  style:--ide-editor-fs={ideMode ? `${idePrefs?.fontSize ?? 13}px` : undefined}
  style:--ide-editor-lh={ideMode ? String(idePrefs?.lineHeight ?? 1.55) : undefined}
  style:--ide-editor-lig={ideMode ? ((fontLigatures ?? idePrefs?.fontLigatures ?? true) ? 'normal' : 'none') : undefined}>
  <div class="code-editor-wrap" dir="ltr" bind:this={container}></div>
  {#if sel && !ideMode && !onAgentAction}<button class="send-to-agent-btn" onclick={sendToAgent} type="button">Send to agent ↗</button>{/if}
</div>

<style>
  .code-editor-outer { position: relative; inline-size: 100%; block-size: 100%; }
  .code-editor-outer.is-ide { background: var(--syn-bg); color: var(--syn-fg); }
  .code-editor-wrap { inline-size: 100%; block-size: 100%; overflow: auto; }
  .send-to-agent-btn {
    position: absolute; inset-block-start: 6px; inset-inline-end: 10px; z-index: var(--z-sticky);
    padding: 2px 10px; font-size: var(--fs-xs); font-family: var(--font-ui); font-weight: 500;
    color: var(--accent-contrast); background: var(--accent-solid); border: none;
    border-radius: var(--radius-s); cursor: pointer; box-shadow: var(--glass-shadow);
    transition: background var(--dur-fast) var(--ease-out); white-space: nowrap; user-select: none;
  }
  .send-to-agent-btn:hover { background: color-mix(in srgb, var(--accent-solid) 88%, var(--text)); }
  .send-to-agent-btn:active { background: color-mix(in srgb, var(--accent-solid) 80%, var(--bg)); }
  .code-editor-wrap :global(.cm-editor) { block-size: 100%; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.55; }
  .code-editor-outer.is-ide .code-editor-wrap :global(.cm-editor) {
    font-size: var(--ide-editor-fs, var(--fs-xs));
    line-height: var(--ide-editor-lh, 1.55);
    font-variant-ligatures: var(--ide-editor-lig, normal);
    background: var(--syn-bg);
  }
  .code-editor-wrap :global(.cm-scroller) { overflow: auto; }
  .code-editor-wrap :global(.cm-editor.cm-focused) { outline: none; }
  .code-editor-wrap:focus-within { box-shadow: inset 0 0 0 1px var(--accent-text); }
  .code-editor-outer.is-ide .code-editor-wrap:focus-within { box-shadow: none; }
</style>
