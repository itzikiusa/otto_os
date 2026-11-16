// Pro IDE extensions for CodeEditor (JetBrains GoLand / IntelliJ parity):
// 1-click gutter breakpoints right beside line numbers (including read-only SDK files),
// combined breakpoint/run/lightbulb gutter, semantic coloring + rainbow brackets + quick-doc hover,
// full JetBrains macOS keymap (`cm-ide-keymap.ts`), and `Git ▸` / `Local History ▸` menus.

import {
  Decoration, EditorView, GutterMarker, ViewPlugin, WidgetType,
  crosshairCursor, gutter, rectangularSelection,
  type DecorationSet, type ViewUpdate,
} from '@codemirror/view';
import {
  EditorState, RangeSet, StateEffect, StateField,
  type Extension, type Range,
} from '@codemirror/state';
import { addCursorAbove, addCursorBelow, moveLineDown, moveLineUp } from '@codemirror/commands';
import { selectNextOccurrence } from '@codemirror/search';
import type {
  IdeBreakpoint, IdeCollabPeer, IdeDiagnostic, IdeGutterItem, IdeInlineValue, IdePeerCursor,
} from '../api/types.ts';
import { ctxMenu } from '../contextmenu.svelte.ts';
import { isIconName } from './Icon.svelte';
import { ideCompactExtension } from './cm-ide-compact.ts';
import { createIdeCsvExtension } from './cm-ide-csv.ts';
import {
  createIdeGitAndAgentExtensions, selectionOrCurrentLine, setBaselineSourceEffect, setBlameLinesEffect, setSavedContentEffect,
  type IdeDiffBaselineSource, type IdeEditorSelectionRange,
} from './cm-ide-git-and-agent.ts';
import { isBpTargetGutterElement } from './cm-ide-history.ts';
import { createIdeHoverExtension } from './cm-ide-hover.ts';
import {
  applyLinePrefixSuffix, buildIdeKeymap as buildIdeKeymapCore, buildIdeKeymapSpecs,
  duplicateLineOrSelection, openReplaceInFile, selectAllOccurrences,
  smartExpandSelection, splitSelectionIntoLines, startNewLineBelow,
  toggleLineComment, wordAtCursor, type IdeEditorCallbacks,
} from './cm-ide-keymap.ts';
import {
  buildPooledLspExtensions, createCmdHoverDiveExtension, createDirectLspExtension,
  disposePooledLspClients, hasWarmPooledLspClient, lspUriToPath, notifyLspDidSave, queryPooledLspRename, requestIdeDiveAtPos,
  type AttachPooledLspOptions, type DirectLspOptions,
} from './cm-ide-lsp.ts';
import {
  applyConcreteEditsToView, computeDeterministicQuickFixes, hasDeterministicQuickFixAtLine, previewConcreteEdits,
  type IdeConcreteTextEdit, type IdeQuickFixAction,
} from './cm-ide-quickfix.ts';
import {
  applyClientQuickFix, createIdeSemanticExtensions, getIdeQuickFixesAtLine,
  type ClientDiagQuickFix,
} from './cm-ide-semantic.ts';
import {
  createIdeRefactorExtensions, detectInlineRefactorHints, setDiskSavedContentEffect,
  type IdeInlineRefactorHint,
} from './cm-ide-refactor.ts';
import { readIdeEditorPreferences } from './cm-ide-theme.ts';
import { isIdeManifestPath } from './cm-ide-manifest.ts';

export {
  addCursorAbove, addCursorBelow, applyClientQuickFix, applyConcreteEditsToView,
  applyLinePrefixSuffix, buildIdeKeymapSpecs, buildPooledLspExtensions,
  computeDeterministicQuickFixes, createDirectLspExtension, crosshairCursor,
  detectInlineRefactorHints, disposePooledLspClients, duplicateLineOrSelection,
  getIdeQuickFixesAtLine, hasWarmPooledLspClient,
  isBpTargetGutterElement, lspUriToPath, moveLineDown, moveLineUp, notifyLspDidSave,
  openReplaceInFile, previewConcreteEdits, queryPooledLspRename, rectangularSelection, requestIdeDiveAtPos,
  selectAllOccurrences, selectNextOccurrence, selectionOrCurrentLine,
  setBaselineSourceEffect, setBlameLinesEffect, setDiskSavedContentEffect, setSavedContentEffect, smartExpandSelection,
  splitSelectionIntoLines, startNewLineBelow, toggleLineComment, wordAtCursor,
  type AttachPooledLspOptions, type ClientDiagQuickFix, type DirectLspOptions,
  type IdeConcreteTextEdit, type IdeDiffBaselineSource, type IdeEditorCallbacks, type IdeEditorSelectionRange,
  type IdeInlineRefactorHint, type IdeQuickFixAction,
};

