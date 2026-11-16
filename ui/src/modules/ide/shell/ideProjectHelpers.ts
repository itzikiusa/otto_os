// Multi-root project management, file template creation, navigation history,
// and instant zero-wait runner seeding for `IdeStore`.

import { api } from '../../../lib/api/client';
import { readIdePreferences } from '../../../lib/components/cm-ide-prefs';
import { confirmer } from '../../../lib/confirm.svelte';
import {
  loadNamedIdeProjectsLocal,
  saveNamedIdeProjectsLocal,
  type NamedIdeProject,
} from '../../../lib/ideProjectsModel';
import { ideWorkspacePort } from '../../../lib/ideWorkspacePort.svelte';
import { lsGet, lsSet } from '../../../lib/storage';
import { toasts } from '../../../lib/toast.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { toastError } from '../../../lib/toastError';
import { ideTreeDirErrors } from '../store/ideTreeDirErrors.svelte';
import {
  autoOpenInitialEntry,
  buildNewFileTemplate,
  ideBasename,
  ideTargetKey,
  type IdeBreakpoint,
  type IdeFsEntry,
  type IdeFsTreeResp,
  type IdeNamedProject,
  type IdeNavLocation,
  type IdeOpenTab,
  type IdeProjectEnvironment,
  type IdeProjectRoot,
  type IdeRunConfig,
  type IdeTargetSpec,
} from '../store/ideTypes';
import { loadWorkspaceOpenTabs } from '../store/ideBufferPerf';

export interface ProjectHelperHost {
  target: IdeTargetSpec;
  readonly targetPayload: IdeTargetSpec;
  additionalRoots: string[];
  readonly projectRoots: IdeProjectRoot[];
  treeRootEntries: IdeFsEntry[];
  childrenByDir: Record<string, IdeFsEntry[]>;
  expandedDirs: Record<string, boolean>;
  showHidden: boolean;
  tabs: IdeOpenTab[];
  projectEnv: IdeProjectEnvironment | null;
  treeRevealRequest: { path: string; seq: number } | null;
  targetQuery(extra?: Record<string, string | boolean | null | undefined>): string;
  resolvePath(p: string): string;
  loadTree(dirPath?: string): Promise<void>;
  openFile(rawPath: string, opts?: { line?: number; col?: number; preview?: boolean; inSplit?: boolean; background?: boolean }): Promise<void>;
  saveEnvironment(patch: Partial<IdeProjectEnvironment>): Promise<boolean>;
}

export function normalizeRootPath(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed || trimmed === '/') return trimmed;
  return trimmed.replace(/\/+$/, '');
}

const PRIVATE_SYMLINK_RE = /^\/private(?=\/(?:var|tmp)(?:\/|$))/;

export function canonicalRootKey(raw: string): string {
  const clean = normalizeRootPath(raw);
  return clean.replace(PRIVATE_SYMLINK_RE, '');
}

export function canonicalizeLocalProjectPath(
  fullPath: string,
  roots: readonly IdeProjectRoot[],
): string {
  if (!fullPath || !fullPath.startsWith('/') || roots.length === 0) return fullPath;
  const cPath = fullPath.replace(PRIVATE_SYMLINK_RE, '');
  for (let i = 0; i < roots.length; i++) {
    const root = normalizeRootPath(roots[i].path);
    if (!root) continue;
    const cRoot = root.replace(PRIVATE_SYMLINK_RE, '');
    if (cPath === cRoot || cPath.startsWith(`${cRoot}/`)) {
      return `${root}${cPath.slice(cRoot.length)}`;
    }
  }
  return fullPath;
}

