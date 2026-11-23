// Zero-jank buffer management, tab heap eviction, debounced background LSP/gutter
// scheduling, and undoable agent edit helpers for `IdeStore`.

import { tick } from 'svelte';
import { api, ApiError } from '../../../lib/api/client';
import {
  readIdeUndoDraft,
  removeIdeUndoDraft,
  writeIdeUndoDraftImmediate,
} from '../../../lib/components/cm-ide-drafts';
import { readIdePreferences } from '../../../lib/components/cm-ide-prefs';
import { confirmer } from '../../../lib/confirm.svelte';
import { normalizeMacPrivatePath } from '../../../lib/fileUri';
import { loadErrorText } from '../../../lib/loadError';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { findContainingRoot, isPathExternal } from '../shell/ideProjectHelpers';
import {
  detectIdeLanguage, detectTextEol, diskEolsOf, ideBasename, isReadOnlySdkOrCachePath,
  normalizeEditorLf, serializeTabDiskContent, sha256HexOfText,
  type IdeDiagnostic, type IdeEditorHandle, type IdeEditorSelectionRange, type IdeFileReadResp,
  type IdeGutterItem, type IdeOpenTab, type IdeProjectRoot, type IdeSymbolNode, type IdeTargetSpec,
} from './ideTypes';

const BUFFER_SYNC_DEBOUNCE_MS = 250;
const AUTO_SAVE_IDLE_MS = 1000;
const DRAFT_PERSIST_IDLE_MS = 2000;
const BG_ANALYZE_DEBOUNCE_MS = 280;
const HISTORY_SNAPSHOT_IDLE_MS = 4000;
const OPEN_TABS_KEY_PREFIX = 'otto.ide.openTabs.v1:';
interface PendingJob { timer: ReturnType<typeof setTimeout> | null; ctl: AbortController | null; seq: number; resolve: () => void }

let lruAccessClock = 0;

export function nextTabAccessStamp(): number {
  const now = Date.now();
  lruAccessClock = now > lruAccessClock ? now : lruAccessClock + 1;
  return lruAccessClock;
}

export function saveWorkspaceOpenTabs(root: string, tabs: readonly (string | Pick<IdeOpenTab, 'path' | 'projectScoped'>)[], activePath: string | null, splitPath: string | null): void {
  if (!root) return;
  try {
    if (typeof localStorage === 'undefined') return;
    const cap = Math.max(15, Math.min(40, readIdePreferences().maxOpenTabs || 20));
    const open = tabs.slice(0, cap);
    const paths = open.map((tab) => typeof tab === 'string' ? tab : tab.path);
    const projectScoped = open.flatMap((tab) => typeof tab !== 'string' && tab.projectScoped ? [tab.path] : []);
    localStorage.setItem(
      `${OPEN_TABS_KEY_PREFIX}${root}`,
      JSON.stringify({ paths, activePath, splitPath, projectScoped }),
    );
  } catch { /* ignore */ }
}

export function loadWorkspaceOpenTabs(root: string): { paths: string[]; activePath: string | null; splitPath: string | null; projectScoped: string[] } | null {
  if (!root) return null;
  try {
    if (typeof localStorage === 'undefined') return null;
    const raw = localStorage.getItem(`${OPEN_TABS_KEY_PREFIX}${root}`);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { paths?: string[]; activePath?: string | null; splitPath?: string | null; projectScoped?: string[] };
    if (!Array.isArray(parsed.paths) || parsed.paths.length === 0) return null;
    return { paths: parsed.paths, activePath: parsed.activePath ?? null, splitPath: parsed.splitPath ?? null, projectScoped: Array.isArray(parsed.projectScoped) ? parsed.projectScoped : [] };
  } catch {
    return null;
  }
}

type EvictionListener = (paths: ReadonlySet<string> | 'all') => void;
type FileSavedListener = (path: string, content: string) => void;

const evictionListeners = new Set<EvictionListener>();
const fileSavedListeners = new Set<FileSavedListener>();

/** Subscribe to tab/buffer evictions so `CodeEditor` can drop cached `EditorState`s immediately. */
export function subscribeIdeBufferEvictions(fn: EvictionListener): () => void {
  evictionListeners.add(fn);
  return () => evictionListeners.delete(fn);
}

/** Subscribe to completed disk/collab saves so LSP `didSave` and post-save hooks fire reliably. */
export function subscribeIdeFileSaved(fn: FileSavedListener): () => void {
  fileSavedListeners.add(fn);
  return () => fileSavedListeners.delete(fn);
}

const diskConflictListeners = new Set<(path: string) => void>();
/** An autosave found the disk copy changed (409): the editor's "changed on disk" banner
 *  takes it from there (re-reads, offers Compare / Reload / Keep mine) instead of a toast. */
export function subscribeIdeDiskConflict(fn: (path: string) => void): () => void {
  diskConflictListeners.add(fn);
  return () => diskConflictListeners.delete(fn);
}

function notifyEvicted(paths: ReadonlySet<string> | 'all'): void {
  for (const fn of evictionListeners) fn(paths);
}

function notifySaved(path: string, content: string): void {
  for (const fn of fileSavedListeners) fn(path, content);
}

