// Reactive Svelte 5 store for the Otto Full IDE module.
// Re-exports all canonical IDE types so components import everything from `./ideStore.svelte`.

export * from './store/ideTypes';

import { api } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import { type Poller } from '../../lib/poll';
import { lsGet, lsSet } from '../../lib/storage';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import {
  applyFallbackPrefixSuffix, computeFallbackSelection, IdeBufferPerfController,
  performAgentEdit, performCloseTab, performSaveFile, reloadTabsFromDisk,
} from './store/ideBufferPerf';
import {
  followCollabPeer, installIdePlugin, joinCollabSession, loadIdePlugins, startCollabSession,
} from './store/ideCollabController';
import {
  fetchEnvironments, fetchToolchains, launchDebugSession, launchGutterAction, launchIdeRun,
  persistEnvironment, refreshWatchValues, sendDebugAction, startJobsPoller, upsertRunConfig,
} from './store/ideDebugAndEnvController';
import {
  autoOpenInitialEntry, collabGuestPost, detectIdeLanguage, ideBasename,
  type IdeBottomTab, type IdeBreakpoint, type IdeCodeAction, type IdeCollabRole, type IdeCollabRoom,
  type IdeDebugSessionSnapshot, type IdeDiagnostic, type IdeEditorHandle, type IdeEditorSelectionRange,
  type IdeEnvVar, type IdeFileReadResp, type IdeFsEntry, type IdeFsTreeResp, type IdeGutterItem,
  type IdeInstallJob, type IdeKeymapPreset, type IdeOpenTab, type IdePeerCursor, type IdePluginCatalogEntry,
  type IdePluginInfo, type IdeProjectEnvironment, type IdePythonVenv, type IdeReferenceLocation,
  type IdeRenameEdit, type IdeReplaceResp, type IdeReplEntry, type IdeRunConfig, type IdeRunState,
  type IdeSearchItem, type IdeSearchMode, type IdeSymbolNode, type IdeTargetSpec,
  type IdeToolchainEcosystem, type IdeVariable, type IdeWatch,
} from './store/ideTypes';

const KEYMAP_STORAGE_KEY = 'otto.ide.keymap';
const DOCK_OPEN_KEY = 'otto.ide.dockOpen';
const DOCK_TAB_KEY = 'otto.ide.dockTab';

class IdeStore {
  target = $state<IdeTargetSpec>({ kind: 'local', workspace_id: null, root: '', connection_id: null, connection_name: null });
  targetOs = $state('');
  targetArch = $state('');
  treeRootEntries = $state<IdeFsEntry[]>([]);
  childrenByDir = $state<Record<string, IdeFsEntry[]>>({});
  expandedDirs = $state<Record<string, boolean>>({});
  treeLoading = $state(false);
  treeError = $state<string | null>(null);
  showHidden = $state(false);
  treeFilter = $state('');

  tabs = $state<IdeOpenTab[]>([]);
  hasClosedAllTabs = $state(false);
  private _activePath = $state<string | null>(null);
  splitPath = $state<string | null>(null);
  splitDirection = $state<'right' | 'down' | null>(null);
  recentPaths = $state<string[]>([]);
  cursorInfo = $state<{ line: number; col: number; cursors: number; selections: { anchor: number; head: number }[] }>({ line: 1, col: 1, cursors: 1, selections: [] });
  jumpRequest = $state<{ path: string; line: number; col: number; seq: number } | null>(null);

  gutterByPath = $state<Record<string, IdeGutterItem[]>>({});
  symbolsByPath = $state<Record<string, IdeSymbolNode[]>>({});
  symbolsLoading = $state(false);
  problems = $state<IdeDiagnostic[]>([]);
  usages = $state<IdeReferenceLocation[]>([]);
  usagesSymbol = $state('');
  usagesLoading = $state(false);
  renameState = $state<{ open: boolean; path: string; line: number; column: number; symbol: string; newName: string; edits: IdeRenameEdit[]; loading: boolean; error: string | null }>({
    open: false, path: '', line: 1, column: 1, symbol: '', newName: '', edits: [], loading: false, error: null,
  });

  searchResults = $state<IdeSearchItem[]>([]);
  searchTruncated = $state(false);
  searchLoading = $state(false);
  searchError = $state<string | null>(null);
  replacePreview = $state<IdeReplaceResp | null>(null);
  replaceLoading = $state(false);

  toolchains = $state<IdeToolchainEcosystem[]>([]);
  toolchainsLoading = $state(false);
  toolchainsError = $state<string | null>(null);
  installJobs = $state<IdeInstallJob[]>([]);
  projectEnv = $state<IdeProjectEnvironment | null>(null);
  pythonVenvs = $state<IdePythonVenv[]>([]);
  dotenvFiles = $state<string[]>([]);
  runConfigs = $state<IdeRunConfig[]>([]);
  selectedConfigId = $state<string | null>(null);
  envLoading = $state(false);
  envError = $state<string | null>(null);
  activeRun = $state<IdeRunState | null>(null);
  runsHistory = $state<IdeRunState[]>([]);

  breakpointsByPath = $state<Record<string, IdeBreakpoint[]>>({});
  breakOnExceptions = $state(false);
  activeDebug = $state<IdeDebugSessionSnapshot | null>(null);
  expandedVariables = $state<Record<number, IdeVariable[]>>({});
  watches = $state<IdeWatch[]>([]);
  replEntries = $state<IdeReplEntry[]>([]);
  debugLoading = $state(false);

