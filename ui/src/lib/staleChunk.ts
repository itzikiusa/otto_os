// A tab opened before a deploy still names the old build's chunk hashes, and
// those 404 now — so the page it shows can't load. Reload once to pick up the
// new build; the guard stops a loop when the failure is something else (the
// error card with Retry / Reload Otto stays the fallback). Pure enough for
// `node --test`: storage, clock and reload are injectable.

const KEY = 'otto.staleChunkReloadAt';
/** No second automatic reload within this window. */
export const STALE_RELOAD_GUARD_MS = 60_000;

type Store = Pick<Storage, 'getItem' | 'setItem'>;

/** Reload unless one already ran within the guard, or the device is offline
 *  (a reload then shows no page at all). Returns whether it reloaded. */
export function reloadForStaleChunk(
  store: Store | null = globalThis.sessionStorage ?? null,
  now: number = Date.now(),
  reload: () => void = () => location.reload(),
  online: boolean = globalThis.navigator?.onLine !== false,
): boolean {
  if (!store || !online) return false;
  try {
    if (now - (Number(store.getItem(KEY)) || 0) < STALE_RELOAD_GUARD_MS) return false;
    store.setItem(KEY, String(now));
  } catch {
    return false; // no storage, no guard: never risk a reload loop
  }
  reload();
  return true;
}
