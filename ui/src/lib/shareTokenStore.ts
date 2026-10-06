// Guest share-link token persistence (S14-07 / S20-05). The router captures
// `#/s/<sid>/<token>` and strips the token from the URL; kept only in memory,
// ANY reload (pull-to-refresh, iOS evicting a background tab, the chunk-error
// Reload) lost it and the guest read "invalid or has expired" for a link that
// was still valid. The token is now ALSO kept in `sessionStorage`:
//  • tab-scoped — it never outlives the tab and is never shared across tabs;
//  • keyed `otto_share:<sid>` — never `otto_token`, so it can't clobber an
//    owner login (the reason it stayed out of localStorage);
//  • still out of the URL, history and referrer.
// Dropped when the link turns out dead (revoked / expired / unknown).

const PREFIX = 'otto_share:';

/** The storage seam (window.sessionStorage in the app; a fake in tests). */
export type TokenStorage = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

function storage(s?: TokenStorage | null): TokenStorage | null {
  if (s !== undefined) return s;
  try {
    return typeof sessionStorage === 'undefined' ? null : sessionStorage;
  } catch {
    return null; // storage disabled (private mode / blocked site data)
  }
}

export function storeShareToken(sessionId: string, token: string, s?: TokenStorage | null): void {
  try {
    storage(s)?.setItem(PREFIX + sessionId, token);
  } catch {
    /* quota / disabled storage: the in-memory token still works this load */
  }
}

export function storedShareToken(sessionId: string, s?: TokenStorage | null): string | null {
  try {
    return storage(s)?.getItem(PREFIX + sessionId) ?? null;
  } catch {
    return null;
  }
}

export function dropShareToken(sessionId: string, s?: TokenStorage | null): void {
  try {
    storage(s)?.removeItem(PREFIX + sessionId);
  } catch {
    /* nothing to drop */
  }
}
