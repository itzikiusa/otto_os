// Named Multi-Repo IDE Projects manager (GoLand / IntelliJ 10–40+ repo workspaces),
// batch multi-repo attach, and <50ms 1-click project switching with per-project tabs.

import { api } from '../../../lib/api/client';
import { lsGet, lsSet } from '../../../lib/storage';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { loadWorkspaceOpenTabs } from '../store/ideBufferPerf';
import {
  autoOpenInitialEntry,
  ideBasename,
  isReadOnlySdkOrCachePath,
  type IdeFsEntry,
  type IdeFsTreeResp,
  type IdeNamedProject,
  type IdeOpenTab,
  type IdeProjectRoot,
  type IdeRunConfig,
  type IdeTargetSpec,
} from '../store/ideTypes';
import {
  findContainingRoot,
  isPathExternal,
  loadAttachedRootEntries,
  normalizeRootPath,
  type ProjectHelperHost,
  saveAdditionalRootsLocal,
} from './ideProjectHelpers';

const NAMED_PROJECTS_KEY = 'otto.ide.namedProjects.v1';
const ACTIVE_PROJECT_KEY = 'otto.ide.activeProjectId.v1';
const LAST_ROOT_KEY = 'otto.ide.lastRoot';
const PROJECT_TABS_PREFIX = 'otto.ide.projectTabs:';

export function emitIdeProjectsChanged(projects?: IdeNamedProject[], activeProjectId?: string | null, root?: string): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('otto:ide-projects-changed', { detail: { projects, activeProjectId, root } }));
}

export function saveLastIdeRoot(root: string): void {
  if (root) lsSet(LAST_ROOT_KEY, root);
}

export interface NamedProjectsHost extends ProjectHelperHost {
  namedProjects: IdeNamedProject[];
  activeProjectId: string | null;
  activePath: string | null;
  hasClosedAllTabs: boolean;
  setLocalTarget(root: string, workspaceId?: string | null, opts?: { additionalRoots?: string[]; projectId?: string | null; skipAutoOpen?: boolean }): Promise<void>;
}

interface SavedProjectTabs {
  paths: string[];
  activePath: string | null;
  pinnedPaths?: string[];
}

