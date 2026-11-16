// Shared file preview (FilePreview.svelte): which renderer a file gets, how its
// relative links/images resolve against the file's folder, and the rules the
// IDE's Editor | Split | Preview control follows. Pure and import-free, so
// unit/filePreview.test.ts loads it directly.

export type FilePreviewKind = 'markdown' | 'html' | 'svg' | 'image' | 'pdf' | 'json' | 'mermaid' | 'csv' | 'tsv' | 'jsonl';
export type FilePreviewMode = 'source' | 'split' | 'preview';

function baseOf(path: string): string {
  const clean = path.split('?')[0].replace(/\/+$/, '');
  return clean.slice(clean.lastIndexOf('/') + 1).toLowerCase();
}

/** The renderer a file previews with, by its name; null = no preview (code). */
export function filePreviewKind(path: string): FilePreviewKind | null {
  const b = baseOf(path);
  if (/\.(md|markdown|mdx)$/.test(b)) return 'markdown';
  if (/\.html?$/.test(b)) return 'html';
  if (b.endsWith('.svg')) return 'svg';
  if (/\.(png|jpe?g|gif|webp|avif|ico|bmp)$/.test(b)) return 'image';
  if (b.endsWith('.pdf')) return 'pdf';
  if (/\.(json|geojson)$/.test(b)) return 'json';
  if (/\.(mmd|mermaid)$/.test(b)) return 'mermaid';
  if (b.endsWith('.csv')) return 'csv';
  if (b.endsWith('.tsv')) return 'tsv';
  if (/\.(jsonl|ndjson)$/.test(b)) return 'jsonl';
  return null;
}

/** Bytes fetched by URL (the editor holds no text for them): always previewed. */
export function isBinaryPreviewKind(kind: FilePreviewKind | null): boolean {
  return kind === 'image' || kind === 'pdf';
}

/** The IDE shows these as its editable table (IdeCsvView) in Preview mode. */
export function isTablePreviewKind(kind: FilePreviewKind | null): boolean {
  return kind === 'csv' || kind === 'tsv' || kind === 'jsonl';
}

/** Threads' file viewer kinds (`threadsHelpers` re-exports these unchanged). */
export type PreviewKind = 'markdown' | 'json' | 'csv' | 'svg' | 'image' | 'code';

/** Rich preview kind supported by Threads' `ThreadFileTabs`. */
export function previewKindForPath(path: string): PreviewKind {
  const lower = path.toLowerCase();
  if (/\.(md|markdown|mdx)$/.test(lower)) return 'markdown';
  if (/\.json$/.test(lower)) return 'json';
  if (/\.(csv|tsv)$/.test(lower)) return 'csv';
  if (/\.svg$/.test(lower)) return 'svg';
  if (/\.(png|jpe?g|gif|webp|avif|ico|bmp)$/.test(lower)) return 'image';
  return 'code';
}

/** The folder holding `path` (`/a/b/c.md` → `/a/b`). */
export function dirOf(path: string): string {
  const clean = path.replace(/\/+$/, '');
  const at = clean.lastIndexOf('/');
  return at <= 0 ? (clean.startsWith('/') ? '/' : '') : clean.slice(0, at);
}

/** A link that leaves the file tree: any scheme (`https:`, `mailto:`, `data:`) or `//host`. */
export function isSchemeHref(href: string): boolean {
  return /^[a-z][a-z0-9+.-]*:/i.test(href.trim()) || /^[/\\]{2}/.test(href.trim());
}

