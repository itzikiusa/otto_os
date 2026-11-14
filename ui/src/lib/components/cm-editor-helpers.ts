// Shared CodeMirror 6 plumbing for `CodeEditor.svelte`:
// LSP WebSocket attachment & capability caching, long-line plain-mode scanner,
// bounded `keptStates` / `lightTabMeta` LRU cache, whole-document ⌘F provider,
// read-only/multi-cursor extensions, and fallback selection/quick-fix helpers.

import { autocompletion, closeBrackets, closeBracketsKeymap, startCompletion, type CompletionSource } from '@codemirror/autocomplete';
import { history, historyField, historyKeymap } from '@codemirror/commands';
import { foldedRanges, foldGutter, unfoldEffect } from '@codemirror/language';
import { Compartment, EditorSelection, EditorState, Text, type Extension, type StateEffect } from '@codemirror/state';
import { crosshairCursor, EditorView, keymap, rectangularSelection } from '@codemirror/view';
import { api, baseUrl, getToken, WS_BEARER_SUBPROTOCOL } from '../api/client';
import type { IdeDiagnostic, LspCapabilities } from '../api/types';
import { forgetEditorState, loadEditorState, PERSIST_MAX_BYTES, prepareEditorHistory, saveEditorState } from '../editor-history';
import type { FindProvider } from '../findProviders';
import type { SqlDialectName } from '../sql-dialects';
import { ws } from '../stores/workspace.svelte';
import { toasts } from '../toast.svelte';
import type * as CmIdeMod from './cm-ide-extensions';
import type * as CmIdeHistType from './cm-ide-history';
import type * as CmIdeThemeType from './cm-ide-theme';

export interface CursorChangeInfo {
  line: number; column: number; cursorCount: number;
  selectedLines: number; selectionLength: number;
  selections: { anchor: number; head: number }[];
}

export const FALLBACK_IDE_PREFS: CmIdeThemeType.IdeEditorPreferences = {
  colorScheme: 'auto', darkColorScheme: 'jetbrains-islands-dark', lightColorScheme: 'jetbrains-islands-light',
  fontSize: 13, lineHeight: 1.55, tabSize: 4, wordWrap: false,
  inlayHints: true, codeVision: true, rainbowBrackets: true, showBreadcrumbs: true, autoFoldImports: true,
  compactHover: true, javaUnresolvedImports: true, lombokSupport: true, unusedImports: true,
  formatOnSave: false, keymapPreset: 'jetbrains',
};

export const lightEditorTheme = EditorView.theme({
  '&': { color: 'var(--text)', backgroundColor: 'transparent' },
  '.cm-content': { caretColor: 'var(--text)' },
  '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
  '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: 'var(--accent-line)' },
  '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--text-dim) 8%, transparent)' },
  '.cm-gutters': { backgroundColor: 'transparent', color: 'var(--text-dim)', border: 'none' },
  '.cm-activeLineGutter': { backgroundColor: 'transparent' },
}, { dark: false });

export const placeholderEditorTheme = EditorView.theme({ '.cm-placeholder': { color: 'var(--text-dim)', fontStyle: 'normal' } });

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

