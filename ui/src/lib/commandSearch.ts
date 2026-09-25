// Command search shared by the ⌘K palette and the floating bar: fuzzy match
// over title + keywords + group, boosted by frecency (how often and how
// recently a command was run). Pure except for the guarded localStorage
// read/write helpers, so the ranking is unit-tested (unit/floatingBar.test.ts).

// The fuzzy matcher lives here (lib/fuzzy.ts re-exports it) so this module has
// no imports and runs under Node's type stripping in unit tests.

export interface FuzzyResult {
  score: number;
  /** indices of matched chars in the haystack (for highlighting) */
  indices: number[];
}

export function fuzzyMatch(needle: string, haystack: string): FuzzyResult | null {
  const n = needle.toLowerCase();
  const h = haystack.toLowerCase();
  if (n.length === 0) return { score: 0, indices: [] };

  let score = 0;
  let hi = 0;
  let prev = -2;
  const indices: number[] = [];

  for (let ni = 0; ni < n.length; ni++) {
    const c = n[ni];
    const found = h.indexOf(c, hi);
    if (found === -1) return null;
    // boundary bonus
    if (found === 0 || h[found - 1] === ' ' || h[found - 1] === '-' || h[found - 1] === '/') {
      score += 8;
    }
    if (found === prev + 1) score += 5; // consecutive
    score -= (found - hi) * 0.5; // gap penalty
    indices.push(found);
    prev = found;
    hi = found + 1;
  }
  // The greedy scan above takes the FIRST occurrence of each letter, so
  // "git" in "Go to Git" matched the G of "Go" and scored as a scattered hit
  // below "Open Git panel". A contiguous run starting on a word boundary is
  // what a person means: score that reading too and keep the better one.
  const word = wordStart(h, n);
  if (word >= 0) {
    const contiguous = 8 + 5 * (n.length - 1) - word * 0.1;
    if (contiguous > score) {
      score = contiguous;
      indices.length = 0;
      for (let i = 0; i < n.length; i++) indices.push(word + i);
    }
  }
  // shorter targets rank higher on equal matches
  score += Math.max(0, 10 - haystack.length * 0.05);
  return { score, indices };
}

const BOUNDARY = ' -/:·';

/** Index of the first occurrence of `n` in `h` that starts a word, or -1. */
function wordStart(h: string, n: string): number {
  let i = h.indexOf(n);
  while (i !== -1) {
    if (i === 0 || BOUNDARY.includes(h[i - 1])) return i;
    i = h.indexOf(n, i + 1);
  }
  return -1;
}

/** The query names a whole word of the title ("git" in "Go to Git"). */
function titleWordHit(title: string, q: string): boolean {
  const t = title.toLowerCase();
  const n = q.toLowerCase();
  const i = wordStart(t, n);
  if (i < 0) return false;
  const end = i + n.length;
  return end === t.length || BOUNDARY.includes(t[end]);
}

/** The fields ranking needs — `Command` (lib/commands.svelte.ts) satisfies it. */
export interface Rankable {
  id: string;
  title: string;
  group?: string;
  keywords?: string;
}

export interface FrecencyEntry {
  count: number;
  lastUsed: number;
}
export type FrecencyMap = Record<string, FrecencyEntry>;

/** Persisted command-usage counts + last-used timestamps (shared by both surfaces). */
export const FRECENCY_KEY = 'otto_palette_frecency';
/** How many rows either surface shows. */
export const RESULT_LIMIT = 14;

const DAY_MS = 1000 * 60 * 60 * 24;

export function loadFrecency(): FrecencyMap {
  try {
    const raw = JSON.parse(localStorage.getItem(FRECENCY_KEY) ?? '{}') as unknown;
    return raw && typeof raw === 'object' && !Array.isArray(raw) ? (raw as FrecencyMap) : {};
  } catch {
    return {};
  }
}

export function recordUsage(id: string, now = Date.now()): void {
  const m = loadFrecency();
  const prev = m[id] ?? { count: 0, lastUsed: 0 };
  m[id] = { count: prev.count + 1, lastUsed: now };
  try {
    localStorage.setItem(FRECENCY_KEY, JSON.stringify(m));
  } catch {
    /* quota / blocked storage — ranking is a convenience */
  }
}

/** Blend frecency into a base fuzzy score. Returns a boost in [0, 20]. */
export function frecencyBoost(id: string, frecency: FrecencyMap, now = Date.now()): number {
  const e = frecency[id];
  if (!e) return 0;
  const countBoost = Math.min(e.count * 1.5, 12); // up to 12
  const recencyBoost = Math.max(0, 8 - (now - e.lastUsed) / DAY_MS); // decays over 8 days
  return countBoost + recencyBoost;
}

export interface Ranked<T> {
  cmd: T;
  score: number;
}

/** Rank commands for `query`. An empty query lists the most-used first. */
export function rankCommands<T extends Rankable>(
  cmds: readonly T[],
  query: string,
  frecency: FrecencyMap,
  now = Date.now(),
  limit = RESULT_LIMIT,
): Ranked<T>[] {
  const q = query.trim();
  if (q === '') {
    return cmds
      .map((cmd) => ({ cmd, score: frecencyBoost(cmd.id, frecency, now) }))
      .sort((a, b) => b.score - a.score)
      .slice(0, limit);
  }
  const out: Ranked<T>[] = [];
  for (const cmd of cmds) {
    const m = fuzzyMatch(q, `${cmd.title} ${cmd.keywords ?? ''} ${cmd.group ?? ''}`);
    if (!m) continue;
    // A query that names a place ("git", "vault") most often means "go there":
    // a whole-word title hit ranks up, and a Navigate one a little more.
    const hit = titleWordHit(cmd.title, q) ? 3 + (cmd.group === 'Navigate' ? 4 : 0) : 0;
    out.push({ cmd, score: m.score + hit + frecencyBoost(cmd.id, frecency, now) });
  }
  return out.sort((a, b) => b.score - a.score).slice(0, limit);
}
