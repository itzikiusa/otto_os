// Pure IDE project discovery, canonical path matching, alias table building,
// and thread-to-IDE-project grouping for the Threads sidebar and `@` completion.
// Zero Svelte/DOM runtime imports so `node --test` runs directly against it.

import type { Session } from '../../lib/api/types';

export const IDE_NAMED_PROJECTS_KEY = 'otto.ide.projects.v1';
export const IDE_LAST_ROOT_KEY = 'otto.ide.lastRoot';
export const IDE_ROOTS_KEY_PREFIX = 'otto.ide.roots:local:';
export const IDE_OPEN_HANDOFF_KEY = 'otto.ide.openThreadHandoff';
export const IDE_PROJECTS_CHANGED_EVENT = 'otto:ide-projects-changed';
export const IDE_CONTEXT_CHANGED_EVENT = 'otto:ide-context-changed';
export const IDE_OPEN_THREAD_EVENT = 'otto:ide-open-thread';

export interface NamedIdeProjectRecord {
  id: string;
  name: string;
  primary_root: string;
  folders: string[];
  pinned?: boolean;
}

export interface NamedIdeProjectInput {
  id: string;
  name: string;
  primaryRoot?: string;
  primary_root?: string;
  folders: readonly string[];
  pinned?: boolean;
}

export interface IdeSidebarProject {
  id: string;
  name: string;
  metaLabel: string | null;
  primaryRoot: string;
  folders: string[];
  pinned: boolean;
}

export interface IdeProjectGroup {
  project: IdeSidebarProject;
  items: Session[];
}

export interface IdeOpenTabSnapshot {
  path: string;
  name: string;
}

export interface IdeLiveContextSnapshot {
  root: string;
  additionalRoots: string[];
  activePath: string | null;
  openTabs: IdeOpenTabSnapshot[];
}

export function trimTrailingSlashes(path: string): string {
  const trimmed = path.trim();
  if (!trimmed || trimmed === '/') return trimmed;
  return trimmed.replace(/\/+$/, '');
}

/** Normalize macOS `/private/var` & `/private/tmp` symlinks + trailing slashes. */
export function canonicalIdePath(raw: string | null | undefined): string {
  const clean = trimTrailingSlashes(raw ?? '');
  if (!clean) return '';
  if (clean === '/private/var' || clean.startsWith('/private/var/')) {
    return clean.slice('/private'.length);
  }
  if (clean === '/private/tmp' || clean.startsWith('/private/tmp/')) {
    return clean.slice('/private'.length);
  }
  return clean;
}

export function basenameOfPath(raw: string): string {
  const clean = trimTrailingSlashes(raw);
  if (!clean || clean === '~') return clean;
  const idx = clean.lastIndexOf('/');
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

export function isSameOrSubpath(candidate: string | null | undefined, root: string | null | undefined): boolean {
  const c = canonicalIdePath(candidate);
  const r = canonicalIdePath(root);
  if (!c || !r || r === '~') return false;
  return c === r || c.startsWith(`${r}/`);
}

export function parseNamedIdeProjects(raw: string | null | undefined): NamedIdeProjectRecord[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: NamedIdeProjectRecord[] = [];
    for (let i = 0; i < parsed.length; i++) {
      const entry = parsed[i];
      if (!entry || typeof entry !== 'object') continue;
      const rec = entry as Record<string, unknown>;
      const rawPrimary =
        typeof rec.primaryRoot === 'string'
          ? rec.primaryRoot
          : typeof rec.primary_root === 'string'
            ? rec.primary_root
            : '';
      const primary = trimTrailingSlashes(rawPrimary);
      const foldersRaw = Array.isArray(rec.folders) ? rec.folders : [];
      const folders: string[] = [];
      const pushFolder = (f: string): void => {
        const clean = trimTrailingSlashes(f);
        if (!clean || clean === '~') return;
        const canon = canonicalIdePath(clean);
        if (!folders.some((x) => canonicalIdePath(x) === canon)) folders.push(clean);
      };
      if (primary) pushFolder(primary);
      for (let j = 0; j < foldersRaw.length; j++) {
        if (typeof foldersRaw[j] === 'string') pushFolder(foldersRaw[j] as string);
      }
      const resolvedPrimary = primary || folders[0] || '';
      if (!resolvedPrimary) continue;
      const id = typeof rec.id === 'string' && rec.id.trim() ? rec.id.trim() : `ide-proj:${canonicalIdePath(resolvedPrimary)}`;
      const name =
        typeof rec.name === 'string' && rec.name.trim()
          ? rec.name.trim()
          : basenameOfPath(resolvedPrimary) || resolvedPrimary;
      out.push({
        id,
        name,
        primary_root: resolvedPrimary,
        folders,
        pinned: rec.pinned === true,
      });
    }
    return out;
  } catch {
    return [];
  }
}