export function extOf(p: string): string {
  if (p.startsWith('jdt://')) return 'java';
  const clean = p.split('?')[0];
  const base = clean.slice(clean.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  return dot < 0 ? '' : base.slice(dot + 1).toLowerCase();
}

export const langExtOf = (filePath: string, hint?: string): string => extOf(filePath) || (hint ?? '');

export function isManifestPath(p: string): boolean {
  const b = (p.split('?')[0].split('/').pop() ?? '').toLowerCase();
  return (
    b === 'go.mod' || b === 'pom.xml' || b === 'package.json' || b === 'cargo.toml' ||
    b === 'pyproject.toml' || b === 'build.gradle' || b === 'build.gradle.kts' ||
    /^requirements(?:[-_.][\w-]+)?\.txt$/.test(b)
  );
}

export function lspLangFor(filePath: string, ideMode: boolean, language?: string): string | null {
  const ext = extOf(filePath);
  if (ext === 'csv' || ext === 'tsv') return null;
  if (EXT_TO_LSP_LANG[ext]) return EXT_TO_LSP_LANG[ext];
  if (!ideMode || !language || language === 'csv' || language === 'tsv' || language === 'plaintext') return null;
  if (language === 'shell') return 'shellscript';
  if (language === 'jsonl') return 'json';
  return language;
}

export function uriToPath(uri: string): string {
  if (!uri.startsWith('file://')) return uri;
  try { return decodeURIComponent(uri.slice('file://'.length)); }
  catch { return uri.slice('file://'.length); }
}

function lspWsUrl(lang: string, rootPath: string, connId?: string | null): string {
  const base = new URL(baseUrl());
  const proto = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const params = new URLSearchParams({ lang, root: rootPath });
  if (connId) params.set('connection_id', connId);
  return `${proto}//${base.host}/ws/lsp?${params.toString()}`;
}

let capabilitiesCache: LspCapabilities | null = null;
let capabilitiesFetching: Promise<LspCapabilities | null> | null = null;

async function getLspCapabilities(force = false): Promise<LspCapabilities | null> {
  if (force) { capabilitiesCache = null; capabilitiesFetching = null; }
  if (capabilitiesCache) return capabilitiesCache;
  if (capabilitiesFetching) return capabilitiesFetching;
  capabilitiesFetching = api.get<LspCapabilities>('/lsp/capabilities')
    .then((c) => ((capabilitiesCache = c), c))
    .catch(() => null);
  return capabilitiesFetching;
}

export async function attachCodeEditorLsp(opts: {
  editorView: EditorView; filePath: string; rootPath: string; connId?: string | null;
  ideMode: boolean; language?: string; lspCompartment: Compartment;
  isCurrent: () => boolean; ensureIdeMod: () => Promise<typeof CmIdeMod>;
  onLocationJump: (uri: string, line: number, column: number) => void;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
}): Promise<void> {
  try {
    const lspLang = lspLangFor(opts.filePath, opts.ideMode, opts.language);
    if (!lspLang || !opts.rootPath) return;
    if (!opts.connId) {
      let caps = await getLspCapabilities(false);
      if (opts.ideMode && caps && !caps.servers.some((s) => s.lang === lspLang && s.available)) {
        caps = await getLspCapabilities(true);
      }
      if (!caps?.servers.some((s) => s.lang === lspLang && s.available)) return;
    }
    if (!opts.isCurrent()) return;

    const wsUri = lspWsUrl(lspLang, opts.rootPath, opts.connId);
    const token = getToken();
    const protocols = token ? [WS_BEARER_SUBPROTOCOL, token] : undefined;
    const rootUri = `file://${opts.rootPath}`;
    const documentUri = opts.filePath.includes('://') ? opts.filePath : `file://${opts.filePath}`;
    const mod = await opts.ensureIdeMod();
    if (!opts.isCurrent()) return;

    try {
      const lspExtensions = await mod.buildPooledLspExtensions({
        wsUri, protocols, rootPath: opts.rootPath, connId: opts.connId,
        filePath: opts.filePath, languageId: lspLang,
        onLocationJump: opts.onLocationJump, onDiagnostics: opts.onDiagnostics,
      });
      if (!opts.isCurrent()) return;
      opts.editorView.dispatch({ effects: opts.lspCompartment.reconfigure(lspExtensions) });
    } catch {
      if (!opts.isCurrent()) return;
      const fallbackExt = mod.createDirectLspExtension({
        wsUri, protocols, rootUri, documentUri, filePath: opts.filePath, languageId: lspLang,
        onLocationJump: opts.onLocationJump, onDiagnostics: opts.onDiagnostics ?? undefined,
      });
      opts.editorView.dispatch({ effects: opts.lspCompartment.reconfigure(fallbackExt) });
    }
  } catch { /* LSP failures are silent — editor still shows highlighted code */ }
}

export function hasLongLine(doc: Text, limit: number): boolean {
  if (doc.length <= limit) return false;
  let run = 0;
  for (const it = doc.iter(); !it.next().done; ) {
    if (it.lineBreak) run = 0;
    else if ((run += it.value.length) > limit) return true;
  }
  return false;
}

export function computeCursorChangeInfo(state: EditorState): CursorChangeInfo {
  const { head } = state.selection.main;
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
  return {
    line: mainLn.number, column: head - mainLn.from + 1,
    cursorCount: state.selection.ranges.length,
    selectedLines: touched.size, selectionLength: selLen,
    selections: state.selection.ranges.map((r) => ({ anchor: r.anchor, head: r.head })),
  };
}

export function fallbackSelectionRange(state: EditorState): CmIdeMod.IdeEditorSelectionRange {
  const { doc, selection } = state;
  const { from, to } = selection.main;
  if (from === to) {
    const ln = doc.lineAt(from);
    return { from: ln.from, to: ln.to, startLine: ln.number, endLine: ln.number, text: ln.text };
  }
  const startLine = doc.lineAt(from).number;
  const endLine = to > from && doc.lineAt(to).from === to ? doc.lineAt(to - 1).number : doc.lineAt(to).number;
  return { from, to, startLine, endLine, text: state.sliceDoc(from, to) };
}

export function applyFallbackQuickFixEdits(view: EditorView, edits: CmIdeMod.IdeConcreteTextEdit[]): void {
  const doc = view.state.doc;
  const specs = edits.map((e) => {
    const sLn = doc.line(Math.max(1, Math.min(doc.lines, e.start_line)));
    const eLn = doc.line(Math.max(1, Math.min(doc.lines, e.end_line)));
    const from = Math.min(doc.length, sLn.from + Math.max(0, e.start_col - 1));
    const to = Math.max(from, Math.min(doc.length, eLn.from + Math.max(0, e.end_col - 1)));
    return { from, to, insert: e.new_text };
  });
  view.dispatch({ changes: specs, userEvent: 'input.quickfix' });
}

const SPACE_TRIGGER_RE = /(?:^|[\s({,])(?:from|join|where|and|or|on|into|update|set|by|using|select)\s$/i;
const PUNCT_TRIGGER_LANGS = new Set(['js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs', 'json', 'jsonc']);

export function createCompletionExtension(
  completionSource: CompletionSource | null, autoTriggerGate: ((state: EditorState, pos: number) => boolean) | null, getLangKey: () => string,
): Extension {
  if (!completionSource) return autocompletion();
  const autoTrigger = EditorView.updateListener.of((u) => {
    if (!u.docChanged || !u.view.hasFocus) return;
    let fire = false;
    for (const tr of u.transactions) {
      if (!tr.isUserEvent('input.type') && !tr.isUserEvent('input.paste')) continue;
      tr.changes.iterChanges((_fa, _ta, _fb, _tb, ins) => {
        if (ins.length !== 1) return;
        const text = ins.toString();
        if (text === '.') fire = true;
        else if (text === ' ') {
          const head = u.state.selection.main.head, back = u.state.sliceDoc(Math.max(0, head - 48), head);
          if (SPACE_TRIGGER_RE.test(back) || (/[({,]\s$/.test(back) && PUNCT_TRIGGER_LANGS.has(getLangKey()))) fire = true;
        }
      });
    }
    if (fire && (!autoTriggerGate || autoTriggerGate(u.state, u.state.selection.main.head))) {
      const v = u.view; setTimeout(() => startCompletion(v), 0);
    }
  });
  return [autocompletion({ override: [completionSource], activateOnTyping: true, maxRenderedOptions: 40 }), autoTrigger];
}

export function buildReadOnlyExtension(
  ro: boolean, ideMode: boolean, hk: { key: string; persist: boolean } | null | undefined, onSplitLines: () => void,
): Extension {
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
          e.preventDefault(); onSplitLines(); return true;
        }
        return false;
      },
    }),
    keymap.of([...closeBracketsKeymap, ...historyKeymap]),
    history(ideMode || hk ? { minDepth: 500 } : undefined),
  ];
}

