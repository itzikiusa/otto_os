// Durable IDE Editor History:
// 1. Module-singleton `EditorState` cache (0ms `IDE -> Git -> Threads -> IDE` round-trips).
// 2. Persistent Undo/Redo step journal + unsaved draft buffer (`localStorage['otto.ide.undoJournal.v1']`)
//    so `⌘Z` / `⌘⇧Z` undoes prior edits even after a full page reload or Otto.app restart.
// 3. Persistent 30-item Clipboard Ring (`localStorage['otto.ide.clipboardHistory.v1']`) for `⌘⇧V`
//    ("Paste from History…") and "Compare with Clipboard".
// 4. Shared JetBrains `Git ▸`, `Local History ▸`, and `Compare with Clipboard` context submenus.

import { historyField } from '@codemirror/commands';
import { EditorState, type Extension, type StateEffect } from '@codemirror/state';
import { ViewPlugin, type EditorView } from '@codemirror/view';
import type { MenuItem } from '../contextmenu.svelte.ts';
import {
  clearIdeSensitiveWebStorage,
  IDE_CURSOR_POS_KEY,
  IDE_UNDO_DRAFT_INDEX_KEY,
  IDE_UNDO_DRAFT_PREFIX,
  IDE_UNDO_JOURNAL_KEY,
  isSecretIdePath,
  MAX_JOURNAL_ENTRY_CHARS,
  readIdeUndoDraft,
  readJournalMap,
  removeIdeUndoDraft,
  writeIdeUndoDraftImmediate,
  writeJournalMap,
  type IdeUndoDraftRecord,
  type IdeUndoJournalEntry,
} from './cm-ide-drafts.ts';

export {
  clearIdeSensitiveWebStorage,
  IDE_CURSOR_POS_KEY,
  IDE_UNDO_DRAFT_INDEX_KEY,
  IDE_UNDO_DRAFT_PREFIX,
  IDE_UNDO_JOURNAL_KEY,
  isSecretIdePath,
  readIdeUndoDraft,
  removeIdeUndoDraft,
  writeIdeUndoDraftImmediate,
  type IdeUndoDraftRecord,
  type IdeUndoJournalEntry,
};

/** Returns true when a `.cm-gutters` target should toggle/preview a breakpoint (all gutter cells except ▶, 💡, 🎯↑, fold chevron, or diff bar). */
export function isBpTargetGutterElement(target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
  if (!el || !el.closest('.cm-gutters')) return false;
  return !el.closest('.cm-ide-run-btn, .cm-ide-gutter-run-btn, .cm-ide-lightbulb-btn, .cm-ide-dep-gutter-btn, .cm-fold-marker, .cm-ide-diff-bar');
}

export const IDE_CLIPBOARD_HISTORY_KEY = 'otto.ide.clipboardHistory.v1';

const MAX_SINGLETON_STATES = 24;
const MAX_CLIPBOARD_ITEMS = 30;
const MAX_CLIPBOARD_ITEM_CHARS = 32_000;

export interface SingletonIdeTabState {
  stateJson: { doc: string; [k: string]: unknown };
  scroll: StateEffect<unknown> | null;
  scrollTop: number;
  chars: number;
  langReady: boolean;
  language: string | undefined;
  updatedAt: number;
}

/** Module-singleton map surviving `CodeEditor.svelte` unmount/remount across Otto modules! */
const singletonIdeStates = new Map<string, SingletonIdeTabState>();

export function getSingletonIdeState(filePath: string): SingletonIdeTabState | undefined {
  const hit = singletonIdeStates.get(filePath);
  if (hit) {
    singletonIdeStates.delete(filePath);
    singletonIdeStates.set(filePath, hit);
  }
  return hit;
}

export function setSingletonIdeState(filePath: string, entry: SingletonIdeTabState): void {
  if (!filePath || entry.chars > 512_000) return;
  singletonIdeStates.delete(filePath);
  singletonIdeStates.set(filePath, entry);
  while (singletonIdeStates.size > MAX_SINGLETON_STATES) {
    const oldest = singletonIdeStates.keys().next().value;
    if (oldest === undefined) break;
    singletonIdeStates.delete(oldest);
  }
}

export function evictSingletonIdeState(paths: Iterable<string> | 'all'): void {
  if (paths === 'all') { singletonIdeStates.clear(); return; }
  for (const p of paths) singletonIdeStates.delete(p);
}

