// Zero-jank buffer management, tab heap eviction, debounced background LSP/gutter
// scheduling, and undoable agent edit helpers for `IdeStore`.

import { tick } from 'svelte';
import { api, ApiError } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import type {
  IdeDiagnostic, IdeEditorHandle, IdeEditorSelectionRange, IdeFileReadResp,
  IdeGutterItem, IdeOpenTab, IdeSymbolNode, IdeTargetSpec,
} from './ideTypes';

const BUFFER_SYNC_DEBOUNCE_MS = 250;
const BG_ANALYZE_DEBOUNCE_MS = 280;
interface PendingJob { timer: ReturnType<typeof setTimeout> | null; ctl: AbortController | null; seq: number; resolve: () => void }

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

function notifyEvicted(paths: ReadonlySet<string> | 'all'): void {
  for (const fn of evictionListeners) fn(paths);
}

function notifySaved(path: string, content: string): void {
  for (const fn of fileSavedListeners) fn(path, content);
}

export interface IdeBufferHost {
  target: IdeTargetSpec; readonly targetPayload: IdeTargetSpec; tabs: IdeOpenTab[]; hasClosedAllTabs: boolean;
  activePath: string | null; splitPath: string | null; readonly activeTab: IdeOpenTab | null;
  gutterByPath: Record<string, IdeGutterItem[]>; symbolsByPath: Record<string, IdeSymbolNode[]>;
  symbolsLoading: boolean; problems: IdeDiagnostic[]; collabIsGuest: boolean; readonly canEditCollab: boolean;
  cursorInfo: { line: number; col: number; cursors: number; selections: { anchor: number; head: number }[] };
  resolvePath(p: string): string; targetQuery(extra?: Record<string, string | boolean | null | undefined>): string;
  openFile(rawPath: string, opts?: { line?: number; col?: number; preview?: boolean; inSplit?: boolean }): Promise<void>;
  closeSplit(): void; saveFile(path?: string, forceOverwrite?: boolean): Promise<boolean>;
  updateBuffer(path: string, content: string): void; sendCollabEdit(path: string, content: string, save?: boolean): Promise<void>;
  loadGutter(path: string, content?: string): Promise<void>; loadSymbols(path: string, content?: string): Promise<void>;
}

export class IdeBufferPerfController {
  readonly liveBuffers = new Map<string, string>();
  private syncTimers = new Map<string, ReturnType<typeof setTimeout>>();
  private gutterJobs = new Map<string, PendingJob>();
  private symbolJobs = new Map<string, PendingJob>();
  private activeSymbolLoads = 0;

