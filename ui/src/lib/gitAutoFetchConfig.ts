// The Git page's auto-fetch policy (split out of stores/git.svelte.ts): the
// cadences, the focus gate and the per-device toggle. Pure — no Svelte state.

/** localStorage key for the auto-fetch toggle/interval (per-device). */
export const AUTO_FETCH_KEY = 'otto_git_auto_fetch';
export const DEFAULT_AUTO_FETCH_SEC = 120;
/** The selected repo's cadence. 30 s made `POST /repos/{id}/fetch` the app's
 *  biggest wall-time consumer (~6/min, p95 3 s, telemetry 2026-10-05). */
export const ACTIVE_AUTO_FETCH_SEC = 60;
/** Background tabs are swept only while recently used (opened, selected or
 *  manually fetched); an idle tab resumes on its next activation. */
export const RECENT_USE_MS = 15 * 60_000;

/** May an auto-fetch round run right now?
 *
 *  `document.hidden` alone is not enough: Otto is multi-window, and a BACKGROUND
 *  window is fully "visible" to the browser, so a second window parked on the
 *  Git page kept firing a fetch per open repo every 10 s forever — each one a
 *  `git fetch --prune` + `status` spawn plus a synchronous Keychain read on the
 *  daemon's runtime, which is load the window in front pays for (investigation
 *  H4/WP3). So we also require FOCUS: exactly the window the user is looking at
 *  polls. `hasFocus` is absent in SSR/jsdom — treat that as "allowed" so tests
 *  and headless renders behave as before. */
export function autoFetchAllowed(): boolean {
  if (typeof document === 'undefined') return true;
  if (document.hidden) return false;
  return typeof document.hasFocus === 'function' ? document.hasFocus() : true;
}

/** Read the persisted auto-fetch config, guarded for SSR/test environments with
 *  no localStorage. Defaults: enabled, every two minutes (selected repo every 60 s). */
export function readAutoFetchConfig(): { enabled: boolean; intervalSec: number } {
  const def = { enabled: true, intervalSec: DEFAULT_AUTO_FETCH_SEC };
  if (typeof localStorage === 'undefined') return def;
  try {
    const raw = localStorage.getItem(AUTO_FETCH_KEY);
    if (!raw) return def;
    const p = JSON.parse(raw) as Partial<{ enabled: boolean; intervalSec: number }>;
    return {
      enabled: typeof p.enabled === 'boolean' ? p.enabled : def.enabled,
      intervalSec:
        typeof p.intervalSec === 'number' && Number.isFinite(p.intervalSec)
          ? Math.max(30, p.intervalSec) : def.intervalSec,
    };
  } catch {
    return def;
  }
}

/** Persist the toggle/interval; storage full or unavailable is non-fatal. */
export function writeAutoFetchConfig(enabled: boolean, intervalSec: number): void {
  if (typeof localStorage === 'undefined') return;
  try {
    localStorage.setItem(AUTO_FETCH_KEY, JSON.stringify({ enabled, intervalSec }));
  } catch {
    /* storage full / unavailable — non-fatal */
  }
}
