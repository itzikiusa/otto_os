// Word-level highlighting inside changed lines (DiffViewer). A deleted line
// and the added line that replaces it (the k-th of a del-run against the k-th
// of the add-run after it — the same pairing as the side-by-side view) get a
// token diff; the changed spans are wrapped in `<span class="dw">` on top of
// the syntax-highlighted HTML, so word marks and token colours compose.
//
// Cost stays O(rows on screen): nothing runs until a row renders, every
// result is cached on the (immutable) line / hunk object, and the guards skip
// what isn't worth it — lines over WORD_MAX_LINE chars, token tables over
// WORD_MAX_CELLS, and pairs too dissimilar for word marks to help (those keep
// the whole-line colour, as before).
import type { DiffLine, FileDiff, Hunk } from '../../lib/api/types';

/** [start, end) in UTF-16 code units of the line's text. */
export type WordRange = [number, number];

export const WORD_MAX_LINE = 500;
/** Tokens(a) × tokens(b) above this: no word diff (the LCS table's size). */
export const WORD_MAX_CELLS = 40_000;
/** Below this share of unchanged characters the lines are a rewrite, not an
 *  edit — marking nearly every word says nothing. */
export const WORD_MIN_SAME = 0.35;

// Words (any script's letters/digits/_), whitespace runs, or one other char.
const TOKEN_RE = /[\p{L}\p{N}_]+|\s+|[^\p{L}\p{N}_\s]/gu;

interface Tok {
  s: string;
  at: number;
}
function tokens(text: string): Tok[] {
  const out: Tok[] = [];
  for (const m of text.matchAll(TOKEN_RE)) out.push({ s: m[0], at: m.index ?? 0 });
  return out;
}

/** Changed spans of `a` (the old line) and `b` (the new line), or null when a
 *  word diff isn't worth showing (too long, too big, too different, equal). */
export function wordDiff(a: string, b: string): { a: WordRange[]; b: WordRange[] } | null {
  if (a === b || a.length > WORD_MAX_LINE || b.length > WORD_MAX_LINE) return null;
  const ta = tokens(a);
  const tb = tokens(b);
  const n = ta.length;
  const m = tb.length;
  if (n === 0 || m === 0 || n * m > WORD_MAX_CELLS) return null;
  // LCS lengths, suffix form: L[i][j] = LCS of ta[i..] and tb[j..].
  const w = m + 1;
  const L = new Uint16Array((n + 1) * w);
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      L[i * w + j] = ta[i].s === tb[j].s ? L[(i + 1) * w + j + 1] + 1 : Math.max(L[(i + 1) * w + j], L[i * w + j + 1]);
    }
  }
  const keepA = new Uint8Array(n);
  const keepB = new Uint8Array(m);
  let same = 0;
  for (let i = 0, j = 0; i < n && j < m; ) {
    if (ta[i].s === tb[j].s) {
      keepA[i++] = 1;
      keepB[j++] = 1;
      same += ta[i - 1].s.length;
    } else if (L[(i + 1) * w + j] >= L[i * w + j + 1]) i++;
    else j++;
  }
  if (same / Math.max(a.length, b.length) < WORD_MIN_SAME) return null;
  return { a: spans(a, ta, keepA), b: spans(b, tb, keepB) };
}

/** Changed tokens → merged ranges; only whitespace between two changed
 *  tokens joins them (one mark for "foo bar", not two). */
function spans(text: string, ts: Tok[], keep: Uint8Array): WordRange[] {
  const out: WordRange[] = [];
  for (let i = 0; i < ts.length; i++) {
    if (keep[i]) continue;
    const s = ts[i].at;
    const e = s + ts[i].s.length;
    const last = out[out.length - 1];
    if (last && /^\s*$/.test(text.slice(last[1], s))) last[1] = e;
    else out.push([s, e]);
  }
  return out;
}

// Per hunk: each paired del/add line → its partner.
const pairCache = new WeakMap<Hunk, Map<DiffLine, DiffLine>>();
function pairsOf(h: Hunk): Map<DiffLine, DiffLine> {
  let p = pairCache.get(h);
  if (p) return p;
  p = new Map();
  const lines = h.lines;
  let i = 0;
  while (i < lines.length) {
    if (lines[i].origin !== 'del') {
      i++;
      continue;
    }
    const d0 = i;
    while (i < lines.length && lines[i].origin === 'del') i++;
    const a0 = i;
    while (i < lines.length && lines[i].origin === 'add') i++;
    const k = Math.min(a0 - d0, i - a0);
    for (let x = 0; x < k; x++) {
      p.set(lines[d0 + x], lines[a0 + x]);
      p.set(lines[a0 + x], lines[d0 + x]);
    }
  }
  pairCache.set(h, p);
  return p;
}

const rangeCache = new WeakMap<DiffLine, WordRange[] | null>();
/** The word marks of `line` (a row of hunk `hi` of `f`), or null. */
export function lineWords(f: FileDiff, hi: number, line: DiffLine): WordRange[] | null {
  if (line.origin === 'context') return null;
  const hit = rangeCache.get(line);
  if (hit !== undefined) return hit;
  const h = f.hunks[hi];
  const other = h ? pairsOf(h).get(line) : undefined;
  let mine: WordRange[] | null = null;
  if (other) {
    const del = line.origin === 'del' ? line : other;
    const add = line.origin === 'del' ? other : line;
    const d = wordDiff(del.content, add.content);
    rangeCache.set(del, d && d.a.length ? d.a : null);
    rangeCache.set(add, d && d.b.length ? d.b : null);
    mine = rangeCache.get(line) ?? null;
  }
  rangeCache.set(line, mine);
  return mine;
}

const OPEN = '<span class="dw">';
const CLOSE = '</span>';
const markCache = new WeakMap<WordRange[], { html: string; out: string }>();
/** Wrap the text ranges of `html` (escaped / syntax-highlighted markup of
 *  the line) in word marks. Tags are skipped (a mark is closed before a tag
 *  and reopened after it, so nesting stays valid); an entity counts as the
 *  one character it stands for. */
export function markWords(html: string, ranges: WordRange[] | null): string {
  if (!ranges || ranges.length === 0) return html;
  const hit = markCache.get(ranges);
  if (hit && hit.html === html) return hit.out;
  let out = '';
  let t = 0;
  let ri = 0;
  let open = false;
  for (let i = 0; i < html.length; ) {
    const c = html[i];
    if (c === '<') {
      const j = html.indexOf('>', i);
      const end = j < 0 ? html.length : j + 1;
      out += open ? CLOSE + html.slice(i, end) + OPEN : html.slice(i, end);
      i = end;
      continue;
    }
    while (ri < ranges.length && ranges[ri][1] <= t) ri++;
    if (!open && ri < ranges.length && t >= ranges[ri][0]) {
      out += OPEN;
      open = true;
    }
    let len = 1;
    if (c === '&') {
      const j = html.indexOf(';', i);
      if (j > i && j - i <= 10) len = j - i + 1;
    }
    out += html.slice(i, i + len);
    i += len;
    t++;
    if (open && t >= ranges[ri][1]) {
      out += CLOSE;
      open = false;
      ri++;
    }
  }
  if (open) out += CLOSE;
  markCache.set(ranges, { html, out });
  return out;
}