  private clearSync(p: string): void { const t = this.syncTimers.get(p); if (t) { clearTimeout(t); this.syncTimers.delete(p); } }
  private syncTab(tab: IdeOpenTab, text: string): void {
    if (tab.content !== text) tab.content = text;
    const d = text !== tab.savedContent;
    if (tab.dirty !== d) tab.dirty = d;
  }
  isTabDirty(tab: IdeOpenTab): boolean { return (this.liveBuffers.get(tab.path) ?? tab.content) !== tab.savedContent; }
  getBuffer(host: IdeBufferHost, fullPath: string): string {
    return this.liveBuffers.get(fullPath) ?? host.tabs.find((t) => t.path === fullPath)?.content ?? '';
  }
  recordEdit(host: IdeBufferHost, fullPath: string, content: string): boolean {
    const tab = host.tabs.find((t) => t.path === fullPath);
    if (!tab || (this.liveBuffers.get(fullPath) ?? tab.content) === content) return false;
    this.liveBuffers.set(fullPath, content);
    const isDirty = content !== tab.savedContent;
    if ((tab.dirty ?? (tab.content !== tab.savedContent)) !== isDirty) tab.dirty = isDirty;
    if (tab.preview) tab.preview = false;
    this.clearSync(fullPath);
    this.syncTimers.set(fullPath, setTimeout(() => {
      this.syncTimers.delete(fullPath);
      const latest = this.liveBuffers.get(fullPath);
      if (latest === undefined) return;
      this.liveBuffers.delete(fullPath);
      const cur = host.tabs.find((t) => t.path === fullPath);
      if (cur) this.syncTab(cur, latest);
    }, BUFFER_SYNC_DEBOUNCE_MS));
    return true;
  }
  flush(host: IdeBufferHost, path?: string | null): string {
    const target = path ? host.resolvePath(path) : host.activePath;
    if (!target) return '';
    this.clearSync(target);
    const tab = host.tabs.find((t) => t.path === target);
    const live = this.liveBuffers.get(target);
    if (live !== undefined) { this.liveBuffers.delete(target); if (tab) this.syncTab(tab, live); return live; }
    if (!tab) return '';
    this.syncTab(tab, tab.content);
    return tab.content;
  }
  flushAll(host: IdeBufferHost): void { for (const p of [...this.liveBuffers.keys()]) this.flush(host, p); }
  markClean(tab: IdeOpenTab, content: string, sha256?: string): void {
    this.clearSync(tab.path); this.liveBuffers.delete(tab.path);
    tab.content = content; tab.savedContent = content; tab.dirty = false;
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
    for (const p of closed) { this.clearSync(p); this.liveBuffers.delete(p); this.cancelJob(this.gutterJobs, p); this.cancelJob(this.symbolJobs, p); }
    if (Object.keys(host.gutterByPath).some((k) => closed.has(k))) host.gutterByPath = Object.fromEntries(Object.entries(host.gutterByPath).filter(([k]) => !closed.has(k)));
    if (Object.keys(host.symbolsByPath).some((k) => closed.has(k))) host.symbolsByPath = Object.fromEntries(Object.entries(host.symbolsByPath).filter(([k]) => !closed.has(k)));
    if (clearProblems && host.problems.some((d) => closed.has(d.path))) host.problems = host.problems.filter((d) => !closed.has(d.path));
    notifyEvicted(closed);
  }
  resetAll(host: IdeBufferHost): void {
    for (const t of this.syncTimers.values()) clearTimeout(t);
    this.syncTimers.clear(); this.liveBuffers.clear();
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

export async function performCloseTab(host: IdeBufferHost, perf: IdeBufferPerfController, path: string): Promise<void> {
  perf.flush(host, path);
  const tab = host.tabs.find((t) => t.path === path);
  if (!tab) return;
  if (perf.isTabDirty(tab)) {
    const res = await confirmer.choose(`“${tab.name}” has unsaved changes.`, {
      title: 'Unsaved changes', options: [{ label: 'Save', value: 'save', kind: 'primary' }, { label: 'Discard changes', value: 'discard', kind: 'danger' }],
    });
    if (!res.value || (res.value === 'save' && !(await host.saveFile(path)))) return;
  }
  const idx = host.tabs.findIndex((t) => t.path === path);
  host.tabs = host.tabs.filter((t) => t.path !== path);
  perf.evictPaths(host, [path], true);
  if (host.tabs.length === 0) host.hasClosedAllTabs = true;
  if (host.activePath === path) host.activePath = host.tabs[Math.min(idx, host.tabs.length - 1)]?.path ?? null;
  if (host.splitPath === path) host.closeSplit();
}

export async function performSaveFile(host: IdeBufferHost, perf: IdeBufferPerfController, path?: string, forceOverwrite = false): Promise<boolean> {
  const targetPath = path ? host.resolvePath(path) : host.activePath;
  const contentToSave = perf.flush(host, targetPath);
  const tab = targetPath ? host.tabs.find((t) => t.path === targetPath) : null;
  if (!targetPath || !tab || tab.readOnly || !host.canEditCollab) return false;
  if (host.collabIsGuest) {
    await host.sendCollabEdit(targetPath, contentToSave, true);
    tab.savedContent = contentToSave; tab.dirty = perf.getBuffer(host, targetPath) !== contentToSave;
    notifySaved(targetPath, contentToSave);
    toasts.success('Saved to shared session', tab.name);
    return true;
  }
  try {
    const resp = await api.put<{ path: string; size: number; sha256: string }>('/ide/fs/file', {
      target: host.targetPayload, path: targetPath, content: contentToSave,
      expected_sha256: forceOverwrite || !tab.sha256 ? undefined : tab.sha256,
    });
    tab.savedContent = contentToSave; tab.sha256 = resp.sha256;
    const latest = perf.getBuffer(host, targetPath);
    tab.dirty = latest !== tab.savedContent;
    notifySaved(targetPath, contentToSave);
    void host.loadGutter(targetPath, latest); void host.loadSymbols(targetPath, latest);
    return true;
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) {
      const choice = await confirmer.choose(`“${tab.name}” changed on disk since you opened it.`, {
        title: 'File conflict on disk', options: [{ label: 'Overwrite disk', value: 'overwrite', kind: 'danger' }, { label: 'Reload from disk', value: 'reload', kind: 'normal' }],
      });
      if (choice.value === 'overwrite') return host.saveFile(targetPath, true);
      if (choice.value === 'reload') { host.tabs = host.tabs.filter((t) => t.path !== targetPath); perf.evictPaths(host, [targetPath], false); await host.openFile(targetPath); }
      return false;
    }
    toastError(`Couldn’t save ${tab.name}`, e);
    return false;
  }
}

function stripMacPrivatePrefix(p: string): string {
  return p.startsWith('/private/var/') || p.startsWith('/private/tmp/') ? p.slice('/private'.length) : p;
}

export async function reloadTabsFromDisk(host: IdeBufferHost, perf: IdeBufferPerfController, paths: Set<string>): Promise<void> {
  const normalized = new Set<string>();
  for (const p of paths) {
    normalized.add(p);
    normalized.add(stripMacPrivatePrefix(p));
    const resolved = host.resolvePath(p);
    normalized.add(resolved);
    normalized.add(stripMacPrivatePrefix(resolved));
  }
  for (const tab of host.tabs) {
    if (normalized.has(tab.path) || normalized.has(stripMacPrivatePrefix(tab.path))) {
      const fresh = await api.get<IdeFileReadResp>(`/ide/fs/file?${host.targetQuery({ path: tab.path })}`);
      perf.markClean(tab, fresh.content, fresh.sha256);
      notifySaved(tab.path, fresh.content);
      if (tab.path === host.activePath || tab.path === host.splitPath) {
        void host.loadGutter(tab.path, fresh.content);
        void host.loadSymbols(tab.path, fresh.content);
      }
    }
  }
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
  host: IdeBufferHost, perf: IdeBufferPerfController, primaryHandle: () => IdeEditorHandle | null,
  rawPath: string, nextContent: string, range?: { from: number; to: number },
): Promise<boolean> {
  const fullPath = host.resolvePath(rawPath);
  const wasActive = host.activePath === fullPath && host.tabs.some((t) => t.path === fullPath && !t.loading);
  await host.openFile(fullPath, { preview: false });
  const tab = host.tabs.find((t) => t.path === fullPath);
  if (!tab || tab.readOnly || !host.canEditCollab) return false;
  if (!wasActive) await tick();
  if (host.activePath === fullPath && primaryHandle()?.applyAgentEdit?.(nextContent, range)) { perf.flush(host, fullPath); return true; }
  const cur = perf.getBuffer(host, fullPath);
  const from = range ? Math.max(0, Math.min(cur.length, range.from)) : 0;
  const to = range ? Math.max(from, Math.min(cur.length, range.to)) : cur.length;
  host.updateBuffer(fullPath, range ? `${cur.slice(0, from)}${nextContent}${cur.slice(to)}` : nextContent);
  perf.flush(host, fullPath);
  return true;
}
