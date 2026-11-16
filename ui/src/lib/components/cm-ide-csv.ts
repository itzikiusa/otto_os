// Fast RFC-4180 CSV / TSV CodeMirror 6 extensions and quote-aware record parser:
// - Auto-detected or preference-selected delimiter (`,`, `\t`, `;`, `|`)
// - Quote-aware multiline record splitting (`""` escapes across `\r\n` and `\n`)
// - Viewport-only Rainbow Column Highlighting (O(visible lines), 0 lag on 100k rows)
// - Sticky Column-Name + Row Hover Tooltip in text mode

import {
  Decoration, EditorView, ViewPlugin, hoverTooltip,
  type DecorationSet, type ViewUpdate,
} from '@codemirror/view';
import { type EditorState, type Extension, type Range } from '@codemirror/state';
import { IDE_PREFS_EVENT, readIdePreferences } from './cm-ide-prefs.ts';

export type CsvDelimiter = ',' | '\t' | ';' | '|';

export interface CsvCellSpan {
  colIndex: number;
  from: number;
  to: number;
  value: string;
}

export interface CsvRawRecord {
  rawText: string;
  startLine: number;
}

export interface CsvRecord {
  cells: string[];
  startLine: number;
}

export interface ParsedCsvTable {
  headers: string[];
  rows: string[][];
  rowStartLines: number[];
}

const CANDIDATE_DELIMITERS: readonly CsvDelimiter[] = [',', '\t', ';', '|'];
const COLOR_SLOTS = 6;
const MAX_DECORATED_LINE_LEN = 8_000;

function isDelimiterChar(ch: string, delimiter?: CsvDelimiter): boolean {
  if (delimiter) return ch === delimiter;
  return ch === ',' || ch === '\t' || ch === ';' || ch === '|';
}

/**
 * Split an RFC-4180 CSV/TSV document into logical records while keeping embedded
 * `\n` and `\r\n` inside quoted fields intact and honoring escaped `""` pairs.
 */
export function splitCsvRecords(text: string, delimiter?: CsvDelimiter): CsvRawRecord[] {
  if (!text) return [];
  const records: CsvRawRecord[] = [];
  const n = text.length;
  let inQuotes = false;
  let atFieldStart = true;
  let line = 1;
  let recStart = 0;
  let recStartLine = 1;

  for (let i = 0; i < n; i++) {
    const ch = text[i];
    if (inQuotes) {
      if (ch === '"') {
        if (i + 1 < n && text[i + 1] === '"') {
          i++;
        } else {
          inQuotes = false;
        }
      } else if (ch === '\r') {
        if (i + 1 < n && text[i + 1] === '\n') i++;
        line++;
      } else if (ch === '\n') {
        line++;
      }
      continue;
    }

    if (atFieldStart && ch === '"') {
      inQuotes = true;
      atFieldStart = false;
      continue;
    }
    if (isDelimiterChar(ch, delimiter)) {
      atFieldStart = true;
      continue;
    }
    if (ch === '\r' || ch === '\n') {
      records.push({ rawText: text.slice(recStart, i), startLine: recStartLine });
      if (ch === '\r' && i + 1 < n && text[i + 1] === '\n') i++;
      line++;
      recStart = i + 1;
      recStartLine = line;
      atFieldStart = true;
      continue;
    }
    atFieldStart = false;
  }

  if (recStart < n) {
    records.push({ rawText: text.slice(recStart, n), startLine: recStartLine });
  }
  while (records.length > 0 && records[records.length - 1].rawText === '') {
    records.pop();
  }
  return records;
}

/** Parse an entire RFC-4180 CSV/TSV document into records with preserved multiline cells. */
export function parseCsvRecords(text: string, delimiter: CsvDelimiter): CsvRecord[] {
  const rawRecords = splitCsvRecords(text, delimiter);
  const out: CsvRecord[] = new Array(rawRecords.length);
  for (let i = 0; i < rawRecords.length; i++) {
    const rec = rawRecords[i];
    out[i] = {
      cells: parseCsvLineSpans(rec.rawText, delimiter, 0).map((c) => c.value),
      startLine: rec.startLine,
    };
  }
  return out;
}