const GUTTER_BTN_EXCL = '.cm-ide-run-btn, .cm-ide-gutter-run-btn, .cm-ide-lightbulb-btn, .cm-ide-dep-gutter-btn, .cm-fold-marker, .cm-ide-diff-bar';

export function createFallbackBpGutterHandlers(opts: {
  isIdeMounted: () => boolean; getToggle: () => ((line: number) => void) | null; getEdit: () => ((line: number, event?: MouseEvent) => void) | null;
}): Extension {
  return EditorView.domEventHandlers({
    mousedown(ev, v) {
      const onToggle = opts.getToggle(), onEdit = opts.getEdit();
      if (opts.isIdeMounted() || ev.button !== 0 || ev.altKey || ev.metaKey || ev.ctrlKey || (!onToggle && !onEdit)) return false;
      if (!(ev.target instanceof Element) || !ev.target.closest('.cm-gutters') || ev.target.closest(GUTTER_BTN_EXCL)) return false;
      const ln = v.state.doc.lineAt(v.lineBlockAtHeight(ev.clientY - v.documentTop).from).number;
      if (ev.shiftKey && onEdit) { ev.preventDefault(); onEdit(ln, ev); return true; }
      if (onToggle) { ev.preventDefault(); onToggle(ln); return true; }
      return false;
    },
    contextmenu(ev, v) {
      const onEdit = opts.getEdit();
      if (opts.isIdeMounted() || !onEdit || !(ev.target instanceof Element) || !ev.target.closest('.cm-gutters') || ev.target.closest(GUTTER_BTN_EXCL)) return false;
      const ln = v.state.doc.lineAt(v.lineBlockAtHeight(ev.clientY - v.documentTop).from).number;
      ev.preventDefault(); ev.stopPropagation(); onEdit(ln, ev); return true;
    },
  });
}

