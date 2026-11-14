// IDE Git/Modified Gutter Indicators (`cm-ide-diff-gutter`) with Clickable Hunk Preview
// & One-Click Rollback, Inline Active-Line Git Blame, and AI Agent + GoLand Context Menu.

import {
  Decoration, EditorView, GutterMarker, WidgetType, gutter, keymap,
  type DecorationSet,
} from '@codemirror/view';
import {
  EditorSelection, EditorState, Prec, RangeSet, StateEffect, StateField,
  type Extension, type Range,
} from '@codemirror/state';
import { copyWithToast } from '../clipboard';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte';
import { diffLineArrays } from '../lineDiff.ts';

export interface IdeEditorSelectionRange {
  from: number;
  to: number;
  startLine: number;
  endLine: number;
  text: string;
}

export interface IdeAgentMenuCallbacks {
  onAgentAction?: ((mode: 'chat' | 'inline', sel: IdeEditorSelectionRange) => void) | null;
  onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string }) => void) | null;
  onImplementationJump?: ((target: { path?: string; line: number; column: number; symbol?: string }) => void) | null;
  onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onFileStructure?: (() => void) | null;
  onQuickFix?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string }) => void) | null;
  onFormat?: (() => void) | null;
  onToggleBlame?: (() => void) | null;
}

export interface IdeDiffHunk {
  kind: 'add' | 'mod' | 'del';
  /** 1-based inclusive start line in current document. */
  newStartLine: number;
  /** 1-based inclusive end line in current document (`< newStartLine` for pure deletion). */
  newEndLine: number;
  /** Anchor line in current document where gutter marker is shown. */
  markerStartLine: number;
  markerEndLine: number;
  oldLines: string[];
  newLines: string[];
}

export const setSavedContentEffect = StateEffect.define<string | null>();
export const setBlameLinesEffect = StateEffect.define<Record<number, string> | null>();

const MAX_DIFF_GUTTER_LINES = 4_000;

export function selectionOrCurrentLine(state: EditorState): IdeEditorSelectionRange {
  const { from, to } = state.selection.main;
  if (from === to) {
    const line = state.doc.lineAt(from);
    return { from: line.from, to: line.to, startLine: line.number, endLine: line.number, text: line.text };
  }
  const startLine = state.doc.lineAt(from).number;
  const endLine = to > from && state.doc.lineAt(to).from === to
    ? state.doc.lineAt(to - 1).number
    : state.doc.lineAt(to).number;
  return { from, to, startLine, endLine, text: state.sliceDoc(from, to) };
}

class DiffGutterMarker extends GutterMarker {
  readonly kind: 'add' | 'mod' | 'del';
  constructor(kind: 'add' | 'mod' | 'del') { super(); this.kind = kind; }
  eq(o: DiffGutterMarker): boolean { return this.kind === o.kind; }
  toDOM(): HTMLElement {
    const bar = document.createElement('span');
    bar.className = `cm-ide-diff-bar cm-ide-diff-${this.kind}`;
    bar.dataset.diffKind = this.kind;
    bar.title = this.kind === 'add'
      ? 'Added lines — click to inspect or rollback hunk'
      : this.kind === 'del'
        ? 'Deleted lines — click to inspect or restore hunk'
        : 'Modified lines — click to inspect or rollback hunk';
    bar.setAttribute('aria-hidden', 'true');
    return bar;
  }
}

const addLineMarker = new DiffGutterMarker('add');
const modLineMarker = new DiffGutterMarker('mod');
const delLineMarker = new DiffGutterMarker('del');

function parseSavedLines(savedContent: string | null | undefined): string[] | null {
  if (savedContent == null) return null;
  const lines = savedContent.split(/\r\n?|\n/);
  return lines.length <= MAX_DIFF_GUTTER_LINES ? lines : null;
}

