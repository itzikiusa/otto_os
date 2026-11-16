// Shared CodeMirror 6 plumbing for `CodeEditor.svelte`:
// 24-tab `keptStates` cache, whole-document ⌘F provider, read-only/multi-cursor
// extensions, durable state restoration, and smart-indent / cursor-position re-exports.

import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
  startCompletion,
  type CompletionSource,
} from '@codemirror/autocomplete';
import { history, historyField, historyKeymap } from '@codemirror/commands';
import { foldedRanges, foldGutter, unfoldEffect } from '@codemirror/language';
import {
  EditorSelection,
  EditorState,
  Prec,
  type ChangeSpec,
  type Extension,
  type SelectionRange,
  type StateEffect,
  type Text,
} from '@codemirror/state';
import { crosshairCursor, EditorView, gutter, keymap, rectangularSelection } from '@codemirror/view';
import {
  forgetEditorState,
  loadEditorState,
  PERSIST_MAX_BYTES,
  prepareEditorHistory,
  saveEditorState,
} from '../editor-history.ts';
import type { FindProvider } from '../findProviders';
import type { SqlDialectName } from '../sql-dialects';
import type * as CmIdeHistType from './cm-ide-history';

export function extOf(p: string): string {
  if (p.startsWith('jdt://')) return 'java';
  const clean = p.split('?')[0];
  const base = clean.slice(clean.lastIndexOf('/') + 1);
  const dot = base.lastIndexOf('.');
  return dot < 0 ? '' : base.slice(dot + 1).toLowerCase();
}

export const langExtOf = (filePath: string, hint?: string): string => extOf(filePath) || (hint ?? '');

export function clampSelectionToDoc(docLen: number, anchor: number, head: number): { anchor: number; head: number } {
  const max = Math.max(0, docLen);
  const safeAnchor = Number.isFinite(anchor) ? Math.max(0, Math.min(max, Math.trunc(anchor))) : 0;
  const safeHead = Number.isFinite(head) ? Math.max(0, Math.min(max, Math.trunc(head))) : safeAnchor;
  return { anchor: safeAnchor, head: safeHead };
}

export function computeMinimalTextUpdate(
  oldText: string,
  newText: string,
  anchor: number,
  head: number,
): { from: number; to: number; insert: string; anchor: number; head: number } | null {
  const normOld = oldText.indexOf('\r') >= 0 ? oldText.replace(/\r\n?/g, '\n') : oldText;
  const normNew = newText.indexOf('\r') >= 0 ? newText.replace(/\r\n?/g, '\n') : newText;
  if (normOld === normNew) return null;
  const minLen = Math.min(normOld.length, normNew.length);
  let prefix = 0;
  while (prefix < minLen && normOld.charCodeAt(prefix) === normNew.charCodeAt(prefix)) prefix++;
  let suffix = 0;
  const maxSuffix = minLen - prefix;
  while (
    suffix < maxSuffix &&
    normOld.charCodeAt(normOld.length - 1 - suffix) === normNew.charCodeAt(normNew.length - 1 - suffix)
  ) {
    suffix++;
  }
  const from = prefix;
  const to = normOld.length - suffix;
  const insert = normNew.slice(prefix, normNew.length - suffix);
  const delta = insert.length - (to - from);

  const mapPos = (p: number): number => {
    const clamped = Math.max(0, Math.min(normOld.length, Number.isFinite(p) ? Math.trunc(p) : 0));
    if (clamped <= from) return clamped;
    if (clamped >= to) return Math.max(0, Math.min(normNew.length, clamped + delta));
    return Math.max(0, Math.min(normNew.length, from + Math.min(insert.length, clamped - from)));
  };

  const mapped = clampSelectionToDoc(normNew.length, mapPos(anchor), mapPos(head));
  return { from, to, insert, anchor: mapped.anchor, head: mapped.head };
}