  collabRoom = $state<IdeCollabRoom | null>(null);
  collabRooms = $state<IdeCollabRoom[]>([]);
  collabInviteToken = $state<string | null>(null);
  collabShareHash = $state<string | null>(null);
  collabIsGuest = $state(false);
  collabPeerId = $state<string | null>(null);
  collabPeerSecret = $state<string | null>(null);
  collabGuestToken = $state<string | null>(null);
  peerCursors = $state<IdePeerCursor[]>([]);
  followingPeerId = $state<string | null>(null);
  sharedBuffers = $state<Record<string, { content: string; version: number }>>({});
  collabLoading = $state(false);
  collabError = $state<string | null>(null);

  plugins = $state<IdePluginInfo[]>([]);
  pluginCatalog = $state<IdePluginCatalogEntry[]>([]);
  pluginsLoading = $state(false);
  pluginsError = $state<string | null>(null);

  keymapPreset = $state<IdeKeymapPreset>(lsGet(KEYMAP_STORAGE_KEY) === 'standard' ? 'standard' : 'jetbrains');
  toolchainsOpen = $state(false);
  environmentsOpen = $state(false);
  sshOpen = $state(false);
  shareOpen = $state(false);
  pluginsOpen = $state(false);
  searchEverywhereOpen = $state(false);
  searchInitialTab = $state<'all' | 'files' | 'symbols' | 'text' | 'actions'>('all');
  prefixSuffixOpen = $state(false);
  bottomDockOpen = $state(lsGet(DOCK_OPEN_KEY) === '1');
  activeBottomTab = $state<IdeBottomTab>(
    (['debug', 'run', 'problems', 'usages', 'terminal'].includes(lsGet(DOCK_TAB_KEY) ?? '') ? lsGet(DOCK_TAB_KEY) : 'run') as IdeBottomTab,
  );

  readonly hasTarget = $derived(Boolean(this.target.root));
  readonly activeTab = $derived(this.tabs.find((t) => t.path === this._activePath) ?? null);
  readonly splitTab = $derived(this.splitPath ? (this.tabs.find((t) => t.path === this.splitPath) ?? null) : null);
  readonly activeSymbols = $derived(this._activePath ? (this.symbolsByPath[this._activePath] ?? []) : []);
  readonly allBreakpoints = $derived(Object.values(this.breakpointsByPath).flat());
  readonly canEditCollab = $derived(
    !this.collabIsGuest ||
      Boolean(this.collabRoom && (this.collabRoom.peers.find((p) => p.peer_id === this.collabPeerId)?.role ?? this.collabRoom.default_role) === 'editor'),
  );

  private bufPerf = new IdeBufferPerfController();
  private editorHandles: { primary: IdeEditorHandle | null; split: IdeEditorHandle | null } = { primary: null, split: null };
  private jumpSeq = 0;
  private runPoller: Poller | null = null;
  private jobsPoller: Poller | null = null;
  private debugSseCtl: AbortController | null = null;
  private collabSseCtl: AbortController | null = null;
  private cursorSendTimer: ReturnType<typeof setTimeout> | null = null;
  private collabEditTimer: ReturnType<typeof setTimeout> | null = null;

  get activePath(): string | null { return this._activePath; }
  set activePath(next: string | null) {
    if (this._activePath === next) return;
    this.bufPerf.flushAll(this);
    this._activePath = next;
    this.bufPerf.cancelHiddenLoads(this);
    if (next && !this.collabIsGuest) {
      const tab = this.tabs.find((t) => t.path === next);
      if (tab && !tab.loading && !tab.error) {
        if (this.gutterByPath[next] === undefined) void this.loadGutter(next, this.getBuffer(next));
        if (this.symbolsByPath[next] === undefined) void this.loadSymbols(next, this.getBuffer(next));
      }
    }
  }

  get targetPayload(): IdeTargetSpec {
    return { kind: this.target.kind, workspace_id: this.target.workspace_id ?? null, root: this.target.root, connection_id: this.target.kind === 'ssh' ? (this.target.connection_id ?? null) : null };
  }

  targetQuery(extra: Record<string, string | boolean | null | undefined> = {}): string {
    const q = new URLSearchParams({ kind: this.target.kind, root: this.target.root });
    if (this.target.kind === 'ssh' && this.target.connection_id) q.set('connection_id', this.target.connection_id);
    if (this.target.workspace_id) q.set('workspace_id', this.target.workspace_id);
    for (const [k, v] of Object.entries(extra)) if (v !== undefined && v !== null && v !== '') q.set(k, String(v));
    return q.toString();
  }

  resolvePath(p: string): string {
    if (!p) return this.target.root;
    if (p.startsWith('/') || /^[A-Za-z]:[\\/]/.test(p)) return p;
    const base = this.target.root.replace(/\/+$/, '');
    return base ? `${base}/${p.replace(/^\/+/, '')}` : p;
  }

  relativePath(p: string): string {
    const root = this.target.root.replace(/\/+$/, '');
    return root && p.startsWith(`${root}/`) ? p.slice(root.length + 1) : p;
  }

  attachDefaultWorkspace(): void {
    if (this.target.root || this.collabIsGuest) return;
    const cur = ws.current ?? ws.workspaces.find((w) => w.id !== SCRATCH_WORKSPACE_ID && w.root_path) ?? ws.workspaces[0];
    if (cur?.root_path) void this.setLocalTarget(cur.root_path, cur.id);
  }

