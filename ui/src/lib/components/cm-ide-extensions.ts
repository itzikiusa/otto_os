// Pro IDE extensions for CodeEditor (JetBrains / GoLand parity):
// Multi-cursor, Option+drag column selection, simultaneous line prefix/suffix,
// interactive hover-preview breakpoint & runnable gutters, paused-line + inline
// ghost debug values, peer cursors, CSV rainbow & Shell var highlights, and GoLand keymaps.

import {
  Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType,
  crosshairCursor, gutter, keymap, rectangularSelection,
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
import { openSearchPanel, selectNextOccurrence, selectSelectionMatches } from '@codemirror/search';
import type {
  IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem, IdeInlineValue, IdePeerCursor,
} from '../api/types';
import { ctxMenu } from '../contextmenu.svelte';
import { isIconName } from './Icon.svelte';
import { createIdeCsvExtension } from './cm-ide-csv';
import {
  createIdeGitAndAgentExtensions, selectionOrCurrentLine, setBlameLinesEffect, setSavedContentEffect,
  type IdeEditorSelectionRange,
} from './cm-ide-git-and-agent';
import {
  buildPooledLspExtensions, createCmdHoverDiveExtension, createDirectLspExtension,
  disposePooledLspClients, lspUriToPath, notifyLspDidSave, requestIdeDiveAtPos,
  type AttachPooledLspOptions, type DirectLspOptions,
} from './cm-ide-lsp';

export {
  addCursorAbove, addCursorBelow, buildPooledLspExtensions, createDirectLspExtension,
  crosshairCursor, disposePooledLspClients, lspUriToPath, moveLineDown, moveLineUp, notifyLspDidSave,
  rectangularSelection, requestIdeDiveAtPos, selectNextOccurrence,
  selectionOrCurrentLine, setBlameLinesEffect, setSavedContentEffect,
  type AttachPooledLspOptions, type DirectLspOptions, type IdeEditorSelectionRange,
};
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

/** Add `prefix` before and `suffix` after every selected/cursor line in one undoable transaction. */
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

export function openReplaceInFile(view: EditorView): boolean {
  openSearchPanel(view);
  requestAnimationFrame(() => {
    const rep = view.dom.querySelector<HTMLInputElement>('.cm-search input[name="replace"]');
    rep?.focus();
    rep?.select();
  });
  return true;
}

// ── State Effects & Callbacks ────────────────────────────────────────────────

export interface IdeEditorCallbacks {
  onToggleBreakpoint?: ((line: number) => void) | null;
  onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
  onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug') => void) | null;
  onDefinitionJump?: ((target: { path?: string; line: number; column: number; symbol?: string }) => void) | null;
  onImplementationJump?: ((target: { path?: string; line: number; column: number; symbol?: string }) => void) | null;
  onFindUsages?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onFileStructure?: (() => void) | null;
  onJumpSymbol?: ((delta: -1 | 1) => void) | null;
  onNavigateBack?: (() => void) | null;
  onNavigateForward?: (() => void) | null;
  onQuickFix?: ((ctx: { line: number; column: number; symbol?: string }) => void) | null;
  onRenameSymbol?: ((ctx: { line: number; column: number; symbol: string }) => void) | null;
  onFormat?: (() => void) | null;
  onToggleBlame?: (() => void) | null;
  onDiagnostics?: ((diags: IdeDiagnostic[]) => void) | null;
  onAgentAction?: ((mode: 'chat' | 'inline', sel: IdeEditorSelectionRange) => void) | null;
}

export interface IdeDebugStatePayload {
  pausedLine: number | null;
  inlineValues: IdeInlineValue[];
}

export type BreakpointsEffectPayload = IdeBreakpoint[] | { breakpoints: IdeBreakpoint[]; muted?: boolean };
export type PeerCursorInput = IdePeerCursor | IdeCollabPeer;

export const setBreakpointsEffect = StateEffect.define<BreakpointsEffectPayload>();
export const setGutterItemsEffect = StateEffect.define<IdeGutterItem[]>();
export const setDebugDecorationsEffect = StateEffect.define<IdeDebugStatePayload>();
export const setPeerCursorsEffect = StateEffect.define<PeerCursorInput[]>();
const setHoverBpLineEffect = StateEffect.define<number | null>();

// ── Gutter Markers & Decorations ─────────────────────────────────────────────

type BpVisualKind = 'enabled' | 'disabled' | 'muted' | 'conditional' | 'logpoint' | 'paused' | 'preview';

const SVG_NS = 'http://www.w3.org/2000/svg';
function gutterSvg(shape: 'enabled' | 'hollow' | 'diamond' | 'play', verified = false): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  for (const [k, v] of [['width', '12'], ['height', '12'], ['viewBox', '0 0 16 16'], ['aria-hidden', 'true']]) svg.setAttribute(k, v);
  if (shape === 'enabled' || shape === 'hollow') {
    const c = document.createElementNS(SVG_NS, 'circle');
    for (const [k, v] of [['cx', '8'], ['cy', '8'], ['r', shape === 'hollow' ? '4.5' : '5'], ['fill', shape === 'hollow' ? 'none' : 'currentColor']]) c.setAttribute(k, v);
    if (shape === 'hollow') { c.setAttribute('stroke', 'currentColor'); c.setAttribute('stroke-width', '1.5'); }
    svg.appendChild(c);
    if (verified && shape === 'enabled') {
      const check = document.createElementNS(SVG_NS, 'path');
      check.setAttribute('d', 'M5.4 8.2 7.2 10 10.8 6.2');
      check.setAttribute('fill', 'none');
      check.setAttribute('stroke', 'var(--bg)');
      check.setAttribute('stroke-width', '1.6');
      check.setAttribute('stroke-linecap', 'round');
      check.setAttribute('stroke-linejoin', 'round');
      svg.appendChild(check);
    }
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
    readonly kind: BpVisualKind,
    readonly tooltip: string,
    readonly alsoPaused: boolean,
    readonly verified: boolean,
  ) { super(); }
  eq(o: BreakpointMarker): boolean {
    return this.kind === o.kind && this.tooltip === o.tooltip && this.alsoPaused === o.alsoPaused && this.verified === o.verified;
  }
  toDOM(): HTMLElement {
    const el = document.createElement('span');
    el.className = `cm-ide-bp-marker cm-ide-bp-${this.kind}${this.alsoPaused ? ' is-paused' : ''}${this.verified ? ' is-verified' : ''}`;
    el.dataset.bpKind = this.kind;
    if (this.kind !== 'preview') {
      el.title = this.tooltip;
      el.setAttribute('role', 'img');
      el.setAttribute('aria-label', this.tooltip);
    } else {
      el.setAttribute('aria-hidden', 'true');
    }
    const shape = this.kind === 'paused'
      ? 'play'
      : (this.kind === 'conditional' || this.kind === 'logpoint')
        ? 'diamond'
        : (this.kind === 'disabled' || this.kind === 'muted')
          ? 'hollow'
          : 'enabled';
    el.appendChild(gutterSvg(shape, this.verified));
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
      m.set(iv.name, iv.value.length > 80 ? `${iv.value.slice(0, 77)}…` : iv.value);
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

// ── Shell `$VAR` Viewport Occurrence Highlighter (`.sh` / `.bash` / `.zsh`) ──

const shellVarMark = Decoration.mark({ class: 'cm-ide-shell-var-hit' });

function createShellVarHighlightExtension(): Extension {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet = Decoration.none;
      constructor(view: EditorView) { this.decorations = this.compute(view); }
      update(u: ViewUpdate) {
        if (u.docChanged || u.selectionSet || u.viewportChanged) this.decorations = this.compute(u.view);
      }
      compute(view: EditorView): DecorationSet {
        const sym = wordAtCursor(view.state).word.replace(/^\$/, '');
        if (!sym || sym.length < 2 || !/^[A-Za-z_]\w*$/.test(sym)) return Decoration.none;
        const lnText = view.state.doc.lineAt(view.state.selection.main.head).text;
        if (!lnText.includes(`$${sym}`) && !lnText.includes(`\${${sym}`) && !new RegExp(`\\b${sym}=`).test(lnText)) {
          return Decoration.none;
        }
        const re = new RegExp(`(?:\\$\\{?${sym}\\}?|\\b${sym}(?==))`, 'g');
        const ranges: Range<Decoration>[] = [];
        for (const { from, to } of view.visibleRanges) {
          const text = view.state.sliceDoc(from, to);
          let m: RegExpExecArray | null;
          while ((m = re.exec(text)) !== null) {
            ranges.push(shellVarMark.range(from + m.index, from + m.index + m[0].length));
          }
        }
        return ranges.length > 1 ? Decoration.set(ranges, true) : Decoration.none;
      }
    },
    { decorations: (v) => v.decorations },
  );
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
  '.cm-ide-bp-gutter, .cm-lineNumbers': { cursor: 'pointer', userSelect: 'none' },
  '.cm-ide-bp-gutter': { width: '16px' },
  '.cm-ide-bp-gutter .cm-gutterElement': { display: 'flex', alignItems: 'center', justifyContent: 'center', paddingInline: '2px' },
  '.cm-ide-bp-spacer, .cm-ide-run-spacer': { visibility: 'hidden', display: 'inline-flex' },
  '.cm-ide-bp-marker': { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', lineHeight: '1', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-preview': { opacity: '0.42', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-disabled, .cm-ide-bp-marker.cm-ide-bp-muted': { color: 'var(--text-dim)' },
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
  '.cm-ide-shell-var-hit': { backgroundColor: 'var(--accent-soft)', borderRadius: '2px' },
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
    { key: 'Mod-Alt-ArrowUp', run: addCursorAbove },
    { key: 'Mod-Alt-ArrowDown', run: addCursorBelow },
    { key: 'Alt-Shift-ArrowUp', run: moveLineUp },
    { key: 'Alt-Shift-ArrowDown', run: moveLineDown },
    { key: 'Alt-ArrowUp', run: (v) => { const cb = getCallbacks().onJumpSymbol; if (cb) { cb(-1); return true; } return moveLineUp(v); } },
    { key: 'Alt-ArrowDown', run: (v) => { const cb = getCallbacks().onJumpSymbol; if (cb) { cb(1); return true; } return moveLineDown(v); } },
    { key: 'Ctrl-g', run: selectNextOccurrence },
    { key: 'Ctrl-Mod-g', run: selectAllOccurrences },
    { key: 'Alt-Shift-i', run: splitSelectionIntoLines },
    { key: 'Alt-Shift-I', run: splitSelectionIntoLines },
    { key: 'Mod-/', run: toggleLineComment },
    { key: 'Mod-r', run: openReplaceInFile },
    { key: 'F12', run: (v) => { const cbs = getCallbacks(); if (!cbs.onDefinitionJump) return false; void requestIdeDiveAtPos(v, v.state.selection.main.head, 'definition', cbs); return true; } },
    { key: 'Mod-b', run: (v) => { const cbs = getCallbacks(); if (!cbs.onDefinitionJump) return false; void requestIdeDiveAtPos(v, v.state.selection.main.head, 'definition', cbs); return true; } },
    { key: 'Mod-Alt-b', run: (v) => { const cbs = getCallbacks(); if (!cbs.onImplementationJump && !cbs.onDefinitionJump) return false; void requestIdeDiveAtPos(v, v.state.selection.main.head, 'implementation', cbs); return true; } },
    { key: 'Alt-F7', run: (v) => { const cb = getCallbacks().onFindUsages; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word || undefined }); return true; } },
    { key: 'Mod-F12', run: () => { const cb = getCallbacks().onFileStructure; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-Alt-ArrowLeft', run: () => { const cb = getCallbacks().onNavigateBack; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-Alt-ArrowRight', run: () => { const cb = getCallbacks().onNavigateForward; if (!cb) return false; cb(); return true; } },
    { key: 'Alt-Enter', run: (v) => { const cb = getCallbacks().onQuickFix; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word || undefined }); return true; } },
    { key: 'Shift-F6', run: (v) => { const cb = getCallbacks().onRenameSymbol; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word }); return true; } },
    { key: 'Mod-Alt-l', run: () => { const cb = getCallbacks().onFormat; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-F8', run: (v) => { const cb = getCallbacks().onToggleBreakpoint; if (!cb) return false; cb(v.state.doc.lineAt(v.state.selection.main.head).number); return true; } },
    ...(preset === 'jetbrains'
      ? [
          { key: 'Mod-[', run: () => { const cb = getCallbacks().onNavigateBack; if (!cb) return false; cb(); return true; } },
          { key: 'Mod-]', run: () => { const cb = getCallbacks().onNavigateForward; if (!cb) return false; cb(); return true; } },
          { key: 'Mod-d', run: duplicateLineOrSelection },
          { key: 'Mod-Backspace', run: deleteLine },
        ]
      : [
          { key: 'Mod-d', run: selectNextOccurrence },
          { key: 'Shift-Mod-d', run: duplicateLineOrSelection },
          { key: 'Shift-Mod-k', run: deleteLine },
        ]),
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
  filePath?: string;
  breakpoints: IdeBreakpoint[];
  breakpointsMuted?: boolean;
  gutterItems: IdeGutterItem[];
  pausedLine: number | null;
  inlineValues: IdeInlineValue[];
  peerCursors: PeerCursorInput[];
  savedContent?: string | null;
  blameByLine?: Record<number, string> | null;
  keymapPreset: 'jetbrains' | 'standard';
  getCallbacks: () => IdeEditorCallbacks;
}

function isBpTargetGutter(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest('.cm-ide-run-gutter, .cm-foldGutter, .cm-ide-diff-gutter')) return false;
  return Boolean(target.closest('.cm-ide-bp-gutter, .cm-lineNumbers'));
}

