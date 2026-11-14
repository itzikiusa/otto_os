// Pro IDE extensions for CodeEditor (JetBrains GoLand / IntelliJ parity):
// 1-click gutter breakpoints right beside line numbers (including read-only SDK files),
// combined breakpoint/run/lightbulb gutter, semantic coloring + rainbow brackets + quick-doc hover,
// full JetBrains macOS keymap, and `Git ▸` / `Local History ▸` / `Compare with Clipboard` menus.

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
  moveLineDown, moveLineUp, selectParentSyntax, simplifySelection,
  toggleBlockComment, toggleComment,
} from '@codemirror/commands';
import { gotoLine as cmGotoLine, openSearchPanel, selectNextOccurrence, selectSelectionMatches } from '@codemirror/search';
import type {
  IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem, IdeInlineValue, IdePeerCursor,
} from '../api/types.ts';
import { ctxMenu, type MenuItem } from '../contextmenu.svelte.ts';
import { isIconName } from './Icon.svelte';
import { createIdeCsvExtension } from './cm-ide-csv.ts';
function isIdeManifestPath(p: string): boolean {
  const b = (p.split('?')[0].split('/').pop() ?? '').toLowerCase();
  return b === 'go.mod' || b === 'pom.xml' || b === 'package.json' || b === 'cargo.toml' || b === 'pyproject.toml' || b === 'build.gradle' || b === 'build.gradle.kts' || /^requirements(?:[-_.][\w-]+)?\.txt$/.test(b);
}
import {
  createIdeGitAndAgentExtensions, selectionOrCurrentLine, setBlameLinesEffect, setSavedContentEffect,
  type IdeEditorSelectionRange,
} from './cm-ide-git-and-agent.ts';
import { buildIdeVcsContextRows } from './cm-ide-history.ts';
import { createIdeHoverExtension } from './cm-ide-hover.ts';
import {
  buildPooledLspExtensions, createCmdHoverDiveExtension, createDirectLspExtension,
  disposePooledLspClients, lspUriToPath, notifyLspDidSave, requestIdeDiveAtPos,
  type AttachPooledLspOptions, type DirectLspOptions,
} from './cm-ide-lsp.ts';
import {
  applyConcreteEditsToView, computeDeterministicQuickFixes, previewConcreteEdits,
  type IdeConcreteTextEdit, type IdeQuickFixAction,
} from './cm-ide-quickfix.ts';
import {
  applyClientQuickFix, createIdeSemanticExtensions, getIdeQuickFixesAtLine,
  type ClientDiagQuickFix,
} from './cm-ide-semantic.ts';
import { readIdeEditorPreferences } from './cm-ide-theme.ts';

export {
  addCursorAbove, addCursorBelow, applyClientQuickFix, applyConcreteEditsToView, buildPooledLspExtensions,
  computeDeterministicQuickFixes, createDirectLspExtension, crosshairCursor,
  disposePooledLspClients, getIdeQuickFixesAtLine, lspUriToPath, moveLineDown, moveLineUp, notifyLspDidSave,
  previewConcreteEdits, rectangularSelection, requestIdeDiveAtPos, selectNextOccurrence, selectionOrCurrentLine,
  setBlameLinesEffect, setSavedContentEffect,
  type AttachPooledLspOptions, type ClientDiagQuickFix, type DirectLspOptions, type IdeConcreteTextEdit,
  type IdeEditorSelectionRange, type IdeQuickFixAction,
};
export const toggleLineComment = toggleComment;

const IDENT_CHAR_RE = /[\w$]/;

export function wordAtCursor(state: EditorState): { word: string; line: number; column: number } {
  const main = state.selection.main, ln = state.doc.lineAt(main.head), column = main.head - ln.from + 1;
  if (!main.empty) return { word: state.sliceDoc(main.from, main.to).trim(), line: ln.number, column };
  const text = ln.text, rel = main.head - ln.from;
  let s = rel, e = rel;
  while (s > 0 && IDENT_CHAR_RE.test(text[s - 1])) s--;
  while (e < text.length && IDENT_CHAR_RE.test(text[e])) e++;
  return { word: text.slice(s, e), line: ln.number, column };
}

export function selectAllOccurrences(view: EditorView): boolean {
  const { state } = view;
  if (state.selection.main.empty) {
    const main = state.selection.main, ln = state.doc.lineAt(main.head), rel = main.head - ln.from;
    let s = rel, e = rel;
    while (s > 0 && IDENT_CHAR_RE.test(ln.text[s - 1])) s--;
    while (e < ln.text.length && IDENT_CHAR_RE.test(ln.text[e])) e++;
    if (e > s) view.dispatch({ selection: { anchor: ln.from + s, head: ln.from + e } });
  }
  return selectSelectionMatches(view);
}

