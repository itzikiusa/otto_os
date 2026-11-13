// Pro IDE extensions for CodeEditor (JetBrains / GoLand parity):
// Multi-cursor, Option+drag column selection, simultaneous line prefix/suffix,
// breakpoint & test-runner gutters, paused-line + inline ghost debug values,
// collaborative peer cursors, JetBrains/Standard keymaps, and direct WS LSP fallback.

import {
  Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType,
  crosshairCursor, gutter, hoverTooltip, keymap, rectangularSelection,
  type DecorationSet, type KeyBinding, type ViewUpdate,
} from '@codemirror/view';
import {
  EditorSelection, EditorState, Prec, RangeSet, StateEffect, StateField,
  type ChangeSpec, type Extension, type Range, type SelectionRange,
} from '@codemirror/state';
import {
  addCursorAbove, addCursorBelow, copyLineDown, deleteLine,
  moveLineDown, moveLineUp, toggleComment,
} from '@codemirror/commands';
import { selectNextOccurrence, selectSelectionMatches } from '@codemirror/search';
import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
import { setDiagnostics, type Diagnostic } from '@codemirror/lint';
import type {
  IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem, IdeInlineValue, IdePeerCursor,
} from '../api/types';
import { ctxMenu } from '../contextmenu.svelte';
import { isIconName } from './Icon.svelte';

export { addCursorAbove, addCursorBelow, crosshairCursor, moveLineUp, moveLineDown, rectangularSelection, selectNextOccurrence };
export const toggleLineComment = toggleComment;

// ── Identifier & Multi-Cursor Line Editing Commands ──────────────────────────

const IDENT_CHAR_RE = /[\w$]/;

export function wordAtCursor(state: EditorState): { word: string; line: number; column: number } {
  const main = state.selection.main;
  const ln = state.doc.lineAt(main.head);
  const column = main.head - ln.from + 1;
  if (!main.empty) return { word: state.sliceDoc(main.from, main.to).trim(), line: ln.number, column };
  const text = ln.text;
  const rel = main.head - ln.from;
  let s = rel;
  let e = rel;
  while (s > 0 && IDENT_CHAR_RE.test(text[s - 1])) s--;
  while (e < text.length && IDENT_CHAR_RE.test(text[e])) e++;
  return { word: text.slice(s, e), line: ln.number, column };
}

export function selectAllOccurrences(view: EditorView): boolean {
  const { state } = view;
  if (state.selection.main.empty) {
    const main = state.selection.main;
    const ln = state.doc.lineAt(main.head);
    const rel = main.head - ln.from;
    let s = rel;
    let e = rel;
    while (s > 0 && IDENT_CHAR_RE.test(ln.text[s - 1])) s--;
    while (e < ln.text.length && IDENT_CHAR_RE.test(ln.text[e])) e++;
    if (e > s) view.dispatch({ selection: { anchor: ln.from + s, head: ln.from + e } });
  }
  return selectSelectionMatches(view);
}

/** Split every multi-line selection into one selection/cursor per touched line (`⌥⇧I`). */
export function splitSelectionIntoLines(view: EditorView): boolean {
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
  if (!changed || next.length === 0) return false;
  view.dispatch({ selection: EditorSelection.create(next) });
  return true;
}

/**
 * Add `prefix` before and `suffix` after every selected / cursor line in a
 * single undoable transaction, preserving multi-cursors across all modified lines.
 */
export function applyLinePrefixSuffix(view: EditorView, prefix: string, suffix: string): boolean {
  if ((!prefix && !suffix) || view.state.readOnly) return false;
  const { doc, selection } = view.state;
  const touched = new Set<number>();
  for (const r of selection.ranges) {
    const startLn = doc.lineAt(r.from).number;
    let endLn = doc.lineAt(r.to).number;
    if (!r.empty && r.to > r.from && doc.lineAt(r.to).from === r.to) {
      endLn = doc.lineAt(r.to - 1).number;
    }
    for (let ln = startLn; ln <= endLn; ln++) touched.add(ln);
  }
  const lines = [...touched].sort((a, b) => a - b);
  if (lines.length === 0) return false;

  const changes: ChangeSpec[] = [];
  const cursors: SelectionRange[] = [];
  const delta = prefix.length + suffix.length;
  lines.forEach((ln, idx) => {
    const line = doc.line(ln);
    if (prefix.length > 0) changes.push({ from: line.from, to: line.from, insert: prefix });
    if (suffix.length > 0) changes.push({ from: line.to, to: line.to, insert: suffix });
    cursors.push(EditorSelection.cursor(line.to + (idx + 1) * delta));
  });

  view.dispatch({
    changes,
    selection: EditorSelection.create(cursors, cursors.length - 1),
    userEvent: 'input',
    scrollIntoView: true,
  });
  return true;
}