export interface IdeBufferHost {
  target: IdeTargetSpec; readonly targetPayload: IdeTargetSpec; readonly projectRoots?: IdeProjectRoot[];
  tabs: IdeOpenTab[]; hasClosedAllTabs: boolean;
  activePath: string | null; splitPath: string | null; readonly activeTab: IdeOpenTab | null;
  gutterByPath: Record<string, IdeGutterItem[]>; symbolsByPath: Record<string, IdeSymbolNode[]>;
  symbolsLoading: boolean; problems: IdeDiagnostic[]; collabIsGuest: boolean; readonly canEditCollab: boolean;
  cursorInfo: { line: number; col: number; cursors: number; selections: { anchor: number; head: number }[] };
  resolvePath(p: string): string; targetQuery(extra?: Record<string, string | boolean | null | undefined>): string;
  replaceBufferContent?(path: string, nextContent: string, range?: { from: number; to: number }): boolean;
  openFile(rawPath: string, opts?: { line?: number; col?: number; preview?: boolean; inSplit?: boolean; reload?: boolean; background?: boolean; projectScoped?: boolean }): Promise<void>;
  closeSplit(): void; saveFile(path?: string, forceOverwrite?: boolean, opts?: { silent?: boolean }): Promise<boolean>;
  updateBuffer(path: string, content: string): void; sendCollabEdit(path: string, content: string, save?: boolean): Promise<boolean>;
  loadGutter(path: string, content?: string): Promise<void>; loadSymbols(path: string, content?: string): Promise<void>;
}

export class IdeBufferPerfController {
  readonly liveBuffers = new Map<string, string>();
  readonly conflictedPaths = new Set<string>();
  /** The save or buffer-format op in flight per path: a second save or format waits
   *  for it, so saves send the fresh sha256 and formatting never races autosave. */
  readonly savesInFlight = new Map<string, Promise<boolean>>();
  enqueuePathOp<T>(key: string, op: () => Promise<T>): Promise<T> {
    const prior = this.savesInFlight.get(key) ?? Promise.resolve(false);
    const run = prior.catch(() => false).then(() => op());
    const settled = run.then(() => true, () => false);
    this.savesInFlight.set(key, settled);
    const done = (): void => { if (this.savesInFlight.get(key) === settled) this.savesInFlight.delete(key); };
    settled.then(done, done);
    return run;
  }
  private syncTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private autoSaveTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private draftTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private historyTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private collabEditTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private lastSnapshotted = new Map<string, string>();
  private gutterJobs = new Map<string, PendingJob>();
  private symbolJobs = new Map<string, PendingJob>();
  private activeSymbolLoads = 0;