export async function performCloseAllTabs(
  host: {
    tabs: IdeOpenTab[];
    hasClosedAllTabs: boolean;
    activePath: string | null;
    closeSplit(): void;
  },
  bufPerf: { flushAll(h: { tabs: IdeOpenTab[] }): void; isTabDirty(t: IdeOpenTab): boolean },
  evict: (paths: string[]) => void,
  saveTab?: (path: string) => Promise<boolean>,
  opts: { force?: boolean } = {},
): Promise<void> {
  if (host.tabs.length === 0) {
    host.hasClosedAllTabs = true;
    host.activePath = null;
    host.closeSplit();
    return;
  }
  const discard =
    opts.force ||
    (await askDiscardDirty(
      host,
      bufPerf,
      async (dirty) => {
        if (!saveTab) return;
        for (const t of dirty) if (!t.readOnly) await saveTab(t.path);
      },
      { question: 'Discard changes and close all tabs?', title: 'Close all tabs?', confirmLabel: 'Discard & close all' },
    ));
  if (!discard) return;
  const allPaths = host.tabs.map((t) => t.path);
  host.tabs = [];
  evict(allPaths);
  host.hasClosedAllTabs = true;
  host.activePath = null;
  host.closeSplit();
}

/** Flush buffers, autosave (when on) what is dirty, then ask before discarding what is still unsaved. */
async function askDiscardDirty(
  host: { tabs: IdeOpenTab[] },
  bufPerf: { flushAll(h: { tabs: IdeOpenTab[] }): void; isTabDirty(t: IdeOpenTab): boolean },
  autosave: (dirty: IdeOpenTab[]) => Promise<void>,
  copy: { question: string; title: string; confirmLabel: string },
): Promise<boolean> {
  bufPerf.flushAll(host);
  let dirty = host.tabs.filter((t) => bufPerf.isTabDirty(t));
  if (dirty.length > 0 && readIdePreferences().autoSave) {
    await autosave(dirty);
    dirty = host.tabs.filter((t) => bufPerf.isTabDirty(t));
  }
  if (dirty.length === 0) return true;
  const what = dirty.length === 1 ? `“${dirty[0].name}” has` : `${dirty.length} tabs have`;
  return confirmer.ask(`${what} unsaved changes. ${copy.question}`, { title: copy.title, confirmLabel: copy.confirmLabel, danger: true });
}

/** A target switch drops every tab: autosave what it can, then ask before
 *  discarding what is still unsaved (autosave off, a conflict, a failed save). */
export function confirmDiscardOnSwitch(
  host: { target: IdeTargetSpec; tabs: IdeOpenTab[] },
  bufPerf: { flushAll(h: { tabs: IdeOpenTab[] }): void; isTabDirty(t: IdeOpenTab): boolean },
  flushAutoSave: () => Promise<void>,
): Promise<boolean> {
  if (!host.target.root) return Promise.resolve(true);
  return askDiscardDirty(host, bufPerf, flushAutoSave, {
    question: 'Discard them and switch?',
    title: 'Discard unsaved changes?',
    confirmLabel: 'Discard & switch',
  });
}

export function projectStorageKey(target: IdeTargetSpec): string {
  if (!target.root) return '';
  return target.kind === 'ssh'
    ? `ssh:${target.connection_id ?? ''}:${normalizeRootPath(target.root)}`
    : `local:${normalizeRootPath(target.root)}`;
}

export function loadSavedAdditionalRoots(target: IdeTargetSpec): string[] {
  if (!target.root) return [];
  if (target.kind === 'local') {
    return ideWorkspacePort.rootsForLocal(target.root);
  }
  const key = projectStorageKey(target);
  if (!key) return [];
  try {
    const raw = lsGet(`otto.ide.roots:${key}`);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed.map((x) => normalizeRootPath(String(x))).filter(Boolean) : [];
  } catch {
    return [];
  }
}

export function saveAdditionalRootsLocal(target: IdeTargetSpec, roots: string[]): void {
  if (!target.root) return;
  if (target.kind === 'local') {
    ideWorkspacePort.saveRootsForLocal(target.root, roots);
    return;
  }
  const key = projectStorageKey(target);
  if (!key) return;
  try {
    lsSet(`otto.ide.roots:${key}`, JSON.stringify(roots));
  } catch { /* quota */ }
}

export function loadSavedBreakpoints(target: IdeTargetSpec): Record<string, IdeBreakpoint[]> {
  const key = projectStorageKey(target);
  if (!key) return {};
  try {
    const raw = lsGet(`otto.ide.bps:${key}`);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, IdeBreakpoint[]>;
    return parsed && typeof parsed === 'object' ? parsed : {};
  } catch {
    return {};
  }
}