export function loadNamedProjectsLocal(): IdeNamedProject[] {
  try {
    const raw = lsGet(NAMED_PROJECTS_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: IdeNamedProject[] = [];
    for (let i = 0; i < parsed.length; i++) {
      const p = parsed[i];
      if (!p || typeof p.id !== 'string' || typeof p.primary_root !== 'string') continue;
      const primary = normalizeRootPath(p.primary_root);
      if (!primary) continue;
      const rawFolders: string[] = Array.isArray(p.folders) ? p.folders.map((f: unknown) => normalizeRootPath(String(f))).filter(Boolean) : [];
      const folders = [primary, ...rawFolders.filter((f) => f !== primary)];
      out.push({
        id: p.id,
        name: String(p.name || ideBasename(primary) || 'Project').trim(),
        primary_root: primary,
        folders,
        pinned: p.pinned !== false,
        updated_at: typeof p.updated_at === 'number' ? p.updated_at : Date.now(),
      });
    }
    return out;
  } catch {
    return [];
  }
}

export function saveNamedProjectsLocal(projects: IdeNamedProject[], activeProjectId?: string | null): void {
  try {
    lsSet(NAMED_PROJECTS_KEY, JSON.stringify(projects));
  } catch { /* quota */ }
  emitIdeProjectsChanged(projects, activeProjectId);
}

export function loadActiveProjectIdLocal(): string | null {
  return lsGet(ACTIVE_PROJECT_KEY) || null;
}

export function saveActiveProjectIdLocal(id: string | null, projects?: IdeNamedProject[], root?: string): void {
  lsSet(ACTIVE_PROJECT_KEY, id ?? '');
  emitIdeProjectsChanged(projects, id, root);
}

export function snapshotProjectTabs(projectId: string | null, tabs: IdeOpenTab[], activePath: string | null): void {
  if (!projectId) return;
  try {
    const payload: SavedProjectTabs = {
      paths: tabs.slice(0, 24).map((t) => t.path),
      activePath,
      pinnedPaths: tabs.filter((t) => t.pinned).map((t) => t.path),
    };
    lsSet(`${PROJECT_TABS_PREFIX}${projectId}`, JSON.stringify(payload));
  } catch { /* quota */ }
}

export function readProjectTabsSnapshot(projectId: string): SavedProjectTabs | null {
  try {
    const raw = lsGet(`${PROJECT_TABS_PREFIX}${projectId}`);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as SavedProjectTabs;
    return parsed && Array.isArray(parsed.paths) ? parsed : null;
  } catch {
    return null;
  }
}

export async function syncNamedProjectsFromBackend(host: NamedProjectsHost): Promise<void> {
  try {
    const resp = await api.bg.get<{ projects?: IdeNamedProject[] }>('/ide/projects');
    const remote = resp.projects ?? [];
    if (remote.length === 0) return;
    const byId = new Map<string, IdeNamedProject>();
    for (let i = 0; i < host.namedProjects.length; i++) byId.set(host.namedProjects[i].id, host.namedProjects[i]);
    for (let i = 0; i < remote.length; i++) {
      const r = remote[i];
      if (!r?.id || !r?.primary_root) continue;
      const primary = normalizeRootPath(r.primary_root);
      const folders = [primary, ...(r.folders ?? []).map((f) => normalizeRootPath(f)).filter((f) => f && f !== primary)];
      byId.set(r.id, {
        id: r.id,
        name: r.name || ideBasename(primary),
        primary_root: primary,
        folders,
        pinned: r.pinned !== false,
        updated_at: r.updated_at ?? Date.now(),
      });
    }
    const merged = [...byId.values()];
    host.namedProjects = merged;
    saveNamedProjectsLocal(merged);
  } catch {
    /* Optional endpoint; local persistence remains authoritative */
  }
}

export async function upsertNamedProject(
  host: NamedProjectsHost,
  spec: { id?: string; name: string; primary_root: string; folders: string[]; pinned?: boolean },
): Promise<IdeNamedProject | null> {
  const primary = normalizeRootPath(spec.primary_root || spec.folders[0] || '');
  if (!primary) return null;
  const uniqueFolders = [primary];
  for (let i = 0; i < spec.folders.length; i++) {
    const f = normalizeRootPath(spec.folders[i]);
    if (f && !uniqueFolders.includes(f)) uniqueFolders.push(f);
  }
  const id = spec.id || `proj-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  const proj: IdeNamedProject = {
    id,
    name: spec.name.trim() || ideBasename(primary) || 'Project',
    primary_root: primary,
    folders: uniqueFolders,
    pinned: spec.pinned !== false,
    updated_at: Date.now(),
  };
  const idx = host.namedProjects.findIndex((p) => p.id === id);
  const next = idx >= 0
    ? host.namedProjects.map((p) => (p.id === id ? proj : p))
    : [...host.namedProjects, proj];
  host.namedProjects = next;
  saveNamedProjectsLocal(next, host.activeProjectId);
  try {
    await api.post('/ide/projects', proj);
  } catch { /* local state persisted */ }
  return proj;
}

export async function removeNamedProject(host: NamedProjectsHost, projectId: string): Promise<void> {
  const next = host.namedProjects.filter((p) => p.id !== projectId);
  host.namedProjects = next;
  if (host.activeProjectId === projectId) {
    host.activeProjectId = null;
    saveActiveProjectIdLocal(null, next, host.target.root);
  } else {
    saveNamedProjectsLocal(next, host.activeProjectId);
  }
  try {
    await api.del(`/ide/projects/${encodeURIComponent(projectId)}`);
  } catch { /* ignore */ }
}

export async function activateNamedProject(host: NamedProjectsHost, projectId: string): Promise<void> {
  const proj = host.namedProjects.find((p) => p.id === projectId);
  if (!proj) return;
  if (host.activeProjectId) {
    snapshotProjectTabs(host.activeProjectId, host.tabs, host.activePath);
  }
  const extraRoots = proj.folders.filter((f) => f !== proj.primary_root);
  const savedTabs = readProjectTabsSnapshot(proj.id);
  const hasSavedTabs = Boolean(savedTabs && savedTabs.paths.length > 0);
  await host.setLocalTarget(proj.primary_root, null, {
    additionalRoots: extraRoots,
    projectId: proj.id,
    skipAutoOpen: hasSavedTabs,
  });
  if (savedTabs && savedTabs.paths.length > 0) {
    const activeFirst = savedTabs.activePath && savedTabs.paths.includes(savedTabs.activePath)
      ? savedTabs.activePath
      : savedTabs.paths[0];
    if (activeFirst) await host.openFile(activeFirst, { preview: false });
    const rest = savedTabs.paths.filter((p) => p !== activeFirst).slice(0, 7);
    for (let i = 0; i < rest.length; i++) {
      void host.openFile(rest[i], { preview: false });
    }
    if (activeFirst) host.activePath = activeFirst;
  }
}

export async function attachProjectFoldersBatch(
  host: NamedProjectsHost,
  rawFolders: string[],
): Promise<number> {
  const primary = normalizeRootPath(host.target.root);
  const nextRoots = [...host.additionalRoots];
  const added: string[] = [];
  for (let i = 0; i < rawFolders.length; i++) {
    const clean = normalizeRootPath(rawFolders[i]);
    if (!clean || clean === primary || nextRoots.includes(clean)) continue;
    nextRoots.push(clean);
    added.push(clean);
  }
  if (added.length === 0) return 0;
  host.additionalRoots = nextRoots;
  saveAdditionalRootsLocal(host.target, nextRoots);

  // Scale discipline (#71): only auto-expand & eagerly load when <= 3 total repos!
  if (nextRoots.length <= 2) {
    const exp = { ...host.expandedDirs };
    for (let i = 0; i < added.length; i++) {
      exp[added[i]] = true;
      void loadAttachedRootEntries(host, added[i]);
    }
    host.expandedDirs = exp;
  }

  const allRoots: IdeProjectRoot[] = host.projectRoots;
  for (let i = 0; i < host.tabs.length; i++) {
    const t = host.tabs[i];
    t.external = isPathExternal(t.path, allRoots);
    if (!t.external && !isReadOnlySdkOrCachePath(t.path)) t.readOnly = false;
  }

  if (host.activeProjectId) {
    const cur = host.namedProjects.find((p) => p.id === host.activeProjectId);
    if (cur) {
      await upsertNamedProject(host, {
        id: cur.id,
        name: cur.name,
        primary_root: primary,
        folders: [primary, ...nextRoots],
        pinned: cur.pinned,
      });
    }
  }

  try {
    await host.saveEnvironment({ additional_roots: nextRoots });
  } catch { /* saved locally */ }
  toasts.success(`Attached ${added.length} ${added.length === 1 ? 'repository' : 'repositories'} to project`);
  return added.length;
}

export function syncCurrentProjectFolders(
  host: { namedProjects: IdeNamedProject[]; activeProjectId: string | null; target: IdeTargetSpec; additionalRoots: string[] },
): void {
  if (!host.activeProjectId || !host.target.root) return;
  const primary = normalizeRootPath(host.target.root);
  const folders = [primary, ...host.additionalRoots.filter((f) => f && f !== primary)];
  const cur = host.namedProjects.find((p) => p.id === host.activeProjectId);
  if (!cur) return;
  if (cur.folders.length === folders.length && cur.folders.every((f, idx) => f === folders[idx])) return;
  const next = host.namedProjects.map((p) =>
    p.id === host.activeProjectId ? { ...p, primary_root: primary, folders, updated_at: Date.now() } : p,
  );
  host.namedProjects = next;
  saveNamedProjectsLocal(next, host.activeProjectId);
}

export async function performMutateFs(
  host: {
    target: IdeTargetSpec;
    readonly targetPayload: IdeTargetSpec;
    readonly projectRoots: IdeProjectRoot[];
    expandedDirs: Record<string, boolean>;
    tabs: IdeOpenTab[];
    activePath: string | null;
    splitPath: string | null;
    loadTree(dirPath?: string): Promise<void>;
    openFile(rawPath: string, opts?: { preview?: boolean }): Promise<void>;
    closeSplit(): void;
  },
  flushBuf: (path: string) => void,
  evict: (paths: string[], clearDirty?: boolean) => void,
  setActiveRaw: (p: string | null) => void,
  op: 'mkdir' | 'create_file' | 'rename' | 'delete' | 'duplicate',
  path: string,
  destPath?: string,
): Promise<boolean> {
  try {
    const containing = findContainingRoot(path, host.projectRoots);
    const targetSpec = containing && !containing.isPrimary && host.target.kind === 'local'
      ? { ...host.targetPayload, root: containing.path }
      : host.targetPayload;
    const resp = await api.post<{ ok: boolean; path: string }>('/ide/fs/mutate', { target: targetSpec, op, path, dest_path: destPath });
    const parentOf = (p: string) => (p.lastIndexOf('/') > 0 ? p.slice(0, p.lastIndexOf('/')) : (containing?.path ?? host.target.root));
    await host.loadTree(containing && !containing.isPrimary ? containing.path : undefined);
    for (const dir of Object.keys(host.expandedDirs)) {
      if (host.expandedDirs[dir] && (dir === parentOf(path) || (destPath && dir === parentOf(destPath)))) await host.loadTree(dir);
    }
    if (op === 'delete') {
      const deleted = host.tabs.filter((t) => t.path === path || t.path.startsWith(`${path}/`)).map((t) => t.path);
      host.tabs = host.tabs.filter((t) => t.path !== path && !t.path.startsWith(`${path}/`));
      evict(deleted);
      if (host.activePath && deleted.includes(host.activePath)) host.activePath = host.tabs[0]?.path ?? null;
      if (host.splitPath && deleted.includes(host.splitPath)) host.closeSplit();
    } else if (op === 'rename' && destPath) {
      flushBuf(path);
      evict([path], false);
      host.tabs = host.tabs.map((t) => (t.path === path ? { ...t, path: destPath, name: ideBasename(destPath) } : t));
      if (host.activePath === path) setActiveRaw(destPath);
      if (host.splitPath === path) host.splitPath = destPath;
    } else if ((op === 'create_file' || op === 'duplicate') && resp.path) {
      await host.openFile(resp.path, { preview: false });
    }
    return true;
  } catch (e) {
    toastError(`Couldn’t ${op.replace('_', ' ')}`, e);
    return false;
  }
}

const BUILD_TOOL_PREFS_PREFIX = 'otto.ide.buildToolPrefs:';

export interface WorkspaceBuildToolPrefs {
  open: boolean;
  mavenProfiles: string[];
  springProfiles: string[];
  skipTests: boolean;
  offline: boolean;
}

export function loadBuildToolPrefsLocal(root: string): WorkspaceBuildToolPrefs {
  const empty: WorkspaceBuildToolPrefs = { open: false, mavenProfiles: [], springProfiles: [], skipTests: false, offline: false };
  if (!root) return empty;
  try {
    const raw = lsGet(`${BUILD_TOOL_PREFS_PREFIX}${normalizeRootPath(root)}`);
    if (!raw) return empty;
    const p = JSON.parse(raw) as Partial<WorkspaceBuildToolPrefs>;
    return {
      open: Boolean(p.open),
      mavenProfiles: Array.isArray(p.mavenProfiles) ? p.mavenProfiles : [],
      springProfiles: Array.isArray(p.springProfiles) ? p.springProfiles : [],
      skipTests: Boolean(p.skipTests),
      offline: Boolean(p.offline),
    };
  } catch {
    return empty;
  }
}

export function saveBuildToolPrefsLocal(root: string, prefs: WorkspaceBuildToolPrefs): void {
  if (!root) return;
  lsSet(`${BUILD_TOOL_PREFS_PREFIX}${normalizeRootPath(root)}`, JSON.stringify(prefs));
}

const GRADLE_FILES = new Set(['build.gradle', 'build.gradle.kts', 'settings.gradle', 'settings.gradle.kts', 'gradlew']);

export function detectBuildToolFromState(
  override: 'maven' | 'gradle' | null,
  rootEntries: IdeFsEntry[],
  childrenByDir: Record<string, IdeFsEntry[]>,
  tabs: IdeOpenTab[],
  runConfigs: IdeRunConfig[],
): 'maven' | 'gradle' | null {
  if (override) return override;
  const checkEntry = (name: string): 'maven' | 'gradle' | null => {
    const lower = name.toLowerCase();
    if (lower === 'pom.xml') return 'maven';
    if (GRADLE_FILES.has(lower)) return 'gradle';
    return null;
  };
  for (let i = 0; i < rootEntries.length; i++) {
    const k = checkEntry(rootEntries[i].name);
    if (k) return k;
  }
  for (const entries of Object.values(childrenByDir)) {
    for (let i = 0; i < entries.length; i++) {
      const k = checkEntry(entries[i].name);
      if (k) return k;
    }
  }
  for (let i = 0; i < tabs.length; i++) {
    const k = checkEntry(tabs[i].name);
    if (k) return k;
  }
  for (let i = 0; i < runConfigs.length; i++) {
    const r = runConfigs[i].runner;
    if (r === 'maven' || r === 'java_maven') return 'maven';
    if (r === 'gradle' || r === 'java_gradle') return 'gradle';
  }
  return null;
}

export async function probeWorkspaceBuildTool(host: {
  target: IdeTargetSpec;
  readonly targetPayload: IdeTargetSpec;
  readonly projectRoots: IdeProjectRoot[];
  additionalRoots: string[];
  childrenByDir: Record<string, IdeFsEntry[]>;
  detectedBuildToolOverride: 'maven' | 'gradle' | null;
}): Promise<void> {
  const startRoot = host.target.root;
  if (!startRoot) return;
  try {
    const resp = await api.bg.post<{ tool?: 'maven' | 'gradle' | null; modules?: { tool: 'maven' | 'gradle' }[] }>(
      '/ide/build-tools/model',
      { target: host.targetPayload, roots: host.projectRoots.map((r) => r.path) },
    );
    if (host.target.root !== startRoot) return;
    const detected = resp.modules?.[0]?.tool ?? resp.tool ?? null;
    if (detected === 'maven' || detected === 'gradle') {
      host.detectedBuildToolOverride = detected;
      return;
    }
  } catch { /* fallback to shallow FS probe on secondary roots */ }
  if (host.target.kind !== 'local' || host.additionalRoots.length === 0) return;
  const unprobed = host.additionalRoots.filter((r) => !host.childrenByDir[r]).slice(0, 8);
  for (let i = 0; i < unprobed.length; i++) {
    if (host.target.root !== startRoot) return;
    try {
      const tr = await api.bg.get<IdeFsTreeResp>(
        `/ide/fs/tree?${new URLSearchParams({ kind: 'local', root: unprobed[i], path: '', show_hidden: 'false' }).toString()}`,
      );
      const names = (tr.entries ?? []).map((e) => e.name.toLowerCase());
      if (names.includes('pom.xml')) { host.detectedBuildToolOverride = 'maven'; return; }
      if (names.some((n) => GRADLE_FILES.has(n))) { host.detectedBuildToolOverride = 'gradle'; return; }
    } catch { /* ignore */ }
  }
}

export async function restoreWorkspaceTabsOrInitial(host: {
  target: IdeTargetSpec;
  collabIsGuest: boolean;
  tabs: IdeOpenTab[];
  hasClosedAllTabs: boolean;
  activePath: string | null;
  treeRootEntries: IdeFsEntry[];
  childrenByDir: Record<string, IdeFsEntry[]>;
  toggleDir(dirPath: string): Promise<void>;
  openFile(rawPath: string, opts?: { preview?: boolean }): Promise<void>;
  openInSplit(path: string, dir?: 'right' | 'down'): void;
}): Promise<void> {
  if (host.collabIsGuest || host.tabs.length > 0 || host.hasClosedAllTabs) return;
  if (typeof window !== 'undefined' && /[?&](file|collab)=/.test(window.location.hash)) return;
  const saved = loadWorkspaceOpenTabs(host.target.root);
  if (saved && saved.paths.length > 0) {
    for (let i = 0; i < saved.paths.length; i++) {
      await host.openFile(saved.paths[i], { preview: false });
    }
    const validTabs = host.tabs.filter((t) => !t.error);
    if (validTabs.length > 0) {
      if (saved.splitPath && validTabs.some((t) => t.path === saved.splitPath)) host.openInSplit(saved.splitPath, 'right');
      if (saved.activePath && validTabs.some((t) => t.path === saved.activePath)) host.activePath = saved.activePath;
      return;
    }
    host.tabs = [];
  }
  await autoOpenInitialEntry(host);
}