  async setLocalTarget(root: string, workspaceId: string | null = null): Promise<void> {
    const clean = root.trim();
    if (!clean || (this.target.kind === 'local' && this.target.root === clean)) return;
    this.resetTargetState({ kind: 'local', workspace_id: workspaceId, root: clean, connection_id: null, connection_name: null });
    await Promise.all([this.loadTree(), this.loadEnvironments(), this.loadToolchains()]);
  }

  async setSshTarget(connectionId: string, root: string, connectionName: string | null = null): Promise<void> {
    this.resetTargetState({ kind: 'ssh', workspace_id: null, root: root.trim() || '~', connection_id: connectionId, connection_name: connectionName });
    await Promise.all([this.loadTree(), this.loadEnvironments(), this.loadToolchains()]);
  }

  clearTarget(): void {
    this.disposeStreams();
    this.resetTargetState({ kind: 'local', workspace_id: null, root: '', connection_id: null, connection_name: null });
  }

  private resetTargetState(next: IdeTargetSpec): void {
    this.bufPerf.resetAll(this);
    this.target = next;
    this.treeRootEntries = []; this.childrenByDir = {}; this.expandedDirs = {}; this.treeError = null;
    this.tabs = []; this.hasClosedAllTabs = false; this._activePath = null; this.splitPath = null; this.splitDirection = null;
    this.gutterByPath = {}; this.symbolsByPath = {}; this.problems = []; this.usages = [];
  }

  setKeymapPreset(preset: IdeKeymapPreset): void { this.keymapPreset = preset; lsSet(KEYMAP_STORAGE_KEY, preset); }
  openBottomTab(tab: IdeBottomTab): void { this.activeBottomTab = tab; this.bottomDockOpen = true; lsSet(DOCK_OPEN_KEY, '1'); lsSet(DOCK_TAB_KEY, tab); }
  toggleBottomDock(open?: boolean): void { this.bottomDockOpen = open ?? !this.bottomDockOpen; lsSet(DOCK_OPEN_KEY, this.bottomDockOpen ? '1' : '0'); }
  registerEditorHandle(slot: 'primary' | 'split', handle: IdeEditorHandle | null): void { this.editorHandles[slot] = handle; }
  clearProblems(): void { this.problems = []; }
  clearRepl(): void { this.replEntries = []; }