  private clearSync(p: string): void { const t = this.syncTimers.get(p); if (t) { clearTimeout(t); this.syncTimers.delete(p); } }
  private clearAutoSaveTimer(p: string): void { const t = this.autoSaveTimers.get(p); if (t) { clearTimeout(t); this.autoSaveTimers.delete(p); } }
  private clearDraftTimer(p: string): void { const t = this.draftTimers.get(p); if (t) { clearTimeout(t); this.draftTimers.delete(p); } }
  private clearHistoryTimer(p: string): void { const t = this.historyTimers.get(p); if (t) { clearTimeout(t); this.historyTimers.delete(p); } }
  clearCollabEditTimer(p: string): void { const t = this.collabEditTimers.get(p); if (t) { clearTimeout(t); this.collabEditTimers.delete(p); } }
  clearAllCollabEditTimers(): void { for (const t of this.collabEditTimers.values()) clearTimeout(t); this.collabEditTimers.clear(); }
  scheduleCollabEdit(fullPath: string, send: () => void, delayMs = 120): void {
    this.clearCollabEditTimer(fullPath);
    this.collabEditTimers.set(fullPath, setTimeout(() => { this.collabEditTimers.delete(fullPath); send(); }, delayMs));
  }
  scheduleAutoSave(host: IdeBufferHost, fullPath: string): void {
    this.clearAutoSaveTimer(fullPath);
    if (!readIdePreferences().autoSave || this.conflictedPaths.has(fullPath) || !host.canEditCollab) return;
    const tab = host.tabs.find((t) => t.path === fullPath);
    if (!tab || tab.readOnly) return;
    this.autoSaveTimers.set(fullPath, setTimeout(() => {
      this.autoSaveTimers.delete(fullPath);
      const cur = host.tabs.find((t) => t.path === fullPath);
      if (!cur || cur.readOnly || !this.isTabDirty(cur) || !readIdePreferences().autoSave || this.conflictedPaths.has(fullPath)) return;
      void host.saveFile(fullPath, false, { silent: true });
    }, AUTO_SAVE_IDLE_MS));
  }
  async flushDirtyAutoSave(host: IdeBufferHost, filterPath?: string): Promise<void> {
    if (filterPath) {
      const target = host.resolvePath(filterPath);
      this.clearAutoSaveTimer(target);
      this.flush(host, target);
    } else {
      for (const t of this.autoSaveTimers.values()) clearTimeout(t);
      this.autoSaveTimers.clear();
      this.flushAll(host);
    }
    if (!readIdePreferences().autoSave || !host.canEditCollab) return;
    const targetFilter = filterPath ? host.resolvePath(filterPath) : null;
    const dirtyTabs = host.tabs.filter(
      (t) => (!targetFilter || t.path === targetFilter) && !t.readOnly && this.isTabDirty(t) && !this.conflictedPaths.has(t.path),
    );
    for (let i = 0; i < dirtyTabs.length; i++) {
      await host.saveFile(dirtyTabs[i].path, false, { silent: true });
    }
  }
  private scheduleDraftPersist(fullPath: string, savedContent: string, draftContent: string): void {
    this.clearDraftTimer(fullPath);
    if (savedContent === draftContent) {
      removeIdeUndoDraft(fullPath);
      return;
    }
    this.draftTimers.set(fullPath, setTimeout(() => {
      this.draftTimers.delete(fullPath);
      writeIdeUndoDraftImmediate(fullPath, savedContent, draftContent);
    }, DRAFT_PERSIST_IDLE_MS));
  }
  pushHistorySnapshot(host: IdeBufferHost, fullPath: string, content: string, label = 'Change'): void {
    // Local History is a local-disk store: the daemon refuses SSH targets.
    if (host.collabIsGuest || host.target.kind !== 'local' || !host.target.root || !fullPath || fullPath.includes('://')) return;
    if (label === 'Change') {
      if (!readIdePreferences().localHistoryEnabled) return;
      if (this.lastSnapshotted.get(fullPath) === content) return;
    }
    this.lastSnapshotted.set(fullPath, content);
    const folders = host.targetPayload.project_folders ?? [];
    const root = folders.find((f) => fullPath === f || fullPath.startsWith(`${f}/`)) ?? host.target.root;
    void api.bg
      .post('/ide/history/snapshot', {
        target: { ...host.targetPayload, root, additional_roots: folders.filter((f) => f !== root) },
        path: fullPath,
        content,
        label,
      })
      .catch(() => {});
  }
  flushHistorySnapshot(host: IdeBufferHost, path?: string): void {
    const keys = path ? [host.resolvePath(path)] : [...this.historyTimers.keys()];
    for (const p of keys) {
      const hadTimer = this.historyTimers.has(p);
      this.clearHistoryTimer(p);
      if (hadTimer) {
        const tab = host.tabs.find((t) => t.path === p);
        if (tab) this.pushHistorySnapshot(host, p, this.getBuffer(host, p), 'Change');
      }
    }
  }
  private syncTab(tab: IdeOpenTab, text: string, immediateDraft = false): void {
    if (tab.content !== text) tab.content = text;
    const d = text !== tab.savedContent;
    if (tab.dirty !== d) tab.dirty = d;
    if (immediateDraft) {
      this.clearDraftTimer(tab.path);
      writeIdeUndoDraftImmediate(tab.path, tab.savedContent, text);
    } else {
      this.scheduleDraftPersist(tab.path, tab.savedContent, text);
    }
  }
  isTabDirty(tab: IdeOpenTab): boolean { return (this.liveBuffers.get(tab.path) ?? tab.content) !== tab.savedContent; }
  getBuffer(host: IdeBufferHost, fullPath: string): string {
    return this.liveBuffers.get(fullPath) ?? host.tabs.find((t) => t.path === fullPath)?.content ?? '';
  }
  recordEdit(host: IdeBufferHost, fullPath: string, content: string): boolean {
    const tab = host.tabs.find((t) => t.path === fullPath);
    if (!tab || (this.liveBuffers.get(fullPath) ?? tab.content) === content) return false;
    tab.lastAccessedAt = nextTabAccessStamp();
    this.liveBuffers.set(fullPath, content);
    const isDirty = content !== tab.savedContent;
    if ((tab.dirty ?? (tab.content !== tab.savedContent)) !== isDirty) tab.dirty = isDirty;
    if (tab.preview) tab.preview = false;
    if (isDirty) this.scheduleAutoSave(host, fullPath);
    else this.clearAutoSaveTimer(fullPath);
    this.clearSync(fullPath);
    this.syncTimers.set(fullPath, setTimeout(() => {
      this.syncTimers.delete(fullPath);
      const latest = this.liveBuffers.get(fullPath);
      if (latest === undefined) return;
      this.liveBuffers.delete(fullPath);
      const cur = host.tabs.find((t) => t.path === fullPath);
      if (cur) this.syncTab(cur, latest, false);
    }, BUFFER_SYNC_DEBOUNCE_MS));
    this.clearHistoryTimer(fullPath);
    if (readIdePreferences().localHistoryEnabled) {
      this.historyTimers.set(fullPath, setTimeout(() => {
        this.historyTimers.delete(fullPath);
        this.pushHistorySnapshot(host, fullPath, this.getBuffer(host, fullPath), 'Change');
      }, HISTORY_SNAPSHOT_IDLE_MS));
    }
    return true;
  }
  flush(host: IdeBufferHost, path?: string | null): string {
    const target = path ? host.resolvePath(path) : host.activePath;
    if (!target) return '';
    this.clearSync(target);
    const tab = host.tabs.find((t) => t.path === target);
    const live = this.liveBuffers.get(target);
    if (live !== undefined) { this.liveBuffers.delete(target); if (tab) this.syncTab(tab, live, true); return live; }
    if (!tab) return '';
    this.syncTab(tab, tab.content, true);
    return tab.content;
  }
  flushAll(host: IdeBufferHost): void {
    const pending = new Set<string>([...this.liveBuffers.keys(), ...this.draftTimers.keys()]);
    for (const p of pending) this.flush(host, p);
  }
  flushAndSnapshotAll(host: IdeBufferHost): void {
    this.flushAll(host);
    this.flushHistorySnapshot(host);
    for (let i = 0; i < host.tabs.length; i++) {
      const t = host.tabs[i];
      writeIdeUndoDraftImmediate(t.path, t.savedContent, t.content);
    }
    if (host.target.root) {
      saveWorkspaceOpenTabs(host.target.root, host.tabs, host.activePath, host.splitPath);
    }
  }
  private discardAllDrafts(host: IdeBufferHost): void {
    this.flushHistorySnapshot(host);
    for (const t of this.draftTimers.values()) clearTimeout(t);
    this.draftTimers.clear();
    for (let i = 0; i < host.tabs.length; i++) removeIdeUndoDraft(host.tabs[i].path);
    if (host.target.root) {
      saveWorkspaceOpenTabs(host.target.root, host.tabs, host.activePath, host.splitPath);
    }
  }
  markClean(tab: IdeOpenTab, content: string, sha256?: string): void {
    this.clearSync(tab.path); this.clearAutoSaveTimer(tab.path); this.clearDraftTimer(tab.path);
    this.liveBuffers.delete(tab.path); this.conflictedPaths.delete(tab.path);
    tab.content = content; tab.savedContent = content; tab.dirty = false;
    this.lastSnapshotted.set(tab.path, content);
    removeIdeUndoDraft(tab.path);
    if (sha256 !== undefined) tab.sha256 = sha256;
  }
  private cancelJob(map: Map<string, PendingJob>, path: string): number {
    const prev = map.get(path);
    if (!prev) return 0;
    if (prev.timer) clearTimeout(prev.timer);
    prev.ctl?.abort(); prev.resolve(); map.delete(path);
    return prev.seq;
  }
  cancelHiddenLoads(host: IdeBufferHost): void {
    for (const map of [this.gutterJobs, this.symbolJobs]) {
      for (const p of [...map.keys()]) {
        if (p !== host.activePath && p !== host.splitPath && host.tabs.some((t) => t.path === p)) this.cancelJob(map, p);
      }
    }
  }
  evictPaths(host: IdeBufferHost, paths: Iterable<string>, clearProblems = true): void {
    const closed = new Set<string>(paths);
    if (closed.size === 0) return;
    for (const p of closed) {
      this.clearSync(p); this.clearAutoSaveTimer(p); this.clearDraftTimer(p); this.clearHistoryTimer(p); this.clearCollabEditTimer(p);
      this.liveBuffers.delete(p); this.lastSnapshotted.delete(p); this.conflictedPaths.delete(p);
      this.cancelJob(this.gutterJobs, p); this.cancelJob(this.symbolJobs, p);
      if (clearProblems) removeIdeUndoDraft(p);
    }
    if (Object.keys(host.gutterByPath).some((k) => closed.has(k))) host.gutterByPath = Object.fromEntries(Object.entries(host.gutterByPath).filter(([k]) => !closed.has(k)));
    if (Object.keys(host.symbolsByPath).some((k) => closed.has(k))) host.symbolsByPath = Object.fromEntries(Object.entries(host.symbolsByPath).filter(([k]) => !closed.has(k)));
    if (clearProblems && host.problems.some((d) => closed.has(d.path))) host.problems = host.problems.filter((d) => !closed.has(d.path));
    notifyEvicted(closed);
  }
  /** Drop every buffer (a target switch). `discardDirty`: the person chose "Discard & switch"
   *  (or storage was cleared), so unsaved edits are NOT written back as drafts that the next
   *  open would resurrect (and autosave would then write). Local History still gets its
   *  snapshot, which is how a discard stays recoverable. */
  resetAll(host: IdeBufferHost, opts: { discardDirty?: boolean } = {}): void {
    if (opts.discardDirty) this.discardAllDrafts(host);
    else this.flushAndSnapshotAll(host);
    for (const t of this.syncTimers.values()) clearTimeout(t);
    for (const t of this.autoSaveTimers.values()) clearTimeout(t);
    for (const t of this.draftTimers.values()) clearTimeout(t);
    for (const t of this.historyTimers.values()) clearTimeout(t);
    this.syncTimers.clear(); this.autoSaveTimers.clear(); this.draftTimers.clear(); this.historyTimers.clear(); this.clearAllCollabEditTimers();
    this.liveBuffers.clear(); this.lastSnapshotted.clear(); this.conflictedPaths.clear();
    for (const p of [...this.gutterJobs.keys()]) this.cancelJob(this.gutterJobs, p);
    for (const p of [...this.symbolJobs.keys()]) this.cancelJob(this.symbolJobs, p);
    this.activeSymbolLoads = 0; host.symbolsLoading = false;
    notifyEvicted('all');
  }
  private runDebouncedJob<T>(
    host: IdeBufferHost, map: Map<string, PendingJob>, path: string,
    trackSymbols: boolean, exec: (sig: AbortSignal) => Promise<T>, apply: (res: T) => void,
  ): Promise<void> {
    if (!host.target.root || host.collabIsGuest) return Promise.resolve();
    const seq = this.cancelJob(map, path) + 1;
    return new Promise<void>((resolve) => {
      const job: PendingJob = { timer: null, ctl: null, seq, resolve };
      job.timer = setTimeout(async () => {
        job.timer = null;
        if (map.get(path)?.seq !== seq || !host.target.root || host.collabIsGuest) { resolve(); return; }
        const ctl = (job.ctl = new AbortController());
        if (trackSymbols) { this.activeSymbolLoads += 1; host.symbolsLoading = true; }
        try {
          const res = await exec(ctl.signal);
          if (!ctl.signal.aborted && map.get(path)?.seq === seq && (host.tabs.length === 0 || host.tabs.some((t) => t.path === path))) apply(res);
        } catch { /* aborted or non-fatal */ } finally {
          if (trackSymbols) { this.activeSymbolLoads = Math.max(0, this.activeSymbolLoads - 1); host.symbolsLoading = this.activeSymbolLoads > 0; }
          if (map.get(path)?.seq === seq) map.delete(path);
          resolve();
        }
      }, BG_ANALYZE_DEBOUNCE_MS);
      map.set(path, job);
    });
  }
  scheduleGutter(host: IdeBufferHost, path: string, content?: string): Promise<void> {
    return this.runDebouncedJob(host, this.gutterJobs, path, false,
      (sig) => api.bg.post<{ items: IdeGutterItem[] }>('/ide/gutter', { target: host.targetPayload, path, content: content ?? this.getBuffer(host, path) }, sig),
      (resp) => { host.gutterByPath = { ...host.gutterByPath, [path]: resp.items ?? [] }; });
  }
  scheduleSymbols(host: IdeBufferHost, path: string, content?: string): Promise<void> {
    return this.runDebouncedJob(host, this.symbolJobs, path, true,
      (sig) => api.bg.post<{ symbols: IdeSymbolNode[] }>('/ide/lsp/symbols', { target: host.targetPayload, path, content: content ?? this.getBuffer(host, path) }, sig),
      (resp) => { host.symbolsByPath = { ...host.symbolsByPath, [path]: resp.symbols ?? [] }; });
  }
}