/** Duplicate non-empty selection(s) inline, or duplicate current line(s) down (`⌘D`). */
export function duplicateLineOrSelection(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  if (!view.state.selection.ranges.some((r) => !r.empty)) return copyLineDown(view);
  const tr = view.state.changeByRange((range) => {
    if (range.empty) {
      const ln = view.state.doc.lineAt(range.head);
      const ins = `\n${ln.text}`;
      return { changes: { from: ln.to, to: ln.to, insert: ins }, range: EditorSelection.cursor(range.head + ins.length) };
    }
    const sel = view.state.sliceDoc(range.from, range.to);
    return { changes: { from: range.to, to: range.to, insert: sel }, range: EditorSelection.range(range.to, range.to + sel.length) };
  });
  view.dispatch(view.state.update(tr, { userEvent: 'input', scrollIntoView: true }));
  return true;
}

// ── State Effects & Callbacks ────────────────────────────────────────────────

export interface IdeEditorCallbacks {
  onToggleBreakpoint?: ((line: number) => void) | null;
  onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
  onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug') => void) | null;
  onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string }) => void) | null;
  onQuickFix?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string }) => void) | null;
  onFormat?: (() => void) | null;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
}

export interface IdeDebugStatePayload {
  pausedLine: number | null;
  inlineValues: IdeInlineValue[];
}

export type PeerCursorInput = IdePeerCursor | IdeCollabPeer;

export const setBreakpointsEffect = StateEffect.define<IdeBreakpoint[]>();
export const setGutterItemsEffect = StateEffect.define<IdeGutterItem[]>();
export const setDebugDecorationsEffect = StateEffect.define<IdeDebugStatePayload>();
export const setPeerCursorsEffect = StateEffect.define<PeerCursorInput[]>();

// ── Gutter Markers & Decorations ─────────────────────────────────────────────

const SVG_NS = 'http://www.w3.org/2000/svg';
function gutterSvg(shape: 'enabled' | 'disabled' | 'diamond' | 'play'): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  for (const [k, v] of [['width', '12'], ['height', '12'], ['viewBox', '0 0 16 16'], ['aria-hidden', 'true']]) svg.setAttribute(k, v);
  if (shape === 'enabled' || shape === 'disabled') {
    const c = document.createElementNS(SVG_NS, 'circle');
    for (const [k, v] of [['cx', '8'], ['cy', '8'], ['r', shape === 'disabled' ? '4.5' : '5'], ['fill', shape === 'disabled' ? 'none' : 'currentColor']]) c.setAttribute(k, v);
    if (shape === 'disabled') { c.setAttribute('stroke', 'currentColor'); c.setAttribute('stroke-width', '1.5'); }
    svg.appendChild(c);
  } else {
    const p = document.createElementNS(SVG_NS, 'path');
    p.setAttribute('d', shape === 'diamond' ? 'M8 2.5L13.5 8L8 13.5L2.5 8Z' : 'M5 3.5l7 4.5-7 4.5v-9Z');
    p.setAttribute('fill', 'currentColor');
    svg.appendChild(p);
  }
  return svg;
}

class BreakpointMarker extends GutterMarker {
  constructor(
    readonly kind: 'enabled' | 'disabled' | 'conditional' | 'logpoint' | 'paused',
    readonly tooltip: string,
    readonly alsoPaused: boolean,
  ) { super(); }
  eq(o: BreakpointMarker): boolean {
    return this.kind === o.kind && this.tooltip === o.tooltip && this.alsoPaused === o.alsoPaused;
  }
  toDOM(): HTMLElement {
    const el = document.createElement('span');
    el.className = `cm-ide-bp-marker cm-ide-bp-${this.kind}${this.alsoPaused ? ' is-paused' : ''}`;
    el.dataset.bpKind = this.kind;
    el.title = this.tooltip;
    el.setAttribute('role', 'img');
    el.setAttribute('aria-label', this.tooltip);
    el.appendChild(gutterSvg(this.kind === 'paused' ? 'play' : (this.kind === 'conditional' || this.kind === 'logpoint') ? 'diamond' : this.kind));
    return el;
  }
}

const bpSpacer = new (class extends GutterMarker {
  toDOM(): HTMLElement {
    const s = document.createElement('span');
    s.className = 'cm-ide-bp-spacer';
    s.setAttribute('aria-hidden', 'true');
    s.appendChild(gutterSvg('enabled'));
    return s;
  }
})();

