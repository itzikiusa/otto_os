// Cohesive reactive sub-stores (`SOLID-4`) for `IdeStore`:
// - `IdeTargetEnvState`: toolchains, SDKs, environments, run configurations, active runs, and build-tool flags
// - `IdeDebugState`: breakpoints, debugger session snapshot, variables, watches, and REPL
// - `IdeCollabState`: live collaboration room, host/guest credentials, peer cursors, shared buffers, and plugins
// Keeps atomic `reset()` / `leave()` / `joinFailed()` transitions in one place so state never tears.

import { loadBuildToolPrefsLocal } from '../shell/ideProjectHelpers';
import type {
  IdeBreakpoint,
  IdeCollabRoom,
  IdeDebugSessionSnapshot,
  IdeDetectedSdk,
  IdeInstalledSdk,
  IdeInstallJob,
  IdePeerCursor,
  IdePluginCatalogEntry,
  IdePluginInfo,
  IdeProjectEnvironment,
  IdePythonVenv,
  IdeReplEntry,
  IdeRunConfig,
  IdeRunState,
  IdeShellEnvResp,
  IdeToolchainEcosystem,
  IdeVariable,
  IdeWatch,
} from './ideTypes';

export class IdeTargetEnvState {
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

  buildToolOpen = $state(false);
  detectedBuildToolOverride = $state<'maven' | 'gradle' | null>(null);
  activeMavenProfiles = $state<string[]>([]);
  activeSpringProfiles = $state<string[]>([]);
  buildSkipTests = $state(false);
  buildOffline = $state(false);

  reset(root: string): void {
    this.projectEnv = null;
    this.effectiveEnv = {};
    this.shellEnv = null;
    this.shellEnvLoading = false;
    this.pythonVenvs = [];
    this.dotenvFiles = [];
    this.runConfigs = [];
    this.detectedConfigs = [];
    this.selectedConfigId = null;
    this.envLoading = false;
    this.envError = null;
    this.activeRun = null;
    this.runsHistory = [];

    const bt = loadBuildToolPrefsLocal(root);
    this.buildToolOpen = bt.open;
    this.detectedBuildToolOverride = null;
    this.activeMavenProfiles = bt.mavenProfiles;
    this.activeSpringProfiles = bt.springProfiles;
    this.buildSkipTests = bt.skipTests;
    this.buildOffline = bt.offline;
  }
}

export class IdeDebugState {
  breakpointsByPath = $state<Record<string, IdeBreakpoint[]>>({});
  breakpointsMuted = $state(false);
  breakOnExceptions = $state(false);
  activeDebug = $state<IdeDebugSessionSnapshot | null>(null);
  expandedVariables = $state<Record<number, IdeVariable[]>>({});
  watches = $state<IdeWatch[]>([]);
  replEntries = $state<IdeReplEntry[]>([]);
  debugLoading = $state(false);

  readonly allBreakpoints = $derived(Object.values(this.breakpointsByPath).flat());
  readonly effectiveBreakpoints = $derived(this.breakpointsMuted ? [] : this.allBreakpoints);

  reset(breakpoints: Record<string, IdeBreakpoint[]> = {}): void {
    this.breakpointsByPath = breakpoints;
    this.activeDebug = null;
    this.expandedVariables = {};
    this.replEntries = [];
    this.debugLoading = false;
  }
}

export class IdeCollabState {
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

  readonly canEditCollab = $derived(
    !this.collabIsGuest ||
      Boolean(
        this.collabRoom &&
          (this.collabRoom.peers.find((p) => p.peer_id === this.collabPeerId)?.role ??
            this.collabRoom.default_role) === 'editor',
      ),
  );

  leave(): void {
    this.collabRoom = null;
    this.collabInviteToken = null;
    this.collabShareHash = null;
    this.collabIsGuest = false;
    this.collabPeerId = null;
    this.collabPeerSecret = null;
    this.collabGuestToken = null;
    this.peerCursors = [];
    this.followingPeerId = null;
    this.sharedBuffers = {};
    this.collabLoading = false;
    this.collabError = null;
  }

  joinFailed(errorMsg: string): void {
    this.collabRoom = null;
    this.collabInviteToken = null;
    this.collabShareHash = null;
    this.collabIsGuest = false;
    this.collabPeerId = null;
    this.collabPeerSecret = null;
    this.collabGuestToken = null;
    this.peerCursors = [];
    this.followingPeerId = null;
    this.sharedBuffers = {};
    this.collabLoading = false;
    this.collabError = errorMsg;
  }

  reset(): void {
    this.leave();
  }
}

export type { IdeInstalledSdk };

