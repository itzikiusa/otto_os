// Clipboard ring: the last copies made INSIDE Otto (DB result cells/rows, the
// query editor). The webview cannot read macOS clipboard history, so this is
// honest about its scope — it only ever sees what Otto itself copied.
//
// 50 entries, newest first, each ≤ 64 KB, de-duplicated (a re-copy moves the
// entry to the top). Pinned entries stay put and survive "Clear" — they act as
// snippets. Persisted in IndexedDB (`otto-clip-history`), never localStorage
// (64 KB × 50 would eat the drafts' quota). Opt-out setting (`enabled`);
// turning it off clears the ring. Copies from a masked DB tab are never
// recorded (the database store installs that guard).

export interface ClipEntry {
  text: string;
  /** Where it was copied ("Results", "Editor"…). */
  source: string;
  /** Epoch ms of the (latest) copy. */
  at: number;
  pinned: boolean;
}

const MAX_ENTRIES = 50;
const MAX_ENTRY_CHARS = 64 * 1024;
const ENABLED_KEY = 'otto_clip_history';
const DB_NAME = 'otto-clip-history';
const STORE = 'ring';
const RING_KEY = 'entries';

function loadEnabled(): boolean {
  try {
    return localStorage.getItem(ENABLED_KEY) !== '0';
  } catch {
    return true;
  }
}

/** Pure ring update (exported for tests): newest first, pinned kept, deduped. */
export function pushClip(ring: readonly ClipEntry[], text: string, source: string, at: number): ClipEntry[] {
  if (!text || text.length > MAX_ENTRY_CHARS) return [...ring];
  const prev = ring.find((e) => e.text === text);
  const rest = ring.filter((e) => e.text !== text);
  const next = [{ text, source, at, pinned: prev?.pinned ?? false }, ...rest];
  // Over the cap: drop the oldest UNPINNED entries.
  while (next.length > MAX_ENTRIES) {
    let idx = -1;
    for (let i = next.length - 1; i >= 0; i--) {
      if (!next[i].pinned) {
        idx = i;
        break;
      }
    }
    if (idx < 0) break;
    next.splice(idx, 1);
  }
  return next;
}

let dbPromise: Promise<IDBDatabase | null> | null = null;
function openDb(): Promise<IDBDatabase | null> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve) => {
    try {
      if (typeof indexedDB === 'undefined') return resolve(null);
      const req = indexedDB.open(DB_NAME, 1);
      req.onupgradeneeded = () => {
        if (!req.result.objectStoreNames.contains(STORE)) req.result.createObjectStore(STORE);
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
  return dbPromise;
}

class ClipHistoryStore {
  entries: ClipEntry[] = $state([]);
  enabled = $state(loadEnabled());
  private loaded: Promise<void> | null = null;
  private guard: (() => boolean) | null = null;
  private saveTimer: ReturnType<typeof setTimeout> | null = null;

  /** Install a "don't record now" predicate (a masked DB tab is active). */
  setGuard(fn: (() => boolean) | null): void {
    this.guard = fn;
  }

  /** Load the persisted ring once (cheap; called before the picker opens). */
  load(): Promise<void> {
    if (this.loaded) return this.loaded;
    this.loaded = (async () => {
      const db = await openDb();
      if (!db) return;
      const stored = await new Promise<unknown>((resolve) => {
        try {
          const req = db.transaction(STORE, 'readonly').objectStore(STORE).get(RING_KEY);
          req.onsuccess = () => resolve(req.result);
          req.onerror = () => resolve(null);
        } catch {
          resolve(null);
        }
      });
      if (!Array.isArray(stored)) return;
      const disk = stored.filter(
        (e): e is ClipEntry =>
          !!e && typeof e.text === 'string' && typeof e.source === 'string' && typeof e.at === 'number',
      );
      // Copies made before the load finished are newer — merge them on top.
      let ring = disk.map((e) => ({ ...e, pinned: e.pinned === true }));
      for (const e of [...this.entries].reverse()) ring = pushClip(ring, e.text, e.source, e.at);
      this.entries = ring;
    })();
    return this.loaded;
  }

  /** Record a copy made in Otto. No-op when disabled or guarded. */
  record(text: string, source: string): void {
    if (!this.enabled || !text.trim()) return;
    if (this.guard?.()) return;
    this.entries = pushClip(this.entries, text, source, Date.now());
    void this.load();
    this.scheduleSave();
  }

  togglePin(text: string): void {
    this.entries = this.entries.map((e) => (e.text === text ? { ...e, pinned: !e.pinned } : e));
    this.scheduleSave();
  }

  remove(text: string): void {
    this.entries = this.entries.filter((e) => e.text !== text);
    this.scheduleSave();
  }

  /** Clear every unpinned entry. */
  clear(): void {
    this.entries = this.entries.filter((e) => e.pinned);
    this.scheduleSave();
  }

  setEnabled(on: boolean): void {
    this.enabled = on;
    try {
      localStorage.setItem(ENABLED_KEY, on ? '1' : '0');
    } catch {
      /* preference only */
    }
    if (!on) {
      // Off means off: nothing copied earlier stays on disk either.
      this.entries = [];
      this.scheduleSave();
    }
  }

  private scheduleSave(): void {
    if (this.saveTimer !== null) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => {
      this.saveTimer = null;
      void this.save();
    }, 500);
  }

  private async save(): Promise<void> {
    const db = await openDb();
    if (!db) return;
    const snapshot = $state.snapshot(this.entries);
    try {
      db.transaction(STORE, 'readwrite').objectStore(STORE).put(snapshot, RING_KEY);
    } catch {
      /* storage blocked — the in-memory ring still works this session */
    }
  }
}

export const clipHistory = new ClipHistoryStore();
