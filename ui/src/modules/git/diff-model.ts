// Pure diff model for DiffViewer: per-file stats, collapse defaults, search,
// split-row pairing, the PR-comment index, and the flattened ROW model the
// windowed renderer draws. Nothing here touches the DOM or Svelte state, and
// every per-hunk cache is a WeakMap keyed by the (immutable, `$state.raw`)
// payload object, so it lives exactly as long as the diff it describes.
import type { DiffLine, FileDiff, Hunk, PrComment } from '../../lib/api/types';

// Huge-diff guard: past this many files or total changed lines, every file
// starts collapsed (not just the big ones) — collapsed headers are near-free.
export const COLLAPSE_ALL_FILES = 40;
export const COLLAPSE_ALL_LINES = 4000;
/** A single file above this many changed lines starts collapsed. */
export const FILE_COLLAPSE_LINES = 400;
/** Matches the server's per-file cap: above it a summary file is shown as
 *  "Large file · Load anyway" instead of fetching a response that would come
 *  back `too_large` anyway. */
export const LARGE_FILE_LINES = 5000;
/** Lines of one hunk put into the row model before a "Show N more lines" row.
 *  The DOM is windowed regardless; the cap bounds the row model (and the work
 *  "Expand all" can trigger) on 50k-line hunks. PR hunks that carry a comment
 *  past the cap are never capped, so "💬 5" always shows its 5 threads. */
export const HUNK_LINE_CAP = 500;

// Lock/generated files start collapsed: nobody reviews a 30k-line lockfile.
const GENERATED_RE =
  /(^|\/)(package-lock\.json|pnpm-lock\.yaml|yarn\.lock|Cargo\.lock|poetry\.lock|Gemfile\.lock|composer\.lock|go\.sum)$|\.min\.(js|css)$|\.(map|snap)$/;
export function isGenerated(path: string): boolean {
  return GENERATED_RE.test(path);
}

export interface Stat {
  add: number;
  del: number;
}
const statCache = new WeakMap<FileDiff, Stat>();
/** Changed-line counts. The server always sends `added`/`deleted` (numstat);
 *  walking the hunks is only the fallback for a provider that leaves them null. */
export function fileStat(f: FileDiff): Stat {
  if (f.added != null && f.deleted != null) return { add: f.added, del: f.deleted };
  let s = statCache.get(f);
  if (s) return s;
  let add = 0;
  let del = 0;
  for (const h of f.hunks) {
    for (const l of h.lines) {
      if (l.origin === 'add') add++;
      else if (l.origin === 'del') del++;
    }
  }
  s = { add, del };
  statCache.set(f, s);
  return s;
}

/** Does this file still need its hunks fetched before it can render? */
export function needsHunks(f: FileDiff): boolean {
  return !!f.hunks_omitted && !f.too_large && !f.is_binary && f.hunks.length === 0;
}

// ── Search ────────────────────────────────────────────────────────────────
// One lowercase blob of a hunk's CHANGED lines, built once per hunk, so a
// search keystroke is one `includes` per hunk instead of a toLowerCase per line.
const hunkText = new WeakMap<Hunk, string>();
function changedText(h: Hunk): string {
  let t = hunkText.get(h);
  if (t === undefined) {
    const parts: string[] = [];
    for (const l of h.lines) if (l.origin !== 'context') parts.push(l.content.toLowerCase());
    t = parts.join('\n');
    hunkText.set(h, t);
  }
  return t;
}
/** Path or changed-line content contains `q` (already lowercased). Content
 *  search covers the hunks at hand — a lazily loaded file matches by path
 *  until its hunks arrive. */
export function fileMatches(f: FileDiff, q: string): boolean {
  if (f.path.toLowerCase().includes(q)) return true;
  if (f.old_path && f.old_path.toLowerCase().includes(q)) return true;
  for (const h of f.hunks) if (changedText(h).includes(q)) return true;
  return false;
}

// ── Side-by-side pairing ──────────────────────────────────────────────────
export interface SplitRow {
  left: DiffLine | null;
  right: DiffLine | null;
}
const splitCache = new WeakMap<Hunk, SplitRow[]>();
/** Context aligns; a del-run pairs with the add-run after it. Computed lazily
 *  on first render of a hunk in split mode and kept by hunk identity. */
export function splitRowsOf(h: Hunk): SplitRow[] {
  let rows = splitCache.get(h);
  if (rows) return rows;
  rows = [];
  const lines = h.lines;
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (l.origin === 'context') {
      rows.push({ left: l, right: l });
      i++;
      continue;
    }
    const dels: DiffLine[] = [];
    const adds: DiffLine[] = [];
    while (i < lines.length && lines[i].origin === 'del') dels.push(lines[i++]);
    while (i < lines.length && lines[i].origin === 'add') adds.push(lines[i++]);
    const n = Math.max(dels.length, adds.length);
    for (let k = 0; k < n; k++) rows.push({ left: dels[k] ?? null, right: adds[k] ?? null });
    if (dels.length === 0 && adds.length === 0) i++; // safety
  }
  splitCache.set(h, rows);
  return rows;
}

