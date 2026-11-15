// Reactive Svelte 5 store for the Otto Full IDE module.
// Re-exports all canonical IDE types so components import everything from `./ideStore.svelte`.

export * from './store/ideTypes';

import { api, ApiError } from '../../lib/api/client';
import { idePreferences } from '../../lib/idePreferences.svelte';
import { ideWorkspacePort } from '../../lib/ideWorkspacePort.svelte';
import { type Poller } from '../../lib/poll';
import { lsGet, lsSet } from '../../lib/storage';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import {
  attachProjectFolder, canonicalizeLocalProjectPath, createTemplatedFile, detachProjectFolder,
  detectBuildToolFromState, formatRelativeToRoots, loadActiveProjectIdLocal, loadNamedProjectsLocal,
  mutatePathBreakpoints, normalizeRootPath, performCloseAllTabs, pushNavigationHistory,
  revealPathInTree, saveActiveProjectIdLocal, saveBuildToolPrefsLocal, saveLastIdeRoot,
} from './shell/ideProjectHelpers';
import {
  applyFallbackPrefixSuffix, computeFallbackSelection, IdeBufferPerfController, nextTabAccessStamp,
  performAgentEdit, performCloseTab, performOpenFile, performSaveFile, saveWorkspaceOpenTabs,
} from './store/ideBufferPerf';
import {
  computeIdeProjectRoots, fetchIdeWorkspaceTree, IdeCollabState, IdeDebugState, IdeTargetEnvState,
  normalizePathDiagnostics, performCloseTabBatch, resetTargetLifecycleState, syncAdditionalRootsFromEnvState,
  type IdeRunLaunchMemo, type IncomingIdeDiagnostic,
} from './store/ideStateSlices.svelte';
import {
  resolveRepoGoBuildTags,
  resolveRepoRootForPath,
  type RepoTestExecutionMode,
} from './store/ideRepoTestAndBuildTags';
import type * as RepoActionsMod from './store/ideRepoActions';
import type * as ProjMod from './shell/ideProjectsManager';
import type * as LspSearchMod from './shell/ideLspAndSearchHelpers';
import type * as CollabMod from './store/ideCollabController';
import type * as DebugEnvMod from './store/ideDebugAndEnvController';
import type {
  IdeBottomTab, IdeBreakpoint, IdeCodeAction, IdeCollabRole, IdeCollabRoom, IdeDebugSessionSnapshot,
  IdeDiagnostic, IdeEditorHandle, IdeEditorSelectionRange, IdeEnvVar, IdeFsEntry, IdeGutterItem,
  IdeInstalledSdk, IdeInstallJob, IdeKeymapPreset, IdeNamedProject, IdeNavLocation, IdeOpenTab,
  IdeProjectEnvironment, IdeProjectRoot, IdeQuickFix, IdeReferenceLocation, IdeRenameEdit, IdeReplaceResp,
  IdeRunConfig, IdeRunState, IdeSearchItem, IdeSearchMode, IdeShellEnvResp, IdeSymbolNode, IdeTargetSpec,
  IdeTextEdit, IdeVariable,
} from './store/ideTypes';

let projModP: Promise<typeof ProjMod> | null = null;
let lspModP: Promise<typeof LspSearchMod> | null = null;
let collabModP: Promise<typeof CollabMod> | null = null;
let debugEnvModP: Promise<typeof DebugEnvMod> | null = null;
let repoActionsModP: Promise<typeof RepoActionsMod> | null = null;
let debugEnvSyncMod: typeof DebugEnvMod | null = null;
const loadProjMod = () => (projModP ??= import('./shell/ideProjectsManager'));
const loadLspMod = () => (lspModP ??= import('./shell/ideLspAndSearchHelpers'));
const loadCollabMod = () => (collabModP ??= import('./store/ideCollabController'));
const loadDebugEnvMod = () => (debugEnvModP ??= import('./store/ideDebugAndEnvController').then((m) => (debugEnvSyncMod = m)));
const loadRepoActionsMod = () => (repoActionsModP ??= import('./store/ideRepoActions'));

const DOCK_OPEN_KEY = 'otto.ide.dockOpen';
const DOCK_TAB_KEY = 'otto.ide.dockTab';

class IdeStore {
  readonly envState = new IdeTargetEnvState();
  readonly debugState = new IdeDebugState();
  readonly collabState = new IdeCollabState();

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

  settingsOpen = $state(false);
  settingsSection = $state<string>('project.project');
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
  goToLineOpen = $state(false);
  keepTestcontainersAlive = $state(false);
  pendingTerminalCwd = $state<{ cwd: string; seq: number } | null>(null);
  bottomDockOpen = $state(lsGet(DOCK_OPEN_KEY) === '1');
  activeBottomTab = $state<IdeBottomTab>(
    (['debug', 'run', 'git', 'problems', 'usages', 'terminal', 'dbwatch'].includes(lsGet(DOCK_TAB_KEY) ?? '') ? lsGet(DOCK_TAB_KEY) : 'run') as IdeBottomTab,
  );