function foldOver(v: EditorView, ln: number): { from: number; to: number } | null {
  const line = v.state.doc.line(ln);
  let hit: { from: number; to: number } | null = null;
  foldedRanges(v.state).between(line.from, line.to, (from, to) => {
    if (from < line.to && to > line.from) { hit = { from, to }; return false; }
  });
  return hit;
}

export function buildEditorFindProviderSpec(getView: () => EditorView | null): FindProvider {
  return {
    root: () => getView()?.dom ?? null,
    count: () => getView()?.state.doc.lines ?? 0,
    text: (i) => { const v = getView(); return v && i < v.state.doc.lines ? v.state.doc.line(i + 1).text : ''; },
    reveal: async (i) => {
      const v = getView();
      if (!v || i >= v.state.doc.lines) return;
      for (let n = 0, fold = foldOver(v, i + 1); fold && n < 32; n++, fold = foldOver(v, i + 1)) {
        v.dispatch({ effects: unfoldEffect.of(fold) });
      }
      v.dispatch({ effects: EditorView.scrollIntoView(v.state.doc.line(i + 1).from, { y: 'center' }) });
      await new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r())));
    },
    rowElement: (i) => {
      const v = getView();
      if (!v || i >= v.state.doc.lines) return null;
      const from = v.state.doc.line(i + 1).from;
      if (from < v.viewport.from || from > v.viewport.to || foldOver(v, i + 1)) return null;
      const { node } = v.domAtPos(from);
      const el = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
      return el?.closest('.cm-line') ?? null;
    },
  };
}

export function foldMarkerGutter(): Extension {
  return foldGutter({
    markerDOM(open) {
      const s = document.createElement('span'); s.className = 'cm-fold-marker'; s.textContent = open ? '⌄' : '›'; return s;
    },
  });
}

export function revealEditorLine(getView: () => EditorView | null, line: number, col?: number | null): void {
  const v = getView();
  if (!v) return;
  const doc = v.state.doc, lineInfo = doc.line(Math.max(1, Math.min(line, doc.lines)));
  const pos = col != null && col > 0 ? Math.min(lineInfo.from + (col - 1), lineInfo.to) : lineInfo.from;
  requestAnimationFrame(() => {
    const cur = getView();
    if (!cur) return;
    try { cur.dispatch({ selection: { anchor: pos, head: pos }, effects: EditorView.scrollIntoView(pos, { y: 'center' }) }); cur.focus(); }
    catch { /* doc replaced mid-flight */ }
  });
}

export function restoreOrCreateEditorState(opts: {
  filePath: string; fileContent: string; doc: Text | null; savedContent?: string | null;
  baseExtensions: Extension[]; ideMode: boolean; readOnly: boolean;
  hk: { key: string; persist: boolean } | null;
  coreHistMod: typeof CmIdeHistType | null;
  onRestoredDraft: (draft: string) => void;
}): { state: EditorState; scrollTop: number | null } {
  if (opts.ideMode && !opts.readOnly && opts.coreHistMod) {
    const restored = opts.coreHistMod.restoreDurableIdeState(opts.filePath, opts.fileContent, opts.savedContent, opts.baseExtensions);
    if (restored) {
      if (restored.restoredDraft != null) opts.onRestoredDraft(restored.restoredDraft);
      return { state: restored.state, scrollTop: restored.scrollTop };
    }
  }
  if (opts.hk?.persist) prepareEditorHistory();
  const saved = opts.hk ? loadEditorState(opts.hk.key) : null;
  if (saved && (!saved.fromDisk || saved.json.doc === opts.fileContent)) {
    try {
      let st = EditorState.fromJSON(saved.json, { extensions: opts.baseExtensions }, { history: historyField });
      if (st.doc.toString() !== opts.fileContent) st = st.update({ changes: { from: 0, to: st.doc.length, insert: opts.fileContent } }).state;
      return { state: st, scrollTop: saved.scrollTop };
    } catch { /* incompatible JSON */ }
  }
  return { state: EditorState.create({ doc: opts.doc ?? opts.fileContent, extensions: opts.baseExtensions }), scrollTop: null };
}

