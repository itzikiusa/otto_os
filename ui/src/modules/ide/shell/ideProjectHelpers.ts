// Multi-root project management, file template creation, navigation history,
// and instant zero-wait runner seeding for `IdeStore`.

import { api } from '../../../lib/api/client';
import { confirmer } from '../../../lib/confirm.svelte';
import { lsGet, lsSet } from '../../../lib/storage';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import {
  buildNewFileTemplate,
  ideBasename,
  type IdeBreakpoint,
  type IdeFsEntry,
  type IdeFsTreeResp,
  type IdeNavLocation,
  type IdeOpenTab,
  type IdeProjectEnvironment,
  type IdeProjectRoot,
  type IdeRunConfig,
  type IdeTargetSpec,
} from '../store/ideTypes';

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
  openFile(rawPath: string, opts?: { line?: number; col?: number; preview?: boolean; inSplit?: boolean }): Promise<void>;
  saveEnvironment(patch: Partial<IdeProjectEnvironment>): Promise<boolean>;
}

export function normalizeRootPath(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed || trimmed === '/') return trimmed;
  return trimmed.replace(/\/+$/, '');
}

const PRIVATE_SYMLINK_RE = /^\/private(?=\/(?:var|tmp)(?:\/|$))/;

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
  opts: { force?: boolean } = {},
): Promise<void> {
  if (host.tabs.length === 0) {
    host.hasClosedAllTabs = true;
    host.activePath = null;
    host.closeSplit();
    return;
  }
  bufPerf.flushAll(host);
  const dirtyTabs = host.tabs.filter((t) => bufPerf.isTabDirty(t));
  if (!opts.force && dirtyTabs.length > 0) {
    const promptMsg =
      dirtyTabs.length === 1
        ? `“${dirtyTabs[0].name}” has unsaved changes. Discard changes and close all tabs?`
        : `${dirtyTabs.length} tabs have unsaved changes. Discard changes and close all tabs?`;
    const ok = await confirmer.ask(promptMsg, {
      title: 'Close all tabs?',
      confirmLabel: 'Discard & close all',
      danger: true,
    });
    if (!ok) return;
  }
  const allPaths = host.tabs.map((t) => t.path);
  host.tabs = [];
  evict(allPaths);
  host.hasClosedAllTabs = true;
  host.activePath = null;
  host.closeSplit();
}

export function projectStorageKey(target: IdeTargetSpec): string {
  if (!target.root) return '';
  return target.kind === 'ssh'
    ? `ssh:${target.connection_id ?? ''}:${normalizeRootPath(target.root)}`
    : `local:${normalizeRootPath(target.root)}`;
}

export function loadSavedAdditionalRoots(target: IdeTargetSpec): string[] {
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

export function isPathExternal(fullPath: string, roots: IdeProjectRoot[]): boolean {
  if (!fullPath || /^(?:jdt|jar):/i.test(fullPath)) return true;
  for (let i = 0; i < roots.length; i++) {
    const r = normalizeRootPath(roots[i].path);
    if (!r) continue;
    if (fullPath === r || fullPath.startsWith(`${r}/`)) return false;
  }
  return true;
}

export function formatRelativeToRoots(fullPath: string, primaryRoot: string, roots: IdeProjectRoot[]): string {
  if (/^(?:jdt|jar):/i.test(fullPath)) {
    const bang = fullPath.lastIndexOf('!/');
    return bang >= 0 ? fullPath.slice(bang + 2).split('?')[0] : ideBasename(fullPath);
  }
  const primary = normalizeRootPath(primaryRoot);
  if (primary && fullPath.startsWith(`${primary}/`)) return fullPath.slice(primary.length + 1);
  for (let i = 0; i < roots.length; i++) {
    const r = normalizeRootPath(roots[i].path);
    if (r && r !== primary && fullPath.startsWith(`${r}/`)) {
      return `${roots[i].name}/${fullPath.slice(r.length + 1)}`;
    }
  }
  return fullPath;
}

export function findContainingRoot(fullPath: string, roots: IdeProjectRoot[]): IdeProjectRoot | null {
  let best: IdeProjectRoot | null = null;
  for (let i = 0; i < roots.length; i++) {
    const r = normalizeRootPath(roots[i].path);
    if (!r) continue;
    if (fullPath === r || fullPath.startsWith(`${r}/`)) {
      if (!best || r.length > best.path.length) best = roots[i];
    }
  }
  return best;
}

export async function loadAttachedRootEntries(host: ProjectHelperHost, rootPath: string): Promise<void> {
  const clean = normalizeRootPath(rootPath);
  if (!clean) return;
  try {
    const query = host.target.kind === 'local'
      ? new URLSearchParams({ kind: 'local', root: clean, show_hidden: String(host.showHidden) }).toString()
      : host.targetQuery({ path: clean, show_hidden: host.showHidden });
    const resp = await api.get<IdeFsTreeResp>(`/ide/fs/tree?${query}`);
    host.childrenByDir = { ...host.childrenByDir, [clean]: resp.entries ?? [] };
  } catch (e) {
    toastError(`Couldn’t open folder ${ideBasename(clean)}`, e);
  }
}

export async function attachProjectFolder(host: ProjectHelperHost, rawFolder: string): Promise<boolean> {
  const clean = normalizeRootPath(rawFolder);
  const primary = normalizeRootPath(host.target.root);
  if (!clean || clean === primary || host.additionalRoots.includes(clean)) {
    if (clean === primary || host.additionalRoots.includes(clean)) {
      toasts.info('Folder is already in this project');
    }
    return false;
  }
  const nextRoots = [...host.additionalRoots, clean];
  host.additionalRoots = nextRoots;
  host.expandedDirs = { ...host.expandedDirs, [clean]: true };
  saveAdditionalRootsLocal(host.target, nextRoots);
  await loadAttachedRootEntries(host, clean);

  // Recompute external status for open tabs inside the newly attached folder.
  const allRoots = host.projectRoots;
  for (let i = 0; i < host.tabs.length; i++) {
    host.tabs[i].external = isPathExternal(host.tabs[i].path, allRoots);
  }

  try {
    const resp = await api.post<{ env: IdeProjectEnvironment }>('/ide/projects/folders', {
      target: host.targetPayload,
      folder_path: clean,
    });
    if (resp?.env) host.projectEnv = resp.env;
  } catch {
    await host.saveEnvironment({ additional_roots: nextRoots });
  }
  toasts.success(`Added ${ideBasename(clean)} to project`);
  return true;
}

export async function detachProjectFolder(host: ProjectHelperHost, rawFolder: string): Promise<boolean> {
  const clean = normalizeRootPath(rawFolder);
  if (!host.additionalRoots.includes(clean)) return false;
  const nextRoots = host.additionalRoots.filter((r) => r !== clean);
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

  try {
    const q = host.targetQuery({ folder_path: clean });
    const resp = await api.del<{ env?: IdeProjectEnvironment }>(`/ide/projects/folders?${q}`);
    if (resp?.env) host.projectEnv = resp.env;
  } catch {
    await host.saveEnvironment({ additional_roots: nextRoots });
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
    await api.put('/ide/fs/file', {
      target: host.targetPayload,
      path: fullPath,
      content: template,
      create_dirs: true,
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