export function normalizeBreakpointsMap(
  raw: Record<string, IdeBreakpoint[]>,
  resolvePath: (p: string) => string,
): Record<string, IdeBreakpoint[]> {
  const out: Record<string, IdeBreakpoint[]> = {};
  for (const [rawKey, list] of Object.entries(raw)) {
    if (!Array.isArray(list) || list.length === 0) continue;
    const normPath = resolvePath(rawKey);
    const merged = [...(out[normPath] ?? [])];
    for (let i = 0; i < list.length; i++) {
      const bp = list[i];
      if (!bp || typeof bp.line !== 'number') continue;
      const line = Math.max(1, Math.floor(bp.line));
      if (!merged.some((x) => x.line === line)) {
        merged.push({ ...bp, path: normPath, line, enabled: bp.enabled !== false });
      }
    }
    if (merged.length > 0) out[normPath] = merged.sort((a, b) => a.line - b.line);
  }
  return out;
}

export function saveBreakpointsLocal(target: IdeTargetSpec, map: Record<string, IdeBreakpoint[]>): void {
  const key = projectStorageKey(target);
  if (!key) return;
  try {
    const compact: Record<string, IdeBreakpoint[]> = {};
    for (const [k, v] of Object.entries(map)) {
      if (Array.isArray(v) && v.length > 0) {
        compact[k] = v.map(({ verified: _v, message: _m, ...rest }) => rest);
      }
    }
    lsSet(`otto.ide.bps:${key}`, JSON.stringify(compact));
  } catch { /* quota */ }
}

export function mutatePathBreakpoints(
  host: {
    target: IdeTargetSpec;
    breakpointsByPath: Record<string, IdeBreakpoint[]>;
    resolvePath(p: string): string;
    syncBreakpointsToSession(): Promise<void>;
  },
  rawPath: string,
  compute: (normPath: string, current: IdeBreakpoint[]) => IdeBreakpoint[],
): void {
  const path = host.resolvePath(rawPath);
  const current = host.breakpointsByPath[path] ?? [];
  const clean = compute(path, current)
    .map((b) => ({ ...b, path, line: Math.max(1, Math.floor(b.line)) }))
    .sort((a, b) => a.line - b.line);
  const nextMap: Record<string, IdeBreakpoint[]> = { ...host.breakpointsByPath };
  if (clean.length > 0) nextMap[path] = clean;
  else delete nextMap[path];
  if (rawPath !== path) delete nextMap[rawPath];
  host.breakpointsByPath = nextMap;
  saveBreakpointsLocal(host.target, nextMap);
  void host.syncBreakpointsToSession();
}

export function isPathExternal(fullPath: string, roots: IdeProjectRoot[]): boolean {
  if (!fullPath || /^(?:jdt|jar):/i.test(fullPath)) return true;
  const cFull = canonicalRootKey(fullPath);
  for (let i = 0; i < roots.length; i++) {
    const r = canonicalRootKey(roots[i].path);
    if (!r) continue;
    if (cFull === r || cFull.startsWith(`${r}/`)) return false;
  }
  return true;
}

export function formatRelativeToRoots(fullPath: string, primaryRoot: string, roots: IdeProjectRoot[]): string {
  if (/^(?:jdt|jar):/i.test(fullPath)) {
    const bang = fullPath.lastIndexOf('!/');
    return bang >= 0 ? fullPath.slice(bang + 2).split('?')[0] : ideBasename(fullPath);
  }
  const cFull = canonicalRootKey(fullPath);
  const primary = canonicalRootKey(primaryRoot);
  if (primary && cFull.startsWith(`${primary}/`)) return cFull.slice(primary.length + 1);
  for (let i = 0; i < roots.length; i++) {
    const r = canonicalRootKey(roots[i].path);
    if (r && r !== primary && cFull.startsWith(`${r}/`)) {
      return `${roots[i].name}/${cFull.slice(r.length + 1)}`;
    }
  }
  return fullPath;
}

