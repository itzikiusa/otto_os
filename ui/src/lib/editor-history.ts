// Per-document editor state + undo history that outlives the CodeEditor
// instance. CodeEditor's own `keptStates` only lives as long as the component:
// every remount (DB Query ↔ Structure/Diagram, a Kafka/SSH pane round trip,
// leaving the page, a reload) used to wipe the undo history of every query tab.
//
// States are kept as JSON (`state.toJSON({ history: historyField })`) and
// rebuilt with FRESH extensions (`EditorState.fromJSON`) — a parked EditorState
// holds closures bound to the destroyed editor (listeners, `onchange`, submit
// keymap) and must never be reused across instances.
//
// Two tiers:
//  - memory: a module-level LRU (MEM_MAX docs) — survives remounts;
//  - IndexedDB (`otto-editor-history` / `docs`): only for entries saved with
//    `persist` (never a masked DB tab: history holds every string ever typed or
//    pasted, secrets included). Written 2 s after the last save and flushed on
//    pagehide; read back once on start (`hydrateEditorHistory`). Per-entry cap
//    PERSIST_MAX_BYTES (bigger → the entry is dropped from disk, the doc itself
//    is the tab's own draft), total cap PERSIST_TOTAL_BYTES (oldest first).

export interface SavedEditorState {
  /** `EditorState.toJSON({ history: historyField })` — doc, selection, history. */
  json: { doc: string; [k: string]: unknown };
  /** Scroll offset of the editor when it was parked. */
  scrollTop: number;
  /** True when this entry came back from disk (a reload): it is only used when
   *  its doc still equals the restored draft — otherwise the draft wins with an
   *  empty history (an undo would otherwise jump to an unrelated text). */
  fromDisk?: boolean;
  /** Last save (ms) — LRU order on disk. */
  at: number;
}

const MEM_MAX = 48;
/** Largest serialized entry the disk tier keeps (exported so an editor can
 *  skip serializing a doc that is bigger than this on its own). */
export const PERSIST_MAX_BYTES = 512 * 1024;
const PERSIST_TOTAL_BYTES = 10 * 1024 * 1024;
/** Disk write delay after a save. The editor saves 1.5 s after the last edit,
 *  so a doc reaches IndexedDB about 2 s after typing stops. */
const WRITE_DELAY_MS = 500;
const DB_NAME = 'otto-editor-history';
const STORE = 'docs';

const mem = new Map<string, SavedEditorState>();
/** Keys whose latest save may go to disk (absent = memory only). */
const persistable = new Set<string>();
/** Keys with a disk write (or delete) queued. */
const dirty = new Set<string>();
let writeTimer: ReturnType<typeof setTimeout> | null = null;

function touch(key: string, entry: SavedEditorState): void {
  mem.delete(key);
  mem.set(key, entry);
  while (mem.size > MEM_MAX) {
    const oldest = mem.keys().next().value;
    if (oldest === undefined) break;
    mem.delete(oldest);
    persistable.delete(oldest);
  }
}

/** Park a document's state. `persist` = may be written to IndexedDB. */
export function saveEditorState(
  key: string,
  json: SavedEditorState['json'],
  scrollTop: number,
  persist: boolean,
): void {
  touch(key, { json, scrollTop, at: Date.now() });
  if (persist) {
    persistable.add(key);
    dirty.add(key);
    // Open the database now, so a flush on page hide (a reload) only has to
    // start its transaction — an open still pending at unload never completes.
    void openDb();
    scheduleWrite();
  } else if (persistable.delete(key)) {
    // It just became non-persistable (the tab was masked): drop the disk copy.
    dirty.add(key);
    scheduleWrite();
  }
}

/** Stop persisting `key` (its tab was just masked): the disk copy goes now;
 *  the in-memory history stays for this session. */
export function unpersistEditorState(key: string): void {
  if (!persistable.delete(key)) return;
  dirty.add(key);
  void flushEditorHistory();
}

/** Open the database ahead of need (an editor that keeps history mounted):
 *  a page-hide flush must only START a transaction, never wait on an open. */
export function prepareEditorHistory(): void {
  void openDb();
}

/** The parked state for `key` (memory, incl. what hydration loaded), or null. */
export function loadEditorState(key: string): SavedEditorState | null {
  return mem.get(key) ?? null;
}

