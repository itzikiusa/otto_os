// CodeMirror 6 inline widgets & client quick-fix applier for `cm-ide-semantic.ts`.

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
    el.textContent = `${this.count} ${this.count === 1 ? 'usage' : 'usages'}`;
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

export function applyClientQuickFix(view: EditorView, fix: ClientDiagQuickFix): void {
  const doc = view.state.doc;
  if (fix.edits?.length) {
    const changes = fix.edits.map((ed) => {
      const s = doc.line(Math.max(1, Math.min(doc.lines, ed.start_line)));
      const e = doc.line(Math.max(1, Math.min(doc.lines, ed.end_line)));
      const from = Math.min(doc.length, s.from + Math.max(0, (ed.start_col || 1) - 1));
      const to = ed.end_line > doc.lines ? doc.length : Math.max(from, Math.min(doc.length, e.from + Math.max(0, (ed.end_col || 1) - 1)));
      return { from, to, insert: ed.new_text };
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
