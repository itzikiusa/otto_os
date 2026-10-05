// Workbench store — per-user scratch files with full, append-only history.
//
// The daemon owns the history (every content save lands in a revision; rapid
// autosaves coalesce server-side into one revision per ~60 s burst). This store
// owns the *editing* state: the doc list + trash, the open tabs (persisted per
// workspace), each open doc's live buffer, the debounced autosave, and the
// per-doc placeholder values / preview toggle.
//
// Unsaved content is never dropped: every dirty buffer is mirrored to
// localStorage (`otto.wb.unsaved.<docId>`) until the daemon has it, a failed
// save keeps the buffer dirty with an inline error + Retry, and a reload
// restores the backup over the server copy (then saves it).

import { untrack } from 'svelte';
import {
  createWorkbenchDoc,
  getWorkbenchDoc,
  listWorkbenchDocs,
  purgeWorkbenchDoc,
  restoreWorkbenchDoc,
  trashWorkbenchDoc,
  updateWorkbenchDoc,
} from '../../lib/api/workbench';
import type { WorkbenchDoc, WorkbenchDocFull, WorkbenchUpdateReq } from '../../lib/api/types';
import { loadErrorText } from '../../lib/loadError';
import { ApiError } from '../../lib/api/client';
import { onLive, appLive } from '../../lib/live';

/** This window's id: echoed back in `workbench_doc_changed` so we can ignore
 *  our own writes. */
export const WB_CLIENT_ID: string = (() => {
  try {
    return crypto.randomUUID();
  } catch {
    const a = new Uint32Array(4);
    crypto.getRandomValues(a);
    return Array.from(a, (n) => n.toString(16)).join('');
  }
})();

/** Debounce between the last keystroke and the autosave. */
export const AUTOSAVE_MS = 800;

export type SidePanel = 'none' | 'placeholders' | 'history';

/** One open tab's editing state. `saved` is the content the daemon has. */
export interface OpenDoc {
  id: string;
  doc: WorkbenchDocFull | null;
  buffer: string;
  saved: string;
  loading: boolean;
  loadError: string | null;
  saving: boolean;
  saveError: string | null;
  /** Another window changed it while we hold local edits. */
  remoteChanged: boolean;
}

// ── localStorage helpers (every access guarded: private mode / blocked) ──────

function lsGet<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw == null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}
function lsSet(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* quota / blocked — best effort */
  }
}
function lsDel(key: string): void {
  try {
    localStorage.removeItem(key);
  } catch {
    /* blocked */
  }
}

const K_TABS = (ws: string) => `otto.wb.tabs.${ws}`;
const K_UNSAVED = (id: string) => `otto.wb.unsaved.${id}`;
const K_VALUES = (id: string) => `otto.wb.values.${id}`;
const K_PREVIEW = 'otto.wb.preview';
const K_PANEL = 'otto.wb.panel';

/** "Untitled N" with N one past the highest existing. */
export function nextUntitledName(names: readonly string[]): string {
  let max = 0;
  for (const n of names) {
    const m = /^Untitled(?: (\d+))?(?:\.[a-z0-9]+)?$/i.exec(n.trim());
    if (m) max = Math.max(max, m[1] ? Number(m[1]) : 1);
  }
  return `Untitled ${max + 1}`;
}

/** Pinned first, then most recently updated. */
export function sortDocs(docs: readonly WorkbenchDoc[]): WorkbenchDoc[] {
  return [...docs].sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return b.updated_at.localeCompare(a.updated_at);
  });
}

class WorkbenchStore {
  ws: string | null = $state(null);
  docs: WorkbenchDoc[] = $state([]);
  trash: WorkbenchDoc[] = $state([]);
  loading = $state(false);
  loaded = $state(false);
  error: string | null = $state(null);
  trashLoading = $state(false);
  trashError: string | null = $state(null);
  showTrash = $state(false);

  tabs: string[] = $state([]);
  active: string | null = $state(null);
  open: Record<string, OpenDoc> = $state({});

  panel: SidePanel = $state(lsGet<SidePanel>(K_PANEL, 'none'));
  previewOn: Record<string, boolean> = $state(lsGet<Record<string, boolean>>(K_PREVIEW, {}));
  values: Record<string, Record<string, string>> = $state({});