class GutterRunMarker extends GutterMarker {
  constructor(readonly item: IdeGutterItem, readonly getCallbacks: () => IdeEditorCallbacks) { super(); }
  eq(o: GutterRunMarker): boolean {
    return this.item.line === o.item.line && this.item.kind === o.item.kind && this.item.label === o.item.label;
  }
  toDOM(): HTMLElement {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = `cm-ide-gutter-run-btn cm-ide-run-btn is-${this.item.kind}`;
    btn.dataset.gutterRun = String(this.item.line);
    btn.dataset.gutterLine = String(this.item.line);
    btn.dataset.gutterKind = this.item.kind;
    const label = `Run or debug ${this.item.label}`;
    btn.title = label;
    btn.setAttribute('aria-label', label);
    btn.appendChild(gutterSvg('play'));
    btn.addEventListener('click', (e) => {
      e.preventDefault();
      e.stopPropagation();
      const cbs = this.getCallbacks();
      ctxMenu.showAt(btn, [
        { label: `Run '${this.item.label}'`, icon: 'play', action: () => cbs.onGutterAction?.(this.item, 'run') },
        { label: `Debug '${this.item.label}'`, icon: isIconName('bug') ? 'bug' : 'target', action: () => cbs.onGutterAction?.(this.item, 'debug') },
      ]);
    });
    return btn;
  }
}

const runSpacer = new (class extends GutterMarker {
  toDOM(): HTMLElement {
    const s = document.createElement('span');
    s.className = 'cm-ide-run-spacer';
    s.setAttribute('aria-hidden', 'true');
    s.appendChild(gutterSvg('play'));
    return s;
  }
})();

class InlineGhostValuesWidget extends WidgetType {
  constructor(readonly summary: string) { super(); }
  eq(o: InlineGhostValuesWidget): boolean { return this.summary === o.summary; }
  toDOM(): HTMLElement {
    const span = document.createElement('span');
    span.className = 'cm-ide-inline-values';
    span.setAttribute('aria-hidden', 'true');
    span.textContent = `  // ${this.summary}`;
    return span;
  }
}

function computeDebugDecorations(doc: EditorState['doc'], payload: IdeDebugStatePayload): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  if (payload.pausedLine != null && payload.pausedLine >= 1 && payload.pausedLine <= doc.lines) {
    ranges.push(Decoration.line({ class: 'cm-ide-paused-line' }).range(doc.line(payload.pausedLine).from));
  }
  if (payload.inlineValues.length > 0) {
    const byLine = new Map<number, Map<string, string>>();
    for (const iv of payload.inlineValues) {
      if (iv.line < 1 || iv.line > doc.lines) continue;
      let m = byLine.get(iv.line);
      if (!m) { m = new Map(); byLine.set(iv.line, m); }
      m.set(iv.name, iv.value.length > 72 ? `${iv.value.slice(0, 69)}…` : iv.value);
    }
    for (const [lineNo, vars] of byLine) {
      const summary = [...vars.entries()].map(([k, v]) => `${k} = ${v}`).join(', ');
      if (summary) ranges.push(Decoration.widget({ widget: new InlineGhostValuesWidget(summary), side: 1 }).range(doc.line(lineNo).to));
    }
  }
  return ranges.length > 0 ? Decoration.set(ranges, true) : Decoration.none;
}

function peerSlot(peer: PeerCursorInput): number {
  const seed = `${peer.color ?? ''}:${peer.peer_id}`;
  let h = 0;
  for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) | 0;
  return Math.abs(h) % 6;
}

type PeerCaretMode = 'above' | 'below' | 'caret' | 'eol';

class PeerCaretWidget extends WidgetType {
  constructor(readonly name: string, readonly slot: number, readonly mode: PeerCaretMode) { super(); }
  eq(o: PeerCaretWidget): boolean { return this.name === o.name && this.slot === o.slot && this.mode === o.mode; }
  toDOM(): HTMLElement {
    const wrap = document.createElement('span');
    wrap.className = `${this.mode === 'eol' ? 'cm-ide-peer-eol' : 'cm-ide-peer-caret'} cm-ide-peer-slot-${this.slot}${this.mode === 'caret' ? ' has-cap' : ''}`;
    wrap.setAttribute('aria-hidden', 'true');
    if (this.mode !== 'caret') {
      const pill = document.createElement('span');
      pill.className = `cm-ide-peer-pill${this.mode === 'below' ? ' is-below' : this.mode === 'eol' ? ' is-eol' : ''}`;
      pill.textContent = this.name;
      wrap.appendChild(pill);
    }
    return wrap;
  }
}

function addPeerCursorWidgets(doc: EditorState['doc'], pos: number, name: string, slot: number, out: Range<Decoration>[]): void {
  const ln = doc.lineAt(pos);
  const col = pos - ln.from;
  const lineClearAt = (n: number) => {
    const l = doc.line(n);
    return l.text.trim().length === 0 || l.length < Math.max(0, col - 1);
  };
  if (ln.number > 1 && lineClearAt(ln.number - 1)) {
    out.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, 'above'), side: 1 }).range(pos));
  } else if (ln.number < doc.lines && lineClearAt(ln.number + 1)) {
    out.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, 'below'), side: 1 }).range(pos));
  } else if (ln.number === 1 && doc.lines === 1) {
    out.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, 'below'), side: 1 }).range(pos));
  } else {
    out.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, 'caret'), side: 1 }).range(pos));
    out.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, 'eol'), side: 2 }).range(ln.to));
  }
}

