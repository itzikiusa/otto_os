// IDE Git/Modified Gutter Indicators (`cm-ide-diff-gutter`) + AI Agent Editor Hooks
// (`⌘L` Add to Agent, `⌘I` Inline Edit, and right-click editor context menu).

import { EditorView, GutterMarker, gutter, keymap } from '@codemirror/view';
import {
  EditorState, Prec, RangeSet, StateEffect, StateField,
  type Extension, type Range,
} from '@codemirror/state';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte';

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
  onQuickFix?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string }) => void) | null;
  onFormat?: (() => void) | null;
}

export const setSavedContentEffect = StateEffect.define<string | null>();

const MAX_DIFF_GUTTER_LINES = 3_000;
const MAX_LCS_CELLS = 36_000;

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
  constructor(readonly kind: 'add' | 'mod') { super(); }
  eq(o: DiffGutterMarker): boolean { return this.kind === o.kind; }
  toDOM(): HTMLElement {
    const bar = document.createElement('span');
    bar.className = `cm-ide-diff-bar cm-ide-diff-${this.kind}`;
    bar.dataset.diffKind = this.kind;
    bar.setAttribute('aria-hidden', 'true');
    return bar;
  }
}

const addLineMarker = new DiffGutterMarker('add');
const modLineMarker = new DiffGutterMarker('mod');

function parseSavedLines(savedContent: string | null | undefined): string[] | null {
  if (savedContent == null) return null;
  const lines = savedContent.split(/\r\n?|\n/);
  return lines.length <= MAX_DIFF_GUTTER_LINES ? lines : null;
}

function classifyMiddleHunk(oldSlice: string[], newSlice: string[]): Map<number, 'add' | 'mod'> {
  const out = new Map<number, 'add' | 'mod'>();
  const m = oldSlice.length;
  const n = newSlice.length;
  if (m * n > MAX_LCS_CELLS) {
    const shared = Math.min(m, n);
    for (let j = 0; j < shared; j++) if (oldSlice[j] !== newSlice[j]) out.set(j, 'mod');
    for (let j = shared; j < n; j++) out.set(j, 'add');
    return out;
  }
  const cols = n + 1;
  const dp = new Uint16Array((m + 1) * cols);
  for (let i = m - 1; i >= 0; i--) {
    const oTxt = oldSlice[i];
    for (let j = n - 1; j >= 0; j--) {
      dp[i * cols + j] = oTxt === newSlice[j]
        ? dp[(i + 1) * cols + (j + 1)] + 1
        : Math.max(dp[(i + 1) * cols + j], dp[i * cols + (j + 1)]);
    }
  }
  const anchors: Array<[number, number]> = [[0, 0]];
  let i = 0;
  let j = 0;
  while (i < m && j < n) {
    if (oldSlice[i] === newSlice[j]) { anchors.push([i, j], [i + 1, j + 1]); i++; j++; }
    else if (dp[(i + 1) * cols + j] >= dp[i * cols + (j + 1)]) i++;
    else j++;
  }
  anchors.push([m, n]);
  for (let k = 0; k < anchors.length - 1; k += 2) {
    const [oStart, nStart] = anchors[k];
    const [oEnd, nEnd] = anchors[k + 1];
    const delCount = oEnd - oStart;
    const insCount = nEnd - nStart;
    const modCount = Math.min(delCount, insCount);
    for (let idx = 0; idx < modCount; idx++) out.set(nStart + idx, 'mod');
    for (let idx = modCount; idx < insCount; idx++) out.set(nStart + idx, 'add');
  }
  return out;
}

