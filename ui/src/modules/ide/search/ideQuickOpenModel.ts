// Pure rules of the IDE quick-open (⌘P / ⇧⇧ / ⌘G / ⌘E): query parsing, the empty
// query's recent-files list, and the "All" merge that drops a text hit already
// shown as a symbol. Type-only imports, so Node runs it directly in unit tests.

/** The fields the merge reads; both the wire type and the store's own row satisfy it. */
interface SearchHitLike {
  kind: string;
  path: string;
  line?: number | null;
}

export type QuickOpenTab = 'all' | 'files' | 'symbols' | 'text' | 'actions';

export interface QuickOpenQuery {
  /** The text sent to the search endpoint ('' = nothing to search). */
  cleanQuery: string;
  line: number | null;
  col: number | null;
  /** `:42` / `:42:7` / `:` — a line in the ACTIVE file, no search at all. */
  lineOnly: boolean;
}

const LINE_ONLY_RE = /^:(\d*)(?::(\d*))?$/;
const FILE_LINE_RE = /^(.+?):(\d+)(?::(\d+))?$/;

function positive(raw: string | undefined): number | null {
  const n = raw ? Number.parseInt(raw, 10) : NaN;
  return Number.isFinite(n) && n > 0 ? n : null;
}

/** `:42` → line 42 of the active file; `main.go:42:7` → that file, line 42, col 7
 *  (All / Files only — Symbols and Find in path search the text as typed). */
export function parseQuickOpenQuery(tab: QuickOpenTab, query: string): QuickOpenQuery {
  if (tab !== 'all' && tab !== 'files') return { cleanQuery: query, line: null, col: null, lineOnly: false };
  const q = query.trim();
  const lineOnly = LINE_ONLY_RE.exec(q);
  if (lineOnly) return { cleanQuery: '', line: positive(lineOnly[1]), col: positive(lineOnly[2]), lineOnly: true };
  const fileLine = FILE_LINE_RE.exec(q);
  if (fileLine) return { cleanQuery: fileLine[1].trim(), line: positive(fileLine[2]), col: positive(fileLine[3]), lineOnly: false };
  return { cleanQuery: q, line: null, col: null, lineOnly: false };
}

/** In "All", a declaration line comes back twice — as the symbol and as a text hit
 *  on the same path:line. Keep the symbol (it says what the thing is). */
export function dropTextHitsShownAsSymbols<T extends SearchHitLike>(items: readonly T[]): T[] {
  const symbolLines = new Set<string>();
  for (const it of items) if (it.kind === 'symbol' && it.line) symbolLines.add(`${it.path}:${it.line}`);
  if (symbolLines.size === 0) return [...items];
  return items.filter((it) => it.kind !== 'text' || !it.line || !symbolLines.has(`${it.path}:${it.line}`));
}

export interface RecentFileRow {
  path: string;
  source: 'recent' | 'open';
}

/** The empty query: recently opened files (newest first), then the other open tabs,
 *  each path once, without the file you are already in. */
export function recentFileRows(
  recentPaths: readonly string[],
  openTabPaths: readonly string[],
  activePath: string | null,
  limit = 30,
): RecentFileRow[] {
  const seen = new Set<string>(activePath ? [activePath] : []);
  const out: RecentFileRow[] = [];
  const push = (path: string, source: RecentFileRow['source']): void => {
    if (!path || seen.has(path) || out.length >= limit) return;
    seen.add(path);
    out.push({ path, source });
  };
  for (const p of recentPaths) push(p, 'recent');
  for (const p of openTabPaths) push(p, 'open');
  return out;
}

/** `notes/2026/a.md` → `notes/2026/`; a root file → '' (the row shows just its name). */
export function parentFolderOf(relPath: string): string {
  const clean = relPath.replace(/\/+$/, '');
  const i = clean.lastIndexOf('/');
  return i > 0 ? clean.slice(0, i + 1) : '';
}

/** Find in path's file mask: `*.go, *.ts, !*_test.go`. A pattern with a `/` matches
 *  the project-relative path, otherwise the file name. */
export function matchesFileMask(baseName: string, relPath: string, rawMask: string): boolean {
  const patterns = rawMask.split(',').map((p) => p.trim()).filter(Boolean);
  if (patterns.length === 0) return true;
  let included = false;
  let hasPositive = false;
  for (const pat of patterns) {
    const neg = pat.startsWith('!');
    const clean = neg ? pat.slice(1).trim() : pat;
    if (!clean) continue;
    if (!neg) hasPositive = true;
    const escaped = clean.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.');
    const ok = new RegExp(`^${escaped}$`, 'i').test(clean.includes('/') ? relPath : baseName);
    if (neg && ok) return false;
    if (!neg && ok) included = true;
  }
  return !hasPositive || included;
}