export function splitSelectionIntoLines(view: EditorView): boolean {
  const { state } = view, next: SelectionRange[] = [];
  let changed = false;
  for (const r of state.selection.ranges) {
    const startLn = state.doc.lineAt(r.from).number;
    let endLn = state.doc.lineAt(r.to).number;
    if (!r.empty && r.to > r.from && state.doc.lineAt(r.to).from === r.to) endLn = state.doc.lineAt(r.to - 1).number;
    if (endLn <= startLn) { next.push(r); continue; }
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

export function applyLinePrefixSuffix(view: EditorView, prefix: string, suffix: string): boolean {
  if ((!prefix && !suffix) || view.state.readOnly) return false;
  const { doc, selection } = view.state, touched = new Set<number>();
  for (const r of selection.ranges) {
    const startLn = doc.lineAt(r.from).number;
    const endLn = !r.empty && r.to > r.from && doc.lineAt(r.to).from === r.to ? doc.lineAt(r.to - 1).number : doc.lineAt(r.to).number;
    for (let ln = startLn; ln <= endLn; ln++) touched.add(ln);
  }
  const lines = [...touched].sort((a, b) => a - b);
  if (!lines.length) return false;
  const changes: ChangeSpec[] = [], cursors: SelectionRange[] = [], delta = prefix.length + suffix.length;
  lines.forEach((ln, idx) => {
    const line = doc.line(ln);
    if (prefix.length) changes.push({ from: line.from, to: line.from, insert: prefix });
    if (suffix.length) changes.push({ from: line.to, to: line.to, insert: suffix });
    cursors.push(EditorSelection.cursor(line.to + (idx + 1) * delta));
  });
  view.dispatch({ changes, selection: EditorSelection.create(cursors, cursors.length - 1), userEvent: 'input', scrollIntoView: true });
  return true;
}

export function duplicateLineOrSelection(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  if (!view.state.selection.ranges.some((r) => !r.empty)) return copyLineDown(view);
  const tr = view.state.changeByRange((range) => {
    if (range.empty) {
      const ln = view.state.doc.lineAt(range.head), ins = `\n${ln.text}`;
      return { changes: { from: ln.to, to: ln.to, insert: ins }, range: EditorSelection.cursor(range.head + ins.length) };
    }
    const sel = view.state.sliceDoc(range.from, range.to);
    return { changes: { from: range.to, to: range.to, insert: sel }, range: EditorSelection.range(range.to, range.to + sel.length) };
  });
  view.dispatch(view.state.update(tr, { userEvent: 'input', scrollIntoView: true }));
  return true;
}

export function startNewLineBelow(view: EditorView): boolean {
  if (view.state.readOnly) return false;
  const ln = view.state.doc.lineAt(view.state.selection.main.head);
  const indent = /^(\s*)/.exec(ln.text)?.[1] ?? '';
  const ins = `\n${indent}`;
  view.dispatch({ changes: { from: ln.to, to: ln.to, insert: ins }, selection: EditorSelection.cursor(ln.to + ins.length), userEvent: 'input', scrollIntoView: true });
  return true;
}

export function smartExpandSelection(view: EditorView): boolean {
  const main = view.state.selection.main, ln = view.state.doc.lineAt(main.head);
  if (main.empty) {
    const rel = main.head - ln.from;
    let s = rel, e = rel;
    while (s > 0 && IDENT_CHAR_RE.test(ln.text[s - 1])) s--;
    while (e < ln.text.length && IDENT_CHAR_RE.test(ln.text[e])) e++;
    if (e > s) { view.dispatch({ selection: { anchor: ln.from + s, head: ln.from + e } }); return true; }
  }
  if (selectParentSyntax(view)) return true;
  if (main.from > ln.from || main.to < ln.to) { view.dispatch({ selection: { anchor: ln.from, head: ln.to } }); return true; }
  return false;
}

export function openReplaceInFile(view: EditorView): boolean {
  openSearchPanel(view);
  requestAnimationFrame(() => { const rep = view.dom.querySelector<HTMLInputElement>('.cm-search input[name="replace"]'); rep?.focus(); rep?.select(); });
  return true;
}

// ── State Effects & Callbacks ────────────────────────────────────────────────

export interface IdeEditorCallbacks {
  onToggleBreakpoint?: ((line: number) => void) | null;
  onEditBreakpoint?: ((line: number, event?: MouseEvent) => void) | null;
  onGutterAction?: ((item: IdeGutterItem, mode: 'run' | 'debug') => void) | null;
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
  onAgentAction?: ((mode: 'chat' | 'inline', sel: IdeEditorSelectionRange) => void) | null;
  onPasteFromHistory?: (() => void) | null;
  onGoToLine?: (() => void) | null;
}

export interface IdeDebugStatePayload { pausedLine: number | null; inlineValues: IdeInlineValue[]; }
export type BreakpointsEffectPayload = IdeBreakpoint[] | { breakpoints: IdeBreakpoint[]; muted?: boolean };
export type PeerCursorInput = IdePeerCursor | IdeCollabPeer;

export const setBreakpointsEffect = StateEffect.define<BreakpointsEffectPayload>();
export const setGutterItemsEffect = StateEffect.define<IdeGutterItem[]>();
export const setDebugDecorationsEffect = StateEffect.define<IdeDebugStatePayload>();
export const setPeerCursorsEffect = StateEffect.define<PeerCursorInput[]>();
const setHoverBpLineEffect = StateEffect.define<number | null>();

type BpVisualKind = 'enabled' | 'disabled' | 'muted' | 'conditional' | 'logpoint' | 'paused' | 'preview';
const SVG_NS = 'http://www.w3.org/2000/svg';

function gutterSvg(shape: 'enabled' | 'hollow' | 'diamond' | 'play', verified = false, conditional = false): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  for (const [k, v] of [['width', '12'], ['height', '12'], ['viewBox', '0 0 16 16'], ['aria-hidden', 'true']]) svg.setAttribute(k, v);
  if (shape === 'enabled' || shape === 'hollow') {
    const c = document.createElementNS(SVG_NS, 'circle');
    for (const [k, v] of [['cx', '8'], ['cy', '8'], ['r', shape === 'hollow' ? '4.8' : '5.5'], ['fill', shape === 'hollow' ? 'none' : 'currentColor']]) c.setAttribute(k, v);
    if (shape === 'hollow') { c.setAttribute('stroke', 'currentColor'); c.setAttribute('stroke-width', '1.6'); }
    svg.appendChild(c);
    if (conditional && shape === 'enabled') {
      const txt = document.createElementNS(SVG_NS, 'text');
      for (const [k, v] of [['x', '8'], ['y', '11'], ['text-anchor', 'middle'], ['font-size', '9'], ['font-weight', '700'], ['fill', 'var(--bg)']]) txt.setAttribute(k, v);
      txt.textContent = '?'; svg.appendChild(txt);
    } else if (verified && shape === 'enabled') {
      const check = document.createElementNS(SVG_NS, 'path');
      for (const [k, v] of [['d', 'M5.2 8.2 7.1 10.1 10.9 6.1'], ['fill', 'none'], ['stroke', 'var(--bg)'], ['stroke-width', '1.7'], ['stroke-linecap', 'round'], ['stroke-linejoin', 'round']]) check.setAttribute(k, v);
      svg.appendChild(check);
    }
  } else {
    const p = document.createElementNS(SVG_NS, 'path');
    p.setAttribute('d', shape === 'diamond' ? 'M8 2.4L13.6 8L8 13.6L2.4 8Z' : 'M5 3.4l7.2 4.6-7.2 4.6v-9.2Z');
    p.setAttribute('fill', 'currentColor'); svg.appendChild(p);
  }
  return svg;
}

class CombinedGutterMarker extends GutterMarker {
  bpKind: BpVisualKind | null; tooltip: string; alsoPaused: boolean; verified: boolean;
  runItem: IdeGutterItem | null; showBulb: boolean; line: number; getCallbacks: () => IdeEditorCallbacks;
  constructor(p: { bpKind: BpVisualKind | null; tooltip: string; alsoPaused: boolean; verified: boolean; runItem: IdeGutterItem | null; showBulb: boolean; line: number; getCallbacks: () => IdeEditorCallbacks }) {
    super(); this.bpKind = p.bpKind; this.tooltip = p.tooltip; this.alsoPaused = p.alsoPaused; this.verified = p.verified;
    this.runItem = p.runItem; this.showBulb = p.showBulb; this.line = p.line; this.getCallbacks = p.getCallbacks;
  }
  eq(o: CombinedGutterMarker): boolean {
    return this.bpKind === o.bpKind && this.tooltip === o.tooltip && this.alsoPaused === o.alsoPaused && this.verified === o.verified &&
      this.showBulb === o.showBulb && this.line === o.line && this.runItem?.line === o.runItem?.line && this.runItem?.kind === o.runItem?.kind && this.runItem?.label === o.runItem?.label;
  }
  toDOM(): HTMLElement {
    const wrap = document.createElement('span'); wrap.className = 'cm-ide-bp-cell';
    if (this.bpKind) {
      const el = document.createElement('span');
      el.className = `cm-ide-bp-marker cm-ide-bp-${this.bpKind}${this.alsoPaused ? ' is-paused' : ''}${this.verified ? ' is-verified' : ''}`;
      el.dataset.bpKind = this.bpKind;
      el.dataset.testid = this.bpKind === 'preview' ? 'ide-bp-hover-preview' : 'ide-bp-marker';
      if (this.bpKind !== 'preview') { el.title = this.tooltip; el.setAttribute('role', 'img'); el.setAttribute('aria-label', this.tooltip); }
      else el.setAttribute('aria-hidden', 'true');
      const shape = this.bpKind === 'paused' ? 'play' : this.bpKind === 'logpoint' ? 'diamond' : (this.bpKind === 'disabled' || this.bpKind === 'muted') ? 'hollow' : 'enabled';
      el.appendChild(gutterSvg(shape, this.verified, this.bpKind === 'conditional'));
      wrap.appendChild(el);
    }
    if (this.runItem) {
      const item = this.runItem, btn = document.createElement('button');
      btn.type = 'button'; btn.className = `cm-ide-gutter-run-btn cm-ide-run-btn is-${item.kind}`;
      btn.dataset.gutterRun = String(item.line); btn.dataset.gutterLine = String(item.line); btn.dataset.gutterKind = item.kind;
      const label = `Run or debug ${item.label}`; btn.title = label; btn.setAttribute('aria-label', label);
      btn.appendChild(gutterSvg('play'));
      btn.addEventListener('mousedown', (e) => { e.stopPropagation(); });
      btn.addEventListener('click', (e) => {
        e.preventDefault(); e.stopPropagation();
        const cbs = this.getCallbacks();
        ctxMenu.showAt(btn, [
          { label: `Run '${item.label}'`, icon: 'play', action: () => cbs.onGutterAction?.(item, 'run') },
          { label: `Debug '${item.label}'`, icon: isIconName('bug') ? 'bug' : 'target', action: () => cbs.onGutterAction?.(item, 'debug') },
        ]);
      });
      wrap.appendChild(btn);
    } else if (this.showBulb) {
      const bulb = document.createElement('button');
      bulb.type = 'button'; bulb.className = 'cm-ide-lightbulb-btn'; bulb.dataset.testid = 'ide-gutter-lightbulb';
      bulb.title = 'Show Quick Fixes (⌥⏎)'; bulb.setAttribute('aria-label', 'Show Quick Fixes'); bulb.textContent = '💡';
      bulb.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); const r = bulb.getBoundingClientRect(); this.getCallbacks().onQuickFix?.({ line: this.line, column: 1, clientX: r.left, clientY: r.bottom + 4 }); });
      wrap.appendChild(bulb);
    }
    return wrap;
  }
}

