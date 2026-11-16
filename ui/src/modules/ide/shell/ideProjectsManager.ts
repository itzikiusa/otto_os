// Named Multi-Repo IDE Projects manager (GoLand / IntelliJ 10–40+ repo workspaces),
// batch multi-repo attach, and <50ms 1-click project switching with per-project tabs.

import { api } from '../../../lib/api/client';
import {
  readIdeUndoDraft,
  removeIdeUndoDraft,
  writeIdeUndoDraftImmediate,
} from '../../../lib/components/cm-ide-drafts';
import { readIdePreferences } from '../../../lib/components/cm-ide-prefs';
import {
  fetchNamedIdeProjectsFromServer,
  parseNamedIdeProject,
} from '../../../lib/ideProjectsModel';
import { lsGet, lsRemove, lsSet } from '../../../lib/storage';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import {
  ideBasename,
  isReadOnlySdkOrCachePath,
  type IdeFsEntry,
  type IdeFsTreeResp,
  type IdeNamedProject,
  type IdeOpenTab,
  type IdeProjectRoot,
  type IdeTargetSpec,
} from '../store/ideTypes';
import {
  canonicalRootKey,
  detectBuildToolFromState,
  emitIdeProjectsChanged,
  findContainingRoot,
  isPathExternal,
  loadActiveProjectIdLocal,
  loadAttachedRootEntries,
  loadBuildToolPrefsLocal,
  loadNamedProjectsLocal,
  normalizeRootPath,
  type ProjectHelperHost,
  restoreWorkspaceTabsOrInitial,
  saveActiveProjectIdLocal,
  saveAdditionalRootsLocal,
  saveBuildToolPrefsLocal,
  saveLastIdeRoot,
  saveNamedProjectsLocal,
  toIdeNamedProject,
  type WorkspaceBuildToolPrefs,
} from './ideProjectHelpers';

export {
  detectBuildToolFromState,
  emitIdeProjectsChanged,
  loadActiveProjectIdLocal,
  loadBuildToolPrefsLocal,
  loadNamedProjectsLocal,
  restoreWorkspaceTabsOrInitial,
  saveActiveProjectIdLocal,
  saveBuildToolPrefsLocal,
  saveLastIdeRoot,
  saveNamedProjectsLocal,
  type WorkspaceBuildToolPrefs,
};

const PROJECT_TABS_PREFIX = 'otto.ide.projectTabs:';
const GRADLE_FILES = new Set(['build.gradle', 'build.gradle.kts', 'settings.gradle', 'settings.gradle.kts', 'gradlew']);

export interface NamedProjectsHost extends ProjectHelperHost {
  namedProjects: IdeNamedProject[];
  activeProjectId: string | null;
  activePath: string | null;
  hasClosedAllTabs: boolean;
  confirmDiscardUnsaved(): Promise<boolean>;
  setLocalTarget(
    root: string,
    workspaceId?: string | null,
    opts?: { additionalRoots?: string[]; projectId?: string | null; skipAutoOpen?: boolean; discardConfirmed?: boolean },
  ): Promise<boolean>;
}

interface SavedProjectTabs {
  paths: string[];
  activePath: string | null;
  pinnedPaths?: string[];
}

