// Side by side: pure helpers for the split of the main content column into a
// primary pane and a SIDE pane (stores/sidePane.svelte.ts, shell/SidePane.svelte).
//
// How it works: modules read the one global hash router, so two modules can't
// share a document. The side pane is therefore a same-origin <iframe> of the
// app at `?embed=1#/<route>` — its own router, stores and event socket, the
// same auth (localStorage). The two documents talk over `postMessage` using
// the {@link SideMsg} protocol below; everything here is pure (no Svelte, no
// DOM, no runtime imports) so it runs under `node --test` (unit/sidePane.test.ts).
//
// One rule keeps the panes honest: a module lives in ONE pane. A navigation
// (in either document) to a route whose {@link paneKey} the OTHER pane shows
// is handed to that pane instead of opening the module twice.

/** `?embed=1`: this document is the side pane inside the main window. */
export function isEmbedSearch(search: string): boolean {
  try {
    return new URLSearchParams(search).get('embed') === '1';
  } catch {
    return false;
  }
}

/** `#/git/x`, `/git/x`, `git/x` → `git/x` (what `router.go` takes). */
export function routeOf(hashOrRoute: string): string {
  return hashOrRoute.replace(/^#?\/?/, '');
}

/**
 * The pane-ownership key of a route: the sidebar entry it belongs to. Mirrors
 * `activeNavId` (lib/sidebar.ts — a parity test pins them together) without
 * importing it, so this file stays dependency-free: the default route is
 * Agents, `plugin/<slug>` is per plugin, the Database / Message Brokers views
 * belong to Connections and Canvas to Design Hall.
 */
export function paneKey(route: string): string {
  const parts = routeOf(route).split(/[/?]/);
  const mod = parts[0] ?? '';
  if (mod === '') return 'agents';
  if (mod === 'plugin') return `plugin/${parts[1] ?? ''}`;
  if (mod === 'database' || mod === 'brokers') return 'connections';
  if (mod === 'canvas') return 'design';
  return mod;
}

// ─── Split geometry ─────────────────────────────────────────────────────────

/** The side pane's share of the split, as a fraction of the content width. */
export const SPLIT_MIN = 0.25;
export const SPLIT_MAX = 0.75;
export const SPLIT_DEFAULT = 0.5;
/** Neither pane goes narrower than this; below 2× it the split can't show. */
export const PANE_MIN_PX = 360;
/** Arrow-key step on the divider (⇧ for the big step), as fractions. */
export const SPLIT_STEP = 0.02;
export const SPLIT_STEP_BIG = 0.1;

/** Which side of the primary pane the side pane sits on (logical: mirrored in RTL). */
export type Placement = 'trailing' | 'leading';

/** Clamp a split fraction to [SPLIT_MIN, SPLIT_MAX] — and, given the content
 *  width, so neither pane drops under `minPx`. Garbage → the default. */
export function clampShare(share: number, widthPx?: number, minPx = PANE_MIN_PX): number {
  const v = Number.isFinite(share) ? share : SPLIT_DEFAULT;
  let lo = SPLIT_MIN;
  let hi = SPLIT_MAX;
  if (widthPx && widthPx > 0) {
    const f = minPx / widthPx;
    if (f < 0.5) {
      lo = Math.max(lo, f);
      hi = Math.min(hi, 1 - f);
    } else {
      lo = hi = 0.5;
    }
  }
  return Math.round(Math.min(hi, Math.max(lo, v)) * 1000) / 1000;
}

/** Two panes fit side by side in this content width. */
export function fitsTwoPanes(widthPx: number, minPx = PANE_MIN_PX): boolean {
  return widthPx >= minPx * 2 + 1;
}

/** The LEADING pane's fraction (what the divider's aria-valuenow reports). */
export function leadingShare(sideShare: number, placement: Placement): number {
  return placement === 'leading' ? sideShare : 1 - sideShare;
}

/** Inverse of {@link leadingShare}: the side pane's fraction for a divider position. */
export function sideShareFor(leading: number, placement: Placement): number {
  return placement === 'leading' ? leading : 1 - leading;
}

/** Pointer x → leading fraction of a container spanning [left, right]
 *  (logical: in RTL the leading pane is on the right). */
export function leadingFromPointer(x: number, left: number, right: number, rtl = false): number {
  const w = right - left;
  if (w <= 0) return SPLIT_DEFAULT;
  return rtl ? (right - x) / w : (x - left) / w;
}

/**
 * Keyboard on the focused divider (role="separator"): ←/→ move it by a step
 * (⇧ = big step) — visually, so the fraction flips in RTL — Home/End jump to
 * the limits, Enter resets to 50/50. Returns the new LEADING fraction, or null
 * when the key isn't the divider's.
 */
export function nudgeLeading(
  leading: number,
  key: string,
  opts: { shift?: boolean; rtl?: boolean } = {},
): number | null {
  const step = opts.shift ? SPLIT_STEP_BIG : SPLIT_STEP;
  const dir = opts.rtl ? -1 : 1;
  switch (key) {
    case 'ArrowLeft':
      return leading - step * dir;
    case 'ArrowRight':
      return leading + step * dir;
    case 'Home':
      return SPLIT_MIN;
    case 'End':
      return SPLIT_MAX;
    case 'Enter':
      return SPLIT_DEFAULT;
    default:
      return null;
  }
}

// ─── Persistence (per device) ───────────────────────────────────────────────

export const SIDE_PANE_KEY = 'otto_side_pane';

export interface SidePaneSaved {
  /** The side pane's route (`connections/<id>`), null when no pane is open. */
  route: string | null;
  share: number;
  placement: Placement;
}

export const SIDE_PANE_DEFAULTS: SidePaneSaved = {
  route: null,
  share: SPLIT_DEFAULT,
  placement: 'trailing',
};

/** A route we'll restore into the side pane: a plain module path. Never the
 *  one-time share view (`s/…`), the snip editor or a chromeless window route. */
export function restorableRoute(route: unknown): string | null {
  if (typeof route !== 'string') return null;
  const r = routeOf(route.trim());
  if (r === '' || r.length > 512) return null;
  const mod = r.split('/')[0];
  if (['s', 'snip', 'bar', 'tray'].includes(mod)) return null;
  return r;
}

export function parseSaved(raw: string | null | undefined): SidePaneSaved {
  if (!raw) return { ...SIDE_PANE_DEFAULTS };
  try {
    const v = JSON.parse(raw) as Partial<Record<keyof SidePaneSaved, unknown>> | null;
    if (!v || typeof v !== 'object') return { ...SIDE_PANE_DEFAULTS };
    return {
      route: restorableRoute(v.route),
      share: clampShare(typeof v.share === 'number' ? v.share : SPLIT_DEFAULT),
      placement: v.placement === 'leading' ? 'leading' : 'trailing',
    };
  } catch {
    return { ...SIDE_PANE_DEFAULTS };
  }
}

export function serializeSaved(s: SidePaneSaved): string {
  return JSON.stringify({ route: restorableRoute(s.route), share: clampShare(s.share), placement: s.placement });
}

/** The side pane iframe's URL: this page with `embed=1` (and without the
 *  window-identity / pop-out flags, which belong to the host window), at `route`. */
export function embedSrc(loc: { pathname: string; search: string }, route: string): string {
  const q = new URLSearchParams(loc.search);
  q.delete('popout');
  q.delete('win');
  q.set('embed', '1');
  return `${loc.pathname || '/'}?${q.toString()}#/${routeOf(route)}`;
}

// ─── Keys and native menu items, per pane ───────────────────────────────────

export type EmbedKeyTarget = 'local' | 'host' | 'close-pane';

/** Always the focused pane's own business. */
const PANE_LOCAL = new Set(['find', 'navBack', 'navForward', 'termZoomIn', 'termZoomOut', 'termZoomReset']);
/** Session verbs: the side pane's own while it shows Agents (its tabs). */
const SESSION_VERBS = new Set([
  'closeTab',
  'reopenTab',
  'nextTab',
  'prevTab',
  'nextSession',
  'prevSession',
  'jumpSession',
  'splitVertical',
  'splitHorizontal',
  'toggleRight',
]);

/**
 * Where a key-map action pressed INSIDE the side pane runs. Window-level verbs
 * (⌘K, ⌘I, ⌘T, ⌘1, ⌘,, zoom, broadcast, snip…) go to the host window, so the
 * pane never grows a second palette or sheet; find, history and terminal zoom
 * stay in the pane; session verbs stay in the pane while it shows Agents.
 * ⌘W anywhere else closes the pane — it's what you're looking at.
 */
export function embeddedKeyTarget(action: string, paneModuleKey: string): EmbedKeyTarget {
  if (PANE_LOCAL.has(action)) return 'local';
  if (SESSION_VERBS.has(action)) {
    if (paneModuleKey === 'agents') return 'local';
    return action === 'closeTab' ? 'close-pane' : 'host';
  }
  return 'host';
}

export type MenuTarget = 'main' | 'side' | 'close-pane';

/**
 * A native menu item (the Rust shell emits it to the WINDOW) while the side
 * pane has focus: Select All acts on the pane's document; Close Tab and the
 * session items act on the pane's session tabs while it shows Agents, and
 * Close Tab closes the pane otherwise. Everything else is the window's.
 */
export function sideMenuTarget(id: string, sideFocused: boolean, paneModuleKey: string | null): MenuTarget {
  if (!sideFocused || !paneModuleKey) return 'main';
  if (id === 'select-all') return 'side';
  if (id === 'close-tab') return paneModuleKey === 'agents' ? 'side' : 'close-pane';
  if (id === 'session-restart' || id === 'session-kill') return paneModuleKey === 'agents' ? 'side' : 'main';
  return 'main';
}

// ─── postMessage protocol ───────────────────────────────────────────────────

export const SIDE_NS = 'otto-side';

/** A ⌘K command the side pane registered, mirrored into the host's palette. */
export interface RemoteCommand {
  id: string;
  title: string;
  group?: string;
  detail?: string;
  keywords?: string;
  shortcut?: string;
}

/** Side pane → host. */
export type GuestMsg =
  | { ns: typeof SIDE_NS; type: 'ready'; route: string }
  | { ns: typeof SIDE_NS; type: 'route'; route: string }
  | { ns: typeof SIDE_NS; type: 'open-in-main'; route: string }
  | { ns: typeof SIDE_NS; type: 'key'; action: string; index?: number }
  | { ns: typeof SIDE_NS; type: 'close' }
  | { ns: typeof SIDE_NS; type: 'swap' }
  | { ns: typeof SIDE_NS; type: 'promote' }
  | { ns: typeof SIDE_NS; type: 'focus' }
  | { ns: typeof SIDE_NS; type: 'workspace'; id: string }
  | { ns: typeof SIDE_NS; type: 'commands'; list: RemoteCommand[] }
  | { ns: typeof SIDE_NS; type: 'open-external'; url: string };

/** Host → side pane. */
export type HostMsg =
  | { ns: typeof SIDE_NS; type: 'navigate'; route: string }
  | { ns: typeof SIDE_NS; type: 'host'; primary: string; padTraffic: boolean }
  | { ns: typeof SIDE_NS; type: 'workspace'; id: string }
  | { ns: typeof SIDE_NS; type: 'run-command'; id: string }
  | { ns: typeof SIDE_NS; type: 'menu'; id: string };

type Obj = Record<string, unknown>;
const str = (v: unknown, max = 2048): v is string => typeof v === 'string' && v.length <= max;

function sideObj(data: unknown): Obj | null {
  if (!data || typeof data !== 'object') return null;
  const o = data as Obj;
  return o.ns === SIDE_NS && typeof o.type === 'string' ? o : null;
}

function readCommand(v: unknown): RemoteCommand | null {
  if (!v || typeof v !== 'object') return null;
  const c = v as Obj;
  if (!str(c.id, 256) || !str(c.title, 256)) return null;
  const opt = (k: string): string | undefined => (str(c[k], 512) ? (c[k] as string) : undefined);
  return { id: c.id, title: c.title, group: opt('group'), detail: opt('detail'), keywords: opt('keywords'), shortcut: opt('shortcut') };
}

/** Validate a message the HOST received (from its side pane frame). */
export function readGuestMsg(data: unknown): GuestMsg | null {
  const o = sideObj(data);
  if (!o) return null;
  const ns = SIDE_NS;
  switch (o.type) {
    case 'ready':
    case 'route':
    case 'open-in-main':
      return str(o.route) ? { ns, type: o.type, route: routeOf(o.route) } : null;
    case 'key':
      if (!str(o.action, 64)) return null;
      return typeof o.index === 'number' && Number.isInteger(o.index)
        ? { ns, type: 'key', action: o.action, index: o.index }
        : { ns, type: 'key', action: o.action };
    case 'close':
    case 'swap':
    case 'promote':
    case 'focus':
      return { ns, type: o.type };
    case 'workspace':
      return str(o.id, 256) ? { ns, type: 'workspace', id: o.id } : null;
    case 'commands':
      if (!Array.isArray(o.list)) return null;
      return {
        ns,
        type: 'commands',
        list: o.list.slice(0, 2000).map(readCommand).filter((c): c is RemoteCommand => c !== null),
      };
    case 'open-external':
      return str(o.url) && /^https?:\/\//i.test(o.url) ? { ns, type: 'open-external', url: o.url } : null;
    default:
      return null;
  }
}

/** Validate a message the SIDE PANE received (from its host). */
export function readHostMsg(data: unknown): HostMsg | null {
  const o = sideObj(data);
  if (!o) return null;
  const ns = SIDE_NS;
  switch (o.type) {
    case 'navigate':
      return str(o.route) ? { ns, type: 'navigate', route: routeOf(o.route) } : null;
    case 'host':
      return str(o.primary, 256) ? { ns, type: 'host', primary: o.primary, padTraffic: o.padTraffic === true } : null;
    case 'workspace':
      return str(o.id, 256) ? { ns, type: 'workspace', id: o.id } : null;
    case 'run-command':
      return str(o.id, 256) ? { ns, type: 'run-command', id: o.id } : null;
    case 'menu':
      return str(o.id, 64) ? { ns, type: 'menu', id: o.id } : null;
    default:
      return null;
  }
}

/** `postMessage` target origin for our own documents: the real origin when
 *  the scheme has one, `*` for an opaque (`null`) origin. */
export function targetOrigin(origin: string | null | undefined): string {
  return origin && origin !== 'null' ? origin : '*';
}
