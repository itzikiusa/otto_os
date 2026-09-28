// Imperative DOM for GridView's body cells. A scroll step swaps ~12 rows × N
// columns in and out; as Svelte markup every cell cost an each-item, an
// if-chain block + branch, five `{@const}` deriveds and a template effect (a
// dozen reactive nodes, ×2 in dev) — ~7 ms of a 300 px step at 100k × 30.
// GridView now renders the <tr> + row-number cell in Svelte and fills the
// data cells from here inside ONE attachment effect per row, so a step creates
// plain elements only. The markup is exactly what the template produced
// (classes, titles, data-r/c/p/k, NULL glyph, expand button, <mark> hits);
// GridView's styles address these nodes through `:global(…)` under `.grid`.
import {
  CELL_MAX,
  SET_EMPTY,
  SET_NULL,
  cellDisplay,
  cellStr,
  clip,
  isComplex,
  previewJson,
} from './results-format';
import { highlightParts, type ColumnKind } from './grid-format';

export interface CellOpts {
  kind: ColumnKind;
  /** Column is inline-editable (flow.isEditableCell). */
  editable: boolean;
  widthCh: number;
  /** Parked whole-cell draft for this cell, if any. */
  pv: string | undefined;
  /** A path-level change is parked under this column (Vertical view ops). */
  pendingUnder: boolean;
  /** Lower-cased toolbar search while it filters, else null. */
  needle: string | null;
  expandJson: boolean;
  /** The keyboard cell cursor sits here. */
  focused: boolean;
}

export function widthStyle(ch: number): string {
  return `width:${ch}ch; max-width:${ch}ch;`;
}

const TITLE_DRAFT = 'Pending change — Review & apply (bar below) writes it; double-click to keep editing';
const TITLE_NESTED = 'Nested change pending — Review & apply (bar below) writes it; see the Vertical view';

let expandProto: HTMLButtonElement | null = null;
function expandButton(): Node {
  if (!expandProto) {
    expandProto = document.createElement('button');
    expandProto.className = 'cell-expand';
    expandProto.title = 'Expand value';
    expandProto.setAttribute('aria-label', 'Expand value');
  }
  return expandProto.cloneNode(false);
}

function glyph(text: string): HTMLSpanElement {
  const s = document.createElement('span');
  s.className = 'null-glyph';
  s.textContent = text;
  return s;
}

/** One data cell (every non-editing case of the old template's if-chain). */
export function buildCell(v: unknown, idx: number, ci: number, pos: number, o: CellOpts): HTMLTableCellElement {
  const td = document.createElement('td');
  let cls = 'cell';
  let k: string;
  if (o.pv !== undefined) {
    cls += ' dirty';
    if (o.kind === 'num') cls += ' num';
    td.title = TITLE_DRAFT;
    k = 'd';
    if (o.pv === '' || o.pv === SET_NULL) td.append(glyph('NULL'));
    else if (o.pv === SET_EMPTY) td.append(glyph("''"));
    else td.append(o.pv);
  } else if (o.pendingUnder) {
    cls += ' dirty';
    td.title = TITLE_NESTED;
    k = 'u';
    td.append(v === null || v === undefined ? '' : isComplex(v) ? clip(previewJson(v)) : clip(cellStr(v)));
  } else if (v === null || v === undefined) {
    cls += ' null';
    if (o.kind === 'num') cls += ' num';
    if (o.editable) cls += ' editable';
    td.title = 'NULL';
    k = 'n';
    td.append(glyph('NULL'));
  } else if (isComplex(v)) {
    cls += ' json';
    if (o.expandJson) cls += ' wrap';
    td.title = 'Click to expand';
    k = 'j';
    td.append(clip(previewJson(v, CELL_MAX, o.expandJson)), expandButton());
  } else {
    if (o.kind === 'num') cls += ' num';
    if (o.kind === 'bool') cls += ' bool';
    if (o.editable) cls += ' editable';
    k = 'p';
    const text = cellDisplay(v);
    if (o.needle) {
      for (const part of highlightParts(text, o.needle)) {
        if (part.hit) {
          const m = document.createElement('mark');
          m.textContent = part.t;
          td.append(m);
        } else td.append(part.t);
      }
    } else td.append(text);
    td.append(expandButton());
  }
  if (o.focused) cls += ' kbd-focus';
  td.className = cls;
  td.setAttribute('style', widthStyle(o.widthCh));
  td.dataset.r = String(idx);
  td.dataset.c = String(ci);
  td.dataset.p = String(pos);
  td.dataset.k = k;
  return td;
}