export interface IdeDebugStatePayload { pausedLine: number | null; inlineValues: IdeInlineValue[]; }
export type BreakpointsEffectPayload = IdeBreakpoint[] | { breakpoints: IdeBreakpoint[]; muted?: boolean };
export type PeerCursorInput = IdePeerCursor | IdeCollabPeer;

export const setBreakpointsEffect = StateEffect.define<BreakpointsEffectPayload>();
export const setGutterItemsEffect = StateEffect.define<IdeGutterItem[]>();
export const setDebugDecorationsEffect = StateEffect.define<IdeDebugStatePayload>();
export const setPeerCursorsEffect = StateEffect.define<PeerCursorInput[]>();
const setHoverBpLineEffect = StateEffect.define<number | null>();

const ideRunItemsField = StateField.define<Map<number, IdeGutterItem>>({
  create() { return new Map(); },
  update(m, tr) {
    for (const ef of tr.effects) {
      if (ef.is(setGutterItemsEffect)) {
        const n = new Map<number, IdeGutterItem>();
        for (const it of ef.value) n.set(it.line, it);
        return n;
      }
    }
    if (tr.docChanged && m.size > 0) {
      const next = new Map<number, IdeGutterItem>();
      let moved = false;
      for (const [ln, it] of m) {
        if (ln < 1 || ln > tr.startState.doc.lines) continue;
        const mappedPos = tr.changes.mapPos(tr.startState.doc.line(ln).from, 1);
        const newLn = tr.state.doc.lineAt(Math.min(mappedPos, tr.state.doc.length)).number;
        if (newLn !== ln) moved = true;
        if (!next.has(newLn)) next.set(newLn, newLn === ln ? it : { ...it, line: newLn });
      }
      if (moved || next.size !== m.size) return next;
    }
    return m;
  },
});

type BpVisualKind = 'enabled' | 'disabled' | 'muted' | 'conditional' | 'logpoint' | 'paused' | 'preview';
const SVG_NS = 'http://www.w3.org/2000/svg';