  // Delegated sub-store accessors (single source of truth on `envState` / `debugState` / `collabState`)
  get toolchains() { return this.envState.toolchains; } set toolchains(v) { this.envState.toolchains = v; }
  get detectedSdks() { return this.envState.detectedSdks; } set detectedSdks(v) { this.envState.detectedSdks = v; }
  get toolchainsLoading() { return this.envState.toolchainsLoading; } set toolchainsLoading(v) { this.envState.toolchainsLoading = v; }
  get toolchainsError() { return this.envState.toolchainsError; } set toolchainsError(v) { this.envState.toolchainsError = v; }
  get installJobs() { return this.envState.installJobs; } set installJobs(v) { this.envState.installJobs = v; }
  get projectEnv() { return this.envState.projectEnv; } set projectEnv(v) { this.envState.projectEnv = v; }
  get effectiveEnv() { return this.envState.effectiveEnv; } set effectiveEnv(v) { this.envState.effectiveEnv = v; }
  get shellEnv() { return this.envState.shellEnv; } set shellEnv(v) { this.envState.shellEnv = v; }
  get shellEnvLoading() { return this.envState.shellEnvLoading; } set shellEnvLoading(v) { this.envState.shellEnvLoading = v; }
  get shellEnvAttempted() { return this.envState.shellEnvAttempted; } set shellEnvAttempted(v) { this.envState.shellEnvAttempted = v; }
  get shellEnvError() { return this.envState.shellEnvError; } set shellEnvError(v) { this.envState.shellEnvError = v; }
  get pythonVenvs() { return this.envState.pythonVenvs; } set pythonVenvs(v) { this.envState.pythonVenvs = v; }
  get dotenvFiles() { return this.envState.dotenvFiles; } set dotenvFiles(v) { this.envState.dotenvFiles = v; }
  get runConfigs() { return this.envState.runConfigs; } set runConfigs(v) { this.envState.runConfigs = v; }
  get detectedConfigs() { return this.envState.detectedConfigs; } set detectedConfigs(v) { this.envState.detectedConfigs = v; }
  get selectedConfigId() { return this.envState.selectedConfigId; } set selectedConfigId(v) { this.envState.selectedConfigId = v; }
  get envLoading() { return this.envState.envLoading; } set envLoading(v) { this.envState.envLoading = v; }
  get envError() { return this.envState.envError; } set envError(v) { this.envState.envError = v; }
  get activeRun() { return this.envState.activeRun; } set activeRun(v) { this.envState.activeRun = v; }
  get runsHistory() { return this.envState.runsHistory; } set runsHistory(v) { this.envState.runsHistory = v; }
  get buildToolOpen() { return this.envState.buildToolOpen; } set buildToolOpen(v) { this.envState.buildToolOpen = v; }
  get detectedBuildToolOverride() { return this.envState.detectedBuildToolOverride; } set detectedBuildToolOverride(v) { this.envState.detectedBuildToolOverride = v; }
  get activeMavenProfiles() { return this.envState.activeMavenProfiles; } set activeMavenProfiles(v) { this.envState.activeMavenProfiles = v; }
  get activeSpringProfiles() { return this.envState.activeSpringProfiles; } set activeSpringProfiles(v) { this.envState.activeSpringProfiles = v; }
  get buildSkipTests() { return this.envState.buildSkipTests; } set buildSkipTests(v) { this.envState.buildSkipTests = v; }
  get buildOffline() { return this.envState.buildOffline; } set buildOffline(v) { this.envState.buildOffline = v; }

  get breakpointsByPath() { return this.debugState.breakpointsByPath; } set breakpointsByPath(v) { this.debugState.breakpointsByPath = v; }
  get breakpointsMuted() { return this.debugState.breakpointsMuted; } set breakpointsMuted(v) { this.debugState.breakpointsMuted = v; }
  get breakOnExceptions() { return this.debugState.breakOnExceptions; } set breakOnExceptions(v) { this.debugState.breakOnExceptions = v; }
  get activeDebug() { return this.debugState.activeDebug; } set activeDebug(v) { this.debugState.activeDebug = v; }
  get expandedVariables() { return this.debugState.expandedVariables; } set expandedVariables(v) { this.debugState.expandedVariables = v; }
  get watches() { return this.debugState.watches; } set watches(v) { this.debugState.watches = v; }
  get replEntries() { return this.debugState.replEntries; } set replEntries(v) { this.debugState.replEntries = v; }
  get debugLoading() { return this.debugState.debugLoading; } set debugLoading(v) { this.debugState.debugLoading = v; }
  get debugStreamActive() { return this.debugState.debugStreamActive; } set debugStreamActive(v) { this.debugState.debugStreamActive = v; }
  get lastDebugLaunch() { return this.debugState.lastDebugLaunch; } set lastDebugLaunch(v) { this.debugState.lastDebugLaunch = v; }
  get canRestartDebug() { return this.debugState.canRestart; }
  get allBreakpoints() { return this.debugState.allBreakpoints; } get breakpoints() { return this.debugState.allBreakpoints; }
  get effectiveBreakpoints() { return this.debugState.effectiveBreakpoints; }