export interface CursorChangeInfo {
  line: number;
  column: number;
  cursorCount: number;
  selectedLines: number;
  selectionLength: number;
  selections: { anchor: number; head: number }[];
}

export const lightEditorTheme = EditorView.theme(
  {
    '&': { color: 'var(--text)', backgroundColor: 'transparent' },
    '.cm-content': { caretColor: 'var(--text)' },
    '.cm-cursor, .cm-dropCursor': { borderLeftColor: 'var(--text)' },
    '.cm-selectionBackground, &.cm-focused .cm-selectionBackground': { backgroundColor: 'var(--accent-line)' },
    '.cm-activeLine': { backgroundColor: 'color-mix(in srgb, var(--text-dim) 8%, transparent)' },
    '.cm-gutters': { backgroundColor: 'transparent', color: 'var(--text-dim)', border: 'none' },
    '.cm-activeLineGutter': { backgroundColor: 'transparent' },
  },
  { dark: false },
);

export const placeholderEditorTheme = EditorView.theme({
  '.cm-placeholder': { color: 'var(--text-dim)', fontStyle: 'normal' },
  // Centre the chevron in its row only: centring the gutter column itself
  // shifts every margin-top-placed marker down by half the free space.
  '.cm-foldGutter': { inlineSize: '14px', minInlineSize: '14px', maxInlineSize: '14px' },
  '.cm-foldGutter .cm-gutterElement': {
    padding: '0',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    boxSizing: 'border-box',
  },
});