function bulbSvg(): SVGSVGElement {
  const svg = document.createElementNS(SVG_NS, 'svg');
  for (const [k, v] of [['width', '12'], ['height', '12'], ['viewBox', '0 0 16 16'], ['fill', 'none'], ['aria-hidden', 'true']]) {
    svg.setAttribute(k, v);
  }
  const p = document.createElementNS(SVG_NS, 'path');
  p.setAttribute('d', 'M8 2a4.2 4.2 0 0 0-2.6 7.5c.4.35.6.8.6 1.3V11.5h4v-.7c0-.5.2-.95.6-1.3A4.2 4.2 0 0 0 8 2Zm-1.5 11.2h3M7 14.6h2');
  p.setAttribute('stroke', 'currentColor');
  p.setAttribute('stroke-width', '1.4');
  p.setAttribute('stroke-linecap', 'round');
  p.setAttribute('stroke-linejoin', 'round');
  svg.appendChild(p);
  return svg;
}

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
    const bpSlot = document.createElement('span'); bpSlot.className = 'cm-ide-bp-slot';
    if (this.bpKind) {
      const el = document.createElement('span');
      const isDirectPaused = this.bpKind === 'paused';
      el.className = `cm-ide-bp-marker cm-ide-bp-${this.bpKind}${this.alsoPaused ? ' is-paused' : ''}${isDirectPaused ? ' cm-ide-exec-arrow' : ''}${this.verified ? ' is-verified' : ''}`;
      el.dataset.bpKind = this.bpKind;
      el.dataset.testid = this.bpKind === 'preview'
        ? 'ide-bp-hover-preview'
        : isDirectPaused
          ? 'ide-exec-gutter-arrow'
          : 'ide-bp-marker';
      if (this.bpKind !== 'preview') { el.title = this.tooltip; el.setAttribute('role', 'img'); el.setAttribute('aria-label', this.tooltip); }
      else el.setAttribute('aria-hidden', 'true');
      const shape = isDirectPaused ? 'play' : this.bpKind === 'logpoint' ? 'diamond' : (this.bpKind === 'disabled' || this.bpKind === 'muted') ? 'hollow' : 'enabled';
      el.appendChild(gutterSvg(shape, this.verified, this.bpKind === 'conditional'));
      bpSlot.appendChild(el);
      if (this.alsoPaused && !isDirectPaused) {
        const execBadge = document.createElement('span');
        execBadge.className = 'cm-ide-exec-arrow is-combo';
        execBadge.dataset.testid = 'ide-exec-gutter-arrow';
        execBadge.title = `Execution paused at line ${this.line}`;
        execBadge.setAttribute('role', 'img');
        execBadge.setAttribute('aria-label', `Execution paused at line ${this.line}`);
        execBadge.appendChild(gutterSvg('play'));
        bpSlot.appendChild(execBadge);
      }
    }
    wrap.appendChild(bpSlot);
    const actSlot = document.createElement('span'); actSlot.className = 'cm-ide-act-slot';
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
          ...(item.kind === 'test'
            ? [{ label: `Run '${item.label}' (keep containers)`, icon: 'play' as const, action: () => cbs.onGutterAction?.(item, 'run_keep_tc') }]
            : []),
          { label: `Debug '${item.label}'`, icon: isIconName('bug') ? 'bug' : 'target', action: () => cbs.onGutterAction?.(item, 'debug') },
        ]);
      });
      actSlot.appendChild(btn);
    } else if (this.showBulb) {
      const bulb = document.createElement('button');
      bulb.type = 'button'; bulb.className = 'cm-ide-lightbulb-btn'; bulb.dataset.testid = 'ide-gutter-lightbulb';
      const label = 'Show Quick Fixes (⌥⏎)';
      bulb.title = label; bulb.setAttribute('aria-label', label);
      bulb.appendChild(bulbSvg());
      bulb.addEventListener('mousedown', (e) => { e.preventDefault(); e.stopPropagation(); });
      bulb.addEventListener('click', (e) => {
        e.preventDefault(); e.stopPropagation();
        const r = bulb.getBoundingClientRect();
        this.getCallbacks().onQuickFix?.({ line: this.line, column: 1, clientX: r.left, clientY: r.bottom + 4 });
      });
      actSlot.appendChild(bulb);
    }
    wrap.appendChild(actSlot);
    return wrap;
  }
}

const bpSpacer = new (class extends GutterMarker {
  toDOM(): HTMLElement {
    const s = document.createElement('span');
    s.className = 'cm-ide-bp-cell cm-ide-bp-spacer';
    s.setAttribute('aria-hidden', 'true');
    const bpSlot = document.createElement('span'); bpSlot.className = 'cm-ide-bp-slot';
    const actSlot = document.createElement('span'); actSlot.className = 'cm-ide-act-slot';
    s.appendChild(bpSlot); s.appendChild(actSlot);
    return s;
  }
})();

export interface CreateIdeExtensionsOptions {
  filePath?: string; language?: string; root?: string; connectionId?: string | null; projectFolders?: string[];
  breakpoints: IdeBreakpoint[]; breakpointsMuted?: boolean; gutterItems: IdeGutterItem[];
  pausedLine: number | null; inlineValues: IdeInlineValue[]; peerCursors: PeerCursorInput[];
  diskSavedContent?: string | null; savedContent?: string | null; baselineSource?: IdeDiffBaselineSource;
  blameByLine?: Record<number, string> | null;
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
      let bpSet = false;
      for (const ef of tr.effects) {
        if (ef.is(setBreakpointsEffect)) {
          const raw = ef.value, list = Array.isArray(raw) ? raw : raw.breakpoints;
          if (!Array.isArray(raw) && raw.muted !== undefined) muted = raw.muted;
          map = new Map(); for (const bp of list) map.set(bp.line, bp);
          bpSet = true;
        } else if (ef.is(setDebugDecorationsEffect)) pausedLine = ef.value.pausedLine;
        else if (ef.is(setHoverBpLineEffect)) hoverLine = ef.value;
      }
      if (tr.docChanged) {
        if (!bpSet && map.size > 0) {
          const nextMap = new Map<number, IdeBreakpoint>();
          let moved = false;
          for (const [ln, bp] of map) {
            if (ln < 1 || ln > tr.startState.doc.lines) continue;
            const mappedPos = tr.changes.mapPos(tr.startState.doc.line(ln).from, 1);
            const newLn = tr.state.doc.lineAt(Math.min(mappedPos, tr.state.doc.length)).number;
            if (newLn !== ln) moved = true;
            if (!nextMap.has(newLn)) nextMap.set(newLn, newLn === ln ? bp : { ...bp, line: newLn });
          }
          if (moved || nextMap.size !== map.size) map = nextMap;
        }
        if (hoverLine != null && hoverLine > tr.state.doc.lines) hoverLine = null;
      }
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