  get collabRoom() { return this.collabState.collabRoom; } set collabRoom(v) { this.collabState.collabRoom = v; }
  get collabRooms() { return this.collabState.collabRooms; } set collabRooms(v) { this.collabState.collabRooms = v; }
  get collabInviteToken() { return this.collabState.collabInviteToken; } set collabInviteToken(v) { this.collabState.collabInviteToken = v; }
  get collabShareHash() { return this.collabState.collabShareHash; } set collabShareHash(v) { this.collabState.collabShareHash = v; }
  get collabIsGuest() { return this.collabState.collabIsGuest; } set collabIsGuest(v) { this.collabState.collabIsGuest = v; }
  get collabPeerId() { return this.collabState.collabPeerId; } set collabPeerId(v) { this.collabState.collabPeerId = v; }
  get collabPeerSecret() { return this.collabState.collabPeerSecret; } set collabPeerSecret(v) { this.collabState.collabPeerSecret = v; }
  get collabGuestToken() { return this.collabState.collabGuestToken; } set collabGuestToken(v) { this.collabState.collabGuestToken = v; }
  get peerCursors() { return this.collabState.peerCursors; } set peerCursors(v) { this.collabState.peerCursors = v; }
  get followingPeerId() { return this.collabState.followingPeerId; } set followingPeerId(v) { this.collabState.followingPeerId = v; }
  get sharedBuffers() { return this.collabState.sharedBuffers; } set sharedBuffers(v) { this.collabState.sharedBuffers = v; }
  get collabLoading() { return this.collabState.collabLoading; } set collabLoading(v) { this.collabState.collabLoading = v; }
  get collabError() { return this.collabState.collabError; } set collabError(v) { this.collabState.collabError = v; }
  get plugins() { return this.collabState.plugins; } set plugins(v) { this.collabState.plugins = v; }
  get pluginCatalog() { return this.collabState.pluginCatalog; } set pluginCatalog(v) { this.collabState.pluginCatalog = v; }
  get pluginsLoading() { return this.collabState.pluginsLoading; } set pluginsLoading(v) { this.collabState.pluginsLoading = v; }
  get pluginsError() { return this.collabState.pluginsError; } set pluginsError(v) { this.collabState.pluginsError = v; }
  get canEditCollab() { return this.collabState.canEditCollab; }

  get keymapPreset(): IdeKeymapPreset { return idePreferences.current.keymapPreset; }
  set keymapPreset(v: IdeKeymapPreset) { idePreferences.update({ keymapPreset: v === 'standard' ? 'standard' : 'jetbrains' }); }
  get formatOnSave(): boolean { return idePreferences.current.formatOnSave; }
  set formatOnSave(v: boolean) { idePreferences.update({ formatOnSave: Boolean(v) }); }

  readonly hasTarget = $derived(Boolean(this.target.root));
  readonly buildToolKind = $derived(detectBuildToolFromState(this.detectedBuildToolOverride, this.treeRootEntries, this.childrenByDir, this.tabs, this.runConfigs));
  readonly activeProject = $derived<IdeNamedProject | null>(
    this.activeProjectId ? (this.namedProjects.find((p) => p.id === this.activeProjectId) ?? null) : null,
  );
  readonly projectRoots = $derived.by((): IdeProjectRoot[] => computeIdeProjectRoots(this.target.root, this.additionalRoots));
  readonly activeTab = $derived(this.tabs.find((t) => t.path === this._activePath) ?? null);
  readonly splitTab = $derived(this.splitPath ? (this.tabs.find((t) => t.path === this.splitPath) ?? null) : null);
  readonly activeSymbols = $derived(this._activePath ? (this.symbolsByPath[this._activePath] ?? []) : []);
  readonly canNavigateBack = $derived(this.navIndex > 0);
  readonly canNavigateForward = $derived(this.navIndex >= 0 && this.navIndex < this.navHistory.length - 1);

  private bufPerf = new IdeBufferPerfController();
  private editorHandles: { primary: IdeEditorHandle | null; split: IdeEditorHandle | null } = { primary: null, split: null };
  private jumpSeq = 0;
  private revealSeq = 0;
  private termSeq = 0;
  private isNavigatingHistory = false;
  private runPoller: Poller | null = null;
  private runSseCtl: AbortController | null = null;
  private jobsPoller: Poller | null = null;
  private debugSseCtl: AbortController | null = null;
  private collabSseCtl: AbortController | null = null;
  private cursorSendTimer: ReturnType<typeof setTimeout> | null = null;
  private collabEditTimer: ReturnType<typeof setTimeout> | null = null;

  syncWorkspacePort(): void {
    ideWorkspacePort.publish({
      root: this.target.kind === 'local' && this.target.root ? this.target.root : null,
      additionalRoots: this.target.kind === 'local' ? this.additionalRoots : [],
      activePath: this._activePath,
      tabs: this.tabs.map((t) => ({ path: t.path, dirty: this.bufPerf.isTabDirty(t) })),
    });
  }

  get activePath(): string | null { return this._activePath; }
  set activePath(next: string | null) {
    if (this._activePath === next) return;
    const prev = this._activePath;
    this.bufPerf.flushAll(this);
    this._activePath = next;
    if (next) {
      const focused = this.tabs.find((t) => t.path === next);
      if (focused) focused.lastAccessedAt = nextTabAccessStamp();
    }
    if (prev && prev !== next && idePreferences.current.autoSave) {
      void this.bufPerf.flushDirtyAutoSave(this, prev);
    }
    this.bufPerf.cancelHiddenLoads(this);
    if (next && !this.collabIsGuest) {
      const tab = this.tabs.find((t) => t.path === next);
      if (tab && !tab.loading && !tab.error) {
        if (this.gutterByPath[next] === undefined) void this.loadGutter(next, this.getBuffer(next));
        if (this.symbolsByPath[next] === undefined) void this.loadSymbols(next, this.getBuffer(next));
      }
    }
    this.syncWorkspacePort();
  }