/** Forget a document everywhere (tab closed / access reset). */
export function forgetEditorState(key: string): void {
  mem.delete(key);
  persistable.delete(key);
  dirty.add(key); // a queued write for a missing/non-persistable key deletes it
  scheduleWrite();
}

/** Forget every document whose key starts with `prefix` (memory + disk). */
export function forgetEditorStates(prefix: string): void {
  for (const k of [...mem.keys()]) if (k.startsWith(prefix)) forgetEditorState(k);
  void withStore('readwrite', (store) => {
    const req = store.openCursor();
    req.onsuccess = () => {
      const cur = req.result;
      if (!cur) return;
      if (String(cur.key).startsWith(prefix)) cur.delete();
      cur.continue();
    };
  });
}

// ── IndexedDB ────────────────────────────────────────────────────────────────

let dbPromise: Promise<IDBDatabase | null> | null = null;
function openDb(): Promise<IDBDatabase | null> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve) => {
    try {
      if (typeof indexedDB === 'undefined') return resolve(null);
      const open = (version?: number): void => {
        const req = version ? indexedDB.open(DB_NAME, version) : indexedDB.open(DB_NAME);
        req.onupgradeneeded = () => {
          if (!req.result.objectStoreNames.contains(STORE)) req.result.createObjectStore(STORE);
        };
        req.onsuccess = () => {
          const db = req.result;
          // A database that exists without our store (created by something
          // else, or an interrupted upgrade) would fail every transaction
          // forever: bump the version once to create it.
          if (!db.objectStoreNames.contains(STORE) && !version) {
            const next = db.version + 1;
            db.close();
            open(next);
            return;
          }
          resolve(db.objectStoreNames.contains(STORE) ? db : null);
        };
        req.onerror = () => resolve(null);
        req.onblocked = () => resolve(null);
      };
      open();
    } catch {
      resolve(null); // private mode / blocked site data: memory tier only
    }
  });
  return dbPromise;
}

async function withStore(
  mode: IDBTransactionMode,
  fn: (store: IDBObjectStore) => void,
): Promise<void> {
  const db = await openDb();
  if (!db) return;
  await new Promise<void>((resolve) => {
    try {
      const tx = db.transaction(STORE, mode);
      fn(tx.objectStore(STORE));
      tx.oncomplete = () => resolve();
      tx.onerror = () => resolve();
      tx.onabort = () => resolve();
    } catch {
      resolve();
    }
  });
}

function scheduleWrite(): void {
  if (writeTimer !== null) return;
  writeTimer = setTimeout(() => {
    writeTimer = null;
    void flushEditorHistory();
  }, WRITE_DELAY_MS);
}

/** Write queued entries to IndexedDB now (pagehide / tests). */
export async function flushEditorHistory(): Promise<void> {
  if (writeTimer !== null) clearTimeout(writeTimer);
  writeTimer = null;
  if (dirty.size === 0) return;
  const keys = [...dirty];
  dirty.clear();
  await withStore('readwrite', (store) => {
    for (const key of keys) {
      const entry = mem.get(key);
      if (!entry || !persistable.has(key)) {
        store.delete(key);
        continue;
      }
      let text: string;
      try {
        text = JSON.stringify(entry.json);
      } catch {
        store.delete(key);
        continue;
      }
      // An oversized history is not worth the disk: drop it (the doc itself
      // is the tab's own persisted draft).
      if (text.length > PERSIST_MAX_BYTES) store.delete(key);
      else store.put({ text, scrollTop: entry.scrollTop, at: entry.at }, key);
    }
  });
}

const RELOAD_PREFIX = 'otto_editor_history:';

/** Synchronously copy queued (not yet on disk) persistable entries to
 *  sessionStorage — the reload-safe path. Size-capped like the disk tier. */
function stashDirtyForReload(): void {
  if (typeof sessionStorage === 'undefined') return;
  for (const key of dirty) {
    const entry = mem.get(key);
    if (!entry || !persistable.has(key)) continue;
    try {
      const text = JSON.stringify(entry.json);
      if (text.length > PERSIST_MAX_BYTES) continue;
      sessionStorage.setItem(
        RELOAD_PREFIX + key,
        JSON.stringify({ text, scrollTop: entry.scrollTop, at: entry.at }),
      );
    } catch {
      /* quota / unavailable — IndexedDB may still make it */
    }
  }
}

