// Text helpers behind the file previews (the Workbench preview pane, the shared
// FilePreview used by the IDE and Threads): JSON error positions, SVG data
// URLs, the CSV/TSV table parse, and the sandboxed HTML frame document.
// Import-free so unit tests and the Workbench's kinds.ts load it directly.

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

/** Lightweight CSV/TSV parser for the structured table preview. */
export function parseDelimitedTable(raw: string, delimiter = ',', maxRows = 120): { headers: string[]; rows: string[][] } {
  const lines = raw
    .split(/\r?\n/)
    .map((l) => l.trimEnd())
    .filter((l) => l.length > 0)
    .slice(0, maxRows + 1);
  if (lines.length === 0) return { headers: [], rows: [] };
  const splitLine = (line: string): string[] => {
    const out: string[] = [];
    let cur = '';
    let quoted = false;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (ch === '"') {
        if (quoted && line[i + 1] === '"') {
          cur += '"';
          i++;
        } else {
          quoted = !quoted;
        }
      } else if (ch === delimiter && !quoted) {
        out.push(cur);
        cur = '';
      } else {
        cur += ch;
      }
    }
    out.push(cur);
    return out;
  };
  const headers = splitLine(lines[0]);
  const rows = lines.slice(1).map(splitLine);
  return { headers, rows };
}

/** No network beyond inline/data images, https styles and fonts. With scripts
 *  on, only the page's own inline scripts run (no `src=` fetches, no XHR). */
function previewCsp(scripts: boolean): string {
  const script = scripts ? "; script-src 'unsafe-inline'" : '';
  return `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: blob: https:; style-src 'unsafe-inline' https:; font-src data: https:${script}">`;
}

/** The `srcdoc` of a sandboxed HTML/SVG preview frame: a strict CSP meta first
 *  (a second fence behind the iframe `sandbox`), then the page as written —
 *  or, for `svg`, the markup in a minimal centred page. */
export function buildHtmlPreviewDoc(content: string, opts: { svg?: boolean; scripts?: boolean } = {}): string {
  const csp = previewCsp(Boolean(opts.scripts));
  return opts.svg
    ? `<!doctype html><html><head>${csp}<meta name="color-scheme" content="light"><style>html,body{margin:0;height:100%;background:Canvas}body{display:flex;align-items:center;justify-content:center}svg{max-width:100%;max-height:100%}</style></head><body>${content}</body></html>`
    : `${csp}${content}`;
}
