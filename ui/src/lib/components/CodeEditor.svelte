<script module lang="ts">
  import type * as CmIdeThemeType from './cm-ide-theme';
  import type * as CmIdeHistType from './cm-ide-history';
  export type { CursorChangeInfo } from './cm-editor-helpers';

  let coreThemeMod: typeof CmIdeThemeType | null = null;
  let coreHistMod: typeof CmIdeHistType | null = null;
  export function registerIdeEditorCore(themeMod: typeof CmIdeThemeType, histMod?: typeof CmIdeHistType | null): void {
    coreThemeMod = themeMod;
    if (histMod) coreHistMod = histMod;
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
    buildReadOnlyExtension, computeCursorChangeInfo, createCompletionExtension,
    createEditorStateCache,
    foldMarkerGutter, hasLongLine, langExtOf,
    lightEditorTheme, parkEditorHistory, placeholderEditorTheme, restoreOrCreateEditorState,
    revealEditorLine, splitSelectionIntoLinesSync, type CursorChangeInfo,
  } from './cm-editor-helpers';
  import { ui } from '../stores/ui.svelte';
  import { keyContext } from '../keys';
  import { registerFindProvider } from '../findProviders';

  const themeCompartment = new Compartment();

  interface Props {
    path: string; content: string; root: string; language?: string; readOnly?: boolean;
    onchange?: (value: string) => void; completionSource?: CompletionSource | null;
    autoTriggerGate?: ((state: EditorState, pos: number) => boolean) | null;
    minimal?: boolean; onsubmit?: () => void; onselect?: (s: { text: string; cursor: number }) => void;
    gotoLine?: number | null; gotoCol?: number | null; findOwner?: boolean; placeholder?: string;
    wrap?: boolean; keepStates?: boolean;
    historyKey?: ((path: string) => { key: string; persist: boolean } | null) | null;
    oncopy?: ((text: string) => void) | null; lsp?: boolean; sqlDialect?: SqlDialectName; highlightLineLimit?: number;
    ideMode?: boolean; connectionId?: string | null; projectFolders?: string[]; savedContent?: string | null;
    breakpoints?: IdeBreakpoint[]; breakpointsMuted?: boolean;
    blameByLine?: Record<number, string> | null;
    onToggleBreakpoint?: ((line: number) => void) | null;
    onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
    gutterItems?: IdeGutterItem[];
    onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug') => void) | null;
    pausedLine?: number | null; inlineValues?: IdeInlineValue[];
    peerCursors?: (IdePeerCursor | IdeCollabPeer)[];
    keymapPreset?: 'jetbrains' | 'standard';
    jumpToLine?: { line: number; column?: number | null; seq?: number } | number | null;
    onCursorChange?: ((info: CursorChangeInfo) => void) | null;
    onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number }) => void) | null;
    onImplementationJump?: ((target: { path?: string; line: number; column: number; symbol?: string; originLine?: number; originColumn?: number }) => void) | null;
    onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
    onFileStructure?: (() => void) | null;
    onJumpSymbol?: ((delta: -1 | 1) => void) | null;
    onNavigateBack?: (() => void) | null;
    onNavigateForward?: (() => void) | null;
    onQuickFix?: ((ctx: { line: number; column: number; symbol?: string; clientX?: number; clientY?: number }) => void) | null;
    onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string }) => void) | null;
    onFormat?: (() => void) | null;
    onToggleBlame?: (() => void) | null;
    onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
    onAgentAction?: ((mode: 'chat' | 'inline', sel: CmIdeMod.IdeEditorSelectionRange) => void) | null;
    onGoToLine?: (() => void) | null;
    onPasteFromHistory?: (() => void) | null;
    onOpenLocalHistory?: (() => void) | null;
    onOpenDepAnalyzer?: (() => void) | null;
    onOpenManifestIntention?: ((line: number, dep: CmIdeDepsMod.IdeDepItem | null) => void) | null;
    onManifestDepsUpdated?: ((items: CmIdeDepsMod.IdeDepItem[]) => void) | null;
  }

  let {
    path, content, root, language, readOnly = true, onchange, completionSource = null, autoTriggerGate = null,
    minimal = false, onsubmit, onselect, gotoLine = null, gotoCol = null, findOwner = true, placeholder = '',
    wrap = false, keepStates = false, historyKey = null, oncopy = null, lsp = false, sqlDialect = 'standard',
    highlightLineLimit = 0, ideMode = false, connectionId = null, projectFolders = [], savedContent = null,
    breakpoints = [], breakpointsMuted = false, blameByLine = null,
    onToggleBreakpoint = null, onEditBreakpoint = null, gutterItems = [], onGutterAction = null,
    pausedLine = null, inlineValues = [], peerCursors = [], keymapPreset = 'jetbrains', jumpToLine = null,
    onCursorChange = null, onDefinitionJump = null, onImplementationJump = null,
    onFindUsages = null, onFileStructure = null, onJumpSymbol = null, onNavigateBack = null, onNavigateForward = null,
    onQuickFix = null, onRenameSymbol = null, onFormat = null, onToggleBlame = null,
    onDiagnostics = null, onAgentAction = null, onGoToLine = null, onPasteFromHistory = null,
    onOpenLocalHistory = null, onOpenDepAnalyzer = null, onOpenManifestIntention = null, onManifestDepsUpdated = null,
  }: Props = $props();

  let idePrefs = $state<CmIdeThemeType.IdeEditorPreferences | null>(coreThemeMod ? coreThemeMod.readIdeEditorPreferences() : null);
  const effectiveIdeScheme = $derived(
    coreThemeMod && idePrefs
      ? coreThemeMod.resolveEffectiveIdeScheme(idePrefs, ui.resolvedScheme)
      : (ui.resolvedScheme === 'dark' ? 'jetbrains-islands-dark' : 'jetbrains-islands-light'),
  );

  function themeExt(appScheme: 'light' | 'dark', plain = false): Extension {
    if (ideMode && coreThemeMod && idePrefs) {
      const scheme = coreThemeMod.resolveEffectiveIdeScheme(idePrefs, appScheme);
      return coreThemeMod.createIdeSyntaxTheme({ dark: coreThemeMod.isDarkIdeScheme(scheme), plain, ideMode: true });
    }
    if (plain) return appScheme === 'dark' ? oneDarkTheme : lightEditorTheme;
    return appScheme === 'dark' ? oneDark : [lightEditorTheme, syntaxHighlighting(defaultHighlightStyle)];
  }

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let lspCompartment = new Compartment(), ideBpCompartment = new Compartment();
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

  let ideMod: typeof CmIdeMod | null = null;
  let ideModPromise: Promise<typeof CmIdeMod> | null = null;
  let ideMounted = false, appliedIdePath = '';
  let appliedKeymapPreset: 'jetbrains' | 'standard' = 'jetbrains';

  function ensureIdeMod(): Promise<typeof CmIdeMod> {
    return ideMod ? Promise.resolve(ideMod) : (ideModPromise ??= import('./cm-ide-extensions').then((m) => ((ideMod = m), m)));
  }

  function isIdeActive(): boolean {
    return Boolean(
      ideMode || savedContent != null || blameByLine != null || onAgentAction || breakpoints.length > 0 ||
      gutterItems.length > 0 || pausedLine != null || inlineValues.length > 0 || peerCursors.length > 0 ||
      onToggleBreakpoint || onEditBreakpoint || onGutterAction || onDefinitionJump || onImplementationJump ||
      onFindUsages || onFileStructure || onQuickFix || onRenameSymbol || onFormat,
    );
  }

  function getIdeCallbacks(): CmIdeMod.IdeEditorCallbacks {
    return {
      onToggleBreakpoint, onEditBreakpoint, onGutterAction,
      onDefinitionJump, onImplementationJump, onFindUsages, onFileStructure,
      onJumpSymbol, onNavigateBack, onNavigateForward,
      onQuickFix, onRenameSymbol, onFormat, onToggleBlame, onDiagnostics, onAgentAction,
      onGoToLine, onPasteFromHistory,
    };
  }

  function buildIdeOpts(forPath?: string): CmIdeMod.CreateIdeExtensionsOptions {
    return {
      filePath: forPath ?? (livePath || path), language, root, connectionId, projectFolders,
      breakpoints, breakpointsMuted, gutterItems, pausedLine, inlineValues, peerCursors,
      savedContent, blameByLine, keymapPreset, getCallbacks: getIdeCallbacks,
    };
  }

  function buildIdeBpExtIfLoaded(forPath?: string): Extension {
    return ideMod && isIdeActive() ? ideMod.createIdeGutterExtensions(buildIdeOpts(forPath)) : [];
  }

  function buildIdeExtIfLoaded(forPath?: string): Extension {
    if (!ideMod || !isIdeActive()) { ideMounted = false; return []; }
    const p = forPath ?? (livePath || path);
    ideMounted = true; appliedIdePath = p; appliedKeymapPreset = keymapPreset;
    return ideMod.createIdeExtensions(buildIdeOpts(p));
  }

  let appliedManifestDepsPath = '';

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
          isIdeMounted: () => ideMounted,
          getToggle: () => onToggleBreakpoint,
          getEdit: () => onEditBreakpoint,
        })
      : [];

  // ── Long-line plain mode (`highlightLineLimit`) ─────────────────────────────
  let appliedPlain = false;
  let plainRecheck: ReturnType<typeof setTimeout> | null = null;

  function setPlain(target: EditorView, on: boolean): void {
    if (on === appliedPlain) return;
    appliedPlain = on;
    queueMicrotask(() => {
      if (view !== target) return;
      target.dispatch({ effects: themeCompartment.reconfigure(themeExt(untrack(() => ui.resolvedScheme), appliedPlain)) });
    });
  }

  const plainWatch = EditorView.updateListener.of((u) => {
    const limit = highlightLineLimit;
    if (!u.docChanged || limit <= 0) return;
    if (appliedPlain) {
      if (plainRecheck) clearTimeout(plainRecheck);
      const target = u.view;
      plainRecheck = setTimeout(() => { plainRecheck = null; if (view === target && !hasLongLine(target.state.doc, limit)) setPlain(target, false); }, 300);
      return;
    }
    const doc = u.state.doc;
    let long = false;
    u.changes.iterChangedRanges((_fa, _ta, fromB, toB) => {
      for (let pos = fromB; !long; ) {
        const line = doc.lineAt(pos);
        if (line.length > limit) long = true;
        if (line.to >= toB) break;
        pos = line.to + 1;
      }
    });
    if (long) setPlain(u.view, true);
  });

  // ── Selection & Change Emitters ────────────────────────────────────────────
  let sel: { from: number; to: number; startLine: number; endLine: number } | null = $state(null);

  function emitSelection(state: EditorState): void {
    const { from, to, head } = state.selection.main;
    sel = from === to ? null : { from, to, startLine: state.doc.lineAt(from).number, endLine: state.doc.lineAt(to).number };
    if (onCursorChange) onCursorChange(computeCursorChangeInfo(state));
    if (!onselect) return;
    let text: string | undefined = from === to ? '' : undefined;
    onselect({ get text() { return (text ??= state.sliceDoc(from, to)); }, cursor: head });
  }

  const selectionListener = EditorView.updateListener.of((u) => { if (u.selectionSet || u.docChanged) emitSelection(u.state); });
  let lastEmitted: string | null = null;
  const changes = createChangeEmitter({
    threshold: 256 * 1024, delayMs: 150,
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
    const text = view.state.sliceDoc(sel.from, sel.to);
    const { startLine, endLine } = sel;
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

  function handleLspJump(targetPath: string, line: number, column: number): void {
    if (targetPath === livePath) revealLine(line, column);
    else onDefinitionJump?.({ path: targetPath, line, column });
  }

  async function attachLsp(editorView: EditorView, filePath: string, rootPath: string, connId?: string | null): Promise<void> {
    const m = await import('./cm-editor-ondemand');
    return m.attachCodeEditorLsp({
      editorView, filePath, rootPath, connId, ideMode, language, lspCompartment,
      isCurrent: () => view === editorView && livePath === filePath,
      ensureIdeMod, onLocationJump: handleLspJump, onDiagnostics,
    });
  }

  function teardownEditor(): void {
    changes.cancel();
    if (plainRecheck) clearTimeout(plainRecheck);
    plainRecheck = null; ideMounted = false; appliedIdePath = ''; appliedManifestDepsPath = '';
    try { view?.destroy(); } catch { /* ignore */ }
    view = null;
    if (keyContext.openFind === openEditorSearch) keyContext.openFind = null;
  }

  const completionExt = (): Extension => createCompletionExtension(completionSource, autoTriggerGate, () => langExtOf(path, language));
  const submitKeymap = keymap.of([{ key: 'Mod-Enter', run: () => { if (!onsubmit) return false; changes.flush(); onsubmit(); return true; } }]);
  function openEditorSearch(): void { if (view) openSearchPanel(view); }

  // ── Exported Component Methods ─────────────────────────────────────────────
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
    if (ideMod) return ideMod.selectionOrCurrentLine(view.state);
    return coreHistMod ? coreHistMod.fallbackSelectionRange(view.state) : null;
  }

  export function applyAgentEdit(nextContent: string, range?: { from: number; to: number }): boolean {
    if (!view || readOnly) return false;
    const docLen = view.state.doc.length;
    const from = range ? Math.max(0, Math.min(docLen, range.from)) : 0;
    const to = range ? Math.max(from, Math.min(docLen, range.to)) : docLen;
    view.dispatch({ changes: { from, to, insert: nextContent }, selection: EditorSelection.cursor(from + nextContent.length), scrollIntoView: true, userEvent: 'input.complete' });
    changes.flush(); parkLive();
    return true;
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

  $effect(() => registerFindProvider(buildEditorFindProviderSpec(() => view)));

  // ── State Creation & Durable History ───────────────────────────────────────
  const readOnlyExt = (ro: boolean, hk?: { key: string; persist: boolean } | null): Extension =>
    buildReadOnlyExtension(ro, ideMode, hk, splitSelectionToLines);

  function createState(filePath: string, fileContent: string): EditorState {
    appliedDialect = sqlDialect; appliedReadOnly = readOnly;
    const hk = readOnly ? null : (historyKey?.(filePath) ?? null);
    const limit = highlightLineLimit;
    const doc = limit > 0 ? Text.of(fileContent.split(/\r\n?|\n/)) : null;
    appliedPlain = !!doc && hasLongLine(doc, limit);
    appliedLanguage = language;
    const langExt = cmLangFor(filePath, language);
    appliedLangReady = langExt != null || !cmLangPending(langExtOf(filePath, language));

    const baseExtensions: Extension[] = [
      ...(minimal ? [] : [lineNumbers(), ideBpCompartment.of(buildIdeBpExtIfLoaded(filePath)), foldMarkerGutter()]),
      indentOnInput(), bracketMatching(), lintGutter(),
      ideCompartment.of(buildIdeExtIfLoaded(filePath)),
      ideDepsCompartment.of([]), drawSelection(),
      readOnlyCompartment.of(readOnlyExt(readOnly, hk)),
      completionCompartment.of(completionExt()),
      placeholderCompartment.of(placeholderExt((appliedPlaceholder = placeholder))),
      wrapCompartment.of(wrapExt((appliedWrap = wrap || Boolean(ideMode && idePrefs?.wordWrap)))),
      placeholderEditorTheme, search({ top: false }),
      themeCompartment.of(themeExt(ui.resolvedScheme, appliedPlain)),
      lspCompartment.of([]), selectionListener, changeListener,
      ...(limit > 0 ? [plainWatch] : []),
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
    if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop, liveSavedContent);
  }

  function applyPendingScroll(): void {
    const top = pendingScrollTop;
    pendingScrollTop = null;
    if (top != null && top > 0) requestAnimationFrame(() => { if (view) view.scrollDOM.scrollTop = top; });
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

  function recordCopy(state: EditorState): void {
    const text = state.selection.ranges.filter((r) => !r.empty).map((r) => state.sliceDoc(r.from, r.to)).join('\n');
    if (!text) return;
    if (ideMode) coreHistMod?.recordIdeClipboardEntry(text, livePath || path);
    oncopy?.(text);
  }
  const copyRecorder = EditorView.domEventHandlers({
    copy: (_e, v) => { recordCopy(v.state); return false; },
    cut: (_e, v) => { recordCopy(v.state); return false; },
  });

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string, connId?: string | null): void {
    parkLive();
    stateCache.drainAndClear((p, st, evSaved) => parkHistory(p, st, 0, evSaved));
    teardownEditor();
    lspCompartment = new Compartment(); ideBpCompartment = new Compartment();
    ideCompartment = new Compartment(); ideDepsCompartment = new Compartment();
    readOnlyCompartment = new Compartment(); completionCompartment = new Compartment();
    placeholderCompartment = new Compartment(); wrapCompartment = new Compartment();
    langCompartment = new Compartment();
    sel = null;

    skipDestroyPark = false;
    livePath = filePath;
    liveSavedContent = savedContent;
    view = new EditorView({ state: createState(filePath, fileContent), parent: el });
    applyPendingScroll();
    ensureLang(filePath);
    mountIdeExtensions(view, filePath);
    if (!completionSource && lsp) void attachLsp(view, filePath, rootPath, connId);
    if (gotoLine != null) revealLine(gotoLine, gotoCol);
    const built = view;
    untrack(() => emitSelection(built.state));
  }

  function swapState(fromPath: string, toPath: string, toContent: string): void {
    if (!view) return;
    changes.flush();
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;

    const prevScrollTop = view.scrollDOM.scrollTop;
    const prevSel = view.state.selection.main;
    const outgoingSaved = liveSavedContent;
    stateCache.saveLightMeta(fromPath, prevScrollTop, prevSel.anchor, prevSel.head);
    view.dispatch({ effects: [lspCompartment.reconfigure([]), ideDepsCompartment.reconfigure([])] });

    parkHistory(fromPath, view.state, prevScrollTop, outgoingSaved);
    stateCache.stashOutgoing(
      fromPath,
      { state: view.state, scroll: view.scrollSnapshot(), chars: view.state.doc.length, dialect: appliedDialect, langReady: appliedLangReady, language: appliedLanguage, savedContent: outgoingSaved },
      ideMode,
      (evPath, evState, evSaved) => parkHistory(evPath, evState, 0, evSaved),
    );

    skipDestroyPark = false;
    livePath = toPath;
    liveSavedContent = savedContent;
    const kept = stateCache.takeKept(toPath);
    if (!kept) {
      const st = createState(toPath, toContent);
      const meta = stateCache.lightTabMeta.get(toPath);
      if (meta && pendingScrollTop == null) {
        pendingScrollTop = meta.scrollTop;
        const maxPos = st.doc.length;
        view.setState(st.update({ selection: { anchor: Math.min(maxPos, Math.max(0, meta.anchor)), head: Math.min(maxPos, Math.max(0, meta.head)) } }).state);
      } else {
        view.setState(st);
      }
      applyPendingScroll();
      ensureLang(toPath);
      mountIdeExtensions(view, toPath);
    } else {
      view.setState(kept.state);
      if (plainRecheck) clearTimeout(plainRecheck);
      plainRecheck = null;
      appliedPlain = highlightLineLimit > 0 && hasLongLine(kept.state.doc, highlightLineLimit);
      const effects: StateEffect<unknown>[] = [
        readOnlyCompartment.reconfigure(readOnlyExt((appliedReadOnly = readOnly), readOnly ? null : (historyKey?.(toPath) ?? null))),
        completionCompartment.reconfigure(completionExt()),
        placeholderCompartment.reconfigure(placeholderExt((appliedPlaceholder = placeholder))),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap || Boolean(ideMode && idePrefs?.wordWrap)))),
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        kept.scroll,
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
      const doc = view.state.doc;
      const stale = doc.length !== toContent.length || doc.toString() !== toContent;
      view.dispatch({ effects, ...(stale ? { changes: { from: 0, to: doc.length, insert: toContent } } : {}) });
      ensureLang(toPath);
      mountIdeExtensions(view, toPath);
    }
    if (!completionSource && lsp) void attachLsp(view, toPath, root, connectionId);
    prevCompletion = completionSource;
    emitSelection(view.state);
  }

  const revealLine = (line: number, col?: number | null): void => revealEditorLine(() => view, line, col);

  // ── Reactive effects ───────────────────────────────────────────────────────
  let prevPath = '';
  let prevRoot = '';
  let prevConnId: string | null = null;
  let prevContent = '';

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
    } else if (curContent !== prevContent && !contentEchoed && !changes.pending) {
      prevContent = curContent;
      if (curContent !== view.state.doc.toString()) view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: curContent } });
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
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap || Boolean(ideMode && idePrefs?.wordWrap)))),
      ];
      if (ideMod && ideMounted && isIdeActive()) effects.push(ideCompartment.reconfigure(buildIdeExtIfLoaded()));
      view.dispatch({ effects });
    };
    window.addEventListener('otto:ide-prefs-changed', onPrefs);
    return () => window.removeEventListener('otto:ide-prefs-changed', onPrefs);
  });

  function syncIdeEffect(needMount: boolean, makeEffect: (m: typeof CmIdeMod) => StateEffect<unknown>): void {
    untrack(() => {
      if (!view) return;
      if (!ideMounted && needMount) mountIdeExtensions(view);
      else if (ideMod && ideMounted) view.dispatch({ effects: makeEffect(ideMod) });
    });
  }
  $effect(() => { const v = savedContent; if (livePath === path) liveSavedContent = v; syncIdeEffect(v != null, (m) => m.setSavedContentEffect.of(v)); });
  $effect(() => { const v = blameByLine; syncIdeEffect(v != null, (m) => m.setBlameLinesEffect.of(v)); });
  $effect(() => { const b = breakpoints, mu = breakpointsMuted; syncIdeEffect(b.length > 0 || !!onToggleBreakpoint, (m) => m.setBreakpointsEffect.of({ breakpoints: b, muted: mu })); });
  $effect(() => { const v = gutterItems; syncIdeEffect(v.length > 0, (m) => m.setGutterItemsEffect.of(v)); });
  $effect(() => { const p = pausedLine, iv = inlineValues; syncIdeEffect(p != null || iv.length > 0, (m) => m.setDebugDecorationsEffect.of({ pausedLine: p, inlineValues: iv })); });
  $effect(() => { const v = peerCursors; syncIdeEffect(v.length > 0, (m) => m.setPeerCursorsEffect.of(v)); });
  $effect(() => {
    const preset = keymapPreset;
    if (!view || !ideMod || !ideMounted || preset === appliedKeymapPreset) return;
    appliedKeymapPreset = preset;
    untrack(() => { if (view) view.dispatch({ effects: ideCompartment.reconfigure(buildIdeExtIfLoaded()) }); });
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
      view.dispatch({ effects: langCompartment.reconfigure(langExt ?? []) });
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
  style:--ide-editor-lh={ideMode ? String(idePrefs?.lineHeight ?? 1.55) : undefined}>
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
    background: var(--syn-bg);
  }
  .code-editor-wrap :global(.cm-scroller) { overflow: auto; }
  .code-editor-wrap :global(.cm-editor.cm-focused) { outline: none; }
  .code-editor-wrap:focus-within { box-shadow: inset 0 0 0 1px var(--accent-text); }
  .code-editor-outer.is-ide .code-editor-wrap:focus-within { box-shadow: none; }
</style>