function buildDiffState(
  doc: EditorState['doc'],
  savedLines: string[] | null,
): { markers: RangeSet<GutterMarker>; hunks: IdeDiffHunk[] } {
  const newLen = doc.lines;
  if (!savedLines || newLen === 0 || newLen > MAX_DIFF_GUTTER_LINES) {
    return { markers: RangeSet.empty, hunks: [] };
  }
  const oldLen = savedLines.length;
  let prefix = 0;
  while (prefix < oldLen && prefix < newLen && doc.line(prefix + 1).text === savedLines[prefix]) prefix++;
  if (prefix === oldLen && prefix === newLen) {
    return { markers: RangeSet.empty, hunks: [] };
  }

  let suffix = 0;
  while (
    suffix < oldLen - prefix &&
    suffix < newLen - prefix &&
    doc.line(newLen - suffix).text === savedLines[oldLen - 1 - suffix]
  ) suffix++;

  const oldMid = oldLen - prefix - suffix;
  const newMid = newLen - prefix - suffix;
  const hunks: IdeDiffHunk[] = [];

  const pushHunk = (oStart: number, oEnd: number, nStart0: number, nEnd0: number): void => {
    const delCount = oEnd - oStart;
    const insCount = nEnd0 - nStart0;
    if (delCount === 0 && insCount === 0) return;
    const oldSlice = savedLines.slice(oStart, oEnd);
    const newSlice: string[] = [];
    for (let k = nStart0; k < nEnd0; k++) newSlice.push(doc.line(k + 1).text);
    if (insCount === 0) {
      const anchor = Math.min(newLen, Math.max(1, nStart0));
      hunks.push({
        kind: 'del',
        newStartLine: nStart0 + 1,
        newEndLine: nStart0,
        markerStartLine: anchor,
        markerEndLine: anchor,
        oldLines: oldSlice,
        newLines: [],
      });
    } else {
      const kind: 'add' | 'mod' = delCount === 0 ? 'add' : 'mod';
      hunks.push({
        kind,
        newStartLine: nStart0 + 1,
        newEndLine: nEnd0,
        markerStartLine: nStart0 + 1,
        markerEndLine: nEnd0,
        oldLines: oldSlice,
        newLines: newSlice,
      });
    }
  };

  if (oldMid === 0 || newMid === 0) {
    pushHunk(prefix, prefix + oldMid, prefix, prefix + newMid);
  } else {
    const oldSlice = savedLines.slice(prefix, prefix + oldMid);
    const newSlice: string[] = new Array(newMid);
    for (let k = 0; k < newMid; k++) newSlice[k] = doc.line(prefix + 1 + k).text;
    const ops = diffLineArrays(oldSlice, newSlice);
    let curO = 0;
    let curN = 0;
    for (let k = 0; k < ops.length; k++) {
      const op = ops[k];
      if (op.op !== 'equal') continue;
      const oIdx = op.oldLineNo - 1;
      const nIdx = op.newLineNo - 1;
      pushHunk(prefix + curO, prefix + oIdx, prefix + curN, prefix + nIdx);
      curO = oIdx + 1;
      curN = nIdx + 1;
    }
    pushHunk(prefix + curO, prefix + oldMid, prefix + curN, prefix + newMid);
  }

  const ranges: Range<GutterMarker>[] = [];
  const lineKind = new Map<number, 'add' | 'mod' | 'del'>();
  for (const h of hunks) {
    for (let ln = h.markerStartLine; ln <= h.markerEndLine; ln++) {
      if (ln >= 1 && ln <= newLen) lineKind.set(ln, h.kind);
    }
  }
  for (const [ln, kind] of [...lineKind.entries()].sort((a, b) => a[0] - b[0])) {
    const marker = kind === 'add' ? addLineMarker : kind === 'del' ? delLineMarker : modLineMarker;
    ranges.push(marker.range(doc.line(ln).from));
  }
  return { markers: ranges.length > 0 ? RangeSet.of(ranges, true) : RangeSet.empty, hunks };
}