import { api } from '../../../lib/api/client';
import type { IdeQuickFix } from '../../../lib/api/types/ide-intelligence';
import { loadErrorText } from '../../../lib/loadError';
import { toastError } from '../../../lib/toastError';
import {
  detectFastRunConfigs,
  findContainingRoot,
  loadAttachedRootEntries,
  loadSavedAdditionalRoots,
  loadSavedBreakpoints,
  mergeFastRunConfigs,
  normalizeBreakpointsMap,
  normalizeRootPath,
  restoreWorkspaceTabsOrInitial,
  saveAdditionalRootsLocal,
  type ProjectHelperHost,
} from '../shell/ideProjectHelpers';
import { saveWorkspaceOpenTabs, type IdeBufferPerfController } from './ideBufferPerf';
import {
  ideBasename,
  type IdeCodeAction,
  type IdeDiagnostic,
  type IdeFsTreeResp,
  type IdeGutterItem,
  type IdeNavLocation,
  type IdeOpenTab,
  type IdeProjectRoot,
  type IdeReferenceLocation,
  type IdeReplaceResp,
  type IdeSearchItem,
  type IdeSymbolNode,
  type IdeTargetSpec,
} from './ideTypes';

export interface IdeTreeTabsHost extends ProjectHelperHost {
  collabIsGuest: boolean;
  targetOs: string;
  targetArch: string;
  treeLoading: boolean;
  treeError: string | null;
  runConfigs: IdeRunConfig[];
  detectedConfigs: IdeRunConfig[];
  selectedConfigId: string | null;
  closedTabPaths: string[];
  hasClosedAllTabs: boolean;
  activePath: string | null;
  splitPath: string | null;
  splitDirection: 'right' | 'down' | null;
  gutterByPath: Record<string, IdeGutterItem[]>;
  symbolsByPath: Record<string, IdeSymbolNode[]>;
  problems: IdeDiagnostic[];
  usages: IdeReferenceLocation[];
  searchResults: IdeSearchItem[];
  replacePreview: IdeReplaceResp | null;
  navHistory: IdeNavLocation[];
  navIndex: number;
  collabState: IdeCollabState;
  debugState: IdeDebugState;
  envState: IdeTargetEnvState;
  toggleDir(dirPath: string): Promise<void>;
  openInSplit(path: string, dir?: 'right' | 'down'): void;
  closeSplit(): void;
  syncWorkspacePort(): void;
}

export function resetTargetLifecycleState(
  host: IdeTreeTabsHost,
  next: IdeTargetSpec,
  clearActivePath: () => void,
  overrideAdditionalRoots?: string[],
): void {
  host.collabState.reset();
  host.target = next;
  const savedRoots = overrideAdditionalRoots
    ? overrideAdditionalRoots.map((r) => normalizeRootPath(r)).filter((r) => r && r !== normalizeRootPath(next.root))
    : (next.root ? loadSavedAdditionalRoots(next) : []);
  host.additionalRoots = savedRoots;
  if (overrideAdditionalRoots && next.root) saveAdditionalRootsLocal(next, savedRoots);
  const bps = next.root ? normalizeBreakpointsMap(loadSavedBreakpoints(next), (p) => host.resolvePath(p)) : {};
  host.debugState.reset(bps);
  host.envState.reset(next.root);
  const initialExpanded: Record<string, boolean> = {};
  if (next.root) initialExpanded[normalizeRootPath(next.root)] = true;
  // Scale discipline: keep 30+ secondary repo roots collapsed on cold open.
  if (savedRoots.length <= 2) {
    for (let i = 0; i < savedRoots.length; i++) initialExpanded[savedRoots[i]] = true;
  }
  host.treeRootEntries = [];
  host.childrenByDir = {};
  host.expandedDirs = initialExpanded;
  host.treeError = null;
  host.tabs = [];
  host.closedTabPaths = [];
  host.hasClosedAllTabs = false;
  clearActivePath();
  host.splitPath = null;
  host.splitDirection = null;
  host.gutterByPath = {};
  host.symbolsByPath = {};
  host.problems = [];
  host.usages = [];
  host.searchResults = [];
  host.replacePreview = null;
  host.navHistory = [];
  host.navIndex = -1;
  host.syncWorkspacePort();
}

export function syncAdditionalRootsFromEnvState(
  host: IdeTreeTabsHost,
  envRoots: string[],
  onChanged: () => void,
): void {
  const primary = normalizeRootPath(host.target.root);
  const merged = [...host.additionalRoots];
  let changed = false;
  for (let i = 0; i < envRoots.length; i++) {
    const clean = normalizeRootPath(envRoots[i]);
    if (clean && clean !== primary && !merged.includes(clean)) {
      merged.push(clean);
      changed = true;
    }
  }
  if (!changed) return;
  host.additionalRoots = merged;
  saveAdditionalRootsLocal(host.target, merged);
  if (merged.length <= 2) {
    const nextExp = { ...host.expandedDirs };
    for (let i = 0; i < merged.length; i++) {
      if (nextExp[merged[i]] === undefined) nextExp[merged[i]] = true;
      if (nextExp[merged[i]] && !host.childrenByDir[merged[i]]) void loadAttachedRootEntries(host, merged[i]);
    }
    host.expandedDirs = nextExp;
  }
  host.syncWorkspacePort();
  onChanged();
}