/** Take the entries `stashDirtyForReload` left (newer than IndexedDB's). */
function takeReloadStash(keep: (key: string) => boolean): void {
  if (typeof sessionStorage === 'undefined') return;
  let keys: string[] = [];
  try {
    for (let i = 0; i < sessionStorage.length; i++) {
      const k = sessionStorage.key(i);
      if (k?.startsWith(RELOAD_PREFIX)) keys.push(k);
    }
  } catch {
    keys = [];
  }
  for (const sk of keys) {
    try {
      const raw = sessionStorage.getItem(sk);
      sessionStorage.removeItem(sk);
      const key = sk.slice(RELOAD_PREFIX.length);
      if (!raw || !keep(key) || mem.has(key)) continue;
      const v = JSON.parse(raw) as { text: string; scrollTop?: number; at?: number };
      const json = JSON.parse(v.text) as SavedEditorState['json'];
      if (typeof json?.doc !== 'string') continue;
      persistable.add(key);
      touch(key, { json, scrollTop: v.scrollTop ?? 0, at: v.at ?? Date.now(), fromDisk: true });
      dirty.add(key); // now make it durable
    } catch {
      /* malformed — dropped */
    }
  }
  if (dirty.size) scheduleWrite();
}

let hydrated: Promise<void> | null = null;
/**
 * Load persisted entries into memory once (newest first, within the total
 * budget; entries past it are deleted). `keep(key)` = the key still belongs to
 * a live document — orphans (closed tabs) are pruned.
 */
export function hydrateEditorHistory(keep: (key: string) => boolean = () => true): Promise<void> {
  if (hydrated) return hydrated;
  // The reload stash first: it is newer than anything on disk.
  takeReloadStash(keep);
  hydrated = withStore('readwrite', (store) => {
    const rows: { key: string; text: string; scrollTop: number; at: number }[] = [];
    const req = store.openCursor();
    req.onsuccess = () => {
      const cur = req.result;
      if (cur) {
        const v = cur.value as { text?: unknown; scrollTop?: unknown; at?: unknown };
        const key = String(cur.key);
        if (typeof v?.text === 'string' && keep(key)) {
          rows.push({
            key,
            text: v.text,
            scrollTop: typeof v.scrollTop === 'number' ? v.scrollTop : 0,
            at: typeof v.at === 'number' ? v.at : 0,
          });
        } else {
          cur.delete();
        }
        cur.continue();
        return;
      }
      rows.sort((a, b) => b.at - a.at);
      let total = 0;
      for (const r of rows) {
        total += r.text.length;
        if (total > PERSIST_TOTAL_BYTES) {
          store.delete(r.key);
          continue;
        }
        // A newer in-memory save (made before hydration finished) wins.
        if (mem.has(r.key)) continue;
        try {
          const json = JSON.parse(r.text) as SavedEditorState['json'];
          if (typeof json?.doc !== 'string') continue;
          persistable.add(r.key);
          touch(r.key, { json, scrollTop: r.scrollTop, at: r.at, fromDisk: true });
        } catch {
          store.delete(r.key);
        }
      }
    };
  });
  return hydrated;
}

/** Live editors' "park my current state now" hooks (CodeEditor registers
 *  one). Run before a page-hide flush, so the last ≤1.5 s of typing — still in
 *  the editor's save debounce — reaches the disk on a reload too. */
const liveParkers = new Set<() => void>();
export function registerLiveParker(park: () => void): () => void {
  liveParkers.add(park);
  return () => liveParkers.delete(park);
}

if (typeof window !== 'undefined') {
  const flush = (): void => {
    for (const park of liveParkers) {
      try {
        park();
      } catch {
        /* one editor failing must not lose the others */
      }
    }
    void flushEditorHistory();
  };
  // An IndexedDB transaction started while the page unloads (a reload) is not
  // guaranteed to commit, so the unload path ALSO writes the not-yet-flushed
  // entries synchronously to sessionStorage (survives a reload of this tab);
  // hydration moves them into IndexedDB.
  const unload = (): void => {
    for (const park of liveParkers) {
      try {
        park();
      } catch {
        /* keep going */
      }
    }
    stashDirtyForReload();
    void flushEditorHistory();
  };
  window.addEventListener('pagehide', unload);
  if (typeof document !== 'undefined') {
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden') flush();
    });
  }
}