const bpSpacer = new (class extends GutterMarker {
  toDOM(): HTMLElement { const s = document.createElement('span'); s.className = 'cm-ide-bp-spacer'; s.setAttribute('aria-hidden', 'true'); s.appendChild(gutterSvg('enabled')); return s; }
})();

import { isBpTargetGutterElement } from './cm-ide-history.ts';
export { isBpTargetGutterElement };

export interface CreateIdeExtensionsOptions {
  filePath?: string; language?: string; root?: string; connectionId?: string | null; projectFolders?: string[];
  breakpoints: IdeBreakpoint[]; breakpointsMuted?: boolean; gutterItems: IdeGutterItem[];
  pausedLine: number | null; inlineValues: IdeInlineValue[]; peerCursors: PeerCursorInput[];
  savedContent?: string | null; blameByLine?: Record<number, string> | null;
  keymapPreset: 'jetbrains' | 'standard'; getCallbacks: () => IdeEditorCallbacks;
}

/** Mounted immediately after `lineNumbers()` so the red breakpoint circle sits RIGHT NEXT to the line number! */
export function createIdeGutterExtensions(opts: CreateIdeExtensionsOptions): Extension {
  const bpLineMark = Decoration.line({ class: 'cm-ide-bp-line' });
  const bpField = StateField.define<{ map: Map<number, IdeBreakpoint>; muted: boolean; pausedLine: number | null; hoverLine: number | null; decos: DecorationSet }>({
    create(st) {
      const map = new Map<number, IdeBreakpoint>();
      for (const bp of opts.breakpoints) map.set(bp.line, bp);
      const muted = Boolean(opts.breakpointsMuted);
      return { map, muted, pausedLine: opts.pausedLine, hoverLine: null, decos: buildBpLineDecos(st.doc, map, muted) };
    },
    update(val, tr) {
      let { map, muted, pausedLine, hoverLine } = val;
      for (const ef of tr.effects) {
        if (ef.is(setBreakpointsEffect)) {
          const raw = ef.value, list = Array.isArray(raw) ? raw : raw.breakpoints;
          if (!Array.isArray(raw) && raw.muted !== undefined) muted = raw.muted;
          map = new Map(); for (const bp of list) map.set(bp.line, bp);
        } else if (ef.is(setDebugDecorationsEffect)) pausedLine = ef.value.pausedLine;
        else if (ef.is(setHoverBpLineEffect)) hoverLine = ef.value;
      }
      if (tr.docChanged && hoverLine != null && hoverLine > tr.state.doc.lines) hoverLine = null;
      if (map === val.map && muted === val.muted && pausedLine === val.pausedLine && hoverLine === val.hoverLine && !tr.docChanged) return val;
      return { map, muted, pausedLine, hoverLine, decos: buildBpLineDecos(tr.state.doc, map, muted) };
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  function buildBpLineDecos(doc: EditorState['doc'], map: Map<number, IdeBreakpoint>, muted: boolean): DecorationSet {
    if (!map.size || muted) return Decoration.none;
    const ranges: Range<Decoration>[] = [];
    for (const ln of [...map.keys()].sort((a, b) => a - b)) {
      const bp = map.get(ln);
      if (bp?.enabled && ln >= 1 && ln <= doc.lines) ranges.push(bpLineMark.range(doc.line(ln).from));
    }
    return ranges.length ? Decoration.set(ranges, true) : Decoration.none;
  }

  const runField = StateField.define<Map<number, IdeGutterItem>>({
    create() { const m = new Map<number, IdeGutterItem>(); for (const it of opts.gutterItems) m.set(it.line, it); return m; },
    update(m, tr) { for (const ef of tr.effects) if (ef.is(setGutterItemsEffect)) { const n = new Map<number, IdeGutterItem>(); for (const it of ef.value) n.set(it.line, it); return n; } return m; },
  });

  const handleBpClick = (_v: EditorView, ln: number, me: MouseEvent): boolean => {
    const cbs = opts.getCallbacks();
    if (me.shiftKey && cbs.onEditBreakpoint) { me.preventDefault(); cbs.onEditBreakpoint(ln, me); return true; }
    if (cbs.onToggleBreakpoint) { me.preventDefault(); cbs.onToggleBreakpoint(ln); return true; }
    return false;
  };
  const handleBpCtx = (v: EditorView, ln: number, me: MouseEvent): boolean => {
    const cbs = opts.getCallbacks();
    if (!cbs.onToggleBreakpoint && !cbs.onEditBreakpoint) return false;
    me.preventDefault(); me.stopPropagation();
    if (cbs.onEditBreakpoint) { cbs.onEditBreakpoint(ln, me); return true; }
    const bp = v.state.field(bpField).map.get(ln);
    ctxMenu.show(me, [{ label: bp ? 'Remove breakpoint' : 'Add breakpoint', icon: bp ? 'trash' : 'plus', action: () => cbs.onToggleBreakpoint?.(ln) }]);
    return true;
  };

  const bpAndRunGutter = gutter({
    class: 'cm-ide-bp-gutter',
    markers(view) {
      const { map, muted, pausedLine, hoverLine } = view.state.field(bpField), runMap = view.state.field(runField);
      const curLn = view.state.doc.lineAt(view.state.selection.main.head).number;
      const hasBulbOnCur = !view.state.readOnly && (getIdeQuickFixesAtLine(view.state, curLn).length > 0 || computeDeterministicQuickFixes(view.state.doc.toString(), opts.filePath ?? '', opts.language, curLn).length > 0);
      const lines = new Set<number>([...map.keys(), ...runMap.keys()]);
      if (pausedLine != null) lines.add(pausedLine);
      if (hoverLine != null) lines.add(hoverLine);
      if (hasBulbOnCur) lines.add(curLn);
      if (!lines.size) return RangeSet.empty;
      const markers: Range<GutterMarker>[] = [];
      for (const ln of [...lines].sort((a, b) => a - b)) {
        if (ln < 1 || ln > view.state.doc.lines) continue;
        const bp = map.get(ln) as (IdeBreakpoint & { verified?: boolean }) | undefined, runItem = runMap.get(ln) ?? null, isPaused = pausedLine === ln;
        let bpKind: BpVisualKind | null = null, tip = '', verified = false;
        if (bp) {
          bpKind = muted ? 'muted' : !bp.enabled ? 'disabled' : bp.log_message ? 'logpoint' : (bp.condition || bp.hit_condition) ? 'conditional' : 'enabled';
          tip = muted ? `Muted breakpoint at line ${ln}` : bp.log_message ? `Logpoint at line ${ln}: ${bp.log_message}` : bp.condition ? `Conditional breakpoint at line ${ln}: ${bp.condition}` : !bp.enabled ? `Disabled breakpoint at line ${ln}` : `${bp.verified ? 'Verified b' : 'B'}reakpoint at line ${ln}`;
          verified = Boolean((bp.verified || pausedLine != null) && !muted && bp.enabled);
        } else if (isPaused) { bpKind = 'paused'; tip = `Paused at line ${ln}`; }
        else if (hoverLine === ln) bpKind = 'preview';
        markers.push(new CombinedGutterMarker({ bpKind, tooltip: tip, alsoPaused: isPaused, verified, runItem, showBulb: hasBulbOnCur && ln === curLn && !runItem, line: ln, getCallbacks: opts.getCallbacks }).range(view.state.doc.line(ln).from));
      }
      return RangeSet.of(markers, true);
    },
    initialSpacer: () => bpSpacer,
  });

  const gutterHandlers = Prec.highest(EditorView.domEventHandlers({
    mousemove(ev, view) {
      if (!opts.getCallbacks().onToggleBreakpoint || !isBpTargetGutterElement(ev.target)) {
        if (view.state.field(bpField).hoverLine !== null) view.dispatch({ effects: setHoverBpLineEffect.of(null) });
        return false;
      }
      const ln = view.state.doc.lineAt(view.lineBlockAtHeight(ev.clientY - view.documentTop).from).number;
      if (view.state.field(bpField).hoverLine !== ln) view.dispatch({ effects: setHoverBpLineEffect.of(ln) });
      return false;
    },
    mouseleave(_ev, view) {
      if (view.state.field(bpField).hoverLine !== null) view.dispatch({ effects: setHoverBpLineEffect.of(null) });
      return false;
    },
    mousedown(ev, view) {
      if (ev.button !== 0 || ev.altKey || ev.metaKey || ev.ctrlKey || !isBpTargetGutterElement(ev.target)) return false;
      const ln = view.state.doc.lineAt(view.lineBlockAtHeight(ev.clientY - view.documentTop).from).number;
      return handleBpClick(view, ln, ev);
    },
    contextmenu(ev, view) {
      if (!isBpTargetGutterElement(ev.target)) return false;
      const ln = view.state.doc.lineAt(view.lineBlockAtHeight(ev.clientY - view.documentTop).from).number;
      return handleBpCtx(view, ln, ev);
    },
  }));

  return [bpField, runField, bpAndRunGutter, gutterHandlers];
}

class InlineGhostValuesWidget extends WidgetType {
  summary: string;
  constructor(summary: string) { super(); this.summary = summary; }
  eq(o: InlineGhostValuesWidget): boolean { return this.summary === o.summary; }
  toDOM(): HTMLElement { const s = document.createElement('span'); s.className = 'cm-ide-inline-values'; s.setAttribute('aria-hidden', 'true'); s.textContent = `  // ${this.summary}`; return s; }
}

function computeDebugDecorations(doc: EditorState['doc'], payload: IdeDebugStatePayload): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  if (payload.pausedLine != null && payload.pausedLine >= 1 && payload.pausedLine <= doc.lines) ranges.push(Decoration.line({ class: 'cm-ide-paused-line' }).range(doc.line(payload.pausedLine).from));
  if (payload.inlineValues.length) {
    const byLine = new Map<number, Map<string, string>>();
    for (const iv of payload.inlineValues) {
      if (iv.line < 1 || iv.line > doc.lines) continue;
      let m = byLine.get(iv.line); if (!m) { m = new Map(); byLine.set(iv.line, m); }
      m.set(iv.name, iv.value.length > 80 ? `${iv.value.slice(0, 77)}…` : iv.value);
    }
    for (const [ln, vars] of byLine) {
      const summary = [...vars.entries()].map(([k, v]) => `${k} = ${v}`).join(', ');
      if (summary) ranges.push(Decoration.widget({ widget: new InlineGhostValuesWidget(summary), side: 1 }).range(doc.line(ln).to));
    }
  }
  return ranges.length ? Decoration.set(ranges, true) : Decoration.none;
}

class PeerCaretWidget extends WidgetType {
  name: string; slot: number; eol: boolean;
  constructor(name: string, slot: number, eol: boolean) { super(); this.name = name; this.slot = slot; this.eol = eol; }
  eq(o: PeerCaretWidget): boolean { return this.name === o.name && this.slot === o.slot && this.eol === o.eol; }
  toDOM(): HTMLElement {
    const w = document.createElement('span'); w.className = `${this.eol ? 'cm-ide-peer-eol' : 'cm-ide-peer-caret'} cm-ide-peer-slot-${this.slot}`; w.setAttribute('aria-hidden', 'true');
    const p = document.createElement('span'); p.className = `cm-ide-peer-pill${this.eol ? ' is-eol' : ''}`; p.textContent = this.name; w.appendChild(p);
    return w;
  }
}

function computePeerDecorations(doc: EditorState['doc'], peers: PeerCursorInput[]): DecorationSet {
  if (!peers.length) return Decoration.none;
  const ranges: Range<Decoration>[] = [], maxPos = doc.length;
  for (const peer of peers) {
    let h = 0; const seed = `${peer.color ?? ''}:${peer.peer_id}`;
    for (let i = 0; i < seed.length; i++) h = (h * 31 + seed.charCodeAt(i)) | 0;
    const slot = Math.abs(h) % 6, name = peer.display_name || ('name' in peer ? peer.name : undefined) || 'Peer';
    if (peer.selections?.length) {
      for (const s of peer.selections) {
        const a = Math.max(0, Math.min(maxPos, s.anchor)), b = Math.max(0, Math.min(maxPos, s.head));
        if (a !== b) ranges.push(Decoration.mark({ class: `cm-ide-peer-sel cm-ide-peer-slot-${slot}` }).range(Math.min(a, b), Math.max(a, b)));
        ranges.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, false), side: 1 }).range(b));
      }
    } else if (peer.active_line != null && peer.active_line >= 1 && peer.active_line <= doc.lines) {
      ranges.push(Decoration.widget({ widget: new PeerCaretWidget(name, slot, true), side: 2 }).range(doc.line(peer.active_line).to));
    }
  }
  return ranges.length ? Decoration.set(ranges, true) : Decoration.none;
}