/** Revert a single diff hunk back to `savedContent` in one undoable CodeMirror transaction. */
export function rollbackDiffHunk(view: EditorView, hunk: IdeDiffHunk): boolean {
  if (view.state.readOnly) return false;
  const { doc } = view.state;
  const oldText = hunk.oldLines.join('\n');

  if (hunk.kind === 'del') {
    const insertBeforeLine = hunk.newStartLine;
    if (insertBeforeLine <= doc.lines) {
      const pos = doc.line(insertBeforeLine).from;
      const ins = `${oldText}\n`;
      view.dispatch({ changes: { from: pos, to: pos, insert: ins }, selection: EditorSelection.cursor(pos), userEvent: 'undo' });
    } else {
      const pos = doc.length;
      const ins = pos > 0 ? `\n${oldText}` : oldText;
      view.dispatch({ changes: { from: pos, to: pos, insert: ins }, selection: EditorSelection.cursor(pos), userEvent: 'undo' });
    }
    return true;
  }

  const startLn = doc.line(Math.max(1, Math.min(doc.lines, hunk.newStartLine)));
  const endLn = doc.line(Math.max(1, Math.min(doc.lines, hunk.newEndLine)));

  if (hunk.oldLines.length === 0) {
    // Pure addition: delete added lines cleanly including one adjacent newline.
    let from = startLn.from;
    let to = endLn.to;
    if (to < doc.length) to += 1;
    else if (from > 0) from -= 1;
    view.dispatch({ changes: { from, to, insert: '' }, selection: EditorSelection.cursor(from), userEvent: 'undo' });
    return true;
  }

  view.dispatch({
    changes: { from: startLn.from, to: endLn.to, insert: oldText },
    selection: EditorSelection.cursor(startLn.from),
    userEvent: 'undo',
  });
  return true;
}

class InlineBlameWidget extends WidgetType {
  readonly text: string;
  constructor(text: string) { super(); this.text = text; }
  eq(o: InlineBlameWidget): boolean { return this.text === o.text; }
  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-ide-inline-blame';
    span.setAttribute('aria-hidden', 'true');
    span.textContent = `    ${this.text}`;
    return span;
  }
}

const ideDiffGutterTheme = EditorView.baseTheme({
  '.cm-ide-diff-gutter': { width: '6px', userSelect: 'none', cursor: 'pointer' },
  '.cm-ide-diff-gutter .cm-gutterElement': {
    padding: '0',
    display: 'flex',
    alignItems: 'stretch',
    justifyContent: 'center',
  },
  '.cm-ide-diff-bar': {
    display: 'block',
    inlineSize: '3px',
    blockSize: '100%',
    minBlockSize: '1.2em',
    borderRadius: '1px',
    transition: 'inline-size var(--dur-fast) var(--ease-out)',
  },
  '.cm-ide-diff-gutter .cm-gutterElement:hover .cm-ide-diff-bar': {
    inlineSize: '5px',
  },
  '.cm-ide-diff-add': { backgroundColor: 'var(--success)' },
  '.cm-ide-diff-mod': { backgroundColor: 'var(--info)' },
  '.cm-ide-diff-del': {
    backgroundColor: 'var(--warning)',
    blockSize: '6px',
    minBlockSize: '6px',
    alignSelf: 'flex-end',
  },
  '.cm-ide-inline-blame': {
    color: 'var(--text-dim)',
    fontFamily: 'var(--font-mono)',
    fontSize: 'var(--fs-xs)',
    opacity: '0.72',
    userSelect: 'none',
    pointerEvents: 'none',
    whiteSpace: 'pre',
  },
});

export interface CreateIdeGitAndAgentOptions {
  savedContent?: string | null;
  blameByLine?: Record<number, string> | null;
  wordAtCursor: (state: EditorState) => { word: string; line: number; column: number };
  getCallbacks: () => IdeAgentMenuCallbacks;
}