// ── PR comments ───────────────────────────────────────────────────────────
export interface FileComments {
  /** Keyed by line number: a comment matches a row by new_line, else old_line. */
  /** Keyed by {@link anchorKey} (`side:line`). */
  anchored: Map<string, PrComment[]>;
  unanchored: PrComment[];
}
/** Key of an inline anchor: the SIDE plus the line number on that side. A
 *  deleted row (old 15) and an added row (new 15) share a displayed number,
 *  so keying by number alone rendered such a comment under both rows. */
export function anchorKey(side: 'old' | 'new', line: number): string {
  return `${side}:${line}`;
}
/** Index every comment by path once (the old renderer re-filtered the whole
 *  comment list for every file on every render). Inline comments key by
 *  `side:line` (no side ⇒ `new`, the forge default); comments with no line
 *  or that the forge marks outdated go to the file-level block. */
export function indexComments(comments: PrComment[]): Map<string, FileComments> {
  const m = new Map<string, FileComments>();
  for (const c of comments) {
    if (c.path === null) continue;
    let fc = m.get(c.path);
    if (!fc) {
      fc = { anchored: new Map(), unanchored: [] };
      m.set(c.path, fc);
    }
    if (c.line === null || c.outdated) fc.unanchored.push(c);
    else {
      const k = anchorKey(c.side ?? 'new', c.line);
      const arr = fc.anchored.get(k);
      if (arr) arr.push(c);
      else fc.anchored.set(k, [c]);
    }
  }
  return m;
}
/** The side a diff row comments on: a deleted row (no new number) is the
 *  old side; added and context rows are the new side. */
export function lineAnchor(l: DiffLine): { side: 'old' | 'new'; line: number } | null {
  if (l.new_line !== null) return { side: 'new', line: l.new_line };
  if (l.old_line !== null) return { side: 'old', line: l.old_line };
  return null;
}
export function commentsForLine(fc: FileComments | undefined, l: DiffLine): PrComment[] | null {
  if (!fc || fc.anchored.size === 0) return null;
  const a = lineAnchor(l);
  return a ? (fc.anchored.get(anchorKey(a.side, a.line)) ?? null) : null;
}

// ── Row model ─────────────────────────────────────────────────────────────
export interface ComposerAt {
  path: string;
  oldLine: number | null;
  newLine: number | null;
  /** The number we post — on `side`. */
  line: number;
  side: 'old' | 'new';
}

interface RowBase {
  /** Stable across rebuilds — measured heights are cached under it. */
  key: string;
  /** The SUMMARY file object (the one in `diff.files`). */
  file: FileDiff;
}
export type Row =
  | (RowBase & { kind: 'file'; collapsed: boolean; first: boolean })
  | (RowBase & { kind: 'note'; text: string })
  | (RowBase & { kind: 'large'; lines: number; canLoad: boolean })
  | (RowBase & { kind: 'pending' })
  | (RowBase & { kind: 'error'; message: string })
  | (RowBase & { kind: 'fcomments'; comments: PrComment[] })
  | (RowBase & { kind: 'hunk'; hi: number; hunk: Hunk; eff: FileDiff })
  | (RowBase & { kind: 'line'; hi: number; li: number; line: DiffLine })
  | (RowBase & { kind: 'split'; hi: number; sr: SplitRow })
  | (RowBase & { kind: 'comment'; hi: number; comments: PrComment[] })
  | (RowBase & { kind: 'composer'; hi: number; at: ComposerAt })
  | (RowBase & { kind: 'more'; hi: number; remaining: number })
  | (RowBase & { kind: 'end' });

/** Rows that live inside a hunk's code block (grouped into one `.dtable`). */
export function inHunk(r: Row): r is Extract<Row, { hi: number }> & Row {
  return r.kind === 'line' || r.kind === 'split' || r.kind === 'comment' || r.kind === 'composer' || r.kind === 'more';
}

export interface FileRowsInput {
  /** Summary file (from `diff.files`). */
  file: FileDiff;
  /** Effective file: the lazily loaded one when present, else `file`. */
  eff: FileDiff;
  collapsed: boolean;
  first: boolean;
  split: boolean;
  fc: FileComments | undefined;
  /** Hunk indices the user uncapped ("Show N more lines"). */
  uncapped: ReadonlySet<number> | undefined;
  composer: ComposerAt | null;
  loadError: string | undefined;
  /** User asked for this too-large file anyway. */
  forceFull: boolean;
  canLoad: boolean;
}

function composerOn(at: ComposerAt | null, l: DiffLine | null): boolean {
  return !!at && !!l && at.oldLine === l.old_line && at.newLine === l.new_line;
}

/** Flatten ONE file into rows. Callers memoize per file on the inputs, so a
 *  toggle / composer / comment change rebuilds one file, not the diff. */
