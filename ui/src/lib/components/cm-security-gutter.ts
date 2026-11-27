import { RangeSet, type Compartment, type Extension } from '@codemirror/state';
import { EditorView, GutterMarker, gutter } from '@codemirror/view';
import type { IdeDiagnostic } from '../api/types';

class SecurityMarker extends GutterMarker {
  constructor(readonly message: string) { super(); }

  eq(other: SecurityMarker): boolean { return this.message === other.message; }

  toDOM(): Node {
    const label = document.createElement('span');
    label.textContent = '●';
    label.title = this.message;
    label.setAttribute('aria-label', this.message);
    return label;
  }
}

export function securityGutter(items: readonly Pick<IdeDiagnostic, "line" | "message">[]): Extension {
  if (items.length === 0) return [];
  const lines = new Map<number, string[]>();
  for (const item of items) {
    const group = lines.get(item.line) ?? [];
    group.push(item.message);
    lines.set(item.line, group);
  }
  return [
    gutter({
      class: 'cm-security-findings',
      markers: (editor) => RangeSet.of([...lines].flatMap(([line, findings]) =>
        line < 1 || line > editor.state.doc.lines ? [] : [
          new SecurityMarker(findings.join('\n')).range(editor.state.doc.line(line).from),
        ]), true),
    }),
    EditorView.baseTheme({ '.cm-security-findings': { color: 'var(--warning)', minWidth: '12px' } }),
  ];
}

export function refreshSecurityGutter(view: EditorView | null, compartment: Compartment, items: readonly Pick<IdeDiagnostic, "line" | "message">[]): void {
  if (view && compartment.get(view.state) !== undefined) {
    view.dispatch({ effects: compartment.reconfigure(securityGutter(items)) });
  }
}