export async function fetchIdeWorkspaceTree(
  host: IdeTreeTabsHost,
  dirPath: string | undefined,
  onProbeBuildTool: () => void,
): Promise<void> {
  if (!host.target.root || host.collabIsGuest) return;
  const primary = normalizeRootPath(host.target.root);
  const normDir = dirPath ? normalizeRootPath(dirPath) : '';
  const isRoot = !normDir || normDir === primary;
  if (!isRoot && host.additionalRoots.includes(normDir)) {
    await loadAttachedRootEntries(host, normDir);
    return;
  }
  const attachedParent = !isRoot ? findContainingRoot(normDir, host.projectRoots) : null;
  if (isRoot) {
    host.treeLoading = true;
    host.treeError = null;
  }
  try {
    const query =
      attachedParent && !attachedParent.isPrimary && host.target.kind === 'local'
        ? new URLSearchParams({ kind: 'local', root: attachedParent.path, path: normDir, show_hidden: String(host.showHidden) }).toString()
        : host.targetQuery({ path: isRoot ? '' : normDir, show_hidden: host.showHidden });
    const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${query}`);
    if (resp.target_os) host.targetOs = resp.target_os;
    if (resp.target_arch) host.targetArch = resp.target_arch;
    if (isRoot) {
      if (resp.root && host.target.root === '~') {
        host.target = { ...host.target, root: resp.root };
        host.expandedDirs = { ...host.expandedDirs, [normalizeRootPath(resp.root)]: true };
      }
      const entries = resp.entries ?? [];
      host.treeRootEntries = entries;
      host.childrenByDir = { ...host.childrenByDir, [normalizeRootPath(host.target.root)]: entries };
      if (host.runConfigs.length === 0) {
        const fast = detectFastRunConfigs(host.target.root, entries);
        host.detectedConfigs = fast;
        host.runConfigs = mergeFastRunConfigs([], fast);
        if (!host.selectedConfigId && host.runConfigs.length > 0) host.selectedConfigId = host.runConfigs[0].id;
      }
      for (let i = 0; i < host.additionalRoots.length; i++) {
        const ar = host.additionalRoots[i];
        if (host.expandedDirs[ar]) void loadAttachedRootEntries(host, ar);
      }
      host.syncWorkspacePort();
      onProbeBuildTool();
      void restoreWorkspaceTabsOrInitial(host);
    } else if (normDir) {
      host.childrenByDir = { ...host.childrenByDir, [normDir]: resp.entries ?? [] };
    }
  } catch (e) {
    if (isRoot) host.treeError = loadErrorText(e);
    else toastError('Couldn’t open folder', e);
  } finally {
    if (isRoot) host.treeLoading = false;
  }
}

export function performCloseTabBatch(
  host: IdeTreeTabsHost,
  bufPerf: IdeBufferPerfController,
  evictClosed: (paths: string[]) => void,
  keepPath?: string,
): void {
  bufPerf.flushAll(host as never);
  const keepPred = (t: IdeOpenTab) =>
    (keepPath !== undefined && t.path === keepPath) || t.pinned || bufPerf.isTabDirty(t);
  const closing = host.tabs.filter((t) => !keepPred(t)).map((t) => t.path);
  host.tabs = host.tabs.filter(keepPred);
  evictClosed(closing);
  if (keepPath !== undefined) {
    host.activePath = keepPath;
    if (host.splitPath && !host.tabs.some((t) => t.path === host.splitPath)) host.closeSplit();
  } else if (host.tabs.length === 0) {
    host.hasClosedAllTabs = true;
    host.activePath = null;
    host.closeSplit();
  } else if (!host.tabs.some((t) => t.path === host.activePath)) {
    host.activePath = host.tabs[0].path;
  }
  saveWorkspaceOpenTabs(host.target.root, host.tabs.map((t) => t.path), host.activePath, host.splitPath);
  host.syncWorkspacePort();
}

export function computeIdeProjectRoots(primaryRoot: string, additionalRoots: readonly string[]): IdeProjectRoot[] {
  if (!primaryRoot) return [];
  const primary = normalizeRootPath(primaryRoot);
  const out: IdeProjectRoot[] = [{ path: primary, name: ideBasename(primary) || primary, isPrimary: true }];
  for (let i = 0; i < additionalRoots.length; i++) {
    const r = normalizeRootPath(additionalRoots[i]);
    if (r && r !== primary && !out.some((x) => x.path === r)) {
      out.push({ path: r, name: ideBasename(r) || r, isPrimary: false });
    }
  }
  return out;
}

export type IncomingIdeDiagnostic = Omit<IdeDiagnostic, 'path' | 'quick_fixes'> & {
  path?: string;
  quick_fixes?: ReadonlyArray<IdeCodeAction | IdeQuickFix>;
};

export function normalizePathDiagnostics(path: string, items: readonly IncomingIdeDiagnostic[]): IdeDiagnostic[] {
  return items.map((d) => ({
    ...d,
    path: d.path || path,
    quick_fixes: d.quick_fixes?.map((qf, idx): IdeCodeAction =>
      'id' in qf
        ? qf
        : { id: `qf-${d.line}-${idx}`, title: qf.title, kind: 'quickfix', is_preferred: qf.is_preferred ?? idx === 0, edits: qf.edits },
    ),
  }));
}
