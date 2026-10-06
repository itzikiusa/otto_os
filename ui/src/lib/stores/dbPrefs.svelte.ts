// Database Explorer preferences that Settings → Appearance also toggles.
//
// They used to live on the 4.4k-line `database` store, so opening Settings
// statically pulled that whole store (plus its import-time side effects:
// announceModule, clipboard guard, page listeners) into the Settings chunk for
// two checkboxes. This module is tiny and import-free: both the store and
// Appearance read it, and the store subscribes (`onKeepAliveChange`) to start
// or stop its keep-alive poller when the toggle flips — only once the store
// itself has loaded, which is the only time a poller can exist.
//
// The storage keys are UNCHANGED from the store-era ones, so saved prefs
// survive the move.

/** "Connect on click only" for restored tabs (see `warmRestored`). */
export const WARM_ON_CLICK_KEY = 'otto_db_warm_on_click';
/** "Keep open connections alive" (see `keepAlive`). */
export const KEEP_ALIVE_KEY = 'otto_db_keep_alive';

/** Read a sticky boolean preference; missing/unreadable falls back to `def`. */
export function loadFlag(key: string, def: boolean): boolean {
  try {
    if (typeof localStorage === 'undefined') return def;
    const v = localStorage.getItem(key);
    return v === null ? def : v === '1';
  } catch {
    return def;
  }
}

/** Persist a sticky boolean. Quota/private-mode failures are never fatal. */
export function saveFlag(key: string, on: boolean): void {
  try {
    localStorage.setItem(key, on ? '1' : '0');
  } catch {
    /* preference-only — losing it must not break the view */
  }
}

export type WarmMode = 'background' | 'on-click';

class DbPrefs {
  /**
   * Restored tabs other than the active one: `background` (default) connects
   * them 3 at a time behind the active tab; `on-click` leaves them "Not
   * connected yet" until the user opens one. Persisted.
   */
  warmRestored: WarmMode = $state(loadFlag(WARM_ON_CLICK_KEY, false) ? 'on-click' : 'background');
  /** Ping open, ready connections every 4 min so pools/tunnels don't idle out
   *  and a dropped one turns red BEFORE the next query. Persisted, default on. */
  keepAlive: boolean = $state(loadFlag(KEEP_ALIVE_KEY, true));

  private keepAliveListeners: ((on: boolean) => void)[] = [];

  setWarmRestored(mode: WarmMode): void {
    this.warmRestored = mode;
    saveFlag(WARM_ON_CLICK_KEY, mode === 'on-click');
  }

  setKeepAlive(on: boolean): void {
    this.keepAlive = on;
    saveFlag(KEEP_ALIVE_KEY, on);
    for (const fn of this.keepAliveListeners) fn(on);
  }

  /** Called on every `setKeepAlive` (the database store's poller start/stop). */
  onKeepAliveChange(fn: (on: boolean) => void): () => void {
    this.keepAliveListeners.push(fn);
    return () => {
      this.keepAliveListeners = this.keepAliveListeners.filter((f) => f !== fn);
    };
  }
}

export const dbPrefs = new DbPrefs();
