// Hover-to-evaluate while a debug session is paused: the identifier (or member chain, or the
// selection) under the pointer is evaluated in the selected frame and shown as `name = value`.
// Active only while the IDE hands over an evaluator (paused, in the paused/selected-frame
// file); otherwise it yields nothing and the language-server hover is alone. When both
// answer, this one is mounted first (Prec.high), so the value sits above the docs.

import { EditorView, hoverTooltip } from '@codemirror/view';
import { Prec, type EditorState, type Extension } from '@codemirror/state';
import type { IdeBreakpointMove } from './cm-ide-bp-model.ts';

/** Debug hooks the IDE passes through the editor callbacks (beside IdeEditorCallbacks). */
export interface IdeDebugEditorCallbacks {
  onBreakpointsMoved?: ((path: string, moves: IdeBreakpointMove[]) => void) | null;
  onDebugHover?: ((expression: string) => Promise<string>) | null;
}

const IDENT = /[\w$]/;
const KEYWORDS = new Set([
  'and', 'as', 'assert', 'async', 'await', 'break', 'case', 'catch', 'class', 'const', 'continue', 'def', 'default',
  'defer', 'del', 'do', 'elif', 'else', 'except', 'export', 'false', 'False', 'finally', 'for', 'from', 'func', 'function',
  'go', 'if', 'import', 'in', 'is', 'lambda', 'let', 'match', 'new', 'nil', 'None', 'not', 'null', 'or', 'package',
  'pass', 'raise', 'range', 'return', 'struct', 'switch', 'throw', 'true', 'True', 'try', 'type', 'var', 'while', 'with', 'yield',
]);

/** The expression to evaluate at `pos`: the selection when the pointer is inside a one-line
 *  selection, else the identifier there extended left over `a.b.` member access. */
export function debugHoverExpressionAt(state: EditorState, pos: number): { from: number; to: number; expr: string } | null {
  const sel = state.selection.main;
  if (!sel.empty && pos >= sel.from && pos <= sel.to && sel.to - sel.from <= 200) {
    const raw = state.sliceDoc(sel.from, sel.to), expr = raw.trim();
    if (expr && !raw.includes('\n')) return { from: sel.from, to: sel.to, expr };
  }
  const line = state.doc.lineAt(pos), text = line.text;
  let i = pos - line.from;
  if (!IDENT.test(text[i] ?? '')) { if (i > 0 && IDENT.test(text[i - 1])) i--; else return null; }
  let s = i, e = i + 1;
  while (s > 0 && IDENT.test(text[s - 1])) s--;
  while (e < text.length && IDENT.test(text[e])) e++;
  const word = text.slice(s, e);
  if (/^\d/.test(word) || KEYWORDS.has(word)) return null;
  while (s > 1 && text[s - 1] === '.' && IDENT.test(text[s - 2])) { s--; while (s > 0 && IDENT.test(text[s - 1])) s--; }
  // Comments are prose, not code to evaluate.
  if (/(?:^|\s)(?:#|\/\/)/.test(text.slice(0, s))) return null;
  return { from: line.from + s, to: line.from + e, expr: text.slice(s, e) };
}

const EVAL_ERROR = /^(?:[\w.]*(?:Error|Exception)\b|error\b|evaluate\b.*failed|Couldn.t\b|not available\b)/i;
const HOVER_VALUE_MAX = 300;

const debugHoverTheme = EditorView.baseTheme({
  '.cm-ide-dbg-hover': { padding: '3px 8px', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-xs)', color: 'var(--text)', maxInlineSize: '560px', whiteSpace: 'pre-wrap', wordBreak: 'break-word' },
  '.cm-ide-dbg-hover-name': { color: 'var(--text-dim)' },
});

export function createIdeDebugHoverExtension(getEvaluate: () => ((expression: string) => Promise<string>) | null | undefined): Extension {
  const tip = hoverTooltip(async (view, pos) => {
    const evaluate = getEvaluate();
    const hit = evaluate ? debugHoverExpressionAt(view.state, pos) : null;
    if (!evaluate || !hit) return null;
    let value: string;
    try { value = (await evaluate(hit.expr)).trim(); } catch { return null; }
    if (!value || EVAL_ERROR.test(value)) return null;
    const shown = value.length > HOVER_VALUE_MAX ? `${value.slice(0, HOVER_VALUE_MAX - 1)}…` : value;
    return {
      pos: hit.from, end: hit.to, above: true,
      create: () => {
        const dom = document.createElement('div');
        dom.className = 'cm-ide-dbg-hover'; dom.dataset.testid = 'ide-debug-hover'; dom.dir = 'ltr';
        const name = document.createElement('span'); name.className = 'cm-ide-dbg-hover-name'; name.textContent = `${hit.expr} = `;
        dom.append(name, document.createTextNode(shown));
        return { dom };
      },
    };
  }, { hoverTime: 350 });
  return [debugHoverTheme, Prec.high(tip)];
}