  const runFieldInit = ideRunItemsField.init(() => {
    const m = new Map<number, IdeGutterItem>();
    for (const it of opts.gutterItems) m.set(it.line, it);
    return m;
  });

  const handleBpClick = (_v: EditorView, ln: number, me: MouseEvent): boolean => {
    const cbs = opts.getCallbacks();
    if (me.shiftKey && cbs.onEditBreakpoint) { me.preventDefault(); me.stopPropagation(); cbs.onEditBreakpoint(ln, me); return true; }
    if (cbs.onToggleBreakpoint) { me.preventDefault(); me.stopPropagation(); cbs.onToggleBreakpoint(ln); return true; }
    return false;
  };
  const handleBpCtx = (v: EditorView, ln: number, me: MouseEvent): boolean => {
    const cbs = opts.getCallbacks();
    if (!cbs.onToggleBreakpoint && !cbs.onEditBreakpoint) return false;
    const bp = v.state.field(bpField).map.get(ln);
    const el = me.target instanceof Element ? me.target : null;
    if (el?.closest('.cm-lineNumbers') && !bp) return false;
    me.preventDefault(); me.stopPropagation();
    if (cbs.onEditBreakpoint) { cbs.onEditBreakpoint(ln, me); return true; }
    ctxMenu.show(me, [{ label: bp ? 'Remove breakpoint' : 'Add breakpoint', icon: bp ? 'trash' : 'plus', action: () => cbs.onToggleBreakpoint?.(ln) }]);
    return true;
  };

  const bpAndRunGutter = gutter({
    class: 'cm-ide-bp-gutter',
    markers(view) {
      const { map, muted, pausedLine, hoverLine } = view.state.field(bpField);
      const runMap = view.state.field(ideRunItemsField);
      const curLn = view.state.doc.lineAt(view.state.selection.main.head).number;
      const hasBulbOnCur =
        !view.state.readOnly &&
        (getIdeQuickFixesAtLine(view.state, curLn).length > 0 ||
          hasDeterministicQuickFixAtLine(view.state.doc, opts.filePath ?? '', opts.language, curLn));
      const lines = new Set<number>([...map.keys(), ...runMap.keys()]);
      if (pausedLine != null) lines.add(pausedLine);
      if (hoverLine != null) lines.add(hoverLine);
      if (hasBulbOnCur) lines.add(curLn);
      if (!lines.size) return RangeSet.empty;
      const markers: Range<GutterMarker>[] = [];
      for (const ln of [...lines].sort((a, b) => a - b)) {
        if (ln < 1 || ln > view.state.doc.lines) continue;
        const bp = map.get(ln) as (IdeBreakpoint & { verified?: boolean }) | undefined;
        const runItem = runMap.get(ln) ?? null;
        const isPaused = pausedLine === ln;
        let bpKind: BpVisualKind | null = null;
        let tip = '';
        let verified = false;
        if (bp) {
          bpKind = muted ? 'muted' : !bp.enabled ? 'disabled' : bp.log_message ? 'logpoint' : (bp.condition || bp.hit_condition) ? 'conditional' : 'enabled';
          tip = muted
            ? `Muted breakpoint at line ${ln}`
            : bp.log_message
              ? `Logpoint at line ${ln}: ${bp.log_message}`
              : bp.condition
                ? `Conditional breakpoint at line ${ln}: ${bp.condition}`
                : !bp.enabled
                  ? `Disabled breakpoint at line ${ln}`
                  : `${bp.verified ? 'Verified b' : 'B'}reakpoint at line ${ln}`;
          verified = Boolean((bp.verified || pausedLine != null) && !muted && bp.enabled);
        } else if (isPaused) {
          bpKind = 'paused';
          tip = `Paused at line ${ln}`;
        } else if (hoverLine === ln) {
          bpKind = 'preview';
        }
        markers.push(
          new CombinedGutterMarker({
            bpKind,
            tooltip: tip,
            alsoPaused: isPaused,
            verified,
            runItem,
            showBulb: hasBulbOnCur && ln === curLn && !runItem,
            line: ln,
            getCallbacks: opts.getCallbacks,
          }).range(view.state.doc.line(ln).from),
        );
      }
      return RangeSet.of(markers, true);
    },
    initialSpacer: () => bpSpacer,
  });