// ── Durable Undo/Redo Step Journal (`localStorage['otto.ide.undoJournal.v1']`) ──

export function persistIdeUndoJournal(
  filePath: string,
  state: EditorState,
  savedContent: string | null | undefined,
  scrollTop = 0,
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null,
): void {
  if (!filePath || isSecretIdePath(filePath) || state.doc.length > MAX_JOURNAL_ENTRY_CHARS) return;
  try {
    const stateJson = state.toJSON({ history: historyField }) as { doc: string; history?: unknown };
    const serialized = JSON.stringify(stateJson);
    if (serialized.length > MAX_JOURNAL_ENTRY_CHARS) return;
    const docStr = stateJson.doc;
    const baseSavedContent = savedContent ?? docStr;
    const unsaved = savedContent != null ? docStr !== savedContent : false;
    const map = readJournalMap(storage);
    map[filePath] = {
      stateJson,
      baseSavedContent,
      savedContent: baseSavedContent,
      ...(unsaved ? { draftContent: docStr } : {}),
      unsaved,
      scrollTop,
      updatedAt: Date.now(),
    };
    writeJournalMap(map, storage);
  } catch { /* state without historyField */ }
}

function sanitizeJournalStateJson(raw: { doc: string; [k: string]: unknown }): { doc: string; [k: string]: unknown } {
  const maxPos = typeof raw.doc === 'string' ? raw.doc.length : 0;
  const sel = raw.selection as { main?: number; ranges?: Array<{ anchor?: number; head?: number }> } | undefined;
  if (!sel || !Array.isArray(sel.ranges) || sel.ranges.length === 0) return raw;
  const clampedRanges = sel.ranges.map((r) => ({
    anchor: Math.max(0, Math.min(maxPos, typeof r?.anchor === 'number' && Number.isFinite(r.anchor) ? r.anchor : 0)),
    head: Math.max(0, Math.min(maxPos, typeof r?.head === 'number' && Number.isFinite(r.head) ? r.head : 0)),
  }));
  const mainIdx = typeof sel.main === 'number' && sel.main >= 0 && sel.main < clampedRanges.length ? sel.main : 0;
  return { ...raw, selection: { main: mainIdx, ranges: clampedRanges } };
}

export function restoreIdeStateFromJournal(
  filePath: string,
  diskContent: string,
  extensions: Extension[],
  storage?: Pick<Storage, 'getItem'> | null,
): { state: EditorState; scrollTop: number; restoredDraft: string | null } | null {
  if (!filePath) return null;
  const entry = readJournalMap(storage)[filePath];
  if (!entry?.stateJson || typeof entry.stateJson.doc !== 'string') return null;
  try {
    const safeJson = sanitizeJournalStateJson(entry.stateJson);
    let st = EditorState.fromJSON(safeJson, { extensions }, { history: historyField });
    let restoredDraft: string | null = null;
    if (safeJson.doc === diskContent) {
      // Saved before restart: `doc` matches disk and full `historyField` undo stack is intact.
    } else if (entry.unsaved && entry.baseSavedContent === diskContent) {
      // Unsaved draft before restart and disk base hasn't diverged: keep unsaved draft + undo stack.
      restoredDraft = safeJson.doc;
    } else {
      const prevSel = st.selection.main;
      const anchor = Math.max(0, Math.min(diskContent.length, prevSel.anchor));
      const head = Math.max(0, Math.min(diskContent.length, prevSel.head));
      st = st.update({
        changes: { from: 0, to: st.doc.length, insert: diskContent },
        selection: { anchor, head },
      }).state;
    }
    return { state: st, scrollTop: entry.scrollTop ?? 0, restoredDraft };
  } catch {
    return null;
  }
}

export function forgetIdeUndoJournal(
  filePath: string | 'all',
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null,
): void {
  if (filePath === 'all') {
    const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
    try {
      if (store && 'removeItem' in store && typeof store.removeItem === 'function') store.removeItem(IDE_UNDO_JOURNAL_KEY);
      else writeJournalMap({}, storage);
    } catch { /* ignore */ }
    return;
  }
  const map = readJournalMap(storage);
  if (filePath in map) {
    delete map[filePath];
    writeJournalMap(map, storage);
  }
}

