// Reactive Svelte 5 store for the Otto Full IDE module.
// Re-exports all canonical IDE types so components import everything from `./ideStore.svelte`.

export * from './store/ideTypes';

import { api } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import { type Poller } from '../../lib/poll';
import { lsGet, lsSet } from '../../lib/storage';
import { toastError } from '../../lib/toastError';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import {
  runApplyCodeAction, runApplyQuickFixEdits, runApplyRename, runEvaluateDebug, runExpandVariables,
  runFetchCodeActions, runFindReferences, runFormatActiveFile, runPreviewRename, runRemovePlugin,
  runReplaceInWorkspace, runSearchWorkspace, runSendCollabEdit, runSendCursorUpdate, runSetCollabPeerRole, runTogglePlugin,
} from './shell/ideLspAndSearchHelpers';
import {
  attachProjectFolder, canonicalizeLocalProjectPath, createTemplatedFile, detachProjectFolder,
  detectFastRunConfigs, findContainingRoot, formatRelativeToRoots, loadAttachedRootEntries,
  loadSavedAdditionalRoots, loadSavedBreakpoints, mutatePathBreakpoints, normalizeBreakpointsMap,
  normalizeRootPath, performCloseAllTabs, pushNavigationHistory, revealPathInTree, saveAdditionalRootsLocal,
} from './shell/ideProjectHelpers';
import {
  activateNamedProject, attachProjectFoldersBatch, detectBuildToolFromState, loadActiveProjectIdLocal,
  loadBuildToolPrefsLocal, loadNamedProjectsLocal, performMutateFs, probeWorkspaceBuildTool,
  removeNamedProject, restoreWorkspaceTabsOrInitial, saveActiveProjectIdLocal, saveBuildToolPrefsLocal,
  saveLastIdeRoot, syncCurrentProjectFolders, syncNamedProjectsFromBackend, upsertNamedProject,
} from './shell/ideProjectsManager';
import {
  applyFallbackPrefixSuffix, computeFallbackSelection, IdeBufferPerfController,
  performAgentEdit, performCloseTab, performOpenFile, performSaveFile, saveWorkspaceOpenTabs,
} from './store/ideBufferPerf';
import {
  followCollabPeer, installIdePlugin, joinCollabSession, loadIdePlugins, startCollabSession,
} from './store/ideCollabController';
import {
  createProjectPythonVenv, fetchEnvironments, fetchShellEnv, fetchToolchains, installToolchainJob,
  launchDebugSession, launchGutterAction, launchIdeRun, mergeRunConfigs, normalizeIncomingDebugSnapshot,
  persistEnvironment, refreshWatchValues, registerLocalToolchain, removeSavedRunConfig,
  sendDebugAction, startJobsPoller, syncSnapshotBreakpointsToStore, upsertRunConfig,
} from './store/ideDebugAndEnvController';
import {
  ideBasename, type IdeBottomTab, type IdeBreakpoint, type IdeCodeAction, type IdeCollabRole, type IdeCollabRoom,
  type IdeDebugSessionSnapshot, type IdeDetectedSdk, type IdeDiagnostic, type IdeEditorHandle, type IdeEditorSelectionRange,
  type IdeEnvVar, type IdeFsEntry, type IdeFsTreeResp, type IdeGutterItem,
  type IdeInstalledSdk, type IdeInstallJob, type IdeKeymapPreset, type IdeNamedProject, type IdeNavLocation, type IdeOpenTab,
  type IdePeerCursor, type IdePluginCatalogEntry, type IdePluginInfo, type IdeProjectEnvironment,
  type IdeProjectRoot, type IdePythonVenv, type IdeReferenceLocation, type IdeRenameEdit,
  type IdeReplaceResp, type IdeReplEntry, type IdeRunConfig, type IdeRunState, type IdeSearchItem,
  type IdeSearchMode, type IdeShellEnvResp, type IdeSymbolNode, type IdeTargetSpec, type IdeTextEdit,
  type IdeToolchainEcosystem, type IdeVariable, type IdeWatch,
} from './store/ideTypes';

const KEYMAP_STORAGE_KEY = 'otto.ide.keymap';
const FORMAT_ON_SAVE_KEY = 'otto.ide.formatOnSave';
const DOCK_OPEN_KEY = 'otto.ide.dockOpen';
const DOCK_TAB_KEY = 'otto.ide.dockTab';

class IdeStore {
  target = $state<IdeTargetSpec>({ kind: 'local', workspace_id: null, root: '', connection_id: null, connection_name: null });
  additionalRoots = $state<string[]>([]);
  namedProjects = $state<IdeNamedProject[]>(loadNamedProjectsLocal());
  activeProjectId = $state<string | null>(loadActiveProjectIdLocal());
  bulkRepoPickerMode = $state<'create' | 'attach' | null>(null);
  targetOs = $state('');
  targetArch = $state('');
  treeRootEntries = $state<IdeFsEntry[]>([]);
  childrenByDir = $state<Record<string, IdeFsEntry[]>>({});
  expandedDirs = $state<Record<string, boolean>>({});
  treeLoading = $state(false);
  treeError = $state<string | null>(null);
  showHidden = $state(false);
  treeFilter = $state('');
  treeRevealRequest = $state<{ path: string; seq: number } | null>(null);