  const gutterHandlers = ViewPlugin.fromClass(class {
    private dom: HTMLElement;
    private onMove: (ev: MouseEvent) => void;
    private onLeave: () => void;
    private onDown: (ev: MouseEvent) => void;
    private onCtx: (ev: MouseEvent) => void;
    constructor(view: EditorView) {
      this.dom = view.scrollDOM;
      const lineAtY = (clientY: number) =>
        view.state.doc.lineAt(view.lineBlockAtHeight(clientY - view.documentTop).from).number;
      this.onMove = (ev) => {
        const st = view.state.field(bpField, false);
        if (!st) return;
        if (!opts.getCallbacks().onToggleBreakpoint || !isBpTargetGutterElement(ev.target)) {
          if (st.hoverLine !== null) view.dispatch({ effects: setHoverBpLineEffect.of(null) });
          return;
        }
        const ln = lineAtY(ev.clientY);
        if (st.hoverLine !== ln) view.dispatch({ effects: setHoverBpLineEffect.of(ln) });
      };
      this.onLeave = () => {
        const st = view.state.field(bpField, false);
        if (st && st.hoverLine !== null) view.dispatch({ effects: setHoverBpLineEffect.of(null) });
      };
      this.onDown = (ev) => {
        if (ev.button !== 0 || ev.altKey || ev.metaKey || ev.ctrlKey || !isBpTargetGutterElement(ev.target)) return;
        handleBpClick(view, lineAtY(ev.clientY), ev);
      };
      this.onCtx = (ev) => {
        if (!isBpTargetGutterElement(ev.target)) return;
        handleBpCtx(view, lineAtY(ev.clientY), ev);
      };
      this.dom.addEventListener('mousemove', this.onMove);
      this.dom.addEventListener('mouseleave', this.onLeave);
      this.dom.addEventListener('mousedown', this.onDown, true);
      this.dom.addEventListener('contextmenu', this.onCtx, true);
    }
    destroy(): void {
      this.dom.removeEventListener('mousemove', this.onMove);
      this.dom.removeEventListener('mouseleave', this.onLeave);
      this.dom.removeEventListener('mousedown', this.onDown, true);
      this.dom.removeEventListener('contextmenu', this.onCtx, true);
    }
  });

  return [bpField, runFieldInit, bpAndRunGutter, gutterHandlers];
}

class InlineGhostValuesWidget extends WidgetType {
  summary: string;
  constructor(summary: string) { super(); this.summary = summary; }
  eq(o: InlineGhostValuesWidget): boolean { return this.summary === o.summary; }
  toDOM(): HTMLElement {
    const s = document.createElement('span');
    s.className = 'cm-ide-inline-values';
    s.setAttribute('aria-hidden', 'true');
    s.textContent = `  // ${this.summary}`;
    return s;
  }
}

