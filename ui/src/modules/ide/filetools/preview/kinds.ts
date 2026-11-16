// File preview (IDE editor "Preview"): which renderer a language id or file
// extension gets, plus the small pure helpers the renderers share (JSON error
// positions, SVG data URLs). CSV/TSV use the IDE's own table view.
// Framework-free so unit/workbench-preview.test.ts can exercise it directly.

export type PreviewKind = 'markdown' | 'html' | 'svg' | 'mermaid' | 'd2' | 'json';

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
    case 'mermaid':
    case 'mmd':
      return 'mermaid';
    case 'd2':
      return 'd2';
    case 'json':
    case 'jsonc':
    case 'geojson':
      return 'json';
    default:
      return null;
  }
}

/** A file path's extension when the IDE can preview it, else null. */
export function previewLangOf(path: string): string | null {
  const base = path.slice(path.lastIndexOf('/') + 1);
  const ext = base.includes('.') ? base.slice(base.lastIndexOf('.') + 1).toLowerCase() : '';
  return previewKind(ext) ? ext : null;
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

/** SVG text → a data URL for an `<img>` (images never run scripts). */
export function svgDataUrl(svg: string): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