export function snapshotProjectTabs(projectId: string | null, tabs: IdeOpenTab[], activePath: string | null): void {
  if (!projectId) return;
  try {
    const cap = Math.max(15, Math.min(40, readIdePreferences().maxOpenTabs || 20));
    const payload: SavedProjectTabs = {
      paths: tabs.slice(0, cap).map((t) => t.path),
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
    const serverList = await fetchNamedIdeProjectsFromServer();
    if (!serverList) return;
    const merged = serverList.map(toIdeNamedProject);
    if (JSON.stringify(host.namedProjects) !== JSON.stringify(merged)) {
      host.namedProjects = merged;
    }
    if (host.activeProjectId && !merged.some((p) => p.id === host.activeProjectId)) {
      host.activeProjectId = null;
      saveActiveProjectIdLocal(null, merged, host.target.root);
    }
  } catch {
    /* Optional endpoint; local persistence remains authoritative when unreachable */
  }
}

export async function upsertNamedProject(
  host: NamedProjectsHost,
  spec: { id?: string; name: string; primary_root: string; folders: string[]; pinned?: boolean },
): Promise<IdeNamedProject | null> {
  const primary = normalizeRootPath(spec.primary_root || spec.folders[0] || '');
  if (!primary) return null;
  const seenKeys = new Set<string>([canonicalRootKey(primary)]);
  const uniqueFolders = [primary];
  for (let i = 0; i < spec.folders.length; i++) {
    const f = normalizeRootPath(spec.folders[i]);
    const k = canonicalRootKey(f);
    if (f && k && !seenKeys.has(k)) {
      seenKeys.add(k);
      uniqueFolders.push(f);
    }
  }
  const id = spec.id || `proj-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
  const draft: IdeNamedProject = {
    id,
    name: spec.name.trim() || ideBasename(primary) || 'Project',
    primary_root: primary,
    folders: uniqueFolders,
    pinned: spec.pinned !== false,
    updated_at: Date.now(),
  };

  let saved = draft;
  try {
    const resp = await api.post<{ project?: Record<string, unknown> }>('/ide/projects', {
      id: draft.id,
      name: draft.name,
      primary_root: draft.primary_root,
      folders: draft.folders,
      pinned: draft.pinned,
    });
    const parsed = resp?.project ? parseNamedIdeProject(resp.project) : null;
    if (parsed) saved = toIdeNamedProject(parsed);
  } catch (e) {
    toastError('Couldn’t save project', e);
    return null;
  }

  const idx = host.namedProjects.findIndex((p) => p.id === saved.id);
  const next = idx >= 0
    ? host.namedProjects.map((p) => (p.id === saved.id ? saved : p))
    : [...host.namedProjects, saved];
  host.namedProjects = next;
  saveNamedProjectsLocal(next, host.activeProjectId);
  return saved;
}

export async function removeNamedProject(host: NamedProjectsHost, projectId: string): Promise<void> {
  const removed = host.namedProjects.find((p) => p.id === projectId);
  try {
    await api.del(`/ide/projects/${encodeURIComponent(projectId)}`);
  } catch (e) {
    toastError('Couldn’t delete project', e);
    return;
  }
  lsRemove(`${PROJECT_TABS_PREFIX}${projectId}`);
  const next = host.namedProjects.filter((p) => p.id !== projectId);
  host.namedProjects = next;
  if (host.activeProjectId === projectId) {
    host.activeProjectId = null;
    saveActiveProjectIdLocal(null, next, host.target.root);
  }
  saveNamedProjectsLocal(next, host.activeProjectId);
  if (removed && removed.primary_root !== normalizeRootPath(host.target.root)) {
    void import('../../../lib/components/cm-ide-lsp')
      .then((m) => {
        for (let i = 0; i < removed.folders.length; i++) {
          m.disposePooledLspClients({ rootPath: removed.folders[i], forceAll: true });
        }
      })
      .catch(() => {});
  }
}

export async function activateNamedProject(host: NamedProjectsHost, projectId: string): Promise<void> {
  const proj = host.namedProjects.find((p) => p.id === projectId);
  if (!proj) return;
  const primaryKey = canonicalRootKey(proj.primary_root);
  const extraRoots = proj.folders.filter((f) => canonicalRootKey(f) !== primaryKey);
  if (
    host.activeProjectId === proj.id &&
    host.target.kind === 'local' &&
    canonicalRootKey(host.target.root) === primaryKey &&
    host.additionalRoots.length === extraRoots.length &&
    extraRoots.every((r, idx) => canonicalRootKey(host.additionalRoots[idx]) === canonicalRootKey(r))
  ) {
    return;
  }
  if (!(await host.confirmDiscardUnsaved())) return;
  if (host.activeProjectId) {
    snapshotProjectTabs(host.activeProjectId, host.tabs, host.activePath);
  }
  void import('../../../lib/components/cm-ide-lsp')
    .then((m) => m.disposePooledLspClients({ forceAll: true }))
    .catch(() => {});
  const savedTabs = readProjectTabsSnapshot(proj.id);
  const cap = Math.max(15, Math.min(40, readIdePreferences().maxOpenTabs || 20));
  const boundedPaths = savedTabs?.paths.slice(0, cap) ?? [];
  const pinnedSet = new Set(savedTabs?.pinnedPaths ?? []);
  const hasSavedTabs = boundedPaths.length > 0;
  await host.setLocalTarget(proj.primary_root, null, {
    additionalRoots: extraRoots,
    projectId: proj.id,
    skipAutoOpen: hasSavedTabs,
    discardConfirmed: true,
  });
  if (hasSavedTabs) {
    const activeFirst = savedTabs?.activePath && boundedPaths.includes(savedTabs.activePath)
      ? savedTabs.activePath
      : boundedPaths[0];
    if (activeFirst) {
      await host.openFile(activeFirst, { preview: false });
      const activeTab = host.tabs.find((t) => t.path === host.resolvePath(activeFirst));
      if (activeTab && pinnedSet.has(activeTab.path)) activeTab.pinned = true;
    }
    const rest = boundedPaths.filter((p) => p !== activeFirst).slice(0, cap - 1);
    for (let i = 0; i < rest.length; i++) {
      void host.openFile(rest[i], { preview: false, background: true });
      const bgTab = host.tabs.find((t) => t.path === host.resolvePath(rest[i]));
      if (bgTab && pinnedSet.has(bgTab.path)) bgTab.pinned = true;
    }
    if (activeFirst) host.activePath = host.resolvePath(activeFirst);
  }
}

export async function attachProjectFoldersBatch(
  host: NamedProjectsHost,
  rawFolders: string[],
  opts: { quiet?: boolean } = {},
): Promise<number> {
  const primary = normalizeRootPath(host.target.root);
  const seenKeys = new Set<string>([
    canonicalRootKey(primary),
    ...host.additionalRoots.map((r) => canonicalRootKey(r)),
  ]);
  const nextRoots = [...host.additionalRoots];
  const added: string[] = [];
  for (let i = 0; i < rawFolders.length; i++) {
    const clean = normalizeRootPath(rawFolders[i]);
    const k = canonicalRootKey(clean);
    if (!clean || !k || seenKeys.has(k)) continue;
    seenKeys.add(k);
    nextRoots.push(clean);
    added.push(clean);
  }
  if (added.length === 0) return 0;

  try {
    await api.post<{ additional_roots: string[] }>('/ide/projects/folders/bulk', {
      target: host.targetPayload,
      folder_paths: added,
    });
  } catch (e) {
    toastError('Couldn’t attach project folders', e);
    return 0;
  }

  if (host.activeProjectId) {
    const cur = host.namedProjects.find((p) => p.id === host.activeProjectId);
    if (cur) {
      const updated = await upsertNamedProject(host, {
        id: cur.id,
        name: cur.name,
        primary_root: primary,
        folders: [primary, ...nextRoots],
        pinned: cur.pinned,
      });
      if (!updated) return 0;
    }
  }

  host.additionalRoots = nextRoots;
  saveAdditionalRootsLocal(host.target, nextRoots);

  // Scale discipline: only auto-expand & eagerly load when <= 3 total repos.
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
    if (!t.external && !t.truncated && !isReadOnlySdkOrCachePath(t.path)) t.readOnly = false;
  }

  if (!opts.quiet) {
    toasts.success(`Attached ${added.length} ${added.length === 1 ? 'repository' : 'repositories'} to project`);
  }
  return added.length;
}

export function syncCurrentProjectFolders(
  host: { namedProjects: IdeNamedProject[]; activeProjectId: string | null; target: IdeTargetSpec; additionalRoots: string[] },
): void {
  if (!host.activeProjectId || !host.target.root) return;
  const primary = normalizeRootPath(host.target.root);
  const seenKeys = new Set<string>([canonicalRootKey(primary)]);
  const folders = [primary];
  for (let i = 0; i < host.additionalRoots.length; i++) {
    const f = normalizeRootPath(host.additionalRoots[i]);
    const k = canonicalRootKey(f);
    if (f && k && !seenKeys.has(k)) {
      seenKeys.add(k);
      folders.push(f);
    }
  }
  const cur = host.namedProjects.find((p) => p.id === host.activeProjectId);
  if (!cur) return;
  if (
    cur.folders.length === folders.length &&
    cur.folders.every((f, idx) => canonicalRootKey(f) === canonicalRootKey(folders[idx]))
  ) {
    return;
  }
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
  evict: (paths: string[], clearProblems?: boolean) => void,
  setActiveRaw: (p: string | null) => void,
  op: 'mkdir' | 'create_file' | 'rename' | 'delete' | 'duplicate',
  path: string,
  destPath?: string,
): Promise<boolean> {
  try {
    const prefs = readIdePreferences();
    const containing = findContainingRoot(path, host.projectRoots);
    const targetSpec = containing && !containing.isPrimary && host.target.kind === 'local'
      ? { ...host.targetPayload, root: containing.path }
      : host.targetPayload;
    const resp = await api.post<{ ok: boolean; path: string }>('/ide/fs/mutate', {
      target: targetSpec,
      op,
      path,
      dest_path: destPath,
      local_history_enabled: prefs.localHistoryEnabled,
      local_history_retention_days: prefs.localHistoryRetentionDays,
    });
    const parentOf = (p: string) => (p.lastIndexOf('/') > 0 ? p.slice(0, p.lastIndexOf('/')) : (containing?.path ?? host.target.root));
    await host.loadTree(containing && !containing.isPrimary ? containing.path : undefined);
    for (const dir of Object.keys(host.expandedDirs)) {
      if (host.expandedDirs[dir] && (dir === parentOf(path) || (destPath && dir === parentOf(destPath)))) await host.loadTree(dir);
    }
    if (op === 'delete') {
      const deleted = host.tabs.filter((t) => t.path === path || t.path.startsWith(`${path}/`)).map((t) => t.path);
      host.tabs = host.tabs.filter((t) => t.path !== path && !t.path.startsWith(`${path}/`));
      evict(deleted, true);
      if (host.activePath && deleted.includes(host.activePath)) host.activePath = host.tabs[0]?.path ?? null;
      if (host.splitPath && deleted.includes(host.splitPath)) host.closeSplit();
    } else if (op === 'rename' && destPath) {
      const mapRenamed = (p: string): string =>
        p === path ? destPath : p.startsWith(`${path}/`) ? `${destPath}${p.slice(path.length)}` : p;
      const affected = host.tabs
        .filter((t) => t.path === path || t.path.startsWith(`${path}/`))
        .map((t) => t.path);
      for (let i = 0; i < affected.length; i++) {
        const oldPath = affected[i];
        flushBuf(oldPath);
        const draft = readIdeUndoDraft(oldPath);
        if (draft) {
          writeIdeUndoDraftImmediate(mapRenamed(oldPath), draft.savedContent, draft.draftContent);
          removeIdeUndoDraft(oldPath);
        }
      }
      evict(affected, false);
      host.tabs = host.tabs.map((t) => {
        const nextPath = mapRenamed(t.path);
        return nextPath === t.path ? t : { ...t, path: nextPath, name: ideBasename(nextPath) };
      });
      if (host.activePath) {
        const nextActive = mapRenamed(host.activePath);
        if (nextActive !== host.activePath) setActiveRaw(nextActive);
      }
      if (host.splitPath) {
        const nextSplit = mapRenamed(host.splitPath);
        if (nextSplit !== host.splitPath) host.splitPath = nextSplit;
      }
    } else if ((op === 'create_file' || op === 'duplicate') && resp.path) {
      await host.openFile(resp.path, { preview: false });
    }
    return true;
  } catch (e) {
    toastError(`Couldn’t ${op.replace('_', ' ')}`, e);
    return false;
  }
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
    const resp = await api.bg.post<{
      has_maven?: boolean;
      has_gradle?: boolean;
      tool?: 'maven' | 'gradle' | null;
      projects?: { tool?: 'maven' | 'gradle'; kind?: 'maven' | 'gradle' }[];
      modules?: { tool?: 'maven' | 'gradle'; kind?: 'maven' | 'gradle' }[];
    }>(
      '/ide/build-tools/model',
      { target: host.targetPayload, roots: host.projectRoots.map((r) => r.path) },
    );
    if (host.target.root !== startRoot) return;
    const detected = resp.has_maven
      ? 'maven'
      : resp.has_gradle
        ? 'gradle'
        : (resp.projects?.[0]?.tool ?? resp.projects?.[0]?.kind ?? resp.modules?.[0]?.tool ?? resp.modules?.[0]?.kind ?? resp.tool ?? null);
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