function computeDebugDecorations(doc: EditorState['doc'], payload: IdeDebugStatePayload): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  if (payload.pausedLine != null && payload.pausedLine >= 1 && payload.pausedLine <= doc.lines) {
    ranges.push(
      Decoration.line({ class: 'cm-ide-exec-line cm-ide-paused-line' }).range(
        doc.line(payload.pausedLine).from,
      ),
    );
  }
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
    const w = document.createElement('span');
    w.className = `${this.eol ? 'cm-ide-peer-eol' : 'cm-ide-peer-caret'} cm-ide-peer-slot-${this.slot}`;
    w.setAttribute('aria-hidden', 'true');
    const p = document.createElement('span');
    p.className = `cm-ide-peer-pill${this.eol ? ' is-eol' : ''}`;
    p.textContent = this.name;
    w.appendChild(p);
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
  '.cm-ide-bp-gutter': { inlineSize: '34px', minInlineSize: '34px', maxInlineSize: '34px', boxSizing: 'border-box', overflow: 'hidden' },
  '.cm-ide-bp-gutter .cm-gutterElement': { inlineSize: '34px', minInlineSize: '34px', maxInlineSize: '34px', display: 'flex', alignItems: 'center', justifyContent: 'center', paddingInline: '2px 1px', boxSizing: 'border-box', overflow: 'hidden' },
  '.cm-ide-bp-cell': { inlineSize: '31px', minInlineSize: '31px', maxInlineSize: '31px', display: 'inline-grid', gridTemplateColumns: '14px 15px', columnGap: '2px', alignItems: 'center', justifyItems: 'center' },
  '.cm-ide-bp-slot': { position: 'relative', inlineSize: '14px', blockSize: '15px', display: 'inline-flex', alignItems: 'center', justifyContent: 'center' },
  '.cm-ide-act-slot': { inlineSize: '15px', blockSize: '15px', display: 'inline-flex', alignItems: 'center', justifyContent: 'center' },
  '.cm-ide-bp-spacer': { visibility: 'hidden' },
  '.cm-ide-bp-marker': { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: '12px', height: '12px', lineHeight: '1', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-preview': { opacity: '0.45', color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-muted': { color: 'var(--text-dim)' },
  '.cm-ide-bp-marker.cm-ide-bp-disabled': { color: 'var(--danger)' },
  '.cm-ide-bp-marker.cm-ide-bp-logpoint, .cm-ide-bp-marker.cm-ide-bp-paused': { color: 'var(--warning)' },
  '.cm-ide-exec-arrow': { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: '12px', height: '12px', lineHeight: '1', color: 'var(--warning)', filter: 'drop-shadow(0 0 1px var(--bg))' },
  '.cm-ide-exec-arrow.is-combo': { position: 'absolute', insetInlineEnd: '-2px', insetBlockEnd: '-1px', width: '9px', height: '9px', pointerEvents: 'none' },
  '.cm-ide-bp-line': { backgroundColor: 'color-mix(in srgb, var(--danger) 9%, transparent)' },
  '.cm-ide-gutter-run-btn, .cm-ide-lightbulb-btn': { appearance: 'none', border: 'none', background: 'transparent', color: 'var(--success)', fontSize: 'var(--fs-xs)', lineHeight: '1', padding: '0', width: '15px', height: '15px', borderRadius: 'var(--radius-s)', cursor: 'pointer', display: 'inline-flex', alignItems: 'center', justifyContent: 'center' },
  '.cm-ide-lightbulb-btn': { color: 'var(--warning)' },
  '.cm-ide-gutter-run-btn:hover': { background: 'var(--success-soft)' },
  '.cm-ide-lightbulb-btn:hover': { background: 'var(--warning-soft)' },
  '.cm-line.cm-ide-exec-line, .cm-activeLine.cm-ide-exec-line, .cm-ide-paused-line': { backgroundColor: 'color-mix(in srgb, var(--accent-solid) 32%, transparent)', boxShadow: 'inset 3px 0 0 var(--accent-text)' },
  '.cm-ide-inline-values': { color: 'var(--text-dim)', fontStyle: 'italic', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-xs)', userSelect: 'none', pointerEvents: 'none', whiteSpace: 'pre' },
  '.cm-ide-shell-var-hit': { backgroundColor: 'var(--accent-soft)', borderRadius: '2px' },
  '.cm-ide-peer-sel': { backgroundColor: 'color-mix(in srgb, var(--ide-peer-color) 22%, transparent)', borderRadius: '2px' },
  '.cm-ide-peer-caret, .cm-ide-peer-eol': { position: 'relative', display: 'inline-block', width: '0', height: '1.15em', verticalAlign: 'text-bottom', overflow: 'visible', pointerEvents: 'none', zIndex: 'var(--z-sticky)' },
  '.cm-ide-peer-caret::before': { content: '""', position: 'absolute', insetBlock: '0', insetInlineStart: '-1px', width: '2px', backgroundColor: 'var(--ide-peer-color)' },
  '.cm-ide-peer-pill': { position: 'absolute', insetBlockEnd: '100%', insetInlineStart: '-1px', padding: '0 4px', borderRadius: 'var(--radius-s)', backgroundColor: 'var(--surface-2)', border: '1px solid var(--ide-peer-color)', color: 'var(--text)', fontFamily: 'var(--font-ui)', fontSize: 'var(--fs-xs)', fontWeight: '500', lineHeight: '1.25', whiteSpace: 'nowrap', pointerEvents: 'none', zIndex: 'var(--z-sticky)' },
  '.cm-ide-peer-pill.is-eol': { insetBlockStart: '0', insetBlockEnd: 'auto', insetInlineStart: 'var(--sp-3)' },
});

