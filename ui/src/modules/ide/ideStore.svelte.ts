// Reactive Svelte 5 store for the Otto Full IDE module.
// Re-exports all canonical IDE types so components import everything from `./ideStore.svelte`.

export * from './store/ideTypes';

import { api, ApiError } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { loadErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import { type Poller } from '../../lib/poll';
import { lsGet, lsSet } from '../../lib/storage';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
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
  type IdeDebugSessionSnapshot, type IdeDiagnostic, type IdeEditorHandle, type IdeEnvVar,
  type IdeFileReadResp, type IdeFsEntry, type IdeFsTreeResp, type IdeGutterItem, type IdeInstallJob,
  type IdeKeymapPreset, type IdeOpenTab, type IdePeerCursor, type IdePluginCatalogEntry,
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
  activePath = $state<string | null>(null);
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
  renameState = $state<{
    open: boolean; path: string; line: number; column: number; symbol: string;
    newName: string; edits: IdeRenameEdit[]; loading: boolean; error: string | null;
  }>({ open: false, path: '', line: 1, column: 1, symbol: '', newName: '', edits: [], loading: false, error: null });

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
  readonly activeTab = $derived(this.tabs.find((t) => t.path === this.activePath) ?? null);
  readonly splitTab = $derived(this.splitPath ? (this.tabs.find((t) => t.path === this.splitPath) ?? null) : null);
  readonly activeSymbols = $derived(this.activePath ? (this.symbolsByPath[this.activePath] ?? []) : []);
  readonly allBreakpoints = $derived(Object.values(this.breakpointsByPath).flat());
  readonly canEditCollab = $derived(
    !this.collabIsGuest ||
      Boolean(this.collabRoom && (this.collabRoom.peers.find((p) => p.peer_id === this.collabPeerId)?.role ?? this.collabRoom.default_role) === 'editor'),
  );

  private editorHandles: { primary: IdeEditorHandle | null; split: IdeEditorHandle | null } = { primary: null, split: null };
  private jumpSeq = 0;
  private runPoller: Poller | null = null;
  private jobsPoller: Poller | null = null;
  private debugSseCtl: AbortController | null = null;
  private collabSseCtl: AbortController | null = null;
  private cursorSendTimer: ReturnType<typeof setTimeout> | null = null;
  private collabEditTimer: ReturnType<typeof setTimeout> | null = null;

  get targetPayload(): IdeTargetSpec {
    return {
      kind: this.target.kind,
      workspace_id: this.target.workspace_id ?? null,
      root: this.target.root,
      connection_id: this.target.kind === 'ssh' ? (this.target.connection_id ?? null) : null,
    };
  }

  targetQuery(extra: Record<string, string | boolean | null | undefined> = {}): string {
    const q = new URLSearchParams();
    q.set('kind', this.target.kind);
    q.set('root', this.target.root);
    if (this.target.kind === 'ssh' && this.target.connection_id) q.set('connection_id', this.target.connection_id);
    if (this.target.workspace_id) q.set('workspace_id', this.target.workspace_id);
    for (const [k, v] of Object.entries(extra)) {
      if (v !== undefined && v !== null && v !== '') q.set(k, String(v));
    }
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
    const clean = root.trim() || '~';
    this.resetTargetState({ kind: 'ssh', workspace_id: null, root: clean, connection_id: connectionId, connection_name: connectionName });
    await Promise.all([this.loadTree(), this.loadEnvironments(), this.loadToolchains()]);
  }

  clearTarget(): void {
    this.disposeStreams();
    this.resetTargetState({ kind: 'local', workspace_id: null, root: '', connection_id: null, connection_name: null });
  }

  private resetTargetState(next: IdeTargetSpec): void {
    this.target = next;
    this.treeRootEntries = []; this.childrenByDir = {}; this.expandedDirs = {}; this.treeError = null;
    this.tabs = []; this.hasClosedAllTabs = false; this.activePath = null; this.splitPath = null; this.splitDirection = null;
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
      if (isRoot) this.treeError = loadErrorText(e);
      else toastError('Couldn’t open folder', e);
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
        this.tabs = this.tabs.filter((t) => t.path !== path && !t.path.startsWith(`${path}/`));
        if (this.activePath === path) this.activePath = this.tabs[0]?.path ?? null;
        if (this.splitPath === path) this.closeSplit();
      } else if (op === 'rename' && destPath) {
        this.tabs = this.tabs.map((t) => (t.path === path ? { ...t, path: destPath, name: ideBasename(destPath) } : t));
        if (this.activePath === path) this.activePath = destPath;
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
      if (opts.inSplit) { this.splitPath = fullPath; if (!this.splitDirection) this.splitDirection = 'right'; }
      else this.activePath = fullPath;
      if (opts.line) this.jumpTo(fullPath, opts.line, opts.col ?? 1);
      return;
    }
    const buf = this.sharedBuffers[fullPath]?.content ?? '';
    const isGuest = this.collabIsGuest;
    const placeholder: IdeOpenTab = {
      path: fullPath, name: ideBasename(fullPath), content: buf, savedContent: buf, sha256: '',
      language: detectIdeLanguage(fullPath), readOnly: false, truncated: false,
      loading: !isGuest, error: null, preview: opts.preview ?? false,
    };
    const prevIdx = opts.preview ? this.tabs.findIndex((t) => t.preview && t.content === t.savedContent) : -1;
    if (prevIdx >= 0) this.tabs[prevIdx] = placeholder;
    else this.tabs = [...this.tabs, placeholder];
    if (opts.inSplit) { this.splitPath = fullPath; if (!this.splitDirection) this.splitDirection = 'right'; }
    else this.activePath = fullPath;
    if (isGuest) { if (opts.line) this.jumpTo(fullPath, opts.line, opts.col ?? 1); return; }
    try {
      const res = await api.get<IdeFileReadResp>(`/ide/fs/file?${this.targetQuery({ path: fullPath })}`);
      const tab = this.tabs.find((t) => t.path === fullPath);
      if (tab) {
        tab.content = res.content; tab.savedContent = res.content; tab.sha256 = res.sha256;
        tab.language = res.language || detectIdeLanguage(fullPath);
        tab.readOnly = res.read_only; tab.truncated = res.truncated; tab.loading = false;
      }
      if (opts.line) this.jumpTo(fullPath, opts.line, opts.col ?? 1);
      void this.loadGutter(fullPath, res.content);
      void this.loadSymbols(fullPath, res.content);
    } catch (e) {
      const tab = this.tabs.find((t) => t.path === fullPath);
      if (tab) { tab.loading = false; tab.error = loadErrorText(e); }
    }
  }

  pinTab(path: string): void { const tab = this.tabs.find((t) => t.path === path); if (tab) tab.preview = false; }
  openInSplit(path: string, dir: 'right' | 'down' = 'right'): void { this.splitPath = path; this.splitDirection = dir; }
  closeSplit(): void { this.splitPath = null; this.splitDirection = null; }
  jumpTo(path: string, line: number, col = 1): void { this.jumpSeq += 1; this.jumpRequest = { path, line, col, seq: this.jumpSeq }; }

  async closeTab(path: string): Promise<void> {
    const tab = this.tabs.find((t) => t.path === path);
    if (!tab) return;
    if (tab.content !== tab.savedContent) {
      const res = await confirmer.choose(`“${tab.name}” has unsaved changes.`, {
        title: 'Unsaved changes',
        options: [{ label: 'Save', value: 'save', kind: 'primary' }, { label: 'Discard changes', value: 'discard', kind: 'danger' }],
      });
      if (!res.value) return;
      if (res.value === 'save' && !(await this.saveFile(path))) return;
    }
    const idx = this.tabs.findIndex((t) => t.path === path);
    this.tabs = this.tabs.filter((t) => t.path !== path);
    if (this.tabs.length === 0) this.hasClosedAllTabs = true;
    if (this.activePath === path) this.activePath = this.tabs[Math.min(idx, this.tabs.length - 1)]?.path ?? null;
    if (this.splitPath === path) this.closeSplit();
  }

  closeOtherTabs(keepPath: string): void {
    this.tabs = this.tabs.filter((t) => t.path === keepPath || t.content !== t.savedContent);
    this.activePath = keepPath;
    if (this.splitPath && this.splitPath !== keepPath) this.closeSplit();
  }

  updateBuffer(path: string, content: string): void {
    const tab = this.tabs.find((t) => t.path === path);
    if (!tab || tab.content === content) return;
    tab.content = content;
    tab.preview = false;
    if (this.collabRoom && this.canEditCollab) {
      if (this.collabEditTimer) clearTimeout(this.collabEditTimer);
      this.collabEditTimer = setTimeout(() => void this.sendCollabEdit(path, content, false), 120);
    }
  }

  updateCursor(path: string, info: { line: number; col: number; cursors: number; selections: { anchor: number; head: number }[] }): void {
    this.cursorInfo = info;
    if (this.collabRoom && this.collabPeerId && this.collabPeerSecret) {
      if (this.cursorSendTimer) clearTimeout(this.cursorSendTimer);
      this.cursorSendTimer = setTimeout(() => void this.sendCursorUpdate(path, info.selections, info.line), 90);
    }
  }

  async saveFile(path?: string, forceOverwrite = false): Promise<boolean> {
    const targetPath = path ?? this.activePath;
    const tab = targetPath ? this.tabs.find((t) => t.path === targetPath) : null;
    if (!targetPath || !tab || tab.readOnly || !this.canEditCollab) return false;
    if (this.collabIsGuest) {
      await this.sendCollabEdit(targetPath, tab.content, true);
      tab.savedContent = tab.content;
      toasts.success('Saved to shared session', tab.name);
      return true;
    }
    try {
      const resp = await api.put<{ path: string; size: number; sha256: string }>('/ide/fs/file', {
        target: this.targetPayload, path: targetPath, content: tab.content,
        expected_sha256: forceOverwrite || !tab.sha256 ? undefined : tab.sha256,
      });
      tab.savedContent = tab.content;
      tab.sha256 = resp.sha256;
      void this.loadGutter(targetPath, tab.content);
      void this.loadSymbols(targetPath, tab.content);
      return true;
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        const choice = await confirmer.choose(`“${tab.name}” changed on disk since you opened it.`, {
          title: 'File conflict on disk',
          options: [{ label: 'Overwrite disk', value: 'overwrite', kind: 'danger' }, { label: 'Reload from disk', value: 'reload', kind: 'normal' }],
        });
        if (choice.value === 'overwrite') return this.saveFile(targetPath, true);
        if (choice.value === 'reload') { this.tabs = this.tabs.filter((t) => t.path !== targetPath); await this.openFile(targetPath); }
        return false;
      }
      toastError(`Couldn’t save ${tab.name}`, e);
      return false;
    }
  }

  async saveAll(): Promise<void> {
    const dirty = this.tabs.filter((t) => t.content !== t.savedContent && !t.readOnly);
    for (let i = 0; i < dirty.length; i++) await this.saveFile(dirty[i].path);
  }

  applyPrefixSuffix(prefix: string, suffix: string): void {
    if (!prefix && !suffix) return;
    if (this.editorHandles.primary) { this.editorHandles.primary.applyPrefixSuffix(prefix, suffix); return; }
    const tab = this.activeTab;
    if (!tab || tab.readOnly) return;
    const lines = tab.content.split('\n');
    const selectedLines = new Set<number>();
    if (this.cursorInfo.selections.length > 0) {
      for (const sel of this.cursorInfo.selections) {
        const startLine = tab.content.slice(0, Math.min(sel.anchor, sel.head)).split('\n').length - 1;
        const endLine = tab.content.slice(0, Math.max(sel.anchor, sel.head)).split('\n').length - 1;
        for (let ln = startLine; ln <= endLine; ln++) selectedLines.add(ln);
      }
    } else selectedLines.add(Math.max(0, this.cursorInfo.line - 1));
    this.updateBuffer(tab.path, lines.map((txt, idx) => (selectedLines.has(idx) ? `${prefix}${txt}${suffix}` : txt)).join('\n'));
  }

  splitSelectionToLines(): void { this.editorHandles.primary?.splitSelectionToLines(); }

  async loadGutter(path: string, content?: string): Promise<void> {
    if (!this.target.root || this.collabIsGuest) return;
    try {
      const resp = await api.bg.post<{ items: IdeGutterItem[] }>('/ide/gutter', { target: this.targetPayload, path, content });
      this.gutterByPath = { ...this.gutterByPath, [path]: resp.items ?? [] };
    } catch { /* non-fatal */ }
  }

  async loadSymbols(path: string, content?: string): Promise<void> {
    if (!this.target.root || this.collabIsGuest) return;
    this.symbolsLoading = true;
    try {
      const resp = await api.bg.post<{ symbols: IdeSymbolNode[] }>('/ide/lsp/symbols', { target: this.targetPayload, path, content });
      this.symbolsByPath = { ...this.symbolsByPath, [path]: resp.symbols ?? [] };
    } catch { /* non-fatal */ }
    finally { this.symbolsLoading = false; }
  }

  setDiagnostics(path: string, items: IdeDiagnostic[]): void {
    this.problems = [...this.problems.filter((d) => d.path !== path), ...items];
  }

  async findReferences(path: string, line: number, column: number, symbol?: string): Promise<void> {
    this.usagesLoading = true;
    this.openBottomTab('usages');
    try {
      const resp = await api.post<{ symbol: string; locations: IdeReferenceLocation[] }>('/ide/lsp/references', {
        target: this.targetPayload, path, line, column, symbol,
      });
      this.usagesSymbol = resp.symbol || symbol || '';
      this.usages = resp.locations ?? [];
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
      for (const tab of this.tabs) {
        if (!touched.has(tab.path)) continue;
        const fresh = await api.get<IdeFileReadResp>(`/ide/fs/file?${this.targetQuery({ path: tab.path })}`);
        tab.content = fresh.content; tab.savedContent = fresh.content; tab.sha256 = fresh.sha256;
      }
      this.renameState.open = false;
      toasts.success(`Renamed to “${next}”`, `${plural(touched.size, 'file')} updated`);
      return true;
    } catch (e) { this.renameState.error = loadErrorText(e); return false; }
    finally { this.renameState.loading = false; }
  }

  async formatActiveFile(): Promise<void> {
    const tab = this.activeTab;
    if (!tab || tab.readOnly) return;
    try {
      const resp = await api.post<{ formatted: string; changed: boolean; formatter: string }>('/ide/lsp/format', { target: this.targetPayload, path: tab.path, content: tab.content, language: tab.language });
      if (resp.changed && typeof resp.formatted === 'string') { this.updateBuffer(tab.path, resp.formatted); toasts.info(`Formatted with ${resp.formatter || 'formatter'}`); }
    } catch (e) { toastError('Couldn’t format file', e); }
  }

  async fetchCodeActions(path: string, line: number, column: number): Promise<IdeCodeAction[]> {
    const tab = this.tabs.find((t) => t.path === path);
    try { return (await api.post<{ actions: IdeCodeAction[] }>('/ide/lsp/code-actions', { target: this.targetPayload, path, line, column, content: tab?.content })).actions ?? []; }
    catch (e) { toastError('Couldn’t load code actions', e); return []; }
  }

  applyCodeAction(path: string, action: IdeCodeAction): void {
    if (typeof action.replacement_content === 'string') { this.updateBuffer(path, action.replacement_content); toasts.info(action.title); }
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
        for (const tab of this.tabs) {
          if (!resp.previews.some((p) => p.path === tab.path)) continue;
          const fresh = await api.get<IdeFileReadResp>(`/ide/fs/file?${this.targetQuery({ path: tab.path })}`);
          tab.content = fresh.content; tab.savedContent = fresh.content; tab.sha256 = fresh.sha256;
        }
        toasts.success(`Replaced ${plural(resp.replacements, 'occurrence')}`, `${plural(resp.files_changed, 'file')} updated`);
      }
      return resp;
    } catch (e) { toastError('Couldn’t replace in project', e); return null; }
    finally { this.replaceLoading = false; }
  }

  async loadToolchains(): Promise<void> { if (!this.collabIsGuest) await fetchToolchains(this, () => this.ensureJobsPoller()); }

  async installToolchain(ecosystem: string, component: 'sdk' | 'lsp' | 'dap' | 'all', version?: string): Promise<IdeInstallJob | null> {
    try {
      const resp = await api.post<{ job: IdeInstallJob }>('/ide/toolchains/install', { target: this.targetPayload, ecosystem, component, version });
      this.installJobs = [resp.job, ...this.installJobs.filter((j) => j.id !== resp.job.id)];
      this.ensureJobsPoller();
      return resp.job;
    } catch (e) { toastError(`Couldn’t install ${ecosystem} ${component}`, e); return null; }
  }

  private ensureJobsPoller(): void {
    this.jobsPoller = startJobsPoller(this, this.jobsPoller, () => void this.loadToolchains());
  }

  async loadEnvironments(): Promise<void> { if (!this.collabIsGuest) await fetchEnvironments(this); }
  async saveEnvironment(patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[] }): Promise<boolean> { return persistEnvironment(this, patch); }

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

  async saveRunConfig(cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> { return upsertRunConfig(this, cfg); }

  async deleteRunConfig(id: string): Promise<void> {
    try {
      await api.del(`/ide/run-configs/${encodeURIComponent(id)}?${this.targetQuery()}`);
      this.runConfigs = this.runConfigs.filter((c) => c.id !== id);
      if (this.selectedConfigId === id) this.selectedConfigId = this.runConfigs[0]?.id ?? null;
    } catch (e) { toastError('Couldn’t delete run configuration', e); }
  }

  async startRun(opts: { configId?: string; adhoc?: Partial<IdeRunConfig> } = {}): Promise<void> {
    this.runPoller = await launchIdeRun(this, opts, this.runPoller);
  }

  async stopRun(runId?: string): Promise<void> {
    const id = runId ?? this.activeRun?.id;
    if (!id) return;
    try {
      const resp = await api.post<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(id)}/stop`, {});
      this.activeRun = resp.run;
      this.runPoller?.stop(); this.runPoller = null;
    } catch (e) { toastError('Couldn’t stop run', e); }
  }

  runFromGutter(item: IdeGutterItem, mode: 'run' | 'debug'): void {
    launchGutterAction(this, item, mode, (adhoc) => void this.startRun({ adhoc }), (opts) => void this.startDebug(opts));
  }

  toggleBreakpoint(path: string, line: number): void {
    const list = this.breakpointsByPath[path] ?? [];
    const next = list.some((b) => b.line === line) ? list.filter((b) => b.line !== line) : [...list, { path, line, enabled: true }].sort((a, b) => a.line - b.line);
    this.breakpointsByPath = { ...this.breakpointsByPath, [path]: next };
    void this.syncBreakpointsToSession();
  }

  updateBreakpoint(path: string, line: number, patch: Partial<IdeBreakpoint>): void {
    const list = this.breakpointsByPath[path] ?? [];
    const next = list.some((b) => b.line === line)
      ? list.map((b) => (b.line === line ? { ...b, ...patch } : b))
      : [...list, { path, line, enabled: true, ...patch }].sort((a, b) => a.line - b.line);
    this.breakpointsByPath = { ...this.breakpointsByPath, [path]: next };
    void this.syncBreakpointsToSession();
  }

  removeBreakpoint(path: string, line: number): void {
    this.breakpointsByPath = { ...this.breakpointsByPath, [path]: (this.breakpointsByPath[path] ?? []).filter((b) => b.line !== line) };
    void this.syncBreakpointsToSession();
  }

  async syncBreakpointsToSession(): Promise<void> {
    if (!this.activeDebug || this.activeDebug.state === 'terminated') return;
    try {
      const resp = await api.put<{ session: IdeDebugSessionSnapshot }>(
        `/ide/debug/sessions/${encodeURIComponent(this.activeDebug.id)}/breakpoints`,
        { breakpoints: this.allBreakpoints, break_on_exceptions: this.breakOnExceptions },
      );
      this.applyDebugSnapshot(resp.session);
    } catch { /* exited */ }
  }

  async startDebug(opts: {
    configId?: string; language?: string;
    launch?: { program: string; args?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string> };
  } = {}): Promise<void> {
    this.debugSseCtl = await launchDebugSession(this, opts, this.debugSseCtl, (s) => this.applyDebugSnapshot(s));
  }

  applyDebugSnapshot(snap: IdeDebugSessionSnapshot): void {
    const prevPaused = `${this.activeDebug?.paused_file ?? ''}:${this.activeDebug?.paused_line ?? 0}`;
    this.activeDebug = snap;
    if (snap.state === 'paused' && snap.paused_file && snap.paused_line) {
      if (`${snap.paused_file}:${snap.paused_line}` !== prevPaused) void this.openFile(snap.paused_file, { line: snap.paused_line, preview: true });
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
  runToCursor = (path?: string, line?: number) => this.debugCommand('run_to_cursor', { path: path ?? this.activePath ?? undefined, line: line ?? this.cursorInfo.line });
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
    try { const resp = await api.get<{ rooms: IdeCollabRoom[] }>('/ide/collab/rooms'); this.collabRooms = resp.rooms ?? []; }
    catch { /* ignore */ }
  }

  async startCollabRoom(defaultRole: IdeCollabRole = 'editor', title?: string): Promise<void> {
    this.collabSseCtl = await startCollabSession(this, defaultRole, title, this.collabSseCtl);
  }

  async setCollabPeerRole(peerId: string, role: IdeCollabRole): Promise<void> {
    if (!this.collabRoom) return;
    try {
      const resp = await api.post<{ room: IdeCollabRoom }>(`/ide/collab/rooms/${encodeURIComponent(this.collabRoom.id)}/role`, { peer_id: peerId, role });
      this.collabRoom = resp.room;
    } catch (e) { toastError('Couldn’t update teammate role', e); }
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
    if (!this.collabRoom || !this.collabPeerId || !this.collabPeerSecret || !this.canEditCollab) return;
    try {
      const res = await collabGuestPost<{ version: number }>(`/ide/collab/guest/${encodeURIComponent(this.collabRoom.id)}/edit`, this.collabGuestToken, {
        peer_id: this.collabPeerId, peer_secret: this.collabPeerSecret, path, content,
        base_version: this.sharedBuffers[path]?.version, save,
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
    this.runPoller?.stop(); this.runPoller = null;
    this.jobsPoller?.stop(); this.jobsPoller = null;
    this.debugSseCtl?.abort(); this.debugSseCtl = null;
    this.collabSseCtl?.abort(); this.collabSseCtl = null;
  }
}

export const ide = new IdeStore();