  private timers = new Map<string, ReturnType<typeof setTimeout>>();
  private backupTimers = new Map<string, ReturnType<typeof setTimeout>>();
  /** A save is in flight for this id; `rerun` → save again after it. */
  private inflight = new Set<string>();
  private rerun = new Map<string, boolean>();
  /** Ids whose next content save skips the `if_hash` precondition ("Keep mine"). */
  private forceNext = new Set<string>();
  private unlisten: (() => void) | null = null;
  private listTimer: ReturnType<typeof setTimeout> | null = null;

  get activeDoc(): OpenDoc | null {
    return this.active ? (this.open[this.active] ?? null) : null;
  }

  isDirty(id: string): boolean {
    const o = this.open[id];
    return !!o && !!o.doc && o.buffer !== o.saved;
  }

  get anyDirty(): boolean {
    return Object.values(this.open).some((o) => o.doc && o.buffer !== o.saved);
  }

  metaOf(id: string): WorkbenchDoc | undefined {
    return this.docs.find((d) => d.id === id) ?? this.trash.find((d) => d.id === id);
  }

  // ── lifecycle ──────────────────────────────────────────────────────────────

  /** Bind to a workspace: load the list, restore its tabs, subscribe to live
   *  updates. Idempotent for the same workspace. */
  async attach(ws: string): Promise<void> {
    if (this.ws === ws && this.loaded) {
      // Re-mounted page: resubscribe (detach dropped it) and catch up.
      if (!this.unlisten) {
        this.subscribe();
        void this.loadList(true);
      }
      return;
    }
    this.flushAll();
    this.ws = ws;
    this.docs = [];
    this.trash = [];
    this.open = {};
    this.loaded = false;
    const saved = lsGet<{ tabs: string[]; active: string | null }>(K_TABS(ws), { tabs: [], active: null });
    this.tabs = Array.isArray(saved.tabs) ? saved.tabs.filter((t) => typeof t === 'string') : [];
    this.active = saved.active && this.tabs.includes(saved.active) ? saved.active : (this.tabs[0] ?? null);
    this.subscribe();
    await this.loadList();
    if (this.ws !== ws) return;
    // Drop tabs whose docs are gone (deleted elsewhere / trashed).
    const live = new Set(this.docs.map((d) => d.id));
    const kept = this.tabs.filter((t) => live.has(t));
    if (kept.length !== this.tabs.length) {
      this.tabs = kept;
      if (this.active && !live.has(this.active)) this.active = kept[0] ?? null;
    }
    // List/detail opens on an item: last open tab, else the most recent file.
    if (!this.active && this.docs.length > 0) {
      const first = sortDocs(this.docs)[0];
      this.tabs = [first.id];
      this.active = first.id;
    }
    this.persistTabs();
    if (this.active) void this.ensureLoaded(this.active);
  }

  detach(): void {
    this.flushAll();
    this.unlisten?.();
    this.unlisten = null;
  }

  private subscribe(): void {
    this.unlisten?.();
    const offEv = onLive(['workbench_doc_changed'], (ev) => this.onEvent(ev));
    const offResync = appLive.onResync(() => {
      if (this.ws) void this.loadList();
    });
    this.unlisten = () => {
      offEv();
      offResync();
    };
  }

  private onEvent(ev: Record<string, unknown>): void {
    if (ev.workspace_id !== this.ws) return;
    if (ev.client_id && ev.client_id === WB_CLIENT_ID) return;
    const id = String(ev.doc_id ?? '');
    const action = String(ev.action ?? '');
    // Coalesce bursts (another window autosaving) into one list refetch.
    if (this.listTimer) clearTimeout(this.listTimer);
    this.listTimer = setTimeout(() => {
      this.listTimer = null;
      void this.loadList(true);
      if (this.showTrash) void this.loadTrash();
    }, 250);
    const o = this.open[id];
    if (!o) return;
    if (action === 'trashed' || action === 'deleted') {
      if (!this.isDirty(id)) this.closeTab(id, true);
      else o.remoteChanged = true;
      return;
    }
    const rev = Number(ev.rev ?? 0);
    // A coalesced autosave in another window keeps `rev` — compare the content
    // hash when the event carries one, `rev` only as the fallback.
    const hash = typeof ev.content_hash === 'string' ? ev.content_hash : '';
    if (o.doc && (rev < o.doc.rev || (hash ? hash === o.doc.content_hash : rev === o.doc.rev))) return;
    if (this.isDirty(id)) this.markRemoteChanged(id);
    else void this.reload(id);
  }

  // ── list ───────────────────────────────────────────────────────────────────

