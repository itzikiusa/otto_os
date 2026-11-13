// Fast RFC-4180 CSV / TSV CodeMirror 6 extensions:
// - Auto-detected delimiter (`,`, `\t`, `;`, `|`)
// - Viewport-only Rainbow Column Highlighting (O(visible lines), 0 lag on 100k rows)
// - Sticky Column-Name + Row Hover Tooltip in text mode

import {
  Decoration, EditorView, ViewPlugin, hoverTooltip,
  type DecorationSet, type ViewUpdate,
} from '@codemirror/view';
import { type EditorState, type Extension, type Range } from '@codemirror/state';

export type CsvDelimiter = ',' | '\t' | ';' | '|';

export interface CsvCellSpan {
  colIndex: number;
  from: number;
  to: number;
  value: string;
}

const CANDIDATE_DELIMITERS: readonly CsvDelimiter[] = [',', '\t', ';', '|'];
const COLOR_SLOTS = 6;
const MAX_DECORATED_LINE_LEN = 8_000;

/** Detect the active delimiter from the first few non-empty lines of a CSV/TSV document. */
export function detectCsvDelimiter(sampleText: string, filePath = ''): CsvDelimiter {
  const lower = filePath.toLowerCase();
  if (lower.endsWith('.tsv')) return '\t';
  const lines: string[] = [];
  let start = 0;
  while (lines.length < 8 && start < sampleText.length) {
    const nl = sampleText.indexOf('\n', start);
    const raw = (nl < 0 ? sampleText.slice(start) : sampleText.slice(start, nl)).replace(/\r$/, '');
    if (raw.trim().length > 0) lines.push(raw);
    if (nl < 0) break;
    start = nl + 1;
  }
  if (lines.length === 0) return ',';

  let bestDelim: CsvDelimiter = ',';
  let bestScore = -1;
  for (const delim of CANDIDATE_DELIMITERS) {
    const counts = lines.map((ln) => parseCsvLineSpans(ln, delim, 0).length);
    const minCols = Math.min(...counts);
    const maxCols = Math.max(...counts);
    if (minCols <= 1) continue;
    const consistentBonus = minCols === maxCols ? 100 : 0;
    const score = consistentBonus + minCols * 4 - (maxCols - minCols);
    if (score > bestScore) {
      bestScore = score;
      bestDelim = delim;
    }
  }
  return bestDelim;
}

/** Parse a single CSV/TSV line into RFC-4180 cell spans (0-based offsets). */
export function parseCsvLine(lineText: string, delimiter: CsvDelimiter): CsvCellSpan[] {
  return parseCsvLineSpans(lineText, delimiter, 0);
}

/** Parse a single CSV/TSV line into RFC-4180 cell spans with document offsets. */
export function parseCsvLineSpans(lineText: string, delimiter: CsvDelimiter, lineFrom = 0): CsvCellSpan[] {
  const spans: CsvCellSpan[] = [];
  const n = lineText.length;
  let i = 0;
  let colIndex = 0;

  while (i <= n) {
    if (i === n) {
      if (colIndex > 0 && lineText[n - 1] === delimiter) {
        spans.push({ colIndex, from: lineFrom + n, to: lineFrom + n, value: '' });
      }
      break;
    }
    const cellStart = i;
    if (lineText[i] === '"') {
      i++;
      let val = '';
      while (i < n) {
        if (lineText[i] === '"') {
          if (i + 1 < n && lineText[i + 1] === '"') {
            val += '"';
            i += 2;
          } else {
            i++;
            break;
          }
        } else {
          val += lineText[i];
          i++;
        }
      }
      while (i < n && lineText[i] !== delimiter) i++;
      spans.push({ colIndex, from: lineFrom + cellStart, to: lineFrom + i, value: val });
      colIndex++;
      if (i < n && lineText[i] === delimiter) i++;
      else break;
    } else {
      let end = i;
      while (end < n && lineText[end] !== delimiter) end++;
      const val = lineText.slice(cellStart, end);
      spans.push({ colIndex, from: lineFrom + cellStart, to: lineFrom + end, value: val });
      colIndex++;
      if (end < n && lineText[end] === delimiter) i = end + 1;
      else break;
    }
  }
  return spans;
}

function readHeaderNames(doc: EditorState['doc'], delimiter: CsvDelimiter): string[] {
  for (let ln = 1; ln <= Math.min(doc.lines, 10); ln++) {
    const text = doc.line(ln).text;
    if (text.trim().length === 0) continue;
    return parseCsvLineSpans(text, delimiter, 0).map((c, idx) => c.value.trim() || `Col ${idx + 1}`);
  }
  return [];
}