function computePeerDecorations(doc: EditorState['doc'], peers: PeerCursorInput[]): DecorationSet {
  if (peers.length === 0) return Decoration.none;
  const ranges: Range<Decoration>[] = [];
  const maxPos = doc.length;
  for (const peer of peers) {
    const slot = peerSlot(peer);
    const name = peer.display_name || ('name' in peer ? peer.name : undefined) || 'Peer';
    if (peer.selections && peer.selections.length > 0) {
      for (const s of peer.selections) {
        const a = Math.max(0, Math.min(maxPos, s.anchor));
        const b = Math.max(0, Math.min(maxPos, s.head));
        if (a !== b) ranges.push(Decoration.mark({ class: `cm-ide-peer-sel cm-ide-peer-slot-${slot}` }).range(Math.min(a, b), Math.max(a, b)));
        addPeerCursorWidgets(doc, b, name, slot, ranges);
      }
    } else if (peer.active_line != null && peer.active_line >= 1 && peer.active_line <= doc.lines) {
      addPeerCursorWidgets(doc, doc.line(peer.active_line).from, name, slot, ranges);
    }
  }
  return ranges.length > 0 ? Decoration.set(ranges, true) : Decoration.none;
}

// ── Theme (100% Otto Design Tokens — zero hex/px/z-index literals) ───────────