export function restoreDurableIdeState(
  filePath: string,
  diskContent: string,
  savedContent: string | null | undefined,
  extensions: Extension[],
): { state: EditorState; scrollTop: number; restoredDraft: string | null } | null {
  const mem = getSingletonIdeState(filePath);
  if (mem) {
    try {
      const safeJson = sanitizeJournalStateJson(mem.stateJson);
      let st = EditorState.fromJSON(safeJson, { extensions }, { history: historyField });
      let restoredDraft: string | null = null;
      if (st.doc.toString() !== diskContent) {
        if (savedContent != null && savedContent === diskContent) {
          restoredDraft = st.doc.toString();
        } else {
          const prevSel = st.selection.main;
          const anchor = Math.max(0, Math.min(diskContent.length, prevSel.anchor));
          const head = Math.max(0, Math.min(diskContent.length, prevSel.head));
          st = st.update({
            changes: { from: 0, to: st.doc.length, insert: diskContent },
            selection: { anchor, head },
          }).state;
        }
      }
      return { state: st, scrollTop: mem.scrollTop, restoredDraft };
    } catch { /* fall through */ }
  }
  return restoreIdeStateFromJournal(filePath, diskContent, extensions);
}

// ── Persistent 30-Item Clipboard Ring (`localStorage['otto.ide.clipboardHistory.v1']`) ──

export interface IdeClipboardEntry {
  id: string;
  text: string;
  preview: string;
  lineCount: number;
  filePath?: string;
  copiedAt: number;
}

export function readIdeClipboardHistory(storage?: Pick<Storage, 'getItem'> | null): IdeClipboardEntry[] {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (!store) return [];
  try {
    const raw = store.getItem(IDE_CLIPBOARD_HISTORY_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw) as unknown;
    return Array.isArray(arr) ? (arr.filter((x) => x && typeof x.text === 'string') as IdeClipboardEntry[]) : [];
  } catch {
    return [];
  }
}