const shellVarMark = Decoration.mark({ class: 'cm-ide-shell-var-hit' });
function createShellVarHighlightExtension(): Extension {
  return ViewPlugin.fromClass(class {
    decorations: DecorationSet = Decoration.none;
    constructor(v: EditorView) { this.decorations = this.compute(v); }
    update(u: ViewUpdate) { if (u.docChanged || u.selectionSet || u.viewportChanged) this.decorations = this.compute(u.view); }
    compute(view: EditorView): DecorationSet {
      const sym = wordAtCursor(view.state).word.replace(/^\$/, '');
      if (!sym || sym.length < 2 || !/^[A-Za-z_]\w*$/.test(sym)) return Decoration.none;
      const re = new RegExp(`(?:\\$\\{?${sym}\\}?|\\b${sym}(?==))`, 'g'), ranges: Range<Decoration>[] = [];
      for (const { from, to } of view.visibleRanges) {
        const txt = view.state.sliceDoc(from, to); let m: RegExpExecArray | null;
        while ((m = re.exec(txt)) !== null) ranges.push(shellVarMark.range(from + m.index, from + m.index + m[0].length));
      }
      return ranges.length > 1 ? Decoration.set(ranges, true) : Decoration.none;
    }
  }, { decorations: (v) => v.decorations });
}