export function buildIdeKeymap(
  preset: 'jetbrains' | 'standard',
  getCallbacks: () => IdeEditorCallbacks,
  filePath = '',
  language?: string,
): Extension {
  return buildIdeKeymapCore(preset, getCallbacks, filePath, language, requestIdeDiveAtPos);
}

export function createIdeExtensions(opts: CreateIdeExtensionsOptions): Extension {
  const debugField = StateField.define<{ payload: IdeDebugStatePayload; decos: DecorationSet }>({
    create(st) {
      const payload = { pausedLine: opts.pausedLine, inlineValues: opts.inlineValues };
      return { payload, decos: computeDebugDecorations(st.doc, payload) };
    },
    update(cur, tr) {
      let payload = cur.payload, changed = false;
      for (const ef of tr.effects) if (ef.is(setDebugDecorationsEffect)) { payload = ef.value; changed = true; }
      return changed || tr.docChanged ? { payload, decos: computeDebugDecorations(tr.state.doc, payload) } : cur;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
  const peersField = StateField.define<{ peers: PeerCursorInput[]; decos: DecorationSet }>({
    create(st) { return { peers: opts.peerCursors, decos: computePeerDecorations(st.doc, opts.peerCursors) }; },
    update(cur, tr) {
      let peers = cur.peers, changed = false;
      for (const ef of tr.effects) if (ef.is(setPeerCursorsEffect)) { peers = ef.value; changed = true; }
      return changed || tr.docChanged ? { peers, decos: computePeerDecorations(tr.state.doc, peers) } : cur;
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });

  const fPath = opts.filePath ?? '', lower = fPath.toLowerCase().split('?')[0];
  const isCsv = lower.endsWith('.csv') || lower.endsWith('.tsv');
  const isShell = lower.endsWith('.sh') || lower.endsWith('.bash') || lower.endsWith('.zsh');
  const isManifest = isIdeManifestPath(fPath);

  return [
    ideEditorTheme, debugField, peersField, ideCompactExtension(),
    createIdeRefactorExtensions({
      filePath: fPath,
      language: opts.language,
      diskSavedContent: opts.diskSavedContent,
      savedContent: opts.savedContent,
      isManifest,
      getCallbacks: opts.getCallbacks,
      getRunMap: (view) => view.state.field(ideRunItemsField, false),
    }),
    createIdeSemanticExtensions({
      filePath: fPath, language: opts.language, root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders,
      getPreferences: () => readIdeEditorPreferences(),
      onDiagnostics: (diags) => opts.getCallbacks().onDiagnostics?.(diags),
      onFindUsages: (ctx) => opts.getCallbacks().onFindUsages?.(ctx),
    }),
    createIdeHoverExtension({
      filePath: fPath, language: opts.language, root: opts.root, connectionId: opts.connectionId, projectFolders: opts.projectFolders,
      getPreferences: () => readIdeEditorPreferences(),
      onDefinitionJump: (t) => opts.getCallbacks().onDefinitionJump?.(t),
      onFindUsages: (ctx) => opts.getCallbacks().onFindUsages?.(ctx),
      onQuickFix: (ctx) => opts.getCallbacks().onQuickFix?.(ctx),
    }),
    createCmdHoverDiveExtension(opts.getCallbacks),
    createIdeGitAndAgentExtensions({
      savedContent: opts.savedContent,
      baselineSource: opts.baselineSource,
      blameByLine: opts.blameByLine,
      wordAtCursor,
      getCallbacks: opts.getCallbacks,
    }),
    ...(isCsv ? [createIdeCsvExtension(fPath)] : []),
    ...(isShell ? [createShellVarHighlightExtension()] : []),
    buildIdeKeymap(opts.keymapPreset, opts.getCallbacks, fPath, opts.language),
  ];
}