  tabs = $state<IdeOpenTab[]>([]);
  hasClosedAllTabs = $state(false);
  private _activePath = $state<string | null>(null);
  splitPath = $state<string | null>(null);
  splitDirection = $state<'right' | 'down' | null>(null);
  recentPaths = $state<string[]>([]);
  navHistory = $state<IdeNavLocation[]>([]);
  navIndex = $state(-1);
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

  searchScopePath = $state<string | null>(null);
  searchReplaceMode = $state(false);
  searchResults = $state<IdeSearchItem[]>([]);
  searchTruncated = $state(false);
  searchLoading = $state(false);
  searchError = $state<string | null>(null);
  replacePreview = $state<IdeReplaceResp | null>(null);
  replaceLoading = $state(false);

  toolchains = $state<IdeToolchainEcosystem[]>([]);
  detectedSdks = $state<IdeDetectedSdk[]>([]);
  toolchainsLoading = $state(false);
  toolchainsError = $state<string | null>(null);
  installJobs = $state<IdeInstallJob[]>([]);
  projectEnv = $state<IdeProjectEnvironment | null>(null);
  effectiveEnv = $state<Record<string, string>>({});
  shellEnv = $state<IdeShellEnvResp | null>(null);
  shellEnvLoading = $state(false);
  pythonVenvs = $state<IdePythonVenv[]>([]);
  dotenvFiles = $state<string[]>([]);
  runConfigs = $state<IdeRunConfig[]>([]);
  detectedConfigs = $state<IdeRunConfig[]>([]);
  selectedConfigId = $state<string | null>(null);
  envLoading = $state(false);
  envError = $state<string | null>(null);
  activeRun = $state<IdeRunState | null>(null);
  runsHistory = $state<IdeRunState[]>([]);

  breakpointsByPath = $state<Record<string, IdeBreakpoint[]>>({});
  breakpointsMuted = $state(false);
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
  formatOnSave = $state<boolean>(lsGet(FORMAT_ON_SAVE_KEY) === '1');
  settingsOpen = $state(false);
  settingsSection = $state<string>('project.project');
  buildToolOpen = $state(false);
  detectedBuildToolOverride = $state<'maven' | 'gradle' | null>(null);
  activeMavenProfiles = $state<string[]>([]);
  activeSpringProfiles = $state<string[]>([]);
  buildSkipTests = $state(false);
  buildOffline = $state(false);
  closedTabPaths = $state<string[]>([]);
  toolchainsOpen = $state(false);
  environmentsOpen = $state(false);
  environmentsInitialTab = $state<'env' | 'configs'>('env');
  sshOpen = $state(false);
  shareOpen = $state(false);
  pluginsOpen = $state(false);
  searchEverywhereOpen = $state(false);
  searchInitialTab = $state<'all' | 'files' | 'symbols' | 'text' | 'actions'>('all');
  prefixSuffixOpen = $state(false);
  pendingTerminalCwd = $state<{ cwd: string; seq: number } | null>(null);
  bottomDockOpen = $state(lsGet(DOCK_OPEN_KEY) === '1');
  activeBottomTab = $state<IdeBottomTab>(
    (['debug', 'run', 'git', 'problems', 'usages', 'terminal'].includes(lsGet(DOCK_TAB_KEY) ?? '') ? lsGet(DOCK_TAB_KEY) : 'run') as IdeBottomTab,
  );

  readonly hasTarget = $derived(Boolean(this.target.root));
  readonly buildToolKind = $derived(detectBuildToolFromState(this.detectedBuildToolOverride, this.treeRootEntries, this.childrenByDir, this.tabs, this.runConfigs));
  readonly activeProject = $derived<IdeNamedProject | null>(
    this.activeProjectId ? (this.namedProjects.find((p) => p.id === this.activeProjectId) ?? null) : null,
  );
  readonly projectRoots = $derived.by((): IdeProjectRoot[] => {
    if (!this.target.root) return [];
    const primary = normalizeRootPath(this.target.root);
    const out: IdeProjectRoot[] = [{ path: primary, name: ideBasename(primary) || primary, isPrimary: true }];
    for (let i = 0; i < this.additionalRoots.length; i++) {
      const r = normalizeRootPath(this.additionalRoots[i]);
      if (r && r !== primary && !out.some((x) => x.path === r)) {
        out.push({ path: r, name: ideBasename(r) || r, isPrimary: false });
      }
    }
    return out;
  });
  readonly activeTab = $derived(this.tabs.find((t) => t.path === this._activePath) ?? null);
  readonly splitTab = $derived(this.splitPath ? (this.tabs.find((t) => t.path === this.splitPath) ?? null) : null);
  readonly activeSymbols = $derived(this._activePath ? (this.symbolsByPath[this._activePath] ?? []) : []);
  readonly allBreakpoints = $derived(Object.values(this.breakpointsByPath).flat());
  readonly breakpoints = $derived(this.allBreakpoints);
  readonly effectiveBreakpoints = $derived(this.breakpointsMuted ? [] : this.allBreakpoints);
  readonly canNavigateBack = $derived(this.navIndex > 0);
  readonly canNavigateForward = $derived(this.navIndex >= 0 && this.navIndex < this.navHistory.length - 1);
  readonly canEditCollab = $derived(
    !this.collabIsGuest ||
      Boolean(this.collabRoom && (this.collabRoom.peers.find((p) => p.peer_id === this.collabPeerId)?.role ?? this.collabRoom.default_role) === 'editor'),
  );