const ideEditorTheme = EditorView.baseTheme({
  '&': { '--ide-peer-color': 'var(--cat-1)' },
  '.cm-ide-peer-slot-0': { '--ide-peer-color': 'var(--cat-1)' }, '.cm-ide-peer-slot-1': { '--ide-peer-color': 'var(--cat-2)' },
  '.cm-ide-peer-slot-2': { '--ide-peer-color': 'var(--cat-3)' }, '.cm-ide-peer-slot-3': { '--ide-peer-color': 'var(--cat-4)' },
  '.cm-ide-peer-slot-4': { '--ide-peer-color': 'var(--cat-5)' }, '.cm-ide-peer-slot-5': { '--ide-peer-color': 'var(--cat-6)' },
  '.cm-gutters, .cm-ide-bp-gutter, .cm-lineNumbers': { cursor: 'pointer', userSelect: 'none' },
  '.cm-ide-bp-gutter': { minWidth: '22px' },
  '.cm-ide-bp-gutter .cm-gutterElement': { display: 'flex', alignItems: 'center', justifyContent: 'center', paddingInline: '2px' },
  '.cm-ide-bp-cell': { display: 'inline-flex', alignItems: 'center', gap: '2px' },
  '.cm-ide-bp-spacer': { visibility: 'hidden', display: 'inline-flex' },
  '.cm-ide-bp-marker': { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: '12px', height: '12px', lineHeight: '1', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-preview': { opacity: '0.45', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-muted': { color: 'var(--text-dim)' },
  '.cm-ide-bp-marker.cm-ide-bp-disabled': { color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-logpoint, .cm-ide-bp-marker.cm-ide-bp-paused': { color: 'var(--warning)' },
  '.cm-ide-bp-line': { backgroundColor: 'color-mix(in srgb, var(--danger) 9%, transparent)' },
  '.cm-ide-gutter-run-btn, .cm-ide-lightbulb-btn': { appearance: 'none', border: 'none', background: 'transparent', color: 'var(--success)', fontSize: 'var(--fs-xs)', lineHeight: '1', padding: '0', width: '15px', height: '15px', borderRadius: 'var(--radius-s)', cursor: 'pointer', display: 'inline-flex', alignItems: 'center', justifyContent: 'center' },
  '.cm-ide-lightbulb-btn': { color: 'var(--warning)' },
  '.cm-ide-gutter-run-btn:hover': { background: 'var(--success-soft)' },
  '.cm-ide-lightbulb-btn:hover': { background: 'var(--warning-soft)' },
  '.cm-ide-paused-line': { backgroundColor: 'var(--warning-soft)', boxShadow: 'inset 2px 0 0 var(--warning)' },
  '.cm-ide-inline-values': { color: 'var(--text-dim)', fontStyle: 'italic', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-xs)', userSelect: 'none', pointerEvents: 'none', whiteSpace: 'pre' },
  '.cm-ide-shell-var-hit': { backgroundColor: 'var(--accent-soft)', borderRadius: '2px' },
  '.cm-ide-peer-sel': { backgroundColor: 'color-mix(in srgb, var(--ide-peer-color) 22%, transparent)', borderRadius: '2px' },
  '.cm-ide-peer-caret, .cm-ide-peer-eol': { position: 'relative', display: 'inline-block', width: '0', height: '1.15em', verticalAlign: 'text-bottom', overflow: 'visible', pointerEvents: 'none', zIndex: 'var(--z-sticky)' },
  '.cm-ide-peer-caret::before': { content: '""', position: 'absolute', insetBlock: '0', insetInlineStart: '-1px', width: '2px', backgroundColor: 'var(--ide-peer-color)' },
  '.cm-ide-peer-pill': { position: 'absolute', insetBlockEnd: '100%', insetInlineStart: '-1px', padding: '0 4px', borderRadius: 'var(--radius-s)', backgroundColor: 'var(--surface-2)', border: '1px solid var(--ide-peer-color)', color: 'var(--text)', fontFamily: 'var(--font-ui)', fontSize: 'var(--fs-xs)', fontWeight: '500', lineHeight: '1.25', whiteSpace: 'nowrap', pointerEvents: 'none', zIndex: 'var(--z-sticky)' },
  '.cm-ide-peer-pill.is-eol': { insetBlockStart: '0', insetBlockEnd: 'auto', insetInlineStart: 'var(--sp-3)' },
});

export function buildIdeKeymap(preset: 'jetbrains' | 'standard', getCallbacks: () => IdeEditorCallbacks, filePath = '', language?: string): Extension {
  const applyFirstFix = (v: EditorView): boolean => {
    const i = wordAtCursor(v.state), lFixes = getIdeQuickFixesAtLine(v.state, i.line);
    if (lFixes.length) { applyClientQuickFix(v, lFixes[0]); return true; }
    const dFixes = computeDeterministicQuickFixes(v.state.doc.toString(), filePath, language, i.line, i.word || undefined);
    if (dFixes.length) return applyConcreteEditsToView(v, dFixes[0].edits, filePath);
    const cb = getCallbacks().onQuickFix; if (cb) { cb({ line: i.line, column: i.column, symbol: i.word || undefined }); return true; }
    return false;
  };
  const bindings: KeyBinding[] = [
    { key: 'Mod-Alt-ArrowUp', run: addCursorAbove }, { key: 'Mod-Alt-ArrowDown', run: addCursorBelow },
    { key: 'Alt-Shift-ArrowUp', run: moveLineUp }, { key: 'Alt-Shift-ArrowDown', run: moveLineDown },
    { key: 'Ctrl-ArrowUp', run: (v) => { const cb = getCallbacks().onJumpSymbol; if (cb) { cb(-1); return true; } return moveLineUp(v); } },
    { key: 'Ctrl-ArrowDown', run: (v) => { const cb = getCallbacks().onJumpSymbol; if (cb) { cb(1); return true; } return moveLineDown(v); } },
    { key: 'Alt-ArrowUp', run: smartExpandSelection }, { key: 'Alt-ArrowDown', run: simplifySelection },
    { key: 'Shift-Enter', run: startNewLineBelow },
    { key: 'Ctrl-g', run: selectNextOccurrence }, { key: 'Ctrl-Mod-g', run: selectAllOccurrences },
    { key: 'Mod-g', run: (v) => { const cb = getCallbacks().onGoToLine; if (cb) { cb(); return true; } return cmGotoLine(v); } },
    { key: 'Alt-Shift-i', run: splitSelectionIntoLines }, { key: 'Alt-Shift-I', run: splitSelectionIntoLines },
    { key: 'Mod-/', run: toggleLineComment }, { key: 'Mod-Alt-/', run: toggleBlockComment }, { key: 'Mod-Shift-/', run: toggleBlockComment },
    { key: 'Mod-r', run: openReplaceInFile },
    { key: 'Mod-Shift-v', run: () => { const cb = getCallbacks().onPasteFromHistory; if (cb) { cb(); return true; } if (typeof window !== 'undefined') { window.dispatchEvent(new CustomEvent('otto:ide-paste-history')); return true; } return false; } },
    { key: 'F12', run: (v) => { const cbs = getCallbacks(); if (!cbs.onDefinitionJump) return false; void requestIdeDiveAtPos(v, v.state.selection.main.head, 'definition', cbs); return true; } },
    { key: 'Mod-b', run: (v) => { const cbs = getCallbacks(); if (!cbs.onDefinitionJump) return false; void requestIdeDiveAtPos(v, v.state.selection.main.head, 'definition', cbs); return true; } },
    { key: 'Mod-Alt-b', run: (v) => { const cbs = getCallbacks(); if (!cbs.onImplementationJump && !cbs.onDefinitionJump) return false; void requestIdeDiveAtPos(v, v.state.selection.main.head, 'implementation', cbs); return true; } },
    { key: 'Alt-F7', run: (v) => { const cb = getCallbacks().onFindUsages; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word || undefined }); return true; } },
    { key: 'Mod-F12', run: () => { const cb = getCallbacks().onFileStructure; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-Alt-ArrowLeft', run: () => { const cb = getCallbacks().onNavigateBack; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-Alt-ArrowRight', run: () => { const cb = getCallbacks().onNavigateForward; if (!cb) return false; cb(); return true; } },
    { key: 'Alt-Enter', run: (v) => { const cb = getCallbacks().onQuickFix; if (!cb) return applyFirstFix(v); const i = wordAtCursor(v.state), c = v.coordsAtPos(v.state.selection.main.head); cb({ line: i.line, column: i.column, symbol: i.word || undefined, clientX: c?.left, clientY: c?.bottom }); return true; } },
    { key: 'Alt-Shift-Enter', run: applyFirstFix },
    { key: 'Shift-F6', run: (v) => { const cb = getCallbacks().onRenameSymbol; if (!cb) return false; const i = wordAtCursor(v.state); cb({ line: i.line, column: i.column, symbol: i.word }); return true; } },
    { key: 'Mod-Alt-l', run: () => { const cb = getCallbacks().onFormat; if (!cb) return false; cb(); return true; } },
    { key: 'Mod-F8', run: (v) => { const cb = getCallbacks().onToggleBreakpoint; if (!cb) return false; cb(v.state.doc.lineAt(v.state.selection.main.head).number); return true; } },
    ...(preset === 'jetbrains'
      ? [{ key: 'Mod-[', run: () => { const cb = getCallbacks().onNavigateBack; if (!cb) return false; cb(); return true; } }, { key: 'Mod-]', run: () => { const cb = getCallbacks().onNavigateForward; if (!cb) return false; cb(); return true; } }, { key: 'Mod-d', run: duplicateLineOrSelection }, { key: 'Mod-Backspace', run: deleteLine }]
      : [{ key: 'Mod-d', run: selectNextOccurrence }, { key: 'Shift-Mod-d', run: duplicateLineOrSelection }, { key: 'Shift-Mod-k', run: deleteLine }]),
  ];
  return Prec.high(keymap.of(bindings));
}

export function createIdeExtensions(opts: CreateIdeExtensionsOptions): Extension {
  const debugField = StateField.define<{ payload: IdeDebugStatePayload; decos: DecorationSet }>({
    create(st) { const payload = { pausedLine: opts.pausedLine, inlineValues: opts.inlineValues }; return { payload, decos: computeDebugDecorations(st.doc, payload) }; },
    update(cur, tr) { let payload = cur.payload, changed = false; for (const ef of tr.effects) if (ef.is(setDebugDecorationsEffect)) { payload = ef.value; changed = true; } return changed || tr.docChanged ? { payload, decos: computeDebugDecorations(tr.state.doc, payload) } : cur; },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
  const peersField = StateField.define<{ peers: PeerCursorInput[]; decos: DecorationSet }>({
    create(st) { return { peers: opts.peerCursors, decos: computePeerDecorations(st.doc, opts.peerCursors) }; },
    update(cur, tr) { let peers = cur.peers, changed = false; for (const ef of tr.effects) if (ef.is(setPeerCursorsEffect)) { peers = ef.value; changed = true; } return changed || tr.docChanged ? { peers, decos: computePeerDecorations(tr.state.doc, peers) } : cur; },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  const fPath = opts.filePath ?? '', lower = fPath.toLowerCase().split('?')[0];
  const isCsv = lower.endsWith('.csv') || lower.endsWith('.tsv');
  const isShell = lower.endsWith('.sh') || lower.endsWith('.bash') || lower.endsWith('.zsh');
  const isManifest = isIdeManifestPath(fPath);

  const fullContextMenu = isManifest ? [] : Prec.highest(EditorView.domEventHandlers({
    contextmenu(event, view) {
      if (event.target instanceof Element && event.target.closest('.cm-gutters')) return false;
      const cbs = opts.getCallbacks();
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY }), main = view.state.selection.main;
      if (pos != null && (main.empty || pos < main.from || pos > main.to)) view.dispatch({ selection: { anchor: pos, head: pos } });
      const sym = wordAtCursor(view.state);
      const showMain = () => {
        const items: MenuItem[] = [];
        if (cbs.onQuickFix) items.push({ label: 'Show Context Actions', icon: 'bulb', hint: '⌥⏎', action: () => cbs.onQuickFix?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined, clientX: event.clientX, clientY: event.clientY }) });
        if (cbs.onAgentAction) {
          items.push(
            { label: 'Add Selection to Agent', icon: 'assistant', hint: '⌘L', action: () => cbs.onAgentAction?.('chat', selectionOrCurrentLine(view.state)) },
            { label: 'Inline Edit with Agent', icon: 'sparkle', hint: '⌘I', disabled: view.state.readOnly, action: () => cbs.onAgentAction?.('inline', selectionOrCurrentLine(view.state)) },
          );
        }
        items.push({ separator: true });
        if (cbs.onDefinitionJump) items.push({ label: 'Go to Definition', icon: 'arrow', hint: '⌘B', action: () => cbs.onDefinitionJump?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
        if (cbs.onImplementationJump) items.push({ label: 'Go to Implementation', icon: 'layers', hint: '⌥⌘B', action: () => cbs.onImplementationJump?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
        if (cbs.onFindUsages) items.push({ label: 'Find Usages', icon: 'search', hint: '⌥F7', action: () => cbs.onFindUsages?.({ line: sym.line, column: sym.column, symbol: sym.word || undefined }) });
        if (cbs.onFileStructure) items.push({ label: 'File Structure…', icon: 'book', hint: '⌘F12', action: () => cbs.onFileStructure?.() });
        if (cbs.onRenameSymbol && sym.word) items.push({ label: 'Rename Symbol…', icon: 'edit', hint: '⇧F6', disabled: view.state.readOnly, action: () => cbs.onRenameSymbol?.({ line: sym.line, column: sym.column, symbol: sym.word }) });
        if (cbs.onFormat) items.push({ label: 'Reformat Code', icon: 'format', hint: '⌥⌘L', disabled: view.state.readOnly, action: () => cbs.onFormat?.() });
        if (fPath) items.push(...buildIdeVcsContextRows(event, showMain, { filePath: fPath, readOnly: view.state.readOnly, onToggleBlame: cbs.onToggleBlame, onPasteFromHistory: cbs.onPasteFromHistory }));
        if (!items.length) return;
        const syn = new MouseEvent('contextmenu', { clientX: event.clientX, clientY: event.clientY, bubbles: true });
        ctxMenu.show(syn, items);
      };
      event.preventDefault();
      showMain();
      return true;
    },
  }));

  return [
    ideEditorTheme, debugField, peersField, fullContextMenu,
    createIdeSemanticExtensions({ filePath: fPath, language: opts.language, root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders, getPreferences: () => readIdeEditorPreferences(), onDiagnostics: (diags) => opts.getCallbacks().onDiagnostics?.(diags), onFindUsages: (ctx) => opts.getCallbacks().onFindUsages?.(ctx) }),
    createIdeHoverExtension({ filePath: fPath, language: opts.language, root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders, getPreferences: () => readIdeEditorPreferences(), onDefinitionJump: (t) => opts.getCallbacks().onDefinitionJump?.(t), onFindUsages: (ctx) => opts.getCallbacks().onFindUsages?.(ctx), onQuickFix: (ctx) => opts.getCallbacks().onQuickFix?.(ctx) }),
    createCmdHoverDiveExtension(opts.getCallbacks),
    createIdeGitAndAgentExtensions({ savedContent: opts.savedContent, blameByLine: opts.blameByLine, wordAtCursor, getCallbacks: opts.getCallbacks }),
    ...(isCsv ? [createIdeCsvExtension(fPath)] : []),
    ...(isShell ? [createShellVarHighlightExtension()] : []),
    buildIdeKeymap(opts.keymapPreset, opts.getCallbacks, fPath, opts.language),
  ];
}