const ideEditorTheme = EditorView.baseTheme({
  '&': { '--ide-peer-color': 'var(--cat-1)' },
  '.cm-ide-peer-slot-0': { '--ide-peer-color': 'var(--cat-1)' },
  '.cm-ide-peer-slot-1': { '--ide-peer-color': 'var(--cat-2)' },
  '.cm-ide-peer-slot-2': { '--ide-peer-color': 'var(--cat-3)' },
  '.cm-ide-peer-slot-3': { '--ide-peer-color': 'var(--cat-4)' },
  '.cm-ide-peer-slot-4': { '--ide-peer-color': 'var(--cat-5)' },
  '.cm-ide-peer-slot-5': { '--ide-peer-color': 'var(--cat-6)' },
  '.cm-ide-bp-gutter': { width: '16px', cursor: 'pointer', userSelect: 'none' },
  '.cm-ide-bp-gutter .cm-gutterElement': { display: 'flex', alignItems: 'center', justifyContent: 'center', paddingInline: '2px' },
  '.cm-ide-bp-spacer, .cm-ide-run-spacer': { visibility: 'hidden', display: 'inline-flex' },
  '.cm-ide-bp-marker': { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', lineHeight: '1', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-disabled': { color: 'var(--text-dim)' },
  '.cm-ide-bp-marker.cm-ide-bp-logpoint, .cm-ide-bp-marker.cm-ide-bp-paused': { color: 'var(--warning)' },
  '.cm-ide-bp-marker.is-paused': { boxShadow: '0 0 0 1px var(--warning)', borderRadius: 'var(--radius-s)' },
  '.cm-ide-run-gutter': { width: '18px', userSelect: 'none' },
  '.cm-ide-run-gutter .cm-gutterElement': { display: 'flex', alignItems: 'center', justifyContent: 'center' },
  '.cm-ide-gutter-run-btn': {
    appearance: 'none', border: 'none', background: 'transparent', color: 'var(--success)',
    fontSize: 'var(--fs-xs)', lineHeight: '1', padding: '0', width: '16px', height: '16px',
    borderRadius: 'var(--radius-s)', cursor: 'pointer', display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
  },
  '.cm-ide-gutter-run-btn:hover': { background: 'var(--success-soft)' },
  '.cm-ide-gutter-run-btn:focus-visible': { outline: '1px solid var(--accent-text)' },
  '.cm-ide-paused-line': { backgroundColor: 'var(--warning-soft)', boxShadow: 'inset 2px 0 0 var(--warning)' },
  '.cm-ide-inline-values': {
    color: 'var(--text-dim)', fontStyle: 'italic', fontFamily: 'var(--font-mono)',
    fontSize: 'var(--fs-xs)', userSelect: 'none', pointerEvents: 'none', whiteSpace: 'pre',
  },
  '.cm-ide-peer-sel': { backgroundColor: 'color-mix(in srgb, var(--ide-peer-color) 22%, transparent)', borderRadius: '2px' },
  '.cm-ide-peer-caret, .cm-ide-peer-eol': {
    position: 'relative', display: 'inline-block', width: '0', height: '1.15em',
    verticalAlign: 'text-bottom', overflow: 'visible', pointerEvents: 'none', zIndex: 'var(--z-sticky)',
  },
  '.cm-ide-peer-caret::before': {
    content: '""', position: 'absolute', insetBlock: '0', insetInlineStart: '-1px',
    width: '2px', backgroundColor: 'var(--ide-peer-color)', borderRadius: '1px',
  },
  '.cm-ide-peer-caret.has-cap::after': {
    content: '""', position: 'absolute', insetBlockStart: '-2px', insetInlineStart: '-2px',
    width: '5px', height: '5px', backgroundColor: 'var(--ide-peer-color)', borderRadius: '1px',
  },
  '.cm-ide-peer-pill': {
    position: 'absolute', insetBlockEnd: '100%', insetInlineStart: '-1px', padding: '0 4px',
    borderRadius: 'var(--radius-s)', backgroundColor: 'var(--surface-2)', border: '1px solid var(--ide-peer-color)',
    color: 'var(--text)', fontFamily: 'var(--font-ui)', fontSize: 'var(--fs-xs)', fontWeight: '500',
    lineHeight: '1.25', whiteSpace: 'nowrap', pointerEvents: 'none', zIndex: 'var(--z-sticky)',
  },
  '.cm-ide-peer-pill.is-below': { insetBlockStart: '100%', insetBlockEnd: 'auto' },
  '.cm-ide-peer-pill.is-eol': { insetBlockStart: '0', insetBlockEnd: 'auto', insetInlineStart: 'var(--sp-3)' },
});

// ── Keymaps (JetBrains macOS & Standard Presets) ─────────────────────────────

export function buildIdeKeymap(preset: 'jetbrains' | 'standard', getCallbacks: () => IdeEditorCallbacks): Extension {
  const bindings: KeyBinding[] = [
    { key: 'Alt-Shift-ArrowUp', run: addCursorAbove },
    { key: 'Alt-Shift-ArrowDown', run: addCursorBelow },
    { key: 'Mod-Alt-ArrowUp', run: addCursorAbove },
    { key: 'Mod-Alt-ArrowDown', run: addCursorBelow },
    { key: 'Ctrl-g', run: selectNextOccurrence },
    { key: 'Ctrl-Mod-g', run: selectAllOccurrences },
    { key: 'Alt-Shift-i', run: splitSelectionIntoLines },
    { key: 'Alt-Shift-I', run: splitSelectionIntoLines },
    { key: 'Alt-ArrowUp', run: moveLineUp },
    { key: 'Alt-ArrowDown', run: moveLineDown },
    { key: 'Mod-/', run: toggleLineComment },
    { key: 'Mod-b', run: (v) => { const cb = getCallbacks().onDefinitionJump; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word || undefined }); return true; } },
    { key: 'Alt-Enter', run: (v) => { const cb = getCallbacks().onQuickFix; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word || undefined }); return true; } },
    { key: 'Shift-F6', run: (v) => { const cb = getCallbacks().onRenameSymbol; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word }); return true; } },
    { key: 'Mod-Alt-l', run: () => { const cb = getCallbacks().onFormat; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-F8', run: (v) => { const cb = getCallbacks().onToggleBreakpoint; if (!cb) return false; cb(v.state.doc.lineAt(v.state.selection.main.head).number); return true; } },
    ...(preset === 'jetbrains'
      ? [{ key: 'Mod-d', run: duplicateLineOrSelection }, { key: 'Mod-Backspace', run: deleteLine }]
      : [{ key: 'Mod-d', run: selectNextOccurrence }, { key: 'Shift-Mod-d', run: duplicateLineOrSelection }, { key: 'Shift-Mod-k', run: deleteLine }]),
  ];
  const altShiftIHandler = Prec.high(EditorView.domEventHandlers({
    keydown(e, view) {
      if (e.altKey && e.shiftKey && !e.metaKey && !e.ctrlKey && (e.code === 'KeyI' || e.key === 'I' || e.key === 'i')) {
        e.preventDefault();
        splitSelectionIntoLines(view);
        return true;
      }
      return false;
    },
  }));
  return [altShiftIHandler, Prec.high(keymap.of(bindings))];
}

// ── Full IDE Suite Builder ───────────────────────────────────────────────────

export interface CreateIdeExtensionsOptions {
  breakpoints: IdeBreakpoint[];
  gutterItems: IdeGutterItem[];
  pausedLine: number | null;
  inlineValues: IdeInlineValue[];
  peerCursors: PeerCursorInput[];
  keymapPreset: 'jetbrains' | 'standard';
  getCallbacks: () => IdeEditorCallbacks;
}

