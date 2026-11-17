// The IDE editor on a phone (part of the stacked layout's compact set, cm-ide-compact.ts):
//
// - The caret stays on screen while typing. iOS does not scroll the caret above the
//   on-screen keyboard here (lib/viewportFit.ts undoes its pan and shrinks the app to
//   the visible area), so when the editor's visible height changes — the keyboard opens,
//   the IDE chrome steps aside, the phone rotates — the line being edited is scrolled
//   back inside the visible band with a margin. Only a height change does this: the
//   person's own scrolling is never undone.
// - Soft-wrapped lines keep their indentation: a continuation line starts under the
//   line's own indent (+2 columns), so wrapped Python/Go still reads as code. Only the
//   visible lines are decorated, so a 30k-line file costs the same as a short one.
// - "Show documentation" by touch: hover tooltips are mouse-only, so the touch command
//   menu replays a hover at the caret.

import { RangeSetBuilder } from '@codemirror/state';
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate } from '@codemirror/view';
import { onKeyboard } from '../viewportFit.ts';

/** Margin kept between the caret line and the visible band's edges, in px. */
export function caretRevealMargin(visibleHeight: number): number {
  return Math.max(16, Math.min(64, Math.round(visibleHeight / 5)));
}

export interface Band {
  top: number;
  bottom: number;
}

/** The part of the editor's scroller the person can see: the scroller's box cut to the
 *  visual viewport (the keyboard covers whatever is below `vvHeight`). */
export function visibleBand(scroller: Band, vvTop: number, vvHeight: number): Band {
  if (!(vvHeight > 0)) return scroller;
  return { top: Math.max(scroller.top, vvTop), bottom: Math.min(scroller.bottom, vvTop + vvHeight) };
}

/** Is the caret line outside the band (or closer than `margin` to its bottom edge)?
 *  `caret` null = not rendered (scrolled far away). */
export function caretNeedsReveal(caret: Band | null, band: Band, margin: number): boolean {
  if (!caret) return true;
  if (band.bottom - band.top <= caret.bottom - caret.top) return false;
  const m = Math.min(margin, Math.max(0, (band.bottom - band.top - (caret.bottom - caret.top)) / 2));
  return caret.top < band.top || caret.bottom > band.bottom - m;
}

/** The yMargin for CodeMirror's scrollIntoView: the band margin plus whatever part of the
 *  scroller sits below the visible band (under the keyboard). */
export function revealYMargin(scroller: Band, band: Band, margin: number): number {
  return Math.round(margin + Math.max(0, scroller.bottom - band.bottom) + Math.max(0, band.top - scroller.top));
}

/** The visible height from the shell's keyboard source of truth (lib/viewportFit.ts;
 *  0 = not measured: a desktop). viewportFit pins the visual viewport's top at 0. */
let visibleHeight = 0;

function revealCaret(view: EditorView): void {
  if (!view.hasFocus) return;
  const head = view.state.selection.main.head;
  const r = view.scrollDOM.getBoundingClientRect();
  const band = visibleBand({ top: r.top, bottom: r.bottom }, 0, visibleHeight);
  const margin = caretRevealMargin(band.bottom - band.top);
  const c = view.coordsAtPos(head);
  if (!caretNeedsReveal(c ? { top: c.top, bottom: c.bottom } : null, band, margin)) return;
  view.dispatch({ effects: EditorView.scrollIntoView(head, { y: 'nearest', yMargin: revealYMargin({ top: r.top, bottom: r.bottom }, band, margin) }) });
}

/** Keeps the edited line inside the visible area when the editor's height changes. */
export const keepCaretVisible = ViewPlugin.fromClass(
  class {
    private height: number;
    private raf = 0;
    private readonly off: () => void;
    private readonly view: EditorView;
    constructor(view: EditorView) {
      this.view = view;
      this.height = view.scrollDOM.clientHeight;
      let first = true;
      // The keyboard store's own signal (a resize of the visual viewport) — the editor
      // box may not change when only the keyboard's height does.
      this.off = onKeyboard((k) => {
        visibleHeight = k.visibleHeight;
        if (first) { first = false; return; }
        this.schedule();
      });
    }
    update(u: ViewUpdate): void {
      if (!u.geometryChanged) return;
      const h = this.view.scrollDOM.clientHeight;
      if (h === this.height) return;
      this.height = h;
      this.schedule();
    }
    schedule(): void {
      if (this.raf || typeof requestAnimationFrame === 'undefined') return;
      // One frame later: the shell and the IDE chrome finish their own relayout first.
      this.raf = requestAnimationFrame(() => {
        this.raf = 0;
        revealCaret(this.view);
      });
    }
    destroy(): void {
      if (this.raf) cancelAnimationFrame(this.raf);
      this.off();
    }
  },
);

/** Cap for the hanging indent, in columns: a deeply nested line on a 320 px screen still
 *  keeps most of its width for text. */
export const WRAP_INDENT_MAX = 14;

/** Columns of leading whitespace (tabs to `tabSize`); -1 for a blank line. */
export function leadingIndentColumns(text: string, tabSize: number): number {
  let col = 0;
  for (let i = 0; i < text.length; i++) {
    const ch = text.charCodeAt(i);
    if (ch === 32) col += 1;
    else if (ch === 9) col += tabSize - (col % tabSize);
    else return col;
  }
  return -1;
}

/** The hanging indent for a wrapped line's continuation rows, in columns (0 = none). */
export function wrapHangColumns(text: string, tabSize: number, max = WRAP_INDENT_MAX): number {
  const indent = leadingIndentColumns(text, tabSize);
  return indent < 0 ? 0 : Math.min(indent + 2, max);
}

const hangDecos = new Map<number, Decoration>();
function hangDeco(cols: number): Decoration {
  let d = hangDecos.get(cols);
  if (!d) {
    // `.cm-line` has a 6 px start padding (CodeMirror's base theme): the first row keeps
    // its place, the continuation rows start `cols` columns further in.
    d = Decoration.line({ attributes: { style: `text-indent: -${cols}ch; padding-inline-start: calc(${cols}ch + 6px)` } });
    hangDecos.set(cols, d);
  }
  return d;
}

function buildHang(view: EditorView): DecorationSet {
  const b = new RangeSetBuilder<Decoration>();
  const tab = view.state.tabSize;
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to; ) {
      const line = view.state.doc.lineAt(pos);
      // Text this short never wraps on any phone: no decoration, no DOM attribute.
      if (line.length > 24) {
        const cols = wrapHangColumns(line.text, tab);
        if (cols > 0) b.add(line.from, line.from, hangDeco(cols));
      }
      pos = line.to + 1;
    }
  }
  return b.finish();
}

/** Soft-wrapped lines continue under their own indentation (visible lines only). */
export const wrapIndent = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildHang(view);
    }
    update(u: ViewUpdate): void {
      if (u.docChanged || u.viewportChanged) this.decorations = buildHang(u.view);
    }
  },
  { decorations: (v) => v.decorations },
);

/** Shows the hover documentation for the symbol at the caret (touch has no hover):
 *  replays a mouse hover there; CodeMirror's hover source does the rest. */
export function showIdeDocsAtCaret(view: EditorView): boolean {
  const c = view.coordsAtPos(view.state.selection.main.head);
  if (!c || typeof document === 'undefined') return false;
  const x = Math.round(c.left + 2);
  const y = Math.round((c.top + c.bottom) / 2);
  const target = document.elementFromPoint(x, y) ?? view.contentDOM;
  target.dispatchEvent(new MouseEvent('mousemove', { clientX: x, clientY: y, bubbles: true }));
  return true;
}