  get targetPayload(): IdeTargetSpec {
    return {
      kind: this.target.kind, workspace_id: this.target.workspace_id ?? null, root: this.target.root,
      connection_id: this.target.kind === 'ssh' ? (this.target.connection_id ?? null) : null,
      additional_roots: this.additionalRoots.length > 0 ? this.additionalRoots : undefined,
      project_folders: this.projectRoots.map((r) => r.path),
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
    void loadProjMod().then((m) => m.syncNamedProjectsFromBackend(this));
    if (this.activeProjectId) {
      const savedProj = this.namedProjects.find((p) => p.id === this.activeProjectId);
      if (savedProj) { void this.activateProject(savedProj.id); return; }
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
    if (this.target.root && idePreferences.current.autoSave) await this.flushAutoSave();
    const matchedProj = opts.projectId !== undefined ? opts.projectId : (this.namedProjects.find((p) => p.primary_root === clean)?.id ?? null);
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
    if (this.target.root && idePreferences.current.autoSave) await this.flushAutoSave();
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
    resetTargetLifecycleState(this, next, () => { this._activePath = null; }, overrideAdditionalRoots);
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
    syncAdditionalRootsFromEnvState(this, envRoots, () => void loadProjMod().then((m) => m.syncCurrentProjectFolders(this)));
  }

  get bottomTab(): IdeBottomTab { return this.activeBottomTab; }
  set bottomTab(tab: IdeBottomTab) { this.openBottomTab(tab); }
  get projectFolders(): IdeProjectRoot[] { return this.projectRoots; }
  get workspaceRoot(): string { return this.target.root; }
  refreshTree(dirPath?: string): Promise<void> { return this.loadTree(dirPath); }
  openSettings(section?: string): void { if (section) this.settingsSection = section; this.settingsOpen = true; }
  setKeymapPreset(preset: IdeKeymapPreset): void { idePreferences.update({ keymapPreset: preset }); }
  setFormatOnSave(next: boolean): void { idePreferences.update({ formatOnSave: Boolean(next) }); }
  openEnvironmentsModal(tab: 'env' | 'configs' = 'env'): void { this.environmentsInitialTab = tab; this.environmentsOpen = true; }
  openSearchEverywhere(tab: 'all' | 'files' | 'symbols' | 'text' | 'actions' = 'all', opts: { scopePath?: string | null; replace?: boolean } = {}): void {
    if (opts.scopePath !== undefined) this.searchScopePath = opts.scopePath;
    this.searchReplaceMode = opts.replace ?? false; this.searchInitialTab = tab; this.searchEverywhereOpen = true;
  }
  openFindInPath(opts: { scopePath?: string | null; replace?: boolean } = {}): void {
    this.openSearchEverywhere('text', { scopePath: opts.scopePath ?? null, replace: opts.replace ?? false });
  }
  openTerminalAt(cwd: string): void { this.termSeq += 1; this.pendingTerminalCwd = { cwd: cwd || this.target.root, seq: this.termSeq }; this.openBottomTab('terminal'); }
  openBottomTab(tab: IdeBottomTab): void { this.activeBottomTab = tab; this.bottomDockOpen = true; lsSet(DOCK_OPEN_KEY, '1'); lsSet(DOCK_TAB_KEY, tab); }
  toggleBottomDock(open?: boolean): void { this.bottomDockOpen = open ?? !this.bottomDockOpen; lsSet(DOCK_OPEN_KEY, this.bottomDockOpen ? '1' : '0'); }
  registerEditorHandle(slot: 'primary' | 'split', handle: IdeEditorHandle | null): void { this.editorHandles[slot] = handle; }
  clearProblems(): void { this.problems = []; }
  clearRepl(): void { this.replEntries = []; }
  flushHistorySnapshot(path?: string): void { this.bufPerf.flushHistorySnapshot(this, path); }
  flushAutoSave(path?: string): Promise<void> { return this.bufPerf.flushDirtyAutoSave(this, path); }
  clearLiveBuffer(path: string): void { this.bufPerf.liveBuffers.delete(this.resolvePath(path)); }
  loadTree(dirPath?: string): Promise<void> { return fetchIdeWorkspaceTree(this, dirPath, () => void loadProjMod().then((m) => m.probeWorkspaceBuildTool(this))); }
  async toggleDir(dirPath: string): Promise<void> {
    const clean = normalizeRootPath(dirPath); const next = !this.expandedDirs[clean];
    this.expandedDirs = { ...this.expandedDirs, [clean]: next };
    if (next && !this.childrenByDir[clean]) await this.loadTree(clean);
  }
  collapseAllDirs(): void {
    const primary = normalizeRootPath(this.target.root);
    this.expandedDirs = primary ? { [primary]: true } : {};
  }
  async addProjectFolder(folderPath: string): Promise<boolean> {
    const ok = await attachProjectFolder(this, folderPath);
    if (ok) { this.syncWorkspacePort(); void loadProjMod().then((m) => m.syncCurrentProjectFolders(this)); }
    return ok;
  }
  async attachProjectFolders(paths: string[], opts?: { quiet?: boolean }): Promise<number> {
    const count = await (await loadProjMod()).attachProjectFoldersBatch(this, paths, opts);
    if (count > 0) this.syncWorkspacePort();
    return count;
  }
  async removeProjectFolder(folderPath: string): Promise<boolean> {
    const ok = await detachProjectFolder(this, folderPath);
    if (ok) { this.syncWorkspacePort(); void loadProjMod().then((m) => m.syncCurrentProjectFolders(this)); }
    return ok;
  }
  activateProject(projectId: string): Promise<void> { return loadProjMod().then((m) => m.activateNamedProject(this, projectId)); }
  saveNamedProject(spec: { id?: string; name: string; primary_root: string; folders: string[]; pinned?: boolean }): Promise<IdeNamedProject | null> { return loadProjMod().then((m) => m.upsertNamedProject(this, spec)); }
  deleteNamedProject(projectId: string): Promise<void> { return loadProjMod().then((m) => m.removeNamedProject(this, projectId)); }
  createFileWithTemplate(fullPath: string): Promise<boolean> { return createTemplatedFile(this, fullPath, (op, p) => this.mutateFs(op, p)); }
  revealInProjectTree(rawPath?: string): Promise<void> {
    const target = rawPath ?? this._activePath;
    if (!target) return Promise.resolve();
    this.revealSeq += 1; return revealPathInTree(this, target, this.revealSeq);
  }

  async mutateFs(op: 'mkdir' | 'create_file' | 'rename' | 'delete' | 'duplicate', path: string, destPath?: string): Promise<boolean> {
    const ok = await (await loadProjMod()).performMutateFs(
      this, (p) => this.bufPerf.flush(this, p), (ps, cp) => this.evictClosedMetadata(ps, cp),
      (p) => { this._activePath = p; }, op, path, destPath,
    );
    if (ok) this.syncWorkspacePort();
    return ok;
  }
  private evictClosedMetadata(paths: string[], clearProblems = true): void {
    if (paths.length === 0) return;
    this.bufPerf.evictPaths(this, paths, clearProblems);
    if (!this.collabRoom) {
      const set = new Set(paths);
      this.sharedBuffers = Object.fromEntries(Object.entries(this.sharedBuffers).filter(([k]) => !set.has(k)));
    }
  }
  async openFile(rawPath: string, opts: { line?: number; col?: number; preview?: boolean; inSplit?: boolean; reload?: boolean; background?: boolean } = {}): Promise<void> {
    await performOpenFile(this, this.bufPerf, this.isNavigatingHistory, rawPath, opts);
    saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
    this.syncWorkspacePort();
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
  openInSplit(path: string, dir: 'right' | 'down' = 'right'): void {
    this.bufPerf.flushAll(this);
    const tab = this.tabs.find((t) => t.path === path);
    if (tab) tab.lastAccessedAt = nextTabAccessStamp();
    this.splitPath = path; this.splitDirection = dir;
  }
  closeSplit(): void { this.bufPerf.flushAll(this); this.splitPath = null; this.splitDirection = null; this.bufPerf.cancelHiddenLoads(this); }
  jumpTo(path: string, line: number, col = 1): void { this.jumpSeq += 1; this.jumpRequest = { path, line, col, seq: this.jumpSeq }; }
  async closeTab(path: string): Promise<void> {
    await performCloseTab(this, this.bufPerf, path);
    if (!this.tabs.some((t) => t.path === path)) {
      this.closedTabPaths = [path, ...this.closedTabPaths.filter((p) => p !== path)].slice(0, 20);
      this.evictClosedMetadata([path]);
      saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
    }
    this.syncWorkspacePort();
  }
  async reopenClosedTab(): Promise<void> {
    const next = this.closedTabPaths[0];
    if (!next) return;
    this.closedTabPaths = this.closedTabPaths.slice(1);
    await this.openFile(next, { preview: false });
  }
  closeOtherTabs(keepPath: string): void { performCloseTabBatch(this, this.bufPerf, (closing) => this.evictClosedMetadata(closing), keepPath); }
  closeUnmodifiedTabs(): void { performCloseTabBatch(this, this.bufPerf, (closing) => this.evictClosedMetadata(closing)); }

  async closeAllTabs(opts?: { force?: boolean }): Promise<void> {
    await performCloseAllTabs(this, this.bufPerf, (paths) => this.evictClosedMetadata(paths), (p) => this.saveFile(p, false, { silent: true }), opts);
    saveWorkspaceOpenTabs(this.target.root, this.tabs.map((t) => t.path), this._activePath, this.splitPath);
    this.syncWorkspacePort();
  }

  getBuffer(path?: string): string {
    const targetPath = path ? this.resolvePath(path) : this._activePath;
    return targetPath ? this.bufPerf.getBuffer(this, targetPath) : '';
  }

  updateBuffer(path: string, content: string): void {
    const fullPath = this.resolvePath(path);
    if (!this.bufPerf.recordEdit(this, fullPath, content)) return;
    this.syncWorkspacePort();
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

  async saveFile(path?: string, forceOverwrite = false, opts: { silent?: boolean } = {}): Promise<boolean> {
    if (this.formatOnSave && !opts.silent && (!path || path === this._activePath)) await this.formatActiveFile();
    const ok = await performSaveFile(this, this.bufPerf, path, forceOverwrite, opts);
    this.syncWorkspacePort();
    return ok;
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

  async applyAgentEdit(path: string, nextContent: string, range?: { from: number; to: number }): Promise<boolean> {
    const ok = await performAgentEdit(this, this.bufPerf, () => this.editorHandles.primary, path, nextContent, range);
    if (ok) this.syncWorkspacePort();
    return ok;
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
  setDiagnostics(path: string, items: readonly IncomingIdeDiagnostic[]): void {
    this.problems = [...this.problems.filter((d) => d.path !== path), ...normalizePathDiagnostics(path, items)];
  }

  findReferences(path: string, line: number, column: number, symbol?: string): Promise<void> { return loadLspMod().then((m) => m.runFindReferences(this, path, line, column, symbol, this.bufPerf)); }
  async openRenameModal(path: string, line: number, column: number, symbol = '', initialNewName?: string): Promise<void> {
    const startName = initialNewName ?? symbol;
    this.renameState = { open: true, path, line, column, symbol, newName: startName, edits: [], loading: Boolean(startName), error: null };
    if (startName) await this.previewRename(startName);
  }
  previewRename(newName?: string): Promise<void> { return loadLspMod().then((m) => m.runPreviewRename(this, newName, this.bufPerf)); }
  applyRename(): Promise<boolean> { return loadLspMod().then((m) => m.runApplyRename(this, this.bufPerf)); }
  formatActiveFile(): Promise<void> { return loadLspMod().then((m) => m.runFormatActiveFile(this, this.bufPerf)); }
  organizeImportsActiveFile(): Promise<void> { return loadLspMod().then((m) => m.runFormatActiveFile(this, this.bufPerf, { organizeImports: true })); }
  fetchCodeActions(path: string, line: number, column: number): Promise<IdeCodeAction[]> { return loadLspMod().then((m) => m.runFetchCodeActions(this, this.bufPerf, path, line, column)); }
  applyCodeAction(path: string, action: IdeCodeAction | IdeQuickFix): Promise<boolean> { return loadLspMod().then((m) => m.runApplyCodeAction(this, this.bufPerf, path, action)); }
  applyQuickFixEdits(edits: IdeTextEdit[], title?: string): Promise<boolean> { return loadLspMod().then((m) => m.runApplyQuickFixEdits(this, this.bufPerf, edits, title)); }
  searchWorkspace(query: string, mode: IdeSearchMode = 'all', opts: { caseSensitive?: boolean; wholeWord?: boolean; regex?: boolean; fileGlob?: string; scopePath?: string | null; limit?: number } = {}): Promise<void> {
    return loadLspMod().then((m) => m.runSearchWorkspace(this, query, mode, opts));
  }
  replaceInWorkspace(query: string, replacement: string, dryRun: boolean, opts: { caseSensitive?: boolean; wholeWord?: boolean; regex?: boolean; fileGlob?: string; scopePath?: string | null; paths?: string[] } = {}): Promise<IdeReplaceResp | null> {
    return loadLspMod().then((m) => m.runReplaceInWorkspace(this, this.bufPerf, query, replacement, dryRun, opts));
  }

  async loadToolchains(opts: { includeCatalog?: boolean } = {}): Promise<void> {
    if (!this.collabIsGuest) await (await loadDebugEnvMod()).fetchToolchains(this, () => this.ensureJobsPoller(), opts);
  }
  private ensureJobsPoller(): void { void loadDebugEnvMod().then((m) => { this.jobsPoller = m.startJobsPoller(this, this.jobsPoller, () => void this.loadToolchains({ includeCatalog: this.toolchainsOpen })); }); }
  addLocalToolchain(ecosystem: string, sdkPath: string): Promise<IdeInstalledSdk | null> { return loadDebugEnvMod().then((m) => m.registerLocalToolchain(this, ecosystem, sdkPath)); }
  async loadShellEnv(reload = false): Promise<IdeShellEnvResp | null> { return this.collabIsGuest ? null : (await loadDebugEnvMod()).fetchShellEnv(this, reload); }
  reloadShellEnv(): Promise<IdeShellEnvResp | null> { return this.loadShellEnv(true); }
  async loadEnvironments(): Promise<void> { if (!this.collabIsGuest) await (await loadDebugEnvMod()).fetchEnvironments(this); }
  saveEnvironment(patch: Partial<IdeProjectEnvironment> & { env_vars?: IdeEnvVar[]; confirm_prod_targets?: boolean }): Promise<boolean> { return loadDebugEnvMod().then((m) => m.persistEnvironment(this, patch)); }
  getRepoGoBuildTags(pathOrRoot?: string | null): string[] {
    const repo = resolveRepoRootForPath(this.target.root, this.projectRoots, pathOrRoot ?? this._activePath);
    return resolveRepoGoBuildTags(this.projectEnv, this.target.root, repo.path);
  }
  setRepoGoBuildTags(pathOrRoot: string, tags: readonly string[] | string): Promise<boolean> { return loadRepoActionsMod().then((m) => m.performSetRepoGoBuildTags(this, pathOrRoot, tags)); }
  runRepoTests(pathOrRoot?: string | null, mode: RepoTestExecutionMode = 'run'): Promise<void> { return loadRepoActionsMod().then((m) => m.performRunRepoTests(this, pathOrRoot, mode)); }
  saveRunConfig(cfg: Partial<IdeRunConfig> & { name: string; runner: string }): Promise<IdeRunConfig | null> { return loadDebugEnvMod().then((m) => m.upsertRunConfig(this, cfg)); }
  recordRunLaunch(runId: string, memo: IdeRunLaunchMemo): void {
    this.envState.lastRunLaunch = memo;
    this.envState.runLaunchById[runId] = memo;
  }
  rerunRun(runId?: string): Promise<void> {
    const memo = (runId ? this.envState.runLaunchById[runId] : null) ?? this.envState.lastRunLaunch ?? {};
    return this.startRun(memo);
  }
  async startRun(opts: { configId?: string; adhoc?: Partial<IdeRunConfig>; keepTestcontainers?: boolean; coverage?: boolean; skipProdConfirm?: boolean } = {}): Promise<void> {
    const gate = await (await loadRepoActionsMod()).confirmIdeTargetProdLaunch(this.target, this.projectEnv, opts.skipProdConfirm);
    if (!gate.allowed) return;
    if (idePreferences.current.autoSave) await this.flushAutoSave();
    const { poller, sseCtl } = await (await loadDebugEnvMod()).launchIdeRun(this, { ...opts, confirmProdTargets: gate.confirmProdTargets }, this.runPoller, this.runSseCtl);
    this.runPoller = poller;
    this.runSseCtl = sseCtl;
  }
  runFromGutter(item: IdeGutterItem, mode: 'run' | 'debug' | 'run_keep_tc'): void {
    void loadDebugEnvMod().then((m) =>
      m.launchGutterAction(this, item, mode, (adhoc, keepTc) => void this.startRun({ adhoc, keepTestcontainers: keepTc }), (opts) => void this.startDebug(opts)),
    );
  }
  installToolchain(ecosystem: string, component: 'sdk' | 'lsp' | 'dap' | 'all', version?: string): Promise<IdeInstallJob | null> {
    return loadDebugEnvMod().then((m) => m.installToolchainJob(this, ecosystem, component, version, () => this.ensureJobsPoller()));
  }
  createPythonVenv(venvDir = '.venv', pythonVersion?: string, moduleRoot?: string): Promise<{ path: string; interpreter_path: string; version: string } | null> {
    return loadDebugEnvMod().then((m) => m.createProjectPythonVenv(this, venvDir, pythonVersion, moduleRoot));
  }
  deleteRunConfig(id: string): Promise<void> { return loadDebugEnvMod().then((m) => m.removeSavedRunConfig(this, id)); }
  async stopRun(runId?: string): Promise<void> {
    const id = runId ?? this.activeRun?.id;
    if (!id) return;
    this.runSseCtl?.abort();
    this.runSseCtl = null;
    try {
      const stopped = (await api.post<{ run: IdeRunState }>(`/ide/runs/${encodeURIComponent(id)}/stop`, {})).run;
      this.activeRun = stopped;
      const archived = stopped.output.length > 200 ? { ...stopped, output: stopped.output.slice(-200) } : stopped;
      this.runsHistory = [archived, ...this.runsHistory.filter((x) => x.id !== archived.id)].slice(0, 8);
      this.runPoller?.stop();
      this.runPoller = null;
    } catch (e) {
      if (e instanceof ApiError && e.status === 404) {
        if (this.activeRun && this.activeRun.id === id) {
          const fallback: IdeRunState = { ...this.activeRun, status: 'stopped' };
          this.activeRun = fallback;
          const archived = fallback.output.length > 200 ? { ...fallback, output: fallback.output.slice(-200) } : fallback;
          this.runsHistory = [archived, ...this.runsHistory.filter((x) => x.id !== archived.id)].slice(0, 8);
        }
        this.runPoller?.stop();
        this.runPoller = null;
        return;
      }
      toastError('Couldn’t stop run', e);
    }
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
  async startDebug(opts: { configId?: string; language?: string; skipProdConfirm?: boolean; launch?: { program: string; args?: string[]; build_flags?: string[]; cwd?: string; test_name?: string; attach_port?: number; env?: Record<string, string> } } = {}): Promise<void> {
    const gate = await (await loadRepoActionsMod()).confirmIdeTargetProdLaunch(this.target, this.projectEnv, opts.skipProdConfirm);
    if (!gate.allowed) return;
    if (idePreferences.current.autoSave) await this.flushAutoSave();
    this.debugSseCtl = await (await loadDebugEnvMod()).launchDebugSession(this, { ...opts, confirmProdTargets: gate.confirmProdTargets }, this.debugSseCtl, (s) => this.applyDebugSnapshot(s));
  }
  applyDebugSnapshot(raw: IdeDebugSessionSnapshot): void {
    const apply = (m: typeof DebugEnvMod): void => {
      const cur = this.activeDebug;
      if (
        cur &&
        cur.id === raw.id &&
        typeof raw.revision === 'number' &&
        typeof cur.revision === 'number' &&
        raw.revision < cur.revision
      ) {
        return;
      }
      const prevPaused = `${cur?.paused_file ?? ''}:${cur?.paused_line ?? 0}`;
      const { snap, paused } = m.normalizeIncomingDebugSnapshot(this, raw);
      this.activeDebug = snap;
      m.syncSnapshotBreakpointsToStore(this, snap);
      if (snap.state === 'paused' && paused && snap.paused_line) {
        if (`${paused}:${snap.paused_line}` !== prevPaused) void this.openFile(paused, { line: snap.paused_line, preview: false });
        void this.refreshWatches();
      }
      if (snap.state === 'terminated') { this.debugSseCtl?.abort(); this.debugSseCtl = null; this.debugStreamActive = false; }
    };
    if (debugEnvSyncMod) apply(debugEnvSyncMod);
    else void loadDebugEnvMod().then(apply);
  }

  async debugCommand(
    action: 'continue' | 'pause' | 'step_over' | 'step_into' | 'step_out' | 'run_to_cursor' | 'select_frame' | 'restart' | 'stop',
    extra: { thread_id?: number; frame_id?: number; path?: string; line?: number } = {},
  ): Promise<void> {
    await (await loadDebugEnvMod()).sendDebugAction(this, action, extra, (s) => this.applyDebugSnapshot(s));
  }

  stepOver = () => this.debugCommand('step_over');
  stepInto = () => this.debugCommand('step_into');
  stepOut = () => this.debugCommand('step_out');
  resumeDebug = () => this.debugCommand('continue');
  pauseDebug = () => this.debugCommand('pause');
  restartDebug = async (): Promise<void> => {
    if (this.activeDebug && this.activeDebug.state !== 'terminated' && this.activeDebug.state !== 'error') {
      await this.stopDebug();
    }
    await this.startDebug(this.debugState.lastDebugLaunch ?? {});
  };
  stopDebug = () => this.debugCommand('stop');
  runToCursor = (path?: string, line?: number) => this.debugCommand('run_to_cursor', { path: path ?? this._activePath ?? undefined, line: line ?? this.cursorInfo.line });
  selectFrame = (frameId: number, threadId?: number) => this.debugCommand('select_frame', { frame_id: frameId, thread_id: threadId });

  expandVariables(variablesRef: number): Promise<IdeVariable[]> { return loadLspMod().then((m) => m.runExpandVariables(this, variablesRef)); }
  evaluateDebug(expression: string, context: 'watch' | 'repl' | 'hover' = 'repl', frameId?: number): Promise<string> {
    return loadLspMod().then((m) => m.runEvaluateDebug(this, expression, context, frameId));
  }
  addWatch(expression: string): void {
    const expr = expression.trim();
    if (!expr) return;
    this.watches = [...this.watches, { id: `w-${Date.now()}-${this.watches.length}`, expression: expr, value: '…' }];
    void this.refreshWatches();
  }
  removeWatch(id: string): void { this.watches = this.watches.filter((w) => w.id !== id); }
  async refreshWatches(): Promise<void> { await (await loadDebugEnvMod()).refreshWatchValues(this); }

  async loadCollabRooms(): Promise<void> {
    if (!this.collabIsGuest) try { this.collabRooms = (await api.get<{ rooms: IdeCollabRoom[] }>('/ide/collab/rooms')).rooms ?? []; } catch { /* ignore */ }
  }
  async startCollabRoom(defaultRole: IdeCollabRole = 'editor', title?: string): Promise<void> {
    this.bufPerf.flushAll(this);
    this.collabSseCtl = await (await loadCollabMod()).startCollabSession(this, defaultRole, title, this.collabSseCtl);
  }
  setCollabPeerRole(peerId: string, role: IdeCollabRole): Promise<boolean> { return loadLspMod().then((m) => m.runSetCollabPeerRole(this, peerId, role)); }
  async stopCollabRoom(): Promise<boolean> {
    if (!this.collabRoom) return false;
    const id = this.collabRoom.id;
    if (!this.collabIsGuest) {
      try {
        await api.del(`/ide/collab/rooms/${encodeURIComponent(id)}`);
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 404)) {
          toastError('Couldn’t close shared session', e);
          return false;
        }
      }
    }
    this.leaveCollabRoom();
    return true;
  }
  leaveCollabRoom(): void { this.collabSseCtl?.abort(); this.collabSseCtl = null; this.collabState.leave(); }
  onCollabJoinFailed(errorMsg: string): void { this.collabSseCtl?.abort(); this.collabSseCtl = null; this.collabState.joinFailed(errorMsg); }
  async joinCollabRoom(roomId: string, token: string, displayName = 'Teammate'): Promise<void> {
    this.collabSseCtl = await (await loadCollabMod()).joinCollabSession(this, roomId, token, displayName, this.collabSseCtl);
  }
  followPeer(peerId: string | null): void { void loadCollabMod().then((m) => m.followCollabPeer(this, peerId)); }
  sendCursorUpdate(path: string, selections: { anchor: number; head: number }[], activeLine: number): Promise<void> { return loadLspMod().then((m) => m.runSendCursorUpdate(this, path, selections, activeLine)); }
  sendCollabEdit(path: string, content: string, save = false): Promise<boolean> { return loadLspMod().then((m) => m.runSendCollabEdit(this, this.bufPerf, path, content, save)); }
  async loadPlugins(): Promise<void> { await (await loadCollabMod()).loadIdePlugins(this); }
  installPlugin(source: string): Promise<boolean> { return loadCollabMod().then((m) => m.installIdePlugin(this, source)); }
  togglePlugin(slug: string, enabled: boolean): Promise<boolean> { return loadLspMod().then((m) => m.runTogglePlugin(this, slug, enabled)); }
  removePlugin(slug: string): Promise<boolean> { return loadLspMod().then((m) => m.runRemovePlugin(this, slug)); }

  suspendOnPageLeave(): void {
    if (idePreferences.current.autoSave) void this.flushAutoSave();
    this.bufPerf.flushAndSnapshotAll(this);
  }
  disposeStreams(): void {
    this.bufPerf.resetAll(this);
    if (this.cursorSendTimer) clearTimeout(this.cursorSendTimer);
    if (this.collabEditTimer) clearTimeout(this.collabEditTimer);
    this.cursorSendTimer = null; this.collabEditTimer = null;
    this.runPoller?.stop(); this.runPoller = null; this.runSseCtl?.abort(); this.runSseCtl = null;
    this.jobsPoller?.stop(); this.jobsPoller = null;
    this.debugSseCtl?.abort(); this.debugSseCtl = null; this.debugStreamActive = false; this.collabSseCtl?.abort(); this.collabSseCtl = null;
  }
}

export const ide = new IdeStore();
