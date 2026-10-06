// Guarded localStorage access. The accessor itself can THROW (a blocked
// storage partition, a private window, a sandboxed/opaque-origin document) and
// setItem throws on quota — an unguarded read on the boot path left the whole
// app blank. Reads fall back to "missing"; writes and removals are
// best-effort (every caller already treats storage as a convenience cache).

export function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function lsSet(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* blocked or over quota — persistence is best-effort */
  }
}

export function lsRemove(key: string): void {
  try {
    localStorage.removeItem(key);
  } catch {
    /* blocked — nothing to remove */
  }
}

// Same guards for sessionStorage — per-tab, cleared when the tab closes. Used
// for credentials that must survive a reload but never outlive the tab (the
// admin's own token while impersonating).

export function ssGet(key: string): string | null {
  try {
    return sessionStorage.getItem(key);
  } catch {
    return null;
  }
}

export function ssSet(key: string, value: string): void {
  try {
    sessionStorage.setItem(key, value);
  } catch {
    /* blocked or over quota — persistence is best-effort */
  }
}

export function ssRemove(key: string): void {
  try {
    sessionStorage.removeItem(key);
  } catch {
    /* blocked — nothing to remove */
  }
}
