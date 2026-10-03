// Which pages the main window warms after its first paint (review 06 F5).
// Pure, so it is unit-tested without a DOM (unit/prefetchPlan.test.ts).
//
// Warming used to import AND evaluate all ~31 pages (7.65 MB) on every boot —
// pages RBAC hides, modules the user hid, and on a phone over the network.
// Now: only the modules the user can see, Favorites first, capped; nothing
// at all on a phone, with Save-Data on, or when the daemon isn't local
// (every byte crosses a real network there). Hover/focus prefetch still
// covers anything else the moment the pointer heads for it.

/** How many pages idle prefetch evaluates eagerly. */
export const PREFETCH_CAP = 6;

/** Page keys to warm, in order: the user's Favorites, then the rest of the
 *  visible sidebar order, then Settings — each mapped to its page key through
 *  `pageKeyOf` (Connections → the Database page), deduped, minus `skip` (the
 *  page already on screen, routes with no page) and capped. */
export function prefetchQueue(
  visibleIds: readonly string[],
  favorites: readonly string[],
  pageKeyOf: (id: string) => string | null,
  opts: { cap?: number; skip?: readonly string[] } = {},
): string[] {
  const cap = opts.cap ?? PREFETCH_CAP;
  const skip = new Set(['plugin', 'snip', ...(opts.skip ?? [])]);
  const visible = new Set(visibleIds);
  const ordered = [...favorites.filter((id) => visible.has(id)), ...visibleIds, 'settings'];
  const out: string[] = [];
  for (const id of ordered) {
    if (out.length >= cap) break;
    const key = pageKeyOf(id);
    if (!key || skip.has(key) || out.includes(key)) continue;
    out.push(key);
  }
  return out;
}

/** Hosts whose daemon is on this machine (prefetch costs no network there). */
export function isLoopbackHost(host: string): boolean {
  const h = host.replace(/^\[|\]$/g, '').toLowerCase();
  return h === 'localhost' || h === '::1' || h.startsWith('127.') || h.endsWith('.localhost');
}

/** Whether idle prefetch should run at all. */
export function shouldPrefetch(env: {
  isPhone: boolean;
  saveData?: boolean;
  /** The daemon's base URL (`baseUrl()`), '' = same origin. */
  base: string;
  /** `location.href`, to resolve a same-origin / relative base. */
  href: string;
}): boolean {
  if (env.isPhone || env.saveData) return false;
  try {
    return isLoopbackHost(new URL(env.base || env.href, env.href).hostname);
  } catch {
    return false;
  }
}
