<script lang="ts">
  // CodeMirror 6 editor with LSP hover/diagnostics/completion/definitions,
  // multi-cursor & rectangular Option+drag selection, and full JetBrains IDE mode.
  import { onDestroy, untrack } from 'svelte';
  import {
    EditorView, lineNumbers, keymap, drawSelection,
    rectangularSelection, crosshairCursor, placeholder as cmPlaceholder,
  } from '@codemirror/view';
  import {
    EditorSelection, EditorState, Compartment, Prec, Text,
    type StateEffect, type Extension,
  } from '@codemirror/state';
  import { defaultKeymap, history, historyField, historyKeymap, selectAll } from '@codemirror/commands';
  import {
    loadEditorState, prepareEditorHistory, registerLiveParker, saveEditorState, PERSIST_MAX_BYTES,
  } from '../editor-history';
  import { search, searchKeymap, openSearchPanel } from '@codemirror/search';
  import {
    autocompletion, completionKeymap, startCompletion, closeBrackets, closeBracketsKeymap,
    type CompletionSource,
  } from '@codemirror/autocomplete';
  import { lintGutter, lintKeymap } from '@codemirror/lint';
  import {
    indentOnInput, bracketMatching, foldGutter, foldKeymap,
    defaultHighlightStyle, syntaxHighlighting, foldedRanges, unfoldEffect,
  } from '@codemirror/language';
  import { oneDark, oneDarkTheme } from '@codemirror/theme-one-dark';

  // Language packages load on demand (cm-langs.ts); SQL/Redis are static there.
  import { cmLangNow, cmLangPending, loadCmLang } from './cm-langs';
  import type { SqlDialectName } from '../sql-dialects';
  import { createChangeEmitter } from './changeEmitter';

  import { api, baseUrl, getToken, WS_BEARER_SUBPROTOCOL } from '../api/client';
  import type {
    IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem,
    IdeInlineValue, IdePeerCursor, LspCapabilities,
  } from '../api/types';
  import type * as CmIdeMod from './cm-ide-extensions';
  import { ws } from '../stores/workspace.svelte';
  import { ui } from '../stores/ui.svelte';
  import { keyContext } from '../keys';
  import { registerFindProvider } from '../findProviders';
  import { toasts } from '../toast.svelte';

  const themeCompartment = new Compartment();
  const lightTheme = EditorView.theme(
    {
      '&': { color: 'var(--text)', backgroundColor: 'transparent' },
      '.cm-content': { caretColor: 'var(--text)' },
      '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
      '.cm-selectionBackground': { backgroundColor: 'var(--accent-line)' },
      '&.cm-focused .cm-selectionBackground': { backgroundColor: 'var(--accent-line)' },
      '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--text-dim) 8%, transparent)' },
      '.cm-gutters': { backgroundColor: 'transparent', color: 'var(--text-dim)', border: 'none' },
      '.cm-activeLineGutter': { backgroundColor: 'transparent' },
    },
    { dark: false },
  );
  function themeExt(scheme: 'light' | 'dark', plain = false): Extension {
    if (plain) return scheme === 'dark' ? oneDarkTheme : lightTheme;
    return scheme === 'dark' ? oneDark : [lightTheme, syntaxHighlighting(defaultHighlightStyle)];
  }

  // ── Props ──────────────────────────────────────────────────────────────────

  export interface CursorChangeInfo {
    line: number; column: number; cursorCount: number;
    selectedLines: number; selectionLength: number;
    selections: { anchor: number; head: number }[];
  }

  interface Props {
    path: string;
    content: string;
    root: string;
    language?: string;
    readOnly?: boolean;
    onchange?: (value: string) => void;
    completionSource?: CompletionSource | null;
    autoTriggerGate?: ((state: EditorState, pos: number) => boolean) | null;
    minimal?: boolean;
    onsubmit?: () => void;
    onselect?: (s: { text: string; cursor: number }) => void;
    gotoLine?: number | null;
    gotoCol?: number | null;
    findOwner?: boolean;
    placeholder?: string;
    wrap?: boolean;
    keepStates?: boolean;
    historyKey?: ((path: string) => { key: string; persist: boolean } | null) | null;
    oncopy?: ((text: string) => void) | null;
    lsp?: boolean;
    sqlDialect?: SqlDialectName;
    highlightLineLimit?: number;

    // Pro IDE props (all optional; cm-ide-extensions lazy-loads when active).
    ideMode?: boolean;
    connectionId?: string | null;
    breakpoints?: IdeBreakpoint[];
    onToggleBreakpoint?: ((line: number) => void) | null;
    onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
    gutterItems?: IdeGutterItem[];
    onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug') => void) | null;
    pausedLine?: number | null;
    inlineValues?: IdeInlineValue[];
    peerCursors?: (IdePeerCursor | IdeCollabPeer)[];
    keymapPreset?: 'jetbrains' | 'standard';
    jumpToLine?: { line: number; column?: number | null; seq?: number } | number | null;
    onCursorChange?: ((info: CursorChangeInfo) => void) | null;
    onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string }) => void) | null;
    onQuickFix?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
    onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string }) => void) | null;
    onFormat?: (() => void) | null;
    onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
  }

  let {
    path, content, root, language,
    readOnly = true, onchange, completionSource = null, autoTriggerGate = null,
    minimal = false, onsubmit, onselect, gotoLine = null, gotoCol = null,
    findOwner = true, placeholder = '', wrap = false, keepStates = false,
    historyKey = null, oncopy = null, lsp = false, sqlDialect = 'standard',
    highlightLineLimit = 0,
    ideMode = false, connectionId = null, breakpoints = [],
    onToggleBreakpoint = null, onEditBreakpoint = null,
    gutterItems = [], onGutterAction = null,
    pausedLine = null, inlineValues = [], peerCursors = [],
    keymapPreset = 'jetbrains', jumpToLine = null,
    onCursorChange = null, onDefinitionJump = null, onQuickFix = null,
    onRenameSymbol = null, onFormat = null, onDiagnostics = null,
  }: Props = $props();

  // ── Container & Compartments ───────────────────────────────────────────────

  let container: HTMLDivElement | undefined = $state();
  let view: EditorView | null = null;
  let lspCompartment = new Compartment();
  let ideCompartment = new Compartment();
  let readOnlyCompartment = new Compartment();
  let appliedReadOnly = true;
  let completionCompartment = new Compartment();
  let placeholderCompartment = new Compartment();
  let appliedPlaceholder = '';
  let wrapCompartment = new Compartment();
  let appliedWrap = false;

  const wrapExt = (on: boolean): Extension => (on ? EditorView.lineWrapping : []);
  const placeholderTheme = EditorView.theme({
    '.cm-placeholder': { color: 'var(--text-dim)', fontStyle: 'normal' },
  });
  const placeholderExt = (text: string): Extension => (text ? cmPlaceholder(text) : []);

  // Lazy-loaded IDE extensions module (loaded once IDE mode or an IDE helper runs).
  let ideMod: typeof CmIdeMod | null = null;
  let ideModPromise: Promise<typeof CmIdeMod> | null = null;
  let ideMounted = false;
  let appliedKeymapPreset: 'jetbrains' | 'standard' = 'jetbrains';

  function ensureIdeMod(): Promise<typeof CmIdeMod> {
    if (ideMod) return Promise.resolve(ideMod);
    return (ideModPromise ??= import('./cm-ide-extensions').then((m) => ((ideMod = m), m)));
  }

  function isIdeActive(): boolean {
    return Boolean(
      ideMode || breakpoints.length > 0 || gutterItems.length > 0 ||
      pausedLine != null || inlineValues.length > 0 || peerCursors.length > 0 ||
      onToggleBreakpoint || onGutterAction || onDefinitionJump ||
      onQuickFix || onRenameSymbol || onFormat,
    );
  }

  function getIdeCallbacks(): CmIdeMod.IdeEditorCallbacks {
    return {
      onToggleBreakpoint, onEditBreakpoint, onGutterAction,
      onDefinitionJump, onQuickFix, onRenameSymbol, onFormat, onDiagnostics,
    };
  }

  function buildIdeExtIfLoaded(): Extension {
    if (!ideMod || !isIdeActive()) {
      ideMounted = false;
      return [];
    }
    ideMounted = true;
    appliedKeymapPreset = keymapPreset;
    return ideMod.createIdeExtensions({
      breakpoints, gutterItems, pausedLine, inlineValues, peerCursors,
      keymapPreset, getCallbacks: getIdeCallbacks,
    });
  }

  function mountIdeExtensions(target: EditorView): void {
    if (!isIdeActive()) return;
    if (ideMod) {
      if (!ideMounted && view === target) {
        target.dispatch({ effects: ideCompartment.reconfigure(buildIdeExtIfLoaded()) });
      }
      return;
    }
    void ensureIdeMod().then(() => {
      if (view === target && isIdeActive()) {
        target.dispatch({ effects: ideCompartment.reconfigure(buildIdeExtIfLoaded()) });
      }
    });
  }

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
    mjs: 'javascript', cjs: 'javascript', py: 'python', go: 'go', rs: 'rust',
    json: 'json', jsonc: 'json', html: 'html', htm: 'html', css: 'css', scss: 'scss',
    less: 'less', md: 'markdown', mdx: 'markdown', java: 'java', kt: 'kotlin', kts: 'kotlin',
    sh: 'shellscript', bash: 'shellscript', yaml: 'yaml', yml: 'yaml',
    graphql: 'graphql', gql: 'graphql', proto: 'protobuf',
  };

  function extOf(p: string): string {
    const base = p.slice(p.lastIndexOf('/') + 1);
    const dot = base.lastIndexOf('.');
    return dot < 0 ? '' : base.slice(dot + 1).toLowerCase();
  }

  // ── Selection & Multi-Cursor State ─────────────────────────────────────────

  interface Sel {
    from: number;
    to: number;
    startLine: number;
    endLine: number;
  }

  let sel: Sel | null = $state(null);

  function emitSelection(state: EditorState): void {
    const { from, to, head } = state.selection.main;
    sel = from === to
      ? null
      : { from, to, startLine: state.doc.lineAt(from).number, endLine: state.doc.lineAt(to).number };
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
        line: mainLn.number,
        column: head - mainLn.from + 1,
        cursorCount: state.selection.ranges.length,
        selectedLines: touched.size,
        selectionLength: selLen,
        selections: state.selection.ranges.map((r) => ({ anchor: r.anchor, head: r.head })),
      });
    }
    if (!onselect) return;
    let text: string | undefined = from === to ? '' : undefined;
    onselect({
      get text() { return (text ??= state.sliceDoc(from, to)); },
      cursor: head,
    });
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
    blur: () => { changes.flush(); return false; },
    keydown: (e) => {
      if ((e.metaKey || e.ctrlKey) && changes.pending) changes.flush();
      return false;
    },
  });

  // ── Send-to-agent handler ─────────────────────────────────────────────────

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
  const cmLangFor = (filePath: string, hint?: string): Extension | null =>
    cmLangNow(langExtOf(filePath, hint), appliedDialect);

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
    return EXT_TO_LSP_LANG[extOf(filePath)] ?? (ideMode && language ? language : null);
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
    try { view?.destroy(); } catch { /* ignore */ }
    view = null;
    if (keyContext.openFind === openEditorSearch) keyContext.openFind = null;
  }

  // ── Attach LSP (with native WebSocket JSON-RPC fallback) ──────────────────

  async function attachLsp(editorView: EditorView, filePath: string, rootPath: string, connId?: string | null): Promise<void> {
    try {
      const lspLang = lspLangFor(filePath);
      if (!lspLang || !rootPath) return;
      if (!connId) {
        let caps = await getCapabilities();
        if (ideMode && caps && !caps.servers.some((s) => s.lang === lspLang && s.available)) {
          capabilitiesCache = null;
          capabilitiesFetching = null;
          caps = await getCapabilities();
        }
        if (!caps?.servers.some((s) => s.lang === lspLang && s.available)) return;
      }
      if (view !== editorView) return;

      const wsUri = lspWsUrl(lspLang, rootPath, connId);
      const token = getToken();
      const protocols = token ? [WS_BEARER_SUBPROTOCOL, token] : undefined;
      const rootUri = `file://${rootPath}`;
      const documentUri = `file://${filePath}`;

      try {
        const { languageServerWithClient, LanguageServerClient, WebSocketTransport } = await import(
          '@marimo-team/codemirror-languageserver'
        );
        if (view !== editorView) return;
        const client = new LanguageServerClient({
          rootUri,
          workspaceFolders: [{ name: 'workspace', uri: rootUri }],
          transport: new WebSocketTransport(wsUri, protocols),
        });
        if (onDiagnostics) {
          client.onNotification((n) => {
            if (n.method === 'textDocument/publishDiagnostics' && n.params?.uri === documentUri) {
              onDiagnostics(
                (n.params.diagnostics ?? []).map((d) => ({
                  path: filePath,
                  line: d.range.start.line + 1,
                  column: d.range.start.character + 1,
                  end_line: d.range.end.line + 1,
                  end_column: d.range.end.character + 1,
                  severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : d.severity === 4 ? 'hint' : 'info',
                  message: typeof d.message === 'string' ? d.message : d.message.value,
                  source: d.source ?? null,
                  code: d.code != null ? String(d.code) : null,
                })),
              );
            }
          });
        }
        const lspExtensions: Extension[] = languageServerWithClient({
          client,
          documentUri,
          languageId: lspLang,
          onGoToDefinition: (res) => handleLspJump(res.uri, res.range.start.line + 1, res.range.start.character + 1),
          onShowLocation: (res) => handleLspJump(res.uri, res.range.start.line + 1, res.range.start.character + 1),
        });
        editorView.dispatch({ effects: lspCompartment.reconfigure(lspExtensions) });
      } catch {
        const mod = await ensureIdeMod();
        if (view !== editorView) return;
        const fallbackExt = mod.createDirectLspExtension({
          wsUri, protocols, rootUri, documentUri, filePath, languageId: lspLang,
          onLocationJump: handleLspJump,
          onDiagnostics: onDiagnostics ?? undefined,
        });
        editorView.dispatch({ effects: lspCompartment.reconfigure(fallbackExt) });
      }
    } catch {
      // LSP failures are silent — editor still shows highlighted code
    }
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
      if (fire) {
        const v = u.view;
        setTimeout(() => startCompletion(v), 0);
      }
    });
  }

  const submitKeymap = keymap.of([{
    key: 'Mod-Enter',
    run: () => {
      if (!onsubmit) return false;
      changes.flush();
      onsubmit();
      return true;
    },
  }]);

  function openEditorSearch(): void {
    if (view) openSearchPanel(view);
  }

  // ── Exported Component Methods ─────────────────────────────────────────────

  export function openSearch(): void {
    openEditorSearch();
  }

  export function focus(): void {
    view?.focus();
  }

  export function insertText(text: string): void {
    if (!view || readOnly) return;
    view.dispatch(view.state.replaceSelection(text), { scrollIntoView: true, userEvent: 'input.paste' });
    view.focus();
  }

  /** Add `prefix` before and `suffix` after every selected/cursor line in one undoable step. */
  export function applyPrefixSuffix(prefix: string, suffix: string): void {
    if (!view || readOnly) return;
    const target = view;
    const run = (m: typeof CmIdeMod) => {
      if (view !== target) return;
      if (m.applyLinePrefixSuffix(target, prefix, suffix)) changes.flush();
      target.focus();
    };
    if (ideMod) run(ideMod);
    else void ensureIdeMod().then(run);
  }

  /** Split any multi-line selection into per-line cursors (`⌥⇧I`). */
  export function splitSelectionToLines(): void {
    if (!view) return;
    const target = view;
    const run = (m: typeof CmIdeMod) => {
      if (view !== target) return;
      m.splitSelectionIntoLines(target);
      target.focus();
    };
    if (ideMod) run(ideMod);
    else void ensureIdeMod().then(run);
  }

  /** Scroll to and focus a 1-based line (and optional 1-based column). */
  export function goToLine(line: number, col?: number): void {
    revealLine(line, col);
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
      EditorState.readOnly.of(false),
      closeBrackets(),
      EditorState.allowMultipleSelections.of(true),
      rectangularSelection(),
      crosshairCursor(),
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
            e.preventDefault();
            splitSelectionToLines();
            return true;
          }
          return false;
        },
      }),
      keymap.of([...closeBracketsKeymap, ...historyKeymap]),
      history(hk ? { minDepth: 500 } : undefined),
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
      ideCompartment.of(buildIdeExtIfLoaded()),
      ...(minimal ? [] : [lineNumbers(), foldGutter()]),
      indentOnInput(),
      bracketMatching(),
      lintGutter(),
      drawSelection(),
      readOnlyCompartment.of(readOnlyExt(readOnly, hk)),
      completionCompartment.of(completionExt()),
      placeholderCompartment.of(placeholderExt((appliedPlaceholder = placeholder))),
      wrapCompartment.of(wrapExt((appliedWrap = wrap))),
      placeholderTheme,
      search({ top: false }),
      themeCompartment.of(themeExt(ui.resolvedScheme, appliedPlain)),
      lspCompartment.of([]),
      selectionListener,
      changeListener,
      ...(limit > 0 ? [plainWatch] : []),
      changeFlushHandlers,
      submitKeymap,
      ...(findOwner
        ? [EditorView.domEventHandlers({
            focus: () => { keyContext.openFind = openEditorSearch; return false; },
            blur: () => { if (keyContext.openFind === openEditorSearch) keyContext.openFind = null; return false; },
          })]
        : []),
      Prec.highest(keymap.of([{ key: 'Mod-a', run: selectAll }])),
      keymap.of([...defaultKeymap, ...searchKeymap, ...lintKeymap, ...completionKeymap, ...foldKeymap]),
      langCompartment.of(langExt ?? []),
      ...(hk ? [historySaver] : []),
      ...(oncopy ? [copyRecorder] : []),
    ];

    if (hk?.persist) prepareEditorHistory();
    const saved = hk ? loadEditorState(hk.key) : null;
    if (saved && (!saved.fromDisk || saved.json.doc === fileContent)) {
      try {
        let st = EditorState.fromJSON(saved.json, { extensions: baseExtensions }, { history: historyField });
        if (st.doc.toString() !== fileContent) {
          st = st.update({ changes: { from: 0, to: st.doc.length, insert: fileContent } }).state;
        }
        pendingScrollTop = saved.scrollTop;
        return st;
      } catch { /* incompatible JSON — start fresh below */ }
    }
    return EditorState.create({ doc: doc ?? fileContent, extensions: baseExtensions });
  }

  // ── Shared history store (historyKey) ──────────────────────────────────────
  let livePath = '';
  let pendingScrollTop: number | null = null;
  let historySaveTimer: ReturnType<typeof setTimeout> | null = null;

  function parkHistory(forPath: string, state: EditorState, scrollTop: number): void {
    if (readOnly || !historyKey || !forPath) return;
    const hk = historyKey(forPath);
    if (!hk) return;
    try {
      saveEditorState(
        hk.key,
        state.toJSON({ history: historyField }) as { doc: string },
        scrollTop,
        hk.persist && state.doc.length <= PERSIST_MAX_BYTES,
      );
    } catch { /* state without history field */ }
  }
  function parkLive(): void {
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop);
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
      if (view) parkHistory(livePath, view.state, view.scrollDOM.scrollTop);
    }, 1500);
  });
  const copyRecorder = EditorView.domEventHandlers({
    copy: (_e, v) => { recordCopy(v.state); return false; },
    cut: (_e, v) => { recordCopy(v.state); return false; },
  });
  function recordCopy(state: EditorState): void {
    const text = state.selection.ranges.filter((r) => !r.empty).map((r) => state.sliceDoc(r.from, r.to)).join('\n');
    if (text) oncopy?.(text);
  }

  function buildEditor(el: HTMLDivElement, filePath: string, fileContent: string, rootPath: string, connId?: string | null): void {
    parkLive();
    for (const [p, k] of keptStates) parkHistory(p, k.state, 0);
    teardownEditor();
    lspCompartment = new Compartment();
    ideCompartment = new Compartment();
    readOnlyCompartment = new Compartment();
    completionCompartment = new Compartment();
    placeholderCompartment = new Compartment();
    wrapCompartment = new Compartment();
    langCompartment = new Compartment();
    keptStates.clear();
    sel = null;

    view = new EditorView({ state: createState(filePath, fileContent), parent: el });
    livePath = filePath;
    applyPendingScroll();
    ensureLang(filePath);
    mountIdeExtensions(view);

    if (!completionSource && lsp) void attachLsp(view, filePath, rootPath, connId);
    if (gotoLine != null) revealLine(gotoLine, gotoCol);
    const built = view;
    untrack(() => emitSelection(built.state));
  }

  // ── Kept per-path states (keepStates) ──────────────────────────────────────

  const keptStates = new Map<string, {
    state: EditorState;
    scroll: StateEffect<unknown>;
    dialect: SqlDialectName;
    langReady: boolean;
    language: string | undefined;
  }>();
  const MAX_KEPT_STATES = 24;

  function swapState(fromPath: string, toPath: string, toContent: string): void {
    if (!view) return;
    changes.cancel();
    if (historySaveTimer !== null) clearTimeout(historySaveTimer);
    historySaveTimer = null;
    keptStates.delete(fromPath);
    keptStates.set(fromPath, {
      state: view.state,
      scroll: view.scrollSnapshot(),
      dialect: appliedDialect,
      langReady: appliedLangReady,
      language: appliedLanguage,
    });
    while (keptStates.size > MAX_KEPT_STATES) {
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
      view.setState(createState(toPath, toContent));
      applyPendingScroll();
      ensureLang(toPath);
      mountIdeExtensions(view);
    } else {
      view.setState(kept.state);
      if (plainRecheck) clearTimeout(plainRecheck);
      plainRecheck = null;
      appliedPlain = highlightLineLimit > 0 && hasLongLine(kept.state.doc, highlightLineLimit);
      const effects: StateEffect<unknown>[] = [
        readOnlyCompartment.reconfigure(readOnlyExt((appliedReadOnly = readOnly), readOnly ? null : (historyKey?.(toPath) ?? null))),
        completionCompartment.reconfigure(completionExt()),
        placeholderCompartment.reconfigure(placeholderExt((appliedPlaceholder = placeholder))),
        wrapCompartment.reconfigure(wrapExt((appliedWrap = wrap))),
        themeCompartment.reconfigure(themeExt(ui.resolvedScheme, appliedPlain)),
        kept.scroll,
      ];
      if (ideMod && isIdeActive()) effects.push(ideCompartment.reconfigure(buildIdeExtIfLoaded()));
      appliedDialect = kept.dialect;
      appliedLangReady = kept.langReady;
      appliedLanguage = kept.language;
      if (sqlDialect !== kept.dialect || !kept.langReady || language !== kept.language) {
        appliedDialect = sqlDialect;
        appliedLanguage = language;
        const langExt = cmLangFor(toPath, language);
        appliedLangReady = langExt != null || !cmLangPending(langExtOf(toPath, language));
        effects.push(langCompartment.reconfigure(langExt ?? []));
      }
      const doc = view.state.doc;
      const stale = doc.length !== toContent.length || doc.toString() !== toContent;
      view.dispatch({
        effects,
        ...(stale ? { changes: { from: 0, to: doc.length, insert: toContent } } : {}),
      });
      ensureLang(toPath);
      mountIdeExtensions(view);
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
        view.dispatch({
          selection: { anchor: pos, head: pos },
          effects: EditorView.scrollIntoView(pos, { y: 'center' }),
        });
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
      prevPath = curPath;
      prevContent = curContent;
      untrack(() => swapState(fromPath, curPath, curContent));
    } else if (!view || curPath !== prevPath || curRoot !== prevRoot || curConn !== prevConnId) {
      prevPath = curPath;
      prevRoot = curRoot;
      prevConnId = curConn;
      prevContent = curContent;
      buildEditor(el, curPath, curContent, curRoot, curConn);
    } else if (curContent !== prevContent && !contentEchoed) {
      prevContent = curContent;
      if (curContent !== view.state.doc.toString()) {
        view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: curContent } });
      }
    } else if (curContent !== prevContent) {
      prevContent = curContent;
    }
  });

  // Live IDE StateEffect updates (zero remounts, preserves undo & multi-cursors).
  $effect(() => {
    const bps = breakpoints;
    if (!view) return;
    if (!ideMounted && bps.length > 0) mountIdeExtensions(view);
    else if (ideMod && ideMounted) view.dispatch({ effects: ideMod.setBreakpointsEffect.of(bps) });
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
    else if (ideMod && ideMounted) {
      view.dispatch({ effects: ideMod.setDebugDecorationsEffect.of({ pausedLine: pLine, inlineValues: inlineVals }) });
    }
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
    const on = wrap;
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
    const seq = typeof j === 'number' ? '' : String(j.seq ?? '');
    const key = `${line}:${col ?? ''}:${seq}`;
    if (key === prevJumpKey) return;
    prevJumpKey = key;
    revealLine(line, col);
  });

  const unregisterParker = registerLiveParker(() => {
    if (historySaveTimer !== null) parkLive();
  });
  onDestroy(() => {
    unregisterParker();
    changes.flush();
    parkLive();
    for (const [p, k] of keptStates) parkHistory(p, k.state, 0);
    teardownEditor();
  });