export async function enforceTabCap(
  host: Pick<IdeBufferHost, 'tabs' | 'activePath' | 'splitPath' | 'canEditCollab' | 'saveFile'>,
  perf: Pick<IdeBufferPerfController, 'isTabDirty' | 'evictPaths' | 'conflictedPaths'>,
  maxTabs = readIdePreferences().maxOpenTabs,
  protectedPaths: Iterable<string> = [],
): Promise<string[]> {
  const cap = Math.max(15, Math.min(40, Math.floor(maxTabs || 20)));
  if (host.tabs.length <= cap) return [];
  const keep = new Set<string>(protectedPaths);
  if (host.activePath) keep.add(host.activePath);
  if (host.splitPath) keep.add(host.splitPath);
  const autoSave = readIdePreferences().autoSave;
  const evicted: string[] = [];

  while (host.tabs.length > cap) {
    const candidates = host.tabs
      .filter((t) => !t.pinned && !keep.has(t.path) && (autoSave || !perf.isTabDirty(t)))
      .sort((a, b) => (a.lastAccessedAt ?? 0) - (b.lastAccessedAt ?? 0));
    const victim = candidates[0];
    if (!victim) break;
    if (perf.isTabDirty(victim)) {
      if (!autoSave || victim.readOnly || !host.canEditCollab || perf.conflictedPaths.has(victim.path)) {
        keep.add(victim.path);
        continue;
      }
      const saved = await host.saveFile(victim.path, false, { silent: true });
      if (!saved) {
        keep.add(victim.path);
        continue;
      }
    }
    host.tabs = host.tabs.filter((t) => t.path !== victim.path);
    evicted.push(victim.path);
    perf.evictPaths(host as IdeBufferHost, [victim.path], true);
  }
  return evicted;
}

