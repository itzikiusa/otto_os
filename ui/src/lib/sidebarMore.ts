// "Show what he uses": the sidebar lists the person's pinned modules, the ones
// used in the last 30 days (GET /telemetry/navigation, all device classes)
// and any module whose badge needs them; everything else folds under ONE
// "More" row. Until there is usage data the built-in defaults stand in.
// Customize sidebar (edit mode) still lists and controls every module; ⌘K
// "Go to" is untouched. Pure (unit/sidebarMore.test.ts).

/** Shown before any usage is known (Settings has its own footer button). */
export const DEFAULT_SHOWN: readonly string[] = ['agents', 'git', 'connections', 'threads', 'home', 'usage'];

/** The sidebar ids a navigation-count list says were used (count > 0),
 *  mapped onto sidebar entries (`database` → Connections, …) by `navId`. */
export function usedModules(counts: readonly { module: string; count: number }[], navId: (module: string) => string): string[] {
  const out = new Set<string>();
  for (const c of counts) if (c.count > 0 && c.module !== 'other' && c.module !== 'shell') out.add(navId(c.module));
  return [...out];
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