  private bufPerf = new IdeBufferPerfController();
  private editorHandles: { primary: IdeEditorHandle | null; split: IdeEditorHandle | null } = { primary: null, split: null };
  private jumpSeq = 0;
  private revealSeq = 0;
  private termSeq = 0;
  private isNavigatingHistory = false;
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
    const folders = this.projectRoots.map((r) => r.path);
    return {
      kind: this.target.kind,
      workspace_id: this.target.workspace_id ?? null,
      root: this.target.root,
      connection_id: this.target.kind === 'ssh' ? (this.target.connection_id ?? null) : null,
      additional_roots: this.additionalRoots.length > 0 ? this.additionalRoots : undefined,
      project_folders: folders,
    };
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
    if (/^(?:jdt|jar):/i.test(p)) return p;
    let resolved = p;
    if (p.startsWith('file://')) {
      try { resolved = decodeURIComponent(p.replace(/^file:\/\/(?:localhost)?/, '')); } catch { resolved = p.slice(7); }
    } else if (!p.startsWith('/') && !/^[A-Za-z]:[\\/]/.test(p)) {
      const base = this.target.root.replace(/\/+$/, '');
      resolved = base ? `${base}/${p.replace(/^\/+/, '')}` : p;
    }
    return this.target.kind === 'local' ? canonicalizeLocalProjectPath(resolved, this.projectRoots) : resolved;
  }

  relativePath(p: string): string { return formatRelativeToRoots(p, this.target.root, this.projectRoots); }

  attachDefaultWorkspace(): void {
    if (this.target.root || this.collabIsGuest) return;
    void syncNamedProjectsFromBackend(this);
    if (this.activeProjectId) {
      const savedProj = this.namedProjects.find((p) => p.id === this.activeProjectId);
      if (savedProj) { void activateNamedProject(this, savedProj.id); return; }
    }
    if (!ws.current && !ws.listSettled) return;
    const cur = ws.current ?? ws.workspaces.find((w) => w.id !== SCRATCH_WORKSPACE_ID && w.root_path) ?? ws.workspaces[0];
    if (cur?.root_path) void this.setLocalTarget(cur.root_path, cur.id);
  }

  // Zero-wait cold open: ONLY awaits the shallow 1-level `/ide/fs/tree` (<10ms).
  async setLocalTarget(
    root: string,
    workspaceId: string | null = null,
    opts: { additionalRoots?: string[]; projectId?: string | null; skipAutoOpen?: boolean } = {},
  ): Promise<void> {
    const clean = normalizeRootPath(root);
    if (!clean) return;
    if (opts.projectId === undefined && this.target.kind === 'local' && normalizeRootPath(this.target.root) === clean && !opts.additionalRoots) return;
    const matchedProj = opts.projectId !== undefined
      ? opts.projectId
      : (this.namedProjects.find((p) => p.primary_root === clean)?.id ?? null);
    this.activeProjectId = matchedProj;
    saveLastIdeRoot(clean);
    saveActiveProjectIdLocal(matchedProj, this.namedProjects, clean);
    this.resetTargetState({ kind: 'local', workspace_id: workspaceId, root: clean, connection_id: null, connection_name: null }, opts.additionalRoots);
    if (opts.skipAutoOpen) this.hasClosedAllTabs = true;
    await this.loadTree();
    if (opts.skipAutoOpen) this.hasClosedAllTabs = false;
    void this.loadEnvironments();
  }

  async setSshTarget(connectionId: string, root: string, connectionName: string | null = null): Promise<void> {
    const clean = normalizeRootPath(root) || '~';
    this.resetTargetState({ kind: 'ssh', workspace_id: null, root: clean, connection_id: connectionId, connection_name: connectionName });
    await this.loadTree();
    void this.loadEnvironments();
  }

  clearTarget(): void {
    this.resetTargetState({ kind: 'local', workspace_id: null, root: '', connection_id: null, connection_name: null });
  }

  private resetTargetState(next: IdeTargetSpec, overrideAdditionalRoots?: string[]): void {
    this.disposeStreams();
    this.target = next;
    const savedRoots = overrideAdditionalRoots
      ? overrideAdditionalRoots.map((r) => normalizeRootPath(r)).filter((r) => r && r !== normalizeRootPath(next.root))
      : (next.root ? loadSavedAdditionalRoots(next) : []);
    this.additionalRoots = savedRoots;
    if (overrideAdditionalRoots && next.root) saveAdditionalRootsLocal(next, savedRoots);
    this.breakpointsByPath = next.root ? normalizeBreakpointsMap(loadSavedBreakpoints(next), (p) => this.resolvePath(p)) : {};
    const initialExpanded: Record<string, boolean> = {};
    if (next.root) initialExpanded[normalizeRootPath(next.root)] = true;
    // Scale discipline (#71): keep 30+ secondary repo roots collapsed on cold open (<80ms, <30MB RAM)!
    if (savedRoots.length <= 2) {
      for (let i = 0; i < savedRoots.length; i++) initialExpanded[savedRoots[i]] = true;
    }
    this.treeRootEntries = []; this.childrenByDir = {}; this.expandedDirs = initialExpanded; this.treeError = null;
    this.tabs = []; this.closedTabPaths = []; this.hasClosedAllTabs = false; this._activePath = null; this.splitPath = null; this.splitDirection = null;
    this.gutterByPath = {}; this.symbolsByPath = {}; this.problems = []; this.usages = [];
    this.searchResults = []; this.replacePreview = null; this.navHistory = []; this.navIndex = -1;
    this.projectEnv = null; this.effectiveEnv = {}; this.runConfigs = []; this.detectedConfigs = []; this.selectedConfigId = null;
    this.activeRun = null; this.runsHistory = []; this.activeDebug = null; this.expandedVariables = {}; this.replEntries = [];
    const bt = loadBuildToolPrefsLocal(next.root);
    this.buildToolOpen = bt.open; this.detectedBuildToolOverride = null; this.activeMavenProfiles = bt.mavenProfiles;
    this.activeSpringProfiles = bt.springProfiles; this.buildSkipTests = bt.skipTests; this.buildOffline = bt.offline;
  }

  private persistBuildToolPrefs(): void {
    saveBuildToolPrefsLocal(this.target.root, { open: this.buildToolOpen, mavenProfiles: this.activeMavenProfiles, springProfiles: this.activeSpringProfiles, skipTests: this.buildSkipTests, offline: this.buildOffline });
  }
  toggleBuildTool(open?: boolean): void { this.buildToolOpen = open ?? !this.buildToolOpen; this.persistBuildToolPrefs(); }
  toggleMavenProfile(id: string): void { this.activeMavenProfiles = this.activeMavenProfiles.includes(id) ? this.activeMavenProfiles.filter((x) => x !== id) : [...this.activeMavenProfiles, id]; this.persistBuildToolPrefs(); }
  toggleSpringProfile(id: string): void { this.activeSpringProfiles = this.activeSpringProfiles.includes(id) ? this.activeSpringProfiles.filter((x) => x !== id) : [...this.activeSpringProfiles, id]; this.persistBuildToolPrefs(); }
  setBuildSkipTests(v: boolean): void { this.buildSkipTests = v; this.persistBuildToolPrefs(); }
  setBuildOffline(v: boolean): void { this.buildOffline = v; this.persistBuildToolPrefs(); }

  syncAdditionalRootsFromEnv(envRoots: string[]): void {
    const primary = normalizeRootPath(this.target.root);
    const merged = [...this.additionalRoots];
    let changed = false;
    for (let i = 0; i < envRoots.length; i++) {
      const clean = normalizeRootPath(envRoots[i]);
      if (clean && clean !== primary && !merged.includes(clean)) { merged.push(clean); changed = true; }
    }
    if (!changed) return;
    this.additionalRoots = merged;
    saveAdditionalRootsLocal(this.target, merged);
    if (merged.length <= 2) {
      const nextExp = { ...this.expandedDirs };
      for (let i = 0; i < merged.length; i++) {
        if (nextExp[merged[i]] === undefined) nextExp[merged[i]] = true;
        if (nextExp[merged[i]] && !this.childrenByDir[merged[i]]) void loadAttachedRootEntries(this, merged[i]);
      }
      this.expandedDirs = nextExp;
    }
    syncCurrentProjectFolders(this);
  }

  get bottomTab(): IdeBottomTab { return this.activeBottomTab; }
  set bottomTab(tab: IdeBottomTab) { this.openBottomTab(tab); }
  get projectFolders(): IdeProjectRoot[] { return this.projectRoots; }
  get workspaceRoot(): string { return this.target.root; }
  refreshTree(dirPath?: string): Promise<void> { return this.loadTree(dirPath); }
  openSettings(section?: string): void { if (section) this.settingsSection = section; this.settingsOpen = true; }
  setKeymapPreset(preset: IdeKeymapPreset): void { this.keymapPreset = preset; lsSet(KEYMAP_STORAGE_KEY, preset); }
  setFormatOnSave(next: boolean): void { this.formatOnSave = next; lsSet(FORMAT_ON_SAVE_KEY, next ? '1' : '0'); }
  openEnvironmentsModal(tab: 'env' | 'configs' = 'env'): void { this.environmentsInitialTab = tab; this.environmentsOpen = true; }
  openFindInPath(opts: { scopePath?: string | null; replace?: boolean } = {}): void {
    this.searchScopePath = opts.scopePath ?? null;
    this.searchReplaceMode = opts.replace ?? false;
    this.searchInitialTab = 'text';
    this.searchEverywhereOpen = true;
  }
  openTerminalAt(cwd: string): void {
    this.termSeq += 1;
    this.pendingTerminalCwd = { cwd: cwd || this.target.root, seq: this.termSeq };
    this.openBottomTab('terminal');
  }
  openBottomTab(tab: IdeBottomTab): void { this.activeBottomTab = tab; this.bottomDockOpen = true; lsSet(DOCK_OPEN_KEY, '1'); lsSet(DOCK_TAB_KEY, tab); }
  toggleBottomDock(open?: boolean): void { this.bottomDockOpen = open ?? !this.bottomDockOpen; lsSet(DOCK_OPEN_KEY, this.bottomDockOpen ? '1' : '0'); }
  registerEditorHandle(slot: 'primary' | 'split', handle: IdeEditorHandle | null): void { this.editorHandles[slot] = handle; }
  clearProblems(): void { this.problems = []; }
  clearRepl(): void { this.replEntries = []; }

  async loadTree(dirPath?: string): Promise<void> {
    if (!this.target.root || this.collabIsGuest) return;
    const primary = normalizeRootPath(this.target.root);
    const normDir = dirPath ? normalizeRootPath(dirPath) : '';
    const isRoot = !normDir || normDir === primary;
    if (!isRoot && this.additionalRoots.includes(normDir)) {
      await loadAttachedRootEntries(this, normDir);
      return;
    }
    const attachedParent = !isRoot ? findContainingRoot(normDir, this.projectRoots) : null;
    if (isRoot) { this.treeLoading = true; this.treeError = null; }
    try {
      const query = attachedParent && !attachedParent.isPrimary && this.target.kind === 'local'
        ? new URLSearchParams({ kind: 'local', root: attachedParent.path, path: normDir, show_hidden: String(this.showHidden) }).toString()
        : this.targetQuery({ path: isRoot ? '' : normDir, show_hidden: this.showHidden });
      const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${query}`);
      if (resp.target_os) this.targetOs = resp.target_os;
      if (resp.target_arch) this.targetArch = resp.target_arch;
      if (isRoot) {
        if (resp.root && this.target.root === '~') {
          this.target = { ...this.target, root: resp.root };
          this.expandedDirs = { ...this.expandedDirs, [normalizeRootPath(resp.root)]: true };
        }
        const entries = resp.entries ?? [];
        this.treeRootEntries = entries;
        this.childrenByDir = { ...this.childrenByDir, [normalizeRootPath(this.target.root)]: entries };
        if (this.runConfigs.length === 0) {
          const fast = detectFastRunConfigs(this.target.root, entries);
          this.detectedConfigs = fast;
          this.runConfigs = mergeRunConfigs([], fast);
          if (!this.selectedConfigId && this.runConfigs.length > 0) this.selectedConfigId = this.runConfigs[0].id;
        }
        for (let i = 0; i < this.additionalRoots.length; i++) {
          const ar = this.additionalRoots[i];
          if (this.expandedDirs[ar]) void loadAttachedRootEntries(this, ar);
        }
        void probeWorkspaceBuildTool(this);
        void restoreWorkspaceTabsOrInitial(this);
      } else if (normDir) {
        this.childrenByDir = { ...this.childrenByDir, [normDir]: resp.entries ?? [] };
      }
    } catch (e) {
      if (isRoot) this.treeError = loadErrorText(e); else toastError('Couldn’t open folder', e);
    } finally {
      if (isRoot) this.treeLoading = false;
    }
  }

  async toggleDir(dirPath: string): Promise<void> {
    const clean = normalizeRootPath(dirPath);
    const next = !this.expandedDirs[clean];
    this.expandedDirs = { ...this.expandedDirs, [clean]: next };
    if (next && !this.childrenByDir[clean]) await this.loadTree(clean);
  }

  collapseAllDirs(): void {
    const rootsOnly: Record<string, boolean> = {};
    const primary = normalizeRootPath(this.target.root);
    if (primary) rootsOnly[primary] = true;
    this.expandedDirs = rootsOnly;
  }

  async addProjectFolder(folderPath: string): Promise<boolean> {
    const ok = await attachProjectFolder(this, folderPath);
    if (ok) syncCurrentProjectFolders(this);
    return ok;
  }
  attachProjectFolders(paths: string[]): Promise<number> { return attachProjectFoldersBatch(this, paths); }
  async removeProjectFolder(folderPath: string): Promise<boolean> {
    const ok = await detachProjectFolder(this, folderPath);
    if (ok) syncCurrentProjectFolders(this);
    return ok;
  }
  activateProject(projectId: string): Promise<void> { return activateNamedProject(this, projectId); }
  saveNamedProject(spec: { id?: string; name: string; primary_root: string; folders: string[]; pinned?: boolean }): Promise<IdeNamedProject | null> {
    return upsertNamedProject(this, spec);
  }
  deleteNamedProject(projectId: string): Promise<void> { return removeNamedProject(this, projectId); }
  createFileWithTemplate(fullPath: string): Promise<boolean> {
    return createTemplatedFile(this, fullPath, (op, p) => this.mutateFs(op, p));
  }
  revealInProjectTree(rawPath?: string): Promise<void> {
    const target = rawPath ?? this._activePath;
    if (!target) return Promise.resolve();
    this.revealSeq += 1;
    return revealPathInTree(this, target, this.revealSeq);
  }

  mutateFs(op: 'mkdir' | 'create_file' | 'rename' | 'delete' | 'duplicate', path: string, destPath?: string): Promise<boolean> {
    return performMutateFs(this, (p) => this.bufPerf.flush(this, p), (ps, cd) => this.evictClosedMetadata(ps, cd), (p) => { this._activePath = p; }, op, path, destPath);
  }

  private evictClosedMetadata(paths: string[], clearDirty = true): void {
    if (paths.length === 0) return;
    this.bufPerf.evictPaths(this, paths, clearDirty);
    if (!this.collabRoom) {
      const set = new Set(paths);
      this.sharedBuffers = Object.fromEntries(Object.entries(this.sharedBuffers).filter(([k]) => !set.has(k)));
    }
  }

  async openFile(rawPath: string, opts: { line?: number; col?: number; preview?: boolean; inSplit?: boolean; reload?: boolean } = {}): Promise<void> {
    await performOpenFile(this, this.bufPerf, this.isNavigatingHistory, rawPath, opts);
    saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
  }

  pushNavLocation(path: string, line: number, col = 1): void {
    const res = pushNavigationHistory(this.navHistory, this.navIndex, { path, line, col });
    this.navHistory = res.history; this.navIndex = res.index;
  }

  async navigateBack(): Promise<void> {
    if (!this.canNavigateBack) return;
    const loc = this.navHistory[(this.navIndex -= 1)];
    if (!loc) return;
    this.isNavigatingHistory = true;
    try { await this.openFile(loc.path, { line: loc.line, col: loc.col }); } finally { this.isNavigatingHistory = false; }
  }

  async navigateForward(): Promise<void> {
    if (!this.canNavigateForward) return;
    const loc = this.navHistory[(this.navIndex += 1)];
    if (!loc) return;
    this.isNavigatingHistory = true;
    try { await this.openFile(loc.path, { line: loc.line, col: loc.col }); } finally { this.isNavigatingHistory = false; }
  }

  pinTab(path: string): void { const tab = this.tabs.find((t) => t.path === path); if (tab) { tab.preview = false; tab.pinned = true; } }
  togglePinTab(path: string): void { const tab = this.tabs.find((t) => t.path === path); if (tab) { tab.preview = false; tab.pinned = !tab.pinned; } }
  openInSplit(path: string, dir: 'right' | 'down' = 'right'): void { this.bufPerf.flushAll(this); this.splitPath = path; this.splitDirection = dir; }
  closeSplit(): void { this.bufPerf.flushAll(this); this.splitPath = null; this.splitDirection = null; this.bufPerf.cancelHiddenLoads(this); }
  jumpTo(path: string, line: number, col = 1): void { this.jumpSeq += 1; this.jumpRequest = { path, line, col, seq: this.jumpSeq }; }

  async closeTab(path: string): Promise<void> {
    await performCloseTab(this, this.bufPerf, path);
    if (!this.tabs.some((t) => t.path === path)) {
      this.closedTabPaths = [path, ...this.closedTabPaths.filter((p) => p !== path)].slice(0, 20);
      this.evictClosedMetadata([path]);
      saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
    }
  }
  async reopenClosedTab(): Promise<void> {
    const next = this.closedTabPaths[0];
    if (!next) return;
    this.closedTabPaths = this.closedTabPaths.slice(1);
    await this.openFile(next, { preview: false });
  }

  closeOtherTabs(keepPath: string): void {
    this.bufPerf.flushAll(this);
    const closing = this.tabs.filter((t) => t.path !== keepPath && !t.pinned && !this.bufPerf.isTabDirty(t)).map((t) => t.path);
    this.tabs = this.tabs.filter((t) => t.path === keepPath || t.pinned || this.bufPerf.isTabDirty(t));
    this.evictClosedMetadata(closing);
    this.activePath = keepPath;
    if (this.splitPath && !this.tabs.some((t) => t.path === this.splitPath)) this.closeSplit();
    saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
  }

  closeUnmodifiedTabs(): void {
    this.bufPerf.flushAll(this);
    const closing = this.tabs.filter((t) => !t.pinned && !this.bufPerf.isTabDirty(t)).map((t) => t.path);
    this.tabs = this.tabs.filter((t) => t.pinned || this.bufPerf.isTabDirty(t));
    this.evictClosedMetadata(closing);
    if (this.tabs.length === 0) { this.hasClosedAllTabs = true; this.activePath = null; this.closeSplit(); }
    else if (!this.tabs.some((t) => t.path === this._activePath)) this.activePath = this.tabs[0].path;
    saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
  }

  async closeAllTabs(opts?: { force?: boolean }): Promise<void> {
    await performCloseAllTabs(this, this.bufPerf, (paths) => this.evictClosedMetadata(paths), opts);
    saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
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

  async saveFile(path?: string, forceOverwrite = false): Promise<boolean> {
    if (this.formatOnSave && (!path || path === this._activePath)) await this.formatActiveFile();
    return performSaveFile(this, this.bufPerf, path, forceOverwrite);
  }

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

  findReferences(path: string, line: number, column: number, symbol?: string): Promise<void> { return runFindReferences(this, path, line, column, symbol); }
  async openRenameModal(path: string, line: number, column: number, symbol = ''): Promise<void> {
    this.renameState = { open: true, path, line, column, symbol, newName: symbol, edits: [], loading: Boolean(symbol), error: null };
    if (symbol) await this.previewRename(symbol);
  }
  previewRename(newName?: string): Promise<void> { return runPreviewRename(this, newName); }
  applyRename(): Promise<boolean> { return runApplyRename(this, this.bufPerf); }
  formatActiveFile(): Promise<void> { return runFormatActiveFile(this, this.bufPerf); }
  fetchCodeActions(path: string, line: number, column: number): Promise<IdeCodeAction[]> { return runFetchCodeActions(this, this.bufPerf, path, line, column); }
  applyCodeAction(path: string, action: IdeCodeAction): Promise<boolean> { return runApplyCodeAction(this, this.bufPerf, path, action); }
  applyQuickFixEdits(edits: IdeTextEdit[], title?: string): Promise<boolean> { return runApplyQuickFixEdits(this, this.bufPerf, edits, title); }
  searchWorkspace(query: string, mode: IdeSearchMode = 'all', opts: { caseSensitive?: boolean; wholeWord?: boolean; regex?: boolean; fileGlob?: string; scopePath?: string | null; limit?: number } = {}): Promise<void> {
    return runSearchWorkspace(this, query, mode, opts);
  }
  replaceInWorkspace(query: string, replacement: string, dryRun: boolean, opts: { caseSensitive?: boolean; wholeWord?: boolean; regex?: boolean; fileGlob?: string; scopePath?: string | null; paths?: string[] } = {}): Promise<IdeReplaceResp | null> {
    return runReplaceInWorkspace(this, this.bufPerf, query, replacement, dryRun, opts);
  }

  async loadToolchains(opts: { includeCatalog?: boolean } = {}): Promise<void> {
    if (!this.collabIsGuest) await fetchToolchains(this, () => this.ensureJobsPoller(), opts);
  }
  private ensureJobsPoller(): void { this.jobsPoller = startJobsPoller(this, this.jobsPoller, () => void this.loadToolchains({ includeCatalog: this.toolchainsOpen })); }
  async addLocalToolchain(ecosystem: string, sdkPath: string): Promise<IdeInstalledSdk | null> { return registerLocalToolchain(this, ecosystem, sdkPath); }
  async loadShellEnv(reload = false): Promise<IdeShellEnvResp | null> { return this.collabIsGuest ? null : fetchShellEnv(this, reload); }
  async reloadShellEnv(): Promise<IdeShellEnvResp | null> { return this.loadShellEnv(true); }
  async loadEnvironments(): Promise<void> { if (!this.collabIsGuest) await fetchEnvironments(this); }
  async saveEnvironment(patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[] }): Promise<boolean> { return persistEnvironment(this, patch); }
  async saveRunConfig(cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> { return upsertRunConfig(this, cfg); }
  async startRun(opts: { configId?: string; adhoc?: Partial<IdeRunConfig> } = {}): Promise<void> { this.runPoller = await launchIdeRun(this, opts, this.runPoller); }
  runFromGutter(item: IdeGutterItem, mode: 'run' | 'debug'): void { launchGutterAction(this, item, mode, (adhoc) => void this.startRun({ adhoc }), (opts) => void this.startDebug(opts)); }

  installToolchain(ecosystem: string, component: 'sdk' | 'lsp' | 'dap' | 'all', version?: string): Promise<IdeInstallJob | null> {
    return installToolchainJob(this, ecosystem, component, version, () => this.ensureJobsPoller());
  }
  createPythonVenv(venvDir = '.venv', pythonVersion?: string, moduleRoot?: string): Promise<{ path: string; interpreter_path: string; version: string } | null> {
    return createProjectPythonVenv(this, venvDir, pythonVersion, moduleRoot);
  }
  deleteRunConfig(id: string): Promise<void> { return removeSavedRunConfig(this, id); }
  async stopRun(runId?: string): Promise<void> {
    const id = runId ?? this.activeRun?.id;
    if (!id) return;
    try { this.activeRun = (await api.post<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(id)}/stop`, {})).run; this.runPoller?.stop(); this.runPoller = null; }
    catch (e) { toastError('Couldn’t stop run', e); }
  }
  async stopActiveExecution(): Promise<void> {
    if (this.activeRun?.status === 'running') await this.stopRun();
    if (this.activeDebug && this.activeDebug.state !== 'terminated') await this.stopDebug();
  }
  toggleBreakpoint(rawPath: string, rawLine: number): void {
    const line = Math.max(1, Math.floor(rawLine));
    mutatePathBreakpoints(this, rawPath, (path, list) =>
      list.some((b) => b.line === line) ? list.filter((b) => b.line !== line) : [...list, { path, line, enabled: true }],
    );
  }
  updateBreakpoint(rawPath: string, rawLine: number, patch: Partial<IdeBreakpoint>): void {
    const line = Math.max(1, Math.floor(rawLine));
    mutatePathBreakpoints(this, rawPath, (path, list) =>
      list.some((b) => b.line === line) ? list.map((b) => (b.line === line ? { ...b, ...patch } : b)) : [...list, { path, line, enabled: true, ...patch }],
    );
  }
  removeBreakpoint(rawPath: string, rawLine: number): void {
    const line = Math.max(1, Math.floor(rawLine));
    mutatePathBreakpoints(this, rawPath, (_path, list) => list.filter((b) => b.line !== line));
  }
  toggleMuteBreakpoints(): void { this.breakpointsMuted = !this.breakpointsMuted; void this.syncBreakpointsToSession(); }
  async syncBreakpointsToSession(): Promise<void> {
    if (!this.activeDebug || this.activeDebug.state === 'terminated') return;
    try {
      const resp = await api.put<{ session: IdeDebugSessionSnapshot }>(`/ide/debug/sessions/${encodeURIComponent(this.activeDebug.id)}/breakpoints`, { breakpoints: this.effectiveBreakpoints, break_on_exceptions: this.breakOnExceptions });
      this.applyDebugSnapshot(resp.session);
    } catch { /* exited */ }
  }
  async startDebug(opts: { configId?: string; language?: string; launch?: { program: string; args?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string> } } = {}): Promise<void> {
    this.debugSseCtl = await launchDebugSession(this, opts, this.debugSseCtl, (s) => this.applyDebugSnapshot(s));
  }
  applyDebugSnapshot(raw: IdeDebugSessionSnapshot): void {
    const prevPaused = `${this.activeDebug?.paused_file ?? ''}:${this.activeDebug?.paused_line ?? 0}`;
    const { snap, paused } = normalizeIncomingDebugSnapshot(this, raw);
    this.activeDebug = snap;
    syncSnapshotBreakpointsToStore(this, snap);
    if (snap.state === 'paused' && paused && snap.paused_line) {
      if (`${paused}:${snap.paused_line}` !== prevPaused) void this.openFile(paused, { line: snap.paused_line, preview: false });
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

  expandVariables(variablesRef: number): Promise<IdeVariable[]> { return runExpandVariables(this, variablesRef); }
  evaluateDebug(expression: string, context: 'watch' | 'repl' | 'hover' = 'repl', frameId?: number): Promise<string> {
    return runEvaluateDebug(this, expression, context, frameId);
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
    if (!this.collabIsGuest) try { this.collabRooms = (await api.get<{ rooms: IdeCollabRoom[] }>('/ide/collab/rooms')).rooms ?? []; } catch { /* ignore */ }
  }
  async startCollabRoom(defaultRole: IdeCollabRole = 'editor', title?: string): Promise<void> {
    this.bufPerf.flushAll(this);
    this.collabSseCtl = await startCollabSession(this, defaultRole, title, this.collabSseCtl);
  }
  setCollabPeerRole(peerId: string, role: IdeCollabRole): Promise<void> { return runSetCollabPeerRole(this, peerId, role); }
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
  sendCursorUpdate(path: string, selections: { anchor: number; head: number }[], activeLine: number): Promise<void> { return runSendCursorUpdate(this, path, selections, activeLine); }
  sendCollabEdit(path: string, content: string, save = false): Promise<void> { return runSendCollabEdit(this, this.bufPerf, path, content, save); }
  async loadPlugins(): Promise<void> { await loadIdePlugins(this); }
  async installPlugin(source: string): Promise<boolean> { return installIdePlugin(this, source); }
  togglePlugin(slug: string, enabled: boolean): Promise<void> { return runTogglePlugin(this, slug, enabled); }
  removePlugin(slug: string): Promise<void> { return runRemovePlugin(this, slug); }

  suspendOnPageLeave(): void { this.bufPerf.flushAndSnapshotAll(this); }
  disposeStreams(): void {
    this.bufPerf.resetAll(this);
    if (this.cursorSendTimer) clearTimeout(this.cursorSendTimer);
    if (this.collabEditTimer) clearTimeout(this.collabEditTimer);
    this.cursorSendTimer = null; this.collabEditTimer = null;
    this.runPoller?.stop(); this.runPoller = null; this.jobsPoller?.stop(); this.jobsPoller = null;
    this.debugSseCtl?.abort(); this.debugSseCtl = null; this.collabSseCtl?.abort(); this.collabSseCtl = null;
  }
}

export const ide = new IdeStore();