export function createIdeExtensions(opts: CreateIdeExtensionsOptions): Extension {
  const bpField = StateField.define<{ map: Map<number, IdeBreakpoint>; muted: boolean; pausedLine: number | null; hoverLine: number | null }>({
    create() {
      const map = new Map<number, IdeBreakpoint>();
      for (const bp of opts.breakpoints) map.set(bp.line, bp);
      return { map, muted: Boolean(opts.breakpointsMuted), pausedLine: opts.pausedLine, hoverLine: null };
    },
    update(val, tr) {
      let { map, muted, pausedLine, hoverLine } = val;
      for (const ef of tr.effects) {
        if (ef.is(setBreakpointsEffect)) {
          const raw = ef.value;
          const list = Array.isArray(raw) ? raw : raw.breakpoints;
          if (!Array.isArray(raw) && raw.muted !== undefined) muted = raw.muted;
          map = new Map();
          for (const bp of list) map.set(bp.line, bp);
        } else if (ef.is(setDebugDecorationsEffect)) {
          pausedLine = ef.value.pausedLine;
        } else if (ef.is(setHoverBpLineEffect)) {
          hoverLine = ef.value;
        }
      }
      if (tr.docChanged && hoverLine != null && hoverLine > tr.state.doc.lines) hoverLine = null;
      return map === val.map && muted === val.muted && pausedLine === val.pausedLine && hoverLine === val.hoverLine
        ? val
        : { map, muted, pausedLine, hoverLine };
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

  const handleBpClick = (_view: EditorView, ln: number, me: MouseEvent): boolean => {
    const cbs = opts.getCallbacks();
    if (me.shiftKey && cbs.onEditBreakpoint) { me.preventDefault(); cbs.onEditBreakpoint(ln, me); return true; }
    if (cbs.onToggleBreakpoint) { me.preventDefault(); cbs.onToggleBreakpoint(ln); return true; }
    return false;
  };
  const handleBpContextMenu = (view: EditorView, ln: number, me: MouseEvent): boolean => {
    const cbs = opts.getCallbacks();
    if (!cbs.onToggleBreakpoint && !cbs.onEditBreakpoint) return false;
    me.preventDefault(); me.stopPropagation();
    if (cbs.onEditBreakpoint) { cbs.onEditBreakpoint(ln, me); return true; }
    const bp = view.state.field(bpField).map.get(ln);
    ctxMenu.show(me, [{ label: bp ? 'Remove breakpoint' : 'Add breakpoint', icon: bp ? 'trash' : 'plus', action: () => cbs.onToggleBreakpoint?.(ln) }]);
    return true;
  };

  const bpGutter = gutter({
    class: 'cm-ide-bp-gutter',
    markers(view) {
      const { map, muted, pausedLine, hoverLine } = view.state.field(bpField);
      if (map.size === 0 && pausedLine == null && hoverLine == null) return RangeSet.empty;
      const markers: Range<GutterMarker>[] = [];
      const lines = new Set<number>(map.keys());
      if (pausedLine != null) lines.add(pausedLine);
      if (hoverLine != null) lines.add(hoverLine);
      for (const ln of [...lines].sort((a, b) => a - b)) {
        if (ln < 1 || ln > view.state.doc.lines) continue;
        const pos = view.state.doc.line(ln).from;
        const bp = map.get(ln) as (IdeBreakpoint & { verified?: boolean }) | undefined;
        const isPaused = pausedLine === ln;
        if (bp) {
          const kind: BpVisualKind = muted ? 'muted' : !bp.enabled ? 'disabled' : bp.log_message ? 'logpoint' : (bp.condition || bp.hit_condition) ? 'conditional' : 'enabled';
          const tip = muted ? `Muted breakpoint at line ${ln}` : bp.log_message ? `Logpoint at line ${ln}: ${bp.log_message}`
            : bp.condition ? `Conditional breakpoint at line ${ln}: ${bp.condition}` : !bp.enabled ? `Disabled breakpoint at line ${ln}`
            : `${bp.verified ? 'Verified b' : 'B'}reakpoint at line ${ln} (right-click for condition/logpoint)`;
          markers.push(new BreakpointMarker(kind, tip, isPaused, Boolean(bp.verified && !muted && bp.enabled)).range(pos));
        } else if (isPaused) markers.push(new BreakpointMarker('paused', `Paused at line ${ln}`, false, false).range(pos));
        else if (hoverLine === ln) markers.push(new BreakpointMarker('preview', '', false, false).range(pos));
      }
      return RangeSet.of(markers, true);
    },
    initialSpacer: () => bpSpacer,
    domEventHandlers: {
      mousedown: (v, line, ev) => (ev as MouseEvent).button === 0 && handleBpClick(v, v.state.doc.lineAt(line.from).number, ev as MouseEvent),
      contextmenu: (v, line, ev) => handleBpContextMenu(v, v.state.doc.lineAt(line.from).number, ev as MouseEvent),
    },
  });

  // Track hover preview + click/right-click across BOTH the breakpoint strip AND line numbers!
  const gutterInteractionHandlers = EditorView.domEventHandlers({
    mousemove(event, view) {
      if (!opts.getCallbacks().onToggleBreakpoint || !isBpTargetGutter(event.target)) {
        if (view.state.field(bpField).hoverLine !== null) view.dispatch({ effects: setHoverBpLineEffect.of(null) });
        return false;
      }
      const block = view.lineBlockAtHeight(event.clientY - view.documentTop);
      const ln = view.state.doc.lineAt(block.from).number;
      if (view.state.field(bpField).hoverLine !== ln) view.dispatch({ effects: setHoverBpLineEffect.of(ln) });
      return false;
    },
    mouseleave(_event, view) {
      if (view.state.field(bpField).hoverLine !== null) view.dispatch({ effects: setHoverBpLineEffect.of(null) });
      return false;
    },
    mousedown(event, view) {
      if (event.button !== 0 || !(event.target instanceof Element)) return false;
      // `.cm-ide-bp-gutter` already handles its own mousedown; wire `.cm-lineNumbers` too!
      if (!event.target.closest('.cm-lineNumbers') || event.metaKey || event.ctrlKey) return false;
      const block = view.lineBlockAtHeight(event.clientY - view.documentTop);
      return handleBpClick(view, view.state.doc.lineAt(block.from).number, event);
    },
    contextmenu(event, view) {
      if (!(event.target instanceof Element) || !event.target.closest('.cm-lineNumbers')) return false;
      const block = view.lineBlockAtHeight(event.clientY - view.documentTop);
      return handleBpContextMenu(view, view.state.doc.lineAt(block.from).number, event);
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

  const fPath = (opts.filePath ?? '').toLowerCase().split('?')[0];
  const isCsv = fPath.endsWith('.csv') || fPath.endsWith('.tsv');
  const isShell = fPath.endsWith('.sh') || fPath.endsWith('.bash') || fPath.endsWith('.zsh');

  return [
    ideEditorTheme, bpField, runField, debugField, peersField, bpGutter, gutterInteractionHandlers, runGutter,
    createCmdHoverDiveExtension(opts.getCallbacks),
    createIdeGitAndAgentExtensions({
      savedContent: opts.savedContent,
      blameByLine: opts.blameByLine,
      wordAtCursor,
      getCallbacks: opts.getCallbacks,
    }),
    ...(isCsv ? [createIdeCsvExtension(opts.filePath ?? '')] : []),
    ...(isShell ? [createShellVarHighlightExtension()] : []),
    buildIdeKeymap(opts.keymapPreset, opts.getCallbacks),
  ];
}
