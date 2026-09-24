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
  // shorter targets rank higher on equal matches
  score += Math.max(0, 10 - haystack.length * 0.05);
  return { score, indices };
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
    if (m) out.push({ cmd, score: m.score + frecencyBoost(cmd.id, frecency, now) });
  }
  return out.sort((a, b) => b.score - a.score).slice(0, limit);
}