export function buildFileRows(i: FileRowsInput): Row[] {
  const { file, eff } = i;
  const k = `${file.path}\u0000`;
  const rows: Row[] = [{ kind: 'file', key: `${k}f`, file, collapsed: i.collapsed, first: i.first }];
  if (i.collapsed) return rows;
  const stat = fileStat(file);
  const lines = stat.add + stat.del;
  const end = (): Row[] => {
    rows.push({ kind: 'end', key: `${k}e`, file });
    return rows;
  };
  if (eff.is_binary) {
    rows.push({ kind: 'note', key: `${k}n`, file, text: 'Binary file — no text diff.' });
    return end();
  }
  if (i.loadError !== undefined) {
    rows.push({ kind: 'error', key: `${k}x`, file, message: i.loadError });
    return end();
  }
  const pendingOrNote = (): Row[] => {
    if (i.canLoad) rows.push({ kind: 'pending', key: `${k}p`, file });
    else rows.push({ kind: 'note', key: `${k}n`, file, text: 'Diff not loaded for this file.' });
    return end();
  };
  if (eff !== file) {
    // Lazily loaded: a normal fetch may come back over the per-file cap; a
    // `full` fetch past the hard cap is final.
    if (eff.too_large) {
      if (i.forceFull) {
        rows.push({ kind: 'note', key: `${k}n`, file, text: `Too large to display (${lines.toLocaleString()} changed lines).` });
      } else {
        rows.push({ kind: 'large', key: `${k}L`, file, lines, canLoad: i.canLoad });
      }
      return end();
    }
    if (needsHunks(eff)) {
      rows.push({ kind: 'note', key: `${k}n`, file, text: 'No diff available for this file.' });
      return end();
    }
  } else {
    const big = !!file.too_large || (needsHunks(file) && lines > LARGE_FILE_LINES);
    if (big && !i.forceFull) {
      rows.push({ kind: 'large', key: `${k}L`, file, lines, canLoad: i.canLoad });
      return end();
    }
    if (big || needsHunks(file)) return pendingOrNote();
  }
  if (i.fc && i.fc.unanchored.length > 0) {
    rows.push({ kind: 'fcomments', key: `${k}c`, file, comments: i.fc.unanchored });
  }
  for (let hi = 0; hi < eff.hunks.length; hi++) {
    const hunk = eff.hunks[hi];
    rows.push({ kind: 'hunk', key: `${k}h${hi}`, file, hi, hunk, eff });
    const src: (DiffLine | SplitRow)[] = i.split ? splitRowsOf(hunk) : hunk.lines;
    let cap = src.length;
    if (src.length > HUNK_LINE_CAP && !i.uncapped?.has(hi)) {
      cap = HUNK_LINE_CAP;
      // A comment (or the open composer) past the cap lifts it: never hide threads.
      if (i.fc || i.composer) {
        for (let j = HUNK_LINE_CAP; j < src.length; j++) {
          const s = src[j];
          const ls = i.split ? [(s as SplitRow).left, (s as SplitRow).right] : [s as DiffLine];
          if (ls.some((l) => l && (commentsForLine(i.fc, l) || composerOn(i.composer, l)))) {
            cap = src.length;
            break;
          }
        }
      }
    }
    for (let j = 0; j < cap; j++) {
      let cs: PrComment[] | null;
      let comp: boolean;
      if (i.split) {
        const sr = src[j] as SplitRow;
        rows.push({ kind: 'split', key: `${k}s${hi}:${j}`, file, hi, sr });
        const lc = sr.left ? commentsForLine(i.fc, sr.left) : null;
        // A context row puts the same line on both halves — count it once;
        // a del/add pair can carry threads on each side.
        const rc = sr.right && sr.right !== sr.left ? commentsForLine(i.fc, sr.right) : null;
        cs = lc && rc && lc !== rc ? [...lc, ...rc] : (lc ?? rc);
        comp = composerOn(i.composer, sr.left) || composerOn(i.composer, sr.right);
      } else {
        const line = src[j] as DiffLine;
        rows.push({ kind: 'line', key: `${k}l${hi}:${j}`, file, hi, li: j, line });
        cs = commentsForLine(i.fc, line);
        comp = composerOn(i.composer, line);
      }
      if (cs) rows.push({ kind: 'comment', key: `${k}c${hi}:${j}`, file, hi, comments: cs });
      if (comp && i.composer) rows.push({ kind: 'composer', key: `${k}m${hi}:${j}`, file, hi, at: i.composer });
    }
    if (cap < src.length) {
      rows.push({ kind: 'more', key: `${k}r${hi}`, file, hi, remaining: src.length - cap });
    }
  }
  return end();
}

/** First-paint height estimates per kind (px); real heights are measured. */
export function estimateRow(r: Row, mobile: boolean): number {
  switch (r.kind) {
    case 'file':
      return (r.first ? 0 : 10) + (mobile ? 42 : 32);
    case 'hunk':
      return mobile ? 26 : 22;
    case 'line':
    case 'split':
      return mobile ? 20 : 18;
    case 'comment':
      return 96;
    case 'composer':
      return 104;
    case 'fcomments':
      return 120;
    case 'more':
      return mobile ? 44 : 32;
    case 'large':
      return 56;
    case 'end':
      return 1;
    default:
      return 44;
  }
}