export function recordIdeClipboardEntry(
  text: string,
  filePath?: string,
  storage?: Pick<Storage, 'getItem' | 'setItem'> | null,
): IdeClipboardEntry[] {
  if (filePath && isSecretIdePath(filePath)) return readIdeClipboardHistory(storage);
  const clean = text.slice(0, MAX_CLIPBOARD_ITEM_CHARS);
  if (!clean.trim()) return readIdeClipboardHistory(storage);
  const prev = readIdeClipboardHistory(storage).filter((it) => it.text !== clean);
  const lines = clean.split('\n');
  const entry: IdeClipboardEntry = {
    id: `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    text: clean,
    preview: lines.slice(0, 2).map((l) => l.trim()).filter(Boolean).join(' ↵ ').slice(0, 96) || clean.slice(0, 96),
    lineCount: lines.length,
    filePath,
    copiedAt: Date.now(),
  };
  const next = [entry, ...prev].slice(0, MAX_CLIPBOARD_ITEMS);
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  try { store?.setItem(IDE_CLIPBOARD_HISTORY_KEY, JSON.stringify(next)); } catch { /* ignore */ }
  return next;
}

export function clearIdeClipboardHistory(storage?: Pick<Storage, 'removeItem'> | null): void {
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  try { store?.removeItem(IDE_CLIPBOARD_HISTORY_KEY); } catch { /* ignore */ }
}

/** Clear all persisted IDE undo journals, unsaved drafts, clipboard history, project caches, and singleton caches on identity change or sign-out. */
export function clearIdeSensitiveStorage(
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem' | 'key' | 'length'> | null,
): void {
  evictSingletonIdeState('all');
  const store = storage ?? (typeof localStorage !== 'undefined' ? localStorage : null);
  if (store) {
    forgetIdeUndoJournal('all', store);
    clearIdeClipboardHistory(store);
    clearIdeSensitiveWebStorage(store);
  }
}

if (typeof window !== 'undefined') {
  const checkAuthCleared = (): void => {
    try {
      const hasToken = Boolean(
        (typeof localStorage !== 'undefined' && localStorage.getItem('otto_token')) ||
        (typeof sessionStorage !== 'undefined' && sessionStorage.getItem('otto_imp_token')),
      );
      if (!hasToken) clearIdeSensitiveStorage();
    } catch { /* ignore */ }
  };
  window.addEventListener('otto:auth-changed', checkAuthCleared);
  window.addEventListener('otto:ide-storage-cleared', () => {
    evictSingletonIdeState('all');
  });
  window.addEventListener('storage', (e) => {
    if (e.key === 'otto_token' && !e.newValue) checkAuthCleared();
  });
}

export async function getLatestClipboardForCompare(storage?: Pick<Storage, 'getItem'> | null): Promise<string> {
  try {
    if (typeof navigator !== 'undefined' && navigator.clipboard?.readText) {
      const sys = await navigator.clipboard.readText();
      if (sys) return sys;
    }
  } catch { /* permission / headless fallback to ring */ }
  return readIdeClipboardHistory(storage)[0]?.text ?? '';
}

// ── Shared `Git ▸`, `Local History ▸` & `Compare with Clipboard` Submenus ────

export interface IdeVcsMenuHooks {
  filePath: string;
  readOnly?: boolean;
  onToggleBlame?: (() => void) | null;
  onPasteFromHistory?: (() => void) | null;
}

export function dispatchIdeCompareEvent(detail: {
  mode: 'local-history' | 'git-revision' | 'git-branch' | 'clipboard' | 'file';
  path: string;
  clipboardText?: string;
  leftContent?: string;
  leftLabel?: string;
}): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('otto:ide-open-compare', { detail }));
}

export function dispatchIdeGitMenuAction(action: string, path: string): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('otto:ide-git-action', { detail: { action, path } }));
}

export function buildIdeVcsContextRows(
  event: MouseEvent,
  reopenMain: () => void,
  hooks: IdeVcsMenuHooks,
): MenuItem[] {
  const reopenSub = (subItems: MenuItem[]) => {
    const synthetic = new MouseEvent('contextmenu', { clientX: event.clientX, clientY: event.clientY, bubbles: true });
    void import('../contextmenu.svelte.ts').then(({ ctxMenu }) => {
      ctxMenu.show(synthetic, [
        { label: '← Back', icon: 'chevronLeft', pinned: true, action: reopenMain },
        { separator: true },
        ...subItems,
      ]);
    });
  };

  const gitSubmenu: MenuItem[] = [
    { label: 'Commit File…', icon: 'commit', hint: '⌘K', action: () => dispatchIdeGitMenuAction('commit', hooks.filePath) },
    { label: 'Add / Stage', icon: 'plus', hint: '⌥⌘A', action: () => dispatchIdeGitMenuAction('stage', hooks.filePath) },
    { label: 'Annotate with Git Blame', icon: 'commit', action: () => hooks.onToggleBlame?.() },
    { label: 'Show Diff', icon: 'split', hint: '⌘D', action: () => dispatchIdeGitMenuAction('diff', hooks.filePath) },
    { label: 'Compare with Revision…', icon: 'clock', action: () => dispatchIdeCompareEvent({ mode: 'git-revision', path: hooks.filePath }) },
    { label: 'Compare with Branch or Tag…', icon: 'branch', action: () => dispatchIdeCompareEvent({ mode: 'git-branch', path: hooks.filePath }) },
    { label: 'Show History', icon: 'clock', action: () => dispatchIdeGitMenuAction('history', hooks.filePath) },
    { label: 'Rollback…', icon: 'undo', hint: '⌥⌘Z', action: () => dispatchIdeGitMenuAction('rollback', hooks.filePath) },
    { separator: true },
    { label: 'Push…', icon: 'arrowUp', hint: '⇧⌘K', action: () => dispatchIdeGitMenuAction('push', hooks.filePath) },
    { label: 'Pull…', icon: 'arrowDown', action: () => dispatchIdeGitMenuAction('pull', hooks.filePath) },
    { label: 'Fetch', icon: 'refresh', action: () => dispatchIdeGitMenuAction('fetch', hooks.filePath) },
    { separator: true },
    { label: 'Branches…', icon: 'branch', action: () => dispatchIdeGitMenuAction('branches', hooks.filePath) },
    { label: 'New Branch…', icon: 'plus', action: () => dispatchIdeGitMenuAction('new-branch', hooks.filePath) },
    { label: 'Stash Changes…', icon: 'stash', action: () => dispatchIdeGitMenuAction('stash', hooks.filePath) },
    { label: 'Unstash Changes…', icon: 'stash', action: () => dispatchIdeGitMenuAction('unstash', hooks.filePath) },
  ];

  const localHistorySubmenu: MenuItem[] = [
    { label: 'Show History…', icon: 'clock', action: () => dispatchIdeCompareEvent({ mode: 'local-history', path: hooks.filePath }) },
    {
      label: 'Put Label…',
      icon: 'tag',
      action: () => {
        if (typeof window !== 'undefined') {
          window.dispatchEvent(new CustomEvent('otto:ide-put-history-label', { detail: { path: hooks.filePath } }));
        }
      },
    },
  ];

  return [
    {
      label: 'Paste from History…',
      icon: 'copy',
      hint: '⇧⌘V',
      disabled: hooks.readOnly,
      action: () => {
        if (hooks.onPasteFromHistory) hooks.onPasteFromHistory();
        else if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent('otto:ide-paste-history'));
      },
    },
    { separator: true },
    { label: 'Local History', icon: 'clock', children: localHistorySubmenu, action: () => reopenSub(localHistorySubmenu) },
    { label: 'Git', icon: 'branch', children: gitSubmenu, action: () => reopenSub(gitSubmenu) },
    {
      label: 'Compare with Clipboard',
      icon: 'split',
      action: () => {
        void getLatestClipboardForCompare().then((clip) => {
          dispatchIdeCompareEvent({ mode: 'clipboard', path: hooks.filePath, clipboardText: clip, leftContent: clip, leftLabel: 'Clipboard' });
        });
      },
    },
  ];
}

export function isManifestPath(p: string): boolean {
  const b = (p.split('?')[0].split('/').pop() ?? '').toLowerCase();
  return (
    b === 'go.mod' || b === 'pom.xml' || b === 'package.json' || b === 'cargo.toml' ||
    b === 'pyproject.toml' || b === 'build.gradle' || b === 'build.gradle.kts' ||
    /^requirements(?:[-_.][\w-]+)?\.txt$/.test(b)
  );
}

export function fallbackSelectionRange(state: EditorState): {
  from: number; to: number; startLine: number; endLine: number; text: string;
} {
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

export function applyFallbackQuickFixEdits(
  view: EditorView,
  edits: readonly { start_line: number; start_col: number; end_line: number; end_col: number; new_text: string }[],
): void {
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

export function createFallbackBpGutterHandlers(opts: {
  isIdeMounted: () => boolean;
  getToggle: () => ((line: number) => void) | null;
  getEdit: () => ((line: number, event?: MouseEvent) => void) | null;
}): Extension {
  return ViewPlugin.fromClass(class {
    private dom: HTMLElement;
    private onDown: (ev: MouseEvent) => void;
    private onCtx: (ev: MouseEvent) => void;
    constructor(v: EditorView) {
      this.dom = v.scrollDOM;
      this.onDown = (ev) => {
        const onToggle = opts.getToggle(), onEdit = opts.getEdit();
        if (opts.isIdeMounted() || ev.button !== 0 || ev.altKey || ev.metaKey || ev.ctrlKey || (!onToggle && !onEdit) || !isBpTargetGutterElement(ev.target)) return;
        const ln = v.state.doc.lineAt(v.lineBlockAtHeight(ev.clientY - v.documentTop).from).number;
        if (ev.shiftKey && onEdit) { ev.preventDefault(); ev.stopPropagation(); onEdit(ln, ev); }
        else if (onToggle) { ev.preventDefault(); ev.stopPropagation(); onToggle(ln); }
      };
      this.onCtx = (ev) => {
        const onEdit = opts.getEdit();
        if (opts.isIdeMounted() || !onEdit || !isBpTargetGutterElement(ev.target)) return;
        const ln = v.state.doc.lineAt(v.lineBlockAtHeight(ev.clientY - v.documentTop).from).number;
        ev.preventDefault(); ev.stopPropagation(); onEdit(ln, ev);
      };
      this.dom.addEventListener('mousedown', this.onDown, true);
      this.dom.addEventListener('contextmenu', this.onCtx, true);
    }
    destroy(): void {
      this.dom.removeEventListener('mousedown', this.onDown, true);
      this.dom.removeEventListener('contextmenu', this.onCtx, true);
    }
  });
}