export async function performCloseTab(host: IdeBufferHost, perf: IdeBufferPerfController, path: string): Promise<void> {
  perf.flush(host, path);
  const tab = host.tabs.find((t) => t.path === path);
  if (!tab) return;
  if (perf.isTabDirty(tab)) {
    const prefs = readIdePreferences();
    if (prefs.autoSave && !tab.readOnly && host.canEditCollab && !perf.conflictedPaths.has(path)) {
      await host.saveFile(path, false, { silent: true });
    }
    if (perf.isTabDirty(tab)) {
      const res = await confirmer.choose(`“${tab.name}” has unsaved changes.`, {
        title: 'Unsaved changes', options: [{ label: 'Save', value: 'save', kind: 'primary' }, { label: 'Discard changes', value: 'discard', kind: 'danger' }],
      });
      if (!res.value || (res.value === 'save' && !(await host.saveFile(path)))) return;
    }
  }
  const idx = host.tabs.findIndex((t) => t.path === path);
  host.tabs = host.tabs.filter((t) => t.path !== path);
  perf.evictPaths(host, [path], true);
  if (host.tabs.length === 0) host.hasClosedAllTabs = true;
  if (host.activePath === path) host.activePath = host.tabs[Math.min(idx, host.tabs.length - 1)]?.path ?? null;
  if (host.splitPath === path) host.closeSplit();
}