export function findContainingRoot(fullPath: string, roots: IdeProjectRoot[]): IdeProjectRoot | null {
  let best: IdeProjectRoot | null = null;
  let bestLen = -1;
  const cFull = canonicalRootKey(fullPath);
  for (let i = 0; i < roots.length; i++) {
    const r = canonicalRootKey(roots[i].path);
    if (!r) continue;
    if (cFull === r || cFull.startsWith(`${r}/`)) {
      if (r.length > bestLen) {
        best = roots[i];
        bestLen = r.length;
      }
    }
  }
  return best;
}

export async function loadAttachedRootEntries(host: ProjectHelperHost, rootPath: string): Promise<void> {
  const clean = normalizeRootPath(rootPath);
  if (!clean) return;
  const key = ideTargetKey(host.target);
  try {
    const query = host.target.kind === 'local'
      ? new URLSearchParams({ kind: 'local', root: clean, show_hidden: String(host.showHidden) }).toString()
      : host.targetQuery({ path: clean, show_hidden: host.showHidden });
    const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${query}`);
    if (ideTargetKey(host.target) !== key) return;
    const rawEntries = resp.entries ?? [];
    const entries = host.target.kind === 'local'
      ? rawEntries.map((e) => ({ ...e, path: canonicalizeLocalProjectPath(e.path, host.projectRoots) }))
      : rawEntries;
    host.childrenByDir = { ...host.childrenByDir, [clean]: entries };
    ideTreeDirErrors.clear(clean);
  } catch (e) {
    // Shown inline under the folder with Retry (IdeProjectTree), not as a toast.
    if (ideTargetKey(host.target) === key) ideTreeDirErrors.set(clean, loadErrorText(e));
  }
}

export async function attachProjectFolder(host: ProjectHelperHost, rawFolder: string): Promise<boolean> {
  const clean = normalizeRootPath(rawFolder);
  const cleanKey = canonicalRootKey(clean);
  const primaryKey = canonicalRootKey(host.target.root);
  const alreadyPresent = cleanKey === primaryKey || host.additionalRoots.some((r) => canonicalRootKey(r) === cleanKey);
  if (!clean || alreadyPresent) {
    if (alreadyPresent) {
      toasts.info('Folder is already in this project');
    }
    return false;
  }
  const nextRoots = [...host.additionalRoots, clean];
  try {
    const resp = await api.post<{ env: IdeProjectEnvironment }>('/ide/projects/folders', {
      target: host.targetPayload,
      folder_path: clean,
    });
    if (resp?.env) host.projectEnv = resp.env;
  } catch (e) {
    toastError(`Couldn’t add ${ideBasename(clean)} to project`, e);
    return false;
  }

  host.additionalRoots = nextRoots;
  host.expandedDirs = { ...host.expandedDirs, [clean]: true };
  saveAdditionalRootsLocal(host.target, nextRoots);
  await loadAttachedRootEntries(host, clean);

  // Recompute external status for open tabs inside the newly attached folder.
  const allRoots = host.projectRoots;
  for (let i = 0; i < host.tabs.length; i++) {
    host.tabs[i].external = isPathExternal(host.tabs[i].path, allRoots);
  }
  toasts.success(`Added ${ideBasename(clean)} to project`);
  return true;
}

export async function detachProjectFolder(host: ProjectHelperHost, rawFolder: string): Promise<boolean> {
  const clean = normalizeRootPath(rawFolder);
  const cleanKey = canonicalRootKey(clean);
  if (!host.additionalRoots.some((r) => canonicalRootKey(r) === cleanKey)) return false;
  const nextRoots = host.additionalRoots.filter((r) => canonicalRootKey(r) !== cleanKey);
  try {
    const q = host.targetQuery({ folder_path: clean });
    const resp = await api.del<{ env?: IdeProjectEnvironment }>(`/ide/projects/folders?${q}`);
    if (resp?.env) host.projectEnv = resp.env;
  } catch (e) {
    toastError(`Couldn’t remove ${ideBasename(clean)} from project`, e);
    return false;
  }

  host.additionalRoots = nextRoots;
  saveAdditionalRootsLocal(host.target, nextRoots);

  const nextChildren: Record<string, IdeFsEntry[]> = {};
  for (const [k, v] of Object.entries(host.childrenByDir)) {
    if (k !== clean && !k.startsWith(`${clean}/`)) nextChildren[k] = v;
  }
  host.childrenByDir = nextChildren;

  const allRoots = host.projectRoots;
  for (let i = 0; i < host.tabs.length; i++) {
    host.tabs[i].external = isPathExternal(host.tabs[i].path, allRoots);
  }
  toasts.info(`Removed ${ideBasename(clean)} from project`);
  return true;
}

export async function createTemplatedFile(
  host: ProjectHelperHost,
  fullPath: string,
  fallbackMutate: (op: 'create_file', p: string) => Promise<boolean>,
): Promise<boolean> {
  const slash = fullPath.lastIndexOf('/');
  const parentDir = slash > 0 ? fullPath.slice(0, slash) : host.target.root;
  const siblings = parentDir === normalizeRootPath(host.target.root)
    ? host.treeRootEntries
    : (host.childrenByDir[parentDir] ?? []);
  const template = buildNewFileTemplate(fullPath, host.target.root, siblings);
  if (!template) {
    return fallbackMutate('create_file', fullPath);
  }
  try {
    const prefs = readIdePreferences();
    await api.put('/ide/fs/file', {
      target: host.targetPayload,
      path: fullPath,
      content: template,
      create_dirs: true,
      local_history_enabled: prefs.localHistoryEnabled,
      local_history_retention_days: prefs.localHistoryRetentionDays,
    });
    const containingRoot = findContainingRoot(fullPath, host.projectRoots);
    if (containingRoot && !containingRoot.isPrimary) {
      await loadAttachedRootEntries(host, containingRoot.path);
    } else {
      await host.loadTree();
    }
    if (parentDir && parentDir !== normalizeRootPath(host.target.root)) {
      host.expandedDirs = { ...host.expandedDirs, [parentDir]: true };
      await host.loadTree(parentDir);
    }
    await host.openFile(fullPath, { preview: false });
    return true;
  } catch {
    return fallbackMutate('create_file', fullPath);
  }
}

export async function revealPathInTree(host: ProjectHelperHost, rawPath: string, seq: number): Promise<void> {
  const fullPath = host.resolvePath(rawPath);
  const root = findContainingRoot(fullPath, host.projectRoots);
  if (!root) return;
  const base = normalizeRootPath(root.path);
  const nextExpanded: Record<string, boolean> = { ...host.expandedDirs, [base]: true };
  const rel = fullPath.startsWith(`${base}/`) ? fullPath.slice(base.length + 1) : '';
  const parts = rel.split('/').filter(Boolean);
  let cursor = base;
  for (let i = 0; i < parts.length - 1; i++) {
    cursor = `${cursor}/${parts[i]}`;
    nextExpanded[cursor] = true;
    if (!host.childrenByDir[cursor]) {
      await host.loadTree(cursor);
    }
  }
  host.expandedDirs = nextExpanded;
  host.treeRevealRequest = { path: fullPath, seq };
}

export function pushNavigationHistory(
  history: IdeNavLocation[],
  index: number,
  loc: IdeNavLocation,
): { history: IdeNavLocation[]; index: number } {
  if (!loc.path) return { history, index };
  const cur = history[index];
  if (cur && cur.path === loc.path && Math.abs(cur.line - loc.line) <= 2) {
    const copy = history.slice();
    copy[index] = loc;
    return { history: copy, index };
  }
  const trimmed = history.slice(0, index + 1);
  trimmed.push(loc);
  const capped = trimmed.length > 60 ? trimmed.slice(trimmed.length - 60) : trimmed;
  return { history: capped, index: capped.length - 1 };
}

export function detectFastRunConfigs(rootPath: string, entries: IdeFsEntry[]): IdeRunConfig[] {
  const names = new Set(entries.map((e) => e.name.toLowerCase()));
  const out: IdeRunConfig[] = [];
  if (names.has('go.mod')) {
    if (names.has('main.go')) {
      out.push({
        id: 'auto:go_run:root', name: 'go run .', runner: 'go_run',
        args: ['.'], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true, target_path: '.',
      });
    }
    out.push({
      id: 'auto:go_test:all', name: 'go test ./...', runner: 'go_test',
      args: ['./...'], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true, target_path: './...',
    });
  }
  if (names.has('cargo.toml')) {
    out.push(
      {
        id: 'auto:cargo_run', name: 'cargo run', runner: 'cargo_run',
        args: [], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true,
      },
      {
        id: 'auto:cargo_test', name: 'cargo test', runner: 'cargo_test',
        args: [], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true,
      },
    );
  }
  if (names.has('package.json')) {
    out.push({
      id: 'auto:npm_test', name: 'npm test', runner: 'npm',
      args: ['test'], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true,
    });
  }
  if (names.has('pyproject.toml') || names.has('pytest.ini') || names.has('requirements.txt')) {
    out.push({
      id: 'auto:pytest', name: 'pytest', runner: 'pytest',
      args: [], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true,
    });
  }
  if (names.has('pom.xml')) {
    out.push({
      id: 'auto:mvn_test', name: 'mvn test', runner: 'maven',
      args: ['test'], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true,
    });
  } else if (names.has('build.gradle') || names.has('build.gradle.kts')) {
    out.push({
      id: 'auto:gradle_test', name: 'gradle test', runner: 'gradle',
      args: ['test'], cwd: rootPath, env_overrides: {}, inherit_shell_env: true, auto_detected: true,
    });
  }
  return out;
}

export function mergeFastRunConfigs(saved: IdeRunConfig[], detected: IdeRunConfig[]): IdeRunConfig[] {
  const explicit = saved.filter((c) => !c.auto_detected);
  const seenIds = new Set(explicit.map((c) => c.id));
  const seenNames = new Set(explicit.map((c) => `${c.runner}:${c.name.toLowerCase()}`));
  const mergedAuto: IdeRunConfig[] = [];
  for (const item of [...saved.filter((c) => c.auto_detected), ...detected]) {
    const key = `${item.runner}:${item.name.toLowerCase()}`;
    if (seenIds.has(item.id) || seenNames.has(key)) continue;
    seenIds.add(item.id);
    seenNames.add(key);
    mergedAuto.push({ ...item, auto_detected: true });
  }
  return [...explicit, ...mergedAuto];
}

const ACTIVE_PROJECT_KEY = 'otto.ide.activeProjectId.v1';
const BUILD_TOOL_PREFS_PREFIX = 'otto.ide.buildToolPrefs:';
const GRADLE_FILES = new Set(['build.gradle', 'build.gradle.kts', 'settings.gradle', 'settings.gradle.kts', 'gradlew']);

export function toIdeNamedProject(p: NamedIdeProject): IdeNamedProject {
  return {
    id: p.id,
    name: p.name,
    primary_root: p.primaryRoot,
    folders: p.folders,
    pinned: p.pinned,
    updated_at: p.updatedAt,
  };
}

export function fromIdeNamedProject(p: IdeNamedProject): NamedIdeProject {
  const rawTs = p.updated_at;
  const updatedAt =
    typeof rawTs === 'number' && Number.isFinite(rawTs)
      ? rawTs
      : rawTs
        ? Date.parse(String(rawTs)) || Date.now()
        : Date.now();
  return {
    id: p.id,
    name: p.name,
    primaryRoot: p.primary_root,
    folders: p.folders,
    pinned: p.pinned,
    updatedAt,
  };
}

export function emitIdeProjectsChanged(projects?: IdeNamedProject[], activeProjectId?: string | null, root?: string): void {
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent('otto:ide-projects-changed', { detail: { projects, activeProjectId, root } }));
  }
}

export function saveLastIdeRoot(root: string): void {
  if (root) ideWorkspacePort.setLastRoot(root);
}
export function loadActiveProjectIdLocal(): string | null { return lsGet(ACTIVE_PROJECT_KEY) || null; }
export function saveActiveProjectIdLocal(id: string | null, projects?: IdeNamedProject[], root?: string): void {
  const next = id ?? '';
  const prev = lsGet(ACTIVE_PROJECT_KEY) ?? '';
  lsSet(ACTIVE_PROJECT_KEY, next);
  if (prev !== next || root) {
    emitIdeProjectsChanged(projects, id, root);
  }
}
export function saveNamedProjectsLocal(
  projects: IdeNamedProject[],
  _activeProjectId?: string | null,
  opts?: { source?: 'local' | 'server' },
): void {
  saveNamedIdeProjectsLocal(projects.map(fromIdeNamedProject), opts);
}

export function loadNamedProjectsLocal(): IdeNamedProject[] {
  return loadNamedIdeProjectsLocal().map(toIdeNamedProject);
}

export interface WorkspaceBuildToolPrefs { open: boolean; mavenProfiles: string[]; springProfiles: string[]; skipTests: boolean; offline: boolean; }

export function loadBuildToolPrefsLocal(root: string): WorkspaceBuildToolPrefs {
  const empty: WorkspaceBuildToolPrefs = { open: false, mavenProfiles: [], springProfiles: [], skipTests: false, offline: false };
  if (!root) return empty;
  try {
    const raw = lsGet(`${BUILD_TOOL_PREFS_PREFIX}${normalizeRootPath(root)}`);
    if (!raw) return empty;
    const p = JSON.parse(raw) as Partial<WorkspaceBuildToolPrefs>;
    return {
      open: Boolean(p.open), mavenProfiles: Array.isArray(p.mavenProfiles) ? p.mavenProfiles : [],
      springProfiles: Array.isArray(p.springProfiles) ? p.springProfiles : [], skipTests: Boolean(p.skipTests), offline: Boolean(p.offline),
    };
  } catch { return empty; }
}

export function saveBuildToolPrefsLocal(root: string, prefs: WorkspaceBuildToolPrefs): void {
  if (root) lsSet(`${BUILD_TOOL_PREFS_PREFIX}${normalizeRootPath(root)}`, JSON.stringify(prefs));
}

export function detectBuildToolFromState(
  override: 'maven' | 'gradle' | null, rootEntries: IdeFsEntry[], childrenByDir: Record<string, IdeFsEntry[]>, tabs: IdeOpenTab[], runConfigs: IdeRunConfig[],
): 'maven' | 'gradle' | null {
  if (override) return override;
  const check = (name: string): 'maven' | 'gradle' | null => {
    const l = name.toLowerCase();
    return l === 'pom.xml' ? 'maven' : GRADLE_FILES.has(l) ? 'gradle' : null;
  };
  for (let i = 0; i < rootEntries.length; i++) { const k = check(rootEntries[i].name); if (k) return k; }
  for (const entries of Object.values(childrenByDir)) for (let i = 0; i < entries.length; i++) { const k = check(entries[i].name); if (k) return k; }
  for (let i = 0; i < tabs.length; i++) { const k = check(tabs[i].name); if (k) return k; }
  for (let i = 0; i < runConfigs.length; i++) {
    const r = runConfigs[i].runner;
    if (r === 'maven' || r === 'java_maven') return 'maven';
    if (r === 'gradle' || r === 'java_gradle') return 'gradle';
  }
  return null;
}

export async function restoreWorkspaceTabsOrInitial(host: {
  target: IdeTargetSpec; collabIsGuest: boolean; tabs: IdeOpenTab[]; hasClosedAllTabs: boolean; activePath: string | null;
  treeRootEntries: IdeFsEntry[]; childrenByDir: Record<string, IdeFsEntry[]>;
  toggleDir(dirPath: string): Promise<void>; openFile(rawPath: string, opts?: { preview?: boolean }): Promise<void>; openInSplit(path: string, dir?: 'right' | 'down'): void;
}): Promise<void> {
  if (host.collabIsGuest || host.tabs.length > 0 || host.hasClosedAllTabs) return;
  if (typeof window !== 'undefined' && /[?&](file|collab)=/.test(window.location.hash)) return;
  const saved = loadWorkspaceOpenTabs(host.target.root);
  // Stop as soon as the project switches: the rest of the list belongs to the old one.
  const key = ideTargetKey(host.target);
  if (saved && saved.paths.length > 0) {
    for (let i = 0; i < saved.paths.length; i++) {
      if (ideTargetKey(host.target) !== key) return;
      await host.openFile(saved.paths[i], { preview: false });
    }
    if (ideTargetKey(host.target) !== key) return;
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