export function createIdeColdGutterPlaceholders(): { bp: Extension; diff: Extension; deps: Extension } {
  return {
    bp: [
      gutter({ class: 'cm-ide-bp-gutter' }),
      EditorView.baseTheme({
        '.cm-ide-bp-gutter, .cm-ide-bp-gutter .cm-gutterElement': {
          inlineSize: '34px',
          minInlineSize: '34px',
          maxInlineSize: '34px',
          paddingInline: '2px 1px',
          boxSizing: 'border-box',
        },
      }),
    ],
    diff: [
      gutter({ class: 'cm-ide-diff-gutter' }),
      EditorView.baseTheme({
        '.cm-ide-diff-gutter, .cm-ide-diff-gutter .cm-gutterElement': {
          inlineSize: '7px',
          minInlineSize: '7px',
          maxInlineSize: '7px',
          padding: '0',
          boxSizing: 'border-box',
        },
      }),
    ],
    deps: [
      Prec.high(gutter({ class: 'cm-ide-dep-gutter' })),
      EditorView.baseTheme({
        '.cm-ide-dep-gutter, .cm-ide-dep-gutter .cm-gutterElement': {
          inlineSize: '18px',
          minInlineSize: '18px',
          maxInlineSize: '18px',
          padding: '0',
          boxSizing: 'border-box',
        },
      }),
    ],
  };
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

export function createPlainLineWatch(opts: {
  getLimit: () => number;
  isPlain: () => boolean;
  setPlain: (target: EditorView, on: boolean) => void;
}): { extension: Extension; cancel: () => void } {
  let plainRecheck: ReturnType<typeof setTimeout> | null = null;
  const cancel = () => {
    if (plainRecheck) clearTimeout(plainRecheck);
    plainRecheck = null;
  };
  const extension = EditorView.updateListener.of((u) => {
    const limit = opts.getLimit();
    if (!u.docChanged || limit <= 0) return;
    if (opts.isPlain()) {
      cancel();
      const target = u.view;
      plainRecheck = setTimeout(() => {
        plainRecheck = null;
        if (!hasLongLine(target.state.doc, limit)) opts.setPlain(target, false);
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
    if (long) opts.setPlain(u.view, true);
  });
  return { extension, cancel };
}

export function createCopyRecorder(onRecord: (text: string) => void): Extension {
  const record = (state: EditorState): void => {
    const text = state.selection.ranges
      .filter((r) => !r.empty)
      .map((r) => state.sliceDoc(r.from, r.to))
      .join('\n');
    if (text) onRecord(text);
  };
  return EditorView.domEventHandlers({
    copy: (_e, v) => {
      record(v.state);
      return false;
    },
    cut: (_e, v) => {
      record(v.state);
      return false;
    },
  });
}

export function computeCursorChangeInfo(state: EditorState): CursorChangeInfo {
  const { head } = state.selection.main;
  const mainLn = state.doc.lineAt(head);
  let selLen = 0;
  const touched = new Set<number>();
  for (const r of state.selection.ranges) {
    selLen += r.to - r.from;
    const sLn = state.doc.lineAt(r.from).number;
    const eLn =
      !r.empty && r.to > r.from && state.doc.lineAt(r.to).from === r.to
        ? state.doc.lineAt(r.to - 1).number
        : state.doc.lineAt(r.to).number;
    for (let ln = sLn; ln <= eLn; ln++) touched.add(ln);
  }
  return {
    line: mainLn.number,
    column: head - mainLn.from + 1,
    cursorCount: state.selection.ranges.length,
    selectedLines: touched.size,
    selectionLength: selLen,
    selections: state.selection.ranges.map((r) => ({ anchor: r.anchor, head: r.head })),
  };
}

const SPACE_TRIGGER_RE = /(?:^|[\s({,])(?:from|join|where|and|or|on|into|update|set|by|using|select)\s$/i;
const PUNCT_TRIGGER_LANGS = new Set(['js', 'jsx', 'ts', 'tsx', 'mjs', 'cjs', 'json', 'jsonc']);

export function createCompletionExtension(
  completionSource: CompletionSource | null,
  autoTriggerGate: ((state: EditorState, pos: number) => boolean) | null,
  getLangKey: () => string,
): Extension {
  if (!completionSource) return autocompletion({ activateOnTyping: true, maxRenderedOptions: 60 });
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
          const head = u.state.selection.main.head;
          const back = u.state.sliceDoc(Math.max(0, head - 48), head);
          if (SPACE_TRIGGER_RE.test(back) || (/[({,]\s$/.test(back) && PUNCT_TRIGGER_LANGS.has(getLangKey()))) {
            fire = true;
          }
        }
      });
    }
    if (fire && (!autoTriggerGate || autoTriggerGate(u.state, u.state.selection.main.head))) {
      const v = u.view;
      setTimeout(() => startCompletion(v), 0);
    }
  });
  return [autocompletion({ override: [completionSource], activateOnTyping: true, maxRenderedOptions: 40 }), autoTrigger];
}

export function splitSelectionIntoLinesSync(view: EditorView): boolean {
  const { state } = view;
  const next: SelectionRange[] = [];
  let changed = false;
  for (const r of state.selection.ranges) {
    const startLn = state.doc.lineAt(r.from).number;
    let endLn = state.doc.lineAt(r.to).number;
    if (!r.empty && r.to > r.from && state.doc.lineAt(r.to).from === r.to) {
      endLn = state.doc.lineAt(r.to - 1).number;
    }
    if (endLn <= startLn) {
      next.push(r);
      continue;
    }
    changed = true;
    for (let ln = startLn; ln <= endLn; ln++) {
      const line = state.doc.line(ln);
      next.push(EditorSelection.range(Math.max(line.from, r.from), Math.min(line.to, r.to)));
    }
  }
  if (!changed || !next.length) return false;
  view.dispatch({ selection: EditorSelection.create(next) });
  return true;
}

export function applyLinePrefixSuffixSync(view: EditorView, prefix: string, suffix: string): boolean {
  if ((!prefix && !suffix) || view.state.readOnly) return false;
  const { doc, selection } = view.state;
  const touched = new Set<number>();
  for (const r of selection.ranges) {
    const startLn = doc.lineAt(r.from).number;
    const endLn =
      !r.empty && r.to > r.from && doc.lineAt(r.to).from === r.to
        ? doc.lineAt(r.to - 1).number
        : doc.lineAt(r.to).number;
    for (let ln = startLn; ln <= endLn; ln++) touched.add(ln);
  }
  const lines = [...touched].sort((a, b) => a - b);
  if (!lines.length) return false;
  const changes: ChangeSpec[] = [];
  const cursors: SelectionRange[] = [];
  const delta = prefix.length + suffix.length;
  lines.forEach((ln, idx) => {
    const line = doc.line(ln);
    if (prefix.length) changes.push({ from: line.from, to: line.from, insert: prefix });
    if (suffix.length) changes.push({ from: line.to, to: line.to, insert: suffix });
    cursors.push(EditorSelection.cursor(line.to + (idx + 1) * delta));
  });
  view.dispatch({ changes, selection: EditorSelection.create(cursors, cursors.length - 1), userEvent: 'input', scrollIntoView: true });
  return true;
}

export function buildReadOnlyExtension(
  ro: boolean,
  ideMode: boolean,
  hk: { key: string; persist: boolean } | null | undefined,
  onSplitLines: () => void,
): Extension {
  if (ro) return EditorState.readOnly.of(true);
  return [
    EditorState.readOnly.of(false),
    closeBrackets(),
    EditorState.allowMultipleSelections.of(true),
    rectangularSelection(),
    crosshairCursor(),
    EditorView.clickAddsSelectionRange.of((e) => e.altKey && !e.shiftKey && !e.metaKey),
    Prec.highest(
      EditorView.domEventHandlers({
        mousedown(event, v) {
          if (!event.altKey || event.button !== 0 || event.shiftKey || event.metaKey || v.state.selection.ranges.length <= 1) {
            return false;
          }
          const pos = v.posAtCoords({ x: event.clientX, y: event.clientY });
          const hit = pos == null ? -1 : v.state.selection.ranges.findIndex((r) => r.empty && Math.abs(r.head - pos) <= 1);
          if (hit < 0) return false;
          event.preventDefault();
          v.dispatch({ selection: EditorSelection.create(v.state.selection.ranges.filter((_, i) => i !== hit)) });
          return true;
        },
        keydown(e, v) {
          if (e.altKey && e.shiftKey && !e.metaKey && !e.ctrlKey && (e.code === 'KeyI' || e.key === 'I' || e.key === 'i')) {
            if (splitSelectionIntoLinesSync(v)) {
              e.preventDefault();
              return true;
            }
            e.preventDefault();
            onSplitLines();
            return true;
          }
          return false;
        },
      }),
    ),
    keymap.of([...closeBracketsKeymap, ...historyKeymap]),
    history(ideMode || hk ? { minDepth: 500 } : undefined),
  ];
}

function foldOver(v: EditorView, ln: number): { from: number; to: number } | null {
  const line = v.state.doc.line(ln);
  let hit: { from: number; to: number } | null = null;
  foldedRanges(v.state).between(line.from, line.to, (from, to) => {
    if (from < line.to && to > line.from) {
      hit = { from, to };
      return false;
    }
  });
  return hit;
}

export function buildEditorFindProviderSpec(getView: () => EditorView | null): FindProvider {
  return {
    root: () => getView()?.dom ?? null,
    count: () => getView()?.state.doc.lines ?? 0,
    text: (i) => {
      const v = getView();
      return v && i < v.state.doc.lines ? v.state.doc.line(i + 1).text : '';
    },
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
      const s = document.createElement('span');
      s.className = 'cm-fold-marker';
      s.textContent = open ? '⌄' : '›';
      return s;
    },
  });
}

