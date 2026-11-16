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

// The JSON/SVG text helpers are shared with the IDE's and Threads' file
// preview (lib/render/previewText.ts); re-exported so existing imports stay.
export { lineColAt, parseJsonWithPos, svgDataUrl, type JsonParse } from '../../../lib/render/previewText.ts';

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