export async function performOpenFile(
  host: IdeBufferHost & {
    readonly projectRoots: IdeProjectRoot[];
    recentPaths: string[];
    splitDirection: 'right' | 'down' | null;
    sharedBuffers: Record<string, { content: string; version: number }>;
    openInSplit(path: string, dir: 'right' | 'down'): void;
    jumpTo(path: string, line: number, col?: number): void;
    pushNavLocation(path: string, line: number, col?: number): void;
  },
  perf: IdeBufferPerfController,
  isNavigatingHistory: boolean,
  rawPath: string,
  opts: { line?: number; col?: number; preview?: boolean; inSplit?: boolean; reload?: boolean; background?: boolean; projectScoped?: boolean } = {},
): Promise<void> {
  const fullPath = host.resolvePath(rawPath);
  if (!opts.background && !isNavigatingHistory && host.activePath) host.pushNavLocation(host.activePath, host.cursorInfo.line, host.cursorInfo.col);
  host.recentPaths = [fullPath, ...host.recentPaths.filter((p) => p !== fullPath)].slice(0, 20);
  const isGuest = host.collabIsGuest;
  const external = isPathExternal(fullPath, host.projectRoots);
  const existing = host.tabs.find((t) => t.path === fullPath);
  const projectScoped = external && Boolean(existing?.projectScoped || (opts.projectScoped && !existing?.dirty));
  const sdkReadOnly = projectScoped || isReadOnlySdkOrCachePath(fullPath);
  const parentDir = fullPath.lastIndexOf('/') > 0 ? fullPath.slice(0, fullPath.lastIndexOf('/')) : host.target.root;
  const containingRoot = host.target.kind === 'local' ? findContainingRoot(fullPath, host.projectRoots) : null;
  const readFileUrl = () => {
    if (external && !sdkReadOnly && host.target.kind === 'local' && fullPath.startsWith('/')) {
      return `/ide/fs/file?${new URLSearchParams({ kind: 'local', root: parentDir, path: fullPath }).toString()}`;
    }
    if (containingRoot && !containingRoot.isPrimary && host.target.kind === 'local') {
      return `/ide/fs/file?${host.targetQuery({ root: containingRoot.path, path: fullPath })}`;
    }
    return `/ide/fs/file?${host.targetQuery({ path: fullPath })}`;
  };
  if (existing) {
    if (projectScoped) {
      existing.projectScoped = true;
      existing.readOnly = true;
    }
    existing.preview = false;
    existing.lastAccessedAt = nextTabAccessStamp();
    if (opts.inSplit) host.openInSplit(fullPath, host.splitDirection ?? 'right'); else if (!opts.background) host.activePath = fullPath;
    if ((existing.error || opts.reload) && !isGuest) {
      existing.error = null; existing.loading = true;
      try {
        const res = await api.get<IdeFileReadResp>(readFileUrl());
        const eol = detectTextEol(res.content);
        const normContent = normalizeEditorLf(res.content);
        existing.eol = eol; existing.diskEols = diskEolsOf(res.content);
        perf.markClean(existing, normContent, res.sha256);
        existing.language = res.language || detectIdeLanguage(fullPath);
        existing.truncated = Boolean(res.truncated);
        existing.binary = Boolean(res.binary);
        existing.readOnly = Boolean(res.read_only) || Boolean(res.truncated) || sdkReadOnly;
        existing.loading = false;
      } catch (e) { existing.loading = false; existing.error = loadErrorText(e); }
    }
    if (!opts.background && opts.line) host.jumpTo(fullPath, opts.line, opts.col ?? 1);
    if (!opts.background && !isNavigatingHistory) host.pushNavLocation(fullPath, opts.line ?? 1, opts.col ?? 1);
    return;
  }
  const rawShared = host.sharedBuffers[fullPath]?.content ?? '';
  const sharedEol = detectTextEol(rawShared);
  const buf = normalizeEditorLf(rawShared);
  const placeholder: IdeOpenTab = {
    path: fullPath, name: ideBasename(fullPath), content: buf, savedContent: buf, dirty: false, eol: sharedEol, sha256: '',
    language: detectIdeLanguage(fullPath), readOnly: sdkReadOnly, truncated: false, loading: !isGuest, error: null,
    preview: false, pinned: false, external, projectScoped, lastAccessedAt: nextTabAccessStamp(),
  };
  host.tabs = [...host.tabs, placeholder];
  if (opts.inSplit) host.openInSplit(fullPath, host.splitDirection ?? 'right'); else if (!opts.background) host.activePath = fullPath;
  await enforceTabCap(host, perf, readIdePreferences().maxOpenTabs, [fullPath]);
  if (!opts.background && !isNavigatingHistory) host.pushNavLocation(fullPath, opts.line ?? 1, opts.col ?? 1);
  if (isGuest) { if (!opts.background && opts.line) host.jumpTo(fullPath, opts.line, opts.col ?? 1); return; }
  try {
    const res = await api.get<IdeFileReadResp>(readFileUrl());
    const eol = detectTextEol(res.content);
    const normContent = normalizeEditorLf(res.content);
    const isReadOnly = Boolean(res.read_only) || Boolean(res.truncated) || sdkReadOnly;
    const tab = host.tabs.find((t) => t.path === fullPath);
    if (tab) {
      tab.eol = eol; tab.diskEols = diskEolsOf(res.content);
      const persisted = !opts.reload ? readIdeUndoDraft(fullPath) : null;
      const persistedSaved = persisted && typeof persisted.savedContent === 'string' ? normalizeEditorLf(persisted.savedContent) : null;
      const persistedDraft = persisted && typeof persisted.draftContent === 'string' ? normalizeEditorLf(persisted.draftContent) : null;
      if (
        persistedSaved !== null &&
        persistedDraft !== null &&
        persistedSaved === normContent &&
        persistedDraft !== normContent &&
        !isReadOnly
      ) {
        tab.savedContent = normContent;
        tab.content = persistedDraft;
        tab.dirty = true;
        tab.sha256 = res.sha256;
      } else {
        perf.markClean(tab, normContent, res.sha256);
      }
      tab.language = res.language || detectIdeLanguage(fullPath);
      tab.truncated = Boolean(res.truncated);
      tab.binary = Boolean(res.binary);
      tab.readOnly = isReadOnly;
      tab.loading = false;
      if (host.target.root) saveWorkspaceOpenTabs(host.target.root, host.tabs, host.activePath, host.splitPath);
    }
    if (!opts.background && opts.line) host.jumpTo(fullPath, opts.line, opts.col ?? 1);
    if (host.activePath === fullPath || host.splitPath === fullPath) {
      void host.loadGutter(fullPath, normContent);
      void host.loadSymbols(fullPath, normContent);
    }
  } catch (e) {
    const tab = host.tabs.find((t) => t.path === fullPath);
    if (tab) { tab.loading = false; tab.error = loadErrorText(e); }
  }
}