const colDecos = Array.from({ length: COLOR_SLOTS }, (_, i) =>
  Decoration.mark({ class: `cm-ide-csv-col cm-ide-csv-col-${i}` }),
);
const sepDeco = Decoration.mark({ class: 'cm-ide-csv-sep' });

function buildCsvViewportDecorations(view: EditorView, delimiter: CsvDelimiter): DecorationSet {
  const ranges: Range<Decoration>[] = [];
  const { doc } = view.state;
  for (const { from, to } of view.visibleRanges) {
    let pos = from;
    while (pos <= to) {
      const line = doc.lineAt(pos);
      if (line.length > 0 && line.length <= MAX_DECORATED_LINE_LEN) {
        const cells = parseCsvLineSpans(line.text, delimiter, line.from);
        for (let i = 0; i < cells.length; i++) {
          const c = cells[i];
          if (c.to > c.from) {
            ranges.push(colDecos[c.colIndex % COLOR_SLOTS].range(c.from, c.to));
          }
          if (i < cells.length - 1 && c.to < line.to) {
            ranges.push(sepDeco.range(c.to, c.to + 1));
          }
        }
      }
      if (line.to >= to) break;
      pos = line.to + 1;
    }
  }
  return ranges.length > 0 ? Decoration.set(ranges, true) : Decoration.none;
}

const csvEditorTheme = EditorView.baseTheme({
  '.cm-ide-csv-col-0': { color: 'var(--code-keyword)' },
  '.cm-ide-csv-col-1': { color: 'var(--code-string)' },
  '.cm-ide-csv-col-2': { color: 'var(--code-number)' },
  '.cm-ide-csv-col-3': { color: 'var(--code-builtin)' },
  '.cm-ide-csv-col-4': { color: 'var(--info)' },
  '.cm-ide-csv-col-5': { color: 'var(--warning)' },
  '.cm-ide-csv-sep': { color: 'var(--text-dim)', fontWeight: '600' },
  '.cm-ide-csv-hover': {
    padding: 'var(--sp-2) var(--sp-3)',
    fontFamily: 'var(--font-mono)',
    fontSize: 'var(--fs-xs)',
    color: 'var(--text)',
    backgroundColor: 'var(--surface-2)',
    maxWidth: 'min(420px, calc(100vw - 24px))',
    display: 'flex',
    flexDirection: 'column',
    gap: 'var(--sp-1)',
  },
  '.cm-ide-csv-hover-meta': { color: 'var(--text-dim)' },
});

/** Create Rainbow CSV/TSV syntax highlighting + column-header hover tooltip extension. */
export function createIdeCsvExtension(filePath: string): Extension {
  const resolveDelim = (state: EditorState): CsvDelimiter =>
    detectCsvDelimiter(state.sliceDoc(0, Math.min(state.doc.length, 4096)), filePath);

  const rainbowPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      delimiter: CsvDelimiter;
      constructor(view: EditorView) {
        this.delimiter = resolveDelim(view.state);
        this.decorations = buildCsvViewportDecorations(view, this.delimiter);
      }
      update(u: ViewUpdate) {
        if (u.docChanged) this.delimiter = resolveDelim(u.state);
        if (u.docChanged || u.viewportChanged) {
          this.decorations = buildCsvViewportDecorations(u.view, this.delimiter);
        }
      }
    },
    { decorations: (v) => v.decorations },
  );

  const cellHover = hoverTooltip((view, pos) => {
    const line = view.state.doc.lineAt(pos);
    if (line.length === 0 || line.length > MAX_DECORATED_LINE_LEN) return null;
    const delim = resolveDelim(view.state);
    const cells = parseCsvLineSpans(line.text, delim, line.from);
    const hit = cells.find((c) => pos >= c.from && pos <= c.to);
    if (!hit) return null;
    const headers = readHeaderNames(view.state.doc, delim);
    const colName = headers[hit.colIndex] ?? `Column ${hit.colIndex + 1}`;
    const delimLabel = delim === '\t' ? 'TSV' : `CSV (${delim})`;
    return {
      pos: hit.from,
      end: hit.to,
      above: true,
      create() {
        const dom = document.createElement('div');
        dom.className = 'cm-ide-csv-hover';
        const title = document.createElement('div');
        title.textContent = `Col ${hit.colIndex + 1}: ${colName}`;
        const val = document.createElement('div');
        val.className = 'cm-ide-csv-hover-meta';
        const preview = hit.value.length > 96 ? `${hit.value.slice(0, 93)}…` : (hit.value || '(empty)');
        val.textContent = `Row ${line.number} · ${delimLabel} · Value: ${preview}`;
        dom.append(title, val);
        return { dom };
      },
    };
  });

  return [csvEditorTheme, rainbowPlugin, cellHover];
}