  async loadTree(dirPath?: string): Promise<void> {
    if (!this.target.root || this.collabIsGuest) return;
    const isRoot = !dirPath || dirPath === this.target.root;
    if (isRoot) { this.treeLoading = true; this.treeError = null; }
    try {
      const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${this.targetQuery({ path: isRoot ? '' : dirPath, show_hidden: this.showHidden })}`);
      if (resp.target_os) this.targetOs = resp.target_os;
      if (resp.target_arch) this.targetArch = resp.target_arch;
      if (isRoot) {
        if (resp.root && this.target.root === '~') this.target = { ...this.target, root: resp.root };
        this.treeRootEntries = resp.entries ?? [];
        void autoOpenInitialEntry(this);
      } else if (dirPath) this.childrenByDir = { ...this.childrenByDir, [dirPath]: resp.entries ?? [] };
    } catch (e) {
      if (isRoot) this.treeError = loadErrorText(e); else toastError('Couldn’t open folder', e);
    } finally {
      if (isRoot) this.treeLoading = false;
    }
  }

  async toggleDir(dirPath: string): Promise<void> {
    const next = !this.expandedDirs[dirPath];
    this.expandedDirs = { ...this.expandedDirs, [dirPath]: next };
    if (next && !this.childrenByDir[dirPath]) await this.loadTree(dirPath);
  }

  async mutateFs(op: 'mkdir' | 'create_file' | 'rename' | 'delete' | 'duplicate', path: string, destPath?: string): Promise<boolean> {
    try {
      const resp = await api.post<{ ok: boolean; path: string }>('/ide/fs/mutate', { target: this.targetPayload, op, path, dest_path: destPath });
      const parentOf = (p: string) => (p.lastIndexOf('/') > 0 ? p.slice(0, p.lastIndexOf('/')) : this.target.root);
      await this.loadTree();
      for (const dir of Object.keys(this.expandedDirs)) {
        if (this.expandedDirs[dir] && (dir === parentOf(path) || (destPath && dir === parentOf(destPath)))) await this.loadTree(dir);
      }
      if (op === 'delete') {
        const deleted = this.tabs.filter((t) => t.path === path || t.path.startsWith(`${path}/`)).map((t) => t.path);
        this.tabs = this.tabs.filter((t) => t.path !== path && !t.path.startsWith(`${path}/`));
        this.bufPerf.evictPaths(this, deleted, true);
        if (this.activePath && deleted.includes(this.activePath)) this.activePath = this.tabs[0]?.path ?? null;
        if (this.splitPath && deleted.includes(this.splitPath)) this.closeSplit();
      } else if (op === 'rename' && destPath) {
        this.bufPerf.flush(this, path);
        this.bufPerf.evictPaths(this, [path], false);
        this.tabs = this.tabs.map((t) => (t.path === path ? { ...t, path: destPath, name: ideBasename(destPath) } : t));
        if (this._activePath === path) this._activePath = destPath;
        if (this.splitPath === path) this.splitPath = destPath;
      } else if ((op === 'create_file' || op === 'duplicate') && resp.path) {
        await this.openFile(resp.path, { preview: false });
      }
      return true;
    } catch (e) { toastError(`Couldn’t ${op.replace('_', ' ')}`, e); return false; }
  }

  async openFile(rawPath: string, opts: { line?: number; col?: number; preview?: boolean; inSplit?: boolean } = {}): Promise<void> {
    const fullPath = this.resolvePath(rawPath);
    this.recentPaths = [fullPath, ...this.recentPaths.filter((p) => p !== fullPath)].slice(0, 20);
    const existing = this.tabs.find((t) => t.path === fullPath);
    if (existing) {
      if (opts.preview === false && existing.preview) existing.preview = false;
      if (opts.inSplit) this.openInSplit(fullPath, this.splitDirection ?? 'right'); else this.activePath = fullPath;
      if (opts.line) this.jumpTo(fullPath, opts.line, opts.col ?? 1);
      return;
    }
    const buf = this.sharedBuffers[fullPath]?.content ?? '';
    const isGuest = this.collabIsGuest;
    const placeholder: IdeOpenTab = {
      path: fullPath, name: ideBasename(fullPath), content: buf, savedContent: buf, dirty: false, sha256: '',
      language: detectIdeLanguage(fullPath), readOnly: false, truncated: false, loading: !isGuest, error: null, preview: opts.preview ?? false,
    };
    const prevIdx = opts.preview ? this.tabs.findIndex((t) => t.preview && !this.bufPerf.isTabDirty(t)) : -1;
    if (prevIdx >= 0) {
      const evicted = this.tabs[prevIdx].path;
      this.tabs[prevIdx] = placeholder;
      if (evicted !== fullPath) this.bufPerf.evictPaths(this, [evicted], true);
    } else this.tabs = [...this.tabs, placeholder];
    if (opts.inSplit) this.openInSplit(fullPath, this.splitDirection ?? 'right'); else this.activePath = fullPath;
    if (isGuest) { if (opts.line) this.jumpTo(fullPath, opts.line, opts.col ?? 1); return; }
    try {
      const res = await api.get<IdeFileReadResp>(`/ide/fs/file?${this.targetQuery({ path: fullPath })}`);
      const tab = this.tabs.find((t) => t.path === fullPath);
      if (tab) {
        this.bufPerf.markClean(tab, res.content, res.sha256);
        tab.language = res.language || detectIdeLanguage(fullPath);
        tab.readOnly = res.read_only; tab.truncated = res.truncated; tab.loading = false;
      }
      if (opts.line) this.jumpTo(fullPath, opts.line, opts.col ?? 1);
      if (this._activePath === fullPath || this.splitPath === fullPath) {
        void this.loadGutter(fullPath, res.content);
        void this.loadSymbols(fullPath, res.content);
      }
    } catch (e) {
      const tab = this.tabs.find((t) => t.path === fullPath);
      if (tab) { tab.loading = false; tab.error = loadErrorText(e); }
    }
  }

  pinTab(path: string): void { const tab = this.tabs.find((t) => t.path === path); if (tab) tab.preview = false; }
  openInSplit(path: string, dir: 'right' | 'down' = 'right'): void { this.bufPerf.flushAll(this); this.splitPath = path; this.splitDirection = dir; }
  closeSplit(): void { this.bufPerf.flushAll(this); this.splitPath = null; this.splitDirection = null; this.bufPerf.cancelHiddenLoads(this); }
  jumpTo(path: string, line: number, col = 1): void { this.jumpSeq += 1; this.jumpRequest = { path, line, col, seq: this.jumpSeq }; }
  closeTab(path: string): Promise<void> { return performCloseTab(this, this.bufPerf, path); }

  closeOtherTabs(keepPath: string): void {
    this.bufPerf.flushAll(this);
    const closing = this.tabs.filter((t) => t.path !== keepPath && !this.bufPerf.isTabDirty(t)).map((t) => t.path);
    this.tabs = this.tabs.filter((t) => t.path === keepPath || this.bufPerf.isTabDirty(t));
    this.bufPerf.evictPaths(this, closing, true);
    this.activePath = keepPath;
    if (this.splitPath && this.splitPath !== keepPath) this.closeSplit();
  }

  getBuffer(path?: string): string {
    const targetPath = path ? this.resolvePath(path) : this._activePath;
    return targetPath ? this.bufPerf.getBuffer(this, targetPath) : '';
  }

  updateBuffer(path: string, content: string): void {
    const fullPath = this.resolvePath(path);
    if (!this.bufPerf.recordEdit(this, fullPath, content)) return;
    if (this.collabRoom && this.canEditCollab) {
      if (this.collabEditTimer) clearTimeout(this.collabEditTimer);
      this.collabEditTimer = setTimeout(() => void this.sendCollabEdit(fullPath, this.getBuffer(fullPath), false), 120);
    }
  }

  updateCursor(path: string, info: { line: number; col: number; cursors: number; selections: { anchor: number; head: number }[] }): void {
    this.cursorInfo = info;
    if (this.collabRoom && this.collabPeerId && this.collabPeerSecret) {
      if (this.cursorSendTimer) clearTimeout(this.cursorSendTimer);
      this.cursorSendTimer = setTimeout(() => void this.sendCursorUpdate(path, info.selections, info.line), 90);
    }
  }

  saveFile(path?: string, forceOverwrite = false): Promise<boolean> { return performSaveFile(this, this.bufPerf, path, forceOverwrite); }

  async saveAll(): Promise<void> {
    this.bufPerf.flushAll(this);
    const dirty = this.tabs.filter((t) => this.bufPerf.isTabDirty(t) && !t.readOnly);
    for (let i = 0; i < dirty.length; i++) await this.saveFile(dirty[i].path);
  }

  getSelectionRange(): IdeEditorSelectionRange | null {
    if (this.editorHandles.primary?.getSelectionRange) return this.editorHandles.primary.getSelectionRange();
    const tab = this.activeTab;
    return tab ? computeFallbackSelection(this.getBuffer(tab.path), this.cursorInfo) : null;
  }

  applyAgentEdit(path: string, nextContent: string, range?: { from: number; to: number }): Promise<boolean> {
    return performAgentEdit(this, this.bufPerf, () => this.editorHandles.primary, path, nextContent, range);
  }

  applyPrefixSuffix(prefix: string, suffix: string): void {
    if (!prefix && !suffix) return;
    if (this.editorHandles.primary) { this.editorHandles.primary.applyPrefixSuffix(prefix, suffix); return; }
    const tab = this.activeTab;
    if (!tab || tab.readOnly) return;
    this.updateBuffer(tab.path, applyFallbackPrefixSuffix(this.getBuffer(tab.path), this.cursorInfo, prefix, suffix));
    this.bufPerf.flush(this, tab.path);
  }

  splitSelectionToLines(): void { this.editorHandles.primary?.splitSelectionToLines(); }
  loadGutter(path: string, content?: string): Promise<void> { return this.bufPerf.scheduleGutter(this, path, content); }
  loadSymbols(path: string, content?: string): Promise<void> { return this.bufPerf.scheduleSymbols(this, path, content); }
  setDiagnostics(path: string, items: IdeDiagnostic[]): void { this.problems = [...this.problems.filter((d) => d.path !== path), ...items]; }

  async findReferences(path: string, line: number, column: number, symbol?: string): Promise<void> {
    this.usagesLoading = true; this.openBottomTab('usages');
    try {
      const resp = await api.post<{ symbol: string; locations: IdeReferenceLocation[] }>('/ide/lsp/references', { target: this.targetPayload, path, line, column, symbol });
      this.usagesSymbol = resp.symbol || symbol || ''; this.usages = resp.locations ?? [];
    } catch (e) { toastError('Couldn’t find usages', e); }
    finally { this.usagesLoading = false; }
  }

  async openRenameModal(path: string, line: number, column: number, symbol = ''): Promise<void> {
    this.renameState = { open: true, path, line, column, symbol, newName: symbol, edits: [], loading: Boolean(symbol), error: null };
    if (symbol) await this.previewRename(symbol);
  }

  async previewRename(newName?: string): Promise<void> {
    const next = (newName ?? this.renameState.newName).trim();
    this.renameState.newName = next;
    if (!next) return;
    this.renameState.loading = true; this.renameState.error = null;
    try {
      const resp = await api.post<{ symbol: string; new_name: string; edits: IdeRenameEdit[] }>('/ide/lsp/rename', {
        target: this.targetPayload, path: this.renameState.path, line: this.renameState.line, column: this.renameState.column, new_name: next, apply: false,
      });
      if (resp.symbol && !this.renameState.symbol) this.renameState.symbol = resp.symbol;
      this.renameState.edits = resp.edits ?? [];
    } catch (e) { this.renameState.error = loadErrorText(e); }
    finally { this.renameState.loading = false; }
  }

  async applyRename(): Promise<boolean> {
    const next = this.renameState.newName.trim();
    if (!next) return false;
    this.renameState.loading = true; this.renameState.error = null;
    try {
      const resp = await api.post<{ applied: boolean; edits: IdeRenameEdit[] }>('/ide/lsp/rename', {
        target: this.targetPayload, path: this.renameState.path, line: this.renameState.line, column: this.renameState.column, new_name: next, apply: true,
      });
      const touched = new Set((resp.edits ?? []).map((e) => e.path));
      await reloadTabsFromDisk(this, this.bufPerf, touched);
      this.renameState.open = false;
      toasts.success(`Renamed to “${next}”`, `${plural(touched.size, 'file')} updated`);
      return true;
    } catch (e) { this.renameState.error = loadErrorText(e); return false; }
    finally { this.renameState.loading = false; }
  }

  async formatActiveFile(): Promise<void> {
    const tab = this.activeTab;
    if (!tab || tab.readOnly) return;
    const content = this.bufPerf.flush(this, tab.path);
    try {
      const resp = await api.post<{ formatted: string; changed: boolean; formatter: string }>('/ide/lsp/format', { target: this.targetPayload, path: tab.path, content, language: tab.language });
      if (resp.changed && typeof resp.formatted === 'string') {
        await this.applyAgentEdit(tab.path, resp.formatted);
        toasts.info(`Formatted with ${resp.formatter || 'formatter'}`);
      }
    } catch (e) { toastError('Couldn’t format file', e); }
  }

  async fetchCodeActions(path: string, line: number, column: number): Promise<IdeCodeAction[]> {
    const content = this.bufPerf.flush(this, path);
    try { return (await api.post<{ actions: IdeCodeAction[] }>('/ide/lsp/code-actions', { target: this.targetPayload, path, line, column, content })).actions ?? []; }
    catch (e) { toastError('Couldn’t load code actions', e); return []; }
  }

  applyCodeAction(path: string, action: IdeCodeAction): void {
    if (typeof action.replacement_content === 'string') { void this.applyAgentEdit(path, action.replacement_content); toasts.info(action.title); }
  }

  async searchWorkspace(query: string, mode: IdeSearchMode = 'all', opts: { caseSensitive?: boolean; regex?: boolean; fileGlob?: string; limit?: number } = {}): Promise<void> {
    if (!this.target.root || !query.trim()) { this.searchResults = []; this.searchTruncated = false; return; }
    this.searchLoading = true; this.searchError = null;
    try {
      const resp = await api.long.post<{ items: IdeSearchItem[]; truncated: boolean }>('/ide/search', {
        target: this.targetPayload, query: query.trim(), mode, case_sensitive: opts.caseSensitive ?? false, regex: opts.regex ?? false, file_glob: opts.fileGlob || undefined, limit: opts.limit ?? 80,
      });
      this.searchResults = resp.items ?? []; this.searchTruncated = Boolean(resp.truncated);
    } catch (e) { this.searchError = loadErrorText(e); }
    finally { this.searchLoading = false; }
  }

  async replaceInWorkspace(query: string, replacement: string, dryRun: boolean, opts: { caseSensitive?: boolean; regex?: boolean; paths?: string[] } = {}): Promise<IdeReplaceResp | null> {
    if (!this.target.root || !query) return null;
    this.replaceLoading = true;
    try {
      const resp = await api.long.post<IdeReplaceResp>('/ide/replace', {
        target: this.targetPayload, query, replacement, case_sensitive: opts.caseSensitive ?? false, regex: opts.regex ?? false, paths: opts.paths, dry_run: dryRun,
      });
      this.replacePreview = resp;
      if (!dryRun) {
        await reloadTabsFromDisk(this, this.bufPerf, new Set(resp.previews.map((p) => p.path)));
        toasts.success(`Replaced ${plural(resp.replacements, 'occurrence')}`, `${plural(resp.files_changed, 'file')} updated`);
      }
      return resp;
    } catch (e) { toastError('Couldn’t replace in project', e); return null; }
    finally { this.replaceLoading = false; }
  }

  async loadToolchains(): Promise<void> { if (!this.collabIsGuest) await fetchToolchains(this, () => this.ensureJobsPoller()); }
  private ensureJobsPoller(): void { this.jobsPoller = startJobsPoller(this, this.jobsPoller, () => void this.loadToolchains()); }
  async loadEnvironments(): Promise<void> { if (!this.collabIsGuest) await fetchEnvironments(this); }
  async saveEnvironment(patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[] }): Promise<boolean> { return persistEnvironment(this, patch); }
  async saveRunConfig(cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> { return upsertRunConfig(this, cfg); }
  async startRun(opts: { configId?: string; adhoc?: Partial<IdeRunConfig> } = {}): Promise<void> { this.runPoller = await launchIdeRun(this, opts, this.runPoller); }
  runFromGutter(item: IdeGutterItem, mode: 'run' | 'debug'): void { launchGutterAction(this, item, mode, (adhoc) => void this.startRun({ adhoc }), (opts) => void this.startDebug(opts)); }

  async installToolchain(ecosystem: string, component: 'sdk' | 'lsp' | 'dap' | 'all', version?: string): Promise<IdeInstallJob | null> {
    try {
      const resp = await api.post<{ job: IdeInstallJob }>('/ide/toolchains/install', { target: this.targetPayload, ecosystem, component, version });
      this.installJobs = [resp.job, ...this.installJobs.filter((j) => j.id !== resp.job.id)];
      this.ensureJobsPoller();
      return resp.job;
    } catch (e) { toastError(`Couldn’t install ${ecosystem} ${component}`, e); return null; }
  }

  async createPythonVenv(venvDir = '.venv', pythonVersion?: string): Promise<{ path: string; interpreter_path: string; version: string } | null> {
    try {
      const resp = await api.long.post<{ path: string; interpreter_path: string; version: string }>('/ide/environments/python-venv', {
        target: this.targetPayload, venv_dir: venvDir, python_version: pythonVersion || undefined,
      });
      await this.loadEnvironments();
      await this.saveEnvironment({ python_interpreter: resp.interpreter_path });
      return resp;
    } catch (e) { toastError('Couldn’t create Python virtual environment', e); return null; }
  }

  async deleteRunConfig(id: string): Promise<void> {
    try {
      await api.del(`/ide/run-configs/${encodeURIComponent(id)}?${this.targetQuery()}`);
      this.runConfigs = this.runConfigs.filter((c) => c.id !== id);
      if (this.selectedConfigId === id) this.selectedConfigId = this.runConfigs[0]?.id ?? null;
    } catch (e) { toastError('Couldn’t delete run configuration', e); }
  }

  async stopRun(runId?: string): Promise<void> {
    const id = runId ?? this.activeRun?.id;
    if (!id) return;
    try {
      const resp = await api.post<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(id)}/stop`, {});
      this.activeRun = resp.run; this.runPoller?.stop(); this.runPoller = null;
    } catch (e) { toastError('Couldn’t stop run', e); }
  }

  private setPathBreakpoints(path: string, next: IdeBreakpoint[]): void {
    this.breakpointsByPath = { ...this.breakpointsByPath, [path]: next };
    void this.syncBreakpointsToSession();
  }
  toggleBreakpoint(path: string, line: number): void {
    const list = this.breakpointsByPath[path] ?? [];
    this.setPathBreakpoints(path, list.some((b) => b.line === line) ? list.filter((b) => b.line !== line) : [...list, { path, line, enabled: true }].sort((a, b) => a.line - b.line));
  }
  updateBreakpoint(path: string, line: number, patch: Partial<IdeBreakpoint>): void {
    const list = this.breakpointsByPath[path] ?? [];
    this.setPathBreakpoints(path, list.some((b) => b.line === line) ? list.map((b) => (b.line === line ? { ...b, ...patch } : b)) : [...list, { path, line, enabled: true, ...patch }].sort((a, b) => a.line - b.line));
  }
  removeBreakpoint(path: string, line: number): void { this.setPathBreakpoints(path, (this.breakpointsByPath[path] ?? []).filter((b) => b.line !== line)); }

  async syncBreakpointsToSession(): Promise<void> {
    if (!this.activeDebug || this.activeDebug.state === 'terminated') return;
    try {
      const resp = await api.put<{ session: IdeDebugSessionSnapshot }>(`/ide/debug/sessions/${encodeURIComponent(this.activeDebug.id)}/breakpoints`, { breakpoints: this.allBreakpoints, break_on_exceptions: this.breakOnExceptions });
      this.applyDebugSnapshot(resp.session);
    } catch { /* exited */ }
  }

  async startDebug(opts: { configId?: string; language?: string; launch?: { program: string; args?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string> } } = {}): Promise<void> {
    this.debugSseCtl = await launchDebugSession(this, opts, this.debugSseCtl, (s) => this.applyDebugSnapshot(s));
  }

  applyDebugSnapshot(snap: IdeDebugSessionSnapshot): void {
    const prevPaused = `${this.activeDebug?.paused_file ?? ''}:${this.activeDebug?.paused_line ?? 0}`;
    const stripPrivate = (p: string) => p.replace(/^\/private(?=\/(?:var|tmp)(?:\/|$))/, '');
    const root = this.target.root.replace(/\/+$/, '');
    let paused = snap.paused_file ? this.resolvePath(snap.paused_file) : null;
    let canOpenPaused = Boolean(paused);
    if (paused && this.target.kind === 'local' && root) {
      const cRoot = stripPrivate(root);
      const cPaused = stripPrivate(paused);
      if (cPaused === cRoot || cPaused.startsWith(`${cRoot}/`)) {
        paused = `${root}${cPaused.slice(cRoot.length)}`;
        snap = { ...snap, paused_file: paused };
      } else canOpenPaused = false;
    }
    this.activeDebug = snap;
    if (snap.state === 'paused' && paused && snap.paused_line) {
      if (canOpenPaused && `${paused}:${snap.paused_line}` !== prevPaused) void this.openFile(paused, { line: snap.paused_line, preview: true });
      void this.refreshWatches();
    }
    if (snap.state === 'terminated') { this.debugSseCtl?.abort(); this.debugSseCtl = null; }
  }

  async debugCommand(
    action: 'continue' | 'pause' | 'step_over' | 'step_into' | 'step_out' | 'run_to_cursor' | 'select_frame' | 'restart' | 'stop',
    extra: { thread_id?: number; frame_id?: number; path?: string; line?: number } = {},
  ): Promise<void> {
    await sendDebugAction(this, action, extra, (s) => this.applyDebugSnapshot(s));
  }

  stepOver = () => this.debugCommand('step_over');
  stepInto = () => this.debugCommand('step_into');
  stepOut = () => this.debugCommand('step_out');
  resumeDebug = () => this.debugCommand('continue');
  pauseDebug = () => this.debugCommand('pause');
  restartDebug = () => this.debugCommand('restart');
  stopDebug = () => this.debugCommand('stop');
  runToCursor = (path?: string, line?: number) => this.debugCommand('run_to_cursor', { path: path ?? this._activePath ?? undefined, line: line ?? this.cursorInfo.line });
  selectFrame = (frameId: number, threadId?: number) => this.debugCommand('select_frame', { frame_id: frameId, thread_id: threadId });

  async expandVariables(variablesRef: number): Promise<IdeVariable[]> {
    if (!this.activeDebug || variablesRef <= 0) return [];
    try {
      const resp = await api.post<{ variables: IdeVariable[] }>(`/ide/debug/sessions/${encodeURIComponent(this.activeDebug.id)}/variables`, { variables_reference: variablesRef });
      const list = resp.variables ?? [];
      this.expandedVariables = { ...this.expandedVariables, [variablesRef]: list };
      return list;
    } catch (e) { toastError('Couldn’t inspect variables', e); return []; }
  }

  async evaluateDebug(expression: string, context: 'watch' | 'repl' | 'hover' = 'repl', frameId?: number): Promise<string> {
    const expr = expression.trim();
    if (!expr || !this.activeDebug) return '';
    try {
      const resp = await api.post<{ result: string; var_type?: string; variables_reference: number }>(
        `/ide/debug/sessions/${encodeURIComponent(this.activeDebug.id)}/evaluate`,
        { expression: expr, frame_id: frameId ?? this.activeDebug.active_frame_id ?? undefined, context },
      );
      if (context === 'repl') this.replEntries = [...this.replEntries, { id: `${Date.now()}-${this.replEntries.length}`, expression: expr, result: resp.result, var_type: resp.var_type }];
      return resp.result;
    } catch (e) {
      const msg = loadErrorText(e);
      if (context === 'repl') this.replEntries = [...this.replEntries, { id: `${Date.now()}-${this.replEntries.length}`, expression: expr, result: msg, isError: true }];
      return msg;
    }
  }

  addWatch(expression: string): void {
    const expr = expression.trim();
    if (!expr) return;
    this.watches = [...this.watches, { id: `w-${Date.now()}-${this.watches.length}`, expression: expr, value: '…' }];
    void this.refreshWatches();
  }
  removeWatch(id: string): void { this.watches = this.watches.filter((w) => w.id !== id); }
  async refreshWatches(): Promise<void> { await refreshWatchValues(this); }

  async loadCollabRooms(): Promise<void> {
    if (this.collabIsGuest) return;
    try { this.collabRooms = (await api.get<{ rooms: IdeCollabRoom[] }>('/ide/collab/rooms')).rooms ?? []; } catch { /* ignore */ }
  }
  async startCollabRoom(defaultRole: IdeCollabRole = 'editor', title?: string): Promise<void> {
    this.bufPerf.flushAll(this);
    this.collabSseCtl = await startCollabSession(this, defaultRole, title, this.collabSseCtl);
  }
  async setCollabPeerRole(peerId: string, role: IdeCollabRole): Promise<void> {
    if (!this.collabRoom) return;
    try { this.collabRoom = (await api.post<{ room: IdeCollabRoom }>(`/ide/collab/rooms/${encodeURIComponent(this.collabRoom.id)}/role`, { peer_id: peerId, role })).room; }
    catch (e) { toastError('Couldn’t update teammate role', e); }
  }
  async stopCollabRoom(): Promise<void> {
    if (!this.collabRoom) return;
    const id = this.collabRoom.id;
    this.collabSseCtl?.abort(); this.collabSseCtl = null;
    try { if (!this.collabIsGuest) await api.del(`/ide/collab/rooms/${encodeURIComponent(id)}`); }
    catch (e) { toastError('Couldn’t close shared session', e); }
    finally { this.leaveCollabRoom(); }
  }
  leaveCollabRoom(): void {
    this.collabSseCtl?.abort(); this.collabSseCtl = null;
    this.collabRoom = null; this.collabInviteToken = null; this.collabShareHash = null;
    this.collabIsGuest = false; this.collabPeerId = null; this.collabPeerSecret = null;
    this.collabGuestToken = null; this.peerCursors = []; this.followingPeerId = null;
  }
  async joinCollabRoom(roomId: string, token: string, displayName = 'Teammate'): Promise<void> {
    this.collabSseCtl = await joinCollabSession(this, roomId, token, displayName, this.collabSseCtl);
  }
  followPeer(peerId: string | null): void { followCollabPeer(this, peerId); }

  async sendCursorUpdate(path: string, selections: { anchor: number; head: number }[], activeLine: number): Promise<void> {
    if (!this.collabRoom || !this.collabPeerId || !this.collabPeerSecret) return;
    try {
      await collabGuestPost(`/ide/collab/guest/${encodeURIComponent(this.collabRoom.id)}/cursor`, this.collabGuestToken, {
        peer_id: this.collabPeerId, peer_secret: this.collabPeerSecret, path, selections, active_line: activeLine,
      });
    } catch { /* transient */ }
  }

  async sendCollabEdit(path: string, content: string, save = false): Promise<void> {
    this.bufPerf.flush(this, path);
    if (!this.collabRoom || !this.collabPeerId || !this.collabPeerSecret || !this.canEditCollab) return;
    try {
      const res = await collabGuestPost<{ version: number }>(`/ide/collab/guest/${encodeURIComponent(this.collabRoom.id)}/edit`, this.collabGuestToken, {
        peer_id: this.collabPeerId, peer_secret: this.collabPeerSecret, path, content, base_version: this.sharedBuffers[path]?.version, save,
      });
      this.sharedBuffers = { ...this.sharedBuffers, [path]: { content, version: res.version } };
    } catch (e) { if (save) toastError('Couldn’t sync edit to room', e); }
  }

  async loadPlugins(): Promise<void> { await loadIdePlugins(this); }
  async installPlugin(source: string): Promise<boolean> { return installIdePlugin(this, source); }
  async togglePlugin(slug: string, enabled: boolean): Promise<void> {
    try {
      const resp = await api.post<{ plugin: IdePluginInfo }>(`/ide/plugins/${encodeURIComponent(slug)}/toggle`, { enabled });
      this.plugins = this.plugins.map((p) => (p.slug === slug ? resp.plugin : p));
    } catch (e) { toastError('Couldn’t toggle IDE plugin', e); }
  }
  async removePlugin(slug: string): Promise<void> {
    try {
      await api.del(`/ide/plugins/${encodeURIComponent(slug)}`);
      this.plugins = this.plugins.filter((p) => p.slug !== slug);
      await this.loadPlugins();
      toasts.info('Removed IDE plugin');
    } catch (e) { toastError('Couldn’t remove IDE plugin', e); }
  }

  disposeStreams(): void {
    this.bufPerf.resetAll(this);
    if (this.cursorSendTimer) clearTimeout(this.cursorSendTimer);
    if (this.collabEditTimer) clearTimeout(this.collabEditTimer);
    this.cursorSendTimer = null; this.collabEditTimer = null;
    this.runPoller?.stop(); this.runPoller = null;
    this.jobsPoller?.stop(); this.jobsPoller = null;
    this.debugSseCtl?.abort(); this.debugSseCtl = null;
    this.collabSseCtl?.abort(); this.collabSseCtl = null;
  }
}

export const ide = new IdeStore();
