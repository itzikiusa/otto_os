// The ⌘K bar's thread rows, the pure half (paletteThreads.svelte.ts):
// typing filters threads by title and project together, and the order the
// bar opened with holds while it is open. Unit-tested in
// unit/threadsSwitcher.test.ts.
import { fuzzyMatch } from './commandSearch.ts';
import { looksLikeCommand } from './floatingBar.ts';

/** Best match first (the switcher order breaks ties); everything for ''.
 *  A row counts only when the query names it — every query word starts a
 *  word of its title or project — so "git" never puts a thread whose title
 *  merely contains g…i…t above "Go to Git". */
export function filterThreadRows<T extends { title: string; project: string }>(rows: readonly T[], query: string): T[] {
  const q = query.trim();
  if (!q) return [...rows];
  const scored: { r: T; s: number; i: number }[] = [];
  rows.forEach((r, i) => {
    const text = `${r.title} ${r.project}`;
    if (looksLikeCommand(q, text)) scored.push({ r, s: fuzzyMatch(q, text)?.score ?? 0, i });
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