export function revealEditorLine(getView: () => EditorView | null, line: number, col?: number | null): void {
  const v = getView();
  if (!v || v.state.doc.lines === 0) return;
  const doc = v.state.doc;
  const lineInfo = doc.line(Math.max(1, Math.min(line, doc.lines)));
  const pos = col != null && col > 0 ? Math.min(lineInfo.from + (col - 1), lineInfo.to) : lineInfo.from;
  requestAnimationFrame(() => {
    const cur = getView();
    if (!cur || cur.state.doc.lines === 0) return;
    const clampedPos = Math.max(0, Math.min(cur.state.doc.length, pos));
    try {
      cur.dispatch({
        selection: { anchor: clampedPos, head: clampedPos },
        effects: EditorView.scrollIntoView(clampedPos, { y: 'center' }),
      });
      cur.focus();
    } catch {
      /* doc replaced mid-flight */
    }
  });
}

function sanitizeSavedJson(json: { doc: string; [k: string]: unknown }): { doc: string; [k: string]: unknown } {
  const docLen = typeof json.doc === 'string' ? json.doc.length : 0;
  const sel = json.selection as { main?: number; ranges?: Array<{ anchor?: number; head?: number }> } | undefined;
  if (!sel || !Array.isArray(sel.ranges) || sel.ranges.length === 0) return json;
  const ranges = sel.ranges.map((r) => clampSelectionToDoc(docLen, r?.anchor ?? 0, r?.head ?? 0));
  const main = typeof sel.main === 'number' && sel.main >= 0 && sel.main < ranges.length ? sel.main : 0;
  return { ...json, selection: { main, ranges } };
}