export function performSaveFile(
  host: IdeBufferHost,
  perf: IdeBufferPerfController,
  path?: string,
  forceOverwrite = false,
  opts: { silent?: boolean } = {},
): Promise<boolean> {
  const key = (path ? host.resolvePath(path) : host.activePath) ?? '';
  return perf.enqueuePathOp(key, () => saveFileNow(host, perf, key || undefined, forceOverwrite, opts));
}

async function saveFileNow(
  host: IdeBufferHost,
  perf: IdeBufferPerfController,
  path: string | undefined,
  forceOverwrite: boolean,
  opts: { silent?: boolean },
): Promise<boolean> {
  const targetPath = path ? host.resolvePath(path) : host.activePath;
  const tab = targetPath ? host.tabs.find((t) => t.path === targetPath) : null;
  // A tab still loading (or whose read failed) holds a placeholder, not the file: saving it
  // would write an empty file over the real one. Checked before the flush, which would
  // otherwise also drop the crash-recovery draft the load is about to restore.
  if (!targetPath || !tab || tab.loading || tab.error) return false;
  const rawFlushed = perf.flush(host, targetPath);
  const contentToSave = normalizeEditorLf(rawFlushed);
  if (tab.readOnly || tab.truncated || !host.canEditCollab) return false;
  if (host.collabIsGuest) {
    const ok = await host.sendCollabEdit(targetPath, contentToSave, true);
    if (!ok) return false;
    tab.savedContent = contentToSave; tab.dirty = perf.getBuffer(host, targetPath) !== contentToSave;
    perf.conflictedPaths.delete(targetPath);
    removeIdeUndoDraft(targetPath);
    notifySaved(targetPath, contentToSave);
    if (!opts.silent) toasts.success('Saved to shared session', tab.name);
    return true;
  }
  try {
    const prefs = readIdePreferences();
    const folders = host.targetPayload.project_folders ?? [];
    const inProject = folders.some((r) => targetPath === r || targetPath.startsWith(`${r}/`));
    const parentDir = targetPath.lastIndexOf('/') > 0 ? targetPath.slice(0, targetPath.lastIndexOf('/')) : host.target.root;
    const saveTarget = !inProject && host.target.kind === 'local'
      ? { ...host.targetPayload, additional_roots: [...(host.targetPayload.additional_roots ?? []), parentDir] }
      : host.targetPayload;
    const diskContent = serializeTabDiskContent(tab, contentToSave);
    const resp = await api.put<{ path: string; size: number; sha256: string }>('/ide/fs/file', {
      target: saveTarget, path: targetPath, content: diskContent,
      expected_sha256: forceOverwrite ? undefined : await expectedDiskSha(tab),
      local_history_enabled: prefs.localHistoryEnabled,
      local_history_retention_days: prefs.localHistoryRetentionDays,
    });
    tab.savedContent = contentToSave; tab.sha256 = resp.sha256; tab.diskEols = diskEolsOf(diskContent);
    perf.conflictedPaths.delete(targetPath);
    const latest = perf.getBuffer(host, targetPath);
    tab.dirty = latest !== tab.savedContent;
    if (!tab.dirty) removeIdeUndoDraft(targetPath);
    notifySaved(targetPath, contentToSave);
    void host.loadGutter(targetPath, latest); void host.loadSymbols(targetPath, latest);
    return true;
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) {
      perf.conflictedPaths.add(targetPath);
      if (opts.silent) {
        if (diskConflictListeners.size) for (const fn of diskConflictListeners) fn(targetPath);
        else toasts.error(`Conflict on disk for ${tab.name}`, 'Save manually to overwrite or reload from disk.');
        return false;
      }
      const choice = await confirmer.choose(`“${tab.name}” changed on disk since you opened it.`, {
        title: 'File conflict on disk', options: [{ label: 'Overwrite disk', value: 'overwrite', kind: 'danger' }, { label: 'Reload from disk', value: 'reload', kind: 'normal' }],
      });
      // Inside this path's queued save, so not via host.saveFile (it would wait on itself).
      if (choice.value === 'overwrite') return saveFileNow(host, perf, targetPath, true, {});
      if (choice.value === 'reload') { await host.openFile(targetPath, { reload: true }); }
      return false;
    }
    if (!opts.silent) toastError(`Couldn’t save ${tab.name}`, e);
    return false;
  }
}

/** The precondition for overwriting an existing file: the sha the tab was read with, or
 *  (a tab whose sha a collab save cleared) the hash of what the tab believes is on disk.
 *  Without one the daemon writes unconditionally, so this only gives up where WebCrypto
 *  is missing. */
export async function expectedDiskSha(tab: IdeOpenTab): Promise<string | undefined> {
  if (tab.sha256) return tab.sha256;
  return sha256HexOfText(serializeTabDiskContent(tab, tab.savedContent));
}

/** Re-read `paths` from disk after a workspace operation (Replace in Path, Rename) wrote
 *  them. A tab that still holds unsaved edits (its save failed or is in conflict) keeps
 *  them: it is not marked clean, its stale sha makes the next save ask, and its path is
 *  returned so the caller can say so. */