/** Build a structured header + rows table for CSV, TSV, or JSONL grid views. */
export function parseCsvOrJsonlTable(
  content: string,
  opts: { isJsonl: boolean; delimiter: CsvDelimiter; firstRowHeader: boolean },
): ParsedCsvTable {
  if (!content.trim()) return { headers: ['col_1'], rows: [], rowStartLines: [] };

  if (opts.isJsonl) {
    const rawLines = content.split(/\r?\n/);
    const objs: Array<Record<string, unknown>> = [];
    const rowStartLines: number[] = [];
    const keySet = new Set<string>();
    for (let i = 0; i < rawLines.length; i++) {
      const t = rawLines[i].trim();
      if (!t) continue;
      rowStartLines.push(i + 1);
      try {
        const obj = JSON.parse(t) as Record<string, unknown>;
        if (obj && typeof obj === 'object' && !Array.isArray(obj)) {
          objs.push(obj);
          for (const k of Object.keys(obj)) keySet.add(k);
        } else {
          objs.push({ value: obj });
          keySet.add('value');
        }
      } catch {
        objs.push({ _raw: t });
        keySet.add('_raw');
      }
    }
    if (objs.length === 0) return { headers: ['col_1'], rows: [], rowStartLines: [] };
    const headers = keySet.size > 0 ? [...keySet] : ['value'];
    const rows = objs.map((o) =>
      headers.map((h) => {
        const v = o[h];
        return v === undefined || v === null ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v);
      }),
    );
    return { headers, rows, rowStartLines };
  }

  const records = parseCsvRecords(content, opts.delimiter);
  if (records.length === 0) return { headers: ['col_1'], rows: [], rowStartLines: [] };

  if (!opts.firstRowHeader) {
    let maxCols = 1;
    const rows: string[][] = new Array(records.length);
    const rowStartLines: number[] = new Array(records.length);
    for (let i = 0; i < records.length; i++) {
      const parsed = records[i].cells;
      if (parsed.length > maxCols) maxCols = parsed.length;
      rows[i] = parsed;
      rowStartLines[i] = records[i].startLine;
    }
    const headers = Array.from({ length: maxCols }, (_, idx) => `col_${idx + 1}`);
    return { headers, rows, rowStartLines };
  }

  const headerCells = records[0].cells.map((val, idx) => val || `col_${idx + 1}`);
  let maxCols = Math.max(1, headerCells.length);
  const dataLen = Math.max(0, records.length - 1);
  const rows: string[][] = new Array(dataLen);
  const rowStartLines: number[] = new Array(dataLen);
  for (let i = 1; i < records.length; i++) {
    const parsed = records[i].cells;
    if (parsed.length > maxCols) maxCols = parsed.length;
    rows[i - 1] = parsed;
    rowStartLines[i - 1] = records[i].startLine;
  }
  const headers = [...headerCells];
  while (headers.length < maxCols) headers.push(`col_${headers.length + 1}`);
  return { headers, rows, rowStartLines };
}

/** Detect the active delimiter from the first few non-empty records of a CSV/TSV document. */
export function detectCsvDelimiter(sampleText: string, filePath = ''): CsvDelimiter {
  const lower = filePath.toLowerCase();
  if (lower.endsWith('.tsv')) return '\t';
  const records = splitCsvRecords(sampleText)
    .map((r) => r.rawText)
    .filter((r) => r.trim().length > 0)
    .slice(0, 8);
  if (records.length === 0) return ',';

  let bestDelim: CsvDelimiter = ',';
  let bestScore = -1;
  for (const delim of CANDIDATE_DELIMITERS) {
    const counts = records.map((ln) => parseCsvLineSpans(ln, delim, 0).length);
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

/** Resolve the effective CSV delimiter, honoring an explicit user preference when not `'auto'`. */
export function resolveCsvDelimiter(
  sampleText: string,
  filePath = '',
  preferred: 'auto' | ',' | '\t' | ';' = 'auto',
): CsvDelimiter {
  if (preferred !== 'auto') return preferred;
  return detectCsvDelimiter(sampleText, filePath);
}

/** Parse a single CSV/TSV record into RFC-4180 cell spans (0-based offsets). */
export function parseCsvLine(lineText: string, delimiter: CsvDelimiter): CsvCellSpan[] {
  return parseCsvLineSpans(lineText, delimiter, 0);
}

/** Parse a single CSV/TSV record into RFC-4180 cell spans with document offsets. */
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

function buildCsvViewportDecorations(
  view: EditorView,
  delimiter: CsvDelimiter,
  rainbowEnabled: boolean,
): DecorationSet {
  if (!rainbowEnabled) return Decoration.none;
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
  const resolveDelim = (state: EditorState): CsvDelimiter => {
    const prefs = readIdePreferences();
    return resolveCsvDelimiter(
      state.sliceDoc(0, Math.min(state.doc.length, 4096)),
      filePath,
      prefs.csvDelimiter,
    );
  };

  const rainbowPlugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      delimiter: CsvDelimiter;
      private onPrefsChanged: (() => void) | null = null;
      constructor(view: EditorView) {
        const prefs = readIdePreferences();
        this.delimiter = resolveDelim(view.state);
        this.decorations = buildCsvViewportDecorations(view, this.delimiter, prefs.csvRainbowColumns);
        if (typeof window !== 'undefined') {
          this.onPrefsChanged = () => {
            const nextPrefs = readIdePreferences();
            this.delimiter = resolveDelim(view.state);
            this.decorations = buildCsvViewportDecorations(
              view,
              this.delimiter,
              nextPrefs.csvRainbowColumns,
            );
            view.dispatch({});
          };
          window.addEventListener(IDE_PREFS_EVENT, this.onPrefsChanged);
        }
      }
      update(u: ViewUpdate) {
        if (u.docChanged) this.delimiter = resolveDelim(u.state);
        if (u.docChanged || u.viewportChanged) {
          const prefs = readIdePreferences();
          this.decorations = buildCsvViewportDecorations(
            u.view,
            this.delimiter,
            prefs.csvRainbowColumns,
          );
        }
      }
      destroy() {
        if (this.onPrefsChanged && typeof window !== 'undefined') {
          window.removeEventListener(IDE_PREFS_EVENT, this.onPrefsChanged);
          this.onPrefsChanged = null;
        }
      }
    },
    { decorations: (v) => v.decorations },
  );

  const cellHover = hoverTooltip((view, pos) => {
    const line = view.state.doc.lineAt(pos);
    if (line.length === 0 || line.length > MAX_DECORATED_LINE_LEN) return null;
    const prefs = readIdePreferences();
    const delim = resolveDelim(view.state);
    const cells = parseCsvLineSpans(line.text, delim, line.from);
    const hit = cells.find((c) => pos >= c.from && pos <= c.to);
    if (!hit) return null;
    const headers = prefs.csvFirstRowHeader ? readHeaderNames(view.state.doc, delim) : [];
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
