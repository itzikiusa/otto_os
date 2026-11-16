// CodeMirror 6 inline widgets & client quick-fix applier for `cm-ide-semantic.ts`.

import type { Text } from '@codemirror/state';
import { EditorView, WidgetType } from '@codemirror/view';
import type { ClientDiagQuickFix } from './cm-ide-semantic-model.ts';

export class InlayHintWidget extends WidgetType {
  label: string;
  constructor(label: string) {
    super();
    this.label = label;
  }
  eq(o: InlayHintWidget): boolean {
    return this.label === o.label;
  }
  toDOM(): HTMLElement {
    const el = document.createElement('span');
    el.className = 'cm-ide-inlay-hint';
    el.textContent = `${this.label}:`;
    return el;
  }
}

export class CodeVisionWidget extends WidgetType {
  count: number;
  line: number;
  symbol: string;
  onUsages: ((line: number, symbol: string) => void) | undefined;
  constructor(count: number, line: number, symbol: string, onUsages?: (line: number, symbol: string) => void) {
    super();
    this.count = count;
    this.line = line;
    this.symbol = symbol;
    this.onUsages = onUsages;
  }
  eq(o: CodeVisionWidget): boolean {
    return this.count === o.count && this.line === o.line && this.symbol === o.symbol;
  }
  toDOM(): HTMLElement {
    const el = this.onUsages ? document.createElement('button') : document.createElement('span');
    if (el instanceof HTMLButtonElement) el.type = 'button';
    el.className = 'cm-ide-codevision';
    el.dataset.testid = 'ide-codevision-usages';
    // A same-file word count, so it never claims project-wide usages.
    el.textContent = `${this.count} in file`;
    el.title = `Uses of ${this.symbol} in this file${this.onUsages ? ' (click to find usages in the project)' : ''}`;
    if (this.onUsages) {
      el.addEventListener('mousedown', (e) => {
        e.preventDefault();
        e.stopPropagation();
      });
      el.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        this.onUsages?.(this.line, this.symbol);
      });
    }
    return el;
  }
}

/** 1-based edit position → doc offset, matching the `⌥⏎` preview and the daemon: a line past EOF is
 *  the end of the doc (an EOF insert on a file without a trailing newline), and a column stops at
 *  its line's newline instead of bleeding onto the next line. */
export function editOffset(doc: Text, line1: number, col1: number): number {
  if (line1 > doc.lines) return doc.length;
  const ln = doc.line(Math.max(1, line1));
  return Math.min(doc.length, ln.from + Math.min(ln.length + 1, Math.max(0, (col1 || 1) - 1)));
}

export function applyClientQuickFix(view: EditorView, fix: ClientDiagQuickFix): void {
  const doc = view.state.doc;
  if (fix.edits?.length) {
    const changes = fix.edits.map((ed) => {
      const from = editOffset(doc, ed.start_line, ed.start_col);
      return { from, to: Math.max(from, editOffset(doc, ed.end_line, ed.end_col)), insert: ed.new_text };
    });
    view.dispatch({ changes, userEvent: 'input.quickfix' });
    return;
  }
  const sLn = Math.max(1, Math.min(doc.lines, fix.line));
  const endLn = doc.line(Math.max(sLn, Math.min(doc.lines, fix.endLine || sLn)));
  view.dispatch({
    changes: {
      from: doc.line(sLn).from,
      to: fix.deleteLine ? Math.min(doc.length, endLn.to + 1) : endLn.to,
      insert: fix.deleteLine ? '' : (fix.replacement ?? ''),
    },
    userEvent: 'input.quickfix',
  });
}
