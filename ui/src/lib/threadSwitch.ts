// The ⌘K bar's thread rows, the pure half (paletteThreads.svelte.ts):
// typing filters threads by title and project together, and the order the
// bar opened with holds while it is open. Unit-tested in
// unit/threadsSwitcher.test.ts.
import { fuzzyMatch } from './commandSearch.ts';

/** Best match first (the switcher order breaks ties); everything for ''. */
export function filterThreadRows<T extends { title: string; project: string }>(rows: readonly T[], query: string): T[] {
  const q = query.trim();
  if (!q) return [...rows];
  const scored: { r: T; s: number; i: number }[] = [];
  rows.forEach((r, i) => {
    const m = fuzzyMatch(q, `${r.title} ${r.project}`);
    if (m) scored.push({ r, s: m.score, i });
  });
  return scored.sort((a, b) => b.s - a.s || a.i - b.i).map((x) => x.r);
}

/** Keep the order the bar opened with (a row that moved stays put; one that
 *  appeared goes last; one that went away is dropped). */
export function frozenOrder<T extends { id: string }>(rows: readonly T[], frozen: readonly string[]): T[] {
  const byId = new Map(rows.map((r) => [r.id, r] as const));
  const out: T[] = [];
  for (const id of frozen) {
    const r = byId.get(id);
    if (r) {
      out.push(r);
      byId.delete(id);
    }
  }
  return [...out, ...byId.values()];
}
