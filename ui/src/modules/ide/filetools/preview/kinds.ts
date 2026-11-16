// Workbench preview: which renderer a language gets, plus the small pure
// helpers the renderers share (JSON error positions, CSV caps, SVG data URLs).
// Framework-free so unit/workbench-preview.test.ts can exercise it directly.

export type PreviewKind = 'markdown' | 'html' | 'image' | 'svg' | 'mermaid' | 'd2' | 'json' | 'csv' | 'tsv';

/** Language id (already effective — never `auto`) → preview renderer, or null
 *  when the language has nothing to preview. */
export function previewKind(lang: string): PreviewKind | null {
  switch ((lang || '').toLowerCase()) {
    case 'md':
    case 'markdown':
    case 'mdx':
      return 'markdown';
    case 'html':
    case 'htm':
    case 'xhtml':
      return 'html';
    case 'svg':
      return 'svg';
    case 'image':
    case 'png':
    case 'jpg':
    case 'jpeg':
    case 'gif':
    case 'webp':
      return 'image';
    case 'mermaid':
    case 'mmd':
      return 'mermaid';
    case 'd2':
      return 'd2';
    case 'json':
    case 'jsonc':
    case 'geojson':
      return 'json';
    case 'csv':
      return 'csv';
    case 'tsv':
      return 'tsv';
    default:
      return null;
  }
}

export function canPreview(lang: string): boolean {
  return previewKind(lang) !== null;
}

/** 1-based line/column of a 0-based character offset in `text`. */
export function lineColAt(text: string, offset: number): { line: number; col: number } {
  const o = Math.max(0, Math.min(offset, text.length));
  let line = 1;
  let last = -1;
  for (let i = 0; i < o; i++) {
    if (text.charCodeAt(i) === 10) {
      line++;
      last = i;
    }
  }
  return { line, col: o - last };
}

export interface JsonParse {
  ok: boolean;
  value?: unknown;
  error?: string;
  line?: number;
  col?: number;
}

/** `JSON.parse` with a readable error position. Engines disagree on the
 *  message: V8 says "at position N" (newer ones "(line L column C)"), WebKit
 *  says nothing — so fall back to scanning for the first offending spot. */
export function parseJsonWithPos(text: string): JsonParse {
  if (!text.trim()) return { ok: false, error: 'Empty document' };
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    const lc = /line (\d+) column (\d+)/i.exec(msg);
    if (lc) return { ok: false, error: msg, line: Number(lc[1]), col: Number(lc[2]) };
    const pos = /position (\d+)/i.exec(msg);
    if (pos) return { ok: false, error: msg, ...lineColAt(text, Number(pos[1])) };
    return { ok: false, error: msg };
  }
}

/** Rows rendered in the CSV/TSV table before the "showing first N" note. */
export const CSV_ROW_CAP = 2000;

export interface CsvView {
  header: string[];
  rows: string[][];
  /** Data rows in the whole document (excluding the header). */
  total: number;
  /** Widest row — short rows are padded so the grid stays rectangular. */
  width: number;
}

/** Split parsed rows into header + capped body; drop one trailing blank row. */
export function csvView(parsed: string[][], cap = CSV_ROW_CAP): CsvView {
  const all = parsed.slice();
  while (all.length && all[all.length - 1].every((c) => c === '')) all.pop();
  const header = all[0] ?? [];
  const body = all.slice(1);
  let width = header.length;
  for (const r of body.slice(0, cap)) width = Math.max(width, r.length);
  return { header, rows: body.slice(0, cap), total: body.length, width };
}

/** SVG text → a data URL for an `<img>` (images never run scripts). */
export function svgDataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