export function parseSavedAdditionalRoots(raw: string | null | undefined): string[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: string[] = [];
    for (let i = 0; i < parsed.length; i++) {
      if (typeof parsed[i] !== 'string') continue;
      const clean = trimTrailingSlashes(parsed[i] as string);
      if (clean && clean !== '~' && !out.some((x) => canonicalIdePath(x) === canonicalIdePath(clean))) {
        out.push(clean);
      }
    }
    return out;
  } catch {
    return [];
  }
}

export interface DiscoverIdeProjectsInput {
  namedProjects?: readonly NamedIdeProjectInput[];
  lastRoot?: string | null;
  additionalRootsByPrimary?: Readonly<Record<string, readonly string[]>>;
  liveContext?: IdeLiveContextSnapshot | null;
  sessions?: readonly Session[];
}

export function discoverIdeProjects(input: DiscoverIdeProjectsInput): IdeSidebarProject[] {
  const out: IdeSidebarProject[] = [];
  const claimedRoots = new Set<string>();

  const registerProject = (proj: {
    id: string;
    name: string;
    primaryRoot: string;
    folders: readonly string[];
    pinned: boolean;
  }): void => {
    const primaryRoot = trimTrailingSlashes(proj.primaryRoot);
    if (!primaryRoot || primaryRoot === '~') return;
    const primaryCanon = canonicalIdePath(primaryRoot);
    const folders: string[] = [];
    const addFolder = (f: string): void => {
      const clean = trimTrailingSlashes(f);
      if (!clean || clean === '~') return;
      const canon = canonicalIdePath(clean);
      if (!folders.some((x) => canonicalIdePath(x) === canon)) folders.push(clean);
    };
    addFolder(primaryRoot);
    for (let i = 0; i < proj.folders.length; i++) addFolder(proj.folders[i]);
    const extra = input.additionalRootsByPrimary?.[primaryRoot] ?? input.additionalRootsByPrimary?.[primaryCanon] ?? [];
    for (let i = 0; i < extra.length; i++) addFolder(extra[i]);
    if (input.liveContext && canonicalIdePath(input.liveContext.root) === primaryCanon) {
      for (let i = 0; i < input.liveContext.additionalRoots.length; i++) {
        addFolder(input.liveContext.additionalRoots[i]);
      }
    }

    const existingIdx = out.findIndex(
      (p) => p.id === proj.id || canonicalIdePath(p.primaryRoot) === primaryCanon,
    );
    if (existingIdx >= 0) {
      const existing = out[existingIdx];
      const mergedFolders = [...existing.folders];
      for (let i = 0; i < folders.length; i++) {
        const canon = canonicalIdePath(folders[i]);
        if (!mergedFolders.some((x) => canonicalIdePath(x) === canon)) mergedFolders.push(folders[i]);
      }
      out[existingIdx] = {
        ...existing,
        folders: mergedFolders,
        metaLabel: mergedFolders.length > 1 ? `${mergedFolders.length} repos` : null,
        pinned: existing.pinned || proj.pinned,
      };
      for (let i = 0; i < mergedFolders.length; i++) claimedRoots.add(canonicalIdePath(mergedFolders[i]));
      return;
    }

    for (let i = 0; i < folders.length; i++) claimedRoots.add(canonicalIdePath(folders[i]));
    out.push({
      id: proj.id,
      name: proj.name || basenameOfPath(primaryRoot) || primaryRoot,
      metaLabel: folders.length > 1 ? `${folders.length} repos` : null,
      primaryRoot,
      folders,
      pinned: proj.pinned,
    });
  };

  for (const np of input.namedProjects ?? []) {
    registerProject({
      id: np.id,
      name: np.name,
      primaryRoot: np.primaryRoot ?? np.primary_root ?? '',
      folders: np.folders,
      pinned: np.pinned === true,
    });
  }

  const candidateRoots: Array<{ root: string; extra: readonly string[] }> = [];
  if (input.liveContext?.root) {
    candidateRoots.push({ root: input.liveContext.root, extra: input.liveContext.additionalRoots });
  }
  if (input.lastRoot) {
    candidateRoots.push({ root: input.lastRoot, extra: [] });
  }
  if (input.additionalRootsByPrimary) {
    for (const [primary, extra] of Object.entries(input.additionalRootsByPrimary)) {
      candidateRoots.push({ root: primary, extra });
    }
  }
  for (const s of input.sessions ?? []) {
    const metaRoot = typeof s.meta?.ide_root === 'string' ? s.meta.ide_root : null;
    if (metaRoot) candidateRoots.push({ root: metaRoot, extra: [] });
    else if (s.meta?.surface === 'ide' && s.cwd) candidateRoots.push({ root: s.cwd, extra: [] });
  }

  for (const cand of candidateRoots) {
    const clean = trimTrailingSlashes(cand.root);
    if (!clean || clean === '~') continue;
    const canon = canonicalIdePath(clean);
    if (claimedRoots.has(canon)) continue;
    registerProject({
      id: `ide-root:${canon}`,
      name: basenameOfPath(clean) || clean,
      primaryRoot: clean,
      folders: [clean, ...cand.extra],
      pinned: false,
    });
  }

  return out.sort((a, b) => {
    if (a.pinned !== b.pinned) return a.pinned ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

/** Find the best matching IDE project for a session (preferring explicit meta id, then deepest matching folder). */
export function matchSessionToIdeProject(
  session: Pick<Session, 'cwd' | 'meta'>,
  projects: readonly IdeSidebarProject[],
): IdeSidebarProject | null {
  if (projects.length === 0) return null;
  const metaProjId = typeof session.meta?.ide_project_id === 'string' ? session.meta.ide_project_id : null;
  if (metaProjId) {
    const byId = projects.find((p) => p.id === metaProjId);
    if (byId) return byId;
  }
  const cwd = session.cwd ?? (typeof session.meta?.ide_root === 'string' ? session.meta.ide_root : null);
  if (!cwd) return null;

  let best: IdeSidebarProject | null = null;
  let bestDepth = -1;
  for (let i = 0; i < projects.length; i++) {
    const proj = projects[i];
    for (let j = 0; j < proj.folders.length; j++) {
      const folder = proj.folders[j];
      if (isSameOrSubpath(cwd, folder)) {
        const depth = canonicalIdePath(folder).length;
        if (depth > bestDepth) {
          best = proj;
          bestDepth = depth;
        }
      }
    }
  }
  return best;
}

export function groupThreadsByIdeProject(
  projects: readonly IdeSidebarProject[],
  sessions: readonly Session[],
): { groups: IdeProjectGroup[]; matchedSessionIds: Set<string> } {
  const byProjectId = new Map<string, Session[]>();
  for (let i = 0; i < projects.length; i++) byProjectId.set(projects[i].id, []);
  const matchedSessionIds = new Set<string>();

  for (let i = 0; i < sessions.length; i++) {
    const s = sessions[i];
    const proj = matchSessionToIdeProject(s, projects);
    if (!proj) continue;
    byProjectId.get(proj.id)?.push(s);
    matchedSessionIds.add(s.id);
  }

  return {
    groups: projects.map((project) => ({
      project,
      items: byProjectId.get(project.id) ?? [],
    })),
    matchedSessionIds,
  };
}

/** Build `@<alias>/...` map across IDE projects, git repos, and workspaces. */
export function buildIdePathAliases(opts: {
  projects?: readonly IdeSidebarProject[];
  repos?: readonly { name?: string; path: string }[];
  workspaces?: readonly { name?: string; root_path?: string | null }[];
}): Record<string, string> {
  const aliases: Record<string, string> = {};
  const put = (alias: string | undefined | null, targetPath: string | undefined | null): void => {
    const cleanTarget = trimTrailingSlashes(targetPath ?? '');
    const cleanAlias = (alias ?? '').trim().replace(/^@+/, '').replace(/\/+$/, '');
    if (!cleanAlias || !cleanTarget || cleanTarget === '~' || /\s/.test(cleanAlias)) return;
    if (!aliases[cleanAlias]) aliases[cleanAlias] = cleanTarget;
  };

  for (const proj of opts.projects ?? []) {
    put(proj.name, proj.primaryRoot);
    put(basenameOfPath(proj.primaryRoot), proj.primaryRoot);
    for (let i = 0; i < proj.folders.length; i++) {
      put(basenameOfPath(proj.folders[i]), proj.folders[i]);
    }
  }
  for (const repo of opts.repos ?? []) {
    put(repo.name, repo.path);
    put(basenameOfPath(repo.path), repo.path);
  }
  for (const w of opts.workspaces ?? []) {
    if (w.root_path) {
      put(w.name, w.root_path);
      put(basenameOfPath(w.root_path), w.root_path);
    }
  }
  return aliases;
}
