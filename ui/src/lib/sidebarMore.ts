// "Show what he uses": the sidebar lists the person's pinned modules, the ones
// used in the last 30 days (GET /telemetry/navigation, all device classes)
// and any module whose badge needs them; everything else folds under ONE
// "More" row. Until there is usage data the built-in defaults stand in.
// Customize sidebar (edit mode) still lists and controls every module; ⌘K
// "Go to" is untouched. Pure (unit/sidebarMore.test.ts).

/** Shown before any usage is known (Settings has its own footer button). */
export const DEFAULT_SHOWN: readonly string[] = ['agents', 'git', 'connections', 'threads', 'home', 'usage'];

/** The sidebar ids a navigation-count list says were used (count > 0),
 *  mapped onto sidebar entries (`database` → Connections, …) by `navId`,
 *  most used first — on the phone first, then overall (the phone bar's order). */
export function usedModules(counts: readonly { module: string; count: number; device?: string }[], navId: (module: string) => string): string[] {
  const tally = new Map<string, { phone: number; all: number }>();
  for (const c of counts) {
    if (c.count <= 0 || c.module === 'other' || c.module === 'shell') continue;
    const t = tally.get(navId(c.module)) ?? { phone: 0, all: 0 };
    t.all += c.count;
    if (c.device === 'phone') t.phone += c.count;
    tally.set(navId(c.module), t);
  }
  return [...tally].sort(([, a], [, b]) => b.phone - a.phone || b.all - a.all).map(([id]) => id);
}

/** The phone bar's order: pinned first, then the most used (`used`, ranked;
 *  the defaults until there is usage), then the rest in sidebar order. */
export function barOrder<M extends { id: string }>(modules: readonly M[], favorites: readonly string[], used: readonly string[] | null): M[] {
  const rank = [...favorites, ...(used && used.length > 0 ? used : DEFAULT_SHOWN)];
  const pos = (id: string): number => (rank.includes(id) ? rank.indexOf(id) : rank.length);
  return [...modules].sort((a, b) => pos(a.id) - pos(b.id));
}

/** Which module ids stay in the sidebar: pinned ∪ (used, else the defaults)
 *  ∪ badged ∪ the page you are on. */
export function shownModules(o: { favorites: readonly string[]; used: readonly string[] | null; badged: readonly string[]; active: string }): Set<string> {
  return new Set([...o.favorites, ...(o.used && o.used.length > 0 ? o.used : DEFAULT_SHOWN), ...o.badged, o.active]);
}

/** Sections keep only the shown modules (an emptied section drops out); the
 *  rest, in sidebar order, go under More. */
export function splitMore<M extends { id: string }, S extends { modules: M[] }>(sections: readonly S[], shown: ReadonlySet<string>): { sections: S[]; more: M[] } {
  const more: M[] = [];
  const kept: S[] = [];
  for (const sec of sections) {
    const modules = sec.modules.filter((m) => shown.has(m.id) || (more.push(m), false));
    if (modules.length > 0) kept.push({ ...sec, modules });
  }
  return { sections: kept, more };
}