function computeDiffMarkers(doc: EditorState['doc'], savedLines: string[] | null): RangeSet<GutterMarker> {
  const newLen = doc.lines;
  if (!savedLines || newLen === 0 || newLen > MAX_DIFF_GUTTER_LINES) return RangeSet.empty;
  const oldLen = savedLines.length;
  let prefix = 0;
  while (prefix < oldLen && prefix < newLen && doc.line(prefix + 1).text === savedLines[prefix]) prefix++;
  if (prefix === oldLen && prefix === newLen) return RangeSet.empty;

  let suffix = 0;
  while (
    suffix < oldLen - prefix &&
    suffix < newLen - prefix &&
    doc.line(newLen - suffix).text === savedLines[oldLen - 1 - suffix]
  ) suffix++;

  const oldMid = oldLen - prefix - suffix;
  const newMid = newLen - prefix - suffix;
  const ranges: Range<GutterMarker>[] = [];

  if (newMid === 0) {
    const targetLn = Math.min(newLen, Math.max(1, prefix));
    ranges.push(modLineMarker.range(doc.line(targetLn).from));
    return RangeSet.of(ranges, true);
  }
  if (oldMid === 0) {
    for (let k = 0; k < newMid; k++) ranges.push(addLineMarker.range(doc.line(prefix + 1 + k).from));
    return RangeSet.of(ranges, true);
  }

  const oldSlice = savedLines.slice(prefix, prefix + oldMid);
  const newSlice: string[] = new Array(newMid);
  for (let k = 0; k < newMid; k++) newSlice[k] = doc.line(prefix + 1 + k).text;
  const kinds = classifyMiddleHunk(oldSlice, newSlice);
  for (let k = 0; k < newMid; k++) {
    const kind = kinds.get(k);
    if (kind) ranges.push((kind === 'add' ? addLineMarker : modLineMarker).range(doc.line(prefix + 1 + k).from));
  }
  return ranges.length > 0 ? RangeSet.of(ranges, true) : RangeSet.empty;
}

const ideDiffGutterTheme = EditorView.baseTheme({
  '.cm-ide-diff-gutter': { width: '4px', userSelect: 'none' },
  '.cm-ide-diff-gutter .cm-gutterElement': {
    padding: '0',
    display: 'flex',
    alignItems: 'stretch',
    justifyContent: 'center',
  },
  '.cm-ide-diff-bar': {
    display: 'block',
    inlineSize: '2px',
    blockSize: '100%',
    minBlockSize: '1.2em',
    borderRadius: '1px',
  },
  '.cm-ide-diff-add': { backgroundColor: 'var(--success)' },
  '.cm-ide-diff-mod': { backgroundColor: 'var(--info)' },
});

export interface CreateIdeGitAndAgentOptions {
  savedContent?: string | null;
  wordAtCursor: (state: EditorState) => { word: string; line: number; column: number };
  getCallbacks: () => IdeAgentMenuCallbacks;
}

export function createIdeGitAndAgentExtensions(opts: CreateIdeGitAndAgentOptions): Extension {
  const diffField = StateField.define<{ savedLines: string[] | null; markers: RangeSet<GutterMarker> }>({
    create(st) {
      const savedLines = parseSavedLines(opts.savedContent);
      return { savedLines, markers: computeDiffMarkers(st.doc, savedLines) };
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
      return { savedLines, markers: computeDiffMarkers(tr.state.doc, savedLines) };
    },
  });

  const diffGutter = gutter({
    class: 'cm-ide-diff-gutter',
    markers: (view) => view.state.field(diffField).markers,
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
      if (cbs.onQuickFix) navItems.push({ label: 'Quick Fixes…', icon: 'bulb', hint: '⌥⏎', action: () => cbs.onQuickFix?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
      if (cbs.onRenameSymbol && sym.word) navItems.push({ label: 'Rename Symbol…', icon: 'edit', hint: '⇧F6', disabled: view.state.readOnly, action: () => cbs.onRenameSymbol?.({ line: sym.line, column: sym.column, symbol: sym.word }) });
      if (cbs.onFormat) navItems.push({ label: 'Reformat Code', icon: 'format', hint: '⌥⌘L', disabled: view.state.readOnly, action: () => cbs.onFormat?.() });
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

  return [ideDiffGutterTheme, diffField, diffGutter, agentHandlers, agentKeymap];
}
