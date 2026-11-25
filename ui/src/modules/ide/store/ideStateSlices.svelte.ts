// Cohesive reactive sub-stores for `IdeStore`:
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

export interface IdeRunLaunchMemo {
  configId?: string;
  adhoc?: Partial<IdeRunConfig>;
  keepTestcontainers?: boolean;
  coverage?: boolean;
}

export interface IdeDebugLaunchMemo {
  configId?: string;
  language?: string;
  launch?: {
    program: string;
    args?: string[];
    build_flags?: string[];
    cwd?: string;
    test_name?: string;
    attach_port?: number;
    env?: Record<string, string>;
    env_file?: string | null;
    inherit_shell_env?: boolean;
    sdk_path?: string | null;
  };
}

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
  shellEnvAttempted = $state(false);
  shellEnvError = $state<string | null>(null);
  pythonVenvs = $state<IdePythonVenv[]>([]);
  dotenvFiles = $state<string[]>([]);
  runConfigs = $state<IdeRunConfig[]>([]);
  detectedConfigs = $state<IdeRunConfig[]>([]);
  selectedConfigId = $state<string | null>(null);
  /** The test / main last launched from the gutter; selected as `GUTTER_TARGET_ID`. */
  gutterTarget = $state<IdeGutterItem | null>(null);
  envLoading = $state(false);
  envError = $state<string | null>(null);
  activeRun = $state<IdeRunState | null>(null);
  runsHistory = $state<IdeRunState[]>([]);
  lastRunLaunch: IdeRunLaunchMemo | null = null;
  runLaunchById: Record<string, IdeRunLaunchMemo> = {};

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
    this.shellEnvAttempted = false;
    this.shellEnvError = null;
    this.pythonVenvs = [];
    this.dotenvFiles = [];
    this.runConfigs = [];
    this.detectedConfigs = [];
    this.selectedConfigId = null;
    this.gutterTarget = null;
    this.envLoading = false;
    this.envError = null;
    this.activeRun = null;
    this.runsHistory = [];
    this.lastRunLaunch = null;
    this.runLaunchById = {};

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
  debugStreamActive = $state(false);
  lastDebugLaunch = $state<IdeDebugLaunchMemo | null>(null);

  readonly allBreakpoints = $derived(Object.values(this.breakpointsByPath).flat());
  readonly effectiveBreakpoints = $derived(this.breakpointsMuted ? [] : this.allBreakpoints);
  readonly canRestart = $derived(Boolean(this.activeDebug || this.lastDebugLaunch));

  reset(breakpoints: Record<string, IdeBreakpoint[]> = {}): void {
    this.breakpointsByPath = breakpoints;
    this.activeDebug = null;
    this.expandedVariables = {};
    this.replEntries = [];
    this.debugLoading = false;
    this.debugStreamActive = false;
    this.lastDebugLaunch = null;
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
import {
  canonicalizeLocalProjectPath,
  canonicalRootKey,
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
import { leaveDebugSessionOnSwitch } from './ideDebugLaunch.svelte';
import { disambiguateLabels } from '../shell/ideProjectRecents.svelte';
import { ideProjectHealth } from './ideProjectHealth.svelte';
import { enterIdeTarget, leaveIdeTarget } from './ideProjectSwitch';
import { ideTreeDirErrors } from './ideTreeDirErrors.svelte';
import {
  ideBasename,
  ideTargetKey,
  type IdeCodeAction,
  type IdeDiagnostic,
  type IdeFsEntry,
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

const targetResetHooks: Array<() => void> = [];

export function onIdeTargetReset(fn: () => void): void {
  targetResetHooks.push(fn);
}

export function resetTargetLifecycleState(
  host: IdeTreeTabsHost,
  next: IdeTargetSpec,
  clearActivePath: () => void,
  overrideAdditionalRoots?: string[],
): void {
  // A session still live in the project being left is stopped on the daemon (best-effort).
  if (ideTargetKey(next) !== ideTargetKey(host.target)) leaveDebugSessionOnSwitch(host.debugState.activeDebug, host.collabState.collabIsGuest);
  // Remember the tree, park a live run, let the agent save its thread (I3-03 / I3-08).
  leaveIdeTarget(host);
  host.collabState.reset();
  for (let i = 0; i < targetResetHooks.length; i++) targetResetHooks[i]();
  host.target = next;
  const primaryKey = canonicalRootKey(next.root);
  const rawSaved = overrideAdditionalRoots
    ? overrideAdditionalRoots.map((r) => normalizeRootPath(r)).filter(Boolean)
    : (next.root ? loadSavedAdditionalRoots(next) : []);
  const seenKeys = new Set<string>(primaryKey ? [primaryKey] : []);
  const savedRoots: string[] = [];
  for (let i = 0; i < rawSaved.length; i++) {
    const k = canonicalRootKey(rawSaved[i]);
    if (k && !seenKeys.has(k)) {
      seenKeys.add(k);
      savedRoots.push(rawSaved[i]);
    }
  }
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
  enterIdeTarget(host);
}

export function syncAdditionalRootsFromEnvState(
  host: IdeTreeTabsHost,
  envRoots: string[],
  onChanged: () => void,
): void {
  const primary = normalizeRootPath(host.target.root);
  const seenKeys = new Set<string>(primary ? [canonicalRootKey(primary)] : []);
  const merged: string[] = [];
  for (let i = 0; i < host.additionalRoots.length; i++) {
    const k = canonicalRootKey(host.additionalRoots[i]);
    if (k && !seenKeys.has(k)) {
      seenKeys.add(k);
      merged.push(host.additionalRoots[i]);
    }
  }
  let changed = merged.length !== host.additionalRoots.length;
  for (let i = 0; i < envRoots.length; i++) {
    const clean = normalizeRootPath(envRoots[i]);
    const k = canonicalRootKey(clean);
    if (clean && k && !seenKeys.has(k)) {
      seenKeys.add(k);
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

/** Most folders one tree read lists (`paths`), as the daemon caps it. */
export const MAX_TREE_PATHS = 200;

/** Same entries in the same order: a live re-read that found nothing new changes nothing. */
function sameListing(a: readonly IdeFsEntry[] | undefined, b: readonly IdeFsEntry[]): boolean {
  return a !== undefined && a.length === b.length && JSON.stringify(a) === JSON.stringify(b);
}

export async function fetchIdeWorkspaceTree(
  host: IdeTreeTabsHost,
  dirPath: string | undefined,
  onProbeBuildTool: () => void,
  // `listingOnly`: a live refresh (ideTreeLive) — re-read this one listing in place, nothing else
  // (`paths`: and these folders of the primary root, in the same call).
  opts: { deferTabRestore?: boolean; listingOnly?: boolean; paths?: string[] } = {},
): Promise<void> {
  if (!host.target.root || host.collabIsGuest) return;
  const live = opts.listingOnly === true;
  const primary = normalizeRootPath(host.target.root);
  const normDir = dirPath ? normalizeRootPath(dirPath) : '';
  const isRoot = !normDir || normDir === primary;
  if (!isRoot && host.additionalRoots.includes(normDir)) {
    await loadAttachedRootEntries(host, normDir);
    return;
  }
  const attachedParent = !isRoot ? findContainingRoot(normDir, host.projectRoots) : null;
  // Two project switches can overlap (pills, switcher, `?root=`): a tree that answers after
  // the target changed belongs to the old project and must not replace the new one.
  const key = ideTargetKey(host.target);
  const stale = (): boolean => ideTargetKey(host.target) !== key;
  if (isRoot && !live) {
    host.treeLoading = true;
    host.treeError = null;
    ideTreeDirErrors.reset();
  }
  // A root load reads the open folders of the primary root in the same call: the tree comes
  // back whole in one paint, not level by level.
  const open = !isRoot ? [] : live ? (opts.paths ?? []) : Object.keys(host.expandedDirs).filter((d) => host.expandedDirs[d] && d.startsWith(`${primary}/`));
  // Sent relative to the root (a short URL) and at most MAX_TREE_PATHS: the rest is read on its own.
  const sent = open.slice(0, MAX_TREE_PATHS);
  const abs = (rel: string): string => `${primary}/${rel}`;
  const canon = (raw: IdeFsEntry[] = []): IdeFsEntry[] =>
    host.target.kind === 'local' ? raw.map((e) => ({ ...e, path: canonicalizeLocalProjectPath(e.path, host.projectRoots) })) : raw;
  // An unchanged listing keeps its array: a refresh of what is shown repaints nothing.
  const same = (prev: IdeFsEntry[] | undefined, next: IdeFsEntry[]): IdeFsEntry[] => (sameListing(prev, next) ? prev! : next);
  try {
    let query =
      attachedParent && !attachedParent.isPrimary && host.target.kind === 'local'
        ? new URLSearchParams({ kind: 'local', root: attachedParent.path, path: normDir, show_hidden: String(host.showHidden) }).toString()
        : host.targetQuery({ path: isRoot ? '' : normDir, show_hidden: host.showHidden });
    if (sent.length > 0) query += `&paths=${encodeURIComponent(JSON.stringify(sent.map((d) => d.slice(primary.length + 1))))}`;
    const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${query}`);
    if (stale()) return;
    if (resp.target_os) host.targetOs = resp.target_os;
    if (resp.target_arch) host.targetArch = resp.target_arch;
    if (isRoot) {
      if (resp.root && host.target.root === '~') {
        host.target = { ...host.target, root: resp.root };
        host.expandedDirs = { ...host.expandedDirs, [normalizeRootPath(resp.root)]: true };
      }
      const entries = canon(resp.entries);
      const rootKey = normalizeRootPath(host.target.root);
      host.treeRootEntries = same(host.treeRootEntries, entries);
      if (live) {
        const next = { ...host.childrenByDir, [rootKey]: host.treeRootEntries };
        for (const l of resp.listings ?? []) next[abs(l.path)] = same(next[abs(l.path)], canon(l.entries));
        if (Object.keys(next).some((d) => next[d] !== host.childrenByDir[d])) host.childrenByDir = next;
        // Folders the call did not answer (an older daemon, past the cap, one that failed) are read on their own.
        const answered = new Set((resp.listings ?? []).map((l) => abs(l.path)));
        await Promise.all(open.filter((d) => !answered.has(d)).map((d) => fetchIdeWorkspaceTree(host, d, () => {}, { listingOnly: true })));
        return;
      }
      ideProjectHealth.clear();
      // A root load is a refresh: expanded folders refetch (the primary root's came with it),
      // and collapsed ones drop their cached listing so the next expand reads the disk.
      const kept: typeof host.childrenByDir = { [rootKey]: host.treeRootEntries };
      for (const l of resp.listings ?? []) kept[abs(l.path)] = same(host.childrenByDir[abs(l.path)], canon(l.entries));
      for (const [dir, children] of Object.entries(host.childrenByDir)) {
        if (dir === rootKey || !host.expandedDirs[dir] || kept[dir]) continue;
        kept[dir] = children;
        if (!host.additionalRoots.includes(dir)) void fetchIdeWorkspaceTree(host, dir, onProbeBuildTool);
      }
      host.childrenByDir = kept;
      if (host.runConfigs.length === 0) {
        const fast = detectFastRunConfigs(host.target.root, entries);
        host.detectedConfigs = fast;
        host.runConfigs = mergeFastRunConfigs([], fast); // nothing auto-selected: "Current file" is the default
      }
      for (let i = 0; i < host.additionalRoots.length; i++) {
        const ar = host.additionalRoots[i];
        if (host.expandedDirs[ar]) void loadAttachedRootEntries(host, ar);
      }
      host.syncWorkspacePort();
      onProbeBuildTool();
      if (!opts.deferTabRestore) void restoreWorkspaceTabsOrInitial(host);
    } else if (normDir) {
      const entries = canon(resp.entries);
      if (live && sameListing(host.childrenByDir[normDir], entries)) return;
      host.childrenByDir = { ...host.childrenByDir, [normDir]: entries };
      ideTreeDirErrors.clear(normDir);
    }
  } catch (e) {
    // A live re-read that fails (folder just deleted, daemon restarting) keeps what is shown.
    if (stale() || live) return;
    if (isRoot) {
      host.treeError = loadErrorText(e);
      ideProjectHealth.noteRootError(host.target, e, host.treeError); // a deleted folder / host down (I3-12)
    } else if (normDir) ideTreeDirErrors.set(normDir, loadErrorText(e));
  } finally {
    if (isRoot && !live && !stale()) host.treeLoading = false;
  }
}

const dirLoads = new Map<string, Promise<void>>();

/** `fetchIdeWorkspaceTree` with one listing of a folder at a time: an expand, a reveal and the
 *  tree's lazy load of a folder on screen share it. */
export function loadIdeWorkspaceTree(host: IdeTreeTabsHost, dirPath: string | undefined, onProbeBuildTool: () => void): Promise<void> {
  if (!dirPath) return fetchIdeWorkspaceTree(host, undefined, onProbeBuildTool);
  const key = `${ideTargetKey(host.target)}\u0000${normalizeRootPath(dirPath)}`;
  const cur = dirLoads.get(key);
  if (cur) return cur;
  const p = fetchIdeWorkspaceTree(host, dirPath, onProbeBuildTool).finally(() => dirLoads.delete(key));
  dirLoads.set(key, p);
  return p;
}

let relistBatch: { dirs: Set<string>; done: Promise<void> } | null = null;

/** A live re-read of `dir` (ideTreeLive). The ones asked for in the same tick go out together:
 *  the primary root's folders ride on one root listing, an attached root's are read on their own. */
export function relistIdeTreeDir(host: IdeTreeTabsHost, dir: string): Promise<void> {
  if (!relistBatch) {
    const batch: { dirs: Set<string>; done: Promise<void> } = { dirs: new Set(), done: Promise.resolve() };
    batch.done = Promise.resolve().then(async () => {
      relistBatch = null;
      const primary = normalizeRootPath(host.target.root);
      const dirs = [...batch.dirs];
      const own = dirs.filter((d) => d === primary || d.startsWith(`${primary}/`));
      await Promise.all([
        own.length > 0 ? fetchIdeWorkspaceTree(host, undefined, () => {}, { listingOnly: true, paths: own.filter((d) => d !== primary) }) : null,
        ...dirs.filter((d) => !own.includes(d)).map((d) => fetchIdeWorkspaceTree(host, d, () => {}, { listingOnly: true })),
      ]);
    });
    relistBatch = batch;
  }
  relistBatch.dirs.add(normalizeRootPath(dir));
  return relistBatch.done;
}

/** The open folders after `dir` is toggled to `open`: collapsing folds everything under it,
 *  so opening it again shows its direct children only. */
export function toggledExpansion(expanded: Record<string, boolean>, dir: string, open: boolean): Record<string, boolean> {
  const next = { ...expanded, [dir]: open };
  if (!open) for (const d of Object.keys(next)) if (d.startsWith(`${dir}/`)) delete next[d];
  return next;
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
  saveWorkspaceOpenTabs(host);
  host.syncWorkspacePort();
}

export function computeIdeProjectRoots(primaryRoot: string, additionalRoots: readonly string[]): IdeProjectRoot[] {
  if (!primaryRoot) return [];
  const primary = normalizeRootPath(primaryRoot);
  const seenKeys = new Set<string>([canonicalRootKey(primary)]);
  const out: IdeProjectRoot[] = [{ path: primary, name: ideBasename(primary) || primary, isPrimary: true }];
  for (let i = 0; i < additionalRoots.length; i++) {
    const r = normalizeRootPath(additionalRoots[i]);
    const k = canonicalRootKey(r);
    if (r && k && !seenKeys.has(k)) {
      seenKeys.add(k);
      out.push({ path: r, name: ideBasename(r) || r, isPrimary: false });
    }
  }
  // Two repos named `api` in one project read "api — team-a" / "api — team-b" (I3-07).
  const labels = disambiguateLabels(out);
  return out.map((r, i) => (labels[i] === r.name ? r : { ...r, name: labels[i] }));
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