</script>

<div class="code-editor-outer" data-lang={language ?? ''}>
  <div class="code-editor-wrap" dir="ltr" bind:this={container}></div>
  {#if sel}<button class="send-to-agent-btn" onclick={sendToAgent} type="button">Send to agent ↗</button>{/if}
</div>

<style>
  .code-editor-outer { position: relative; width: 100%; height: 100%; }
  .code-editor-wrap { width: 100%; height: 100%; overflow: auto; }
  .send-to-agent-btn {
    position: absolute; top: 6px; inset-inline-end: 10px; z-index: var(--z-sticky);
    padding: 2px 10px; font-size: var(--fs-xs); font-family: var(--font-ui); font-weight: 500;
    color: var(--accent-contrast); background: var(--accent-solid); border: none;
    border-radius: var(--radius-s); cursor: pointer; box-shadow: var(--glass-shadow);
    transition: background var(--dur-fast) var(--ease-out); white-space: nowrap; user-select: none;
  }
  .send-to-agent-btn:hover { background: color-mix(in srgb, var(--accent-solid) 88%, var(--text)); }
  .send-to-agent-btn:active { background: color-mix(in srgb, var(--accent-solid) 80%, var(--bg)); }
  .code-editor-wrap :global(.cm-editor) { height: 100%; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.55; }
  .code-editor-wrap :global(.cm-scroller) { overflow: auto; }
  .code-editor-wrap :global(.cm-editor.cm-focused) { outline: none; }
  .code-editor-wrap:focus-within { box-shadow: inset 0 0 0 1px var(--accent-text); }
</style>