export function restoreOrCreateEditorState(opts: {
  filePath: string;
  fileContent: string;
  doc: Text | null;
  savedContent?: string | null;
  baseExtensions: Extension[];
  ideMode: boolean;
  readOnly: boolean;
  hk: { key: string; persist: boolean } | null;
  coreHistMod: typeof CmIdeHistType | null;
  onRestoredDraft: (draft: string) => void;
}): { state: EditorState; scrollTop: number | null } {
  if (opts.ideMode && !opts.readOnly && opts.coreHistMod) {
    const restored = opts.coreHistMod.restoreDurableIdeState(
      opts.filePath,
      opts.fileContent,
      opts.savedContent,
      opts.baseExtensions,
    );
    if (restored) {
      if (restored.restoredDraft != null) opts.onRestoredDraft(restored.restoredDraft);
      return { state: restored.state, scrollTop: restored.scrollTop };
    }
  }
  if (opts.hk?.persist) prepareEditorHistory();
  const saved = opts.hk ? loadEditorState(opts.hk.key) : null;
  if (saved && (!saved.fromDisk || saved.json.doc === opts.fileContent)) {
    try {
      const safeJson = sanitizeSavedJson(saved.json);
      let st = EditorState.fromJSON(safeJson, { extensions: opts.baseExtensions }, { history: historyField });
      if (st.doc.toString() !== opts.fileContent) {
        const clamped = clampSelectionToDoc(opts.fileContent.length, st.selection.main.anchor, st.selection.main.head);
        st = st.update({ changes: { from: 0, to: st.doc.length, insert: opts.fileContent }, selection: clamped }).state;
      }
      return { state: st, scrollTop: saved.scrollTop };
    } catch {
      /* incompatible JSON */
    }
  }
  return {
    state: EditorState.create({ doc: opts.doc ?? opts.fileContent, extensions: opts.baseExtensions }),
    scrollTop: null,
  };
}

