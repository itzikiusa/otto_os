// Spaces 01–04: the one shared notion of "which space am I in" for the Home
// desktop (its up-to-four views) and the floating "type or speak" bar's
// 01 02 03 04 switcher. Tiny on purpose — Home owns what a space CONTAINS
// (modules/home/home.svelte.ts); this store only carries the active index and
// the names, so the bar can show and switch spaces without importing Home.
//
// Persisted per device in localStorage and synced across windows through the
// `storage` event, so the desktop shell's separate bar window and the main
// window agree without a round-trip.

export const SPACE_COUNT = 4;

const LS = {
  active: 'otto_space_active',
  names: 'otto_space_names',
  /** Home's pre-spaces key, read once as a fallback. */
  legacyActive: 'otto_home_active',
};

function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function lsSet(key: string, val: string): void {
  try {
    localStorage.setItem(key, val);
  } catch {
    /* private mode */
  }
}

function clampIndex(n: number): number {
  return Number.isFinite(n) ? Math.min(SPACE_COUNT - 1, Math.max(0, Math.trunc(n))) : 0;
}

function readNames(): string[] {
  try {
    const v: unknown = JSON.parse(lsGet(LS.names) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string').slice(0, SPACE_COUNT) : [];
  } catch {
    return [];
  }
}

/** `0` → `"01"`: how a space is numbered everywhere (Home, the bar). */
export function spaceNumber(index: number): string {
  return String(index + 1).padStart(2, '0');
}

class SpacesStore {
  /** Active space, 0-based. */
  active = $state(clampIndex(Number(lsGet(LS.active) ?? lsGet(LS.legacyActive) ?? 0)));
  /** Names of the spaces that exist (length 0–4); Home publishes them. */
  names: string[] = $state(readNames());

  /** Label for space `i`: its name, else "Space 01". */
  label(i: number): string {
    return this.names[i] ?? `Space ${spaceNumber(i)}`;
  }

  select(i: number): void {
    const n = clampIndex(i);
    if (n === this.active) return;
    this.active = n;
    lsSet(LS.active, String(n));
  }

  setNames(names: string[]): void {
    const next = names.slice(0, SPACE_COUNT);
    if (next.length === this.names.length && next.every((n, i) => n === this.names[i])) return;
    this.names = next;
    lsSet(LS.names, JSON.stringify(next));
  }

  constructor() {
    if (typeof window === 'undefined') return;
    // Another window (the bar panel, a pop-out) switched space or renamed one.
    window.addEventListener('storage', (e) => {
      if (e.key === LS.active && e.newValue != null) this.active = clampIndex(Number(e.newValue));
      else if (e.key === LS.names) this.names = readNames();
    });
  }
}

export const spaces = new SpacesStore();