export async function reloadTabsFromDisk(host: IdeBufferHost, perf: IdeBufferPerfController, paths: Set<string>): Promise<string[]> {
  const kept: string[] = [];
  const normalized = new Set<string>();
  for (const p of paths) {
    normalized.add(p);
    normalized.add(normalizeMacPrivatePath(p));
    const resolved = host.resolvePath(p);
    normalized.add(resolved);
    normalized.add(normalizeMacPrivatePath(resolved));
  }
  const matching = host.tabs.filter(
    (tab) => normalized.has(tab.path) || normalized.has(normalizeMacPrivatePath(tab.path)),
  );
  await Promise.allSettled(
    matching.map(async (tab) => {
      perf.flush(host, tab.path);
      if (perf.isTabDirty(tab)) { kept.push(tab.path); return; }
      const before = perf.getBuffer(host, tab.path);
      const containing = host.target.kind === 'local' && host.projectRoots
        ? findContainingRoot(tab.path, host.projectRoots)
        : null;
      const query = containing && !containing.isPrimary && host.target.kind === 'local'
        ? host.targetQuery({ root: containing.path, path: tab.path })
        : host.targetQuery({ path: tab.path });
      const fresh = await api.get<IdeFileReadResp>(`/ide/fs/file?${query}`);
      // Typed into while the read was in flight: those keystrokes win over the disk copy.
      if (!host.tabs.includes(tab) || perf.getBuffer(host, tab.path) !== before) { kept.push(tab.path); return; }
      const eol = detectTextEol(fresh.content);
      const normContent = normalizeEditorLf(fresh.content);
      tab.eol = eol; tab.diskEols = diskEolsOf(fresh.content);
      tab.truncated = Boolean(fresh.truncated);
      tab.readOnly = Boolean(fresh.read_only) || Boolean(fresh.truncated) || Boolean(tab.external && tab.projectScoped) || isReadOnlySdkOrCachePath(tab.path);
      tab.error = null;
      host.replaceBufferContent?.(tab.path, normContent);
      perf.markClean(tab, normContent, fresh.sha256);
      notifySaved(tab.path, normContent);
      if (tab.path === host.activePath || tab.path === host.splitPath) {
        void host.loadGutter(tab.path, normContent);
        void host.loadSymbols(tab.path, normContent);
      }
    }),
  );
  return kept;
}

export function computeFallbackSelection(content: string, cursorInfo: IdeBufferHost['cursorInfo']): IdeEditorSelectionRange {
  const first = cursorInfo.selections[0];
  if (first && first.anchor !== first.head) {
    const from = Math.max(0, Math.min(content.length, Math.min(first.anchor, first.head)));
    const to = Math.max(from, Math.min(content.length, Math.max(first.anchor, first.head)));
    const startLine = content.slice(0, from).split('\n').length;
    const slice = content.slice(from, to);
    return { from, to, startLine, endLine: startLine + Math.max(0, (slice.endsWith('\n') ? slice.slice(0, -1) : slice).split('\n').length - 1), text: slice };
  }
  const lines = content.split('\n');
  const ln = Math.max(1, Math.min(lines.length, cursorInfo.line || 1));
  let from = 0;
  for (let i = 0; i < ln - 1; i++) from += lines[i].length + 1;
  const text = lines[ln - 1] ?? '';
  return { from, to: from + text.length, startLine: ln, endLine: ln, text };
}

export function applyFallbackPrefixSuffix(cur: string, cursorInfo: IdeBufferHost['cursorInfo'], prefix: string, suffix: string): string {
  const lines = cur.split('\n');
  const selectedLines = new Set<number>();
  if (cursorInfo.selections.length > 0) {
    for (const sel of cursorInfo.selections) {
      for (let ln = cur.slice(0, Math.min(sel.anchor, sel.head)).split('\n').length - 1, e = cur.slice(0, Math.max(sel.anchor, sel.head)).split('\n').length - 1; ln <= e; ln++) selectedLines.add(ln);
    }
  } else selectedLines.add(Math.max(0, cursorInfo.line - 1));
  return lines.map((txt, idx) => (selectedLines.has(idx) ? `${prefix}${txt}${suffix}` : txt)).join('\n');
}

export async function performAgentEdit(
  host: IdeBufferHost, perf: IdeBufferPerfController, handleForPath: (resolvedPath: string) => IdeEditorHandle | null,
  rawPath: string, nextContent: string, range?: { from: number; to: number },
): Promise<boolean> {
  const fullPath = host.resolvePath(rawPath);
  const wasMounted = (host.activePath === fullPath || host.splitPath === fullPath) && host.tabs.some((t) => t.path === fullPath && !t.loading);
  if (host.splitPath !== fullPath) await host.openFile(fullPath, { preview: false });
  const tab = host.tabs.find((t) => t.path === fullPath);
  if (!tab || tab.readOnly || !host.canEditCollab) return false;
  if (!wasMounted) await tick();
  if (handleForPath(fullPath)?.applyAgentEdit?.(nextContent, range)) { perf.flush(host, fullPath); return true; }
  const cur = perf.getBuffer(host, fullPath);
  const from = range ? Math.max(0, Math.min(cur.length, range.from)) : 0;
  const to = range ? Math.max(from, Math.min(cur.length, range.to)) : cur.length;
  host.updateBuffer(fullPath, range ? `${cur.slice(0, from)}${nextContent}${cur.slice(to)}` : nextContent);
  perf.flush(host, fullPath);
  return true;
}
