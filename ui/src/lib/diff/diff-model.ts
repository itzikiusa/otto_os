// Pure diff model for DiffViewer: per-file stats, collapse defaults, search,
// split-row pairing, the PR-comment index, and the flattened ROW model the
// windowed renderer draws.
import type { DiffLine, FileDiff, Hunk, PrComment } from '../api/types';

export const COLLAPSE_ALL_FILES = 40;
export const COLLAPSE_ALL_LINES = 4000;
export const FILE_COLLAPSE_LINES = 400;
export const LARGE_FILE_LINES = 5000;
export const HUNK_LINE_CAP = 500;

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

export function needsHunks(f: FileDiff): boolean {
  return !!f.hunks_omitted && !f.too_large && !f.is_binary && f.hunks.length === 0;
}

export function prefetchable(f: FileDiff): boolean {
  if (!needsHunks(f) || isGenerated(f.path)) return false;
  const s = fileStat(f);
  return s.add + s.del <= LARGE_FILE_LINES;
}

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

export function fileMatches(f: FileDiff, q: string): boolean {
  if (f.path.toLowerCase().includes(q)) return true;
  if (f.old_path && f.old_path.toLowerCase().includes(q)) return true;
  for (const h of f.hunks) if (changedText(h).includes(q)) return true;
  return false;
}

export interface SplitRow {
  left: DiffLine | null;
  right: DiffLine | null;
}
const splitCache = new WeakMap<Hunk, SplitRow[]>();
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
    if (dels.length === 0 && adds.length === 0) i++;
  }
  splitCache.set(h, rows);
  return rows;
}

export interface FileComments {
  anchored: Map<string, PrComment[]>;
  unanchored: PrComment[];
}
export function anchorKey(side: 'old' | 'new', line: number): string {
  return `${side}:${line}`;
}
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
export function lineAnchor(l: DiffLine): { side: 'old' | 'new'; line: number } | null {
  if (l.new_line !== null) return { side: 'new', line: l.new_line };
  if (l.old_line !== null) return { side: 'old', line: l.old_line };
  return null;
}
export function orphanComments(fc: FileComments | undefined, f: FileDiff): PrComment[] {
  if (!fc || fc.anchored.size === 0) return [];
  const seen = new Set<string>();
  for (const h of f.hunks) {
    for (const l of h.lines) {
      const a = lineAnchor(l);
      if (!a) continue;
      const k = anchorKey(a.side, a.line);
      if (fc.anchored.has(k)) seen.add(k);
    }
  }
  if (seen.size === fc.anchored.size) return [];
  const out: PrComment[] = [];
  for (const [k, cs] of fc.anchored) if (!seen.has(k)) out.push(...cs);
  return out;
}
export function commentsForLine(fc: FileComments | undefined, l: DiffLine): PrComment[] | null {
  if (!fc || fc.anchored.size === 0) return null;
  const a = lineAnchor(l);
  return a ? (fc.anchored.get(anchorKey(a.side, a.line)) ?? null) : null;
}

export interface ComposerAt {
  path: string;
  oldLine: number | null;
  newLine: number | null;
  line: number;
  side: 'old' | 'new';
}

interface RowBase {
  key: string;
  file: FileDiff;
}
export type Row =
  | (RowBase & { kind: 'file'; collapsed: boolean; first: boolean })
  | (RowBase & { kind: 'note'; text: string })
  | (RowBase & { kind: 'large'; lines: number; canLoad: boolean })
  | (RowBase & { kind: 'pending' })
  | (RowBase & { kind: 'error'; message: string })
  | (RowBase & { kind: 'fcomments'; comments: PrComment[]; notShown?: ReadonlySet<PrComment> })
  | (RowBase & { kind: 'hunk'; hi: number; hunk: Hunk; eff: FileDiff })
  | (RowBase & { kind: 'line'; hi: number; li: number; line: DiffLine })
  | (RowBase & { kind: 'split'; hi: number; sr: SplitRow })
  | (RowBase & { kind: 'comment'; hi: number; comments: PrComment[] })
  | (RowBase & { kind: 'composer'; hi: number; at: ComposerAt })
  | (RowBase & { kind: 'more'; hi: number; remaining: number })
  | (RowBase & { kind: 'end' });

export function inHunk(r: Row): r is Extract<Row, { hi: number }> & Row {
  return r.kind === 'line' || r.kind === 'split' || r.kind === 'comment' || r.kind === 'composer' || r.kind === 'more';
}

export interface FileRowsInput {
  file: FileDiff;
  eff: FileDiff;
  collapsed: boolean;
  first: boolean;
  split: boolean;
  fc: FileComments | undefined;
  uncapped: ReadonlySet<number> | undefined;
  composer: ComposerAt | null;
  loadError: string | undefined;
  forceFull: boolean;
  canLoad: boolean;
}

function composerOn(at: ComposerAt | null, l: DiffLine | null): boolean {
  return !!at && !!l && at.oldLine === l.old_line && at.newLine === l.new_line;
}

export const ROW_KEY_SEP = '\u001f';

export function buildFileRows(i: FileRowsInput): Row[] {
  const { file, eff } = i;
  const k = `${file.path}${ROW_KEY_SEP}`;
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
  const orphans = orphanComments(i.fc, eff);
  if (i.fc && i.fc.unanchored.length + orphans.length > 0) {
    const comments = orphans.length > 0 ? [...i.fc.unanchored, ...orphans] : i.fc.unanchored;
    rows.push({ kind: 'fcomments', key: `${k}c`, file, comments, notShown: orphans.length > 0 ? new Set(orphans) : undefined });
  }
  for (let hi = 0; hi < eff.hunks.length; hi++) {
    const hunk = eff.hunks[hi];
    rows.push({ kind: 'hunk', key: `${k}h${hi}`, file, hi, hunk, eff });
    const src: (DiffLine | SplitRow)[] = i.split ? splitRowsOf(hunk) : hunk.lines;
    let cap = src.length;
    if (src.length > HUNK_LINE_CAP && !i.uncapped?.has(hi)) {
      cap = HUNK_LINE_CAP;
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

export function findTextOf(r: Row): string {
  switch (r.kind) {
    case 'file':
      return r.file.old_path ? `${r.file.old_path} → ${r.file.path}` : r.file.path;
    case 'hunk':
      return r.hunk.header;
    case 'line':
      return r.line.content;
    case 'split':
      return `${r.sr.left?.content ?? ''}\n${r.sr.right?.content ?? ''}`;
    case 'note':
      return r.text;
    case 'error':
      return r.message;
    case 'comment':
    case 'fcomments':
      return r.comments.map((c) => c.body).join('\n');
    default:
      return '';
  }
}
