<script module lang="ts">
  import type * as CmIdeThemeType from './cm-ide-theme';
  import type * as CmIdeHistType from './cm-ide-history';
  let coreThemeMod: typeof CmIdeThemeType | null = null;
  let coreHistMod: typeof CmIdeHistType | null = null;
  export function registerIdeEditorCore(themeMod: typeof CmIdeThemeType, histMod: typeof CmIdeHistType): void {
    coreThemeMod = themeMod; coreHistMod = histMod;
  }
</script>

<script lang="ts">
  // CodeMirror 6 editor with LSP, multi-cursor, 1-click gutter breakpoints, 6 IDE color schemes & durable ⌘Z undo.
  import { onDestroy, untrack } from 'svelte';
  import { EditorView, lineNumbers, keymap, drawSelection, rectangularSelection, crosshairCursor, placeholder as cmPlaceholder } from '@codemirror/view';
  import { EditorSelection, EditorState, Compartment, Prec, Text, type StateEffect, type Extension } from '@codemirror/state';
  import { defaultKeymap, history, historyField, historyKeymap, selectAll } from '@codemirror/commands';
  import { forgetEditorState, loadEditorState, prepareEditorHistory, registerLiveParker, saveEditorState, PERSIST_MAX_BYTES } from '../editor-history';
  import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
  import { autocompletion, completionKeymap, startCompletion, closeBrackets, closeBracketsKeymap, type CompletionSource } from '@codemirror/autocomplete';
  import { lintGutter, lintKeymap } from '@codemirror/lint';
  import { indentOnInput, bracketMatching, foldGutter, foldKeymap, defaultHighlightStyle, syntaxHighlighting, foldedRanges, unfoldEffect } from '@codemirror/language';
  import { oneDark, oneDarkTheme } from '@codemirror/theme-one-dark';
  import { cmLangNow, cmLangPending, loadCmLang } from './cm-langs';
  import type { SqlDialectName } from '../sql-dialects';
  import { createChangeEmitter } from './changeEmitter';
  import { api, baseUrl, getToken, WS_BEARER_SUBPROTOCOL } from '../api/client';
  import type { IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem, IdeInlineValue, IdePeerCursor, LspCapabilities } from '../api/types';
  import type * as CmIdeMod from './cm-ide-extensions';
  import type * as CmIdeDepsMod from './cm-ide-deps';
  import { ws } from '../stores/workspace.svelte';
  import { ui } from '../stores/ui.svelte';
  import { keyContext } from '../keys';
  import { registerFindProvider } from '../findProviders';
  import { toasts } from '../toast.svelte';

  const themeCompartment = new Compartment();
  const lightTheme = EditorView.theme({
    '&': { color: 'var(--text)', backgroundColor: 'transparent' }, '.cm-content': { caretColor: 'var(--text)' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
    '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: 'var(--accent-line)' },
    '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--text-dim) 8%, transparent)' },
    '.cm-gutters': { backgroundColor: 'transparent', color: 'var(--text-dim)', border: 'none' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent' },
  }, { dark: false });

  export interface CursorChangeInfo {
    line: number; column: number; cursorCount: number;
    selectedLines: number; selectionLength: number;
    selections: { anchor: number; head: number }[];
  }

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

  const FALLBACK_PREFS: CmIdeThemeType.IdeEditorPreferences = {
    colorScheme: 'auto', darkColorScheme: 'jetbrains-islands-dark', lightColorScheme: 'jetbrains-islands-light',
    fontSize: 13, lineHeight: 1.55, tabSize: 4, wordWrap: false, inlayHints: true, codeVision: true,
    rainbowBrackets: true, showBreadcrumbs: true, autoFoldImports: true, formatOnSave: false, keymapPreset: 'jetbrains',
  };
  let idePrefs = $state<CmIdeThemeType.IdeEditorPreferences>(coreThemeMod ? coreThemeMod.readIdeEditorPreferences() : FALLBACK_PREFS);
  const effectiveIdeScheme = $derived(
    coreThemeMod
      ? coreThemeMod.resolveEffectiveIdeScheme(idePrefs, ui.resolvedScheme)
      : (ui.resolvedScheme === 'dark' ? 'jetbrains-islands-dark' : 'jetbrains-islands-light'),
  );

  function themeExt(appScheme: 'light' | 'dark', plain = false): Extension {
    if (ideMode && coreThemeMod) {
      const scheme = coreThemeMod.resolveEffectiveIdeScheme(idePrefs, appScheme);
      return coreThemeMod.createIdeSyntaxTheme({ dark: coreThemeMod.isDarkIdeScheme(scheme), plain, ideMode: true });
    }
    if (plain) return appScheme === 'dark' ? oneDarkTheme : lightTheme;
    return appScheme === 'dark' ? oneDark : [lightTheme, syntaxHighlighting(defaultHighlightStyle)];
  }

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let lspCompartment = new Compartment();
  let ideBpCompartment = new Compartment();
  let ideCompartment = new Compartment();
  let ideDepsCompartment = new Compartment();
  let readOnlyCompartment = new Compartment();
  let appliedReadOnly = true;
  let completionCompartment = new Compartment();
  let placeholderCompartment = new Compartment();
  let appliedPlaceholder = '';
  let wrapCompartment = new Compartment();
  let appliedWrap = false;

  const wrapExt = (on: boolean): Extension => (on ? EditorView.lineWrapping : []);
  const placeholderTheme = EditorView.theme({ '.cm-placeholder': { color: 'var(--text-dim)', fontStyle: 'normal' } });
  const placeholderExt = (text: string): Extension => (text ? cmPlaceholder(text) : []);

  let ideMod: typeof CmIdeMod | null = null;
  let ideModPromise: Promise<typeof CmIdeMod> | null = null;
  let depsMod: typeof CmIdeDepsMod | null = null;
  let depsModPromise: Promise<typeof CmIdeDepsMod> | null = null;
  let ideMounted = false;
  let appliedIdePath = '';
  let appliedKeymapPreset: 'jetbrains' | 'standard' = 'jetbrains';

  function ensureIdeMod(): Promise<typeof CmIdeMod> {
    if (ideMod) return Promise.resolve(ideMod);
    return (ideModPromise ??= import('./cm-ide-extensions').then((m) => ((ideMod = m), m)));
  }

  function isManifestPath(p: string): boolean {
    const b = (p.split('?')[0].split('/').pop() ?? '').toLowerCase();
    return b === 'go.mod' || b === 'pom.xml' || b === 'package.json' || b === 'cargo.toml' || b === 'pyproject.toml' || b === 'build.gradle' || b === 'build.gradle.kts' || /^requirements(?:[-_.][\w-]+)?\.txt$/.test(b);
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
    const p = forPath ?? (livePath || path);
    return {
      filePath: p, language, root, connectionId, projectFolders,
      breakpoints, breakpointsMuted, gutterItems, pausedLine, inlineValues, peerCursors,
      savedContent, blameByLine, keymapPreset, getCallbacks: getIdeCallbacks,
    };
  }

  function buildIdeBpExtIfLoaded(forPath?: string): Extension {
    if (!ideMod || !isIdeActive()) return [];
    return ideMod.createIdeGutterExtensions(buildIdeOpts(forPath));
  }

  function buildIdeExtIfLoaded(forPath?: string): Extension {
    if (!ideMod || !isIdeActive()) { ideMounted = false; return []; }
    const p = forPath ?? (livePath || path);
    ideMounted = true;
    appliedIdePath = p;
    appliedKeymapPreset = keymapPreset;
    return ideMod.createIdeExtensions(buildIdeOpts(p));
  }

  function mountIdeExtensions(target: EditorView, forPath?: string): void {
    if (!isIdeActive()) return;
    const p = forPath ?? (livePath || path);
    const apply = () => {
      if (view === target && isIdeActive() && (!ideMounted || appliedIdePath !== p)) {
        target.dispatch({
          effects: [
            ideBpCompartment.reconfigure(buildIdeBpExtIfLoaded(p)),
            ideCompartment.reconfigure(buildIdeExtIfLoaded(p)),
          ],
        });
      }
    };
    if (ideMod) apply(); else void ensureIdeMod().then(apply);
    mountManifestDeps(target, p);
  }

  function mountManifestDeps(target: EditorView, forPath: string): void {
    if (!ideMode || !isManifestPath(forPath)) {
      target.dispatch({ effects: ideDepsCompartment.reconfigure([]) });
      onManifestDepsUpdated?.([]);
      return;
    }
    const apply = (m: typeof CmIdeDepsMod) => {
      if (view !== target || livePath !== forPath) return;
      const ext = m.createIdeDepsExtension({
        filePath: forPath, root, connectionId, projectFolders,
        onApplyContent: (nextContent) => { applyAgentEdit(nextContent); },
        onOpenIntentionMenu: (ln, dep) => onOpenManifestIntention?.(ln, dep),
        onJumpLocation: (loc) => onDefinitionJump?.(loc),
        onAddAgentContext: () => { const r = getSelectionRange(); if (r) onAgentAction?.('chat', r); },
        onOpenLocalHistory: () => onOpenLocalHistory?.(),
        onOpenDepAnalyzer: () => onOpenDepAnalyzer?.(),
        onToggleBlame,
        onDepsUpdated: (items) => onManifestDepsUpdated?.(items),
      });
      target.dispatch({ effects: ideDepsCompartment.reconfigure(ext) });
    };
    if (depsMod) apply(depsMod);
    else void (depsModPromise ??= import('./cm-ide-deps').then((m) => ((depsMod = m), m))).then(apply, () => {});
  }

  const fallbackBpGutterExt = EditorView.domEventHandlers({
    mousedown(ev, v) {
      if (ideMounted || ev.button !== 0 || ev.altKey || ev.metaKey || ev.ctrlKey || (!onToggleBreakpoint && !onEditBreakpoint)) return false;
      if (!(ev.target instanceof Element) || !ev.target.closest('.cm-gutters') || ev.target.closest('.cm-ide-run-btn, .cm-ide-gutter-run-btn, .cm-ide-lightbulb-btn, .cm-ide-dep-gutter-btn, .cm-fold-marker, .cm-ide-diff-bar')) return false;
      const ln = v.state.doc.lineAt(v.lineBlockAtHeight(ev.clientY - v.documentTop).from).number;
      if (ev.shiftKey && onEditBreakpoint) { ev.preventDefault(); onEditBreakpoint(ln, ev); return true; }
      if (onToggleBreakpoint) { ev.preventDefault(); onToggleBreakpoint(ln); return true; }
      return false;
    },
    contextmenu(ev, v) {
      if (ideMounted || !onEditBreakpoint || !(ev.target instanceof Element) || !ev.target.closest('.cm-gutters') || ev.target.closest('.cm-ide-run-btn, .cm-ide-gutter-run-btn, .cm-ide-lightbulb-btn, .cm-ide-dep-gutter-btn, .cm-fold-marker, .cm-ide-diff-bar')) return false;
      const ln = v.state.doc.lineAt(v.lineBlockAtHeight(ev.clientY - v.documentTop).from).number;
      ev.preventDefault(); ev.stopPropagation(); onEditBreakpoint(ln, ev); return true;
    },
  });

  // ── Long-line plain mode (highlightLineLimit) ───────────────────────────────
  let appliedPlain = false;
  let plainRecheck: ReturnType<typeof setTimeout> | null = null;

  function hasLongLine(doc: Text, limit: number): boolean {
    if (doc.length <= limit) return false;
    let run = 0;
    for (const it = doc.iter(); !it.next().done; ) {
      if (it.lineBreak) run = 0;
      else if ((run += it.value.length) > limit) return true;
    }
    return false;
  }

  function setPlain(target: EditorView, on: boolean): void {
    if (on === appliedPlain) return;
    appliedPlain = on;
    queueMicrotask(() => {
      if (view !== target) return;
      const scheme = untrack(() => ui.resolvedScheme);
      target.dispatch({ effects: themeCompartment.reconfigure(themeExt(scheme, appliedPlain)) });
    });
  }

  const plainWatch = EditorView.updateListener.of((u) => {
    const limit = highlightLineLimit;
    if (!u.docChanged || limit <= 0) return;
    if (appliedPlain) {
      if (plainRecheck) clearTimeout(plainRecheck);
      const target = u.view;
      plainRecheck = setTimeout(() => {
        plainRecheck = null;
        if (view === target && !hasLongLine(target.state.doc, limit)) setPlain(target, false);
      }, 300);
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

  // ── Language extension map ─────────────────────────────────────────────────

  let appliedDialect: SqlDialectName = 'standard';
  let langCompartment = new Compartment();
  let appliedLangReady = true;
  let appliedLanguage: string | undefined = undefined;

  const EXT_TO_LSP_LANG: Record<string, string> = {
    js: 'javascript', jsx: 'javascriptreact', ts: 'typescript', tsx: 'typescriptreact',
    mjs: 'javascript', cjs: 'javascript', mts: 'typescript', cts: 'typescript',
    py: 'python', pyi: 'python', go: 'go', rs: 'rust',
    json: 'json', jsonc: 'json', jsonl: 'json', ndjson: 'json',
    html: 'html', htm: 'html', css: 'css', scss: 'scss',
    less: 'less', md: 'markdown', mdx: 'markdown',
    java: 'java', class: 'java', kt: 'kotlin', kts: 'kotlin',
    sh: 'shellscript', bash: 'shellscript', zsh: 'shellscript', bats: 'shellscript',
    yaml: 'yaml', yml: 'yaml', graphql: 'graphql', gql: 'graphql', proto: 'protobuf',
  };

  function extOf(p: string): string {
    if (p.startsWith('jdt://')) return 'java';
    const clean = p.split('?')[0];
    const base = clean.slice(clean.lastIndexOf('/') + 1);
    const dot = base.lastIndexOf('.');
    return dot < 0 ? '' : base.slice(dot + 1).toLowerCase();
  }

  // ── Selection & Multi-Cursor State ─────────────────────────────────────────

  let sel: { from: number; to: number; startLine: number; endLine: number } | null = $state(null);

  function emitSelection(state: EditorState): void {
    const { from, to, head } = state.selection.main;
    sel = from === to ? null : { from, to, startLine: state.doc.lineAt(from).number, endLine: state.doc.lineAt(to).number };
    if (onCursorChange) {
      const mainLn = state.doc.lineAt(head);
      let selLen = 0;
      const touched = new Set<number>();
      for (const r of state.selection.ranges) {
        selLen += r.to - r.from;
        const sLn = state.doc.lineAt(r.from).number;
        const eLn = !r.empty && r.to > r.from && state.doc.lineAt(r.to).from === r.to
          ? state.doc.lineAt(r.to - 1).number
          : state.doc.lineAt(r.to).number;
        for (let ln = sLn; ln <= eLn; ln++) touched.add(ln);
      }
      onCursorChange({
        line: mainLn.number, column: head - mainLn.from + 1, cursorCount: state.selection.ranges.length,
        selectedLines: touched.size, selectionLength: selLen,
        selections: state.selection.ranges.map((r) => ({ anchor: r.anchor, head: r.head })),
      });
    }
    if (!onselect) return;
    let text: string | undefined = from === to ? '' : undefined;
    onselect({ get text() { return (text ??= state.sliceDoc(from, to)); }, cursor: head });
  }

  const selectionListener = EditorView.updateListener.of((update) => {
    if (update.selectionSet || update.docChanged) emitSelection(update.state);
  });

  let lastEmitted: string | null = null;
  const LAZY_CHANGE_MIN = 256 * 1024;
  const changes = createChangeEmitter({
    threshold: LAZY_CHANGE_MIN,
    delayMs: 150,
    read: () => view?.state.doc.toString() ?? null,
    emit: (value) => { lastEmitted = value; onchange?.(value); },
  });

  const changeListener = EditorView.updateListener.of((update) => {
    if (update.docChanged && onchange) changes.changed(update.state.doc.length);
  });

  const changeFlushHandlers = EditorView.domEventHandlers({
    blur: () => { changes.flush(); parkLive(); return false; },
    keydown: (e) => { if ((e.metaKey || e.ctrlKey) && changes.pending) changes.flush(); return false; },
  });

  async function sendToAgent(): Promise<void> {
    if (!sel || !view) return;
    const text = view.state.sliceDoc(sel.from, sel.to);
    const sessionId = ws.activeSessionId;
    if (!sessionId || ws.activeSession?.kind !== 'agent') {
      toasts.error('No agent session', 'Open an agent session first.');
      return;
    }
    const snippet = `Re: ${path}:${sel.startLine}-${sel.endLine}\n\n\`\`\`${langExtOf(path, language)}\n${text}\n\`\`\`\n\n`;
    try {
      await api.post(`/sessions/${sessionId}/input`, { text: snippet, submit: false });
      toasts.success('Sent to agent', `${path} lines ${sel.startLine}-${sel.endLine}`);
    } catch {
      toasts.error('Couldn’t send to the session', 'The text couldn’t be injected into the agent session.');
    }
  }

  // ── LSP capability cache & helpers ─────────────────────────────────────────

  let capabilitiesCache: LspCapabilities | null = null;
  let capabilitiesFetching: Promise<LspCapabilities | null> | null = null;

  async function getCapabilities(): Promise<LspCapabilities | null> {
    if (capabilitiesCache) return capabilitiesCache;
    if (capabilitiesFetching) return capabilitiesFetching;
    capabilitiesFetching = api.get<LspCapabilities>('/lsp/capabilities')
      .then((c) => ((capabilitiesCache = c), c))
      .catch(() => null);
    return capabilitiesFetching;
  }

  const langExtOf = (filePath: string, hint?: string): string => extOf(filePath) || (hint ?? '');
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

  function lspLangFor(filePath: string): string | null {
    const ext = extOf(filePath);
    if (ext === 'csv' || ext === 'tsv') return null;
    if (EXT_TO_LSP_LANG[ext]) return EXT_TO_LSP_LANG[ext];
    if (!ideMode || !language || language === 'csv' || language === 'tsv' || language === 'plaintext') return null;
    if (language === 'shell') return 'shellscript';
    if (language === 'jsonl') return 'json';
    return language;
  }

  function lspWsUrl(lang: string, rootPath: string, connId?: string | null): string {
    const base = new URL(baseUrl());
    const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
    const params = new URLSearchParams({ lang, root: rootPath });
    if (connId) params.set('connection_id', connId);
    return `${proto}//${base.host}/ws/lsp?${params.toString()}`;
  }

  function uriToPath(uri: string): string {
    if (!uri.startsWith('file://')) return uri;
    try { return decodeURIComponent(uri.slice('file://'.length)); }
    catch { return uri.slice('file://'.length); }
  }

  function handleLspJump(uri: string, line: number, column: number): void {
    const targetPath = uriToPath(uri);
    if (targetPath === livePath) revealLine(line, column);
    else onDefinitionJump?.({ path: targetPath, line, column });
  }

  function teardownEditor(): void {
    changes.cancel();
    if (plainRecheck) clearTimeout(plainRecheck);
    plainRecheck = null;
    ideMounted = false;
    appliedIdePath = '';
    try { view?.destroy(); } catch { /* ignore */ }
    view = null;
    if (keyContext.openFind === openEditorSearch) keyContext.openFind = null;
  }

  async function attachLsp(editorView: EditorView, filePath: string, rootPath: string, connId?: string | null): Promise<void> {
    try {
      const lspLang = lspLangFor(filePath);
      if (!lspLang || !rootPath) return;
      if (!connId) {
        let caps = await getCapabilities();
        if (ideMode && caps && !caps.servers.some((s) => s.lang === lspLang && s.available)) {
          capabilitiesCache = null; capabilitiesFetching = null;
          caps = await getCapabilities();
        }
        if (!caps?.servers.some((s) => s.lang === lspLang && s.available)) return;
      }
      if (view !== editorView || livePath !== filePath) return;

      const wsUri = lspWsUrl(lspLang, rootPath, connId);
      const token = getToken();
      const protocols = token ? [WS_BEARER_SUBPROTOCOL, token] : undefined;
      const rootUri = `file://${rootPath}`;
      const documentUri = filePath.includes('://') ? filePath : `file://${filePath}`;
      const mod = await ensureIdeMod();
      if (view !== editorView || livePath !== filePath) return;

      try {
        const lspExtensions = await mod.buildPooledLspExtensions({
          wsUri, protocols, rootPath, connId, filePath, languageId: lspLang,
          onLocationJump: handleLspJump, onDiagnostics,
        });
        if (view !== editorView || livePath !== filePath) return;
        editorView.dispatch({ effects: lspCompartment.reconfigure(lspExtensions) });
      } catch {
        if (view !== editorView || livePath !== filePath) return;
        const fallbackExt = mod.createDirectLspExtension({
          wsUri, protocols, rootUri, documentUri, filePath, languageId: lspLang,
          onLocationJump: handleLspJump, onDiagnostics: onDiagnostics ?? undefined,
        });
        editorView.dispatch({ effects: lspCompartment.reconfigure(fallbackExt) });
      }
    } catch { /* LSP failures are silent — editor still shows highlighted code */ }
  }

  // ── Autocompletion & Keybindings ───────────────────────────────────────────

  function completionExt(): Extension {
    return completionSource
      ? [autocompletion({ override: [completionSource], activateOnTyping: true, maxRenderedOptions: 40 }), autoTriggerExt()]
      : autocompletion();
  }

  const SPACE_TRIGGER_RE = /(?:^|[\s({,])(?:from|join|where|and|or|on|into|update|set|by|using|select)\s$/i;
  const PUNCT_TRIGGER_LANGS = new Set(['js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs', 'json', 'jsonc']);

  function autoTriggerExt(): Extension {
    return EditorView.updateListener.of((u) => {
      if (!u.docChanged || !u.view.hasFocus) return;
      let fire = false;
      for (const tr of u.transactions) {
        if (!tr.isUserEvent('input.type') && !tr.isUserEvent('input.paste')) continue;
        tr.changes.iterChanges((_fa, _ta, _fb, _tb, inserted) => {
          if (inserted.length !== 1) return;
          const text = inserted.toString();
          if (text === '.') fire = true;
          else if (text === ' ') {
            const head = u.state.selection.main.head;
            const back = u.state.sliceDoc(Math.max(0, head - 48), head);
            if (SPACE_TRIGGER_RE.test(back)) fire = true;
            else if (/[({,]\s$/.test(back) && PUNCT_TRIGGER_LANGS.has(extOf(path) || (language ?? ''))) fire = true;
          }
        });
      }
      if (fire && autoTriggerGate && !autoTriggerGate(u.state, u.state.selection.main.head)) fire = false;
      if (fire) { const v = u.view; setTimeout(() => startCompletion(v), 0); }
    });
  }

  const submitKeymap = keymap.of([{
    key: 'Mod-Enter',
    run: () => { if (!onsubmit) return false; changes.flush(); onsubmit(); return true; },
  }]);

  function openEditorSearch(): void { if (view) openSearchPanel(view); }

  // ── Exported Component Methods ─────────────────────────────────────────────

  export function openSearch(): void { openEditorSearch(); }
  export function focus(): void { view?.focus(); }
  export function goToLine(line: number, col?: number): void { revealLine(line, col); }

  export function insertText(text: string): void {
    if (!view || readOnly) return;
    view.dispatch(view.state.replaceSelection(text), { scrollIntoView: true, userEvent: 'input.paste' });
    changes.flush();
    parkLive();
    view.focus();
  }

  export function getSelectionRange(): CmIdeMod.IdeEditorSelectionRange | null {
    if (!view) return null;
    if (ideMod) return ideMod.selectionOrCurrentLine(view.state);
    const { doc, selection } = view.state;
    const { from, to } = selection.main;
    if (from === to) { const ln = doc.lineAt(from); return { from: ln.from, to: ln.to, startLine: ln.number, endLine: ln.number, text: ln.text }; }
    const startLine = doc.lineAt(from).number;
    const endLine = to > from && doc.lineAt(to).from === to ? doc.lineAt(to - 1).number : doc.lineAt(to).number;
    return { from, to, startLine, endLine, text: view.state.sliceDoc(from, to) };
  }

  /** Apply an AI agent edit via a single CodeMirror transaction (`⌘Z` reverts in one undo step). */
  export function applyAgentEdit(nextContent: string, range?: { from: number; to: number }): boolean {
    if (!view || readOnly) return false;
    const docLen = view.state.doc.length;
    const from = range ? Math.max(0, Math.min(docLen, range.from)) : 0;
    const to = range ? Math.max(from, Math.min(docLen, range.to)) : docLen;
    view.dispatch({
      changes: { from, to, insert: nextContent },
      selection: EditorSelection.cursor(from + nextContent.length),
      scrollIntoView: true,
      userEvent: 'input.complete',
    });
    changes.flush();
    parkLive();
    return true;
  }

  /** Deterministically apply concrete quick-fix text edits in `<5ms` (`⌘Z` reverts in one undo step). */
  export function applyQuickFixEdits(edits: CmIdeMod.IdeConcreteTextEdit[]): boolean {
    if (!view || readOnly || !edits.length) return false;
    let ok = false;
    if (ideMod) {
      ok = ideMod.applyConcreteEditsToView(view, edits, livePath || path);
    } else {
      const doc = view.state.doc;
      const specs = edits.map((e) => {
        const sLn = doc.line(Math.max(1, Math.min(doc.lines, e.start_line)));
        const eLn = doc.line(Math.max(1, Math.min(doc.lines, e.end_line)));
        const from = Math.min(doc.length, sLn.from + Math.max(0, e.start_col - 1));
        const to = Math.max(from, Math.min(doc.length, eLn.from + Math.max(0, e.end_col - 1)));
        return { from, to, insert: e.new_text };
      });
      view.dispatch({ changes: specs, userEvent: 'input.quickfix' });
      ok = true;
    }
    if (ok) { changes.flush(); parkLive(); view.focus(); }
    return ok;
  }

  export function getAvailableQuickFixes(line?: number, symbol?: string): {
    clientFixes: CmIdeMod.ClientDiagQuickFix[];
    deterministicFixes: CmIdeMod.IdeQuickFixAction[];
    docText: string;
  } {
    if (!view || !ideMod) return { clientFixes: [], deterministicFixes: [], docText: view?.state.doc.toString() ?? '' };
    const docText = view.state.doc.toString();
    const ln = line ?? view.state.doc.lineAt(view.state.selection.main.head).number;
    return {
      clientFixes: ideMod.getIdeQuickFixesAtLine(view.state, ln),
      deterministicFixes: ideMod.computeDeterministicQuickFixes(docText, livePath || path, language, ln, symbol),
      docText,
    };
  }

  export function applyClientDiagFix(fix: CmIdeMod.ClientDiagQuickFix): boolean {
    if (!view || readOnly || !ideMod) return false;
    ideMod.applyClientQuickFix(view, fix);
    changes.flush();
    parkLive();
    view.focus();
    return true;
  }

  export function applyPrefixSuffix(prefix: string, suffix: string): void {
    if (!view || readOnly) return;
    const target = view;
    const run = (m: typeof CmIdeMod) => {
      if (view !== target) return;
      if (m.applyLinePrefixSuffix(target, prefix, suffix)) { changes.flush(); parkLive(); }
      target.focus();
    };
    if (ideMod) run(ideMod); else void ensureIdeMod().then(run);
  }

  export function splitSelectionToLines(): void {
    if (!view) return;
    const target = view;
    const run = (m: typeof CmIdeMod) => { if (view === target) { m.splitSelectionIntoLines(target); target.focus(); } };
    if (ideMod) run(ideMod); else void ensureIdeMod().then(run);
  }

  export function evictPaths(paths: Iterable<string> | 'all'): void {
    if (paths === 'all') {
      if (historyKey) {
        for (const p of keptStates.keys()) { const hk = historyKey(p); if (hk) forgetEditorState(hk.key); }
      }
      keptStates.clear();
      lightTabMeta.clear();
      coreHistMod?.evictSingletonIdeState('all');
      ideMod?.disposePooledLspClients({ forceAll: true });
      return;
    }
    for (const p of paths) {
      keptStates.delete(p);
      lightTabMeta.delete(p);
      coreHistMod?.evictSingletonIdeState([p]);
      coreHistMod?.forgetIdeUndoJournal(p);
      if (historyKey) { const hk = historyKey(p); if (hk) forgetEditorState(hk.key); }
    }
  }

  export function notifyDidSave(): void {
    if (view && ideMod) ideMod.notifyLspDidSave(view);
    if (view && ideMode && !readOnly && livePath) {
      coreHistMod?.persistIdeUndoJournal(livePath, view.state, view.state.doc.toString(), view.scrollDOM.scrollTop);
    }
  }

  export function disposeLspClients(rootPath?: string): void {
    ideMod?.disposePooledLspClients({ rootPath, forceAll: true });
  }

  // ── Whole-document ⌘F provider ─────────────────────────────────────────────

  function foldOver(v: EditorView, ln: number): { from: number; to: number } | null {
    const line = v.state.doc.line(ln);
    let hit: { from: number; to: number } | null = null;
    foldedRanges(v.state).between(line.from, line.to, (from, to) => {
      if (from < line.to && to > line.from) { hit = { from, to }; return false; }
    });
    return hit;
  }
  $effect(() =>
    registerFindProvider({
      root: () => view?.dom ?? null,
      count: () => view?.state.doc.lines ?? 0,
      text: (i) => (view && i < view.state.doc.lines ? view.state.doc.line(i + 1).text : ''),
      reveal: async (i) => {
        const v = view;
        if (!v || i >= v.state.doc.lines) return;
        for (let n = 0, fold = foldOver(v, i + 1); fold && n < 32; n++, fold = foldOver(v, i + 1)) {
          v.dispatch({ effects: unfoldEffect.of(fold) });
        }
        v.dispatch({ effects: EditorView.scrollIntoView(v.state.doc.line(i + 1).from, { y: 'center' }) });
        await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
      },
      rowElement: (i) => {
        const v = view;
        if (!v || i >= v.state.doc.lines) return null;
        const from = v.state.doc.line(i + 1).from;
        if (from < v.viewport.from || from > v.viewport.to || foldOver(v, i + 1)) return null;
        const { node } = v.domAtPos(from);
        const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
        return el?.closest('.cm-line') ?? null;
      },
    }),
  );

  // ── State Creation ─────────────────────────────────────────────────────────

  function readOnlyExt(ro: boolean, hk?: { key: string; persist: boolean } | null): Extension {
    if (ro) return EditorState.readOnly.of(true);
    return [
      EditorState.readOnly.of(false), closeBrackets(), EditorState.allowMultipleSelections.of(true),
      rectangularSelection(), crosshairCursor(),
      EditorView.clickAddsSelectionRange.of((e) => e.altKey && !e.shiftKey && !e.metaKey),
      EditorView.domEventHandlers({
        mousedown(event, v) {
          if (!event.altKey || event.button !== 0 || event.shiftKey || event.metaKey || v.state.selection.ranges.length <= 1) return false;
          const pos = v.posAtCoords({ x: event.clientX, y: event.clientY });
          const hit = pos == null ? -1 : v.state.selection.ranges.findIndex((r) => r.empty && Math.abs(r.head - pos) <= 1);
          if (hit < 0) return false;
          event.preventDefault();
          v.dispatch({ selection: EditorSelection.create(v.state.selection.ranges.filter((_, i) => i !== hit)) });
          return true;
        },
        keydown(e) {
          if (e.altKey && e.shiftKey && !e.metaKey && !e.ctrlKey && (e.code === 'KeyI' || e.key === 'I' || e.key === 'i')) {
            e.preventDefault(); splitSelectionToLines(); return true;
          }
          return false;
        },
      }),
      keymap.of([...closeBracketsKeymap, ...historyKeymap]),
      history(ideMode || hk ? { minDepth: 500 } : undefined),
    ];
  }

  function createState(filePath: string, fileContent: string): EditorState {
    appliedDialect = sqlDialect;
    appliedReadOnly = readOnly;
    const hk = readOnly ? null : (historyKey?.(filePath) ?? null);
    const limit = highlightLineLimit;
    const doc = limit > 0 ? Text.of(fileContent.split(/\r\n?|\n/)) : null;
    appliedPlain = !!doc && hasLongLine(doc, limit);
    appliedLanguage = language;
    const langExt = cmLangFor(filePath, language);
    appliedLangReady = langExt != null || !cmLangPending(langExtOf(filePath, language));

    const baseExtensions: Extension[] = [
      ...(minimal
        ? []
        : [
            lineNumbers(),
            ideBpCompartment.of(buildIdeBpExtIfLoaded(filePath)),
            foldGutter({
              markerDOM(open) {
                const s = document.createElement('span');
                s.className = 'cm-fold-marker';
                s.textContent = open ? '⌄' : '›';
                return s;
              },
            }),
          ]),
      indentOnInput(), bracketMatching(), lintGutter(),
      ideCompartment.of(buildIdeExtIfLoaded(filePath)),
      ideDepsCompartment.of([]),
      drawSelection(),
      readOnlyCompartment.of(readOnlyExt(readOnly, hk)),
      completionCompartment.of(completionExt()),
      placeholderCompartment.of(placeholderExt((appliedPlaceholder = placeholder))),
      wrapCompartment.of(wrapExt((appliedWrap = wrap || (ideMode && idePrefs.wordWrap)))),
      placeholderTheme, search({ top: false }),
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
      langCompartment.of(langExt ?? []),
      fallbackBpGutterExt,
      ...(hk || (ideMode && !readOnly) ? [historySaver] : []),
      ...(oncopy || ideMode ? [copyRecorder] : []),
    ];

    if (ideMode && !readOnly && coreHistMod) {
      const restored = coreHistMod.restoreDurableIdeState(filePath, fileContent, savedContent, baseExtensions);
      if (restored) {
        pendingScrollTop = restored.scrollTop;
        if (restored.restoredDraft != null) {
          const draft = restored.restoredDraft;
          queueMicrotask(() => { lastEmitted = draft; onchange?.(draft); });
        }
        return restored.state;
      }
    }

    if (hk?.persist) prepareEditorHistory();
    const saved = hk ? loadEditorState(hk.key) : null;
    if (saved && (!saved.fromDisk || saved.json.doc === fileContent)) {
      try {
        let st = EditorState.fromJSON(saved.json, { extensions: baseExtensions }, { history: historyField });
        if (st.doc.toString() !== fileContent) st = st.update({ changes: { from: 0, to: st.doc.length, insert: fileContent } }).state;
        pendingScrollTop = saved.scrollTop;
        return st;
      } catch { /* incompatible JSON — start fresh below */ }
    }
    return EditorState.create({ doc: doc ?? fileContent, extensions: baseExtensions });
  }

  // ── Shared history store & durable IDE Undo Journal ────────────────────────
  let livePath = '';
  let pendingScrollTop: number | null = null;
  let historySaveTimer: ReturnType<typeof setTimeout> | null = null;

  function parkHistory(forPath: string, state: EditorState, scrollTop: number, diskSaved?: string | null): void {
    if (readOnly || !forPath) return;
    if (ideMode && coreHistMod) {
      coreHistMod.setSingletonIdeState(forPath, {
        state, scroll: null, scrollTop, chars: state.doc.length,
        langReady: appliedLangReady, language: appliedLanguage, updatedAt: Date.now(),
      });
      coreHistMod.persistIdeUndoJournal(forPath, state, diskSaved ?? savedContent, scrollTop);
    }
    if (!historyKey) return;
    const hk = historyKey(forPath);
    if (!hk) return;
    try {
      saveEditorState(hk.key, state.toJSON({ history: historyField }) as { doc: string }, scrollTop, hk.persist && state.doc.length <= PERSIST_MAX_BYTES);
    } catch { /* state without history field */ }
  }
  function parkLive(): void {
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop, savedContent);
  }
  function applyPendingScroll(): void {
    const top = pendingScrollTop;
    pendingScrollTop = null;
    if (top == null || top <= 0) return;
    requestAnimationFrame(() => { if (view) view.scrollDOM.scrollTop = top; });
  }
  const historySaver = EditorView.updateListener.of((u) => {
    if (!u.docChanged) return;
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    if (u.state.doc.length > PERSIST_MAX_BYTES) return;
    historySaveTimer = setTimeout(() => {
      historySaveTimer = null;
      if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop, savedContent);
    }, ideMode ? 600 : 1500);
  });
  const copyRecorder = EditorView.domEventHandlers({
    copy: (_e, v) => { recordCopy(v.state); return false; },
    cut: (_e, v) => { recordCopy(v.state); return false; },
  });
  function recordCopy(state: EditorState): void {
    const text = state.selection.ranges.filter((r) => !r.empty).map((r) => state.sliceDoc(r.from, r.to)).join('\n');
    if (!text) return;
    if (ideMode) coreHistMod?.recordIdeClipboardEntry(text, livePath || path);
    oncopy?.(text);
  }

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string, connId?: string | null): void {
    parkLive();
    for (const [p, k] of keptStates) parkHistory(p, k.state, 0);
    teardownEditor();
    lspCompartment = new Compartment(); ideBpCompartment = new Compartment();
    ideCompartment = new Compartment(); ideDepsCompartment = new Compartment();
    readOnlyCompartment = new Compartment(); completionCompartment = new Compartment();
    placeholderCompartment = new Compartment(); wrapCompartment = new Compartment();
    langCompartment = new Compartment();
    keptStates.clear();
    lightTabMeta.clear();
    sel = null;

    livePath = filePath;
    view = new EditorView({ state: createState(filePath, fileContent), parent: el });
    applyPendingScroll();
    ensureLang(filePath);
    mountIdeExtensions(view, filePath);
    if (!completionSource && lsp) void attachLsp(view, filePath, rootPath, connId);
    if (gotoLine != null) revealLine(gotoLine, gotoCol);
    const built = view;
    untrack(() => emitSelection(built.state));
  }

  // ── Kept per-path states (`<= 8` inactive tabs & `<= 8 MB` total text bytes) ──

  const keptStates = new Map<string, { state: EditorState; scroll: StateEffect<unknown>; chars: number; dialect: SqlDialectName; langReady: boolean; language: string | undefined }>();
  const lightTabMeta = new Map<string, { scrollTop: number; anchor: number; head: number }>();
  const MAX_KEPT_STATES = 24;
  const IDE_MAX_KEPT_STATES = 8;
  const KEPT_STATE_MAX_CHARS = 512_000;
  const IDE_MAX_KEPT_TOTAL_CHARS = 8 * 1024 * 1024;
  const MAX_LIGHT_META = 64;

  function saveLightMeta(forPath: string, scrollTop: number, anchor: number, head: number): void {
    if (!forPath) return;
    lightTabMeta.delete(forPath);
    lightTabMeta.set(forPath, { scrollTop, anchor, head });
    while (lightTabMeta.size > MAX_LIGHT_META) {
      const oldest = lightTabMeta.keys().next().value;
      if (oldest === undefined) break;
      lightTabMeta.delete(oldest);
    }
  }

  function totalKeptChars(): number {
    let sum = 0;
    for (const k of keptStates.values()) sum += k.chars;
    return sum;
  }

  function swapState(fromPath: string, toPath: string, toContent: string): void {
    if (!view) return;
    changes.flush();
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;

    const prevScrollTop = view.scrollDOM.scrollTop;
    const prevSel = view.state.selection.main;
    saveLightMeta(fromPath, prevScrollTop, prevSel.anchor, prevSel.head);

    view.dispatch({ effects: [lspCompartment.reconfigure([]), ideDepsCompartment.reconfigure([])] });

    parkHistory(fromPath, view.state, prevScrollTop, savedContent);
    keptStates.delete(fromPath);
    const docChars = view.state.doc.length;
    if (docChars <= KEPT_STATE_MAX_CHARS) {
      keptStates.set(fromPath, {
        state: view.state, scroll: view.scrollSnapshot(), chars: docChars,
        dialect: appliedDialect, langReady: appliedLangReady, language: appliedLanguage,
      });
    }
    const maxKept = ideMode ? IDE_MAX_KEPT_STATES : MAX_KEPT_STATES;
    while (keptStates.size > maxKept || (ideMode && keptStates.size > 1 && totalKeptChars() > IDE_MAX_KEPT_TOTAL_CHARS)) {
      const oldest = keptStates.keys().next().value;
      if (oldest === undefined) break;
      const ev = keptStates.get(oldest);
      if (ev) parkHistory(oldest, ev.state, 0);
      keptStates.delete(oldest);
    }

    livePath = toPath;
    const kept = keptStates.get(toPath);
    keptStates.delete(toPath);
    if (!kept) {
      const st = createState(toPath, toContent);
      const meta = lightTabMeta.get(toPath);
      if (meta && pendingScrollTop == null) {
        pendingScrollTop = meta.scrollTop;
        const maxPos = st.doc.length;
        const nextSt = st.update({
          selection: { anchor: Math.min(maxPos, Math.max(0, meta.anchor)), head: Math.min(maxPos, Math.max(0, meta.head)) },
        }).state;
        view.setState(nextSt);
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
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap || (ideMode && idePrefs.wordWrap)))),
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        kept.scroll,
      ];
      if (ideMod && isIdeActive()) {
        effects.push(
          ideBpCompartment.reconfigure(buildIdeBpExtIfLoaded(toPath)),
          ideCompartment.reconfigure(buildIdeExtIfLoaded(toPath)),
        );
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

  function revealLine(line: number, col?: number | null): void {
    if (!view) return;
    const doc = view.state.doc;
    const lineInfo = doc.line(Math.max(1, Math.min(line, doc.lines)));
    const pos = col != null && col > 0 ? Math.min(lineInfo.from + (col - 1), lineInfo.to) : lineInfo.from;
    requestAnimationFrame(() => {
      if (!view) return;
      try {
        view.dispatch({ selection: { anchor: pos, head: pos }, effects: EditorView.scrollIntoView(pos, { y: 'center' }) });
        view.focus();
      } catch { /* doc replaced mid-flight */ }
    });
  }

  // ── Reactive effects ───────────────────────────────────────────────────────

  let prevPath = '';
  let prevRoot = '';
  let prevConnId: string | null = null;
  let prevContent = '';

  $effect(() => {
    const el = container;
    const curPath = path;
    const curContent = content;
    const curRoot = root;
    const curConn = connectionId ?? null;
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
      if (curContent !== view.state.doc.toString()) {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: curContent } });
      }
    } else if (curContent !== prevContent) {
      prevContent = curContent;
    }
  });

  // Live IDE Preferences listener (`otto.ide.preferences.v1` / Color Scheme picker).
  $effect(() => {
    if (!ideMode || typeof window === 'undefined') return;
    const onPrefs = () => {
      if (coreThemeMod) idePrefs = coreThemeMod.readIdeEditorPreferences();
      if (!view) return;
      const effects: StateEffect<unknown>[] = [
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap || (ideMode && idePrefs.wordWrap)))),
      ];
      if (ideMod && ideMounted && isIdeActive()) {
        effects.push(ideCompartment.reconfigure(buildIdeExtIfLoaded()));
      }
      view.dispatch({ effects });
    };
    window.addEventListener('otto:ide-prefs-changed', onPrefs);
    return () => window.removeEventListener('otto:ide-prefs-changed', onPrefs);
  });

  // Live IDE StateEffect updates (zero remounts, preserves undo & multi-cursors).
  $effect(() => {
    const sc = savedContent;
    if (!view) return;
    if (!ideMounted && sc != null) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setSavedContentEffect.of(sc) });
  });
  $effect(() => {
    const blame = blameByLine;
    if (!view) return;
    if (!ideMounted && blame != null) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setBlameLinesEffect.of(blame) });
  });
  $effect(() => {
    const bps = breakpoints;
    const muted = breakpointsMuted;
    if (!view) return;
    if (!ideMounted && (bps.length > 0 || onToggleBreakpoint)) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setBreakpointsEffect.of({ breakpoints: bps, muted }) });
  });
  $effect(() => {
    const items = gutterItems;
    if (!view) return;
    if (!ideMounted && items.length > 0) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setGutterItemsEffect.of(items) });
  });
  $effect(() => {
    const pLine = pausedLine;
    const inlineVals = inlineValues;
    if (!view) return;
    if (!ideMounted && (pLine != null || inlineVals.length > 0)) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setDebugDecorationsEffect.of({ pausedLine: pLine, inlineValues: inlineVals }) });
  });
  $effect(() => {
    const peers = peerCursors;
    if (!view) return;
    if (!ideMounted && peers.length > 0) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setPeerCursorsEffect.of(peers) });
  });
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
    prevCompletion = src;
    view.dispatch({ effects: completionCompartment.reconfigure(completionExt()) });
  });
  $effect(() => {
    const ro = readOnly;
    if (!view || ro === appliedReadOnly) return;
    appliedReadOnly = ro;
    const hk = ro ? null : untrack(() => historyKey?.(livePath) ?? null);
    view.dispatch({ effects: readOnlyCompartment.reconfigure(readOnlyExt(ro, hk)) });
  });
  $effect(() => {
    const on = wrap || (ideMode && idePrefs.wordWrap);
    if (!view || on === appliedWrap) return;
    appliedWrap = on;
    view.dispatch({ effects: wrapCompartment.reconfigure(wrapExt(on)) });
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
    appliedPlaceholder = text;
    view.dispatch({ effects: placeholderCompartment.reconfigure(placeholderExt(text)) });
  });
  $effect(() => {
    const scheme = ui.resolvedScheme;
    const _eff = effectiveIdeScheme;
    void _eff;
    if (view) view.dispatch({ effects: themeCompartment.reconfigure(themeExt(scheme, appliedPlain)) });
  });

  let prevGoto: string | null = null;
  $effect(() => {
    const line = gotoLine;
    const col = gotoCol;
    if (!view || line == null) return;
    const key = `${line}:${col ?? ''}`;
    if (key === prevGoto) return;
    prevGoto = key;
    revealLine(line, col);
  });

  let prevJumpKey: string | null = null;
  $effect(() => {
    const j = jumpToLine;
    if (!view || j == null) return;
    const line = typeof j === 'number' ? j : j.line;
    const col = typeof j === 'number' ? null : (j.column ?? null);
    const key = `${line}:${col ?? ''}:${typeof j === 'number' ? '' : String(j.seq ?? '')}`;
    if (key === prevJumpKey) return;
    prevJumpKey = key;
    revealLine(line, col);
  });

  const unregisterParker = registerLiveParker(() => { if (historySaveTimer !== null) parkLive(); });
  onDestroy(() => {
    unregisterParker();
    changes.flush();
    parkLive();
    for (const [p, k] of keptStates) parkHistory(p, k.state, 0);
    keptStates.clear();
    lightTabMeta.clear();
    teardownEditor();
  });
</script>

<div
  class="code-editor-outer"
  class:is-ide={ideMode}
  data-lang={language ?? ''}
  data-ide-scheme={ideMode ? effectiveIdeScheme : undefined}
  style:--ide-editor-fs={ideMode ? `${idePrefs.fontSize}px` : undefined}
  style:--ide-editor-lh={ideMode ? String(idePrefs.lineHeight) : undefined}
>
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