export function createIdeExtensions(opts: CreateIdeExtensionsOptions): Extension {
  const bpField = StateField.define<{ map: Map<number, IdeBreakpoint>; pausedLine: number | null }>({
    create() {
      const map = new Map<number, IdeBreakpoint>();
      for (const bp of opts.breakpoints) map.set(bp.line, bp);
      return { map, pausedLine: opts.pausedLine };
    },
    update(val, tr) {
      let { map, pausedLine } = val;
      for (const ef of tr.effects) {
        if (ef.is(setBreakpointsEffect)) {
          map = new Map();
          for (const bp of ef.value) map.set(bp.line, bp);
        } else if (ef.is(setDebugDecorationsEffect)) pausedLine = ef.value.pausedLine;
      }
      return map === val.map && pausedLine === val.pausedLine ? val : { map, pausedLine };
    },
  });

  const runField = StateField.define<Map<number, IdeGutterItem>>({
    create() {
      const map = new Map<number, IdeGutterItem>();
      for (const it of opts.gutterItems) map.set(it.line, it);
      return map;
    },
    update(map, tr) {
      for (const ef of tr.effects) {
        if (ef.is(setGutterItemsEffect)) {
          const next = new Map<number, IdeGutterItem>();
          for (const it of ef.value) next.set(it.line, it);
          return next;
        }
      }
      return map;
    },
  });

  const debugField = StateField.define<{ payload: IdeDebugStatePayload; decos: DecorationSet }>({
    create(st) {
      const payload = { pausedLine: opts.pausedLine, inlineValues: opts.inlineValues };
      return { payload, decos: computeDebugDecorations(st.doc, payload) };
    },
    update(cur, tr) {
      let payload = cur.payload;
      let changed = false;
      for (const ef of tr.effects) if (ef.is(setDebugDecorationsEffect)) { payload = ef.value; changed = true; }
      return changed || tr.docChanged ? { payload, decos: computeDebugDecorations(tr.state.doc, payload) } : cur;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  const peersField = StateField.define<{ peers: PeerCursorInput[]; decos: DecorationSet }>({
    create(st) {
      return { peers: opts.peerCursors, decos: computePeerDecorations(st.doc, opts.peerCursors) };
    },
    update(cur, tr) {
      let peers = cur.peers;
      let changed = false;
      for (const ef of tr.effects) if (ef.is(setPeerCursorsEffect)) { peers = ef.value; changed = true; }
      return changed || tr.docChanged ? { peers, decos: computePeerDecorations(tr.state.doc, peers) } : cur;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  const bpGutter = gutter({
    class: 'cm-ide-bp-gutter',
    markers(view) {
      const { map, pausedLine } = view.state.field(bpField);
      if (map.size === 0 && pausedLine == null) return RangeSet.empty;
      const markers: Range<GutterMarker>[] = [];
      const lines = new Set<number>(map.keys());
      if (pausedLine != null) lines.add(pausedLine);
      for (const ln of [...lines].sort((a, b) => a - b)) {
        if (ln < 1 || ln > view.state.doc.lines) continue;
        const pos = view.state.doc.line(ln).from;
        const bp = map.get(ln);
        const isPaused = pausedLine === ln;
        if (bp) {
          const kind = !bp.enabled ? 'disabled' : bp.log_message ? 'logpoint' : (bp.condition || bp.hit_condition) ? 'conditional' : 'enabled';
          const tip = bp.log_message ? `Logpoint at line ${ln}: ${bp.log_message}` : bp.condition ? `Conditional breakpoint at line ${ln}: ${bp.condition}` : !bp.enabled ? `Disabled breakpoint at line ${ln}` : `Breakpoint at line ${ln}`;
          markers.push(new BreakpointMarker(kind, tip, isPaused).range(pos));
        } else if (isPaused) {
          markers.push(new BreakpointMarker('paused', `Paused at line ${ln}`, false).range(pos));
        }
      }
      return RangeSet.of(markers, true);
    },
    initialSpacer: () => bpSpacer,
    domEventHandlers: {
      mousedown(view, line, event) {
        const me = event as MouseEvent;
        if (me.button !== 0) return false;
        const ln = view.state.doc.lineAt(line.from).number;
        const cbs = opts.getCallbacks();
        if (me.shiftKey && cbs.onEditBreakpoint) { me.preventDefault(); cbs.onEditBreakpoint(ln, me); return true; }
        if (cbs.onToggleBreakpoint) { me.preventDefault(); cbs.onToggleBreakpoint(ln); return true; }
        return false;
      },
      contextmenu(view, line, event) {
        const me = event as MouseEvent;
        const ln = view.state.doc.lineAt(line.from).number;
        const cbs = opts.getCallbacks();
        if (!cbs.onToggleBreakpoint && !cbs.onEditBreakpoint) return false;
        const bp = view.state.field(bpField).map.get(ln);
        ctxMenu.show(me, [
          { label: bp ? 'Remove breakpoint' : 'Add breakpoint', icon: bp ? 'trash' : 'plus', action: () => cbs.onToggleBreakpoint?.(ln) },
          ...(cbs.onEditBreakpoint ? [{ label: 'Edit condition / logpoint…', icon: 'edit', action: () => cbs.onEditBreakpoint?.(ln, me) }] : []),
        ]);
        return true;
      },
    },
  });

  const runGutter = gutter({
    class: 'cm-ide-run-gutter',
    markers(view) {
      const map = view.state.field(runField);
      if (map.size === 0) return RangeSet.empty;
      const markers: Range<GutterMarker>[] = [];
      for (const [ln, it] of [...map.entries()].sort((a, b) => a[0] - b[0])) {
        if (ln >= 1 && ln <= view.state.doc.lines) markers.push(new GutterRunMarker(it, opts.getCallbacks).range(view.state.doc.line(ln).from));
      }
      return RangeSet.of(markers, true);
    },
    initialSpacer: () => runSpacer,
  });

  return [ideEditorTheme, bpField, runField, debugField, peersField, bpGutter, runGutter, buildIdeKeymap(opts.keymapPreset, opts.getCallbacks)];
}

// ── Direct WebSocket JSON-RPC LSP Fallback (Zero Node Polyfills) ─────────────

export interface DirectLspOptions {
  wsUri: string;
  protocols?: string[];
  rootUri: string;
  documentUri: string;
  filePath: string;
  languageId: string;
  onLocationJump?: (uri: string, line: number, column: number) => void;
  onDiagnostics?: (diags: IdeDiagnostic[]) => void;
}

export function createDirectLspExtension(opts: DirectLspOptions): Extension {
  let socket: WebSocket | null = null;
  let nextId = 1;
  let version = 1;
  let ready = false;
  let closed = false;
  let liveView: EditorView | null = null;
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

  const send = (p: Record<string, unknown>) => {
    if (!closed && socket?.readyState === WebSocket.OPEN) socket.send(JSON.stringify(p));
  };
  const request = <T = unknown>(method: string, params: unknown): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      if (!ready || socket?.readyState !== WebSocket.OPEN) return reject(new Error('LSP unavailable'));
      const id = nextId++;
      pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      send({ jsonrpc: '2.0', id, method, params });
      setTimeout(() => { if (pending.delete(id)) reject(new Error('LSP timeout')); }, 6000);
    });

  const posToLsp = (doc: EditorState['doc'], pos: number) => {
    const ln = doc.lineAt(pos);
    return { line: ln.number - 1, character: pos - ln.from };
  };
  const lspToPos = (doc: EditorState['doc'], p: { line: number; character: number }) => {
    const ln = doc.line(Math.max(1, Math.min(doc.lines, p.line + 1)));
    return Math.min(ln.to, ln.from + Math.max(0, p.character));
  };

  const plugin = ViewPlugin.fromClass(class {
    constructor(view: EditorView) {
      liveView = view;
      try { socket = new WebSocket(opts.wsUri, opts.protocols); } catch { return; }
      socket.addEventListener('open', () => {
        if (closed || !socket) return;
        const initId = nextId++;
        pending.set(initId, {
          resolve: () => {
            ready = true;
            send({ jsonrpc: '2.0', method: 'initialized', params: {} });
            send({ jsonrpc: '2.0', method: 'textDocument/didOpen', params: { textDocument: { uri: opts.documentUri, languageId: opts.languageId, version, text: view.state.doc.toString() } } });
          },
          reject: () => {},
        });
        send({
          jsonrpc: '2.0', id: initId, method: 'initialize',
          params: {
            processId: null, rootUri: opts.rootUri, workspaceFolders: [{ uri: opts.rootUri, name: 'workspace' }],
            capabilities: { textDocument: { hover: { contentFormat: ['plaintext', 'markdown'] }, completion: { completionItem: { snippetSupport: false } }, definition: { linkSupport: true }, publishDiagnostics: { relatedInformation: true } } },
          },
        });
      });
      socket.addEventListener('message', (ev) => {
        if (closed || typeof ev.data !== 'string') return;
        let msg: Record<string, unknown>;
        try { msg = JSON.parse(ev.data) as Record<string, unknown>; } catch { return; }
        if (typeof msg.id === 'number' && !msg.method) {
          const p = pending.get(msg.id);
          if (p) { pending.delete(msg.id); if (msg.error) p.reject(new Error('LSP error')); else p.resolve(msg.result); }
          return;
        }
        if (typeof msg.id === 'number' && typeof msg.method === 'string') {
          send({ jsonrpc: '2.0', id: msg.id, result: null });
          return;
        }
        if (msg.method === 'textDocument/publishDiagnostics' && liveView) {
          const params = msg.params as { uri?: string; diagnostics?: Array<{ range: { start: { line: number; character: number }; end: { line: number; character: number } }; severity?: number; message: string; source?: string; code?: string | number }> };
          if (params?.uri && params.uri !== opts.documentUri) return;
          const raw = params?.diagnostics ?? [];
          const doc = liveView.state.doc;
          const cmDiags: Diagnostic[] = raw.map((d) => ({
            from: lspToPos(doc, d.range.start),
            to: Math.max(lspToPos(doc, d.range.start), lspToPos(doc, d.range.end)),
            severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : 'info',
            message: d.message, source: d.source,
          }));
          liveView.dispatch(setDiagnostics(liveView.state, cmDiags));
          opts.onDiagnostics?.(raw.map((d) => ({
            path: opts.filePath,
            line: d.range.start.line + 1, column: d.range.start.character + 1,
            end_line: d.range.end.line + 1, end_column: d.range.end.character + 1,
            severity: d.severity === 1 ? 'error' : d.severity === 2 ? 'warning' : d.severity === 4 ? 'hint' : 'info',
            message: d.message, source: d.source ?? null, code: d.code != null ? String(d.code) : null,
          })));
        }
      });
    }
    update(u: ViewUpdate) {
      if (u.docChanged && ready) {
        version++;
        send({ jsonrpc: '2.0', method: 'textDocument/didChange', params: { textDocument: { uri: opts.documentUri, version }, contentChanges: [{ text: u.state.doc.toString() }] } });
      }
    }
    destroy() {
      closed = true;
      if (ready && socket?.readyState === WebSocket.OPEN) {
        try { socket.send(JSON.stringify({ jsonrpc: '2.0', method: 'textDocument/didClose', params: { textDocument: { uri: opts.documentUri } } })); } catch { /* ignore */ }
      }
      for (const [, p] of pending) p.reject(new Error('LSP closed'));
      pending.clear();
      socket?.close();
      socket = null;
      liveView = null;
    }
  });

  const hover = hoverTooltip(async (view, pos) => {
    if (!ready) return null;
    try {
      const res = await request<{ contents?: string | { value?: string } | Array<string | { value?: string }> } | null>('textDocument/hover', {
        textDocument: { uri: opts.documentUri }, position: posToLsp(view.state.doc, pos),
      });
      if (!res?.contents) return null;
      const parts = Array.isArray(res.contents) ? res.contents : [res.contents];
      const text = parts.map((p) => (typeof p === 'string' ? p : (p?.value ?? ''))).filter(Boolean).join('\n\n').trim();
      if (!text) return null;
      return {
        pos,
        create() {
          const dom = document.createElement('div');
          const maxH = typeof window !== 'undefined' ? Math.min(260, Math.max(120, window.innerHeight - 80)) : 260;
          dom.style.cssText = `padding:var(--sp-2) var(--sp-3);font-family:var(--font-mono);font-size:var(--fs-xs);max-width:min(480px, calc(100vw - 24px));max-height:${maxH}px;overflow:auto;white-space:pre-wrap`;
          dom.textContent = text;
          return { dom };
        },
      };
    } catch { return null; }
  });

  const completion = EditorState.languageData.of(() => [{
    autocomplete: async (ctx: CompletionContext): Promise<CompletionResult | null> => {
      if (!ready) return null;
      const word = ctx.matchBefore(/[\w$]+/);
      if (!word && !ctx.explicit) return null;
      try {
        const res = await request<Array<{ label: string; detail?: string; insertText?: string }> | { items?: Array<{ label: string; detail?: string; insertText?: string }> } | null>('textDocument/completion', {
          textDocument: { uri: opts.documentUri }, position: posToLsp(ctx.state.doc, ctx.pos),
        });
        const items = Array.isArray(res) ? res : (res?.items ?? []);
        if (!items.length) return null;
        const options: Completion[] = items.slice(0, 120).map((it) => ({ label: it.label, detail: it.detail, apply: it.insertText || it.label }));
        return { from: word ? word.from : ctx.pos, options };
      } catch { return null; }
    },
  }]);

  const cmdClickDef = EditorView.domEventHandlers({
    mousedown(event, view) {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.button !== 0 || !ready) return false;
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null) return false;
      type Loc = { uri?: string; targetUri?: string; range?: { start: { line: number; character: number } }; targetSelectionRange?: { start: { line: number; character: number } } };
      void request<Loc | Loc[] | null>('textDocument/definition', { textDocument: { uri: opts.documentUri }, position: posToLsp(view.state.doc, pos) })
        .then((res) => {
          const first = Array.isArray(res) ? res[0] : res;
          const uri = first?.targetUri ?? first?.uri;
          const start = first?.targetSelectionRange?.start ?? first?.range?.start;
          if (uri && start) opts.onLocationJump?.(uri, start.line + 1, start.character + 1);
        }).catch(() => {});
      return false;
    },
  });

  return [plugin, hover, completion, cmdClickDef];
}