export function parkEditorHistory(opts: {
  forPath: string; state: EditorState; scrollTop: number; readOnly: boolean; ideMode: boolean;
  diskSaved?: string | null; langReady: boolean; language: string | undefined;
  coreHistMod: typeof CmIdeHistType | null;
  historyKey?: ((p: string) => { key: string; persist: boolean } | null) | null;
}): void {
  if (opts.readOnly || !opts.forPath) return;
  if (opts.ideMode && opts.coreHistMod) {
    opts.coreHistMod.setSingletonIdeState(opts.forPath, {
      state: opts.state, scroll: null, scrollTop: opts.scrollTop, chars: opts.state.doc.length,
      langReady: opts.langReady, language: opts.language, updatedAt: Date.now(),
    });
    opts.coreHistMod.persistIdeUndoJournal(opts.forPath, opts.state, opts.diskSaved, opts.scrollTop);
  }
  const hk = opts.historyKey?.(opts.forPath);
  if (!hk) return;
  try { saveEditorState(hk.key, opts.state.toJSON({ history: historyField }) as { doc: string }, opts.scrollTop, hk.persist && opts.state.doc.length <= PERSIST_MAX_BYTES); }
  catch { /* state without history field */ }
}

export async function sendSelectionToAgentSession(
  filePath: string, language: string | undefined, startLine: number, endLine: number, text: string,
): Promise<void> {
  const sessionId = ws.activeSessionId;
  if (!sessionId || ws.activeSession?.kind !== 'agent') {
    toasts.error('No agent session', 'Open an agent session first.');
    return;
  }
  const snippet = `Re: ${filePath}:${startLine}-${endLine}\n\n\`\`\`${langExtOf(filePath, language)}\n${text}\n\`\`\`\n\n`;
  try {
    await api.post(`/sessions/${sessionId}/input`, { text: snippet, submit: false });
    toasts.success('Sent to agent', `${filePath} lines ${startLine}-${endLine}`);
  } catch {
    toasts.error('Couldn’t send to the session', 'The text couldn’t be injected into the agent session.');
  }
}

export interface KeptTabStateEntry {
  state: EditorState; scroll: StateEffect<unknown>; chars: number;
  dialect: SqlDialectName; langReady: boolean; language: string | undefined;
}

export function createEditorStateCache() {
  const keptStates = new Map<string, KeptTabStateEntry>();
  const lightTabMeta = new Map<string, { scrollTop: number; anchor: number; head: number }>();
  const MAX_KEPT = 24, IDE_MAX_KEPT = 8, KEPT_MAX_CHARS = 512_000, IDE_MAX_TOTAL_CHARS = 8 * 1024 * 1024, MAX_META = 64;
  const totalChars = (): number => { let s = 0; for (const k of keptStates.values()) s += k.chars; return s; };

  return {
    keptStates,
    lightTabMeta,
    saveLightMeta(forPath: string, scrollTop: number, anchor: number, head: number): void {
      if (!forPath) return;
      lightTabMeta.delete(forPath);
      lightTabMeta.set(forPath, { scrollTop, anchor, head });
      while (lightTabMeta.size > MAX_META) {
        const oldest = lightTabMeta.keys().next().value;
        if (oldest === undefined) break;
        lightTabMeta.delete(oldest);
      }
    },
    stashOutgoing(fromPath: string, entry: KeptTabStateEntry, ideMode: boolean, onEvict: (p: string, st: EditorState) => void): void {
      keptStates.delete(fromPath);
      if (entry.chars <= KEPT_MAX_CHARS) keptStates.set(fromPath, entry);
      const maxKept = ideMode ? IDE_MAX_KEPT : MAX_KEPT;
      while (keptStates.size > maxKept || (ideMode && keptStates.size > 1 && totalChars() > IDE_MAX_TOTAL_CHARS)) {
        const oldest = keptStates.keys().next().value;
        if (oldest === undefined) break;
        const ev = keptStates.get(oldest);
        if (ev) onEvict(oldest, ev.state);
        keptStates.delete(oldest);
      }
    },
    takeKept(toPath: string): KeptTabStateEntry | undefined {
      const k = keptStates.get(toPath);
      keptStates.delete(toPath);
      return k;
    },
    evict(paths: Iterable<string> | 'all', historyKey?: ((p: string) => { key: string; persist: boolean } | null) | null): void {
      if (paths === 'all') {
        if (historyKey) for (const p of keptStates.keys()) { const hk = historyKey(p); if (hk) forgetEditorState(hk.key); }
        keptStates.clear(); lightTabMeta.clear();
        return;
      }
      for (const p of paths) {
        keptStates.delete(p); lightTabMeta.delete(p);
        if (historyKey) { const hk = historyKey(p); if (hk) forgetEditorState(hk.key); }
      }
    },
    drainAndClear(onEach: (p: string, st: EditorState) => void): void {
      for (const [p, k] of keptStates) onEach(p, k.state);
      keptStates.clear();
      lightTabMeta.clear();
    },
  };
}