function decodeSafe(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

/** `a/./b/../c` → `a/c`; never climbs above the leading `/`. */
function normalizeSegments(path: string): string {
  const abs = path.startsWith('/');
  const out: string[] = [];
  for (const seg of path.split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') out.pop();
    else out.push(seg);
  }
  return `${abs ? '/' : ''}${out.join('/')}`;
}

/** A relative link/image in a previewed file → the absolute file it names (and
 *  its `#anchor`). `./x`, `../x` and `x` resolve against the file's folder; a
 *  leading `/` means the project root (GitHub's README convention) unless the
 *  href already is an absolute path inside it. Null for external/scheme links
 *  and same-file `#anchor`s (the caller scrolls those). */
export function resolvePreviewHref(fromFile: string, href: string, root?: string | null): { path: string; anchor: string | null } | null {
  const raw = href.trim();
  if (!raw || raw.startsWith('#') || isSchemeHref(raw)) return null;
  const hashAt = raw.indexOf('#');
  const anchor = hashAt >= 0 ? decodeSafe(raw.slice(hashAt + 1)) || null : null;
  const pathPart = decodeSafe((hashAt >= 0 ? raw.slice(0, hashAt) : raw).split('?')[0]);
  if (!pathPart) return null;
  const r = (root ?? '').replace(/\/+$/, '');
  let joined: string;
  if (pathPart.startsWith('/')) joined = r && pathPart !== r && !pathPart.startsWith(`${r}/`) ? `${r}${pathPart}` : pathPart;
  else joined = `${dirOf(fromFile)}/${pathPart}`;
  return { path: normalizeSegments(joined), anchor };
}

/** What a kind opens in before the person picks: images/PDF/SVG and tables
 *  show rendered (CSV/TSV already opened as a table); text formats show source. */
export function defaultPreviewMode(kind: FilePreviewKind | null): FilePreviewMode {
  if (kind === 'image' || kind === 'pdf' || kind === 'svg' || kind === 'csv' || kind === 'tsv') return 'preview';
  return 'source';
}

/** The modes the control offers: none for binary files (always previewed) or
 *  code, Editor | Preview in the stacked phone layout, all three on desktop. */
export function availablePreviewModes(kind: FilePreviewKind | null, stack: boolean): FilePreviewMode[] {
  if (!kind || isBinaryPreviewKind(kind)) return [];
  return stack ? ['source', 'preview'] : ['source', 'split', 'preview'];
}

/** The mode actually shown: binary always previews; no Split when stacked. */
export function effectivePreviewMode(kind: FilePreviewKind | null, saved: FilePreviewMode | undefined, stack: boolean): FilePreviewMode {
  if (!kind) return 'source';
  if (isBinaryPreviewKind(kind)) return 'preview';
  const m = saved ?? defaultPreviewMode(kind);
  return stack && m === 'split' ? 'source' : m;
}

const MODES: readonly FilePreviewMode[] = ['source', 'split', 'preview'];
const KINDS: readonly FilePreviewKind[] = ['markdown', 'html', 'svg', 'image', 'pdf', 'json', 'mermaid', 'csv', 'tsv', 'jsonl'];

/** The remembered mode per kind (localStorage JSON); junk entries are dropped. */
export function parsePreviewModes(raw: string | null | undefined): Partial<Record<FilePreviewKind, FilePreviewMode>> {
  const out: Partial<Record<FilePreviewKind, FilePreviewMode>> = {};
  if (!raw) return out;
  try {
    const v = JSON.parse(raw) as unknown;
    if (!v || typeof v !== 'object') return out;
    for (const [k, m] of Object.entries(v as Record<string, unknown>)) {
      if (KINDS.includes(k as FilePreviewKind) && MODES.includes(m as FilePreviewMode)) out[k as FilePreviewKind] = m as FilePreviewMode;
    }
  } catch {
    /* corrupt value: defaults */
  }
  return out;
}

/** Live preview re-renders this long after the last keystroke. */
export const PREVIEW_DEBOUNCE_MS = 200;
/** Above this many characters a Markdown/HTML/Mermaid preview renders on demand. */
export const PREVIEW_LARGE_CHARS = 1_000_000;

/** How a live preview takes the editor's next text: `skip` while it isn't
 *  shown or the window is hidden (it catches up when visible again), `now` for
 *  another file or the first show (`shownPath` empty), `debounce` while the
 *  person types, `same` when nothing changed. */
export function previewTextUpdate(
  shownPath: string,
  shownText: string,
  nextPath: string,
  nextText: string,
  opts: { shown: boolean; visible: boolean },
): 'skip' | 'now' | 'debounce' | 'same' {
  if (!opts.shown || !opts.visible) return 'skip';
  if (nextPath !== shownPath) return 'now';
  return nextText === shownText ? 'same' : 'debounce';
}

/** Whether a document is too big to re-render on every pause in typing. */
export function previewNeedsManualRender(kind: FilePreviewKind | null, length: number): boolean {
  return (kind === 'markdown' || kind === 'html' || kind === 'mermaid' || kind === 'svg') && length > PREVIEW_LARGE_CHARS;
}

/** The `sandbox` attribute for an HTML preview frame. Never `allow-same-origin`
 *  (the page would share Otto's origin: its storage, its API token), never
 *  top navigation, forms or popups — at most its own inline scripts. */
export function htmlSandboxFor(scripts: boolean): string {
  return scripts ? 'allow-scripts' : '';
}

/** 0-based index of the Markdown heading the 1-based `line` sits under (ATX
 *  `#` headings and setext underlines, outside fences and front matter), or
 *  -1 before the first one. The Split view scrolls the preview to the same
 *  heading ordinal, which survives any id/slug collision. */
export function headingOrdinalAtLine(md: string, line: number): number {
  const lines = md.split('\n');
  let i = 0;
  if (/^---\s*$/.test(lines[0] ?? '')) {
    const end = lines.findIndex((l, n) => n > 0 && /^(---|\.\.\.)\s*$/.test(l));
    if (end > 0) i = end + 1;
  }
  let fence: string | null = null;
  let ordinal = -1;
  for (; i < lines.length && i < line; i++) {
    const t = lines[i];
    const f = /^ {0,3}(`{3,}|~{3,})/.exec(t);
    if (fence) {
      if (f && f[1][0] === fence[0] && f[1].length >= fence.length) fence = null;
      continue;
    }
    if (f) {
      fence = f[1];
      continue;
    }
    if (/^ {0,3}#{1,6}(\s|$)/.test(t)) ordinal++;
    else if (/^ {0,3}(=+|-+)\s*$/.test(t) && i > 0) {
      const prev = lines[i - 1];
      // A setext heading underlines a paragraph line; `---` after a blank line is a rule.
      if (prev.trim() && !/^ {0,3}(#|[-*+>]\s|\d+[.)]\s|`{3}|~{3})/.test(prev)) ordinal++;
    }
  }
  return ordinal;
}