export function createIdeGitAndAgentExtensions(opts: CreateIdeGitAndAgentOptions): Extension {
  const diffField = StateField.define<{ savedLines: string[] | null; markers: RangeSet<GutterMarker>; hunks: IdeDiffHunk[] }>({
    create(st) {
      const savedLines = parseSavedLines(opts.savedContent);
      return { savedLines, ...buildDiffState(st.doc, savedLines) };
    },
    update(cur, tr) {
      let savedLines = cur.savedLines;
      let baselineChanged = false;
      for (const ef of tr.effects) {
        if (ef.is(setSavedContentEffect)) {
          savedLines = parseSavedLines(ef.value);
          baselineChanged = true;
        }
      }
      if (!baselineChanged && !tr.docChanged) return cur;
      return { savedLines, ...buildDiffState(tr.state.doc, savedLines) };
    },
  });

  const blameField = StateField.define<{ byLine: Record<number, string> | null; decos: DecorationSet }>({
    create(st) {
      const byLine = opts.blameByLine ?? null;
      return { byLine, decos: computeBlameDecos(st, byLine) };
    },
    update(cur, tr) {
      let byLine = cur.byLine;
      let changed = false;
      for (const ef of tr.effects) {
        if (ef.is(setBlameLinesEffect)) { byLine = ef.value; changed = true; }
      }
      if (!byLine) return cur.byLine ? { byLine: null, decos: Decoration.none } : cur;
      if (changed || tr.docChanged || tr.selection) {
        return { byLine, decos: computeBlameDecos(tr.state, byLine) };
      }
      return cur;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  function computeBlameDecos(st: EditorState, byLine: Record<number, string> | null): DecorationSet {
    if (!byLine) return Decoration.none;
    const ln = st.doc.lineAt(st.selection.main.head);
    const label = byLine[ln.number];
    if (!label) return Decoration.none;
    return Decoration.set([Decoration.widget({ widget: new InlineBlameWidget(label), side: 2 }).range(ln.to)]);
  }

  const showHunkMenu = (view: EditorView, lineFrom: number, event: MouseEvent): boolean => {
    const ln = view.state.doc.lineAt(lineFrom).number;
    const { hunks } = view.state.field(diffField);
    const hunk = hunks.find((h) => ln >= h.markerStartLine && ln <= h.markerEndLine);
    if (!hunk) return false;
    event.preventDefault();
    event.stopPropagation();

    const title = hunk.kind === 'add'
      ? `Added ${hunk.newLines.length} line(s) at L${hunk.newStartLine}`
      : hunk.kind === 'del'
        ? `Deleted ${hunk.oldLines.length} line(s) near L${hunk.markerStartLine}`
        : `Modified L${hunk.newStartLine}–${hunk.newEndLine}`;

    const items: MenuItem[] = [{ label: title, disabled: true }];
    for (const oldLn of hunk.oldLines.slice(0, 3)) {
      const trimmed = oldLn.trim();
      items.push({ label: `- ${trimmed.length > 48 ? `${trimmed.slice(0, 45)}…` : (trimmed || '(blank)')}`, disabled: true });
    }
    for (const newLn of hunk.newLines.slice(0, 3)) {
      const trimmed = newLn.trim();
      items.push({ label: `+ ${trimmed.length > 48 ? `${trimmed.slice(0, 45)}…` : (trimmed || '(blank)')}`, disabled: true });
    }
    items.push(
      { separator: true },
      {
        label: 'Rollback hunk',
        icon: 'undo',
        disabled: view.state.readOnly,
        action: () => { rollbackDiffHunk(view, hunk); },
      },
    );
    if (hunk.oldLines.length > 0) {
      items.push({
        label: 'Copy original lines',
        icon: 'copy',
        action: () => void copyWithToast(hunk.oldLines.join('\n'), 'original hunk'),
      });
    }
    const cbs = opts.getCallbacks();
    if (cbs.onToggleBlame) {
      items.push({ separator: true }, { label: 'Toggle Git blame annotations', icon: 'commit', action: () => cbs.onToggleBlame?.() });
    }
    ctxMenu.show(event, items);
    return true;
  };

  const diffGutter = gutter({
    class: 'cm-ide-diff-gutter',
    markers: (view) => view.state.field(diffField).markers,
    domEventHandlers: {
      mousedown(view, line, event) {
        const me = event as MouseEvent;
        if (me.button !== 0) return false;
        return showHunkMenu(view, line.from, me);
      },
      contextmenu(view, line, event) {
        return showHunkMenu(view, line.from, event as MouseEvent);
      },
    },
  });

  const runAgent = (view: EditorView, mode: 'chat' | 'inline'): boolean => {
    const cb = opts.getCallbacks().onAgentAction;
    if (!cb) return false;
    cb(mode, selectionOrCurrentLine(view.state));
    return true;
  };

  const agentHandlers = Prec.high(EditorView.domEventHandlers({
    keydown(e, view) {
      if ((e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
        if (e.code === 'KeyL' || e.key === 'l' || e.key === 'L') {
          if (runAgent(view, 'chat')) { e.preventDefault(); e.stopPropagation(); return true; }
        } else if (e.code === 'KeyI' || e.key === 'i' || e.key === 'I') {
          if (runAgent(view, 'inline')) { e.preventDefault(); e.stopPropagation(); return true; }
        }
      }
      return false;
    },
    contextmenu(event, view) {
      if (event.target instanceof Element && event.target.closest('.cm-gutters')) return false;
      const cbs = opts.getCallbacks();
      if (!cbs.onAgentAction && !cbs.onDefinitionJump && !cbs.onQuickFix && !cbs.onRenameSymbol && !cbs.onFormat) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      const main = view.state.selection.main;
      if (pos != null && (main.empty || pos < main.from || pos > main.to)) {
        view.dispatch({ selection: { anchor: pos, head: pos } });
      }
      const items: MenuItem[] = [];
      if (cbs.onAgentAction) {
        items.push(
          { label: 'Add Selection to Agent (⌘L)', icon: 'assistant', action: () => cbs.onAgentAction?.('chat', selectionOrCurrentLine(view.state)) },
          { label: 'Inline Edit with Agent (⌘I)', icon: 'sparkle', disabled: view.state.readOnly, action: () => cbs.onAgentAction?.('inline', selectionOrCurrentLine(view.state)) },
        );
      }
      const sym = opts.wordAtCursor(view.state);
      const navItems: MenuItem[] = [];
      if (cbs.onDefinitionJump) navItems.push({ label: 'Go to Definition', icon: 'arrow', hint: '⌘B', action: () => cbs.onDefinitionJump?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
      if (cbs.onImplementationJump) navItems.push({ label: 'Go to Implementation', icon: 'layers', hint: '⌥⌘B', action: () => cbs.onImplementationJump?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
      if (cbs.onFindUsages) navItems.push({ label: 'Find Usages', icon: 'search', hint: '⌥F7', action: () => cbs.onFindUsages?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
      if (cbs.onFileStructure) navItems.push({ label: 'File Structure…', icon: 'book', hint: '⌘F12', action: () => cbs.onFileStructure?.() });
      if (cbs.onQuickFix) navItems.push({ label: 'Quick Fixes…', icon: 'bulb', hint: '⌥⏎', action: () => cbs.onQuickFix?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
      if (cbs.onRenameSymbol && sym.word) navItems.push({ label: 'Rename Symbol…', icon: 'edit', hint: '⇧F6', disabled: view.state.readOnly, action: () => cbs.onRenameSymbol?.({ line: sym.line, column: sym.column, symbol: sym.word }) });
      if (cbs.onFormat) navItems.push({ label: 'Reformat Code', icon: 'format', hint: '⌥⌘L', disabled: view.state.readOnly, action: () => cbs.onFormat?.() });
      if (cbs.onToggleBlame) navItems.push({ label: 'Toggle Git Blame', icon: 'commit', action: () => cbs.onToggleBlame?.() });
      if (items.length > 0 && navItems.length > 0) items.push({ separator: true });
      items.push(...navItems);
      if (items.length === 0) return false;
      ctxMenu.show(event, items);
      return true;
    },
  }));

  const agentKeymap = Prec.high(keymap.of([
    { key: 'Mod-l', run: (v) => runAgent(v, 'chat') },
    { key: 'Mod-i', run: (v) => runAgent(v, 'inline') },
  ]));

  return [ideDiffGutterTheme, diffField, blameField, diffGutter, agentHandlers, agentKeymap];
}