export function parkEditorHistory(opts: {
  forPath: string;
  state: EditorState;
  scrollTop: number;
  readOnly: boolean;
  ideMode: boolean;
  diskSaved?: string | null;
  langReady: boolean;
  language: string | undefined;
  coreHistMod: typeof CmIdeHistType | null;
  historyKey?: ((p: string) => { key: string; persist: boolean } | null) | null;
}): void {
  if (opts.readOnly || !opts.forPath) return;
  let cachedJson: { doc: string; [k: string]: unknown } | null = null;
  const getJson = (): { doc: string; [k: string]: unknown } | null => {
    if (cachedJson) return cachedJson;
    try {
      cachedJson = opts.state.toJSON({ history: historyField }) as { doc: string; [k: string]: unknown };
    } catch {
      cachedJson = null;
    }
    return cachedJson;
  };
  if (opts.ideMode && opts.coreHistMod) {
    const stateJson = getJson();
    if (stateJson) {
      opts.coreHistMod.setSingletonIdeState(opts.forPath, {
        stateJson,
        scroll: null,
        scrollTop: opts.scrollTop,
        chars: opts.state.doc.length,
        langReady: opts.langReady,
        language: opts.language,
        updatedAt: Date.now(),
      });
    }
    opts.coreHistMod.persistIdeUndoJournal(opts.forPath, opts.state, opts.diskSaved, opts.scrollTop);
  }
  const hk = opts.historyKey?.(opts.forPath);
  if (!hk) return;
  const json = getJson();
  if (json) saveEditorState(hk.key, json, opts.scrollTop, hk.persist && opts.state.doc.length <= PERSIST_MAX_BYTES);
}

export interface KeptTabStateEntry {
  state: EditorState;
  scroll: StateEffect<unknown>;
  chars: number;
  dialect: SqlDialectName;
  langReady: boolean;
  language: string | undefined;
  savedContent?: string | null;
}

export const IDE_MAX_KEPT = 24;

export function createEditorStateCache() {
  const keptStates = new Map<string, KeptTabStateEntry>();
  const lightTabMeta = new Map<string, { scrollTop: number; anchor: number; head: number }>();
  const MAX_KEPT = 24;
  const KEPT_MAX_CHARS = 512_000;
  const IDE_MAX_TOTAL_CHARS = 8 * 1024 * 1024;
  const MAX_META = 300;
  const totalChars = (): number => {
    let s = 0;
    for (const k of keptStates.values()) s += k.chars;
    return s;
  };

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
    stashOutgoing(
      fromPath: string,
      entry: KeptTabStateEntry,
      ideMode: boolean,
      onEvict: (p: string, st: EditorState, savedContent?: string | null) => void,
    ): void {
      keptStates.delete(fromPath);
      if (entry.chars <= KEPT_MAX_CHARS) keptStates.set(fromPath, entry);
      const maxKept = ideMode ? IDE_MAX_KEPT : MAX_KEPT;
      while (keptStates.size > maxKept || (ideMode && keptStates.size > 1 && totalChars() > IDE_MAX_TOTAL_CHARS)) {
        const oldest = keptStates.keys().next().value;
        if (oldest === undefined) break;
        const ev = keptStates.get(oldest);
        if (ev) onEvict(oldest, ev.state, ev.savedContent);
        keptStates.delete(oldest);
      }
    },
    takeKept(toPath: string): KeptTabStateEntry | undefined {
      const k = keptStates.get(toPath);
      keptStates.delete(toPath);
      return k;
    },
    evict(
      paths: Iterable<string> | 'all',
      historyKey?: ((p: string) => { key: string; persist: boolean } | null) | null,
    ): void {
      if (paths === 'all') {
        if (historyKey) {
          for (const p of keptStates.keys()) {
            const hk = historyKey(p);
            if (hk) forgetEditorState(hk.key);
          }
        }
        keptStates.clear();
        lightTabMeta.clear();
        return;
      }
      for (const p of paths) {
        keptStates.delete(p);
        lightTabMeta.delete(p);
        if (historyKey) {
          const hk = historyKey(p);
          if (hk) forgetEditorState(hk.key);
        }
      }
    },
    drainAndClear(onEach: (p: string, st: EditorState, savedContent?: string | null) => void): void {
      for (const [p, k] of keptStates) onEach(p, k.state, k.savedContent);
      keptStates.clear();
      lightTabMeta.clear();
    },
  };
}
