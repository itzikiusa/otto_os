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