  async loadList(quiet = false): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    if (!quiet) this.loading = true;
    try {
      const docs = await listWorkbenchDocs(ws);
      if (this.ws !== ws) return;
      this.docs = docs;
      this.error = null;
      this.loaded = true;
    } catch (e) {
      if (this.ws === ws) this.error = loadErrorText(e);
    } finally {
      if (this.ws === ws) this.loading = false;
    }
  }

  async loadTrash(): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    this.trashLoading = true;
    try {
      const t = await listWorkbenchDocs(ws, { trash: true });
      if (this.ws !== ws) return;
      this.trash = t;
      this.trashError = null;
    } catch (e) {
      if (this.ws === ws) this.trashError = loadErrorText(e);
    } finally {
      if (this.ws === ws) this.trashLoading = false;
    }
  }

  private upsertMeta(meta: WorkbenchDoc): void {
    const i = this.docs.findIndex((d) => d.id === meta.id);
    if (meta.deleted_at) {
      if (i >= 0) this.docs.splice(i, 1);
      return;
    }
    if (i >= 0) this.docs[i] = meta;
    else this.docs.unshift(meta);
  }

  // ── tabs ───────────────────────────────────────────────────────────────────

  private persistTabs(): void {
    if (this.ws) lsSet(K_TABS(this.ws), { tabs: this.tabs, active: this.active });
  }

  openDoc(id: string): void {
    const prev = this.active;
    if (prev && prev !== id) this.flush(prev);
    if (!this.tabs.includes(id)) this.tabs.push(id);
    this.active = id;
    this.persistTabs();
    void this.ensureLoaded(id);
  }

  /** Close a tab, flushing any unsaved content first (never dropped). */
  closeTab(id: string, skipFlush = false): void {
    if (!skipFlush) this.flush(id);
    const i = this.tabs.indexOf(id);
    if (i < 0) return;
    this.tabs.splice(i, 1);
    if (this.active === id) this.active = this.tabs[Math.min(i, this.tabs.length - 1)] ?? null;
    // Keep the OpenDoc while a save is still in flight; else drop it.
    if (!this.inflight.has(id) && !this.isDirty(id)) delete this.open[id];
    this.persistTabs();
    if (this.active) void this.ensureLoaded(this.active);
  }

  cycle(dir: 1 | -1): void {
    if (this.tabs.length < 2 || !this.active) return;
    const i = this.tabs.indexOf(this.active);
    this.openDoc(this.tabs[(i + dir + this.tabs.length) % this.tabs.length]);
  }

  // ── doc load ───────────────────────────────────────────────────────────────

  async ensureLoaded(id: string): Promise<void> {
    const existing = this.open[id];
    if (existing && (existing.doc || existing.loading)) return;
    this.open[id] = existing ?? {
      id,
      doc: null,
      buffer: '',
      saved: '',
      loading: true,
      loadError: null,
      saving: false,
      saveError: null,
      remoteChanged: false,
    };
    await this.fetchInto(id, true);
  }

  async retryLoad(id: string): Promise<void> {
    const o = this.open[id];
    if (o) {
      o.loadError = null;
      o.loading = true;
    }
    await this.fetchInto(id, true);
  }

  private async fetchInto(id: string, restoreBackup: boolean): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    const o = this.open[id];
    if (!o) return;
    o.loading = true;
    try {
      const doc = await getWorkbenchDoc(ws, id);
      if (this.ws !== ws || !this.open[id]) return;
      const cur = this.open[id];
      cur.doc = doc;
      cur.saved = doc.content;
      cur.buffer = doc.content;
      cur.loadError = null;
      cur.remoteChanged = false;
      if (!this.values[id]) this.values[id] = lsGet<Record<string, string>>(K_VALUES(id), {});
      if (restoreBackup) {
        const backup = lsGet<{ content: string } | null>(K_UNSAVED(id), null);
        if (backup && typeof backup.content === 'string' && backup.content !== doc.content) {
          cur.buffer = backup.content;
          this.schedule(id);
        } else if (backup) {
          lsDel(K_UNSAVED(id));
        }
      }
      this.upsertMeta(doc);
    } catch (e) {
      const cur = this.open[id];
      if (cur) cur.loadError = loadErrorText(e);
    } finally {
      const cur = this.open[id];
      if (cur) cur.loading = false;
    }
  }

  /** Re-fetch from the daemon, discarding unsaved local edits (the "Reload"
   *  choice on a remote-change banner — the page confirms that first). */
  async reload(id: string): Promise<void> {
    lsDel(K_UNSAVED(id));
    await this.fetchInto(id, false);
  }

  /** "Keep mine": dismiss the banner and save the local buffer over it (the
   *  other window's version stays in the history) — the one save that goes
   *  out WITHOUT the `if_hash` precondition. */
  keepMine(id: string): void {
    const o = this.open[id];
    if (!o) return;
    o.remoteChanged = false;
    this.forceNext.add(id);
    void this.save(id, true);
  }

  /** Another window changed this doc while ours is dirty: show the banner and
   *  STOP autosaving until the user picks Reload or Keep mine — the pending
   *  timer (and every later keystroke) used to overwrite the other window's
   *  save before the choice was made (S18-06). */
  private markRemoteChanged(id: string): void {
    const o = this.open[id];
    if (!o) return;
    o.remoteChanged = true;
    const t = this.timers.get(id);
    if (t) {
      clearTimeout(t);
      this.timers.delete(id);
    }
  }

  // ── editing + autosave ─────────────────────────────────────────────────────

  setBuffer(id: string, text: string): void {
    const o = this.open[id];
    if (!o || !o.doc) return;
    if (o.buffer === text) return;
    o.buffer = text;
    this.scheduleBackup(id);
    this.schedule(id);
  }

  /** Mirror the dirty buffer to localStorage (debounced: a whole-doc
   *  stringify per keystroke is too much for a large file). */
  private scheduleBackup(id: string): void {
    if (this.backupTimers.has(id)) return;
    this.backupTimers.set(
      id,
      setTimeout(() => {
        this.backupTimers.delete(id);
        const o = untrack(() => this.open[id]);
        if (o && o.buffer !== o.saved) lsSet(K_UNSAVED(id), { content: o.buffer, at: Date.now() });
      }, 250),
    );
  }

  private schedule(id: string): void {
    const t = this.timers.get(id);
    if (t) clearTimeout(t);
    this.timers.set(
      id,
      setTimeout(() => {
        this.timers.delete(id);
        void this.save(id, false);
      }, AUTOSAVE_MS),
    );
  }

  /** Save now if dirty (tab switch, page hide, close). */
  flush(id: string): void {
    const t = this.timers.get(id);
    if (t) {
      clearTimeout(t);
      this.timers.delete(id);
    }
    if (this.isDirty(id)) {
      // Back up synchronously: on unload the request may never complete.
      const o = untrack(() => this.open[id]);
      if (o) lsSet(K_UNSAVED(id), { content: o.buffer, at: Date.now() });
      void this.save(id, false);
    }
  }

  flushAll(): void {
    for (const id of Object.keys(untrack(() => this.open))) this.flush(id);
  }

  /** PATCH the buffer. `checkpoint` (⌘S) forces a fresh revision. A failed
   *  save leaves the buffer dirty (and backed up) with `saveError` set. */
  async save(id: string, checkpoint: boolean): Promise<void> {
    const ws = this.ws;
    const o = this.open[id];
    if (!ws || !o || !o.doc) return;
    const t = this.timers.get(id);
    if (t) {
      clearTimeout(t);
      this.timers.delete(id);
    }
    if (this.inflight.has(id)) {
      this.rerun.set(id, checkpoint || (this.rerun.get(id) ?? false));
      return;
    }
    // Held while the remote-change banner is up (the buffer stays backed up
    // locally); only Reload / Keep mine resolves it.
    if (o.remoteChanged) return;
    const content = o.buffer;
    if (content === o.saved && !checkpoint) return;
    const force = this.forceNext.has(id);
    // A checkpoint of unchanged content still goes out: the daemon SEALS the
    // open autosave burst (its revision becomes a checkpoint), so the next
    // edit starts a new revision instead of folding into this one.
    this.inflight.add(id);
    o.saving = true;
    try {
      const body: WorkbenchUpdateReq = { content, client_id: WB_CLIENT_ID };
      if (checkpoint) body.checkpoint = true;
      // Optimistic concurrency: the daemon 409s if another window saved since
      // our buffer's base (even a coalesced autosave that kept `rev`).
      if (!force) body.if_hash = o.doc.content_hash;
      const meta = await updateWorkbenchDoc(ws, id, body);
      this.forceNext.delete(id);
      const cur = this.open[id];
      if (cur && cur.doc) {
        cur.saved = content;
        cur.doc = { ...cur.doc, ...meta, content };
        cur.saveError = null;
        if (cur.buffer === content) lsDel(K_UNSAVED(id));
      } else {
        lsDel(K_UNSAVED(id));
      }
      if (this.ws === ws) this.upsertMeta(meta);
    } catch (e) {
      const cur = this.open[id];
      if (cur && e instanceof ApiError && e.status === 409 && !force) {
        // Someone else's save won the race: ask (banner) instead of erroring.
        this.markRemoteChanged(id);
        this.rerun.delete(id);
      } else if (cur) cur.saveError = loadErrorText(e);
    } finally {
      this.inflight.delete(id);
      const cur = this.open[id];
      if (cur) cur.saving = false;
      const again = this.rerun.get(id);
      if (again !== undefined) {
        this.rerun.delete(id);
        void this.save(id, again);
      } else if (cur && !this.tabs.includes(id) && cur.buffer === cur.saved) {
        delete this.open[id];
      }
    }
  }

  // ── metadata ───────────────────────────────────────────────────────────────

  async patchMeta(id: string, body: Omit<WorkbenchUpdateReq, 'content' | 'checkpoint'>): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    const meta = await updateWorkbenchDoc(ws, id, { ...body, client_id: WB_CLIENT_ID });
    this.upsertMeta(meta);
    const o = this.open[id];
    if (o?.doc) o.doc = { ...o.doc, ...meta, content: o.doc.content };
  }

  /** Apply a restored doc (history restore / external reload) to the tab. */
  applyDoc(doc: WorkbenchDocFull): void {
    const o = this.open[doc.id];
    if (o) {
      o.doc = doc;
      o.saved = doc.content;
      o.buffer = doc.content;
      o.saveError = null;
      o.remoteChanged = false;
      lsDel(K_UNSAVED(doc.id));
    }
    this.upsertMeta(doc);
  }

  async create(body: { name?: string; language?: string; content?: string }): Promise<WorkbenchDocFull | null> {
    const ws = this.ws;
    if (!ws) return null;
    const name = body.name ?? nextUntitledName(this.docs.map((d) => d.name));
    const doc = await createWorkbenchDoc(ws, { name, language: body.language ?? 'auto', content: body.content ?? '' });
    this.upsertMeta(doc);
    this.open[doc.id] = {
      id: doc.id,
      doc,
      buffer: doc.content,
      saved: doc.content,
      loading: false,
      loadError: null,
      saving: false,
      saveError: null,
      remoteChanged: false,
    };
    this.values[doc.id] = {};
    this.openDoc(doc.id);
    return doc;
  }

  async duplicate(id: string): Promise<WorkbenchDocFull | null> {
    const ws = this.ws;
    if (!ws) return null;
    const src = this.open[id]?.doc ? { ...this.open[id].doc!, content: this.open[id].buffer } : await getWorkbenchDoc(ws, id);
    const dot = src.name.lastIndexOf('.');
    const name = dot > 0 ? `${src.name.slice(0, dot)} copy${src.name.slice(dot)}` : `${src.name} copy`;
    const doc = await createWorkbenchDoc(ws, {
      name,
      language: src.language,
      content: src.content,
      folder: src.folder,
      tags: src.tags,
    });
    this.upsertMeta(doc);
    this.openDoc(doc.id);
    return doc;
  }

  async moveToTrash(id: string): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    // Persist the last keystrokes first: trash keeps the full history.
    if (this.isDirty(id)) await this.save(id, false);
    const meta = await trashWorkbenchDoc(ws, id);
    this.closeTab(id, true);
    delete this.open[id];
    this.docs = this.docs.filter((d) => d.id !== id);
    if (meta && typeof meta === 'object') this.trash = [meta, ...this.trash.filter((d) => d.id !== id)];
  }

  async restoreFromTrash(id: string): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    const meta = await restoreWorkbenchDoc(ws, id);
    this.trash = this.trash.filter((d) => d.id !== id);
    this.upsertMeta(meta);
  }

  async purge(id: string): Promise<void> {
    const ws = this.ws;
    if (!ws) return;
    await purgeWorkbenchDoc(ws, id);
    this.trash = this.trash.filter((d) => d.id !== id);
    lsDel(K_UNSAVED(id));
    lsDel(K_VALUES(id));
    delete this.values[id];
  }

  // ── view prefs ─────────────────────────────────────────────────────────────

  setPanel(p: SidePanel): void {
    this.panel = this.panel === p ? 'none' : p;
    lsSet(K_PANEL, this.panel);
  }

  togglePreview(id: string): void {
    this.previewOn[id] = !this.previewOn[id];
    lsSet(K_PREVIEW, this.previewOn);
  }

  setValues(id: string, values: Record<string, string>): void {
    this.values[id] = values;
    lsSet(K_VALUES(id), values);
  }
}

export const workbench = new WorkbenchStore();
