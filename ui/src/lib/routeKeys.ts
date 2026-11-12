// Every first route segment the app answers (pure, no imports): the router
// reads it so an unknown route (`#/nope`) is never saved as the window's last
// route or a module's resume point, and the shell's page map
// (shell/pages.svelte.ts) shows "Page not found" for anything else. Kept in
// step with that map by unit/routeGate.test.ts.

/** Routes the shell renders as a page (pages.svelte.ts LOADERS keys). */
export const PAGE_ROUTES: readonly string[] = [
  'agents', 'threads', 'home', 'git', 'database', 'vault', 'workbench', 'api', 'rooms', 'workflows',
  'scheduled-tasks', 'personal-agents', 'loops', 'product', 'design', 'canvas', 'browser', 'mcp',
  'brokers', 'kubernetes', 'aws', 'insights', 'usage', 'skills-eval', 'settings', 'walkthroughs',
  'plugin', 'snip',
];

/** Routes handled outside the page map: the default route, the root App's
 *  own views (share, bar, tray, rooms, pairing), aliases and redirects. */
const OTHER_ROUTES: readonly string[] = ['', 's', 'bar', 'tray', 'room', 'room-host', 'pair', 'connections', 'projects'];

const KNOWN = new Set([...PAGE_ROUTES, ...OTHER_ROUTES]);

/** Whether a route's first segment is one the app answers. */
export function knownRoute(module: string): boolean {
  return KNOWN.has(module);
}

/** {@link knownRoute} for a full hash (`#/git/…`). */
export function knownHash(hash: string): boolean {
  const first = hash.replace(/^#\/?/, '').split(/[/?]/)[0] ?? '';
  let mod = first;
  try {
    mod = decodeURIComponent(first);
  } catch {
    /* malformed escape: compare as typed */
  }
  return knownRoute(mod);
}

/** One-time share / pairing / room views: never a window's last route nor a
 *  module's resume point. */
export function ephemeralHash(hash: string): boolean {
  return hash.startsWith('#/s/') || hash.startsWith('#/pair') || hash.startsWith('#/room/') || hash.startsWith('#/room-host/');
}

/** The module resume points saved for workspace `scope` ('' = none selected)
 *  in the per-window `{scope: {navId: hash}}` record — only routes a page
 *  still answers, never an ephemeral one. Corrupt storage = none. */
export function parseResumePoints(raw: string | null, scope: string): Map<string, string> {
  const m = new Map<string, string>();
  try {
    const all = JSON.parse(raw ?? '{}') as unknown;
    const obj = all && typeof all === 'object' ? (all as Record<string, unknown>)[scope] : null;
    if (obj && typeof obj === 'object') {
      for (const [k, v] of Object.entries(obj as Record<string, unknown>)) {
        if (typeof v === 'string' && v.startsWith('#/') && !ephemeralHash(v) && knownHash(v)) m.set(k, v);
      }
    }
  } catch {
    /* corrupt entry — start fresh */
  }
  return m;
}
