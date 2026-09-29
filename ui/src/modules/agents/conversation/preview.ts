// Pure helpers for the side panel's file preview (PreviewBody.svelte): what a
// file renders AS, and the parsers behind the rendered views (CSV/TSV table,
// pretty JSON). Kept DOM-free so they are unit-testable.

export type PreviewKind = 'markdown' | 'html' | 'svg' | 'image' | 'json' | 'csv' | 'tsv' | 'code';

const IMAGE = /\.(png|jpe?g|gif|webp|avif|bmp|ico)$/i;

/** How a file previews, by its name. `code` has no rendered view (Source only). */
export function previewKind(path: string): PreviewKind {
  const p = path.toLowerCase();
  if (/\.(md|mdx|markdown)$/.test(p)) return 'markdown';
  if (/\.html?$/.test(p)) return 'html';
  if (p.endsWith('.svg')) return 'svg';
  if (IMAGE.test(p)) return 'image';
  if (/\.(json|jsonc|geojson|ipynb)$/.test(p)) return 'json';
  if (p.endsWith('.csv')) return 'csv';
  if (p.endsWith('.tsv')) return 'tsv';
  return 'code';
}

/** Rows past this are cut from the table view (the Source tab has them all). */
export const TABLE_MAX_ROWS = 2000;

/** RFC-4180-ish CSV/TSV parse: quoted fields, doubled quotes, newlines inside
 *  quotes. Returns at most `maxRows` rows (+ whether more were cut). */
export function parseDelimited(text: string, sep: ',' | '\t', maxRows = TABLE_MAX_ROWS): { rows: string[][]; cut: boolean } {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = '';
  let quoted = false;
  let i = 0;
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (quoted) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        quoted = false;
        i++;
        continue;
      }
      field += c;
      i++;
      continue;
    }
    if (c === '"' && field === '') {
      quoted = true;
      i++;
      continue;
    }
    if (c === sep) {
      row.push(field);
      field = '';
      i++;
      continue;
    }
    if (c === '\n' || c === '\r') {
      row.push(field);
      field = '';
      rows.push(row);
      row = [];
      if (c === '\r' && text[i + 1] === '\n') i++;
      i++;
      if (rows.length > maxRows) return { rows: rows.slice(0, maxRows), cut: true };
      continue;
    }
    field += c;
    i++;
  }
  if (field !== '' || row.length) {
    row.push(field);
    rows.push(row);
  }
  return { rows: rows.slice(0, maxRows), cut: rows.length > maxRows };
}

/** Pretty-printed JSON, or null when the text does not parse. */
export function prettyJson(text: string): string | null {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return null;
  }
}

/** The file's name and the folder it sits in. */
export function splitName(path: string): { dir: string; base: string } {
  const clean = path.replace(/\/+$/, '');
  const at = clean.lastIndexOf('/');
  return at < 0 ? { dir: '', base: clean } : { dir: clean.slice(0, at), base: clean.slice(at + 1) };
}

/** A standalone document for a sandboxed `srcdoc` iframe: the page as written,
 *  with a base style so an unstyled fragment still reads (scripts never run —
 *  the frame is `sandbox=""`). */
export function htmlDoc(html: string): string {
  const hasHtml = /<html[\s>]/i.test(html);
  if (hasHtml) return html;
  return `<!doctype html><html><head><meta charset="utf-8"><style>body{font:14px/1.5 -apple-system,system-ui,sans-serif;margin:16px;}</style></head><body>${html}</body></html>`;
}
